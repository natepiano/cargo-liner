use std::borrow::Cow;
use std::collections::HashMap;
use std::collections::HashSet;
use std::ffi::OsStr;
use std::ffi::OsString;
use std::fs;
use std::io::ErrorKind;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::symlink;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;
use std::time::Instant;
use std::time::UNIX_EPOCH;

use sysinfo::Pid;
#[cfg(any(not(target_os = "linux"), test))]
use sysinfo::ProcessesToUpdate;
use sysinfo::System;
use sysinfo::Users;
use tempfile::tempdir;

use super::CargoAncestry;
use super::Census;
#[cfg(test)]
use super::CensusSequence;
use super::GroupAbsence;
use super::forwards_capture_command;
use super::observed_shim_match;
#[cfg(any(target_os = "linux", target_os = "macos", test))]
use crate::birth_stamp;
use crate::birth_stamp::IdentityEvidence;
use crate::birth_stamp::KernelObservation;
use crate::birth_stamp::LifetimeEvidence;
use crate::birth_stamp::Observation;
#[cfg(any(target_os = "linux", test))]
use crate::birth_stamp::ProcessLifetime;
use crate::census::command_text;
use crate::census::command_text::CommandText;
use crate::census::command_text::RowAbsence;
use crate::census::command_text::ScannerHome;
use crate::census::direct_capture::DirectAssociation;
use crate::census::direct_capture::NearestRegistration;
use crate::census::direct_capture::SelectedProof;
use crate::census::invocation_cpu_accounting;
use crate::census::invocation_cpu_accounting::CensusCadence;
use crate::census::invocation_cpu_accounting::CompileMemoryShares;
use crate::census::invocation_cpu_accounting::CpuBaseline;
use crate::census::invocation_cpu_accounting::InvocationCpuAccounting;
use crate::census::invocation_cpu_accounting::InvocationCpuContributions;
use crate::census::invocation_cpu_accounting::Measurement;
use crate::census::invocation_cpu_accounting::MeasurementAbsence;
use crate::census::process_identity::CaptureMembership;
use crate::census::process_identity::InvocationId;
use crate::census::process_identity::ProcessIdentities;
use crate::census::process_identity::ProcessIdentity;
use crate::census::process_identity::RunId;
use crate::census::process_identity::VisibleParent;
use crate::census::scan::OriginStart;
use crate::census::scan::invocation;
use crate::census::scan::invocation::AncestryWalk;
use crate::census::scan::invocation::CargoGroup;
use crate::census::scan::invocation::CargoProcess;
use crate::census::scan::invocation::DirectoryComparison;
use crate::census::scan::invocation::ProcessOwner;
use crate::census::scan::invocation::RowProvenance;
use crate::census::scan::invocation::RunStart;
#[cfg(any(target_os = "linux", test))]
use crate::census::scan::observation::DetailPresence;
#[cfg(any(target_os = "linux", test))]
use crate::census::scan::observation::ProcessField;
use crate::census::scan::observation::ProcessObservation;
#[cfg(any(target_os = "linux", test))]
use crate::census::scan::observation::ProcessObservations;
use crate::census::scan::observation::WorkingDirectoryObservation;
#[cfg(any(not(target_os = "linux"), test))]
use crate::census::scan::refresh;
use crate::census::scan::resource_use;
#[cfg(test)]
use crate::census::scan::resource_use::Compiler;
use crate::census::scan::resource_use::CompilerObservation;
use crate::census::scan::resource_use::InvocationResourceUse;
use crate::census::scan::tests;
use crate::census::scan::tests::FixtureExecution;
use crate::census::scan::tests::ShellRun;
use crate::census::scan::tests::directory_row;
use crate::constants::CAPTURE_ASSOCIATION_AMBIGUOUS;
use crate::constants::CAPTURE_LIVE_RUNS_DIR;
use crate::constants::CARGO_DISPLAY_NAME;
use crate::constants::CARGO_TARGET_DIR_FLAG;
use crate::constants::PARENT_WALK_LIMIT;
use crate::constants::PID_SEPARATOR;
use crate::constants::ROOT_PROCESS_PID;
use crate::constants::RUN_LOG_PREFIX;
use crate::constants::RUN_LOG_SUFFIX;
use crate::constants::RUSTC_BINARY;
use crate::constants::RUSTC_OUT_DIR_FLAG;
use crate::constants::SCCACHE_BINARY;
use crate::constants::UNAVAILABLE_MEASUREMENT;
use crate::constants::UNRESOLVED_TIME;
use crate::progress::capture::Capture;
use crate::progress::capture::CaptureRootIndex;
use crate::progress::capture_diagnostic::CaptureDiagnostic;
use crate::progress::capture_read::CaptureLookup;
use crate::progress::capture_read::CaptureRead;
use crate::progress::capture_read::RunState;
use crate::progress::capture_roots::AccountName;
use crate::registration::Registration;
use crate::registration::WorkingDirectoryIdentity;
use crate::render::Account;
use crate::root_scan;
use crate::root_scan::EffectiveUser;
use crate::root_scan::RootOwner;
use crate::roster::Roster;
use crate::settings::AssociationSelection;
use crate::settings::UnusedCaptureReason;

#[test]
fn excluded_cargo_and_its_compilers_stay_out_of_memory_shares() {
    let build = ["cargo", "build"].map(OsString::from);
    let check = ["cargo", "check"].map(OsString::from);
    let compiler = ["rustc", "--crate-name", "application"].map(OsString::from);
    let request = [vec![OsString::from("sccache")], compiler.to_vec()].concat();
    let server = ["sccache", "--start-server"].map(OsString::from);
    let input = [
        (100, "cargo", build.as_slice(), 0, 10),
        (200, "cargo", check.as_slice(), 0, 20),
        (201, "sccache", request.as_slice(), 200, 2),
        (202, "rustc", compiler.as_slice(), 200, 30),
        (500, "sccache", server.as_slice(), 0, 0),
        (501, "rustc", compiler.as_slice(), 500, 40),
    ];
    let records =
        ProcessObservations::new(input.into_iter().map(|(pid, name, argv, parent, memory)| {
            let mut process = ProcessObservation::cargo(pid, argv);
            process.name = ProcessField::Observed(OsStr::new(name));
            process.parent = if parent == 0 {
                ProcessField::Unavailable
            } else {
                ProcessField::Observed(Pid::from_u32(parent))
            };
            process.memory = memory;
            process
        }));
    let mut census = Census::take(records.records.values());
    census.prepare(&records, &Capture::default(), &["check".to_owned()]);
    let sample = census.memory_sample(&records);
    assert_eq!(
        sample.cargo,
        HashMap::from([(Pid::from_u32(100), Measurement::Reading(10))])
    );
    assert_eq!(sample.shares, CompileMemoryShares::default());
}

#[test]
fn a_shell_handover_keeps_one_key_start_and_live_row() {
    let run = ShellRun::new();
    let root = tempdir().expect("capture root");
    let mut sequence = CensusSequence::default();
    let mut roster = Roster::new();
    let now = Instant::now();
    let first = sequence.sample(&run.records(FixtureExecution::OriginOnly));
    assert_eq!(first.len(), 1);
    let key = first[0].id();
    roster.observe(first, now);
    roster.observe(
        sequence.sample_capture(
            &run.records(FixtureExecution::Working),
            &ShellRun::capture(root.path()),
        ),
        now + Duration::from_secs(1),
    );
    let visible = roster.visible(&[]);
    assert_eq!(
        visible.groups().len(),
        1,
        "a hand-over cannot create a departing copy"
    );
    let row = &visible.groups()[0];
    assert_eq!(row.id, key);
    assert_eq!(row.lead.process.pid, 100);
    assert_eq!(row.lead.process.started(), RunStart::Known(10));
    assert_eq!(row.lead.process.duration(), "00:10");
    assert!(!row.lead.is_ended());
    assert_eq!(
        row.rows().count(),
        1,
        "forwarding workers are execution roles"
    );
    assert_eq!(
        row.lead.process.subtree_resource_use.memory,
        Measurement::Reading(135 * 1024 * 1024)
    );
}

#[test]
fn a_handover_keeps_the_origins_dispatched_run_count() {
    let shim = ["/bin/sh", "/tools/bin/cargo", "nextest", "run"].map(OsString::from);
    let executor = ["cargo-nextest", "nextest", "run"].map(OsString::from);
    let mut sequence = CensusSequence::default();
    let mut roster = Roster::new();
    let now = Instant::now();
    let observations = |execution| {
        ProcessObservations::new(
            [
                (100, 1, "sh", shim.as_slice()),
                (200, 100, "sh", shim.as_slice()),
                (300, 200, "cargo-nextest", executor.as_slice()),
            ]
            .into_iter()
            .take(match execution {
                FixtureExecution::Working => 3,
                FixtureExecution::OriginOnly => 1,
            })
            .map(|(pid, parent, name, argv)| {
                let mut process = ProcessObservation::cargo(pid, argv);
                process.parent = ProcessField::Observed(Pid::from_u32(parent));
                process.name = ProcessField::Observed(OsStr::new(name));
                process
            }),
        )
    };
    let first = sequence.sample(&observations(FixtureExecution::OriginOnly));
    let key = first[0].id();
    assert_eq!(first[0].lead.managed, Measurement::Reading(0));
    roster.observe(first, now);
    roster.observe(
        sequence.sample(&observations(FixtureExecution::Working)),
        now + Duration::from_secs(1),
    );
    let visible = roster.visible(&[]);
    assert_eq!(visible.groups().len(), 1);
    assert_eq!(visible.groups()[0].id, key);
    assert_eq!(visible.groups()[0].lead.process.pid, 100);
    assert_eq!(
        visible.groups()[0].lead.process.managed,
        Measurement::Reading(2)
    );
    assert_eq!(visible.groups()[0].rows().count(), 1);
}

#[test]
fn repair_continuity_an_executor_seen_first_keeps_one_origin_row() {
    let run = ShellRun::new();
    let mut records = run.records(FixtureExecution::Working);
    records.records.retain(|pid, _| pid.as_u32() == 300);
    let mut sequence = CensusSequence::default();
    let mut roster = Roster::new();
    let now = Instant::now();
    let first = sequence.sample(&records);
    assert_eq!(first.len(), 1);
    let key = first[0].id();
    roster.observe(first, now);
    roster.observe(
        sequence.sample(&run.records(FixtureExecution::Working)),
        now + Duration::from_secs(1),
    );
    let visible = roster.visible(&[]);
    assert_eq!(visible.groups().len(), 1);
    assert_eq!(visible.groups()[0].id, key);
    assert_eq!(visible.groups()[0].lead.process.pid, 100);
    assert!(!visible.groups()[0].lead.is_ended());
    assert_eq!(visible.groups()[0].rows().count(), 1);
}

#[test]
fn repair_continuity_an_executor_without_birth_keeps_one_origin_row()
-> Result<(), Box<dyn std::error::Error>> {
    let run = ShellRun::new();
    let mut records = run.records(FixtureExecution::Working);
    records.records.retain(|pid, _| pid.as_u32() == 300);
    records
        .records
        .get_mut(&Pid::from_u32(300))
        .ok_or("executor")?
        .lifetime = Cow::Owned(LifetimeEvidence::Unavailable);
    let mut sequence = CensusSequence::default();
    let mut roster = Roster::new();
    let now = Instant::now();
    let first = sequence.sample(&records);
    assert_eq!(first.len(), 1);
    let key = first[0].id();
    roster.observe(first, now);
    let mut working = run.records(FixtureExecution::Working);
    working
        .records
        .get_mut(&Pid::from_u32(300))
        .ok_or("executor")?
        .lifetime = Cow::Owned(LifetimeEvidence::Unavailable);
    roster.observe(sequence.sample(&working), now + Duration::from_secs(1));
    let visible = roster.visible(&[]);
    assert_eq!(visible.groups().len(), 1);
    assert_eq!(visible.groups()[0].id, key);
    assert_eq!(visible.groups()[0].lead.process.pid, 100);
    assert!(!visible.groups()[0].lead.is_ended());
    assert_eq!(visible.groups()[0].rows().count(), 1);
    Ok(())
}

#[test]
fn an_installed_shim_pipeline_keeps_the_driver_and_child_pids() {
    let shim = [
        OsString::from("/bin/sh"),
        "/home/u/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/bin/cargo".into(),
        "p20bdrv".into(),
    ];
    let driver = [
        OsString::from("/bin/sh"),
        "/tools/drvbin/cargo-p20bdrv".into(),
        "p20bdrv".into(),
    ];
    let nextest = [
        OsString::from("cargo-nextest"),
        "nextest".into(),
        "run".into(),
    ];
    let tee = [
        OsString::from("tee"),
        "-a".into(),
        "/capture/run.log".into(),
    ];
    let records = ProcessObservations::new(
        [
            (2_928_730, 1, "sh", shim.as_slice()),
            (2_929_160, 2_928_730, "sh", shim.as_slice()),
            (2_929_161, 2_929_160, "sh", shim.as_slice()),
            (2_929_162, 2_929_161, "cargo-p20bdrv", driver.as_slice()),
            (2_929_286, 2_929_162, "cargo-nextest", nextest.as_slice()),
            (2_929_163, 2_929_160, "tee", tee.as_slice()),
        ]
        .map(|(pid, parent, name, argv)| {
            let mut record = ProcessObservation::cargo(pid, argv);
            record.name = ProcessField::Observed(OsStr::new(name));
            record.parent = ProcessField::Observed(Pid::from_u32(parent));
            record
        }),
    );
    let groups = CensusSequence::default().sample(&records);
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].lead.pid, 2_928_730);
    assert_eq!(groups[0].rest.len(), 1);
    assert_eq!(groups[0].rest[0].pid, 2_929_286);
    assert_eq!(
        groups[0].rest[0].parent,
        VisibleParent::Invocation {
            id:  groups[0].lead.invocation_id.clone(),
            pid: 2_928_730,
        }
    );
}

