//! Frame rendering: the app's panes, the framework status line along the
//! bottom, and whichever framework overlay is open above them.

use ratatui::Frame;
use ratatui::layout::Constraint;
use ratatui::layout::Layout;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Block;
use ratatui::widgets::Paragraph;
use ratatui::widgets::Row;
use ratatui::widgets::Table;
use tui_pane::BarPalette;
use tui_pane::ColumnSpec;
use tui_pane::ColumnWidths;
use tui_pane::FrameworkOverlayId;
use tui_pane::Keymap;
use tui_pane::KeymapPane;
use tui_pane::PaneFocusState;
use tui_pane::PopupFrame;
use tui_pane::RenderFocus;
use tui_pane::SECTION_HEADER_INDENT;
use tui_pane::SECTION_ITEM_INDENT;
use tui_pane::ScanIndicator;
use tui_pane::SettingsRenderOptions;
use tui_pane::StatusLine;
use tui_pane::StatusLineGlobal;
use tui_pane::accent_color;
use tui_pane::active_border_color;
use tui_pane::error_color;
use tui_pane::hover_focus_color;
use tui_pane::inline_error_color;
use tui_pane::label_color;
use tui_pane::render_status_line;
use tui_pane::secondary_text_color;
use tui_pane::selection_style;
use tui_pane::status_bar_color;
use tui_pane::success_color;
use tui_pane::text_default;
use tui_pane::title_color;

use crate::app::App;
use crate::constants::COMMAND_COLUMN;
use crate::constants::COMPILER_COLUMN;
use crate::constants::COMPILER_SEPARATOR_WIDTH;
use crate::constants::DURATION_COLUMN;
use crate::constants::GROUP_GAP_HEIGHT;
use crate::constants::GROUP_HEADER_HEIGHT;
use crate::constants::NO_PROCESSES_NOTE;
use crate::constants::PANE_TITLE;
use crate::constants::PID_COLUMN;
use crate::constants::POPUP_CHROME_HEIGHT;
use crate::constants::POPUP_CHROME_WIDTH;
use crate::constants::SETTINGS_POPUP_WIDTH;
use crate::constants::START_COLUMN;
use crate::constants::STATUS_LINE_HEIGHT;
use crate::constants::TABLE_COLUMN_SPACING;
use crate::constants::TABLE_HEADER_HEIGHT;
use crate::constants::TABLE_HEADERS;
use crate::globals::AppGlobalAction;
use crate::processes::CargoProcess;
use crate::settings;

/// Draw one frame: panes fill the terminal above the status line, and an
/// open overlay floats above both.
pub(crate) fn draw(frame: &mut Frame, app: &mut App, keymap: &Keymap<App>) {
    let [body, status] =
        Layout::vertical([Constraint::Min(0), Constraint::Length(STATUS_LINE_HEIGHT)])
            .areas(frame.area());

    draw_panes(frame, app, body);
    draw_status_line(frame, app, keymap, status);

    match app.framework.overlay() {
        Some(FrameworkOverlayId::Settings) => draw_settings(frame, app),
        Some(FrameworkOverlayId::Keymap) => draw_keymap(frame, app, keymap),
        Some(FrameworkOverlayId::GlobalShortcuts) => draw_global_shortcuts(frame, app, keymap),
        _ => (),
    }
}

/// Draw the app's panes. One pane here — the running-cargo table —
/// filling the body above the status line. A larger app splits `area`
/// with [`Layout`] and draws one pane per [`crate::app::AppPaneId`].
fn draw_panes(frame: &mut Frame, app: &App, area: Rect) {
    let block = Block::bordered()
        .border_style(Style::default().fg(active_border_color()))
        .title(Span::styled(PANE_TITLE, Style::default().fg(title_color())));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    draw_process_table(frame, inner, &app.processes);
}

/// The invocations sharing one working directory, in scan order.
struct PathGroup<'a> {
    /// The working directory, as it heads the group.
    path:      &'a str,
    /// Every invocation running there, newest first.
    processes: Vec<&'a CargoProcess>,
}

