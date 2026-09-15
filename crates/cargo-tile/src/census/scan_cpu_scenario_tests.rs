// Reader scenarios sampled without subprocess timing or terminal polling.

use super::*;

#[test]
fn cpu_cache_identity_recovery_preserves_detached_credit_and_retained_target() {
    let root = tempdir().expect("capture and target root");
    let target = root.path().join("target");
    fs::create_dir_all(target.join("debug/deps")).expect("compiler output");
    let initial_argv = target_arguments("build", &target);
    let argv = [OsString::from("cargo"), "build".into()];
    let compiler_argv = compiler_arguments(&target);
    let server_argv = [OsString::from("sccache")];
    write_versioned_capture(
        root.path(),
        10,
        "recovered",
        "/work",
        "/writer",
        "build",
        "",
    );
    let registration = root.path().join(CAPTURE_LIVE_RUNS_DIR).join("10.recovered");
    let proven = fs::read(&registration).expect("proven registration");
    let mut fields: Vec<_> = proven.split(|byte| *byte == 0).collect();
    fields[3] = b"";
    fs::write(&registration, fields.join(&0)).expect("unproven registration");
    let mut sequence = CensusSequence::default();
    let now = Instant::now();
    let mut samples = Vec::new();
    let mut identities = Vec::new();
    for index in 0..16 {
        // Target argv and the requesting client disappear after the first scan.
        // Later ownership must come from the retained destination.
        let mut records = vec![
            cpu_record(
                100,
                10,
                "cargo",
                if index == 0 { &initial_argv } else { &argv },
                100,
            ),
            cpu_record(200, 1, "sccache", &server_argv, 100),
            cpu_record(201, 200, "rustc", &compiler_argv, 100 + index * 200),
        ];
        if index == 0 {
            records.push(cpu_record(110, 100, "sccache", &compiler_argv, 0));
        }
        if index == 8 {
            fs::write(&registration, &proven).expect("recover registration identity");
        }
        let capture = verified_capture(root.path());
        let groups = sequence.sample_counters_with_capture(
            &mut ProcessObservations::new(records),
            now + Duration::from_secs(index),
            &capture,
            &[],
        );
        assert_eq!(groups.len(), 1, "sample {index}");
        assert!(groups[0].rest.is_empty());
        assert_eq!(groups[0].lead.pid, 100);
        let identity = groups[0].id();
        assert_eq!(matches!(identity, InvocationId::Captured(_)), index >= 8);
        assert_eq!(sequence.smoothing.invocations.len(), 1);
        assert!(
            sequence.smoothing.targets[&identity].contains(&target.canonicalize().expect("target"))
        );
        let compiler = ProcessIdentity::Known {
            pid:      201,
            lifetime: ProcessLifetime::for_test(201),
        };
        assert_eq!(
            sequence.smoothing.cache_owners.get(&compiler),
            Some(&identity)
        );
        assert_eq!(
            sequence.smoothing.invocations[&identity].detached[&compiler].total,
            Duration::from_millis(100 + index * 200)
        );
        identities.push(identity);
        samples.push(groups);
    }
    assert!(
        identities[..8]
            .iter()
            .all(|identity| identity == &identities[0])
    );
    assert!(
        identities[8..]
            .iter()
            .all(|identity| identity == &identities[8])
    );
    assert_ne!(identities[0], identities[8]);
    assert_eq!(
        samples[0][0].lead.cpu,
        Measurement::Unavailable(MeasurementAbsence::FirstObservation)
    );
    for (index, groups) in samples.iter().enumerate().skip(1) {
        // Exact deltas detect both stranded credit and replay of accumulated history.
        assert_eq!(
            groups[0].lead.cpu,
            Measurement::Reading("20%".into()),
            "sample {index}"
        );
    }
}

#[test]
fn cpu_cache_ambiguous_shared_target_refuses_both_requesters_on_every_sample() {
    assert_shared_target_refusal("build", &[]);
}

#[test]
fn cpu_cache_excluded_requester_retains_artifacts_and_prevents_visible_credit() {
    assert_shared_target_refusal("check", &["check".into()]);
}

