use super::*;

/// A process row that reached the output buffer, identified before its text was drawn.
pub(super) struct ForeignLoadPaintedRow {
    pub(super) id: InvocationId,
    pub(super) area: Rect,
    pub(super) content: TileContent,
}

#[derive(Clone, Copy)]
enum JobPaint {
    Unpublished,
    Published { at: u64 },
    Drawn { at: u64 },
}

struct ForeignLoadJob {
    pid: u32,
    starts: u64,
    ends: u64,
    paint: JobPaint,
}

impl ForeignLoadJob {
    fn scheduled(index: u32) -> Self {
        Self {
            pid: 90_000 + index,
            starts: 1500 + u64::from(index) * 250,
            ends: 1500 + u64::from(index) * 250 + 500 + u64::from(index % 5) * 250,
            paint: JobPaint::Unpublished,
        }
    }

    fn record_publication(&mut self, sequence: &CensusSequence, millis: u64) {
    if millis >= self.starts && millis < self.ends {
        assert!(sequence.publication().groups().iter().any(|group| group.lead.pid == self.pid),
            "census withheld job {} on its first eligible scan at {millis}ms", self.pid);
        if matches!(self.paint, JobPaint::Unpublished) {
            self.paint = JobPaint::Published { at: millis };
        }
    }
    }
}

struct PreviousCellInk {
    id: InvocationId,
    contrast: f64,
    ended: bool,
}

/// Foreign examples occupy the grid while an independent session submits short jobs.
#[test]
fn a_full_tile_under_foreign_load_draws_every_job_at_80_columns() {
    foreign_load(Rect::new(0, 0, 80, 48), 6, 6, 50);
}

/// A single narrow column must list waiting jobs before commands that already have cells.
#[test]
fn a_full_tile_under_foreign_load_draws_every_job_at_40_columns() {
    foreign_load(Rect::new(0, 0, 40, 120), 8, 10, 20);
}

struct FixtureAppearance {
    theme: Arc<tui_pane::Theme>,
    transparent: bool,
}

impl Drop for FixtureAppearance {
    fn drop(&mut self) {
        tui_pane::set_active_theme(self.theme.clone());
        tui_pane::set_transparent_background(self.transparent);
    }
}

impl FixtureAppearance {
    /// Use resolved dark inks and restore the caller's appearance when the fixture ends.
    fn dark() -> Self {
    let appearance = Self {
        theme: tui_pane::theme(),
        transparent: tui_pane::transparent_background(),
    };
    let dark = crate::theme::builtins::builtins().into_iter()
        .find(|variant| variant.appearance == tui_pane::Appearance::Dark)
        .expect("the app ships a dark theme").theme;
    tui_pane::set_active_theme(Arc::new(dark));
    tui_pane::set_transparent_background(false);
        appearance
    }
}

