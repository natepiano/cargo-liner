use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Paragraph;
use tui_pane::CpuUsage;
use tui_pane::PaneFocusState;
use tui_pane::PaneRule;
use tui_pane::Region;
use tui_pane::Size;
use tui_pane::Viewport;
use tui_pane::ViewportOverflow;
use tui_pane::accent_color;
use tui_pane::error_color;
use tui_pane::label_color;
use tui_pane::render_overflow_affordance;
use tui_pane::text_default;
use tui_pane::warning_color;

use super::CpuPane;
use crate::config::CpuConfig;
use crate::tui::panes::RenderStyles;
use crate::tui::panes::constants::CPU_BAR_WIDTH;
use crate::tui::panes::constants::CPU_BREAKDOWN_ROWS;
use crate::tui::panes::constants::CPU_GPU_ROWS;
use crate::tui::panes::constants::CPU_PINNED_HEAD_ROWS;
use crate::tui::panes::constants::CPU_STATIC_INNER_HEIGHT;
use crate::tui::panes::constants::GPU_UNAVAILABLE_TEXT;
use crate::tui::render_context::PaneRenderCtx;

/// Pinned tail rows below the scrolling cores band: breakdown rows plus GPU.
const fn cpu_pinned_tail_rows() -> usize { CPU_BREAKDOWN_ROWS + CPU_GPU_ROWS }

const fn total_selectable_rows(core_count: usize) -> usize {
    CPU_PINNED_HEAD_ROWS + core_count + cpu_pinned_tail_rows()
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
    Device,
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
    #[cfg(not(target_os = "macos"))]
    Gpu,
}

impl CpuSelectableRow {
    const fn logical_index(self, core_count: usize) -> usize {
        match self {
            Self::CoreCount => 0,
            Self::Device => 1,
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
            #[cfg(not(target_os = "macos"))]
            Self::Gpu => core_count + 5,
        }
    }
}

/// One scrolling core row: the usage bar with the 1-based core number
/// embedded in the bar's leftmost cells, right-aligned to `number_width` and
/// followed by a one-cell margin before the block glyphs. Digits render in
/// the default text color; the fill behind the number and margin is drawn as
/// background shading, so the bar still spans its full width.
fn cpu_bar_line(
    core_number: usize,
    number_width: usize,
    percent: u8,
    cpu_cfg: &CpuConfig,
) -> Line<'static> {
    let filled = tui_pane::cpu_filled_cells(percent);
    let severity = tui_pane::cpu_severity(
        percent,
        cpu_cfg.low_utilization_max_percent,
        cpu_cfg.medium_utilization_max_percent,
    )
    .color();
    let number_text = format!("{core_number:>number_width$} ");
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

/// Single-line GPU percent row. Falls back to compact unavailable text in
/// warning color when the OS exposes no matching metric.
fn gpu_metric_line(
    label: &str,
    percent: Option<u8>,
    cpu_cfg: &CpuConfig,
    width: u16,
) -> Line<'static> {
    let (value_text, value_color) = percent.map_or_else(
        || (GPU_UNAVAILABLE_TEXT.to_string(), warning_color()),
        |percent| {
            let severity = tui_pane::cpu_severity(
                percent,
                cpu_cfg.low_utilization_max_percent,
                cpu_cfg.medium_utilization_max_percent,
            )
            .color();
            (format!("{percent:>3}%"), severity)
        },
    );
    value_line(label, value_text, value_color, width)
}

#[cfg(target_os = "macos")]
fn gpu_core_count_line(core_count: Option<u16>, width: u16) -> Line<'static> {
    let (value_text, value_color) = core_count.map_or_else(
        || (GPU_UNAVAILABLE_TEXT.to_string(), warning_color()),
        |core_count| (core_count.to_string(), text_default()),
    );
    value_line("GPU Cores", value_text, value_color, width)
}

/// Pinned `CPU Cores` head row: the live CPU core count.
fn cpu_core_count_line(core_count: usize, width: u16) -> Line<'static> {
    value_line("CPU Cores", core_count.to_string(), text_default(), width)
}

