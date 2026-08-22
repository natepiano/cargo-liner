//! Frame rendering: the app's panes, the framework status line along the
//! bottom, and whichever framework overlay is open above them.

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Constraint;
use ratatui::layout::Layout;
use ratatui::layout::Rect;
use ratatui::style::Color;
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
use tui_pane::warning_color;

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
use crate::constants::PROGRESS_ABSENT;
use crate::constants::PROGRESS_CELL_BAR_WIDTH;
use crate::constants::PROGRESS_CELL_EMPTY;
use crate::constants::PROGRESS_CELL_PARTIALS;
use crate::constants::PROGRESS_HEADING_EMPTY;
use crate::constants::PROGRESS_HEADING_FILLED;
use crate::constants::PROGRESS_HEADING_MARGINS;
use crate::constants::PROGRESS_HEADING_MIN_WIDTH;
use crate::constants::PROGRESS_HEADING_READING_CAPACITY;
use crate::constants::PROGRESS_READING_WIDTH;
use crate::constants::SETTINGS_POPUP_WIDTH;
use crate::constants::START_COLUMN;
use crate::constants::STATE_BLOCKED;
use crate::constants::STATE_COLUMN;
use crate::constants::STATUS_LINE_HEIGHT;
use crate::constants::SUMMARY_CELL_TITLE;
use crate::constants::SUMMARY_HIDDEN_COLUMNS;
use crate::constants::TABLE_COLUMN_SPACING;
use crate::constants::TABLE_HEADER_HEIGHT;
use crate::constants::TABLE_HEADERS;
use crate::constants::TILE_NUMBER_INDENT;
use crate::globals::AppGlobalAction;
use crate::processes::CargoProcess;
use crate::processes::ManifestPath;
use crate::progress::Progress;
use crate::progress::RunState;
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
fn draw_summary(buffer: &mut Buffer, roster: &Roster, inner: Rect) {
    let rows: Vec<&TrackedRow> = roster.groups().iter().map(|group| &group.lead).collect();
    draw_process_table(buffer, inner, &rows, TableKind::Summary);
}

/// Where a cell puts what it knows of each command's progress.
///
/// The heading reads better and costs the table no width, so it carries
/// the reading wherever it can. It cannot where one directory has two
/// commands reporting at once -- a heading stands over both, and the
/// second reading would have nowhere to go -- and the whole cell falls
/// back to a column rather than quietly dropping one.
fn progress_placement(rows: &[&TrackedRow]) -> ProgressPlacement {
    if group_by_path(rows).iter().all(|group| {
        group
            .rows
            .iter()
            .filter(|row| reading(row).is_some())
            .count()
            <= PROGRESS_HEADING_READING_CAPACITY
    }) {
        ProgressPlacement::Heading
    } else {
        ProgressPlacement::Column
    }
}

/// One command's own cell: every invocation the summary put behind that
/// command's single row, the command itself included.
fn draw_group(buffer: &mut Buffer, roster: &Roster, id: u32, inner: Rect) {
    let Some(group) = roster.groups().iter().find(|group| group.id == id) else {
        return;
    };
    let rows: Vec<&TrackedRow> = group.rows().collect();
    draw_process_table(buffer, inner, &rows, TableKind::Command);
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

/// Which cell is drawing a table, which is what settles how much of a
/// row it has the room to say.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TableKind {
    /// The summary: one row per command, over every directory at once.
    Summary,
    /// One command's own cell: every invocation running under it.
    Command,
}

impl TableKind {
    /// Whether this cell describes single invocations, which is what
    /// the columns in [`SUMMARY_HIDDEN_COLUMNS`] have to say.
    const fn shows_invocation_detail(self) -> bool { matches!(self, Self::Command) }

    /// Whether a row here needs its manifest path. A summary row already
    /// sits under the working directory heading its group, so the path
    /// says nothing new while costing the width the command line wants.
    const fn manifest(self) -> ManifestPath {
        match self {
            Self::Summary => ManifestPath::Hidden,
            Self::Command => ManifestPath::Shown,
        }
    }
}