fn foreign_load(area: Rect, foreign_cells: u32, children: u32, frame_millis: u64) {
    let appearance = FixtureAppearance::dark();
    let elapsed = Instant::now();
    let epoch = Instant::now() + Duration::from_secs(60);
    let fade = Duration::from_secs(3);
    let mut sequence = CensusSequence::default();
    let mut roster = Roster::new();
    let mut grid = live_test_grid();
    grid.set_motion_elapsed_for_test(Duration::ZERO);
    let mut paint = ForeignLoadPaint {
        frame_millis,
        fade,
        ..ForeignLoadPaint::default()
    };
    let mut jobs: Vec<_> = (0..16_u32)
        .map(ForeignLoadJob::scheduled)
        .collect();
    let foreign_arguments: Vec<_> = (0..foreign_cells)
        .map(|index| ["cargo".into(), "run".into(), "--example".into(), format!("foreign{index:02}").into()])
        .collect();
    let job_arguments: Vec<_> = (0..16)
        .map(|index| [OsString::from("cargo"), OsString::from(format!("p22k{index:02}"))])
        .collect();
    let child_arguments = ["cargo", "check"].map(OsString::from);
    let session_arguments = ["python3", "foreign-session.py"].map(OsString::from);
    let job_session_arguments = ["python3", "jobs-session.py"].map(OsString::from);
    let foreign_directories: Vec<_> = (0..foreign_cells)
        .map(|index| PathBuf::from(format!("/foreign/example{index:02}")))
        .collect();
    let job_directories: Vec<_> = (0..16)
        .map(|index| PathBuf::from(format!("/jobs/p22k{index:02}")))
        .collect();
    let mut saw_foreign_growth = false;

    for scan in 0..29_u64 {
        // No process changes during the initial foreign-only opening.
        if (1..6).contains(&scan) {
            grid.elapse_for_test(Duration::from_millis(250));
            continue;
        }
        let millis = scan * 250;
        let mut observations = Vec::new();
        observations.push(ProcessObservation::cargo(80_000, &session_arguments));
        observations.push(ProcessObservation::cargo(89_000, &job_session_arguments));
        for index in 0..foreign_cells {
            if (index == foreign_cells - 1 && scan >= 8) || (index == foreign_cells - 2 && scan >= 12) { continue; }
            let pid = 81_000 + index * 100;
            let index_usize = usize::try_from(index).expect("foreign index");
            let mut lead = ProcessObservation::cargo(pid, &foreign_arguments[index_usize]);
            lead.parent = ProcessField::Observed(Pid::from_u32(80_000));
            lead.directory = ProcessField::Observed(&foreign_directories[index_usize]);
            lead.uid = ProcessField::Observed(2000);
            observations.push(lead);
            let added = if index == 0 && scan >= 12 { 3 } else { 0 };
            saw_foreign_growth |= added > 0;
            for child in 0..children + added {
                let mut cargo = ProcessObservation::cargo(pid + child + 1, &child_arguments);
                cargo.parent = ProcessField::Observed(Pid::from_u32(pid));
                cargo.directory = ProcessField::Observed(&foreign_directories[index_usize]);
                cargo.uid = ProcessField::Observed(2000);
                observations.push(cargo);
            }
        }
        for (index, job) in jobs.iter().enumerate() {
            if millis >= job.starts && millis < job.ends {
                let mut process = ProcessObservation::cargo(job.pid, &job_arguments[index]);
                process.parent = ProcessField::Observed(Pid::from_u32(89_000));
                process.directory = ProcessField::Observed(&job_directories[index]);
                observations.push(process);
            }
        }
        let scan_started = Instant::now();
        sequence.sample_published(
            &ProcessObservations::new(observations),
            &Capture::default(),
            epoch + Duration::from_millis(millis),
        );
        assert!(scan_started.elapsed() < Duration::from_secs(1), "census exceeded one second at {millis}ms");
        for job in &mut jobs {
            job.record_publication(&sequence, millis);
        }
        roster.observe_invocations(sequence.publication());
        for offset in [0, frame_millis] {
            let now = millis + offset;
            roster.advance(epoch + Duration::from_millis(now), fade);
            paint.draw_and_check(&mut roster, &mut grid, area, &mut jobs, now);
            let delta = if offset == frame_millis { 250 - offset } else { frame_millis };
            grid.elapse_for_test(Duration::from_millis(delta));
        }
    }
    for job in &jobs {
        let JobPaint::Drawn { at } = job.paint else { panic!("job {} was never drawn", job.pid); };
        assert!(at.saturating_sub(job.starts) < 1000, "job {} missed its census deadline", job.pid);
    }
    assert!(paint.saw_job_cell, "fixture never admitted a short job cell");
    assert!(paint.saw_listed_job, "fixture never filled the tile with a waiting job");
    assert!(saw_foreign_growth, "fixture did not grow a foreign cell");
    assert!(paint.saw_ended_cell, "fixture did not paint an ended job cell");
    assert!(elapsed.elapsed() < Duration::from_secs(1), "foreign-load fixture took {:?}", elapsed.elapsed());
    drop(appearance);
}

/// Paint receipts and ink history across the fixture's injected frames.
#[derive(Default)]
struct ForeignLoadPaint {
    invariant: CaptureFrameInvariant,
    previous_ink: Vec<PreviousCellInk>,
    previous_live_cells: Vec<InvocationId>,
    frame_millis: u64,
    fade: Duration,
    saw_job_cell: bool,
    saw_listed_job: bool,
    saw_ended_cell: bool,
}