#[test]
fn cpu_turnover_preserves_every_sample_as_busy_and_idle_children_change() {
    let argv = [OsString::from("cargo"), "build".into()];
    let busy_argv = [OsString::from("rustc")];
    let idle_argv = [OsString::from("sleep")];
    let mut sequence = CensusSequence::default();
    let now = Instant::now();
    let poll = invocation_cpu_accounting::poll();
    let work = u64::try_from((poll / 5).as_millis()).expect("CPU work milliseconds");
    let mut busy_samples: HashMap<u32, Vec<(u32, u64)>> = HashMap::new();
    let mut idle_pids = HashSet::new();
    let mut samples = Vec::new();
    for index in 0..16_u32 {
        let completed = u64::from(index / 3);
        let busy_pid = 200 + index / 3;
        let busy_time = u64::from(index % 3 + 1) * work;
        let reaped = if cfg!(target_os = "linux") {
            completed * 3 * work
        } else {
            0
        };
        let idle_pid = 300 + index;
        busy_samples
            .entry(busy_pid)
            .or_default()
            .push((index, busy_time));
        idle_pids.insert(idle_pid);
        samples.push(sequence.sample_counters(
            &mut ProcessObservations::new([
                cpu_record(100, 1, "cargo", &argv, 100 + reaped),
                cpu_record(busy_pid, 100, "rustc", &busy_argv, busy_time),
                cpu_record(idle_pid, 100, "sleep", &idle_argv, 0),
            ]),
            now + poll * index,
        ));
    }
    assert!(idle_pids.len() >= 10);
    let completed: Vec<_> = busy_samples
        .values()
        .filter(|samples| samples.len() == 3)
        .collect();
    assert!(completed.len() >= 3);
    for samples in completed {
        assert!(samples[2].0 >= samples[0].0 + 2);
        assert!(samples[2].1 >= work);
    }
    for (index, groups) in samples.iter().enumerate() {
        assert_eq!(groups.len(), 1, "sample {index}");
        assert!(groups[0].rest.is_empty());
        assert_eq!(groups[0].lead.pid, 100);
        assert_eq!(groups[0].id(), samples[0][0].id());
        let expected = if index == 0 {
            Measurement::Unavailable(MeasurementAbsence::FirstObservation)
        } else {
            Measurement::Reading("20%".into())
        };
        assert_eq!(groups[0].lead.cpu, expected, "sample {index}");
    }
}

#[test]
fn fallback_nested_source_switch_keeps_children_commands_measurements_and_headings() {
    let root = tempdir().expect("capture root");
    write_versioned_capture(
        root.path(),
        10,
        "carrier",
        "/writer/project",
        "/writer",
        "build",
        "Blocking waiting for file lock on build directory\n",
    );
    let parent_argv = [
        OsString::from("cargo"),
        "build".into(),
        "probe-carrier".into(),
    ];
    set_capture_arguments(
        &root.path().join(CAPTURE_LIVE_RUNS_DIR).join("10.carrier"),
        &parent_argv[1..],
    );
    let capture = verified_capture(root.path());
    assert_eq!(capture.confirmed().len(), 1);
    let check_argv = [
        OsString::from("cargo"),
        "check".into(),
        "probe-nested-check".into(),
    ];
    let test_argv = [
        OsString::from("cargo"),
        "test".into(),
        "probe-nested-test".into(),
    ];
    let mut sequence = CensusSequence::default();
    let mut samples = Vec::new();
    for source in ["registration", "process", "registration"] {
        let mut parent = cpu_record(10, 1, "cargo", &parent_argv, 100);
        parent.directory = ProcessField::Observed(Path::new("/writer/project"));
        parent.cpu = Measurement::Reading(42.0);
        if source == "registration" {
            parent.name = ProcessField::Observed(OsStr::new("python3"));
            parent.argv = ProcessField::Unavailable;
        }
        let mut check = cpu_record(20, 10, "cargo", &check_argv, 100);
        let mut test = cpu_record(30, 10, "cargo", &test_argv, 100);
        for child in [&mut check, &mut test] {
            child.directory = ProcessField::Observed(Path::new("/writer/nested-carrier"));
            child.cpu = Measurement::Reading(0.0);
        }
        samples.push(sequence.sample_with_capture(
            &ProcessObservations::new([parent, check, test]),
            &capture,
            &[],
            ScannerHome::Known(Path::new("/writer")),
        ));
    }
    assert_nested_source_samples(&samples, &capture);
}

fn cpu_record<'scan>(
    pid: u32,
    parent: u32,
    name: &'scan str,
    argv: &'scan [OsString],
    millis: u64,
) -> ProcessObservation<'scan> {
    let mut record = ProcessObservation::cargo(pid, argv);
    record.parent = ProcessField::Observed(Pid::from_u32(parent));
    record.name = ProcessField::Observed(OsStr::new(name));
    record.accumulated = millis;
    record
        .native_cpu
        .set(Measurement::Reading(Duration::from_millis(millis)));
    record
}

