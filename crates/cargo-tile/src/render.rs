//! Frame rendering: the app's panes, the framework status line along the
//! bottom, and whichever framework overlay is open above them.

use std::borrow::Cow;
use std::cell::RefCell;
use std::path::Path;

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Constraint;
use ratatui::layout::Layout;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::text::Text;
use ratatui::widgets::Paragraph;
use ratatui::widgets::Row;
use ratatui::widgets::Table;
use ratatui::widgets::Widget;
use tui_pane::AttractWork;
use tui_pane::BarPalette;
use tui_pane::ColumnSpec;
use tui_pane::ColumnWidths;
use tui_pane::Keymap;
use tui_pane::PaneFrameLabel;
use tui_pane::SECTION_HEADER_INDENT;
use tui_pane::SECTION_ITEM_INDENT;
use tui_pane::ScanIndicator;
use tui_pane::StatusLine;
use tui_pane::StatusLineGlobal;
use tui_pane::StatusLineNote;
use tui_pane::SummaryFoot;
use tui_pane::TileCells;
use tui_pane::TileGridContents;
use tui_pane::Updates;
use tui_pane::accent_color;
use tui_pane::blend_color;
use tui_pane::draw_attract_layers;
use tui_pane::label_color;
use tui_pane::pane_background;
use tui_pane::render_status_line;
use tui_pane::secondary_text_color;
use tui_pane::success_color;
use tui_pane::text_default;
use tui_pane::warning_color;
use unicode_width::UnicodeWidthChar;
use unicode_width::UnicodeWidthStr;

use crate::app::App;
use crate::app::ProcessTree;
use crate::census;
use crate::census::Ancestor;
use crate::census::CargoProcess;
use crate::census::CompilerObservation;
use crate::census::InvocationId;
use crate::census::Measurement;
use crate::census::ProcessOwner;
use crate::census::RowProvenance;
use crate::census::RunStart;
use crate::census::VisibleParent;
use crate::census::command_text::CommandText;
use crate::constants::ACCOUNT_HEADING_CLOSE;
use crate::constants::ACCOUNT_HEADING_OPEN;
use crate::constants::ANCESTRY_GAP_HEIGHT;
use crate::constants::ANCESTRY_LEVEL_INDENT;
use crate::constants::ANCESTRY_MIN_COMMAND_WIDTH;
use crate::constants::ANCESTRY_MIN_ELIDED_ROWS;
use crate::constants::APP_NAME;
use crate::constants::APP_VERSION;
use crate::constants::ATTRACT_NOTE_LABEL;
use crate::constants::BYTES_PER_GIBIBYTE;
use crate::constants::COMMAND_COLUMN;
use crate::constants::COMPILER_COLUMN;
use crate::constants::COMPILER_SEPARATOR_WIDTH;
use crate::constants::CPU_COLUMN;
use crate::constants::DURATION_COLUMN;
use crate::constants::ELISION;
use crate::constants::FROZEN_NOTE_LABEL;
use crate::constants::GROUP_GAP_HEIGHT;
use crate::constants::GROUP_HEADER_HEIGHT;
use crate::constants::HEADING_MIN_TAIL;
use crate::constants::MANAGED_COLUMN;
use crate::constants::MEMORY_COLUMN;
use crate::constants::MEMORY_UNIT;
use crate::constants::NO_PROCESSES_NOTE;
use crate::constants::PARENT_COLUMN;
use crate::constants::PARTIAL_TOTAL_MARK;
use crate::constants::PID_COLUMN;
use crate::constants::PROCESS_TREE_NOTE_LABEL;
use crate::constants::PROGRESS_HEADING_EMPTY;
use crate::constants::PROGRESS_HEADING_FILLED;
use crate::constants::PROGRESS_HEADING_MARGINS;
use crate::constants::PROGRESS_HEADING_MIN_WIDTH;
use crate::constants::PROGRESS_HEADING_PHASE_MARGIN;
use crate::constants::PROGRESS_READING_TENTHS_WIDTH;
use crate::constants::PROGRESS_READING_WIDTH;
use crate::constants::PROGRESS_TENTHS_MIN_TOTAL;
use crate::constants::START_COLUMN;
use crate::constants::STATE_BLOCKED;
use crate::constants::STATE_COLUMN;
use crate::constants::STATUS_LINE_HEIGHT;
use crate::constants::SUMMARY_CELL_TITLE;
use crate::constants::SUMMARY_HIDDEN_COLUMNS;
use crate::constants::SUMMARY_LABEL_BORDER_RESERVE;
use crate::constants::SUMMARY_LABEL_RIGHT_INSET;
use crate::constants::SUMMARY_MEMORY_LABEL;
use crate::constants::TABLE_COLUMN_DROP_ORDER;
use crate::constants::TABLE_COLUMN_SPACING;
use crate::constants::TABLE_HEADER_HEIGHT;
use crate::constants::TABLE_HEADERS;
use crate::constants::TIGHT_TABLE_COLUMN_SPACING;
use crate::constants::UNAVAILABLE_MEASUREMENT;
use crate::globals::AppGlobalAction;
use crate::progress::Progress;
use crate::progress::capture::CaptureRootIndex;
use crate::progress::capture_read::CaptureLookup;
use crate::progress::capture_read::CaptureRead;
use crate::progress::capture_read::Phase;
use crate::progress::capture_read::RunState;
use crate::progress::capture_roots::AccountName;
use crate::registration::WorkingDirectoryIdentity;
use crate::root_scan::RootIncarnation;
use crate::roster::FamilyHead;
use crate::roster::ParentFamily;
use crate::roster::Roster;
use crate::roster::TrackedGroup;
use crate::roster::TrackedRow;
use crate::sccache::LabelRunKind;
use crate::sccache::SccacheStats;
use crate::settings;
use crate::theme;
use crate::tiles::TileContent;
use crate::tiles::TileDemand;
use crate::tiles::TileDemands;

/// A numeric account and its independently resolved display label.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Account {
    /// Ownership identity comes from the opened root or the process table, never a pathname.
    pub(crate) uid:  u32,
    /// Failure to resolve a label does not change grouping.
    pub(crate) name: AccountName,
}

/// Root incarnation and owner qualify a captured working directory.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CaptureContext {
    /// Stable account position distinguishes capture directories.
    pub(crate) root:        CaptureRootIndex,
    /// Replacing a directory invalidates its retained grouping identity.
    pub(crate) incarnation: RootIncarnation,
    /// Numeric identity remains separate from its display name.
    pub(crate) account:     Account,
}

/// What a gauge can show, preserving the reason it has no counter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CounterState {
    /// A phase owns the current numerator and denominator.
    Working {
        /// Counters from different phases must not be combined.
        phase:    Phase,
        /// The current progress in this phase.
        progress: Progress,
    },
    /// The capture reports a build-directory wait.
    Blocked,
    /// Readable output currently supplies no counter, including after Finished.
    NoCurrentProgress,
    /// The selected capture could not be read.
    Unavailable,
    /// No registration supplies a counter for this invocation.
    Unregistered,
}

impl From<&CaptureLookup> for CounterState {
    fn from(lookup: &CaptureLookup) -> Self {
        match lookup {
            CaptureLookup::Registered(CaptureRead::Progress(RunState::Working {
                phase,
                progress,
            })) => Self::Working {
                phase:    *phase,
                progress: *progress,
            },
            CaptureLookup::Registered(CaptureRead::Progress(RunState::Blocked)) => Self::Blocked,
            CaptureLookup::Registered(CaptureRead::NoCurrentProgress) => Self::NoCurrentProgress,
            CaptureLookup::Registered(CaptureRead::Unreadable(_)) => Self::Unavailable,
            CaptureLookup::Unregistered => Self::Unregistered,
        }
    }
}

/// How much of an invocation's command line a cell prints.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SummaryDetail {
    /// The whole line, the way a command's own cell shows it.
    Full,
    /// The manifest path and the summary's noise flags left out, the
    /// way the summary shows it. Every row there already sits under the
    /// working directory heading its group, and cargo is handed the
    /// manifest as an absolute path -- long enough to push the
    /// subcommand off the edge of a narrow cell to repeat what the
    /// header just said.
    Trimmed,
}

/// The memory of every running command together, as far as it can be read.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SummaryMemoryTotal {
    /// Nothing is running, so there is no total.
    NoRunningCommands,
    /// Every running command was read; the bytes are the whole total.
    Complete(u64),
    /// At least one running command could not be read; the bytes are the rest.
    AtLeast(u64),
    /// Commands are running and none could be read.
    NoReadableCommands,
}

impl SummaryMemoryTotal {
    /// The text this state supplies to the summary cell's readout row.
    fn foot(&self) -> SummaryFoot {
        let value = match *self {
            Self::NoRunningCommands => return SummaryFoot::Empty,
            Self::Complete(bytes) => memory_label(bytes),
            Self::AtLeast(bytes) => format!("{}{PARTIAL_TOTAL_MARK}", memory_label(bytes)),
            Self::NoReadableCommands => UNAVAILABLE_MEASUREMENT.to_string(),
        };
        SummaryFoot::Text(Line::from(vec![
            Span::styled(SUMMARY_MEMORY_LABEL, Style::default().fg(label_color())),
            Span::styled(value, Style::default().fg(text_default())),
        ]))
    }
}

/// Draw one frame: panes fill the body above the status line, and toasts
/// are drawn inside the grid's outer frame. An open overlay floats above
/// both.
pub(crate) fn draw(frame: &mut Frame, app: &mut App, keymap: &Keymap<App>) {
    let [body, status] =
        Layout::vertical([Constraint::Min(0), Constraint::Length(STATUS_LINE_HEIGHT)])
            .areas(frame.area());

    // What gets a cell, not what the roster holds: a command held back
    // by `commands.hidden_when_idle` keeps a summary line and draws no
    // cell, and a screen with nothing but that on it is one the reader
    // reads as idle. `groups` counts it and would keep the attract
    // screen off for as long as a `cargo port` watcher sat there.
    let work = if app
        .roster
        .tiled_ids(&app.loaded_config.config.commands.hidden_when_idle)
        .is_empty()
    {
        AttractWork::Idle
    } else {
        AttractWork::Running
    };
    let updates = app.updates;
    draw_attract_layers(frame, app, body, work, updates, |frame, app, contents| {
        draw_panes(frame, app, body, contents);
    });
    draw_status_line(frame, app, keymap, status);
    tui_pane::render_toasts(frame, &mut app.framework, tui_pane::frame_inner(body));
    app.favorites_overlay.render(frame);
    tui_pane::draw_framework_overlay(frame, app, keymap, settings::rows);
}

/// Draw the tile grid into the body above the status line.
///
/// The summary cell carries one row per command; every other cell
/// carries one command's own invocations, which is what the summary
/// collapsed into that command's single row.
/// [`tui_pane::draw_tile_grid`] decides where each cell goes, how far
/// through a transition it is, and draws the frames and readouts;
/// [`Cells`] is what goes inside them.
///
/// With [`TileGridContents::Hidden`] the frames, the borders and the
/// titles remain and the tables, the readouts and the cache figures go
/// -- which is what the grid looks like while the attract screen is
/// arriving or leaving.
fn draw_panes(frame: &mut Frame, app: &mut App, area: Rect, contents: TileGridContents) {
    let growth = app.loaded_config.config.tiles.growth();
    let cells = Cells::new(
        &app.roster,
        &app.loaded_config.config.commands.hidden_when_idle,
        app.tree,
        &app.sccache,
    );
    app.tiles.set_view(app.loaded_config.config.tiles.view);
    tui_pane::draw_tile_grid(
        frame.buffer_mut(),
        &mut app.tiles,
        area,
        growth,
        contents,
        &cells,
    );
}

/// What the tile grid's cells hold, borrowed out of [`App`] for one
/// frame.
struct Cells<'a> {
    /// Every command group, which the summary gathers and each group's
    /// own cell draws.
    roster:           &'a Roster,
    /// `commands.hidden_when_idle`, which settles whether a driver draws
    /// as a row or as the last step of its chain.
    hidden_when_idle: &'a [String],
    /// How much of what stands above a command goes over its table.
    tree:             ProcessTree,
    /// What sccache reports, written along the summary cell's top border.
    sccache:          &'a SccacheStats,
    /// Content-row answers already computed during this draw.
    row_uses:         RefCell<Vec<MeasuredCellRowUse>>,
}

impl<'a> Cells<'a> {
    /// Cell provider with an empty cache for this draw.
    const fn new(
        roster: &'a Roster,
        hidden_when_idle: &'a [String],
        tree: ProcessTree,
        sccache: &'a SccacheStats,
    ) -> Self {
        Self {
            roster,
            hidden_when_idle,
            tree,
            sccache,
            row_uses: RefCell::new(Vec::new()),
        }
    }

    /// Painted and retained rows for one allocation, computed once per draw.
    fn row_use(&self, content: &TileContent, area: Rect) -> GroupRowUse {
        if let Some(use_rows) = self
            .row_uses
            .borrow()
            .iter()
            .find(|measured| measured.content == *content && measured.area == area)
            .map(|measured| measured.use_rows)
        {
            return use_rows;
        }
        let use_rows =
            content_row_use(self.roster, content, area, self.hidden_when_idle, self.tree);
        self.row_uses.borrow_mut().push(MeasuredCellRowUse {
            content: content.clone(),
            area,
            use_rows,
        });
        use_rows
    }
}

impl TileCells<InvocationId> for Cells<'_> {
    fn summary_title(&self) -> &str { SUMMARY_CELL_TITLE }

    fn summary_foot(&self) -> SummaryFoot { memory_total(self.roster).foot() }

    fn demands(&self, widths: &[(TileContent, u16)]) -> TileDemands {
        tile_demands(self.roster, widths, self.hidden_when_idle, self.tree)
    }

    fn rows_drawn(&self, content: &TileContent, area: Rect) -> u16 {
        self.row_use(content, area).drawn
    }

    fn rows_kept(&self, content: &TileContent, area: Rect) -> u16 {
        self.row_use(content, area).kept
    }

    fn draw(&self, buffer: &mut Buffer, content: &TileContent, inner: Rect, ground: Color) {
        draw_contents(
            buffer,
            self.roster,
            content,
            inner,
            ground,
            self.hidden_when_idle,
            self.tree,
        );
    }

    fn summary_labels(&self, rect: Rect) -> Vec<PaneFrameLabel> {
        sccache_label(self.sccache, rect)
    }
}

/// One content-row answer cached during a draw.
struct MeasuredCellRowUse {
    /// Cell whose content supplied the answer.
    content:  TileContent,
    /// Allocation at which the answer was measured.
    area:     Rect,
    /// Painted and retained rows at that allocation.
    use_rows: GroupRowUse,
}

/// Rows one cell paints inside the area above its readout.
#[cfg(test)]
fn content_rows_drawn(
    roster: &Roster,
    content: &TileContent,
    area: Rect,
    hidden_when_idle: &[String],
    tree: ProcessTree,
) -> u16 {
    content_row_use(roster, content, area, hidden_when_idle, tree).drawn
}

/// Painted and retained rows for one cell allocation.
fn content_row_use(
    roster: &Roster,
    content: &TileContent,
    area: Rect,
    hidden_when_idle: &[String],
    tree: ProcessTree,
) -> GroupRowUse {
    match content {
        TileContent::Summary => {
            let rows = summary_rows(roster, hidden_when_idle);
            let rows: Vec<&TrackedRow> = rows.iter().map(AsRef::as_ref).collect();
            let drawn = process_table_rows_drawn(
                &rows,
                TableKind::Summary,
                area,
                PinnedGroup::Unpinned,
                tree,
            );
            GroupRowUse { drawn, kept: drawn }
        },
        TileContent::Group(id) => group_row_use(roster, id, area, hidden_when_idle, tree),
        TileContent::Empty(_) => GroupRowUse { drawn: 0, kept: 0 },
    }
}

/// Rows one cell retains before lending to a column mate.
#[cfg(test)]
fn content_rows_kept(
    roster: &Roster,
    content: &TileContent,
    area: Rect,
    hidden_when_idle: &[String],
    tree: ProcessTree,
) -> u16 {
    content_row_use(roster, content, area, hidden_when_idle, tree).kept
}

/// Painted and retained rows across one command cell's two sections.
#[derive(Clone, Copy)]
struct GroupRowUse {
    /// Rows the cell paints at this allocation.
    drawn: u16,
    /// Rows it retains before lending to a mate.
    kept:  u16,
}

impl GroupRowUse {
    /// Combine one ancestry allocation with the table below it.
    const fn with_ancestry(ancestry: &AncestryRowUse, table_rows: u16) -> Self {
        Self {
            drawn: ancestry.drawn.saturating_add(table_rows),
            kept:  ancestry.required.saturating_add(table_rows),
        }
    }
}

/// How one command cell uses its ancestry and process-table allocation.
fn group_row_use(
    roster: &Roster,
    id: &InvocationId,
    area: Rect,
    hidden_when_idle: &[String],
    tree: ProcessTree,
) -> GroupRowUse {
    let Some(group) = roster.groups().iter().find(|group| &group.id == id) else {
        return GroupRowUse { drawn: 0, kept: 0 };
    };
    let leads_as_ancestor = group.leads_as_ancestor(hidden_when_idle);
    let rows: Vec<&TrackedRow> = group.rows().skip(usize::from(leads_as_ancestor)).collect();
    let ancestry = drawn_ancestry(group, leads_as_ancestor, tree);
    let table_demand = table_height(
        &rows,
        TableKind::Command,
        area.width,
        PinnedGroup::Lead(GroupingIdentity::from(&group.lead.process)),
        tree,
    );
    let ancestry_rows = ancestry_row_use(&ancestry, area, table_demand);
    let table_area = Rect {
        y: area.y.saturating_add(ancestry_rows.drawn),
        height: area.height.saturating_sub(ancestry_rows.drawn),
        ..area
    };
    let table_rows = process_table_rows_drawn(
        &rows,
        TableKind::Command,
        table_area,
        PinnedGroup::Lead(GroupingIdentity::from(&group.lead.process)),
        tree,
    );
    GroupRowUse::with_ancestry(&ancestry_rows, table_rows)
}

/// What every cell is asking for, each measured at the width it will be
/// drawn at.
///
/// The demand is counted here rather than in [`crate::roster`] because a
/// command line wraps, and how many lines it wraps to is something only
/// the table layout knows. The roster still says which groups get cells;
/// this says how tall each of them wants to be. The readout row under
/// each is not counted here: [`tui_pane::draw_tile_grid`] adds it.
///
/// A group that has no cell yet -- a command first seen on this scan --
/// is measured at the narrowest cell on screen, which is the width it
/// will have once the grid has opened one for it.
fn tile_demands(
    roster: &Roster,
    widths: &[(TileContent, u16)],
    hidden_when_idle: &[String],
    tree: ProcessTree,
) -> TileDemands {
    let narrowest = widths
        .iter()
        .map(|&(_, width)| width)
        .min()
        .unwrap_or_default();
    let width_of = |wanted: TileContent| {
        widths
            .iter()
            .find(|(content, _)| *content == wanted)
            .map_or(narrowest, |&(_, width)| width)
    };
    let summary = summary_rows(roster, hidden_when_idle);
    let summary: Vec<&TrackedRow> = summary.iter().map(AsRef::as_ref).collect();
    TileDemands {
        summary:       table_height(
            &summary,
            TableKind::Summary,
            width_of(TileContent::Summary),
            PinnedGroup::Unpinned,
            tree,
        ),
        summary_width: summary_width(&summary, tree),
        groups:        roster
            .tiled_ids(hidden_when_idle)
            .into_iter()
            .filter_map(|id| roster.groups().iter().find(|group| group.id == id))
            .map(|group| {
                let width = width_of(TileContent::Group(group.id.clone()));
                TileDemand {
                    id:   group.id.clone(),
                    rows: group_height(group, width, hidden_when_idle, tree),
                }
            })
            .collect(),
    }
}

/// Columns the summary's widest line takes given all the room it wants:
/// a directory heading, or a row with its command line unwrapped.
///
/// What the summary asks the grid for across, read only under
/// `tiles.widen_summary`.
fn summary_width(rows: &[&TrackedRow], tree: ProcessTree) -> u16 {
    if rows.is_empty() {
        return cell_width(SECTION_HEADER_INDENT).saturating_add(cell_width(NO_PROCESSES_NOTE));
    }
    let kind = TableKind::Summary;
    let columns = visible_columns(rows, kind);
    let command = longest_command_width(rows, kind.detail(), tree)
        .max(cell_width(TABLE_HEADERS[COMMAND_COLUMN]));
    let gaps = u16::try_from(columns.len().saturating_sub(1)).unwrap_or(u16::MAX);
    let table = fitted_constraints(rows, &columns)
        .iter()
        .zip(&columns)
        .map(|(constraint, &column)| match constraint {
            _ if column == COMMAND_COLUMN => command,
            Constraint::Length(width) | Constraint::Min(width) => *width,
            _ => 0,
        })
        .fold(
            cell_width(SECTION_ITEM_INDENT)
                .saturating_add(TABLE_COLUMN_SPACING.saturating_mul(gaps)),
            u16::saturating_add,
        );
    let headings = group_by_path(rows, PinnedGroup::Unpinned)
        .iter()
        .map(|group| cell_width(SECTION_HEADER_INDENT).saturating_add(cell_width(&group.heading())))
        .max()
        .unwrap_or_default();
    table.max(headings)
}

/// Rows one command's cell lays out with all the room it could want: the
/// ancestry block and the blank row under it, then the table.
///
/// The whole chain counts, not the levels a cell of some particular
/// height has room for -- the ask is what the cell wants, and what it is
/// given is the answer. A lead drawn as the foot of its own chain is
/// left out of the table, the same as [`draw_group`] leaves it out.
fn group_height(
    group: &TrackedGroup,
    width: u16,
    hidden_when_idle: &[String],
    tree: ProcessTree,
) -> usize {
    let leads_as_ancestor = group.leads_as_ancestor(hidden_when_idle);
    let rows: Vec<&TrackedRow> = group.rows().skip(usize::from(leads_as_ancestor)).collect();
    // The same steps [`draw_group`] draws, measured the same way: a
    // command wrapping down the cell wants the rows it wraps onto, and
    // asking for one row a level would leave the block short of what it
    // would have drawn with the room.
    let ancestry = drawn_ancestry(group, leads_as_ancestor, tree);
    let table = table_height(
        &rows,
        TableKind::Command,
        width,
        PinnedGroup::Lead(GroupingIdentity::from(&group.lead.process)),
        tree,
    );
    ancestry_demand(&ancestry, width).saturating_add(table)
}

/// Rows the block above the table asks for: the whole chain, plus the
/// blank row under it.
///
/// The ask is exactly what [`draw_ancestry`] draws when the cell is
/// given it, because [`ancestry_allocation`] hands the block whatever the
/// table does not need. Elision is then the answer to a cell that got
/// less than it asked for, never to a cell that got what it asked for --
/// which is what keeps the count the readout writes out equal to the
/// rows on screen.
fn ancestry_demand(ancestry: &[Ancestor], width: u16) -> usize {
    if ancestry.is_empty() {
        return 0;
    }
    let whole: Vec<AncestryLevel<'_>> = ancestry.iter().map(AncestryLevel::Ancestor).collect();
    ancestry_height(&whole, width).saturating_add(usize::from(ANCESTRY_GAP_HEIGHT))
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
fn table_height(
    rows: &[&TrackedRow],
    kind: TableKind,
    width: u16,
    pinned: PinnedGroup<'_>,
    tree: ProcessTree,
) -> usize {
    if rows.is_empty() {
        return 0;
    }
    let area = Rect {
        x: 0,
        y: 0,
        width,
        height: TABLE_HEADER_HEIGHT,
    };
    let layout = TableLayout::of(rows, kind, area, pane_background(false), tree);
    if layout.columns.is_empty() {
        return usize::from(TABLE_HEADER_HEIGHT);
    }
    let groups = group_by_path(rows, pinned);
    grouped_table_height(&groups, &layout)
}

/// Rows occupied by grouped table content under one solved layout.
fn grouped_table_height(groups: &[PathGroup<'_>], layout: &TableLayout) -> usize {
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
                .map(|row| usize::from(process_row(row, layout).lines))
                .sum();
            usize::from(GROUP_HEADER_HEIGHT).saturating_add(lines)
        })
        .sum();
    usize::from(TABLE_HEADER_HEIGHT)
        .saturating_add(laid_out)
        .saturating_add(gaps)
}

/// Render fixed cell geometry through the production content/readout sequence.
#[cfg(test)]
pub(crate) fn draw_cell_for_test(
    buffer: &mut Buffer,
    roster: &Roster,
    content: &TileContent,
    inner: Rect,
    content_rows: usize,
) {
    tui_pane::draw_tile_cell(
        buffer,
        content,
        inner,
        content_rows,
        inner.width,
        |buffer, inner| {
            draw_contents(
                buffer,
                roster,
                content,
                inner,
                pane_background(false),
                &[],
                ProcessTree::Long,
            );
        },
    );
}

/// What a cell holds inside its borders. `ground` is the colour the
/// cell is painted on, which a finished row's text fades toward, and
/// `hidden_when_idle` is what settles whether a command's own cell
/// draws it as a row or as the last step of its chain. `tree` says how
/// much of what stands above the command goes over the table; the
/// summary cell has no such block, so it never reads it. An empty cell
/// holds only its number, which [`tui_pane::draw_tile_cell`] draws.
fn draw_contents(
    buffer: &mut Buffer,
    roster: &Roster,
    content: &TileContent,
    inner: Rect,
    ground: Color,
    hidden_when_idle: &[String],
    tree: ProcessTree,
) {
    match content {
        TileContent::Summary => draw_summary(buffer, roster, inner, ground, hidden_when_idle, tree),
        TileContent::Group(id) => {
            draw_group(buffer, roster, id, inner, ground, hidden_when_idle, tree);
        },
        TileContent::Empty(_) => {},
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
    tree: ProcessTree,
) {
    let rows = summary_rows(roster, hidden_when_idle);
    draw_process_table(
        buffer,
        inner,
        &rows.iter().map(AsRef::as_ref).collect::<Vec<_>>(),
        TableKind::Summary,
        ground,
        PinnedGroup::Unpinned,
        tree,
    );
}

/// Every row the summary draws: one per command, and for an active
/// driver the commands it is driving instead.
///
/// A driver `commands.hidden_when_idle` names gives up its own row while
/// it has work beneath it, the same as [`TrackedGroup::leads_as_ancestor`]
/// gives it up in the driver's cell. The invocations it drives say where
/// the work is, from the directories they are building in. With no such
/// invocation, its own row remains so the summary still reports that the
/// driver is running.
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
///
/// Promoted copies read the worker's subtree total while the roster retains
/// the command table's individual invocation measurements and row display state.
fn summary_rows<'a>(roster: &'a Roster, hidden_when_idle: &[String]) -> Vec<Cow<'a, TrackedRow>> {
    let mut rows = Vec::new();
    for group in roster.groups() {
        if group.leads_as_ancestor(hidden_when_idle) {
            let before = rows.len();
            rows.extend(
                group
                    .rows()
                    .skip(1)
                    .filter(|row| !row.process.nested)
                    .map(|row| {
                        let mut promoted = row.clone();
                        promoted.process.cpu.clone_from(&row.process.subtree_cpu);
                        promoted.process.memory = row.process.subtree_memory;
                        Cow::Owned(promoted)
                    }),
            );
            if rows.len() == before {
                rows.push(Cow::Borrowed(&group.lead));
            }
        } else {
            rows.push(Cow::Borrowed(&group.lead));
        }
    }
    rows
}

/// The summary's memory total: every running command's group total.
fn memory_total(roster: &Roster) -> SummaryMemoryTotal {
    let mut running_groups = 0usize;
    let mut unreadable_groups = 0usize;
    let mut bytes = 0u64;
    for group in roster
        .groups()
        .iter()
        .filter(|group| !group.lead.is_ended())
    {
        running_groups = running_groups.saturating_add(1);
        match group.lead.process.memory {
            Measurement::Reading(reading) => bytes = bytes.saturating_add(reading),
            Measurement::Unavailable(_) => {
                unreadable_groups = unreadable_groups.saturating_add(1);
            },
        }
    }
    match (running_groups, unreadable_groups) {
        (0, _) => SummaryMemoryTotal::NoRunningCommands,
        (running, unreadable) if running == unreadable => SummaryMemoryTotal::NoReadableCommands,
        (_, 0) => SummaryMemoryTotal::Complete(bytes),
        _ => SummaryMemoryTotal::AtLeast(bytes),
    }
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
    id: &InvocationId,
    inner: Rect,
    ground: Color,
    hidden_when_idle: &[String],
    tree: ProcessTree,
) {
    let Some(group) = roster.groups().iter().find(|group| &group.id == id) else {
        return;
    };
    let leads_as_ancestor = group.leads_as_ancestor(hidden_when_idle);
    let rows: Vec<&TrackedRow> = group.rows().skip(usize::from(leads_as_ancestor)).collect();
    let ancestry = drawn_ancestry(group, leads_as_ancestor, tree);
    // Where the driver stands at the foot of its own chain, that is
    // where its pid is written, and the rows pointing at it in the
    // table below need it in the colour they are pointing with.
    let foot = if leads_as_ancestor {
        match group.lead.family() {
            FamilyHead::Heads(family) => AncestryFoot::ColoredCommand(theme::family_color(family)),
            FamilyHead::NoChildren => AncestryFoot::PlainCommand,
        }
    } else {
        AncestryFoot::Other
    };
    // The lead's own fade goes into the block whether or not it is a
    // row there: the chain stands over the whole cell, and the cell
    // goes out when the command does.
    let faded = heading_fade(&rows).min(group.lead.faded());
    // The same measurement [`group_height`] made when the cell
    // asked for its room, so the block is given back exactly what the
    // ask left it.
    let table_rows = table_height(
        &rows,
        TableKind::Command,
        inner.width,
        PinnedGroup::Lead(GroupingIdentity::from(&group.lead.process)),
        tree,
    );
    let used = draw_ancestry(buffer, inner, &ancestry, faded, ground, foot, table_rows);
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
        PinnedGroup::Lead(GroupingIdentity::from(&group.lead.process)),
        tree,
    );
}

/// The steps `group`'s cell puts above its table: its chain, closed by
/// the command itself where the command is a driver, with the steps
/// that only passed something through taken out.
///
/// Read here rather than off [`crate::roster::TrackedGroup::ancestry`]
/// directly so the cell's height is asked for against the steps it will
/// actually draw. Counting the raw chain asked for rows that
/// [`carried`] then dropped.
///
/// A [`ProcessTree::Short`] cell draws the same steps: what the setting
/// takes off is the arguments on each of them, which is where the block
/// spends most of its width and every row it wraps onto.
fn drawn_ancestry(
    group: &TrackedGroup,
    leads_as_ancestor: bool,
    tree: ProcessTree,
) -> Vec<Ancestor> {
    let mut chain = group.ancestry().to_vec();
    if leads_as_ancestor {
        chain.push(as_ancestor(&group.lead.process));
    }
    let chain = carried(chain);
    match tree {
        ProcessTree::Long => chain,
        // The block is there to say where the command came from, and
        // the name of each step says that on its own. What the
        // arguments add is the run's own detail, which the table below
        // already carries a row of.
        ProcessTree::Short => chain.iter().map(shortened).collect(),
    }
}

/// One step of a chain with its omitted arguments marked.
fn shortened(ancestor: &Ancestor) -> Ancestor {
    let name = census::command_name(&ancestor.command);
    let command = if name == ancestor.command {
        name
    } else {
        format!("{name} {ELISION}")
    };
    Ancestor {
        pid: ancestor.pid,
        command,
        passes_through: ancestor.passes_through,
    }
}

/// A command as the last step of its own cell's chain: its pid, and the
/// whole line it was typed as.
fn as_ancestor(process: &CargoProcess) -> Ancestor {
    let arguments = process.command.line(SummaryDetail::Full);
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
///
/// A pid here is drawn the way a pid in the table below is: the foot of
/// the chain is the very number the first row's `parent` cell names, so
/// the two are read as one thing or not at all. Where that foot heads a
/// family, `foot` carries the colour its `parent` cells point with; the
/// rest of the chain heads nothing, and takes the same plain text
/// [`pid_style`] leaves an unfamilied pid in.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AncestryFoot {
    /// The lead command closes its own chain with a family colour.
    ColoredCommand(Color),
    /// The lead command closes its own chain without a family colour.
    PlainCommand,
    /// A shell or other ordinary ancestor closes the chain.
    Other,
}