/// Where a cell puts what it knows of a command's build progress.
///
/// The two cells ask different questions of it. A command's own cell
/// stands over one command, so the working-directory header can carry
/// the whole reading and be read across the room; the summary's header
/// stands over every command running in that directory at once, so the
/// reading has to sit on the row it belongs to.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ProgressPlacement {
    /// A `state` cell on each row.
    Column,
    /// A rule along the working-directory header.
    Heading,
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
fn draw_process_table(buffer: &mut Buffer, area: Rect, rows: &[&TrackedRow], kind: TableKind) {
    let manifest = kind.manifest();
    let placement = progress_placement(rows);
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
    let columns = visible_columns(rows, kind, placement);
    let constraints = fitted_constraints(rows, &columns);
    Table::new(Vec::<Row>::new(), constraints.iter().copied())
        .header(column_header(&columns))
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
        let used = draw_path_group(
            buffer,
            remaining,
            &group,
            &constraints,
            &columns,
            manifest,
            placement,
        );
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
    columns: &[usize],
    manifest: ManifestPath,
    placement: ProgressPlacement,
) -> u16 {
    let mut heading = vec![
        Span::raw(SECTION_HEADER_INDENT),
        Span::styled(group.path.to_string(), Style::default().fg(accent_color())),
    ];
    if placement == ProgressPlacement::Heading {
        heading.extend(heading_gauge(group, area.width));
    }
    Paragraph::new(Line::from(heading)).render(
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
        group
            .rows
            .iter()
            .map(|row| process_row(row, manifest, columns)),
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
fn fitted_constraints(rows: &[&TrackedRow], columns: &[usize]) -> Vec<Constraint> {
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
        widths.observe_cell_usize(STATE_COLUMN, state_width(process.state));
        widths.observe_cell_usize(COMPILER_COLUMN, compiler_width(process));
        widths.observe_cell_usize(MANAGED_COLUMN, managed_text(process).chars().count());
    }

    // The command absorbs whatever the fitted columns leave, which is
    // why it alone is a `Min`. It is no longer the last column, so the
    // slack has to follow the column rather than the position.
    widths
        .to_constraints()
        .into_iter()
        .enumerate()
        .filter(|(column, _)| columns.contains(column))
        .map(|(column, constraint)| {
            if column == COMMAND_COLUMN {
                Constraint::Min(cell_width(TABLE_HEADERS[COMMAND_COLUMN]))
            } else {
                constraint
            }
        })
        .collect()
}

/// The cell's one column-label row, drawn above the first group and
/// aligned with every group's rows by [`indented`].
fn column_header(columns: &[usize]) -> Row<'static> {
    Row::new(
        TABLE_HEADERS
            .iter()
            .enumerate()
            .filter(|(column, _)| columns.contains(column))
            .map(|(_, label)| {
                Span::styled((*label).to_string(), Style::default().fg(label_color()))
            }),
    )
}

/// One table row, styled so the invocation reads before its metadata.
///
/// A finished invocation goes flat grey for the seconds it lingers:
/// nothing on the row is live any more, so nothing on it should still
/// read as live.
fn process_row(row: &TrackedRow, manifest: ManifestPath, columns: &[usize]) -> Row<'static> {
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
    let cells = [
        Line::from(Span::styled(process.pid.to_string(), muted)),
        Line::from(Span::styled(process.start.clone(), muted)),
        Line::from(Span::styled(process.duration.clone(), muted)),
        state_cell(row),
        Line::from(vec![
            Span::styled(process.command.program.clone(), program),
            Span::raw(" "),
            Span::styled(process.command.line(manifest), arguments),
        ]),
        compiler_cell(row),
        Line::from(Span::styled(managed_text(process), muted)),
    ];
    Row::new(
        cells
            .into_iter()
            .enumerate()
            .filter(|(column, _)| columns.contains(column))
            .map(|(_, cell)| cell),
    )
}