fn target_arguments(command: &str, target: &Path) -> Vec<OsString> {
    vec![
        "cargo".into(),
        command.into(),
        "--target-dir".into(),
        target.as_os_str().into(),
    ]
}

fn compiler_arguments(target: &Path) -> Vec<OsString> {
    vec![
        "rustc".into(),
        "--out-dir".into(),
        target.join("debug/deps").into_os_string(),
    ]
}

fn set_capture_arguments(registration: &Path, arguments: &[OsString]) {
    let bytes = fs::read(registration).expect("registration fields");
    let mut fields: Vec<Vec<u8>> = bytes
        .split(|byte| *byte == 0)
        .take(8)
        .map(<[u8]>::to_vec)
        .collect();
    fields[7] = arguments.len().to_string().into_bytes();
    fields.extend(
        arguments
            .iter()
            .map(|argument| argument.as_bytes().to_vec()),
    );
    fields.push(Vec::new());
    fs::write(registration, fields.join(&0)).expect("registration arguments");
}

fn assert_shared_target_refusal(command: &str, excluded: &[String]) {
    let root = tempdir().expect("captures and shared target");
    let target = root.path().join("target");
    fs::create_dir_all(target.join("debug/deps")).expect("compiler output");
    let first_argv = target_arguments("build", &target);
    let other_argv = target_arguments(command, &target);
    let compiler_argv = compiler_arguments(&target);
    let server_argv = [OsString::from("sccache")];
    write_versioned_capture(
        root.path(),
        30,
        "excluded",
        "/work",
        "/writer",
        command,
        "Blocking waiting for file lock on build directory\n",
    );
    let registration = root.path().join(CAPTURE_LIVE_RUNS_DIR).join("30.excluded");
    let log = root.path().join("run-excluded-30.log");
    set_capture_arguments(&registration, &other_argv[1..]);
    let original_registration = fs::read(&registration).expect("registration bytes");
    let original_log = fs::read(&log).expect("log bytes");
    let mut sequence = CensusSequence::default();
    let now = Instant::now();
    for index in 0..16 {
        let capture = verified_capture(root.path());
        assert_eq!(capture.confirmed().len(), 1);
        let mut records = ProcessObservations::new([
            cpu_record(100, 1, "cargo", &first_argv, 100),
            cpu_record(300, 30, "cargo", &other_argv, 100),
            cpu_record(110, 100, "sccache", &compiler_argv, 0),
            cpu_record(310, 300, "sccache", &compiler_argv, 0),
            cpu_record(200, 1, "sccache", &server_argv, 100),
            cpu_record(201, 200, "rustc", &compiler_argv, 100 + index * 800),
        ]);
        assert_eq!(
            records
                .process(Pid::from_u32(201))
                .expect("detached compiler")
                .parent,
            ProcessField::Observed(Pid::from_u32(200))
        );
        let groups = sequence.sample_counters_with_capture(
            &mut records,
            now + Duration::from_secs(index),
            &capture,
            excluded,
        );
        assert_eq!(
            groups.len(),
            if excluded.is_empty() { 2 } else { 1 },
            "sample {index}"
        );
        let mut lead_pids: Vec<_> = groups.iter().map(|group| group.lead.pid).collect();
        lead_pids.sort_unstable();
        let expected_pids: &[u32] = if excluded.is_empty() {
            &[100, 300]
        } else {
            &[100]
        };
        assert_eq!(lead_pids, expected_pids, "sample {index}");
        for group in &groups {
            assert!(group.rest.is_empty());
            assert!(group.lead.pid == 100 || (excluded.is_empty() && group.lead.pid == 300));
            let expected = if index == 0 {
                Measurement::Unavailable(MeasurementAbsence::FirstObservation)
            } else {
                Measurement::Reading("0%".into())
            };
            assert_eq!(
                group.lead.cpu, expected,
                "sample {index}, pid {}",
                group.lead.pid
            );
        }
        assert!(sequence.smoothing.cache_owners.is_empty());
        assert_eq!(
            sequence.smoothing.owners.len(),
            2,
            "excluded requester remains an owner"
        );
        assert_eq!(
            fs::read(&registration).expect("retained registration"),
            original_registration
        );
        assert_eq!(fs::read(&log).expect("retained log"), original_log);
    }
}

