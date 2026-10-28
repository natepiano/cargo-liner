use std::collections::HashSet;
use std::fmt::Display;
use std::iter;
use std::slice;
use std::str;

use tempfile::TempDir;

use super::*;
use crate::census::scan::CargoGroup;
use crate::census::scan::FixturePublication;
use crate::census::scan::PublishedInvocation;

/// Measured publication, draw and execution-check work for a source handoff.
const FORWARDING_WORK_LIMIT: FixtureWork = FixtureWork {
    census_passes:    12,
    draws:            12,
    frames:           12,
    invariant_checks: 11,
};

#[derive(Clone, Copy)]
enum HeaderChange {
    Unchanged,
    AtEnd,
}

#[derive(Debug, PartialEq, Eq)]
enum HeaderParent {
    Absent,
    Present,
}

impl From<bool> for HeaderParent {
    fn from(present: bool) -> Self { if present { Self::Present } else { Self::Absent } }
}

#[derive(Clone, Copy)]
enum JobPresence {
    Absent,
    Present,
}

/// Verified process subjects for one forwarding execution.
struct ForwardingExecution {
    parent: u32,
    shim:   u32,
    cargo:  u32,
}

impl ForwardingExecution {
    fn observations<'a>(&self, argv: &'a [OsString]) -> [ProcessObservation<'a>; 3] {
        let make = |pid| {
            ProcessObservation::cargo(pid, argv).with_lifetime(LifetimeEvidence::Available(
                ProcessLifetime::matching_registration_for_test(&subject_birth(pid)),
            ))
        };
        let parent = make(self.parent);
        let mut shim = make(self.shim);
        shim.parent = ProcessField::Observed(Pid::from_u32(self.parent));
        let mut cargo = make(self.cargo);
        cargo.parent = ProcessField::Observed(Pid::from_u32(self.shim));
        [parent, shim, cargo]
    }
}

/// The result of one frame condition at the assertion boundary.
#[derive(Clone, Copy)]
enum FrameCheck {
    Passed,
    Failed,
}

impl From<bool> for FrameCheck {
    fn from(passed: bool) -> Self { if passed { Self::Passed } else { Self::Failed } }
}

/// Frame failures retain the first transition that violated the execution contract.
#[derive(Default)]
struct ExecutionFrameChecks {
    failures: Vec<String>,
}

impl ExecutionFrameChecks {
    fn require(&mut self, result: FrameCheck, transition: &str, detail: impl Display) {
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
        let pieces = grid.painted_placements_for_test();
        for (owner, execution) in owners.iter().zip(executions) {
            let groups: Vec<_> = sequence
                .publication()
                .groups()
                .iter()
                .filter(|group| {
                    iter::once(&group.lead).chain(&group.rest).any(|row| {
                        [execution.parent, execution.shim, execution.cargo].contains(&row.pid)
                    })
                })
                .collect();
            self.require(
                FrameCheck::from(groups.len() == 1),
                transition,
                format!(
                    "execution {} has {} owner records",
                    execution.cargo,
                    groups.len()
                ),
            );
            let state = sequence.publication().state(owner);
            let state_name = match state {
                Some(PublishedInvocation::Live(_)) => "Live",
                Some(PublishedInvocation::Missing(_)) => "Missing",
                Some(PublishedInvocation::Ended(_)) => "Ended",
                Some(PublishedInvocation::AwaitingCargo(_)) => "AwaitingCargo",
                Some(PublishedInvocation::Joined { .. }) => "Joined",
                None => "removed",
            };
            self.require(
                FrameCheck::from(matches!(
                    state,
                    Some(PublishedInvocation::Live(_) | PublishedInvocation::Missing(_))
                )),
                transition,
                format!(
                    "matching native executor {} changed from Live to {state_name}",
                    execution.cargo
                ),
            );
            let cells: Vec<_> = pieces
                .iter()
                .filter(|piece| {
                    matches!(piece.content, TileContent::Group(_))
                        && [execution.parent, execution.shim, execution.cargo]
                            .iter()
                            .any(|pid| drawn_process_in(buffer, piece, *pid))
                })
                .collect();
            self.require(
                FrameCheck::from(cells.len() == 1),
                transition,
                format!(
                    "execution {} has {} painted cells",
                    execution.cargo,
                    cells.len()
                ),
            );
            self.require(
                FrameCheck::from(
                    cells
                        .iter()
                        .all(|piece| piece.content == TileContent::Group(owner.clone())),
                ),
                transition,
                format!("execution {} changed its assigned cell", execution.cargo),
            );
            self.require(
                FrameCheck::from(
                    cells
                        .iter()
                        .map(|piece| process_row_count(buffer, piece, execution.cargo))
                        .sum::<usize>()
                        == 1,
                ),
                transition,
                format!(
                    "cargo pid {} must begin one cell process row",
                    execution.cargo
                ),
            );
            self.require(
                FrameCheck::from(cells.iter().all(|piece| {
                    !drawn_process_in(buffer, piece, execution.parent)
                        && !drawn_process_in(buffer, piece, execution.shim)
                })),
                transition,
                format!(
                    "forwarder {} or shim {} became a lead row",
                    execution.parent, execution.shim
                ),
            );
        }
    }

