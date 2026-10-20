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
    use tui_pane::Hittable;
    use tui_pane::PaneFrameChrome;

    use super::CpuMonitorSlot;
    use super::CpuPane;
    use super::CpuSamplePhase;
    use crate::channel;
    use crate::config::CpuConfig;
    use crate::tui::panes::cpu::render;
    use crate::tui::startup_services::StartupServices;

    fn quiet_pane() -> CpuPane {
        CpuPane::new(&CpuConfig::default(), StartupServices::quiet_unit_test())
    }

    #[allow(
        clippy::default_trait_access,
        reason = "CpuUsage exposes its GPU field, but tui_pane does not export the GPU type"
    )]
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
            gpu:           Default::default(),
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
        assert!(screen.contains("CPU suppressed"));
        assert!(!screen.contains('%'));
    }

    #[test]
    fn unsampled_phases_render_status_without_metrics_or_stale_hit_targets() {
        let mut pane = quiet_pane();
        for (phase, message) in [
            (
                CpuSamplePhase::AwaitingFirstSample { core_count: 8 },
                "Awaiting CPU sample",
            ),
            (CpuSamplePhase::Suppressed, "CPU suppressed"),
            (CpuSamplePhase::Unavailable, "CPU unavailable"),
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
                assert!(screen.contains("8 cores"));
                assert_eq!(pane.core_count(), 8);
            }
        }
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
        assert!(!draw(&mut pane).contains("Awaiting CPU sample"));

        // Disconnect sampling so a later tick has nothing new to replace the last real sample.
        let (_, samples) = channel::unbounded();
        pane.monitor = CpuMonitorSlot::Inert(samples);
        let previous = pane.core_count();
        pane.tick();
        assert!(matches!(pane.sample_phase(), CpuSamplePhase::Sampled(_)));
        assert_eq!(pane.core_count(), previous);
    }
}
