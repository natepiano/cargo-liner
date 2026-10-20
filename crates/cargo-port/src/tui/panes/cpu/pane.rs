use ratatui::Frame;
use ratatui::layout::Position;
use ratatui::layout::Rect;
use tui_pane::CpuMonitor;
use tui_pane::CpuUsage;
use tui_pane::Hittable;
use tui_pane::PaneFrameChrome;
use tui_pane::RenderFocus;
use tui_pane::Renderable;
use tui_pane::Viewport;

use crate::channel;
use crate::channel::Receiver;
use crate::config::CpuConfig;
use crate::tui::hit_test::HoverTarget;
use crate::tui::panes::PaneId;
use crate::tui::panes::cpu;
use crate::tui::render_context::PaneRenderCtx;
use crate::tui::startup_services::StartupEffect;
use crate::tui::startup_services::StartupServices;

enum CpuMonitorSlot {
    Active(CpuMonitor),
    Inert(Receiver<CpuUsage>),
}

impl CpuMonitorSlot {
    fn new(cpu_config: &CpuConfig, startup_services: &StartupServices) -> Self {
        let effect = startup_services.cpu_monitor_effect();
        startup_services.record_cpu_monitor(effect);
        match effect {
            StartupEffect::Real => Self::Active(CpuMonitor::new(cpu_config.poll_ms)),
            StartupEffect::Suppressed => {
                let (_sample_tx, samples) = channel::unbounded();
                Self::Inert(samples)
            },
        }
    }

    fn latest(&self) -> Option<CpuUsage> {
        match self {
            Self::Active(monitor) => monitor.latest(),
            Self::Inert(_) => None,
        }
    }

    const fn initial_sample_phase(&self) -> CpuSamplePhase {
        match self {
            Self::Active(monitor) if monitor.is_sampling() => CpuSamplePhase::AwaitingFirstSample {
                core_count: monitor.core_count(),
            },
            Self::Active(_) => CpuSamplePhase::Unavailable,
            Self::Inert(_) => CpuSamplePhase::Suppressed,
        }
    }

    const fn receiver(&self) -> &Receiver<CpuUsage> {
        match self {
            Self::Active(monitor) => monitor.receiver(),
            Self::Inert(samples) => samples,
        }
    }

    const fn is_sampling(&self) -> bool {
        match self {
            Self::Active(monitor) => monitor.is_sampling(),
            Self::Inert(_) => false,
        }
    }
}

/// Whether CPU monitoring has produced a real sample for this pane.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum CpuSamplePhase {
    AwaitingFirstSample {
        core_count: usize,
    },
    Sampled(CpuUsage),
    Suppressed,
    /// The monitor worker could not be started.
    Unavailable,
}

impl CpuSamplePhase {
    const fn core_count(&self) -> usize {
        match self {
            Self::AwaitingFirstSample { core_count } => *core_count,
            Self::Sampled(usage) => usage.cores.len(),
            Self::Suppressed | Self::Unavailable => 0,
        }
    }
}

// `CpuPane` owns CPU samples, focus, viewport and per-row hit-test rectangles.
pub struct CpuPane {
    pub viewport:     Viewport,
    pub focus:        RenderFocus,
    sample_phase:     CpuSamplePhase,
    monitor:          CpuMonitorSlot,
    startup_services: StartupServices,
    /// Per-rendered-row `(Rect, logical_row)` recorded each frame
    /// so `Hittable::hit_test_at` can map `pos` back to the logical
    /// row. CPU rows are non-uniform (aggregate, per-core,
    /// breakdown, GPU) so a flat `viewport.pos_to_local_row` won't
    /// work.
    row_rects:        Vec<(Rect, usize)>,
}

impl CpuPane {
    pub fn new(cpu_config: &CpuConfig, startup_services: StartupServices) -> Self {
        let monitor = CpuMonitorSlot::new(cpu_config, &startup_services);
        Self {
            viewport: Viewport::new(),
            focus: RenderFocus::inactive(),
            sample_phase: monitor.initial_sample_phase(),
            monitor,
            startup_services,
            row_rects: Vec::new(),
        }
    }

    pub fn tick(&mut self) {
        if let Some(usage) = self.monitor.latest() {
            self.sample_phase = CpuSamplePhase::Sampled(usage);
        }
    }

    /// The monitor's sample-channel receiver, for registering in the
    /// render-loop `Select` so a new CPU sample wakes the loop. Register
    /// only — `tick` remains the sole drain. Gate registration on
    /// [`is_sampling`](Self::is_sampling).
    pub const fn sample_rx(&self) -> &Receiver<CpuUsage> { self.monitor.receiver() }