#[test]
fn a_rustup_shim_dispatched_nextest_counts_only_its_managed_run() {
    let shim = [
        "/bin/sh",
        "/home/u/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/bin/cargo",
        "nextest",
        "run",
        "--locked",
    ]
    .map(OsString::from);
    let nextest = ["cargo-nextest", "nextest", "run", "--locked"].map(OsString::from);
    let test = ["cargo", "test", "--no-run"].map(OsString::from);
    let records = ProcessObservations::new(
        [
            (3_214_500, 1, "cargo", shim.as_slice()),
            (3_215_101, 3_214_500, "cargo", shim.as_slice()),
            (3_215_102, 3_215_101, "cargo", shim.as_slice()),
            (3_215_108, 3_215_102, "cargo-nextest", nextest.as_slice()),
            (3_262_399, 3_215_108, "cargo", test.as_slice()),
        ]
        .map(|(pid, parent, name, argv)| {
            let mut record = ProcessObservation::cargo(pid, argv);
            record.name = ProcessField::Observed(OsStr::new(name));
            record.parent = ProcessField::Observed(Pid::from_u32(parent));
            record
        }),
    );
    let groups = CensusSequence::default().sample(&records);
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].lead.pid, 3_214_500);
    assert_eq!(groups[0].rest.len(), 1);
    assert_eq!(groups[0].rest[0].pid, 3_262_399);
    assert_eq!(groups[0].lead.managed, Measurement::Reading(1));
}

#[test]
fn a_dispatched_nextest_keeps_its_hidden_ci_drivers_runs() {
    let shim = [
        OsString::from("/bin/sh"),
        "/tools/bin/cargo".into(),
        "nextest".into(),
        "run".into(),
        "--profile".into(),
        "ci".into(),
    ];
    let nextest = [
        OsString::from("cargo-nextest"),
        "nextest".into(),
        "run".into(),
        "--profile".into(),
        "ci".into(),
    ];
    let records = ProcessObservations::new(
        [
            (172_043, 170_718, "sh", shim.as_slice()),
            (172_044, 172_043, "sh", shim.as_slice()),
            (172_047, 172_044, "cargo-nextest", nextest.as_slice()),
        ]
        .map(|(pid, parent, name, argv)| {
            let mut record = ProcessObservation::cargo(pid, argv);
            record.name = ProcessField::Observed(OsStr::new(name));
            record.parent = ProcessField::Observed(Pid::from_u32(parent));
            record
        }),
    );
    let discovered = Census::take(records.processes().values());
    assert_eq!(
        discovered.cargo.len(),
        3,
        "both interpreted drivers must enter discovery"
    );
    let children = discovered.cargo_children();
    assert_eq!(
        Census::descendants(&children, Pid::from_u32(172_043)).len(),
        2
    );
    assert_eq!(
        Census::descendants(&children, Pid::from_u32(172_044)).len(),
        1
    );
    let groups = CensusSequence::default().sample(&records);
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].lead.pid, 172_043);
    assert_eq!(groups[0].rest, [] as [CargoProcess; 0]);
    assert_eq!(groups[0].lead.managed, Measurement::Reading(2));
}

#[test]
fn a_driver_row_carries_the_dispatched_process_pid() {
    let shim = [OsString::from("cargo"), "p20driver".into()];
    let driver = [OsString::from("/tools/cargo-p20driver"), "p20driver".into()];
    let child = [
        OsString::from("cargo-nextest"),
        "nextest".into(),
        "run".into(),
    ];
    let records = ProcessObservations::new(
        [
            (100, 1, "cargo", shim.as_slice()),
            (200, 100, "cargo-p20driver", driver.as_slice()),
            (300, 200, "cargo-nextest", child.as_slice()),
        ]
        .map(|(pid, parent, name, argv)| {
            let mut record = ProcessObservation::cargo(pid, argv);
            record.name = ProcessField::Observed(OsStr::new(name));
            record.parent = ProcessField::Observed(Pid::from_u32(parent));
            if pid == 200 {
                record.directory = ProcessField::Unavailable;
            }
            record
        }),
    );
    let groups = CensusSequence::default().sample(&records);
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].lead.pid, 200);
    assert_eq!(groups[0].rest.len(), 1);
    assert_eq!(groups[0].rest[0].pid, 300);
    assert_eq!(
        groups[0].rest[0].parent,
        VisibleParent::Invocation {
            id:  groups[0].lead.invocation_id.clone(),
            pid: 200,
        }
    );
}

#[test]
fn an_interpreted_driver_carries_its_dispatched_pid_and_capture_identity() {
    let root = tempdir().expect("capture root");
    tests::write_versioned_capture(
        root.path(),
        100,
        "driver-generation",
        "/work",
        "/writer",
        "p20driver",
        "",
    );
    let capture = tests::verified_capture(root.path());
    let shim = [OsString::from("cargo"), "p20driver".into()];
    let driver = [
        OsString::from("python3"),
        "/tools/cargo-p20driver".into(),
        "p20driver".into(),
    ];
    let child = [
        OsString::from("cargo-nextest"),
        "nextest".into(),
        "run".into(),
    ];
    let records = ProcessObservations::new(
        [
            (100, 1, "cargo", shim.as_slice()),
            (200, 100, "python3", driver.as_slice()),
            (300, 200, "cargo-nextest", child.as_slice()),
        ]
        .map(|(pid, parent, name, argv)| {
            let mut record = ProcessObservation::cargo(pid, argv);
            record.name = ProcessField::Observed(OsStr::new(name));
            record.parent = ProcessField::Observed(Pid::from_u32(parent));
            record
        }),
    );
    let groups = CensusSequence::default().sample_capture(&records, &capture);
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].lead.pid, 100);
    assert!(
        matches!(&groups[0].lead.invocation_id, InvocationId::Captured(run) if run.shim_pid == 100)
    );
    assert_eq!(
        groups[0].lead.command,
        CommandText::of("cargo", &["p20driver"])
    );
    assert_eq!(groups[0].rest.len(), 1);
    assert_eq!(groups[0].rest[0].pid, 300);
    assert_eq!(
        groups[0].rest[0].parent,
        VisibleParent::Invocation {
            id:  groups[0].lead.invocation_id.clone(),
            pid: 100,
        }
    );
    assert!(matches!(
        groups[0].rest[0].capture_membership,
        CaptureMembership::Enclosing(_)
    ));
}

#[test]
fn a_driver_that_changes_directory_keeps_its_dispatched_pid() {
    let root = tempdir().expect("directories");
    let parent_directory = root.path().join("parent");
    let child_directory = root.path().join("child");
    fs::create_dir(&parent_directory).expect("parent directory");
    fs::create_dir(&child_directory).expect("child directory");
    let shim = [OsString::from("cargo"), "p20driver".into()];
    let driver = [OsString::from("/tools/cargo-p20driver"), "p20driver".into()];
    let nextest = [
        OsString::from("cargo-nextest"),
        "nextest".into(),
        "run".into(),
    ];
    let mut parent = ProcessObservation::cargo(100, &shim);
    parent.directory = ProcessField::Observed(&parent_directory);
    let mut child = ProcessObservation::cargo(200, &driver);
    child.name = ProcessField::Observed(OsStr::new("cargo-p20driver"));
    child.directory = ProcessField::Observed(&child_directory);
    child.parent = ProcessField::Observed(Pid::from_u32(100));
    let mut nested = ProcessObservation::cargo(300, &nextest);
    nested.name = ProcessField::Observed(OsStr::new("cargo-nextest"));
    nested.parent = ProcessField::Observed(Pid::from_u32(200));
    let groups =
        CensusSequence::default().sample(&ProcessObservations::new([parent, child, nested]));
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].lead.pid, 200);
    assert_eq!(groups[0].rest.len(), 1);
    assert_eq!(groups[0].rest[0].pid, 300);
    assert_eq!(
        groups[0].rest[0].parent,
        VisibleParent::Invocation {
            id:  groups[0].lead.invocation_id.clone(),
            pid: 200,
        }
    );
}

#[test]
fn an_interpreter_with_flags_keeps_the_driver_and_child_pids() {
    let shim = [OsString::from("cargo"), "p20driver".into()];
    let driver = [
        OsString::from("python3"),
        "-u".into(),
        "/tools/cargo-p20driver".into(),
        "p20driver".into(),
    ];
    let nextest = [
        OsString::from("cargo-nextest"),
        "nextest".into(),
        "run".into(),
    ];
    let records = ProcessObservations::new(
        [
            (100, 1, "cargo", shim.as_slice()),
            (200, 100, "python3", driver.as_slice()),
            (300, 200, "cargo-nextest", nextest.as_slice()),
        ]
        .map(|(pid, parent, name, argv)| {
            let mut record = ProcessObservation::cargo(pid, argv);
            record.name = ProcessField::Observed(OsStr::new(name));
            record.parent = ProcessField::Observed(Pid::from_u32(parent));
            record
        }),
    );
    let groups = CensusSequence::default().sample(&records);
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].lead.pid, 200);
    assert_eq!(groups[0].rest.len(), 1);
    assert_eq!(groups[0].rest[0].pid, 300);
    assert_eq!(
        groups[0].rest[0].parent,
        VisibleParent::Invocation {
            id:  groups[0].lead.invocation_id.clone(),
            pid: 200,
        }
    );
}

#[test]
fn different_external_arguments_keep_both_invocations() {
    let shim = [
        OsString::from("cargo"),
        "p20driver".into(),
        "--jobs".into(),
        "2".into(),
    ];
    let driver = [
        OsString::from("/tools/cargo-p20driver"),
        "p20driver".into(),
        "--jobs".into(),
        "4".into(),
    ];
    let parent = ProcessObservation::cargo(100, &shim);
    let mut child = ProcessObservation::cargo(200, &driver);
    child.name = ProcessField::Observed(OsStr::new("cargo-p20driver"));
    child.parent = ProcessField::Observed(Pid::from_u32(100));
    let groups = CensusSequence::default().sample(&ProcessObservations::new([parent, child]));
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].lead.pid, 100);
    assert_eq!(groups[0].rest.len(), 1);
    assert_eq!(groups[0].rest[0].pid, 200);
}

#[test]
fn each_invocation_keeps_its_own_resource_use_including_the_lead() {
    let parent = [OsString::from("cargo"), "build".into()];
    let child = [OsString::from("cargo"), "check".into()];
    let compiler = [OsString::from("sccache"), "rustc".into()];
    let records = ProcessObservations::new(
        [
            (100, 1, "cargo", parent.as_slice(), 10.25, 100),
            (200, 100, "cargo", child.as_slice(), 89.75, 200),
            (300, 200, "sccache", compiler.as_slice(), 0.0, 300),
        ]
        .map(|(pid, parent, name, argv, cpu, memory)| {
            let mut record = ProcessObservation::cargo(pid, argv);
            record.name = ProcessField::Observed(OsStr::new(name));
            record.parent = ProcessField::Observed(Pid::from_u32(parent));
            record.cpu = Measurement::Reading(cpu);
            record.memory = memory;
            record
        }),
    );
    let groups = CensusSequence::default().sample(&records);
    assert_eq!(groups.len(), 1);
    let lead = &groups[0].lead;
    assert_eq!(lead.resource_use.cpu, Measurement::Reading(10.25));
    assert_eq!(lead.resource_use.memory, Measurement::Reading(100));
    assert_eq!(lead.resource_use.compiler, CompilerObservation::None);
    let child = &groups[0].rest[0];
    assert_eq!(child.resource_use.cpu, Measurement::Reading(89.75));
    assert_eq!(child.resource_use.memory, Measurement::Reading(500));
    assert_eq!(
        child.resource_use.compiler,
        CompilerObservation::Running(Compiler {
            name:  SCCACHE_BINARY,
            count: 1,
        })
    );
    assert_eq!(lead.subtree_resource_use.cpu, Measurement::Reading(100.0));
    assert_eq!(lead.subtree_resource_use.memory, Measurement::Reading(600));
    assert_eq!(
        lead.subtree_resource_use.compiler,
        child.resource_use.compiler
    );
}

#[test]
fn an_unavailable_contributor_keeps_the_total_unavailable_and_the_lead_known() {
    let parent = [OsString::from("cargo"), "build".into()];
    let child = [OsString::from("cargo"), "check".into()];
    let records = ProcessObservations::new(
        [
            (100, 1, parent.as_slice(), Measurement::Reading(12.5)),
            (
                200,
                100,
                child.as_slice(),
                Measurement::Unavailable(MeasurementAbsence::ReadFailed),
            ),
        ]
        .map(|(pid, parent, argv, cpu)| {
            let mut record = ProcessObservation::cargo(pid, argv);
            record.parent = ProcessField::Observed(Pid::from_u32(parent));
            record.cpu = cpu;
            record
        }),
    );
    let groups = CensusSequence::default().sample(&records);
    assert_eq!(groups.len(), 1);
    assert_eq!(
        groups[0].lead.subtree_resource_use.cpu,
        Measurement::Unavailable(MeasurementAbsence::ReadFailed)
    );
    assert_eq!(groups[0].lead.resource_use.cpu, Measurement::Reading(12.5));
}

#[test]
fn confirmed_registration_sources_directory_command_time_and_unknown_measurements() {
    let root = tempdir().expect("root");
    tests::write_versioned_capture(
        root.path(),
        10,
        "generation",
        "/writer/project",
        "/writer",
        "build",
        "    Blocking waiting for file lock on build directory",
    );
    let capture = tests::verified_capture(root.path());
    let mut census = tests::census_of(&[]);
    let groups =
        CensusSequence::assemble(&mut census, &ProcessObservations::default(), &capture, &[]);
    assert_eq!(groups.len(), 1);
    let row = &groups[0].lead;
    assert_eq!(row.pid, 10);
    assert_eq!(row.path, "~/project");
    assert_eq!(row.command, CommandText::of("cargo", &["build"]));
    assert_eq!(
        row.directory_identity,
        WorkingDirectoryIdentity::Absolute("/writer/project".into())
    );
    assert!(matches!(row.started(), RunStart::Known(_)));
    assert!(matches!(row.provenance, RowProvenance::Direct(_)));
    assert_eq!(
        row.resource_use.cpu,
        Measurement::Unavailable(MeasurementAbsence::Unproven)
    );
    assert_eq!(
        row.managed,
        Measurement::Unavailable(MeasurementAbsence::Unproven)
    );
    assert_eq!(row.resource_use.compiler, CompilerObservation::Unknown);
    assert_eq!(
        row.state,
        CaptureLookup::Registered(CaptureRead::Progress(RunState::Blocked))
    );
}

/// Missing account records change the label without discarding verified ownership.
#[test]
fn unresolved_account_lookup_preserves_fallback_row_owner() {
    let root = tempdir().expect("root");
    tests::write_versioned_capture(
        root.path(),
        10,
        "generation",
        "/writer/project",
        "/writer",
        "build",
        "",
    );
    let mut capture = tests::verified_capture(root.path());
    let users = Users::new();
    let status = capture.root_status.first_mut().expect("observed root");
    let RootOwner::Uid(uid) = status.owner else {
        panic!("verified root has an observed owner");
    };
    status.account = AccountName::resolve(status.owner, &users);
    assert_eq!(status.account, AccountName::Unavailable);
    assert_eq!(
        AccountName::resolve(RootOwner::Unavailable, &users),
        AccountName::Unavailable
    );
    let mut census = tests::census_of(&[]);
    let groups =
        CensusSequence::assemble(&mut census, &ProcessObservations::default(), &capture, &[]);
    assert_eq!(groups.len(), 1);
    let RowProvenance::Direct(context) = &groups[0].lead.provenance else {
        panic!("missing account name does not remove verified provenance");
    };
    assert_eq!(context.account.uid, uid);
    assert_eq!(context.account.name, AccountName::Unavailable);
}

