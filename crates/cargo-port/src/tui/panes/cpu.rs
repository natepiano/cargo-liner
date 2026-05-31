use ratatui::Frame;
use ratatui::layout::Constraint;
use ratatui::layout::Direction;
use ratatui::layout::Layout;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Paragraph;
use tui_pane::Band;
use tui_pane::CpuUsage;
use tui_pane::PaneFocusState;
use tui_pane::PaneRule;
use tui_pane::PaneTitleCount;
use tui_pane::Viewport;
use tui_pane::ViewportOverflow;
use tui_pane::accent_color;
use tui_pane::column_header_color;
use tui_pane::error_color;
use tui_pane::keep_visible_scroll_offset;
use tui_pane::label_color;
use tui_pane::render_overflow_affordance;
use tui_pane::text_default;
use tui_pane::warning_color;

use super::package::RenderStyles;
use super::pane_impls::CpuPane;
use crate::config::CpuConfig;
use crate::tui::pane::PaneRenderCtx;

const CPU_BAR_WIDTH: usize = 10;
/// Shown in the GPU row when the OS exposes no GPU utilization (e.g. the
/// Apple `asahi` driver on Linux). Kept within `CPU_CONTENT_WIDTH` so it
/// never widens the pane.
const GPU_UNAVAILABLE_TEXT: &str = "unavailable";
pub(super) const CPU_CONTENT_WIDTH: u16 = 17;
pub const CPU_PANE_WIDTH: u16 = CPU_CONTENT_WIDTH + 2;
/// Pixel height of every inner row except the scrolling cores band: the
/// aggregate row, the two separator rules, the three breakdown rows, and the
/// one GPU row. The cores band gets `inner.height - CPU_STATIC_INNER_HEIGHT`.
const CPU_STATIC_INNER_HEIGHT: u16 = 7;
/// Pinned head rows above the scrolling cores band: the aggregate line.
const CPU_PINNED_HEAD_ROWS: usize = 1;
/// Breakdown rows pinned below the cores band: System, User, Idle.
const CPU_BREAKDOWN_ROWS: usize = 3;
/// GPU rows pinned below the breakdown. A single aggregate row; multi-GPU is
/// deferred (see `docs/subpane-scrolling.md` → Related, separate work), so
/// growing the pinned tail for more GPUs changes only this count.
const CPU_GPU_ROWS: usize = 1;

/// Pinned tail rows below the scrolling cores band: breakdown rows plus GPU.
const fn cpu_pinned_tail_rows() -> usize { CPU_BREAKDOWN_ROWS + CPU_GPU_ROWS }

const fn total_selectable_rows(core_count: usize) -> usize {
    CPU_PINNED_HEAD_ROWS + core_count + cpu_pinned_tail_rows()
}

pub(super) fn cpu_required_inner_height(core_count: usize) -> u16 {
    let core_rows = u16::try_from(core_count).unwrap_or(u16::MAX);
    CPU_STATIC_INNER_HEIGHT.saturating_add(core_rows)
}

pub fn cpu_required_pane_height(core_count: usize) -> u16 {
    cpu_required_inner_height(core_count).saturating_add(2)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CpuSelectableRow {
    Aggregate,
    Core(usize),
    System,
    User,
    Idle,
    Gpu,
}

impl CpuSelectableRow {
    const fn logical_index(self, core_count: usize) -> usize {
        match self {
            Self::Aggregate => 0,
            Self::Core(index) => index + 1,
            Self::System => core_count + 1,
            Self::User => core_count + 2,
            Self::Idle => core_count + 3,
            Self::Gpu => core_count + 4,
        }
    }
}

fn cpu_bar_line(percent: u8, cpu_cfg: &CpuConfig) -> Line<'static> {
    let filled = tui_pane::cpu_filled_cells(percent);
    let severity = tui_pane::cpu_severity(
        percent,
        cpu_cfg.green_max_percent,
        cpu_cfg.yellow_max_percent,
    )
    .color();
    let filled_span = Span::styled("█".repeat(filled), Style::default().fg(severity));
    let empty_span = Span::styled(
        " ".repeat(CPU_BAR_WIDTH.saturating_sub(filled)),
        Style::default().fg(tui_pane::cpu_blank_bar_color()),
    );
    let percent_span = Span::raw(format!("{percent:>3}%"));
    Line::from(vec![
        Span::raw(" "),
        filled_span,
        empty_span,
        Span::raw(" "),
        percent_span,
        Span::raw(" "),
    ])
}

