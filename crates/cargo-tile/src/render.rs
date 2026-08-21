//! Frame rendering: the app's panes, the framework status line along the
//! bottom, and whichever framework overlay is open above them.

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Constraint;
use ratatui::layout::Layout;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Paragraph;
use ratatui::widgets::Row;
use ratatui::widgets::Table;
use ratatui::widgets::Widget;
use tui_pane::BarPalette;
use tui_pane::ColumnSpec;
use tui_pane::ColumnWidths;
use tui_pane::FrameworkOverlayId;
use tui_pane::GridLines;
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
use tui_pane::StatusLineNote;
use tui_pane::accent_color;
use tui_pane::default_pane_chrome;
use tui_pane::draw_clipped;
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
use crate::constants::APP_NAME;
use crate::constants::APP_VERSION;
use crate::constants::COMMAND_COLUMN;
use crate::constants::COMPILER_COLUMN;
use crate::constants::COMPILER_SEPARATOR_WIDTH;
use crate::constants::DURATION_COLUMN;
use crate::constants::GROUP_GAP_HEIGHT;
use crate::constants::GROUP_HEADER_HEIGHT;
use crate::constants::MANAGED_COLUMN;
use crate::constants::NO_PROCESSES_NOTE;
use crate::constants::PID_COLUMN;
use crate::constants::POPUP_CHROME_HEIGHT;
use crate::constants::POPUP_CHROME_WIDTH;
use crate::constants::SETTINGS_POPUP_WIDTH;
use crate::constants::START_COLUMN;
use crate::constants::STATUS_LINE_HEIGHT;
use crate::constants::SUMMARY_CELL_TITLE;
use crate::constants::TABLE_COLUMN_SPACING;
use crate::constants::TABLE_HEADER_HEIGHT;
use crate::constants::TABLE_HEADERS;
use crate::constants::TILE_NUMBER_INDENT;
use crate::globals::AppGlobalAction;
use crate::processes::CargoProcess;
use crate::processes::ManifestPath;
use crate::roster::Roster;
use crate::roster::TrackedRow;
use crate::settings;
use crate::tiles::TileContent;

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

/// Draw the tile grid into the body above the status line.
///
/// The summary cell carries one row per command; every other cell
/// carries one command's own invocations, which is what the summary
/// collapsed into that command's single row. [`crate::tiles`] decides
/// where each cell goes and how far through a transition it is, so what
/// is left here is drawing.
///
/// Contents go down first and the frame over the top of them, because a
/// border belongs to the grid rather than to either cell it divides:
/// neighbours share one line, and [`GridLines`] is what knows the glyph
/// each crossing wants. [`SUMMARY_CELL_TITLE`] goes on through
/// [`GridLines::add_titled`] for the same reason: a border a cell shares
/// is drawn by the pass that owns it, so anything written there has to
/// go in with it.
fn draw_panes(frame: &mut Frame, app: &mut App, area: Rect) {
    let initial_rows = app.loaded_config.config.tiles.initial_rows();
    app.tiles.set_layout(area, initial_rows);
    let ids = app.roster.ids();
    app.tiles.sync(&ids, initial_rows);
    let placements = app.tiles.placements(area, initial_rows);
    let mut grid_lines = GridLines::new(area);
    for placement in &placements {
        draw_clipped(frame.buffer_mut(), placement.frame, |buffer, inner| {
            draw_contents(buffer, &app.roster, placement.content, inner);
        });
        match placement.content {
            TileContent::Summary => grid_lines.add_titled(placement.frame, SUMMARY_CELL_TITLE),
            TileContent::Group(_) | TileContent::Empty(_) | TileContent::Gap => {
                grid_lines.add(placement.frame);
            },
        }
    }
    // Each frame carries its own focus, so `GridLines` gives the lines
    // the focused cell touches the theme's active shade and leaves
    // every other line alone.
    grid_lines.render(frame.buffer_mut(), default_pane_chrome());
}

/// What a cell holds inside its borders.
fn draw_contents(buffer: &mut Buffer, roster: &Roster, content: TileContent, inner: Rect) {
    match content {
        TileContent::Summary => draw_summary(buffer, roster, inner),
        TileContent::Group(id) => draw_group(buffer, roster, id, inner),
        TileContent::Empty(number) => draw_number(buffer, number, inner),
        // The hole a finished command left, on its way out of the grid.
        // Nothing goes in it: what the eye follows is the cells trading
        // places with it until it reaches the end.
        TileContent::Gap => (),
    }
}

