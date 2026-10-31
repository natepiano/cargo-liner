use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::text::Text;
use ratatui::widgets::Paragraph;
#[cfg(target_os = "macos")]
use tui_pane::CoreCluster;
use tui_pane::CpuUsage;
use tui_pane::GpuUsage;
use tui_pane::PaneFocusState;
use tui_pane::PaneFrameChrome;
use tui_pane::Region;
use tui_pane::Size;
use tui_pane::Viewport;
use tui_pane::ViewportOverflow;
use tui_pane::accent_color;
use tui_pane::error_color;
use tui_pane::label_color;
use tui_pane::text_default;
use tui_pane::warning_color;

use super::CpuPane;
use super::pane::CpuSamplePhase;
use crate::config::CpuConfig;
use crate::tui::panes::constants::CPU_BAR_WIDTH;
use crate::tui::panes::constants::CPU_BREAKDOWN_ROWS;
use crate::tui::panes::constants::CPU_GPU_ROWS;
use crate::tui::panes::constants::CPU_PINNED_HEAD_ROWS;
use crate::tui::panes::constants::CPU_STATIC_INNER_HEIGHT;
use crate::tui::render_context::PaneRenderCtx;

/// The GPU section expands only when the platform has metrics to show.
#[derive(Clone, Copy)]
enum GpuRows {
    Unavailable,
    Metrics,
}

impl From<GpuUsage> for GpuRows {
    fn from(gpu: GpuUsage) -> Self {
        #[cfg(target_os = "macos")]
        let has_metrics = gpu.core_count.is_some()
            || gpu.device_percent.is_some()
            || gpu.renderer_percent.is_some()
            || gpu.tiler_percent.is_some();
        #[cfg(not(target_os = "macos"))]
        let has_metrics = gpu.device_percent.is_some();
        if has_metrics {
            Self::Metrics
        } else {
            Self::Unavailable
        }
    }
}

impl GpuRows {
    const fn count(self) -> usize {
        match self {
            Self::Unavailable => 1,
            Self::Metrics => CPU_GPU_ROWS,
        }
    }
}

fn cpu_required_inner_height(core_count: usize) -> u16 {
    let core_rows = u16::try_from(core_count).unwrap_or(u16::MAX);
    CPU_STATIC_INNER_HEIGHT.saturating_add(core_rows)
}