/// The worker names an uncaptured row's owner from the table that named its roots.
#[test]
fn an_uncaptured_row_names_its_process_owner_on_the_worker() {
    let root = tempdir().expect("root");
    let capture = tests::verified_capture(root.path());
    let EffectiveUser::Known(uid) = root_scan::effective_user() else {
        panic!("fixture owner");
    };
    assert_eq!(capture.root_status[0].root.uid, uid);
    let fixture = tests::cargo_process("build");
    for (observed, expected) in [
        (
            ProcessField::Observed(uid),
            ProcessOwner::Observed(Account {
                uid,
                name: capture.root_status[0].account.clone(),
            }),
        ),
        (ProcessField::Unavailable, ProcessOwner::Unavailable),
    ] {
        let mut record = fixture.observation();
        record.uid = observed;
        let records = ProcessObservations::new([record]);
        let groups = CensusSequence::default().sample_capture(&records, &capture);
        assert_eq!(groups.len(), 1);
        assert_eq!(
            groups[0].lead.provenance,
            RowProvenance::Uncaptured(expected)
        );
    }
    assert_eq!(
        capture.account_name(uid),
        AccountName::resolve(RootOwner::Uid(uid), &Users::new_with_refreshed_list())
    );
}

#[test]
fn registration_timestamp_failure_preserves_row_and_orders_it_after_known_starts() {
    let root = tempdir().expect("root");
    tests::write_versioned_capture(
        root.path(),
        10,
        "generation",
        "/writer/project",
        "/writer",
        "build",
        "",
    );
    let capture = tests::verified_capture(root.path());
    let DirectAssociation::Direct(mut direct) = capture.row_source(10) else {
        panic!("verified source");
    };
    direct.modified = Err(std::io::Error::from(ErrorKind::PermissionDenied).into());
    let unknown = invocation::registration_row(&direct, &capture, ScannerHome::Unavailable)
        .expect("row survives missing mtime");
    assert_eq!(unknown.started(), RunStart::Unavailable);
    assert_eq!(unknown.start(), UNRESOLVED_TIME);
    assert_eq!(unknown.duration(), UNRESOLVED_TIME);
    let mut known = directory_row();
    known.origin_start = OriginStart::Process {
        epoch:   1,
        elapsed: Duration::from_secs(0),
    };
    let mut ordered = vec![unknown.clone(), known.clone()];
    ordered.sort_by(invocation::newest_first);
    assert_eq!(ordered, [known, unknown]);
    assert!(RunStart::Known(u64::MAX) < RunStart::Unavailable);
}

#[test]
fn direct_registration_fills_cwd_and_argv_independently_before_formatting() {
    let root = tempdir().expect("root");
    tests::write_versioned_capture(
        root.path(),
        10,
        "generation",
        "/writer/project",
        "/writer",
        "build",
        "",
    );
    let capture = tests::verified_capture(root.path());
    let direct = capture.row_source(10);
    let observed = [OsString::from("cargo"), OsString::from("test")];
    let (identity, path, command) = invocation::row_fields(
        WorkingDirectoryObservation::Unavailable,
        &observed,
        &direct,
        ScannerHome::Known(Path::new("/writer")),
    )
    .expect("merge directory");
    assert_eq!(
        identity,
        WorkingDirectoryIdentity::Absolute("/writer/project".into())
    );
    assert_eq!(path, "~/project");
    assert_eq!(command, CommandText::of("cargo", &["test"]));
    let (identity, path, command) = invocation::row_fields(
        WorkingDirectoryObservation::Observed(Path::new("/own")),
        &[],
        &direct,
        ScannerHome::Unavailable,
    )
    .expect("merge argv");
    assert_eq!(identity, WorkingDirectoryIdentity::Absolute("/own".into()));
    assert_eq!(path, "/own");
    assert_eq!(command, CommandText::of("cargo", &["build"]));
    assert_eq!(
        invocation::row_fields(
            WorkingDirectoryObservation::Unavailable,
            &[OsString::from("application")],
            &direct,
            ScannerHome::Unavailable
        ),
        Err(RowAbsence::ProgramRejected)
    );
}

#[test]
fn process_with_unavailable_cwd_keeps_pid_and_measurements_after_merge() {
    let fixture = tests::cargo_process("build");
    let pid = Pid::from_u32(fixture.pid);
    let root = tempdir().expect("root");
    tests::write_versioned_capture(
        root.path(),
        10,
        "generation",
        "/writer/project",
        "/writer",
        "build",
        "",
    );
    let capture = tests::verified_capture(root.path());
    let mut record = fixture.observation();
    record.directory = ProcessField::Unavailable;
    let system = ProcessObservations::new([record]);
    let process = system.process(pid).expect("live process");
    assert_eq!(process.cwd(), WorkingDirectoryObservation::Unavailable);
    let row = invocation::row(
        process,
        process.cmd(),
        InvocationId::for_test(pid.as_u32()),
        Measurement::Reading(3),
        ScannerHome::Known(Path::new("/writer")),
        InvocationResourceUse {
            cpu:      Measurement::Reading(42.0),
            memory:   Measurement::Reading(1_024),
            compiler: CompilerObservation::None,
        },
        &capture.row_source(10),
    )
    .expect("merged process row");
    assert_eq!(row.pid, pid.as_u32());
    assert_eq!(row.path, "~/project");
    assert_eq!(row.resource_use.cpu, Measurement::Reading(42.0));
    assert_eq!(row.resource_use.memory, Measurement::Reading(1_024));
    assert_eq!(row.managed, Measurement::Reading(3));
    assert_eq!(row.resource_use.compiler, CompilerObservation::None);
}

#[test]
fn registration_and_process_scans_keep_one_invocation_and_exclusions_apply_to_both() {
    let mut fixture = tests::cargo_process("build");
    fixture.directory = PathBuf::from("/writer/project");
    let pid = Pid::from_u32(fixture.pid);
    let mut record = fixture.observation();
    record.parent = ProcessField::Observed(Pid::from_u32(10));
    record.cpu = Measurement::Reading(42.0);
    let system = ProcessObservations::new([record]);
    let process = system.process(pid).unwrap_or_else(|| {
        panic!("ready fixture pid {pid} is absent from the detailed process snapshot");
    });
    assert_eq!(
        command_text::subcommand(process.cmd()).as_deref(),
        Ok("build"),
        "fixture pid {pid}: argv={:?}, cwd={:?}",
        process.cmd(),
        process.cwd(),
    );
    let directory = PathBuf::from("/writer/project");
    assert_eq!(
        process.cwd(),
        WorkingDirectoryObservation::Observed(directory.as_path()),
        "fixture pid {pid}: argv={:?}, cwd={:?}",
        process.cmd(),
        process.cwd(),
    );
    let root = tempdir().expect("root");
    tests::write_versioned_capture(
        root.path(),
        10,
        "generation",
        "/writer/project",
        "/writer",
        "build",
        "    Blocking waiting for file lock on build directory",
    );
    let modified = UNIX_EPOCH + Duration::from_secs(1234);
    fs::File::open(
        root.path()
            .join(CAPTURE_LIVE_RUNS_DIR)
            .join("10.generation"),
    )
    .expect("registration descriptor")
    .set_times(fs::FileTimes::new().set_modified(modified))
    .expect("distinct invocation start");
    let capture = tests::verified_capture(root.path());
    let mut sequence = CensusSequence::default();
    let home = ScannerHome::Known(Path::new("/writer"));
    let first = sequence.sample_with_capture(&ProcessObservations::default(), &capture, &[], home);
    let second = sequence.sample_with_capture(&system, &capture, &[], home);
    let third = sequence.sample_with_capture(&ProcessObservations::default(), &capture, &[], home);
    tests::assert_single_source_transitions(&[first.clone(), second.clone(), third], &[10, 10, 10]);
    assert_eq!(first.len(), 1);
    assert_eq!(second.len(), 1);
    assert_eq!(first[0].id(), second[0].id());
    assert_eq!(first[0].lead.started(), RunStart::Known(1234));
    assert_eq!(second[0].lead.started(), first[0].lead.started());
    assert_eq!(second[0].lead.start(), first[0].lead.start());
    assert_eq!(first[0].lead.pid, 10);
    assert_eq!(second[0].lead.pid, 10);
    let mut roster = crate::roster::Roster::new();
    let now = Instant::now();
    roster.observe(first, now);
    let ids = roster.tiled_ids(&[]);
    roster.observe(second, now + invocation_cpu_accounting::poll());
    assert_eq!(roster.tiled_ids(&[]), ids);
    assert_eq!(roster.groups().len(), 1);
    for mut census in [
        tests::census_of(&[]),
        tests::census_of(&[(pid.as_u32(), 10)]),
    ] {
        census.cargo.push(pid);
        assert_eq!(
            CensusSequence::assemble(&mut census, &system, &capture, &["build".into()]),
            [] as [CargoGroup; 0]
        );
    }
}

#[test]
fn unknown_legacy_and_ambiguous_publications_never_source_rows_and_ambiguity_recovers() {
    let root = tempdir().expect("root");
    tests::write_versioned_capture(
        root.path(),
        10,
        "first",
        "/writer/project",
        "/writer",
        "build",
        "    Blocking waiting for file lock on build directory",
    );
    let unknown = Capture::take_from(root.path(), |pid| {
        KernelObservation::for_test(pid, Observation::Unknown)
    });
    tests::assert_no_registration_rows(&unknown);
    assert!(matches!(unknown.row_source(10), DirectAssociation::None));
    tests::write_versioned_capture(root.path(), 10, "second", "/other", "/writer", "test", "");
    let mut competing = tests::verified_capture(root.path());
    let census = tests::census_of(&[]);
    tests::assert_no_registration_rows(&competing);
    census.associate_status(&mut competing, &[]);
    let associations = &competing.root_status[0].associations;
    assert_eq!(associations.len(), 1);
    assert!(
        matches!(&associations[0].selection, AssociationSelection::Ambiguous { candidates } if candidates.len() == 2)
    );
    let mut app = crate::app::App::new_for_test().expect("quiet settings app");
    app.root_status.clone_from(&competing.root_status);
    let ambiguous = tests::account_settings_text(&app);
    assert!(
        ambiguous.contains(CAPTURE_ASSOCIATION_AMBIGUOUS),
        "{ambiguous}"
    );
    assert!(ambiguous.contains("10.first"), "{ambiguous}");
    assert!(ambiguous.contains("10.second"), "{ambiguous}");
    let mut row = directory_row();
    census.annotate_capture(&mut row, &competing, ScannerHome::Unavailable);
    assert_eq!(
        row.provenance,
        RowProvenance::Uncaptured(ProcessOwner::Unavailable)
    );
    assert_eq!(row.state, CaptureLookup::Unregistered);
    fs::remove_file(root.path().join(CAPTURE_LIVE_RUNS_DIR).join("10.second"))
        .expect("remove competing publication");
    let mut recovered = tests::verified_capture(root.path());
    let mut census = tests::census_of(&[]);
    let groups = CensusSequence::assemble(
        &mut census,
        &ProcessObservations::default(),
        &recovered,
        &[],
    );
    assert_eq!(groups.len(), 1);
    assert_eq!(
        groups[0].lead.state,
        CaptureLookup::Registered(CaptureRead::Progress(RunState::Blocked))
    );
    let rendered = tests::assert_registration_display(&groups, &recovered);
    assert_eq!(rendered.matches("blocked").count(), 1, "{rendered}");
    census.associate_status(&mut recovered, &groups);
    assert!(
        matches!(&recovered.root_status[0].associations[0].selection,
        AssociationSelection::Selected { proof: SelectedProof::Confirmed, unused, .. } if unused.is_empty())
    );
    app.root_status.clone_from(&recovered.root_status);
    let recovered_settings = tests::account_settings_text(&app);
    assert!(
        !recovered_settings.contains('\n'),
        "a confirmed selection standing alone adds no line: {recovered_settings}"
    );
    assert_ne!(recovered_settings, ambiguous);
    let legacy_root = tests::capture_root(&[(10, "")]);
    let legacy = tests::verified_capture(legacy_root.path());
    tests::assert_no_registration_rows(&legacy);
}

#[test]
fn foreign_owned_duplicate_cannot_add_or_relabel_a_process_row() {
    let argv = [OsString::from("cargo"), "build".into()];
    let mut process = ProcessObservation::cargo(11, &argv);
    process.parent = ProcessField::Observed(Pid::from_u32(10));
    process.directory = ProcessField::Observed(Path::new("/writer/project"));
    tests::assert_foreign_owned_duplicate(&ProcessObservations::new([process]), "100", &[10]);
}

#[test]
fn foreign_owned_duplicate_cannot_add_or_relabel_a_registration_row() {
    tests::assert_foreign_owned_duplicate(&ProcessObservations::default(), "100", &[10]);
}

#[test]
fn foreign_owned_duplicate_cannot_confirm_an_unknown_selected_registration() {
    tests::assert_foreign_owned_duplicate(&ProcessObservations::default(), "", &[]);
}

#[test]
fn two_confirmed_roots_select_one_fallback_and_report_the_unused_proof() {
    let first = tempdir().expect("first root");
    let second = tempdir().expect("second root");
    tests::write_versioned_capture(
        first.path(),
        10,
        "first",
        "/writer/project",
        "/writer",
        "build",
        "",
    );
    tests::write_versioned_capture(second.path(), 10, "second", "/other", "/writer", "test", "");
    let roots = tests::resolved_test_roots(&[first.path(), second.path()]);
    let stamp = tests::directory_record("/writer").identity().clone();
    let IdentityEvidence::Available(stamp) = stamp else {
        panic!("birth");
    };
    let mut capture = Capture::take_roots(&roots, &|pid| {
        KernelObservation::for_test(pid, Observation::Present(stamp.clone()))
    });
    let mut census = tests::census_of(&[]);
    let groups =
        CensusSequence::assemble(&mut census, &ProcessObservations::default(), &capture, &[]);
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].lead.path, "~/project");
    assert_eq!(groups[0].lead.command, CommandText::of("cargo", &["build"]));
    census.associate_status(&mut capture, &groups);
    assert!(matches!(&capture.root_status[0].associations[0].selection,
        AssociationSelection::Selected { key, proof: SelectedProof::Confirmed, unused }
        if key.root == CaptureRootIndex(0) && unused.len() == 1 && unused[0].key.root == CaptureRootIndex(1) && unused[0].reason == UnusedCaptureReason::RootPrecedence));
}

