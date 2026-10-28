mod one_place_tests;

use tui_pane::Appearance;
use tui_pane::Theme;

use super::*;
use crate::constants::PROCESS_POLL_MILLIS;
use crate::theme::builtins;

/// Scans with a job entry, a foreign departure, or foreign growth.
const MOTION_SAMPLE_SCANS: [u64; 3] = [6, 8, 12];

/// Process identities used by the two independent fixture sessions.
const FOREIGN_SESSION_PID: u32 = 80_000;
const FOREIGN_LEAD_PID: u32 = 81_000;
const JOB_SESSION_PID: u32 = 89_000;
const JOB_PID: u32 = 90_000;

/// Measured census and frame work for the two foreign-load viewports.
const FOREIGN_40_WORK_LIMIT: FixtureWork = FixtureWork {
    census_passes:    10,
    draws:            13,
    frames:           13,
    invariant_checks: 13,
};
const FOREIGN_80_WORK_LIMIT: FixtureWork = FixtureWork {
    census_passes:    15,
    draws:            18,
    frames:           18,
    invariant_checks: 18,
};

/// Evidence that the fixture increased a foreign cell's content.
enum ForeignGrowth {
    Unchanged,
    Observed,
}

#[derive(Default)]
enum Evidence {
    #[default]
    Absent,
    Observed,
}

enum InvocationState {
    Live,
    Ended,
}

/// Work done by one fixture, independent of time spent on other processes.
#[derive(Debug, Default)]
struct FixtureWork {
    census_passes:    usize,
    draws:            usize,
    frames:           usize,
    invariant_checks: usize,
}

impl FixtureWork {
    fn census<T>(&mut self, sample: impl FnOnce() -> T) -> T {
        self.census_passes += 1;
        sample()
    }

    fn draw<T>(&mut self, draw: impl FnOnce() -> T) -> T {
        self.draws += 1;
        let frame = draw();
        self.frames += 1;
        frame
    }

    fn check(&mut self, check: impl FnOnce()) {
        self.invariant_checks += 1;
        check();
    }

    fn assert_within(&self, limit: &Self, elapsed: Duration) {
        assert!(
            self.census_passes <= limit.census_passes
                && self.draws <= limit.draws
                && self.frames <= limit.frames
                && self.invariant_checks <= limit.invariant_checks,
            "fixture work grew: {self:?}, limit {limit:?}, elapsed {elapsed:?}"
        );
    }
}

/// A process row that reached the output buffer, identified before its text was drawn.
pub(super) struct ForeignLoadPaintedRow {
    pub(super) id:      InvocationId,
    pub(super) area:    Rect,
    pub(super) content: TileContent,
}

#[derive(Clone, Copy)]
enum JobPaint {
    Unpublished,
    Published { at: u64 },
    Drawn { at: u64 },
}

struct ForeignLoadJob {
    pid:    u32,
    starts: u64,
    ends:   u64,
    paint:  JobPaint,
}

impl ForeignLoadJob {
    fn scheduled(index: u32) -> Self {
        Self {
            pid:    JOB_PID + index,
            starts: 1500 + u64::from(index / 2) * PROCESS_POLL_MILLIS,
            ends:   1500
                + u64::from(index / 2) * PROCESS_POLL_MILLIS
                + 500
                + u64::from(index % 5) * PROCESS_POLL_MILLIS,
            paint:  JobPaint::Unpublished,
        }
    }

    fn record_publication(&mut self, sequence: &CensusSequence, millis: u64) {
        if millis >= self.starts && millis < self.ends {
            assert!(
                sequence
                    .publication()
                    .groups()
                    .iter()
                    .any(|group| group.lead.pid == self.pid),
                "census withheld job {} on its first eligible scan at {millis}ms",
                self.pid
            );
            if matches!(self.paint, JobPaint::Unpublished) {
                self.paint = JobPaint::Published { at: millis };
            }
        }
    }
}

struct PreviousCellInk {
    id:               InvocationId,
    contrast:         f64,
    invocation_state: InvocationState,
}

struct FixtureAppearance {
    theme:       Arc<Theme>,
    transparent: bool,
}