impl ForeignLoadPaint {
    /// Measure and check the provider that painted this frame without preparing another.
    fn draw_and_check(
        &mut self,
        roster: &mut Roster,
        grid: &mut TileGrid<InvocationId>,
        area: Rect,
        jobs: &mut [ForeignLoadJob],
        now: u64,
    ) {
        let (buffer, rows) = foreign_load_frame_with(
            roster, grid, area, self.fade,
            |cells, grid, roster| self.invariant.measure_required_bodies(cells, grid, roster),
        );
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
    for row in rows {
        let clipped = row.area.intersection(buffer.area);
        assert!(!clipped.is_empty(), "painted invocation {:?} lies outside the buffer", row.id);
        assert!((clipped.y..clipped.bottom()).any(|y|
            (clipped.x..clipped.right()).any(|x| !buffer[(x, y)].symbol().trim().is_empty())),
            "painted invocation {:?} has a blank row", row.id);
    }
    let live: Vec<_> = roster.groups().iter().filter(|group| !group.lead.is_ended()).collect();
    let celled: Vec<_> = grid.painted_placements_for_test().iter()
        .filter_map(|piece| match &piece.content {
            TileContent::Group(id) if live.iter().any(|group| &group.id == id) => Some(id.clone()),
            _ => None,
        }).collect();
    for job in jobs {
        let Some(group) = roster.groups().iter().find(|group| group.lead.process.pid == job.pid) else { continue; };
        if !group.lead.is_ended() {
            let painted = rows.iter().any(|row| row.id == group.id);
            assert!(painted, "live job {} has neither a cell row nor a listed summary row at {now}ms\n{}", job.pid, buffer_rows(buffer).join("\n"));
            if let JobPaint::Published { at } = job.paint {
                assert!(now - at <= self.frame_millis, "job {} waited {}ms after publication", job.pid, now - at);
                job.paint = JobPaint::Drawn { at: now };
            }
            self.saw_listed_job |= rows.iter().any(|row| row.id == group.id && row.content == TileContent::Summary) && !celled.contains(&group.id);
        }
    }
    for id in &self.previous_live_cells {
        if live.iter().any(|group| &group.id == id) && !celled.contains(id) {
            assert!(grid.summary_live_for_test().contains(id), "lost live cell {id:?} at {now}ms without capacity release");
            assert!(rows.iter().any(|row| &row.id == id && row.content == TileContent::Summary), "capacity release lost the listed row for {id:?}");
        }
    }
    for group in roster.groups().iter().filter(|group| group.lead.process.pid >= 90_000) {
        let Some(row) = rows.iter().find(|row| row.id == group.id && matches!(row.content, TileContent::Group(_))) else { continue; };
        self.saw_job_cell = true;
        let contrast = row_contrast(buffer, row.area);
        let ended = group.lead.is_ended();
        if let Some(before) = self.previous_ink.iter_mut().find(|before| before.id == group.id) {
            if ended {
                self.saw_ended_cell = true;
                assert!(contrast <= before.contrast + 0.02, "ended job {} ink rose after publication at {now}ms: {} -> {contrast}", group.lead.process.pid, before.contrast);
            } else if !before.ended {
                assert!(contrast + 0.02 >= before.contrast, "job {} entry ink reversed at {now}ms", group.lead.process.pid);
            }
            before.contrast = contrast;
            before.ended = ended;
        } else {
            assert!(contrast > 1.015, "new cell {} painted at start ink at {now}ms: contrast {contrast}", group.lead.process.pid);
            self.previous_ink.push(PreviousCellInk { id: group.id.clone(), contrast, ended });
        }
    }
    self.invariant.check(buffer, grid, roster, now, self.fade);
    self.invariant.assert_valid();
    self.previous_live_cells = celled;
    }
}

fn row_contrast(buffer: &Buffer, area: Rect) -> f64 {
    let area = area.intersection(buffer.area);
    (area.y..area.bottom()).flat_map(|y| (area.x..area.right()).map(move |x| (x, y)))
        .filter_map(|position| {
            let cell = &buffer[position];
            if cell.symbol().trim().is_empty() { return None; }
            let foreground = relative_luminance(cell.fg);
            let background = relative_luminance(cell.bg);
            Some((foreground.max(background) + 0.05) / (foreground.min(background) + 0.05))
        }).fold(1.0, f64::max)
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
        if value <= 0.04045 { value / 12.92 } else { ((value + 0.055) / 1.055).powf(2.4) }
    };
    0.0722f64.mul_add(component(blue), 0.7152f64.mul_add(component(green), 0.2126 * component(red)))
}