/// Pinned `Device` head row: aggregate CPU utilization, mirroring the GPU
/// `Device` row.
fn device_line(percent: u8, width: u16) -> Line<'static> {
    value_line("Device", format!("{percent:>3}%"), text_default(), width)
}

/// One `label  value` row: the label in label color on the left, the value
/// right-aligned in `value_color`, padded to `width`. No trailing colon.
fn value_line(label: &str, value_text: String, value_color: Color, width: u16) -> Line<'static> {
    let space_count = usize::from(width).saturating_sub(
        label
            .len()
            .saturating_add(value_text.len())
            .saturating_add(2),
    );
    Line::from(vec![
        Span::raw(" "),
        Span::styled(label.to_string(), Style::default().fg(label_color())),
        Span::raw(" ".repeat(space_count)),
        Span::styled(value_text, Style::default().fg(value_color)),
        Span::raw(" "),
    ])
}

/// System / user / idle breakdown row: a `label:` in label color with the
/// percent right-aligned in its fixed `color`.
fn metric_line(label: &str, percent: u8, color: Color, width: u16) -> Line<'static> {
    value_line(&format!("{label}:"), format!("{percent:>3}%"), color, width)
}

/// The CPU pane's box tree: a pinned aggregate row, the scrolling cores band
/// (the one `Fill` box), the System/User/Idle breakdown (a rule above it), and
/// the GPU row (a rule above it). Rebuilt each frame from the live core count.
fn cpu_region(core_count: usize) -> Region {
    Region::stack(vec![
        Region::rows(CPU_PINNED_HEAD_ROWS, Size::Fixed),
        Region::rows(core_count, Size::Fill),
        Region::rows(CPU_BREAKDOWN_ROWS, Size::Fixed).rule(),
        Region::rows(CPU_GPU_ROWS, Size::Fixed).rule(),
    ])
}

/// Resolved rects for one CPU frame. The `Cores` count and `Device`
/// utilization rows are pinned at the top; the cores band is the `Fill` box
/// and scrolls; the breakdown rows and GPU follow. `band_offset` is the cores
/// band's scroll position, held across frames so the cursor stays visible.
struct CpuPanelLayout {
    core_count:     usize,
    core_count_row: Rect,
    device:         Rect,
    cores:          Rect,
    cores_divider:  Rect,
    system:         Rect,
    user:           Rect,
    idle:           Rect,
    gpu_divider:    Rect,
    #[cfg(target_os = "macos")]
    gpu_cores:      Rect,
    #[cfg(target_os = "macos")]
    gpu_device:     Rect,
    #[cfg(target_os = "macos")]
    gpu_renderer:   Rect,
    #[cfg(target_os = "macos")]
    gpu_tiler:      Rect,
    #[cfg(not(target_os = "macos"))]
    gpu:            Rect,
    band_offset:    usize,
}

impl CpuPanelLayout {
    fn new(inner: Rect, core_count: usize, cursor_pos: usize, prior_offset: usize) -> Self {
        // The cores band is box 1; only its prior offset is meaningful, the
        // pinned boxes never scroll.
        let placed = cpu_region(core_count).place(inner, cursor_pos, &[0, prior_offset, 0, 0]);
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
        #[cfg(target_os = "macos")]
        let gpu_row = {
            let gpu = placed[3].content;
            move |offset: u16| Rect {
                y: gpu.y.saturating_add(offset),
                height: 1,
                ..gpu
            }
        };
        Self {
            core_count,
            core_count_row: head_row(0),
            device: head_row(1),
            cores: placed[1].content,
            cores_divider: placed[2].chrome,
            system: breakdown_row(0),
            user: breakdown_row(1),
            idle: breakdown_row(2),
            gpu_divider: placed[3].chrome,
            #[cfg(target_os = "macos")]
            gpu_cores: gpu_row(0),
            #[cfg(target_os = "macos")]
            gpu_device: gpu_row(1),
            #[cfg(target_os = "macos")]
            gpu_renderer: gpu_row(2),
            #[cfg(target_os = "macos")]
            gpu_tiler: gpu_row(3),
            #[cfg(not(target_os = "macos"))]
            gpu: placed[3].content,
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

fn render_cpu_dividers(
    frame: &mut Frame,
    area: Rect,
    layout: &CpuPanelLayout,
    border_style: Style,
) {
    tui_pane::render_rules(
        frame,
        &[
            PaneRule::Horizontal {
                area:        Rect {
                    x:      area.x,
                    y:      layout.cores_divider.y,
                    width:  area.width,
                    height: 1,
                },
                connector_x: None,
            },
            PaneRule::Horizontal {
                area:        Rect {
                    x:      area.x,
                    y:      layout.gpu_divider.y,
                    width:  area.width,
                    height: 1,
                },
                connector_x: None,
            },
        ],
        border_style,
    );
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
        layout.device,
        CpuSelectableRow::Device.logical_index(layout.core_count),
        focus,
        Paragraph::new(device_line(usage.total_percent, layout.device.width)),
    );
}

fn render_core_rows(
    frame: &mut Frame,
    viewport: &Viewport,
    row_rects: &mut Vec<(Rect, usize)>,
    cpu_cfg: &CpuConfig,
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
        let core = &usage.cores[core_index];
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
                core.percent,
                cpu_cfg,
            )),
        );
    }
}