impl FixtureAppearance {
    /// Use resolved dark inks and restore the caller's appearance when the fixture ends.
    fn dark() -> Self {
        let appearance = Self {
            theme:       tui_pane::theme(),
            transparent: tui_pane::transparent_background(),
        };
        let dark = builtins::builtins()
            .into_iter()
            .find(|variant| variant.appearance == Appearance::Dark)
            .expect("the app ships a dark theme")
            .theme;
        tui_pane::set_active_theme(Arc::new(dark));
        tui_pane::set_transparent_background(false);
        appearance
    }
}

impl Drop for FixtureAppearance {
    fn drop(&mut self) {
        tui_pane::set_active_theme(self.theme.clone());
        tui_pane::set_transparent_background(self.transparent);
    }
}

/// Paint receipts and ink history across the fixture's injected frames.
#[derive(Default)]
struct ForeignLoadPaint {
    invariant:           CaptureFrameInvariant,
    work:                FixtureWork,
    previous_ink:        Vec<PreviousCellInk>,
    previous_live_cells: Vec<InvocationId>,
    ancestry_cuts:       Vec<(u64, usize, usize)>,
    frame_millis:        u64,
    fade:                Duration,
    job_cell:            Evidence,
    listed_job:          Evidence,
    ended_cell:          Evidence,
}

impl ForeignLoadPaint {
    fn assert_complete(&self, jobs: &[ForeignLoadJob], foreign_growth: &ForeignGrowth) {
        for job in jobs {
            let JobPaint::Drawn { at } = job.paint else {
                panic!("job {} was never drawn", job.pid);
            };
            assert!(
                at.saturating_sub(job.starts) < 1000,
                "job {} missed its census deadline",
                job.pid
            );
        }
        assert!(
            matches!(self.job_cell, Evidence::Observed),
            "fixture never admitted a short job cell"
        );
        assert!(
            matches!(self.listed_job, Evidence::Observed),
            "fixture never filled the tile with a waiting job"
        );
        assert!(
            matches!(foreign_growth, ForeignGrowth::Observed),
            "fixture did not grow a foreign cell"
        );
        assert!(
            matches!(self.ended_cell, Evidence::Observed),
            "fixture did not paint an ended job cell"
        );
    }

    /// Measure and check the provider that painted this frame without preparing another.
    fn draw_and_check(
        &mut self,
        roster: &mut Roster,
        grid: &mut TileGrid<InvocationId>,
        area: Rect,
        jobs: &mut [ForeignLoadJob],
        now: u64,
    ) {
        let (buffer, rows) = self.work.draw(|| {
            foreign_load_frame_with(roster, grid, area, self.fade, |cells, grid, roster| {
                self.invariant.measure_required_bodies(cells, grid, roster);
            })
        });
        self.check(roster, grid, &buffer, &rows, jobs, now);
    }

    fn check(
        &mut self,
        roster: &Roster,
        grid: &TileGrid<InvocationId>,
        buffer: &Buffer,
        rows: &[ForeignLoadPaintedRow],
        jobs: &mut [ForeignLoadJob],
        now: u64,
    ) {
        assert_drawn_rows(buffer, rows);
        let live: Vec<_> = roster
            .groups()
            .iter()
            .filter(|group| !group.lead.is_ended())
            .collect();
        let celled: Vec<_> = grid
            .painted_placements_for_test()
            .iter()
            .filter_map(|piece| match &piece.content {
                TileContent::Group(id) if live.iter().any(|group| &group.id == id) => {
                    Some(id.clone())
                },
                _ => None,
            })
            .collect();
        self.check_jobs(roster, buffer, rows, jobs, &celled, now);
        for id in &self.previous_live_cells {
            if live.iter().any(|group| &group.id == id) && !celled.contains(id) {
                assert!(
                    grid.summary_live_for_test().contains(id),
                    "lost live cell {id:?} at {now}ms without capacity release"
                );
                assert!(
                    rows.iter()
                        .any(|row| &row.id == id && row.content == TileContent::Summary),
                    "capacity release lost the listed row for {id:?}"
                );
            }
        }
        self.check_job_ink(roster, buffer, rows, now);
        self.invariant.placements = grid
            .painted_placements_for_test()
            .iter()
            .map(|placement| TilePlacement {
                content: placement.content.clone(),
                frame:   placement.frame,
            })
            .collect();
        let ancestry_rows = self.invariant.ancestry_rows(buffer, roster);
        if ancestry_rows.iter().any(|row| !row.retained) {
            assert_ancestry_cut_frame(&self.invariant, buffer, roster, &ancestry_rows, now);
            let retained = ancestry_rows.iter().filter(|row| row.retained).count();
            self.ancestry_cuts
                .push((now, retained, ancestry_rows.len() - retained));
        }
        self.work
            .check(|| self.invariant.check(buffer, grid, roster, now, self.fade));
        self.invariant.assert_valid();
        self.previous_live_cells = celled;
    }