#[test]
fn selected_unconfirmed_root_never_borrows_another_roots_metadata_or_adds_a_row() {
    let first = tests::capture_root(&[(10, "")]);
    let second = tempdir().expect("second root");
    tests::write_versioned_capture(second.path(), 10, "second", "/other", "/writer", "test", "");
    let roots = tests::resolved_test_roots(&[first.path(), second.path()]);
    let IdentityEvidence::Available(stamp) = tests::directory_record("/writer").identity().clone()
    else {
        panic!("birth");
    };
    let mut capture = Capture::take_roots(&roots, &|pid| {
        KernelObservation::for_test(pid, Observation::Present(stamp.clone()))
    });
    let mut census = tests::census_of(&[]);
    census.select_rows(&ProcessObservations::default(), &capture, &[]);
    let mut row = directory_row();
    census.annotate_capture(&mut row, &capture, ScannerHome::Unavailable);
    let mut rows = vec![row.clone()];
    census.add_registration_rows(&mut rows, &capture, ScannerHome::Unavailable);
    assert_eq!(rows, [row]);
    assert_eq!(
        rows[0].provenance,
        RowProvenance::Uncaptured(ProcessOwner::Unavailable)
    );
    assert_eq!(rows[0].command, CommandText::of("cargo", &["build"]));
    let groups = census.assemble_groups(
        &ProcessObservations::default(),
        ScannerHome::Unavailable,
        rows,
    );
    census.associate_status(&mut capture, &groups);
    assert!(matches!(&capture.root_status[0].associations[0].selection,
        AssociationSelection::Selected { proof: SelectedProof::Unconfirmed, unused, .. }
        if unused.len() == 1 && unused[0].reason == UnusedCaptureReason::SelectedUnconfirmed));
}

#[test]
fn unreadable_log_does_not_remove_verified_registration_row_or_change_active_count() {
    let root = tempdir().expect("root");
    tests::write_versioned_capture(
        root.path(),
        10,
        "generation",
        "/writer/project",
        "/writer",
        "build",
        "",
    );
    let log = root.path().join("run-generation-10.log");
    fs::remove_file(&log).expect("remove log");
    fs::create_dir(&log).expect("unreadable log");
    let capture = tests::verified_capture(root.path());
    assert_eq!(capture.root_status[0].confirmed, 0);
    assert!(
        capture.root_status[0]
            .diagnostics
            .iter()
            .any(|diagnostic| matches!(diagnostic, CaptureDiagnostic::LogUnreadable(_)))
    );
    let groups = CensusSequence::assemble(
        &mut tests::census_of(&[]),
        &ProcessObservations::default(),
        &capture,
        &[],
    );
    assert_eq!(groups.len(), 1);
    assert!(matches!(
        groups[0].lead.state,
        CaptureLookup::Registered(CaptureRead::Unreadable(_))
    ));
    tests::assert_registration_display(&groups, &capture);
    assert_eq!(capture.root_status[0].confirmed, 0);
}

#[test]
fn enclosing_descendant_does_not_suppress_fallback_and_both_sources_assemble_one_tree() {
    let child = tests::cargo_process("test");
    let child_pid = Pid::from_u32(child.pid);
    let parent = tests::cargo_process("build");
    let parent_pid = Pid::from_u32(parent.pid);
    let root = tempdir().expect("root");
    tests::write_versioned_capture(
        root.path(),
        10,
        "generation",
        "/writer/project",
        "/writer",
        "build",
        "",
    );
    let capture = tests::verified_capture(root.path());
    let child_only = ProcessObservations::new([child.observation()]);
    let both = ProcessObservations::new([parent.observation(), child.observation()]);
    let mut census = tests::census_of(&[
        (parent_pid.as_u32(), 10),
        (child_pid.as_u32(), parent_pid.as_u32()),
    ]);
    census.cargo = vec![child_pid];
    let first = CensusSequence::assemble(&mut census, &child_only, &capture, &[]);
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].rest.len(), 1);
    assert!(matches!(
        first[0].rest[0].capture_membership,
        CaptureMembership::Enclosing(_)
    ));
    assert!(matches!(
        first[0].rest[0].provenance,
        RowProvenance::Enclosing(_)
    ));
    assert_eq!(
        first[0].rest[0].command,
        CommandText::of("cargo", &["test"])
    );
    assert_eq!(
        first[0].rest[0].parent,
        VisibleParent::Invocation {
            id:  first[0].id(),
            pid: 10,
        }
    );
    let mut present = tests::census_of(&[
        (parent_pid.as_u32(), 10),
        (child_pid.as_u32(), parent_pid.as_u32()),
    ]);
    present.cargo = vec![parent_pid, child_pid];
    let second = CensusSequence::assemble(&mut present, &both, &capture, &[]);
    assert_eq!(second.len(), 1);
    assert_eq!(second[0].rest.len(), 1);
    assert_eq!(second[0].id(), first[0].id());
    assert_eq!(second[0].lead.pid, 10);
    assert_eq!(
        second[0].rest[0].parent,
        VisibleParent::Invocation {
            id:  first[0].id(),
            pid: 10,
        }
    );
    let mut missing_parent = tests::census_of(&[
        (parent_pid.as_u32(), 10),
        (child_pid.as_u32(), parent_pid.as_u32()),
    ]);
    missing_parent.cargo = vec![parent_pid, child_pid];
    let partial = CensusSequence::assemble(&mut missing_parent, &child_only, &capture, &[]);
    assert_eq!(partial.len(), 1);
    assert_eq!(partial[0].id(), first[0].id());
    assert_eq!(partial[0].rest.len(), 1);
    assert_eq!(
        partial[0].rest[0].invocation_id,
        first[0].rest[0].invocation_id
    );
    tests::assert_family_source_transitions(&[first.clone(), second, partial, first]);
}

#[test]
fn enclosing_membership_never_suppresses_fallback_even_at_the_registered_pid() {
    let root = tempdir().expect("root");
    tests::write_versioned_capture(
        root.path(),
        10,
        "generation",
        "/writer/project",
        "/writer",
        "build",
        "",
    );
    let capture = tests::verified_capture(root.path());
    let mut census = tests::census_of(&[(10, 1)]);
    census.capture_mismatches.insert(Pid::from_u32(10));
    census.select_rows(&ProcessObservations::default(), &capture, &[]);
    let mut process = directory_row();
    process.command = CommandText::of("cargo", &["test"]);
    census.annotate_capture(&mut process, &capture, ScannerHome::Unavailable);
    assert!(matches!(
        process.capture_membership,
        CaptureMembership::Enclosing(_)
    ));
    let mut rows = vec![process];
    census.add_registration_rows(&mut rows, &capture, ScannerHome::Unavailable);
    assert_eq!(rows.len(), 2);
    assert_ne!(rows[0].invocation_id, rows[1].invocation_id);
    assert_eq!(rows[0].command, CommandText::of("cargo", &["test"]));
    assert_eq!(rows[1].command, CommandText::of("cargo", &["build"]));
}

#[test]
fn current_registration_argv_is_reconsidered_when_the_census_name_was_not_cargo() {
    let fixture = tests::cargo_process("build");
    let pid = Pid::from_u32(fixture.pid);
    let system = ProcessObservations::new([fixture.observation()]);
    let root = tempdir().expect("root");
    tests::write_versioned_capture(
        root.path(),
        pid.as_u32(),
        "generation",
        "/writer/project",
        "/writer",
        "build",
        "",
    );
    let capture = tests::verified_capture(root.path());
    let mut census = tests::census_of(&[(pid.as_u32(), 1)]);
    assert_eq!(census.cargo, [] as [Pid; 0]);
    census.include_registered_processes(&system, &capture);
    assert_eq!(census.cargo, [pid]);
    let groups = CensusSequence::assemble(&mut census, &system, &capture, &[]);
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].lead.pid, pid.as_u32());
    assert_eq!(groups[0].lead.managed, Measurement::Reading(0));
    assert!(matches!(
        groups[0].lead.invocation_id,
        InvocationId::Captured(_)
    ));
}

#[test]
fn assembled_managed_counts_include_registration_children_and_nested_parents() {
    let root = tempdir().expect("root");
    for pid in [20, 30] {
        tests::write_versioned_capture(
            root.path(),
            pid,
            "generation",
            "/writer/project",
            "/writer",
            "build",
            "",
        );
    }
    let capture = tests::verified_capture(root.path());
    let mut census = tests::census_of(&[(20, 10), (30, 20)]);
    census.select_rows(&ProcessObservations::default(), &capture, &[]);
    let mut rows = vec![directory_row()];
    census.add_registration_rows(&mut rows, &capture, ScannerHome::Unavailable);
    let groups = census.assemble_groups(
        &ProcessObservations::default(),
        ScannerHome::Unavailable,
        rows,
    );
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].lead.pid, 10);
    assert_eq!(groups[0].lead.managed, Measurement::Reading(2));
    assert_eq!(groups[0].rest.len(), 2);
    let parent = groups[0]
        .rest
        .iter()
        .find(|row| row.pid == 20)
        .expect("parent");
    assert_eq!(parent.managed, Measurement::Reading(1));
    let child = groups[0]
        .rest
        .iter()
        .find(|row| row.pid == 30)
        .expect("child");
    assert_eq!(
        child.managed,
        Measurement::Unavailable(MeasurementAbsence::Unproven)
    );
    assert_eq!(
        child.parent,
        VisibleParent::Invocation {
            id:  parent.invocation_id.clone(),
            pid: parent.pid,
        }
    );
}

#[test]
fn group_absence_distinguishes_process_identity_and_deliberate_exclusion() {
    let fixture = tests::cargo_process("build");
    let pid = Pid::from_u32(fixture.pid);
    let system = ProcessObservations::new([fixture.observation()]);
    // An unavailable CPU bucket does not remove an otherwise displayable group.
    let groups = CensusSequence::default().sample(&system);
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].lead.pid, pid.as_u32());
    assert_eq!(
        groups[0].lead.resource_use.cpu,
        Measurement::Unavailable(MeasurementAbsence::Unproven)
    );
    assert_eq!(
        groups[0].lead.subtree_resource_use.cpu,
        groups[0].lead.resource_use.cpu
    );
    let unobserved = CensusSequence::default().sample(&system);
    assert_eq!(
        unobserved[0].lead.resource_use.cpu,
        groups[0].lead.resource_use.cpu
    );
    let mut census = tests::census_of(&[]);
    let children = HashMap::new();
    let capture = Capture::default();
    assert_eq!(
        census.group(
            &ProcessObservations::default(),
            &tests::no_measurements(),
            ScannerHome::Unavailable,
            &children,
            &capture,
            pid
        ),
        Err(GroupAbsence::NoProcess)
    );
    assert_eq!(
        census.group(
            &system,
            &tests::no_measurements(),
            ScannerHome::Unavailable,
            &children,
            &capture,
            pid
        ),
        Err(GroupAbsence::NoIdentity)
    );
    census
        .eligibility
        .insert(pid, Err(RowAbsence::PolicyExcluded));
    assert_eq!(
        census.group(
            &system,
            &tests::no_measurements(),
            ScannerHome::Unavailable,
            &children,
            &capture,
            pid
        ),
        Err(GroupAbsence::Excluded)
    );
}

#[test]
fn wrapper_classification_requires_fresh_nonempty_arguments() {
    let root = tempdir().expect("capture root");
    tests::write_versioned_capture(
        root.path(),
        10,
        "generation",
        "/writer/project",
        "/writer",
        "build",
        "",
    );
    let capture = tests::verified_capture(root.path());
    let forwarding = [OsString::from("cargo"), OsString::from("build")];
    let mismatched = [OsString::from("application")];
    let records = ProcessObservations::new(
        [
            (100, forwarding.as_slice(), DetailPresence::Absent),
            (101, mismatched.as_slice(), DetailPresence::Absent),
            (102, forwarding.as_slice(), DetailPresence::Observed),
            (103, mismatched.as_slice(), DetailPresence::Observed),
            (104, &[], DetailPresence::Observed),
        ]
        .map(|(pid, argv, details)| {
            let mut record = ProcessObservation::cargo(pid, argv);
            record.parent = ProcessField::Observed(Pid::from_u32(10));
            record.details = details;
            record
        }),
    );
    let mut census = Census::take(records.records.values());
    census.identify_capture_wrappers(&records, &capture);
    assert_eq!(census.capture_wrappers, HashSet::from([Pid::from_u32(102)]));
    assert_eq!(
        census.capture_mismatches,
        HashSet::from([Pid::from_u32(103)])
    );
}

#[test]
fn observation_targets_prefer_arguments_and_resolve_environment_against_cwd() {
    let root = tempdir().expect("target root");
    let explicit = root.path().join("explicit");
    let environment = root.path().join("environment");
    fs::create_dir(&explicit).expect("explicit target");
    fs::create_dir(&environment).expect("environment target");
    let argv = [
        OsString::from("cargo"),
        "build".into(),
        CARGO_TARGET_DIR_FLAG.into(),
        "explicit".into(),
    ];
    let variables = [OsString::from("CARGO_TARGET_DIR=environment")];
    let mut record = ProcessObservation::cargo(100, &argv);
    record.directory = ProcessField::Observed(root.path());
    record.environment = &variables;
    assert_eq!(
        invocation_cpu_accounting::cargo_target_directory(&record).expect("argument target"),
        explicit.canonicalize().expect("explicit path")
    );
    record.argv = ProcessField::Observed(&argv[..2]);
    assert_eq!(
        invocation_cpu_accounting::cargo_target_directory(&record).expect("environment target"),
        environment.canonicalize().expect("environment path")
    );
    record.directory = ProcessField::Unavailable;
    assert!(invocation_cpu_accounting::cargo_target_directory(&record).is_err());
    record.argv = ProcessField::Unavailable;
    assert!(invocation_cpu_accounting::cargo_target_directory(&record).is_err());
}

