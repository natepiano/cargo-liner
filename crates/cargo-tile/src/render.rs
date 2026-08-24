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
use ratatui::text::Text;
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
use tui_pane::PaneBorders;
use tui_pane::PaneFocusState;
use tui_pane::PaneFrameLabel;
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
use tui_pane::blend_color;
use tui_pane::default_pane_chrome;
use tui_pane::draw_clipped;
use tui_pane::error_color;
use tui_pane::hover_focus_color;
use tui_pane::inline_error_color;
use tui_pane::label_color;
use tui_pane::pane_background;
use tui_pane::render_status_line;
use tui_pane::secondary_text_color;
use tui_pane::selection_style;
use tui_pane::status_bar_color;
use tui_pane::success_color;
use tui_pane::text_default;
use tui_pane::title_color;
use tui_pane::warning_color;

use crate::app::App;
use crate::constants::ANCESTRY_ELISION;
use crate::constants::ANCESTRY_GAP_HEIGHT;
use crate::constants::ANCESTRY_LEVEL_INDENT;
use crate::constants::ANCESTRY_MIN_ELIDED_ROWS;
use crate::constants::APP_NAME;
use crate::constants::APP_VERSION;
use crate::constants::COMMAND_COLUMN;
use crate::constants::COMPILER_COLUMN;
use crate::constants::COMPILER_SEPARATOR_WIDTH;
use crate::constants::CPU_COLUMN;
use crate::constants::DURATION_COLUMN;
use crate::constants::GROUP_GAP_HEIGHT;
use crate::constants::GROUP_HEADER_HEIGHT;
use crate::constants::MANAGED_COLUMN;
use crate::constants::NO_PROCESSES_NOTE;
use crate::constants::PID_COLUMN;
use crate::constants::POPUP_CHROME_HEIGHT;
use crate::constants::POPUP_CHROME_WIDTH;
use crate::constants::PROGRESS_HEADING_EMPTY;
use crate::constants::PROGRESS_HEADING_FILLED;
use crate::constants::PROGRESS_HEADING_MARGINS;
use crate::constants::PROGRESS_HEADING_MIN_WIDTH;
use crate::constants::PROGRESS_HEADING_PHASE_MARGIN;
use crate::constants::PROGRESS_READING_TENTHS_WIDTH;
use crate::constants::PROGRESS_READING_WIDTH;
use crate::constants::PROGRESS_TENTHS_MIN_TOTAL;
use crate::constants::SETTINGS_POPUP_WIDTH;
use crate::constants::START_COLUMN;
use crate::constants::STATE_BLOCKED;
use crate::constants::STATE_COLUMN;
use crate::constants::STATUS_LINE_HEIGHT;
use crate::constants::SUMMARY_CELL_TITLE;
use crate::constants::SUMMARY_HIDDEN_COLUMNS;
use crate::constants::SUMMARY_LABEL_BORDER_RESERVE;
use crate::constants::SUMMARY_LABEL_RIGHT_INSET;
use crate::constants::TABLE_COLUMN_SPACING;
use crate::constants::TABLE_HEADER_HEIGHT;
use crate::constants::TABLE_HEADERS;
use crate::constants::TILE_NUMBER_INDENT;
use crate::constants::TILE_ROWS_CELL_LABEL;
use crate::constants::TILE_ROWS_CONTENT_LABEL;
use crate::constants::TILE_ROWS_READOUT_HEIGHT;
use crate::constants::TILE_ROWS_RIGHT_INSET;
use crate::globals::AppGlobalAction;
use crate::processes::Ancestor;
use crate::processes::CargoProcess;
use crate::processes::ManifestPath;
use crate::progress::Progress;
use crate::progress::RunState;
use crate::roster::Roster;
use crate::roster::TrackedGroup;
use crate::roster::TrackedRow;
use crate::sccache::LabelRunKind;
use crate::sccache::SccacheStats;
use crate::settings;
use crate::tiles::TileContent;
use crate::tiles::TileDemand;
use crate::tiles::TileDemands;
use crate::wrap;

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
    let widths = app.tiles.content_widths(area, initial_rows);
    let demands = tile_demands(
        &app.roster,
        &widths,
        &app.loaded_config.config.commands.hidden_when_idle,
    );
    app.tiles.sync(&demands, initial_rows);
    let placements = app.tiles.placements(area, initial_rows);
    let mut grid_lines = GridLines::new(area);
    for placement in &placements {
        // The ground a fading row is carried toward is the one its own
        // cell is painted on, which focus moves.
        let ground = pane_background(placement.frame.is_focused());
        let hidden_when_idle = &app.loaded_config.config.commands.hidden_when_idle;
        let content_rows = match placement.content {
            TileContent::Summary => demands.summary,
            TileContent::Group(id) => demands.rows_for(id),
            TileContent::Empty(_) => 0,
        };
        draw_clipped(frame.buffer_mut(), placement.frame, |buffer, inner| {
            draw_contents(
                buffer,
                &app.roster,
                placement.content,
                inner,
                ground,
                hidden_when_idle,
            );
            draw_rows_readout(buffer, inner, content_rows);
        });
        match placement.content {
            TileContent::Summary => {
                grid_lines.add_titled(placement.frame, SUMMARY_CELL_TITLE);
                for label in sccache_label(&app.sccache, placement.frame.rect()) {
                    grid_lines.add_label(placement.frame, label);
                }
            },
            TileContent::Group(_) | TileContent::Empty(_) => {
                grid_lines.add(placement.frame);
            },
        }
    }
    // Neighbouring tiles meet on one line, so no cell belongs to a
    // single tile and none of them can carry focus. Focus is the
    // background tint under a tile's contents instead.
    grid_lines.render(
        frame.buffer_mut(),
        default_pane_chrome(),
        PaneBorders::Shared,
    );
}

/// What every cell is asking for, each measured at the width it will be
/// drawn at.
///
/// The demand is counted here rather than in [`crate::roster`] because a
/// command line wraps, and how many lines it wraps to is something only
/// the table layout knows. The roster still says which groups get cells;
/// this says how tall each of them wants to be.
///
/// A group that has no cell yet -- a command first seen on this scan --
/// is measured at the narrowest cell on screen, which is the width it
/// will have once the grid has opened one for it.
fn tile_demands(
    roster: &Roster,
    widths: &[(TileContent, u16)],
    hidden_when_idle: &[String],
) -> TileDemands {
    let narrowest = widths
        .iter()
        .map(|&(_, width)| width)
        .min()
        .unwrap_or_default();
    let width_of = |wanted: TileContent| {
        widths
            .iter()
            .find(|&&(content, _)| content == wanted)
            .map_or(narrowest, |&(_, width)| width)
    };
    TileDemands {
        summary: table_height(
            &summary_rows(roster, hidden_when_idle),
            TableKind::Summary,
            width_of(TileContent::Summary),
            None,
        ),
        groups:  roster
            .tiled_ids(hidden_when_idle)
            .into_iter()
            .filter_map(|id| roster.groups().iter().find(|group| group.id == id))
            .map(|group| TileDemand {
                id:   group.id,
                rows: group_height(
                    group,
                    width_of(TileContent::Group(group.id)),
                    hidden_when_idle,
                ),
            })
            .collect(),
    }
}