    fn finish(self) {
        assert!(
            self.failures.is_empty(),
            "first wrong transition: {}\nall frame failures: {:?}",
            self.failures.first().map_or("none", String::as_str),
            self.failures
        );
    }
}

struct ForwardingPaint {
    initial_ink:  Vec<f64>,
    short_owners: Vec<InvocationId>,
}

struct ForwardingFixture {
    header_change: HeaderChange,
    executions:    Vec<ForwardingExecution>,
    root:          TempDir,
    capture:       Capture,
    epoch:         Instant,
    fade:          Duration,
    end_millis:    u64,
    area:          Rect,
    short:         [OsString; 2],
    initial:       [OsString; 2],
    wide:          [OsString; 2],
    sequence:      CensusSequence,
    roster:        Roster,
    grid:          TileGrid<InvocationId>,
    work:          FixtureWork,
    checks:        ExecutionFrameChecks,
    owners:        Vec<InvocationId>,
}

impl ForwardingFixture {
    fn new(header_change: HeaderChange) -> Self {
        let mut work = FixtureWork::default();
        let executions = forwarding_executions(header_change);
        let (root, capture) = registered_executions(&executions);
        let epoch = Instant::now() + Duration::from_secs(60);
        let fade = Duration::from_secs(DEFAULT_FADE_SECONDS);
        let end_millis = 1000;
        let area = Rect::new(0, 0, 40, 120);
        let short = ["cargo", "check"].map(OsString::from);
        let initial = match header_change {
            HeaderChange::AtEnd => ["cargo", "build"],
            HeaderChange::Unchanged => ["cargo", "check"],
        }
        .map(OsString::from);
        let wide = ["cargo", "long-command-form-change"].map(OsString::from);
        let mut sequence = CensusSequence::default();
        let roster = Roster::new();
        let mut grid = live_test_grid();
        grid.set_motion_elapsed_for_test(Duration::ZERO);
        work.census(|| sequence.sample_published(&ProcessObservations::default(), &capture, epoch));
        let owners = registered_owners(&sequence, &executions);
        Self {
            header_change,
            executions,
            root,
            capture,
            epoch,
            fade,
            end_millis,
            area,
            short,
            initial,
            wide,
            sequence,
            roster,
            grid,
            work,
            owners,
            checks: ExecutionFrameChecks::default(),
        }
    }