pub fn cpu_required_pane_height(core_count: usize) -> u16 {
    cpu_required_inner_height(core_count).saturating_add(2)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CpuSelectableRow {
    CoreCount,
    Total,
    Core(usize),
    System,
    User,
    Idle,
    #[cfg(target_os = "macos")]
    GpuCores,
    #[cfg(target_os = "macos")]
    GpuDevice,
    #[cfg(target_os = "macos")]
    GpuRenderer,
    #[cfg(target_os = "macos")]
    GpuTiler,
    Gpu,
}

impl CpuSelectableRow {
    const fn logical_index(self, core_count: usize) -> usize {
        match self {
            Self::CoreCount => 0,
            Self::Total => 1,
            Self::Core(index) => index + 2,
            Self::System => core_count + 2,
            Self::User => core_count + 3,
            Self::Idle => core_count + 4,
            #[cfg(target_os = "macos")]
            Self::GpuCores => core_count + 5,
            #[cfg(target_os = "macos")]
            Self::GpuDevice => core_count + 6,
            #[cfg(target_os = "macos")]
            Self::GpuRenderer => core_count + 7,
            #[cfg(target_os = "macos")]
            Self::GpuTiler => core_count + 8,
            Self::Gpu => core_count + 5,
        }
    }
}

/// One scrolling core row: the usage bar with the 1-based core number
/// embedded in the bar's leftmost cells, right-aligned to `number_width` and
/// followed by a one-cell margin before the block glyphs. Digits render in
/// the default text color; the fill behind the number and margin is drawn as
/// background shading, so the bar still spans its full width.
///
/// On macOS a `P` / `E` cluster marker follows the core number. The cluster
/// map is all-or-nothing, so either every row carries a marker or none do and
/// the number field keeps one width across the pane.
fn cpu_bar_line(
    core_number: usize,
    number_width: usize,
    percent: u8,
    #[cfg(target_os = "macos")] core_cluster: Option<CoreCluster>,
    cpu_cfg: &CpuConfig,
) -> Line<'static> {
    let filled = tui_pane::cpu_filled_cells(percent);
    let severity = tui_pane::cpu_severity(
        percent,
        cpu_cfg.low_utilization_max_percent,
        cpu_cfg.medium_utilization_max_percent,
    )
    .color();
    #[cfg(target_os = "macos")]
    let number_text = core_cluster.map_or_else(
        || format!("{core_number:>number_width$} "),
        |core_cluster| {
            let marker = match core_cluster {
                CoreCluster::Performance => "P",
                CoreCluster::Efficiency => "E",
            };
            format!("{core_number:>number_width$} {marker} ")
        },
    );
    #[cfg(not(target_os = "macos"))]
    let number_text = format!("{core_number:>number_width$} ");
    // The embedded label consumes bar cells. Positive usage must still leave
    // at least one visible block after it, even when rounding falls in the label.
    let filled = match percent {
        0 => 0,
        _ => filled.max(number_text.len() + 1).min(CPU_BAR_WIDTH),
    };
    let (number_on_filled, number_past_fill) = number_text.split_at(filled.min(number_text.len()));
    let number_filled_span = Span::styled(
        number_on_filled.to_string(),
        Style::default().fg(text_default()).bg(severity),
    );
    let number_empty_span = Span::styled(
        number_past_fill.to_string(),
        Style::default().fg(text_default()),
    );
    let filled_span = Span::styled(
        "█".repeat(filled.saturating_sub(number_text.len())),
        Style::default().fg(severity),
    );
    let empty_span = Span::styled(
        " ".repeat(CPU_BAR_WIDTH.saturating_sub(filled.max(number_text.len()))),
        Style::default().fg(tui_pane::cpu_blank_bar_color()),
    );
    let percent_span = Span::raw(format!("{percent:>3}%"));
    Line::from(vec![
        Span::raw(" "),
        number_filled_span,
        number_empty_span,
        filled_span,
        empty_span,
        Span::raw(" "),
        percent_span,
        Span::raw(" "),
    ])
}

/// Availability of one metric reported by the GPU monitor.
enum GpuMetric<T> {
    Available(T),
    Missing,
}

impl<T> From<Option<T>> for GpuMetric<T> {
    fn from(value: Option<T>) -> Self { value.map_or_else(|| Self::Missing, Self::Available) }
}

/// Single-line GPU percent row. Missing metrics keep their label and show
/// a dim en dash in the value column.
fn gpu_metric_line(
    label: &str,
    percent: Option<u8>,
    cpu_cfg: &CpuConfig,
    width: u16,
) -> Line<'static> {
    let value = match GpuMetric::from(percent) {
        GpuMetric::Missing => Span::styled("–", Style::default().add_modifier(Modifier::DIM)),
        GpuMetric::Available(percent) => {
            let severity = tui_pane::cpu_severity(
                percent,
                cpu_cfg.low_utilization_max_percent,
                cpu_cfg.medium_utilization_max_percent,
            )
            .color();
            Span::styled(format!("{percent:>3}%"), Style::default().fg(severity))
        },
    };
    value_span_line(label, value, width)
}

#[cfg(target_os = "macos")]
fn gpu_core_count_line(core_count: Option<u16>, width: u16) -> Line<'static> {
    let value = match GpuMetric::from(core_count) {
        GpuMetric::Missing => Span::styled("–", Style::default().add_modifier(Modifier::DIM)),
        GpuMetric::Available(core_count) => {
            Span::styled(core_count.to_string(), Style::default().fg(text_default()))
        },
    };
    value_span_line("GPU cores", value, width)
}

/// Pinned `CPU cores` head row: the live CPU core count.
fn cpu_core_count_line(core_count: usize, width: u16) -> Line<'static> {
    value_line("CPU cores", core_count.to_string(), text_default(), width)
}

/// Pinned `Total` head row: aggregate CPU utilization.
fn total_line(percent: u8, width: u16) -> Line<'static> {
    value_line("Total", format!("{percent:>3}%"), text_default(), width)
}

