//! Compile memory follows its requesting cargo and excludes the shared cache server.

use std::error::Error;
use std::ffi::OsStr;
use std::ffi::OsString;
use std::path::Path;
use std::time::Instant;

use sysinfo::Pid;

use super::CargoGroup;
use super::Measurement;
use super::scan::Census;
use super::scan::CensusSequence;
use super::scan::DetailPresence;
use super::scan::ProcessField;
use super::scan::ProcessObservation;
use super::scan::ProcessObservations;

fn process<'scan>(
    pid: u32,
    name: &'static str,
    argv: &'scan [OsString],
    parent: ProcessField<Pid>,
    memory: u64,
) -> ProcessObservation<'scan> {
    let mut process = ProcessObservation::cargo(pid, argv);
    process.name = ProcessField::Observed(OsStr::new(name));
    process.parent = parent;
    process.memory = memory;
    process
}

fn parent(pid: u32) -> ProcessField<Pid> { ProcessField::Observed(Pid::from_u32(pid)) }

fn cargo_arguments(target: &Path) -> Vec<OsString> {
    vec![
        "cargo".into(),
        "build".into(),
        "--target-dir".into(),
        target.as_os_str().into(),
    ]
}

fn compiler_arguments(name: &str, output: &Path) -> Vec<OsString> {
    vec![
        "/toolchain/rustc".into(),
        "--crate-name".into(),
        name.into(),
        "--out-dir".into(),
        output.as_os_str().into(),
    ]
}

fn memory(groups: &[CargoGroup], pid: u32) -> Result<Measurement<u64>, Box<dyn Error>> {
    groups
        .iter()
        .flat_map(|group| std::iter::once(&group.lead).chain(&group.rest))
        .find(|process| process.pid == pid)
        .map(|process| process.resource_use.memory)
        .ok_or_else(|| format!("cargo {pid} has no memory row").into())
}

/// The client's arguments and sccache's rewritten compiler arguments.
struct RewrittenCompileRequest {
    client:   Vec<OsString>,
    compiler: Vec<OsString>,
}

fn rewritten_compile_request(target: &Path) -> RewrittenCompileRequest {
    // Capture sample 3: client 735 and rustc 803 use these distinct spellings.
    let observed_client: Vec<OsString> = [
        "sccache",
        "rustc",
        "--crate-name",
        "phase39_first",
        "--edition=2024",
        "first/src/lib.rs",
        "--error-format=json",
        "--json=diagnostic-rendered-ansi,artifacts,future-incompat",
        "--crate-type",
        "lib",
        "--emit=dep-info,metadata,link",
        "-C",
        "opt-level=3",
        "-C",
        "embed-bitcode=no",
        "--check-cfg",
        "cfg(docsrs,test)",
        "--check-cfg",
        "cfg(feature, values())",
        "-C",
        "metadata=e5c3d3bb01ff2808",
        "-C",
        "extra-filename=-3700d9ae344213f3",
        "--out-dir",
    ]
    .into_iter()
    .map(OsString::from)
    .chain(std::iter::once(target.as_os_str().to_owned()))
    .chain(
        [
            "-C",
            "strip=debuginfo",
            "-L",
            "dependency=/fixture/target/release/deps",
            "-C",
            "target-cpu=x86-64-v3",
            "-C",
            "link-arg=-fuse-ld=mold",
        ]
        .into_iter()
        .map(OsString::from),
    )
    .collect();
    let observed_compiler: Vec<OsString> = [
        "/toolchain/rustc",
        "--crate-name",
        "phase39_first",
        "--edition=2024",
        "first/src/lib.rs",
        "--error-format",
        "json",
        "--json",
        "diagnostic-rendered-ansi,artifacts,future-incompat",
        "--crate-type",
        "rlib",
        "--emit",
        "dep-info,metadata,link",
        "-C",
        "opt-level=3",
        "-C",
        "embed-bitcode=no",
        "--check-cfg",
        "cfg(docsrs,test)",
        "--check-cfg",
        "cfg(feature, values())",
        "-C",
        "metadata=e5c3d3bb01ff2808",
        "-C",
        "extra-filename=-3700d9ae344213f3",
        "--out-dir",
    ]
    .into_iter()
    .map(OsString::from)
    .chain(std::iter::once(target.as_os_str().to_owned()))
    .chain(
        [
            "-C",
            "strip=debuginfo",
            "-L",
            "dependency=/fixture/target/release/deps",
            "-C",
            "target-cpu=x86-64-v3",
            "-C",
            "link-arg=-fuse-ld=mold",
        ]
        .into_iter()
        .map(OsString::from),
    )
    .collect();
    RewrittenCompileRequest {
        client:   observed_client,
        compiler: observed_compiler,
    }
}