/// Render the running-cargo table: one working-directory header per
/// distinct path, with that directory's invocations tabulated beneath it.
///
/// Column widths are fitted across every process rather than per group,
/// so the tables line up down the pane instead of stepping in and out as
/// the eye moves between them.
fn draw_process_table(frame: &mut Frame, area: Rect, processes: &[CargoProcess]) {
    if processes.is_empty() {
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(""),
                Line::from(Span::styled(
                    format!("{SECTION_HEADER_INDENT}{NO_PROCESSES_NOTE}"),
                    Style::default().fg(label_color()),
                )),
            ]),
            area,
        );
        return;
    }

    // One column-label row for the whole pane. Every group's table is laid
    // out with the same constraints and the same indent, so the labels
    // stay over their columns without costing a row per group.
    let constraints = fitted_constraints(processes);
    frame.render_widget(
        Table::new(Vec::<Row>::new(), constraints.iter().copied())
            .header(column_header())
            .column_spacing(TABLE_COLUMN_SPACING),
        Rect {
            height: TABLE_HEADER_HEIGHT.min(area.height),
            ..indented(area)
        },
    );

    let mut remaining = area;
    remaining.y = remaining.y.saturating_add(TABLE_HEADER_HEIGHT);
    remaining.height = remaining.height.saturating_sub(TABLE_HEADER_HEIGHT);
    for group in group_by_path(processes) {
        if remaining.height == 0 {
            break;
        }
        let used = draw_path_group(frame, remaining, &group, &constraints);
        remaining.y = remaining.y.saturating_add(used);
        remaining.height = remaining.height.saturating_sub(used);
    }
}

/// Collect the processes by working directory, preserving scan order.
///
/// [`crate::processes::spawn`] hands back newest first, so the directory
/// whose build started most recently heads the list and each group stays
/// newest first inside. A linear search per process is enough: the
/// grouping key is a path a developer is building in, and there are only
/// ever a handful of those at once.
fn group_by_path(processes: &[CargoProcess]) -> Vec<PathGroup<'_>> {
    let mut groups: Vec<PathGroup<'_>> = Vec::new();
    for process in processes {
        if let Some(group) = groups.iter_mut().find(|group| group.path == process.path) {
            group.processes.push(process);
            continue;
        }
        groups.push(PathGroup {
            path:      &process.path,
            processes: vec![process],
        });
    }
    groups
}

/// Draw one working directory's header and table into the top of `area`,
/// answering how many rows that took including the blank row below it.
fn draw_path_group(
    frame: &mut Frame,
    area: Rect,
    group: &PathGroup<'_>,
    constraints: &[Constraint],
) -> u16 {
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::raw(SECTION_HEADER_INDENT),
            Span::styled(group.path.to_string(), Style::default().fg(accent_color())),
        ])),
        Rect {
            height: GROUP_HEADER_HEIGHT.min(area.height),
            ..area
        },
    );

    // No header on this table: the column labels are drawn once for the
    // whole pane by `draw_process_table`, over these same constraints.
    let rows = u16::try_from(group.processes.len()).unwrap_or(u16::MAX);
    let table_height = area.height.saturating_sub(GROUP_HEADER_HEIGHT).min(rows);
    frame.render_widget(
        Table::new(
            group.processes.iter().copied().map(process_row),
            constraints.iter().copied(),
        )
        .column_spacing(TABLE_COLUMN_SPACING),
        Rect {
            y: area.y.saturating_add(GROUP_HEADER_HEIGHT),
            height: table_height,
            ..indented(area)
        },
    );

    GROUP_HEADER_HEIGHT
        .saturating_add(table_height)
        .saturating_add(GROUP_GAP_HEIGHT)
}