/// One `label  value` row: the label in label color on the left, the value
/// right-aligned in `value_color`, padded to `width`. No trailing colon.
fn value_line(label: &str, value_text: String, value_color: Color, width: u16) -> Line<'static> {
    value_span_line(
        label,
        Span::styled(value_text, Style::default().fg(value_color)),
        width,
    )
}

fn value_span_line(label: &str, value: Span<'static>, width: u16) -> Line<'static> {
    let space_count = usize::from(width)
        .saturating_sub(label.len().saturating_add(value.width()).saturating_add(2));
    Line::from(vec![
        Span::raw(" "),
        Span::styled(label.to_string(), Style::default().fg(label_color())),
        Span::raw(" ".repeat(space_count)),
        value,
        Span::raw(" "),
    ])
}

/// System / user / idle breakdown row: a label in label color with the
/// percent right-aligned in its fixed `color`.
fn metric_line(label: &str, percent: u8, color: Color, width: u16) -> Line<'static> {
    value_line(label, format!("{percent:>3}%"), color, width)
}

/// The CPU pane's box tree: a pinned aggregate row, the scrolling cores band
/// (the one `Fill` box), the System/User/Idle breakdown (a rule above it), and
/// the GPU row (a rule above it). Rebuilt each frame from the live core count.
fn cpu_region(core_count: usize, gpu_rows: GpuRows, inner_height: u16) -> Region {
    let pinned_rows = CPU_PINNED_HEAD_ROWS + CPU_BREAKDOWN_ROWS + gpu_rows.count() + 2;
    let band_height = usize::from(inner_height).saturating_sub(pinned_rows);
    let cores = Region::rows(core_count, Size::Fill);
    // Leave at least one bar visible in very short panes. Otherwise reserve
    // a footer only when the cores overflow; it never owns a selectable row.
    let cores = if core_count > band_height && band_height > 1 {
        cores.footer()
    } else {
        cores
    };
    Region::stack(vec![
        Region::rows(CPU_PINNED_HEAD_ROWS, Size::Fixed),
        cores,
        Region::rows(CPU_BREAKDOWN_ROWS, Size::Fixed).rule(),
        Region::rows(gpu_rows.count(), Size::Fixed).rule(),
    ])
}

/// Resolved rects for one CPU frame. The `CPU cores` count and `Total`
/// utilization rows are pinned at the top; the cores band is the `Fill` box
/// and scrolls; the breakdown rows and GPU follow. `band_offset` is the cores
/// band's scroll position, held across frames so the cursor stays visible.
struct CpuPanelLayout {
    core_count:     usize,
    core_count_row: Rect,
    total:          Rect,
    cores:          Rect,
    cores_divider:  Rect,
    system:         Rect,
    user:           Rect,
    idle:           Rect,
    gpu_divider:    Rect,
    gpu:            Rect,
    gpu_rows:       GpuRows,
    cores_footer:   Rect,
    band_offset:    usize,
}

impl CpuPanelLayout {
    fn new(
        inner: Rect,
        core_count: usize,
        gpu_rows: GpuRows,
        cursor_pos: usize,
        prior_offset: usize,
    ) -> Self {
        // `cpu_region(...).place` applies `prior_offset` only to region 1, the core band;
        // the pinned regions never scroll.
        let placed = cpu_region(core_count, gpu_rows, inner.height).place(
            inner,
            cursor_pos,
            &[0, prior_offset, 0, 0],
        );
        let head = placed[0].content;
        let head_row = |offset: u16| Rect {
            y: head.y.saturating_add(offset),
            height: 1,
            ..head
        };
        let breakdown = placed[2].content;
        let breakdown_row = |offset: u16| Rect {
            y: breakdown.y.saturating_add(offset),
            height: 1,
            ..breakdown
        };
        Self {
            core_count,
            core_count_row: head_row(0),
            total: head_row(1),
            cores: placed[1].content,
            cores_divider: placed[2].chrome,
            system: breakdown_row(0),
            user: breakdown_row(1),
            idle: breakdown_row(2),
            gpu_divider: placed[3].chrome,
            gpu: placed[3].content,
            gpu_rows,
            cores_footer: placed[1].footer,
            band_offset: placed[1].scroll_offset,
        }
    }

