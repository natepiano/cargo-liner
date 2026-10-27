/// Verified process subjects for one forwarding execution.
struct ForwardingExecution {
    parent: u32,
    shim: u32,
    cargo: u32,
}

impl ForwardingExecution {
    fn observations<'a>(&self, argv: &'a [OsString]) -> [ProcessObservation<'a>; 3] {
        let make = |pid| ProcessObservation::cargo(pid, argv).with_lifetime(
            LifetimeEvidence::Available(ProcessLifetime::matching_registration_for_test(&subject_birth(pid))),
        );
        let parent = make(self.parent);
        let mut shim = make(self.shim);
        shim.parent = ProcessField::Observed(Pid::from_u32(self.parent));
        let mut cargo = make(self.cargo);
        cargo.parent = ProcessField::Observed(Pid::from_u32(self.shim));
        [parent, shim, cargo]
    }
}

fn execution_records<'a>(executions: &[ForwardingExecution], argv: &'a [OsString],
    short: &'a [OsString], include_job: bool) -> ProcessObservations<'a> {
    let mut records: Vec<_> = executions.iter().flat_map(|execution| execution.observations(argv)).collect();
    if executions.len() > 1 {
        // A distinct cargo child supplies a parent value in the command table.
        // The forwarding pids above it still represent just one execution.
        let execution = &executions[0];
        let pid = execution.cargo + 100;
        let mut child = ProcessObservation::cargo(pid, short).with_lifetime(
            LifetimeEvidence::Available(ProcessLifetime::matching_registration_for_test(&subject_birth(pid))),
        );
        child.parent = ProcessField::Observed(Pid::from_u32(execution.cargo));
        records.push(child);
    }
    if include_job {
        records.push(ProcessObservation::cargo(6102, short).with_lifetime(
            LifetimeEvidence::Available(ProcessLifetime::matching_registration_for_test(&subject_birth(6102))),
        ));
    }
    ProcessObservations::new(records)
}

fn subject_birth(pid: u32) -> BirthStamp {
    let IdentityEvidence::Available(birth) = BirthStamp::from_fields("boot", &pid.to_string()) else {
        panic!("fixture process birth");
    };
    birth
}

fn registered_executions(executions: &[ForwardingExecution]) -> (tempfile::TempDir, Capture) {
    let root = tempfile::tempdir().expect("registration fixture");
    fs::create_dir_all(root.path().join("state/pids")).expect("registration directory");
    let magic = std::str::from_utf8(REGISTRATION_MAGIC).expect("registration magic");
    for execution in executions {
        fs::write(
            root.path().join(format!("state/pids/{}.generation", execution.shim)),
            format!("{magic}\0generation\0boot\0{}\0run.log\0/work\0/writer\x001\0check\0", execution.shim),
        ).expect("verified registration");
    }
    let capture = Capture::take_from(root.path(), |pid| {
        KernelObservation::for_test(pid, Observation::Present(subject_birth(pid)))
    });
    (root, capture)
}

/// The result of one frame condition at the assertion boundary.
#[derive(Clone, Copy)]
enum FrameCheck {
    Passed,
    Failed,
}

impl From<bool> for FrameCheck {
    fn from(passed: bool) -> Self {
        if passed { Self::Passed } else { Self::Failed }
    }
}

/// Frame failures retain the first transition that violated the execution contract.
#[derive(Default)]
struct ExecutionFrameChecks {
    failures: Vec<String>,
}

impl ExecutionFrameChecks {
    fn require(&mut self, result: FrameCheck, transition: &str, detail: impl std::fmt::Display) {
        if matches!(result, FrameCheck::Failed) {
            self.failures.push(format!("{transition}: {detail}"));
        }
    }