    /// Whether the monitor's worker spawned and is producing samples.
    /// `false` means [`sample_rx`](Self::sample_rx) is disconnected and
    /// must not be registered in a `Select` (it would report permanently
    /// ready and busy-spin the loop).
    pub const fn is_sampling(&self) -> bool { self.monitor.is_sampling() }

    pub fn reset(&mut self, cpu_config: &CpuConfig) {
        self.monitor = CpuMonitorSlot::new(cpu_config, &self.startup_services);
        self.sample_phase = self.monitor.initial_sample_phase();
        self.viewport.clear_surface();
        self.clear_row_rects();
    }

    pub(super) const fn sample_phase(&self) -> &CpuSamplePhase { &self.sample_phase }

    pub const fn core_count(&self) -> usize { self.sample_phase.core_count() }

    pub fn set_row_rects(&mut self, rects: Vec<(Rect, usize)>) { self.row_rects = rects; }

    pub fn clear_row_rects(&mut self) { self.row_rects.clear(); }
}

impl Renderable<PaneRenderCtx<'_>> for CpuPane {
    fn render(
        &mut self,
        frame: &mut Frame<'_>,
        area: Rect,
        ctx: &PaneRenderCtx<'_>,
    ) -> Option<PaneFrameChrome> {
        Some(cpu::render_cpu_pane_body(frame, area, self, ctx))
    }
}