#[test]
fn shim_detection_requires_observed_cwds_and_subcommands() {
    let argv = [OsString::from("cargo"), OsString::from("build")];
    let cwd = Path::new("/work");
    assert!(!observed_shim_match(
        &argv,
        WorkingDirectoryObservation::Unavailable,
        &argv,
        WorkingDirectoryObservation::Unavailable
    ));
    assert!(!observed_shim_match(
        &argv,
        WorkingDirectoryObservation::Observed(cwd),
        &argv,
        WorkingDirectoryObservation::Unavailable
    ));
    assert!(!observed_shim_match(
        &[],
        WorkingDirectoryObservation::Observed(cwd),
        &[],
        WorkingDirectoryObservation::Observed(cwd)
    ));
    assert!(observed_shim_match(
        &argv,
        WorkingDirectoryObservation::Observed(cwd),
        &argv,
        WorkingDirectoryObservation::Observed(cwd)
    ));
    assert!(!observed_shim_match(
        &argv,
        WorkingDirectoryObservation::Observed(cwd),
        &argv,
        WorkingDirectoryObservation::Observed(Path::new("/other"))
    ));
}

#[test]
fn directory_aliases_preserve_wrapper_and_registration_associations() {
    let root = tempdir().expect("capture root");
    let real = root.path().join("real");
    fs::create_dir(&real).expect("working directory");
    let alias = root.path().join("alias");
    symlink(&real, &alias).expect("working directory alias");
    let argv = [OsString::from("cargo"), OsString::from("build")];
    assert!(observed_shim_match(
        &argv,
        WorkingDirectoryObservation::Observed(&real),
        &argv,
        WorkingDirectoryObservation::Observed(&alias),
    ));
    assert_eq!(
        DirectoryComparison::between(&real, root.path()),
        DirectoryComparison::Different
    );
    assert_eq!(
        DirectoryComparison::between(&real, &root.path().join("missing")),
        DirectoryComparison::Unavailable
    );
    assert_eq!(
        DirectoryComparison::between(Path::new("~/real"), &real),
        DirectoryComparison::Unavailable
    );

    tests::write_versioned_capture(
        root.path(),
        10,
        "generation",
        real.to_str().expect("fixture path"),
        root.path().to_str().expect("fixture home"),
        "build",
        "",
    );
    let capture = tests::verified_capture(root.path());
    let home = ScannerHome::Known(root.path());
    let (identity, path, _) = invocation::row_fields(
        WorkingDirectoryObservation::Observed(&alias),
        &argv,
        &capture.row_source(10),
        home,
    )
    .expect("alias agrees with verified registration");
    assert_eq!(identity, WorkingDirectoryIdentity::Absolute(real.clone()));
    assert_eq!(path, "~/real");

    let mut census = tests::census_of(&[]);
    census.identify_captures(&capture);
    let mut row = directory_row();
    row.directory_identity = WorkingDirectoryIdentity::Absolute(alias);
    census.annotate_capture(&mut row, &capture, home);
    assert_eq!(
        row.directory_identity,
        WorkingDirectoryIdentity::Absolute(real)
    );
    assert_eq!(row.path, "~/real");
}

#[test]
fn an_unconfirmed_driver_capture_belongs_only_to_its_enclosing_invocation() {
    for (program, arguments) in [
        ("cargo-mend", vec!["cargo-mend", "mend"]),
        ("cargo-nextest", vec!["cargo-nextest", "nextest", "run"]),
    ] {
        let root = tests::capture_root(&[(10, "Building [=>] 998/1000: checking\r")]);
        let capture = Capture::take_from(root.path(), |pid| {
            KernelObservation::for_test(pid, Observation::Unknown)
        });
        let shell = [OsString::from("bash")];
        let lead = arguments
            .into_iter()
            .map(OsString::from)
            .collect::<Vec<_>>();
        let child = ["cargo", "check"].map(OsString::from);
        let records = ProcessObservations::new(
            [
                (9, 1, "bash", shell.as_slice()),
                (10, 9, program, lead.as_slice()),
                (11, 10, "cargo", child.as_slice()),
            ]
            .map(|(pid, parent, name, argv)| {
                let mut process = ProcessObservation::cargo(pid, argv);
                process.name = ProcessField::Observed(OsStr::new(name));
                process.parent = ProcessField::Observed(Pid::from_u32(parent));
                process.uid = ProcessField::Observed(2000);
                process.directory = ProcessField::Observed(Path::new("/runner/project"));
                process
            }),
        );
        let mut census = Census::take(records.processes().values());
        census.prepare(&records, &capture, &[]);
        let NearestRegistration::Registered(lead_key) =
            census.captured_run(&capture, Pid::from_u32(10))
        else {
            panic!("driver capture");
        };
        assert_eq!(lead_key.pid, 10);
        assert_eq!(
            census.captured_run(&capture, Pid::from_u32(11)),
            NearestRegistration::Registered(lead_key)
        );
        assert_eq!(
            census.direct_capture(&capture, Pid::from_u32(10)),
            DirectAssociation::None
        );
        assert_eq!(
            census.direct_capture(&capture, Pid::from_u32(11)),
            DirectAssociation::None
        );
        let groups = CensusSequence::default().sample_capture(&records, &capture);
        assert_eq!(groups.len(), 1);
        assert_eq!(
            groups[0].lead.capture_membership,
            CaptureMembership::Outside
        );
        assert_eq!(
            groups[0].rest[0].capture_membership,
            CaptureMembership::EnclosingUnconfirmed
        );
    }
}

#[test]
fn direct_registration_and_process_share_identity_while_nested_invocations_do_not() {
    let root = tempdir().expect("capture root");
    tests::write_versioned_capture(
        root.path(),
        10,
        "generation",
        "/writer/project",
        "/writer",
        "build",
        "",
    );
    let capture = tests::verified_capture(root.path());
    let mut census = tests::census_of(&[(11, 10), (12, 11)]);
    census.cargo = vec![Pid::from_u32(11), Pid::from_u32(12)];
    census.identify_captures(&capture);
    let direct = match census.direct_capture(&capture, Pid::from_u32(11)) {
        DirectAssociation::Direct(direct) => Ok(direct),
        DirectAssociation::None => Err("first cargo must directly represent the registration"),
    }
    .expect("direct proof");
    let mut process = directory_row();
    process.pid = 11;
    census.annotate_capture(&mut process, &capture, ScannerHome::Unavailable);
    assert_eq!(process.invocation_id, direct.invocation_id());
    assert_eq!(process.capture_membership, CaptureMembership::Outside);
    let mut nested = directory_row();
    nested.pid = 12;
    nested.path = "/nested".to_owned();
    nested.command = CommandText::of("cargo", &["test"]);
    census.annotate_capture(&mut nested, &capture, ScannerHome::Unavailable);
    assert_ne!(process.invocation_id, nested.invocation_id);
    assert_eq!(
        nested.capture_membership,
        CaptureMembership::Enclosing(direct.run_id)
    );
    assert_eq!(nested.path, "/nested");
    assert_eq!(nested.command, CommandText::of("cargo", &["test"]));
    assert_eq!(
        census.direct_capture(&capture, Pid::from_u32(12)),
        DirectAssociation::None
    );
}

#[test]
fn unverified_nearest_registration_cannot_supply_direct_row_metadata() {
    let root = tests::capture_root(&[(10, "")]);
    let capture = Capture::take_from(root.path(), |pid| {
        KernelObservation::for_test(pid, Observation::Unknown)
    });
    let census = tests::census_of(&[(11, 10)]);
    assert!(matches!(
        census.captured_run(&capture, Pid::from_u32(11)),
        NearestRegistration::Registered(_)
    ));
    assert_eq!(
        census.direct_capture(&capture, Pid::from_u32(11)),
        DirectAssociation::None
    );
}

#[test]
fn identical_pid_and_birth_with_different_generations_never_share_cpu_history() {
    let root = tempdir().expect("capture root");
    for generation in ["first", "second"] {
        tests::write_versioned_capture(
            root.path(),
            10,
            generation,
            "/writer/project",
            "/writer",
            "build",
            "",
        );
    }
    let capture = tests::verified_capture(root.path());
    let identities: Vec<_> = capture
        .confirmed()
        .iter()
        .map(|confirmed| {
            InvocationId::Captured(RunId::verified(&confirmed.key, &confirmed.registration))
        })
        .collect();
    assert_eq!(identities.len(), 2);
    assert_ne!(identities[0], identities[1]);
    let mut smoothing = InvocationCpuAccounting::default();
    let now = Instant::now();
    smoothing.settle(
        &HashMap::from([(identities[0].clone(), Measurement::Reading(400.0))]),
        &identities[..1],
        now,
    );
    let reported = smoothing.settle(
        &HashMap::from([(
            identities[1].clone(),
            Measurement::Unavailable(MeasurementAbsence::FirstObservation),
        )]),
        &identities[1..],
        now + invocation_cpu_accounting::poll(),
    );
    assert_eq!(
        reported[&identities[1]],
        Measurement::Unavailable(MeasurementAbsence::FirstObservation)
    );
    assert!(!smoothing.settled.contains_key(&identities[0]));
    assert!(!smoothing.reported.contains_key(&identities[0]));

    let pid = Pid::from_u32(10);
    let compiler = invocation_cpu_accounting::cpu_process_identity(30);
    let mut census = tests::census_of(&[]);
    census.cargo.push(pid);
    census.lifetimes.insert(
        pid,
        LifetimeEvidence::Available(ProcessLifetime::for_test(10)),
    );
    census.lifetimes.insert(
        Pid::from_u32(30),
        LifetimeEvidence::Available(ProcessLifetime::for_test(30)),
    );
    census.accumulated.insert(pid, Duration::ZERO);
    census.cpu.insert(pid, Measurement::Reading(0.0));
    census.identities.insert(pid, identities[0].clone());
    let first =
        census.attribute_cpu_with(&ProcessObservations::default(), &mut smoothing, now, |_| {
            Measurement::Reading(Duration::from_millis(100))
        });
    assert_eq!(
        first[&pid],
        Measurement::Unavailable(MeasurementAbsence::FirstObservation)
    );
    smoothing
        .cache_owners
        .insert(compiler.clone(), identities[0].clone());
    smoothing
        .invocations
        .get_mut(&identities[0])
        .expect("first generation counter")
        .detached
        .entry(compiler.clone())
        .or_default()
        .update(&invocation_cpu_accounting::cpu_work(&[(30, 400)]).tree);
    census.identities.insert(pid, identities[1].clone());
    let replaced = census.attribute_cpu_with(
        &ProcessObservations::default(),
        &mut smoothing,
        now + Duration::from_secs(1),
        |_| Measurement::Reading(Duration::from_millis(200)),
    );
    assert_eq!(
        replaced[&pid],
        Measurement::Unavailable(MeasurementAbsence::FirstObservation)
    );
    assert!(!smoothing.invocations.contains_key(&identities[0]));
    assert_eq!(
        smoothing.invocations[&identities[1]].accumulated,
        Duration::from_millis(200)
    );
    assert!(smoothing.invocations[&identities[1]].detached.is_empty());
    assert_eq!(smoothing.cache_owners[&compiler], identities[0]);
}

#[test]
fn unavailable_lifetimes_retain_present_rows_without_claiming_cpu_continuity() {
    let pid = Pid::from_u32(10);
    let lifetimes = HashMap::from([(pid, LifetimeEvidence::Unavailable)]);
    let mut identities = ProcessIdentities::default();
    let first = identities.observe(&lifetimes);
    assert_eq!(first, identities.observe(&lifetimes));
    assert!(matches!(
        first[&pid],
        InvocationId::Process(ProcessIdentity::Unavailable { .. })
    ));
    assert!(identities.observe(&HashMap::new()).is_empty());
    assert_ne!(first, identities.observe(&lifetimes));
    let previous = HashMap::from([(
        Pid::from_u32(10),
        CpuBaseline {
            lifetime:    LifetimeEvidence::Unavailable,
            accumulated: 10,
        },
    )]);
    assert_eq!(
        Census::measure_cpu(
            Pid::from_u32(10),
            50.0,
            &CpuBaseline {
                lifetime:    LifetimeEvidence::Unavailable,
                accumulated: 20,
            },
            &previous
        ),
        Measurement::Unavailable(MeasurementAbsence::Unproven)
    );
}

#[test]
fn exec_replaced_application_separates_nested_cargo_from_the_enclosing_run() {
    let root = tempdir().expect("capture root");
    tests::write_versioned_capture(
        root.path(),
        10,
        "generation",
        "/writer/project",
        "/writer",
        "run",
        "",
    );
    let capture = tests::verified_capture(root.path());
    let mut census = tests::census_of(&[(11, 10), (12, 11), (13, 11)]);
    census.cargo = vec![Pid::from_u32(12), Pid::from_u32(13)];
    census.identify_captures(&capture);
    let mut nested = Vec::new();
    for pid in [12, 13] {
        assert_eq!(
            census.direct_capture(&capture, Pid::from_u32(pid)),
            DirectAssociation::None
        );
        let mut row = directory_row();
        row.pid = pid;
        census.annotate_capture(&mut row, &capture, ScannerHome::Unavailable);
        assert!(matches!(
            row.capture_membership,
            CaptureMembership::Enclosing(_)
        ));
        nested.push(row);
    }
    assert_ne!(nested[0].invocation_id, nested[1].invocation_id);
    census.select_rows(
        &ProcessObservations::default(),
        &capture,
        &["run".to_owned()],
    );
    for pid in [12, 13] {
        assert_eq!(
            census.eligibility[&Pid::from_u32(pid)],
            Err(RowAbsence::ArgvUnavailable)
        );
    }
}

#[test]
fn direct_capture_crosses_only_observed_command_forwarders() {
    let root = tempdir().expect("capture root");
    tests::write_versioned_capture(
        root.path(),
        10,
        "generation",
        "/writer/project",
        "/writer",
        "build",
        "",
    );
    let capture = tests::verified_capture(root.path());
    let mut census = tests::census_of(&[(11, 10), (12, 11), (13, 12)]);
    census.cargo = vec![Pid::from_u32(13)];
    assert_eq!(
        census.direct_capture(&capture, Pid::from_u32(13)),
        DirectAssociation::None
    );
    census
        .capture_wrappers
        .extend([Pid::from_u32(11), Pid::from_u32(12)]);
    assert!(matches!(
        census.direct_capture(&capture, Pid::from_u32(13)),
        DirectAssociation::Direct(_)
    ));
    census.capture_wrappers.remove(&Pid::from_u32(12));
    assert_eq!(
        census.direct_capture(&capture, Pid::from_u32(13)),
        DirectAssociation::None
    );
}