/// Single-line GPU row for the breakdown section: a `GPU` label with the
/// utilization percent right-aligned and colored by severity, matching the
/// System / User / Idle rows. Falls back to the unavailable text in warning
/// color when the OS exposes no GPU utilization.
fn gpu_metric_line(percent: Option<u8>, cpu_cfg: &CpuConfig, width: u16) -> Line<'static> {
    let (value_text, value_color) = percent.map_or_else(
        || (GPU_UNAVAILABLE_TEXT.to_string(), warning_color()),
        |percent| {
            let severity = tui_pane::cpu_severity(
                percent,
                cpu_cfg.green_max_percent,
                cpu_cfg.yellow_max_percent,
            )
            .color();
            (format!("{percent:>3}%"), severity)
        },
    );
    let label_text = "GPU:";
    let space_count = usize::from(width).saturating_sub(
        label_text
            .len()
            .saturating_add(value_text.len())
            .saturating_add(2),
    );
    Line::from(vec![
        Span::raw(" "),
        Span::styled(label_text, Style::default().fg(text_default())),
        Span::raw(" ".repeat(space_count)),
        Span::styled(value_text, Style::default().fg(value_color)),
        Span::raw(" "),
    ])
}

fn metric_line(label: &str, percent: u8, color: Color, width: u16) -> Line<'static> {
    let label_text = format!("{label}:");
    let value_text = format!("{percent:>3}%");
    let space_count = usize::from(width).saturating_sub(
        label_text
            .len()
            .saturating_add(value_text.len())
            .saturating_add(2),
    );
    Line::from(vec![
        Span::raw(" "),
        Span::styled(label_text, Style::default().fg(text_default())),
        Span::raw(" ".repeat(space_count)),
        Span::styled(value_text, Style::default().fg(color)),
        Span::raw(" "),
    ])
}

fn aggregate_line(percent: u8, width: u16) -> Line<'static> {
    let label_text = "Aggregate";
    let value_text = format!("{percent:>3}%");
    let space_count = usize::from(width).saturating_sub(
        label_text
            .len()
            .saturating_add(value_text.len())
            .saturating_add(2),
    );
    Line::from(vec![
        Span::raw(" "),
        Span::styled(label_text, Style::default().fg(column_header_color())),
        Span::raw(" ".repeat(space_count)),
        Span::styled(value_text, Style::default().fg(text_default())),
        Span::raw(" "),
    ])
}

/// Resolved rects for one CPU frame. The aggregate row is the pinned head;
/// the cores band scrolls; the breakdown rows and GPU are the pinned tail.
/// `band` partitions the selectable-row list and `band_offset` is the band's
/// scroll position (held across frames so the cursor stays visible).
struct CpuPanelLayout {
    core_count:    usize,
    aggregate:     Rect,
    cores:         Rect,
    cores_divider: Rect,
    system:        Rect,
    user:          Rect,
    idle:          Rect,
    gpu_divider:   Rect,
    gpu:           Rect,
    band:          Option<Band>,
    band_offset:   usize,
}