/// Rows one command's cell lays out with all the room it could want: the
/// ancestry block and the blank row under it, then the table.
///
/// The whole chain counts, not the levels a cell of some particular
/// height has room for -- the ask is what the cell wants, and what it is
/// given is the answer. A lead drawn as the foot of its own chain is
/// left out of the table, the same as [`draw_group`] leaves it out.
fn group_height(group: &TrackedGroup, width: u16, hidden_when_idle: &[String]) -> usize {
    let leads_as_ancestor = group.leads_as_ancestor(hidden_when_idle);
    let rows: Vec<&TrackedRow> = group.rows().skip(usize::from(leads_as_ancestor)).collect();
    let chain = group
        .ancestry()
        .len()
        .saturating_add(usize::from(leads_as_ancestor));
    let above = if chain == 0 {
        0
    } else {
        chain.saturating_add(usize::from(ANCESTRY_GAP_HEIGHT))
    };
    above.saturating_add(table_height(
        &rows,
        TableKind::Command,
        width,
        Some(group.lead.process.path.as_str()),
    ))
}

/// Rows a table of `rows` lays out at `width`: the one column-label row
/// the whole cell shares, then a heading and the rows under it for each
/// working directory they were run in, with a gap between one directory
/// and the next.
///
/// A row is as tall as its command line wraps to, which is why this goes
/// through the same [`TableLayout`] and the same [`process_row`] the
/// draw does -- counting a row as one line is what had a cell ask for a
/// third of the rows it went on to lay out. The ground passed to the
/// layout only settles what colour a faded row is written in, so any of
/// them measures the same.
///
/// Only the gaps between directories count: [`draw_path_group`] advances
/// past one after the last directory too, but nothing follows it there.
fn table_height(rows: &[&TrackedRow], kind: TableKind, width: u16, pinned: Option<&str>) -> usize {
    if rows.is_empty() {
        return 0;
    }
    let area = Rect {
        x: 0,
        y: 0,
        width,
        height: TABLE_HEADER_HEIGHT,
    };
    let layout = TableLayout::of(rows, kind, area, pane_background(false));
    let groups = group_by_path(rows, pinned);
    let gaps = groups
        .len()
        .saturating_sub(1)
        .saturating_mul(usize::from(GROUP_GAP_HEIGHT));
    let laid_out: usize = groups
        .iter()
        .map(|group| {
            let lines: usize = group
                .rows
                .iter()
                .map(|row| usize::from(process_row(row, &layout).lines))
                .sum();
            usize::from(GROUP_HEADER_HEIGHT).saturating_add(lines)
        })
        .sum();
    usize::from(TABLE_HEADER_HEIGHT)
        .saturating_add(laid_out)
        .saturating_add(gaps)
}

/// Write what a cell's contents ask for against what the cell was
/// given, along the foot of the cell and over whatever its contents
/// drew there.
///
/// `rows` is the unrounded count the contents would take if nothing
/// stopped them -- what [`crate::tiles`] rounds to a step of demand
/// before dividing a column by it -- and `inner.height` is the rows the
/// cell actually has to draw into, which is one short of its allotment
/// wherever it shares a border with the cell below. The count is
/// written green while it fits and red once it does not.
fn draw_rows_readout(buffer: &mut Buffer, inner: Rect, rows: usize) {
    let asked = u16::try_from(rows).unwrap_or(u16::MAX);
    let reading = if asked <= inner.height {
        success_color()
    } else {
        error_color()
    };
    let line = Line::from(vec![
        Span::styled(TILE_ROWS_CONTENT_LABEL, Style::default().fg(label_color())),
        Span::styled(rows.to_string(), Style::default().fg(reading)),
        Span::styled(TILE_ROWS_CELL_LABEL, Style::default().fg(label_color())),
        Span::styled(
            inner.height.to_string(),
            Style::default().fg(text_default()),
        ),
    ]);
    let Some(area) = readout_area(inner, u16::try_from(line.width()).unwrap_or(u16::MAX)) else {
        return;
    };
    Paragraph::new(line).render(area, buffer);
}

/// The last row of a cell's interior, right-aligned and held off the
/// border, or `None` when the cell has no room for the readout at all.
fn readout_area(inner: Rect, width: u16) -> Option<Rect> {
    let room = inner.width.saturating_sub(TILE_ROWS_RIGHT_INSET);
    if inner.height < TILE_ROWS_READOUT_HEIGHT || room == 0 {
        return None;
    }
    let width = width.min(room);
    Some(Rect {
        x: inner
            .right()
            .saturating_sub(TILE_ROWS_RIGHT_INSET)
            .saturating_sub(width),
        y: inner.bottom().saturating_sub(TILE_ROWS_READOUT_HEIGHT),
        width,
        height: TILE_ROWS_READOUT_HEIGHT,
    })
}

/// What a cell holds inside its borders. `ground` is the colour the
/// cell is painted on, which a finished row's text fades toward, and
/// `hidden_when_idle` is what settles whether a command's own cell
/// draws it as a row or as the last step of its chain.
fn draw_contents(
    buffer: &mut Buffer,
    roster: &Roster,
    content: TileContent,
    inner: Rect,
    ground: Color,
    hidden_when_idle: &[String],
) {
    match content {
        TileContent::Summary => draw_summary(buffer, roster, inner, ground, hidden_when_idle),
        TileContent::Group(id) => {
            draw_group(buffer, roster, id, inner, ground, hidden_when_idle);
        },
        TileContent::Empty(number) => draw_number(buffer, number, inner),
    }
}

/// The summary cell: every invocation running anywhere, gathered under
/// the working directory it was run in.
///
/// Grouped by directory rather than by the command that launched it,
/// which is what the cells already do. A directory is where invocations
/// queue up -- one holds the build-directory lock and the rest wait on
/// it -- so heading them together says which path is backed up and by
/// what, whoever started them. The same lock read off the cells would
/// mean reading across them.
fn draw_summary(
    buffer: &mut Buffer,
    roster: &Roster,
    inner: Rect,
    ground: Color,
    hidden_when_idle: &[String],
) {
    draw_process_table(
        buffer,
        inner,
        &summary_rows(roster, hidden_when_idle),
        TableKind::Summary,
        ground,
        None,
    );
}

/// Every row the summary draws: one per command, and for a driver the
/// commands it is driving instead.
///
/// A driver `commands.hidden_when_idle` names gives up its own row, the
/// same as [`TrackedGroup::leads_as_ancestor`] gives it up in the
/// driver's cell. It compiles nothing and sits in a directory of its
/// own, so among rows gathered by working directory it would head a
/// directory holding nothing else; the invocations it drives say where
/// the work is, from the directories they are building in.
///
/// Every other command gives its lead row and nothing under it. What a
/// command started is its cell's business -- one `cargo nextest run`
/// whose suite runs `cargo mend` per case would otherwise put every one
/// of them in the summary and bury the handful of commands actually
/// worth reading. The lead carries the reading either way, since a row
/// takes the state of the nearest capture at or above it.
///
/// Read by [`draw_summary`] and by [`tile_demands`] both, so the cell is
/// measured over exactly the rows it goes on to lay out.
fn summary_rows<'a>(roster: &'a Roster, hidden_when_idle: &[String]) -> Vec<&'a TrackedRow> {
    let mut rows: Vec<&TrackedRow> = Vec::new();
    for group in roster.groups() {
        if group.leads_as_ancestor(hidden_when_idle) {
            rows.extend(group.rows().skip(1).filter(|row| !row.process.nested));
        } else {
            rows.push(&group.lead);
        }
    }
    rows
}