    fn check_frame(
        &mut self,
        buffer: &Buffer,
        grid: &TileGrid<InvocationId>,
        sequence: &CensusSequence,
        owners: &[InvocationId],
        executions: &[ForwardingExecution],
        transition: &str,
    ) {
        use crate::census::scan::PublishedInvocation;
        let pieces = grid.painted_placements_for_test();
        for (owner, execution) in owners.iter().zip(executions) {
            let groups: Vec<_> = sequence.publication().groups().iter().filter(|group|
                std::iter::once(&group.lead).chain(&group.rest).any(|row|
                    [execution.parent, execution.shim, execution.cargo].contains(&row.pid)
                )
            ).collect();
            self.require(FrameCheck::from(groups.len() == 1), transition,
                format!("execution {} has {} owner records", execution.cargo, groups.len()));
            let state = sequence.publication().state(owner);
            let state_name = match state {
                Some(PublishedInvocation::Live(_)) => "Live",
                Some(PublishedInvocation::Missing(_)) => "Missing",
                Some(PublishedInvocation::Ended(_)) => "Ended",
                Some(PublishedInvocation::AwaitingCargo(_)) => "AwaitingCargo",
                Some(PublishedInvocation::Joined { .. }) => "Joined",
                None => "removed",
            };
            self.require(FrameCheck::from(matches!(state,
                Some(PublishedInvocation::Live(_) | PublishedInvocation::Missing(_)))), transition,
                format!("matching native executor {} changed from Live to {state_name}", execution.cargo));
            let cells: Vec<_> = pieces.iter().filter(|piece|
                matches!(piece.content, TileContent::Group(_)) &&
                [execution.parent, execution.shim, execution.cargo].iter().any(|pid|
                    drawn_process_in(buffer, piece, *pid))
            ).collect();
            self.require(FrameCheck::from(cells.len() == 1), transition,
                format!("execution {} has {} painted cells", execution.cargo, cells.len()));
            self.require(FrameCheck::from(cells.iter().all(|piece| piece.content == TileContent::Group(owner.clone()))),
                transition, format!("execution {} changed its assigned cell", execution.cargo));
            self.require(FrameCheck::from(cells.iter().map(|piece| process_row_count(buffer, piece, execution.cargo)).sum::<usize>() == 1),
                transition, format!("cargo pid {} must begin one cell process row", execution.cargo));
            self.require(FrameCheck::from(cells.iter().all(|piece|
                !drawn_process_in(buffer, piece, execution.parent) && !drawn_process_in(buffer, piece, execution.shim))),
                transition, format!("forwarder {} or shim {} became a lead row", execution.parent, execution.shim));
        }
    }

    fn finish(self) {
        assert!(self.failures.is_empty(), "first wrong transition: {}\nall frame failures: {:?}",
            self.failures.first().map_or("none", String::as_str), self.failures);
    }
}

/// A retained source cannot end a proven live executor or create another parent cell.
#[test]
fn a_forwarding_parent_refresh_never_opens_a_second_cell() {
    forwarding_frames(false);
}

/// An end and a source change replace the header layout of the existing cells.
#[test]
fn a_header_form_change_at_an_end_keeps_one_cell_per_invocation() {
    forwarding_frames(true);
}