    fn check_jobs(
        &mut self,
        roster: &Roster,
        buffer: &Buffer,
        rows: &[ForeignLoadPaintedRow],
        jobs: &mut [ForeignLoadJob],
        celled: &[InvocationId],
        now: u64,
    ) {
        for job in jobs {
            let Some(group) = roster
                .groups()
                .iter()
                .find(|group| group.lead.process.pid == job.pid)
            else {
                continue;
            };
            if !group.lead.is_ended() {
                let painted = rows.iter().any(|row| row.id == group.id);
                assert!(
                    painted,
                    "live job {} has neither a cell row nor a listed summary row at {now}ms\n{}",
                    job.pid,
                    buffer_rows(buffer).join("\n")
                );
                if let JobPaint::Published { at } = job.paint {
                    assert!(
                        now - at <= self.frame_millis,
                        "job {} waited {}ms after publication",
                        job.pid,
                        now - at
                    );
                    job.paint = JobPaint::Drawn { at: now };
                }
                if rows
                    .iter()
                    .any(|row| row.id == group.id && row.content == TileContent::Summary)
                    && !celled.contains(&group.id)
                {
                    self.listed_job = Evidence::Observed;
                }
            }
        }
    }

    fn check_job_ink(
        &mut self,
        roster: &Roster,
        buffer: &Buffer,
        rows: &[ForeignLoadPaintedRow],
        now: u64,
    ) {
        for group in roster
            .groups()
            .iter()
            .filter(|group| group.lead.process.pid >= JOB_PID)
        {
            let Some(row) = rows
                .iter()
                .find(|row| row.id == group.id && matches!(row.content, TileContent::Group(_)))
            else {
                continue;
            };
            self.job_cell = Evidence::Observed;
            let contrast = row_contrast(buffer, row.area);
            let invocation_state = if group.lead.is_ended() {
                InvocationState::Ended
            } else {
                InvocationState::Live
            };
            if let Some(before) = self
                .previous_ink
                .iter_mut()
                .find(|before| before.id == group.id)
            {
                if matches!(invocation_state, InvocationState::Ended) {
                    self.ended_cell = Evidence::Observed;
                    assert!(
                        contrast <= before.contrast + 0.02,
                        "ended job {} ink rose after publication at {now}ms: {} -> {contrast}",
                        group.lead.process.pid,
                        before.contrast
                    );
                } else if matches!(before.invocation_state, InvocationState::Live) {
                    assert!(
                        contrast + 0.02 >= before.contrast,
                        "job {} entry ink reversed at {now}ms",
                        group.lead.process.pid
                    );
                }
                before.contrast = contrast;
                before.invocation_state = invocation_state;
            } else {
                assert!(
                    contrast > 1.015,
                    "new cell {} painted at start ink at {now}ms: contrast {contrast}",
                    group.lead.process.pid
                );
                self.previous_ink.push(PreviousCellInk {
                    id: group.id.clone(),
                    contrast,
                    invocation_state,
                });
            }
        }
    }
}

fn assert_drawn_rows(buffer: &Buffer, rows: &[ForeignLoadPaintedRow]) {
    for row in rows {
        let clipped = row.area.intersection(buffer.area);
        assert!(
            !clipped.is_empty(),
            "painted invocation {:?} lies outside the buffer",
            row.id
        );
        assert!(
            (clipped.y..clipped.bottom()).any(|y| fixture_row_cells(buffer, clipped, y)
                .iter()
                .any(|cell| !cell.symbol().trim().is_empty())),
            "painted invocation {:?} has a blank row",
            row.id
        );
    }
}

struct ForeignLoadCommands {
    foreign_cells:         u32,
    foreign_arguments:     Vec<[OsString; 4]>,
    job_arguments:         Vec<[OsString; 2]>,
    child_arguments:       [OsString; 2],
    session_arguments:     [OsString; 2],
    job_session_arguments: [OsString; 2],
    foreign_directories:   Vec<PathBuf>,
    job_directories:       Vec<PathBuf>,
}