#[test]
fn two_cargos_on_one_target_keep_their_requested_compiler_memory() -> Result<(), Box<dyn Error>> {
    let target = tempfile::tempdir()?;
    let cargo = cargo_arguments(target.path());
    let first = compiler_arguments("first", target.path());
    let second = compiler_arguments("second", target.path());
    let first_request = [vec![OsString::from("sccache")], first.clone()].concat();
    let second_request = [vec![OsString::from("sccache")], second.clone()].concat();
    let server = vec![OsString::from("sccache"), "--start-server".into()];
    let input = [
        process(100, "cargo", &cargo, ProcessField::Unavailable, 1_000),
        process(200, "cargo", &cargo, ProcessField::Unavailable, 2_000),
        process(101, "sccache", &first_request, parent(100), 100),
        process(201, "sccache", &second_request, parent(200), 200),
        process(500, "sccache", &server, ProcessField::Unavailable, 400),
        process(501, "rustc", &first, parent(500), 4_000),
        process(502, "rustc", &second, parent(500), 8_000),
    ];
    let census = Census::take(input.iter());
    let mut records = ProcessObservations::new(input);
    let sample = census.memory_sample(&records);
    let groups = CensusSequence::default().sample_counters(&mut records, Instant::now());

    assert_eq!(memory(&groups, 100)?, Measurement::Reading(5_100));
    assert_eq!(memory(&groups, 200)?, Measurement::Reading(10_200));
    assert_eq!(sample.shares.shared_compiler_cache, 400);
    assert_eq!(sample.shares.unattributed, 0);
    let RewrittenCompileRequest {
        client: observed_client,
        compiler: observed_compiler,
    } = rewritten_compile_request(target.path());
    for (name, competing, expected, unattributed) in [
        ("capture sample 3", &second_request, [5_100, 10_200], 0),
        (
            "two matching clients",
            &observed_client,
            [1_100, 2_200],
            12_000,
        ),
    ] {
        let input = [
            process(100, "cargo", &cargo, ProcessField::Unavailable, 1_000),
            process(200, "cargo", &cargo, ProcessField::Unavailable, 2_000),
            process(101, "sccache", &observed_client, parent(100), 100),
            process(201, "sccache", competing, parent(200), 200),
            process(500, "sccache", &server, ProcessField::Unavailable, 400),
            process(501, "rustc", &observed_compiler, parent(500), 4_000),
            process(502, "rustc", &second, parent(500), 8_000),
        ];
        let census = Census::take(input.iter());
        let records = ProcessObservations::new(input);
        let sample = census.memory_sample(&records);
        assert_eq!(
            sample.cargo[&Pid::from_u32(100)],
            Measurement::Reading(expected[0]),
            "{name}"
        );
        assert_eq!(
            sample.cargo[&Pid::from_u32(200)],
            Measurement::Reading(expected[1]),
            "{name}"
        );
        assert_eq!(sample.shares.shared_compiler_cache, 400, "{name}");
        assert_eq!(sample.shares.unattributed, unattributed, "{name}");
    }
    Ok(())
}