/// What sccache reports, written along the summary cell's top border.
///
/// The border rather than a row inside the cell: the summary is the one
/// cell competing for rows against the builds themselves, and the top
/// line is already drawn. It carries [`SUMMARY_CELL_TITLE`] at its left
/// and has the rest of its length spare.
///
/// `None` when no server is running, when nothing has been read yet, or
/// when the cell is too narrow for even the hit rate --
/// [`SccacheStats::label`] is what settles which of those it is.
fn sccache_label(sccache: &SccacheStats, rect: Rect) -> Vec<PaneFrameLabel> {
    let room = rect
        .width
        .saturating_sub(cell_width(SUMMARY_CELL_TITLE))
        .saturating_sub(SUMMARY_LABEL_BORDER_RESERVE);
    let Some(runs) = sccache.label(room) else {
        return Vec::new();
    };
    // Set from the right, so the reading stays where the eye last found
    // it as the grid opens and closes cells around it.
    let width = runs.iter().fold(0, |total: u16, run| {
        total.saturating_add(cell_width(&run.text))
    });
    let mut x = rect
        .right()
        .saturating_sub(SUMMARY_LABEL_RIGHT_INSET)
        .saturating_sub(width);
    // A label carries one style, so each run is set as a label of its
    // own beside the last. They land where they are put: the rung was
    // chosen to fit the room left over, so no run ever asks for a cell
    // another has taken.
    runs.into_iter()
        .map(|run| {
            let width = cell_width(&run.text);
            let area = Rect {
                x,
                y: rect.top(),
                width,
                height: 1,
            };
            x = x.saturating_add(width);
            PaneFrameLabel {
                area,
                text: run.text,
                style: Style::default().fg(run_color(run.kind)),
            }
        })
        .collect()
}

/// The colour a run of the sccache label is set in: the figures stand
/// out from the words naming them.
fn run_color(kind: LabelRunKind) -> Color {
    match kind {
        LabelRunKind::Name => label_color(),
        LabelRunKind::Value => warning_color(),
    }
}

/// One command's own cell: what launched the command, then every
/// invocation the summary put behind that command's single row.
///
/// The command itself is usually the first of those rows. A driver
/// that `commands.hidden_when_idle` names is the exception -- see
/// [`crate::roster::TrackedGroup::leads_as_ancestor`] -- and closes the chain instead,
/// leaving the table to the invocations the cell was opened for.
fn draw_group(
    buffer: &mut Buffer,
    roster: &Roster,
    id: u32,
    inner: Rect,
    ground: Color,
    hidden_when_idle: &[String],
) {
    let Some(group) = roster.groups().iter().find(|group| group.id == id) else {
        return;
    };
    let leads_as_ancestor = group.leads_as_ancestor(hidden_when_idle);
    let rows: Vec<&TrackedRow> = group.rows().skip(usize::from(leads_as_ancestor)).collect();
    let mut chain = group.ancestry().to_vec();
    if leads_as_ancestor {
        chain.push(as_ancestor(&group.lead.process));
    }
    let ancestry = carried(chain);
    // The lead's own fade goes into the block whether or not it is a
    // row there: the chain stands over the whole cell, and the cell
    // goes out when the command does.
    let faded = heading_fade(&rows).min(group.lead.faded());
    let used = draw_ancestry(buffer, inner, &ancestry, faded, ground);
    let table = Rect {
        y: inner.y.saturating_add(used),
        height: inner.height.saturating_sub(used),
        ..inner
    };
    // The directory pinned to the top of the cell is the command's own,
    // drawn as a row there or not: the invocations under a driver run
    // wherever the work is, and none of those is what the cell is about.
    draw_process_table(
        buffer,
        table,
        &rows,
        TableKind::Command,
        ground,
        Some(group.lead.process.path.as_str()),
    );
}

/// A command as the last step of its own cell's chain: its pid, and the
/// whole line it was typed as.
fn as_ancestor(process: &CargoProcess) -> Ancestor {
    let arguments = process.command.line(ManifestPath::Shown);
    let program = process.command.program.as_str();
    Ancestor {
        pid:            process.pid,
        command:        if arguments.is_empty() {
            program.to_string()
        } else {
            format!("{program} {arguments}")
        },
        // The command is what the chain is a chain *to*, so it is never
        // one of the steps the chain passes through.
        passes_through: false,
    }
}

/// The steps of `chain` a cell draws: everything that started
/// something, plus whatever stands at the foot.
///
/// A command a developer typed has a shell at the foot and nothing else
/// above it but the terminal, which says which window rather than who
/// typed it -- so dropping that one would leave the cell unable to tell
/// a command run by hand from one an editor or an agent ran. Every
/// shell further up did only pass a command through, and says no more
/// than that a terminal was involved.
///
/// Which step is the foot is why this runs here rather than in the
/// scan: a driver closes its own cell's chain, and that puts the
/// driver at the foot and the shell that started it back among the
/// steps passed through.
fn carried(chain: Vec<Ancestor>) -> Vec<Ancestor> {
    let last = chain.len().saturating_sub(1);
    chain
        .into_iter()
        .enumerate()
        .filter(|&(at, ref ancestor)| at == last || !ancestor.passes_through)
        .map(|(_, ancestor)| ancestor)
        .collect()
}

/// Draw what stands above a command into the top of `area`, outermost
/// first and one space deeper per level, answering how many rows that
/// took including the blank row below it.
///
/// The block fades the way a heading does -- with the least-faded row
/// under it -- so it holds its colour while a single invocation in the
/// cell is still running, and sinks with the cell when none is.
fn draw_ancestry(
    buffer: &mut Buffer,
    area: Rect,
    ancestry: &[Ancestor],
    faded: u8,
    ground: Color,
) -> u16 {
    let levels = ancestry_levels(ancestry, ancestry_budget(area.height));
    if levels.is_empty() {
        return 0;
    }
    let pid = blend_color(label_color(), ground, faded);
    let command = blend_color(secondary_text_color(), ground, faded);
    let lines: Vec<Line<'static>> = levels
        .iter()
        .enumerate()
        .map(|(level, ancestor)| ancestry_line(*ancestor, level, area.width, pid, command))
        .collect();
    // `u16` because the count came out of a budget measured in rows of
    // this same area.
    let height = u16::try_from(lines.len()).unwrap_or(u16::MAX);
    Paragraph::new(lines).render(Rect { height, ..area }, buffer);
    height.saturating_add(ANCESTRY_GAP_HEIGHT)
}

/// Rows the ancestry block may take at a cell of `height`.
///
/// Never more than half the cell, and the blank row under the block
/// comes out of that half: whatever the chain has to say, the table it
/// stands over is what the cell is for.
fn ancestry_budget(height: u16) -> usize {
    usize::from(height / 2).saturating_sub(usize::from(ANCESTRY_GAP_HEIGHT))
}

/// Which levels of `ancestry` a block of `budget` rows carries, `None`
/// standing for the levels left out.
///
/// A chain that fits is drawn whole. One that does not keeps both ends:
/// the top-level parent, and the levels nearest the command, which are
/// what say how it was actually started. Below
/// [`ANCESTRY_MIN_ELIDED_ROWS`] there is no room for two ends and an
/// elision between them, and the foot of the chain is what stays -- it
/// is the step closest to the work, and in a driver's cell it is the
/// driver itself.
fn ancestry_levels(ancestry: &[Ancestor], budget: usize) -> Vec<Option<&Ancestor>> {
    if ancestry.len() <= budget {
        return ancestry.iter().map(Some).collect();
    }
    if budget < ANCESTRY_MIN_ELIDED_ROWS {
        return ancestry[ancestry.len() - budget..]
            .iter()
            .map(Some)
            .collect();
    }
    let tail = budget - 2;
    ancestry
        .first()
        .map(Some)
        .into_iter()
        .chain(std::iter::once(None))
        .chain(ancestry[ancestry.len() - tail..].iter().map(Some))
        .collect()
}

