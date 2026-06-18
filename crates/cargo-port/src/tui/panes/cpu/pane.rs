use ratatui::Frame;
use ratatui::layout::Position;
use ratatui::layout::Rect;
use tui_pane::CpuMonitor;
use tui_pane::CpuUsage;
use tui_pane::Hittable;
use tui_pane::RenderFocus;
use tui_pane::Renderable;
use tui_pane::Viewport;

use crate::channel;
use crate::channel::Receiver;
use crate::config::CpuConfig;
use crate::tui::hit_test::HoverTarget;
use crate::tui::panes::PaneId;
use crate::tui::panes::RenderStyles;
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

    fn placeholder_cpu_usage(&self) -> CpuUsage {
        match self {
            Self::Active(monitor) => monitor.placeholder_cpu_usage(),
            Self::Inert(_) => CpuUsage::placeholder(0),
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

// ── Cpu ─────────────────────────────────────────────────────────
pub struct CpuPane {
    pub viewport:     Viewport,
    pub focus:        RenderFocus,
    content:          Option<CpuUsage>,
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
        let mut pane = Self {
            viewport: Viewport::new(),
            focus: RenderFocus::inactive(),
            content: None,
            monitor: CpuMonitorSlot::new(cpu_config, &startup_services),
            startup_services,
            row_rects: Vec::new(),
        };
        pane.install_placeholder();
        pane
    }

    pub fn tick(&mut self) {
        if let Some(usage) = self.monitor.latest() {
            self.content = Some(usage);
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
        self.install_placeholder();
    }

    pub fn install_placeholder(&mut self) {
        self.content = Some(self.monitor.placeholder_cpu_usage());
    }

    pub const fn content(&self) -> Option<&CpuUsage> { self.content.as_ref() }

    pub fn set_row_rects(&mut self, rects: Vec<(Rect, usize)>) { self.row_rects = rects; }

    pub fn clear_row_rects(&mut self) { self.row_rects.clear(); }
}

impl Renderable<PaneRenderCtx<'_>> for CpuPane {
    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, ctx: &PaneRenderCtx<'_>) {
        let styles = RenderStyles {
            readonly_label: ratatui::style::Style::default().fg(tui_pane::label_color()),
            chrome:         tui_pane::default_pane_chrome(),
        };
        cpu::render_cpu_pane_body(frame, area, self, &styles, ctx);
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