fn render_breakdown_row(
    frame: &mut Frame,
    viewport: &Viewport,
    row_rects: &mut Vec<(Rect, usize)>,
    focus: PaneFocusState,
    row: BreakdownRowSpec<'_>,
) {
    render_selectable_row(
        frame,
        viewport,
        row_rects,
        row.area,
        row.logical_row,
        focus,
        Paragraph::new(metric_line(
            row.label,
            row.percent,
            row.color,
            row.area.width,
        )),
    );
}

#[cfg(target_os = "macos")]
fn render_gpu_rows(
    frame: &mut Frame,
    viewport: &Viewport,
    row_rects: &mut Vec<(Rect, usize)>,
    cpu_cfg: &CpuConfig,
    usage: &CpuUsage,
    layout: &CpuPanelLayout,
    focus: PaneFocusState,
) {
    let gpu = usage.gpu;
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
        layout.gpu_cores,
        gpu_core_count_line(gpu.core_count, layout.gpu_cores.width),
    );
    render(
        CpuSelectableRow::GpuDevice,
        layout.gpu_device,
        gpu_metric_line(
            "Device",
            gpu.device_percent,
            cpu_cfg,
            layout.gpu_device.width,
        ),
    );
    render(
        CpuSelectableRow::GpuRenderer,
        layout.gpu_renderer,
        gpu_metric_line(
            "Renderer",
            gpu.renderer_percent,
            cpu_cfg,
            layout.gpu_renderer.width,
        ),
    );
    render(
        CpuSelectableRow::GpuTiler,
        layout.gpu_tiler,
        gpu_metric_line("Tiler", gpu.tiler_percent, cpu_cfg, layout.gpu_tiler.width),
    );
}