    /// Number of core rows visible in the band this frame.
    fn band_visible(&self) -> usize { usize::from(self.cores.height) }
}

#[derive(Clone, Copy)]
struct BreakdownRowSpec<'a> {
    area:        Rect,
    logical_row: usize,
    label:       &'a str,
    percent:     u8,
    color:       Color,
}

const fn cpu_panel_title() -> &'static str { " CPU / GPU " }

fn cpu_row_overlay_style(viewport: &Viewport, logical_row: usize, focus: PaneFocusState) -> Style {
    tui_pane::selection_state(viewport, logical_row, focus).overlay_style()
}

fn render_selectable_row(
    frame: &mut Frame,
    viewport: &Viewport,
    row_rects: &mut Vec<(Rect, usize)>,
    area: Rect,
    logical_row: usize,
    focus: PaneFocusState,
    paragraph: Paragraph<'static>,
) {
    frame.render_widget(
        paragraph.style(cpu_row_overlay_style(viewport, logical_row, focus)),
        area,
    );
    row_rects.push((area, logical_row));
}

/// Report the band dividers, full pane width so the grid pass tees each
/// one into the side borders it runs into.
fn push_cpu_dividers(area: Rect, layout: &CpuPanelLayout, chrome: &mut PaneFrameChrome) {
    chrome.rules.extend([
        Rect {
            x:      area.x,
            y:      layout.cores_divider.y,
            width:  area.width,
            height: 1,
        },
        Rect {
            x:      area.x,
            y:      layout.gpu_divider.y,
            width:  area.width,
            height: 1,
        },
    ]);
}

fn render_head_rows(
    frame: &mut Frame,
    viewport: &Viewport,
    row_rects: &mut Vec<(Rect, usize)>,
    usage: &CpuUsage,
    layout: &CpuPanelLayout,
    focus: PaneFocusState,
) {
    render_selectable_row(
        frame,
        viewport,
        row_rects,
        layout.core_count_row,
        CpuSelectableRow::CoreCount.logical_index(layout.core_count),
        focus,
        Paragraph::new(cpu_core_count_line(
            usage.cores.len(),
            layout.core_count_row.width,
        )),
    );
    render_selectable_row(
        frame,
        viewport,
        row_rects,
        layout.total,
        CpuSelectableRow::Total.logical_index(layout.core_count),
        focus,
        Paragraph::new(total_line(usage.total_percent, layout.total.width)),
    );
}

fn render_core_rows(
    frame: &mut Frame,
    viewport: &Viewport,
    row_rects: &mut Vec<(Rect, usize)>,
    cpu_config: &CpuConfig,
    usage: &CpuUsage,
    layout: &CpuPanelLayout,
    focus: PaneFocusState,
) {
    let end = layout
        .band_offset
        .saturating_add(layout.band_visible())
        .min(usage.cores.len());
    let number_width = layout.core_count.to_string().len();
    for (slot, core_index) in (layout.band_offset..end).enumerate() {
        let cpu_core_usage = &usage.cores[core_index];
        let logical_row = CpuSelectableRow::Core(core_index).logical_index(layout.core_count);
        let area = Rect {
            x:      layout.cores.x,
            y:      layout
                .cores
                .y
                .saturating_add(u16::try_from(slot).unwrap_or(u16::MAX)),
            width:  layout.cores.width,
            height: 1,
        };
        render_selectable_row(
            frame,
            viewport,
            row_rects,
            area,
            logical_row,
            focus,
            Paragraph::new(cpu_bar_line(
                core_index + 1,
                number_width,
                cpu_core_usage.percent,
                #[cfg(target_os = "macos")]
                cpu_core_usage.cluster,
                cpu_config,
            )),
        );
    }
}

fn render_breakdown_row(
    frame: &mut Frame,
    viewport: &Viewport,
    row_rects: &mut Vec<(Rect, usize)>,
    focus: PaneFocusState,
    breakdown_row_spec: BreakdownRowSpec<'_>,
) {
    render_selectable_row(
        frame,
        viewport,
        row_rects,
        breakdown_row_spec.area,
        breakdown_row_spec.logical_row,
        focus,
        Paragraph::new(metric_line(
            breakdown_row_spec.label,
            breakdown_row_spec.percent,
            breakdown_row_spec.color,
            breakdown_row_spec.area.width,
        )),
    );
}