impl ForeignLoadCommands {
    fn new(foreign_cells: u32, job_count: u32) -> Self {
        let foreign_arguments: Vec<_> = (0..foreign_cells)
            .map(|index| {
                [
                    "cargo".into(),
                    "run".into(),
                    "--example".into(),
                    format!("foreign{index:02}").into(),
                ]
            })
            .collect();
        let job_arguments: Vec<_> = (0..job_count)
            .map(|index| {
                [
                    OsString::from("cargo"),
                    OsString::from(format!("p22k{index:02}")),
                ]
            })
            .collect();
        let child_arguments = ["cargo", "check"].map(OsString::from);
        let session_arguments = ["python3", "foreign-session.py"].map(OsString::from);
        let job_session_arguments = ["python3", "jobs-session.py"].map(OsString::from);
        let foreign_directories: Vec<_> = (0..foreign_cells)
            .map(|index| PathBuf::from(format!("/foreign/example{index:02}")))
            .collect();
        let job_directories: Vec<_> = (0..job_count)
            .map(|index| PathBuf::from(format!("/jobs/p22k{index:02}")))
            .collect();
        Self {
            foreign_cells,
            foreign_arguments,
            job_arguments,
            child_arguments,
            session_arguments,
            job_session_arguments,
            foreign_directories,
            job_directories,
        }
    }

    fn observations(
        &self,
        children: u32,
        jobs: &[ForeignLoadJob],
        scan: u64,
        foreign_growth: &mut ForeignGrowth,
    ) -> ProcessObservations<'_> {
        let millis = scan * PROCESS_POLL_MILLIS;
        let mut observations = Vec::new();
        observations.push(ProcessObservation::cargo(
            FOREIGN_SESSION_PID,
            &self.session_arguments,
        ));
        observations.push(ProcessObservation::cargo(
            JOB_SESSION_PID,
            &self.job_session_arguments,
        ));
        for index in 0..self.foreign_cells {
            if (index == self.foreign_cells - 1 && scan >= 8)
                || (index == self.foreign_cells - 2 && scan >= 12)
            {
                continue;
            }
            let pid = FOREIGN_LEAD_PID + index * 100;
            let index_usize = usize::try_from(index).expect("foreign index");
            let mut lead = ProcessObservation::cargo(pid, &self.foreign_arguments[index_usize]);
            lead.parent = ProcessField::Observed(Pid::from_u32(FOREIGN_SESSION_PID));
            lead.directory = ProcessField::Observed(&self.foreign_directories[index_usize]);
            lead.uid = ProcessField::Observed(2000);
            observations.push(lead);
            let added = if index == 0 && scan >= 12 { 3 } else { 0 };
            if added > 0 {
                *foreign_growth = ForeignGrowth::Observed;
            }
            for child in 0..children + added {
                let mut cargo = ProcessObservation::cargo(pid + child + 1, &self.child_arguments);
                cargo.parent = ProcessField::Observed(Pid::from_u32(pid));
                cargo.directory = ProcessField::Observed(&self.foreign_directories[index_usize]);
                cargo.uid = ProcessField::Observed(2000);
                observations.push(cargo);
            }
        }
        for (index, job) in jobs.iter().enumerate() {
            if millis >= job.starts && millis < job.ends {
                let mut process = ProcessObservation::cargo(job.pid, &self.job_arguments[index]);
                process.parent = ProcessField::Observed(Pid::from_u32(JOB_SESSION_PID));
                process.directory = ProcessField::Observed(&self.job_directories[index]);
                observations.push(process);
            }
        }
        ProcessObservations::new(observations)
    }
}