    fn initial_frame(&mut self) -> ForwardingPaint {
        let header_change = self.header_change;
        let epoch = self.epoch;
        let fade = self.fade;
        let area = self.area;
        let Self {
            work,
            sequence,
            roster,
            grid,
            executions,
            owners,
            capture,
            initial,
            short,
            checks,
            ..
        } = self;
        work.census(|| {
            sequence.sample_with_native_evidence(
                &execution_records(
                    executions,
                    initial,
                    short,
                    match header_change {
                        HeaderChange::AtEnd => JobPresence::Present,
                        HeaderChange::Unchanged => JobPresence::Absent,
                    },
                ),
                capture,
                epoch,
                FixturePublication::Current,
                forwarding_lifetime,
            );
        });
        roster.observe_invocations(sequence.publication());
        work.draw(|| foreign_load_frame(roster, grid, area, fade));
        grid.elapse_for_test(Duration::from_millis(800));
        roster.advance(epoch + Duration::from_millis(800), fade);
        let (before, _) = work.draw(|| foreign_load_frame(roster, grid, area, fade));
        work.check(|| {
            checks.check_frame(
                &before,
                grid,
                sequence,
                owners,
                executions,
                "initial cargo proof -> Celled",
            );
        });
        let initial_forms = command_header_forms(&before, grid);
        let initial_ink: Vec<_> = executions
            .iter()
            .map(|execution| process_ink(&before, grid, execution.cargo))
            .collect();
        let short_owners: Vec<_> = sequence
            .publication()
            .groups()
            .iter()
            .filter(|group| group.lead.pid == 6102)
            .map(CargoGroup::id)
            .collect();
        if matches!(header_change, HeaderChange::AtEnd) {
            assert_eq!(
                short_owners.len(),
                1,
                "short job must have one initial owner"
            );
            assert_eq!(
                process_cells(&before, grid, 6102),
                1,
                "short job must paint before its end"
            );
            checks.require(
                FrameCheck::from(
                    initial_forms
                        .iter()
                        .any(|parent| matches!(parent, HeaderParent::Present)),
                ),
                "initial header",
                "fixture did not paint a parent header",
            );
        }
        ForwardingPaint {
            initial_ink,
            short_owners,
        }
    }

    fn retain_source(&mut self, paint: &ForwardingPaint) {
        let header_change = self.header_change;
        let epoch = self.epoch;
        let fade = self.fade;
        let area = self.area;
        let end_millis = self.end_millis;
        let Self {
            work,
            sequence,
            roster,
            grid,
            executions,
            owners,
            initial,
            short,
            checks,
            ..
        } = self;
        // Retain a source snapshot with no selected process rows. The matching executor
        // remains alive; disappearance of its registration is not an end proof.
        work.census(|| {
            sequence.sample_with_native_evidence(
                &execution_records(executions, initial, short, JobPresence::Absent),
                &Capture::default(),
                epoch + Duration::from_millis(end_millis),
                FixturePublication::Retained,
                forwarding_lifetime,
            );
        });
        roster.observe_invocations(sequence.publication());
        roster.advance(epoch + Duration::from_millis(end_millis), fade);
        grid.elapse_for_test(Duration::from_millis(200));
        let (retained, _) = work.draw(|| foreign_load_frame(roster, grid, area, fade));
        work.check(|| {
            checks.check_frame(
                &retained,
                grid,
                sequence,
                owners,
                executions,
                "retained source -> native lifetime probe",
            );
        });
        if matches!(header_change, HeaderChange::AtEnd) {
            checks.require(
                FrameCheck::from(matches!(
                    sequence.publication().state(&paint.short_owners[0]),
                    Some(PublishedInvocation::Ended(_))
                )),
                "short job native end proof",
                "short job did not end on its first missing publication",
            );
        }
    }