fn render_gpu_rows(
    frame: &mut Frame,
    viewport: &Viewport,
    row_rects: &mut Vec<(Rect, usize)>,
    cpu_cfg: &CpuConfig,
    usage: &CpuUsage,
    layout: &CpuPanelLayout,
    focus: PaneFocusState,
) {
    match layout.gpu_rows {
        GpuRows::Unavailable => render_selectable_row(
            frame,
            viewport,
            row_rects,
            layout.gpu,
            CpuSelectableRow::Gpu.logical_index(layout.core_count),
            focus,
            Paragraph::new(value_line(
                "GPU",
                "unavailable".to_string(),
                warning_color(),
                layout.gpu.width,
            )),
        ),
        GpuRows::Metrics => {
            render_gpu_metrics(frame, viewport, row_rects, cpu_cfg, usage, layout, focus);
        },
    }
}

#[cfg(target_os = "macos")]
fn render_gpu_metrics(
    frame: &mut Frame,
    viewport: &Viewport,
    row_rects: &mut Vec<(Rect, usize)>,
    cpu_cfg: &CpuConfig,
    usage: &CpuUsage,
    layout: &CpuPanelLayout,
    focus: PaneFocusState,
) {
    let gpu = usage.gpu;
    let gpu_row = |offset: u16| Rect {
        y: layout.gpu.y.saturating_add(offset),
        height: 1,
        ..layout.gpu
    };
    let mut render = |row: CpuSelectableRow, area: Rect, line: Line<'static>| {
        render_selectable_row(
            frame,
            viewport,
            row_rects,
            area,
            row.logical_index(layout.core_count),
            focus,
            Paragraph::new(line),
        );
    };
    render(
        CpuSelectableRow::GpuCores,
        gpu_row(0),
        gpu_core_count_line(gpu.core_count, layout.gpu.width),
    );
    render(
        CpuSelectableRow::GpuDevice,
        gpu_row(1),
        gpu_metric_line("Device", gpu.device_percent, cpu_cfg, layout.gpu.width),
    );
    render(
        CpuSelectableRow::GpuRenderer,
        gpu_row(2),
        gpu_metric_line("Renderer", gpu.renderer_percent, cpu_cfg, layout.gpu.width),
    );
    render(
        CpuSelectableRow::GpuTiler,
        gpu_row(3),
        gpu_metric_line("Tiler", gpu.tiler_percent, cpu_cfg, layout.gpu.width),
    );
}

/// Single aggregate `GPU` row on platforms that expose only device
/// utilization (Linux, Windows).
#[cfg(not(target_os = "macos"))]
fn render_gpu_metrics(
    frame: &mut Frame,
    viewport: &Viewport,
    row_rects: &mut Vec<(Rect, usize)>,
    cpu_cfg: &CpuConfig,
    usage: &CpuUsage,
    layout: &CpuPanelLayout,
    focus: PaneFocusState,
) {
    render_selectable_row(
        frame,
        viewport,
        row_rects,
        layout.gpu,
        CpuSelectableRow::Gpu.logical_index(layout.core_count),
        focus,
        Paragraph::new(gpu_metric_line(
            "GPU",
            usage.gpu.device_percent,
            cpu_cfg,
            layout.gpu.width,
        )),
    );
}

const fn sync_cpu_pane_state(viewport: &mut Viewport, inner: Rect, layout: &CpuPanelLayout) {
    viewport.set_len(
        CPU_PINNED_HEAD_ROWS + layout.core_count + CPU_BREAKDOWN_ROWS + layout.gpu_rows.count(),
    );
    viewport.set_content_area(inner);
    viewport.set_scroll_offset(layout.band_offset);
}