fn draw_ancestry(
    buffer: &mut Buffer,
    area: Rect,
    ancestry: &[Ancestor],
    faded: u8,
    ground: Color,
    foot: AncestryFoot,
    table: usize,
) -> u16 {
    let allocation = ancestry_allocation(area.height, table);
    let budget = allocation.content_rows;
    let levels = ancestry_fit(ancestry, budget, |levels| {
        ancestry_height(levels, area.width)
    });
    if levels.is_empty() {
        return 0;
    }
    let pid = blend_color(text_default(), ground, faded);
    let command = blend_color(secondary_text_color(), ground, faded);
    let last = levels.len().saturating_sub(1);
    let mut lines: Vec<Line<'static>> = levels
        .iter()
        .enumerate()
        .flat_map(|(level, ancestor)| {
            let ink = match foot {
                AncestryFoot::ColoredCommand(color) if level == last => {
                    blend_color(color, ground, faded)
                },
                AncestryFoot::ColoredCommand(_)
                | AncestryFoot::PlainCommand
                | AncestryFoot::Other => pid,
            };
            ancestry_lines(*ancestor, level, area.width, ink, command)
        })
        .collect();
    // Only ever reached by the single level [`ancestry_fit`] stops at,
    // whose command outruns the whole budget on its own. The lines run
    // head first, so what goes is the tail of that command rather than
    // the pid that names it.
    let kept = budget.max(1);
    if lines.len() > kept {
        lines.truncate(kept);
        if let Some(line) = lines.last_mut() {
            mark_cut_line(line, area.width);
        }
    }
    // `u16` because the count came out of a budget measured in rows of
    // this same area.
    let height = u16::try_from(lines.len()).unwrap_or(u16::MAX);
    Paragraph::new(lines).render(Rect { height, ..area }, buffer);
    height.saturating_add(allocation.gap_rows)
}

/// End a line where its remaining content was cut.
fn mark_cut_line(line: &mut Line<'static>, width: u16) {
    let room = usize::from(width).saturating_sub(UnicodeWidthStr::width(ELISION));
    let marker_style = line
        .spans
        .iter()
        .rev()
        .find(|span| !span.content.is_empty())
        .map_or_else(Style::default, |span| span.style);
    let mut kept = Vec::new();
    let mut used: usize = 0;
    for span in std::mem::take(&mut line.spans) {
        let span_width = UnicodeWidthStr::width(span.content.as_ref());
        if used.saturating_add(span_width) <= room {
            used = used.saturating_add(span_width);
            kept.push(span);
            continue;
        }
        let content: String = span
            .content
            .chars()
            .take_while(|character| {
                let character_width = UnicodeWidthChar::width(*character).unwrap_or_default();
                let fits = used.saturating_add(character_width) <= room;
                if fits {
                    used = used.saturating_add(character_width);
                }
                fits
            })
            .collect();
        if !content.is_empty() {
            kept.push(Span::styled(content, span.style));
        }
        break;
    }
    line.spans = kept;
    if UnicodeWidthStr::width(ELISION) <= usize::from(width) {
        line.spans.push(Span::styled(ELISION, marker_style));
    }
}

/// Apply the shared cut mark without making a whole command look shortened.
fn mark_table_continuation(
    buffer: &mut Buffer,
    area: Rect,
    layout: &TableLayout,
    continuation: TableContinuation,
) {
    let mut command = layout.command_area(area);
    command.y = command.y.saturating_add(TABLE_HEADER_HEIGHT);
    command.height = command.height.saturating_sub(TABLE_HEADER_HEIGHT);
    let Some(y) = (command.top()..command.bottom())
        .rev()
        .find(|&y| (command.left()..command.right()).any(|x| buffer[(x, y)].symbol() != " "))
    else {
        return;
    };
    let Some(right) = (command.left()..command.right())
        .rev()
        .find(|&x| buffer[(x, y)].symbol() != " ")
    else {
        return;
    };
    if matches!(continuation, TableContinuation::RowsWereOmitted) {
        if right < command.right() - 1 {
            let style = buffer[(right, y)].style();
            buffer[(command.right() - 1, y)]
                .set_symbol(ELISION)
                .set_style(style);
            return;
        }
        if move_last_word_to_table_continuation_line(buffer, command, y, right) {
            return;
        }
    }
    let mut spans = Vec::new();
    let mut x = command.left();
    while x <= right {
        let cell = &buffer[(x, y)];
        let symbol = cell.symbol();
        spans.push(Span::styled(symbol.to_owned(), cell.style()));
        let cells = UnicodeWidthStr::width(symbol).max(1);
        x = x.saturating_add(u16::try_from(cells).unwrap_or(u16::MAX));
    }
    let mut line = Line::from(spans);
    if let TableContinuation::CommandWasCut { continuation } = continuation {
        continuation.append_to(&mut line, command.width);
    }
    mark_cut_line(&mut line, command.width);
    Paragraph::new(line).render(
        Rect {
            y,
            height: 1,
            ..command
        },
        buffer,
    );
}

/// Use a blank line below an exact-width command to keep its final word whole.
fn move_last_word_to_table_continuation_line(
    buffer: &mut Buffer,
    command: Rect,
    y: u16,
    right: u16,
) -> bool {
    let next_y = y.saturating_add(1);
    if next_y >= command.bottom()
        || (command.left()..command.right()).any(|x| buffer[(x, next_y)].symbol() != " ")
    {
        return false;
    }
    let Some(separator) = (command.left()..right).rev().find(|&x| {
        let symbol = buffer[(x, y)].symbol();
        !symbol.is_empty() && symbol.chars().all(char::is_whitespace)
    }) else {
        return false;
    };
    let word_start = separator.saturating_add(1);
    let mut word = Vec::new();
    let mut word_width: usize = 0;
    let mut x = word_start;
    while x <= right {
        let cell = &buffer[(x, y)];
        let symbol = cell.symbol();
        let cells = UnicodeWidthStr::width(symbol).max(1);
        word_width = word_width.saturating_add(cells);
        word.push(Span::styled(symbol.to_owned(), cell.style()));
        x = x.saturating_add(u16::try_from(cells).unwrap_or(u16::MAX));
    }
    if word_width.saturating_add(UnicodeWidthStr::width(ELISION)) > usize::from(command.width) {
        return false;
    }

    let marker_style = buffer[(right, y)].style();
    for x in separator..command.right() {
        buffer[(x, y)].set_symbol(" ");
    }
    Paragraph::new(Line::from(word)).render(
        Rect {
            y: next_y,
            height: 1,
            ..command
        },
        buffer,
    );
    buffer[(command.right() - 1, next_y)]
        .set_symbol(ELISION)
        .set_style(marker_style);
    true
}

/// Styled command text immediately below a cell's last visible table line.
struct CutCommandContinuation {
    /// How this line follows the last visible one in the source command.
    join:      CommandContinuationJoin,
    /// The next wrapped line, whose beginning can fill the visible line's room.
    next_line: Line<'static>,
}

impl CutCommandContinuation {
    /// Continue `visible` across its unused cells, retaining the command's styles.
    fn append_to(self, visible: &mut Line<'static>, width: u16) {
        if UnicodeWidthStr::width(visible) < usize::from(width)
            && matches!(self.join, CommandContinuationJoin::WordSeparator)
            && !visible.spans.is_empty()
            && !self.next_line.spans.is_empty()
        {
            let style = self.next_line.spans[0].style;
            visible.spans.push(Span::styled(" ", style));
        }
        visible.spans.extend(self.next_line.spans);
    }
}

/// What stood between adjacent wrapped command lines before wrapping.
#[derive(Clone, Copy)]
enum CommandContinuationJoin {
    /// A punctuation boundary split one word, so the pieces remain adjacent.
    Direct,
    /// Whitespace split two words, represented by one space when the lines rejoin.
    WordSeparator,
}

/// A drawn ancestor or the marker for a deliberately omitted chain segment.
#[derive(Clone, Copy)]
enum AncestryLevel<'a> {
    /// This ancestor is retained in the visible chain.
    Ancestor(&'a Ancestor),
    /// The layout omits one or more intervening ancestors.
    Elided,
    /// Earlier ancestors are omitted, with the cut marked on this row.
    AncestorAfterElision(&'a Ancestor),
}

/// Which levels of `ancestry` the block draws once wrapping is counted.
///
/// [`ancestry_levels`] answers in levels, one row apiece, which is what
/// the budget used to buy. A level whose command wraps costs more than
/// the row it was budgeted, so the block asks for one fewer level and
/// measures again. Giving a level up beats trimming the drawn lines,
/// which would cut the foot of the chain -- the very pid the table's
/// `parent` column points at.
///
/// It stops at one level rather than at none. A single command wide
/// enough to outrun the whole budget on its own would otherwise take
/// the block away entirely, and a pid with the head of its command is
/// worth more than a blank half-cell. What that one level overruns by
/// is what [`draw_ancestry`] cuts off the bottom.
///
/// `height` is what a candidate block is measured with --
/// [`ancestry_height`] at the cell's width -- and every call to it wraps
/// each level's command line again.
fn ancestry_fit(
    ancestry: &[Ancestor],
    budget: usize,
    mut height: impl FnMut(&[AncestryLevel<'_>]) -> usize,
) -> Vec<AncestryLevel<'_>> {
    // A budget past the chain's length asks for the whole chain however
    // far past it is, so the first ask is the chain. Starting from the
    // budget measured that same whole chain once per spare row, and a
    // command line of tens of kilobytes in a cell hundreds of rows tall
    // made every frame cost seconds.
    let mut asked = budget.min(ancestry.len());
    loop {
        let levels = if budget >= ANCESTRY_MIN_ELIDED_ROWS
            && asked == ANCESTRY_MIN_ELIDED_ROWS.saturating_sub(1)
            && ancestry.len() > asked
        {
            ancestry
                .first()
                .map(AncestryLevel::Ancestor)
                .into_iter()
                .chain(std::iter::once(AncestryLevel::Elided))
                .chain(ancestry.last().map(AncestryLevel::Ancestor))
                .collect()
        } else {
            ancestry_levels(ancestry, asked)
        };
        if levels.len() <= 1 || height(&levels) <= budget {
            return levels;
        }
        asked = asked.saturating_sub(1);
    }
}

/// Rows `levels` take at `width`, every level's wrapping counted.
fn ancestry_height(levels: &[AncestryLevel<'_>], width: u16) -> usize {
    levels
        .iter()
        .enumerate()
        .map(|(level, ancestor)| ancestry_rows(*ancestor, level, width))
        .sum()
}

/// Rows the ancestry block may take at a cell of `height` standing over
/// a table of `table` rows.
///
/// What the table needs comes off the top. Two or more remaining rows
/// reserve the last for the gap below the block; one remaining row draws
/// ancestry text without a gap. A cell granted its [`ancestry_demand`]
/// has exactly the whole chain left over, so it draws the chain whole; a
/// cell given less elides by the shortfall.
#[cfg(test)]
fn ancestry_budget(height: u16, table: usize) -> usize {
    ancestry_allocation(height, table).content_rows
}

/// Rows available to ancestry content and to its separation from the table.
struct AncestryAllocation {
    /// Rows in which ancestry text may be drawn.
    content_rows: usize,
    /// Blank rows retained below ancestry text.
    gap_rows:     u16,
}

/// Give a lone row to ancestry text, adding its lower gap when room remains.
fn ancestry_allocation(height: u16, table: usize) -> AncestryAllocation {
    let available = usize::from(height).saturating_sub(table);
    let full_gap = usize::from(ANCESTRY_GAP_HEIGHT);
    let gap_rows = if available > full_gap {
        ANCESTRY_GAP_HEIGHT
    } else {
        0
    };
    AncestryAllocation {
        content_rows: available.saturating_sub(usize::from(gap_rows)),
        gap_rows,
    }
}

/// Rows a fitted ancestry block draws and requires, including its lower gap.
///
/// One ancestry row beyond a nonempty table is optional: the grid may
/// give it to a mate that completes a content step. If no mate can use
/// it, the allocation stays here and [`draw_ancestry`] draws the line.
struct AncestryRowUse {
    /// Rows the block draws when it keeps this allocation.
    drawn:    u16,
    /// Rows the grid must retain before considering a transfer.
    required: u16,
}

fn ancestry_row_use(ancestry: &[Ancestor], area: Rect, table: usize) -> AncestryRowUse {
    let allocation = ancestry_allocation(area.height, table);
    let budget = allocation.content_rows;
    let levels = ancestry_fit(ancestry, budget, |levels| {
        ancestry_height(levels, area.width)
    });
    if levels.is_empty() {
        return AncestryRowUse {
            drawn:    0,
            required: 0,
        };
    }
    let rows = ancestry_height(&levels, area.width).min(budget.max(1));
    let drawn = u16::try_from(rows)
        .unwrap_or(u16::MAX)
        .saturating_add(allocation.gap_rows);
    let required = if table > 0 && allocation.content_rows == 1 && allocation.gap_rows == 0 {
        0
    } else {
        drawn
    };
    AncestryRowUse { drawn, required }
}

/// Which ancestors a block of `budget` rows carries and which segment it elides.
///
/// A chain that fits is drawn whole. One that does not keeps both ends:
/// the top-level parent, and the levels nearest the command, which are
/// what say how it was actually started. Below
/// [`ANCESTRY_MIN_ELIDED_ROWS`] there is no room for both ends and a
/// separate elision between them. Two rows therefore keep the elision
/// and the nearest ancestor. One row combines those two, so even the
/// smallest visible fragment says both that the chain was cut and what
/// stood nearest the command.
fn ancestry_levels(ancestry: &[Ancestor], budget: usize) -> Vec<AncestryLevel<'_>> {
    if budget == 0 {
        return Vec::new();
    }
    if ancestry.len() <= budget {
        return ancestry.iter().map(AncestryLevel::Ancestor).collect();
    }
    if budget < ANCESTRY_MIN_ELIDED_ROWS {
        let Some(nearest) = ancestry.last() else {
            return Vec::new();
        };
        if budget == 1 {
            return vec![AncestryLevel::AncestorAfterElision(nearest)];
        }
        return vec![AncestryLevel::Elided, AncestryLevel::Ancestor(nearest)];
    }
    let tail = budget - 2;
    ancestry
        .first()
        .map(AncestryLevel::Ancestor)
        .into_iter()
        .chain(std::iter::once(AncestryLevel::Elided))
        .chain(
            ancestry[ancestry.len() - tail..]
                .iter()
                .map(AncestryLevel::Ancestor),
        )
        .collect()
}

/// What stands before a level's command: the indent its depth earns it
/// and the pid, with the space that parts the two.
///
/// The cells this occupies are where the command starts, and where
/// every line the command wraps onto is set to -- under the command
/// rather than under the pid, so a wrapped line reads as more of the
/// same command instead of as another step of the chain.
fn ancestry_stem(ancestor: &Ancestor, level: usize, width: u16) -> (String, String) {
    let indent = format!(
        "{SECTION_HEADER_INDENT}{}",
        ANCESTRY_LEVEL_INDENT.repeat(level)
    );
    let label = ancestor.pid.to_string();
    if cell_width(&indent).saturating_add(cell_width(&label)) <= width {
        (indent, label)
    } else {
        (indent, String::new())
    }
}

/// Cells `level`'s command has to itself at `width`.
fn ancestry_room(ancestor: &Ancestor, level: usize, width: u16) -> u16 {
    let (indent, label) = ancestry_stem(ancestor, level, width);
    width
        .saturating_sub(cell_width(&indent))
        .saturating_sub(cell_width(&label))
        .saturating_sub(u16::from(!label.is_empty()))
}

/// Cells a nearest ancestor has after an inline mark for an omitted head.
fn ancestry_room_after_elision(ancestor: &Ancestor, width: u16) -> u16 {
    let indent = format!("{SECTION_HEADER_INDENT}{ELISION} ");
    let label = ancestor.pid.to_string();
    let label = if cell_width(&indent).saturating_add(cell_width(&label)) <= width {
        label
    } else {
        String::new()
    };
    width
        .saturating_sub(cell_width(&indent))
        .saturating_sub(cell_width(&label))
        .saturating_sub(u16::from(!label.is_empty()))
}

/// Rows `ancestor` takes at `width` once its command has wrapped: one
/// for the pid and the head of the command, and one more for every line
/// the command carried on to.
///
/// An elided level is a single character and never wraps.
fn ancestry_rows(ancestor: AncestryLevel<'_>, level: usize, width: u16) -> usize {
    let follows_elision = matches!(ancestor, AncestryLevel::AncestorAfterElision(_));
    let ancestor = match ancestor {
        AncestryLevel::Ancestor(ancestor) | AncestryLevel::AncestorAfterElision(ancestor) => {
            ancestor
        },
        AncestryLevel::Elided => return 1,
    };
    let room = if follows_elision {
        ancestry_room_after_elision(ancestor, width)
    } else {
        ancestry_room(ancestor, level, width)
    };
    if room < ANCESTRY_MIN_COMMAND_WIDTH {
        return 1;
    }
    tui_pane::wrapped(vec![Span::raw(ancestor.command.clone())], room)
        .height()
        .max(1)
}

/// One level of the ancestry block: its pid and what the process is,
/// set one space further in than the level above it.
///
/// A command too long for the cell wraps rather than being cut at the
/// edge, breaking at whitespace through [`tui_pane::wrapped`] -- the same
/// break a command's own row in the table below takes -- and falling
/// back to breaking mid-word only where no whitespace will do. Every
/// line after the first is set to the column the command started at, so
/// the block still reads as one step per pid.
fn ancestry_lines(
    ancestor: AncestryLevel<'_>,
    level: usize,
    width: u16,
    pid: Color,
    command: Color,
) -> Vec<Line<'static>> {
    let (ancestor, indent) = match ancestor {
        AncestryLevel::Ancestor(ancestor) => {
            let (indent, _) = ancestry_stem(ancestor, level, width);
            (ancestor, indent)
        },
        AncestryLevel::AncestorAfterElision(ancestor) => {
            (ancestor, format!("{SECTION_HEADER_INDENT}{ELISION} "))
        },
        AncestryLevel::Elided => {
            let indent = format!(
                "{SECTION_HEADER_INDENT}{}",
                ANCESTRY_LEVEL_INDENT.repeat(level)
            );
            return vec![Line::from(vec![
                Span::raw(indent),
                Span::styled(ELISION, Style::default().fg(pid)),
            ])];
        },
    };
    let label = ancestor.pid.to_string();
    let label = if cell_width(&indent).saturating_add(cell_width(&label)) <= width {
        label
    } else {
        String::new()
    };
    let room = width
        .saturating_sub(cell_width(&indent))
        .saturating_sub(cell_width(&label))
        .saturating_sub(u16::from(!label.is_empty()));
    if room < ANCESTRY_MIN_COMMAND_WIDTH {
        return vec![Line::from(vec![
            Span::raw(indent),
            Span::styled(label, Style::default().fg(pid)),
        ])];
    }
    let separator = if label.is_empty() { "" } else { " " };
    let hanging = " ".repeat(
        indent
            .chars()
            .count()
            .saturating_add(label.chars().count())
            .saturating_add(separator.len()),
    );
    let wrapped = tui_pane::wrapped(
        vec![Span::styled(
            ancestor.command.clone(),
            Style::default().fg(command),
        )],
        room,
    );
    wrapped
        .lines
        .into_iter()
        .enumerate()
        .map(|(carried_on, line)| {
            let stem = if carried_on == 0 {
                vec![
                    Span::raw(indent.clone()),
                    Span::styled(label.clone(), Style::default().fg(pid)),
                    Span::raw(separator),
                ]
            } else {
                vec![Span::raw(hanging.clone())]
            };
            Line::from([stem, line.spans].concat())
        })
        .collect()
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

    /// How much command detail a row of this kind prints. A summary row
    /// already sits under the working directory heading its group, so
    /// the manifest path says nothing new while costing the width the
    /// command line wants.
    const fn detail(self) -> SummaryDetail {
        match self {
            Self::Summary => SummaryDetail::Trimmed,
            Self::Command => SummaryDetail::Full,
        }
    }
}

/// Whether the command's own directory must head the cell.
#[derive(Clone, Copy)]
enum PinnedGroup<'a> {
    /// The lead supplies the complete directory qualification; its text is only a label.
    Lead(GroupingIdentity<'a>),
    /// The summary has no preferred directory.
    Unpinned,
}

/// Only numeric ownership and root incarnation determine capture grouping.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum GroupQualification {
    /// Neither a verified capture nor an observed process owner supplies an account.
    Unqualified,
    /// An uncaptured process names its owner's account without a capture root.
    Owner {
        /// A name lookup cannot change the account identity.
        uid: u32,
    },
    /// Direct and enclosing membership share the same directory qualification.
    Captured {
        /// A name lookup cannot change the account identity.
        uid:         u32,
        /// Root order alone cannot identify a replacement directory object.
        root:        CaptureRootIndex,
        /// Retained rows from an earlier root object remain separate.
        incarnation: RootIncarnation,
    },
}

impl GroupQualification {
    /// An owner row shares a heading with captured rows of its own uid; any other
    /// pair shares one only when equal, so roots and incarnations stay apart.
    fn is_compatible(self, other: Self) -> bool {
        match (self, other) {
            (Self::Owner { uid: owner }, Self::Captured { uid, .. })
            | (Self::Captured { uid, .. }, Self::Owner { uid: owner }) => owner == uid,
            _ => self == other,
        }
    }
}

impl From<&RowProvenance> for GroupQualification {
    fn from(provenance: &RowProvenance) -> Self {
        match provenance {
            RowProvenance::Uncaptured(ProcessOwner::Unavailable) => Self::Unqualified,
            RowProvenance::Uncaptured(ProcessOwner::Observed(account)) => {
                Self::Owner { uid: account.uid }
            },
            RowProvenance::Direct(context) | RowProvenance::Enclosing(context) => Self::Captured {
                uid:         context.account.uid,
                root:        context.root,
                incarnation: context.incarnation,
            },
        }
    }
}

/// An unobserved working directory cannot make unrelated invocations neighbors.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum GroupDirectory<'a> {
    /// Compare raw Unix path bytes independently from the heading text.
    Absolute(&'a Path),
    /// Pin an unavailable-directory row by its stable invocation identity.
    Unavailable(&'a InvocationId),
}

/// The same compatibility relation selects group members and the pinned heading.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct GroupingIdentity<'a> {
    /// Ownership remains independent from source and account-name resolution.
    qualification: GroupQualification,
    /// Keep the raw working directory through display shortening and source changes.
    directory:     GroupDirectory<'a>,
}

impl<'a> From<&'a CargoProcess> for GroupingIdentity<'a> {
    fn from(process: &'a CargoProcess) -> Self {
        Self {
            qualification: GroupQualification::from(&process.provenance),
            directory:     match &process.directory_identity {
                WorkingDirectoryIdentity::Absolute(path) => GroupDirectory::Absolute(path),
                WorkingDirectoryIdentity::Unavailable => {
                    GroupDirectory::Unavailable(&process.invocation_id)
                },
            },
        }
    }
}

impl GroupingIdentity<'_> {
    /// Equal raw directories whose qualifications do not name different accounts or roots.
    fn is_compatible(&self, other: &GroupingIdentity<'_>) -> bool {
        self.directory == other.directory && self.qualification.is_compatible(other.qualification)
    }
}

/// Invocations sharing an account, root incarnation and absolute working directory,
/// joined by uncaptured invocations whose process owner is that account.
struct PathGroup<'a> {
    /// Display text never participates in membership or pin selection. A captured
    /// member replaces an owner-only identity, so it names the group's root.
    identity: GroupingIdentity<'a>,
    /// The first member's display label preserves the existing heading choice.
    path:     &'a str,
    /// Every invocation running there, oldest known start first.
    rows:     Vec<&'a TrackedRow>,
}

impl PathGroup<'_> {
    /// Account qualifier for the heading, including its trailing separator.
    fn heading_prefix(&self) -> String {
        let uid = match self.identity.qualification {
            GroupQualification::Unqualified => return String::new(),
            GroupQualification::Owner { uid } | GroupQualification::Captured { uid, .. } => uid,
        };
        let account = self
            .rows
            .iter()
            .find_map(|row| match &row.process.provenance {
                RowProvenance::Uncaptured(ProcessOwner::Observed(account))
                | RowProvenance::Direct(CaptureContext { account, .. })
                | RowProvenance::Enclosing(CaptureContext { account, .. }) => match &account.name {
                    AccountName::Resolved(name) => Some(name.clone()),
                    AccountName::Unavailable => None,
                },
                RowProvenance::Uncaptured(ProcessOwner::Unavailable) => None,
            })
            .unwrap_or_else(|| uid.to_string());
        format!("{ACCOUNT_HEADING_OPEN}{account}{ACCOUNT_HEADING_CLOSE}")
    }

    /// Qualify the heading once; unresolved account names retain their numeric uid.
    /// Every member names the group's uid, and a name any member resolved labels
    /// it, so the text does not depend on which member sorts first.
    fn heading(&self) -> String { format!("{}{}", self.heading_prefix(), self.path) }

    /// The most informative whole or marked directory heading that fits `room`.
    fn fitted_heading(&self, room: u16) -> String {
        let whole = self.heading();
        if cell_width(&whole) <= room {
            return whole;
        }
        let qualified = self.heading_prefix();
        if !qualified.is_empty() {
            let qualified_components = self.component_heading(&qualified, room);
            if !qualified_components.is_empty() {
                return qualified_components;
            }
        }
        let components = self.component_heading("", room);
        if !components.is_empty() {
            return components;
        }
        if !qualified.is_empty() {
            let qualified_tail = self.tail_heading(&qualified, room);
            if !qualified_tail.is_empty() {
                return qualified_tail;
            }
        }
        let tail = self.tail_heading("", room);
        if !tail.is_empty() {
            return tail;
        }
        String::new()
    }

    /// A marked heading retaining the greatest number of whole trailing components.
    fn component_heading(&self, prefix: &str, room: u16) -> String {
        let components: Vec<&str> = self
            .path
            .rsplit(std::path::MAIN_SEPARATOR)
            .filter(|component| !component.is_empty())
            .collect();
        let retained = components.len().saturating_sub(usize::from(
            !self.path.starts_with(std::path::MAIN_SEPARATOR),
        ));
        for count in (1..=retained).rev() {
            let suffix = components[..count]
                .iter()
                .rev()
                .copied()
                .collect::<Vec<&str>>()
                .join(std::path::MAIN_SEPARATOR_STR);
            let candidate = format!("{prefix}{ELISION}{}{suffix}", std::path::MAIN_SEPARATOR);
            if cell_width(&candidate) <= room {
                return candidate;
            }
        }
        String::new()
    }

    /// A marked heading retaining the tail of the final component.
    fn tail_heading(&self, prefix: &str, room: u16) -> String {
        let component = self
            .path
            .rsplit(std::path::MAIN_SEPARATOR)
            .find(|component| !component.is_empty())
            .unwrap_or_default();
        let tail_room = room
            .saturating_sub(cell_width(prefix))
            .saturating_sub(cell_width(ELISION))
            .min(cell_width(component));
        if tail_room < HEADING_MIN_TAIL {
            return String::new();
        }
        let mut tail: Vec<char> = component
            .chars()
            .rev()
            .take(usize::from(tail_room))
            .collect();
        tail.reverse();
        format!("{prefix}{ELISION}{}", tail.into_iter().collect::<String>())
    }
}

/// How one cell's tables are laid out, settled once for the whole cell.
///
/// Every group in the cell is drawn against this, which is what keeps
/// the tables lined up down the cell instead of each one fitting itself
/// and the columns stepping in and out as the eye moves between them.
struct TableLayout {
    /// Column widths, in table order.
    constraints:    Vec<Constraint>,
    /// The columns this cell draws, in table order.
    columns:        Vec<usize>,
    /// Solved widths for `columns`, in the same order.
    column_widths:  Vec<u16>,
    /// Blank cells between adjacent columns.
    column_spacing: u16,
    /// How much of each row's command line the cell prints.
    detail:         SummaryDetail,
    /// Whether a row spells out its whole command line or only the
    /// name of what runs.
    tree:           ProcessTree,
    /// The colour the cell is painted on, which a finished row's text
    /// is carried toward as it fades.
    ground:         Color,
}

impl TableLayout {
    /// The layout for a cell of `kind` drawing `rows` into `area`, over
    /// a cell painted `ground`.
    fn of(
        rows: &[&TrackedRow],
        kind: TableKind,
        area: Rect,
        ground: Color,
        tree: ProcessTree,
    ) -> Self {
        let mut columns = visible_columns(rows, kind);
        let mut constraints = fitted_constraints(rows, &columns);
        let table_width = indented(area).width;
        for column in TABLE_COLUMN_DROP_ORDER {
            let Some(position) = columns.iter().position(|candidate| *candidate == column) else {
                continue;
            };
            if table_constraints_fit(table_width, &constraints) {
                break;
            }
            columns.remove(position);
            constraints.remove(position);
        }
        if !table_constraints_fit(table_width, &constraints) {
            remove_column(COMMAND_COLUMN, &mut columns, &mut constraints);
        }
        if !table_constraints_fit(table_width, &constraints) {
            remove_column(PID_COLUMN, &mut columns, &mut constraints);
        }
        let command_width = longest_command_width(rows, kind.detail(), tree);
        let column_spacing =
            table_column_spacing(table_width, &constraints, &columns, command_width);
        let column_widths = solved_column_widths(table_width, &constraints, column_spacing);
        Self {
            constraints,
            columns,
            column_widths,
            column_spacing,
            detail: kind.detail(),
            tree,
            ground,
        }
    }

    /// Solved cells for `column`, or none when the table left it out.
    fn column_width(&self, column: usize) -> u16 {
        self.columns
            .iter()
            .position(|candidate| *candidate == column)
            .map_or(0, |position| self.column_widths[position])
    }