fn assert_nested_source_samples(samples: &[Vec<CargoGroup>], capture: &Capture) {
    let mut roster = crate::roster::Roster::new();
    let mut headings = Vec::new();
    for (index, groups) in samples.iter().enumerate() {
        assert_eq!(groups.len(), 1);
        let group = &groups[0];
        assert_eq!(group.id(), samples[0][0].id());
        assert_eq!(group.lead.pid, 10);
        assert_eq!(
            group.lead.command,
            CommandText::of("cargo", &["build", "probe-carrier"])
        );
        assert_eq!(group.lead.managed, Measurement::Reading(2));
        assert_eq!(
            group.lead.state,
            CaptureLookup::Registered(CaptureRead::Progress(RunState::Blocked))
        );
        assert_eq!(group.rest.len(), 2);
        for (pid, command, marker) in [
            (20, "check", "probe-nested-check"),
            (30, "test", "probe-nested-test"),
        ] {
            let row = group
                .rest
                .iter()
                .find(|row| row.pid == pid)
                .expect("nested invocation");
            assert_eq!(row.command, CommandText::of("cargo", &[command, marker]));
            assert_eq!(row.path, "~/nested-carrier");
            assert_eq!(
                row.parent,
                VisibleParent::Invocation {
                    id:  group.id(),
                    pid: 10,
                }
            );
            assert_eq!(
                row.state,
                CaptureLookup::Registered(CaptureRead::Progress(RunState::Blocked))
            );
        }
        if index == 1 {
            assert_eq!(group.lead.cpu, Measurement::Reading("42%".into()));
            assert_eq!(group.lead.compiler, CompilerObservation::None);
        } else {
            assert_eq!(
                group.lead.cpu,
                Measurement::Unavailable(MeasurementAbsence::Unproven)
            );
            assert_eq!(group.lead.compiler, CompilerObservation::Unknown);
        }
        roster.observe(groups.clone(), Instant::now());
        assert_eq!(roster.groups().len(), 1);
        assert_eq!(roster.groups()[0].rows().count(), 3);
        headings.push(assert_nested_render(&roster, group, capture, index));
    }
    assert!(headings.iter().all(|heading| heading == &headings[0]));
}

fn assert_nested_render(
    roster: &crate::roster::Roster,
    group: &CargoGroup,
    capture: &Capture,
    index: usize,
) -> Vec<String> {
    let account = match &capture.root_status[0].account {
        AccountName::Resolved(name) => name.clone(),
        AccountName::Unavailable => capture.root_status[0].root.uid.to_string(),
    };
    let area = ratatui::layout::Rect::new(0, 0, 200, 20);
    let mut buffer = ratatui::buffer::Buffer::empty(area);
    crate::render::draw_cell_for_test(
        &mut buffer,
        roster,
        &crate::tiles::TileContent::Group(group.id()),
        area,
        4,
    );
    let lines: Vec<String> = (0..area.height)
        .map(|y| (0..area.width).map(|x| buffer[(x, y)].symbol()).collect())
        .collect();
    let text = lines.join("\n");
    for command in [
        "cargo build probe-carrier",
        "cargo check probe-nested-check",
        "cargo test probe-nested-test",
    ] {
        assert_eq!(text.matches(command).count(), 1, "{text}");
    }
    let expected_headings = [
        format!("[{account}] ~/project"),
        format!("[{account}] ~/nested-carrier"),
    ];
    for heading in &expected_headings {
        assert_eq!(text.matches(heading).count(), 1, "{text}");
    }

    let parent = lines
        .iter()
        .find(|line| line.contains("cargo build probe-carrier"))
        .expect("parent row");
    let cells: Vec<_> = parent
        .split(['│', ' '])
        .filter(|cell| !cell.is_empty())
        .collect();
    assert_eq!(cells[0], "10");
    assert_eq!(cells.last(), Some(&"2"));
    assert_eq!(
        cells.iter().filter(|&&cell| cell == "--").count(),
        if index == 1 { 0 } else { 2 }
    );
    for (pid, marker) in [("20", "probe-nested-check"), ("30", "probe-nested-test")] {
        let row = lines
            .iter()
            .find(|line| line.contains(marker))
            .expect("child row");
        let cells: Vec<_> = row
            .split(['│', ' '])
            .filter(|cell| !cell.is_empty())
            .collect();
        assert_eq!(&cells[..2], &[pid, "10"], "{text}");
    }
    lines
        .iter()
        .filter(|line| {
            expected_headings
                .iter()
                .any(|heading| line.contains(heading))
        })
        .map(|line| line.trim().to_owned())
        .collect()
}