/// Check the body and admission order when a painted ancestry prefix yields.
fn foreign_load(
    area: Rect,
    foreign_cells: u32,
    children: u32,
    job_count: u32,
    frame_millis: u64,
    work_limit: &FixtureWork,
) {
    let appearance = FixtureAppearance::dark();
    let elapsed = Instant::now();
    let epoch = Instant::now() + Duration::from_secs(60);
    let fade = Duration::from_secs(DEFAULT_FADE_SECONDS);
    let mut sequence = CensusSequence::default();
    let mut roster = Roster::new();
    let mut grid = live_test_grid();
    grid.set_motion_elapsed_for_test(Duration::ZERO);
    let mut paint = ForeignLoadPaint {
        frame_millis,
        fade,
        ..ForeignLoadPaint::default()
    };
    let mut jobs: Vec<_> = (0..job_count).map(ForeignLoadJob::scheduled).collect();
    let commands = ForeignLoadCommands::new(foreign_cells, job_count);
    let mut foreign_growth = ForeignGrowth::Unchanged;

    let last_scan = jobs.iter().map(|job| job.ends).max().unwrap_or(0) / PROCESS_POLL_MILLIS;
    for scan in 0..=last_scan {
        // No process changes during the initial foreign-only opening.
        if (1..6).contains(&scan) {
            grid.elapse_for_test(Duration::from_millis(PROCESS_POLL_MILLIS));
            continue;
        }
        let millis = scan * PROCESS_POLL_MILLIS;
        let observations = commands.observations(children, &jobs, scan, &mut foreign_growth);
        paint.work.census(|| {
            sequence.sample_published(
                &observations,
                &Capture::default(),
                epoch + Duration::from_millis(millis),
            );
        });
        for job in &mut jobs {
            job.record_publication(&sequence, millis);
        }
        roster.observe_invocations(sequence.publication());
        // Keep intermediate job entry, foreign departure and growth frames.
        let motion_sample = MOTION_SAMPLE_SCANS.contains(&scan);
        let offsets: &[u64] = if motion_sample {
            &[0, frame_millis]
        } else {
            &[0]
        };
        for &offset in offsets {
            let now = millis + offset;
            roster.advance(epoch + Duration::from_millis(now), fade);
            paint.draw_and_check(&mut roster, &mut grid, area, &mut jobs, now);
            let delta = if offset == 0 && motion_sample {
                frame_millis
            } else {
                PROCESS_POLL_MILLIS - offset
            };
            grid.elapse_for_test(Duration::from_millis(delta));
        }
    }
    paint.assert_complete(&jobs, &foreign_growth);
    if area.width == 80 {
        assert_eq!(
            paint.ancestry_cuts,
            [(2750, 2, 3), (3050, 1, 2), (3750, 0, 1)],
            "the full tile must check each measured ancestry shortfall"
        );
    }
    paint.work.assert_within(work_limit, elapsed.elapsed());
    drop(appearance);
}

fn assert_ancestry_cut_frame(
    invariant: &CaptureFrameInvariant,
    buffer: &Buffer,
    roster: &Roster,
    ancestry_rows: &[CaptureAncestryRow],
    millis: u64,
) {
    for (index, row) in ancestry_rows
        .iter()
        .enumerate()
        .filter(|(_, row)| !row.retained)
    {
        let placement = invariant
            .placements
            .iter()
            .find(|placement| placement.content == TileContent::Group(row.cell.clone()))
            .expect("a yielding prefix retains its cell");
        let frame = placement.frame.rect();
        let column = Rect::new(
            frame.x.saturating_add(1),
            buffer.area.y,
            1,
            buffer.area.height,
        );
        assert_eq!(
            invariant.free_column_rows(column),
            0,
            "painted ancestor yielded with a free row in its column at {millis}ms"
        );
        assert!(
            !ancestry_rows[index + 1..]
                .iter()
                .any(|newer| newer.cell != row.cell
                    && newer.retained
                    && invariant.placements.iter().any(|other| {
                        other.content == TileContent::Group(newer.cell.clone())
                            && other.frame.rect().x == frame.x
                    })),
            "an older painted prefix yielded while a newer prefix stayed at {millis}ms"
        );
        let group = roster
            .groups()
            .iter()
            .find(|group| group.id == row.cell)
            .expect("a yielding prefix retains its roster group");
        assert!(
            drawn_process_in(buffer, placement, group.lead.process.pid),
            "a cut cell lost its body with its prefix at {millis}ms"
        );
    }
}

/// Borrow one clipped row from the buffer without indexing every cell by coordinates.
fn fixture_row_cells(buffer: &Buffer, area: Rect, row: u16) -> &[Cell] {
    let start = usize::from(row - buffer.area.y) * usize::from(buffer.area.width)
        + usize::from(area.x - buffer.area.x);
    &buffer.content[start..start + usize::from(area.width)]
}

fn row_contrast(buffer: &Buffer, area: Rect) -> f64 {
    let area = area.intersection(buffer.area);
    (area.y..area.bottom())
        .flat_map(|y| fixture_row_cells(buffer, area, y))
        .filter_map(|cell| {
            if cell.symbol().trim().is_empty() {
                return None;
            }
            let foreground = relative_luminance(cell.fg);
            let background = relative_luminance(cell.bg);
            Some((foreground.max(background) + 0.05) / (foreground.min(background) + 0.05))
        })
        .fold(1.0, f64::max)
}