#[test]
fn capture_forwarding_matches_exact_registered_arguments_in_both_pty_forms() {
    let record = tests::directory_record("/writer");
    let native = format!(
        "exec '/writer'\\''s toolchain/cargo-tile-capture' '{}' '/toolchain/cargo' '/capture' 'build'",
        crate::constants::CAPTURE_SHIM_ARGUMENT
    );
    for argv in [
        vec![
            "script",
            "-q",
            "-t",
            "0",
            "/capture/run-generation-10.log",
            "/toolchain/cargo-tile-capture",
            crate::constants::CAPTURE_SHIM_ARGUMENT,
            "/toolchain/cargo",
            "/capture",
            "build",
        ],
        vec![
            "script",
            "-q",
            "-e",
            "-f",
            "-c",
            native.as_str(),
            "/capture/run-generation-10.log",
        ],
        vec!["sh", "-c", native.as_str()],
        vec![
            "script",
            "-q",
            "-t",
            "0",
            "/capture/run-generation-10.log",
            "/toolchain/cargo-tile-real",
            "build",
        ],
        vec![
            "script",
            "-q",
            "-e",
            "-f",
            "-c",
            "'/toolchain/cargo-tile-real' 'build' ",
            "/capture/run-generation-10.log",
        ],
        vec!["sh", "-c", "'/toolchain/cargo-tile-real' 'build' "],
        vec![
            "sh",
            "-c",
            "'/writer'\\''s toolchain/cargo-tile-real' 'build' ",
        ],
    ] {
        assert!(forwards_capture_command(
            &argv.into_iter().map(OsString::from).collect::<Vec<_>>(),
            &record
        ));
    }
    for argv in [
        vec![
            "script",
            "-q",
            "-t",
            "0",
            "/capture/run-generation-10.log",
            "/toolchain/cargo-tile-capture",
            crate::constants::CAPTURE_SHIM_ARGUMENT,
            "/toolchain/cargo",
            "/capture",
            "test",
        ],
        vec!["sh", "/writer/application"],
        vec!["cargo-tile-real", "test"],
        vec!["sh", "-c", "'/toolchain/cargo-tile-real' 'test' "],
        vec!["sh", "-c", "'printf' '/toolchain/cargo-tile-real' 'build' "],
    ] {
        assert!(!forwards_capture_command(
            &argv.into_iter().map(OsString::from).collect::<Vec<_>>(),
            &record
        ));
    }
}

#[test]
fn capture_forwarding_preserves_non_utf8_paths_and_arguments() {
    let mut bytes = tests::directory_record_bytes("/writer");
    bytes.insert(bytes.len() - 1, 0xff);
    let record = match Registration::parse(&bytes).expect("byte-preserving registration") {
        Registration::Versioned(record) => Ok(record),
        Registration::Legacy(_) => Err("expected v2"),
    }
    .expect("versioned record");
    let argv = [
        OsString::from("sh"),
        OsString::from("-c"),
        OsString::from_vec(b"'/toolchain\xff/cargo-tile-real' 'build\xff' ".to_vec()),
    ];
    assert!(forwards_capture_command(&argv, &record));
}

#[test]
fn nested_selected_rows_keep_commands_pids_directories_and_enclosing_progress() {
    tests::assert_nested_selected_rows("build", &[]);
}

#[test]
fn exec_nested_selected_rows_keep_commands_pids_directories_and_enclosing_progress() {
    tests::assert_nested_selected_rows("run", &[]);
}

#[test]
fn exec_excluded_nested_rows_keep_progress_and_writes_continue_after_sweep() {
    tests::assert_nested_selected_rows("run", &["run".into()]);
}

#[test]
fn quiet_json_short_selects_one_direct_row_with_original_arguments() {
    tests::assert_accepted_rewrite_rows(
        &[
            "check",
            "probe-json",
            "-q",
            "--message-format=json",
            "-q",
            "--",
            "--quiet",
            "-q",
        ],
        &[
            "check",
            "probe-json",
            "--message-format=json",
            "--",
            "--quiet",
            "-q",
        ],
    );
}

#[test]
fn quiet_json_separate_selects_one_direct_row_with_original_arguments() {
    tests::assert_accepted_rewrite_rows(
        &[
            "check",
            "probe-json",
            "-q",
            "--message-format",
            "json",
            "-q",
            "--",
            "--quiet",
            "-q",
        ],
        &[
            "check",
            "probe-json",
            "--message-format",
            "json",
            "--",
            "--quiet",
            "-q",
        ],
    );
}

#[test]
fn rejected_non_json_quiet_rewrites_keep_process_and_registration_rows() {
    for quiet in ["--quiet", "-q"] {
        tests::assert_rejected_rewrite_rows(
            &["check", "probe-mismatch", quiet],
            &["check", "probe-mismatch"],
        );
    }
}

#[test]
fn rejected_post_separator_quiet_rewrites_keep_process_and_registration_rows() {
    for quiet in ["--quiet", "-q"] {
        tests::assert_rejected_rewrite_rows(
            &[
                "check",
                "probe-mismatch",
                quiet,
                "--message-format=json",
                "--",
                quiet,
            ],
            &["check", "probe-mismatch", "--message-format=json", "--"],
        );
    }
}

#[test]
fn rejected_unrelated_rewrite_keeps_process_and_registration_rows() {
    tests::assert_rejected_rewrite_rows(
        &[
            "check",
            "probe-mismatch",
            "--quiet",
            "--message-format=json",
            "--workspace",
        ],
        &[
            "check",
            "probe-mismatch",
            "--message-format=json",
            "--release",
        ],
    );
}

#[test]
fn same_second_replacement_with_a_higher_counter_begins_a_new_sample() {
    let pid = Pid::from_u32(10);
    let before = LifetimeEvidence::Available(birth_stamp::ProcessLifetime::for_test(100_001));
    let after = LifetimeEvidence::Available(birth_stamp::ProcessLifetime::for_test(100_002));
    assert_ne!(
        ProcessIdentity::observed(10, before.clone()),
        ProcessIdentity::observed(10, after.clone())
    );
    let previous = HashMap::from([(
        pid,
        CpuBaseline {
            lifetime:    before,
            accumulated: 10,
        },
    )]);
    assert_eq!(
        Census::measure_cpu(
            pid,
            80.0,
            &CpuBaseline {
                lifetime:    after,
                accumulated: 50,
            },
            &previous
        ),
        Measurement::Unavailable(MeasurementAbsence::FirstObservation)
    );
}

#[test]
fn excluded_registration_stays_live_and_preserves_nested_membership_boundaries() {
    let root = tempdir().expect("capture root");
    tests::write_versioned_capture(
        root.path(),
        10,
        "live",
        "/writer/project",
        "/writer",
        "build",
        "",
    );
    let capture = tests::verified_capture(root.path());
    let mut census = tests::census_of(&[(11, 10), (12, 11)]);
    census.cargo = vec![Pid::from_u32(11), Pid::from_u32(12)];
    census.identify_captures(&capture);
    let parents = census.parents.clone();
    census.select_rows(
        &ProcessObservations::default(),
        &capture,
        &["build".to_owned()],
    );
    assert_eq!(census.cargo, [] as [Pid; 0]);
    assert_eq!(
        census.eligibility[&Pid::from_u32(11)],
        Err(RowAbsence::PolicyExcluded)
    );
    assert_eq!(
        census.eligibility[&Pid::from_u32(12)],
        Err(RowAbsence::ArgvUnavailable)
    );
    assert_eq!(census.parents, parents);
    assert_eq!(
        census.direct_capture(&capture, Pid::from_u32(12)),
        DirectAssociation::None
    );
    assert_eq!(tests::verified_capture(root.path()).confirmed().len(), 1);
    assert!(root.path().join("state/pids/10.live").exists());
    assert!(root.path().join("run-live-10.log").exists());
}

#[test]
fn capture_annotation_uses_one_root_for_progress_directory_and_command() {
    let first = tempdir().expect("first root");
    let second = tempdir().expect("second root");
    tests::write_versioned_capture(
        first.path(),
        10,
        "first",
        "/writer/project",
        "/writer",
        "build",
        "",
    );
    tests::write_versioned_capture(
        second.path(),
        10,
        "second",
        "/runner/project",
        "/runner",
        "test",
        "    Blocking waiting for file lock on build directory",
    );
    let record = tests::directory_record("/writer");
    let stamp = match record.identity() {
        IdentityEvidence::Available(stamp) => Ok(stamp),
        IdentityEvidence::Unavailable => Err("fixture identity unavailable"),
    }
    .expect("fixture supplies a complete birth");
    for (paths, directory, command, expected_path, expected_state) in [
        (
            [first.path(), second.path()],
            "/writer/project",
            "build",
            "~/project",
            CaptureLookup::Registered(CaptureRead::NoCurrentProgress),
        ),
        (
            [second.path(), first.path()],
            "/runner/project",
            "test",
            "/runner/project",
            CaptureLookup::Registered(CaptureRead::Progress(RunState::Blocked)),
        ),
    ] {
        let roots = tests::resolved_test_roots(&paths);
        let capture = Capture::take_roots(&roots, &|pid| {
            KernelObservation::for_test(pid, Observation::Present(stamp.clone()))
        });
        let census = tests::census_of(&[]);
        let key = capture.keys(10).next().expect("preferred root key");
        assert_eq!(capture.keys(10).count(), 2);
        assert_eq!(capture.confirmed().len(), 2);
        assert_eq!(
            census.captured_run(&capture, Pid::from_u32(10)),
            NearestRegistration::Registered(key)
        );

        let mut row = directory_row();
        row.path = "process directory".to_owned();
        row.directory_identity = WorkingDirectoryIdentity::Absolute(directory.into());
        row.command = CommandText::of("cargo", &[command]);
        census.annotate_capture(&mut row, &capture, ScannerHome::Known(Path::new("/writer")));

        assert_eq!(row.path, expected_path);
        assert_eq!(
            row.directory_identity,
            WorkingDirectoryIdentity::Absolute(directory.into())
        );
        assert_eq!(row.command, CommandText::of("cargo", &[command]));
        assert_eq!(row.state, expected_state);
    }
}

#[test]
fn associated_root_names_proofs_suppressed_by_precedence_or_nearer_ancestry() {
    for (preferred_index, confirmed_pid) in [(0, 10), (1, 20)] {
        let preferred = tests::capture_root(&[(10, "")]);
        let confirmed = tempdir().expect("confirmed root");
        tests::write_versioned_capture(
            confirmed.path(),
            confirmed_pid,
            "verified",
            "/writer/project",
            "/writer",
            "build",
            "",
        );
        let record = tests::directory_record("/writer");
        let stamp = match record.identity() {
            IdentityEvidence::Available(stamp) => Ok(stamp),
            IdentityEvidence::Unavailable => Err("fixture has no identity"),
        }
        .expect("complete fixture birth");
        let paths = if preferred_index == 0 {
            [preferred.path(), confirmed.path()]
        } else {
            [confirmed.path(), preferred.path()]
        };
        let roots = tests::resolved_test_roots(&paths);
        let mut capture = Capture::take_roots(&roots, &|pid| {
            KernelObservation::for_test(pid, Observation::Present(stamp.clone()))
        });
        let groups = vec![CargoGroup {
            lead:     directory_row(),
            rest:     Vec::new(),
            ancestry: AncestryWalk::Ended(Vec::new()),
        }];
        tests::census_of(&[(10, 20)]).associate_status(&mut capture, &groups);
        let associations = &capture.root_status[preferred_index].associations;
        assert_eq!(associations.len(), 1);
        assert_eq!(associations[0].pid, 10);
        let AssociationSelection::Selected { key, proof, unused } = &associations[0].selection
        else {
            panic!("preferred root selects its reading");
        };
        assert_eq!(key.pid, 10);
        assert_eq!(*proof, SelectedProof::Unconfirmed);
        if confirmed_pid == 10 {
            assert_eq!(unused.len(), 1);
            assert_eq!(unused[0].reason, UnusedCaptureReason::SelectedUnconfirmed);
            assert_eq!(
                unused[0].root,
                confirmed.path().canonicalize().expect("root")
            );
        } else {
            assert_eq!(
                capture.root_status[1 - preferred_index].associations.len(),
                1
            );
        }
    }
}

#[test]
fn an_unconfirmed_root_does_not_borrow_another_roots_confirmed_metadata() {
    let first = tests::capture_root(&[(10, "")]);
    let second = tempdir().expect("confirmed root");
    tests::write_versioned_capture(
        second.path(),
        10,
        "second",
        "/writer/project",
        "/custom",
        "test",
        "    Blocking waiting for file lock on build directory",
    );
    let record = tests::directory_record("/writer");
    let stamp = match record.identity() {
        IdentityEvidence::Available(stamp) => Ok(stamp),
        IdentityEvidence::Unavailable => Err("fixture identity unavailable"),
    }
    .expect("fixture supplies a complete birth");
    let roots = tests::resolved_test_roots(&[first.path(), second.path()]);
    let capture = Capture::take_roots(&roots, &|pid| {
        KernelObservation::for_test(pid, Observation::Present(stamp.clone()))
    });
    assert_eq!(capture.keys(10).count(), 2);
    assert_eq!(capture.confirmed().len(), 1);
    assert_eq!(capture.confirmed()[0].key.root, CaptureRootIndex(1));
    let mut row = directory_row();
    row.path = "process directory".to_owned();
    tests::census_of(&[]).annotate_capture(
        &mut row,
        &capture,
        ScannerHome::Known(Path::new("/writer")),
    );

    assert_eq!(row.path, "process directory");
    assert_eq!(
        row.directory_identity,
        WorkingDirectoryIdentity::Absolute("/writer/project".into())
    );
    assert_eq!(row.command, CommandText::of("cargo", &["build"]));
    assert_eq!(
        row.state,
        CaptureLookup::Registered(CaptureRead::NoCurrentProgress)
    );
}

#[test]
fn a_nearer_capture_in_an_additional_root_precedes_the_own_root_ancestor() {
    let first =
        tests::capture_root(&[(20, "    Blocking waiting for file lock on build directory")]);
    let second = tests::capture_root(&[(10, "")]);
    let roots = tests::resolved_test_roots(&[first.path(), second.path()]);
    let capture = Capture::take_roots(&roots, &|pid| {
        KernelObservation::for_test(pid, Observation::Unknown)
    });
    let census = tests::census_of(&[(11, 10), (10, 20)]);
    assert_eq!(
        census.captured_run(&capture, Pid::from_u32(11)),
        NearestRegistration::Registered(capture.keys(10).next().expect("nearest key")),
    );
    assert_eq!(
        tests::captured_state(&census, &capture, 11),
        CaptureLookup::Registered(CaptureRead::NoCurrentProgress)
    );
}