fn forwarding_frames(change_form: bool) {
    let appearance = FixtureAppearance::dark();
    let elapsed = Instant::now();
    let executions = if change_form {
        vec![ForwardingExecution { parent: 4100, shim: 4101, cargo: 4102 },
             ForwardingExecution { parent: 5100, shim: 5101, cargo: 5102 }]
    } else {
        vec![ForwardingExecution { parent: 4100, shim: 4101, cargo: 4102 }]
    };
    let (root, capture) = registered_executions(&executions);
    let epoch = Instant::now() + Duration::from_secs(60);
    let fade = Duration::from_secs(3);
    let area = Rect::new(0, 0, 40, 120);
    let short = ["cargo", "check"].map(OsString::from);
    let initial = ["cargo", "build"].map(OsString::from);
    let wide = ["cargo", "long-command-form-change"].map(OsString::from);
    let initial = if change_form { &initial } else { &short };
    let mut sequence = CensusSequence::default();
    let mut roster = Roster::new();
    let mut grid = live_test_grid();
    grid.set_motion_elapsed_for_test(Duration::ZERO);
    sequence.sample_published(&ProcessObservations::default(), &capture, epoch);
    let owners: Vec<_> = executions.iter().map(|execution|
        sequence.publication().groups().iter().find(|group| group.lead.pid == execution.shim)
            .expect("registered owner").id()
    ).collect();
    // Native reads name their actual subjects; no owner birth is used for another pid.
    let read = |pid: u32| if pid == 6102 { Observation::Ended } else { Observation::Present(subject_birth(pid)) };
    sequence.sample_with_native_evidence(&execution_records(&executions, initial, &short, change_form), &capture, epoch,
        crate::census::scan::FixturePublication::Current, read);
    roster.observe_invocations(sequence.publication());
    foreign_load_frame(&mut roster, &mut grid, area, fade);
    grid.elapse_for_test(Duration::from_millis(800));
    roster.advance(epoch + Duration::from_millis(800), fade);
    let (before, _) = foreign_load_frame(&mut roster, &mut grid, area, fade);
    let mut checks = ExecutionFrameChecks::default();
    checks.check_frame(&before, &grid, &sequence, &owners, &executions, "initial cargo proof -> Celled");
    let initial_forms = command_header_forms(&before, &grid);
    let initial_ink: Vec<_> = executions.iter().map(|execution|
        process_ink(&before, &grid, execution.cargo)
    ).collect();
    let short_owners: Vec<_> = sequence.publication().groups().iter()
        .filter(|group| group.lead.pid == 6102).map(crate::census::scan::CargoGroup::id).collect();
    if change_form {
        assert_eq!(short_owners.len(), 1, "short job must have one initial owner");
        assert_eq!(process_cells(&before, &grid, 6102), 1, "short job must paint before its end");
        checks.require(FrameCheck::from(initial_forms.iter().any(|parent| *parent)), "initial header", "fixture did not paint a parent header");
    }
    // Retain a source snapshot with no selected process rows. The matching executor
    // remains alive; disappearance of its registration is not an end proof.
    sequence.sample_with_native_evidence(&execution_records(&executions, initial, &short, false), &Capture::default(),
        epoch + Duration::from_millis(1000), crate::census::scan::FixturePublication::Retained, read);
    roster.observe_invocations(sequence.publication());
    roster.advance(epoch + Duration::from_millis(1000), fade);
    grid.elapse_for_test(Duration::from_millis(200));
    let (retained, _) = foreign_load_frame(&mut roster, &mut grid, area, fade);
    checks.check_frame(&retained, &grid, &sequence, &owners, &executions, "retained source -> native lifetime probe");
    if change_form {
        checks.require(FrameCheck::from(matches!(sequence.publication().state(&short_owners[0]),
            Some(crate::census::scan::PublishedInvocation::Ended(_)))),
            "short job native end proof", "short job did not end on its first missing publication");
    }
    let mut saw_short_departure = false;
    for millis in [1020, 1060, 1100, 1140, 1180, 1220, 1260, 1300, 4200] {
        let argv = if change_form { &wide } else { &short };
        sequence.sample_with_native_evidence(&execution_records(&executions, argv, &short, false), &Capture::default(),
            epoch + Duration::from_millis(millis), crate::census::scan::FixturePublication::Current, read);
        roster.observe_invocations(sequence.publication());
        roster.advance(epoch + Duration::from_millis(millis), fade);
        grid.elapse_for_test(Duration::from_millis(if millis == 4200 { 2900 } else { 40 }));
        let (buffer, _) = foreign_load_frame(&mut roster, &mut grid, area, fade);
        let transition = format!("current native source at {millis}ms");
        checks.check_frame(&buffer, &grid, &sequence, &owners, &executions, &transition);
        for (execution, ink) in executions.iter().zip(&initial_ink) {
            checks.require(FrameCheck::from(process_ink(&buffer, &grid, execution.cargo) + 0.05 >= *ink),
                &transition, format!("cargo {} restarted its entry ink", execution.cargo));
        }
        if change_form {
            let short_cells = process_cells(&buffer, &grid, 6102);
            checks.require(FrameCheck::from(short_cells <= 1), &transition,
                format!("ended short job has {short_cells} cells"));
            saw_short_departure |= short_cells == 1;
            if millis == 4200 {
                checks.require(FrameCheck::from(short_cells == 0), &transition, "short job reentered after its departure");
            }
            let forms = command_header_forms(&buffer, &grid);
            checks.require(FrameCheck::from(forms.windows(2).all(|pair| pair[0] == pair[1])), &transition,
                format!("mixed command header forms: {forms:?}"));
            checks.require(FrameCheck::from(forms.iter().any(|parent| !*parent)), &transition,
                "fixture did not select the compact command header");
        }
    }
    checks.require(FrameCheck::from(!change_form || saw_short_departure), "short job departure",
        "fixture did not paint the ended job within its fade");
    checks.require(FrameCheck::from(elapsed.elapsed() < Duration::from_secs(1)), "fixture time", elapsed.elapsed().as_secs_f64());
    drop(appearance);
    checks.finish();
    drop(root);
}

