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

use super::PaneRule;
use super::package::RenderStyles;
use super::pane_title;
use super::render_rules;
use crate::tui::app::App;
use crate::tui::constants::ACCENT_COLOR;
use crate::tui::constants::COLUMN_HEADER_COLOR;
use crate::tui::constants::ERROR_COLOR;
use crate::tui::cpu;
use crate::tui::interaction;
use crate::tui::interaction::UiSurface::Content;
use crate::tui::types::PaneFocusState;
use crate::tui::types::PaneId;

const CPU_BAR_WIDTH: usize = 10;
pub(super) const CPU_CONTENT_WIDTH: u16 = 17;
pub const CPU_PANE_WIDTH: u16 = CPU_CONTENT_WIDTH + 2;
const CPU_STATIC_INNER_HEIGHT: u16 = 8;

const fn total_selectable_rows(core_count: usize) -> usize { core_count + 5 }

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

fn cpu_bar_line(percent: u8, app: &App) -> Line<'static> {
    let cpu_cfg = &app.current_config().cpu;
    let filled = cpu::filled_cells(percent);
    let severity = cpu::severity(percent, cpu_cfg).color();
    let filled_span = Span::styled("█".repeat(filled), Style::default().fg(severity));
    let empty_span = Span::styled(
        " ".repeat(CPU_BAR_WIDTH.saturating_sub(filled)),
        Style::default().fg(cpu::blank_bar_color()),
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

fn gpu_bar_line(percent: Option<u8>, app: &App) -> Line<'static> {
    let value = percent.unwrap_or(0);
    let filled = cpu::filled_cells(value);
    let severity = cpu::severity(value, &app.current_config().cpu).color();
    let filled_span = Span::styled("█".repeat(filled), Style::default().fg(severity));
    let empty_span = Span::styled(
        " ".repeat(CPU_BAR_WIDTH.saturating_sub(filled)),
        Style::default().fg(cpu::blank_bar_color()),
    );
    let percent_text = percent.map_or_else(|| " --%".to_string(), |value| format!("{value:>3}%"));
    Line::from(vec![
        Span::raw(" "),
        filled_span,
        empty_span,
        Span::raw(" "),
        Span::raw(percent_text),
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
        Span::styled(label_text, Style::default().fg(Color::White)),
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
        Span::styled(label_text, Style::default().fg(COLUMN_HEADER_COLOR)),
        Span::raw(" ".repeat(space_count)),
        Span::styled(value_text, Style::default().fg(Color::White)),
        Span::raw(" "),
    ])
}

struct CpuPanelLayout {
    rows:       Vec<Rect>,
    core_count: usize,
    system_row: usize,
    user_row:   usize,
    idle_row:   usize,
    gpu_row:    usize,
}

impl CpuPanelLayout {
    fn new(inner: Rect, core_count: usize) -> Self {
        let mut constraints =
            Vec::with_capacity(usize::from(cpu_required_inner_height(core_count)) + 1);
        constraints.push(Constraint::Length(1));
        constraints.extend(std::iter::repeat_n(Constraint::Length(1), core_count));
        constraints.extend([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(2),
            Constraint::Min(0),
        ]);
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(inner);
        let system_row = 1 + core_count + 1;
        let user_row = system_row + 1;
        let idle_row = user_row + 1;
        let gpu_row = idle_row + 2;
        Self {
            rows: rows.to_vec(),
            core_count,
            system_row,
            user_row,
            idle_row,
            gpu_row,
        }
    }
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
        return pane_title(
            "CPU",
            &super::PaneTitleCount::Single {
                len:    core_count,
                cursor: Some(pos - 1),
            },
        );
    }

    let core_label = if core_count == 1 { "core" } else { "cores" };
    format!(" CPU ({core_count} {core_label}) ")
}

fn cpu_row_overlay_style(app: &App, logical_row: usize, focus: PaneFocusState) -> Style {
    app.pane_manager()
        .pane(PaneId::Cpu)
        .selection_state(logical_row, focus)
        .overlay_style()
}

fn render_selectable_row(
    frame: &mut Frame,
    app: &mut App,
    area: Rect,
    logical_row: usize,
    focus: PaneFocusState,
    paragraph: Paragraph<'static>,
) {
    frame.render_widget(
        paragraph.style(cpu_row_overlay_style(app, logical_row, focus)),
        area,
    );
    interaction::register_pane_row_hitbox(app, area, PaneId::Cpu, logical_row, Content);
}