/// The columns a cell draws, in table order.
///
/// Every column but `state` is always there. That one joins only where
/// it has something to say -- the summary, and only while some command
/// on it has a capture behind it -- because a column of dashes costs a
/// narrow tile the width its command line needs and reports nothing.
fn visible_columns(
    rows: &[&TrackedRow],
    kind: TableKind,
    placement: ProgressPlacement,
) -> Vec<usize> {
    let carries_state = rows.iter().any(|row| match row.process.state {
        // A heading is per directory and blocked is per row, so nothing
        // but the column can say which row is waiting.
        Some(RunState::Blocked) => true,
        Some(RunState::Compiling(_)) => placement == ProgressPlacement::Column,
        None => false,
    });
    (0..TABLE_HEADERS.len())
        .filter(|column| *column != STATE_COLUMN || carries_state)
        .filter(|column| kind.shows_invocation_detail() || !SUMMARY_HIDDEN_COLUMNS.contains(column))
        .collect()
}

/// The `state` cell: the reading and a bar of it, or the word for a
/// command that is not building at all.
///
/// A run with no capture behind it gets [`PROGRESS_ABSENT`] rather than
/// an empty cell, which would read as nought percent.
fn state_cell(row: &TrackedRow) -> Line<'static> {
    let muted = Style::default().fg(label_color());
    let Some(state) = row.process.state else {
        return Line::from(Span::styled(PROGRESS_ABSENT, muted));
    };
    let Some(progress) = state.reading() else {
        // Waiting is not failing, and it is not work either. The warning
        // colour says both, where the success green a reading gets would
        // claim the row is getting somewhere.
        return Line::from(Span::styled(
            STATE_BLOCKED,
            Style::default().fg(warning_color()),
        ));
    };
    let fill = if row.is_ended() {
        label_color()
    } else {
        success_color()
    };
    Line::from(progress_bar(progress, fill))
}

/// Cells the `state` column needs for one row.
fn state_width(state: Option<RunState>) -> usize {
    match state.map(RunState::reading) {
        None => PROGRESS_ABSENT.chars().count(),
        Some(None) => STATE_BLOCKED.chars().count(),
        Some(Some(_)) => PROGRESS_CELL_BAR_WIDTH,
    }
}

/// The reading behind a row: nothing for a row with no capture behind
/// it, and nothing for one that is waiting on a lock.
fn reading(row: &TrackedRow) -> Option<Progress> { row.process.state?.reading() }

/// How far along, written to a fixed width so a column of readings
/// stays in line as the number grows a digit.
fn percent_reading(progress: Progress) -> String {
    format!(
        "{:>width$}%",
        progress.percent(),
        width = PROGRESS_READING_WIDTH.saturating_sub(1)
    )
}

/// The `state` column's bar: one field of [`PROGRESS_CELL_BAR_WIDTH`]
/// cells with the reading set at its right, filled from the left.
///
/// The fill is a background rather than a run of glyphs, so it passes
/// under the reading instead of stopping short of it, and a finished
/// build ends as a solid ground behind `100%`. `REVERSED` is what puts
/// the text in the pane's own background colour over it, whatever
/// colour that is, rather than naming a second colour that every theme
/// would then have to keep legible against the first.
///
/// One cell of the field is still measured in eighths. Whole cells
/// alone would move the bar once every ninth of the build, and the
/// reading is right-aligned, so the cell the fill has reached is blank
/// for most of a run and free to carry the partial glyph.
fn progress_bar(progress: Progress, fill: Color) -> Vec<Span<'static>> {
    let per_cell = PROGRESS_CELL_PARTIALS.len().saturating_add(1);
    let eighths = progress
        .done
        .saturating_mul(PROGRESS_CELL_BAR_WIDTH)
        .saturating_mul(per_cell)
        / progress.total;
    let whole = eighths / per_cell;
    let partial = (eighths % per_cell)
        .checked_sub(1)
        .and_then(|index| PROGRESS_CELL_PARTIALS.get(index))
        .copied();

    let reading = percent_reading(progress);
    let lead = PROGRESS_CELL_BAR_WIDTH.saturating_sub(reading.chars().count());
    let ground = Style::default().fg(fill).add_modifier(Modifier::REVERSED);
    let muted = Style::default().fg(label_color());

    (0..PROGRESS_CELL_BAR_WIDTH)
        .map(|cell| {
            let character = cell
                .checked_sub(lead)
                .and_then(|index| reading.chars().nth(index));
            match (cell < whole, character, partial) {
                // Under the fill: the glyph rides on it, and a cell the
                // reading does not reach is the fill and nothing else.
                (true, character, _) => Span::styled(character.unwrap_or(' ').to_string(), ground),
                // The cell the fill has reached part way, with nothing
                // of the reading in it to be drawn over.
                (false, None, Some(eighth)) if cell == whole => {
                    Span::styled(eighth.to_string(), Style::default().fg(fill))
                },
                (false, None, _) => Span::styled(PROGRESS_CELL_EMPTY.to_string(), muted),
                (false, Some(character), _) => {
                    Span::styled(character.to_string(), Style::default().fg(fill))
                },
            }
        })
        .collect()
}