fn process_cells(buffer: &Buffer, grid: &TileGrid<InvocationId>, pid: u32) -> usize {
    grid.painted_placements_for_test().iter().filter(|piece|
        matches!(piece.content, TileContent::Group(_)) && drawn_process_in(buffer, piece, pid)
    ).count()
}

fn process_ink(buffer: &Buffer, grid: &TileGrid<InvocationId>, pid: u32) -> f64 {
    let prefix = format!("{pid} ");
    grid.painted_placements_for_test().iter()
        .filter(|piece| matches!(piece.content, TileContent::Group(_)))
        .flat_map(|piece| {
            let clip = piece.frame.clip().intersection(buffer.area);
            (clip.y..clip.bottom()).filter_map(|y| {
                let row: String = (clip.x..clip.right()).map(|x| buffer[(x, y)].symbol()).collect();
                row.trim_matches(|character: char| character == '│' || character.is_whitespace())
                    .starts_with(&prefix).then(|| row_contrast(buffer, Rect::new(clip.x, y, clip.width, 1)))
            }).collect::<Vec<_>>()
        }).fold(0.0, f64::max)
}

fn command_header_forms(buffer: &Buffer, grid: &TileGrid<InvocationId>) -> Vec<bool> {
    grid.painted_placements_for_test().iter().filter(|piece| matches!(piece.content, TileContent::Group(_)))
        .flat_map(|piece| {
            let clip = piece.frame.clip().intersection(buffer.area);
            (clip.y..clip.bottom()).filter_map(move |y| {
                let line = buffer_line(buffer, y);
                (line.contains("pid") && line.contains("command")).then(|| line.contains("parent"))
            })
        }).collect()
}


/// Cargo proof admits every job while its CPU rate still awaits a first sample.
#[test]
fn cargo_proof_admits_all_jobs_before_their_first_cpu_sample() {
    let appearance = FixtureAppearance::dark();
    let elapsed = Instant::now();
    let executions: Vec<_> = (0..3).map(|index| ForwardingExecution {
        parent: 7100 + index * 10, shim: 7101 + index * 10, cargo: 7102 + index * 10,
    }).collect();
    let (root, capture) = registered_executions(&executions);
    let args = ["cargo", "check"].map(OsString::from);
    let epoch = Instant::now() + Duration::from_secs(60);
    let fade = Duration::from_secs(3);
    let mut sequence = CensusSequence::default();
    let mut roster = Roster::new();
    let mut grid = live_test_grid();
    grid.set_motion_elapsed_for_test(Duration::ZERO);
    sequence.sample_published(&ProcessObservations::default(), &capture, epoch);
    let owners: Vec<_> = executions.iter().map(|execution|
        sequence.publication().groups().iter().find(|group| group.lead.pid == execution.shim)
            .expect("registered owner").id()
    ).collect();
    let mut exclusive_failures = Vec::new();
    let mut previous_ink = vec![0.0; executions.len()];
    for (index, measurement) in [Measurement::Unavailable(MeasurementAbsence::FirstObservation),
                               Measurement::Reading(0.0), Measurement::Reading(12.5)].into_iter().enumerate() {
        let now = epoch + Duration::from_millis(u64::try_from(index).expect("sample index") * 20);
        let records = ProcessObservations::new(executions.iter().flat_map(|execution|
            execution.observations(&args).into_iter().enumerate().map(move |(position, row)| {
                let cpu = if position == 2 || index == 0 { measurement } else { Measurement::Reading(0.0) };
                row.with_cpu_for_test(cpu)
            })));
        sequence.sample_published(&records, &capture, now);
        roster.observe_invocations(sequence.publication());
        roster.advance(now, fade);
        let (buffer, spans) = foreign_load_frame(&mut roster, &mut grid, Rect::new(0, 0, 80, 48), fade);
        let pieces = grid.painted_placements_for_test();
        for ((owner, execution), previous) in owners.iter().zip(&executions).zip(&mut previous_ink) {
            let group = roster.groups().iter().find(|group| &group.id == owner).expect("same owner after sample");
            assert_eq!(group.lead.process.resource_use.cpu, measurement, "CPU state changed before rendering");
            let cell = pieces.iter().find(|piece| piece.content == TileContent::Group(owner.clone()))
                .expect("cargo proof must admit before the first CPU sample");
            assert!(drawn_process_in(&buffer, cell, execution.cargo), "first paint omitted the cargo pid");
            assert!(!drawn_process_in(&buffer, cell, execution.shim), "first paint used the shim pid");
            assert!(!drawn_process_in(&buffer, cell, execution.parent), "first paint used the parent pid");
            let ink = process_ink(&buffer, &grid, execution.cargo);
            assert!(ink + 0.05 >= *previous, "CPU sampling restarted the entry ink for {}", execution.cargo);
            *previous = ink;
            let sites: std::collections::HashSet<_> = spans.iter().filter(|span| &span.id == owner)
                .map(|span| format!("{:?}", span.content)).collect();
            if sites.len() != 1 {
                exclusive_failures.push(format!("sample {index}, cargo {}: {sites:?}", execution.cargo));
            }
        }
        grid.elapse_for_test(Duration::from_millis(20));
    }
    assert!(elapsed.elapsed() < Duration::from_secs(1), "CPU fixture took {:?}", elapsed.elapsed());
    drop(appearance);
    assert!(exclusive_failures.is_empty(), "first wrong transition: AwaitingFirstSample cargo proof -> two paint sites: {exclusive_failures:?}");
    drop(root);
}