impl CpuPanelLayout {
    fn new(inner: Rect, core_count: usize, cursor_pos: usize, prior_offset: usize) -> Self {
        let core_rows = u16::try_from(core_count).unwrap_or(u16::MAX);
        let band_height = inner
            .height
            .saturating_sub(CPU_STATIC_INNER_HEIGHT)
            .min(core_rows);
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),           // aggregate (pinned head)
                Constraint::Length(band_height), // cores band
                Constraint::Length(1),           // cores | breakdown rule
                Constraint::Length(1),           // System
                Constraint::Length(1),           // User
                Constraint::Length(1),           // Idle
                Constraint::Length(1),           // Idle | GPU rule
                Constraint::Length(1),           // GPU
                Constraint::Min(0),              // slack below the pinned tail
            ])
            .split(inner);
        let band = Band::new(
            CPU_PINNED_HEAD_ROWS,
            cpu_pinned_tail_rows(),
            total_selectable_rows(core_count),
        );
        let band_visible = usize::from(rows[1].height);
        let band_offset = band.map_or(0, |band| {
            cpu_band_offset(cursor_pos, prior_offset, band, band_visible)
        });
        Self {
            core_count,
            aggregate: rows[0],
            cores: rows[1],
            cores_divider: rows[2],
            system: rows[3],
            user: rows[4],
            idle: rows[5],
            gpu_divider: rows[6],
            gpu: rows[7],
            band,
            band_offset,
        }
    }

    /// Number of core rows visible in the band this frame.
    fn band_visible(&self) -> usize { usize::from(self.cores.height) }
}

/// Band scroll offset for this frame. While the cursor is inside the cores
/// band, clamp the offset to keep it visible; on a pinned row, hold the prior
/// offset (clamped to the band's range in case the core count shrank).
fn cpu_band_offset(
    cursor_pos: usize,
    prior_offset: usize,
    band: Band,
    band_visible: usize,
) -> usize {
    band.band_local_cursor(cursor_pos).map_or_else(
        || prior_offset.min(band.band_len().saturating_sub(band_visible)),
        |band_local| keep_visible_scroll_offset(band_local, band_visible, band.band_len()),
    )
}

#[derive(Clone, Copy)]
struct BreakdownRowSpec<'a> {
    area:        Rect,
    logical_row: usize,
    label:       &'a str,
    percent:     u8,
    color:       Color,
}

fn cpu_panel_title(core_count: usize, cursor: Option<usize>) -> String {
    if let Some(pos) = cursor
        && (1..=core_count).contains(&pos)
    {
        return tui_pane::pane_title(
            "CPU",
            &PaneTitleCount::Single {
                len:    core_count,
                cursor: Some(pos - 1),
            },
        );
    }

    let core_label = if core_count == 1 { "core" } else { "cores" };
    format!(" CPU ({core_count} {core_label}) ")
}

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