#[test]
fn capture_annotation_changes_only_display_when_writer_home_differs() {
    for (home, display) in [("/writer", "~/project"), ("/custom", "/writer/project")] {
        let root = tempdir().expect("capture root");
        let markers = root.path().join(CAPTURE_LIVE_RUNS_DIR);
        fs::create_dir_all(&markers).expect("registration directory");
        fs::write(
            markers.join("10.generation"),
            tests::directory_record_bytes(home),
        )
        .expect("versioned registration");
        let record = tests::directory_record(home);
        let stamp = match record.identity() {
            IdentityEvidence::Available(stamp) => Ok(stamp),
            IdentityEvidence::Unavailable => Err("fixture identity unavailable"),
        }
        .expect("fixture supplies a complete birth");
        let capture = Capture::take_from(root.path(), |pid| {
            KernelObservation::for_test(pid, Observation::Present(stamp.clone()))
        });
        assert_eq!(capture.confirmed().len(), 1);
        let mut row = directory_row();
        let identity = row.directory_identity.clone();

        tests::census_of(&[]).annotate_capture(
            &mut row,
            &capture,
            ScannerHome::Known(Path::new("/writer")),
        );

        assert_eq!(row.path, display);
        assert_eq!(row.directory_identity, identity);

        row.directory_identity = WorkingDirectoryIdentity::Absolute("/other/project".into());
        row.path = "~/project".to_owned();
        tests::census_of(&[]).annotate_capture(
            &mut row,
            &capture,
            ScannerHome::Known(Path::new("/writer")),
        );
        assert_eq!(row.path, "~/project");
        assert_eq!(
            row.directory_identity,
            WorkingDirectoryIdentity::Absolute("/other/project".into())
        );
    }
}

#[test]
fn nearest_empty_capture_does_not_inherit_an_enclosing_progress_reading() {
    let root = tests::capture_root(&[
        (10, ""),
        (20, "    Blocking waiting for file lock on build directory"),
    ]);
    let census = tests::census_of(&[(11, 10), (10, 20)]);
    let capture = Capture::take_from(root.path(), |pid| {
        KernelObservation::for_test(pid, Observation::Unknown)
    });
    assert_eq!(
        tests::captured_state(&census, &capture, 11),
        CaptureLookup::Registered(CaptureRead::NoCurrentProgress)
    );
}

#[test]
fn nearest_unreadable_capture_does_not_inherit_an_enclosing_progress_reading() {
    let root = tests::capture_root(&[
        (10, ""),
        (20, "    Blocking waiting for file lock on build directory"),
    ]);
    let log = root.path().join(format!(
        "{RUN_LOG_PREFIX}20260824-084300{PID_SEPARATOR}10{RUN_LOG_SUFFIX}"
    ));
    fs::remove_file(&log).expect("remove unreadable log placeholder");
    fs::create_dir(&log).expect("a directory cannot supply a log tail");
    let census = tests::census_of(&[(11, 10), (10, 20)]);
    let capture = Capture::take_from(root.path(), |pid| {
        KernelObservation::for_test(pid, Observation::Unknown)
    });
    assert!(matches!(
        tests::captured_state(&census, &capture, 11),
        CaptureLookup::Registered(CaptureRead::Unreadable(_))
    ));
}

/// A cell names what launched the command, so the walk has to
/// reach past the shell to whatever started that.
#[test]
fn the_chain_above_a_command_reads_outermost_first() {
    // cargo (64432) under a shell (12445) under a login (12444)
    // under the editor that opened it (6218), which launchd owns.
    let census = tests::census_of(&[(64432, 12445), (12445, 12444), (12444, 6218), (6218, 1)]);

    assert_eq!(
        census.ancestor_pids(Pid::from_u32(64432)),
        AncestryWalk::Ended(vec![
            Pid::from_u32(6218),
            Pid::from_u32(12444),
            Pid::from_u32(12445),
        ]),
    );
}

/// Every command on the machine descends from the init process, so
/// a row naming it tells one command from no other.
#[test]
fn the_walk_stops_short_of_the_process_the_tree_roots_at() {
    let census = tests::census_of(&[(64432, 12445), (12445, 1)]);

    assert_eq!(
        census.ancestor_pids(Pid::from_u32(64432)),
        AncestryWalk::Ended(vec![Pid::from_u32(12445)]),
    );
}

/// A reparented chain that comes back round on itself must end the
/// walk rather than spin it.
#[test]
fn a_chain_that_loops_ends_where_it_repeats() {
    let census = tests::census_of(&[(64432, 900), (900, 901), (901, 900)]);

    assert_eq!(
        census.ancestor_pids(Pid::from_u32(64432)),
        AncestryWalk::Ended(vec![Pid::from_u32(901), Pid::from_u32(900)]),
    );
}

#[test]
fn a_parent_walk_reports_a_cut_only_when_an_unvisited_parent_remains() {
    for count in [
        0,
        PARENT_WALK_LIMIT - 1,
        PARENT_WALK_LIMIT,
        PARENT_WALK_LIMIT + 1,
    ] {
        let parents: Vec<_> = (0..count)
            .map(|index| {
                let offset = u32::try_from(index).expect("fixture fits a pid");
                (100 + offset, 101 + offset)
            })
            .chain(std::iter::once((
                100 + u32::try_from(count).expect("fixture fits a pid"),
                ROOT_PROCESS_PID,
            )))
            .collect();
        let census = tests::census_of(&parents);
        let walk = census.ancestor_pids(Pid::from_u32(100));
        assert_eq!(walk.as_slice().len(), count.min(PARENT_WALK_LIMIT));
        let expected: Vec<_> = (0..count.min(PARENT_WALK_LIMIT))
            .rev()
            .map(|index| Pid::from_u32(101 + u32::try_from(index).expect("fixture fits a pid")))
            .collect();
        assert_eq!(walk.as_slice(), expected);
        assert_eq!(
            matches!(walk, AncestryWalk::LimitCut(_)),
            count > PARENT_WALK_LIMIT
        );
    }
    let mut parents: Vec<_> = (0..PARENT_WALK_LIMIT)
        .map(|index| {
            let offset = u32::try_from(index).expect("fixture fits a pid");
            (100 + offset, 101 + offset)
        })
        .collect();
    parents.push((
        100 + u32::try_from(PARENT_WALK_LIMIT).expect("fixture fits a pid"),
        101,
    ));
    assert!(matches!(
        tests::census_of(&parents).ancestor_pids(Pid::from_u32(100)),
        AncestryWalk::Ended(_)
    ));
    parents.pop();
    assert!(matches!(
        tests::census_of(&parents).ancestor_pids(Pid::from_u32(100)),
        AncestryWalk::Ended(_)
    ));
}

#[test]
fn unreadable_and_cargo_ancestors_do_not_erase_a_walk_limit_cut() {
    let parents: Vec<_> = (0..=PARENT_WALK_LIMIT)
        .map(|index| {
            let offset = u32::try_from(index).expect("fixture fits a pid");
            (100 + offset, 101 + offset)
        })
        .collect();
    let census = tests::census_of(&parents);
    let runner_argv = [
        OsString::from("/nix/store/runner/bin/Runner.Worker"),
        "spawnclient".into(),
    ];
    let cargo_argv = [OsString::from("cargo"), "check".into()];
    let observations = ProcessObservations::new([
        ProcessObservation::cargo(101, &runner_argv),
        ProcessObservation::cargo(102, &cargo_argv),
    ]);
    let ancestry = census.ancestry(&observations, ScannerHome::Unavailable, Pid::from_u32(100));
    assert!(matches!(ancestry, AncestryWalk::LimitCut(_)));
    assert_eq!(ancestry.as_slice().len(), 1);
    assert_eq!(ancestry.as_slice()[0].pid, 101);
    assert!(
        matches!(census.ancestry(&ProcessObservations::default(), ScannerHome::Unavailable, Pid::from_u32(100)), AncestryWalk::LimitCut(levels) if levels.is_empty())
    );
}

/// A nested cargo waits on the build-directory lock in its own
/// right, and nothing above it can say which invocation is the one
/// waiting -- so the row has to carry it.
#[test]
fn an_invocation_under_the_lead_reports_its_own_wait() {
    // `cargo doc` (76847) under a shim (76846) the lead (64432)
    // started, which is how a manager's nested cargo is captured.
    let root = tests::capture_root(&[(
        76846,
        "    Blocking waiting for file lock on build directory",
    )]);
    let census = tests::census_of(&[(76847, 76846), (76846, 64432)]);
    let capture = Capture::take_from(root.path(), |pid| {
        KernelObservation::for_test(pid, Observation::Unknown)
    });

    assert_eq!(
        tests::captured_state(&census, &capture, 76847),
        CaptureLookup::Registered(CaptureRead::Progress(RunState::Blocked)),
    );
}

/// A cargo the enclosing run started has no capture of its own --
/// the shim declines to open a second one inside a run it is
/// already capturing. The enclosing one is still its own reading:
/// the wait it prints about is mirrored into that log because it is
/// the process doing the waiting.
#[test]
fn a_nested_invocation_reads_the_run_it_is_inside() {
    let root = tests::capture_root(&[(
        64431,
        "    Blocking waiting for file lock on build directory",
    )]);
    let census = tests::census_of(&[(76847, 64432), (64432, 64431)]);
    let capture = Capture::take_from(root.path(), |pid| {
        KernelObservation::for_test(pid, Observation::Unknown)
    });

    assert_eq!(
        tests::captured_state(&census, &capture, 76847),
        CaptureLookup::Registered(CaptureRead::Progress(RunState::Blocked)),
    );
    assert_eq!(
        tests::captured_state(&census, &capture, 64432),
        CaptureLookup::Registered(CaptureRead::Progress(RunState::Blocked)),
        "the lead still reads its own shim",
    );
}

#[test]
fn a_live_compiler_keeps_its_first_owner_after_owner_and_census_disappear() {
    let pid = std::process::id();
    let LifetimeEvidence::Available(lifetime) = birth_stamp::lifetime(pid) else {
        panic!("current process must have a native lifetime");
    };
    let compiler = ProcessIdentity::Known { pid, lifetime };
    let owner = InvocationId::for_test(1);
    let mut smoothing = InvocationCpuAccounting::default();
    smoothing
        .cache_owners
        .insert(compiler.clone(), owner.clone());
    let mut census = tests::census_of(&[]);
    census.attribute(
        &ProcessObservations::default(),
        &mut smoothing,
        Instant::now(),
    );
    assert_eq!(smoothing.cache_owners.get(&compiler), Some(&owner));
    census
        .lifetimes
        .insert(Pid::from_u32(pid), LifetimeEvidence::Unavailable);
    census.attribute(
        &ProcessObservations::default(),
        &mut smoothing,
        Instant::now(),
    );
    assert_eq!(smoothing.cache_owners.get(&compiler), Some(&owner));
}

#[test]
fn native_replacement_retires_the_prior_compilers_owner() {
    let mut census = tests::census_of(&[]);
    let compiler = ProcessIdentity::Known {
        pid:      123,
        lifetime: ProcessLifetime::for_test(1),
    };
    let mut smoothing = InvocationCpuAccounting::default();
    smoothing
        .cache_owners
        .insert(compiler.clone(), InvocationId::for_test(1));
    census.lifetimes.insert(
        Pid::from_u32(123),
        LifetimeEvidence::Available(ProcessLifetime::for_test(2)),
    );
    census.attribute(
        &ProcessObservations::default(),
        &mut smoothing,
        Instant::now(),
    );
    assert!(!smoothing.cache_owners.contains_key(&compiler));
}

#[test]
fn root_cargo_is_read_before_the_compiler_it_can_reap() {
    let root = Pid::from_u32(ROOT_PROCESS_PID);
    let child = Pid::from_u32(ROOT_PROCESS_PID + 1);
    let mut census = tests::census_of(&[(child.as_u32(), root.as_u32()), (root.as_u32(), 0)]);
    census.cargo.push(root);
    for pid in [root, child] {
        census.lifetimes.insert(
            pid,
            LifetimeEvidence::Available(ProcessLifetime::for_test(u64::from(pid.as_u32()))),
        );
        census.accumulated.insert(pid, Duration::ZERO);
    }
    let mut order = Vec::new();
    let mut work = HashMap::from([(root, InvocationCpuContributions::default())]);
    census.collect_cpu_work(&HashMap::new(), &mut work, &mut HashMap::new(), |pid| {
        // Reading the child first lets a subsequent wait repeat its time in root.
        let elapsed = if pid == child || order.contains(&child) {
            100
        } else {
            0
        };
        order.push(pid);
        Measurement::Reading(Duration::from_millis(elapsed))
    });
    assert_eq!(order, [root, child]);
    assert_eq!(
        work[&root]
            .tree
            .values()
            .map(|counter| counter.elapsed)
            .sum::<Duration>(),
        Duration::from_millis(100)
    );
}

/// An idle owner keeps the rate macOS last computed for it, yet its sound
/// counter still lets its invocation publish a reading.
#[test]
fn an_owner_with_a_retained_rate_still_reads_its_invocation() {
    let owner = Pid::from_u32(ROOT_PROCESS_PID + 1);
    let mut census = tests::census_of(&[(owner.as_u32(), ROOT_PROCESS_PID)]);
    census.cargo.push(owner);
    census.lifetimes.insert(
        owner,
        LifetimeEvidence::Available(ProcessLifetime::for_test(u64::from(owner.as_u32()))),
    );
    census.accumulated.insert(owner, Duration::from_millis(10));
    let mut work = HashMap::from([(owner, InvocationCpuContributions::default())]);
    let mut evidence = HashMap::from([(
        owner,
        Measurement::Unavailable(MeasurementAbsence::RetainedRate),
    )]);
    census.collect_cpu_work(&HashMap::new(), &mut work, &mut evidence, |_| {
        Measurement::Reading(Duration::from_millis(10))
    });
    assert_eq!(evidence[&owner], Measurement::Reading(0.0));
}