fn relative_luminance(color: Color) -> f64 {
    let (red, green, blue) = match color {
        Color::Rgb(red, green, blue) => (red, green, blue),
        Color::Black => (0, 0, 0),
        Color::White => (255, 255, 255),
        other => panic!("foreign-load frame must use a resolved ink: {other:?}"),
    };
    let component = |value| {
        let value = f64::from(value) / 255.0;
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    0.0722f64.mul_add(
        component(blue),
        0.7152f64.mul_add(component(green), 0.2126 * component(red)),
    )
}

#[test]
#[should_panic(expected = "painted ancestor yielded with a free row in its column")]
fn a_supplied_free_row_in_the_cut_column_cannot_replace_an_ancestor() {
    let (invariant, buffer, roster) = supplied_ancestry_cut(1, false);
    let ancestry_rows = invariant.ancestry_rows(&buffer, &roster);
    assert_ancestry_cut_frame(&invariant, &buffer, &roster, &ancestry_rows, 0);
}

#[test]
#[should_panic(expected = "an older painted prefix yielded while a newer prefix stayed")]
fn a_supplied_older_prefix_cannot_yield_before_the_newest() {
    let (invariant, buffer, roster) = supplied_ancestry_cut(0, true);
    let ancestry_rows = invariant.ancestry_rows(&buffer, &roster);
    assert_ancestry_cut_frame(&invariant, &buffer, &roster, &ancestry_rows, 0);
}

#[test]
#[should_panic(expected = "an older painted prefix yielded while a newer prefix stayed")]
fn two_supplied_cuts_cannot_skip_a_newer_retained_prefix() {
    let groups = live_scan_groups(4);
    let mut roster = Roster::new();
    roster.observe_invocations(&roster.fixture_with_ended_absences(groups.clone(), Instant::now()));
    let mut previous = Buffer::empty(Rect::new(0, 0, 40, 25));
    let mut current = Buffer::empty(Rect::new(0, 0, 40, 23));
    let mut prior_cells = Vec::new();
    let mut placements = Vec::new();
    let mut top = 0;
    for (index, group) in groups.iter().enumerate() {
        let old_top = u16::try_from(index).unwrap_or(u16::MAX) * 6;
        previous.set_string(1, old_top + 1, "3334 systemd", Style::default());
        prior_cells.push((
            group.id(),
            PaneFrame::new(Rect::new(0, old_top, 40, 7)),
            group.lead.pid,
        ));
        let retained = index % 2 == 0;
        let height = if retained { 7 } else { 6 };
        if retained {
            current.set_string(1, top + 1, "3334 systemd", Style::default());
        }
        current.set_string(
            1,
            top + height - 2,
            format!("{} cargo check", group.lead.pid),
            Style::default(),
        );
        placements.push(TilePlacement {
            content: TileContent::Group(group.id()),
            frame:   PaneFrame::new(Rect::new(0, top, 40, height)),
        });
        top += height - 1;
    }
    let invariant = CaptureFrameInvariant {
        previous: PreviousCaptureFrame::Drawn {
            buffer: previous,
            cells:  prior_cells,
        },
        placements,
        ..CaptureFrameInvariant::default()
    };
    let rows = invariant.ancestry_rows(&current, &roster);
    assert_eq!(rows.iter().filter(|row| !row.retained).count(), 2);
    assert_eq!(invariant.free_column_rows(current.area), 0);
    assert_ancestry_cut_frame(&invariant, &current, &roster, &rows, 0);
}

/// Foreign examples occupy the grid while an independent session submits short jobs.
#[test]
fn a_full_tile_under_foreign_load_draws_every_job_at_80_columns() {
    foreign_load(
        Rect::new(0, 0, 80, 48),
        6,
        6,
        16,
        50,
        &FOREIGN_80_WORK_LIMIT,
    );
}

/// A single narrow column must list waiting jobs before commands that already have cells.
#[test]
fn a_full_tile_under_foreign_load_draws_every_job_at_40_columns() {
    foreign_load(Rect::new(0, 0, 40, 72), 6, 6, 8, 20, &FOREIGN_40_WORK_LIMIT);
}