fn render_aggregate_row(
    frame: &mut Frame,
    viewport: &Viewport,
    row_rects: &mut Vec<(Rect, usize)>,
    usage: &CpuUsage,
    layout: &CpuPanelLayout,
    focus: PaneFocusState,
) {
    let logical_row = CpuSelectableRow::Aggregate.logical_index(layout.core_count);
    render_selectable_row(
        frame,
        viewport,
        row_rects,
        layout.aggregate,
        logical_row,
        focus,
        Paragraph::new(aggregate_line(usage.total_percent, layout.aggregate.width)),
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
            Paragraph::new(cpu_bar_line(core.percent, cpu_cfg)),
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

fn render_gpu_row(
    frame: &mut Frame,
    viewport: &Viewport,
    row_rects: &mut Vec<(Rect, usize)>,
    cpu_cfg: &CpuConfig,
    usage: &CpuUsage,
    layout: &CpuPanelLayout,
    focus: PaneFocusState,
) {
    let logical_row = CpuSelectableRow::Gpu.logical_index(layout.core_count);
    let area = layout.gpu;
    render_selectable_row(
        frame,
        viewport,
        row_rects,
        area,
        logical_row,
        focus,
        Paragraph::new(gpu_metric_line(usage.gpu_percent, cpu_cfg, area.width)),
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

/// Body of `CpuPane::render`. Lives here (next to its helpers)
/// rather than inline in `pane_impls.rs` because the helpers
/// belong with the per-pane render code; only the trait method
/// itself sits in `pane_impls.rs` and delegates here.
pub(super) fn render_cpu_pane_body(
    frame: &mut Frame,
    area: Rect,
    pane: &mut CpuPane,
    styles: &RenderStyles,
    ctx: &PaneRenderCtx<'_>,
) {
    let focus = pane.focus.state;
    let cursor = matches!(focus, PaneFocusState::Active).then(|| pane.viewport.pos());
    let title = pane.content().map_or_else(
        || " CPU ".to_string(),
        |usage| cpu_panel_title(usage.cores.len(), cursor),
    );
    let block = styles.chrome.block(title, pane.focus.is_focused);
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

    let border_style = if matches!(focus, PaneFocusState::Active) {
        styles.chrome.active_border
    } else {
        styles.chrome.inactive_border
    };
    render_cpu_dividers(frame, area, &layout, border_style);

    let cpu_cfg = &ctx.config.current().cpu;
    let mut row_rects: Vec<(Rect, usize)> = Vec::new();
    let viewport = &pane.viewport;
    render_aggregate_row(frame, viewport, &mut row_rects, &usage, &layout, focus);
    render_core_rows(
        frame,
        viewport,
        &mut row_rects,
        cpu_cfg,
        &usage,
        &layout,
        focus,
    );
    render_breakdown_row(
        frame,
        viewport,
        &mut row_rects,
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
        &mut row_rects,
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
        &mut row_rects,
        focus,
        BreakdownRowSpec {
            area:        layout.idle,
            logical_row: CpuSelectableRow::Idle.logical_index(layout.core_count),
            label:       "Idle",
            percent:     usage.breakdown.idle,
            color:       text_default(),
        },
    );
    render_gpu_row(
        frame,
        viewport,
        &mut row_rects,
        cpu_cfg,
        &usage,
        &layout,
        focus,
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
/// exist than fit. Skipped when the band has no visible rows or no partition.
fn render_cores_affordance(frame: &mut Frame, layout: &CpuPanelLayout, cursor_pos: usize) {
    if layout.band_visible() == 0 {
        return;
    }
    let Some(band) = layout.band else {
        return;
    };
    let band_cursor = band
        .band_local_cursor(cursor_pos)
        .unwrap_or(layout.band_offset);
    render_overflow_affordance(
        frame,
        layout.cores,
        ViewportOverflow::band(
            band.band_len(),
            layout.band_offset,
            layout.band_visible(),
            band_cursor,
        ),
        Style::default().fg(label_color()),
    );
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use super::Band;
    use super::cpu_band_offset;

    // A 15-core CPU: 1 pinned aggregate, 15-core band, 4 pinned breakdown
    // rows. The band is 5 rows tall, so the cores must scroll.
    fn cores_band() -> Band {
        Band::new(
            super::CPU_PINNED_HEAD_ROWS,
            super::cpu_pinned_tail_rows(),
            20,
        )
        .expect("head + tail fits within len")
    }

    #[test]
    fn band_offset_tracks_cursor_inside_the_band() {
        // Cursor on logical row 14 (band-local 13) scrolls a 5-tall band to
        // its last full page.
        assert_eq!(cpu_band_offset(14, 0, cores_band(), 5), 9);
    }

    #[test]
    fn band_offset_holds_prior_on_a_pinned_head_row() {
        // Cursor on the aggregate row (logical 0) is outside the band, so the
        // band stays where it was.
        assert_eq!(cpu_band_offset(0, 7, cores_band(), 5), 7);
    }

    #[test]
    fn band_offset_holds_prior_on_a_pinned_tail_row() {
        // Cursor on the first breakdown row (logical 16) holds the prior
        // offset, clamped to the band's last page.
        assert_eq!(cpu_band_offset(16, 20, cores_band(), 5), 10);
    }

    #[test]
    fn band_offset_is_zero_when_every_core_fits() {
        // Band taller than the core count: no scroll regardless of cursor.
        assert_eq!(cpu_band_offset(14, 0, cores_band(), 15), 0);
    }
}