/// With free rows, only the missing cargo proof can keep a registered job listed.
#[test]
fn a_registered_jobs_listed_wait_ends_on_the_cargo_discovery_frame() {
    let appearance = FixtureAppearance::dark();
    let elapsed = Instant::now();
    let execution = ForwardingExecution { parent: 8100, shim: 8101, cargo: 8102 };
    let (root, capture) = registered_executions(std::slice::from_ref(&execution));
    let epoch = Instant::now() + Duration::from_secs(60);
    let args = ["cargo", "check"].map(OsString::from);
    let fade = Duration::from_secs(3);
    let area = Rect::new(0, 0, 80, 32);
    let mut sequence = CensusSequence::default();
    let mut roster = Roster::new();
    let mut grid = live_test_grid();
    grid.set_motion_elapsed_for_test(Duration::ZERO);
    let mut first_cell = Vec::new();
    for millis in [0, 250, 500, 750] {
        let records = if millis == 750 {
            ProcessObservations::new(execution.observations(&args))
        } else {
            ProcessObservations::default()
        };
        sequence.sample_published(&records, &capture, epoch + Duration::from_millis(millis));
        roster.observe_invocations(sequence.publication());
        let (buffer, rows) = foreign_load_frame(&mut roster, &mut grid, area, fade);
        let cell = grid.painted_placements_for_test().iter().find(|piece|
            matches!(piece.content, TileContent::Group(_)) && drawn_process_in(&buffer, piece, execution.cargo));
        if cell.is_some() {
            first_cell.push(millis);
        } else {
            assert!(millis < 750, "cargo proof did not admit on its discovery frame");
            assert!(roster.groups()[0].lead.admissible().is_err(), "free rows wait despite cargo proof");
            assert!(rows.iter().any(|row| row.content == TileContent::Summary), "listed registration has no row");
            assert!(grid.painted_placements_for_test().iter().all(|piece| piece.content == TileContent::Summary),
                "unproven registration opened a cell");
        }
        grid.elapse_for_test(Duration::from_millis(250));
    }
    drop(root);
    assert_eq!(first_cell, [750], "Listed-to-Celled must occur on cargo discovery, without an extra frame");
    assert!(elapsed.elapsed() < Duration::from_secs(1), "admission fixture took {:?}", elapsed.elapsed());
    drop(appearance);
}

/// Count process starts, excluding wrapped continuation rows and ancestor headings.
fn process_row_count(buffer: &Buffer, piece: &TilePlacement<InvocationId>, pid: u32) -> usize {
    let clip = piece.frame.clip().intersection(buffer.area);
    let prefix = format!("{pid} ");
    (clip.y.saturating_add(1)..clip.bottom().saturating_sub(1)).filter(|&y| {
        let line: String = (clip.x.saturating_add(1)..clip.right().saturating_sub(1))
            .map(|x| buffer[(x, y)].symbol()).collect();
        line.trim_start().starts_with(&prefix)
    }).count()
}