    fn current_sources(&mut self, paint: &ForwardingPaint) {
        let header_change = self.header_change;
        let epoch = self.epoch;
        let fade = self.fade;
        let area = self.area;
        let end_millis = self.end_millis;
        let Self {
            work,
            sequence,
            roster,
            grid,
            executions,
            owners,
            short,
            wide,
            checks,
            ..
        } = self;
        let fade_millis = u64::try_from(fade.as_millis()).expect("fixture fade fits milliseconds");
        let final_millis = end_millis + fade_millis + 200;
        let mut short_departure = Evidence::Absent;
        for millis in [1020, 1060, 1100, 1140, 1180, 1220, 1260, 1300, final_millis] {
            let argv = match header_change {
                HeaderChange::AtEnd => &*wide,
                HeaderChange::Unchanged => &*short,
            };
            work.census(|| {
                sequence.sample_with_native_evidence(
                    &execution_records(executions, argv, short, JobPresence::Absent),
                    &Capture::default(),
                    epoch + Duration::from_millis(millis),
                    FixturePublication::Current,
                    forwarding_lifetime,
                );
            });
            roster.observe_invocations(sequence.publication());
            roster.advance(epoch + Duration::from_millis(millis), fade);
            grid.elapse_for_test(Duration::from_millis(if millis == final_millis {
                final_millis - 1300
            } else {
                40
            }));
            let (buffer, _) = work.draw(|| foreign_load_frame(roster, grid, area, fade));
            let transition = format!("current native source at {millis}ms");
            work.check(|| {
                checks.check_frame(&buffer, grid, sequence, owners, executions, &transition);
            });
            for (execution, ink) in executions.iter().zip(&paint.initial_ink) {
                checks.require(
                    FrameCheck::from(process_ink(&buffer, grid, execution.cargo) + 0.05 >= *ink),
                    &transition,
                    format!("cargo {} restarted its entry ink", execution.cargo),
                );
            }
            if matches!(header_change, HeaderChange::AtEnd) {
                let short_cells = process_cells(&buffer, grid, 6102);
                checks.require(
                    FrameCheck::from(short_cells <= 1),
                    &transition,
                    format!("ended short job has {short_cells} cells"),
                );
                if short_cells == 1 {
                    short_departure = Evidence::Observed;
                }
                if millis == final_millis {
                    checks.require(
                        FrameCheck::from(short_cells == 0),
                        &transition,
                        "short job reentered after its departure",
                    );
                }
                let forms = command_header_forms(&buffer, grid);
                checks.require(
                    FrameCheck::from(forms.windows(2).all(|pair| pair[0] == pair[1])),
                    &transition,
                    format!("mixed command header forms: {forms:?}"),
                );
                checks.require(
                    FrameCheck::from(
                        forms
                            .iter()
                            .any(|parent| matches!(parent, HeaderParent::Absent)),
                    ),
                    &transition,
                    "fixture did not select the compact command header",
                );
            }
        }
        checks.require(
            FrameCheck::from(
                matches!(header_change, HeaderChange::Unchanged)
                    || matches!(short_departure, Evidence::Observed),
            ),
            "short job departure",
            "fixture did not paint the ended job within its fade",
        );
    }
}

fn execution_records<'a>(
    executions: &[ForwardingExecution],
    argv: &'a [OsString],
    short: &'a [OsString],
    job_presence: JobPresence,
) -> ProcessObservations<'a> {
    let mut records: Vec<_> = executions
        .iter()
        .flat_map(|execution| execution.observations(argv))
        .collect();
    if executions.len() > 1 {
        // A distinct cargo child supplies a parent value in the command table.
        // The forwarding pids above it still represent just one execution.
        let execution = &executions[0];
        let pid = execution.cargo + 100;
        let mut child =
            ProcessObservation::cargo(pid, short).with_lifetime(LifetimeEvidence::Available(
                ProcessLifetime::matching_registration_for_test(&subject_birth(pid)),
            ));
        child.parent = ProcessField::Observed(Pid::from_u32(execution.cargo));
        records.push(child);
    }
    if matches!(job_presence, JobPresence::Present) {
        records.push(ProcessObservation::cargo(6102, short).with_lifetime(
            LifetimeEvidence::Available(ProcessLifetime::matching_registration_for_test(
                &subject_birth(6102),
            )),
        ));
    }
    ProcessObservations::new(records)
}