/// Single aggregate `GPU:` row on platforms that expose only device
/// utilization (Linux, Windows).
#[cfg(not(target_os = "macos"))]
fn render_gpu_rows(
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

const fn sync_cpu_pane_state(
    viewport: &mut Viewport,
    inner: Rect,
    core_count: usize,
    band_offset: usize,
) {
    viewport.set_len(total_selectable_rows(core_count));
    viewport.set_content_area(inner);
    viewport.set_scroll_offset(band_offset);
}

fn render_cpu_metric_rows(
    frame: &mut Frame,
    viewport: &Viewport,
    row_rects: &mut Vec<(Rect, usize)>,
    cpu_cfg: &CpuConfig,
    usage: &CpuUsage,
    layout: &CpuPanelLayout,
    focus: PaneFocusState,
) {
    render_head_rows(frame, viewport, row_rects, usage, layout, focus);
    render_core_rows(frame, viewport, row_rects, cpu_cfg, usage, layout, focus);
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
    render_gpu_rows(frame, viewport, row_rects, cpu_cfg, usage, layout, focus);
}

/// Body of `CpuPane::render`. Lives here (next to its helpers)
/// rather than inline in `pane.rs` because the helpers
/// belong with the per-pane render code; only the trait method
/// itself sits in `pane.rs` and delegates here.
pub(super) fn render_cpu_pane_body(
    frame: &mut Frame,
    area: Rect,
    pane: &mut CpuPane,
    styles: &RenderStyles,
    ctx: &PaneRenderCtx<'_>,
) {
    let pane_focus_state = pane.focus.pane_focus_state;
    let block = styles
        .chrome
        .block(cpu_panel_title().to_string(), pane.focus.is_focused());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height == 0 {
        pane.viewport.clear_surface();
        pane.clear_row_rects();
        return;
    }

    let usage = pane
        .content()
        .cloned()
        .unwrap_or_else(|| tui_pane::CpuUsage::placeholder(1));
    let cursor_pos = pane.viewport.pos();
    let layout = CpuPanelLayout::new(
        inner,
        usage.cores.len(),
        cursor_pos,
        pane.viewport.scroll_offset(),
    );

    let border_style = if matches!(pane_focus_state, PaneFocusState::Active) {
        styles.chrome.active_border
    } else {
        styles.chrome.inactive_border
    };
    render_cpu_dividers(frame, area, &layout, border_style);

    let cpu_cfg = &ctx.config.current().cpu;
    let mut row_rects: Vec<(Rect, usize)> = Vec::new();
    let viewport = &pane.viewport;
    render_cpu_metric_rows(
        frame,
        viewport,
        &mut row_rects,
        cpu_cfg,
        &usage,
        &layout,
        pane_focus_state,
    );
    render_cores_affordance(frame, &layout, cursor_pos);
    sync_cpu_pane_state(
        &mut pane.viewport,
        inner,
        usage.cores.len(),
        layout.band_offset,
    );
    pane.set_row_rects(row_rects);
}

/// Draw the `▲ n of m ▼` overflow label on the cores band when more cores
/// exist than fit. Skipped when the band has no visible rows.
fn render_cores_affordance(frame: &mut Frame, layout: &CpuPanelLayout, cursor_pos: usize) {
    let band_visible = layout.band_visible();
    if band_visible == 0 {
        return;
    }
    // The cores band owns selectable rows [CPU_PINNED_HEAD_ROWS,
    // CPU_PINNED_HEAD_ROWS + core_count); on one of those the pager anchors to
    // that core, otherwise to the current scroll offset.
    let band_cursor = cursor_pos
        .checked_sub(CPU_PINNED_HEAD_ROWS)
        .filter(|local| *local < layout.core_count)
        .unwrap_or(layout.band_offset);
    render_overflow_affordance(
        frame,
        layout.cores,
        ViewportOverflow::new(
            layout.core_count,
            layout.band_offset,
            band_visible,
            band_cursor,
        ),
        Style::default().fg(label_color()),
    );
}

#[cfg(test)]
mod tests {
    use ratatui::layout::Rect;
    use tui_pane::text_default;

    use super::cpu_bar_line;
    use super::cpu_region;
    #[cfg(target_os = "macos")]
    use super::gpu_core_count_line;
    use super::gpu_metric_line;
    use crate::config::CpuConfig;
    use crate::tui::panes::constants::CPU_CONTENT_WIDTH;

    // A 15-core CPU. The pinned head is 2 rows (Cores, Device), so the cores
    // band is logical rows [2, 17) and the breakdown starts at logical 17. On
    // macOS the fixed rows total 11 (2 head + 1+3 breakdown + 1+4 GPU), so a
    // 12-row inner leaves the cores band 1 row and it must scroll; a 26-row
    // inner fits all 15 cores. Other platforms have a single GPU row (8 fixed
    // rows), so the same 12-row inner leaves a 4-row band and a 23-row inner
    // fits every core. The cores band's resolved scroll offset is box index
    // 1's `scroll_offset`.
    fn cores_offset(inner_height: u16, cursor: usize, prior: usize) -> usize {
        let inner = Rect {
            x:      0,
            y:      0,
            width:  20,
            height: inner_height,
        };
        cpu_region(15).place(inner, cursor, &[0, prior, 0, 0])[1].scroll_offset
    }

    #[test]
    fn band_offset_tracks_cursor_inside_the_band() {
        // Cursor on logical row 14 (band-local 12) scrolls the band to its
        // last full page.
        #[cfg(target_os = "macos")]
        assert_eq!(cores_offset(12, 14, 0), 12);
        #[cfg(not(target_os = "macos"))]
        assert_eq!(cores_offset(12, 14, 0), 9);
    }

    #[test]
    fn band_offset_holds_prior_on_a_pinned_head_row() {
        // Cursor on the Cores count row (logical 0) is outside the band, so the
        // band stays where it was.
        assert_eq!(cores_offset(12, 0, 7), 7);
    }

    #[test]
    fn band_offset_holds_prior_on_a_pinned_tail_row() {
        // Cursor on the first breakdown row (logical 17) holds the prior
        // offset, clamped to the band's last page (15 cores - visible band).
        #[cfg(target_os = "macos")]
        assert_eq!(cores_offset(12, 17, 20), 14);
        #[cfg(not(target_os = "macos"))]
        assert_eq!(cores_offset(12, 17, 20), 11);
    }

    #[test]
    fn band_offset_is_zero_when_every_core_fits() {
        // Cores band taller than the core count: no scroll regardless of
        // cursor.
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
        // 35% fills 4 of 10 cells, so a 2-wide number plus its margin sit
        // entirely on the fill (as background shading) and the bar
        // continues with one block glyph.
        let line = cpu_bar_line(12, 2, 35, &CpuConfig::default());
        assert_eq!(line.spans[1].content, "12 ");
        assert_eq!(line.spans[1].style.fg, Some(text_default()));
        assert!(line.spans[1].style.bg.is_some());
        assert_eq!(line.spans[2].content, "");
        assert_eq!(line.spans[3].content, "█");
    }

    #[test]
    fn core_number_splits_at_the_fill_boundary() {
        // 10% fills 1 cell: the first digit is shaded by the fill, the
        // second digit and the margin render on the row background.
        let line = cpu_bar_line(12, 2, 10, &CpuConfig::default());
        assert_eq!(line.spans[1].content, "1");
        assert_eq!(line.spans[2].content, "2 ");
        assert_eq!(line.spans[2].style.fg, Some(text_default()));
        assert_eq!(line.spans[2].style.bg, None);
        assert_eq!(line.spans[3].content, "");
    }

    #[test]
    fn single_digit_core_number_is_right_aligned() {
        let line = cpu_bar_line(3, 2, 0, &CpuConfig::default());
        assert_eq!(line.spans[1].content, "");
        assert_eq!(line.spans[2].content, " 3 ");
    }

    #[test]
    fn embedded_core_number_keeps_the_content_width() {
        for percent in [0, 10, 35, 100] {
            let line = cpu_bar_line(12, 2, percent, &CpuConfig::default());
            assert_eq!(line.width(), usize::from(CPU_CONTENT_WIDTH));
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn gpu_rows_keep_the_content_width() {
        let cfg = CpuConfig::default();
        let rows = [
            gpu_core_count_line(Some(38), CPU_CONTENT_WIDTH),
            gpu_core_count_line(None, CPU_CONTENT_WIDTH),
            gpu_metric_line("Device", Some(100), &cfg, CPU_CONTENT_WIDTH),
            gpu_metric_line("Renderer", None, &cfg, CPU_CONTENT_WIDTH),
            gpu_metric_line("Tiler", Some(7), &cfg, CPU_CONTENT_WIDTH),
        ];

        for row in rows {
            assert_eq!(row.width(), usize::from(CPU_CONTENT_WIDTH));
        }
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn gpu_rows_keep_the_content_width() {
        let cfg = CpuConfig::default();
        let rows = [
            gpu_metric_line("GPU", Some(100), &cfg, CPU_CONTENT_WIDTH),
            gpu_metric_line("GPU", None, &cfg, CPU_CONTENT_WIDTH),
        ];

        for row in rows {
            assert_eq!(row.width(), usize::from(CPU_CONTENT_WIDTH));
        }
    }
}