/// Column widths fitted to the widest cell across every group.
///
/// `command` is left out of the fitting and takes whatever the other
/// columns leave, so a long argument list truncates instead of pushing
/// the columns that identify the invocation off the edge.
fn fitted_constraints(processes: &[CargoProcess]) -> Vec<Constraint> {
    let mut widths = ColumnWidths::new(
        TABLE_HEADERS
            .iter()
            .map(|header| ColumnSpec::fit(cell_width(header)))
            .collect(),
    );
    for process in processes {
        widths.observe_cell_usize(PID_COLUMN, process.pid.to_string().chars().count());
        widths.observe_cell_usize(START_COLUMN, process.start.chars().count());
        widths.observe_cell_usize(DURATION_COLUMN, process.duration.chars().count());
        widths.observe_cell_usize(COMPILER_COLUMN, compiler_width(process));
    }

    let mut constraints = widths.to_constraints();
    let _ = constraints.pop();
    constraints.push(Constraint::Min(cell_width(TABLE_HEADERS[COMMAND_COLUMN])));
    constraints
}

/// The pane's one column-label row, drawn above the first group and
/// aligned with every group's rows by [`indented`].
fn column_header() -> Row<'static> {
    Row::new(
        TABLE_HEADERS
            .iter()
            .map(|label| Span::styled((*label).to_string(), Style::default().fg(label_color()))),
    )
}

/// One table row, styled so the invocation reads before its metadata.
fn process_row(process: &CargoProcess) -> Row<'static> {
    let muted = Style::default().fg(label_color());
    Row::new(vec![
        Line::from(Span::styled(process.pid.to_string(), muted)),
        Line::from(Span::styled(process.start.clone(), muted)),
        Line::from(Span::styled(process.duration.clone(), muted)),
        compiler_cell(process),
        Line::from(vec![
            Span::styled(
                process.command.program.clone(),
                Style::default().fg(text_default()),
            ),
            Span::raw(" "),
            Span::styled(
                process.command.arguments.clone(),
                Style::default().fg(success_color()),
            ),
        ]),
    ])
}

/// The `compiler` cell: driver name in the active color, its count muted
/// beside it, and nothing at all when no compile is in flight.
fn compiler_cell(process: &CargoProcess) -> Line<'static> {
    process
        .compiler
        .as_ref()
        .map_or_else(Line::default, |compiler| {
            Line::from(vec![
                Span::styled(compiler.name, Style::default().fg(success_color())),
                Span::styled(
                    format!("\u{d7}{}", compiler.count),
                    Style::default().fg(label_color()),
                ),
            ])
        })
}

/// Cells the `compiler` column needs for one row.
fn compiler_width(process: &CargoProcess) -> usize {
    process.compiler.as_ref().map_or(0, |compiler| {
        compiler.name.chars().count()
            + COMPILER_SEPARATOR_WIDTH
            + compiler.count.to_string().chars().count()
    })
}

/// `area` indented one level, where the column labels and every group's
/// rows both sit — the same two-level hierarchy the framework overlays
/// use, with the working directory heading each group at the outer level.
fn indented(area: Rect) -> Rect {
    let indent = cell_width(SECTION_ITEM_INDENT);
    Rect {
        x: area.x.saturating_add(indent),
        width: area.width.saturating_sub(indent),
        ..area
    }
}

/// A string's width in cells, clamped into the column-width type.
fn cell_width(text: &str) -> u16 { u16::try_from(text.chars().count()).unwrap_or(u16::MAX) }

/// Draw the framework status line.
///
/// The shortcut strip on the right is composed by the framework from the
/// slots named here, so `?` lands in the bottom-right corner and stays
/// bound to whatever `keymap.toml` maps it to.
fn draw_status_line(frame: &mut Frame, app: &App, keymap: &Keymap<App>, area: Rect) {
    let globals = [StatusLineGlobal::global_shortcuts_help()];
    let status = StatusLine::new(
        app.started.elapsed().as_secs(),
        ScanIndicator::Hidden,
        &[],
        &globals,
    );
    render_status_line::<App, AppGlobalAction>(
        frame,
        area,
        app,
        keymap,
        &app.framework,
        &bar_palette(),
        &status,
    );
}

