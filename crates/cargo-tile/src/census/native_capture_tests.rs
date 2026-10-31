//! The native capture transport and its worker are one displayed invocation.

use super::*;
use crate::constants::CAPTURE_RUNNER_NAME;
use crate::constants::CAPTURE_SHIM_ARGUMENT;
use crate::constants::CAPTURE_SHIM_PASSTHROUGH_ARGUMENT;
use crate::render;
use crate::roster::Roster;

/// The installed v4 entry passes the cargo path and capture parent before cargo's argv.
struct NativeRun {
    argv: [Vec<OsString>; 5],
}

impl NativeRun {
    fn new() -> Self {
        Self {
            argv: [
                vec![
                    Path::new("/toolchain")
                        .join(CAPTURE_RUNNER_NAME)
                        .into_os_string(),
                    CAPTURE_SHIM_ARGUMENT.into(),
                    "/toolchain/cargo".into(),
                    "/tmp/cargo-tile".into(),
                    "nextest".into(),
                    "run".into(),
                    "--workspace".into(),
                ],
                ["/tools/cargo-nextest", "nextest", "run", "--workspace"]
                    .map(OsString::from)
                    .to_vec(),
                ["cargo", "test", "--no-run"].map(OsString::from).to_vec(),
                ["rustc", "--crate-name", "fixture"]
                    .map(OsString::from)
                    .to_vec(),
                ["tee", "-a", "/capture/run.log"]
                    .map(OsString::from)
                    .to_vec(),
            ],
        }
    }

    fn records(&self, sample: u64) -> ProcessObservations<'_> {
        ProcessObservations::new(
            [
                (100, 1, "cargo-tile-capture", 4, 0),
                (200, 100, "cargo-nextest", 16, 500),
                (300, 200, "cargo", 32, 750),
                (400, 300, "rustc", 512, 250),
                (500, 100, "tee", 1, 0),
            ]
            .into_iter()
            .zip(&self.argv)
            .map(|((pid, parent, name, mib, work), argv)| {
                let mut record = ProcessObservation::cargo(pid, argv);
                record.name = ProcessField::Observed(OsStr::new(name));
                record.parent = ProcessField::Observed(Pid::from_u32(parent));
                record.memory = mib * 1024 * 1024;
                record.accumulated = 100 + sample * work;
                record
                    .native_cpu
                    .set(Measurement::Reading(Duration::from_millis(
                        record.accumulated,
                    )));
                record
            }),
        )
    }

    fn sample(&self, capture: &Capture) -> Vec<CargoGroup> {
        let mut sequence = CensusSequence::default();
        let now = Instant::now();
        let mut groups = Vec::new();
        for sample in 0..13 {
            groups = sequence.sample_counters_with_capture(
                &mut self.records(sample),
                now + Duration::from_secs(sample),
                capture,
                &[],
            );
        }
        groups
    }
}

fn assert_native_run(groups: &[CargoGroup]) {
    assert_eq!(groups.len(), 1, "one captured command must form one cell");
    let group = &groups[0];
    assert_eq!(
        group.lead.pid, 100,
        "the native runner is the invocation identity"
    );
    assert_eq!(
        group.lead.command,
        CommandText::of("cargo", &["nextest", "run", "--workspace"])
    );
    assert_eq!(
        group.rest.iter().map(|row| row.pid).collect::<Vec<_>>(),
        [300]
    );
    assert_eq!(
        group.lead.resource_use.cpu.map(cpu_label),
        Measurement::Reading("50%".into())
    );
    assert_eq!(
        group.lead.resource_use.memory,
        Measurement::Reading(21 * 1024 * 1024)
    );
    assert_eq!(
        group.rest[0].resource_use.cpu.map(cpu_label),
        Measurement::Reading("100%".into())
    );
    assert_eq!(
        group.rest[0].resource_use.memory,
        Measurement::Reading(544 * 1024 * 1024)
    );
    assert_eq!(
        group.lead.subtree_resource_use.cpu.map(cpu_label),
        Measurement::Reading("150%".into())
    );
    assert_eq!(
        group.lead.subtree_resource_use.memory,
        Measurement::Reading(565 * 1024 * 1024)
    );
    let mut roster = Roster::new();
    roster.observe(groups.to_vec(), Instant::now());
    assert_eq!(roster.tiled_ids(&[]), [group.id()]);
    assert_eq!(
        render::summary_cpu_for_test(&roster, &[]),
        [(100, Measurement::Reading("150%".into()))]
    );
    let area = ratatui::layout::Rect::new(0, 0, 180, 16);
    let mut buffer = ratatui::buffer::Buffer::empty(area);
    render::draw_cell_for_test(&mut buffer, &roster, &TileContent::Summary, area, 4);
    let lines: Vec<String> = (0..area.height)
        .map(|y| (0..area.width).map(|x| buffer[(x, y)].symbol()).collect())
        .collect();
    let text = lines.join("\n");
    let header = lines
        .iter()
        .find(|line| line.contains("cpu") && line.contains("command"))
        .expect("summary header");
    let rows: Vec<_> = lines
        .iter()
        .filter(|line| line.split_whitespace().next() == Some("100"))
        .collect();
    assert_eq!(rows.len(), 1, "{text}");
    assert!(rows[0].contains("cargo nextest run --workspace"), "{text}");
    assert_eq!(
        crate::census::measurement_cell(header, rows[0], "cpu", &text),
        "150%"
    );
    assert_eq!(
        crate::census::measurement_cell(header, rows[0], "mem", &text),
        "0.6G"
    );
    assert!(!text.contains("cargo --workspace"), "{text}");
    let details = render_group_lines(group).join("\n");
    assert!(
        details.contains("cargo nextest run --workspace"),
        "{details}"
    );
    assert!(details.contains("cargo test --no-run"), "{details}");
}