fn render_cpu_metric_rows(
    frame: &mut Frame,
    viewport: &Viewport,
    row_rects: &mut Vec<(Rect, usize)>,
    cpu_config: &CpuConfig,
    usage: &CpuUsage,
    layout: &CpuPanelLayout,
    focus: PaneFocusState,
) {
    render_head_rows(frame, viewport, row_rects, usage, layout, focus);
    render_core_rows(frame, viewport, row_rects, cpu_config, usage, layout, focus);
    render_breakdown_row(
        frame,
        viewport,
        row_rects,
        focus,
        BreakdownRowSpec {
            area:        layout.system,
            logical_row: CpuSelectableRow::System.logical_index(layout.core_count),
            label:       "System",
            percent:     usage.breakdown.system,
            color:       error_color(),
        },
    );
    render_breakdown_row(
        frame,
        viewport,
        row_rects,
        focus,
        BreakdownRowSpec {
            area:        layout.user,
            logical_row: CpuSelectableRow::User.logical_index(layout.core_count),
            label:       "User",
            percent:     usage.breakdown.user,
            color:       accent_color(),
        },
    );
    render_breakdown_row(
        frame,
        viewport,
        row_rects,
        focus,
        BreakdownRowSpec {
            area:        layout.idle,
            logical_row: CpuSelectableRow::Idle.logical_index(layout.core_count),
            label:       "Idle",
            percent:     usage.breakdown.idle,
            color:       text_default(),
        },
    );
    render_gpu_rows(frame, viewport, row_rects, cpu_config, usage, layout, focus);
}

/// Body of `CpuPane::render`. Lives here (next to its helpers)
/// rather than inline in `pane.rs` because the helpers
/// belong with the per-pane render code; only the trait method
/// itself sits in `pane.rs` and delegates here.
pub(super) fn render_cpu_pane_body(
    frame: &mut Frame,
    area: Rect,
    pane: &mut CpuPane,
    pane_render_ctx: &PaneRenderCtx<'_>,
) -> PaneFrameChrome {
    let inner = tui_pane::frame_inner(area);
    let mut chrome = PaneFrameChrome {
        title: cpu_panel_title().to_string(),
        focused: pane.focus.is_focused(),
        ..PaneFrameChrome::default()
    };

    if inner.height == 0 {
        pane.viewport.clear_surface();
        pane.clear_row_rects();
        return chrome;
    }

    render_cpu_sample_phase(
        frame,
        inner,
        area,
        pane,
        &pane_render_ctx.config.current().cpu,
        &mut chrome,
    );
    chrome
}

pub(super) fn render_cpu_sample_phase(
    frame: &mut Frame,
    inner: Rect,
    area: Rect,
    pane: &mut CpuPane,
    cpu_cfg: &CpuConfig,
    chrome: &mut PaneFrameChrome,
) {
    let usage = match pane.sample_phase() {
        CpuSamplePhase::AwaitingFirstSample { core_count } => {
            let lines = vec![
                cpu_core_count_line(*core_count, inner.width),
                Line::from(" Sampling…"),
            ];
            render_cpu_status(frame, inner, pane, lines);
            return;
        },
        CpuSamplePhase::Suppressed => {
            render_cpu_status(frame, inner, pane, " Monitoring off");
            return;
        },
        CpuSamplePhase::Unavailable => {
            render_cpu_status(frame, inner, pane, " Unavailable");
            return;
        },
        CpuSamplePhase::Sampled(usage) => usage,
    };
    let core_count = usage.cores.len();
    let cursor_pos = pane.viewport.pos();
    let layout = CpuPanelLayout::new(
        inner,
        core_count,
        GpuRows::from(usage.gpu),
        cursor_pos,
        pane.viewport.scroll_offset(),
    );

    push_cpu_dividers(area, &layout, chrome);

    let mut row_rects: Vec<(Rect, usize)> = Vec::new();
    let viewport = &pane.viewport;
    render_cpu_metric_rows(
        frame,
        viewport,
        &mut row_rects,
        cpu_cfg,
        usage,
        &layout,
        pane.focus.pane_focus_state,
    );
    render_cores_affordance(frame, &layout, cursor_pos);
    sync_cpu_pane_state(&mut pane.viewport, inner, &layout);
    pane.set_row_rects(row_rects);
}

fn render_cpu_status(
    frame: &mut Frame,
    inner: Rect,
    pane: &mut CpuPane,
    message: impl Into<Text<'static>>,
) {
    pane.viewport.clear_surface();
    pane.clear_row_rects();
    frame.render_widget(
        Paragraph::new(message.into()).style(Style::default().fg(label_color())),
        inner,
    );
}