#[test]
fn proven_registration_identity_changes_preserve_the_same_process_cpu_baseline() {
    let root = tempdir().expect("root");
    tests::write_versioned_capture(
        root.path(),
        10,
        "generation",
        "/writer/project",
        "/writer",
        "build",
        "",
    );
    let capture = tests::verified_capture(root.path());
    let pid = Pid::from_u32(20);
    let mut census = tests::census_of(&[(pid.as_u32(), 10)]);
    census.cargo.push(pid);
    census.lifetimes.insert(
        pid,
        LifetimeEvidence::Available(ProcessLifetime::for_test(20)),
    );
    census.accumulated.insert(pid, Duration::ZERO);
    census.cpu.insert(
        pid,
        Measurement::Unavailable(MeasurementAbsence::FirstObservation),
    );
    let mut smoothing = InvocationCpuAccounting::default();
    let now = Instant::now();
    let first =
        census.attribute_cpu_with(&ProcessObservations::default(), &mut smoothing, now, |_| {
            Measurement::Reading(Duration::from_millis(100))
        });
    assert_eq!(
        first[&pid],
        Measurement::Unavailable(MeasurementAbsence::FirstObservation)
    );
    census.cpu.insert(pid, Measurement::Reading(0.0));
    census.identify_captures(&capture);
    assert!(matches!(census.identities[&pid], InvocationId::Captured(_)));
    let recovered = census.attribute_cpu_with(
        &ProcessObservations::default(),
        &mut smoothing,
        now + Duration::from_secs(1),
        |_| Measurement::Reading(Duration::from_millis(200)),
    );
    assert_eq!(recovered[&pid], Measurement::Reading(10.0));
    census
        .identities
        .insert(pid, InvocationId::for_test(pid.as_u32()));
    let unconfirmed = census.attribute_cpu_with(
        &ProcessObservations::default(),
        &mut smoothing,
        now + Duration::from_secs(2),
        |_| Measurement::Reading(Duration::from_millis(300)),
    );
    assert_eq!(unconfirmed[&pid], Measurement::Reading(10.0));
    assert_eq!(smoothing.invocations.len(), 1);
}

#[test]
fn a_late_cpu_scan_keeps_the_configured_smoothing_window() {
    let pid = Pid::from_u32(100);
    let mut census = tests::census_of(&[(pid.as_u32(), ROOT_PROCESS_PID)]);
    census.cargo.push(pid);
    census.lifetimes.insert(
        pid,
        LifetimeEvidence::Available(ProcessLifetime::for_test(100)),
    );
    census.identities.insert(pid, InvocationId::for_test(100));
    census.accumulated.insert(pid, Duration::ZERO);
    census.cpu.insert(pid, Measurement::Reading(0.0));
    let mut accounting = InvocationCpuAccounting::new(CensusCadence::for_test());
    let now = Instant::now();
    for (elapsed, accumulated) in [(0, 0), (50, 50), (1050, 110)] {
        let measured = census.attribute_with(
            &ProcessObservations::default(),
            &mut accounting,
            now + Duration::from_millis(elapsed),
            |_| Measurement::Reading(Duration::from_millis(accumulated)),
        );
        if elapsed == 1050 {
            let Measurement::Reading(cpu) = measured.cpu[&pid] else {
                panic!("the same live process has three CPU counter samples");
            };
            assert!(
                (cpu - 6.0).abs() < 0.01,
                "one second exceeds ten test smoothing windows: {cpu}%"
            );
        }
    }
}

#[test]
fn non_cargo_argv_does_not_hide_a_detached_compiler_beneath_a_cargo_name() {
    let root = tempdir().expect("compiler metadata fixture");
    let [requester, server, worker, compiler] = [100, 101, 102, 103].map(Pid::from_u32);
    let arguments = [
        vec![
            OsString::from(CARGO_DISPLAY_NAME),
            "build".into(),
            CARGO_TARGET_DIR_FLAG.into(),
            root.path().as_os_str().into(),
        ],
        vec![SCCACHE_BINARY.into()],
        vec!["cache-worker".into()],
        vec![
            RUSTC_BINARY.into(),
            RUSTC_OUT_DIR_FLAG.into(),
            root.path().as_os_str().into(),
        ],
    ];
    let mut system = ProcessObservations::new(
        [
            (requester, ROOT_PROCESS_PID, CARGO_DISPLAY_NAME),
            (server, ROOT_PROCESS_PID, SCCACHE_BINARY),
            (worker, server.as_u32(), CARGO_DISPLAY_NAME),
            (compiler, worker.as_u32(), RUSTC_BINARY),
        ]
        .into_iter()
        .zip(&arguments)
        .map(|((pid, parent, name), argv)| {
            let mut record = ProcessObservation::cargo(pid.as_u32(), argv);
            record.parent = ProcessField::Observed(Pid::from_u32(parent));
            record.name = ProcessField::Observed(OsStr::new(name));
            record
        }),
    );
    let mut census = Census::take(system.records.values());
    // Native name and argv disagree; ancestry crosses the intermediate worker.
    assert!(
        census.cargo.contains(&worker),
        "native cargo name must enter discovery"
    );
    let process = system.process(worker).expect("worker metadata");
    assert_eq!(
        command_text::select_cargo(process.cmd(), &[]),
        Err(RowAbsence::ProgramRejected)
    );
    census.identities = ProcessIdentities::default().observe(&census.lifetimes);
    census.select_rows(&system, &Capture::default(), &[]);
    assert_eq!(census.live_cargo().collect::<Vec<_>>(), [requester]);
    let targets = census.observe_targets(&system, &HashMap::new(), &mut HashMap::new());
    assert_eq!(
        census.detached_compilers(&system, &targets),
        HashMap::from([(compiler, requester)])
    );

    census.cpu.insert(requester, Measurement::Reading(0.0));
    let mut smoothing = InvocationCpuAccounting::default();
    let now = Instant::now();
    for (offset, milliseconds, expected) in [
        (
            0,
            100,
            Measurement::Unavailable(MeasurementAbsence::FirstObservation),
        ),
        (1, 200, Measurement::Reading(10.0)),
    ] {
        let measured = census.attribute_cpu_with(
            &system,
            &mut smoothing,
            now + Duration::from_secs(offset),
            |pid| {
                Measurement::Reading(if pid == compiler {
                    Duration::from_millis(milliseconds)
                } else {
                    Duration::ZERO
                })
            },
        );
        assert_eq!(measured[&requester], expected);
    }
    for uid in [ProcessField::Unavailable, ProcessField::Observed(1001)] {
        system
            .records
            .get_mut(&compiler)
            .expect("compiler observation")
            .uid = uid;
        assert!(
            census.detached_compilers(&system, &targets).is_empty(),
            "unobserved or different users cannot share target ownership"
        );
    }
}

#[test]
fn collection_names_the_first_observation_before_publishing_a_rate() {
    assert_eq!(
        Census::measure_cpu(Pid::from(1), 0.0, &tests::cpu_baseline(10), &HashMap::new()),
        Measurement::Unavailable(MeasurementAbsence::FirstObservation)
    );
}

#[test]
fn collection_keeps_a_regressing_counter_unproven() {
    let pid = Pid::from(1);
    let previous = HashMap::from([(pid, tests::cpu_baseline(10))]);
    for cpu in [0.0, 50.0] {
        assert_eq!(
            Census::measure_cpu(pid, cpu, &tests::cpu_baseline(0), &previous),
            Measurement::Unavailable(MeasurementAbsence::Unproven)
        );
    }
}

#[test]
fn collection_keeps_consecutive_failed_reads_and_recovery_unproven() {
    let pid = Pid::from(1);
    let mut previous = HashMap::from([(pid, tests::cpu_baseline(10))]);
    // Failed task-info reads keep the old rate. The first successful read
    // after them also keeps it because sysinfo's previous counter was zero.
    for accumulated in [0, 0, 0, 20] {
        let baseline = tests::cpu_baseline(accumulated);
        assert_eq!(
            Census::measure_cpu(pid, 0.4, &baseline, &previous),
            Measurement::Unavailable(MeasurementAbsence::Unproven)
        );
        previous.insert(pid, baseline);
    }
    assert_eq!(
        Census::measure_cpu(pid, 0.5, &tests::cpu_baseline(30), &previous),
        Measurement::Reading(0.5)
    );
}

#[test]
fn collection_waits_for_a_positive_baseline_after_an_initial_failed_read() {
    let pid = Pid::from(1);
    let mut previous = HashMap::new();
    let initial = tests::cpu_baseline(0);
    assert_eq!(
        Census::measure_cpu(pid, 0.0, &initial, &previous),
        Measurement::Unavailable(MeasurementAbsence::FirstObservation)
    );
    previous.insert(pid, initial);
    let recovered = tests::cpu_baseline(20);
    assert_eq!(
        Census::measure_cpu(pid, 0.0, &recovered, &previous),
        Measurement::Unavailable(MeasurementAbsence::Unproven)
    );
    previous.insert(pid, recovered.clone());
    assert_eq!(
        Census::measure_cpu(pid, 0.0, &recovered, &previous),
        Measurement::Reading(0.0)
    );
}

#[test]
fn collection_preserves_a_measured_zero_with_nonzero_accumulated_time() {
    let pid = Pid::from(1);
    let previous = HashMap::from([(pid, tests::cpu_baseline(10))]);
    let measured = Census::measure_cpu(pid, 0.0, &tests::cpu_baseline(10), &previous);
    assert_eq!(measured, Measurement::Reading(0.0));
    assert_eq!(measured.map(resource_use::cpu_label).to_string(), "0%");
}

#[test]
fn collection_names_a_retained_rate_when_cpu_time_is_unchanged() {
    let pid = Pid::from(1);
    let previous = HashMap::from([(pid, tests::cpu_baseline(10))]);
    let measured = Census::measure_cpu(pid, 80.0, &tests::cpu_baseline(10), &previous);
    assert_eq!(
        measured,
        Measurement::Unavailable(MeasurementAbsence::RetainedRate)
    );
    assert_eq!(
        measured.map(resource_use::cpu_label).to_string(),
        UNAVAILABLE_MEASUREMENT
    );
}

#[test]
fn readable_quantized_zero_is_unproven_instead_of_failed() {
    let pid = Pid::from(1);
    let previous = HashMap::from([(pid, tests::cpu_baseline(0))]);
    assert_eq!(
        Census::measure_cpu(pid, 0.0, &tests::cpu_baseline(0), &previous),
        Measurement::Unavailable(MeasurementAbsence::Unproven)
    );
    // A nonzero rate cannot establish a fresh computation from a zero baseline.
    assert_eq!(
        Census::measure_cpu(pid, 0.5, &tests::cpu_baseline(0), &previous),
        Measurement::Unavailable(MeasurementAbsence::Unproven)
    );
}

#[test]
fn collection_rejects_invalid_rates() {
    let pid = Pid::from(1);
    let previous = HashMap::from([(pid, tests::cpu_baseline(10))]);
    for cpu in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -1.0] {
        assert_eq!(
            Census::measure_cpu(pid, cpu, &tests::cpu_baseline(10), &previous),
            Measurement::Unavailable(MeasurementAbsence::ReadFailed)
        );
    }
}

#[test]
fn a_reused_pid_starts_a_new_cpu_observation() {
    let pid = Pid::from(1);
    let previous = HashMap::from([(pid, tests::cpu_baseline(10))]);
    let replacement = CpuBaseline {
        lifetime:    LifetimeEvidence::Available(birth_stamp::ProcessLifetime::for_test(2)),
        accumulated: 0,
    };
    assert_eq!(
        Census::measure_cpu(pid, 0.0, &replacement, &previous),
        Measurement::Unavailable(MeasurementAbsence::FirstObservation)
    );
}

#[test]
fn a_counter_regression_with_unchanged_native_lifetime_remains_unproven() {
    let pid = Pid::from(1);
    let previous = HashMap::from([(pid, tests::cpu_baseline(10))]);
    let replacement = tests::cpu_baseline(5);
    assert_eq!(
        Census::measure_cpu(pid, 0.0, &replacement, &previous),
        Measurement::Unavailable(MeasurementAbsence::Unproven)
    );
}

#[test]
fn sequence_retains_cpu_and_identity_but_replaces_each_process_snapshot() {
    let argv = [OsString::from("cargo"), OsString::from("build")];
    let mut sequence = CensusSequence::default();
    let now = Instant::now();
    let snapshots = [(10, 100), (10, 300), (20, 500)];
    let samples: Vec<_> = snapshots
        .into_iter()
        .enumerate()
        .map(|(index, (birth, millis))| {
            let mut record = ProcessObservation::cargo(100, &argv);
            record.lifetime = Cow::Owned(LifetimeEvidence::Available(ProcessLifetime::for_test(
                birth,
            )));
            record.accumulated = millis;
            record
                .native_cpu
                .set(Measurement::Reading(Duration::from_millis(millis)));
            sequence.sample_counters(
                &mut ProcessObservations::new([record]),
                now + Duration::from_secs(u64::try_from(index).expect("sample index")),
            )
        })
        .collect();
    assert!(
        samples
            .iter()
            .all(|groups| groups.len() == 1 && groups[0].rest.is_empty())
    );
    assert_eq!(
        samples[0][0].lead.resource_use.cpu,
        Measurement::Unavailable(MeasurementAbsence::FirstObservation)
    );
    assert_eq!(
        samples[1][0].lead.resource_use.cpu,
        Measurement::Reading(20.0)
    );
    assert_eq!(samples[0][0].id(), samples[1][0].id());
    assert_ne!(samples[1][0].id(), samples[2][0].id());
    assert_eq!(
        samples[2][0].lead.resource_use.cpu,
        Measurement::Unavailable(MeasurementAbsence::FirstObservation)
    );
    assert_eq!(
        sequence.sample_counters(
            &mut ProcessObservations::default(),
            now + Duration::from_secs(3)
        ),
        [] as [CargoGroup; 0]
    );
}

#[test]
fn census_keeps_first_observations_instead_of_dropping_zero_samples() {
    let pid = Pid::from_u32(std::process::id());
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[pid]),
        true,
        refresh::process_discovery_refresh_kind(),
    );
    let discovery: Vec<_> = system
        .processes()
        .values()
        .map(|process| (process, CpuBaseline::from(process)))
        .collect();
    let observations = ProcessObservations::from_discovery(&discovery, &HashMap::new());
    let census = Census::take(observations.records.values());
    assert_eq!(
        census.cpu.get(&pid),
        Some(&Measurement::Unavailable(
            MeasurementAbsence::FirstObservation
        ))
    );
}

#[test]
fn cargo_ancestry_preserves_unavailable_parent_evidence() {
    let census = tests::census_of(&[(2, 1)]);
    assert_eq!(
        census.owning_cargo(Pid::from_u32(2)),
        CargoAncestry::ParentUnavailable
    );
}

#[test]
fn cargo_ancestry_preserves_a_reached_walk_limit() {
    let census = tests::census_of(&[(2, 3), (3, 2)]);
    assert_eq!(
        census.owning_cargo(Pid::from_u32(2)),
        CargoAncestry::WalkLimitReached
    );
}

#[test]
fn cargo_ancestry_identifies_an_observed_owner() {
    let mut census = tests::census_of(&[(3, 2), (2, 1)]);
    census.cargo.push(Pid::from_u32(1));
    assert_eq!(
        census.owning_cargo(Pid::from_u32(3)),
        CargoAncestry::Owner(Pid::from_u32(1))
    );
}