/// One level of the ancestry block: its pid and what the process is,
/// set one space further in than the level above it.
///
/// The command is cut at the cell's edge rather than wrapped. A row
/// here identifies an ancestor rather than reporting it, and the head
/// of a command line is what does that -- wrapping one would spend
/// rows the table below is owed.
fn ancestry_line(
    ancestor: Option<&Ancestor>,
    level: usize,
    width: u16,
    pid: Color,
    command: Color,
) -> Line<'static> {
    let indent = format!(
        "{SECTION_HEADER_INDENT}{}",
        ANCESTRY_LEVEL_INDENT.repeat(level)
    );
    let Some(ancestor) = ancestor else {
        return Line::from(vec![
            Span::raw(indent),
            Span::styled(ANCESTRY_ELISION, Style::default().fg(pid)),
        ]);
    };
    let label = ancestor.pid.to_string();
    let room = usize::from(width)
        .saturating_sub(indent.chars().count())
        .saturating_sub(label.chars().count())
        .saturating_sub(1);
    Line::from(vec![
        Span::raw(indent),
        Span::styled(label, Style::default().fg(pid)),
        Span::raw(" "),
        Span::styled(
            truncated(&ancestor.command, room),
            Style::default().fg(command),
        ),
    ])
}

/// `text` cut to `cells`, the last cell kept for an ellipsis whenever
/// anything was taken off.
fn truncated(text: &str, cells: usize) -> String {
    if text.chars().count() <= cells {
        return text.to_string();
    }
    let kept = cells.saturating_sub(ANCESTRY_ELISION.chars().count());
    let mut out: String = text.chars().take(kept).collect();
    out.push_str(ANCESTRY_ELISION);
    out
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
    /// The summary: every invocation running, over every directory at
    /// once.
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

/// The invocations sharing one working directory.
struct PathGroup<'a> {
    /// The working directory, as it heads the group.
    path: &'a str,
    /// Every invocation running there, newest first.
    rows: Vec<&'a TrackedRow>,
}

/// How one cell's tables are laid out, settled once for the whole cell.
///
/// Every group in the cell is drawn against this, which is what keeps
/// the tables lined up down the cell instead of each one fitting itself
/// and the columns stepping in and out as the eye moves between them.
struct TableLayout {
    /// Column widths, in table order.
    constraints:   Vec<Constraint>,
    /// The columns this cell draws, in table order.
    columns:       Vec<usize>,
    /// Cells the `command` column absorbed, which is what a command
    /// line too long for it is wrapped to.
    command_width: u16,
    /// Whether a row spells out `--manifest-path`.
    manifest:      ManifestPath,
    /// The colour the cell is painted on, which a finished row's text
    /// is carried toward as it fades.
    ground:        Color,
}

impl TableLayout {
    /// The layout for a cell of `kind` drawing `rows` into `area`, over
    /// a cell painted `ground`.
    fn of(rows: &[&TrackedRow], kind: TableKind, area: Rect, ground: Color) -> Self {
        let columns = visible_columns(rows, kind);
        let constraints = fitted_constraints(rows, &columns);
        Self {
            command_width: command_column_width(indented(area).width, &constraints, &columns),
            constraints,
            columns,
            manifest: kind.manifest(),
            ground,
        }
    }

    /// `color` as something `faded` of the way out draws it: carried
    /// that far toward the ground the cell stands on, so a row on its
    /// way off the display sinks into the cell rather than switching
    /// off at the end of its spell.
    fn ink(&self, color: Color, faded: u8) -> Color { blend_color(color, self.ground, faded) }
}

/// How far the least-faded of `rows` has travelled, which is what
/// anything standing over them fades with: a heading or a column label
/// holds its colour while a single row under it is still running.
fn heading_fade(rows: &[&TrackedRow]) -> u8 {
    rows.iter().map(|row| row.faded()).min().unwrap_or_default()
}

/// Render a cargo table: one working-directory header per distinct path,
/// with that directory's invocations tabulated beneath it.
///
/// `pinned` is the directory that heads the cell whatever the rest sort
/// to. A command's own cell pins the command's: the invocations under
/// it are often somewhere else entirely -- a test run drives cargo in a
/// temporary directory per case, each one alive for seconds -- and
/// sorted with the rest those come out ahead of a home-relative path
/// and push the command being watched off the bottom of its own cell.
/// The summary pins nothing: every row there leads a command of its
/// own, so there is no one directory the cell is about.
fn draw_process_table(
    buffer: &mut Buffer,
    area: Rect,
    rows: &[&TrackedRow],
    kind: TableKind,
    ground: Color,
    pinned: Option<&str>,
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
    let layout = TableLayout::of(rows, kind, area, ground);
    Table::new(Vec::<Row>::new(), layout.constraints.iter().copied())
        .header(column_header(&layout, heading_fade(rows)))
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
    for group in group_by_path(rows, pinned) {
        if remaining.height == 0 {
            break;
        }
        let used = draw_path_group(buffer, remaining, &group, &layout);
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
///
/// A linear search per row is enough: the grouping key is a path a
/// developer is building in, and there are only ever a handful of those
/// at once.
fn group_by_path<'a>(rows: &[&'a TrackedRow], pinned: Option<&str>) -> Vec<PathGroup<'a>> {
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
    // Oldest work first, within a directory and between them. A run
    // that started later can only be waiting on one that started
    // earlier, so the earlier start reads above the runs queued behind
    // it -- the directory holding the build-directory lock over the
    // directories waiting on it, and inside each one the build over the
    // lint that queued behind it. Sorting the directories by name
    // instead put a nested crate's live build under blocked commands
    // that came after it, and leaving the rows in arrival order did the
    // same thing one directory down.
    for group in &mut groups {
        group.rows.sort_by_key(|row| row.process.started);
    }
    groups.sort_by_key(|group| {
        group
            .rows
            .first()
            .map_or(u64::MAX, |row| row.process.started)
    });
    // Whatever the rest sort to, the pinned directory heads the cell.
    // The others keep the order they had under it, so a group that
    // comes and goes moves nothing but itself.
    let Some(at) = pinned.and_then(|path| groups.iter().position(|group| group.path == path))
    else {
        return groups;
    };
    groups[..=at].rotate_right(1);
    groups
}