/// Status-line styling, drawn from the active theme.
fn bar_palette() -> BarPalette {
    let enabled_key_style = Style::default()
        .fg(accent_color())
        .add_modifier(Modifier::BOLD);
    let disabled_key_style = Style::default()
        .fg(secondary_text_color())
        .add_modifier(Modifier::BOLD);
    BarPalette {
        status_line_style: Style::default().bg(status_bar_color()).fg(text_default()),
        status_activity_style: enabled_key_style,
        status_label_style: Style::default()
            .fg(title_color())
            .add_modifier(Modifier::BOLD),
        status_value_style: Style::default().fg(text_default()),
        enabled_key_style,
        enabled_label_style: Style::default().fg(text_default()),
        disabled_key_style,
        disabled_label_style: Style::default().fg(secondary_text_color()),
        separator_style: Style::default(),
    }
}

/// Draw the framework's keymap overlay: every registered action, its
/// scope, and the key it currently resolves to.
fn draw_keymap(frame: &mut Frame, app: &mut App, keymap: &Keymap<App>) {
    app.framework.keymap_pane.focus = RenderFocus {
        pane_focus_state: PaneFocusState::Active,
    };
    let inputs = KeymapPane::prepare_overlay_inputs(app, keymap);
    app.framework
        .keymap_pane
        .render_overlay(frame, frame.area(), &inputs);
}

/// Draw the framework's global-shortcuts overlay — the `?` popup.
fn draw_global_shortcuts(frame: &mut Frame, app: &mut App, keymap: &Keymap<App>) {
    app.framework.global_shortcuts_pane.focus = RenderFocus {
        pane_focus_state: PaneFocusState::Active,
    };
    app.framework
        .global_shortcuts_pane
        .render(frame, frame.area(), keymap);
}

/// Popup width that fits the widest row plus borders, never narrower
/// than [`SETTINGS_POPUP_WIDTH`]. The caller clamps it to the terminal.
fn fitted_width(widest_row: usize) -> u16 {
    let width = u16::try_from(widest_row.saturating_add(usize::from(POPUP_CHROME_WIDTH)))
        .unwrap_or(u16::MAX);
    width.max(SETTINGS_POPUP_WIDTH)
}

/// Draw the framework settings overlay, sized to its content and
/// clamped to the terminal so a resize never clips a row.
fn draw_settings(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    let built = settings::rows(app);
    // The popup centers itself and clamps to the frame, so the rows have
    // to be laid out for the width that survives that clamp.
    let width = fitted_width(built.widest_row).min(area.width);
    let content_width = usize::from(width.saturating_sub(POPUP_CHROME_WIDTH));
    let options = SettingsRenderOptions {
        focus: PaneFocusState::Active,
        inline_error: None,
        content_width,
        section_header_indent: SECTION_HEADER_INDENT,
        section_item_indent: SECTION_ITEM_INDENT,
        title_style: Style::default().fg(title_color()),
        label_style: Style::default().fg(label_color()),
        muted_style: Style::default().fg(label_color()),
        success_style: Style::default().fg(success_color()),
        error_style: Style::default().fg(error_color()),
        inline_error_style: Style::default().fg(inline_error_color()),
        active_style: selection_style(PaneFocusState::Active),
        remembered_style: selection_style(PaneFocusState::Remembered),
        hovered_style: Style::default().bg(hover_focus_color()),
    };
    let rendered = app
        .framework
        .settings_pane
        .render_rows(&built.rows, options);
    let line_count = u16::try_from(rendered.lines.len()).unwrap_or(u16::MAX);
    let height = line_count
        .saturating_add(POPUP_CHROME_HEIGHT)
        .min(area.height);
    let popup = PopupFrame {
        title: Some(" Settings ".to_string()),
        border_color: active_border_color(),
        width,
        height,
    }
    .render_with_areas(frame);

    let viewport = app.framework.settings_pane.viewport_mut();
    viewport.set_len(rendered.selectable_count);
    viewport.set_content_area(popup.inner);
    viewport.set_viewport_rows(usize::from(popup.inner.height));
    frame.render_widget(Paragraph::new(rendered.lines), popup.inner);
}