/// The summary cell: one row per command, whatever each one is running
/// underneath.
///
/// The manifest path stays out of it. A row here is already under the
/// working directory heading its group, so the path says nothing new
/// while costing the width the subcommand and its flags need.
fn draw_summary(buffer: &mut Buffer, roster: &Roster, inner: Rect) {
    let rows: Vec<&TrackedRow> = roster.groups().iter().map(|group| &group.lead).collect();
    draw_process_table(buffer, inner, &rows, ManifestPath::Hidden);
}

/// One command's own cell: every invocation the summary put behind that
/// command's single row, the command itself included.
fn draw_group(buffer: &mut Buffer, roster: &Roster, id: u32, inner: Rect) {
    let Some(group) = roster.groups().iter().find(|group| group.id == id) else {
        return;
    };
    let rows: Vec<&TrackedRow> = group.rows().collect();
    draw_process_table(buffer, inner, &rows, ManifestPath::Shown);
}

/// A cell opened with `+` that no command has claimed: its number, on
/// the first row it has to give.
fn draw_number(buffer: &mut Buffer, number: usize, inner: Rect) {
    Paragraph::new(Line::from(vec![
        Span::raw(TILE_NUMBER_INDENT),
        Span::styled(number.to_string(), Style::default().fg(title_color())),
    ]))
    .render(Rect { height: 1, ..inner }, buffer);
}

/// The invocations sharing one working directory.
struct PathGroup<'a> {
    /// The working directory, as it heads the group.
    path: &'a str,
    /// Every invocation running there, newest first.
    rows: Vec<&'a TrackedRow>,
}

/// Render a cargo table: one working-directory header per distinct path,
/// with that directory's invocations tabulated beneath it.
///
/// Column widths are fitted across every row rather than per group, so
/// the tables line up down the cell instead of stepping in and out as
/// the eye moves between them.
fn draw_process_table(
    buffer: &mut Buffer,
    area: Rect,
    rows: &[&TrackedRow],
    manifest: ManifestPath,
) {
    if rows.is_empty() {
        Paragraph::new(vec![
            Line::from(""),
            Line::from(Span::styled(
                format!("{SECTION_HEADER_INDENT}{NO_PROCESSES_NOTE}"),
                Style::default().fg(label_color()),
            )),
        ])
        .render(area, buffer);
        return;
    }

    // One column-label row for the whole cell. Every group's table is
    // laid out with the same constraints and the same indent, so the
    // labels stay over their columns without costing a row per group.
    let constraints = fitted_constraints(rows);
    Table::new(Vec::<Row>::new(), constraints.iter().copied())
        .header(column_header())
        .column_spacing(TABLE_COLUMN_SPACING)
        .render(
            Rect {
                height: TABLE_HEADER_HEIGHT.min(area.height),
                ..indented(area)
            },
            buffer,
        );

    let mut remaining = area;
    remaining.y = remaining.y.saturating_add(TABLE_HEADER_HEIGHT);
    remaining.height = remaining.height.saturating_sub(TABLE_HEADER_HEIGHT);
    for group in group_by_path(rows) {
        if remaining.height == 0 {
            break;
        }
        let used = draw_path_group(buffer, remaining, &group, &constraints, manifest);
        remaining.y = remaining.y.saturating_add(used);
        remaining.height = remaining.height.saturating_sub(used);
    }
}

/// Collect the rows by working directory, groups ordered by path.
///
/// Ordering the groups by recency instead would rank each one by its
/// newest invocation, which moves a directory down the cell when the
/// build holding its place finishes -- a reshuffle triggered by the most
/// routine event on this screen. Path order never moves on its own.
/// Recency is not lost: the rows arrive newest first and that carries
/// into each group, so a build just fired off still heads its own
/// directory.
///
/// A linear search per row is enough: the grouping key is a path a
/// developer is building in, and there are only ever a handful of those
/// at once.
fn group_by_path<'a>(rows: &[&'a TrackedRow]) -> Vec<PathGroup<'a>> {
    let mut groups: Vec<PathGroup<'a>> = Vec::new();
    for row in rows {
        if let Some(group) = groups
            .iter_mut()
            .find(|group| group.path == row.process.path)
        {
            group.rows.push(row);
            continue;
        }
        groups.push(PathGroup {
            path: &row.process.path,
            rows: vec![row],
        });
    }
    groups.sort_by(|left, right| left.path.cmp(right.path));
    groups
}