/// The rule a working-directory header carries to the right of the
/// directory, and the reading closing it.
///
/// Nothing at all when the command running there was not captured, or
/// when the directory is long enough that the rule left would be too
/// short to read as one.
fn heading_gauge(group: &PathGroup<'_>, width: u16) -> Vec<Span<'static>> {
    let Some(progress) = group.rows.iter().find_map(|row| Some((row, reading(row)?))) else {
        return Vec::new();
    };
    let (row, progress) = progress;
    let reading = percent_reading(progress);
    let taken = cell_width(SECTION_HEADER_INDENT)
        .saturating_add(cell_width(group.path))
        .saturating_add(cell_width(&reading))
        .saturating_add(PROGRESS_HEADING_MARGINS);
    let rule = width.saturating_sub(taken);
    if rule < PROGRESS_HEADING_MIN_WIDTH {
        return Vec::new();
    }
    let filled = usize::from(rule)
        .saturating_mul(progress.done)
        .checked_div(progress.total)
        .unwrap_or_default();
    let style = if row.is_ended() {
        Style::default().fg(label_color())
    } else {
        Style::default().fg(success_color())
    };
    vec![
        Span::raw(" "),
        Span::styled(PROGRESS_HEADING_FILLED.to_string().repeat(filled), style),
        Span::styled(
            PROGRESS_HEADING_EMPTY
                .to_string()
                .repeat(usize::from(rule).saturating_sub(filled)),
            Style::default().fg(label_color()),
        ),
        Span::raw(" "),
        Span::styled(reading, style),
    ]
}

/// The `runs` cell: how many cargo invocations this command is managing,
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

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use super::*;
    use crate::processes::CargoProcess;
    use crate::processes::CommandText;

    /// The state of a command compiling `done` of `total` units.
    fn compiling(done: usize, total: usize) -> RunState {
        RunState::Compiling(Progress { done, total })
    }

    /// A row for a command whose capture last reported `state`.
    fn row(state: Option<RunState>) -> TrackedRow { row_at("~/rust/cargo-tile", state) }

    /// A row for a command running in `path`.
    fn row_at(path: &str, state: Option<RunState>) -> TrackedRow {
        TrackedRow::from(CargoProcess {
            path: path.to_string(),
            pid: 41233,
            start: "11:04".to_string(),
            duration: "00:18".to_string(),
            compiler: None,
            state,
            managed: 0,
            command: CommandText::of("cargo", &["build"]),
        })
    }

    /// The cells of a bar, as text, so a test can read the field the way
    /// the pane draws it.
    fn bar_text(done: usize, total: usize) -> String {
        progress_bar(Progress { done, total }, success_color())
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    /// The cells the fill covers, which are the ones drawn in reverse so
    /// that the fill colour lands as their background.
    fn filled_cells(done: usize, total: usize) -> usize {
        progress_bar(Progress { done, total }, success_color())
            .iter()
            .filter(|span| span.style.add_modifier.contains(Modifier::REVERSED))
            .count()
    }

    #[test]
    fn a_bar_is_always_the_column_width_however_far_along_it_is() {
        for done in 0..=16 {
            assert_eq!(bar_text(done, 16).chars().count(), PROGRESS_CELL_BAR_WIDTH);
        }
    }

    /// The reading is set at the right of the field at every point in
    /// the build, so a column of them stays in line.
    #[test]
    fn the_reading_sits_at_the_right_of_the_field_whatever_the_fill() {
        for done in 0..=16 {
            let text = bar_text(done, 16);
            let reading = percent_reading(Progress { done, total: 16 });
            assert!(text.ends_with(reading.trim_end()), "{text:?}");
        }
    }

    #[test]
    fn a_build_that_has_done_nothing_has_no_cell_filled() {
        assert_eq!(filled_cells(0, 16), 0);
    }

    /// The whole field, the reading included: at a hundred percent the
    /// fill is a ground behind the number rather than a bar beside it.
    #[test]
    fn a_finished_build_fills_every_cell_of_the_field() {
        assert_eq!(filled_cells(16, 16), PROGRESS_CELL_BAR_WIDTH);
        assert!(bar_text(16, 16).ends_with("100%"));
    }

    /// One unit of sixteen is part way through the first cell -- too
    /// little to fill one, which is what the partial glyphs are for.
    #[test]
    fn a_build_part_way_through_a_cell_draws_the_eighth_it_reached() {
        let text = bar_text(1, 16);
        let reached = text.chars().next().unwrap();

        assert_eq!(filled_cells(1, 16), 0);
        assert!(PROGRESS_CELL_PARTIALS.contains(&reached), "{reached:?}");
    }

    /// A partial glyph is only ever drawn where the reading is not, so
    /// it can never take a digit's cell.
    #[test]
    fn the_fill_never_draws_over_a_digit_of_the_reading() {
        for done in 0..=16 {
            let text = bar_text(done, 16);
            let digits: String = text.chars().filter(char::is_ascii_digit).collect();
            assert_eq!(
                digits,
                percent_reading(Progress { done, total: 16 })
                    .trim()
                    .trim_end_matches('%'),
                "{text:?}"
            );
        }
    }

    #[test]
    fn the_state_column_stays_out_while_no_row_has_anything_to_say() {
        let rows = [row(None)];
        let columns = visible_columns(
            &rows.iter().collect::<Vec<&TrackedRow>>(),
            TableKind::Command,
            ProgressPlacement::Column,
        );
        assert!(!columns.contains(&STATE_COLUMN));
        assert_eq!(columns.len(), TABLE_HEADERS.len() - 1);
    }

    #[test]
    fn one_row_with_a_reading_brings_the_state_column_in_for_the_cell() {
        let rows = [row(None), row(Some(compiling(149, 403)))];
        let columns = visible_columns(
            &rows.iter().collect::<Vec<&TrackedRow>>(),
            TableKind::Command,
            ProgressPlacement::Column,
        );
        assert!(columns.contains(&STATE_COLUMN));
        assert_eq!(columns.len(), TABLE_HEADERS.len());
    }

    /// A command's own cell puts the reading on its working-directory
    /// header, so the column would only repeat it.
    #[test]
    fn a_commands_own_cell_leaves_the_state_column_out() {
        let rows = [row(Some(compiling(1, 2)))];
        let columns = visible_columns(
            &rows.iter().collect::<Vec<&TrackedRow>>(),
            TableKind::Command,
            ProgressPlacement::Heading,
        );
        assert!(!columns.contains(&STATE_COLUMN));
    }

    /// A reading each, in directories of their own: every heading can
    /// carry its own, so the column costs width for nothing.
    #[test]
    fn one_reading_per_directory_goes_on_the_headings() {
        let first = row_at("~/rust/bevy_hana", Some(compiling(99, 100)));
        let second = row_at("~/rust/hana_tool_graph", Some(compiling(5, 40)));

        assert_eq!(
            progress_placement(&[&first, &second]),
            ProgressPlacement::Heading
        );
    }

    /// Two commands compiling in one directory: the heading over them
    /// can only say one thing, so the readings go back in the column.
    #[test]
    fn two_readings_in_one_directory_bring_the_column_back() {
        let first = row_at("~/rust/bevy_hana", Some(compiling(99, 100)));
        let second = row_at("~/rust/bevy_hana", Some(compiling(5, 40)));

        assert_eq!(
            progress_placement(&[&first, &second]),
            ProgressPlacement::Column
        );
    }

    /// Only readings compete for the heading. A command with no capture
    /// behind it is not one, so it does not cost the cell its rules.
    #[test]
    fn a_second_command_without_a_reading_leaves_the_heading_alone() {
        let building = row_at("~/rust/bevy_hana", Some(compiling(99, 100)));
        let idle = row_at("~/rust/bevy_hana", None);

        assert_eq!(
            progress_placement(&[&building, &idle]),
            ProgressPlacement::Heading
        );
    }

    /// A heading is per directory and a wait is per row, so the column
    /// is the only place that can name which command is waiting -- even
    /// where every reading on the cell is on a heading already.
    #[test]
    fn a_blocked_row_brings_the_state_column_in_against_the_headings() {
        let rows = [
            row_at("~/rust/bevy_hana", Some(compiling(99, 100))),
            row_at("~/rust/bevy_hana", Some(RunState::Blocked)),
        ];

        let columns = visible_columns(
            &rows.iter().collect::<Vec<&TrackedRow>>(),
            TableKind::Command,
            ProgressPlacement::Heading,
        );

        assert!(columns.contains(&STATE_COLUMN));
    }

    /// A waiting command has no reading, so it does not contend for the
    /// heading the command holding the lock is ruling.
    #[test]
    fn a_blocked_row_does_not_take_the_heading_from_a_reading() {
        let building = row_at("~/rust/bevy_hana", Some(compiling(99, 100)));
        let waiting = row_at("~/rust/bevy_hana", Some(RunState::Blocked));

        assert_eq!(
            progress_placement(&[&building, &waiting]),
            ProgressPlacement::Heading
        );
    }

    /// The three things a row can say, each measured as it is drawn, so
    /// the fitted column is wide enough for whichever turns up.
    #[test]
    fn the_state_column_is_fitted_to_whichever_of_the_three_it_holds() {
        assert_eq!(state_width(None), PROGRESS_ABSENT.chars().count());
        assert_eq!(
            state_width(Some(RunState::Blocked)),
            STATE_BLOCKED.chars().count()
        );
        assert_eq!(
            state_width(Some(compiling(149, 403))),
            PROGRESS_CELL_BAR_WIDTH
        );
    }

    /// A summary row stands for a whole command, so the two columns
    /// describing a single invocation have nothing to say on it.
    #[test]
    fn the_summary_leaves_out_the_columns_that_describe_one_invocation() {
        let rows = [row(None)];

        let columns = visible_columns(
            &rows.iter().collect::<Vec<&TrackedRow>>(),
            TableKind::Summary,
            ProgressPlacement::Heading,
        );

        assert!(!columns.contains(&COMPILER_COLUMN));
        assert!(!columns.contains(&MANAGED_COLUMN));
        assert!(columns.contains(&COMMAND_COLUMN));
    }

    /// A command's own cell is where those two belong: its rows are the
    /// invocations they describe.
    #[test]
    fn a_commands_own_cell_keeps_them() {
        let rows = [row(None)];

        let columns = visible_columns(
            &rows.iter().collect::<Vec<&TrackedRow>>(),
            TableKind::Command,
            ProgressPlacement::Heading,
        );

        assert!(columns.contains(&COMPILER_COLUMN));
        assert!(columns.contains(&MANAGED_COLUMN));
    }
}