    /// The command column inside one table area, or an empty rectangle
    /// where the table left that column out.
    fn command_area(&self, area: Rect) -> Rect {
        let table = indented(area);
        let Some(position) = self
            .columns
            .iter()
            .position(|candidate| *candidate == COMMAND_COLUMN)
        else {
            return Rect { width: 0, ..table };
        };
        let preceding_width = self.column_widths[..position]
            .iter()
            .copied()
            .fold(0, u16::saturating_add);
        let preceding_gaps = u16::try_from(position)
            .unwrap_or(u16::MAX)
            .saturating_mul(self.column_spacing);
        Rect {
            x: table
                .x
                .saturating_add(preceding_width)
                .saturating_add(preceding_gaps),
            width: self.column_widths[position],
            ..table
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
fn process_table_rows_drawn(
    rows: &[&TrackedRow],
    kind: TableKind,
    area: Rect,
    pinned: PinnedGroup<'_>,
    tree: ProcessTree,
) -> u16 {
    if rows.is_empty() {
        let empty_table_rows = TABLE_HEADER_HEIGHT.saturating_add(GROUP_HEADER_HEIGHT);
        return area.height.min(empty_table_rows);
    }
    let layout = TableLayout::of(rows, kind, area, pane_background(false), tree);
    if layout.columns.is_empty() {
        return area.height.min(TABLE_HEADER_HEIGHT);
    }
    let groups = group_by_path(rows, pinned);
    let drawn_groups: Vec<Vec<DrawnRow<'_>>> = groups
        .iter()
        .map(|group| {
            group
                .rows
                .iter()
                .map(|row| process_row(row, &layout))
                .collect()
        })
        .collect();
    let header_rows = area.height.min(TABLE_HEADER_HEIGHT);
    let available_height = area.height.saturating_sub(header_rows);
    let plan = process_table_plan(&drawn_groups, available_height);
    header_rows.saturating_add(plan.rows_drawn(&drawn_groups, available_height))
}

fn draw_process_table(
    buffer: &mut Buffer,
    area: Rect,
    rows: &[&TrackedRow],
    kind: TableKind,
    ground: Color,
    pinned: PinnedGroup<'_>,
    tree: ProcessTree,
) {
    if rows.is_empty() {
        let text = format!("{SECTION_HEADER_INDENT}{NO_PROCESSES_NOTE}");
        let text = match kind {
            TableKind::Summary => elide_summary_end(&text, area.width),
            TableKind::Command => text,
        };
        Paragraph::new(vec![
            Line::from(""),
            Line::from(Span::styled(text, Style::default().fg(label_color()))),
        ])
        .render(area, buffer);
        return;
    }

    // One column-label row for the whole cell. Every group's table is
    // laid out with the same constraints and the same indent, so the
    // labels stay over their columns without costing a row per group.
    let layout = TableLayout::of(rows, kind, area, ground, tree);
    let header_area = Rect {
        height: TABLE_HEADER_HEIGHT.min(area.height),
        ..indented(area)
    };
    let faded = heading_fade(rows);
    if layout.columns.is_empty() {
        Paragraph::new(Line::from(Span::styled(
            ELISION,
            column_header_style(&layout, faded),
        )))
        .render(header_area, buffer);
        return;
    }
    Table::new(Vec::<Row>::new(), layout.constraints.iter().copied())
        .header(column_header(&layout, faded))
        .column_spacing(layout.column_spacing)
        .render(header_area, buffer);

    let groups = group_by_path(rows, pinned);
    let drawn_groups: Vec<Vec<DrawnRow<'_>>> = groups
        .iter()
        .map(|group| {
            group
                .rows
                .iter()
                .map(|row| process_row(row, &layout))
                .collect()
        })
        .collect();
    let available_height = area.height.saturating_sub(TABLE_HEADER_HEIGHT);
    let plan = process_table_plan(&drawn_groups, available_height);
    let mut continuation = TableContinuation::FullyDrawn;
    let mut drew_process_value = false;
    let mut remaining = area;
    remaining.y = remaining.y.saturating_add(TABLE_HEADER_HEIGHT);
    remaining.height = remaining.height.saturating_sub(TABLE_HEADER_HEIGHT);
    for (index, (group, rows)) in groups
        .iter()
        .zip(drawn_groups)
        .take(plan.visible_group_count)
        .enumerate()
    {
        let gap_before = if index == 0 || !plan.keep_gaps {
            0
        } else {
            GROUP_GAP_HEIGHT
        };
        remaining.y = remaining.y.saturating_add(gap_before);
        remaining.height = remaining.height.saturating_sub(gap_before);
        let drawn = draw_path_group(buffer, remaining, group, &layout, rows);
        drew_process_value = true;
        continuation = drawn.continuation;
        remaining.y = remaining.y.saturating_add(drawn.height);
        remaining.height = remaining.height.saturating_sub(drawn.height);
        if !matches!(continuation, TableContinuation::FullyDrawn) {
            break;
        }
    }
    if plan.visible_group_count < groups.len()
        && drew_process_value
        && matches!(continuation, TableContinuation::FullyDrawn)
    {
        continuation = TableContinuation::RowsWereOmitted;
    }
    if !matches!(continuation, TableContinuation::FullyDrawn) {
        mark_table_continuation(buffer, area, &layout, continuation);
    }
}

/// Which directory groups fit, with one uniform separator policy between them.
struct ProcessTablePlan {
    /// Whether every drawn pair of adjacent groups keeps its blank row.
    keep_gaps:           bool,
    /// Groups with room for both their heading and first process line.
    visible_group_count: usize,
}

impl ProcessTablePlan {
    /// Rows occupied by the groups selected for this height.
    fn rows_drawn(&self, groups: &[Vec<DrawnRow<'_>>], available_height: u16) -> u16 {
        let mut remaining = available_height;
        let mut drawn = 0u16;
        for (index, rows) in groups.iter().take(self.visible_group_count).enumerate() {
            let gap = if index == 0 || !self.keep_gaps {
                0
            } else {
                GROUP_GAP_HEIGHT
            };
            let gap = gap.min(remaining);
            remaining = remaining.saturating_sub(gap);
            drawn = drawn.saturating_add(gap);

            let process_rows = rows
                .iter()
                .map(|row| row.lines)
                .fold(0, u16::saturating_add);
            let heading = GROUP_HEADER_HEIGHT.min(remaining);
            let processes = remaining.saturating_sub(heading).min(process_rows);
            let group_rows = heading.saturating_add(processes);
            remaining = remaining.saturating_sub(group_rows);
            drawn = drawn.saturating_add(group_rows);
            if processes < process_rows {
                break;
            }
        }
        drawn
    }
}

/// Choose all inter-directory gaps before drawing any table content.
fn process_table_plan(groups: &[Vec<DrawnRow<'_>>], available_height: u16) -> ProcessTablePlan {
    let group_heights: Vec<u16> = groups
        .iter()
        .map(|rows| {
            GROUP_HEADER_HEIGHT.saturating_add(
                rows.iter()
                    .map(|row| row.lines)
                    .fold(0, u16::saturating_add),
            )
        })
        .collect();
    let all_groups_height = group_heights.iter().copied().fold(0, u16::saturating_add);
    let all_gaps_height = u16::try_from(groups.len().saturating_sub(1))
        .unwrap_or(u16::MAX)
        .saturating_mul(GROUP_GAP_HEIGHT);
    let keep_gaps = all_groups_height.saturating_add(all_gaps_height) <= available_height;
    if keep_gaps {
        return ProcessTablePlan {
            keep_gaps,
            visible_group_count: groups.len(),
        };
    }

    let minimum_group_height = GROUP_HEADER_HEIGHT.saturating_add(1);
    let mut full_groups_height = 0u16;
    let mut visible_group_count = 0usize;

    for group_height in group_heights {
        let minimum_end = full_groups_height.saturating_add(minimum_group_height);
        if minimum_end > available_height {
            break;
        }

        visible_group_count = visible_group_count.saturating_add(1);
        full_groups_height = full_groups_height.saturating_add(group_height);
    }

    ProcessTablePlan {
        keep_gaps,
        visible_group_count,
    }
}

/// Collect rows by account, root incarnation and raw working directory.
///
/// A row joins the first group, in arrival order, whose identity is compatible
/// with its own: an uncaptured row whose owner is observed joins a captured
/// group of that uid rather than repeating its heading. The oldest invocation
/// orders each group; the pinned identity keeps the command's own directory
/// first even when another heading has identical text. A linear search
/// preserves arrival order for ties across the handful of working directories
/// normally visible at once.
fn group_by_path<'a>(rows: &[&'a TrackedRow], pinned: PinnedGroup<'_>) -> Vec<PathGroup<'a>> {
    let mut groups: Vec<PathGroup<'a>> = Vec::new();
    for row in rows {
        let identity = GroupingIdentity::from(&row.process);
        if let Some(group) = groups.iter_mut().find(|group| {
            matches!(identity.directory, GroupDirectory::Absolute(_))
                && group.identity.is_compatible(&identity)
        }) {
            // Owner rows accept any root of their uid; once a captured row names
            // one, a different root or incarnation forms its own group.
            if matches!(identity.qualification, GroupQualification::Captured { .. }) {
                group.identity = identity;
            }
            group.rows.push(row);
            continue;
        }
        groups.push(PathGroup {
            identity,
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
    //
    // Pid breaks a tie, the same rule [`Census::groups`] orders leads
    // by: the start time is whole seconds, so a command and the cargo it
    // launches almost always share one, and macOS hands pids out in
    // order -- the lower pid is the earlier start. Without it the tied
    // rows kept arrival order, which drew `cargo clippy` under the
    // `cargo check` it had just started.
    for group in &mut groups {
        group
            .rows
            .sort_by_key(|row| (row.process.started, row.process.pid));
    }
    // The lead finds its group the way a row does, first compatible in
    // arrival order, so an owner-only lead pins the group it joined even
    // when a later root of the same uid sorts ahead of it.
    let pinned = match pinned {
        PinnedGroup::Lead(lead) => groups
            .iter()
            .position(|group| group.identity.is_compatible(&lead))
            .map(|at| groups.remove(at)),
        PinnedGroup::Unpinned => None,
    };
    groups.sort_by_key(|group| {
        group
            .rows
            .first()
            .map_or((RunStart::Unavailable, u32::MAX), |row| {
                (row.process.started, row.process.pid)
            })
    });
    // Whatever the rest sort to, the pinned directory heads the cell.
    // The others keep the order they had under it, so a group that
    // comes and goes moves nothing but itself.
    if let Some(group) = pinned {
        groups.insert(0, group);
    }
    groups
}

/// Result of drawing one working-directory group into its remaining table area.
struct PathGroupDraw {
    /// Rows occupied by the heading and process table.
    height:       u16,
    /// What, if anything, was left below the group's visible table lines.
    continuation: TableContinuation,
}

/// How a table continues below the lines that reached the buffer.
enum TableContinuation {
    /// Every value was drawn in full.
    FullyDrawn,
    /// The final visible command continues onto lines that did not fit.
    CommandWasCut {
        /// Styled text that follows the last visible command line.
        continuation: CutCommandContinuation,
    },
    /// The final visible command is whole, but later process rows did not fit.
    RowsWereOmitted,
}

/// Classify a vertical cut through drawn rows and retain a cut command's next line.
fn table_continuation(rows: &[DrawnRow<'_>], visible_lines: u16) -> TableContinuation {
    let mut used = 0u16;
    for row in rows {
        if used >= visible_lines {
            return TableContinuation::RowsWereOmitted;
        }
        if used.saturating_add(row.lines) > visible_lines {
            let first_hidden_line = usize::from(visible_lines.saturating_sub(used));
            return TableContinuation::CommandWasCut {
                continuation: row.cut_command_continuation(first_hidden_line),
            };
        }
        used = used.saturating_add(row.lines);
    }
    TableContinuation::FullyDrawn
}

/// Draw one working directory's header and table into the top of `area`.
fn draw_path_group(
    buffer: &mut Buffer,
    area: Rect,
    group: &PathGroup<'_>,
    layout: &TableLayout,
    rows: Vec<DrawnRow<'_>>,
) -> PathGroupDraw {
    let faded = heading_fade(&group.rows);
    let heading_room = area.width.saturating_sub(cell_width(SECTION_HEADER_INDENT));
    let heading_text = group.fitted_heading(heading_room);
    let mut heading = vec![
        Span::raw(SECTION_HEADER_INDENT),
        Span::styled(
            heading_text,
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
    let lines = rows
        .iter()
        .map(|drawn| drawn.lines)
        .fold(0, u16::saturating_add);
    let table_height = area.height.saturating_sub(GROUP_HEADER_HEIGHT).min(lines);
    let continuation = table_continuation(&rows, table_height);
    Table::new(
        rows.into_iter().map(|drawn| drawn.into_table_row(layout)),
        layout.constraints.iter().copied(),
    )
    .column_spacing(layout.column_spacing)
    .render(
        Rect {
            y: area.y.saturating_add(GROUP_HEADER_HEIGHT),
            height: table_height,
            ..indented(area)
        },
        buffer,
    );

    PathGroupDraw {
        height: GROUP_HEADER_HEIGHT.saturating_add(table_height),
        continuation,
    }
}

/// Keep the start of summary text and mark a missing tail.
fn elide_summary_end(text: &str, width: u16) -> String {
    if UnicodeWidthStr::width(text) <= usize::from(width) {
        return text.to_string();
    }
    let marker_width = UnicodeWidthStr::width(ELISION);
    if marker_width > usize::from(width) {
        return String::new();
    }
    let room = usize::from(width).saturating_sub(marker_width);
    let mut used: usize = 0;
    let prefix: String = text
        .chars()
        .take_while(|character| {
            let character_width = UnicodeWidthChar::width(*character).unwrap_or_default();
            let fits = used.saturating_add(character_width) <= room;
            if fits {
                used = used.saturating_add(character_width);
            }
            fits
        })
        .collect();
    let prefix = prefix.trim_end();
    if prefix.is_empty() {
        return String::new();
    }
    format!("{prefix}{ELISION}")
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
        widths.observe_cell_usize(PARENT_COLUMN, parent_text(process).chars().count());
        widths.observe_cell_usize(START_COLUMN, process.start.chars().count());
        widths.observe_cell_usize(DURATION_COLUMN, process.duration.chars().count());
        widths.observe_cell_usize(CPU_COLUMN, process.cpu.to_string().chars().count());
        widths.observe_cell_usize(
            MEMORY_COLUMN,
            process.memory.map(memory_label).to_string().chars().count(),
        );
        widths.observe_cell_usize(STATE_COLUMN, state_width(&process.state));
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

/// Gap that keeps a whole command on one line before spreading columns apart.
fn table_column_spacing(
    width: u16,
    constraints: &[Constraint],
    columns: &[usize],
    command_width: u16,
) -> u16 {
    if table_width_with_spacing(constraints, TABLE_COLUMN_SPACING) > width {
        return TIGHT_TABLE_COLUMN_SPACING;
    }
    let Some(command_position) = columns.iter().position(|column| *column == COMMAND_COLUMN) else {
        return TABLE_COLUMN_SPACING;
    };
    let solved = solved_column_widths(width, constraints, TABLE_COLUMN_SPACING);
    if solved[command_position] >= command_width {
        TABLE_COLUMN_SPACING
    } else {
        TIGHT_TABLE_COLUMN_SPACING
    }
}

/// Width of the widest command line as this table spells it.
fn longest_command_width(rows: &[&TrackedRow], detail: SummaryDetail, tree: ProcessTree) -> u16 {
    rows.iter()
        .map(|row| command_line_width(row, detail, tree))
        .max()
        .unwrap_or_default()
}

/// Cells one displayed command takes before wrapping.
fn command_line_width(row: &TrackedRow, detail: SummaryDetail, tree: ProcessTree) -> u16 {
    let arguments = match tree {
        ProcessTree::Long => row.process.command.line(detail),
        ProcessTree::Short => row.process.command.named(),
    };
    row.process
        .command
        .program
        .split_whitespace()
        .chain(arguments.split_whitespace())
        .enumerate()
        .fold(0, |width, (index, word)| {
            width
                .saturating_add(u16::from(index > 0))
                .saturating_add(cell_width(word))
        })
}

/// Whether every requested width and one-cell gap fits in `width`.
fn table_constraints_fit(width: u16, constraints: &[Constraint]) -> bool {
    table_width_with_spacing(constraints, TIGHT_TABLE_COLUMN_SPACING) <= width
}

/// Width requested by `constraints`, including the gaps between them.
fn table_width_with_spacing(constraints: &[Constraint], spacing: u16) -> u16 {
    let columns_width = constraints
        .iter()
        .map(|constraint| match constraint {
            Constraint::Length(width) | Constraint::Min(width) => *width,
            // A future constraint kind cannot establish a finite table width.
            _ => u16::MAX,
        })
        .fold(0, u16::saturating_add);
    let gaps = u16::try_from(constraints.len().saturating_sub(1)).unwrap_or(u16::MAX);
    columns_width.saturating_add(spacing.saturating_mul(gaps))
}

/// Remove `column` and its aligned constraint when it is present.
fn remove_column(column: usize, columns: &mut Vec<usize>, constraints: &mut Vec<Constraint>) {
    if let Some(position) = columns.iter().position(|candidate| *candidate == column) {
        columns.remove(position);
        constraints.remove(position);
    }
}

/// The cell's one column-label row, drawn above the first group and
/// aligned with every group's rows by [`indented`].
fn column_header(layout: &TableLayout, faded: u8) -> Row<'static> {
    let style = column_header_style(layout, faded);
    Row::new(
        TABLE_HEADERS
            .iter()
            .enumerate()
            .filter(|(column, _)| layout.columns.contains(column))
            .map(|(_, label)| Span::styled((*label).to_string(), style)),
    )
}

/// Style shared by the column labels and their no-room stand-in.
fn column_header_style(layout: &TableLayout, faded: u8) -> Style {
    Style::default().fg(layout.ink(label_color(), faded))
}

/// One process row prepared for measurement and table rendering.
struct DrawnRow<'a> {
    /// Cells in canonical column order, filtered only when the table takes them.
    cells:   [Text<'static>; TABLE_HEADERS.len()],
    /// Lines the row occupies. The command is the one cell that ever
    /// asks for more than one, and it asks for as many as its line
    /// wrapped to.
    lines:   u16,
    /// Original command retained only to classify the boundary of an actual cut.
    command: &'a CommandText,
    /// Detail used to prepare the command cell.
    detail:  SummaryDetail,
    /// Tree mode used to prepare the command cell.
    tree:    ProcessTree,
}

impl DrawnRow<'_> {
    /// Turn the prepared cells into the columns selected for this table.
    fn into_table_row(self, layout: &TableLayout) -> Row<'static> {
        Row::new(
            self.cells
                .into_iter()
                .enumerate()
                .filter(|(column, _)| layout.columns.contains(column))
                .map(|(_, cell)| cell),
        )
        .height(self.lines)
    }

    /// Styled text immediately below the one visible line where this command was cut.
    fn cut_command_continuation(&self, first_hidden_line: usize) -> CutCommandContinuation {
        let arguments = match self.tree {
            ProcessTree::Long => self.command.line(self.detail),
            ProcessTree::Short => self.command.named(),
        };
        let command = format!("{} {arguments}", self.command.program);
        command_line_continuation(
            &command,
            &self.cells[COMMAND_COLUMN].lines,
            first_hidden_line,
        )
    }
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
fn process_row<'a>(row: &'a TrackedRow, layout: &TableLayout) -> DrawnRow<'a> {
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
    let command_width = layout.column_width(COMMAND_COLUMN);
    let arguments_text = match layout.tree {
        ProcessTree::Long => process.command.line(layout.detail),
        ProcessTree::Short => process.command.named(),
    };
    let command = if command_width == 0 {
        Text::default()
    } else {
        tui_pane::wrapped(
            vec![
                Span::styled(process.command.program.clone(), program),
                Span::styled(arguments_text, arguments),
            ],
            command_width,
        )
    };
    let lines = u16::try_from(command.height()).unwrap_or(u16::MAX).max(1);
    let pid = number_text_if_fits(process.pid.to_string(), layout.column_width(PID_COLUMN));
    let parent = number_text_if_fits(parent_text(process), layout.column_width(PARENT_COLUMN));
    let cells = [
        Text::from(Span::styled(pid, pid_style(row, layout))),
        Text::from(Span::styled(parent, parent_style(row, layout))),
        Text::from(Span::styled(process.start.clone(), muted)),
        Text::from(Span::styled(process.duration.clone(), muted)),
        Text::from(Span::styled(process.cpu.to_string(), muted)),
        Text::from(Span::styled(
            process.memory.map(memory_label).to_string(),
            muted,
        )),
        Text::from(state_cell(row, layout)),
        command,
        Text::from(compiler_cell(row, layout)),
        Text::from(Span::styled(managed_text(process), muted)),
    ];
    DrawnRow {
        cells,
        lines,
        command: &process.command,
        detail: layout.detail,
        tree: layout.tree,
    }
}

/// The one wrapped line below a vertical cut and its source boundary.
fn command_line_continuation(
    command: &str,
    lines: &[Line<'static>],
    first_hidden_line: usize,
) -> CutCommandContinuation {
    let source: Vec<char> = command
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .collect();
    let mut consumed = 0usize;
    for line in lines.iter().take(first_hidden_line) {
        if source.get(consumed).is_some_and(char::is_ascii_whitespace) {
            consumed = consumed.saturating_add(1);
        }
        consumed = consumed.saturating_add(
            line.spans
                .iter()
                .map(|span| span.content.chars().count())
                .sum::<usize>(),
        );
    }
    let join = if source.get(consumed).is_some_and(char::is_ascii_whitespace) {
        CommandContinuationJoin::WordSeparator
    } else {
        CommandContinuationJoin::Direct
    };
    CutCommandContinuation {
        join,
        next_line: lines[first_hidden_line].clone(),
    }
}

/// The `parent` cell: the cargo an invocation is running under, and an
/// empty cell for a command nothing above it started.
fn parent_text(process: &CargoProcess) -> String {
    match &process.parent {
        VisibleParent::Invocation { pid, .. } | VisibleParent::Ancestor(pid) => pid.to_string(),
        VisibleParent::None => String::new(),
    }
}

/// Keep a numeric cell only when its solved column holds every digit.
fn number_text_if_fits(text: String, width: u16) -> String {
    if cell_width(&text) <= width {
        text
    } else {
        String::new()
    }
}

/// The `pid` cell's style: the family colour where the invocation has
/// cargo running under it, and plain text where it does not.
///
/// Only a command with something under it takes a family colour. The
/// colour exists to be matched against the `parent` cells pointing at
/// it, so on a row nothing points at it would be a mark meaning
/// nothing.
///
/// What is left over is [`text_default`] rather than the muted colour
/// the rest of the row is drawn in, which is the colour of the column
/// headers. A pid written in the header's own colour disappears into
/// the header, and a pid is the one number on this screen that is read
/// by being searched for.
fn pid_style(row: &TrackedRow, layout: &TableLayout) -> Style {
    let color = match row.family() {
        FamilyHead::Heads(family) => theme::family_color(family),
        FamilyHead::NoChildren => text_default(),
    };
    Style::default().fg(layout.ink(color, row.faded()))
}

/// The `parent` cell's style: the family colour of the cargo named,
/// which is the colour that cargo's own `pid` cell carries, and plain
/// text where the pid names something that heads no family.
fn parent_style(row: &TrackedRow, layout: &TableLayout) -> Style {
    let color = match row.parent_family() {
        ParentFamily::Member(family) => theme::family_color(family),
        ParentFamily::NoFamily => text_default(),
    };
    Style::default().fg(layout.ink(color, row.faded()))
}

/// Solved widths for every constraint, matching [`Table`]'s layout.
fn solved_column_widths(width: u16, constraints: &[Constraint], column_spacing: u16) -> Vec<u16> {
    if constraints.is_empty() {
        return Vec::new();
    }
    Layout::horizontal(constraints.iter().copied())
        .spacing(column_spacing)
        .split(Rect {
            x: 0,
            y: 0,
            width,
            height: 1,
        })
        .iter()
        .map(|rect| rect.width)
        .collect()
}

/// The columns a cell draws, in table order.
///
/// The summary omits observed compiler and managed counts, but preserves
/// unavailable measurements. The `state` column joins only for a row waiting
/// on a lock: a heading is per directory and a wait is per
/// row, so nothing but the column can say which row is waiting. A
/// reading never brings it in -- the heading over the row is already
/// ruling that -- and an empty column costs a narrow tile the width its
/// command line needs while reporting nothing.
fn visible_columns(rows: &[&TrackedRow], kind: TableKind) -> Vec<usize> {
    let carries_state = rows.iter().any(|row| {
        matches!(
            row.process.state,
            CaptureLookup::Registered(CaptureRead::Progress(RunState::Blocked))
        )
    });
    (0..TABLE_HEADERS.len())
        .filter(|column| *column != STATE_COLUMN || carries_state)
        .filter(|column| {
            if !SUMMARY_HIDDEN_COLUMNS.contains(column) {
                return true;
            }
            rows.iter().any(|row| match *column {
                PARENT_COLUMN => kind.shows_invocation_detail(),
                COMPILER_COLUMN => match &row.process.compiler {
                    CompilerObservation::Unknown => true,
                    CompilerObservation::Running(_) => kind.shows_invocation_detail(),
                    CompilerObservation::None => false,
                },
                MANAGED_COLUMN => match &row.process.managed {
                    Measurement::Unavailable(_) => true,
                    Measurement::Reading(count) => kind.shows_invocation_detail() && *count > 0,
                },
                _ => false,
            })
        })
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
    let CaptureLookup::Registered(CaptureRead::Progress(RunState::Blocked)) = row.process.state
    else {
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
fn state_width(state: &CaptureLookup) -> usize {
    match state {
        CaptureLookup::Registered(CaptureRead::Progress(RunState::Blocked)) => {
            STATE_BLOCKED.chars().count()
        },
        CaptureLookup::Unregistered
        | CaptureLookup::Registered(
            CaptureRead::Progress(RunState::Working { .. })
            | CaptureRead::NoCurrentProgress
            | CaptureRead::Unreadable(_),
        ) => 0,
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
    let Some((row, (phase, progress))) =
        group
            .rows
            .iter()
            .find_map(|row| match CounterState::from(&row.process.state) {
                CounterState::Working { phase, progress } => Some((row, (phase, progress))),
                CounterState::Blocked
                | CounterState::NoCurrentProgress
                | CounterState::Unavailable
                | CounterState::Unregistered => None,
            })
    else {
        return Vec::new();
    };
    let reading = percent_reading(progress);
    let fixed = cell_width(SECTION_HEADER_INDENT)
        .saturating_add(cell_width(&group.heading()))
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
/// nothing for a measured zero, and a marker when the count is unavailable.
fn managed_text(process: &CargoProcess) -> String {
    match process.managed {
        Measurement::Reading(0) => String::new(),
        Measurement::Reading(count) => count.to_string(),
        Measurement::Unavailable(_) => UNAVAILABLE_MEASUREMENT.to_string(),
    }
}

/// Resident bytes as gibibytes to one decimal place, rounded half up.
fn memory_label(bytes: u64) -> String {
    let tenths = (u128::from(bytes) * 10 + u128::from(BYTES_PER_GIBIBYTE) / 2)
        / u128::from(BYTES_PER_GIBIBYTE);
    format!("{}.{}{MEMORY_UNIT}", tenths / 10, tenths % 10)
}

/// The `compiler` cell: driver name in the active color, its count muted
/// beside it, a marker when observation is unknown, and nothing when no
/// compile is in flight.
fn compiler_cell(row: &TrackedRow, layout: &TableLayout) -> Line<'static> {
    let faded = row.faded();
    let driver = if row.is_ended() {
        label_color()
    } else {
        success_color()
    };
    let name = Style::default().fg(layout.ink(driver, faded));
    let count = Style::default().fg(layout.ink(label_color(), faded));
    match &row.process.compiler {
        CompilerObservation::Unknown => Line::from(Span::styled(UNAVAILABLE_MEASUREMENT, count)),
        CompilerObservation::None => Line::default(),
        CompilerObservation::Running(compiler) => Line::from(vec![
            Span::styled(compiler.name, name),
            Span::styled(format!("\u{d7}{}", compiler.count), count),
        ]),
    }
}

/// Cells the `compiler` column needs for one row.
fn compiler_width(process: &CargoProcess) -> usize {
    match &process.compiler {
        CompilerObservation::Unknown => UNAVAILABLE_MEASUREMENT.chars().count(),
        CompilerObservation::None => 0,
        CompilerObservation::Running(compiler) => {
            compiler.name.chars().count()
                + COMPILER_SEPARATOR_WIDTH
                + compiler.count.to_string().chars().count()
        },
    }
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
    let mut notes = vec![StatusLineNote {
        label: APP_NAME.to_string(),
        value: APP_VERSION.to_string(),
    }];
    // A held display looks exactly like an idle one, and the difference
    // between the two is whether anything is being missed. Say which.
    if app.updates == Updates::Frozen {
        notes.push(StatusLineNote::flag(FROZEN_NOTE_LABEL));
    }
    // And a grid drawn over looks exactly like an empty one, which is
    // the same problem: say that the work is behind the animation
    // rather than absent.
    if app.attract.asked_for() {
        notes.push(StatusLineNote::flag(ATTRACT_NOTE_LABEL));
    }
    // The chain is half a cell that nothing on screen explains: the
    // short setting is where the display starts, so a cell full of
    // ancestry is a key that was pressed rather than a command with a
    // long history. Say which.
    if app.tree == ProcessTree::Long {
        notes.push(StatusLineNote::flag(PROCESS_TREE_NOTE_LABEL));
    }
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
        &BarPalette::themed(),
        &status,
    );
}

/// Read the CPU values selected by the production summary without exposing its rows.
#[cfg(test)]
pub(crate) fn summary_cpu_for_test(
    roster: &Roster,
    hidden_when_idle: &[String],
) -> Vec<(u32, Measurement<String>)> {
    summary_rows(roster, hidden_when_idle)
        .into_iter()
        .map(|row| (row.process.pid, row.process.cpu.clone()))
        .collect()
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::panic,
    clippy::unwrap_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::collections::HashMap;
    use std::ffi::OsStr;
    use std::ffi::OsString;
    use std::fs;
    use std::io::ErrorKind;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::MetadataExt;
    use std::path::Path;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::Instant;

    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Cell;
    use sysinfo::Pid;
    use tui_pane::Appearance;
    use tui_pane::BackdropNotice;
    use tui_pane::FavoritesFileState;
    use tui_pane::GlobalAction;
    use tui_pane::TileFill;
    use tui_pane::TileGrid;
    use tui_pane::TileGrowth;
    use tui_pane::TilePlacement;
    use tui_pane::TileView;
    use tui_pane::ToastStyle;
    use tui_pane::draw_backdrop_notice;
    use tui_pane::fade_to_background;

    use super::*;
    use crate::birth_stamp::BirthStamp;
    use crate::birth_stamp::IdentityEvidence;
    use crate::birth_stamp::KernelObservation;
    use crate::birth_stamp::Observation;
    use crate::census;
    use crate::census::CargoGroup;
    use crate::census::CargoProcess;
    use crate::census::InvocationId;
    use crate::census::VisibleParent;
    use crate::census::command_text::CommandText;
    use crate::census::invocation_cpu_accounting::MeasurementAbsence;
    use crate::census::process_identity::CaptureMembership;
    use crate::census::process_identity::RunId;
    use crate::census::scan::CensusSequence;
    use crate::census::scan::Compiler;
    use crate::census::scan::ProcessField;
    use crate::census::scan::ProcessObservation;
    use crate::census::scan::ProcessObservations;
    use crate::constants::COMPILER_PROCESS_NAMES;
    use crate::constants::LOCK_WAIT_MARKER;
    use crate::constants::PHASE_TESTING;
    use crate::constants::PORT_SUBCOMMAND_NAME;
    use crate::constants::REGISTRATION_MAGIC;
    use crate::constants::UNRESOLVED_TIME;
    use crate::probe::FrameLog;
    use crate::progress::capture::Capture;
    use crate::progress::capture_read::Phase;
    use crate::progress::capture_roots::CaptureRoots;
    use crate::progress::capture_roots::RootReadStatus;

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
        capture_gauge_text(
            CaptureLookup::Registered(CaptureRead::Progress(state)),
            width,
        )
    }

    /// Exercise the gauge at the complete capture-outcome boundary.
    fn capture_gauge_text(state: CaptureLookup, width: u16) -> String {
        let mut row = row_at(GAUGE_PATH, None);
        row.process.state = state;
        let rows = [&row];
        let area = Rect {
            x: 0,
            y: 0,
            width,
            height: 1,
        };
        let layout = TableLayout::of(
            &rows,
            TableKind::Command,
            area,
            pane_background(false),
            ProcessTree::Long,
        );
        let group = PathGroup {
            identity: GroupingIdentity::from(&row.process),
            path:     GAUGE_PATH,
            rows:     rows.to_vec(),
        };
        heading_gauge(&group, width, &layout)
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    /// The gauge keeps no-counter reasons distinct while drawing no false progress.
    #[test]
    fn named_counter_absences_do_not_draw_a_gauge() {
        for (lookup, counter) in [
            (CaptureLookup::Unregistered, CounterState::Unregistered),
            (
                CaptureLookup::Registered(CaptureRead::NoCurrentProgress),
                CounterState::NoCurrentProgress,
            ),
            (
                CaptureLookup::Registered(CaptureRead::Progress(RunState::Blocked)),
                CounterState::Blocked,
            ),
            (
                CaptureLookup::Registered(CaptureRead::Unreadable(
                    std::io::Error::from(ErrorKind::PermissionDenied).into(),
                )),
                CounterState::Unavailable,
            ),
        ] {
            assert_eq!(CounterState::from(&lookup), counter);
            assert!(capture_gauge_text(lookup, 60).is_empty(), "{counter:?}");
        }
        assert_ne!(gauge_text(compiling(1, 2), 60), "");
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

    /// A row whose fitted columns require the smaller gap in a 65-cell area.
    fn tight_command_row(pid: u32, parent: u32, arguments: &[&str]) -> TrackedRow {
        let mut row = row(None);
        row.process.pid = pid;
        row.process.invocation_id = InvocationId::for_test(pid);
        row.process.parent = VisibleParent::Ancestor(parent);
        row.process.start = "17:18".to_string();
        row.process.duration = "01:12".to_string();
        row.process.cpu = Measurement::Reading("1177%".to_string());
        row.process.memory = Measurement::Reading(BYTES_PER_GIBIBYTE * 2);
        row.process.compiler = CompilerObservation::Running(Compiler {
            name:  COMPILER_PROCESS_NAMES[0],
            count: 1,
        });
        row.process.managed = Measurement::Reading(1);
        row.process.command = CommandText::of("cargo", arguments);
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

    /// Text drawn inside one solved table column.
    fn table_column_text(buffer: &Buffer, layout: &TableLayout, column: usize, y: u16) -> String {
        let position = layout
            .columns
            .iter()
            .position(|candidate| *candidate == column)
            .expect("the requested test column is visible");
        let preceding_width: u16 = layout.column_widths[..position]
            .iter()
            .copied()
            .fold(0, u16::saturating_add);
        let preceding_gaps = u16::try_from(position)
            .unwrap_or(u16::MAX)
            .saturating_mul(layout.column_spacing);
        let start = cell_width(SECTION_ITEM_INDENT)
            .saturating_add(preceding_width)
            .saturating_add(preceding_gaps);
        let end = start.saturating_add(layout.column_widths[position]);
        (start..end)
            .map(|x| buffer[(x, y)].symbol())
            .collect::<String>()
    }

    /// Contiguous ASCII digit sequences in `text`.
    fn digit_runs(text: &str) -> Vec<String> {
        text.split(|character: char| !character.is_ascii_digit())
            .filter(|run| !run.is_empty())
            .map(str::to_string)
            .collect()
    }

    /// Draw the complete table so measurement assertions include column fitting.
    fn measurement_row_buffer(row: &TrackedRow, kind: TableKind) -> Buffer {
        measurement_group_buffer(row.process.clone(), Vec::new(), kind)
    }

    /// Keep ancestry empty so both production layouts draw CPU below the same headers.
    fn measurement_group_buffer(
        lead: CargoProcess,
        rest: Vec<CargoProcess>,
        kind: TableKind,
    ) -> Buffer {
        let id = lead.invocation_id.clone();
        let roster = roster_with_ancestry(lead, rest, Vec::new());
        let area = Rect::new(0, 0, 120, 6);
        let mut buffer = Buffer::empty(area);
        match kind {
            TableKind::Command => draw_group(
                &mut buffer,
                &roster,
                &id,
                area,
                Color::Reset,
                &hidden_when_idle(),
                ProcessTree::Long,
            ),
            TableKind::Summary => draw_summary(
                &mut buffer,
                &roster,
                area,
                Color::Reset,
                &hidden_when_idle(),
                ProcessTree::Long,
            ),
        }
        buffer
    }

    /// Read the invocation below the column labels and directory heading.
    fn measurement_row_text(row: &TrackedRow, kind: TableKind) -> String {
        buffer_line(
            &measurement_row_buffer(row, kind),
            TABLE_HEADER_HEIGHT + GROUP_HEADER_HEIGHT,
        )
    }

    /// Locate CPU by the drawn column labels so markers in other cells cannot pass.
    fn cpu_cell_text(buffer: &Buffer, y: u16) -> String {
        let header = buffer_line(buffer, 0);
        let start = header
            .find(TABLE_HEADERS[CPU_COLUMN])
            .expect("the table draws a CPU column");
        let end = TABLE_HEADERS[CPU_COLUMN + 1..]
            .iter()
            .filter_map(|label| header.find(label))
            .min()
            .expect("the table draws a column after CPU");
        buffer_line(buffer, y)[start..end].trim().to_string()
    }

    /// Locate memory by the drawn column labels.
    fn memory_cell_text(buffer: &Buffer, y: u16) -> String {
        let header = buffer_line(buffer, 0);
        let start = header
            .find(TABLE_HEADERS[MEMORY_COLUMN])
            .expect("the table draws a memory column");
        let end = TABLE_HEADERS[MEMORY_COLUMN + 1..]
            .iter()
            .filter_map(|label| header.find(label))
            .min()
            .expect("the table draws a column after memory");
        buffer_line(buffer, y)[start..end].trim().to_string()
    }

    #[test]
    fn memory_column_follows_cpu_and_precedes_state_and_command() {
        assert_eq!(TABLE_HEADERS[CPU_COLUMN + 1], "mem");
        assert_eq!(TABLE_HEADERS[MEMORY_COLUMN + 1], "state");

        let buffer = measurement_row_buffer(&row(None), TableKind::Command);
        let header = buffer_line(&buffer, 0);
        let cpu = header.find("cpu").expect("CPU heading");
        let memory = header.find("mem").expect("memory heading");
        let command = header.find("command").expect("command heading");

        assert!(!header.contains("state"), "{header:?}");
        assert!(cpu < memory && memory < command, "{header:?}");
    }

    #[test]
    fn memory_labels_use_gibibytes_rounded_to_one_decimal_place() {
        for (bytes, expected) in [
            (0, "0.0G"),
            (BYTES_PER_GIBIBYTE / 20, "0.0G"),
            (BYTES_PER_GIBIBYTE / 20 + 1, "0.1G"),
            (BYTES_PER_GIBIBYTE, "1.0G"),
            (BYTES_PER_GIBIBYTE * 1234 / 100, "12.3G"),
        ] {
            assert_eq!(memory_label(bytes), expected);
        }
    }

    /// A complete total adds each live group's aggregate memory.
    #[test]
    fn summary_memory_total_is_complete_when_every_running_group_is_readable() {
        let roster = memory_roster(&[
            (4100, Measurement::Reading(BYTES_PER_GIBIBYTE)),
            (4200, Measurement::Reading(BYTES_PER_GIBIBYTE * 3 / 2)),
        ]);

        assert_eq!(
            memory_total(&roster),
            SummaryMemoryTotal::Complete(BYTES_PER_GIBIBYTE * 5 / 2)
        );
        assert_eq!(summary_foot_text(memory_total(&roster).foot()), "mem 2.5G");
    }

    /// The light summary value reads on both its transparent and painted grounds.
    #[test]
    fn the_summary_memory_total_reads_on_the_light_theme() {
        let previous_theme = tui_pane::theme();
        let previous_transparency = tui_pane::transparent_background();
        let light = theme::builtins::builtins()
            .into_iter()
            .find(|variant| variant.appearance == Appearance::Light)
            .expect("the app ships a light theme")
            .theme;
        let expected_ink = light.text.default.color;
        tui_pane::set_active_theme(Arc::new(light));

        for transparent in [false, true] {
            tui_pane::set_transparent_background(transparent);
            let SummaryFoot::Text(line) = SummaryMemoryTotal::Complete(BYTES_PER_GIBIBYTE).foot()
            else {
                panic!("a memory reading supplies summary text");
            };
            let value = line.spans.last().expect("the total has a value span");
            let ground = pane_background(false);

            assert_eq!(value.style.fg, Some(expected_ink));
            assert_ne!(value.style.fg, Some(ground), "transparent={transparent}");
        }

        tui_pane::set_active_theme(previous_theme);
        tui_pane::set_transparent_background(previous_transparency);
    }

    /// A fading group has stopped and contributes nothing to the running total.
    #[test]
    fn summary_memory_total_excludes_ended_groups() {
        let live = memory_invocation(4100, Measurement::Reading(BYTES_PER_GIBIBYTE));
        let ended = memory_invocation(4200, Measurement::Reading(BYTES_PER_GIBIBYTE * 3 / 2));
        let mut roster = Roster::new();
        roster.observe(
            vec![cargo_group(live.clone()), cargo_group(ended)],
            Instant::now(),
        );
        roster.observe(vec![cargo_group(live)], Instant::now());

        assert_eq!(
            memory_total(&roster),
            SummaryMemoryTotal::Complete(BYTES_PER_GIBIBYTE)
        );
        assert_eq!(summary_foot_text(memory_total(&roster).foot()), "mem 1.0G");
    }

    /// Readable groups remain useful while an unreadable group marks the sum as partial.
    #[test]
    fn summary_memory_total_is_at_least_the_readable_groups() {
        let roster = memory_roster(&[
            (4100, Measurement::Reading(BYTES_PER_GIBIBYTE)),
            (
                4200,
                Measurement::Unavailable(MeasurementAbsence::ReadFailed),
            ),
        ]);

        assert_eq!(
            memory_total(&roster),
            SummaryMemoryTotal::AtLeast(BYTES_PER_GIBIBYTE)
        );
        assert_eq!(summary_foot_text(memory_total(&roster).foot()), "mem 1.0G+");
    }

    /// Running groups with no readable memory retain their distinct state.
    #[test]
    fn summary_memory_total_names_when_no_command_is_readable() {
        let roster = memory_roster(&[(
            4100,
            Measurement::Unavailable(MeasurementAbsence::ReadFailed),
        )]);

        assert_eq!(
            memory_total(&roster),
            SummaryMemoryTotal::NoReadableCommands
        );
        assert_eq!(summary_foot_text(memory_total(&roster).foot()), "mem --");
    }

    /// An empty roster has no memory total.
    #[test]
    fn summary_memory_total_names_when_nothing_is_running() {
        let total = memory_total(&Roster::new());

        assert_eq!(total, SummaryMemoryTotal::NoRunningCommands);
        assert_eq!(total.foot(), SummaryFoot::Empty);
    }

    /// Flatten a text foot for assertions on its label and value.
    fn summary_foot_text(foot: SummaryFoot) -> String {
        match foot {
            SummaryFoot::Text(line) => line
                .spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect(),
            SummaryFoot::Empty => String::new(),
        }
    }

    #[test]
    fn memory_cell_draws_available_and_unavailable_readings() {
        for (memory, expected) in [
            (
                Measurement::Unavailable(MeasurementAbsence::Unproven),
                UNAVAILABLE_MEASUREMENT,
            ),
            (
                Measurement::Reading(BYTES_PER_GIBIBYTE * 1234 / 100),
                "12.3G",
            ),
        ] {
            let mut row = row(None);
            row.process.memory = memory;

            for kind in [TableKind::Command, TableKind::Summary] {
                let buffer = measurement_row_buffer(&row, kind);
                assert_eq!(
                    memory_cell_text(&buffer, TABLE_HEADER_HEIGHT + GROUP_HEADER_HEIGHT),
                    expected,
                    "{kind:?}"
                );
            }
        }
    }

    /// Each absence reason occupies the CPU cell in both table layouts.
    #[test]
    fn unavailable_cpu_never_renders_as_a_measurement() {
        for reason in [
            MeasurementAbsence::FirstObservation,
            MeasurementAbsence::ReadFailed,
            MeasurementAbsence::Unproven,
        ] {
            let mut row = row(None);
            let pid = Pid::from_u32(row.process.pid);
            let shares = HashMap::from([(pid, Measurement::Unavailable(reason))]);
            row.process.cpu =
                census::scan::aggregate(&shares, std::iter::once(pid)).map(census::scan::cpu_label);

            for kind in [TableKind::Command, TableKind::Summary] {
                let buffer = measurement_row_buffer(&row, kind);

                assert_eq!(
                    cpu_cell_text(&buffer, TABLE_HEADER_HEIGHT + GROUP_HEADER_HEIGHT),
                    UNAVAILABLE_MEASUREMENT,
                    "{reason:?} in {kind:?}"
                );
            }
        }
    }

    /// Availability must not suppress measured zero or positive CPU shares.
    #[test]
    fn measured_cpu_including_zero_remains_visible() {
        for (reading, expected) in [(0.0, "0%"), (137.0, "137%")] {
            let mut row = row(None);
            let pid = Pid::from_u32(row.process.pid);
            let shares = HashMap::from([(pid, Measurement::Reading(reading))]);
            row.process.cpu =
                census::scan::aggregate(&shares, std::iter::once(pid)).map(census::scan::cpu_label);

            for kind in [TableKind::Command, TableKind::Summary] {
                let buffer = measurement_row_buffer(&row, kind);

                assert_eq!(
                    cpu_cell_text(&buffer, TABLE_HEADER_HEIGHT + GROUP_HEADER_HEIGHT),
                    expected,
                    "{reading} in {kind:?}"
                );
            }
        }
    }

    #[test]
    fn two_row_render_samples_keep_memory_aligned() {
        let mut build = row(None);
        build.process.memory = Measurement::Reading(322_122_547);
        let mut test = row(None);
        test.process.pid = 41_234;
        test.process.invocation_id = InvocationId::for_test(41_234);
        test.process.start = "11:05".to_string();
        test.process.duration = "00:08".to_string();
        test.process.cpu = Measurement::Reading("5%".to_string());
        test.process.memory = Measurement::Reading(BYTES_PER_GIBIBYTE * 124 / 10);
        test.process.command = CommandText::of("cargo", &["test"]);
        let rows = [&build, &test];
        let area = Rect::new(0, 0, 80, 5);

        let draw = |kind| {
            let mut buffer = Buffer::empty(area);
            draw_process_table(
                &mut buffer,
                area,
                &rows,
                kind,
                Color::Reset,
                PinnedGroup::Unpinned,
                ProcessTree::Long,
            );
            [
                buffer_line(&buffer, 0),
                buffer_line(&buffer, 2),
                buffer_line(&buffer, 3),
            ]
        };

        assert_eq!(
            draw(TableKind::Summary),
            [
                " pid    start  dur    cpu  mem    command",
                " 41233  11:04  00:18  12%  0.3G   cargo build",
                " 41234  11:05  00:08  5%   12.4G  cargo test",
            ]
        );
        assert_eq!(
            draw(TableKind::Command),
            [
                " pid    parent  start  dur    cpu  mem    command",
                " 41233          11:04  00:18  12%  0.3G   cargo build",
                " 41234          11:05  00:08  5%   12.4G  cargo test",
            ]
        );
    }

    #[test]
    fn an_unknown_compiler_observation_has_a_visible_cell() {
        let mut row = row(None);
        row.process.compiler = CompilerObservation::Unknown;

        let text = measurement_row_text(&row, TableKind::Command);

        assert!(text.ends_with(UNAVAILABLE_MEASUREMENT), "{text:?}");
        assert_eq!(
            compiler_width(&row.process),
            UNAVAILABLE_MEASUREMENT.chars().count()
        );
    }

    #[test]
    fn an_unknown_managed_count_has_a_visible_cell() {
        let mut row = row(None);
        row.process.managed = Measurement::Unavailable(MeasurementAbsence::Unproven);

        let text = measurement_row_text(&row, TableKind::Command);

        assert!(text.ends_with(UNAVAILABLE_MEASUREMENT), "{text:?}");
    }

    /// A registration cannot supply process measurements in either table layout.
    #[test]
    fn all_unproven_measurements_render_in_command_and_summary_views() {
        let mut row = row(None);
        row.process.cpu = Measurement::Unavailable(MeasurementAbsence::Unproven);
        row.process.compiler = CompilerObservation::Unknown;
        row.process.managed = Measurement::Unavailable(MeasurementAbsence::Unproven);

        for kind in [TableKind::Command, TableKind::Summary] {
            let buffer = measurement_row_buffer(&row, kind);
            let header = buffer_line(&buffer, 0);
            let text = buffer_line(&buffer, TABLE_HEADER_HEIGHT + GROUP_HEADER_HEIGHT);
            for column in [CPU_COLUMN, MEMORY_COLUMN, COMPILER_COLUMN, MANAGED_COLUMN] {
                let start = header
                    .find(TABLE_HEADERS[column])
                    .expect("measurement column");
                let end = TABLE_HEADERS[column + 1..]
                    .iter()
                    .filter_map(|label| header.find(label))
                    .min()
                    .unwrap_or(text.len());
                assert_eq!(text[start..end].trim(), UNAVAILABLE_MEASUREMENT, "{kind:?}");
            }
        }
    }

    #[test]
    fn observed_idle_compilers_and_zero_managed_runs_keep_their_cells_empty() {
        let row = row(None);

        let text = measurement_row_text(&row, TableKind::Command);

        assert!(text.ends_with("cargo build"), "{text:?}");
        assert_eq!(compiler_width(&row.process), 0);
        assert_eq!(managed_text(&row.process), "");
    }

    #[test]
    fn observed_compilers_and_managed_runs_keep_their_counts() {
        let mut row = row(None);
        let driver = COMPILER_PROCESS_NAMES[0];
        row.process.compiler = CompilerObservation::Running(Compiler {
            name:  driver,
            count: 2,
        });
        row.process.managed = Measurement::Reading(3);

        let text = measurement_row_text(&row, TableKind::Command);

        assert!(text.contains(&format!("{driver}\u{d7}2")), "{text:?}");
        assert!(text.ends_with('3'), "{text:?}");
    }

    /// A row for a command running in `path`.
    fn row_at(path: &str, state: Option<RunState>) -> TrackedRow { started_at(path, state, 0) }

    /// The same, for a command that started `started` seconds into the
    /// epoch -- which is what orders one directory against another.
    fn started_at(path: &str, state: Option<RunState>, started: u64) -> TrackedRow {
        TrackedRow::from(CargoProcess {
            path:               path.to_string(),
            directory_identity: WorkingDirectoryIdentity::Absolute(
                Path::new("/test-home").join(path),
            ),
            pid:                41233,
            invocation_id:      InvocationId::for_test(41233),
            capture_membership: CaptureMembership::Outside,
            provenance:         RowProvenance::Uncaptured(ProcessOwner::Unavailable),
            parent:             VisibleParent::None,
            start:              "11:04".to_string(),
            started:            RunStart::Known(started),
            duration:           "00:18".to_string(),
            cpu:                Measurement::Reading("12%".to_string()),
            subtree_cpu:        Measurement::Reading("12%".to_string()),
            memory:             Measurement::Unavailable(MeasurementAbsence::Unproven),
            subtree_memory:     Measurement::Unavailable(MeasurementAbsence::Unproven),
            compiler:           CompilerObservation::None,
            state:              state.map_or(CaptureLookup::Unregistered, |state| {
                CaptureLookup::Registered(CaptureRead::Progress(state))
            }),
            managed:            Measurement::Reading(0),
            nested:             false,
            command:            CommandText::of("cargo", &["build"]),
        })
    }

    /// The same, for one of a pair that started inside the same second
    /// -- where the pid is all that separates them.
    fn same_second(path: &str, pid: u32) -> TrackedRow {
        let mut row = started_at(path, None, 100);
        row.process.pid = pid;
        row.process.invocation_id = InvocationId::for_test(pid);
        row
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
    fn drawn(levels: &[AncestryLevel<'_>]) -> Vec<Option<u32>> {
        levels
            .iter()
            .map(|level| match level {
                AncestryLevel::Ancestor(ancestor)
                | AncestryLevel::AncestorAfterElision(ancestor) => Some(ancestor.pid),
                AncestryLevel::Elided => None,
            })
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

    /// Under three rows there is no room for two ends and a separate
    /// elision between them, so the cut and the nearest ancestor survive.
    #[test]
    fn a_short_ancestry_keeps_its_cut_mark_and_nearest_ancestor() {
        let ancestry = chain(6);
        assert_eq!(drawn(&ancestry_levels(&ancestry, 2)), vec![None, Some(5)]);
        assert_eq!(drawn(&ancestry_levels(&ancestry, 1)), vec![Some(5)]);
    }

    /// One available ancestry row combines the cut mark with the nearest
    /// ancestor rather than showing a bare root and dropping the leaf.
    #[test]
    fn one_ancestry_row_draws_the_marked_nearest_ancestor() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 24, 3));
        let area = buffer.area;
        let ancestry = vec![
            ancestor(3334, "/run/current-system/systemd"),
            ancestor(3_577_444, "sh"),
        ];

        let used = draw_ancestry(
            &mut buffer,
            area,
            &ancestry,
            0,
            pane_background(false),
            AncestryFoot::Other,
            1,
        );

        assert_eq!(used, 2, "the marked row and its blank gap");
        assert_eq!(buffer_line(&buffer, 0), format!(" {ELISION} 3577444 sh"));
    }

    /// The table receives its rows first. A second remaining row permits
    /// the gap; a lone remaining row carries ancestry text itself.
    #[test]
    fn the_block_never_takes_more_than_half_the_cell() {
        assert_eq!(ancestry_budget(12, 6), 5);
        assert_eq!(ancestry_budget(4, 2), 1);
        assert_eq!(ancestry_budget(2, 1), 1);
        assert_eq!(ancestry_budget(0, 0), 0);
    }

    /// A cell with no row beyond the table draws no ancestry.
    #[test]
    fn a_cell_too_short_for_the_block_spends_nothing_on_it() {
        assert!(ancestry_levels(&chain(3), ancestry_budget(1, 1)).is_empty());
    }

    #[test]
    fn one_spare_row_draws_ancestry_without_a_gap() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 24, 2));
        let area = buffer.area;
        let ancestry = vec![ancestor(3334, "systemd"), ancestor(4445, "sh")];

        let used = draw_ancestry(
            &mut buffer,
            area,
            &ancestry,
            0,
            pane_background(false),
            AncestryFoot::Other,
            1,
        );

        assert_eq!(used, 1);
        assert_eq!(buffer_line(&buffer, 0), format!(" {ELISION} 4445 sh"));
    }

    #[test]
    fn one_spare_ancestry_row_does_not_have_to_be_kept() {
        let area = Rect::new(0, 0, 24, 2);
        let ancestry = vec![ancestor(3334, "systemd"), ancestor(4445, "sh")];
        let row_use = ancestry_row_use(&ancestry, area, 1);

        assert_eq!((row_use.drawn, row_use.required), (1, 0));
    }

    #[test]
    fn one_table_row_and_one_ancestry_row_report_two_painted_rows() {
        let use_rows = GroupRowUse::with_ancestry(
            &AncestryRowUse {
                drawn:    1,
                required: 0,
            },
            1,
        );

        assert_eq!((use_rows.drawn, use_rows.kept), (2, 1));
    }

    /// A command whose parents could not be read costs the table
    /// nothing, not even the blank row.
    #[test]
    fn a_command_with_no_ancestry_costs_the_table_no_rows() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 40, 10));
        let area = buffer.area;
        assert_eq!(
            draw_ancestry(
                &mut buffer,
                area,
                &[],
                0,
                pane_background(false),
                AncestryFoot::Other,
                usize::from(area.height - area.height / 2),
            ),
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

        let used = draw_ancestry(
            &mut buffer,
            area,
            &ancestry,
            0,
            pane_background(false),
            AncestryFoot::Other,
            usize::from(area.height - area.height / 2),
        );

        assert_eq!(used, 4, "three levels and the blank row under them");
        assert_eq!(buffer_line(&buffer, 0), " 6218 zed");
        assert_eq!(buffer_line(&buffer, 1), "  12445 -zsh");
        assert_eq!(buffer_line(&buffer, 2), "   18581 claude");
    }

    #[test]
    fn a_level_without_command_room_draws_its_pid_alone() {
        let ancestor = ancestor(6218, "cargo nextest run");
        let width = 13;
        let lines = ancestry_lines(
            AncestryLevel::Ancestor(&ancestor),
            0,
            width,
            Color::White,
            Color::Gray,
        );
        let text: String = lines[0]
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect();

        assert_eq!(ancestry_room(&ancestor, 0, width), 7);
        assert_eq!(
            ancestry_rows(AncestryLevel::Ancestor(&ancestor), 0, width),
            1
        );
        assert_eq!(text, " 6218");
    }

    #[test]
    fn a_cut_ancestry_level_ends_in_the_mark() {
        let area = Rect::new(0, 0, 20, 3);
        let mut buffer = Buffer::empty(area);
        let ancestry = [ancestor(
            6218,
            "cargo nextest run --workspace --all-features",
        )];

        draw_ancestry(
            &mut buffer,
            area,
            &ancestry,
            0,
            pane_background(false),
            AncestryFoot::Other,
            1,
        );

        assert!(buffer_line(&buffer, 0).ends_with(ELISION));
    }

    #[test]
    fn an_ancestry_cut_appends_or_replaces_inside_its_width() {
        for (text, width, retained) in [("short", 6, "short"), ("abcdef", 6, "abcde")] {
            let mut line = Line::from(text);
            mark_cut_line(&mut line, width);
            let marked: String = line
                .spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect();
            assert_eq!(marked, format!("{retained}{ELISION}"));
        }
    }

    #[test]
    fn every_ancestry_width_draws_each_pid_whole_or_leaves_it_out() {
        let ancestry = vec![ancestor(1_234_567, "zed"), ancestor(7_654_321, "zsh")];
        for width in 0..=90 {
            let area = Rect::new(0, 0, width, 16);
            let mut buffer = Buffer::empty(area);
            draw_ancestry(
                &mut buffer,
                area,
                &ancestry,
                0,
                pane_background(false),
                AncestryFoot::Other,
                0,
            );
            let digits: Vec<String> = (0..area.height)
                .flat_map(|y| digit_runs(&buffer_line(&buffer, y)))
                .collect();
            assert!(
                digits
                    .iter()
                    .all(|run| run == "1234567" || run == "7654321"),
                "at width {width}: {digits:?}",
            );
            for (level, pid) in ["1234567", "7654321"].into_iter().enumerate() {
                let indent_width = cell_width(SECTION_HEADER_INDENT).saturating_add(
                    cell_width(ANCESTRY_LEVEL_INDENT)
                        .saturating_mul(u16::try_from(level).unwrap_or(u16::MAX)),
                );
                assert_eq!(
                    digits.iter().any(|run| run == pid),
                    indent_width.saturating_add(cell_width(pid)) <= width,
                    "level {level} at width {width}: {digits:?}",
                );
            }
        }
    }

    /// The foot of the chain is the pid the first row's `parent` cell
    /// names, so the two have to be drawn alike: the family colour
    /// where that foot heads a family, and otherwise the same plain
    /// text an unfamilied pid takes in the table. The column headers'
    /// colour is what this used to draw, and it tied the chain to the
    /// headings instead of to the rows underneath.
    #[test]
    fn a_chain_writes_its_pids_the_way_the_table_writes_its_own() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 40, 10));
        let area = buffer.area;
        let ground = pane_background(false);
        let ancestry = vec![ancestor(6218, "zed"), ancestor(12445, "-zsh")];

        draw_ancestry(
            &mut buffer,
            area,
            &ancestry,
            0,
            ground,
            AncestryFoot::ColoredCommand(theme::family_color(0)),
            usize::from(area.height - area.height / 2),
        );

        assert_eq!(buffer[(1, 0)].fg, blend_color(text_default(), ground, 0));
        assert_ne!(buffer[(1, 0)].fg, blend_color(label_color(), ground, 0));
        assert_eq!(
            buffer[(2, 1)].fg,
            blend_color(theme::family_color(0), ground, 0),
            "the foot is drawn in the colour its parent cells point with"
        );
    }

    /// A command too wide for the cell carries on down it rather than
    /// being cut at the edge, and every line it carries on to lines up
    /// under where the command started rather than under its pid -- so
    /// the block still reads as one step per pid.
    #[test]
    fn a_command_too_wide_for_the_cell_wraps_under_where_it_started() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 30, 12));
        let area = buffer.area;
        let ancestry = vec![ancestor(6218, "claude --remote-control hana_clerestory")];

        let used = draw_ancestry(
            &mut buffer,
            area,
            &ancestry,
            0,
            pane_background(false),
            AncestryFoot::Other,
            usize::from(area.height - area.height / 2),
        );

        assert_eq!(buffer_line(&buffer, 0), " 6218 claude --remote-control");
        assert_eq!(
            buffer_line(&buffer, 1),
            "      hana_clerestory",
            "set to the column the command started at, not the pid's"
        );
        assert_eq!(used, 3, "both rows, and the blank one under them");
    }

    /// An overlong path breaks after the last punctuation boundary that
    /// fits, the same wrap a command's own row in the table below takes.
    #[test]
    fn an_overlong_ancestry_path_breaks_after_boundaries() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 20, 12));
        let area = buffer.area;
        let ancestry = vec![ancestor(6218, "node ~/.claude/local/claude")];

        let _ = draw_ancestry(
            &mut buffer,
            area,
            &ancestry,
            0,
            pane_background(false),
            AncestryFoot::Other,
            usize::from(area.height - area.height / 2),
        );

        assert_eq!(
            buffer_line(&buffer, 0),
            " 6218 node",
            "the program stays whole on the pid line"
        );
        assert_eq!(buffer_line(&buffer, 1), "      ~/.claude/");
        assert_eq!(buffer_line(&buffer, 2), "      local/claude");
    }

    /// A command wide enough to outrun the whole budget on its own is
    /// cut off at the bottom rather than taking the block away: a pid
    /// with the head of its command says more than a blank half-cell.
    #[test]
    fn a_command_that_outruns_the_budget_keeps_its_pid() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 20, 6));
        let area = buffer.area;
        let ancestry = vec![ancestor(
            6218,
            "node ~/.claude/local/claude --dangerously-skip-permissions",
        )];

        let used = draw_ancestry(
            &mut buffer,
            area,
            &ancestry,
            0,
            pane_background(false),
            AncestryFoot::Other,
            usize::from(area.height - area.height / 2),
        );

        assert_eq!(used, 3, "the two rows the budget bought, and the gap");
        assert_eq!(buffer_line(&buffer, 0), " 6218 node");
        assert_eq!(
            buffer_line(&buffer, 1),
            format!("      ~/.claude/{ELISION}")
        );
        assert_eq!(buffer_line(&buffer, 2), "", "and nothing past the budget");
    }

    /// Wrapping never costs the table the half of the cell it is owed:
    /// the block gives a level up and measures again rather than
    /// growing past the budget.
    #[test]
    fn a_wrapped_command_gives_up_a_level_rather_than_the_budget() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 24, 12));
        let area = buffer.area;
        let ancestry = vec![
            ancestor(1, "zed"),
            ancestor(2, "claude --remote-control hana_clerestory_recovery"),
            ancestor(3, "cargo nextest run"),
        ];

        let used = draw_ancestry(
            &mut buffer,
            area,
            &ancestry,
            0,
            pane_background(false),
            AncestryFoot::Other,
            usize::from(area.height - area.height / 2),
        );

        assert!(
            used <= 6,
            "the block and its gap stay inside half the cell: {used}"
        );
    }

    /// A cargo process for a group the roster is to carry.
    fn invocation(pid: u32, arguments: &[&str]) -> CargoProcess {
        CargoProcess {
            path: "~/rust/cargo-liner".to_string(),
            directory_identity: WorkingDirectoryIdentity::Absolute(
                "/test-home/rust/cargo-liner".into(),
            ),
            pid,
            invocation_id: InvocationId::for_test(pid),
            capture_membership: CaptureMembership::Outside,
            provenance: RowProvenance::Uncaptured(ProcessOwner::Unavailable),
            parent: VisibleParent::None,
            start: "11:04".to_string(),
            started: RunStart::Known(0),
            duration: "00:18".to_string(),
            cpu: Measurement::Reading("12%".to_string()),
            subtree_cpu: Measurement::Reading("12%".to_string()),
            memory: Measurement::Unavailable(MeasurementAbsence::Unproven),
            subtree_memory: Measurement::Unavailable(MeasurementAbsence::Unproven),
            compiler: CompilerObservation::None,
            state: CaptureLookup::Unregistered,
            managed: Measurement::Reading(0),
            nested: false,
            command: CommandText::of("cargo", arguments),
        }
    }

    /// A process with the group-total memory used by summary-foot tests.
    fn memory_invocation(pid: u32, memory: Measurement<u64>) -> CargoProcess {
        let mut process = invocation(pid, &["build"]);
        process.memory = memory;
        process
    }

    /// One group with `lead` and no child invocations.
    fn cargo_group(lead: CargoProcess) -> CargoGroup {
        CargoGroup {
            lead,
            rest: Vec::new(),
            ancestry: Vec::new(),
        }
    }

    /// A roster whose running groups carry the supplied memory states.
    fn memory_roster(memories: &[(u32, Measurement<u64>)]) -> Roster {
        let groups = memories
            .iter()
            .map(|&(pid, memory)| cargo_group(memory_invocation(pid, memory)))
            .collect();
        let mut roster = Roster::new();
        roster.observe(groups, Instant::now());
        roster
    }

    /// `commands.hidden_when_idle` as the config hands it over.
    fn hidden_when_idle() -> Vec<String> { vec![PORT_SUBCOMMAND_NAME.to_string()] }

    /// A roster carrying one command, with `rest` running under it.
    fn roster_of(lead: CargoProcess, rest: Vec<CargoProcess>) -> Roster {
        roster_with_ancestry(
            lead,
            rest,
            vec![ancestor(6218, "zed"), shell(36744, "-zsh")],
        )
    }

    #[test]
    fn an_idle_driver_keeps_one_summary_row() {
        let roster = roster_of(invocation(4100, &[PORT_SUBCOMMAND_NAME]), Vec::new());

        let pids = summary_cpu_for_test(&roster, &hidden_when_idle())
            .into_iter()
            .map(|(pid, _)| pid)
            .collect::<Vec<_>>();

        assert_eq!(pids, vec![4100]);
    }

    #[test]
    fn an_active_driver_summarizes_its_direct_work() {
        let mut nested = invocation(4300, &["check"]);
        nested.nested = true;
        let roster = roster_of(
            invocation(4100, &[PORT_SUBCOMMAND_NAME]),
            vec![invocation(4200, &["build"]), nested],
        );

        let pids = summary_cpu_for_test(&roster, &hidden_when_idle())
            .into_iter()
            .map(|(pid, _)| pid)
            .collect::<Vec<_>>();

        assert_eq!(pids, vec![4200]);
    }

    #[test]
    fn a_promoted_summary_row_draws_its_subtree_memory() {
        let mut work = invocation(4200, &["build"]);
        work.memory = Measurement::Reading(BYTES_PER_GIBIBYTE);
        work.subtree_memory = Measurement::Reading(BYTES_PER_GIBIBYTE * 1234 / 100);
        let lead = invocation(4100, &[PORT_SUBCOMMAND_NAME]);

        let buffer = measurement_group_buffer(lead, vec![work], TableKind::Summary);

        assert_eq!(
            memory_cell_text(&buffer, TABLE_HEADER_HEIGHT + GROUP_HEADER_HEIGHT),
            "12.3G"
        );
    }

    /// Aggregate member samples before the roster carries them to either layout.
    fn group_cpu_buffer(samples: [Measurement<f32>; 3], kind: TableKind) -> Buffer {
        let members = [
            Pid::from_u32(4100),
            Pid::from_u32(4200),
            Pid::from_u32(4300),
        ];
        let shares: HashMap<_, _> = members.into_iter().zip(samples).collect();
        let mut lead = invocation(4100, &["test"]);
        lead.cpu =
            census::scan::aggregate(&shares, members.into_iter()).map(census::scan::cpu_label);
        let rest = members[1..]
            .iter()
            .map(|pid| {
                let mut child = invocation(pid.as_u32(), &["build"]);
                child.cpu = shares[pid].map(census::scan::cpu_label);
                child
            })
            .collect();
        measurement_group_buffer(lead, rest, kind)
    }

    /// An unknown member at either end or in the middle prevents a partial total.
    #[test]
    fn an_unavailable_group_contributor_never_renders_a_partial_total() {
        for reason in [
            MeasurementAbsence::FirstObservation,
            MeasurementAbsence::ReadFailed,
            MeasurementAbsence::Unproven,
        ] {
            let readings = [
                Measurement::Reading(12.0),
                Measurement::Reading(18.0),
                Measurement::Reading(7.0),
            ];
            for unknown in 0..readings.len() {
                let mut samples = readings;
                samples[unknown] = Measurement::Unavailable(reason);

                for kind in [TableKind::Command, TableKind::Summary] {
                    let buffer = group_cpu_buffer(samples, kind);

                    assert_eq!(
                        cpu_cell_text(&buffer, TABLE_HEADER_HEIGHT + GROUP_HEADER_HEIGHT),
                        UNAVAILABLE_MEASUREMENT,
                        "{reason:?} at member {unknown} in {kind:?}"
                    );
                }
            }
        }
    }

    /// A known zero contributes normally and does not hide the other members' total.
    #[test]
    fn a_fully_measured_group_renders_its_total_including_zero() {
        for (samples, expected) in [([12.0, 18.0, 0.0], "30%"), ([0.0, 0.0, 0.0], "0%")] {
            for kind in [TableKind::Command, TableKind::Summary] {
                let buffer = group_cpu_buffer(samples.map(Measurement::Reading), kind);

                assert_eq!(
                    cpu_cell_text(&buffer, TABLE_HEADER_HEIGHT + GROUP_HEADER_HEIGHT),
                    expected,
                    "{samples:?} in {kind:?}"
                );
            }
        }
    }

    /// The same, for the tests that care what stands above the command.
    fn roster_with_ancestry(
        lead: CargoProcess,
        rest: Vec<CargoProcess>,
        ancestry: Vec<Ancestor>,
    ) -> Roster {
        let mut roster = Roster::new();
        roster.observe(
            vec![CargoGroup {
                lead,
                rest,
                ancestry,
            }],
            Instant::now(),
        );
        roster
    }

    /// Rows of `buffer` the draw put something in, counted from the top:
    /// the last row carrying anything, and every row above it.
    fn filled_rows(buffer: &Buffer) -> usize {
        (0..buffer.area.height)
            .rev()
            .find(|&y| !buffer_line(buffer, y).is_empty())
            .map_or(0, |y| usize::from(y).saturating_add(1))
    }

    /// A chain no cell of a readable width can draw whole: three steps
    /// whose command lines each wrap several times over.
    fn long_chain() -> Vec<Ancestor> {
        vec![
            ancestor(6218, "zed"),
            ancestor(
                18581,
                &format!("node ~/.claude/local/claude {}", "--setting on ".repeat(8)),
            ),
            ancestor(
                24101,
                &format!(
                    "zsh -c cargo nextest run {}",
                    "--package tui_pane ".repeat(8)
                ),
            ),
        ]
    }

    /// The rows a cell asks for are the rows it goes on to draw.
    ///
    /// The chain here is far too long for the half-cell the block is
    /// given, so [`ancestry_fit`] gives up whole levels -- and a demand
    /// counting the chain instead asked for every row of every level
    /// the cell was always going to elide, then sat in a cell sized for
    /// the larger number with the difference blank.
    #[test]
    fn a_cell_asks_for_the_rows_its_chain_actually_draws() {
        let width = 60;
        let hidden = hidden_when_idle();
        let roster = roster_with_ancestry(
            invocation(4100, &["build"]),
            vec![invocation(4212, &["test"])],
            long_chain(),
        );
        let group = roster.groups().first().unwrap();
        let demand = group_height(group, width, &hidden, ProcessTree::Long);

        let mut buffer = Buffer::empty(Rect::new(0, 0, width, u16::try_from(demand).unwrap()));
        let area = buffer.area;
        draw_group(
            &mut buffer,
            &roster,
            &InvocationId::for_test(4100),
            area,
            Color::Reset,
            &hidden,
            ProcessTree::Long,
        );

        assert_eq!(
            filled_rows(&buffer),
            demand,
            "a cell should be sized for the rows it draws, not for rows it elides",
        );
    }

    /// And the same where nothing is elided: a chain the cell has room
    /// for is asked for whole, so the fixed point is not simply the
    /// smallest answer it can reach.
    #[test]
    fn a_cell_whose_chain_fits_asks_for_all_of_it() {
        let width = 60;
        let hidden = hidden_when_idle();
        let roster = roster_of(
            invocation(4100, &["build"]),
            vec![invocation(4212, &["test"])],
        );
        let group = roster.groups().first().unwrap();
        let demand = group_height(group, width, &hidden, ProcessTree::Long);

        let mut buffer = Buffer::empty(Rect::new(0, 0, width, u16::try_from(demand).unwrap()));
        let area = buffer.area;
        draw_group(
            &mut buffer,
            &roster,
            &InvocationId::for_test(4100),
            area,
            Color::Reset,
            &hidden,
            ProcessTree::Long,
        );

        assert_eq!(filled_rows(&buffer), demand);
        assert_eq!(
            buffer_line(&buffer, 0),
            " 6218 zed",
            "the whole chain should be drawn",
        );
    }

    /// The ask is the chain itself, so a cell granted it has the room to
    /// draw every level -- which is what makes the count the readout
    /// writes out the count that goes on screen.
    #[test]
    fn a_cell_granted_its_ask_draws_the_whole_chain() {
        let width = 60;
        let ancestry = long_chain();
        let whole: Vec<AncestryLevel<'_>> = ancestry.iter().map(AncestryLevel::Ancestor).collect();
        let table = 6;
        let gap = usize::from(ANCESTRY_GAP_HEIGHT);

        let asked = ancestry_demand(&ancestry, width);

        assert_eq!(
            asked,
            ancestry_height(&whole, width).saturating_add(gap),
            "the ask is the whole chain plus the blank row under it",
        );

        // The cell the grid would build from that ask, and the room the
        // block is left once the table has taken its rows.
        let height = u16::try_from(asked + table).expect("a test cell should fit a u16");
        let budget = ancestry_budget(height, table);
        let levels = ancestry_fit(&ancestry, budget, |levels| ancestry_height(levels, width));

        assert_eq!(
            levels.len(),
            ancestry.len(),
            "a cell sized to the ask elides no level",
        );
        assert_eq!(
            ancestry_height(&levels, width).saturating_add(gap),
            asked,
            "and draws exactly the rows it asked for",
        );
    }

    /// A budget far past the chain's length measures no more candidate
    /// blocks than the chain has levels.
    ///
    /// Every measurement wraps each level's command line again. Counted
    /// down from the budget, a cell hundreds of rows tall over a command
    /// line of tens of kilobytes measured the same whole chain once per
    /// spare row, every frame took seconds, and a key that changed the
    /// view showed only as each of those frames finished.
    #[test]
    fn a_tall_budget_measures_at_most_one_block_per_level() {
        let width = 60;
        let ancestry = vec![
            ancestor(6218, "zed"),
            ancestor(18581, &format!("python3 -c {}", "script ".repeat(2000))),
            ancestor(24101, "zsh -c cargo check"),
        ];
        let whole: Vec<AncestryLevel<'_>> = ancestry.iter().map(AncestryLevel::Ancestor).collect();
        let budget = 150;
        assert!(
            ancestry_height(&whole, width) > budget,
            "the chain should outrun the budget for the fit to give levels up",
        );

        let mut measured = 0;
        let levels = ancestry_fit(&ancestry, budget, |levels| {
            measured += 1;
            ancestry_height(levels, width)
        });

        assert_eq!(
            drawn(&levels),
            vec![Some(6218), None, Some(24101)],
            "the long middle step is replaced by its cut mark",
        );
        assert!(
            measured <= ancestry.len(),
            "measured {measured} candidate blocks for a chain of {} levels",
            ancestry.len(),
        );
    }

    /// A discarded wrapping level cannot consume the budget invisibly:
    /// the row it frees becomes the cut mark above the nearest ancestor.
    #[test]
    fn a_deep_ancestry_uses_its_row_budget_and_marks_the_cut() {
        let mut ancestry = chain(24);
        ancestry[22].command = format!("python3 {}", "nested.py ".repeat(12));
        ancestry[23].command = "uv run".to_string();
        let mut buffer = Buffer::empty(Rect::new(0, 0, 32, 4));
        let area = buffer.area;

        let used = draw_ancestry(
            &mut buffer,
            area,
            &ancestry,
            0,
            pane_background(false),
            AncestryFoot::PlainCommand,
            1,
        );

        assert_eq!(used, 3, "both ancestry rows and their blank gap");
        assert_eq!(buffer_line(&buffer, 0), format!(" {ELISION}"));
        assert_eq!(buffer_line(&buffer, 1), "  23 uv run");
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
            invocation(4100, &[PORT_SUBCOMMAND_NAME]),
            vec![invocation(4212, &["build"])],
        );

        draw_group(
            &mut buffer,
            &roster,
            &InvocationId::for_test(4100),
            area,
            Color::Reset,
            &hidden_when_idle(),
            ProcessTree::Long,
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

    /// A short tree draws every step the long one does. What it takes
    /// off is the arguments: a step reached as `zsh -c cargo nextest
    /// run --package tui_pane ...` is a shell however it was called, and
    /// the eight words saying which package are the run's business, not
    /// the chain's.
    #[test]
    fn a_short_tree_keeps_the_chain_and_takes_its_arguments_off() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 60, 14));
        let area = buffer.area;
        let roster = roster_with_ancestry(invocation(4100, &["build"]), Vec::new(), long_chain());

        draw_group(
            &mut buffer,
            &roster,
            &InvocationId::for_test(4100),
            area,
            Color::Reset,
            &hidden_when_idle(),
            ProcessTree::Short,
        );

        let drawn: Vec<String> = (0..area.height).map(|y| buffer_line(&buffer, y)).collect();
        let drawn = drawn.join("\n");
        for pid in ["6218", "18581", "24101"] {
            assert!(
                drawn.contains(pid),
                "step {pid} should still be drawn: {drawn}"
            );
        }
        assert!(
            !drawn.contains("--setting") && !drawn.contains("--package"),
            "and none of them should carry arguments: {drawn}",
        );
    }

    #[test]
    fn a_short_ancestry_entry_marks_its_dropped_wrapped_arguments() {
        let area = Rect::new(0, 0, 40, 6);
        let mut buffer = Buffer::empty(area);
        let ancestry = vec![ancestor(
            596_633,
            "sh /tmp/mac-test-phase8/companion.sh check check",
        )];
        let roster = roster_with_ancestry(invocation(4100, &["check"]), Vec::new(), ancestry);

        draw_group(
            &mut buffer,
            &roster,
            &InvocationId::for_test(4100),
            area,
            Color::Reset,
            &hidden_when_idle(),
            ProcessTree::Short,
        );

        assert_eq!(buffer_line(&buffer, 0), format!(" 596633 sh {ELISION}"));
    }

    /// A row names what runs and stops there. `cargo build` is the
    /// command; the flags saying which features and how many targets are
    /// what the short setting is for, and they are what a cell spends
    /// three wrapped rows on when the column is narrow.
    #[test]
    fn a_short_row_names_the_command_without_its_arguments() {
        // Wide enough that the command column holds the whole name: the
        // point here is what the row says, not where it wraps.
        let mut buffer = Buffer::empty(Rect::new(0, 0, 90, 14));
        let area = buffer.area;
        let roster = roster_of(
            invocation(
                4100,
                &["build", "--features", "one,two,three", "--all-targets"],
            ),
            Vec::new(),
        );

        draw_group(
            &mut buffer,
            &roster,
            &InvocationId::for_test(4100),
            area,
            Color::Reset,
            &hidden_when_idle(),
            ProcessTree::Short,
        );

        let drawn: Vec<String> = (0..area.height).map(|y| buffer_line(&buffer, y)).collect();
        let drawn = drawn.join("\n");
        assert!(drawn.contains("cargo build"), "{drawn}");
        assert!(
            !drawn.contains("--features") && !drawn.contains("--all-targets"),
            "the arguments should be off the row: {drawn}",
        );
    }

    /// A cell asks for fewer rows with the tree short than with it long.
    /// The demand and the draw are measured by different code, so a
    /// setting that shortened what reached the screen without shortening
    /// what was asked for would leave the cell sized for command lines
    /// it no longer draws -- which is the defect the demand was rewritten
    /// to fix, arriving again through a key.
    #[test]
    fn a_short_tree_asks_for_fewer_rows_than_a_long_one() {
        let width = 60;
        let hidden = hidden_when_idle();
        let roster = roster_with_ancestry(
            invocation(4100, &["build"]),
            vec![invocation(
                4212,
                &["test", "--features", "one,two,three", "--all-targets"],
            )],
            long_chain(),
        );
        let group = roster.groups().first().unwrap();

        let short = group_height(group, width, &hidden, ProcessTree::Short);
        let long = group_height(group, width, &hidden, ProcessTree::Long);

        assert!(
            short < long,
            "a cell drawing named commands asked for {short} rows against the {long} whole lines \
             cost",
        );
        let mut buffer = Buffer::empty(Rect::new(0, 0, width, u16::try_from(short).unwrap()));
        let area = buffer.area;
        draw_group(
            &mut buffer,
            &roster,
            &InvocationId::for_test(4100),
            area,
            Color::Reset,
            &hidden,
            ProcessTree::Short,
        );
        assert_eq!(
            filled_rows(&buffer),
            short,
            "and it should fill the cell that ask bought",
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
            &InvocationId::for_test(4100),
            area,
            Color::Reset,
            &hidden_when_idle(),
            ProcessTree::Long,
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

        let pinned = group_by_path(
            &rows,
            PinnedGroup::Lead(GroupingIdentity::from(&lead.process)),
        );

        assert_eq!(
            pinned.iter().map(|group| group.path).collect::<Vec<&str>>(),
            [
                "~/rust/cargo-berth-init",
                "/private/var/folders/T/case-1",
                "/private/var/folders/T/case-2"
            ]
        );
    }

    /// A real directory supplies an incarnation without requiring a second account.
    fn capture_context() -> CaptureContext {
        let root = tempfile::tempdir().expect("capture root");
        let scan = crate::root_scan::RootScan::open(
            root.path(),
            &mut crate::root_scan::RootHistory::default(),
        )
        .expect("open capture root");
        CaptureContext {
            root:        CaptureRootIndex(0),
            incarnation: scan.incarnation(),
            account:     Account {
                uid:  1000,
                name: AccountName::Resolved("runner-one".into()),
            },
        }
    }

    /// Exercise the shared production heading renderer with enough room for every group.
    fn grouped_table_text(
        rows: &[&TrackedRow],
        kind: TableKind,
        pinned: PinnedGroup<'_>,
    ) -> String {
        let area = Rect::new(0, 0, 140, 16);
        let mut buffer = Buffer::empty(area);
        draw_process_table(
            &mut buffer,
            area,
            rows,
            kind,
            Color::Reset,
            pinned,
            ProcessTree::Long,
        );
        (0..area.height)
            .map(|y| buffer_line(&buffer, y))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Each component must distinguish groups while the other two remain equal.
    #[test]
    fn account_root_and_raw_directory_independently_separate_headings() {
        let context = capture_context();
        let mut first = same_second("/workspace/project", 40);
        first.process.command = CommandText::of("cargo", &["build", "first-project"]);
        first.process.provenance = RowProvenance::Direct(context.clone());
        let mut other_account = same_second("/workspace/project", 41);
        let mut account_context = context.clone();
        account_context.account.uid += 1;
        other_account.process.provenance = RowProvenance::Direct(account_context);
        let mut other_root = same_second("/workspace/project", 42);
        let mut root_context = context.clone();
        root_context.root = CaptureRootIndex(1);
        other_root.process.provenance = RowProvenance::Direct(root_context);
        let mut other_directory = same_second("/workspace/other", 43);
        other_directory.process.provenance = RowProvenance::Direct(context);

        for other in [&mut other_account, &mut other_root, &mut other_directory] {
            other.process.command = CommandText::of("cargo", &["test", "second-project"]);
            let rows = [&first, other];
            assert_eq!(group_by_path(&rows, PinnedGroup::Unpinned).len(), 2);
            for kind in [TableKind::Command, TableKind::Summary] {
                let text = grouped_table_text(&rows, kind, PinnedGroup::Unpinned);
                assert_eq!(text.matches("[runner-one]").count(), 2, "{kind:?}: {text}");
                for command in ["cargo build first-project", "cargo test second-project"] {
                    assert_eq!(text.matches(command).count(), 1, "{kind:?}: {text}");
                }
                for pid in [first.process.pid, other.process.pid] {
                    assert_eq!(
                        text.split_whitespace()
                            .filter(|word| *word == pid.to_string())
                            .count(),
                        1,
                        "{kind:?}: {text}"
                    );
                }
                let expected_project_headings = if other.process.path == "/workspace/other" {
                    1
                } else {
                    2
                };
                assert_eq!(
                    text.lines()
                        .filter(|line| line.trim() == "[runner-one] /workspace/project")
                        .count(),
                    expected_project_headings,
                    "{kind:?}: {text}"
                );
                assert_eq!(
                    text.lines()
                        .filter(|line| line.trim() == "[runner-one] /workspace/other")
                        .count(),
                    2 - expected_project_headings,
                    "{kind:?}: {text}"
                );
            }
        }
    }

    /// Display resolution and direct representation are independent from membership.
    #[test]
    fn account_name_and_enclosing_membership_do_not_split_a_group() {
        let context = capture_context();
        let mut direct = same_second("/workspace/project", 40);
        direct.process.provenance = RowProvenance::Direct(context.clone());
        let mut enclosing = same_second("/workspace/project", 41);
        let mut unresolved = context;
        unresolved.account.name = AccountName::Unavailable;
        enclosing.process.provenance = RowProvenance::Enclosing(unresolved);
        let rows = [&direct, &enclosing];
        let groups = group_by_path(&rows, PinnedGroup::Unpinned);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].rows.len(), 2);
        assert_eq!(groups[0].heading(), "[runner-one] /workspace/project");
    }

    /// Prefixes distinguish identical shortened headings in both production views.
    #[test]
    fn two_roots_render_their_own_account_prefixes_on_identical_labels() {
        let context = capture_context();
        let mut first = same_second("~/x", 40);
        first.process.provenance = RowProvenance::Direct(context.clone());
        first.process.directory_identity = WorkingDirectoryIdentity::Absolute("/one/x".into());
        let mut second = same_second("~/x", 41);
        let mut second_context = context;
        second_context.root = CaptureRootIndex(1);
        second_context.account.uid = 2000;
        second_context.account.name = AccountName::Resolved("runner-two".into());
        second.process.provenance = RowProvenance::Direct(second_context);
        second.process.directory_identity = WorkingDirectoryIdentity::Absolute("/two/x".into());
        for kind in [TableKind::Command, TableKind::Summary] {
            let text = grouped_table_text(&[&first, &second], kind, PinnedGroup::Unpinned);
            assert!(text.contains("[runner-one] ~/x"), "{text}");
            assert!(text.contains("[runner-two] ~/x"), "{text}");
        }
    }

    /// Discovery rejects a false account claim before its metadata reaches the summary.
    #[test]
    fn summary_rejects_foreign_owned_capture_attribution() {
        let parent = tempfile::tempdir().expect("shared capture parent");
        let uid = fs::metadata(parent.path()).expect("fixture owner").uid();
        let foreign_uid = uid.checked_add(1).expect("another account uid");
        let capture = summary_capture_accounts(parent.path(), uid, foreign_uid);
        let first_argv = ["cargo", "build", "first-writer"].map(OsString::from);
        let second_argv = ["cargo", "build", "second-writer"].map(OsString::from);
        let records = ProcessObservations::new(
            [(41, 40, &first_argv), (51, 50, &second_argv)].map(|(pid, shim_pid, argv)| {
                let mut process = ProcessObservation::cargo(pid, argv);
                process.parent = ProcessField::Observed(Pid::from_u32(shim_pid));
                process.directory = ProcessField::Observed(Path::new("/writer/project"));
                process.uid = ProcessField::Observed(uid);
                process
            }),
        );
        let mut groups = CensusSequence::default().sample_capture(&records, &capture);
        // The reader scenario launches the first writer before the second writer.
        // Give both render inputs the same clock instead of the fixture's process epoch
        // and registration discovery time, which describe unrelated instants.
        for group in &mut groups {
            group.lead.started = RunStart::Known(u64::from(group.lead.pid));
        }
        assert_eq!(groups.len(), 2);
        let own_row = &groups
            .iter()
            .find(|group| group.lead.pid == 41)
            .expect("own writer")
            .lead;
        let RowProvenance::Direct(context) = &own_row.provenance else {
            panic!("own writer has verified capture attribution");
        };
        assert_eq!(context.account.uid, uid);
        let account = match &context.account.name {
            AccountName::Resolved(name) => name.clone(),
            AccountName::Unavailable => uid.to_string(),
        };
        let heading = format!("[{account}] {}", own_row.path);
        let other = &groups
            .iter()
            .find(|group| group.lead.pid == 51)
            .expect("foreign writer process")
            .lead;
        // The rejected root supplies nothing; the process owner still names the
        // accepted account, so both writers share one heading.
        assert_eq!(
            other.provenance,
            RowProvenance::Uncaptured(ProcessOwner::Observed(context.account.clone()))
        );
        let mut roster = Roster::new();
        roster.observe(groups, Instant::now());
        let area = Rect::new(0, 0, 180, 16);
        let mut buffer = Buffer::empty(area);
        draw_cell_for_test(&mut buffer, &roster, &TileContent::Summary, area, 4);
        let text = (0..area.height)
            .map(|y| buffer_line(&buffer, y))
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(text.matches(&heading).count(), 1, "{text}");
        assert!(!text.contains(&format!("[{foreign_uid}]")), "{text}");
        for marker in ["first-writer", "second-writer"] {
            assert_eq!(text.matches(marker).count(), 1, "{text}");
        }
        let first = text
            .lines()
            .position(|line| line.contains("first-writer"))
            .expect("first summary writer");
        let second = text
            .lines()
            .position(|line| line.contains("second-writer"))
            .expect("second summary writer");
        assert!(first < second, "{text}");
        assert!(second < usize::from(area.height - 1), "{text}");
        for (account, pid) in [(uid, 40), (foreign_uid, 50)] {
            let root = parent.path().join(account.to_string());
            assert!(root.join(format!("state/pids/{pid}.generation")).exists());
            assert!(root.join("run.log").exists());
        }
    }

    /// Publish one accepted account and one directory owned by the wrong account.
    fn summary_capture_accounts(parent: &Path, uid: u32, foreign_uid: u32) -> Capture {
        let own = parent.join(uid.to_string());
        let foreign = parent.join(foreign_uid.to_string());
        for (directory, pid, marker) in
            [(&own, 40, "first-writer"), (&foreign, 50, "second-writer")]
        {
            fs::create_dir_all(directory.join("state/pids")).expect("publication directory");
            let magic = std::str::from_utf8(REGISTRATION_MAGIC).expect("registration magic");
            let bytes = format!(
                "{magic}\0generation\0boot\x00100\0run.log\0/writer/project\0/writer\x002\0build\0{marker}\0"
            );
            fs::write(
                directory.join(format!("state/pids/{pid}.generation")),
                bytes,
            )
            .expect("publish writer");
            fs::write(directory.join("run.log"), format!("{LOCK_WAIT_MARKER}\n"))
                .expect("writer progress");
        }
        let IdentityEvidence::Available(birth) = BirthStamp::from_fields("boot", "100") else {
            panic!("fixture birth stamp");
        };
        let capture = Capture::take_roots(&CaptureRoots::from_parent(parent), &|pid| {
            KernelObservation::for_test(pid, Observation::Present(birth.clone()))
        });
        assert_eq!(capture.confirmed().len(), 1);
        let rejected = capture
            .root_status
            .iter()
            .find(|status| status.root.uid == foreign_uid)
            .expect("foreign-owned account diagnostic");
        assert!(matches!(
            rejected.state,
            RootReadStatus::ForeignOwned { .. }
        ));
        capture
    }

    /// Missing passwd entries display the same numeric identity that settings reports.
    #[test]
    fn unresolved_account_names_render_numbers_and_keep_owners_separate() {
        let mut context = capture_context();
        context.account.name = AccountName::Unavailable;
        let mut first = same_second("/workspace/project", 40);
        first.process.provenance = RowProvenance::Direct(context.clone());
        let mut second = same_second("/workspace/project", 41);
        context.account.uid = 2000;
        second.process.provenance = RowProvenance::Direct(context);
        let rows = [&first, &second];
        assert_eq!(group_by_path(&rows, PinnedGroup::Unpinned).len(), 2);
        for kind in [TableKind::Command, TableKind::Summary] {
            let text = grouped_table_text(&rows, kind, PinnedGroup::Unpinned);
            assert!(text.contains("[1000] /workspace/project"), "{text}");
            assert!(text.contains("[2000] /workspace/project"), "{text}");
            assert!(!text.contains("[]"), "{text}");
        }
    }

    /// The old root stays allocated while a replacement acquires a new incarnation.
    #[test]
    fn retained_rows_under_a_replaced_root_keep_separate_groups_and_pins() {
        let parent = tempfile::tempdir().expect("root parent");
        let path = parent.path().join("capture");
        std::fs::create_dir(&path).expect("first root");
        let mut history = crate::root_scan::RootHistory::default();
        let first_scan =
            crate::root_scan::RootScan::open(&path, &mut history).expect("scan first root");
        let mut context = capture_context();
        context.incarnation = first_scan.incarnation();
        let mut retained = same_second("~/x", 40);
        retained.process.provenance = RowProvenance::Direct(context.clone());
        std::fs::rename(&path, parent.path().join("previous")).expect("retain previous root");
        std::fs::create_dir(&path).expect("replacement root");
        let replacement_scan =
            crate::root_scan::RootScan::open(&path, &mut history).expect("scan replacement root");
        context.incarnation = replacement_scan.incarnation();
        let mut current = same_second("~/x", 41);
        current.process.provenance = RowProvenance::Direct(context);
        let rows = [&retained, &current];
        let groups = group_by_path(
            &rows,
            PinnedGroup::Lead(GroupingIdentity::from(&current.process)),
        );
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].rows[0].process.pid, current.process.pid);
        assert_eq!(groups[1].rows[0].process.pid, retained.process.pid);
    }

    /// A changed process pid and newly available measurements cannot repin a label collision.
    #[test]
    fn source_transition_preserves_the_intended_lead_among_identical_headings() {
        let context = capture_context();
        let mut registration = same_second("~/x", 40);
        let IdentityEvidence::Available(birth) =
            crate::birth_stamp::BirthStamp::from_fields("test-boot", "40")
        else {
            panic!("valid fixture birth stamp");
        };
        registration.process.invocation_id = InvocationId::Captured(RunId {
            root: context.root,
            incarnation: context.incarnation,
            shim_pid: 40,
            generation: "source-switch".into(),
            birth,
        });
        registration.process.provenance = RowProvenance::Direct(context.clone());
        registration.process.cpu = Measurement::Unavailable(MeasurementAbsence::Unproven);
        registration.process.compiler = CompilerObservation::Unknown;
        registration.process.managed = Measurement::Unavailable(MeasurementAbsence::Unproven);
        let mut competing = same_second("~/x", 30);
        let mut competing_context = context.clone();
        competing_context.root = CaptureRootIndex(1);
        competing.process.provenance = RowProvenance::Direct(competing_context);
        let mut other_directory = same_second("~/x", 31);
        other_directory.process.provenance = RowProvenance::Direct(context);
        other_directory.process.directory_identity =
            WorkingDirectoryIdentity::Absolute("/other/x".into());
        let mut process = TrackedRow::from(registration.process.clone());
        process.process.pid = 50;
        process.process.cpu = Measurement::Reading("12%".into());
        process.process.compiler = CompilerObservation::None;
        process.process.managed = Measurement::Reading(0);
        let pinned = PinnedGroup::Lead(GroupingIdentity::from(&registration.process));
        assert_eq!(
            GroupingIdentity::from(&registration.process),
            GroupingIdentity::from(&process.process)
        );
        for lead in [&registration, &process] {
            let rows = [&competing, &other_directory, lead];
            let groups = group_by_path(&rows, pinned);
            assert_eq!(groups.len(), 3);
            assert_eq!(groups[0].rows[0].process.pid, lead.process.pid);
            assert_eq!(groups[0].heading(), groups[1].heading());
        }
    }

    /// Unknown timestamps sort after observed starts and still retain a duration cell.
    #[test]
    fn unavailable_start_sorts_deterministically_and_renders_its_duration() {
        let mut unavailable = same_second("/unknown/start", 40);
        unavailable.process.started = RunStart::Unavailable;
        unavailable.process.start = UNRESOLVED_TIME.into();
        unavailable.process.duration = UNRESOLVED_TIME.into();
        let known = started_at("/known/start", None, 100);
        let rows = [&unavailable, &known];
        let groups = group_by_path(&rows, PinnedGroup::Unpinned);
        assert_eq!(groups[0].path, "/known/start");
        assert_eq!(groups[1].path, "/unknown/start");
        for kind in [TableKind::Command, TableKind::Summary] {
            let buffer = measurement_row_buffer(&unavailable, kind);
            let header = buffer_line(&buffer, 0);
            let text = buffer_line(&buffer, TABLE_HEADER_HEIGHT + GROUP_HEADER_HEIGHT);
            let start = header
                .find(TABLE_HEADERS[DURATION_COLUMN])
                .expect("duration column");
            let end = header.find(TABLE_HEADERS[CPU_COLUMN]).expect("CPU column");
            assert_eq!(text[start..end].trim(), UNRESOLVED_TIME);
        }
    }

    /// The renderer preserves the absolute label supplied for a different writer home.
    #[test]
    fn qualified_absolute_heading_is_preserved_in_command_and_summary_cells() {
        let mut row = row_at("/writer-home/project", None);
        row.process.provenance = RowProvenance::Direct(capture_context());
        for kind in [TableKind::Command, TableKind::Summary] {
            let buffer = measurement_row_buffer(&row, kind);
            let heading = buffer_line(&buffer, TABLE_HEADER_HEIGHT);
            assert_eq!(heading.trim(), "[runner-one] /writer-home/project");
        }
    }

    #[test]
    fn a_directory_heading_keeps_the_most_informative_marked_tail() {
        let mut row = row_at("/very-long-parent/component", None);
        row.process.provenance = RowProvenance::Direct(capture_context());
        let rows = [&row];
        let groups = group_by_path(&rows, PinnedGroup::Unpinned);
        let group = &groups[0];
        let whole = "[runner-one] /very-long-parent/component";

        assert_eq!(group.fitted_heading(cell_width(whole)), whole);
        assert_eq!(
            group.fitted_heading(24),
            format!("[runner-one] {ELISION}/component")
        );
        assert_eq!(group.fitted_heading(22), format!("{ELISION}/component"));
        assert_eq!(group.fitted_heading(11), format!("{ELISION}/component"));
        assert_eq!(group.fitted_heading(9), format!("{ELISION}omponent"));
        assert_eq!(group.fitted_heading(8), "");

        let mut row = row_at("/very-long-parent/tool-based-ui-frame-time", None);
        row.process.provenance = RowProvenance::Direct(capture_context());
        let rows = [&row];
        let groups = group_by_path(&rows, PinnedGroup::Unpinned);
        assert_eq!(
            groups[0].fitted_heading(25),
            format!("[runner-one] {ELISION}-frame-time")
        );

        let row = row_at("/very-long-parent/tool-based-ui-frame-time", None);
        let rows = [&row];
        let groups = group_by_path(&rows, PinnedGroup::Unpinned);
        assert_eq!(
            groups[0].fitted_heading(25),
            format!("{ELISION}tool-based-ui-frame-time")
        );
    }

    /// Enclosing capture qualifies only the heading; nested fields remain the row's own.
    #[test]
    fn enclosing_and_uncaptured_rows_keep_their_own_directory_and_command() {
        let context = capture_context();
        let mut enclosing = same_second("/nested/check", 41);
        enclosing.process.provenance = RowProvenance::Enclosing(context);
        enclosing.process.command = CommandText::of("cargo", &["check"]);
        let uncaptured = same_second("/nested/check", 42);
        let rows = [&enclosing, &uncaptured];
        let groups = group_by_path(&rows, PinnedGroup::Unpinned);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].heading(), "[runner-one] /nested/check");
        assert_eq!(groups[1].heading(), "/nested/check");
        assert_eq!(
            groups[1].identity.qualification,
            GroupQualification::Unqualified
        );
        for kind in [TableKind::Command, TableKind::Summary] {
            let text = grouped_table_text(&rows, kind, PinnedGroup::Unpinned);
            assert!(text.contains("cargo check"), "{text}");
            assert!(text.contains("cargo build"), "{text}");
        }
    }

    /// An uncaptured row owned by `uid`, whose owner name resolved to `name`.
    fn owned(path: &str, pid: u32, uid: u32, name: AccountName) -> TrackedRow {
        let mut row = same_second(path, pid);
        row.process.provenance =
            RowProvenance::Uncaptured(ProcessOwner::Observed(Account { uid, name }));
        row
    }

    /// A passed-through command names its process owner; an unread owner names nothing.
    #[test]
    fn an_observed_owner_names_an_uncaptured_heading() {
        let resolved = owned(
            "/workspace/project",
            40,
            1000,
            AccountName::Resolved("runner-one".into()),
        );
        let unresolved = owned("/workspace/project", 40, 1000, AccountName::Unavailable);
        let unobserved = same_second("/workspace/project", 40);
        for (row, heading) in [
            (&resolved, "[runner-one] /workspace/project"),
            (&unresolved, "[1000] /workspace/project"),
            (&unobserved, "/workspace/project"),
        ] {
            let groups = group_by_path(&[row], PinnedGroup::Unpinned);
            assert_eq!(groups[0].heading(), heading);
            for kind in [TableKind::Command, TableKind::Summary] {
                let text = grouped_table_text(&[row], kind, PinnedGroup::Unpinned);
                assert!(
                    text.lines().any(|line| line.trim() == heading),
                    "{kind:?}: {text}"
                );
            }
        }
    }

    /// A runner's build whose directory cannot be read heads as its account and
    /// "unavailable"; once the shim registers it, the heading is its checkout.
    #[test]
    fn a_runner_heading_names_its_checkout_once_the_shim_registers_it() {
        let checkout = "/run/github-runner/runner-one/hana/hana";
        let mut unread = owned(
            crate::constants::UNRESOLVED_PATH,
            40,
            1000,
            AccountName::Resolved("runner-one".into()),
        );
        unread.process.directory_identity = WorkingDirectoryIdentity::Unavailable;
        let mut registered = same_second(checkout, 41);
        registered.process.provenance = RowProvenance::Direct(capture_context());
        for (row, heading) in [
            (&unread, "[runner-one] unavailable"),
            (
                &registered,
                "[runner-one] /run/github-runner/runner-one/hana/hana",
            ),
        ] {
            let groups = group_by_path(&[row], PinnedGroup::Unpinned);
            assert_eq!(groups[0].heading(), heading);
            for kind in [TableKind::Command, TableKind::Summary] {
                let text = grouped_table_text(&[row], kind, PinnedGroup::Unpinned);
                assert!(
                    text.lines().any(|line| line.trim() == heading),
                    "{kind:?}: {text}"
                );
            }
        }
    }

    /// One uid in one absolute directory draws one heading, whichever member sorts first.
    #[test]
    fn an_owner_row_joins_a_captured_group_of_its_uid_and_directory() {
        let mut captured = same_second("/workspace/project", 41);
        captured.process.provenance = RowProvenance::Direct(capture_context());
        for owner_pid in [40, 42] {
            let owner = owned(
                "/workspace/project",
                owner_pid,
                1000,
                AccountName::Unavailable,
            );
            for rows in [[&owner, &captured], [&captured, &owner]] {
                let groups = group_by_path(&rows, PinnedGroup::Unpinned);
                assert_eq!(groups.len(), 1);
                assert_eq!(groups[0].rows.len(), 2);
                assert_eq!(groups[0].heading(), "[runner-one] /workspace/project");
            }
        }
        let other_uid = owned("/workspace/project", 40, 2000, AccountName::Unavailable);
        let other_directory = owned("/workspace/other", 40, 1000, AccountName::Unavailable);
        for other in [&other_uid, &other_directory] {
            assert_eq!(
                group_by_path(&[other, &captured], PinnedGroup::Unpinned).len(),
                2
            );
        }
    }

    /// An owner row joins the first root of its uid; a second root still heads its own group.
    #[test]
    fn an_owner_row_joins_the_first_root_of_its_uid() {
        let context = capture_context();
        let owner = owned("/workspace/project", 40, 1000, AccountName::Unavailable);
        let mut first_root = same_second("/workspace/project", 41);
        first_root.process.provenance = RowProvenance::Direct(context.clone());
        let mut second_root = same_second("/workspace/project", 42);
        let mut second_context = context;
        second_context.root = CaptureRootIndex(1);
        second_root.process.provenance = RowProvenance::Direct(second_context);
        let rows = [&owner, &first_root, &second_root];
        let groups = group_by_path(&rows, PinnedGroup::Unpinned);
        assert_eq!(groups.len(), 2);
        assert_eq!(
            groups[0]
                .rows
                .iter()
                .map(|row| row.process.pid)
                .collect::<Vec<_>>(),
            [40, 41]
        );
        assert_eq!(groups[1].rows[0].process.pid, 42);
    }

    /// A passed-through lead merged into a captured group still pins that group.
    #[test]
    fn a_pinned_uncaptured_lead_heads_the_captured_group_it_joined() {
        let older = started_at("/other/project", None, 1);
        let mut captured = started_at("/workspace/project", None, 100);
        captured.process.provenance = RowProvenance::Direct(capture_context());
        captured.process.pid = 41;
        let mut lead = owned("/workspace/project", 42, 1000, AccountName::Unavailable);
        lead.process.command = CommandText::of("cargo", &["handler"]);
        let rows = [&older, &lead, &captured];
        let pinned = PinnedGroup::Lead(GroupingIdentity::from(&lead.process));
        let groups = group_by_path(&rows, pinned);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].rows.len(), 2);
        assert!(
            groups[0]
                .rows
                .iter()
                .any(|row| row.process.pid == lead.process.pid)
        );
        assert_eq!(groups[0].heading(), "[runner-one] /workspace/project");
        assert_eq!(groups[1].path, "/other/project");
    }

    /// An unavailable cwd remains a drawable row and keeps its own pin.
    #[test]
    fn unavailable_directory_rows_render_and_pin_by_invocation() {
        let mut first = same_second("unavailable", 40);
        first.process.directory_identity = WorkingDirectoryIdentity::Unavailable;
        let mut second = same_second("unavailable", 41);
        second.process.directory_identity = WorkingDirectoryIdentity::Unavailable;
        let rows = [&first, &second];
        let pinned = PinnedGroup::Lead(GroupingIdentity::from(&second.process));
        let groups = group_by_path(&rows, pinned);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].rows[0].process.pid, 41);
        for kind in [TableKind::Command, TableKind::Summary] {
            let text = grouped_table_text(&rows, kind, pinned);
            assert_eq!(text.matches("unavailable").count(), 2, "{text}");
            assert_eq!(text.matches("cargo build").count(), 2, "{text}");
        }
    }

    /// Prefix width is included before the gauge chooses how much room remains.
    #[test]
    fn account_prefix_reduces_gauge_room_without_changing_counter_absences() {
        let mut row = row_at(GAUGE_PATH, Some(compiling(1, 2)));
        let mut context = capture_context();
        context.account.name = AccountName::Resolved("long-runner-account-name".into());
        row.process.provenance = RowProvenance::Direct(context);
        let rows = [&row];
        let groups = group_by_path(&rows, PinnedGroup::Unpinned);
        let layout = TableLayout::of(
            &rows,
            TableKind::Command,
            Rect::new(0, 0, 60, 5),
            Color::Reset,
            ProcessTree::Long,
        );
        let gauge = heading_gauge(&groups[0], 60, &layout);
        let width = cell_width(SECTION_HEADER_INDENT)
            + cell_width(&groups[0].heading())
            + gauge
                .iter()
                .map(|span| cell_width(&span.content))
                .sum::<u16>();
        assert!(width <= 60);
        assert_eq!(heading_gauge(&groups[0], 40, &layout), [] as [Span<'_>; 0]);
    }

    #[test]
    fn display_shortening_does_not_split_a_directorys_progress_group() {
        let mut ordinary = started_at("~/project", Some(compiling(1, 2)), 100);
        let mut custom_home = started_at("/writer/project", Some(compiling(1, 2)), 101);
        ordinary.process.directory_identity =
            WorkingDirectoryIdentity::Absolute("/writer/project".into());
        custom_home.process.directory_identity = ordinary.process.directory_identity.clone();
        let rows = [&ordinary, &custom_home];

        let groups = group_by_path(&rows, PinnedGroup::Unpinned);

        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].rows.len(), 2);
        assert_eq!(groups[0].path, "~/project");
        assert_eq!(groups[0].rows[1].process.path, "/writer/project");
        let area = Rect::new(0, 0, 80, 10);
        let layout = TableLayout::of(
            &rows,
            TableKind::Summary,
            area,
            pane_background(false),
            ProcessTree::Long,
        );
        assert_eq!(
            groups
                .iter()
                .filter(|group| !heading_gauge(group, area.width, &layout).is_empty())
                .count(),
            1
        );
        ordinary.process.command = CommandText::of("cargo", &["build", "first-project"]);
        custom_home.process.command = CommandText::of("cargo", &["test", "second-project"]);
        for row in [&mut ordinary, &mut custom_home] {
            row.process.state = CaptureLookup::Registered(CaptureRead::Progress(RunState::Blocked));
        }
        let text = grouped_table_text(
            &[&ordinary, &custom_home],
            TableKind::Command,
            PinnedGroup::Unpinned,
        );
        assert_eq!(text.matches("~/project").count(), 1, "{text}");
        assert_eq!(text.matches("/writer/project").count(), 0, "{text}");
        for command in ["cargo build first-project", "cargo test second-project"] {
            assert_eq!(text.matches(command).count(), 1, "{text}");
        }
        assert_eq!(text.matches("blocked").count(), 2, "{text}");
    }

    #[test]
    fn identical_display_labels_do_not_merge_distinct_absolute_directories() {
        let mut first = row_at("~/project", None);
        let mut second = row_at("~/project", None);
        first.process.directory_identity =
            WorkingDirectoryIdentity::Absolute("/first/project".into());
        second.process.directory_identity =
            WorkingDirectoryIdentity::Absolute("/second/project".into());

        assert_eq!(
            group_by_path(&[&first, &second], PinnedGroup::Unpinned).len(),
            2
        );
    }

    #[test]
    fn lossy_display_does_not_merge_distinct_raw_directory_bytes() {
        let first_path = Path::new(OsStr::from_bytes(b"/writer/\xff"));
        let second_path = Path::new(OsStr::from_bytes(b"/writer/\xfe"));
        let mut first = row_at(&first_path.display().to_string(), None);
        let mut second = row_at(&second_path.display().to_string(), None);
        first.process.directory_identity = WorkingDirectoryIdentity::from(first_path);
        second.process.directory_identity = WorkingDirectoryIdentity::from(second_path);

        assert_eq!(first.process.path, second.process.path);
        assert_eq!(
            group_by_path(&[&first, &second], PinnedGroup::Unpinned).len(),
            2
        );
    }

    #[test]
    fn missing_directory_identity_does_not_group_by_placeholder_text() {
        let mut first = row_at("unavailable", None);
        let mut second = row_at("unavailable", None);
        first.process.directory_identity = WorkingDirectoryIdentity::Unavailable;
        second.process.directory_identity = WorkingDirectoryIdentity::Unavailable;

        assert_eq!(
            group_by_path(&[&first, &second], PinnedGroup::Unpinned).len(),
            2
        );
    }

    #[test]
    fn pinning_finds_a_group_through_any_members_directory_identity() {
        let mut ordinary = started_at("~/project", None, 100);
        let mut custom_home = started_at("/writer/project", None, 101);
        ordinary.process.directory_identity =
            WorkingDirectoryIdentity::Absolute("/writer/project".into());
        custom_home.process.directory_identity = ordinary.process.directory_identity.clone();
        let older = started_at("/other/project", None, 1);
        let rows = [&older, &custom_home, &ordinary];

        let groups = group_by_path(
            &rows,
            PinnedGroup::Lead(GroupingIdentity::from(&ordinary.process)),
        );

        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].rows.len(), 2);
        assert_eq!(groups[0].rows[0].process.path, "~/project");
    }

    /// The summary is about no one directory, so nothing is pinned and
    /// the directories fall in the order their work began.
    #[test]
    fn the_summary_pins_no_directory() {
        let lead = row_at("~/rust/cargo-berth-init", None);
        let other = row_at("/private/var/folders/T/case-1", None);
        let rows = [&lead, &other];

        let sorted = group_by_path(&rows, PinnedGroup::Unpinned);

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

        let sorted = group_by_path(&rows, PinnedGroup::Unpinned);

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

        let sorted = group_by_path(&rows, PinnedGroup::Unpinned);

        assert_eq!(
            sorted.first().map(|group| group
                .rows
                .iter()
                .map(|row| row.process.started)
                .collect::<Vec<RunStart>>()),
            Some(vec![RunStart::Known(100), RunStart::Known(160)])
        );
    }

    /// The start time is whole seconds, and a cargo that launches
    /// another shares one with it, so the pair reads in whatever order
    /// the tie is broken in.
    #[test]
    fn a_tied_second_reads_by_pid() {
        let nested = same_second("~/rust/cargo-liner", 93739);
        let driver = same_second("~/rust/cargo-liner", 93738);
        let rows = [&nested, &driver];

        let sorted = group_by_path(&rows, PinnedGroup::Unpinned);

        assert_eq!(
            sorted.first().map(|group| group
                .rows
                .iter()
                .map(|row| row.process.pid)
                .collect::<Vec<u32>>()),
            Some(vec![93738, 93739])
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
        assert_eq!(columns.len(), TABLE_HEADERS.len() - 3);
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
        assert_eq!(columns.len(), TABLE_HEADERS.len() - 3);
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
        assert_eq!(state_width(&CaptureLookup::Unregistered), 0);
        assert_eq!(
            state_width(&CaptureLookup::Registered(CaptureRead::Progress(
                compiling(149, 403)
            ))),
            0,
            "a reading is the heading's to say",
        );
        assert_eq!(
            state_width(&CaptureLookup::Registered(CaptureRead::Progress(
                RunState::Blocked
            ))),
            STATE_BLOCKED.chars().count()
        );
    }

    /// A summary row stands for a whole command, so the columns
    /// describing a single invocation have nothing to say on it --
    /// `parent` among them, which would be blank on every row there.
    #[test]
    fn the_summary_leaves_out_the_columns_that_describe_one_invocation() {
        let rows = [row(None)];

        let columns = visible_columns(
            &rows.iter().collect::<Vec<&TrackedRow>>(),
            TableKind::Summary,
        );

        assert!(!columns.contains(&PARENT_COLUMN));
        assert!(!columns.contains(&COMPILER_COLUMN));
        assert!(!columns.contains(&MANAGED_COLUMN));
        assert!(columns.contains(&COMMAND_COLUMN));
    }

    /// A command's own cell keeps invocation detail when a row has a value for it.
    #[test]
    fn a_commands_own_cell_keeps_them() {
        let mut row = row(None);
        row.process.compiler = CompilerObservation::Running(Compiler {
            name:  COMPILER_PROCESS_NAMES[0],
            count: 1,
        });
        row.process.managed = Measurement::Reading(1);
        let rows = [row];

        let columns = visible_columns(
            &rows.iter().collect::<Vec<&TrackedRow>>(),
            TableKind::Command,
        );

        assert!(columns.contains(&PARENT_COLUMN));
        assert!(columns.contains(&COMPILER_COLUMN));
        assert!(columns.contains(&MANAGED_COLUMN));
    }

    #[test]
    fn an_empty_column_yields_its_cells_to_the_command() {
        let mut row = row(None);
        row.process.command = CommandText::of("cargo", &["check"]);
        let rows = [&row];
        let area = Rect::new(0, 0, 62, 4);
        let layout = TableLayout::of(
            &rows,
            TableKind::Command,
            area,
            Color::Reset,
            ProcessTree::Long,
        );
        let mut buffer = Buffer::empty(area);

        draw_process_table(
            &mut buffer,
            area,
            &rows,
            TableKind::Command,
            Color::Reset,
            PinnedGroup::Unpinned,
            ProcessTree::Long,
        );

        assert!(!layout.columns.contains(&COMPILER_COLUMN));
        assert!(!layout.columns.contains(&MANAGED_COLUMN));
        assert_eq!(
            table_column_text(&buffer, &layout, COMMAND_COLUMN, 2).trim(),
            "cargo check"
        );
        assert_eq!(
            table_column_text(&buffer, &layout, COMMAND_COLUMN, 3).trim(),
            ""
        );
    }

    /// Two rows whose process and parent identifiers have seven digits.
    fn whole_pid_rows() -> [TrackedRow; 2] {
        [
            tight_command_row(1_234_567, 7_654_321, &["build"]),
            tight_command_row(2_345_678, 8_765_432, &["check"]),
        ]
    }

    /// Draw a table whose usable width is stated independently of its indent.
    fn narrow_table_buffer(
        rows: &[&TrackedRow],
        kind: TableKind,
        interior_width: u16,
        height: u16,
    ) -> Buffer {
        let area = Rect::new(
            0,
            0,
            interior_width.saturating_add(cell_width(SECTION_ITEM_INDENT)),
            height,
        );
        let mut buffer = Buffer::empty(area);
        draw_process_table(
            &mut buffer,
            area,
            rows,
            kind,
            Color::Reset,
            PinnedGroup::Unpinned,
            ProcessTree::Long,
        );
        buffer
    }

    #[test]
    fn occupied_tables_without_room_for_a_whole_pid_draw_only_an_elision() {
        let rows = whole_pid_rows();
        let row_refs: Vec<&TrackedRow> = rows.iter().collect();
        let narrowest_pid_width = rows
            .iter()
            .map(|row| cell_width(&row.process.pid.to_string()))
            .min()
            .expect("the fixture has rows");
        let marker_column = cell_width(SECTION_ITEM_INDENT);

        for kind in [TableKind::Command, TableKind::Summary] {
            for interior_width in 1..narrowest_pid_width {
                let buffer =
                    narrow_table_buffer(&row_refs, kind, interior_width, TABLE_HEADER_HEIGHT);
                let line = buffer_line(&buffer, 0);

                assert_eq!(line.trim(), ELISION);
                assert_eq!(digit_runs(&line), Vec::<String>::new());
                assert_eq!(filled_rows(&buffer), usize::from(TABLE_HEADER_HEIGHT));
                assert_eq!(buffer[(marker_column, 0)].fg, label_color());
            }
        }
    }

    #[test]
    fn occupied_tables_with_no_interior_area_draw_nothing() {
        let rows = whole_pid_rows();
        let row_refs: Vec<&TrackedRow> = rows.iter().collect();

        for kind in [TableKind::Command, TableKind::Summary] {
            let zero_width = narrow_table_buffer(&row_refs, kind, 0, TABLE_HEADER_HEIGHT);
            assert_eq!(filled_rows(&zero_width), 0);

            let zero_height = narrow_table_buffer(&row_refs, kind, 1, 0);
            assert_eq!(zero_height.content, Vec::<Cell>::new());
        }
    }

    #[test]
    fn empty_tables_without_room_for_a_whole_pid_draw_no_elision() {
        let rows = whole_pid_rows();
        let narrowest_pid_width = rows
            .iter()
            .map(|row| cell_width(&row.process.pid.to_string()))
            .min()
            .expect("the fixture has rows");

        for kind in [TableKind::Command, TableKind::Summary] {
            for interior_width in 1..narrowest_pid_width {
                let buffer = narrow_table_buffer(&[], kind, interior_width, 3);
                assert!(
                    buffer_rows(&buffer)
                        .iter()
                        .all(|line| line.trim() != ELISION)
                );
            }
        }
    }

    /// Check every inner width for one table kind.
    fn assert_whole_table_columns(kind: TableKind, interior_widths: impl Iterator<Item = u16>) {
        let rows = whole_pid_rows();
        let row_refs: Vec<&TrackedRow> = rows.iter().collect();
        let indent = cell_width(SECTION_ITEM_INDENT);
        let full_columns = visible_columns(&row_refs, kind);
        let full_width = table_width_with_spacing(
            &fitted_constraints(&row_refs, &full_columns),
            TIGHT_TABLE_COLUMN_SPACING,
        );
        for interior_width in interior_widths {
            let area = Rect::new(0, 0, interior_width + indent, 8);
            let layout = TableLayout::of(&row_refs, kind, area, Color::Reset, ProcessTree::Long);
            let mut buffer = Buffer::empty(area);
            draw_process_table(
                &mut buffer,
                area,
                &row_refs,
                kind,
                Color::Reset,
                PinnedGroup::Unpinned,
                ProcessTree::Long,
            );

            if layout.columns.is_empty() {
                let expected = if interior_width == 0 { "" } else { ELISION };
                assert_eq!(buffer_line(&buffer, 0).trim(), expected);
            }

            if interior_width >= full_width {
                assert_eq!(layout.columns, full_columns, "{kind:?} at {interior_width}");
            }
            for &column in &layout.columns {
                assert_eq!(
                    table_column_text(&buffer, &layout, column, 0).trim_end(),
                    TABLE_HEADERS[column],
                    "{kind:?} column {column} at {interior_width}",
                );
                let expected = match column {
                    PID_COLUMN => &["1234567", "2345678"][..],
                    PARENT_COLUMN => &["7654321", "8765432"][..],
                    _ => continue,
                };
                let mut drawn: Vec<String> = (1..area.height)
                    .flat_map(|y| digit_runs(&table_column_text(&buffer, &layout, column, y)))
                    .collect();
                drawn.sort_unstable();
                drawn.dedup();
                assert_eq!(
                    drawn, expected,
                    "{kind:?} column {column} at {interior_width}"
                );
            }
        }
    }

    #[test]
    fn command_widths_0_to_22_draw_whole_pids_headers_and_columns() {
        assert_whole_table_columns(TableKind::Command, 0..=22);
    }

    #[test]
    fn command_widths_23_to_45_draw_whole_pids_headers_and_columns() {
        assert_whole_table_columns(TableKind::Command, 23..=45);
    }

    #[test]
    fn command_widths_46_to_68_draw_whole_pids_headers_and_columns() {
        assert_whole_table_columns(TableKind::Command, 46..=68);
    }

    #[test]
    fn command_widths_69_to_90_draw_whole_pids_headers_and_columns() {
        assert_whole_table_columns(TableKind::Command, 69..=90);
    }

    #[test]
    fn summary_widths_0_to_22_draw_whole_pids_headers_and_columns() {
        assert_whole_table_columns(TableKind::Summary, 0..=22);
    }

    #[test]
    fn summary_widths_23_to_45_draw_whole_pids_headers_and_columns() {
        assert_whole_table_columns(TableKind::Summary, 23..=45);
    }

    #[test]
    fn summary_widths_46_to_68_draw_whole_pids_headers_and_columns() {
        assert_whole_table_columns(TableKind::Summary, 46..=68);
    }

    #[test]
    fn summary_widths_69_to_90_draw_whole_pids_headers_and_columns() {
        assert_whole_table_columns(TableKind::Summary, 69..=90);
    }

    #[test]
    fn shortened_summary_text_keeps_a_non_space_character_beside_its_mark() {
        assert_eq!(elide_summary_end(" abc", 2), "");
        assert_eq!(elide_summary_end(" abc", 3), format!(" a{ELISION}"));
        assert_eq!(elide_summary_end("abc def", 5), format!("abc{ELISION}"));
    }

    /// Draw the production grid at a chosen summary interior width.
    fn summary_alone_buffer(roster: &Roster, inner_width: u16) -> Buffer {
        let area = Rect::new(0, 0, inner_width.saturating_add(2), 8);
        let mut buffer = Buffer::empty(area);
        let mut grid = tui_pane::TileGrid::new();
        grid.set_min_tile_width(crate::constants::MIN_CELL_WIDTH);
        let sccache = SccacheStats::new();
        let hidden_when_idle = hidden_when_idle();
        let cells = Cells::new(roster, &hidden_when_idle, ProcessTree::Long, &sccache);
        tui_pane::draw_tile_grid(
            &mut buffer,
            &mut grid,
            area,
            tui_pane::TileGrowth::default(),
            TileGridContents::Shown,
            &cells,
        );
        grid.settle_for_test();
        buffer = Buffer::empty(area);
        tui_pane::draw_tile_grid(
            &mut buffer,
            &mut grid,
            area,
            tui_pane::TileGrowth::default(),
            TileGridContents::Shown,
            &cells,
        );
        buffer
    }

    /// Draw a settled grid using one cell-content implementation.
    fn settled_buffer_with_cells(
        cells: &impl TileCells<InvocationId>,
        area: Rect,
    ) -> (TileGrid<InvocationId>, Buffer) {
        let growth = TileGrowth {
            initial_rows:  12,
            fill:          TileFill::Redistribute,
            widen_summary: false,
        };
        let mut grid = tui_pane::TileGrid::new();
        grid.set_min_tile_width(crate::constants::MIN_CELL_WIDTH);
        grid.set_min_tile_height(crate::constants::MIN_CELL_HEIGHT);
        grid.set_view(TileView::Cells);
        let mut buffer = Buffer::empty(area);
        tui_pane::draw_tile_grid(
            &mut buffer,
            &mut grid,
            area,
            growth,
            TileGridContents::Shown,
            cells,
        );
        grid.settle_for_test();
        buffer = Buffer::empty(area);
        tui_pane::draw_tile_grid(
            &mut buffer,
            &mut grid,
            area,
            growth,
            TileGridContents::Shown,
            cells,
        );
        (grid, buffer)
    }

    /// Draw a settled production grid for the command-cell content checks.
    fn settled_cells_buffer(roster: &Roster, area: Rect) -> (TileGrid<InvocationId>, Buffer) {
        let sccache = SccacheStats::new();
        let hidden_when_idle = hidden_when_idle();
        let cells = Cells::new(roster, &hidden_when_idle, ProcessTree::Long, &sccache);
        settled_buffer_with_cells(&cells, area)
    }

    /// Production cells with the trait's every-row default retained.
    struct CellsWithoutRowReport<'a>(Cells<'a>);

    impl TileCells<InvocationId> for CellsWithoutRowReport<'_> {
        fn summary_title(&self) -> &str { self.0.summary_title() }

        fn summary_foot(&self) -> SummaryFoot { self.0.summary_foot() }

        fn demands(&self, widths: &[(TileContent, u16)]) -> TileDemands { self.0.demands(widths) }

        fn draw(&self, buffer: &mut Buffer, content: &TileContent, inner: Rect, ground: Color) {
            self.0.draw(buffer, content, inner, ground);
        }

        fn summary_labels(&self, rect: Rect) -> Vec<PaneFrameLabel> { self.0.summary_labels(rect) }
    }

    /// A roster with one independently tiled command per requested cell.
    fn tiled_roster(commands: u32, ancestry: &[Ancestor]) -> Roster {
        let groups = (0..commands)
            .map(|offset| {
                let mut lead = invocation(4100_u32.saturating_add(offset), &[]);
                let directory = offset.min(2);
                lead.path = format!("~/rust/cargo-liner-{directory}");
                lead.directory_identity = WorkingDirectoryIdentity::Absolute(
                    format!("/test-home/rust/cargo-liner-{directory}").into(),
                );
                CargoGroup {
                    lead,
                    rest: Vec::new(),
                    ancestry: ancestry.to_vec(),
                }
            })
            .collect();
        let mut roster = Roster::new();
        roster.observe(groups, Instant::now());
        roster
    }

    /// One independently tiled command and one companion row per cell.
    fn paired_tiled_roster(commands: u32, ancestry: &[Ancestor]) -> Roster {
        let groups = (0..commands)
            .map(|offset| {
                let pid = 6100_u32.saturating_add(offset.saturating_mul(2));
                let directory = offset.min(2);
                let path = format!("~/rust/cargo-liner-{directory}");
                let identity = WorkingDirectoryIdentity::Absolute(
                    format!("/test-home/rust/cargo-liner-{directory}").into(),
                );
                let mut lead = invocation(pid, &["check"]);
                lead.path.clone_from(&path);
                lead.directory_identity.clone_from(&identity);
                let mut companion = invocation(pid.saturating_add(1), &["check"]);
                companion.path = path;
                companion.directory_identity = identity;
                CargoGroup {
                    lead,
                    rest: vec![companion],
                    ancestry: ancestry.to_vec(),
                }
            })
            .collect();
        let mut roster = Roster::new();
        roster.observe(groups, Instant::now());
        roster
    }

    /// One optional ancestry row beside a command with `companions` extra rows.
    fn ancestry_transfer_roster(companions: u32) -> Roster {
        let mut donor = invocation(7100, &["check"]);
        donor.path = "~/rust/transfer".to_string();
        donor.directory_identity =
            WorkingDirectoryIdentity::Absolute("/test-home/rust/transfer".into());
        let mut recipient = invocation(7200, &["check"]);
        recipient.path.clone_from(&donor.path);
        recipient
            .directory_identity
            .clone_from(&donor.directory_identity);
        let rest = (1..=companions)
            .map(|offset| {
                let mut companion = invocation(7200_u32.saturating_add(offset), &["check"]);
                companion.path.clone_from(&donor.path);
                companion
                    .directory_identity
                    .clone_from(&donor.directory_identity);
                companion
            })
            .collect();
        let groups = vec![
            CargoGroup {
                lead:     donor,
                rest:     Vec::new(),
                ancestry: vec![ancestor(7000, "cargo")],
            },
            CargoGroup {
                lead: recipient,
                rest,
                ancestry: Vec::new(),
            },
        ];
        let mut roster = Roster::new();
        roster.observe(groups, Instant::now());
        roster
    }

    /// A roster whose command cells span the requested directory counts.
    fn directory_rich_roster(directory_counts: &[u32]) -> Roster {
        let groups = directory_counts
            .iter()
            .enumerate()
            .map(|(group_index, &directory_count)| {
                let group_index = u32::try_from(group_index).unwrap_or(u32::MAX);
                let first_pid = 4100_u32.saturating_add(group_index.saturating_mul(100));
                let mut processes: Vec<CargoProcess> = (0..directory_count)
                    .map(|directory_index| {
                        let pid = first_pid.saturating_add(directory_index);
                        let mut process = invocation(pid, &["build"]);
                        process.path = format!("~/work-{group_index}/dir-{directory_index}");
                        process.directory_identity = WorkingDirectoryIdentity::Absolute(
                            format!("/test-home/work-{group_index}/dir-{directory_index}").into(),
                        );
                        process.started = RunStart::Known(u64::from(directory_index));
                        process
                    })
                    .collect();
                let lead = processes.remove(0);
                CargoGroup {
                    lead,
                    rest: processes,
                    ancestry: Vec::new(),
                }
            })
            .collect();
        let mut roster = Roster::new();
        roster.observe(groups, Instant::now());
        roster
    }

    /// Draw the same roster with and without the production row report.
    fn row_report_comparison(
        roster: &Roster,
        area: Rect,
    ) -> (
        (TileGrid<InvocationId>, Buffer),
        (TileGrid<InvocationId>, Buffer),
    ) {
        let sccache = SccacheStats::new();
        let hidden_when_idle = hidden_when_idle();
        let production = Cells::new(roster, &hidden_when_idle, ProcessTree::Long, &sccache);
        let without_report = CellsWithoutRowReport(Cells::new(
            roster,
            &hidden_when_idle,
            ProcessTree::Long,
            &sccache,
        ));
        (
            settled_buffer_with_cells(&production, area),
            settled_buffer_with_cells(&without_report, area),
        )
    }

    /// Content demand for one placement.
    fn placement_demand(demands: &TileDemands, content: &TileContent) -> usize {
        match content {
            TileContent::Summary => demands.summary,
            TileContent::Group(id) => demands
                .groups
                .iter()
                .find(|demand| &demand.id == id)
                .map_or(0, |demand| demand.rows),
            TileContent::Empty(_) => 0,
        }
    }

    /// Rows above the readout inside one settled placement.
    fn placement_content_rows(placement: &TilePlacement<InvocationId>) -> u16 {
        placement.frame.inner().height.saturating_sub(1)
    }

    /// Whether the row immediately above a placement's readout has no text.
    fn has_blank_row_above_foot(buffer: &Buffer, placement: &TilePlacement<InvocationId>) -> bool {
        let inner = placement.frame.inner();
        if inner.height < 2 {
            return false;
        }
        let y = inner.bottom().saturating_sub(2);
        (inner.left()..inner.right()).all(|x| buffer[(x, y)].symbol().trim().is_empty())
    }

    /// Blank content rows inside one placement, excluding its readout.
    fn blank_content_row_count(buffer: &Buffer, placement: &TilePlacement<InvocationId>) -> usize {
        let inner = placement.frame.inner();
        let foot = inner.bottom().saturating_sub(1);
        (inner.top()..foot)
            .filter(|&y| {
                (inner.left()..inner.right()).all(|x| buffer[(x, y)].symbol().trim().is_empty())
            })
            .count()
    }

    /// Text inside one tile placement.
    fn placement_text(buffer: &Buffer, placement: &TilePlacement<InvocationId>) -> String {
        let inner = placement.frame.inner();
        (inner.top()..inner.bottom())
            .map(|y| {
                (inner.left()..inner.right())
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Placements that retain a blank row while a column mate is short.
    fn blank_rows_beside_hidden_content(
        roster: &Roster,
        grid: &TileGrid<InvocationId>,
        buffer: &Buffer,
        area: Rect,
    ) -> Vec<TileContent> {
        let growth = TileGrowth {
            initial_rows:  12,
            fill:          TileFill::Redistribute,
            widen_summary: false,
        };
        let placements = grid.placements(area, growth);
        let widths: Vec<_> = placements
            .iter()
            .map(|placement| (placement.content.clone(), placement.frame.inner().width))
            .collect();
        let demands = tile_demands(roster, &widths, &hidden_when_idle(), ProcessTree::Long);
        placements
            .iter()
            .filter(|placement| {
                let frame = placement.frame.rect();
                let has_short_mate = placements.iter().any(|mate| {
                    let mate_frame = mate.frame.rect();
                    mate.content != placement.content
                        && mate_frame.x == frame.x
                        && mate_frame.width == frame.width
                        && placement_demand(&demands, &mate.content)
                            > usize::from(placement_content_rows(mate))
                });
                has_short_mate && has_blank_row_above_foot(buffer, placement)
            })
            .map(|placement| placement.content.clone())
            .collect()
    }

    #[test]
    fn a_near_full_summary_takes_rows_and_keeps_its_directory_gaps() {
        let roster = directory_rich_roster(&[4, 5, 4, 5, 4, 5]);
        let area = Rect::new(0, 0, 200, 57);
        let (grid, buffer) = settled_cells_buffer(&roster, area);
        let placements = grid.placements(
            area,
            TileGrowth {
                initial_rows:  12,
                fill:          TileFill::Redistribute,
                widen_summary: false,
            },
        );
        let summary = placements
            .into_iter()
            .find(|placement| placement.content == TileContent::Summary)
            .expect("the summary is placed");
        let widths = [(TileContent::Summary, summary.frame.inner().width)];
        let demands = tile_demands(&roster, &widths, &hidden_when_idle(), ProcessTree::Long);

        assert_eq!(
            usize::from(placement_content_rows(&summary)),
            demands.summary
        );
        assert_eq!(
            blank_content_row_count(&buffer, &summary),
            roster.groups().len().saturating_sub(1)
        );
        assert!(!has_blank_row_above_foot(&buffer, &summary));
    }

    #[test]
    fn a_summary_hiding_a_directory_leaves_no_blank_row_above_its_foot() {
        let roster = directory_rich_roster(&[4, 5, 4, 5, 4, 5]);
        let area = Rect::new(0, 0, 64, 49);
        let (production, without_report) = row_report_comparison(&roster, area);
        let former =
            blank_rows_beside_hidden_content(&roster, &without_report.0, &without_report.1, area);
        let repaired =
            blank_rows_beside_hidden_content(&roster, &production.0, &production.1, area);
        let growth = TileGrowth {
            initial_rows:  12,
            fill:          TileFill::Redistribute,
            widen_summary: false,
        };
        let placements = production.0.placements(area, growth);
        let summary = placements
            .iter()
            .find(|placement| placement.content == TileContent::Summary)
            .expect("the summary is placed");
        let widths: Vec<_> = placements
            .iter()
            .map(|placement| (placement.content.clone(), placement.frame.inner().width))
            .collect();
        let demands = tile_demands(&roster, &widths, &hidden_when_idle(), ProcessTree::Long);

        assert!(former.contains(&TileContent::Summary));
        assert!(!repaired.contains(&TileContent::Summary));
        assert!(demands.summary > usize::from(placement_content_rows(summary)));
    }

    #[test]
    fn a_summary_hiding_rows_has_no_blank_foot_row_beside_a_hidden_directory() {
        let roster = directory_rich_roster(&[2, 3, 2, 3]);
        let area = Rect::new(0, 0, 200, 30);
        let (grid, buffer) = settled_cells_buffer(&roster, area);
        let blank = blank_rows_beside_hidden_content(&roster, &grid, &buffer, area);

        assert!(!blank.contains(&TileContent::Summary), "{blank:?}");
    }

    #[test]
    fn a_command_cell_leaves_no_spare_row_while_a_column_mate_is_short() {
        let roster = directory_rich_roster(&[4, 5, 4, 5, 4, 5]);
        let area = Rect::new(0, 0, 200, 50);
        let target = TileContent::Group(roster.groups()[0].id.clone());
        let (production, without_report) = row_report_comparison(&roster, area);
        let former =
            blank_rows_beside_hidden_content(&roster, &without_report.0, &without_report.1, area);
        let repaired =
            blank_rows_beside_hidden_content(&roster, &production.0, &production.1, area);

        assert!(former.contains(&target));
        assert!(!repaired.contains(&target));
    }

    #[test]
    fn a_command_hiding_rows_has_no_blank_foot_row_beside_hidden_ancestry() {
        let ancestry = [ancestor(4001, "cargo"), ancestor(4002, "cargo")];
        let roster = tiled_roster(4, &ancestry);
        let area = Rect::new(0, 0, 200, 30);
        let (grid, buffer) = settled_cells_buffer(&roster, area);
        let blank = blank_rows_beside_hidden_content(&roster, &grid, &buffer, area);

        assert!(
            blank.iter().all(|content| *content == TileContent::Summary),
            "{blank:?}"
        );
    }

    #[test]
    fn a_lone_ancestry_row_goes_to_a_mates_next_process_step() {
        let roster = ancestry_transfer_roster(2);
        let area = Rect::new(0, 0, 80, 19);
        let (grid, buffer) = settled_cells_buffer(&roster, area);
        let placements = grid.placements(
            area,
            TileGrowth {
                initial_rows:  12,
                fill:          TileFill::Redistribute,
                widen_summary: false,
            },
        );
        let donor = placements
            .iter()
            .find(|placement| {
                placement.content == TileContent::Group(roster.groups()[0].id.clone())
            })
            .expect("the ancestry donor is placed");
        let recipient = placements
            .iter()
            .find(|placement| {
                placement.content == TileContent::Group(roster.groups()[1].id.clone())
            })
            .expect("the process-row recipient is placed");

        assert_eq!(placement_content_rows(donor), 3);
        assert_eq!(placement_content_rows(recipient), 5);
        assert!(!placement_text(&buffer, donor).contains("7000"));
        assert!(placement_text(&buffer, recipient).contains("7202"));
    }

    #[test]
    fn an_unusable_lone_row_draws_ancestry_without_a_blank_foot_row() {
        let roster = ancestry_transfer_roster(1);
        let area = Rect::new(0, 0, 80, 19);
        let (grid, buffer) = settled_cells_buffer(&roster, area);
        let donor = grid
            .placements(
                area,
                TileGrowth {
                    initial_rows:  12,
                    fill:          TileFill::Redistribute,
                    widen_summary: false,
                },
            )
            .into_iter()
            .find(|placement| {
                placement.content == TileContent::Group(roster.groups()[0].id.clone())
            })
            .expect("the ancestry cell is placed");

        assert_eq!(placement_content_rows(&donor), 4);
        let content_area = Rect {
            height: placement_content_rows(&donor),
            ..donor.frame.inner()
        };
        let content = &donor.content;
        let hidden = hidden_when_idle();
        assert_eq!(
            (
                content_rows_drawn(&roster, content, content_area, &hidden, ProcessTree::Long),
                content_rows_kept(&roster, content, content_area, &hidden, ProcessTree::Long),
            ),
            (4, 3)
        );
        assert!(placement_text(&buffer, &donor).contains("7000"));
        assert!(!has_blank_row_above_foot(&buffer, &donor));
    }

    #[test]
    fn painted_and_retained_rows_share_one_measurement() {
        let roster = ancestry_transfer_roster(1);
        let hidden = hidden_when_idle();
        let sccache = SccacheStats::new();
        let cells = Cells::new(&roster, &hidden, ProcessTree::Long, &sccache);
        let content = TileContent::Group(roster.groups()[0].id.clone());
        let area = Rect::new(0, 0, 24, 2);

        let drawn = cells.rows_drawn(&content, area);
        let kept = cells.rows_kept(&content, area);

        assert_eq!(
            (drawn, kept),
            (
                content_rows_drawn(&roster, &content, area, &hidden, ProcessTree::Long),
                content_rows_kept(&roster, &content, area, &hidden, ProcessTree::Long),
            )
        );
        assert_eq!(cells.row_uses.borrow().len(), 1);
    }

    #[test]
    fn seven_short_cells_leave_no_blank_row_above_their_foot() {
        let ancestry = [
            ancestor(4001, "cargo"),
            ancestor(4002, "cargo"),
            ancestor(4003, "cargo"),
        ];
        let roster = paired_tiled_roster(7, &ancestry);
        let area = Rect::new(0, 0, 200, 50);
        let (grid, buffer) = settled_cells_buffer(&roster, area);
        let growth = TileGrowth {
            initial_rows:  12,
            fill:          TileFill::Redistribute,
            widen_summary: false,
        };
        let placements = grid.placements(area, growth);
        let widths: Vec<_> = placements
            .iter()
            .map(|placement| (placement.content.clone(), placement.frame.inner().width))
            .collect();
        let demands = tile_demands(&roster, &widths, &hidden_when_idle(), ProcessTree::Long);
        let hidden = placements
            .iter()
            .filter(|placement| {
                placement_demand(&demands, &placement.content)
                    > usize::from(placement_content_rows(placement))
            })
            .collect::<Vec<_>>();
        assert!(!hidden.is_empty(), "the fixture must hide content");
        let blank = hidden
            .into_iter()
            .filter(|placement| has_blank_row_above_foot(&buffer, placement))
            .map(|placement| placement.content.clone())
            .collect::<Vec<_>>();

        assert!(blank.is_empty(), "blank rows above the foot: {blank:?}");
    }

    #[test]
    fn a_short_command_cell_takes_the_row_its_column_mate_cannot_use() {
        let roster = directory_rich_roster(&[2, 4, 6, 3, 5, 4]);
        let area = Rect::new(0, 0, 126, 79);
        let target = TileContent::Group(roster.groups()[1].id.clone());
        let (production, without_report) = row_report_comparison(&roster, area);
        let former =
            blank_rows_beside_hidden_content(&roster, &without_report.0, &without_report.1, area);
        let repaired =
            blank_rows_beside_hidden_content(&roster, &production.0, &production.1, area);

        assert!(former.contains(&target));
        assert!(!repaired.contains(&target));
    }

    #[test]
    fn a_command_refused_for_height_remains_in_the_summary_until_it_opens() {
        let roster = tiled_roster(1, &[]);
        let growth = TileGrowth {
            initial_rows:  12,
            fill:          TileFill::Redistribute,
            widen_summary: false,
        };
        let mut grid = tui_pane::TileGrid::new();
        grid.set_min_tile_width(crate::constants::MIN_CELL_WIDTH);
        grid.set_min_tile_height(crate::constants::MIN_CELL_HEIGHT);
        let sccache = SccacheStats::new();
        let hidden_when_idle = hidden_when_idle();
        let cells = Cells::new(&roster, &hidden_when_idle, ProcessTree::Long, &sccache);
        let short = Rect::new(0, 0, 40, 6);
        let mut buffer = Buffer::empty(short);
        tui_pane::draw_tile_grid(
            &mut buffer,
            &mut grid,
            short,
            growth,
            TileGridContents::Shown,
            &cells,
        );
        grid.settle_for_test();
        buffer = Buffer::empty(short);
        tui_pane::draw_tile_grid(
            &mut buffer,
            &mut grid,
            short,
            growth,
            TileGridContents::Shown,
            &cells,
        );

        let placements = grid.placements(short, growth);
        assert_eq!(
            placements
                .iter()
                .filter(|placement| matches!(placement.content, TileContent::Group(_)))
                .count(),
            0
        );
        let summary = placements
            .iter()
            .find(|placement| matches!(placement.content, TileContent::Summary))
            .expect("the summary stays open");
        let summary_text = (summary.frame.inner().top()..summary.frame.inner().bottom())
            .map(|y| buffer_line(&buffer, y))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(summary_text.contains("4100"), "{summary_text:?}");
        assert!(
            !summary_text.contains(NO_PROCESSES_NOTE),
            "{summary_text:?}"
        );

        let tall = Rect::new(0, 0, 40, 12);
        buffer = Buffer::empty(tall);
        tui_pane::draw_tile_grid(
            &mut buffer,
            &mut grid,
            tall,
            growth,
            TileGridContents::Shown,
            &cells,
        );
        grid.settle_for_test();
        buffer = Buffer::empty(tall);
        tui_pane::draw_tile_grid(
            &mut buffer,
            &mut grid,
            tall,
            growth,
            TileGridContents::Shown,
            &cells,
        );

        let command = grid
            .placements(tall, growth)
            .into_iter()
            .find(|placement| matches!(placement.content, TileContent::Group(_)))
            .expect("the command opens after the resize");
        assert!(
            (command.frame.inner().top()..command.frame.inner().bottom())
                .map(|y| buffer_line(&buffer, y))
                .any(|line| line.contains("4100"))
        );
    }

    #[test]
    fn every_command_cell_shows_a_process() {
        let cases = [
            (Rect::new(0, 0, 40, 50), 6, Vec::new(), Some(12)),
            (Rect::new(0, 0, 48, 50), 6, Vec::new(), Some(12)),
            (Rect::new(0, 0, 64, 50), 6, Vec::new(), Some(12)),
            (Rect::new(0, 0, 200, 50), 10, Vec::new(), None),
            (Rect::new(0, 0, 126, 80), 6, long_chain(), None),
        ];
        let growth = TileGrowth {
            initial_rows:  12,
            fill:          TileFill::Redistribute,
            widen_summary: false,
        };

        for (area, command_count, ancestry, expected_summary_rows) in cases {
            let roster = tiled_roster(command_count, &ancestry);
            let (grid, buffer) = settled_cells_buffer(&roster, area);
            let placements = grid.placements(area, growth);
            if let Some(expected) = expected_summary_rows {
                let widths: Vec<_> = placements
                    .iter()
                    .map(|placement| (placement.content.clone(), placement.frame.inner().width))
                    .collect();
                let demands =
                    tile_demands(&roster, &widths, &hidden_when_idle(), ProcessTree::Long);
                assert_eq!(demands.summary, expected, "{area:?}");
            }
            let commands: Vec<_> = placements
                .iter()
                .filter(|placement| matches!(&placement.content, TileContent::Group(_)))
                .collect();
            assert!(!commands.is_empty());

            for placement in commands {
                let TileContent::Group(id) = &placement.content else {
                    continue;
                };
                let group = roster
                    .groups()
                    .iter()
                    .find(|group| &group.id == id)
                    .unwrap();
                let pid = group.lead.process.pid.to_string();
                let inner = placement.frame.inner();
                let lines: Vec<String> = (inner.top()..inner.bottom())
                    .map(|y| {
                        (inner.left()..inner.right())
                            .map(|x| buffer[(x, y)].symbol())
                            .collect()
                    })
                    .collect();
                let header = lines
                    .iter()
                    .position(|line| {
                        line.contains(TABLE_HEADERS[PID_COLUMN])
                            && line.contains(TABLE_HEADERS[COMMAND_COLUMN])
                    })
                    .unwrap();
                assert!(
                    lines[header + 1].contains("cargo-liner"),
                    "{area:?}: {lines:#?}"
                );
                assert!(lines[header + 2].contains(&pid), "{area:?}: {lines:#?}");
                assert!(lines[header + 2].contains("cargo"), "{area:?}: {lines:#?}");
                assert!(
                    lines
                        .last()
                        .is_some_and(|line| line.contains("content rows:")),
                    "{area:?}: {lines:#?}"
                );
            }
        }
    }

    /// Whether one drawn word is complete or carries the summary's cut mark.
    fn whole_or_marked_summary_word(word: &str, values: &[&str]) -> bool {
        if values.contains(&word) {
            return true;
        }
        if let Some(prefix) = word.strip_suffix(ELISION) {
            return !prefix.is_empty() && values.iter().any(|value| value.starts_with(prefix));
        }
        if let Some(suffix) = word.strip_prefix(ELISION) {
            return !suffix.is_empty() && values.iter().any(|value| value.ends_with(suffix));
        }
        false
    }

    /// Every word above the summary's foot must be complete or carry its cut mark.
    fn assert_summary_words(buffer: &Buffer, values: &[&str], inner_width: u16) {
        for y in 1..buffer.area.height.saturating_sub(2) {
            let text: String = (1..=inner_width).map(|x| buffer[(x, y)].symbol()).collect();
            for word in text.split_whitespace() {
                if word == ELISION {
                    assert_eq!(y, 1, "bare summary mark at inner width {inner_width}");
                    continue;
                }
                assert!(
                    whole_or_marked_summary_word(word, values),
                    "bare summary word {word:?} at inner width {inner_width}: {text:?}",
                );
            }
        }
    }

    /// The summary frame occupies the area and no group-only text appears inside it.
    fn assert_summary_alone(buffer: &Buffer, inner_width: u16) {
        let area = buffer.area;
        assert_eq!(area.width, inner_width.saturating_add(2));
        assert_eq!(buffer[(area.left(), area.top())].symbol(), "┌");
        assert_eq!(buffer[(area.right() - 1, area.top())].symbol(), "┐");
        assert_eq!(buffer[(area.left(), area.bottom() - 1)].symbol(), "└");
        assert_eq!(buffer[(area.right() - 1, area.bottom() - 1)].symbol(), "┘");
        for y in area.top() + 1..area.bottom() - 1 {
            assert_eq!(buffer[(area.left(), y)].symbol(), "│");
            assert_eq!(buffer[(area.right() - 1, y)].symbol(), "│");
            for x in area.left() + 1..area.right() - 1 {
                assert!(
                    !matches!(
                        buffer[(x, y)].symbol(),
                        "┌" | "┐" | "└" | "┘" | "├" | "┤" | "┬" | "┴" | "┼" | "│"
                    ),
                    "interior frame glyph at ({x}, {y}) for inner width {inner_width}",
                );
            }
        }
        assert!(
            buffer_rows(buffer).iter().all(|line| !line.contains("zed")),
            "group-only ancestry at inner width {inner_width}",
        );
    }

    /// Text in the cells reserved for the summary foot, with padding removed.
    fn drawn_summary_foot_text(buffer: &Buffer, whole_foot: &str) -> String {
        let y = buffer.area.bottom().saturating_sub(2);
        let start = buffer.area.left().saturating_add(2);
        let end = start
            .saturating_add(cell_width(whole_foot))
            .min(buffer.area.right().saturating_sub(1));
        (start..end)
            .map(|x| buffer[(x, y)].symbol())
            .collect::<String>()
            .trim_end()
            .to_string()
    }

    #[test]
    fn the_summary_alone_is_whole_marked_or_absent_at_every_width() {
        let mut lead = invocation(4100, &["build"]);
        lead.memory = Measurement::Reading(BYTES_PER_GIBIBYTE);
        let roster = roster_of(lead, Vec::new());
        let occupied_values = [
            "~/rust/cargo-liner",
            "4100",
            "11:04",
            "00:18",
            "12%",
            "1.0G",
            "cargo",
            "build",
            TABLE_HEADERS[PID_COLUMN],
            TABLE_HEADERS[START_COLUMN],
            TABLE_HEADERS[DURATION_COLUMN],
            TABLE_HEADERS[CPU_COLUMN],
            TABLE_HEADERS[MEMORY_COLUMN],
            TABLE_HEADERS[COMMAND_COLUMN],
        ];
        let empty_values = ["no", "cargo", "processes", "running"];
        let whole_foot = "mem 1.0G";

        let last_summary_only_inner_width = crate::constants::MIN_CELL_WIDTH.saturating_sub(3);
        for inner_width in 6..=last_summary_only_inner_width {
            let occupied = summary_alone_buffer(&roster, inner_width);
            assert_summary_alone(&occupied, inner_width);
            assert_summary_words(&occupied, &occupied_values, inner_width);
            let foot = drawn_summary_foot_text(&occupied, whole_foot);
            assert!(
                foot.is_empty() || foot == whole_foot,
                "bare summary foot at inner width {inner_width}: {foot:?}",
            );

            let empty = summary_alone_buffer(&Roster::new(), inner_width);
            assert_summary_alone(&empty, inner_width);
            assert_summary_words(&empty, &empty_values, inner_width);
            let empty_note: String = (1..=inner_width)
                .map(|x| empty[(x, 2)].symbol())
                .collect::<String>()
                .trim()
                .to_string();
            let marked_note = empty_note.strip_suffix(ELISION).is_some_and(|prefix| {
                !prefix.trim().is_empty() && NO_PROCESSES_NOTE.starts_with(prefix)
            });
            assert!(
                empty_note == NO_PROCESSES_NOTE || marked_note || empty_note.is_empty(),
                "bare empty-summary note at inner width {inner_width}: {empty_note:?}",
            );
        }

        let floor_inner_width = crate::constants::MIN_CELL_WIDTH.saturating_sub(2);
        let floor = summary_alone_buffer(&roster, floor_inner_width);
        let right = floor.area.right().saturating_sub(1);
        assert!(
            (floor.area.top() + 1..floor.area.bottom() - 1).any(|y| {
                floor[(floor.area.left(), y)].symbol() == "├" && floor[(right, y)].symbol() == "┤"
            }),
            "a command cell is not drawn at the cell-width floor",
        );
    }

    #[test]
    fn narrow_tables_drop_columns_in_order_and_blank_undersized_pids() {
        assert_eq!(number_text_if_fits("1234567".to_string(), 6), "");
        assert_eq!(number_text_if_fits("1234567".to_string(), 7), "1234567");

        let mut blocked_rows = whole_pid_rows();
        for row in &mut blocked_rows {
            row.process.state = CaptureLookup::Registered(CaptureRead::Progress(RunState::Blocked));
        }
        let blocked_refs: Vec<&TrackedRow> = blocked_rows.iter().collect();
        let indent = cell_width(SECTION_ITEM_INDENT);
        let mut prior = visible_columns(&blocked_refs, TableKind::Command);
        let mut removed = Vec::new();
        for interior_width in (0..=90).rev() {
            let layout = TableLayout::of(
                &blocked_refs,
                TableKind::Command,
                Rect::new(0, 0, interior_width + indent, 1),
                Color::Reset,
                ProcessTree::Long,
            );
            let departed: Vec<usize> = prior
                .iter()
                .copied()
                .filter(|column| !layout.columns.contains(column))
                .collect();
            assert!(
                departed.len() <= 1,
                "at interior width {interior_width}: {departed:?}"
            );
            removed.extend(departed);
            prior = layout.columns;
        }
        assert_eq!(
            removed,
            TABLE_COLUMN_DROP_ORDER
                .into_iter()
                .chain([COMMAND_COLUMN, PID_COLUMN])
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_tight_command_table_keeps_fixed_headers_and_parent_pids_whole() {
        let area = Rect::new(0, 0, 65, 6);
        let mut buffer = Buffer::empty(area);
        let rows = [
            tight_command_row(359_829, 359_762, &["build"]),
            tight_command_row(359_830, 359_763, &["check"]),
        ];

        draw_process_table(
            &mut buffer,
            area,
            &rows.iter().collect::<Vec<&TrackedRow>>(),
            TableKind::Command,
            Color::Reset,
            PinnedGroup::Unpinned,
            ProcessTree::Long,
        );

        let header = buffer_line(&buffer, 0);
        let parent_start = header.find(TABLE_HEADERS[PARENT_COLUMN]).unwrap();
        let parent_end = header.find(TABLE_HEADERS[START_COLUMN]).unwrap();
        assert_eq!(
            parent_end - (parent_start + TABLE_HEADERS[PARENT_COLUMN].len()),
            usize::from(TIGHT_TABLE_COLUMN_SPACING)
        );
        assert!(
            header.contains(TABLE_HEADERS[COMPILER_COLUMN]),
            "{header:?}"
        );
        for (y, parent) in [(2, "359762"), (3, "359763")] {
            let line = buffer_line(&buffer, y);
            assert_eq!(line[parent_start..parent_end].trim(), parent, "{line:?}");
        }
    }

    #[test]
    fn a_fitting_command_table_keeps_two_cell_gaps_with_a_compiler() {
        let area = Rect::new(0, 0, 80, 4);
        let mut buffer = Buffer::empty(area);
        let rows = [tight_command_row(359_829, 359_762, &["build"])];

        draw_process_table(
            &mut buffer,
            area,
            &rows.iter().collect::<Vec<&TrackedRow>>(),
            TableKind::Command,
            Color::Reset,
            PinnedGroup::Unpinned,
            ProcessTree::Long,
        );

        let header = buffer_line(&buffer, 0);
        let parent = header.find(TABLE_HEADERS[PARENT_COLUMN]).unwrap();
        let start = header.find(TABLE_HEADERS[START_COLUMN]).unwrap();
        assert_eq!(
            start - (parent + TABLE_HEADERS[PARENT_COLUMN].len()),
            usize::from(TABLE_COLUMN_SPACING)
        );
        assert!(buffer_line(&buffer, 2).contains(&format!("{}×1", COMPILER_PROCESS_NAMES[0])));
    }

    #[test]
    fn a_summary_command_word_is_not_broken_while_gaps_can_give_cells() {
        let mut row = row(None);
        row.process.pid = 359_829;
        row.process.invocation_id = InvocationId::for_test(359_829);
        row.process.duration = "18s".to_string();
        row.process.command = CommandText::of("cargo", &["metadata"]);
        let rows = [&row];
        let interior_width: u16 = 30;
        let area = Rect::new(
            0,
            0,
            interior_width.saturating_add(cell_width(SECTION_ITEM_INDENT)),
            6,
        );
        let layout = TableLayout::of(
            &rows,
            TableKind::Summary,
            area,
            Color::Reset,
            ProcessTree::Long,
        );
        let buffer = narrow_table_buffer(&rows, TableKind::Summary, interior_width, area.height);
        let command_lines: Vec<String> = (2..area.height)
            .map(|y| table_column_text(&buffer, &layout, COMMAND_COLUMN, y))
            .map(|line| line.trim().to_string())
            .take_while(|line| !line.is_empty())
            .collect();

        assert_eq!(command_lines, ["cargo", "metadata"]);
    }

    #[test]
    fn a_two_word_command_stays_on_one_line_when_the_row_has_room() {
        let lead = invocation(4100, &["check"]);
        let rest = ["test", "build", "doc"]
            .into_iter()
            .enumerate()
            .map(|(offset, command)| {
                invocation(
                    4200_u32.saturating_add(u32::try_from(offset).unwrap()),
                    &[command],
                )
            })
            .collect();
        let roster = roster_with_ancestry(lead, rest, Vec::new());
        let group = roster.groups().first().unwrap();
        let inner_width = 64;
        let content_rows = group_height(group, inner_width, &[], ProcessTree::Long);
        let inner = Rect::new(
            0,
            0,
            inner_width,
            u16::try_from(content_rows).unwrap().saturating_add(1),
        );
        let mut buffer = Buffer::empty(inner);

        draw_cell_for_test(
            &mut buffer,
            &roster,
            &TileContent::Group(group.id.clone()),
            inner,
            content_rows,
        );

        let rows = buffer_rows(&buffer);
        for command in ["cargo check", "cargo test", "cargo build", "cargo doc"] {
            assert!(
                rows.iter().any(|row| row.contains(command)),
                "{command}: {rows:#?}"
            );
        }
        assert!(
            rows.last().is_some_and(|row| row.contains("content rows:")),
            "{rows:#?}"
        );
    }

    /// The `command` column is the one that absorbs the slack, so what
    /// it is worth has to come off the solved layout rather than off the
    /// `Min` it is declared with.
    #[test]
    fn the_command_column_is_measured_at_the_width_it_absorbs() {
        let mut row = long_row();
        row.process.memory = Measurement::Reading(BYTES_PER_GIBIBYTE * 124 / 10);
        row.process.compiler = CompilerObservation::Running(Compiler {
            name:  COMPILER_PROCESS_NAMES[0],
            count: 1,
        });
        row.process.managed = Measurement::Reading(1);
        let rows = [row];
        let rows: Vec<&TrackedRow> = rows.iter().collect();
        let columns = visible_columns(&rows, TableKind::Command);
        let constraints = fitted_constraints(&rows, &columns);

        let command_position = columns
            .iter()
            .position(|column| *column == COMMAND_COLUMN)
            .unwrap();
        let command_width =
            longest_command_width(&rows, TableKind::Command.detail(), ProcessTree::Long);
        let narrow_spacing = table_column_spacing(65, &constraints, &columns, command_width);
        let wide_spacing = table_column_spacing(95, &constraints, &columns, command_width);
        let narrow = solved_column_widths(65, &constraints, narrow_spacing)[command_position];
        let wide = solved_column_widths(95, &constraints, wide_spacing)[command_position];

        assert!(narrow > cell_width(TABLE_HEADERS[COMMAND_COLUMN]));
        assert_eq!(wide.saturating_sub(narrow), 30);
    }

    #[test]
    fn command_width_joins_displayed_words_with_one_space() {
        let mut row = row(None);
        row.process.command = CommandText::of(" cargo ", &["build  ", "", "--workspace"]);

        assert_eq!(
            command_line_width(&row, SummaryDetail::Full, ProcessTree::Long),
            cell_width("cargo build --workspace")
        );
    }

    /// A command that outruns its column carries on down the rows of
    /// that column: every line after the first starts where the column
    /// starts, and nothing of it is dropped.
    #[test]
    fn a_long_command_wraps_within_its_own_column() {
        let area = Rect::new(0, 0, 65, 8);
        let mut buffer = Buffer::empty(area);
        let rows = [tight_command_row(
            359_829,
            359_762,
            &["build", "--features", "one,two", "--all"],
        )];

        draw_process_table(
            &mut buffer,
            area,
            &rows.iter().collect::<Vec<&TrackedRow>>(),
            TableKind::Command,
            Color::Reset,
            PinnedGroup::Unpinned,
            ProcessTree::Long,
        );

        // The column labels are drawn once at the top of the cell, so
        // the header row is where the column's own left edge is.
        let header = buffer_line(&buffer, 0);
        let left = header.find(TABLE_HEADERS[COMMAND_COLUMN]).unwrap();
        let right = header.find(TABLE_HEADERS[COMPILER_COLUMN]).unwrap();
        // Row zero is the labels and row one the working directory, so
        // the invocation starts on row two and runs to the first blank.
        let lines: Vec<u16> = (2..buffer.area.height)
            .take_while(|y| !buffer_line(&buffer, *y).is_empty())
            .collect();

        assert!(lines.len() > 1, "{lines:#?}");
        for &y in lines.iter().skip(1) {
            let line = buffer_line(&buffer, y);
            assert!(line.len() > left, "{line:?}");
            assert!(line[..left].trim().is_empty(), "{line:?}");
        }
        let command_lines: Vec<String> = lines
            .iter()
            .map(|&y| {
                (left..right)
                    .map(|x| buffer[(u16::try_from(x).unwrap(), y)].symbol())
                    .collect::<String>()
                    .trim()
                    .to_string()
            })
            .collect();
        assert_eq!(command_lines[0], "cargo build");
        assert_eq!(
            command_lines.join(" "),
            "cargo build --features one,two --all"
        );
    }

    #[test]
    fn a_cut_command_row_ends_in_the_mark() {
        let area = Rect::new(0, 0, 65, 3);
        let mut buffer = Buffer::empty(area);
        let rows = [long_row()];

        draw_process_table(
            &mut buffer,
            area,
            &rows.iter().collect::<Vec<&TrackedRow>>(),
            TableKind::Command,
            Color::Reset,
            PinnedGroup::Unpinned,
            ProcessTree::Long,
        );

        assert!(buffer_line(&buffer, 2).ends_with(ELISION));
    }

    #[test]
    fn a_cut_command_marks_the_command_and_leaves_runs_whole() {
        let area = Rect::new(0, 0, 64, 3);
        let mut buffer = Buffer::empty(area);
        let mut row = long_row();
        row.process.compiler = CompilerObservation::Running(Compiler {
            name:  COMPILER_PROCESS_NAMES[0],
            count: 1,
        });
        row.process.managed = Measurement::Reading(1);
        let rows = [&row];
        let layout = TableLayout::of(
            &rows,
            TableKind::Command,
            area,
            Color::Reset,
            ProcessTree::Long,
        );

        draw_process_table(
            &mut buffer,
            area,
            &rows,
            TableKind::Command,
            Color::Reset,
            PinnedGroup::Unpinned,
            ProcessTree::Long,
        );

        assert!(
            table_column_text(&buffer, &layout, COMMAND_COLUMN, 2)
                .trim_end()
                .ends_with(ELISION)
        );
        assert_eq!(
            table_column_text(&buffer, &layout, MANAGED_COLUMN, 2).trim(),
            "1"
        );
    }

    #[test]
    fn a_cut_command_keeps_text_after_a_wide_glyph_in_place() {
        let area = Rect::new(0, 0, 56, 3);
        let mut buffer = Buffer::empty(area);
        let mut row = row(None);
        row.process.command = CommandText::of("cargo", &["界", "alpha", "beta", "gamma", "delta"]);
        let rows = [&row];
        let layout = TableLayout::of(
            &rows,
            TableKind::Command,
            area,
            Color::Reset,
            ProcessTree::Long,
        );

        draw_process_table(
            &mut buffer,
            area,
            &rows,
            TableKind::Command,
            Color::Reset,
            PinnedGroup::Unpinned,
            ProcessTree::Long,
        );

        let command = layout.command_area(area);
        let text = table_column_text(&buffer, &layout, COMMAND_COLUMN, 2);
        assert_eq!(buffer[(command.x.saturating_add(6), 2)].symbol(), "界");
        assert_eq!(buffer[(command.x.saturating_add(9), 2)].symbol(), "a");
        assert!(text.trim_end().ends_with(ELISION), "{text:?}");
    }

    #[test]
    fn a_cut_command_fills_its_column_before_the_mark() {
        let mut row = row(None);
        row.process.command = CommandText::of("cargo", &["nextest", "run"]);
        let rows = [&row];
        let area = (1..=120)
            .map(|width| Rect::new(0, 0, width, 3))
            .find(|area| {
                TableLayout::of(
                    &rows,
                    TableKind::Command,
                    *area,
                    Color::Reset,
                    ProcessTree::Long,
                )
                .column_width(COMMAND_COLUMN)
                    == 11
            })
            .expect("the command column can be eleven cells wide");
        let layout = TableLayout::of(
            &rows,
            TableKind::Command,
            area,
            Color::Reset,
            ProcessTree::Long,
        );
        let mut buffer = Buffer::empty(area);

        draw_process_table(
            &mut buffer,
            area,
            &rows,
            TableKind::Command,
            Color::Reset,
            PinnedGroup::Unpinned,
            ProcessTree::Long,
        );

        assert_eq!(
            table_column_text(&buffer, &layout, COMMAND_COLUMN, 2),
            format!("cargo next{ELISION}")
        );
    }

    #[test]
    fn a_whole_command_keeps_its_free_cells_before_a_table_continuation_mark() {
        let area = Rect::new(0, 0, 198, 6);
        let mut buffer = Buffer::empty(area);
        let mut lead = same_second("/workspace/project", 1_881_177);
        lead.process.command = CommandText::of("cargo", &["nextest", "run"]);
        lead.process.compiler = CompilerObservation::Running(Compiler {
            name:  COMPILER_PROCESS_NAMES[0],
            count: 15,
        });
        lead.process.managed = Measurement::Reading(5);
        let mut child = same_second("/workspace/project", 1_881_284);
        child.process.command = CommandText::of("cargo", &[]);
        child.process.managed = Measurement::Reading(2);
        let mut later = same_second("/workspace/later", 1_881_300);
        later.process.command = CommandText::of("cargo", &["check"]);
        let final_row = same_second("/workspace/final", 1_881_400);
        let rows = [&lead, &child, &later, &final_row];
        let layout = TableLayout::of(
            &rows,
            TableKind::Command,
            area,
            Color::Reset,
            ProcessTree::Long,
        );

        draw_process_table(
            &mut buffer,
            area,
            &rows,
            TableKind::Command,
            Color::Reset,
            PinnedGroup::Lead(GroupingIdentity::from(&lead.process)),
            ProcessTree::Long,
        );

        let command = table_column_text(&buffer, &layout, COMMAND_COLUMN, 5);
        assert!(command.starts_with("cargo check"), "{command:?}");
        assert!(command.ends_with(ELISION), "{command:?}");
        assert!(
            command["cargo check".len()..command.len() - ELISION.len()]
                .chars()
                .all(|character| character == ' ')
        );
        assert!(buffer_line(&buffer, 4).contains("/workspace/later"));
        assert_ne!(buffer_line(&buffer, 5), "");
    }

    #[test]
    fn a_whole_exact_width_command_uses_a_free_row_before_marking_later_rows() {
        let area = Rect::new(0, 0, 62, 4);
        let mut buffer = Buffer::empty(area);
        let mut lead = same_second("/workspace/project", 2_398_793);
        lead.process.parent = VisibleParent::Ancestor(2_398_735);
        lead.process.cpu = Measurement::Reading("1192%".to_string());
        lead.process.memory = Measurement::Reading(BYTES_PER_GIBIBYTE * 98 / 10);
        lead.process.command = CommandText::of("cargo", &["nextest", "run"]);
        lead.process.managed = Measurement::Reading(2);
        let later = same_second("/workspace/later", 2_614_582);
        let rows = [&lead, &later];
        let layout = TableLayout::of(
            &rows,
            TableKind::Command,
            area,
            Color::Reset,
            ProcessTree::Long,
        );

        draw_process_table(
            &mut buffer,
            area,
            &rows,
            TableKind::Command,
            Color::Reset,
            PinnedGroup::Lead(GroupingIdentity::from(&lead.process)),
            ProcessTree::Long,
        );

        assert_eq!(
            layout.column_width(COMMAND_COLUMN),
            cell_width("cargo nextest run")
        );
        assert_eq!(
            table_column_text(&buffer, &layout, COMMAND_COLUMN, 2).trim_end(),
            "cargo nextest"
        );
        let continuation = table_column_text(&buffer, &layout, COMMAND_COLUMN, 3);
        assert!(continuation.starts_with("run"), "{continuation:?}");
        assert!(continuation.ends_with(ELISION), "{continuation:?}");
        assert!(
            continuation["run".len()..continuation.len() - ELISION.len()]
                .chars()
                .all(|character| character == ' ')
        );
    }

    #[test]
    fn a_whole_exact_width_command_yields_its_last_cell_when_no_row_is_free() {
        let area = Rect::new(0, 0, 62, 3);
        let mut buffer = Buffer::empty(area);
        let mut lead = same_second("/workspace/project", 2_398_793);
        lead.process.parent = VisibleParent::Ancestor(2_398_735);
        lead.process.cpu = Measurement::Reading("1192%".to_string());
        lead.process.memory = Measurement::Reading(BYTES_PER_GIBIBYTE * 98 / 10);
        lead.process.command = CommandText::of("cargo", &["nextest", "run"]);
        lead.process.managed = Measurement::Reading(2);
        let later = same_second("/workspace/later", 2_614_582);
        let rows = [&lead, &later];
        let layout = TableLayout::of(
            &rows,
            TableKind::Command,
            area,
            Color::Reset,
            ProcessTree::Long,
        );

        draw_process_table(
            &mut buffer,
            area,
            &rows,
            TableKind::Command,
            Color::Reset,
            PinnedGroup::Lead(GroupingIdentity::from(&lead.process)),
            ProcessTree::Long,
        );

        assert_eq!(
            layout.column_width(COMMAND_COLUMN),
            cell_width("cargo nextest run")
        );
        assert_eq!(
            table_column_text(&buffer, &layout, COMMAND_COLUMN, 2),
            format!("cargo nextest ru{ELISION}")
        );
    }

    #[test]
    fn a_column_header_never_carries_the_cut_mark() {
        let area = Rect::new(0, 0, 198, 1);
        let mut buffer = Buffer::empty(area);
        let row = row(None);

        draw_process_table(
            &mut buffer,
            area,
            &[&row],
            TableKind::Command,
            Color::Reset,
            PinnedGroup::Unpinned,
            ProcessTree::Long,
        );

        let header = buffer_line(&buffer, 0);
        assert!(header.contains(TABLE_HEADERS[COMMAND_COLUMN]), "{header:?}");
        assert!(!header.contains(ELISION), "{header:?}");
    }

    #[test]
    fn a_summary_with_hidden_rows_ends_in_the_mark() {
        let rows: Vec<TrackedRow> = (0..8)
            .map(|offset| {
                let mut row = row(None);
                let pid = 4100_u32.saturating_add(offset);
                row.process.pid = pid;
                row.process.invocation_id = InvocationId::for_test(pid);
                row.process.command = CommandText::of("cargo", &["build"]);
                row
            })
            .collect();
        let rows: Vec<&TrackedRow> = rows.iter().collect();
        let area = Rect::new(0, 0, 80, 7);
        let mut buffer = Buffer::empty(area);

        draw_process_table(
            &mut buffer,
            area,
            &rows,
            TableKind::Summary,
            Color::Reset,
            PinnedGroup::Unpinned,
            ProcessTree::Long,
        );

        assert!(buffer_line(&buffer, 6).ends_with(ELISION));
    }

    #[test]
    fn a_summary_drops_a_heading_that_has_no_process_row() {
        let mut rows: Vec<TrackedRow> = (0..3)
            .map(|offset| {
                let path = format!("/workspace/project-{offset}");
                let mut row = row_at(&path, None);
                let pid = 4100_u32.saturating_add(offset);
                row.process.pid = pid;
                row.process.invocation_id = InvocationId::for_test(pid);
                row.process.directory_identity = WorkingDirectoryIdentity::Absolute(path.into());
                row
            })
            .collect();
        rows[0].process.command = CommandText::of("cargo", &["build"]);
        let rows: Vec<&TrackedRow> = rows.iter().collect();
        let area = Rect::new(0, 0, 80, 6);
        let mut buffer = Buffer::empty(area);

        draw_process_table(
            &mut buffer,
            area,
            &rows,
            TableKind::Summary,
            Color::Reset,
            PinnedGroup::Unpinned,
            ProcessTree::Long,
        );

        let marked = buffer_line(&buffer, 4);
        assert!(!marked.contains("project-2"), "{marked:?}");
        assert!(marked.ends_with(ELISION), "{marked:?}");
        assert_eq!(buffer_line(&buffer, 5), "");
    }

    /// Draw four one-row directory groups in a cell with a readout foot.
    fn four_directory_cell(height: u16) -> Buffer {
        let rows: Vec<TrackedRow> = (0..4)
            .map(|offset| {
                let path = format!("/workspace/project-{offset}");
                let mut row = started_at(&path, None, u64::from(offset));
                let pid = 4100_u32.saturating_add(offset);
                row.process.pid = pid;
                row.process.invocation_id = InvocationId::for_test(pid);
                row.process.directory_identity = WorkingDirectoryIdentity::Absolute(path.into());
                row
            })
            .collect();
        let rows: Vec<&TrackedRow> = rows.iter().collect();
        let area = Rect::new(0, 0, 80, height);
        let mut buffer = Buffer::empty(area);

        tui_pane::draw_tile_cell(
            &mut buffer,
            &TileContent::Summary,
            area,
            12,
            area.width,
            |buffer, area| {
                draw_process_table(
                    buffer,
                    area,
                    &rows,
                    TableKind::Summary,
                    Color::Reset,
                    PinnedGroup::Unpinned,
                    ProcessTree::Long,
                );
            },
        );
        buffer
    }

    #[test]
    fn a_table_that_yields_one_gap_yields_them_all_before_its_foot() {
        let buffer = four_directory_cell(11);
        let drawn = buffer_rows(&buffer).join("\n");

        for (heading_row, process_row, offset) in [(1, 2, 0), (3, 4, 1), (5, 6, 2), (7, 8, 3)] {
            assert!(
                buffer_line(&buffer, heading_row).contains(&format!("project-{offset}")),
                "{drawn}"
            );
            assert!(
                buffer_line(&buffer, process_row).contains(&format!("410{offset}")),
                "{drawn}"
            );
        }
        assert_eq!(buffer_line(&buffer, 9), "", "{drawn}");
        assert!(
            buffer_line(&buffer, 10).contains("content rows:"),
            "{drawn}"
        );
    }

    #[test]
    fn a_fully_drawn_table_keeps_every_inter_group_gap() {
        let buffer = four_directory_cell(13);
        let drawn = buffer_rows(&buffer).join("\n");

        for row in [3, 6, 9] {
            assert_eq!(buffer_line(&buffer, row), "", "{drawn}");
        }
        for (heading_row, process_row, offset) in [(1, 2, 0), (4, 5, 1), (7, 8, 2), (10, 11, 3)] {
            assert!(
                buffer_line(&buffer, heading_row).contains(&format!("project-{offset}")),
                "{drawn}"
            );
            assert!(
                buffer_line(&buffer, process_row).contains(&format!("410{offset}")),
                "{drawn}"
            );
        }
        assert!(
            buffer_line(&buffer, 12).contains("content rows:"),
            "{drawn}"
        );
    }

    #[test]
    fn a_cell_hiding_rows_leaves_no_blank_row_above_its_foot() {
        let fixtures = [
            ("/workspace/project-0", 0),
            ("/workspace/project-0", 1),
            ("/workspace/project-1", 2),
            ("/workspace/project-2", 3),
            ("/workspace/project-3", 4),
        ];
        let rows: Vec<TrackedRow> = fixtures
            .into_iter()
            .map(|(path, offset)| {
                let mut row = started_at(path, None, offset);
                let pid = 4100_u32.saturating_add(u32::try_from(offset).unwrap());
                row.process.pid = pid;
                row.process.invocation_id = InvocationId::for_test(pid);
                row.process.directory_identity =
                    WorkingDirectoryIdentity::Absolute(path.to_string().into());
                row
            })
            .collect();
        let rows: Vec<&TrackedRow> = rows.iter().collect();
        let area = Rect::new(0, 0, 80, 8);
        let layout = TableLayout::of(
            &rows,
            TableKind::Command,
            area,
            Color::Reset,
            ProcessTree::Long,
        );
        let mut buffer = Buffer::empty(area);

        draw_process_table(
            &mut buffer,
            area,
            &rows,
            TableKind::Command,
            Color::Reset,
            PinnedGroup::Unpinned,
            ProcessTree::Long,
        );

        let drawn = buffer_rows(&buffer).join("\n");
        assert!(buffer_line(&buffer, 1).contains("project-0"), "{drawn}");
        assert!(buffer_line(&buffer, 2).contains("4100"), "{drawn}");
        assert!(buffer_line(&buffer, 3).contains("4101"), "{drawn}");
        assert!(buffer_line(&buffer, 4).contains("project-1"), "{drawn}");
        assert!(buffer_line(&buffer, 5).contains("4102"), "{drawn}");
        assert!(buffer_line(&buffer, 6).contains("project-2"), "{drawn}");
        assert!(buffer_line(&buffer, 7).contains("4103"), "{drawn}");
        assert!(table_column_text(&buffer, &layout, COMMAND_COLUMN, 7).ends_with(ELISION));
        assert!(!drawn.contains("project-3"), "{drawn}");
    }

    #[test]
    fn inter_group_gaps_yield_earliest_first_to_show_one_more_group() {
        let rows: Vec<TrackedRow> = (0..4)
            .map(|offset| {
                let path = format!("/workspace/project-{offset}");
                let mut row = started_at(&path, None, u64::from(offset));
                let pid = 4100_u32.saturating_add(offset);
                row.process.pid = pid;
                row.process.invocation_id = InvocationId::for_test(pid);
                row.process.directory_identity = WorkingDirectoryIdentity::Absolute(path.into());
                row
            })
            .collect();
        let rows: Vec<&TrackedRow> = rows.iter().collect();
        let area = Rect::new(0, 0, 80, 7);
        let layout = TableLayout::of(
            &rows,
            TableKind::Command,
            area,
            Color::Reset,
            ProcessTree::Long,
        );
        let mut buffer = Buffer::empty(area);

        draw_process_table(
            &mut buffer,
            area,
            &rows,
            TableKind::Command,
            Color::Reset,
            PinnedGroup::Unpinned,
            ProcessTree::Long,
        );

        let drawn = buffer_rows(&buffer).join("\n");
        assert!(buffer_line(&buffer, 1).contains("project-0"), "{drawn}");
        assert!(buffer_line(&buffer, 2).contains("4100"), "{drawn}");
        assert!(buffer_line(&buffer, 3).contains("project-1"), "{drawn}");
        assert!(buffer_line(&buffer, 4).contains("4101"), "{drawn}");
        assert!(buffer_line(&buffer, 5).contains("project-2"), "{drawn}");
        assert!(buffer_line(&buffer, 6).contains("4102"), "{drawn}");
        assert!(table_column_text(&buffer, &layout, COMMAND_COLUMN, 6).ends_with(ELISION));
        assert!(!drawn.contains("project-3"), "{drawn}");
    }

    #[test]
    fn a_wrapped_summary_command_cut_at_the_foot_ends_in_the_mark() {
        let mut row = row(None);
        row.process.pid = 1_359_829;
        row.process.invocation_id = InvocationId::for_test(1_359_829);
        row.process.command = CommandText::of("cargo", &["clippy"]);
        let rows = [&row];
        let area = Rect::new(0, 0, 40, 3);
        let mut buffer = Buffer::empty(area);

        draw_process_table(
            &mut buffer,
            area,
            &rows,
            TableKind::Summary,
            Color::Reset,
            PinnedGroup::Unpinned,
            ProcessTree::Long,
        );

        let last = buffer_line(&buffer, 2);
        assert!(last.ends_with(&format!("cargo clip{ELISION}")), "{last:?}");
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
            PinnedGroup::Unpinned,
            ProcessTree::Long,
        );

        let header = buffer_line(&buffer, 0);
        let left = header.find(TABLE_HEADERS[COMMAND_COLUMN]).unwrap();
        for y in 2..6 {
            let line = buffer_line(&buffer, y);
            assert!(line.len() > left, "row {y} is empty: {line:?}");
        }
    }
    #[test]
    fn a_blocked_state_has_no_reading_to_draw() {
        assert_eq!(
            CounterState::from(&CaptureLookup::Registered(CaptureRead::Progress(
                RunState::Blocked
            ))),
            CounterState::Blocked
        );
    }

    /// Fading the grid carries each cell's text half way toward the colour
    /// that cell is painted on, a cell painted on nothing toward the attract
    /// ground, and leaves every cell outside the faded area as it was.
    #[test]
    fn a_faded_grid_carries_each_cell_toward_its_own_background() {
        const WIDTH: u16 = 40;
        const HEIGHT: u16 = 6;
        const PAINTED_ON_NOTHING: (u16, u16) = (20, 3);
        // The cells stand on a painted tint, which a transparent screen leaves off.
        tui_pane::set_transparent_background(false);
        let painted: [(Color, Color); HEIGHT as usize] = [
            (Color::Rgb(200, 100, 50), Color::Rgb(10, 20, 30)),
            (Color::Rgb(200, 100, 50), Color::Rgb(10, 20, 30)),
            (Color::White, Color::Rgb(0, 0, 0)),
            (Color::Rgb(255, 255, 255), Color::Indexed(236)),
            (Color::Reset, Color::Rgb(40, 40, 40)),
            (Color::Rgb(90, 180, 30), Color::Rgb(250, 250, 250)),
        ];
        let mut buffer = Buffer::empty(Rect::new(0, 0, WIDTH, HEIGHT));
        for (row, &(fg, bg)) in (0..).zip(&painted) {
            for column in 0..WIDTH {
                buffer
                    .cell_mut((column, row))
                    .expect("the cell is inside the buffer")
                    .set_fg(fg)
                    .set_bg(bg);
            }
        }
        buffer
            .cell_mut(PAINTED_ON_NOTHING)
            .expect("the cell is inside the buffer")
            .set_bg(Color::Reset);
        let area = Rect::new(4, 1, 32, 4);

        fade_to_background(&mut buffer, area, 128);

        // What each row of `area` fades to, top to bottom, and what the
        // one cell painted on nothing fades to.
        let faded = [
            Color::Rgb(104, 59, 39),
            Color::Rgb(127, 127, 127),
            Color::Rgb(151, 151, 151),
            Color::Reset,
        ];
        let faded_on_nothing = Color::Rgb(134, 134, 135);
        let expected: Vec<Color> = (0..HEIGHT)
            .flat_map(|row| (0..WIDTH).map(move |column| (column, row)))
            .map(|(column, row)| {
                if (column, row) == PAINTED_ON_NOTHING {
                    faded_on_nothing
                } else if area.contains((column, row).into()) {
                    faded[usize::from(row - area.top())]
                } else {
                    painted[usize::from(row)].0
                }
            })
            .collect();
        let actual: Vec<Color> = buffer.content.iter().map(|cell| cell.fg).collect();
        assert_eq!(actual, expected);
    }

    /// A whole frame three frames after the attract screen is asked for
    /// over an idle grid: the panes stand bare while it arrives, the
    /// status line goes back on top, and no backdrop notice is due yet.
    #[test]
    fn an_arriving_attract_screen_draws_the_pinned_frame() {
        const WIDTH: u16 = 80;
        const HEIGHT: u16 = 24;
        let mut app = App::new_for_test().expect("test app should build");
        app.attract.toggle();
        app.started = Instant::now();
        let keymap = std::rc::Rc::clone(&app.keymap);
        let mut terminal =
            Terminal::new(TestBackend::new(WIDTH, HEIGHT)).expect("the test terminal opens");
        for _ in 0..3 {
            terminal
                .draw(|frame| draw(frame, &mut app, &keymap))
                .expect("the test terminal draws");
        }

        let inner = usize::from(WIDTH) - 2;
        let title = " summary";
        let notes = format!("cargo-tile {APP_VERSION} attract  ? shortcuts ");
        let mut expected = vec![format!(
            "┌{title}{}┐",
            "─".repeat(inner - title.chars().count())
        )];
        expected.extend((0..HEIGHT - 3).map(|_| format!("│{}│", " ".repeat(inner))));
        expected.push(format!("└{}┘", "─".repeat(inner)));
        expected.push(format!(
            "{:<width$}{notes}",
            " Uptime: 0s",
            width = usize::from(WIDTH) - notes.chars().count()
        ));
        assert_eq!(buffer_rows(terminal.backend().buffer()), expected);
    }

    /// Width of the frame the tail test draws.
    const FRAME_TAIL_WIDTH: u16 = 60;
    /// Height of the frame the tail test draws.
    const FRAME_TAIL_HEIGHT: u16 = 14;

    /// The frames the tail test pins: the favorites overlay alone, then
    /// each framework overlay.
    const FRAME_TAIL_CASES: [(Option<GlobalAction>, [&str; FRAME_TAIL_HEIGHT as usize]); 4] = [
        (
            None,
            [
                "┌ summary──────────────────────────────────────────────────┐",
                "│                                                          │",
                "│ ┌ toast title ─────────────────────────────────────[x]─┐ │",
                "│ │ the toast's body                                     │ │",
                "│ │                                                      │ │",
                "│ ┌ Favorites -- 0 saved -- ● matches the current paramet┐ │",
                "│ │No favorites saved -- press Esc, then ⌃s while the att│ │",
                "│ │Esc close                                             │ │",
                "│ └──────────────────────────────────────────────────────┘ │",
                "│ │                                                      │ │",
                "│ │                                                      │ │",
                "│ └──────────────────────────────────────────────────────┘ │",
                "└──────────────────────────────────────────────────────────┘",
                " Uptime: 0s              cargo-tile <version>  ? shortcuts ",
            ],
        ),
        (
            Some(GlobalAction::OpenSettings),
            [
                "┌ Settings ────────────────────────────────────────────────┐",
                "│ Appearance:                                              │",
                "│ ▶ mode              < auto >                             │",
                "│   light theme       < Default Light >                    │",
                "│   dark theme        < Default Dark >                     │",
                "│   transparent       < true >                             │",
                "│ Tiles:                                                   │",
                "│   initial rows      < 4 >                                │",
                "│   fill              < redistribute >                     │",
                "│   view              < auto >                             │",
                "│   widen summary     < false >                            │",
                "│   fade seconds      < 3 >                                │",
                "│ Capture:                                                 │",
                "└──────────────────────────────────────────────────────────┘",
            ],
        ),
        (
            Some(GlobalAction::OpenKeymap),
            [
                "┌ summary──────────────────────────────────────────────────┐",
                "│ ┌ Keymap ──────────────────────────────────────────────┐ │",
                "│ │                                                      │ │",
                "│ │ Global Navigation:                                   │ │",
                "│ │ ▸ Next pane                                     tab  │ │",
                "│ │   Previous pane                                 shift│ │",
                "│ │ Global Shortcuts:                                    │ │",
                "│ │   Add a tile                                    +    │ │",
                "│ │   Dismiss overlay / output                      x    │ │",
                "│ │   Focus the tile above                          up   │ │",
                "│ │   Focus the tile below                          down │ │",
                "│ └──────────────────────1 of 11 ▼───────────────────────┘ │",
                "└──────────────────────────────────────────────────────────┘",
                " Uptime: 0s              cargo-tile <version>  ? shortcuts ",
            ],
        ),
        (
            Some(GlobalAction::OpenGlobalShortcuts),
            [
                "┌ summary──────────────────────────────────────────────────┐",
                "│    ┌ Global Shortcuts ─────────────────────────────┐     │",
                "│ ┌ t│                                               │x]─┐ │",
                "│ │ t│ Global Navigation:                            │   │ │",
                "│ │  │ ▸ Next pane                         tab       │   │ │",
                "│ │  │   Previous pane                     shift-tab │   │ │",
                "│ │  │ Global Shortcuts:                             │   │ │",
                "│ │  │   Add a tile                        +         │   │ │",
                "│ │  │   Focus the tile above              up        │   │ │",
                "│ │  │   Focus the tile below              down      │   │ │",
                "│ │  │   Focus the tile to the left        left      │   │ │",
                "│ └──│   Focus the tile to the right       right     │───┘ │",
                "└────└───────────────────1 of 3 ▼────────────────────┘─────┘",
                " Uptime: 0s              cargo-tile <version>  ? shortcuts ",
            ],
        ),
    ];

    #[test]
    fn a_toast_never_covers_the_status_line() {
        let mut app = App::new_for_test().expect("test app should build");
        app.started = Instant::now();
        app.framework.toasts.push_persistent(
            "toast title",
            "the toast's body",
            ToastStyle::Normal,
            None,
            8,
        );
        let keymap = std::rc::Rc::clone(&app.keymap);
        let mut terminal = Terminal::new(TestBackend::new(FRAME_TAIL_WIDTH, FRAME_TAIL_HEIGHT))
            .expect("the test terminal opens");

        terminal
            .draw(|frame| draw(frame, &mut app, &keymap))
            .expect("the test terminal draws");

        let status = buffer_line(
            terminal.backend().buffer(),
            FRAME_TAIL_HEIGHT.saturating_sub(1),
        );
        assert!(
            status.contains(&format!("cargo-tile {APP_VERSION}")),
            "{status:?}"
        );
        assert!(status.contains("? shortcuts"), "{status:?}");
    }

    /// Toasts float over cell contents, not over the outer frame that
    /// separates the grid from the terminal edge and status line.
    #[test]
    fn a_toast_stays_inside_the_grid_right_and_bottom_borders() {
        let mut app = App::new_for_test().expect("test app should build");
        app.started = Instant::now();
        let keymap = std::rc::Rc::clone(&app.keymap);
        let mut terminal = Terminal::new(TestBackend::new(FRAME_TAIL_WIDTH, FRAME_TAIL_HEIGHT))
            .expect("the test terminal opens");
        terminal
            .draw(|frame| draw(frame, &mut app, &keymap))
            .expect("the frame without a toast draws");
        let bare = terminal.backend().buffer().clone();

        app.framework.toasts.push_persistent(
            "toast title",
            "the toast's body",
            ToastStyle::Normal,
            None,
            8,
        );
        terminal
            .draw(|frame| draw(frame, &mut app, &keymap))
            .expect("the frame with a toast draws");
        let with_toast = terminal.backend().buffer();
        let border_right = FRAME_TAIL_WIDTH.saturating_sub(1);
        let border_bottom = FRAME_TAIL_HEIGHT.saturating_sub(2);

        for row in 0..=border_bottom {
            assert_eq!(
                with_toast[(border_right, row)].symbol(),
                bare[(border_right, row)].symbol(),
                "right border row {row}",
            );
        }
        for column in 0..FRAME_TAIL_WIDTH {
            assert_eq!(
                with_toast[(column, border_bottom)].symbol(),
                bare[(column, border_bottom)].symbol(),
                "bottom border column {column}",
            );
        }
    }

    /// What follows the grid in a frame: body-bound toasts, then the
    /// favorites overlay, then whichever framework overlay is open.
    #[test]
    fn the_frame_ends_with_toasts_then_favorites_then_the_framework_overlay() {
        // Tall enough to reach up under every popup, so the frame shows
        // which of the two is drawn over the other.
        const TOAST_LINES: usize = 8;
        for (opener, expected) in FRAME_TAIL_CASES {
            let mut app = App::new_for_test().expect("test app should build");
            app.started = Instant::now();
            app.framework.toasts.push_persistent(
                "toast title",
                "the toast's body",
                ToastStyle::Normal,
                None,
                TOAST_LINES,
            );
            let keymap = std::rc::Rc::clone(&app.keymap);
            match opener {
                Some(action) => keymap.dispatch_framework_global(action, &mut app),
                None => tui_pane::open_favorites_on_state_for_test(
                    &mut app,
                    FavoritesFileState::Missing {
                        path: PathBuf::from("/tmp/favorites.toml"),
                    },
                ),
            }
            let mut terminal = Terminal::new(TestBackend::new(FRAME_TAIL_WIDTH, FRAME_TAIL_HEIGHT))
                .expect("the test terminal opens");
            terminal
                .draw(|frame| draw(frame, &mut app, &keymap))
                .expect("the test terminal draws");

            let actual: Vec<String> = buffer_rows(terminal.backend().buffer())
                .into_iter()
                .map(|row| row.replace(APP_VERSION, "<version>"))
                .collect();
            assert_eq!(actual, expected, "{opener:?}");
        }
    }

    /// Each backdrop notice is written on the last row of the body in
    /// the label colour, leaving every other row of the frame as it was,
    /// and no notice leaves the frame blank.
    #[test]
    fn each_backdrop_notice_is_written_on_the_last_body_row() {
        const WIDTH: u16 = 128;
        const HEIGHT: u16 = 4;
        let cases = [
            (BackdropNotice::None, ""),
            (
                BackdropNotice::ScreenRecordingAccessInstruction,
                "attract: no desktop capture -- allow Screen Recording for this terminal in System Settings \u{203a} Privacy & Security",
            ),
            (
                BackdropNotice::CaptureStalled,
                "attract: desktop capture stalled -- retrying with a replacement capture worker",
            ),
            (
                BackdropNotice::CaptureRecoveryStopped,
                "attract: desktop capture recovery stopped -- worker replacement limit reached",
            ),
            (
                BackdropNotice::CaptureUnavailable,
                "attract: desktop capture unavailable -- set CARGO_TILE_FRAME_LOG to record why",
            ),
        ];
        let blank = " ".repeat(usize::from(WIDTH));
        for (notice, text) in cases {
            let buffer = drawn_backdrop_notice(notice, WIDTH, HEIGHT);
            let expected = [
                blank.clone(),
                blank.clone(),
                format!("{text:<width$}", width = usize::from(WIDTH)),
                blank.clone(),
            ];
            assert_eq!(buffer_rows(&buffer), expected, "{notice:?}");
            for (column, character) in (0..).zip(text.chars()) {
                let cell = buffer
                    .cell((column, 2))
                    .expect("the cell is inside the buffer");
                assert_eq!(
                    cell.fg,
                    label_color(),
                    "{notice:?} at column {column} ({character})"
                );
            }
        }
    }

    /// A terminal too narrow for the whole notice gets as much of it as
    /// fits on the one row rather than a second line over the grid.
    #[test]
    fn a_narrow_body_cuts_the_backdrop_notice_at_its_edge() {
        let buffer = drawn_backdrop_notice(BackdropNotice::CaptureStalled, 40, 3);
        assert_eq!(
            buffer_rows(&buffer),
            [
                " ".repeat(40),
                "attract: desktop capture stalled -- retr".to_string(),
                " ".repeat(40),
            ]
        );
    }

    /// Draw `notice` into a `width` by `height` frame whose body is every
    /// row but the last, which stands in for the status line.
    fn drawn_backdrop_notice(notice: BackdropNotice, width: u16, height: u16) -> Buffer {
        let mut terminal =
            Terminal::new(TestBackend::new(width, height)).expect("the test terminal opens");
        terminal
            .draw(|frame| {
                draw_backdrop_notice::<FrameLog>(frame, notice, Rect::new(0, 0, width, height - 1));
            })
            .expect("the test terminal draws");
        terminal.backend().buffer().clone()
    }

    /// Every row of `buffer`, top to bottom, as the text it shows.
    fn buffer_rows(buffer: &Buffer) -> Vec<String> {
        let area = buffer.area;
        (area.top()..area.bottom())
            .map(|row| {
                (area.left()..area.right())
                    .filter_map(|column| buffer.cell((column, row)).map(Cell::symbol))
                    .collect()
            })
            .collect()
    }
}