fn subject_birth(pid: u32) -> BirthStamp {
    let IdentityEvidence::Available(birth) = BirthStamp::from_fields("boot", &pid.to_string())
    else {
        panic!("fixture process birth");
    };
    birth
}

fn registered_executions(executions: &[ForwardingExecution]) -> (TempDir, Capture) {
    let root = tempfile::tempdir().expect("registration fixture");
    fs::create_dir_all(root.path().join("state/pids")).expect("registration directory");
    let magic = str::from_utf8(REGISTRATION_MAGIC).expect("registration magic");
    for execution in executions {
        fs::write(
            root.path()
                .join(format!("state/pids/{}.generation", execution.shim)),
            format!(
                "{magic}\0generation\0boot\0{}\0run.log\0/work\0/writer\x001\0check\0",
                execution.shim
            ),
        )
        .expect("verified registration");
    }
    let capture = Capture::take_from(root.path(), |pid| {
        KernelObservation::for_test(pid, Observation::Present(subject_birth(pid)))
    });
    (root, capture)
}

fn forwarding_executions(header_change: HeaderChange) -> Vec<ForwardingExecution> {
    if matches!(header_change, HeaderChange::AtEnd) {
        vec![
            ForwardingExecution {
                parent: 4100,
                shim:   4101,
                cargo:  4102,
            },
            ForwardingExecution {
                parent: 5100,
                shim:   5101,
                cargo:  5102,
            },
        ]
    } else {
        vec![ForwardingExecution {
            parent: 4100,
            shim:   4101,
            cargo:  4102,
        }]
    }
}

// Native reads name their actual subjects; no owner birth is used for another pid.
fn forwarding_lifetime(pid: u32) -> Observation {
    if pid == 6102 {
        Observation::Ended
    } else {
        Observation::Present(subject_birth(pid))
    }
}

fn registered_owners(
    sequence: &CensusSequence,
    executions: &[ForwardingExecution],
) -> Vec<InvocationId> {
    executions
        .iter()
        .map(|execution| {
            sequence
                .publication()
                .groups()
                .iter()
                .find(|group| group.lead.pid == execution.shim)
                .expect("registered owner")
                .id()
        })
        .collect()
}

fn forwarding_frames(header_change: HeaderChange) {
    let appearance = FixtureAppearance::dark();
    let elapsed = Instant::now();
    let mut fixture = ForwardingFixture::new(header_change);
    let paint = fixture.initial_frame();
    fixture.retain_source(&paint);
    fixture.current_sources(&paint);
    fixture
        .work
        .assert_within(&FORWARDING_WORK_LIMIT, elapsed.elapsed());
    drop(appearance);
    fixture.checks.finish();
    drop(fixture.root);
}

fn process_cells(buffer: &Buffer, grid: &TileGrid<InvocationId>, pid: u32) -> usize {
    grid.painted_placements_for_test()
        .iter()
        .filter(|piece| {
            matches!(piece.content, TileContent::Group(_)) && drawn_process_in(buffer, piece, pid)
        })
        .count()
}

fn process_ink(buffer: &Buffer, grid: &TileGrid<InvocationId>, pid: u32) -> f64 {
    let prefix = format!("{pid} ");
    grid.painted_placements_for_test()
        .iter()
        .filter(|piece| matches!(piece.content, TileContent::Group(_)))
        .flat_map(|piece| {
            let clip = piece.frame.clip().intersection(buffer.area);
            (clip.y..clip.bottom())
                .filter_map(|y| {
                    let row: String = fixture_row_cells(buffer, clip, y)
                        .iter()
                        .map(Cell::symbol)
                        .collect();
                    row.trim_matches(|character: char| {
                        character == '│' || character.is_whitespace()
                    })
                    .starts_with(&prefix)
                    .then(|| row_contrast(buffer, Rect::new(clip.x, y, clip.width, 1)))
                })
                .collect::<Vec<_>>()
        })
        .fold(0.0, f64::max)
}