impl Hittable<HoverTarget> for CpuPane {
    fn hit_test_at(&self, pos: Position) -> Option<HoverTarget> {
        let (_rect, row) = self
            .row_rects
            .iter()
            .find(|(rect, _)| rect.contains(pos))
            .copied()?;
        Some(HoverTarget::PaneRow {
            pane: PaneId::Cpu,
            row,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::layout::Position;
    use ratatui::layout::Rect;
    use tui_pane::CpuBreakdown;
    use tui_pane::CpuCoreUsage;
    use tui_pane::CpuMonitor;
    use tui_pane::CpuUsage;
    use tui_pane::GpuUsage;
    use tui_pane::Hittable;
    use tui_pane::PaneBorders;
    use tui_pane::PaneFrameChrome;

    use super::CpuMonitorSlot;
    use super::CpuPane;
    use super::CpuSamplePhase;
    use crate::channel;
    use crate::config::CpuConfig;
    use crate::tui::panes::constants::CPU_PANE_WIDTH;
    use crate::tui::panes::cpu::render;
    use crate::tui::startup_services::StartupServices;

    const CPU_AWAITING_SHOT: &str = r"|┌ CPU / GPU ──────┐|
|│ CPU Cores    16 │|
|│ Sampling…       │|
|│                 │|
|│                 │|
|│                 │|
|│                 │|
|│                 │|
|│                 │|
|│                 │|
|│                 │|
|│                 │|
|│                 │|
|│                 │|
|│                 │|
|└─────────────────┘|
";

    const CPU_SAMPLED_SHOT: &str = r"|┌ CPU / GPU ──────┐|
|│ CPU Cores     8 │|
|│ Device      37% │|
|│ 1 ███       42% │|
|│ 2 █         17% │|
|│ 3 ███████   88% │|
|│ 4            0% │|
|│ 5 ████████ 100% │|
|│    1 of 2 ▼     │|
|├─────────────────┤|
|│ System      12% │|
|│ User        25% │|
|│ Idle        63% │|
|├─────────────────┤|
|│ GPU         n/a │|
|└─────────────────┘|
";

    const CPU_SUPPRESSED_SHOT: &str = r"|┌ CPU / GPU ──────┐|
|│ Monitoring off  │|
|│                 │|
|│                 │|
|│                 │|
|│                 │|
|│                 │|
|│                 │|
|│                 │|
|│                 │|
|│                 │|
|│                 │|
|│                 │|
|│                 │|
|│                 │|
|└─────────────────┘|
";

    const CPU_UNAVAILABLE_SHOT: &str = r"|┌ CPU / GPU ──────┐|
|│ Unavailable     │|
|│                 │|
|│                 │|
|│                 │|
|│                 │|
|│                 │|
|│                 │|
|│                 │|
|│                 │|
|│                 │|
|│                 │|
|│                 │|
|│                 │|
|│                 │|
|└─────────────────┘|
";

    fn quiet_pane() -> CpuPane {
        CpuPane::new(&CpuConfig::default(), StartupServices::quiet_unit_test())
    }

    fn sampled_usage() -> CpuUsage {
        CpuUsage {
            total_percent: 37,
            cores:         vec![CpuCoreUsage {
                label:                               "CPU 1".to_string(),
                percent:                             42,
                #[cfg(target_os = "macos")]
                cluster:                             None,
            }],
            breakdown:     CpuBreakdown {
                system: 12,
                user:   25,
                idle:   63,
            },
            gpu:           GpuUsage::default(),
        }
    }

    fn draw(pane: &mut CpuPane) -> String {
        let mut terminal = Terminal::new(TestBackend::new(24, 24)).unwrap();
        terminal
            .draw(|frame| {
                let area = frame.area();
                render::render_cpu_sample_phase(
                    frame,
                    tui_pane::frame_inner(area),
                    area,
                    pane,
                    &CpuConfig::default(),
                    &mut PaneFrameChrome::default(),
                );
            })
            .unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect()
    }

    fn draw_rows(pane: &mut CpuPane, height: u16) -> Vec<String> {
        let mut terminal = Terminal::new(TestBackend::new(CPU_PANE_WIDTH, height)).unwrap();
        terminal
            .draw(|frame| {
                let area = frame.area();
                render::render_cpu_sample_phase(
                    frame,
                    tui_pane::frame_inner(area),
                    area,
                    pane,
                    &CpuConfig::default(),
                    &mut PaneFrameChrome::default(),
                );
            })
            .unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .chunks(usize::from(CPU_PANE_WIDTH))
            .map(|row| row.iter().map(ratatui::buffer::Cell::symbol).collect())
            .collect()
    }

    fn usage_with_cores(count: usize, percent: u8) -> CpuUsage {
        let mut usage = sampled_usage();
        usage.cores[0].percent = percent;
        usage.cores = vec![usage.cores[0].clone(); count];
        usage
    }

    #[test]
    fn overflowing_cores_keep_the_pager_on_its_own_row() {
        let mut pane = quiet_pane();
        pane.sample_phase = CpuSamplePhase::Sampled(usage_with_cores(16, 42));
        for cursor in [0, 17] {
            pane.viewport.set_pos(cursor);
            let rows = draw_rows(&mut pane, 16);
            let pagers: Vec<usize> = rows
                .iter()
                .enumerate()
                .filter_map(|(index, row)| row.contains(" of ").then_some(index))
                .collect();
            assert_eq!(pagers.len(), 1, "one overflow pager: {rows:?}");
            let pager = pagers[0];
            assert!(!rows[pager].contains('%'), "pager covers a core: {rows:?}");
            assert!(
                rows[pager - 1].contains("42%"),
                "pager is not at the band edge: {rows:?}"
            );
            assert!(
                pane.row_rects
                    .iter()
                    .all(|(rect, _)| usize::from(rect.y) != pager)
            );
            let core_rows = pane
                .row_rects
                .iter()
                .filter(|(_, row)| (2..18).contains(row));
            for (rect, _) in core_rows {
                assert!(
                    rows[usize::from(rect.y)].contains("42%"),
                    "core bar was covered: {rows:?}"
                );
            }
            if cursor == 17 {
                assert!(rows.iter().any(|row| row.trim_start().starts_with("16 ")));
            }
        }
    }

    #[test]
    fn every_positive_core_usage_draws_a_visible_bar_cell() {
        let mut pane = quiet_pane();
        for percent in [0, 1, 17, 100] {
            pane.sample_phase = CpuSamplePhase::Sampled(usage_with_cores(1, percent));
            let rows = draw_rows(&mut pane, 24);
            let row = &rows[3];
            assert!(
                row.trim_start().starts_with("1 "),
                "first core row: {rows:?}"
            );
            assert_eq!(
                row.contains('█'),
                percent > 0,
                "core at {percent}%: {row:?}"
            );
        }
    }

    #[test]
    fn sampled_totals_use_labels_without_colons() {
        let mut pane = quiet_pane();
        pane.sample_phase = CpuSamplePhase::Sampled(sampled_usage());
        let rows = draw_rows(&mut pane, 24);
        for expected in ["System      12%", "User        25%", "Idle        63%"] {
            assert!(
                rows.iter().any(|row| row.trim() == expected),
                "missing {expected:?}: {rows:?}"
            );
        }
    }

    #[test]
    fn absent_gpu_data_uses_one_row_and_returns_space_to_cores() {
        let mut pane = quiet_pane();
        pane.sample_phase = CpuSamplePhase::Sampled(usage_with_cores(16, 42));
        let rows = draw_rows(&mut pane, 16);
        assert_eq!(
            rows.iter().filter(|row| row.contains("n/a")).count(),
            1,
            "{rows:?}"
        );
        assert!(
            rows.iter().any(|row| row.trim() == "GPU         n/a"),
            "{rows:?}"
        );
        assert_eq!(pane.viewport.len(), 22);
        let unavailable_core_rows = rows.iter().filter(|row| row.contains("42%")).count();
        assert_eq!(
            unavailable_core_rows, 5,
            "freed GPU rows must go to cores, with a separate pager: {rows:?}"
        );
        #[cfg(target_os = "macos")]
        {
            let mut usage = usage_with_cores(16, 42);
            usage.gpu.core_count = Some(38);
            pane.sample_phase = CpuSamplePhase::Sampled(usage);
            let available = draw_rows(&mut pane, 16);
            assert_eq!(pane.viewport.len(), 25);
            assert_eq!(
                available.iter().filter(|row| row.contains("42%")).count() + 3,
                unavailable_core_rows
            );
        }
    }

    #[test]
    fn phase_status_text_shares_the_metric_labels_left_edge() {
        let mut pane = quiet_pane();
        pane.sample_phase = CpuSamplePhase::Sampled(sampled_usage());
        let rows = draw_rows(&mut pane, 24);
        let label_column = Some(2);
        assert_eq!(rows[1].find("CPU Cores"), label_column);
        for (phase, word) in [
            (
                CpuSamplePhase::AwaitingFirstSample { core_count: 16 },
                "Sampling",
            ),
            (CpuSamplePhase::Suppressed, "Monitoring off"),
            (CpuSamplePhase::Unavailable, "Unavailable"),
        ] {
            pane.sample_phase = phase;
            let rows = draw_rows(&mut pane, 24);
            assert!(
                rows.iter().any(|row| row.find(word) == label_column),
                "status shifted: {rows:?}"
            );
        }
    }

    #[test]
    fn awaiting_sample_uses_a_single_cell_ellipsis() {
        let mut pane = quiet_pane();
        pane.sample_phase = CpuSamplePhase::AwaitingFirstSample { core_count: 16 };
        let rows = draw_rows(&mut pane, 24);
        assert!(rows.iter().any(|row| row.trim() == "Sampling…"), "{rows:?}");
        assert!(!rows.iter().any(|row| row.contains("...")));
    }

    #[test]
    fn cpu_phase_shots_match_the_framed_pane_at_its_fixed_width() {
        let mut pane = quiet_pane();
        let mut usage = usage_with_cores(8, 42);
        for (core, percent) in usage.cores.iter_mut().zip([42, 17, 88, 0, 100, 1, 25, 63]) {
            core.percent = percent;
        }
        for (phase, expected) in [
            (
                CpuSamplePhase::AwaitingFirstSample { core_count: 16 },
                CPU_AWAITING_SHOT,
            ),
            (CpuSamplePhase::Sampled(usage), CPU_SAMPLED_SHOT),
            (CpuSamplePhase::Suppressed, CPU_SUPPRESSED_SHOT),
            (CpuSamplePhase::Unavailable, CPU_UNAVAILABLE_SHOT),
        ] {
            pane.sample_phase = phase;
            let mut terminal = Terminal::new(TestBackend::new(CPU_PANE_WIDTH, 16)).unwrap();
            terminal
                .draw(|frame| {
                    let area = frame.area();
                    let pane_frame = tui_pane::PaneFrame::new(area);
                    let mut chrome = PaneFrameChrome::default();
                    render::render_cpu_sample_phase(
                        frame,
                        pane_frame.inner(),
                        area,
                        &mut pane,
                        &CpuConfig::default(),
                        &mut chrome,
                    );
                    let mut grid = tui_pane::GridLines::new(area);
                    grid.add_titled(pane_frame, " CPU / GPU ");
                    for rule in chrome.rules {
                        grid.add_rule(pane_frame, rule);
                    }
                    grid.render(
                        frame.buffer_mut(),
                        tui_pane::default_pane_chrome(),
                        PaneBorders::Separate,
                    );
                })
                .unwrap();
            let mut shot = String::new();
            for row in terminal
                .backend()
                .buffer()
                .content()
                .chunks(usize::from(CPU_PANE_WIDTH))
            {
                shot.push('|');
                for cell in row {
                    shot.push_str(cell.symbol());
                }
                shot.push_str("|\n");
            }
            assert_eq!(shot, expected, "phase {:?}", pane.sample_phase());
        }
    }

    #[test]
    fn suppressed_monitor_stays_suppressed_and_reset_discards_old_rows() {
        let mut pane = quiet_pane();
        assert_eq!(pane.sample_phase(), &CpuSamplePhase::Suppressed);
        assert!(!pane.is_sampling());
        pane.tick();
        assert_eq!(pane.sample_phase(), &CpuSamplePhase::Suppressed);

        pane.sample_phase = CpuSamplePhase::Sampled(sampled_usage());
        assert!(draw(&mut pane).contains("37%"));
        assert_ne!(pane.row_rects, [] as [(ratatui::prelude::Rect, usize); 0]);
        pane.reset(&CpuConfig::default());
        assert_eq!(pane.sample_phase(), &CpuSamplePhase::Suppressed);
        assert_eq!(pane.row_rects, [] as [(ratatui::prelude::Rect, usize); 0]);
        let screen = draw(&mut pane);
        assert!(screen.contains(" Monitoring off"));
        assert!(!screen.contains('%'));
    }

    #[test]
    fn unsampled_phases_render_status_without_metrics_or_stale_hit_targets() {
        let mut pane = quiet_pane();
        for (phase, message) in [
            (
                CpuSamplePhase::AwaitingFirstSample { core_count: 8 },
                " Sampling…",
            ),
            (CpuSamplePhase::Suppressed, " Monitoring off"),
            (CpuSamplePhase::Unavailable, " Unavailable"),
        ] {
            pane.sample_phase = phase;
            pane.set_row_rects(vec![(Rect::new(1, 1, 10, 1), 1)]);
            let screen = draw(&mut pane);
            assert!(screen.contains(message));
            assert!(!screen.contains('%'));
            assert_eq!(pane.row_rects, [] as [(ratatui::prelude::Rect, usize); 0]);
            assert!(pane.hit_test_at(Position::new(1, 1)).is_none());
            if matches!(
                pane.sample_phase(),
                CpuSamplePhase::AwaitingFirstSample { .. }
            ) {
                assert!(screen.contains("CPU Cores"));
                assert_eq!(pane.core_count(), 8);
            }
        }
    }

    #[test]
    fn every_sample_phase_renders_complete_lines_at_cpu_pane_width() {
        let mut pane = quiet_pane();
        let mut usage = sampled_usage();
        usage.cores = vec![usage.cores[0].clone(); 16];
        let mut core_rows = Vec::new();
        for (phase, expected_lines) in [
            (
                CpuSamplePhase::AwaitingFirstSample { core_count: 16 },
                &["CPU Cores    16", "Sampling…"][..],
            ),
            (
                CpuSamplePhase::Sampled(usage),
                &["CPU Cores    16", "Device      37%"][..],
            ),
            (CpuSamplePhase::Suppressed, &["Monitoring off"][..]),
            (CpuSamplePhase::Unavailable, &["Unavailable"][..]),
        ] {
            pane.sample_phase = phase;
            let rows = draw_rows(&mut pane, 24);
            if matches!(
                pane.sample_phase(),
                CpuSamplePhase::AwaitingFirstSample { .. } | CpuSamplePhase::Sampled(_)
            ) {
                core_rows.push(rows[1].clone());
            }
            for expected in expected_lines {
                assert!(
                    rows.iter().any(|row| row.trim() == *expected),
                    "missing complete line {expected:?} for {:?}: {rows:?}",
                    pane.sample_phase(),
                );
            }
        }
        assert_eq!(
            core_rows[0], core_rows[1],
            "first sample moved the core-count row"
        );
    }

    #[test]
    fn tick_receives_real_sample_and_retains_it_until_the_next_sample() {
        let mut pane = quiet_pane();
        let monitor = CpuMonitorSlot::Active(CpuMonitor::new(1));
        assert!(monitor.is_sampling());
        pane.sample_phase = monitor.initial_sample_phase();
        assert!(
            matches!(pane.sample_phase(), CpuSamplePhase::AwaitingFirstSample { core_count } if *core_count > 0)
        );
        // Readiness observes a sample without consuming it: tick is the sole drain.
        pane.monitor = monitor;
        let mut select = crossbeam_channel::Select::new();
        select.recv(pane.sample_rx());
        assert!(
            select.ready_timeout(Duration::from_secs(5)).is_ok(),
            "CPU sample ready"
        );
        drop(select);
        pane.tick();
        assert!(matches!(pane.sample_phase(), CpuSamplePhase::Sampled(_)));
        assert!(pane.core_count() > 0);
        assert!(!draw(&mut pane).contains("Sampling…"));

        // Disconnect sampling so a later tick has nothing new to replace the last real sample.
        let (_, samples) = channel::unbounded();
        pane.monitor = CpuMonitorSlot::Inert(samples);
        let previous = pane.core_count();
        pane.tick();
        assert!(matches!(pane.sample_phase(), CpuSamplePhase::Sampled(_)));
        assert_eq!(pane.core_count(), previous);
    }
}