/// Draw one working directory's header and table into the top of `area`,
/// answering how many rows that took including the blank row below it.
fn draw_path_group(
    buffer: &mut Buffer,
    area: Rect,
    group: &PathGroup<'_>,
    layout: &TableLayout,
) -> u16 {
    let faded = heading_fade(&group.rows);
    let mut heading = vec![
        Span::raw(SECTION_HEADER_INDENT),
        Span::styled(
            group.path.to_string(),
            Style::default().fg(layout.ink(accent_color(), faded)),
        ),
    ];
    heading.extend(heading_gauge(group, area.width, layout));
    Paragraph::new(Line::from(heading)).render(
        Rect {
            height: GROUP_HEADER_HEIGHT.min(area.height),
            ..area
        },
        buffer,
    );

    // No header on this table: the column labels are drawn once for the
    // whole cell by `draw_process_table`, over these same constraints.
    // A row is as tall as its wrapped command line, so the table's own
    // height is the sum of them rather than one line per row.
    let rows: Vec<DrawnRow> = group
        .rows
        .iter()
        .map(|row| process_row(row, layout))
        .collect();
    let lines = rows
        .iter()
        .map(|drawn| drawn.lines)
        .fold(0, u16::saturating_add);
    let table_height = area.height.saturating_sub(GROUP_HEADER_HEIGHT).min(lines);
    Table::new(
        rows.into_iter().map(|drawn| drawn.row),
        layout.constraints.iter().copied(),
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
/// columns leave, so a long argument list wraps down the column instead
/// of pushing the columns that identify the invocation off the edge.
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
        widths.observe_cell_usize(CPU_COLUMN, process.cpu.chars().count());
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
fn column_header(layout: &TableLayout, faded: u8) -> Row<'static> {
    let style = Style::default().fg(layout.ink(label_color(), faded));
    Row::new(
        TABLE_HEADERS
            .iter()
            .enumerate()
            .filter(|(column, _)| layout.columns.contains(column))
            .map(|(_, label)| Span::styled((*label).to_string(), style)),
    )
}

/// One table row and how many lines it stands on.
struct DrawnRow {
    /// The row as the table takes it.
    row:   Row<'static>,
    /// Lines the row occupies. The command is the one cell that ever
    /// asks for more than one, and it asks for as many as its line
    /// wrapped to.
    lines: u16,
}

/// One table row, styled so the invocation reads before its metadata.
///
/// A finished invocation goes flat grey for the seconds it lingers:
/// nothing on the row is live any more, so nothing on it should still
/// read as live.
///
/// The command line is wrapped to the column's width rather than
/// truncated at it: an argument list that outruns the column carries on
/// down the rows of that column, and the row grows to hold it.
fn process_row(row: &TrackedRow, layout: &TableLayout) -> DrawnRow {
    let process = &row.process;
    let faded = row.faded();
    let muted = Style::default().fg(layout.ink(label_color(), faded));
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
    let command = wrap::wrapped(
        vec![
            Span::styled(process.command.program.clone(), program),
            Span::styled(process.command.line(layout.manifest), arguments),
        ],
        layout.command_width,
    );
    let lines = u16::try_from(command.height()).unwrap_or(u16::MAX);
    let cells = [
        Text::from(Span::styled(process.pid.to_string(), muted)),
        Text::from(Span::styled(process.start.clone(), muted)),
        Text::from(Span::styled(process.duration.clone(), muted)),
        Text::from(Span::styled(process.cpu.clone(), muted)),
        Text::from(state_cell(row, layout)),
        command,
        Text::from(compiler_cell(row, layout)),
        Text::from(Span::styled(managed_text(process), muted)),
    ];
    DrawnRow {
        row: Row::new(
            cells
                .into_iter()
                .enumerate()
                .filter(|(column, _)| layout.columns.contains(column))
                .map(|(_, cell)| cell),
        )
        .height(lines),
        lines,
    }
}

/// Cells the `command` column is left with once the fitted columns have
/// taken theirs.
///
/// [`Table`] solves its columns with [`Layout`], so this solves the same
/// one -- the same constraints at the same spacing -- and the command
/// line is wrapped to the width it is actually drawn in. Reading the
/// column's own [`Constraint`] instead would give the floor it is never
/// held to, since it is the column that absorbs the slack.
fn command_column_width(width: u16, constraints: &[Constraint], columns: &[usize]) -> u16 {
    let Some(column) = columns.iter().position(|column| *column == COMMAND_COLUMN) else {
        return 0;
    };
    Layout::horizontal(constraints.iter().copied())
        .spacing(TABLE_COLUMN_SPACING)
        .split(Rect {
            x: 0,
            y: 0,
            width,
            height: 1,
        })
        .get(column)
        .map_or(0, |rect| rect.width)
}

/// The columns a cell draws, in table order.
///
/// Every column but `state` is always there. That one joins only for a
/// row waiting on a lock: a heading is per directory and a wait is per
/// row, so nothing but the column can say which row is waiting. A
/// reading never brings it in -- the heading over the row is already
/// ruling that -- and an empty column costs a narrow tile the width its
/// command line needs while reporting nothing.
fn visible_columns(rows: &[&TrackedRow], kind: TableKind) -> Vec<usize> {
    let carries_state = rows
        .iter()
        .any(|row| matches!(row.process.state, Some(RunState::Blocked)));
    (0..TABLE_HEADERS.len())
        .filter(|column| *column != STATE_COLUMN || carries_state)
        .filter(|column| kind.shows_invocation_detail() || !SUMMARY_HIDDEN_COLUMNS.contains(column))
        .collect()
}

/// The `state` cell: the word for a row waiting on a lock, and an empty
/// cell for every other row.
///
/// The column is only ever in because some row is waiting, and it is
/// that row the eye is looking for. Every other row leaves the cell
/// blank rather than marking it, so the one word in the column is the
/// only thing in it.
///
/// A row that is getting somewhere has nothing to say here either way.
/// The working-directory heading over it is already ruling that
/// reading, and nothing in a Rust build gets past the build-directory
/// lock, so two commands in one directory are never both reporting at
/// once -- the heading has room for the one that is.
fn state_cell(row: &TrackedRow, layout: &TableLayout) -> Line<'static> {
    let Some(RunState::Blocked) = row.process.state else {
        return Line::default();
    };
    // Waiting is not failing, and it is not work either. The warning
    // colour says both, where a success green would claim the row is
    // getting somewhere.
    Line::from(Span::styled(
        STATE_BLOCKED,
        Style::default().fg(layout.ink(warning_color(), row.faded())),
    ))
}

/// Cells the `state` column needs for one row, which follows what
/// [`state_cell`] draws: the word for a wait, and nothing at all
/// otherwise.
fn state_width(state: Option<RunState>) -> usize {
    match state {
        Some(RunState::Blocked) => STATE_BLOCKED.chars().count(),
        None | Some(RunState::Working { .. }) => 0,
    }
}

/// How far along, written to a fixed width so a column of readings
/// stays in line as the number grows a digit.
///
/// A plan of more than a hundred units gets a tenth after the point.
/// Whole percent is the right resolution for a small plan, where one
/// unit moves the number by at least one; over a hundred units it
/// stalls the reading for several units at a time, and a run that is
/// visibly working has a header that reads as stuck.
fn percent_reading(progress: Progress) -> String {
    if progress.total > PROGRESS_TENTHS_MIN_TOTAL {
        let tenths = progress.percent_tenths();
        return format!(
            "{whole:>width$}.{tenth}%",
            whole = tenths / 10,
            tenth = tenths % 10,
            // The point, the tenth and the sign, which are what the
            // whole number has left of the reading's width.
            width = PROGRESS_READING_TENTHS_WIDTH.saturating_sub(3)
        );
    }
    format!(
        "{:>width$}%",
        progress.percent(),
        width = PROGRESS_READING_WIDTH.saturating_sub(1)
    )
}