fn command_header_forms(buffer: &Buffer, grid: &TileGrid<InvocationId>) -> Vec<HeaderParent> {
    grid.painted_placements_for_test()
        .iter()
        .filter(|piece| matches!(piece.content, TileContent::Group(_)))
        .flat_map(|piece| {
            let clip = piece.frame.clip().intersection(buffer.area);
            (clip.y..clip.bottom()).filter_map(move |y| {
                let line = buffer_line(buffer, y);
                (line.contains("pid") && line.contains("command"))
                    .then(|| HeaderParent::from(line.contains("parent")))
            })
        })
        .collect()
}

/// Count process starts, excluding wrapped continuation rows and ancestor headings.
fn process_row_count(buffer: &Buffer, piece: &TilePlacement<InvocationId>, pid: u32) -> usize {
    let clip = piece.frame.clip().intersection(buffer.area);
    let prefix = format!("{pid} ");
    (clip.y.saturating_add(1)..clip.bottom().saturating_sub(1))
        .filter(|&y| {
            let inner = Rect::new(clip.x.saturating_add(1), y, clip.width.saturating_sub(2), 1);
            let line: String = fixture_row_cells(buffer, inner, y)
                .iter()
                .map(Cell::symbol)
                .collect();
            line.trim_start().starts_with(&prefix)
        })
        .count()
}

/// A retained source cannot end a proven live executor or create another parent cell.
#[test]
fn a_forwarding_parent_refresh_never_opens_a_second_cell() {
    forwarding_frames(HeaderChange::Unchanged);
}

/// An end and a source change replace the header layout of the existing cells.
#[test]
fn a_header_form_change_at_an_end_keeps_one_cell_per_invocation() {
    forwarding_frames(HeaderChange::AtEnd);
}

fn cpu_sample_records<'a>(
    executions: &[ForwardingExecution],
    args: &'a [OsString],
    measurement: Measurement<f32>,
    index: usize,
) -> ProcessObservations<'a> {
    ProcessObservations::new(executions.iter().flat_map(|execution| {
        execution
            .observations(args)
            .into_iter()
            .enumerate()
            .map(move |(position, row)| {
                let cpu = if position == 2 || index == 0 {
                    measurement
                } else {
                    Measurement::Reading(0.0)
                };
                row.with_cpu_for_test(cpu)
            })
    }))
}