#[test]
fn sccache_server_memory_stays_outside_the_cargo_bucket() -> Result<(), Box<dyn Error>> {
    let target = tempfile::tempdir()?;
    let cargo = cargo_arguments(target.path());
    let server = vec![OsString::from("sccache"), "--start-server".into()];
    let compiler = compiler_arguments("application", target.path());
    let input = [
        process(100, "cargo", &cargo, ProcessField::Unavailable, 10),
        process(200, "sccache", &server, parent(100), 20),
        process(201, "rustc", &compiler, parent(200), 30),
    ];
    let census = Census::take(input.iter());
    let mut records = ProcessObservations::new(input);
    let sample = census.memory_sample(&records);
    let groups = CensusSequence::default().sample_counters(&mut records, Instant::now());

    assert_eq!(memory(&groups, 100)?, Measurement::Reading(40));
    assert_eq!(sample.shares.shared_compiler_cache, 20);
    assert_eq!(sample.shares.unattributed, 0);
    let mut discovered_server = process(200, "sccache", &server, ProcessField::Unavailable, 20);
    discovered_server.details = DetailPresence::Absent;
    discovered_server.argv = ProcessField::Unavailable;
    let input = [
        process(100, "cargo", &cargo, ProcessField::Unavailable, 10),
        discovered_server,
        process(201, "rustc", &compiler, parent(200), 30),
    ];
    let census = Census::take(input.iter());
    let records = ProcessObservations::new(input);
    let sample = census.memory_sample(&records);
    assert_eq!(
        sample.shares.shared_compiler_cache, 20,
        "discovery keeps the server RSS without a detail read"
    );
    assert_eq!(sample.cargo[&Pid::from_u32(100)], Measurement::Reading(40));
    Ok(())
}

#[test]
fn a_compile_without_a_target_directory_keeps_its_unattributed_memory() -> Result<(), Box<dyn Error>>
{
    let output = tempfile::tempdir()?;
    let cargo = vec![OsString::from("cargo"), "build".into()];
    let compiler = compiler_arguments("application", output.path());
    let server = vec![OsString::from("sccache"), "--start-server".into()];
    let records = [
        process(100, "cargo", &cargo, ProcessField::Unavailable, 10),
        process(200, "sccache", &server, ProcessField::Unavailable, 20),
        process(201, "rustc", &compiler, parent(200), 30),
    ];
    let census = Census::take(records.iter());
    let observations = ProcessObservations::new(records);
    let sample = census.memory_sample(&observations);
    assert_eq!(sample.shares.unattributed, 30);
    assert_eq!(sample.cargo[&Pid::from_u32(100)], Measurement::Reading(10));
    Ok(())
}

#[test]
fn a_compile_without_an_out_dir_keeps_its_unattributed_memory() -> Result<(), Box<dyn Error>> {
    let target = tempfile::tempdir()?;
    let cargo = cargo_arguments(target.path());
    let compiler = vec![
        OsString::from("rustc"),
        "--crate-name".into(),
        "application".into(),
    ];
    let server = vec![OsString::from("sccache"), "--start-server".into()];
    let records = [
        process(100, "cargo", &cargo, ProcessField::Unavailable, 10),
        process(200, "sccache", &server, ProcessField::Unavailable, 20),
        process(201, "rustc", &compiler, parent(200), 30),
    ];
    let census = Census::take(records.iter());
    let observations = ProcessObservations::new(records);
    let sample = census.memory_sample(&observations);
    assert_eq!(sample.shares.unattributed, 30);
    assert_eq!(sample.cargo[&Pid::from_u32(100)], Measurement::Reading(10));
    Ok(())
}

#[test]
fn a_compile_without_a_uid_keeps_its_unattributed_memory() -> Result<(), Box<dyn Error>> {
    let target = tempfile::tempdir()?;
    let cargo = cargo_arguments(target.path());
    let compiler = compiler_arguments("application", target.path());
    let request = [vec![OsString::from("sccache")], compiler.clone()].concat();
    let server = vec![OsString::from("sccache"), "--start-server".into()];
    let mut detached = process(201, "rustc", &compiler, parent(200), 30);
    detached.uid = ProcessField::Unavailable;
    let records = [
        process(100, "cargo", &cargo, ProcessField::Unavailable, 10),
        process(101, "sccache", &request, parent(100), 5),
        process(200, "sccache", &server, ProcessField::Unavailable, 20),
        detached,
    ];
    let census = Census::take(records.iter());
    let observations = ProcessObservations::new(records);
    let sample = census.memory_sample(&observations);
    assert_eq!(sample.shares.unattributed, 30);
    assert_eq!(sample.cargo[&Pid::from_u32(100)], Measurement::Reading(15));
    Ok(())
}