/// Draw the `▲ n of m ▼` overflow label in the cores footer when more cores
/// exist than fit. Skipped when the band has no visible rows.
fn render_cores_affordance(frame: &mut Frame, layout: &CpuPanelLayout, cursor_pos: usize) {
    let band_visible = layout.band_visible();
    if band_visible == 0 {
        return;
    }
    // `band_cursor` maps `cursor_pos` in [CPU_PINNED_HEAD_ROWS, CPU_PINNED_HEAD_ROWS +
    // layout.core_count) to its core, otherwise falling back to `layout.band_offset` for the pager
    // anchor.
    let band_cursor = cursor_pos
        .checked_sub(CPU_PINNED_HEAD_ROWS)
        .filter(|local| *local < layout.core_count)
        .unwrap_or(layout.band_offset);
    let overflow = ViewportOverflow::new(
        layout.core_count,
        layout.band_offset,
        band_visible,
        band_cursor,
    );
    if let Some(label) = overflow.label() {
        frame.render_widget(
            Paragraph::new(format!(" {label}")).style(Style::default().fg(label_color())),
            layout.cores_footer,
        );
    }
}

#[cfg(test)]
mod tests {
    use ratatui::layout::Rect;
    use ratatui::text::Line;
    #[cfg(target_os = "macos")]
    use tui_pane::CoreCluster;
    use tui_pane::text_default;

    use super::GpuRows;
    use super::cpu_bar_line;
    use super::cpu_region;
    #[cfg(target_os = "macos")]
    use super::gpu_core_count_line;
    use super::gpu_metric_line;
    use crate::config::CpuConfig;
    use crate::tui::panes::constants::CPU_CONTENT_WIDTH;