/// Cargo proof admits every job while its CPU rate still awaits a first sample.
#[test]
fn cargo_proof_admits_all_jobs_before_their_first_cpu_sample() {
    let appearance = FixtureAppearance::dark();
    let elapsed = Instant::now();
    let executions: Vec<_> = (0..3)
        .map(|index| ForwardingExecution {
            parent: 7100 + index * 10,
            shim:   7101 + index * 10,
            cargo:  7102 + index * 10,
        })
        .collect();
    let (root, capture) = registered_executions(&executions);
    let args = ["cargo", "check"].map(OsString::from);
    let epoch = Instant::now() + Duration::from_secs(60);
    let fade = Duration::from_secs(DEFAULT_FADE_SECONDS);
    let mut sequence = CensusSequence::default();
    let mut roster = Roster::new();
    let mut grid = live_test_grid();
    grid.set_motion_elapsed_for_test(Duration::ZERO);
    sequence.sample_published(&ProcessObservations::default(), &capture, epoch);
    let owners = registered_owners(&sequence, &executions);
    let mut exclusive_failures = Vec::new();
    let mut previous_ink = vec![0.0; executions.len()];
    for (index, measurement) in [
        Measurement::Unavailable(MeasurementAbsence::FirstObservation),
        Measurement::Reading(0.0),
        Measurement::Reading(12.5),
    ]
    .into_iter()
    .enumerate()
    {
        let now = epoch + Duration::from_millis(u64::try_from(index).expect("sample index") * 20);
        let records = cpu_sample_records(&executions, &args, measurement, index);
        sequence.sample_published(&records, &capture, now);
        roster.observe_invocations(sequence.publication());
        roster.advance(now, fade);
        let (buffer, spans) =
            foreign_load_frame(&mut roster, &mut grid, Rect::new(0, 0, 80, 48), fade);
        let pieces = grid.painted_placements_for_test();
        for ((owner, execution), previous) in owners.iter().zip(&executions).zip(&mut previous_ink)
        {
            let group = roster
                .groups()
                .iter()
                .find(|group| &group.id == owner)
                .expect("same owner after sample");
            assert_eq!(
                group.lead.process.resource_use.cpu, measurement,
                "CPU state changed before rendering"
            );
            let cell = pieces
                .iter()
                .find(|piece| piece.content == TileContent::Group(owner.clone()))
                .expect("cargo proof must admit before the first CPU sample");
            assert!(
                drawn_process_in(&buffer, cell, execution.cargo),
                "first paint omitted the cargo pid"
            );
            assert!(
                !drawn_process_in(&buffer, cell, execution.shim),
                "first paint used the shim pid"
            );
            assert!(
                !drawn_process_in(&buffer, cell, execution.parent),
                "first paint used the parent pid"
            );
            let ink = process_ink(&buffer, &grid, execution.cargo);
            assert!(
                ink + 0.05 >= *previous,
                "CPU sampling restarted the entry ink for {}",
                execution.cargo
            );
            *previous = ink;
            let sites: HashSet<_> = spans
                .iter()
                .filter(|span| &span.id == owner)
                .map(|span| format!("{:?}", span.content))
                .collect();
            if sites.len() != 1 {
                exclusive_failures.push(format!(
                    "sample {index}, cargo {}: {sites:?}",
                    execution.cargo
                ));
            }
        }
        grid.elapse_for_test(Duration::from_millis(20));
    }
    assert!(
        elapsed.elapsed() < Duration::from_secs(1),
        "CPU fixture took {:?}",
        elapsed.elapsed()
    );
    drop(appearance);
    assert!(
        exclusive_failures.is_empty(),
        "first wrong transition: AwaitingFirstSample cargo proof -> two paint sites: {exclusive_failures:?}"
    );
    drop(root);
}

/// With free rows, only the missing cargo proof can keep a registered job listed.
#[test]
fn a_registered_jobs_listed_wait_ends_on_the_cargo_discovery_frame() {
    let appearance = FixtureAppearance::dark();
    let elapsed = Instant::now();
    let execution = ForwardingExecution {
        parent: 8100,
        shim:   8101,
        cargo:  8102,
    };
    let (root, capture) = registered_executions(slice::from_ref(&execution));
    let epoch = Instant::now() + Duration::from_secs(60);
    let args = ["cargo", "check"].map(OsString::from);
    let fade = Duration::from_secs(DEFAULT_FADE_SECONDS);
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
        let cell = grid.painted_placements_for_test().iter().find(|piece| {
            matches!(piece.content, TileContent::Group(_))
                && drawn_process_in(&buffer, piece, execution.cargo)
        });
        if cell.is_some() {
            first_cell.push(millis);
        } else {
            assert!(
                millis < 750,
                "cargo proof did not admit on its discovery frame"
            );
            assert!(
                roster.groups()[0].lead.admissible().is_err(),
                "free rows wait despite cargo proof"
            );
            assert!(
                rows.iter().any(|row| row.content == TileContent::Summary),
                "listed registration has no row"
            );
            assert!(
                grid.painted_placements_for_test()
                    .iter()
                    .all(|piece| piece.content == TileContent::Summary),
                "unproven registration opened a cell"
            );
        }
        grid.elapse_for_test(Duration::from_millis(250));
    }
    drop(root);
    assert_eq!(
        first_cell,
        [750],
        "Listed-to-Celled must occur on cargo discovery, without an extra frame"
    );
    assert!(
        elapsed.elapsed() < Duration::from_secs(1),
        "admission fixture took {:?}",
        elapsed.elapsed()
    );
    drop(appearance);
}