#[test]
fn a_compile_without_a_request_keeps_its_unattributed_memory() -> Result<(), Box<dyn Error>> {
    let target = tempfile::tempdir()?;
    let output = tempfile::tempdir()?;
    let cargo = cargo_arguments(target.path());
    let compiler = compiler_arguments("application", output.path());
    let server = vec![OsString::from("sccache"), "--start-server".into()];
    let records = [
        process(100, "cargo", &cargo, ProcessField::Unavailable, 10),
        process(200, "sccache", &server, ProcessField::Unavailable, 20),
        process(201, "rustc", &compiler, parent(200), 30),
    ];
    let census = Census::take(records.iter());
    let observations = ProcessObservations::new(records);
    let sample = census.memory_sample(&observations);
    assert_eq!(sample.shares.unattributed, 30);
    assert_eq!(sample.cargo[&Pid::from_u32(100)], Measurement::Reading(10));
    Ok(())
}

#[test]
fn cargo_and_compiler_shares_count_every_resident_byte_once() -> Result<(), Box<dyn Error>> {
    let target = tempfile::tempdir()?;
    let cargo = cargo_arguments(target.path());
    let compiler = compiler_arguments("application", target.path());
    let unknown = vec![
        OsString::from("rustc"),
        "--crate-name".into(),
        "unknown".into(),
    ];
    let server = vec![OsString::from("sccache"), "--start-server".into()];
    let records = [
        process(100, "cargo", &cargo, ProcessField::Unavailable, 10),
        process(200, "sccache", &server, parent(100), 20),
        process(201, "rustc", &compiler, parent(200), 30),
        process(202, "rustc", &unknown, parent(200), 50),
    ];
    let census = Census::take(records.iter());
    let observations = ProcessObservations::new(records);
    let sample = census.memory_sample(&observations);
    let cargo_bytes: u64 = sample
        .cargo
        .values()
        .filter_map(|memory| match memory {
            Measurement::Reading(bytes) => Some(*bytes),
            Measurement::Unavailable(_) => None,
        })
        .sum();
    assert_eq!(sample.shares.shared_compiler_cache, 20);
    assert_eq!(sample.shares.unattributed, 50);
    assert_eq!(cargo_bytes, 40);
    assert_eq!(
        cargo_bytes + sample.shares.shared_compiler_cache + sample.shares.unattributed,
        110
    );
    Ok(())
}

#[test]
fn emit_values_keep_each_compiler_with_its_requesting_cargo() -> Result<(), Box<dyn Error>> {
    let target = tempfile::tempdir()?;
    let cargo = cargo_arguments(target.path());
    let mut check = compiler_arguments("application", target.path());
    check.push("--emit=dep-info,metadata".into());
    let mut build = compiler_arguments("application", target.path());
    build.extend(["--emit".into(), "dep-info,metadata,link".into()]);
    let check_request = [vec![OsString::from("sccache")], check.clone()].concat();
    let build_request = [vec![OsString::from("sccache")], build.clone()].concat();
    let server = vec![OsString::from("sccache"), "--start-server".into()];
    let input = [
        process(100, "cargo", &cargo, ProcessField::Unavailable, 10),
        process(200, "cargo", &cargo, ProcessField::Unavailable, 20),
        process(101, "sccache", &check_request, parent(100), 1),
        process(201, "sccache", &build_request, parent(200), 2),
        process(500, "sccache", &server, ProcessField::Unavailable, 4),
        process(501, "rustc", &check, parent(500), 40),
        process(502, "rustc", &build, parent(500), 80),
    ];
    let census = Census::take(input.iter());
    let records = ProcessObservations::new(input);
    let sample = census.memory_sample(&records);
    assert_eq!(sample.cargo[&Pid::from_u32(100)], Measurement::Reading(51));
    assert_eq!(sample.cargo[&Pid::from_u32(200)], Measurement::Reading(102));
    assert_eq!(sample.shares.shared_compiler_cache, 4);
    assert_eq!(sample.shares.unattributed, 0);
    Ok(())
}