/// Draw one working directory's header and table into the top of `area`,
/// answering how many rows that took including the blank row below it.
fn draw_path_group(
    buffer: &mut Buffer,
    area: Rect,
    group: &PathGroup<'_>,
    constraints: &[Constraint],
    manifest: ManifestPath,
) -> u16 {
    Paragraph::new(Line::from(vec![
        Span::raw(SECTION_HEADER_INDENT),
        Span::styled(group.path.to_string(), Style::default().fg(accent_color())),
    ]))
    .render(
        Rect {
            height: GROUP_HEADER_HEIGHT.min(area.height),
            ..area
        },
        buffer,
    );

    // No header on this table: the column labels are drawn once for the
    // whole cell by `draw_process_table`, over these same constraints.
    let rows = u16::try_from(group.rows.len()).unwrap_or(u16::MAX);
    let table_height = area.height.saturating_sub(GROUP_HEADER_HEIGHT).min(rows);
    Table::new(
        group.rows.iter().map(|row| process_row(row, manifest)),
        constraints.iter().copied(),
    )
    .column_spacing(TABLE_COLUMN_SPACING)
    .render(
        Rect {
            y: area.y.saturating_add(GROUP_HEADER_HEIGHT),
            height: table_height,
            ..indented(area)
        },
        buffer,
    );

    GROUP_HEADER_HEIGHT
        .saturating_add(table_height)
        .saturating_add(GROUP_GAP_HEIGHT)
}

/// Column widths fitted to the widest cell across every row.
///
/// `command` is left out of the fitting and takes whatever the other
/// columns leave, so a long argument list truncates instead of pushing
/// the columns that identify the invocation off the edge.
fn fitted_constraints(rows: &[&TrackedRow]) -> Vec<Constraint> {
    let mut widths = ColumnWidths::new(
        TABLE_HEADERS
            .iter()
            .map(|header| ColumnSpec::fit(cell_width(header)))
            .collect(),
    );
    for row in rows {
        let process = &row.process;
        widths.observe_cell_usize(PID_COLUMN, process.pid.to_string().chars().count());
        widths.observe_cell_usize(START_COLUMN, process.start.chars().count());
        widths.observe_cell_usize(DURATION_COLUMN, process.duration.chars().count());
        widths.observe_cell_usize(COMPILER_COLUMN, compiler_width(process));
        widths.observe_cell_usize(MANAGED_COLUMN, managed_text(process).chars().count());
    }

    let mut constraints = widths.to_constraints();
    let _ = constraints.pop();
    constraints.push(Constraint::Min(cell_width(TABLE_HEADERS[COMMAND_COLUMN])));
    constraints
}

/// The cell's one column-label row, drawn above the first group and
/// aligned with every group's rows by [`indented`].
fn column_header() -> Row<'static> {
    Row::new(
        TABLE_HEADERS
            .iter()
            .map(|label| Span::styled((*label).to_string(), Style::default().fg(label_color()))),
    )
}

/// One table row, styled so the invocation reads before its metadata.
///
/// A finished invocation goes flat grey for the seconds it lingers:
/// nothing on the row is live any more, so nothing on it should still
/// read as live.
fn process_row(row: &TrackedRow, manifest: ManifestPath) -> Row<'static> {
    let process = &row.process;
    let muted = Style::default().fg(label_color());
    let program = if row.is_ended() {
        muted
    } else {
        Style::default().fg(text_default())
    };
    let arguments = if row.is_ended() {
        muted
    } else {
        Style::default().fg(success_color())
    };
    Row::new(vec![
        Line::from(Span::styled(process.pid.to_string(), muted)),
        Line::from(Span::styled(process.start.clone(), muted)),
        Line::from(Span::styled(process.duration.clone(), muted)),
        compiler_cell(row),
        Line::from(Span::styled(managed_text(process), muted)),
        Line::from(vec![
            Span::styled(process.command.program.clone(), program),
            Span::raw(" "),
            Span::styled(process.command.line(manifest), arguments),
        ]),
    ])
}

/// The `sub` cell: how many cargo invocations this command is managing,
/// and nothing at all for the rows that manage none.
fn managed_text(process: &CargoProcess) -> String {
    if process.managed == 0 {
        return String::new();
    }
    process.managed.to_string()
}

/// The `compiler` cell: driver name in the active color, its count muted
/// beside it, and nothing at all when no compile is in flight.
fn compiler_cell(row: &TrackedRow) -> Line<'static> {
    let name = if row.is_ended() {
        Style::default().fg(label_color())
    } else {
        Style::default().fg(success_color())
    };
    row.process
        .compiler
        .as_ref()
        .map_or_else(Line::default, |compiler| {
            Line::from(vec![
                Span::styled(compiler.name, name),
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
    let notes = [StatusLineNote {
        label: APP_NAME.to_string(),
        value: APP_VERSION.to_string(),
    }];
    let status = StatusLine::new(
        app.started.elapsed().as_secs(),
        ScanIndicator::Hidden,
        &notes,
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
        border_color: title_color(),
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