fn render_cpu_dividers(
    frame: &mut Frame,
    area: Rect,
    layout: &CpuPanelLayout,
    border_style: Style,
) {
    render_rules(
        frame,
        &[
            PaneRule::Horizontal {
                area:        Rect {
                    x:      area.x,
                    y:      layout.rows[1 + layout.core_count].y,
                    width:  area.width,
                    height: 1,
                },
                connector_x: None,
            },
            PaneRule::Horizontal {
                area:        Rect {
                    x:      area.x,
                    y:      layout.rows[layout.gpu_row - 1].y,
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
    app: &mut App,
    snapshot: &cpu::CpuSnapshot,
    layout: &CpuPanelLayout,
    focus: PaneFocusState,
) {
    let logical_row = CpuSelectableRow::Aggregate.logical_index(layout.core_count);
    render_selectable_row(
        frame,
        app,
        layout.rows[0],
        logical_row,
        focus,
        Paragraph::new(aggregate_line(snapshot.total_percent, layout.rows[0].width)),
    );
}

fn render_core_rows(
    frame: &mut Frame,
    app: &mut App,
    snapshot: &cpu::CpuSnapshot,
    layout: &CpuPanelLayout,
    focus: PaneFocusState,
) {
    for (core_index, core) in snapshot.cores.iter().enumerate() {
        let logical_row = CpuSelectableRow::Core(core_index).logical_index(layout.core_count);
        render_selectable_row(
            frame,
            app,
            layout.rows[1 + core_index],
            logical_row,
            focus,
            Paragraph::new(cpu_bar_line(core.percent, app)),
        );
    }
}

fn render_breakdown_row(
    frame: &mut Frame,
    app: &mut App,
    focus: PaneFocusState,
    row: BreakdownRowSpec<'_>,
) {
    render_selectable_row(
        frame,
        app,
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
    app: &mut App,
    snapshot: &cpu::CpuSnapshot,
    layout: &CpuPanelLayout,
    focus: PaneFocusState,
) {
    let logical_row = CpuSelectableRow::Gpu.logical_index(layout.core_count);
    render_selectable_row(
        frame,
        app,
        layout.rows[layout.gpu_row],
        logical_row,
        focus,
        Paragraph::new(vec![
            Line::from(vec![
                Span::raw(" "),
                Span::styled("GPU", Style::default().fg(COLUMN_HEADER_COLOR)),
            ]),
            gpu_bar_line(snapshot.gpu_percent, app),
        ]),
    );
}

fn sync_cpu_pane_state(app: &mut App, inner: Rect, core_count: usize) {
    let pane = app.pane_manager_mut().pane_mut(PaneId::Cpu);
    pane.set_len(total_selectable_rows(core_count));
    pane.set_content_area(inner);
    pane.set_scroll_offset(0);
}

pub fn render_cpu_panel(frame: &mut Frame, app: &mut App, styles: &RenderStyles, area: Rect) {
    let focus = app.pane_focus_state(PaneId::Cpu);
    let pane = app.pane_manager().pane(PaneId::Cpu);
    let cursor = matches!(focus, PaneFocusState::Active).then(|| pane.pos());
    let title = app.pane_manager().cpu_data.as_ref().map_or_else(
        || " CPU ".to_string(),
        |snapshot| cpu_panel_title(snapshot.cores.len(), cursor),
    );
    let block = styles.chrome.block(title, app.is_focused(PaneId::Cpu));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height == 0 {
        app.pane_manager_mut().pane_mut(PaneId::Cpu).clear_surface();
        return;
    }

    let snapshot = app
        .pane_manager()
        .cpu_data
        .clone()
        .unwrap_or_else(|| cpu::CpuSnapshot::placeholder(1));
    let layout = CpuPanelLayout::new(inner, snapshot.cores.len());

    let border_style = if matches!(focus, PaneFocusState::Active) {
        styles.chrome.active_border
    } else {
        styles.chrome.inactive_border
    };
    render_cpu_dividers(frame, area, &layout, border_style);
    render_aggregate_row(frame, app, &snapshot, &layout, focus);
    render_core_rows(frame, app, &snapshot, &layout, focus);
    render_breakdown_row(
        frame,
        app,
        focus,
        BreakdownRowSpec {
            area:        layout.rows[layout.system_row],
            logical_row: CpuSelectableRow::System.logical_index(layout.core_count),
            label:       "System",
            percent:     snapshot.breakdown.system,
            color:       ERROR_COLOR,
        },
    );
    render_breakdown_row(
        frame,
        app,
        focus,
        BreakdownRowSpec {
            area:        layout.rows[layout.user_row],
            logical_row: CpuSelectableRow::User.logical_index(layout.core_count),
            label:       "User",
            percent:     snapshot.breakdown.user,
            color:       ACCENT_COLOR,
        },
    );
    render_breakdown_row(
        frame,
        app,
        focus,
        BreakdownRowSpec {
            area:        layout.rows[layout.idle_row],
            logical_row: CpuSelectableRow::Idle.logical_index(layout.core_count),
            label:       "Idle",
            percent:     snapshot.breakdown.idle,
            color:       Color::White,
        },
    );
    render_gpu_row(frame, app, &snapshot, &layout, focus);
    sync_cpu_pane_state(app, inner, snapshot.cores.len());
}