fn measured_sccache_client_arguments() -> Vec<OsString> {
    [
        "/run/current-system/sw/bin/sccache",
        "rustc",
        "--crate-name",
        "phase39_dep4",
        "--edition=2024",
        "dep4/src/lib.rs",
        "--error-format=json",
        "--json=diagnostic-rendered-ansi,artifacts,future-incompat",
        "--crate-type",
        "lib",
        "--emit=dep-info,metadata,link",
        "-C",
        "opt-level=3",
        "-C",
        "embed-bitcode=no",
        "-C",
        "codegen-units=1",
        "--check-cfg",
        "cfg(docsrs,test)",
        "--check-cfg",
        "cfg(feature, values())",
        "-C",
        "metadata=17bc27b3ceaa2ede",
        "-C",
        "extra-filename=-138654da3f730a81",
        "--out-dir",
        "/tmp/claude/session/da82452f-3640-4dec-b6c5-0c783d77e79f/phase39-live-check-mq_snn65/target/release/deps",
        "-C",
        "strip=debuginfo",
        "-L",
        "dependency=/tmp/claude/session/da82452f-3640-4dec-b6c5-0c783d77e79f/phase39-live-check-mq_snn65/target/release/deps",
        "-C",
        "target-cpu=x86-64-v3",
        "-C",
        "link-arg=-fuse-ld=mold",
    ].into_iter().map(OsString::from).collect()
}

fn measured_sccache_dependency_probe_arguments() -> Vec<OsString> {
    [
        "/home/natepiano/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/bin/rustc",
        "--crate-name",
        "phase39_dep4",
        "--edition=2024",
        "dep4/src/lib.rs",
        "--error-format",
        "json",
        "--json",
        "diagnostic-rendered-ansi,artifacts,future-incompat",
        "--crate-type",
        "rlib",
        "-C",
        "opt-level=3",
        "-C",
        "embed-bitcode=no",
        "-C",
        "codegen-units=1",
        "--check-cfg",
        "cfg(docsrs,test)",
        "--check-cfg",
        "cfg(feature, values())",
        "-C",
        "metadata=17bc27b3ceaa2ede",
        "-C",
        "extra-filename=-138654da3f730a81",
        "-C",
        "strip=debuginfo",
        "-L",
        "dependency=/tmp/claude/session/da82452f-3640-4dec-b6c5-0c783d77e79f/phase39-live-check-mq_snn65/target/release/deps",
        "-C",
        "target-cpu=x86-64-v3",
        "-C",
        "link-arg=-fuse-ld=mold",
        "--emit",
        "dep-info",
        "-o",
        "/tmp/claude/session/da82452f-3640-4dec-b6c5-0c783d77e79f/sccache7s48a7/deps.d",
    ].into_iter().map(OsString::from).collect()
}

/// Capture 2 includes sccache's dependency probe and its live requesting client.
#[test]
fn measured_sccache_probes_keep_their_requesting_cargos_memory() {
    let client = measured_sccache_client_arguments();
    let dependency_probe = measured_sccache_dependency_probe_arguments();
    let mut output_probe = client[1..].to_vec();
    output_probe.extend(["--print".into(), "file-names".into()]);
    let target = Path::new(
        "/tmp/claude/session/da82452f-3640-4dec-b6c5-0c783d77e79f/phase39-live-check-mq_snn65/target",
    );
    let cargo = cargo_arguments(target);
    let server = vec![OsString::from("sccache"), "--start-server".into()];
    for (name, compiler) in [
        ("dependency discovery", dependency_probe),
        ("output discovery", output_probe),
    ] {
        let input = [
            process(100, "cargo", &cargo, ProcessField::Unavailable, 10),
            process(200, "cargo", &cargo, ProcessField::Unavailable, 20),
            process(101, "sccache", &client, parent(100), 1),
            process(500, "sccache", &server, ProcessField::Unavailable, 4),
            process(501, "rustc", &compiler, parent(500), 40),
        ];
        let census = Census::take(input.iter());
        let records = ProcessObservations::new(input);
        let sample = census.memory_sample(&records);
        assert_eq!(
            sample.cargo[&Pid::from_u32(100)],
            Measurement::Reading(51),
            "{name}"
        );
        assert_eq!(
            sample.cargo[&Pid::from_u32(200)],
            Measurement::Reading(20),
            "{name}"
        );
        assert_eq!(sample.shares.shared_compiler_cache, 4, "{name}");
        assert_eq!(sample.shares.unattributed, 0, "{name}");
    }
}