#[test]
fn a_native_capture_runner_heads_one_command_with_its_workers_resources() {
    assert_native_run(&NativeRun::new().sample(&Capture::default()));
}

#[test]
fn a_registered_native_capture_runner_and_its_worker_share_one_row() {
    let root = tempdir().expect("capture root");
    write_versioned_capture(
        root.path(),
        100,
        "native",
        "/work",
        "/writer",
        "nextest",
        "",
    );
    let path = root.path().join(CAPTURE_LIVE_RUNS_DIR).join("100.native");
    let bytes = fs::read(&path).expect("registration");
    let mut fields: Vec<_> = bytes.split(|byte| *byte == 0).collect();
    fields[7] = b"3";
    fields.pop();
    fields.extend([b"run".as_slice(), b"--workspace", b""]);
    fs::write(path, fields.join(&0)).expect("complete command arguments");
    let capture = verified_capture(root.path());
    let groups = NativeRun::new().sample(&capture);
    assert_native_run(&groups);
    assert!(
        matches!(&groups[0].lead.invocation_id, InvocationId::Captured(run) if run.shim_pid == 100)
    );
    assert!(matches!(
        groups[0].lead.provenance,
        RowProvenance::Direct(_)
    ));
}

/// Check retained invocation rows and the summary through the production census.
fn assert_separate_native_invocations(argv: &[Vec<OsString>], names: &[&str], expected: &[u32]) {
    let mut sequence = CensusSequence::default();
    let now = Instant::now();
    let mut groups = Vec::new();
    for sample in 0..13 {
        let mut records =
            ProcessObservations::new(argv.iter().zip(names).zip([100, 200, 300, 400]).map(
                |((argv, name), pid)| {
                    let mut record = ProcessObservation::cargo(pid, argv);
                    record.name = ProcessField::Observed(OsStr::new(name));
                    record.parent = ProcessField::Observed(Pid::from_u32(if pid == 100 {
                        1
                    } else {
                        pid - 100
                    }));
                    record.memory = 1024 * 1024;
                    record.accumulated = 100 + sample * 500;
                    record
                        .native_cpu
                        .set(Measurement::Reading(Duration::from_millis(
                            record.accumulated,
                        )));
                    record
                },
            ));
        groups = sequence.sample_counters_with_capture(
            &mut records,
            now + Duration::from_secs(sample),
            &Capture::default(),
            &[],
        );
    }
    let rows: Vec<_> = groups
        .iter()
        .flat_map(|group| std::iter::once(&group.lead).chain(&group.rest))
        .collect();
    assert_eq!(rows.iter().map(|row| row.pid).collect::<Vec<_>>(), expected);
    for row in &rows {
        assert_eq!(row.command, CommandText::of("cargo", &["run"]));
    }
    let mut roster = Roster::new();
    roster.observe(groups.clone(), now);
    assert_eq!(
        render::summary_cpu_for_test(&roster, &[]),
        [(100, Measurement::Reading(format!("{}%", names.len() * 50)))]
    );
    assert_eq!(
        groups[0].lead.subtree_resource_use.memory,
        Measurement::Reading(names.len() as u64 * 1024 * 1024)
    );
    let area = ratatui::layout::Rect::new(0, 0, 180, 16);
    let mut buffer = ratatui::buffer::Buffer::empty(area);
    render::draw_cell_for_test(&mut buffer, &roster, &TileContent::Summary, area, 4);
    let text: String = (0..area.height)
        .map(|y| {
            (0..area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(text.matches("cargo run").count(), 1, "{text}");
    let details = render_group_lines(&groups[0]).join("\n");
    assert_eq!(
        details.matches("cargo run").count(),
        expected.len(),
        "{details}"
    );
}

#[test]
fn an_application_breaks_the_native_capture_forwarding_chain() {
    let run = NativeRun::new();
    let mut runner = run.argv[0][..4].to_vec();
    runner.push("run".into());
    let cargo = vec!["cargo".into(), "run".into()];
    assert_separate_native_invocations(
        &[runner, cargo.clone(), vec!["application".into()], cargo],
        &["cargo-tile-capture", "cargo", "application", "cargo"],
        &[100, 400],
    );
}

#[test]
fn a_nested_native_capture_runner_keeps_its_own_invocation() {
    let run = NativeRun::new();
    let mut runner = run.argv[0][..4].to_vec();
    runner.push("run".into());
    assert_separate_native_invocations(
        &[runner.clone(), vec!["cargo".into(), "run".into()], runner],
        &["cargo-tile-capture", "cargo", "cargo-tile-capture"],
        &[100, 300],
    );
}

#[test]
fn native_capture_protocol_words_define_the_transport_boundary() {
    for flag in [CAPTURE_SHIM_ARGUMENT, CAPTURE_SHIM_PASSTHROUGH_ARGUMENT] {
        let mut argv = vec![
            Path::new("/toolchain")
                .join(CAPTURE_RUNNER_NAME)
                .into_os_string(),
            flag.into(),
            "/toolchain/cargo".into(),
            "/tmp/cargo-tile".into(),
            "run".into(),
        ];
        assert_eq!(
            CargoTransport::from_argv(&argv),
            CargoTransport::NativeCapture {
                arguments: &argv[4..],
            }
        );
        assert_eq!(
            command_text::cargo_split(&argv).map(|arguments| arguments.start),
            Ok(4)
        );
        argv[1] = "--unrelated-entry".into();
        assert_eq!(CargoTransport::from_argv(&argv), CargoTransport::Ordinary);
        argv[1] = flag.into();
        argv[0] = "/toolchain/unrelated-runner".into();
        assert_eq!(CargoTransport::from_argv(&argv), CargoTransport::Ordinary);
    }
}