/// A completed cell receipt cannot send its ended rows back to the summary.
#[test]
fn a_completed_cell_departure_stays_out_of_the_summary() {
    let appearance = FixtureAppearance::dark();
    let epoch = Instant::now() + Duration::from_secs(60);
    let fade = Duration::from_secs(DEFAULT_FADE_SECONDS);
    let end_millis = 1000;
    let area = Rect::new(0, 0, 40, 48);
    let args = ["cargo", "completed-departure"].map(OsString::from);
    let mut sequence = CensusSequence::default();
    let mut roster = Roster::new();
    let mut grid = live_test_grid();
    grid.set_motion_elapsed_for_test(Duration::ZERO);
    let process =
        ProcessObservation::cargo(9200, &args).with_lifetime(LifetimeEvidence::Available(
            ProcessLifetime::matching_registration_for_test(&subject_birth(9200)),
        ));
    sequence.sample_published(
        &ProcessObservations::new([process]),
        &Capture::default(),
        epoch,
    );
    let owner = sequence.publication().groups()[0].id();
    roster.observe_invocations(sequence.publication());
    foreign_load_frame(&mut roster, &mut grid, area, fade);
    grid.elapse_for_test(Duration::from_millis(800));
    roster.advance(epoch + Duration::from_millis(800), fade);
    let (live, rows) = foreign_load_frame(&mut roster, &mut grid, area, fade);
    assert_eq!(
        process_cells(&live, &grid, 9200),
        1,
        "live execution needs one painted cell"
    );
    assert!(
        rows.iter().any(|row| row.id == owner),
        "live receipt needs an actual process span"
    );
    sequence.sample_with_native_evidence(
        &ProcessObservations::default(),
        &Capture::default(),
        epoch + Duration::from_millis(end_millis),
        FixturePublication::Current,
        |_| Observation::Ended,
    );
    roster.observe_invocations(sequence.publication());
    roster.advance(epoch + Duration::from_millis(end_millis), fade);
    grid.elapse_for_test(Duration::from_millis(200));
    let (ended, _) = foreign_load_frame(&mut roster, &mut grid, area, fade);
    assert_eq!(
        process_cells(&ended, &grid, 9200),
        1,
        "end publication must retain the existing cell"
    );
    let fade_millis = u64::try_from(fade.as_millis()).expect("fixture fade fits milliseconds");
    let completed_millis = end_millis + fade_millis;
    let mut previous_millis = end_millis;
    for millis in (end_millis + 100..=completed_millis)
        .step_by(100)
        .chain([completed_millis + 20])
    {
        grid.elapse_for_test(Duration::from_millis(millis - previous_millis));
        previous_millis = millis;
        roster.advance(epoch + Duration::from_millis(millis), fade);
        let (buffer, rows) = foreign_load_frame(&mut roster, &mut grid, area, fade);
        assert!(
            rows.iter()
                .all(|row| row.id != owner || row.content == TileContent::Group(owner.clone())),
            "first wrong transition: completed cell receipt -> summary at {millis}ms"
        );
        if millis >= completed_millis {
            assert!(
                rows.iter().all(|row| row.id != owner),
                "completed receipt retained a painted process span"
            );
            assert!(
                buffer_rows(&buffer)
                    .iter()
                    .all(|row| !row.contains("completed-departure")),
                "completed execution returned to the painted summary at {millis}ms"
            );
        }
    }
    drop(appearance);
}