    // The metrics GPU section pins 11 rows on macOS, 8 elsewhere. At height
    // 14 the remaining band reserves one pager row, leaving 2 or 5 core rows.
    /// Build a core row with the default config, supplying the macOS-only
    /// cluster argument as unmapped so the bar assertions stay platform-neutral.
    fn bar_line_for_test(core_number: usize, number_width: usize, percent: u8) -> Line<'static> {
        cpu_bar_line(
            core_number,
            number_width,
            percent,
            #[cfg(target_os = "macos")]
            None,
            &CpuConfig::default(),
        )
    }

    fn cores_offset(inner_height: u16, cursor: usize, prior: usize) -> usize {
        let inner = Rect {
            x:      0,
            y:      0,
            width:  20,
            height: inner_height,
        };
        cpu_region(15, GpuRows::Metrics, inner.height).place(inner, cursor, &[0, prior, 0, 0])[1]
            .scroll_offset
    }

    #[test]
    fn band_offset_tracks_cursor_inside_the_band() {
        // `cores_offset(14, 14, 0)` places logical cursor 14 (core-band row 12) on the last full
        // page.
        #[cfg(target_os = "macos")]
        assert_eq!(cores_offset(14, 14, 0), 11);
        #[cfg(not(target_os = "macos"))]
        assert_eq!(cores_offset(14, 14, 0), 8);
    }

    #[test]
    fn band_offset_holds_prior_on_a_pinned_head_row() {
        // `cores_offset(14, 0, 7)` retains prior offset 7 because logical cursor 0 is the pinned
        // core-count row outside the band.
        assert_eq!(cores_offset(14, 0, 7), 7);
    }

    #[test]
    fn band_offset_holds_prior_on_a_pinned_tail_row() {
        // `cores_offset(14, 17, 20)` keeps the prior offset for breakdown cursor 17 outside the
        // band, clamped to its last page (15 minus visible core rows).
        #[cfg(target_os = "macos")]
        assert_eq!(cores_offset(14, 17, 20), 13);
        #[cfg(not(target_os = "macos"))]
        assert_eq!(cores_offset(14, 17, 20), 10);
    }

    #[test]
    fn band_offset_is_zero_when_every_core_fits() {
        // `cores_offset` returns zero when the core band fits all 15 cores, regardless of cursor.
        #[cfg(target_os = "macos")]
        assert_eq!(cores_offset(26, 14, 0), 0);
        #[cfg(not(target_os = "macos"))]
        assert_eq!(cores_offset(23, 14, 0), 0);
    }

    // cpu_bar_line spans: [space, number-on-filled, number-past-fill,
    // filled bar, empty bar, space, percent, space]. The number text
    // carries a one-cell margin before the bar's block glyphs.

    #[test]
    fn core_number_renders_as_text_on_the_filled_bar() {
        // `bar_line_for_test(12, 2, 35)` fills 4 of 10 cells: the two-column number and margin fit
        // entirely on shaded fill, leaving one block glyph in `line.spans[3]`.
        let line = bar_line_for_test(12, 2, 35);
        assert_eq!(line.spans[1].content, "12 ");
        assert_eq!(line.spans[1].style.fg, Some(text_default()));
        assert!(line.spans[1].style.bg.is_some());
        assert_eq!(line.spans[2].content, "");
        assert_eq!(line.spans[3].content, "█");
    }

    #[test]
    fn low_usage_still_fills_beyond_the_core_number() {
        // Low usage fills the embedded number and its margin, plus one
        // visible block. It must not disappear behind the number.
        let line = bar_line_for_test(12, 2, 10);
        assert_eq!(line.spans[1].content, "12 ");
        assert_eq!(line.spans[2].content, "");
        assert_eq!(line.spans[2].style.fg, Some(text_default()));
        assert_eq!(line.spans[2].style.bg, None);
        assert_eq!(line.spans[3].content, "█");
    }

    #[test]
    fn single_digit_core_number_is_right_aligned() {
        let line = bar_line_for_test(3, 2, 0);
        assert_eq!(line.spans[1].content, "");
        assert_eq!(line.spans[2].content, " 3 ");
    }

    #[test]
    fn embedded_core_number_keeps_the_content_width() {
        for percent in [0, 10, 35, 100] {
            let line = bar_line_for_test(12, 2, percent);
            assert_eq!(line.width(), usize::from(CPU_CONTENT_WIDTH));
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn cluster_marker_follows_the_core_number() {
        let config = CpuConfig::default();
        let row = |core_cluster| {
            cpu_bar_line(12, 2, 35, core_cluster, &config)
                .spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>()
        };
        assert!(row(Some(tui_pane::CoreCluster::Performance)).starts_with(" 12 P "));
        assert!(row(Some(tui_pane::CoreCluster::Efficiency)).starts_with(" 12 E "));
        // `row(None)` retains the number field without a cluster marker, so block glyphs resume
        // immediately after its one-cell margin.
        assert!(row(None).starts_with(" 12 █"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn cluster_marker_keeps_the_content_width() {
        let config = CpuConfig::default();
        for core_cluster in [
            Some(CoreCluster::Performance),
            Some(CoreCluster::Efficiency),
            None,
        ] {
            let line = cpu_bar_line(12, 2, 35, core_cluster, &config);
            assert_eq!(line.width(), usize::from(CPU_CONTENT_WIDTH));
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn gpu_rows_keep_the_content_width() {
        let config = CpuConfig::default();
        let rows = [
            gpu_core_count_line(Some(38), CPU_CONTENT_WIDTH),
            gpu_core_count_line(None, CPU_CONTENT_WIDTH),
            gpu_metric_line("Device", Some(100), &config, CPU_CONTENT_WIDTH),
            gpu_metric_line("Renderer", None, &config, CPU_CONTENT_WIDTH),
            gpu_metric_line("Tiler", Some(7), &config, CPU_CONTENT_WIDTH),
        ];

        for row in rows {
            assert_eq!(row.width(), usize::from(CPU_CONTENT_WIDTH));
        }
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn gpu_rows_keep_the_content_width() {
        let config = CpuConfig::default();
        let rows = [
            gpu_metric_line("GPU", Some(100), &config, CPU_CONTENT_WIDTH),
            gpu_metric_line("GPU", None, &config, CPU_CONTENT_WIDTH),
        ];

        for row in rows {
            assert_eq!(row.width(), usize::from(CPU_CONTENT_WIDTH));
        }
    }
}