/// The rule a working-directory header carries to the right of the
/// directory, the word for what the run is doing ahead of it, and the
/// reading closing it.
///
/// Nothing at all when the command running there was not captured, or
/// when the directory is long enough that the rule left would be too
/// short to read as one. The phase word is what the header gives up
/// first on its way there: a run counts two different things over its
/// life -- units, then tests -- so naming which is a reading of what,
/// but a cell too narrow to carry both still says how far along it is.
fn heading_gauge(group: &PathGroup<'_>, width: u16, layout: &TableLayout) -> Vec<Span<'static>> {
    let Some((row, (phase, progress))) = group
        .rows
        .iter()
        .find_map(|row| Some((row, row.process.state?.working()?)))
    else {
        return Vec::new();
    };
    let reading = percent_reading(progress);
    let fixed = cell_width(SECTION_HEADER_INDENT)
        .saturating_add(cell_width(group.path))
        .saturating_add(cell_width(&reading))
        .saturating_add(PROGRESS_HEADING_MARGINS);
    let labelled = fixed
        .saturating_add(cell_width(phase.label()))
        .saturating_add(PROGRESS_HEADING_PHASE_MARGIN);
    let (label, taken) = if width.saturating_sub(labelled) >= PROGRESS_HEADING_MIN_WIDTH {
        (Some(phase.label()), labelled)
    } else {
        (None, fixed)
    };
    let rule = width.saturating_sub(taken);
    if rule < PROGRESS_HEADING_MIN_WIDTH {
        return Vec::new();
    }
    let filled = usize::from(rule)
        .saturating_mul(progress.done)
        .checked_div(progress.total)
        .unwrap_or_default();
    let faded = row.faded();
    let filled_color = if row.is_ended() {
        label_color()
    } else {
        success_color()
    };
    let style = Style::default().fg(layout.ink(filled_color, faded));
    let empty = Style::default().fg(layout.ink(label_color(), faded));
    let mut spans = Vec::new();
    if let Some(label) = label {
        spans.push(Span::raw(" "));
        spans.push(Span::styled(label, empty));
    }
    spans.extend([
        Span::raw(" "),
        Span::styled(PROGRESS_HEADING_FILLED.to_string().repeat(filled), style),
        Span::styled(
            PROGRESS_HEADING_EMPTY
                .to_string()
                .repeat(usize::from(rule).saturating_sub(filled)),
            empty,
        ),
        Span::raw(" "),
        Span::styled(reading, style),
    ]);
    spans
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
fn compiler_cell(row: &TrackedRow, layout: &TableLayout) -> Line<'static> {
    let faded = row.faded();
    let driver = if row.is_ended() {
        label_color()
    } else {
        success_color()
    };
    let name = Style::default().fg(layout.ink(driver, faded));
    let count = Style::default().fg(layout.ink(label_color(), faded));
    row.process
        .compiler
        .as_ref()
        .map_or_else(Line::default, |compiler| {
            Line::from(vec![
                Span::styled(compiler.name, name),
                Span::styled(format!("\u{d7}{}", compiler.count), count),
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
    use std::time::Instant;

    use super::*;
    use crate::constants::PHASE_TESTING;
    use crate::constants::SIBLING_SUBCOMMAND_NAME;
    use crate::processes::CargoGroup;
    use crate::processes::CargoProcess;
    use crate::processes::CommandText;
    use crate::progress::Phase;

    /// The state of a command compiling `done` of `total` units.
    fn compiling(done: usize, total: usize) -> RunState {
        RunState::Working {
            phase:    Phase::Building,
            progress: Progress { done, total },
        }
    }

    /// The state of a command working through `done` of `total` tests.
    fn testing(done: usize, total: usize) -> RunState {
        RunState::Working {
            phase:    Phase::Testing,
            progress: Progress { done, total },
        }
    }

    /// A row for a command whose capture last reported `state`.
    fn row(state: Option<RunState>) -> TrackedRow { row_at("~/rust/cargo-tile", state) }

    /// What a working-directory header draws to the right of the
    /// directory, as text, for a cell `width` cells across.
    fn gauge_text(state: RunState, width: u16) -> String {
        let row = row_at(GAUGE_PATH, Some(state));
        let rows = [&row];
        let area = Rect {
            x: 0,
            y: 0,
            width,
            height: 1,
        };
        let layout = TableLayout::of(&rows, TableKind::Command, area, pane_background(false));
        let group = PathGroup {
            path: GAUGE_PATH,
            rows: rows.to_vec(),
        };
        heading_gauge(&group, width, &layout)
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    /// The working directory the gauge tests head their group with.
    const GAUGE_PATH: &str = "~/rust/cargo-tile";

    /// A row whose command line is long enough to outrun the column it
    /// is drawn in.
    fn long_row() -> TrackedRow {
        let mut row = row(None);
        row.process.command = CommandText::of(
            "cargo",
            &["build", "--features", "one,two,three", "--all-targets"],
        );
        row
    }

    /// One row of `buffer` as text, with the blanks to the right of it
    /// trimmed off.
    fn buffer_line(buffer: &Buffer, y: u16) -> String {
        let line: String = (0..buffer.area.width)
            .map(|x| buffer[(x, y)].symbol())
            .collect();
        line.trim_end().to_string()
    }

    /// A row for a command running in `path`.
    fn row_at(path: &str, state: Option<RunState>) -> TrackedRow { started_at(path, state, 0) }

    /// The same, for a command that started `started` seconds into the
    /// epoch -- which is what orders one directory against another.
    fn started_at(path: &str, state: Option<RunState>, started: u64) -> TrackedRow {
        TrackedRow::from(CargoProcess {
            path: path.to_string(),
            pid: 41233,
            start: "11:04".to_string(),
            started,
            duration: "00:18".to_string(),
            cpu: "12%".to_string(),
            compiler: None,
            state,
            managed: 0,
            nested: false,
            command: CommandText::of("cargo", &["build"]),
        })
    }

    /// One process above a command, for the ancestry tests.
    fn ancestor(pid: u32, command: &str) -> Ancestor {
        Ancestor {
            pid,
            command: command.to_string(),
            passes_through: false,
        }
    }

    /// One shell or login process above a command: a step the chain
    /// passes through rather than something that started anything.
    fn shell(pid: u32, command: &str) -> Ancestor {
        Ancestor {
            passes_through: true,
            ..ancestor(pid, command)
        }
    }

    /// A chain of `count` ancestors, outermost first.
    fn chain(count: u32) -> Vec<Ancestor> { (0..count).map(|step| ancestor(step, "sh")).collect() }

    /// The pids the block draws, `None` where a level was elided.
    fn drawn(levels: &[Option<&Ancestor>]) -> Vec<Option<u32>> {
        levels
            .iter()
            .map(|level| level.map(|ancestor| ancestor.pid))
            .collect()
    }

    #[test]
    fn a_chain_that_fits_is_drawn_whole() {
        let ancestry = chain(3);
        assert_eq!(
            drawn(&ancestry_levels(&ancestry, 4)),
            vec![Some(0), Some(1), Some(2)],
        );
    }

    /// The top of the chain is the cell's answer to what launched the
    /// command, and the levels nearest it say how -- so a short cell
    /// keeps both ends and drops the middle.
    #[test]
    fn a_chain_too_long_for_the_cell_keeps_both_ends() {
        let ancestry = chain(6);
        assert_eq!(
            drawn(&ancestry_levels(&ancestry, 4)),
            vec![Some(0), None, Some(4), Some(5)],
        );
    }

    /// Under three rows there is no room for two ends and an elision
    /// between them, and the foot of the chain is the end that matters
    /// -- in a driver's cell it is the driver itself.
    #[test]
    fn a_block_too_short_for_an_elision_keeps_the_foot_of_the_chain() {
        let ancestry = chain(6);
        assert_eq!(
            drawn(&ancestry_levels(&ancestry, 2)),
            vec![Some(4), Some(5)]
        );
    }

    /// Half a cell, with the blank row under the block taken out of
    /// that half: whatever the chain says, the table is what the cell
    /// is for.
    #[test]
    fn the_block_never_takes_more_than_half_the_cell() {
        assert_eq!(ancestry_budget(12), 5);
        assert_eq!(ancestry_budget(4), 1);
        assert_eq!(ancestry_budget(2), 0);
        assert_eq!(ancestry_budget(0), 0);
    }

    /// A cell with no room for the block at all draws none of it, and
    /// leaves the table every row it had.
    #[test]
    fn a_cell_too_short_for_the_block_spends_nothing_on_it() {
        assert!(ancestry_levels(&chain(3), ancestry_budget(2)).is_empty());
    }

    /// A command whose parents could not be read costs the table
    /// nothing, not even the blank row.
    #[test]
    fn a_command_with_no_ancestry_costs_the_table_no_rows() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 40, 10));
        let area = buffer.area;
        assert_eq!(
            draw_ancestry(&mut buffer, area, &[], 0, pane_background(false)),
            0
        );
    }

    /// The block reads as a staircase: outermost first, one space
    /// further in per level, each row its pid and what the process is.
    #[test]
    fn the_block_steps_one_space_in_per_level() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 40, 10));
        let area = buffer.area;
        let ancestry = vec![
            ancestor(6218, "zed"),
            ancestor(12445, "-zsh"),
            ancestor(18581, "claude"),
        ];

        let used = draw_ancestry(&mut buffer, area, &ancestry, 0, pane_background(false));

        assert_eq!(used, 4, "three levels and the blank row under them");
        assert_eq!(buffer_line(&buffer, 0), " 6218 zed");
        assert_eq!(buffer_line(&buffer, 1), "  12445 -zsh");
        assert_eq!(buffer_line(&buffer, 2), "   18581 claude");
    }

    /// A row here identifies an ancestor rather than reporting it, so
    /// a long command line is cut at the cell's edge rather than
    /// wrapped into rows the table below is owed.
    #[test]
    fn a_command_too_wide_for_the_cell_is_cut_rather_than_wrapped() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 20, 6));
        let area = buffer.area;
        let ancestry = vec![ancestor(6218, "node ~/.claude/local/claude")];

        let used = draw_ancestry(&mut buffer, area, &ancestry, 0, pane_background(false));

        assert_eq!(used, 2);
        assert_eq!(buffer_line(&buffer, 0), " 6218 node ~/.claud\u{2026}");
    }

    /// A cargo process for a group the roster is to carry.
    fn invocation(pid: u32, arguments: &[&str]) -> CargoProcess {
        CargoProcess {
            path: "~/rust/cargo-liner".to_string(),
            pid,
            start: "11:04".to_string(),
            started: 0,
            duration: "00:18".to_string(),
            cpu: "12%".to_string(),
            compiler: None,
            state: None,
            managed: 0,
            nested: false,
            command: CommandText::of("cargo", arguments),
        }
    }

    /// `commands.hidden_when_idle` as the config hands it over.
    fn hidden_when_idle() -> Vec<String> { vec![SIBLING_SUBCOMMAND_NAME.to_string()] }

    /// A roster carrying one command, with `rest` running under it.
    fn roster_of(lead: CargoProcess, rest: Vec<CargoProcess>) -> Roster {
        let mut roster = Roster::new();
        roster.observe(
            vec![CargoGroup {
                lead,
                rest,
                ancestry: vec![ancestor(6218, "zed"), shell(36744, "-zsh")],
            }],
            Instant::now(),
        );
        roster
    }

    /// A command typed by hand has a shell at the foot of its chain and
    /// nothing else above it worth naming, so that shell stays.
    #[test]
    fn the_shell_a_command_was_typed_into_stays() {
        let chain = vec![ancestor(6218, "zed"), shell(12445, "-zsh")];

        assert_eq!(
            carried(chain)
                .iter()
                .map(|step| step.pid)
                .collect::<Vec<u32>>(),
            vec![6218, 12445],
        );
    }

    /// A shell further up did only pass the command through, and says
    /// no more than that a terminal was involved.
    #[test]
    fn a_shell_partway_up_the_chain_goes() {
        let chain = vec![
            ancestor(6218, "zed"),
            shell(12444, "login -pf natepiano"),
            shell(12445, "-zsh"),
            ancestor(18581, "node ~/.claude/local/claude"),
        ];

        assert_eq!(
            carried(chain)
                .iter()
                .map(|step| step.pid)
                .collect::<Vec<u32>>(),
            vec![6218, 18581],
        );
    }

    /// A driver that `commands.hidden_when_idle` names closes its
    /// cell's chain rather than taking a row in the table: its row
    /// would say the same thing on every scan and cost the cell one of
    /// the invocations it was opened for.
    #[test]
    fn a_driver_closes_its_cells_chain_instead_of_heading_its_table() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 60, 14));
        let area = buffer.area;
        let roster = roster_of(
            invocation(4100, &[SIBLING_SUBCOMMAND_NAME]),
            vec![invocation(4212, &["build"])],
        );

        draw_group(
            &mut buffer,
            &roster,
            4100,
            area,
            Color::Reset,
            &hidden_when_idle(),
        );

        // The driver is the foot of the chain now, so the shell above
        // it is a step passed through like any other.
        assert_eq!(buffer_line(&buffer, 0), " 6218 zed");
        assert_eq!(buffer_line(&buffer, 1), "  4100 cargo port");
        let table: Vec<String> = (2..area.height).map(|y| buffer_line(&buffer, y)).collect();
        let table = table.join("\n");
        assert!(table.contains("4212"), "{table}");
        assert!(
            !table.contains("4100"),
            "the driver is not a row too: {table}"
        );
    }

    /// Every other command is a row in its own cell, the chain above it
    /// ending where the command begins.
    #[test]
    fn an_ordinary_command_still_heads_its_own_table() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 60, 14));
        let area = buffer.area;
        let roster = roster_of(invocation(4100, &["build"]), Vec::new());

        draw_group(
            &mut buffer,
            &roster,
            4100,
            area,
            Color::Reset,
            &hidden_when_idle(),
        );

        // Nothing closes this chain, so its shell is the foot and stays.
        assert_eq!(buffer_line(&buffer, 0), " 6218 zed");
        assert_eq!(buffer_line(&buffer, 1), "  36744 -zsh");
        let table: Vec<String> = (2..area.height).map(|y| buffer_line(&buffer, y)).collect();
        assert!(table.join("\n").contains("4100"), "{table:#?}");
    }

    /// A test run drives cargo in a directory per case, each alive for
    /// seconds, and those directories sort ahead of a home-relative one
    /// -- so the command being watched went under the fold of its own
    /// cell while the rows that pushed it there came and went.
    #[test]
    fn a_commands_own_directory_heads_its_cell_however_the_rest_sort() {
        let lead = row_at("~/rust/cargo-berth-init", None);
        let first = row_at("/private/var/folders/T/case-1", None);
        let second = row_at("/private/var/folders/T/case-2", None);
        let rows = [&lead, &first, &second];

        let pinned = group_by_path(&rows, Some("~/rust/cargo-berth-init"));

        assert_eq!(
            pinned.iter().map(|group| group.path).collect::<Vec<&str>>(),
            [
                "~/rust/cargo-berth-init",
                "/private/var/folders/T/case-1",
                "/private/var/folders/T/case-2"
            ]
        );
    }

    /// The summary is about no one directory, so nothing is pinned and
    /// the directories fall in the order their work began.
    #[test]
    fn the_summary_pins_no_directory() {
        let lead = row_at("~/rust/cargo-berth-init", None);
        let other = row_at("/private/var/folders/T/case-1", None);
        let rows = [&lead, &other];

        let sorted = group_by_path(&rows, None);

        assert_eq!(
            sorted.first().map(|group| group.path),
            Some("~/rust/cargo-berth-init"),
            "nothing pinned, and neither started first, so the order stands"
        );
    }

    /// A directory holding the build-directory lock started before
    /// whatever is queued behind it, and the eye wants the run doing
    /// the work above the runs waiting on it. Sorting by name put a
    /// nested crate's live build under blocked commands that came after
    /// it, since the nested path sorts second.
    #[test]
    fn a_directory_that_started_first_heads_the_summary() {
        let building = started_at("~/rust/hana_recovery/crates/hana", None, 100);
        let blocked = started_at("~/rust/hana_recovery", Some(RunState::Blocked), 160);
        let rows = [&blocked, &building];

        let sorted = group_by_path(&rows, None);

        assert_eq!(
            sorted.iter().map(|group| group.path).collect::<Vec<&str>>(),
            ["~/rust/hana_recovery/crates/hana", "~/rust/hana_recovery"]
        );
    }

    /// Two commands in one directory, the second necessarily waiting on
    /// the first. Reading them in arrival order showed the lint that had
    /// just queued above the test run it was queued behind.
    #[test]
    fn a_directorys_own_rows_read_oldest_first() {
        let building = started_at("~/rust/cargo-liner", None, 100);
        let queued = started_at("~/rust/cargo-liner", Some(RunState::Blocked), 160);
        let rows = [&queued, &building];

        let sorted = group_by_path(&rows, None);

        assert_eq!(
            sorted.first().map(|group| group
                .rows
                .iter()
                .map(|row| row.process.started)
                .collect::<Vec<u64>>()),
            Some(vec![100, 160])
        );
    }

    /// A run counts units and then tests, so a reading says nothing on
    /// its own about which of the two it is a reading of.
    #[test]
    fn a_header_names_the_phase_its_reading_came_from() {
        assert!(
            gauge_text(compiling(149, 403), 60).starts_with(" building "),
            "{:?}",
            gauge_text(compiling(149, 403), 60)
        );
        assert!(
            gauge_text(testing(12, 24), 60).starts_with(" testing "),
            "{:?}",
            gauge_text(testing(12, 24), 60)
        );
    }

    /// A plan of hundreds of units moves a whole percent only every
    /// few units, so the reading needs a tenth to keep up with a run
    /// that is plainly getting somewhere.
    #[test]
    fn a_reading_of_more_than_a_hundred_units_carries_a_tenth() {
        assert!(
            gauge_text(compiling(149, 403), 60).ends_with(" 36.9%"),
            "{:?}",
            gauge_text(compiling(149, 403), 60)
        );
        assert!(
            gauge_text(compiling(403, 403), 60).ends_with("100.0%"),
            "{:?}",
            gauge_text(compiling(403, 403), 60)
        );
    }

    /// Up to a hundred units the count already moves the whole number
    /// every time, and a tenth would only ever read as nought.
    #[test]
    fn a_reading_of_a_hundred_units_or_fewer_stays_a_whole_number() {
        assert!(
            gauge_text(testing(12, 24), 60).ends_with(" 50%"),
            "{:?}",
            gauge_text(testing(12, 24), 60)
        );
        assert!(
            gauge_text(compiling(99, 100), 60).ends_with(" 99%"),
            "{:?}",
            gauge_text(compiling(99, 100), 60)
        );
    }

    /// The word is what the header gives up first, the reading and its
    /// rule being what the cell is narrow for.
    #[test]
    fn a_header_too_narrow_for_the_phase_word_still_rules_its_reading() {
        let text = gauge_text(testing(12, 24), 30);

        assert!(!text.contains(PHASE_TESTING), "{text:?}");
        assert!(text.ends_with('%'), "{text:?}");
    }

    #[test]
    fn the_state_column_stays_out_while_no_row_has_anything_to_say() {
        let rows = [row(None)];
        let columns = visible_columns(
            &rows.iter().collect::<Vec<&TrackedRow>>(),
            TableKind::Command,
        );
        assert!(!columns.contains(&STATE_COLUMN));
        assert_eq!(columns.len(), TABLE_HEADERS.len() - 1);
    }

    /// The heading over the row is ruling the reading, so the column
    /// has nothing to add and does not cost the cell its width.
    #[test]
    fn a_reading_never_brings_the_state_column_in() {
        let rows = [row(None), row(Some(compiling(149, 403)))];
        let columns = visible_columns(
            &rows.iter().collect::<Vec<&TrackedRow>>(),
            TableKind::Command,
        );
        assert!(!columns.contains(&STATE_COLUMN));
        assert_eq!(columns.len(), TABLE_HEADERS.len() - 1);
    }

    /// Two commands in one directory are never both reporting: the
    /// second is waiting on the build-directory lock, which is the one
    /// thing the column is still for.
    #[test]
    fn a_blocked_row_brings_the_state_column_in() {
        let rows = [
            row_at("~/rust/bevy_hana", Some(compiling(99, 100))),
            row_at("~/rust/bevy_hana", Some(RunState::Blocked)),
        ];

        let columns = visible_columns(
            &rows.iter().collect::<Vec<&TrackedRow>>(),
            TableKind::Command,
        );

        assert!(columns.contains(&STATE_COLUMN));
    }

    /// Only a wait takes any width here, so the fitted column is the
    /// width of that one word however many rows are drawn beside it.
    #[test]
    fn only_a_wait_is_worth_any_width_in_the_state_column() {
        assert_eq!(state_width(None), 0);
        assert_eq!(
            state_width(Some(compiling(149, 403))),
            0,
            "a reading is the heading's to say",
        );
        assert_eq!(
            state_width(Some(RunState::Blocked)),
            STATE_BLOCKED.chars().count()
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
        );

        assert!(columns.contains(&COMPILER_COLUMN));
        assert!(columns.contains(&MANAGED_COLUMN));
    }

    /// The `command` column is the one that absorbs the slack, so what
    /// it is worth has to come off the solved layout rather than off the
    /// `Min` it is declared with.
    #[test]
    fn the_command_column_is_measured_at_the_width_it_absorbs() {
        let rows = [long_row()];
        let rows: Vec<&TrackedRow> = rows.iter().collect();
        let columns = visible_columns(&rows, TableKind::Command);
        let constraints = fitted_constraints(&rows, &columns);

        let narrow = command_column_width(50, &constraints, &columns);
        let wide = command_column_width(80, &constraints, &columns);

        assert!(narrow > cell_width(TABLE_HEADERS[COMMAND_COLUMN]));
        assert_eq!(wide.saturating_sub(narrow), 30);
    }

    /// A command that outruns its column carries on down the rows of
    /// that column: every line after the first starts where the column
    /// starts, and nothing of it is dropped.
    #[test]
    fn a_long_command_wraps_within_its_own_column() {
        let area = Rect::new(0, 0, 56, 8);
        let mut buffer = Buffer::empty(area);
        let rows = [long_row()];

        draw_process_table(
            &mut buffer,
            area,
            &rows.iter().collect::<Vec<&TrackedRow>>(),
            TableKind::Command,
            Color::Reset,
            None,
        );

        // The column labels are drawn once at the top of the cell, so
        // the header row is where the column's own left edge is.
        let header = buffer_line(&buffer, 0);
        let left = header.find(TABLE_HEADERS[COMMAND_COLUMN]).unwrap();
        // Row zero is the labels and row one the working directory, so
        // the invocation starts on row two and runs to the first blank.
        let lines: Vec<String> = (2..buffer.area.height)
            .map(|y| buffer_line(&buffer, y))
            .take_while(|line| !line.is_empty())
            .collect();

        assert!(lines.len() > 1, "{lines:#?}");
        for line in lines.iter().skip(1) {
            assert!(line.len() > left, "{line:?}");
            assert!(line[..left].trim().is_empty(), "{line:?}");
        }
        assert_eq!(
            lines
                .iter()
                .map(|line| line[left..].trim())
                .collect::<Vec<&str>>()
                .join(" "),
            "cargo build --features one,two,three --all-targets"
        );
    }

    /// The rows below a wrapped one are pushed down by it rather than
    /// drawn over it, so the group is as tall as its rows came out.
    #[test]
    fn a_wrapped_row_makes_the_group_taller() {
        let area = Rect::new(0, 0, 56, 12);
        let mut buffer = Buffer::empty(area);
        let rows = [long_row(), long_row()];

        draw_process_table(
            &mut buffer,
            area,
            &rows.iter().collect::<Vec<&TrackedRow>>(),
            TableKind::Command,
            Color::Reset,
            None,
        );

        let header = buffer_line(&buffer, 0);
        let left = header.find(TABLE_HEADERS[COMMAND_COLUMN]).unwrap();
        for y in 2..6 {
            let line = buffer_line(&buffer, y);
            assert!(line.len() > left, "row {y} is empty: {line:?}");
        }
    }
}
