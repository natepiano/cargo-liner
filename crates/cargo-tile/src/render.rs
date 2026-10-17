//! Frame rendering: the app's panes, the framework status line along the
//! bottom, and whichever framework overlay is open above them.

use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;
use std::sync::OnceLock;

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Constraint;
use ratatui::layout::Flex;
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
use tui_pane::CellContentExtent;
use tui_pane::CellFadeProgress;
use tui_pane::CellReadoutFade;
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
use crate::census::command_text::ArgumentOmission;
use crate::census::command_text::CargoProfile;
use crate::census::command_text::CommandName;
use crate::census::command_text::CommandText;
use crate::census::scan::AncestryWalk;
use crate::census::scan::InvocationResourceUse;
use crate::constants::ACCOUNT_HEADING_CLOSE;
use crate::constants::ACCOUNT_HEADING_OPEN;
use crate::constants::ANCESTRY_GAP_HEIGHT;
use crate::constants::ANCESTRY_LEVEL_INDENT;
use crate::constants::ANCESTRY_MIN_COMMAND_WIDTH;
use crate::constants::APP_NAME;
use crate::constants::APP_VERSION;
use crate::constants::ATTRACT_NOTE_LABEL;
use crate::constants::BYTES_PER_GIBIBYTE;
use crate::constants::COMMAND_COLUMN;
use crate::constants::COMPILER_COLUMN;
use crate::constants::CONTENT_RIGHT_INSET;
use crate::constants::CPU_COLUMN;
use crate::constants::DURATION_COLUMN;
use crate::constants::ELISION;
use crate::constants::EXEC_WRAPPER_PROCESS_NAMES;
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
use crate::constants::PROFILE_LABEL_CLOSE;
use crate::constants::PROFILE_LABEL_OPEN;
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
use crate::constants::TABLE_CONTINUATION_CLEARANCE;
use crate::constants::TABLE_HEADER_HEIGHT;
use crate::constants::TABLE_HEADERS;
use crate::constants::TIGHT_TABLE_COLUMN_SPACING;
use crate::constants::TRANSPARENT_PROCESS_NAMES;
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
            Self::NoRunningCommands | Self::NoReadableCommands => return SummaryFoot::Empty,
            Self::Complete(bytes) => memory_label(bytes),
            Self::AtLeast(bytes) => format!("{}{PARTIAL_TOTAL_MARK}", memory_label(bytes)),
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
    let mut summary_frames = DrawnSummaryFrames(Vec::new());
    draw_attract_layers(frame, app, body, work, updates, |frame, app, contents| {
        summary_frames = draw_panes(frame, app, body, contents);
    });
    draw_status_line(frame, app, keymap, status);
    let toast_area = app
        .framework
        .toasts
        .area_avoiding_frames(tui_pane::frame_inner(body), &summary_frames.0);
    tui_pane::render_toasts(frame, &mut app.framework, toast_area);
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
fn draw_panes(
    frame: &mut Frame,
    app: &mut App,
    area: Rect,
    contents: TileGridContents,
) -> DrawnSummaryFrames {
    let growth = app.loaded_config.config.tiles.growth();
    let cells = Cells::for_frame(
        &app.roster,
        &app.loaded_config.config.commands.hidden_when_idle,
        app.tree,
        &app.sccache,
        area.height.saturating_sub(3),
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
    DrawnSummaryFrames(app.tiles.painted_summary_frames().to_vec())
}

/// Summary frames resolved and painted by this draw's grid snapshot.
struct DrawnSummaryFrames(Vec<Rect>);

/// What the tile grid's cells hold, borrowed out of [`App`] for one
/// frame.
struct Cells<'a> {
    /// Every command group, which the summary gathers and each group's
    /// own cell draws.
    roster:           &'a Roster,
    /// `commands.hidden_when_idle`, which settles whether a driver draws
    /// as a row or as the last step of its chain.
    hidden_when_idle: &'a [String],
    /// What sccache reports, written along the summary cell's top border.
    sccache:          &'a SccacheStats,
    /// Content-row answers already computed during this draw.
    row_uses:         RefCell<HashMap<(CellRowIdentity, u16), PreparedCellRows>>,
    /// Intrinsic column inputs shared by every allocation in this frame.
    grids:            FrameColumnGrids,
}

impl<'a> Cells<'a> {
    #[cfg(test)]
    fn new(
        roster: &'a Roster,
        hidden_when_idle: &'a [String],
        tree: ProcessTree,
        sccache: &'a SccacheStats,
    ) -> Self {
        Self::for_frame(roster, hidden_when_idle, tree, sccache, u16::MAX)
    }

    /// Cell provider with an empty cache for this draw.
    fn for_frame(
        roster: &'a Roster,
        hidden_when_idle: &'a [String],
        tree: ProcessTree,
        sccache: &'a SccacheStats,
        height: u16,
    ) -> Self {
        Self {
            roster,
            hidden_when_idle,
            sccache,
            row_uses: RefCell::new(HashMap::new()),
            grids: FrameColumnGrids::for_frame(roster, hidden_when_idle, tree, height),
        }
    }

    /// Reuse the width-only layout across all heights considered in this draw.
    fn prepared_rows<T>(
        &self,
        content: &TileContent,
        width: u16,
        read: impl FnOnce(&PreparedCellRows) -> T,
    ) -> T {
        let mut layouts = self.row_uses.borrow_mut();
        let prepared = layouts
            .entry((CellRowIdentity::from(content), width))
            .or_insert_with(|| {
                PreparedCellRows::new(
                    self.roster,
                    content,
                    width,
                    self.hidden_when_idle,
                    &self.grids,
                )
            });
        read(prepared)
    }

    /// Painted and retained rows at one height, from the prepared width layout.
    fn row_use(&self, content: &TileContent, area: Rect) -> GroupRowUse {
        self.prepared_rows(content, area.width, |rows| rows.at(area.height))
    }
}

impl TileCells<InvocationId> for Cells<'_> {
    fn summary_title(&self) -> &str { SUMMARY_CELL_TITLE }

    fn prepare_frame(&self, allocations: impl FnOnce() -> Vec<(TileContent, Rect)>) {
        self.grids.summary.0.row_end.set(TableRowEnd::Unreserved);
        self.grids.commands.0.row_end.set(TableRowEnd::Unreserved);
        self.grids.commands.0.cell_gap.set(CellTableGap::Unprepared);
        self.grids
            .commands
            .0
            .whole_short_tables
            .borrow_mut()
            .clear();
        let mut summary_cut = false;
        let mut commands_cut = false;
        let allocations = allocations();
        self.grids.commands.0.layouts.borrow_mut().clear();
        self.grids.summary.0.layouts.borrow_mut().clear();
        for (content, area) in &allocations {
            let area = *area;
            match content {
                TileContent::Summary => {
                    let rows = summary_rows(self.roster, self.hidden_when_idle);
                    let rows: Vec<_> = rows.iter().map(AsRef::as_ref).collect();
                    summary_cut |= table_needs_continuation(
                        &self.grids.summary.0,
                        area,
                        &rows,
                        PinnedGroup::Unpinned,
                    );
                },
                TileContent::Group(id) => {
                    let Some(group) = self.roster.groups().iter().find(|group| &group.id == id)
                    else {
                        continue;
                    };
                    let leads = group.leads_as_ancestor(self.hidden_when_idle);
                    let rows: Vec<_> = group.rows().skip(usize::from(leads)).collect();
                    let grid = &self.grids.commands.0;
                    let demand = frame_table_height(
                        &rows,
                        area.width,
                        PinnedGroup::Lead(&group.lead.process),
                        grid,
                    );
                    let ancestry = drawn_ancestry(group, leads, grid.tree);
                    let used = ancestry_source_row_use(
                        AncestrySource::for_group(&ancestry, group, leads, grid.tree),
                        area,
                        demand,
                    )
                    .drawn;
                    let table = Rect {
                        y: area.y.saturating_add(used),
                        height: area.height.saturating_sub(used),
                        ..area
                    };
                    let table_grid = table_grid_after_ancestry(grid, used);
                    let pinned = PinnedGroup::Lead(&group.lead.process);
                    let cut = table_needs_continuation(&table_grid, table, &rows, pinned);
                    commands_cut |= cut;
                    if !cut && grid.tree == ProcessTree::Short {
                        preserve_whole_short_table(&table_grid, table, &rows);
                    }
                },
                TileContent::Empty(_) => {},
            }
        }
        self.grids.summary.0.row_end.set(if summary_cut {
            TableRowEnd::Reserved
        } else {
            TableRowEnd::Unreserved
        });
        self.grids.commands.0.row_end.set(if commands_cut {
            TableRowEnd::Reserved
        } else {
            TableRowEnd::Unreserved
        });
        self.grids
            .commands
            .0
            .frame_row_end
            .set(self.grids.commands.0.row_end.get());
        let grid = &self.grids.commands.0;
        let spacing = allocations
            .iter()
            .filter(|(content, _)| matches!(content, TileContent::Group(_)))
            .map(|(_, area)| grid.cell_column_spacing(indented(*area).width))
            .min()
            .unwrap_or(TABLE_COLUMN_SPACING);
        grid.cell_gap.set(CellTableGap::Settled(spacing));
        self.grids.commands.0.layouts.borrow_mut().clear();
        self.row_uses.borrow_mut().clear();
        #[cfg(test)]
        tests::ROW_USE_LAYOUTS.with(|layouts| layouts.borrow_mut().clear());
    }

    fn summary_foot(&self) -> SummaryFoot { memory_total(self.roster).foot() }

    fn content_extent(&self, content: &TileContent) -> CellContentExtent {
        let cut = match content {
            TileContent::Group(id) => self.roster.groups().iter().any(|group| {
                &group.id == id && matches!(group.ancestry(), AncestryWalk::LimitCut(_))
            }),
            TileContent::Summary | TileContent::Empty(_) => false,
        };
        if cut {
            CellContentExtent::CutAtSource
        } else {
            CellContentExtent::Measured
        }
    }

    fn readout_rows(&self, content: &TileContent, area: Rect, measured_rows: usize) -> usize {
        if !matches!(content, TileContent::Summary) {
            return measured_rows;
        }
        measured_rows.saturating_sub(usize::from(
            self.row_use(content, area).separator_rows_yielded,
        ))
    }

    fn readout_fade(&self, content: &TileContent) -> CellReadoutFade {
        let faded = match content {
            TileContent::Summary => {
                let rows = summary_rows(self.roster, self.hidden_when_idle);
                heading_fade(&rows.iter().map(AsRef::as_ref).collect::<Vec<_>>())
            },
            TileContent::Group(id) => {
                let Some(group) = self.roster.groups().iter().find(|group| &group.id == id) else {
                    return CellReadoutFade::Unfaded;
                };
                let rows: Vec<_> = group
                    .rows()
                    .skip(usize::from(group.leads_as_ancestor(self.hidden_when_idle)))
                    .collect();
                group_fade(group, &rows)
            },
            TileContent::Empty(_) => return CellReadoutFade::Unfaded,
        };
        match faded {
            0 => CellReadoutFade::Unfaded,
            amount => CellReadoutFade::Fading(CellFadeProgress::new(amount)),
        }
    }

    fn demands(&self, widths: &[(TileContent, u16)]) -> TileDemands {
        #[cfg(test)]
        tests::FRAME_DEMAND_QUERIES.with(|queries| queries.set(queries.get().saturating_add(1)));
        let narrowest = widths
            .iter()
            .map(|(_, width)| *width)
            .min()
            .unwrap_or_default();
        let width_of = |content: &TileContent| {
            widths
                .iter()
                .find(|(known, _)| known == content)
                .map_or(narrowest, |(_, width)| *width)
        };
        let summary = summary_rows(self.roster, self.hidden_when_idle);
        let summary: Vec<_> = summary.iter().map(AsRef::as_ref).collect();
        TileDemands {
            summary:       self.prepared_rows(
                &TileContent::Summary,
                width_of(&TileContent::Summary),
                PreparedCellRows::demand,
            ),
            summary_width: frame_summary_width(&summary, &self.grids.summary.0),
            groups:        self
                .roster
                .tiled_ids(self.hidden_when_idle)
                .into_iter()
                .map(|id| {
                    let content = TileContent::Group(id.clone());
                    TileDemand {
                        id,
                        rows: self.prepared_rows(
                            &content,
                            width_of(&content),
                            PreparedCellRows::demand,
                        ),
                    }
                })
                .collect(),
        }
    }

    fn rows_drawn(&self, content: &TileContent, area: Rect) -> u16 {
        self.row_use(content, area).drawn
    }

    fn rows_kept(&self, content: &TileContent, area: Rect) -> u16 {
        self.row_use(content, area).kept
    }

    fn draw(&self, buffer: &mut Buffer, content: &TileContent, inner: Rect, ground: Color) {
        frame_draw_contents(
            buffer,
            self.roster,
            content,
            inner,
            ground,
            self.hidden_when_idle,
            &self.grids,
        );
    }

    fn summary_labels(&self, rect: Rect) -> Vec<PaneFrameLabel> {
        sccache_label(self.sccache, rect)
    }
}

/// Stable cell identity for the frame's indexed width layouts.
#[derive(Eq, Hash, PartialEq)]
enum CellRowIdentity {
    Summary,
    Command(InvocationId),
    Empty,
}

impl From<&TileContent> for CellRowIdentity {
    fn from(content: &TileContent) -> Self {
        match content {
            TileContent::Summary => Self::Summary,
            TileContent::Group(id) => Self::Command(id.clone()),
            TileContent::Empty(_) => Self::Empty,
        }
    }
}

/// Prepared ancestry and table measurements for one frame cell.
enum PreparedCellRows {
    Empty,
    Summary(PreparedSummaryRows),
    Command {
        ancestry:     PreparedAncestryRows,
        tables:       AncestryTableRows,
        table_demand: usize,
    },
}

/// The parent column can change when no ancestry line fits above the table.
enum AncestryTableRows {
    StableParentColumn(PreparedTableRows),
    ParentColumnFollowsAncestry {
        above: PreparedTableRows,
        alone: PreparedTableRows,
    },
}

impl AncestryTableRows {
    fn at(&self, ancestry_rows: u16, height: u16) -> GroupRowUse {
        match self {
            Self::StableParentColumn(table) => table.at(height),
            Self::ParentColumnFollowsAncestry { above, alone } => {
                if ancestry_rows == 0 {
                    alone.at(height)
                } else {
                    above.at(height)
                }
            },
        }
    }
}

impl PreparedCellRows {
    fn new(
        roster: &Roster,
        content: &TileContent,
        width: u16,
        hidden_when_idle: &[String],
        grids: &FrameColumnGrids,
    ) -> Self {
        match content {
            TileContent::Summary => {
                let rows = summary_rows(roster, hidden_when_idle);
                let rows: Vec<_> = rows.iter().map(AsRef::as_ref).collect();
                Self::Summary(PreparedSummaryRows::new(&rows, width, &grids.summary.0))
            },
            TileContent::Group(id) => {
                let Some(group) = roster.groups().iter().find(|group| &group.id == id) else {
                    return Self::Empty;
                };
                let grid = &grids.commands.0;
                let leads = group.leads_as_ancestor(hidden_when_idle);
                let rows: Vec<_> = group.rows().skip(usize::from(leads)).collect();
                let walk = drawn_ancestry(group, leads, grid.tree);
                let ancestry = PreparedAncestryRows::new(
                    AncestrySource::for_group(&walk, group, leads, grid.tree),
                    width,
                );
                let pinned = PinnedGroup::Lead(&group.lead.process);
                let tables = if walk.as_slice().is_empty() {
                    AncestryTableRows::StableParentColumn(PreparedTableRows::new(
                        &rows,
                        width,
                        pinned,
                        &table_grid_after_ancestry(grid, 0),
                    ))
                } else if grid.tree == ProcessTree::Short {
                    AncestryTableRows::ParentColumnFollowsAncestry {
                        above: PreparedTableRows::new(&rows, width, pinned, grid),
                        alone: PreparedTableRows::new(
                            &rows,
                            width,
                            pinned,
                            &table_grid_after_ancestry(grid, 0),
                        ),
                    }
                } else {
                    AncestryTableRows::StableParentColumn(PreparedTableRows::new(
                        &rows, width, pinned, grid,
                    ))
                };
                let table_demand = match &tables {
                    AncestryTableRows::StableParentColumn(table) => table.demand(),
                    AncestryTableRows::ParentColumnFollowsAncestry { above, .. } => above.demand(),
                };
                Self::Command {
                    ancestry,
                    tables,
                    table_demand,
                }
            },
            TileContent::Empty(_) => Self::Empty,
        }
    }

    fn demand(&self) -> usize {
        match self {
            Self::Empty => 0,
            Self::Summary(table) => table.demand(),
            Self::Command {
                ancestry,
                table_demand,
                ..
            } => {
                let ancestry = if ancestry.heights.len() > 1 {
                    ancestry
                        .heights
                        .last()
                        .copied()
                        .unwrap_or_default()
                        .saturating_add(usize::from(ANCESTRY_GAP_HEIGHT))
                } else {
                    0
                };
                ancestry.saturating_add(*table_demand)
            },
        }
    }

    fn at(&self, height: u16) -> GroupRowUse {
        match self {
            Self::Empty => GroupRowUse {
                drawn:                  0,
                kept:                   0,
                separator_rows_yielded: 0,
            },
            Self::Summary(table) => table.at(height),
            Self::Command {
                ancestry,
                tables,
                table_demand,
            } => {
                let ancestry = ancestry.at(height, *table_demand);
                let table = tables.at(ancestry.drawn, height.saturating_sub(ancestry.drawn));
                GroupRowUse::with_ancestry(&ancestry, table.drawn)
            },
        }
    }
}

/// Heights of every retained ancestry form at this width.
struct PreparedAncestryRows {
    heights: Vec<usize>,
}

impl PreparedAncestryRows {
    fn new(source: AncestrySource<'_>, width: u16) -> Self {
        let width = cleared(Rect::new(0, 0, width, 0)).width;
        let heights = (0..=source.levels.len())
            .map(|count| {
                let mut levels = ancestry_levels(source.levels, count);
                source.mark(&mut levels);
                source.height(&levels, width)
            })
            .collect();
        Self { heights }
    }

    fn at(&self, height: u16, table: usize) -> AncestryRowUse {
        let levels = self.heights.len().saturating_sub(1);
        let allocation = ancestry_allocation(height, table);
        let gap = if levels <= 1 || self.heights[levels] <= allocation.content_rows {
            allocation.gap_rows
        } else {
            0
        };
        let budget = usize::from(height)
            .saturating_sub(table)
            .saturating_sub(usize::from(gap));
        let mut count = budget.min(levels);
        loop {
            if count <= 1 && budget >= 2 && levels > 1 {
                count = 2;
                break;
            }
            if count <= 1 || self.heights[count] <= budget {
                break;
            }
            count = count.saturating_sub(1);
        }
        if count == 0 {
            return AncestryRowUse {
                drawn:    0,
                required: 0,
            };
        }
        let drawn = u16::try_from(self.heights[count].min(budget.max(1)))
            .unwrap_or(u16::MAX)
            .saturating_add(gap);
        let required = if table > 0 && budget == 1 && gap == 0 {
            0
        } else {
            drawn
        };
        AncestryRowUse { drawn, required }
    }
}

/// Painted and retained rows at any height of one solved table width.
enum TableHeightUse {
    EmptyTable,
    NoReadableColumns,
    ProcessRows {
        groups:     Vec<Vec<u16>>,
        body_rows:  u16,
        gap_rows:   u16,
        exact_rows: usize,
    },
}

impl TableHeightUse {
    fn new(groups: &[PathGroup<'_>], layout: &CellTableLayout) -> Self {
        if groups.is_empty() {
            return Self::EmptyTable;
        }
        if layout.columns.is_empty() {
            return Self::NoReadableColumns;
        }
        #[cfg(test)]
        tests::LIMIT_ROW_USE_LAYOUTS.with(|limit| {
            if limit.get() {
                let rows: Vec<_> = groups
                    .iter()
                    .flat_map(|group| group.rows.iter().copied())
                    .collect();
                tests::record_row_use_layout(&rows, layout.table_width);
            }
        });
        let mut process_rows = 0_usize;
        let groups: Vec<Vec<u16>> = groups
            .iter()
            .map(|group| {
                let mut prefix = vec![0_u16];
                let mut used = 0_u16;
                for row in &group.rows {
                    let lines = process_row_lines(row, layout);
                    process_rows = process_rows.saturating_add(usize::from(lines));
                    used = used.saturating_add(lines);
                    prefix.push(used);
                }
                prefix
            })
            .collect();
        let body_rows = groups
            .iter()
            .map(|rows| GROUP_HEADER_HEIGHT.saturating_add(*rows.last().unwrap_or(&0)))
            .fold(0, u16::saturating_add);
        let gap_rows = u16::try_from(groups.len().saturating_sub(1))
            .unwrap_or(u16::MAX)
            .saturating_mul(GROUP_GAP_HEIGHT);
        let exact_rows = usize::from(TABLE_HEADER_HEIGHT)
            .saturating_add(process_rows)
            .saturating_add(
                groups
                    .len()
                    .saturating_mul(usize::from(GROUP_HEADER_HEIGHT)),
            )
            .saturating_add(
                groups
                    .len()
                    .saturating_sub(1)
                    .saturating_mul(usize::from(GROUP_GAP_HEIGHT)),
            );
        Self::ProcessRows {
            groups,
            body_rows,
            gap_rows,
            exact_rows,
        }
    }

    fn exact_rows(&self) -> usize {
        match self {
            Self::EmptyTable => 0,
            Self::NoReadableColumns => usize::from(TABLE_HEADER_HEIGHT),
            Self::ProcessRows { exact_rows, .. } => *exact_rows,
        }
    }

    fn at(&self, height: u16, separation: DirectorySeparation) -> GroupRowUse {
        match self {
            Self::EmptyTable => {
                let drawn = height.min(TABLE_HEADER_HEIGHT.saturating_add(GROUP_HEADER_HEIGHT));
                GroupRowUse {
                    drawn,
                    kept: drawn,
                    separator_rows_yielded: 0,
                }
            },
            Self::NoReadableColumns => {
                let drawn = height.min(TABLE_HEADER_HEIGHT);
                GroupRowUse {
                    drawn,
                    kept: drawn,
                    separator_rows_yielded: 0,
                }
            },
            Self::ProcessRows {
                groups,
                body_rows,
                gap_rows,
                ..
            } => {
                let header = height.min(TABLE_HEADER_HEIGHT);
                let available = height.saturating_sub(header);
                let total = *body_rows;
                let gaps = *gap_rows;
                let retained =
                    if separation == DirectorySeparation::YieldEarliest && total <= available {
                        usize::from(available.saturating_sub(total) / GROUP_GAP_HEIGHT)
                            .min(groups.len().saturating_sub(1))
                    } else if total.saturating_add(gaps) <= available {
                        groups.len().saturating_sub(1)
                    } else {
                        0
                    };
                let mut separators = TableSeparators {
                    first:  groups.len().saturating_sub(retained),
                    groups: groups.len(),
                };
                let mut remaining = available;
                let mut visible = 0_usize;
                for (index, rows) in groups.iter().enumerate() {
                    let gap = if separators.before(index) {
                        GROUP_GAP_HEIGHT
                    } else {
                        0
                    };
                    if remaining < gap.saturating_add(GROUP_HEADER_HEIGHT).saturating_add(1) {
                        break;
                    }
                    visible += 1;
                    remaining = remaining
                        .saturating_sub(gap)
                        .saturating_sub(GROUP_HEADER_HEIGHT)
                        .saturating_sub(*rows.last().unwrap_or(&0));
                }
                if visible < groups.len() && visible > 1 {
                    let spare =
                        usize::from(remaining / GROUP_GAP_HEIGHT).min(visible.saturating_sub(1));
                    separators = TableSeparators {
                        first:  visible.saturating_sub(spare),
                        groups: visible,
                    };
                    remaining = remaining.saturating_sub(
                        u16::try_from(spare)
                            .unwrap_or(u16::MAX)
                            .saturating_mul(GROUP_GAP_HEIGHT),
                    );
                }
                let drawn = if visible == 0 {
                    header
                } else {
                    header.saturating_add(available.saturating_sub(remaining))
                };
                let separator_rows = if separation == DirectorySeparation::YieldEarliest {
                    u16::try_from(
                        (0..visible)
                            .filter(|&index| separators.before(index))
                            .count(),
                    )
                    .unwrap_or(u16::MAX)
                    .saturating_mul(GROUP_GAP_HEIGHT)
                } else {
                    0
                };
                GroupRowUse {
                    drawn,
                    kept: drawn.saturating_sub(separator_rows),
                    separator_rows_yielded: if separation == DirectorySeparation::YieldEarliest {
                        u16::try_from(separators.first.saturating_sub(1))
                            .unwrap_or(u16::MAX)
                            .saturating_mul(GROUP_GAP_HEIGHT)
                    } else {
                        0
                    },
                }
            },
        }
    }
}

/// Summary row heights change only when a newly visible row owns a state slot.
struct PreparedSummaryRows {
    layouts:    Vec<SummaryRowsAtHeight>,
    separation: DirectorySeparation,
}

/// The first allocation height that uses these summary command widths.
struct SummaryRowsAtHeight {
    height: u16,
    rows:   TableHeightUse,
}

impl PreparedSummaryRows {
    fn new(rows: &[&TrackedRow], width: u16, grid: &FrameTableColumnGrid) -> Self {
        let groups = group_by_path(rows, PinnedGroup::Unpinned);
        let prepare = |height| {
            let area = Rect::new(0, 0, width, height);
            let layout = CellTableLayout::for_table_slots(grid, area, Color::Reset, rows);
            SummaryRowsAtHeight {
                height,
                rows: if rows.is_empty() {
                    TableHeightUse::EmptyTable
                } else if width == 0 {
                    TableHeightUse::NoReadableColumns
                } else {
                    TableHeightUse::new(&groups, &layout)
                },
            }
        };
        let mut layouts = vec![prepare(0)];
        let mut state = TableStateColumn::Absent;
        let mut height = TABLE_HEADER_HEIGHT;
        for group in &groups {
            height = height.saturating_add(GROUP_HEADER_HEIGHT);
            for row in &group.rows {
                height = height.saturating_add(1);
                let next = if grid.row_progress.contains(&row.process.invocation_id)
                    || state == TableStateColumn::ProgressReading
                {
                    TableStateColumn::ProgressReading
                } else if state_width(&row.process.state) > 0
                    || state == TableStateColumn::FilledValue
                {
                    TableStateColumn::FilledValue
                } else {
                    TableStateColumn::Absent
                };
                if next != state {
                    layouts.push(prepare(height));
                    state = next;
                }
            }
        }
        Self {
            layouts,
            separation: grid.separation,
        }
    }

    fn demand(&self) -> usize {
        self.layouts
            .last()
            .map_or(0, |layout| layout.rows.exact_rows())
    }

    fn at(&self, height: u16) -> GroupRowUse {
        self.layouts
            .iter()
            .rev()
            .find(|layout| layout.height <= height)
            .map_or(
                GroupRowUse {
                    drawn:                  0,
                    kept:                   0,
                    separator_rows_yielded: 0,
                },
                |layout| layout.rows.at(height, self.separation),
            )
    }
}

/// Width layout shared by every height considered for this frame table.
struct PreparedTableRows {
    demand:     usize,
    rows:       TableHeightUse,
    separation: DirectorySeparation,
}

impl PreparedTableRows {
    fn new(
        rows: &[&TrackedRow],
        width: u16,
        pinned: PinnedGroup<'_>,
        grid: &FrameTableColumnGrid,
    ) -> Self {
        let lines = if rows.is_empty() {
            TableHeightUse::EmptyTable
        } else if width == 0 {
            TableHeightUse::NoReadableColumns
        } else {
            let grid = table_grid_for_whole_commands(grid, width, rows);
            let area = Rect::new(0, 0, width, u16::MAX);
            let layout = CellTableLayout::from_grid(&grid, area, pane_background(false));
            if layout.columns.is_empty() {
                TableHeightUse::NoReadableColumns
            } else {
                TableHeightUse::new(&group_by_path(rows, pinned), &layout)
            }
        };
        Self {
            demand:     lines.exact_rows(),
            rows:       lines,
            separation: grid.separation,
        }
    }

    const fn demand(&self) -> usize { self.demand }

    fn at(&self, height: u16) -> GroupRowUse { self.rows.at(height, self.separation) }
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
#[cfg(test)]
fn content_row_use(
    roster: &Roster,
    content: &TileContent,
    area: Rect,
    hidden_when_idle: &[String],
    tree: ProcessTree,
) -> GroupRowUse {
    let stats = SccacheStats::default();
    let cells = Cells::new(roster, hidden_when_idle, tree, &stats);
    cells.prepare_frame(|| vec![(content.clone(), area)]);
    cells.row_use(content, area)
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
    drawn:                  u16,
    /// Rows it retains before lending to a mate.
    kept:                   u16,
    /// Decorative directory gaps omitted at this allocation.
    separator_rows_yielded: u16,
}

impl GroupRowUse {
    /// Combine one ancestry allocation with the table below it.
    const fn with_ancestry(ancestry: &AncestryRowUse, table_rows: u16) -> Self {
        Self {
            drawn:                  ancestry.drawn.saturating_add(table_rows),
            kept:                   ancestry.required.saturating_add(table_rows),
            separator_rows_yielded: 0,
        }
    }
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
#[cfg(test)]
fn tile_demands(
    roster: &Roster,
    widths: &[(TileContent, u16)],
    hidden_when_idle: &[String],
    tree: ProcessTree,
) -> TileDemands {
    frame_tile_demands(
        roster,
        widths,
        hidden_when_idle,
        &FrameColumnGrids::new(roster, hidden_when_idle, tree),
    )
}

#[cfg(test)]
fn frame_tile_demands(
    roster: &Roster,
    widths: &[(TileContent, u16)],
    hidden_when_idle: &[String],
    grids: &FrameColumnGrids,
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
        summary:       frame_table_height(
            &summary,
            width_of(TileContent::Summary),
            PinnedGroup::Unpinned,
            &grids.summary.0,
        ),
        summary_width: frame_summary_width(&summary, &grids.summary.0),
        groups:        roster
            .tiled_ids(hidden_when_idle)
            .into_iter()
            .filter_map(|id| roster.groups().iter().find(|group| group.id == id))
            .map(|group| {
                let width = width_of(TileContent::Group(group.id.clone()));
                TileDemand {
                    id:   group.id.clone(),
                    rows: frame_group_height(group, width, hidden_when_idle, &grids.commands.0),
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
fn frame_summary_width(rows: &[&TrackedRow], grid: &FrameTableColumnGrid) -> u16 {
    if rows.is_empty() {
        return cell_width(SECTION_HEADER_INDENT)
            .saturating_add(cell_width(NO_PROCESSES_NOTE))
            .saturating_add(CONTENT_RIGHT_INSET);
    }
    let columns = &grid.slots;
    let command = grid.command_width();
    let gaps = u16::try_from(columns.len().saturating_sub(1)).unwrap_or(u16::MAX);
    let table = grid
        .constraints(columns)
        .iter()
        .zip(columns)
        .map(|(constraint, &column)| match constraint {
            _ if column == COMMAND_COLUMN => command,
            Constraint::Length(width) | Constraint::Min(width) => *width,
            _ => 0,
        })
        .fold(
            cell_width(SECTION_ITEM_INDENT)
                .saturating_add(CONTENT_RIGHT_INSET)
                .saturating_add(TABLE_COLUMN_SPACING.saturating_mul(gaps)),
            u16::saturating_add,
        );
    let headings = group_by_path(rows, PinnedGroup::Unpinned)
        .iter()
        .map(|group| cell_width(SECTION_HEADER_INDENT).saturating_add(cell_width(&group.heading())))
        .max()
        .unwrap_or_default();
    table.max(headings.saturating_add(CONTENT_RIGHT_INSET))
}

/// Rows one command's cell lays out with all the room it could want: the
/// ancestry block and the blank row under it, then the table.
///
/// The whole chain counts, not the levels a cell of some particular
/// height has room for -- the ask is what the cell wants, and what it is
/// given is the answer. A lead drawn as the foot of its own chain is
/// left out of the table, the same as [`frame_draw_group`] leaves it out.
#[cfg(test)]
fn group_height(
    group: &TrackedGroup,
    width: u16,
    hidden_when_idle: &[String],
    tree: ProcessTree,
) -> usize {
    let rows: Vec<_> = group
        .rows()
        .skip(usize::from(group.leads_as_ancestor(hidden_when_idle)))
        .collect();
    let grid = FrameTableColumnGrid::of(&rows, TableKind::Command, tree);
    frame_group_height(group, width, hidden_when_idle, &grid)
}

#[cfg(test)]
fn frame_group_height(
    group: &TrackedGroup,
    width: u16,
    hidden_when_idle: &[String],
    grid: &FrameTableColumnGrid,
) -> usize {
    let leads_as_ancestor = group.leads_as_ancestor(hidden_when_idle);
    let rows: Vec<&TrackedRow> = group.rows().skip(usize::from(leads_as_ancestor)).collect();
    // The same steps [`frame_draw_group`] draws, measured the same way: a
    // command wrapping down the cell wants the rows it wraps onto, and
    // asking for one row a level would leave the block short of what it
    // would have drawn with the room.
    let ancestry = drawn_ancestry(group, leads_as_ancestor, grid.tree);
    let table_grid = if ancestry.as_slice().is_empty() {
        table_grid_after_ancestry(grid, 0)
    } else {
        Cow::Borrowed(grid)
    };
    let table = frame_table_height(
        &rows,
        width,
        PinnedGroup::Lead(&group.lead.process),
        &table_grid,
    );
    ancestry_source_demand(
        AncestrySource::for_group(&ancestry, group, leads_as_ancestor, grid.tree),
        width,
    )
    .saturating_add(table)
}

/// Rows the block above the table asks for: the whole chain, plus the
/// blank row under it.
///
/// The ask is exactly what [`draw_ancestry_source`] draws when the cell is
/// given it, because [`ancestry_allocation`] hands the block whatever the
/// table does not need. Elision is then the answer to a cell that got
/// less than it asked for, never to a cell that got what it asked for --
/// which is what keeps the count the readout writes out equal to the
/// rows on screen.
#[cfg(test)]
fn ancestry_demand(ancestry: &[Ancestor], width: u16) -> usize {
    ancestry_source_demand(
        AncestrySource {
            levels:     ancestry,
            wrapping:   AncestryWrapping::Allowed,
            visibility: AncestryVisibility::Whole,
        },
        width,
    )
}

#[cfg(test)]
fn ancestry_source_demand(source: AncestrySource<'_>, width: u16) -> usize {
    if source.levels.is_empty() {
        return 0;
    }
    let mut whole = ancestry_levels(source.levels, source.levels.len());
    source.mark(&mut whole);
    source
        .height(&whole, width.saturating_sub(CONTENT_RIGHT_INSET))
        .saturating_add(usize::from(ANCESTRY_GAP_HEIGHT))
}

/// Rows a table of `rows` lays out at `width`: the one column-label row
/// the whole cell shares, then a heading and the rows under it for each
/// working directory they were run in, with a gap between one directory
/// and the next.
///
/// A row is as tall as its command line wraps to, which is why this goes
/// through the same [`CellTableLayout`] and the same [`process_row`] the
/// draw does -- counting a row as one line is what had a cell ask for a
/// third of the rows it went on to lay out. The ground passed to the
/// layout only settles what colour a faded row is written in, so any of
/// them measures the same.
///
/// Only the gaps between directories count: [`draw_path_group`] advances
/// past one after the last directory too, but nothing follows it there.
fn frame_table_height(
    rows: &[&TrackedRow],
    width: u16,
    pinned: PinnedGroup<'_>,
    grid: &FrameTableColumnGrid,
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
    let grid = table_grid_for_whole_commands(grid, width, rows);
    let layout = CellTableLayout::from_grid(&grid, area, pane_background(false));
    if layout.columns.is_empty() {
        return usize::from(TABLE_HEADER_HEIGHT);
    }
    let groups = group_by_path(rows, pinned);
    grouped_table_height(&groups, &layout)
}

/// Rows occupied by grouped table content under one solved layout.
fn grouped_table_height(groups: &[PathGroup<'_>], layout: &CellTableLayout) -> usize {
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
                .map(|row| usize::from(process_row_lines(row, layout)))
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
#[cfg(test)]
fn draw_contents(
    buffer: &mut Buffer,
    roster: &Roster,
    content: &TileContent,
    inner: Rect,
    ground: Color,
    hidden_when_idle: &[String],
    tree: ProcessTree,
) {
    frame_draw_contents(
        buffer,
        roster,
        content,
        inner,
        ground,
        hidden_when_idle,
        &FrameColumnGrids::new(roster, hidden_when_idle, tree),
    );
}

fn frame_draw_contents(
    buffer: &mut Buffer,
    roster: &Roster,
    content: &TileContent,
    inner: Rect,
    ground: Color,
    hidden_when_idle: &[String],
    grids: &FrameColumnGrids,
) {
    match content {
        TileContent::Summary => frame_draw_summary(
            buffer,
            roster,
            inner,
            ground,
            hidden_when_idle,
            &grids.summary.0,
        ),
        TileContent::Group(id) => {
            frame_draw_group(
                buffer,
                roster,
                id,
                inner,
                ground,
                hidden_when_idle,
                &grids.commands.0,
            );
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
#[cfg(test)]
fn draw_summary(
    buffer: &mut Buffer,
    roster: &Roster,
    inner: Rect,
    ground: Color,
    hidden_when_idle: &[String],
    tree: ProcessTree,
) {
    frame_draw_summary(
        buffer,
        roster,
        inner,
        ground,
        hidden_when_idle,
        &FrameColumnGrids::for_frame(roster, hidden_when_idle, tree, inner.height)
            .summary
            .0,
    );
}

fn frame_draw_summary(
    buffer: &mut Buffer,
    roster: &Roster,
    inner: Rect,
    ground: Color,
    hidden_when_idle: &[String],
    grid: &FrameTableColumnGrid,
) {
    let rows = summary_rows(roster, hidden_when_idle);
    frame_draw_process_table(
        buffer,
        inner,
        &rows.iter().map(AsRef::as_ref).collect::<Vec<_>>(),
        ground,
        PinnedGroup::Unpinned,
        grid,
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
/// Read by [`frame_draw_summary`] and by [`Cells::demands`] both, so the cell is
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
                    .map(Cow::Borrowed),
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
        match group.lead.process.subtree_resource_use.memory {
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
#[cfg(test)]
fn draw_group(
    buffer: &mut Buffer,
    roster: &Roster,
    id: &InvocationId,
    inner: Rect,
    ground: Color,
    hidden_when_idle: &[String],
    tree: ProcessTree,
) {
    frame_draw_group(
        buffer,
        roster,
        id,
        inner,
        ground,
        hidden_when_idle,
        &FrameColumnGrids::new(roster, hidden_when_idle, tree)
            .commands
            .0,
    );
}

fn frame_draw_group(
    buffer: &mut Buffer,
    roster: &Roster,
    id: &InvocationId,
    inner: Rect,
    ground: Color,
    hidden_when_idle: &[String],
    grid: &FrameTableColumnGrid,
) {
    let Some(group) = roster.groups().iter().find(|group| &group.id == id) else {
        return;
    };
    let leads_as_ancestor = group.leads_as_ancestor(hidden_when_idle);
    let rows: Vec<&TrackedRow> = group.rows().skip(usize::from(leads_as_ancestor)).collect();
    let ancestry = drawn_ancestry(group, leads_as_ancestor, grid.tree);
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
    let faded = group_fade(group, &rows);
    // The same measurement [`frame_group_height`] made when the cell
    // asked for its room, so the block is given back exactly what the
    // ask left it.
    let table_rows = frame_table_height(
        &rows,
        inner.width,
        PinnedGroup::Lead(&group.lead.process),
        grid,
    );
    let used = draw_ancestry_source(
        buffer,
        inner,
        AncestrySource::for_group(&ancestry, group, leads_as_ancestor, grid.tree),
        faded,
        ground,
        foot,
        table_rows,
    );
    let table = Rect {
        y: inner.y.saturating_add(used),
        height: inner.height.saturating_sub(used),
        ..inner
    };
    // The directory pinned to the top of the cell is the command's own,
    // drawn as a row there or not: the invocations under a driver run
    // wherever the work is, and none of those is what the cell is about.
    let table_grid = table_grid_after_ancestry(grid, used);
    frame_draw_process_table(
        buffer,
        table,
        &rows,
        ground,
        PinnedGroup::Lead(&group.lead.process),
        &table_grid,
    );
}

/// A table without its Short ancestry block retains the first row's parent link.
fn table_grid_after_ancestry(
    grid: &FrameTableColumnGrid,
    rows_drawn: u16,
) -> Cow<'_, FrameTableColumnGrid> {
    if rows_drawn == 0 && grid.tree == ProcessTree::Short {
        let mut table_grid = grid.clone();
        table_grid.parent_links = AncestryParentLinks::default();
        table_grid.parent_content = ParentColumnContent::NamesParents;
        Cow::Owned(table_grid)
    } else {
        Cow::Borrowed(grid)
    }
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
/// Short view names the nearest session runner by executable file name.
/// A chain consisting only of shells and wrappers retains its known steps.
fn drawn_ancestry(
    group: &TrackedGroup,
    leads_as_ancestor: bool,
    tree: ProcessTree,
) -> AncestryWalk<Ancestor> {
    let mut chain = group.ancestry().as_slice().to_vec();
    if leads_as_ancestor {
        chain.push(as_ancestor(&group.lead.process));
    }
    let chain = match tree {
        ProcessTree::Long => carried(chain),
        ProcessTree::Short => {
            let runner = chain.iter().rev().find(|ancestor| {
                let program = ancestor
                    .command
                    .split_whitespace()
                    .next()
                    .unwrap_or_default();
                let name = Path::new(program)
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or(program)
                    .trim_start_matches('-');
                !TRANSPARENT_PROCESS_NAMES.contains(&name)
                    && !EXEC_WRAPPER_PROCESS_NAMES.contains(&name)
            });
            match runner {
                Some(runner) if leads_as_ancestor && runner.pid == group.lead.process.pid => {
                    vec![Ancestor {
                        command: displayed_short_name(&group.lead.process.command).words,
                        ..runner.clone()
                    }]
                },
                Some(runner) => vec![shortened(runner)],
                None => carried(chain).iter().map(shortened).collect(),
            }
        },
    };
    match group.ancestry() {
        AncestryWalk::Ended(_) => AncestryWalk::Ended(chain),
        AncestryWalk::LimitCut(_) => AncestryWalk::LimitCut(chain),
    }
}

/// One step of a chain with its arguments omitted.
fn shortened(ancestor: &Ancestor) -> Ancestor {
    let command = census::command_name(&ancestor.command).words;
    Ancestor {
        pid: ancestor.pid,
        command,
        passes_through: ancestor.passes_through,
    }
}

/// A command as the last step of its own cell's chain: its pid, and the
/// whole line it was typed as.
fn as_ancestor(process: &CargoProcess) -> Ancestor {
    Ancestor {
        pid:            process.pid,
        command:        displayed_command(&process.command, SummaryDetail::Full, ProcessTree::Long),
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

#[cfg(test)]
fn draw_ancestry(
    buffer: &mut Buffer,
    area: Rect,
    ancestry: &[Ancestor],
    faded: u8,
    ground: Color,
    foot: AncestryFoot,
    table: usize,
) -> u16 {
    draw_ancestry_source(
        buffer,
        area,
        AncestrySource {
            levels:     ancestry,
            wrapping:   AncestryWrapping::Allowed,
            visibility: AncestryVisibility::Whole,
        },
        faded,
        ground,
        foot,
        table,
    )
}

fn draw_ancestry_source(
    buffer: &mut Buffer,
    area: Rect,
    source: AncestrySource<'_>,
    faded: u8,
    ground: Color,
    foot: AncestryFoot,
    table: usize,
) -> u16 {
    let area = cleared(area);
    let plan = source.plan(area, table);
    let budget = plan.budget;
    let levels = plan.levels;
    if levels.is_empty() {
        return 0;
    }
    let pid = blend_color(text_default(), ground, faded);
    let command = cut_mark_ink(ground, faded);
    let last = levels.len().saturating_sub(1);
    let indentation = AncestryIndentation::of(&levels);
    let mut blocks: Vec<Vec<Line<'static>>> = levels
        .iter()
        .enumerate()
        .map(|(level, ancestor)| {
            let ink = match foot {
                AncestryFoot::ColoredCommand(color) if level == last => {
                    blend_color(color, ground, faded)
                },
                AncestryFoot::ColoredCommand(_)
                | AncestryFoot::PlainCommand
                | AncestryFoot::Other => pid,
            };
            if matches!(source.wrapping, AncestryWrapping::SingleRow) {
                vec![short_ancestry_line(
                    *ancestor,
                    level,
                    area.width,
                    ink,
                    command,
                    &indentation,
                    source.visibility,
                )]
            } else {
                ancestry_lines(*ancestor, level, area.width, ink, command, &indentation)
            }
        })
        .collect();
    let mut remaining = budget.max(1);
    let count = blocks.len();
    for (index, lines) in blocks.iter_mut().enumerate() {
        let reserved = count.saturating_sub(index).saturating_sub(1);
        let kept = lines.len().min(remaining.saturating_sub(reserved));
        if lines.len() > kept {
            let continuation = ancestry_continuation(levels[index], lines, kept, command);
            lines.truncate(kept);
            if let Some(line) = lines.last_mut() {
                continuation.append_to(line, area.width);
                mark_cut_line(line, area.width);
            }
        }
        remaining = remaining.saturating_sub(lines.len());
    }
    let lines: Vec<_> = blocks.into_iter().flatten().collect();
    // `u16` because the count came out of a budget measured in rows of
    // this same area.
    let height = u16::try_from(lines.len()).unwrap_or(u16::MAX);
    Paragraph::new(lines).render(Rect { height, ..area }, buffer);
    height.saturating_add(plan.gap)
}

/// A basename cut touches its last character; hidden lower levels have a spaced mark.
fn short_ancestry_line(
    ancestor: AncestryLevel<'_>,
    level: usize,
    width: u16,
    pid: Color,
    command: Color,
    indentation: &AncestryIndentation,
    visibility: AncestryVisibility,
) -> Line<'static> {
    let (indent, label) = indentation.stem(ancestor, level, width);
    let ancestor = match ancestor {
        AncestryLevel::Ancestor(ancestor) | AncestryLevel::AncestorAfterElision(ancestor) => {
            ancestor
        },
    };
    let separator = if label.is_empty() { "" } else { " " };
    let suffix = if visibility.hides_lower_levels() {
        format!(" {ELISION}")
    } else {
        String::new()
    };
    let room = width
        .saturating_sub(cell_width(&indent))
        .saturating_sub(cell_width(&label))
        .saturating_sub(cell_width(separator))
        .saturating_sub(cell_width(&suffix));
    let name = elide_summary_end(&ancestor.command, room);
    Line::from(vec![
        Span::styled(indent, Style::default().fg(command)),
        Span::styled(label, Style::default().fg(pid)),
        Span::raw(separator),
        Span::styled(name, Style::default().fg(command)),
        Span::styled(suffix, Style::default().fg(command)),
    ])
}

/// Recover the unwrapped source immediately following the ancestry's visible lines.
fn ancestry_continuation(
    ancestor: AncestryLevel<'_>,
    lines: &[Line<'static>],
    kept: usize,
    command: Color,
) -> CutCommandContinuation {
    let ancestor = match ancestor {
        AncestryLevel::Ancestor(ancestor) | AncestryLevel::AncestorAfterElision(ancestor) => {
            ancestor
        },
    };
    let command_lines: Vec<_> = lines
        .iter()
        .enumerate()
        .map(|(index, line)| {
            Line::from(
                line.spans
                    .iter()
                    .skip(if index == 0 { 3 } else { 1 })
                    .cloned()
                    .collect::<Vec<_>>(),
            )
        })
        .collect();
    let continuation = command_line_continuation(&ancestor.command, &command_lines, kept);
    let source = ancestor
        .command
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let consumed: usize = command_lines
        .iter()
        .take(kept)
        .map(|line| line.to_string().chars().count())
        .sum();
    let separators = command_lines
        .iter()
        .take(kept)
        .enumerate()
        .skip(1)
        .filter(|(index, _)| {
            matches!(
                command_line_continuation(&source, &command_lines, *index).join,
                CommandContinuationJoin::WordSeparator
            )
        })
        .count();
    let remaining: String = source
        .chars()
        .skip(consumed + separators)
        .skip_while(|character| character.is_whitespace())
        .collect();
    CutCommandContinuation {
        join:      continuation.join,
        next_line: Line::from(Span::styled(remaining, Style::default().fg(command))),
    }
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
            .styled_graphemes(Style::default())
            .take_while(|grapheme| {
                let grapheme_width = UnicodeWidthStr::width(grapheme.symbol);
                let fits = used.saturating_add(grapheme_width) <= room;
                if fits {
                    used = used.saturating_add(grapheme_width);
                }
                fits
            })
            .map(|grapheme| grapheme.symbol)
            .collect();
        if !content.is_empty() {
            kept.push(Span::styled(content, span.style));
        }
        break;
    }
    while let Some(span) = kept.last_mut() {
        let trimmed = span.content.trim_end().len();
        if trimmed == span.content.len() {
            break;
        }
        span.content = span.content[..trimmed].to_owned().into();
        if !span.content.is_empty() {
            break;
        }
        kept.pop();
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
    layout: &CellTableLayout,
    continuation: TableContinuation,
    faded: u8,
) {
    let area = area.intersection(buffer.area);
    let mut command = layout.command_area(area).intersection(buffer.area);
    command.y = command.y.saturating_add(TABLE_HEADER_HEIGHT);
    command.height = command.height.saturating_sub(TABLE_HEADER_HEIGHT);
    let Some(y) = (command.top()..command.bottom())
        .rev()
        .find(|&y| (command.left()..command.right()).any(|x| buffer[(x, y)].symbol() != " "))
    else {
        return;
    };
    if matches!(continuation, TableContinuation::RowsWereOmitted) {
        let right = area
            .right()
            .saturating_sub(CONTENT_RIGHT_INSET)
            .saturating_sub(1);
        buffer[(right, y)]
            .set_symbol(ELISION)
            .set_style(cut_mark_style(layout.ground, faded));
        return;
    }
    let Some(right) = (command.left()..command.right())
        .rev()
        .find(|&x| buffer[(x, y)].symbol() != " ")
    else {
        return;
    };
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
    if let TableContinuation::CommandWasCut { continuation, tree } = continuation {
        continuation.append_to(&mut line, command.width);
        match tree {
            ProcessTree::Long => mark_cut_line(&mut line, command.width),
            ProcessTree::Short => mark_short_cut_line(&mut line, command.width),
        }
    }
    if let Some(marker) = line.spans.last_mut() {
        marker.style = cut_mark_style(layout.ground, faded);
    }
    for x in command.left()..command.right() {
        buffer[(x, y)].set_symbol(" ");
    }
    Paragraph::new(line).render(
        Rect {
            y,
            height: 1,
            ..command
        },
        buffer,
    );
}

/// Keep complete Short-name words and their styles before the spaced cut mark.
fn mark_short_cut_line(line: &mut Line<'static>, width: u16) {
    let room = width.saturating_sub(cell_width(crate::constants::ARGUMENT_OMISSION_MARK));
    let mut words = Vec::new();
    let mut word = Vec::new();
    for span in &line.spans {
        for grapheme in span.styled_graphemes(Style::default()) {
            if grapheme.symbol.chars().all(char::is_whitespace) {
                if !word.is_empty() {
                    words.push(Line::from(std::mem::take(&mut word)));
                }
            } else {
                word.push(Span::styled(grapheme.symbol.to_owned(), grapheme.style));
            }
        }
    }
    if !word.is_empty() {
        words.push(Line::from(word));
    }
    let mut kept = Line::default();
    for word in words {
        let gap = u16::from(!kept.spans.is_empty());
        if cell_width(&kept.to_string())
            .saturating_add(gap)
            .saturating_add(cell_width(&word.to_string()))
            > room
        {
            break;
        }
        if gap > 0 {
            kept.spans.push(Span::raw(" "));
        }
        kept.spans.extend(word.spans);
    }
    if kept.spans.is_empty() {
        let first_width = line
            .to_string()
            .split_whitespace()
            .next()
            .map_or(0, cell_width);
        if first_width <= width && cell_width(crate::constants::ARGUMENT_OMISSION_MARK) <= width {
            *line = Line::from(crate::constants::ARGUMENT_OMISSION_MARK);
        } else {
            mark_cut_line(line, width);
        }
    } else {
        kept.spans
            .push(Span::raw(crate::constants::ARGUMENT_OMISSION_MARK));
        *line = kept;
    }
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
/// A lone row keeps the nearest ancestor. Two or more keep the root as
/// well, clipping a wrapping endpoint's command when both endpoints cannot
/// otherwise fit. The drawing reserves a row for each retained pid before
/// using the remaining rows for its command text.
///
/// `height` is what a candidate block is measured with --
/// [`ancestry_height`] at the cell's width -- and every call to it wraps
/// each level's command line again.
fn ancestry_fit(
    ancestry: &[Ancestor],
    budget: usize,
    mut height: impl FnMut(&[AncestryLevel<'_>]) -> usize,
) -> Vec<AncestryLevel<'_>> {
    // The whole chain is measured once even when the budget has spare rows.
    let mut asked = budget.min(ancestry.len());
    loop {
        if asked <= 1 && budget >= 2 && ancestry.len() > 1 {
            return ancestry_levels(ancestry, 2);
        }
        let levels = ancestry_levels(ancestry, asked);
        if levels.len() <= 1 || height(&levels) <= budget {
            return levels;
        }
        asked = asked.saturating_sub(1);
    }
}

/// Omitted levels above or below the ancestry selected for this view.
#[derive(Clone, Copy)]
enum AncestryVisibility {
    Whole,
    LevelsHidden,
    CutAtWalkLimit,
    WalkLimitAndHiddenLevels,
}

impl AncestryVisibility {
    const fn hides_lower_levels(self) -> bool {
        matches!(self, Self::LevelsHidden | Self::WalkLimitAndHiddenLevels)
    }
}

/// Known levels and their source's completeness before allocating screen rows.
#[derive(Clone, Copy)]
struct AncestrySource<'a> {
    levels:     &'a [Ancestor],
    wrapping:   AncestryWrapping,
    visibility: AncestryVisibility,
}

/// Whether a selected session runner can carry its command onto another row.
#[derive(Clone, Copy)]
enum AncestryWrapping {
    Allowed,
    SingleRow,
}

impl<'a> From<&'a AncestryWalk<Ancestor>> for AncestrySource<'a> {
    fn from(walk: &'a AncestryWalk<Ancestor>) -> Self {
        Self {
            levels:     walk.as_slice(),
            wrapping:   AncestryWrapping::Allowed,
            visibility: match walk {
                AncestryWalk::Ended(_) => AncestryVisibility::Whole,
                AncestryWalk::LimitCut(_) => AncestryVisibility::CutAtWalkLimit,
            },
        }
    }
}

/// The levels painted at this allocation and the separation they retain.
struct AncestryPlan<'a> {
    levels: Vec<AncestryLevel<'a>>,
    budget: usize,
    gap:    u16,
}

impl<'a> AncestrySource<'a> {
    fn for_tree(walk: &'a AncestryWalk<Ancestor>, tree: ProcessTree) -> Self {
        let mut source = Self::from(walk);
        if tree == ProcessTree::Short && source.levels.len() == 1 {
            source.wrapping = AncestryWrapping::SingleRow;
        }
        source
    }

    fn for_group(
        walk: &'a AncestryWalk<Ancestor>,
        group: &TrackedGroup,
        leads_as_ancestor: bool,
        tree: ProcessTree,
    ) -> Self {
        let mut source = Self::for_tree(walk, tree);
        let hidden = tree == ProcessTree::Short
            && match walk.as_slice().last() {
                Some(kept) if leads_as_ancestor => kept.pid != group.lead.process.pid,
                Some(kept) => group
                    .ancestry()
                    .as_slice()
                    .last()
                    .is_some_and(|last| last.pid != kept.pid),
                None => false,
            };
        source.visibility = match (walk, hidden) {
            (AncestryWalk::Ended(_), false) => AncestryVisibility::Whole,
            (AncestryWalk::Ended(_), true) => AncestryVisibility::LevelsHidden,
            (AncestryWalk::LimitCut(_), false) => AncestryVisibility::CutAtWalkLimit,
            (AncestryWalk::LimitCut(_), true) => AncestryVisibility::WalkLimitAndHiddenLevels,
        };
        source
    }

    fn height(self, levels: &[AncestryLevel<'_>], width: u16) -> usize {
        match self.wrapping {
            AncestryWrapping::Allowed => ancestry_height(levels, width),
            AncestryWrapping::SingleRow => levels.len(),
        }
    }

    const fn mark(self, levels: &mut [AncestryLevel<'a>]) {
        if matches!(
            self.visibility,
            AncestryVisibility::CutAtWalkLimit | AncestryVisibility::WalkLimitAndHiddenLevels
        ) && let Some(first) = levels.first_mut()
            && let AncestryLevel::Ancestor(ancestor) = *first
        {
            *first = AncestryLevel::AncestorAfterElision(ancestor);
        }
    }

    fn plan(self, area: Rect, table: usize) -> AncestryPlan<'a> {
        let allocation = ancestry_allocation(area.height, table);
        let mut whole: Vec<_> = self.levels.iter().map(AncestryLevel::Ancestor).collect();
        self.mark(&mut whole);
        let gap = if self.levels.len() <= 1
            || self.height(&whole, area.width) <= allocation.content_rows
        {
            allocation.gap_rows
        } else {
            0
        };
        let budget = usize::from(area.height)
            .saturating_sub(table)
            .saturating_sub(usize::from(gap));
        let levels = ancestry_fit(self.levels, budget, |candidate| {
            let mut candidate = candidate.to_vec();
            self.mark(&mut candidate);
            self.height(&candidate, area.width)
        });
        let mut levels = levels;
        self.mark(&mut levels);
        AncestryPlan {
            levels,
            budget,
            gap,
        }
    }
}

/// Rows `levels` take at `width`, every level's wrapping counted.
fn ancestry_height(levels: &[AncestryLevel<'_>], width: u16) -> usize {
    let indentation = AncestryIndentation::of(levels);
    levels
        .iter()
        .enumerate()
        .map(|(level, ancestor)| ancestry_rows(*ancestor, level, width, &indentation))
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
/// it, the allocation stays here and [`draw_ancestry_source`] draws the line.
struct AncestryRowUse {
    /// Rows the block draws when it keeps this allocation.
    drawn:    u16,
    /// Rows the grid must retain before considering a transfer.
    required: u16,
}

#[cfg(test)]
fn ancestry_row_use(ancestry: &[Ancestor], area: Rect, table: usize) -> AncestryRowUse {
    ancestry_source_row_use(
        AncestrySource {
            levels:     ancestry,
            wrapping:   AncestryWrapping::Allowed,
            visibility: AncestryVisibility::Whole,
        },
        area,
        table,
    )
}

fn ancestry_source_row_use(source: AncestrySource<'_>, area: Rect, table: usize) -> AncestryRowUse {
    let area = cleared(area);
    let plan = source.plan(area, table);
    let budget = plan.budget;
    let levels = plan.levels;
    if levels.is_empty() {
        return AncestryRowUse {
            drawn:    0,
            required: 0,
        };
    }
    let rows = source.height(&levels, area.width).min(budget.max(1));
    let drawn = u16::try_from(rows)
        .unwrap_or(u16::MAX)
        .saturating_add(plan.gap);
    let required = if table > 0 && budget == 1 && plan.gap == 0 {
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
/// what say how it was actually started. The first level after an omitted
/// segment carries its inline mark. A single row keeps the marked nearest
/// ancestor; two or more retain the root ahead of the nearest levels.
fn ancestry_levels(ancestry: &[Ancestor], budget: usize) -> Vec<AncestryLevel<'_>> {
    if budget == 0 {
        return Vec::new();
    }
    if ancestry.len() <= budget {
        return ancestry.iter().map(AncestryLevel::Ancestor).collect();
    }
    if budget == 1 {
        return ancestry
            .last()
            .map(AncestryLevel::AncestorAfterElision)
            .into_iter()
            .collect();
    }
    let tail = budget.saturating_sub(1);
    let mut levels = vec![AncestryLevel::Ancestor(&ancestry[0])];
    levels.push(AncestryLevel::AncestorAfterElision(
        &ancestry[ancestry.len() - tail],
    ));
    levels.extend(
        ancestry[ancestry.len() - tail + 1..]
            .iter()
            .map(AncestryLevel::Ancestor),
    );
    levels
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
#[cfg(test)]
fn ancestry_room(ancestor: &Ancestor, level: usize, width: u16) -> u16 {
    let (indent, label) = ancestry_stem(ancestor, level, width);
    width
        .saturating_sub(cell_width(&indent))
        .saturating_sub(cell_width(&label))
        .saturating_sub(u16::from(!label.is_empty()))
}

/// Indentation inherited from cut marks on preceding ancestry levels.
enum AncestryIndentation {
    WholeChain,
    MarkedChain { inherited_mark_cells: Vec<usize> },
}

impl AncestryIndentation {
    fn of(levels: &[AncestryLevel<'_>]) -> Self {
        let mut mark_cells = 0;
        let inherited_mark_cells = levels
            .iter()
            .map(|level| {
                let inherited = mark_cells;
                if matches!(level, AncestryLevel::AncestorAfterElision(_)) {
                    mark_cells += usize::from(cell_width(ELISION)) + 1;
                }
                inherited
            })
            .collect();
        if mark_cells == 0 {
            Self::WholeChain
        } else {
            Self::MarkedChain {
                inherited_mark_cells,
            }
        }
    }

    fn stem(&self, ancestor: AncestryLevel<'_>, level: usize, width: u16) -> (String, String) {
        let (ancestor, marked) = match ancestor {
            AncestryLevel::Ancestor(ancestor) => (ancestor, false),
            AncestryLevel::AncestorAfterElision(ancestor) => (ancestor, true),
        };
        let (mut indent, _) = ancestry_stem(ancestor, level, width);
        if let Self::MarkedChain {
            inherited_mark_cells,
        } = self
        {
            indent.push_str(&" ".repeat(inherited_mark_cells[level]));
        }
        if marked {
            indent.push_str(ELISION);
            indent.push(' ');
        }
        let label = ancestor.pid.to_string();
        let label = if cell_width(&indent).saturating_add(cell_width(&label)) <= width {
            label
        } else {
            String::new()
        };
        (indent, label)
    }
}

/// Rows `ancestor` takes at `width` once its command has wrapped: one
/// for the pid and the head of the command, and one more for every line
/// the command carried on to.
///
/// An elided level is a single character and never wraps.
fn ancestry_rows(
    ancestor: AncestryLevel<'_>,
    level: usize,
    width: u16,
    indentation: &AncestryIndentation,
) -> usize {
    let (indent, label) = indentation.stem(ancestor, level, width);
    let room = width
        .saturating_sub(cell_width(&indent))
        .saturating_sub(cell_width(&label))
        .saturating_sub(u16::from(!label.is_empty()));
    let ancestor = match ancestor {
        AncestryLevel::Ancestor(ancestor) | AncestryLevel::AncestorAfterElision(ancestor) => {
            ancestor
        },
    };
    if room < ANCESTRY_MIN_COMMAND_WIDTH {
        return 1;
    }
    wrapped_command(vec![Span::raw(ancestor.command.clone())], room)
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
    indentation: &AncestryIndentation,
) -> Vec<Line<'static>> {
    let (indent, label) = indentation.stem(ancestor, level, width);
    let ancestor = match ancestor {
        AncestryLevel::Ancestor(ancestor) | AncestryLevel::AncestorAfterElision(ancestor) => {
            ancestor
        },
    };
    let room = width
        .saturating_sub(cell_width(&indent))
        .saturating_sub(cell_width(&label))
        .saturating_sub(u16::from(!label.is_empty()));
    if room < ANCESTRY_MIN_COMMAND_WIDTH {
        return vec![Line::from(vec![
            Span::styled(indent, Style::default().fg(command)),
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
    let wrapped = wrapped_command(
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
                    Span::styled(indent.clone(), Style::default().fg(command)),
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
    /// The invocation pins its directory and leads its table when it is drawn there.
    Lead(&'a CargoProcess),
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
    /// Invocations in this directory, with each parent ahead of its children.
    rows:     Vec<&'a TrackedRow>,
}

impl PathGroup<'_> {
    /// An unreadable directory leaves only the group's account to name it.
    const fn names_only_account(&self) -> bool {
        matches!(self.identity.directory, GroupDirectory::Unavailable(_))
            && !matches!(self.identity.qualification, GroupQualification::Unqualified)
    }

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
    fn heading(&self) -> String {
        if self.names_only_account() {
            self.heading_prefix().trim_end().to_string()
        } else {
            format!("{}{}", self.heading_prefix(), self.path)
        }
    }

    /// The most informative whole or marked directory heading that fits `room`.
    fn fitted_heading(&self, room: u16) -> String {
        let whole = self.heading();
        if self.names_only_account() {
            return elide_summary_end(&whole, room);
        }
        if cell_width(&whole) <= room {
            return whole;
        }
        if cell_width(self.path) <= room {
            return self.path.to_string();
        }
        let components = self.component_heading("", room);
        if !components.is_empty() {
            return components;
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
            .min(cell_width(component).saturating_sub(1));
        if tail_room < HEADING_MIN_TAIL {
            return String::new();
        }
        let span = Span::raw(component);
        let graphemes: Vec<_> = span.styled_graphemes(Style::default()).collect();
        let mut used = 0u16;
        let mut first = graphemes.len();
        for (index, grapheme) in graphemes.iter().enumerate().rev() {
            let next = used.saturating_add(cell_width(grapheme.symbol));
            if next > tail_room {
                break;
            }
            used = next;
            first = index;
        }
        if used < HEADING_MIN_TAIL {
            return String::new();
        }
        let tail: String = graphemes[first..]
            .iter()
            .map(|grapheme| grapheme.symbol)
            .collect();
        format!("{prefix}{ELISION}{tail}")
    }
}

/// Width fitting needs numeric text; row selection defers it until contributors are known.
#[derive(Clone, Copy)]
enum ProcessValueUse {
    FitColumns,
    SelectRows,
}

/// Text and command-name demand before any cell width is assigned.
struct ProcessRowValues<'a> {
    /// Invocation supplying styles, grouping and cut-command provenance.
    source:    &'a TrackedRow,
    /// Column metadata; numeric display text is deferred during row selection.
    cells:     [String; TABLE_HEADERS.len()],
    /// Command arguments in the frame's detail and tree mode.
    arguments: String,
}

impl<'a> ProcessRowValues<'a> {
    /// Read every value independently of the eventual viewport.
    fn of(source: &'a TrackedRow, detail: SummaryDetail, tree: ProcessTree) -> Self {
        Self::for_use(source, detail, tree, ProcessValueUse::FitColumns)
    }

    fn for_use(
        source: &'a TrackedRow,
        detail: SummaryDetail,
        tree: ProcessTree,
        usage: ProcessValueUse,
    ) -> Self {
        let process = &source.process;
        let arguments = match tree {
            ProcessTree::Long => process.command.line(detail),
            ProcessTree::Short => process.command.named().words,
        };
        let command = displayed_command(&process.command, detail, tree);
        let (cpu, memory, compiler) = match usage {
            ProcessValueUse::SelectRows => (String::new(), String::new(), String::new()),
            ProcessValueUse::FitColumns => {
                let compiler = match &process.resource_use.compiler {
                    CompilerObservation::Unknown | CompilerObservation::None => String::new(),
                    CompilerObservation::Running(compiler) => {
                        format!("{}×{}", compiler.name, compiler.count)
                    },
                };
                (
                    cpu_text(process.resource_use.cpu),
                    memory_text(process.resource_use.memory),
                    compiler,
                )
            },
        };
        let state = if state_width(&process.state) > 0 {
            STATE_BLOCKED.to_string()
        } else {
            String::new()
        };
        Self {
            source,
            cells: [
                process.pid.to_string(),
                parent_text(process),
                process.start.clone(),
                process.duration.clone(),
                cpu,
                memory,
                state,
                command,
                compiler,
                managed_text(process),
            ],
            arguments,
        }
    }
}

/// Metadata widths held for the lifetime of the terminal session.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SessionColumnWidths {
    cpu:      u16,
    memory:   u16,
    compiler: u16,
}

impl SessionColumnWidths {
    /// Format the machine's maximum CPU and resident-memory readings once.
    fn for_machine(cores: usize, memory: u64) -> Self {
        Self {
            cpu:      cell_width(&format!("{}%", cores.saturating_mul(100)))
                .max(cell_width(TABLE_HEADERS[CPU_COLUMN])),
            memory:   cell_width(&memory_label(memory))
                .max(cell_width(TABLE_HEADERS[MEMORY_COLUMN])),
            compiler: cell_width(TABLE_HEADERS[COMPILER_COLUMN]),
        }
    }

    /// Machine capacities are shared by every roster created in this process.
    pub(crate) fn machine() -> Self {
        static WIDTHS: OnceLock<SessionColumnWidths> = std::sync::OnceLock::new();
        *WIDTHS.get_or_init(|| {
            let system = sysinfo::System::new_with_specifics(
                sysinfo::RefreshKind::nothing()
                    .with_memory(sysinfo::MemoryRefreshKind::nothing().with_ram()),
            );
            let cores = machine_processor_count();
            Self::for_machine(cores, system.total_memory())
        })
    }

    /// An unusually large aggregate widens once; later readings keep that room.
    pub(crate) fn observe(&mut self, process: &CargoProcess) {
        for cpu in [&process.resource_use.cpu, &process.subtree_resource_use.cpu] {
            self.cpu = self.cpu.max(cell_width(&cpu_text(*cpu)));
        }
        for memory in [
            process.resource_use.memory,
            process.subtree_resource_use.memory,
        ] {
            self.memory = self.memory.max(cell_width(&memory_text(memory)));
        }
        for contribution in [&process.resource_use, &process.subtree_resource_use] {
            let compiler = match &contribution.compiler {
                CompilerObservation::Unknown | CompilerObservation::None => String::new(),
                CompilerObservation::Running(compiler) => {
                    format!("{}×{}", compiler.name, compiler.count)
                },
            };
            self.compiler = self.compiler.max(cell_width(&compiler));
        }
    }
}

/// Online logical processors, without collecting their counters and brand strings.
fn machine_processor_count() -> usize {
    #[cfg(target_os = "linux")]
    if let Ok(statistics) = std::fs::read_to_string(crate::constants::MACHINE_CPU_STAT_FILE) {
        let cores = processor_rows(&statistics);
        if cores > 0 {
            return cores;
        }
    }
    #[cfg(not(target_os = "linux"))]
    if let Ok(cores) = std::thread::available_parallelism() {
        return cores.get();
    }
    sysinfo::System::new_with_specifics(
        sysinfo::RefreshKind::nothing().with_cpu(sysinfo::CpuRefreshKind::nothing()),
    )
    .cpus()
    .len()
}

/// Count per-processor entries, excluding the aggregate and unrelated statistics.
#[cfg(target_os = "linux")]
fn processor_rows(statistics: &str) -> usize {
    statistics
        .lines()
        .filter_map(|line| line.split_whitespace().next())
        .filter(|label| {
            label
                .strip_prefix(crate::constants::MACHINE_CPU_STAT_PREFIX)
                .is_some_and(|number| {
                    !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit())
                })
        })
        .count()
}

/// Intrinsic inputs for the command tables of one immutable frame.
struct FrameCommandColumnGrid(FrameTableColumnGrid);

/// Intrinsic inputs for the summary's distinct column set in one frame.
struct FrameSummaryColumnGrid(FrameTableColumnGrid);

/// The two independent grids read by demands, drawing and row probes.
struct FrameColumnGrids {
    /// Every row belonging to a command cell participates in this grid.
    commands: FrameCommandColumnGrid,
    /// Only promoted summary rows participate in this grid.
    summary:  FrameSummaryColumnGrid,
}

impl FrameColumnGrids {
    #[cfg(test)]
    fn new(roster: &Roster, hidden_when_idle: &[String], tree: ProcessTree) -> Self {
        Self::for_frame(roster, hidden_when_idle, tree, u16::MAX)
    }

    /// Fit intrinsic widths once over the rows this frame can possibly draw.
    fn for_frame(
        roster: &Roster,
        hidden_when_idle: &[String],
        tree: ProcessTree,
        height: u16,
    ) -> Self {
        let mut command_rows = Vec::new();
        let mut command_tables = Vec::new();
        let mut command_progress = Vec::new();
        let mut parent_links = AncestryParentLinks::default();
        let tiled_ids = roster.tiled_ids(hidden_when_idle);
        for group in roster.groups() {
            if !tiled_ids.contains(&group.id) {
                continue;
            }
            let rows: Vec<_> = group
                .rows()
                .skip(usize::from(group.leads_as_ancestor(hidden_when_idle)))
                .collect();
            command_progress.extend(invocation_progress_rows(&rows, TableKind::Command));
            if tree == ProcessTree::Short {
                let ancestry =
                    drawn_ancestry(group, group.leads_as_ancestor(hidden_when_idle), tree);
                for row in &rows {
                    if let VisibleParent::Ancestor(pid) = row.process.parent
                        && ancestry
                            .as_slice()
                            .last()
                            .is_none_or(|ancestor| ancestor.pid != pid)
                    {
                        parent_links.hidden.push(row.process.invocation_id.clone());
                    }
                }
            }
            let groups = group_by_path(&rows, PinnedGroup::Lead(&group.lead.process));
            let possible = possible_frame_rows(&groups, height);
            command_tables.push(possible.clone());
            command_rows.extend(possible);
        }
        let summary = summary_rows(roster, hidden_when_idle);
        let summary: Vec<_> = summary.iter().map(AsRef::as_ref).collect();
        let summary_progress = invocation_progress_rows(&summary, TableKind::Summary);
        let summary_groups = group_by_path(&summary, PinnedGroup::Unpinned);
        let summary = possible_frame_rows(&summary_groups, height);
        let mut commands = FrameTableColumnGrid::with_parent_links(
            &command_rows,
            TableKind::Command,
            tree,
            parent_links,
            command_progress,
        );
        let mut summary = FrameTableColumnGrid::with_parent_links(
            &summary,
            TableKind::Summary,
            tree,
            AncestryParentLinks::default(),
            summary_progress,
        );
        for grid in [&mut commands, &mut summary] {
            grid.contributions = roster
                .groups()
                .iter()
                .flat_map(TrackedGroup::rows)
                .map(InvocationContribution::from)
                .collect();
            grid.intrinsic_widths[CPU_COLUMN] =
                grid.intrinsic_widths[CPU_COLUMN].max(roster.column_widths().cpu);
            grid.intrinsic_widths[MEMORY_COLUMN] =
                grid.intrinsic_widths[MEMORY_COLUMN].max(roster.column_widths().memory);
            grid.intrinsic_widths[COMPILER_COLUMN] =
                grid.intrinsic_widths[COMPILER_COLUMN].max(roster.column_widths().compiler);
        }
        if !tiled_ids.is_empty() {
            summary.separation = DirectorySeparation::YieldEarliest;
        }
        commands.fit_planned_compilers(&command_rows);
        commands.cell_gap.set(CellTableGap::Unprepared);
        commands.table_budgets = command_tables
            .iter()
            .map(|rows| {
                let local = commands.for_visible_rows(rows);
                TableWidthBudget::of(rows, local.state_column)
            })
            .collect();
        Self {
            commands: FrameCommandColumnGrid(commands),
            summary:  FrameSummaryColumnGrid(summary),
        }
    }
}

/// Rows that fit even with every optional gap and wrap row removed.
fn possible_frame_rows<'a>(groups: &[PathGroup<'a>], height: u16) -> Vec<&'a TrackedRow> {
    let mut remaining = height.saturating_sub(TABLE_HEADER_HEIGHT);
    let mut rows = Vec::new();
    for group in groups {
        if remaining <= GROUP_HEADER_HEIGHT {
            break;
        }
        remaining = remaining.saturating_sub(GROUP_HEADER_HEIGHT);
        for row in &group.rows {
            if remaining == 0 {
                break;
            }
            rows.push(*row);
            remaining -= 1;
        }
    }
    rows
}

/// Parent links whose ancestry endpoint remains visible in this tree view.
#[derive(Clone, Default)]
struct AncestryParentLinks {
    hidden: Vec<InvocationId>,
}

impl AncestryParentLinks {
    fn text(&self, process: &CargoProcess) -> String {
        if self.hidden.contains(&process.invocation_id) {
            String::new()
        } else {
            parent_text(process)
        }
    }
}

/// Whether a parent column carries any process relationship.
#[derive(Clone, Copy, Eq, PartialEq)]
enum ParentColumnContent {
    NoParentReadings,
    NamesParents,
}

/// Progress placement is decided by the complete directory group, before row cuts.
fn invocation_progress_rows(rows: &[&TrackedRow], kind: TableKind) -> Vec<InvocationId> {
    if kind == TableKind::Summary {
        return rows
            .iter()
            .filter(|row| matches!(own_counter(row), CounterState::Working { .. }))
            .map(|row| row.process.invocation_id.clone())
            .collect();
    }
    group_by_path(rows, PinnedGroup::Unpinned)
        .iter()
        .flat_map(|group| {
            let readings: Vec<_> = group
                .rows
                .iter()
                .filter(|row| matches!(own_counter(row), CounterState::Working { .. }))
                .collect();
            if readings.len() > 1 {
                readings
                    .into_iter()
                    .map(|row| row.process.invocation_id.clone())
                    .collect::<Vec<_>>()
            } else {
                Vec::new()
            }
        })
        .collect()
}

/// One disjoint numeric contribution and the invocation relationship locating it.
#[derive(Clone)]
struct InvocationContribution {
    invocation: InvocationId,
    pid:        u32,
    parent:     VisibleParent,
    resources:  InvocationResourceUse,
    /// Finished rows keep historical readings without joining a live row's current total.
    ended:      bool,
}

impl From<&CargoProcess> for InvocationContribution {
    fn from(process: &CargoProcess) -> Self {
        Self {
            invocation: process.invocation_id.clone(),
            pid:        process.pid,
            parent:     process.parent.clone(),
            resources:  process.resource_use.clone(),
            ended:      false,
        }
    }
}

impl From<&TrackedRow> for InvocationContribution {
    fn from(row: &TrackedRow) -> Self {
        Self {
            ended: row.is_ended(),
            ..Self::from(&row.process)
        }
    }
}

/// Numeric use owned by one visible row after hidden descendants join it.
struct RowResourceReading {
    invocation: InvocationId,
    pid:        u32,
    resources:  InvocationResourceUse,
}

/// Assign each contribution to its nearest invocation with a drawn row.
fn row_resource_readings(
    rows: &[&TrackedRow],
    contributions: &[InvocationContribution],
) -> Vec<RowResourceReading> {
    let mut assigned: Vec<_> = rows
        .iter()
        .map(|row| {
            vec![(
                sysinfo::Pid::from_u32(row.process.pid),
                row.process.resource_use.clone(),
            )]
        })
        .collect();
    for contribution in contributions {
        if rows.iter().any(|row| {
            row.process.invocation_id == contribution.invocation
                && row.process.pid == contribution.pid
        }) {
            continue;
        }
        let mut invocation = &contribution.invocation;
        let mut pid = contribution.pid;
        for _ in 0..=contributions.len() {
            if let Some(index) = rows
                .iter()
                .position(|row| &row.process.invocation_id == invocation && row.process.pid == pid)
            {
                if contribution.ended == rows[index].is_ended() {
                    assigned[index].push((
                        sysinfo::Pid::from_u32(contribution.pid),
                        contribution.resources.clone(),
                    ));
                }
                break;
            }
            let Some(ancestor) = contributions
                .iter()
                .find(|candidate| &candidate.invocation == invocation && candidate.pid == pid)
            else {
                break;
            };
            match &ancestor.parent {
                VisibleParent::Invocation {
                    id,
                    pid: parent_pid,
                } => {
                    invocation = id;
                    pid = *parent_pid;
                },
                VisibleParent::Ancestor(_) | VisibleParent::None => break,
            }
        }
    }
    rows.iter()
        .zip(assigned)
        .map(|(row, resources)| RowResourceReading {
            invocation: row.process.invocation_id.clone(),
            pid:        row.process.pid,
            resources:  census::sum_resource_use_once_per_pid(resources),
        })
        .collect()
}

/// A reduction considered in the shared table-width priority.
#[derive(Clone, Copy)]
pub(crate) enum TableWidthReduction {
    /// Wrap or shorten argument tails before removing any metadata.
    ArgumentTails,
    /// Remove the listed metadata at this position in the shared order.
    Columns(&'static [RemovableTableColumn]),
    /// With metadata gone, wrap a subcommand only between whole words.
    CommandWords,
}

/// Width reductions can address metadata and dispensable state, never progress.
#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) enum RemovableTableColumn {
    Pid,
    Parent,
    Start,
    Duration,
    Cpu,
    Memory,
    Compiler,
    Runs,
    Command,
    BlankStateAlignment,
    BlankParent,
    FilledStateValue,
}

/// A state slot's owner determines whether it can participate in width reduction.
#[derive(Clone, Copy, Eq, PartialEq)]
enum TableStateColumn {
    Absent,
    BlankAlignment,
    FilledValue,
    ProgressReading,
}

/// Columns and widths selected together by the frame's reduction policy.
struct FittedTableColumns {
    columns:     Vec<usize>,
    constraints: Vec<Constraint>,
    value_width: u16,
}

/// Complete column slots and intrinsic widths before a cell drops or spaces them.
#[derive(Clone)]
struct FrameTableColumnGrid {
    /// One gap for the command cells, independent of the summary's gap.
    cell_gap:                 Rc<std::cell::Cell<CellTableGap>>,
    /// Shared row-end cells reserved only after a table omits process rows.
    row_end:                  std::cell::Cell<TableRowEnd>,
    /// The frame decision remains shared when one whole Short table uses its edge.
    frame_row_end:            Rc<std::cell::Cell<TableRowEnd>>,
    /// Fully drawn Short tables that need the edge cells for a whole command name.
    whole_short_tables:       Rc<RefCell<Vec<WholeShortCommandTable>>>,
    layouts:                  Rc<RefCell<Vec<CachedTableLayout>>>,
    prepared_rows:            Rc<RefCell<PreparedProcessRows>>,
    prepared_plans:           Rc<RefCell<Vec<CachedTablePlan>>>,
    /// Disjoint resources stay available even when a height cut hides their row.
    contributions:            Vec<InvocationContribution>,
    /// The frame-wide presence of optional slots, independent of visible rows.
    slots:                    Vec<usize>,
    /// Whether this table can lend directory separators independently.
    separation:               DirectorySeparation,
    /// Whether the parent slot names anything in this frame's row set.
    parent_content:           ParentColumnContent,
    /// Tree-selected ancestry links used by both values and width fitting.
    parent_links:             AncestryParentLinks,
    /// Invocations whose directory has more than one independent reading.
    row_progress:             Vec<InvocationId>,
    /// Widths large enough for each whole metadata value and its header.
    intrinsic_widths:         [u16; TABLE_HEADERS.len()],
    /// Actual metadata values, excluding padding supplied by their headers.
    value_widths:             [u16; TABLE_HEADERS.len()],
    /// The fixed command title width, shared by every allocation probe.
    command_title_width:      u16,
    /// The widest displayed command over the whole frame's row set.
    longest_command_width:    u16,
    /// The widest program, profile and subcommand with its argument mark.
    longest_subcommand_width: u16,
    /// Whole rows keep room to distinguish a later-row mark from a cut command.
    continuation_clearance:   u16,
    /// Each table contributes its own name and state ownership to one frame choice.
    table_budgets:            Vec<TableWidthBudget>,
    /// Empty columns after the command can lend their cells to its argument tail.
    empty_tail_columns:       Vec<usize>,
    /// A progress reading has no removable state-column handle.
    state_column:             TableStateColumn,
    /// Summary and command cells use separate grids.
    kind:                     TableKind,
    /// Detail used to read the row texts that fitted this grid.
    tree:                     ProcessTree,
}

impl FrameTableColumnGrid {
    /// Cells withheld once at the table edge when any frame table hides rows.
    const fn rightmost_value_clearance(&self) -> u16 {
        match self.row_end.get() {
            TableRowEnd::Unresolved | TableRowEnd::Unreserved => 0,
            TableRowEnd::Reserved => TABLE_CONTINUATION_CLEARANCE,
        }
    }
    /// Hidden compiler processes contribute to the slot of their drawn ancestor.
    fn fit_planned_compilers(&mut self, rows: &[&TrackedRow]) {
        if !self.kind.shows_invocation_detail() {
            return;
        }
        for reading in row_resource_readings(rows, &self.contributions) {
            if let CompilerObservation::Running(compiler) = reading.resources.compiler {
                if !self.slots.contains(&COMPILER_COLUMN) {
                    self.slots.push(COMPILER_COLUMN);
                    self.slots.sort_unstable();
                }
                let width = cell_width(&format!("{}×{}", compiler.name, compiler.count));
                self.intrinsic_widths[COMPILER_COLUMN] =
                    self.intrinsic_widths[COMPILER_COLUMN].max(width);
                self.value_widths[COMPILER_COLUMN] = self.value_widths[COMPILER_COLUMN].max(width);
            }
        }
    }
    /// Fixture slot changes retain the same frame policy as production construction.
    #[cfg(test)]
    fn set_frame_slots(&mut self, slots: Vec<usize>) { self.slots = slots; }

    /// Fit all intrinsic values without depending on a cell's width or height.
    #[cfg(test)]
    fn of(rows: &[&TrackedRow], kind: TableKind, tree: ProcessTree) -> Self {
        Self::with_parent_links(
            rows,
            kind,
            tree,
            AncestryParentLinks::default(),
            invocation_progress_rows(rows, kind),
        )
    }

    fn with_parent_links(
        rows: &[&TrackedRow],
        kind: TableKind,
        tree: ProcessTree,
        parent_links: AncestryParentLinks,
        row_progress: Vec<InvocationId>,
    ) -> Self {
        let values: Vec<_> = rows
            .iter()
            .map(|row| {
                let mut values = ProcessRowValues::of(row, kind.detail(), tree);
                values.cells[PARENT_COLUMN] = parent_text(&row.process);
                if row_progress.contains(&row.process.invocation_id)
                    && let CounterState::Working { progress, .. } = own_counter(row)
                {
                    values.cells[STATE_COLUMN] = percent_reading(progress);
                }
                values
            })
            .collect();
        let mut intrinsic_widths = TABLE_HEADERS.map(cell_width);
        let mut value_widths = [0; TABLE_HEADERS.len()];
        for row in &values {
            for (column, value) in row.cells.iter().enumerate() {
                let width = cell_width(value);
                intrinsic_widths[column] = intrinsic_widths[column].max(width);
                value_widths[column] = value_widths[column].max(width);
            }
        }
        if tree == ProcessTree::Short {
            intrinsic_widths[STATE_COLUMN] = values
                .iter()
                .map(|row| cell_width(row.cells[STATE_COLUMN].trim()))
                .max()
                .unwrap_or_default()
                .max(cell_width(TABLE_HEADERS[STATE_COLUMN]));
        }
        let slots = frame_slot_columns(&values, kind);
        let state_column = if !slots.contains(&STATE_COLUMN) {
            TableStateColumn::Absent
        } else if row_progress.is_empty() {
            TableStateColumn::FilledValue
        } else {
            TableStateColumn::ProgressReading
        };
        Self {
            cell_gap: Rc::new(std::cell::Cell::new(CellTableGap::Independent)),
            row_end: std::cell::Cell::new(TableRowEnd::Unresolved),
            frame_row_end: Rc::new(std::cell::Cell::new(TableRowEnd::Unresolved)),
            whole_short_tables: std::rc::Rc::new(RefCell::new(Vec::new())),
            layouts: std::rc::Rc::new(RefCell::new(Vec::new())),
            prepared_rows: std::rc::Rc::new(RefCell::new(PreparedProcessRows::default())),
            prepared_plans: std::rc::Rc::new(RefCell::new(Vec::new())),
            contributions: rows
                .iter()
                .copied()
                .map(InvocationContribution::from)
                .collect(),
            row_progress,
            parent_links,
            slots,
            separation: DirectorySeparation::KeepTogether,
            parent_content: if values
                .iter()
                .any(|row| !row.cells[PARENT_COLUMN].is_empty())
            {
                ParentColumnContent::NamesParents
            } else {
                ParentColumnContent::NoParentReadings
            },
            intrinsic_widths,
            value_widths,
            command_title_width: cell_width(TABLE_HEADERS[COMMAND_COLUMN]),
            longest_command_width: longest_command_width(&values),
            continuation_clearance: TABLE_CONTINUATION_CLEARANCE,
            table_budgets: vec![TableWidthBudget::of(rows, state_column)],
            empty_tail_columns: if values.iter().all(|value| {
                value.cells[COMPILER_COLUMN].is_empty() && value.cells[MANAGED_COLUMN].is_empty()
            }) {
                vec![COMPILER_COLUMN, MANAGED_COLUMN]
            } else {
                Vec::new()
            },
            state_column,
            longest_subcommand_width: rows
                .iter()
                .map(|row| {
                    cell_width(&displayed_subcommand_name(&row.process.command).words)
                        .saturating_add(TABLE_CONTINUATION_CLEARANCE)
                })
                .max()
                .unwrap_or_default(),
            kind,
            tree,
        }
    }

    /// Choose slot ownership before running the shared width policy.
    fn for_visible_rows(&self, rows: &[&TrackedRow]) -> Self {
        let mut local = self.clone();
        local.empty_tail_columns = if rows.iter().all(|row| {
            let values = ProcessRowValues::of(row, self.kind.detail(), self.tree);
            values.cells[COMPILER_COLUMN].is_empty() && values.cells[MANAGED_COLUMN].is_empty()
        }) {
            vec![COMPILER_COLUMN, MANAGED_COLUMN]
        } else {
            Vec::new()
        };
        local
            .row_progress
            .retain(|id| rows.iter().any(|row| &row.process.invocation_id == id));
        let filled = rows.iter().any(|row| {
            state_width(&row.process.state) > 0
                || local.row_progress.contains(&row.process.invocation_id)
        });
        local.state_column = if !local.row_progress.is_empty() {
            TableStateColumn::ProgressReading
        } else if filled {
            TableStateColumn::FilledValue
        } else if self.kind == TableKind::Summary || !self.slots.contains(&STATE_COLUMN) {
            local.slots.retain(|column| *column != STATE_COLUMN);
            TableStateColumn::Absent
        } else {
            TableStateColumn::BlankAlignment
        };
        if self.kind == TableKind::Summary {
            let mut budget = self.table_budgets[0].clone();
            budget.state_column = local.state_column;
            local.table_budgets = vec![budget];
        }
        local
    }

    /// Requested command width, with the clearance needed for later rows.
    fn command_width(&self) -> u16 {
        self.longest_command_width
            .saturating_add(self.continuation_clearance)
            .max(cell_width(TABLE_HEADERS[COMMAND_COLUMN]))
    }

    /// Fixed metadata and the whole subcommand, independent of argument tails or view.
    fn constraints(&self, columns: &[usize]) -> Vec<Constraint> {
        columns
            .iter()
            .map(|&column| {
                if column == COMMAND_COLUMN {
                    Constraint::Min(self.longest_subcommand_width.max(self.command_title_width))
                } else {
                    Constraint::Length(self.intrinsic_widths[column])
                }
            })
            .collect()
    }

    /// Apply the same reductions for every view and every table in the frame.
    fn columns_for_command_name(&self, width: u16) -> FittedTableColumns {
        let mut budgets: Vec<TableWidthBudget> = Vec::new();
        for table in &self.table_budgets {
            let state_column = table.state_column;
            if !budgets
                .iter()
                .any(|budget| budget.state_column == state_column)
            {
                budgets.push(TableWidthBudget {
                    short: self.longest_command_width,
                    full: self
                        .longest_command_width
                        .saturating_add(TABLE_CONTINUATION_CLEARANCE),
                    subcommand: self.longest_subcommand_width,
                    state_column,
                });
            }
        }
        let own_state = self.state_column;
        let mut removed = Vec::new();
        let mut command = CommandWidthBudget::Arguments;
        for reduction in TABLE_COLUMN_DROP_ORDER {
            if budgets
                .iter()
                .all(|table| self.budget_fits(width, table, command, &removed))
            {
                break;
            }
            match reduction {
                TableWidthReduction::ArgumentTails => command = CommandWidthBudget::Subcommand,
                TableWidthReduction::Columns(departing) => removed.extend_from_slice(departing),
                TableWidthReduction::CommandWords => command = CommandWidthBudget::Words,
            }
        }
        let mut columns = self.budget_columns(own_state, &removed);
        // Once even the command has left, match the physical whole-value floor:
        // a pid identifies the reading, so an undersized state cell stays blank.
        if own_state == TableStateColumn::ProgressReading
            && width
                < self.intrinsic_widths[PID_COLUMN]
                    .saturating_add(self.intrinsic_widths[STATE_COLUMN])
                    .saturating_add(TIGHT_TABLE_COLUMN_SPACING)
        {
            columns.retain(|&column| column != STATE_COLUMN);
            if width >= self.intrinsic_widths[PID_COLUMN] && !columns.contains(&PID_COLUMN) {
                columns.insert(0, PID_COLUMN);
            }
        }
        let mut constraints = self.constraints(&columns);
        let value_width = width.saturating_sub(self.allocation_clearance(&columns));
        let kept: Vec<_> = columns
            .iter()
            .enumerate()
            .filter(|(_, column)| !self.empty_tail_columns.contains(column))
            .map(|(position, _)| constraints[position])
            .collect();
        columns.retain(|column| !self.empty_tail_columns.contains(column));
        constraints = kept;
        FittedTableColumns {
            columns,
            constraints,
            value_width,
        }
    }

    /// A Long command owns its cut edge; only later metadata needs a separate slot.
    fn allocation_clearance(&self, columns: &[usize]) -> u16 {
        if !columns.contains(&COMMAND_COLUMN) {
            return 0;
        }
        columns.last().map_or(0, |&column| {
            if self.tree == ProcessTree::Long && column == COMMAND_COLUMN {
                0
            } else {
                self.trailing_column_clearance(column)
            }
        })
    }

    /// Choose this table's gap after its column reductions, as main does.
    fn preferred_column_spacing(&self, width: u16) -> u16 {
        let fitted = self.columns_for_command_name(width);
        table_column_spacing(
            fitted.value_width,
            &fitted.constraints,
            &fitted.columns,
            self.longest_command_width,
        )
    }

    /// Tighten the cells when a whole command or its protected subcommand gains a line.
    fn cell_column_spacing(&self, width: u16) -> u16 {
        for table in &self.table_budgets {
            let mut local = self.clone();
            local.state_column = table.state_column;
            let fitted = local.columns_for_command_name(width);
            let Some(position) = fitted
                .columns
                .iter()
                .position(|&column| column == COMMAND_COLUMN)
            else {
                continue;
            };
            // Argument tails have already yielded to the subcommand in Short view.
            let command_width = match self.tree {
                ProcessTree::Short => table.short.min(table.subcommand),
                ProcessTree::Long => table.full.saturating_sub(TABLE_CONTINUATION_CLEARANCE),
            };
            let fixed = fitted
                .constraints
                .iter()
                .enumerate()
                .filter_map(|(index, constraint)| {
                    if index == position {
                        None
                    } else if let Constraint::Length(width) = constraint {
                        Some(*width)
                    } else {
                        None
                    }
                })
                .fold(0, u16::saturating_add);
            let gaps = u16::try_from(fitted.columns.len().saturating_sub(1)).unwrap_or(u16::MAX);
            let ordinary = fitted
                .value_width
                .saturating_sub(fixed)
                .saturating_sub(gaps.saturating_mul(TABLE_COLUMN_SPACING));
            let tight = fitted
                .value_width
                .saturating_sub(fixed)
                .saturating_sub(gaps.saturating_mul(TIGHT_TABLE_COLUMN_SPACING));
            // Long argument tails may wrap or be cut, but the subcommand must
            // fit the same tight-gap budget that retained the metadata columns.
            let protected_width = command_width.min(table.subcommand);
            if (ordinary < command_width && tight >= command_width)
                || (ordinary < protected_width && tight >= protected_width)
            {
                return TIGHT_TABLE_COLUMN_SPACING;
            }
        }
        TABLE_COLUMN_SPACING
    }

    /// Trailing header padding can carry a Long table's continuation mark
    /// without taking cells from the command or hiding a metadata value.
    fn trailing_column_clearance(&self, column: usize) -> u16 {
        let clearance = self.rightmost_value_clearance();
        if self.tree == ProcessTree::Long && matches!(column, COMPILER_COLUMN | MANAGED_COLUMN) {
            let padding = self.intrinsic_widths[column].saturating_sub(self.value_widths[column]);
            clearance.saturating_sub(padding)
        } else {
            clearance
        }
    }

    /// Check a reduction without allocating column and constraint vectors per probe.
    fn budget_fits(
        &self,
        width: u16,
        table: &TableWidthBudget,
        command: CommandWidthBudget,
        removed: &[RemovableTableColumn],
    ) -> bool {
        let mut used: u16 = 0;
        let mut count: u16 = 0;
        let mut last = PID_COLUMN;
        let mut has_command = false;
        let mut has_memory = false;
        for column in self.budget_columns_iter(table.state_column, removed) {
            let value = if column == COMMAND_COLUMN {
                has_command = true;
                table.command_width(command).max(self.command_title_width)
            } else {
                self.intrinsic_widths[column]
            };
            has_memory |= column == MEMORY_COLUMN;
            used = used.saturating_add(value);
            count = count.saturating_add(1);
            last = column;
        }
        used =
            used.saturating_add(TIGHT_TABLE_COLUMN_SPACING.saturating_mul(count.saturating_sub(1)));
        used <= width.saturating_sub(self.trailing_column_clearance(last))
            || (used <= width
                && (!has_command
                    || (self.tree == ProcessTree::Long && last == COMMAND_COLUMN && has_memory)))
    }

    /// Only dispensable state owns a handle that a reduction can name.
    fn budget_columns(
        &self,
        state: TableStateColumn,
        removed: &[RemovableTableColumn],
    ) -> Vec<usize> {
        self.budget_columns_iter(state, removed).collect()
    }

    /// Retained slots are shared by width probes and the final allocation.
    fn budget_columns_iter<'a>(
        &'a self,
        state: TableStateColumn,
        removed: &'a [RemovableTableColumn],
    ) -> impl Iterator<Item = usize> + 'a {
        self.slots.iter().copied().filter(move |&column| {
            let handle = match column {
                PID_COLUMN => RemovableTableColumn::Pid,
                PARENT_COLUMN => match self.parent_content {
                    ParentColumnContent::NamesParents => RemovableTableColumn::Parent,
                    ParentColumnContent::NoParentReadings => RemovableTableColumn::BlankParent,
                },
                START_COLUMN => RemovableTableColumn::Start,
                DURATION_COLUMN => RemovableTableColumn::Duration,
                CPU_COLUMN => RemovableTableColumn::Cpu,
                MEMORY_COLUMN => RemovableTableColumn::Memory,
                COMPILER_COLUMN => RemovableTableColumn::Compiler,
                MANAGED_COLUMN => RemovableTableColumn::Runs,
                COMMAND_COLUMN => RemovableTableColumn::Command,
                STATE_COLUMN => {
                    return match state {
                        TableStateColumn::Absent => false,
                        TableStateColumn::ProgressReading => true,
                        TableStateColumn::BlankAlignment => {
                            !removed.contains(&RemovableTableColumn::BlankStateAlignment)
                        },
                        TableStateColumn::FilledValue => {
                            !removed.contains(&RemovableTableColumn::FilledStateValue)
                        },
                    };
                },
                _ => return false,
            };
            !removed.contains(&handle)
        })
    }
}

/// The command portion reached by the ordered reductions.
#[derive(Clone, Copy)]
enum CommandWidthBudget {
    Arguments,
    Subcommand,
    Words,
}

/// One actual table's name and state, kept together when fitting a frame.
#[derive(Clone)]
struct TableWidthBudget {
    short:        u16,
    full:         u16,
    subcommand:   u16,
    state_column: TableStateColumn,
}

impl TableWidthBudget {
    fn of(rows: &[&TrackedRow], state_column: TableStateColumn) -> Self {
        Self {
            short: rows
                .iter()
                .map(|row| cell_width(&displayed_short_name(&row.process.command).marked()))
                .max()
                .unwrap_or_default(),
            full: rows
                .iter()
                .map(|row| {
                    let values = ProcessRowValues::of(row, SummaryDetail::Full, ProcessTree::Long);
                    cell_width(&values.cells[COMMAND_COLUMN])
                        .saturating_add(TABLE_CONTINUATION_CLEARANCE)
                })
                .max()
                .unwrap_or_default(),
            subcommand: rows
                .iter()
                .map(|row| {
                    cell_width(&displayed_subcommand_name(&row.process.command).words)
                        .saturating_add(TABLE_CONTINUATION_CLEARANCE)
                })
                .max()
                .unwrap_or_default(),
            state_column,
        }
    }

    const fn command_width(&self, budget: CommandWidthBudget) -> u16 {
        match budget {
            CommandWidthBudget::Arguments => self.full,
            CommandWidthBudget::Subcommand => self.subcommand,
            CommandWidthBudget::Words => 0,
        }
    }
}

/// Optional slots present anywhere in the frame's complete row set.
fn frame_slot_columns(values: &[ProcessRowValues<'_>], kind: TableKind) -> Vec<usize> {
    (0..TABLE_HEADERS.len())
        .filter(|&column| {
            if !SUMMARY_HIDDEN_COLUMNS.contains(&column) && column != STATE_COLUMN {
                return true;
            }
            match column {
                STATE_COLUMN => values.iter().any(|row| !row.cells[column].is_empty()),
                PARENT_COLUMN => kind.shows_invocation_detail(),
                COMPILER_COLUMN | MANAGED_COLUMN => values
                    .iter()
                    .any(|row| !row.cells[column].is_empty() && kind.shows_invocation_detail()),
                _ => true,
            }
        })
        .collect()
}

/// Keep a complete Short command when the shared reservation would cut it.
fn preserve_whole_short_table(grid: &FrameTableColumnGrid, table: Rect, rows: &[&TrackedRow]) {
    let whole_width = rows
        .iter()
        .map(|row| cell_width(&displayed_short_name(&row.process.command).marked()))
        .max()
        .unwrap_or_default();
    let unreserved = CellTableLayout::for_table_slots(grid, table, Color::Reset, rows);
    let reserved_grid = grid.clone();
    reserved_grid.row_end.set(TableRowEnd::Reserved);
    let reserved = CellTableLayout::for_table_slots(&reserved_grid, table, Color::Reset, rows);
    if unreserved.column_width(COMMAND_COLUMN) >= whole_width
        && reserved.column_width(COMMAND_COLUMN) < whole_width
    {
        grid.whole_short_tables
            .borrow_mut()
            .push(WholeShortCommandTable {
                width: table.width,
                rows:  rows
                    .iter()
                    .map(|row| (row.process.invocation_id.clone(), row.process.pid))
                    .collect(),
            });
    }
}

/// Check row identities using the layout's cached wrapped rows, without another plan.
fn table_needs_continuation(
    grid: &FrameTableColumnGrid,
    area: Rect,
    rows: &[&TrackedRow],
    pinned: PinnedGroup<'_>,
) -> bool {
    let layout = CellTableLayout::for_table_slots(grid, area, Color::Reset, rows);
    let mut remaining = area.height.saturating_sub(TABLE_HEADER_HEIGHT);
    let groups = group_by_path(rows, pinned);
    for group in groups {
        if remaining <= GROUP_HEADER_HEIGHT {
            return true;
        }
        remaining = remaining.saturating_sub(GROUP_HEADER_HEIGHT);
        for row in group.rows {
            if remaining == 0 {
                return true;
            }
            remaining = remaining.saturating_sub(process_row(row, &layout).lines);
        }
    }
    false
}

/// Solved table layouts retained across demands, allocation probes and drawing.
#[derive(Clone)]
struct CachedTableLayout {
    width:          u16,
    height:         u16,
    slots:          Vec<usize>,
    row_end:        TableRowEnd,
    frame_row_end:  TableRowEnd,
    rows:           Vec<(InvocationId, u32)>,
    layout:         CellTableLayout,
    parents:        Vec<InvocationId>,
    parent_content: ParentColumnContent,
}

/// Wrapped rows indexed by pid before checking full invocation identity and layout.
#[derive(Default)]
struct PreparedProcessRows {
    by_pid: HashMap<u32, Vec<CachedProcessRow>>,
}

/// Styled row cells prepared once for one solved layout.
#[derive(Clone)]
struct CachedProcessRow {
    process:      CargoProcess,
    faded:        u8,
    cells:        [Text<'static>; TABLE_HEADERS.len()],
    lines:        u16,
    widths:       Vec<u16>,
    columns:      Vec<usize>,
    ground:       Color,
    hides_parent: bool,
}

impl CachedProcessRow {
    fn matches(&self, row: &TrackedRow, layout: &CellTableLayout, hides_parent: bool) -> bool {
        self.process == row.process
            && self.faded == row.faded()
            && self.widths == layout.column_widths
            && self.columns == layout.columns
            && self.ground == layout.ground
            && self.hides_parent == hides_parent
    }
}

/// The command cells' gap is settled before any cell in their frame is painted.
#[derive(Clone, Copy)]
enum CellTableGap {
    Independent,
    Unprepared,
    Settled(u16),
}

/// Whether any table in the frame grid needs a later-row mark.
#[derive(Clone, Copy, Eq, PartialEq)]
enum TableRowEnd {
    Unresolved,
    Unreserved,
    Reserved,
}

/// A table whose whole Short command would be shortened by the shared reservation.
#[derive(Clone)]
struct WholeShortCommandTable {
    width: u16,
    rows:  Vec<(InvocationId, u32)>,
}

/// Keep a prepared table's command whole at every measurement and paint boundary.
fn table_grid_for_whole_commands<'a>(
    grid: &'a FrameTableColumnGrid,
    width: u16,
    rows: &[&TrackedRow],
) -> Cow<'a, FrameTableColumnGrid> {
    let local = grid.for_visible_rows(rows);
    let grid = &local;
    if grid.row_end.get() == TableRowEnd::Reserved
        && grid.whole_short_tables.borrow().iter().any(|table| {
            table.width == width
                && table.rows.len() == rows.len()
                && table.rows.iter().zip(rows).all(|((id, pid), row)| {
                    *id == row.process.invocation_id && *pid == row.process.pid
                })
        })
    {
        let unreserved = grid.clone();
        unreserved.row_end.set(TableRowEnd::Unreserved);
        Cow::Owned(unreserved)
    } else {
        Cow::Owned(local)
    }
}

/// How one cell's tables are laid out, settled once for the whole cell.
///
/// Every group in the cell is drawn against this, which is what keeps
/// the tables lined up down the cell instead of each one fitting itself
/// and the columns stepping in and out as the eye moves between them.
#[derive(Clone)]
struct CellTableLayout {
    #[cfg(test)]
    table_width:    u16,
    prepared_rows:  Rc<RefCell<PreparedProcessRows>>,
    prepared_plans: Rc<RefCell<Vec<CachedTablePlan>>>,
    /// The summary keeps readings beside rows; command headings keep their gauge.
    kind:           TableKind,
    /// Column widths, in table order.
    constraints:    Vec<Constraint>,
    /// The columns this cell draws, in table order.
    columns:        Vec<usize>,
    /// Solved widths for `columns`, in the same order.
    column_widths:  Vec<u16>,
    /// Blank cells between adjacent columns.
    column_spacing: u16,
    /// Invocations that show their own reading in the state slot.
    row_progress:   Vec<InvocationId>,
    /// Ancestry links retained in the frame's selected tree view.
    parent_links:   AncestryParentLinks,
    /// How much of each row's command line the cell prints.
    detail:         SummaryDetail,
    /// Whether a row spells out its whole command line or only the
    /// name of what runs.
    tree:           ProcessTree,
    /// The colour the cell is painted on, which a finished row's text
    /// is carried toward as it fades.
    ground:         Color,
}

impl CellTableLayout {
    /// The layout for a cell of `kind` drawing `rows` into `area`, over
    /// a cell painted `ground`.
    #[cfg(test)]
    fn of(
        rows: &[&TrackedRow],
        kind: TableKind,
        area: Rect,
        ground: Color,
        tree: ProcessTree,
    ) -> Self {
        Self::from_grid(&FrameTableColumnGrid::of(rows, kind, tree), area, ground)
    }

    /// Reuse the frame slots while reserving the edge for any hidden rows.
    fn for_table(
        grid: &FrameTableColumnGrid,
        area: Rect,
        ground: Color,
        rows: &[&TrackedRow],
        pinned: PinnedGroup<'_>,
    ) -> Self {
        if matches!(grid.row_end.get(), TableRowEnd::Unresolved)
            && table_needs_continuation(grid, area, rows, pinned)
        {
            let reserved = grid.clone();
            reserved.row_end.set(TableRowEnd::Reserved);
            return Self::for_table_slots(&reserved, area, ground, rows);
        }
        Self::for_table_slots(grid, area, ground, rows)
    }

    fn for_table_slots(
        grid: &FrameTableColumnGrid,
        area: Rect,
        ground: Color,
        rows: &[&TrackedRow],
    ) -> Self {
        let grid = table_grid_for_whole_commands(grid, area.width, rows);
        let visible = possible_frame_rows(&group_by_path(rows, PinnedGroup::Unpinned), area.height);
        let local;
        let grid = if grid.kind == TableKind::Summary {
            local = grid.for_visible_rows(&visible);
            &local
        } else {
            grid.as_ref()
        };
        let identities: Vec<_> = rows
            .iter()
            .map(|row| (row.process.invocation_id.clone(), row.process.pid))
            .collect();
        if let Some(cached) = grid.layouts.borrow().iter().find(|cached| {
            cached.width == area.width
                && cached.height == area.height
                && cached.slots == grid.slots
                && cached.row_end == grid.row_end.get()
                && cached.frame_row_end == grid.frame_row_end.get()
                && cached.rows == identities
                && cached.parents == grid.parent_links.hidden
                && cached.parent_content == grid.parent_content
        }) {
            let mut layout = cached.layout.clone();
            layout.ground = ground;
            return layout;
        }
        let layout = Self::from_grid(grid, area, ground);
        grid.layouts.borrow_mut().push(CachedTableLayout {
            width:          area.width,
            height:         area.height,
            slots:          grid.slots.clone(),
            row_end:        grid.row_end.get(),
            frame_row_end:  grid.frame_row_end.get(),
            rows:           identities,
            layout:         layout.clone(),
            parents:        grid.parent_links.hidden.clone(),
            parent_content: grid.parent_content,
        });
        layout
    }

    /// Lay out one frame grid at this cell's actual inner width.
    fn from_grid(grid: &FrameTableColumnGrid, area: Rect, ground: Color) -> Self {
        let FittedTableColumns {
            columns,
            mut constraints,
            value_width: table_width,
            ..
        } = grid.columns_for_command_name(indented(area).width);
        let column_spacing = match grid.kind {
            TableKind::Summary => grid.preferred_column_spacing(indented(area).width),
            TableKind::Command => match grid.cell_gap.get() {
                CellTableGap::Independent => grid.preferred_column_spacing(indented(area).width),
                CellTableGap::Unprepared => grid.cell_column_spacing(indented(area).width),
                CellTableGap::Settled(spacing) => spacing,
            },
        };
        if let Some(position) = columns.iter().position(|&column| column == COMMAND_COLUMN) {
            let other_width = constraints
                .iter()
                .enumerate()
                .filter(|&(index, _)| index != position)
                .map(|(_, constraint)| match constraint {
                    Constraint::Length(width) => *width,
                    _ => 0,
                })
                .fold(0, u16::saturating_add);
            let gaps = u16::try_from(columns.len().saturating_sub(1))
                .unwrap_or(u16::MAX)
                .saturating_mul(column_spacing);
            constraints[position] = Constraint::Length(
                grid.command_width()
                    .min(table_width.saturating_sub(other_width).saturating_sub(gaps)),
            );
        }
        let column_widths = solved_column_widths(table_width, &constraints, column_spacing);
        Self {
            #[cfg(test)]
            table_width: area.width,
            prepared_rows: std::rc::Rc::clone(&grid.prepared_rows),
            prepared_plans: std::rc::Rc::clone(&grid.prepared_plans),
            kind: grid.kind,
            row_progress: grid.row_progress.clone(),
            parent_links: grid.parent_links.clone(),
            constraints,
            columns,
            column_widths,
            column_spacing,
            detail: grid.kind.detail(),
            tree: grid.tree,
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

/// The cell's shared ink follows its rows and the command owning it.
fn group_fade(group: &TrackedGroup, rows: &[&TrackedRow]) -> u8 {
    heading_fade(rows).min(group.lead.faded())
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
/// Apply contributor totals and family ink after the table selects its rows.
fn apply_planned_row_readings(
    drawn_groups: &mut [Vec<LaidOutProcessRow<'_>>],
    plan: &mut ProcessTablePlan<'_>,
    layout: &CellTableLayout,
    grid: &FrameTableColumnGrid,
) {
    let readings = match &plan.visible_rows {
        PlannedVisibleRows::NoProcessRows => Vec::new(),
        PlannedVisibleRows::ProcessRows(rows) => row_resource_readings(rows, &grid.contributions),
    };
    let family_links = AllocationFamilyLinks::from_plan(plan);
    for row in drawn_groups.iter_mut().flatten() {
        if let Some(reading) = readings.iter().find(|reading| {
            reading.invocation == row.source.process.invocation_id
                && reading.pid == row.source.process.pid
        }) {
            let muted = Style::default().fg(layout.ink(label_color(), row.source.faded()));
            row.cells[CPU_COLUMN] =
                Text::from(Span::styled(cpu_text(reading.resources.cpu), muted));
            row.cells[MEMORY_COLUMN] =
                Text::from(Span::styled(memory_text(reading.resources.memory), muted));
            row.cells[COMPILER_COLUMN] = Text::from(compiler_reading(
                &reading.resources.compiler,
                row.source,
                layout,
            ));
            plan.header_and_value_visibility[COMPILER_COLUMN] |= row.cells[COMPILER_COLUMN]
                .lines
                .iter()
                .any(|line| !line.to_string().is_empty());
        }
        for line in &mut row.cells[PARENT_COLUMN].lines {
            for span in &mut line.spans {
                span.style = parent_style(row.source, layout, &family_links);
            }
        }
        for line in &mut row.cells[PID_COLUMN].lines {
            for span in &mut line.spans {
                span.style = pid_style(row.source, layout, &family_links);
            }
        }
    }
}

#[cfg(test)]
fn draw_process_table(
    buffer: &mut Buffer,
    area: Rect,
    rows: &[&TrackedRow],
    kind: TableKind,
    ground: Color,
    pinned: PinnedGroup<'_>,
    tree: ProcessTree,
) {
    frame_draw_process_table(
        buffer,
        area,
        rows,
        ground,
        pinned,
        &FrameTableColumnGrid::of(rows, kind, tree),
    );
}

fn draw_empty_process_table(buffer: &mut Buffer, area: Rect, kind: TableKind) {
    let text = format!("{SECTION_HEADER_INDENT}{NO_PROCESSES_NOTE}");
    let text = match kind {
        TableKind::Summary => elide_summary_end(&text, cleared(area).width),
        TableKind::Command => text,
    };
    Paragraph::new(vec![
        Line::from(""),
        Line::from(Span::styled(text, Style::default().fg(label_color()))),
    ])
    .render(cleared(area), buffer);
}

fn frame_draw_process_table(
    buffer: &mut Buffer,
    area: Rect,
    rows: &[&TrackedRow],
    ground: Color,
    pinned: PinnedGroup<'_>,
    grid: &FrameTableColumnGrid,
) {
    if rows.is_empty() {
        draw_empty_process_table(buffer, area, grid.kind);
        return;
    }

    // One column-label row for the whole cell. Every group's table is
    // laid out with the same constraints and the same indent, so the
    // labels stay over their columns without costing a row per group.
    let layout = CellTableLayout::for_table(grid, area, ground, rows, pinned);
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
    let groups = group_by_path(rows, pinned);
    let mut drawn_groups: Vec<Vec<LaidOutProcessRow<'_>>> = groups
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
    let mut plan = planned_table_rows(&drawn_groups, available_height, grid.separation, &layout);
    let headings = TableAccountHeadings::of(
        &groups[..plan.visible_group_count],
        cleared(area)
            .width
            .saturating_sub(cell_width(SECTION_HEADER_INDENT)),
    );
    apply_planned_row_readings(&mut drawn_groups, &mut plan, &layout, grid);
    Table::new(Vec::<Row>::new(), layout.constraints.iter().copied())
        .header(column_header(&layout, faded, &plan))
        .column_spacing(layout.column_spacing)
        .render(header_area, buffer);
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
        let gap_before = if plan.separators.before(index) {
            GROUP_GAP_HEIGHT
        } else {
            0
        };
        remaining.y = remaining.y.saturating_add(gap_before);
        remaining.height = remaining.height.saturating_sub(gap_before);
        let drawn = draw_path_group(buffer, remaining, group, &layout, rows, &headings);
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
    if matches!(continuation, TableContinuation::FullyDrawn)
        && let PlannedVisibleRows::ProcessRows(visible) = &plan.visible_rows
        && visible.len() < rows.len()
    {
        continuation = TableContinuation::RowsWereOmitted;
    }
    if !matches!(continuation, TableContinuation::FullyDrawn) {
        let faded = match &plan.visible_rows {
            PlannedVisibleRows::NoProcessRows => 0,
            PlannedVisibleRows::ProcessRows(rows) => rows.last().map_or(0, |row| row.faded()),
        };
        mark_table_continuation(buffer, area, &layout, continuation, faded);
    }
}

/// Process identities receiving at least one visible line in this viewport.
enum PlannedVisibleRows<'a> {
    /// The area admits no process value.
    NoProcessRows,
    /// Every entry identifies a drawn row, including a partially drawn command.
    ProcessRows(Vec<&'a TrackedRow>),
}

/// Whether the table's separators can lend independently to a command cell.
#[derive(Clone, Copy, Eq, PartialEq)]
enum DirectorySeparation {
    KeepTogether,
    YieldEarliest,
}

/// Retained directory separators, with the earliest ones yielded first.
#[derive(Clone, Copy)]
struct TableSeparators {
    first:  usize,
    groups: usize,
}

impl TableSeparators {
    const fn before(&self, group: usize) -> bool {
        group > 0 && group >= self.first && group < self.groups
    }
}

/// Parent links proved by the process rows this allocation paints.
enum AllocationFamilyLinks {
    NoDrawnLinks,
    DrawnRows {
        parents:     Vec<InvocationId>,
        invocations: Vec<(InvocationId, u32)>,
    },
}

impl AllocationFamilyLinks {
    fn from_plan(plan: &ProcessTablePlan<'_>) -> Self {
        match &plan.visible_rows {
            PlannedVisibleRows::NoProcessRows => Self::NoDrawnLinks,
            PlannedVisibleRows::ProcessRows(rows) => Self::DrawnRows {
                invocations: rows
                    .iter()
                    .map(|row| (row.process.invocation_id.clone(), row.process.pid))
                    .collect(),
                parents:     rows
                    .iter()
                    .filter_map(|row| match &row.process.parent {
                        VisibleParent::Invocation { id, .. } => Some(id.clone()),
                        VisibleParent::Ancestor(_) | VisibleParent::None => None,
                    })
                    .collect(),
            },
        }
    }

    fn contains(&self, id: &InvocationId, pid: u32) -> bool {
        match self {
            Self::NoDrawnLinks => false,
            Self::DrawnRows { invocations, .. } => invocations
                .iter()
                .any(|(invocation, native)| invocation == id && *native == pid),
        }
    }

    fn heads(&self, id: &InvocationId) -> bool {
        match self {
            Self::NoDrawnLinks => false,
            Self::DrawnRows { parents, .. } => parents.contains(id),
        }
    }
}

/// Which directory groups and values fit, with one separator policy.
struct ProcessTablePlan<'a> {
    /// Whether every drawn pair of adjacent groups keeps its blank row.
    separators:                  TableSeparators,
    /// Groups with room for both their heading and first process line.
    visible_group_count:         usize,
    /// Identities from which row usage and optional-column labels are read.
    visible_rows:                PlannedVisibleRows<'a>,
    /// Heading, process and separator rows occupied by these identities.
    body_rows_drawn:             u16,
    /// Painted process rows and headings, with lendable separators removed.
    body_rows_required:          u16,
    /// Optional headers and values shown here; slots remain in the frame grid.
    header_and_value_visibility: [bool; TABLE_HEADERS.len()],
}

impl ProcessTablePlan<'_> {
    /// A present slot labels itself only when a drawn row supplies its value.
    const fn labels_column(&self, column: usize) -> bool {
        if matches!(column, STATE_COLUMN | COMPILER_COLUMN | MANAGED_COLUMN) {
            match &self.visible_rows {
                PlannedVisibleRows::NoProcessRows => false,
                PlannedVisibleRows::ProcessRows(rows) => {
                    !rows.is_empty() && self.header_and_value_visibility[column]
                },
            }
        } else {
            true
        }
    }
}

/// Viewport plan shared by row-use probes and paint at one solved width.
#[derive(Clone)]
struct CachedTablePlan {
    rows:                Vec<Vec<(InvocationId, u32)>>,
    widths:              Vec<u16>,
    columns:             Vec<usize>,
    height:              u16,
    separation:          DirectorySeparation,
    visible:             Vec<usize>,
    separators:          TableSeparators,
    visible_group_count: usize,
    drawn:               u16,
    required:            u16,
    labels:              [bool; TABLE_HEADERS.len()],
}

fn planned_table_rows<'a>(
    groups: &[Vec<LaidOutProcessRow<'a>>],
    height: u16,
    separation: DirectorySeparation,
    layout: &CellTableLayout,
) -> ProcessTablePlan<'a> {
    let identities: Vec<Vec<_>> = groups
        .iter()
        .map(|group| {
            group
                .iter()
                .map(|row| {
                    (
                        row.source.process.invocation_id.clone(),
                        row.source.process.pid,
                    )
                })
                .collect()
        })
        .collect();
    if let Some(cached) = layout.prepared_plans.borrow().iter().find(|cached| {
        cached.rows == identities
            && cached.widths == layout.column_widths
            && cached.columns == layout.columns
            && cached.height == height
            && cached.separation == separation
    }) {
        let visible: Vec<_> = groups
            .iter()
            .flatten()
            .enumerate()
            .filter(|(index, _)| cached.visible.contains(index))
            .map(|(_, row)| row.source)
            .collect();
        return ProcessTablePlan {
            separators:                  cached.separators,
            visible_group_count:         cached.visible_group_count,
            visible_rows:                if visible.is_empty() {
                PlannedVisibleRows::NoProcessRows
            } else {
                PlannedVisibleRows::ProcessRows(visible)
            },
            body_rows_drawn:             cached.drawn,
            body_rows_required:          cached.required,
            header_and_value_visibility: cached.labels,
        };
    }
    let plan = process_table_plan(groups, height, separation);
    let visible = match &plan.visible_rows {
        PlannedVisibleRows::NoProcessRows => Vec::new(),
        PlannedVisibleRows::ProcessRows(rows) => groups
            .iter()
            .flatten()
            .enumerate()
            .filter(|(_, row)| rows.iter().any(|source| std::ptr::eq(*source, row.source)))
            .map(|(index, _)| index)
            .collect(),
    };
    layout.prepared_plans.borrow_mut().push(CachedTablePlan {
        rows: identities,
        widths: layout.column_widths.clone(),
        columns: layout.columns.clone(),
        height,
        separation,
        visible,
        separators: plan.separators,
        visible_group_count: plan.visible_group_count,
        drawn: plan.body_rows_drawn,
        required: plan.body_rows_required,
        labels: plan.header_and_value_visibility,
    });
    plan
}

/// Choose visible row identities and all inter-directory gaps before drawing.
fn process_table_plan<'a>(
    groups: &[Vec<LaidOutProcessRow<'a>>],
    available_height: u16,
    separation: DirectorySeparation,
) -> ProcessTablePlan<'a> {
    let all_groups_height = groups
        .iter()
        .map(|rows| {
            GROUP_HEADER_HEIGHT.saturating_add(
                rows.iter()
                    .map(|row| row.lines)
                    .fold(0, u16::saturating_add),
            )
        })
        .fold(0, u16::saturating_add);
    let all_gaps_height = u16::try_from(groups.len().saturating_sub(1))
        .unwrap_or(u16::MAX)
        .saturating_mul(GROUP_GAP_HEIGHT);
    let retained = if matches!(separation, DirectorySeparation::YieldEarliest)
        && all_groups_height <= available_height
    {
        usize::from(available_height.saturating_sub(all_groups_height) / GROUP_GAP_HEIGHT)
            .min(groups.len().saturating_sub(1))
    } else if all_groups_height.saturating_add(all_gaps_height) <= available_height {
        groups.len().saturating_sub(1)
    } else {
        0
    };
    let mut separators = TableSeparators {
        first:  groups.len().saturating_sub(retained),
        groups: groups.len(),
    };
    let mut remaining = available_height;
    let mut visible_group_count = 0;
    let mut visible_rows = Vec::new();
    let mut header_and_value_visibility = [false; TABLE_HEADERS.len()];
    for (index, rows) in groups.iter().enumerate() {
        let gap = if separators.before(index) {
            GROUP_GAP_HEIGHT
        } else {
            0
        };
        if remaining < gap.saturating_add(GROUP_HEADER_HEIGHT).saturating_add(1) {
            break;
        }
        visible_group_count += 1;
        remaining = remaining
            .saturating_sub(gap)
            .saturating_sub(GROUP_HEADER_HEIGHT);
        for row in rows {
            if remaining == 0 {
                break;
            }
            visible_rows.push(row.source);
            for column in [STATE_COLUMN, COMPILER_COLUMN, MANAGED_COLUMN] {
                header_and_value_visibility[column] |= row.cells[column]
                    .lines
                    .iter()
                    .any(|line| line.spans.iter().any(|span| !span.content.is_empty()));
            }
            remaining = remaining.saturating_sub(row.lines);
        }
    }
    if visible_group_count < groups.len() && visible_group_count > 1 {
        let spare_separators =
            usize::from(remaining / GROUP_GAP_HEIGHT).min(visible_group_count.saturating_sub(1));
        separators = TableSeparators {
            first:  visible_group_count.saturating_sub(spare_separators),
            groups: visible_group_count,
        };
        remaining = remaining.saturating_sub(
            u16::try_from(spare_separators)
                .unwrap_or(u16::MAX)
                .saturating_mul(GROUP_GAP_HEIGHT),
        );
    }
    ProcessTablePlan {
        separators,
        visible_group_count,
        visible_rows: if visible_rows.is_empty() {
            PlannedVisibleRows::NoProcessRows
        } else {
            PlannedVisibleRows::ProcessRows(visible_rows)
        },
        body_rows_drawn: available_height.saturating_sub(remaining),
        body_rows_required: available_height.saturating_sub(remaining).saturating_sub(
            if matches!(separation, DirectorySeparation::YieldEarliest) {
                u16::try_from(
                    (0..visible_group_count)
                        .filter(|&group| separators.before(group))
                        .count(),
                )
                .unwrap_or(u16::MAX)
                .saturating_mul(GROUP_GAP_HEIGHT)
            } else {
                0
            },
        ),
        header_and_value_visibility,
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
    let preferred = pinned;
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
    let mut pinned = match pinned {
        PinnedGroup::Lead(lead) => groups
            .iter()
            .position(|group| group.identity.is_compatible(&GroupingIdentity::from(lead)))
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
    for group in groups.iter_mut().chain(pinned.iter_mut()) {
        let mut pending = std::mem::take(&mut group.rows);
        while !pending.is_empty() {
            let position = pending
                .iter()
                .position(|row| match &row.process.parent {
                    VisibleParent::Invocation { id, .. } => !pending
                        .iter()
                        .any(|parent| &parent.process.invocation_id == id),
                    VisibleParent::Ancestor(_) | VisibleParent::None => true,
                })
                .unwrap_or(0);
            group.rows.push(pending.remove(position));
        }
        if let PinnedGroup::Lead(lead) = preferred
            && let Some(position) = group
                .rows
                .iter()
                .position(|row| row.process.invocation_id == lead.invocation_id)
        {
            let lead = group.rows.remove(position);
            group.rows.insert(0, lead);
        }
    }
    // Whatever the rest sort to, the pinned directory heads the cell.
    // The others keep the order they had under it, so a group that
    // comes and goes moves nothing but itself.
    if let Some(group) = pinned {
        groups.insert(0, group);
    }
    groups
}

/// Accounts whose readable directory headings use the path-only form in this table.
struct TableAccountHeadings {
    path_only: Vec<u32>,
}

impl TableAccountHeadings {
    fn of(groups: &[PathGroup<'_>], width: u16) -> Self {
        let mut path_only = Vec::new();
        for group in groups {
            if group.names_only_account() || cell_width(&group.heading()) <= width {
                continue;
            }
            match group.identity.qualification {
                GroupQualification::Owner { uid } | GroupQualification::Captured { uid, .. } => {
                    if !path_only.contains(&uid)
                        && (path_only.is_empty()
                            || group
                                .component_heading(&group.heading_prefix(), width)
                                .is_empty())
                    {
                        path_only.push(uid);
                    }
                },
                GroupQualification::Unqualified => {},
            }
        }
        Self { path_only }
    }

    fn heading(&self, group: &PathGroup<'_>, width: u16) -> String {
        let drops = match group.identity.qualification {
            GroupQualification::Owner { uid } | GroupQualification::Captured { uid, .. } => {
                self.path_only.contains(&uid)
            },
            GroupQualification::Unqualified => false,
        };
        if drops && !group.names_only_account() {
            if cell_width(group.path) <= width {
                return group.path.to_string();
            }
            let components = group.component_heading("", width);
            if !components.is_empty() {
                return components;
            }
            return group.tail_heading("", width);
        }
        if !self.path_only.is_empty()
            && !group.names_only_account()
            && !matches!(
                group.identity.qualification,
                GroupQualification::Unqualified
            )
        {
            let whole = group.heading();
            if cell_width(&whole) <= width {
                return whole;
            }
            let prefix = group.heading_prefix();
            let components = group.component_heading(&prefix, width);
            if !components.is_empty() {
                return components;
            }
            let tail = group.tail_heading(&prefix, width);
            if !tail.is_empty() {
                return tail;
            }
            return group.fitted_heading(width);
        }
        group.fitted_heading(width)
    }
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
        /// Short names lose complete words; Long arguments can end inside a word.
        tree:         ProcessTree,
    },
    /// The final visible command is whole, but later process rows did not fit.
    RowsWereOmitted,
}

/// Classify a vertical cut through drawn rows and retain a cut command's next line.
fn table_continuation(rows: &[LaidOutProcessRow<'_>], visible_lines: u16) -> TableContinuation {
    let mut used = 0u16;
    for row in rows {
        if used >= visible_lines {
            return TableContinuation::RowsWereOmitted;
        }
        if used.saturating_add(row.lines) > visible_lines {
            let first_hidden_line = usize::from(visible_lines.saturating_sub(used));
            return TableContinuation::CommandWasCut {
                continuation: row.cut_command_continuation(first_hidden_line),
                tree:         row.tree,
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
    layout: &CellTableLayout,
    rows: Vec<LaidOutProcessRow<'_>>,
    headings: &TableAccountHeadings,
) -> PathGroupDraw {
    let faded = heading_fade(&group.rows);
    let heading_area = cleared(area);
    let heading_room = heading_area
        .width
        .saturating_sub(cell_width(SECTION_HEADER_INDENT));
    let heading_text = headings.heading(group, heading_room);
    let mut heading = vec![
        Span::raw(SECTION_HEADER_INDENT),
        Span::styled(
            heading_text,
            Style::default().fg(layout.ink(accent_color(), faded)),
        ),
    ];
    heading.extend(heading_gauge(group, heading_area.width, layout));
    Paragraph::new(Line::from(heading)).render(
        Rect {
            height: GROUP_HEADER_HEIGHT.min(area.height),
            ..heading_area
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
    let span = Span::raw(text);
    let prefix: String = span
        .styled_graphemes(Style::default())
        .take_while(|grapheme| {
            let grapheme_width = UnicodeWidthStr::width(grapheme.symbol);
            let fits = used.saturating_add(grapheme_width) <= room;
            if fits {
                used = used.saturating_add(grapheme_width);
            }
            fits
        })
        .map(|grapheme| grapheme.symbol)
        .collect();
    let prefix = prefix.trim_end();
    if prefix.is_empty() {
        return String::new();
    }
    format!("{prefix}{ELISION}")
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
fn longest_command_width(rows: &[ProcessRowValues<'_>]) -> u16 {
    rows.iter()
        .map(|row| cell_width(&row.cells[COMMAND_COLUMN]))
        .max()
        .unwrap_or_default()
}

/// The executable spelling selected by the current tree view.
fn displayed_program(command: &CommandText, tree: ProcessTree) -> &str {
    match tree {
        ProcessTree::Short => command.program_name(),
        ProcessTree::Long => &command.program,
    }
}

/// Leading profile label retained when the command's tail is cut.
fn profile_label(command: &CommandText) -> String {
    match command.profile() {
        CargoProfile::Default => String::new(),
        CargoProfile::Release => format!(
            "{PROFILE_LABEL_OPEN}{}{PROFILE_LABEL_CLOSE}",
            crate::constants::CARGO_RELEASE_PROFILE
        ),
        CargoProfile::Named(name) => format!("{PROFILE_LABEL_OPEN}{name}{PROFILE_LABEL_CLOSE}"),
    }
}

/// All words painted by the command column, including its profile.
fn displayed_command(command: &CommandText, detail: SummaryDetail, tree: ProcessTree) -> String {
    if tree == ProcessTree::Short {
        return displayed_short_name(command).marked();
    }
    let arguments = command.line(detail);
    [
        profile_label(command),
        displayed_program(command, tree).to_string(),
        arguments,
    ]
    .iter()
    .flat_map(|part| part.split_whitespace())
    .collect::<Vec<_>>()
    .join(" ")
}

/// Short command words retain the source's argument-omission state.
fn displayed_short_name(command: &CommandText) -> CommandName {
    let arguments = command.named();
    CommandName {
        words:     [
            profile_label(command),
            command.program_name().to_string(),
            arguments.words,
        ]
        .iter()
        .flat_map(|part| part.split_whitespace())
        .collect::<Vec<_>>()
        .join(" "),
        arguments: arguments.arguments,
    }
}

/// Only the toolchain, profile and subcommand set the resource-column budget.
fn displayed_subcommand_name(command: &CommandText) -> CommandName {
    let named = command.named();
    let mut words = named.words.split_whitespace().peekable();
    let mut kept = vec![profile_label(command), command.program_name().to_string()];
    while words
        .peek()
        .is_some_and(|word| word.starts_with(crate::constants::CARGO_TOOLCHAIN_SELECTOR))
    {
        if let Some(selector) = words.next() {
            kept.push(selector.to_string());
        }
    }
    if let Some(subcommand) = words.next() {
        kept.push(subcommand.to_string());
        if subcommand == crate::constants::NEXTEST_SUBCOMMAND_NAME
            && let Some(action) = words.next()
        {
            kept.push(action.to_string());
        }
    }
    let arguments = if words.next().is_some() {
        ArgumentOmission::Dropped
    } else {
        named.arguments
    };
    CommandName {
        words: kept
            .into_iter()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join(" "),
        arguments,
    }
}

/// A missing CPU sample leaves the cell empty until a reading arrives.
fn cpu_text(reading: Measurement<f32>) -> String {
    match reading.map(census::scan::cpu_label) {
        Measurement::Reading(value) => value,
        Measurement::Unavailable(_) => String::new(),
    }
}

/// Missing resident-memory readings occupy no cells.
fn memory_text(reading: Measurement<u64>) -> String {
    match reading {
        Measurement::Reading(bytes) => memory_label(bytes),
        Measurement::Unavailable(_) => String::new(),
    }
}

/// Keep the argument mark beside the final whole word of a Short name.
fn wrapped_short_command(
    spans: Vec<Span<'static>>,
    width: u16,
    omission: ArgumentOmission,
) -> Text<'static> {
    let marker_style = spans.last().map_or_else(Style::default, |span| span.style);
    let mut spans = spans;
    if omission == ArgumentOmission::Dropped {
        while let Some(span) = spans.last_mut()
            && let Some(word) = span.content.split_whitespace().next_back()
            && cell_width(word) > width
        {
            let kept = span.content.trim_end().len() - word.len();
            span.content = span.content[..kept].trim_end().to_string().into();
            if span.content.is_empty() {
                spans.pop();
                if spans
                    .last()
                    .is_some_and(|span| span.content.trim().is_empty())
                {
                    spans.pop();
                }
            }
        }
    }
    let mut text = tui_pane::wrapped(spans, width);
    if omission == ArgumentOmission::Dropped
        && let Some(last) = text.lines.last_mut()
    {
        if cell_width(&last.to_string())
            .saturating_add(cell_width(crate::constants::ARGUMENT_OMISSION_MARK))
            <= width
        {
            last.spans.push(Span::styled(
                crate::constants::ARGUMENT_OMISSION_MARK,
                marker_style,
            ));
        } else if let Some(word) = last.spans.pop() {
            if last
                .spans
                .last()
                .is_some_and(|span| span.content.trim().is_empty())
            {
                last.spans.pop();
            }
            let empty = last.spans.is_empty();
            if empty {
                text.lines.pop();
            }
            let word_width = cell_width(word.content.as_ref());
            let mark = Span::styled(crate::constants::ARGUMENT_OMISSION_MARK, marker_style);
            if word_width.saturating_add(cell_width(mark.content.as_ref())) <= width {
                text.lines.push(Line::from(vec![word, mark]));
            } else {
                text.lines.push(Line::from(word));
                if cell_width(mark.content.as_ref()) <= width {
                    text.lines.push(Line::from(mark));
                }
            }
        }
    }
    text
}

/// Wrap complete command words at their available column width.
fn wrapped_command(spans: Vec<Span<'static>>, width: u16) -> Text<'static> {
    tui_pane::wrapped(spans, width)
}

/// The common neutral ink of cut marks, faded on the row's own ground.
fn cut_mark_ink(ground: Color, faded: u8) -> Color {
    blend_color(secondary_text_color(), ground, faded)
}

fn cut_mark_style(ground: Color, faded: u8) -> Style {
    Style::default().fg(cut_mark_ink(ground, faded))
}

/// Cells one displayed command takes before wrapping.
#[cfg(test)]
fn command_line_width(row: &TrackedRow, detail: SummaryDetail, tree: ProcessTree) -> u16 {
    cell_width(&displayed_command(&row.process.command, detail, tree))
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

/// The cell's one column-label row, drawn above the first group and
/// aligned with every group's rows by [`indented`].
fn column_header(layout: &CellTableLayout, faded: u8, plan: &ProcessTablePlan<'_>) -> Row<'static> {
    let style = column_header_style(layout, faded);
    Row::new(
        TABLE_HEADERS
            .iter()
            .enumerate()
            .filter(|(column, _)| layout.columns.contains(column))
            .map(|(column, label)| {
                let line = Line::from(Span::styled(
                    if plan.labels_column(column) {
                        (*label).to_string()
                    } else {
                        String::new()
                    },
                    style,
                ));
                if column == STATE_COLUMN {
                    line.right_aligned()
                } else {
                    line
                }
            }),
    )
}

/// Style shared by the column labels and their no-room stand-in.
fn column_header_style(layout: &CellTableLayout, faded: u8) -> Style {
    Style::default().fg(layout.ink(label_color(), faded))
}

/// One process row prepared for measurement and table rendering.
struct LaidOutProcessRow<'a> {
    /// Cells in canonical column order, filtered only when the table takes them.
    cells:   [Text<'static>; TABLE_HEADERS.len()],
    /// Stable invocation identity for viewport planning.
    source:  &'a TrackedRow,
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

impl LaidOutProcessRow<'_> {
    /// Turn the prepared cells into the columns selected for this table.
    fn into_table_row(self, layout: &CellTableLayout) -> Row<'static> {
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
        let command = displayed_command(self.command, self.detail, self.tree);
        command_line_continuation(
            &command,
            &self.cells[COMMAND_COLUMN].lines,
            first_hidden_line,
        )
    }
}

fn command_cell(
    row: &TrackedRow,
    layout: &CellTableLayout,
    arguments_text: String,
    muted: Style,
) -> Text<'static> {
    let process = &row.process;
    let faded = row.faded();
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
    if command_width == 0 {
        Text::default()
    } else {
        let mut spans = Vec::new();
        let profile = profile_label(&process.command);
        if !profile.is_empty() {
            spans.push(Span::styled(profile, cut_mark_style(layout.ground, faded)));
        }
        spans.extend([
            Span::styled(
                displayed_program(&process.command, layout.tree).to_string(),
                program,
            ),
            Span::styled(arguments_text, arguments),
        ]);
        match layout.tree {
            ProcessTree::Long => wrapped_command(spans, command_width),
            ProcessTree::Short => {
                wrapped_short_command(spans, command_width, process.command.named().arguments)
            },
        }
    }
}

/// Read a prepared height without cloning the row's styled cells.
fn process_row_lines(row: &TrackedRow, layout: &CellTableLayout) -> u16 {
    let hides_parent = layout
        .parent_links
        .hidden
        .contains(&row.process.invocation_id);
    let cached_lines = layout
        .prepared_rows
        .borrow()
        .by_pid
        .get(&row.process.pid)
        .and_then(|rows| {
            rows.iter()
                .find(|cached| cached.matches(row, layout, hides_parent))
        })
        .map(|cached| cached.lines);
    cached_lines.unwrap_or_else(|| process_row(row, layout).lines)
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
fn process_row<'a>(row: &'a TrackedRow, layout: &CellTableLayout) -> LaidOutProcessRow<'a> {
    let hides_parent = layout
        .parent_links
        .hidden
        .contains(&row.process.invocation_id);
    if let Some(cached) = layout
        .prepared_rows
        .borrow()
        .by_pid
        .get(&row.process.pid)
        .and_then(|rows| {
            rows.iter()
                .find(|cached| cached.matches(row, layout, hides_parent))
        })
    {
        return LaidOutProcessRow {
            cells:   cached.cells.clone(),
            lines:   cached.lines,
            source:  row,
            command: &row.process.command,
            detail:  layout.detail,
            tree:    layout.tree,
        };
    }

    let values =
        ProcessRowValues::for_use(row, layout.detail, layout.tree, ProcessValueUse::SelectRows);
    let process = &values.source.process;
    let faded = row.faded();
    let muted = Style::default().fg(layout.ink(label_color(), faded));
    let command = command_cell(row, layout, values.arguments, muted);
    let lines = u16::try_from(command.height()).unwrap_or(u16::MAX).max(1);
    let pid = number_text_if_fits(
        values.cells[PID_COLUMN].clone(),
        layout.column_width(PID_COLUMN),
    );
    let parent = number_text_if_fits(
        layout.parent_links.text(process),
        layout.column_width(PARENT_COLUMN),
    );
    let cells = [
        Text::from(Span::styled(
            pid,
            pid_style(row, layout, &AllocationFamilyLinks::NoDrawnLinks),
        )),
        Text::from(Span::styled(
            parent,
            parent_style(row, layout, &AllocationFamilyLinks::NoDrawnLinks),
        )),
        Text::from(Span::styled(values.cells[START_COLUMN].clone(), muted)),
        Text::from(Span::styled(values.cells[DURATION_COLUMN].clone(), muted)),
        Text::from(Span::styled(values.cells[CPU_COLUMN].clone(), muted)),
        Text::from(Span::styled(values.cells[MEMORY_COLUMN].clone(), muted)),
        Text::from(state_cell(row, layout)),
        command,
        Text::default(),
        Text::from(Span::styled(values.cells[MANAGED_COLUMN].clone(), muted)),
    ];
    layout
        .prepared_rows
        .borrow_mut()
        .by_pid
        .entry(row.process.pid)
        .or_default()
        .push(CachedProcessRow {
            process: process.clone(),
            faded,
            cells: cells.clone(),
            lines,
            widths: layout.column_widths.clone(),
            columns: layout.columns.clone(),
            ground: layout.ground,
            hides_parent,
        });
    LaidOutProcessRow {
        cells,
        lines,
        source: row,
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
fn pid_style(row: &TrackedRow, layout: &CellTableLayout, links: &AllocationFamilyLinks) -> Style {
    let family = if links.heads(&row.process.invocation_id) {
        row.family()
    } else {
        FamilyHead::NoChildren
    };
    let color = match family {
        FamilyHead::Heads(family) => theme::family_color(family),
        FamilyHead::NoChildren => text_default(),
    };
    Style::default().fg(layout.ink(color, row.faded()))
}

/// The `parent` cell's style: the family colour of the cargo named,
/// which is the colour that cargo's own `pid` cell carries, and plain
/// text where the pid names something that heads no family.
fn parent_style(
    row: &TrackedRow,
    layout: &CellTableLayout,
    links: &AllocationFamilyLinks,
) -> Style {
    let family = match &row.process.parent {
        VisibleParent::Invocation { id, pid } if links.contains(id, *pid) => row.parent_family(),
        VisibleParent::Invocation { .. } | VisibleParent::Ancestor(_) | VisibleParent::None => {
            ParentFamily::NoFamily
        },
    };
    let color = match family {
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
        .flex(Flex::Start)
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
/// unavailable measurements. The state slot identifies waits and independently
/// owned readings when a directory contains more than one progressing command.
#[cfg(test)]
fn visible_columns(rows: &[&TrackedRow], kind: TableKind) -> Vec<usize> {
    FrameTableColumnGrid::of(rows, kind, ProcessTree::Long).slots
}
/// A wait or the invocation's own reading when its directory has several.
fn state_cell(row: &TrackedRow, layout: &CellTableLayout) -> Line<'static> {
    if layout.row_progress.contains(&row.process.invocation_id)
        && let CounterState::Working { progress, .. } = own_counter(row)
    {
        return Line::from(Span::styled(
            percent_reading(progress),
            Style::default().fg(layout.ink(
                if row.is_ended() {
                    label_color()
                } else {
                    success_color()
                },
                row.faded(),
            )),
        ))
        .right_aligned();
    }
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
    .right_aligned()
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

/// Which invocation owns the capture's numeric progress.
enum InvocationProgressOwnership {
    OwnInvocation,
    EnclosingInvocation,
}

const fn progress_ownership(row: &TrackedRow) -> InvocationProgressOwnership {
    if matches!(row.process.provenance, RowProvenance::Enclosing(_))
        || !matches!(
            row.process.capture_membership,
            crate::census::process_identity::CaptureMembership::Outside
        )
    {
        InvocationProgressOwnership::EnclosingInvocation
    } else {
        InvocationProgressOwnership::OwnInvocation
    }
}

/// A reading owned by this invocation rather than an enclosing capture.
fn own_counter(row: &TrackedRow) -> CounterState {
    match progress_ownership(row) {
        InvocationProgressOwnership::OwnInvocation => CounterState::from(&row.process.state),
        InvocationProgressOwnership::EnclosingInvocation => CounterState::NoCurrentProgress,
    }
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
fn heading_gauge(
    group: &PathGroup<'_>,
    width: u16,
    layout: &CellTableLayout,
) -> Vec<Span<'static>> {
    if layout.kind == TableKind::Summary {
        return Vec::new();
    }
    let mut readings = group
        .rows
        .iter()
        .filter(|row| !layout.row_progress.contains(&row.process.invocation_id))
        .filter_map(|row| match own_counter(row) {
            CounterState::Working { phase, progress } => Some((row, (phase, progress))),
            _ => None,
        });
    let Some((row, (phase, progress))) = readings.next() else {
        return Vec::new();
    };
    if readings.next().is_some() {
        return Vec::new();
    }
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
/// nothing for a measured zero or an unavailable count.
fn managed_text(process: &CargoProcess) -> String {
    match process.managed {
        Measurement::Reading(0) | Measurement::Unavailable(_) => String::new(),
        Measurement::Reading(count) => count.to_string(),
    }
}

/// Resident bytes as gibibytes to one decimal place, rounded half up.
fn memory_label(bytes: u64) -> String {
    let tenths = (u128::from(bytes) * 10 + u128::from(BYTES_PER_GIBIBYTE) / 2)
        / u128::from(BYTES_PER_GIBIBYTE);
    format!("{}.{}{MEMORY_UNIT}", tenths / 10, tenths % 10)
}

/// The `compiler` cell: driver name in the active color, its count muted
/// beside it, and nothing when observation is unknown or no
/// compile is in flight.
/// Compiler counts are styled after their numeric contributors have been selected.
fn compiler_reading(
    compiler: &CompilerObservation,
    row: &TrackedRow,
    layout: &CellTableLayout,
) -> Line<'static> {
    let faded = row.faded();
    let driver = if row.is_ended() {
        label_color()
    } else {
        success_color()
    };
    let name = Style::default().fg(layout.ink(driver, faded));
    let count = Style::default().fg(layout.ink(label_color(), faded));
    match compiler {
        CompilerObservation::Unknown | CompilerObservation::None => Line::default(),
        CompilerObservation::Running(compiler) => Line::from(vec![
            Span::styled(compiler.name, name),
            Span::styled(format!("\u{d7}{}", compiler.count), count),
        ]),
    }
}

/// `area` indented one level, where the column labels and every group's
/// rows both sit — the same two-level hierarchy the framework overlays
/// use, with the working directory heading each group at the outer level.
fn indented(area: Rect) -> Rect {
    let indent = cell_width(SECTION_ITEM_INDENT);
    Rect {
        x: area.x.saturating_add(indent),
        width: area
            .width
            .saturating_sub(indent)
            .saturating_sub(CONTENT_RIGHT_INSET),
        ..area
    }
}

/// Content width with the cell next to the right frame kept clear.
const fn cleared(area: Rect) -> Rect {
    Rect {
        width: area.width.saturating_sub(CONTENT_RIGHT_INSET),
        ..area
    }
}

/// A string's width in cells, clamped into the column-width type.
fn cell_width(text: &str) -> u16 { u16::try_from(UnicodeWidthStr::width(text)).unwrap_or(u16::MAX) }

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
    let summary = summary_rows(roster, hidden_when_idle);
    let rows: Vec<_> = summary.iter().map(AsRef::as_ref).collect();
    let contributions: Vec<_> = roster
        .groups()
        .iter()
        .flat_map(TrackedGroup::rows)
        .map(InvocationContribution::from)
        .collect();
    row_resource_readings(&rows, &contributions)
        .into_iter()
        .map(|reading| {
            (
                reading.pid,
                reading.resources.cpu.map(census::scan::cpu_label),
            )
        })
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
    std::thread_local! {
        /// Intrinsic demand queries made by the live frame on this thread.
        pub(super) static FRAME_DEMAND_QUERIES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    }
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
    use std::time::Duration;
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
    use tui_pane::ToastActivation;
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
    use crate::census::scan::AncestryWalk;
    use crate::census::scan::CensusSequence;
    use crate::census::scan::Compiler;
    use crate::census::scan::ProcessField;
    use crate::census::scan::ProcessObservation;
    use crate::census::scan::ProcessObservations;
    use crate::constants::COMPILER_PROCESS_NAMES;
    use crate::constants::LARGE_TABLE_TEST_AREA;
    use crate::constants::LARGE_TABLE_TEST_COMMAND;
    use crate::constants::LARGE_TABLE_TEST_FIRST_PID;
    use crate::constants::LARGE_TABLE_TEST_ROWS;
    use crate::constants::LOCK_WAIT_MARKER;
    use crate::constants::PHASE_TESTING;
    use crate::constants::PORT_SUBCOMMAND_NAME;
    use crate::constants::REGISTRATION_MAGIC;
    use crate::constants::UNRESOLVED_PATH;
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

    fn clippy_with_unconfirmed_child() -> CargoGroup {
        let root = tempfile::tempdir().expect("capture root");
        fs::create_dir_all(root.path().join("state/pids")).expect("registration directory");
        let magic = std::str::from_utf8(REGISTRATION_MAGIC).expect("registration magic");
        fs::write(root.path().join("state/pids/741480.generation"),
            format!("{magic}\0generation\0boot\x00100\0run.log\0/run/github-runner/hana-linux-2/hana/hana\0/runner\x002\0clippy\0--workspace\0"))
            .expect("registration");
        fs::write(
            root.path().join("run.log"),
            "Building [=>] 837/1000: checking\r",
        )
        .expect("progress");
        let capture = Capture::take_from(root.path(), |pid| {
            KernelObservation::for_test(pid, Observation::Unknown)
        });
        assert_eq!(capture.confirmed().len(), 0);
        let lead_argv = ["cargo", "clippy", "--workspace"].map(OsString::from);
        let child_argv = ["cargo", "check", "--workspace"].map(OsString::from);
        let records = ProcessObservations::new(
            [
                (741_928, 741_480, &lead_argv),
                (742_007, 741_928, &child_argv),
            ]
            .map(|(pid, parent, argv)| {
                let mut process = ProcessObservation::cargo(pid, argv);
                process.parent = ProcessField::Observed(Pid::from_u32(parent));
                process.directory =
                    ProcessField::Observed(Path::new("/run/github-runner/hana-linux-2/hana/hana"));
                process
            }),
        );
        let mut groups = CensusSequence::default().sample_capture(&records, &capture);
        assert_eq!(groups.len(), 1);
        let mut group = groups.remove(0);
        assert_eq!(group.lead.pid, 741_928);
        assert_eq!(group.rest.len(), 1);
        for process in std::iter::once(&mut group.lead).chain(&mut group.rest) {
            assert!(matches!(process.provenance, RowProvenance::Uncaptured(_)));
            process.provenance = RowProvenance::Uncaptured(ProcessOwner::Observed(Account {
                uid:  2000,
                name: AccountName::Resolved("hana-linux-2".into()),
            }));
            process.path = "~/hana".into();
            process.resource_use.cpu =
                Measurement::Reading(if process.pid == 741_928 { 0.0 } else { 440.0 });
            process.resource_use.memory = Measurement::Reading(BYTES_PER_GIBIBYTE);
        }
        group
    }

    #[test]
    fn an_invocations_child_keeps_resource_columns_without_repeating_its_reading() {
        let mut app = App::new_for_test().expect("test app");
        app.loaded_config.config.tiles.view = TileView::Cells;
        let group = clippy_with_unconfirmed_child();
        let mut other = invocation(750_000, &["nextest", "run", "--workspace"]);
        other.resource_use.memory = Measurement::Reading(BYTES_PER_GIBIBYTE);
        let mut cutting = invocation(760_000, &["nextest", "run", "--workspace"]);
        cutting.state = CaptureLookup::Registered(CaptureRead::Progress(RunState::Blocked));
        let cut_group = CargoGroup {
            lead:     cutting,
            rest:     (760_001..760_031)
                .map(|pid| invocation(pid, &["check"]))
                .collect(),
            ancestry: AncestryWalk::Ended(vec![]),
        };
        app.roster
            .observe(vec![group, cargo_group(other), cut_group], Instant::now());
        let area = Rect::new(0, 0, 64, 50);
        let mut terminal =
            Terminal::new(TestBackend::new(area.width, area.height)).expect("terminal");
        for _ in 0..3 {
            app.tiles.settle_for_test();
            terminal
                .draw(|frame| {
                    draw_panes(frame, &mut app, area, TileGridContents::Shown);
                })
                .expect("frame");
        }
        let placements = app
            .tiles
            .placements(area, app.loaded_config.config.tiles.growth());
        let cell = placements
            .iter()
            .find(|placement| {
                placement.content == TileContent::Group(InvocationId::for_test(741_928))
            })
            .expect("runner cell");
        let text = placement_text(terminal.backend().buffer(), cell);
        assert_eq!(text.matches("83.7%").count(), 1, "{text}");
        let owner = text
            .lines()
            .find(|line| line.contains("741928") && line.contains("clippy"))
            .expect("owner row");
        assert!(!owner.contains("83.7%"), "{text}");
        assert!(text.lines().nth(1).unwrap().contains("83.7%"), "{text}");
        let child = text
            .lines()
            .find(|line| line.contains("742007") && line.contains("check"))
            .expect("child row");
        assert!(!child.contains("83.7%"), "{text}");
        assert!(text.lines().next().unwrap().contains("cpu"), "{text}");
        assert!(text.lines().next().unwrap().contains("mem"), "{text}");
        for pid in [750_000, 760_000] {
            let other = placements
                .iter()
                .find(|placement| {
                    placement.content == TileContent::Group(InvocationId::for_test(pid))
                })
                .expect("other cell");
            let other_text = placement_text(terminal.backend().buffer(), other);
            for column in ["cpu", "mem"] {
                assert_eq!(
                    text.lines().next().unwrap().find(column),
                    other_text.lines().next().unwrap().find(column),
                    "{text}\n{other_text}"
                );
            }
        }
        assert!(owner.contains("0%") && owner.contains("1.0G"), "{text}");
        assert!(child.contains("440%") && child.contains("1.0G"), "{text}");
        let summary = placements
            .iter()
            .find(|placement| placement.content == TileContent::Summary)
            .expect("summary");
        assert_eq!(
            placement_text(terminal.backend().buffer(), summary)
                .matches("83.7%")
                .count(),
            1
        );
    }

    fn blocked_command_frame(enclosed_child: bool) -> App {
        let mut app = App::new_for_test().expect("test app");
        app.loaded_config.config.tiles.view = TileView::Cells;
        app.tiles.set_min_tile_height(4);
        let mut groups = Vec::new();
        for (pid, arguments) in [
            (840_000, vec!["build", "--workspace"]),
            (850_000, vec!["build", "--profile", "p20h", "--tests"]),
            (860_000, vec!["nextest", "run", "--workspace"]),
        ] {
            let mut lead = invocation(pid, &arguments);
            lead.resource_use.memory = Measurement::Reading(BYTES_PER_GIBIBYTE);
            if pid == 840_000 {
                lead.state = CaptureLookup::Registered(CaptureRead::Progress(RunState::Blocked));
            }
            let mut group = cargo_group(lead);
            if pid == 840_000 && enclosed_child {
                let mut child = invocation(840_001, &["check"]);
                child.capture_membership = CaptureMembership::EnclosingUnconfirmed;
                child.state = group.lead.state.clone();
                child.parent = VisibleParent::Invocation {
                    id: group.lead.invocation_id.clone(),
                    pid,
                };
                child.resource_use.memory = Measurement::Reading(BYTES_PER_GIBIBYTE);
                group.rest.push(child);
            }
            if pid == 840_000 {
                group
                    .rest
                    .extend((840_002..840_032).map(|pid| invocation(pid, &["check"])));
            }
            groups.push(group);
        }
        app.roster.observe(groups, Instant::now());
        app
    }

    #[test]
    fn a_plain_blocked_frame_aligns_its_resource_columns() {
        let mut app = blocked_command_frame(false);
        let terminal = draw_nested_progress(&mut app);
        let placements = app.tiles.placements(
            Rect::new(0, 0, 64, 50),
            app.loaded_config.config.tiles.growth(),
        );
        let mut offsets = Vec::new();
        for pid in [840_000, 850_000, 860_000] {
            let cell = placements
                .iter()
                .find(|placement| {
                    placement.content == TileContent::Group(InvocationId::for_test(pid))
                })
                .expect("command cell");
            let text = placement_text(terminal.backend().buffer(), cell);
            let header = text.lines().next().unwrap();
            let columns = ["cpu", "mem"].map(|title| header.find(title).expect(&text));
            assert!(text.contains("1.0G"), "{text}");
            offsets.push(columns);
        }
        assert!(
            offsets.iter().all(|offset| *offset == offsets[0]),
            "{offsets:?}"
        );
    }

    #[test]
    fn one_row_cells_keep_their_subcommands_in_a_blocked_frame() {
        let mut app = blocked_command_frame(true);
        let terminal = draw_nested_progress(&mut app);
        let placements = app.tiles.placements(
            Rect::new(0, 0, 64, 50),
            app.loaded_config.config.tiles.growth(),
        );
        for (pid, command) in [
            (850_000, "[p20h] cargo build …"),
            (860_000, "cargo nextest run …"),
        ] {
            let cell = placements
                .iter()
                .find(|placement| {
                    placement.content == TileContent::Group(InvocationId::for_test(pid))
                })
                .expect("one-row cell");
            let text = placement_text(terminal.backend().buffer(), cell);
            assert_eq!(cell.frame.inner().height, 4, "{text}");
            assert!(text.contains(command), "{text}");
            assert!(!text.contains("cargo …"), "{text}");
        }
    }

    fn mend_with_unconfirmed_child() -> CargoGroup {
        let root = tempfile::tempdir().expect("capture root");
        fs::create_dir_all(root.path().join("state/pids")).expect("registration directory");
        let magic = std::str::from_utf8(REGISTRATION_MAGIC).expect("magic");
        fs::write(
            root.path().join("state/pids/741928.generation"),
            format!(
                "{magic}\0generation\0boot\x00100\0run.log\0/runner/project\0/runner\x001\0mend\0"
            ),
        )
        .expect("registration");
        fs::write(
            root.path().join("run.log"),
            "Building [=>] 998/1000: checking\r",
        )
        .expect("log");
        let capture = Capture::take_from(root.path(), |pid| {
            KernelObservation::for_test(pid, Observation::Unknown)
        });
        let shell = [OsString::from("bash")];
        let driver = ["cargo-mend", "mend"].map(OsString::from);
        let child = ["cargo", "check", "--workspace"].map(OsString::from);
        let records = ProcessObservations::new(
            [
                (741_480, 1, "bash", shell.as_slice()),
                (741_928, 741_480, "cargo-mend", driver.as_slice()),
                (742_007, 741_928, "cargo", child.as_slice()),
            ]
            .map(|(pid, parent, name, argv)| {
                let mut process = ProcessObservation::cargo(pid, argv);
                process.name = ProcessField::Observed(std::ffi::OsStr::new(name));
                process.parent = ProcessField::Observed(Pid::from_u32(parent));
                process.uid = ProcessField::Observed(2000);
                process.directory = ProcessField::Observed(Path::new("/runner/project"));
                process
            }),
        );
        let mut groups = CensusSequence::default().sample_capture(&records, &capture);
        assert_eq!(groups.len(), 1);
        let group = &mut groups[0];
        for process in std::iter::once(&mut group.lead).chain(&mut group.rest) {
            process.resource_use.cpu = Measurement::Reading(12.0);
            process.resource_use.memory = Measurement::Reading(BYTES_PER_GIBIBYTE);
        }
        groups.remove(0)
    }

    #[test]
    fn a_driver_child_borrows_progress_while_its_heading_keeps_the_reading() {
        let mut groups = vec![mend_with_unconfirmed_child()];
        let mut other = invocation(750_000, &["build", "--workspace"]);
        other.resource_use.memory = Measurement::Reading(BYTES_PER_GIBIBYTE);
        groups.push(cargo_group(other));
        let mut app = App::new_for_test().expect("app");
        app.loaded_config.config.tiles.view = TileView::Cells;
        app.roster.observe(groups, Instant::now());
        let terminal = draw_nested_progress(&mut app);
        let placements = app.tiles.placements(
            Rect::new(0, 0, 64, 50),
            app.loaded_config.config.tiles.growth(),
        );
        let text = placements
            .iter()
            .filter_map(|cell| match cell.content {
                TileContent::Group(_) => Some(placement_text(terminal.backend().buffer(), cell)),
                TileContent::Summary | TileContent::Empty(_) => None,
            })
            .collect::<Vec<_>>();
        let driver = text
            .iter()
            .find(|text| text.contains("741928"))
            .expect("driver cell");
        assert_eq!(driver.matches("99.8%").count(), 1, "{driver}");
        assert!(
            driver
                .lines()
                .find(|line| line.contains("/runner/project"))
                .unwrap()
                .contains("99.8%"),
            "{driver}"
        );
        for pid in ["741928", "742007"] {
            let row = driver
                .lines()
                .find(|line| {
                    line.contains(pid) && (line.contains("mend") || line.contains("check"))
                })
                .expect("row");
            assert!(!row.contains("99.8%"), "{driver}");
        }
        for column in ["cpu", "mem"] {
            let offsets = text
                .iter()
                .map(|text| {
                    text.lines()
                        .find(|line| line.contains("command") && line.contains("cpu"))
                        .unwrap()
                        .find(column)
                        .expect(text)
                })
                .collect::<Vec<_>>();
            assert!(
                offsets.iter().all(|offset| *offset == offsets[0]),
                "{text:#?}"
            );
        }
        let summary = placements
            .iter()
            .find(|cell| cell.content == TileContent::Summary)
            .expect("summary");
        assert_eq!(
            placement_text(terminal.backend().buffer(), summary)
                .matches("99.8%")
                .count(),
            1
        );
    }

    #[test]
    fn working_invocations_share_resource_offsets_with_an_ordinary_cell() {
        for tree in [ProcessTree::Short, ProcessTree::Long] {
            let mut app = App::new_for_test().expect("app");
            app.loaded_config.config.tiles.view = TileView::Cells;
            app.tree = tree;
            app.tiles.set_min_tile_height(4);
            let mut first = invocation(870_000, &["nextest", "run", "--workspace"]);
            first.state = CaptureLookup::Registered(CaptureRead::Progress(compiling(1, 2)));
            first.resource_use.memory = Measurement::Reading(BYTES_PER_GIBIBYTE);
            let mut second = invocation(870_001, &["nextest", "run", "--workspace"]);
            second.state = first.state.clone();
            second.resource_use.memory = first.resource_use.memory;
            let mut other = invocation(
                880_000,
                &[
                    "nextest",
                    "run",
                    "--workspace",
                    "--all-targets",
                    "--no-capture",
                    "--run-ignored",
                    "only",
                ],
            );
            other.path = "~/other".into();
            other.directory_identity = WorkingDirectoryIdentity::Absolute("/other".into());
            other.resource_use.memory = first.resource_use.memory;
            let mut rest = vec![second];
            rest.extend((870_002..870_032).map(|pid| invocation(pid, &["check"])));
            let mut neighbour = invocation(890_000, &["build", "--workspace"]);
            neighbour.path = "~/neighbour".into();
            neighbour.directory_identity = WorkingDirectoryIdentity::Absolute("/neighbour".into());
            neighbour.resource_use.memory = first.resource_use.memory;
            app.roster.observe(
                vec![
                    CargoGroup {
                        lead: first,
                        rest,
                        ancestry: AncestryWalk::Ended(vec![]),
                    },
                    cargo_group(other),
                    cargo_group(neighbour),
                ],
                Instant::now(),
            );
            let terminal = draw_nested_progress(&mut app);
            let placements = app.tiles.placements(
                Rect::new(0, 0, 64, 50),
                app.loaded_config.config.tiles.growth(),
            );
            let headers = placements
                .iter()
                .filter(|cell| matches!(cell.content, TileContent::Group(_)))
                .map(|cell| {
                    placement_text(terminal.backend().buffer(), cell)
                        .lines()
                        .next()
                        .unwrap()
                        .to_string()
                })
                .collect::<Vec<_>>();
            assert_eq!(headers.len(), 3);
            for header in &headers[1..] {
                for title in ["cpu", "mem"] {
                    assert_eq!(
                        headers[0].find(title).expect(&headers[0]),
                        header.find(title).expect(header),
                        "{headers:#?}"
                    );
                }
                assert_eq!(
                    headers[0].contains("dur"),
                    header.contains("dur"),
                    "{headers:#?}"
                );
            }
        }
    }

    #[test]
    fn a_state_frame_cuts_argument_tails_before_its_memory_column() {
        let mut app = App::new_for_test().expect("app");
        app.loaded_config.config.tiles.view = TileView::Cells;
        app.tiles.set_min_tile_height(4);
        app.roster = Roster::for_test(SessionColumnWidths::for_machine(
            256,
            128 * BYTES_PER_GIBIBYTE,
        ));
        let mut groups = vec![];
        for (pid, arguments) in [
            (
                1_840_000,
                vec![
                    "nextest",
                    "run",
                    "--workspace",
                    "--run-ignored",
                    "only",
                    "-E",
                    "test(a_very_long_test_filter)",
                    "--no-capture",
                ],
            ),
            (1_850_000, vec!["build", "--profile", "p20i", "--tests"]),
            (1_860_000, vec!["check", "--workspace"]),
        ] {
            let mut process = invocation(pid, &arguments);
            process.resource_use.memory = Measurement::Reading(BYTES_PER_GIBIBYTE);
            if pid == 1_840_000 {
                process.state = CaptureLookup::Registered(CaptureRead::Progress(RunState::Blocked));
                process.parent = VisibleParent::Invocation {
                    id:  InvocationId::for_test(1_830_000),
                    pid: 1_830_000,
                };
            }
            let mut group = cargo_group(process);
            if pid == 1_840_000 {
                let mut child = invocation(
                    1_840_001,
                    &["test", "a_very_long_positional_test_filter", "--no-capture"],
                );
                child.resource_use.memory = Measurement::Reading(BYTES_PER_GIBIBYTE);
                group.rest.push(child);
                group
                    .rest
                    .extend((1_840_002..1_840_032).map(|pid| invocation(pid, &["check"])));
            }
            groups.push(group);
        }
        app.roster.observe(groups, Instant::now());
        let terminal = draw_nested_progress(&mut app);
        let placements = app.tiles.placements(
            Rect::new(0, 0, 64, 50),
            app.loaded_config.config.tiles.growth(),
        );
        let mut offsets = vec![];
        for (pid, command) in [
            (1_840_000, "cargo nextest run"),
            (1_850_000, "[p20i] cargo build"),
            (1_860_000, "cargo check"),
        ] {
            let cell = placements
                .iter()
                .find(|cell| cell.content == TileContent::Group(InvocationId::for_test(pid)))
                .expect("cell");
            let text = placement_text(terminal.backend().buffer(), cell);
            let header = text.lines().next().unwrap();
            offsets.push(["cpu", "mem"].map(|title| header.find(title).expect(&text)));
            assert!(
                !header.contains("start") && !header.contains("dur"),
                "{text}"
            );
            assert!(text.contains(command), "{text}");
            assert!(text.contains("1.0G"), "{text}");
        }
        assert!(
            offsets.iter().all(|offset| *offset == offsets[0]),
            "{offsets:?}"
        );
    }

    fn hidden_state_frame(tree: ProcessTree) -> App {
        hidden_state_frame_with_arguments(tree, &["clippy", "--locked", "--offline", "-j", "1"])
    }

    fn hidden_state_frame_with_arguments(tree: ProcessTree, blocked_arguments: &[&str]) -> App {
        let mut app = App::new_for_test().expect("app");
        app.loaded_config.config.tiles.view = TileView::Cells;
        app.tiles.set_min_tile_height(6);
        app.loaded_config.config.tiles.initial_rows =
            toml::from_str::<crate::config::TilesConfig>("initial_rows = 12")
                .expect("tile config")
                .initial_rows;
        app.loaded_config.config.commands.hidden_when_idle.clear();
        app.tree = tree;
        app.roster = Roster::for_test(SessionColumnWidths::for_machine(
            64,
            64 * BYTES_PER_GIBIBYTE,
        ));
        let mut groups = Vec::new();
        for index in 0..8 {
            let pid = 3_160_000 + index * 100;
            let arguments = match index {
                0 | 1 => vec!["clippy", "--locked", "--offline", "-j", "1"],
                2 => vec!["build", "--release", "--locked", "--offline", "-j", "1"],
                _ => vec!["check", "--locked", "--offline", "-j", "1"],
            };
            let mut lead = invocation(
                pid,
                if index == 1 {
                    blocked_arguments
                } else {
                    &arguments
                },
            );
            lead.path = format!("~/work-{index}");
            lead.directory_identity =
                WorkingDirectoryIdentity::Absolute(format!("/test-home/work-{index}").into());
            lead.parent = VisibleParent::Ancestor(3_150_000);
            lead.resource_use.memory = Measurement::Reading(BYTES_PER_GIBIBYTE);
            lead.state = CaptureLookup::Registered(CaptureRead::Progress(compiling(1, 2)));
            if index == 1 {
                lead.state = CaptureLookup::Registered(CaptureRead::Progress(RunState::Blocked));
            }
            let mut group = cargo_group(lead);
            if index == 0 {
                group.rest.extend((1..32).map(|offset| {
                    let mut child = invocation(pid + offset, &["check", "--locked", "--offline"]);
                    child.path = group.lead.path.clone();
                    child.directory_identity = group.lead.directory_identity.clone();
                    child.parent = VisibleParent::Invocation {
                        id: InvocationId::for_test(pid),
                        pid,
                    };
                    child.resource_use.memory = Measurement::Reading(BYTES_PER_GIBIBYTE);
                    child
                }));
            }
            groups.push(group);
        }
        app.roster.observe(groups, Instant::now());
        app
    }

    fn subcommand_frame(tree: ProcessTree, blocked: &[&str], first_pid: u32) -> App {
        let mut app = hidden_state_frame_with_arguments(tree, blocked);
        let groups = (0..8)
            .map(|index| {
                let pid = first_pid + index * 100;
                let arguments = match index {
                    1 => blocked,
                    2 => &["build", "--release", "--locked", "--offline"][..],
                    _ => &["clippy", "--locked", "--offline", "-j", "1"][..],
                };
                let mut lead = invocation(pid, arguments);
                lead.parent = VisibleParent::Ancestor(first_pid - 100);
                lead.resource_use.memory = Measurement::Reading(BYTES_PER_GIBIBYTE);
                lead.path = format!("~/work-{index}");
                lead.directory_identity =
                    WorkingDirectoryIdentity::Absolute(format!("/test-home/work-{index}").into());
                if index == 1 {
                    lead.state =
                        CaptureLookup::Registered(CaptureRead::Progress(RunState::Blocked));
                }
                lead.managed = Measurement::Reading(1);
                let mut group = cargo_group(lead);
                if index == 0 {
                    group.rest = (1..12)
                        .map(|offset| {
                            let mut child = invocation(pid + offset, &["check", "--locked"]);
                            child.parent = VisibleParent::Invocation {
                                id: InvocationId::for_test(pid),
                                pid,
                            };
                            child.path = group.lead.path.clone();
                            child.directory_identity = group.lead.directory_identity.clone();
                            child
                        })
                        .collect();
                }
                group
            })
            .collect();
        app.roster = Roster::for_test(SessionColumnWidths::for_machine(
            64,
            64 * BYTES_PER_GIBIBYTE,
        ));
        app.roster.observe(groups, Instant::now());
        app
    }

    fn assert_frame_column_priority(
        grids: &FrameColumnGrids,
        tree: ProcessTree,
        pid: u32,
        failures: &mut Vec<String>,
    ) {
        // These nested header sets pin the requested shared priority.
        // Argument tails use only the space left after the whole subcommand.
        let priorities = [
            "pid command",
            "pid parent command",
            "pid parent state command",
            "pid parent cpu state command",
            "pid parent cpu mem state command",
            "pid parent dur cpu mem state command",
            "pid parent start dur cpu mem state command",
            "pid parent start dur cpu mem state command runs",
            "pid parent start dur cpu mem state command compiler runs",
        ];
        for grid in [&grids.commands.0, &grids.summary.0] {
            grid.row_end.set(TableRowEnd::Reserved);
            let expected_sets: Vec<_> = priorities
                .iter()
                .map(|titles| {
                    let columns: Vec<_> = titles
                        .split_whitespace()
                        .filter_map(|title| {
                            TABLE_HEADERS.iter().position(|header| *header == title)
                        })
                        .filter(|column| grid.slots.contains(column))
                        .collect();
                    let used = columns
                        .iter()
                        .map(|&column| {
                            if column == COMMAND_COLUMN {
                                cell_width("[release] cargo build …")
                            } else {
                                grid.intrinsic_widths[column]
                            }
                        })
                        .sum::<u16>()
                        + u16::try_from(columns.len().saturating_sub(1)).unwrap()
                        + 2;
                    (used, columns)
                })
                .collect();
            for width in 36..=220 {
                let available = width - 4;
                let expected = expected_sets
                    .iter()
                    .rev()
                    .find(|(used, _)| *used <= available)
                    .map_or(&[PID_COLUMN, COMMAND_COLUMN][..], |(_, columns)| {
                        columns.as_slice()
                    });
                let columns = grid.columns_for_command_name(available).columns;
                if !expected.iter().all(|column| columns.contains(column)) {
                    failures.push(format!(
                        "{tree:?}, pid {pid}, width {width}, {:?}: expected {expected:?}, got {:?}",
                        grid.kind, columns
                    ));
                }
            }
        }
    }

    /// Paint every table with the same prepared provider and a hidden-row allocation.
    fn shot_table_texts(cells: &Cells<'_>, width: u16) -> Vec<String> {
        cells
            .roster
            .groups()
            .iter()
            .enumerate()
            .map(|(index, group)| {
                let area = Rect::new(0, 0, width - 2, if index == 0 { 4 } else { 8 });
                let mut buffer = Buffer::empty(area);
                cells.draw(
                    &mut buffer,
                    &TileContent::Group(group.id.clone()),
                    area,
                    Color::Reset,
                );
                buffer_rows(&buffer).join("\n")
            })
            .collect()
    }

    fn assert_shot_frame_columns(
        cells: &Cells<'_>,
        tree: ProcessTree,
        arguments: &[&str],
        pid: u32,
        shot_width: u16,
    ) {
        let texts = shot_table_texts(cells, shot_width);
        assert_eq!(texts.len(), 8);
        for text in &texts {
            let header = text
                .lines()
                .find(|line| line.contains("command"))
                .expect(text);
            if shot_width == 48 {
                assert!(
                    header.contains("parent") && !header.contains("state"),
                    "{text}"
                );
            }
            if tree == ProcessTree::Long && pid >= 3_920_000 {
                assert!(header.contains("mem"), "{text}");
            }
            let command = if text.contains(&(pid + 200).to_string()) {
                "[release] cargo build"
            } else if arguments[0] == "install" && text.contains(&(pid + 100).to_string()) {
                "cargo install"
            } else {
                "cargo clippy"
            };
            assert!(text.contains(command), "{text}");
        }
        if pid == 860_000 {
            let wide = shot_table_texts(cells, 200);
            for title in ["cpu", "mem", "command", "runs"] {
                let offsets: Vec<_> = wide
                    .iter()
                    .map(|text| {
                        text.lines()
                            .find(|line| line.contains("command"))
                            .and_then(|line| line.find(title))
                    })
                    .collect();
                assert!(
                    offsets[0].is_some() && offsets.iter().all(|offset| *offset == offsets[0]),
                    "{title}: {wide:#?}"
                );
            }
        }
        if shot_width == 48 {
            let narrow = shot_table_texts(cells, 40);
            assert!(
                narrow
                    .iter()
                    .any(|text| text.contains("[release] cargo build")),
                "{narrow:#?}"
            );
        }
    }

    fn assert_main_state_columns(cells: &Cells<'_>, width: u16, failures: &mut Vec<String>) {
        let width = if width == 48 { 64 } else { width };
        let tree = cells.grids.commands.0.tree;
        for group in cells.roster.groups() {
            if !matches!(
                group.lead.process.state,
                CaptureLookup::Registered(CaptureRead::Progress(RunState::Blocked))
            ) {
                continue;
            }
            let area = Rect::new(0, 0, width - 2, 8);
            let mut buffer = Buffer::empty(area);
            cells.draw(
                &mut buffer,
                &TileContent::Group(group.id.clone()),
                area,
                Color::Reset,
            );
            let text = buffer_rows(&buffer).join("\n");
            let header = buffer_line(&buffer, 0);
            if width >= 54 && (!header.contains("state") || !text.contains("blocked")) {
                failures.push(format!("filled state at {width}, {tree:?}: {text}"));
            }
            if width == 64
                && (!header.contains("cpu")
                    || (tree == ProcessTree::Long && !header.contains("mem")))
            {
                failures.push(format!("resources beside state: {text}"));
            }
        }
        let area = Rect::new(0, 0, 62, 3);
        let mut buffer = Buffer::empty(area);
        cells.draw(&mut buffer, &TileContent::Summary, area, Color::Reset);
        let header = buffer_line(&buffer, 0);
        if !header.contains("start") || !header.contains("dur") {
            failures.push(format!("summary without state keeps timing: {header}"));
        }
        let command = header.find("command").unwrap();
        let mem = header.find("mem").unwrap();
        if command.saturating_sub(mem) > 8 {
            failures.push(format!("summary has a blank state slot: {header}"));
        }
    }

    #[derive(Clone, Copy)]
    struct ShotColumnFixture {
        tree:       ProcessTree,
        arguments:  &'static [&'static str],
        pid:        u32,
        blocked:    bool,
        shot_width: u16,
        height:     u16,
    }

    fn shot_column_fixtures() -> [ShotColumnFixture; 7] {
        [
            (
                ProcessTree::Short,
                &["clippy", "--locked"][..],
                860_000,
                true,
                64,
                50,
            ),
            (
                ProcessTree::Long,
                &["clippy", "--locked"][..],
                870_000,
                true,
                64,
                50,
            ),
            (
                ProcessTree::Short,
                &["install", "cargo-nextest", "--locked"][..],
                3_910_000,
                true,
                54,
                50,
            ),
            (
                ProcessTree::Short,
                &["clippy", "--locked"][..],
                4_180_000,
                true,
                48,
                120,
            ),
            (
                ProcessTree::Long,
                &["install", "cargo-nextest-with-extra-components", "--locked"][..],
                3_920_000,
                false,
                64,
                50,
            ),
            (
                ProcessTree::Long,
                &[
                    "clippy",
                    "--locked",
                    "--config",
                    "build.target-dir=/tmp/long-argument-tail",
                ][..],
                3_930_000,
                true,
                64,
                50,
            ),
            (
                ProcessTree::Short,
                &["clippy", "--locked"][..],
                4_190_000,
                false,
                64,
                50,
            ),
        ]
        .map(
            |(tree, arguments, pid, blocked, shot_width, height)| ShotColumnFixture {
                tree,
                arguments,
                pid,
                blocked,
                shot_width,
                height,
            },
        )
    }

    /// The w40 roster includes a blocked command below the visible summary rows.
    fn forty_column_summary_roster() -> Roster {
        let groups = [
            (
                2_098_105,
                "/tmp/tmpde6i0dyv (deleted)",
                vec!["nextest", "run", "--locked", "--offline"],
                false,
            ),
            (2_504_044, "~", vec!["handler"], false),
            (
                2_840_216,
                "/scratchpad/ci_fix",
                vec!["nextest", "run", "--locked"],
                true,
            ),
            (
                3_178_227,
                "~/rust/hana_catalyst-cables",
                vec!["nextest", "run", "--locked"],
                false,
            ),
            (
                3_215_108,
                "~/rust/obsidian_knife",
                vec!["nextest", "run", "--locked", "--offline"],
                true,
            ),
            (
                3_215_167,
                "~/rust/obsidian_knife",
                vec!["check", "--locked", "--offline"],
                false,
            ),
            (
                3_215_208,
                "~/rust/obsidian_knife",
                vec!["build", "--profile", "p20w"],
                false,
            ),
            (
                3_215_229,
                "~/rust/obsidian_knife",
                vec!["build", "--release"],
                false,
            ),
            (
                3_281_964,
                "~/rust/cargo-liner-berth-parallel",
                vec!["check", "--locked"],
                false,
            ),
        ]
        .into_iter()
        .map(|(pid, path, arguments, child)| {
            let mut lead = invocation(pid, &arguments);
            lead.path = path.to_string();
            lead.directory_identity = WorkingDirectoryIdentity::Absolute(path.into());
            if pid == 2_840_216 || pid == 3_178_227 {
                lead.state = CaptureLookup::Registered(CaptureRead::Progress(compiling(996, 1000)));
            }
            if pid == 3_215_167 {
                lead.state = CaptureLookup::Registered(CaptureRead::Progress(RunState::Blocked));
            }
            if pid == 3_178_227 {
                lead.resource_use.compiler = CompilerObservation::Running(Compiler {
                    name:  "sccache",
                    count: 1,
                });
                lead.managed = Measurement::Reading(1);
            }
            if child {
                lead.managed = Measurement::Reading(if pid == 2_840_216 { 2 } else { 1 });
            }
            let rest = if child {
                let mut child = invocation(pid + 100, &["test", "--no-run"]);
                child.path = lead.path.clone();
                child.directory_identity = lead.directory_identity.clone();
                vec![child]
            } else {
                Vec::new()
            };
            CargoGroup {
                lead,
                rest,
                ancestry: AncestryWalk::Ended(vec![ancestor(pid - 1, "python3")]),
            }
        })
        .collect();
        let mut roster = Roster::for_test(SessionColumnWidths::for_machine(
            32,
            128 * BYTES_PER_GIBIBYTE,
        ));
        roster.observe(groups, Instant::now());
        roster
    }

    #[test]
    fn a_forty_column_summary_uses_its_rows_for_the_next_directory() {
        let roster = forty_column_summary_roster();
        let area = Rect::new(0, 0, 40, 49);
        let sccache = SccacheStats::new();
        let hidden = hidden_when_idle();
        let cells = Cells::new(&roster, &hidden, ProcessTree::Short, &sccache);
        let (grid, buffer) = settled_buffer_with_cells(&cells, area);
        let placements = grid.placements(
            area,
            TileGrowth {
                initial_rows:  12,
                fill:          TileFill::Redistribute,
                widen_summary: false,
            },
        );
        let summary = placements
            .iter()
            .find(|p| p.content == TileContent::Summary)
            .expect("summary");
        let text = placement_text(&buffer, summary);
        assert!(text.contains("2098105"), "{text}");
        assert!(text.contains("2504044"), "{text}");
        assert!(text.contains("cargo handler"), "{text}");
        assert!(!has_blank_row_above_foot(&buffer, summary), "{text}");
        for height in 3..=8 {
            let content = Rect::new(0, 0, 38, height);
            cells.prepare_frame(|| vec![(TileContent::Summary, content)]);
            let reported = cells.rows_drawn(&TileContent::Summary, content);
            let mut buffer = Buffer::empty(content);
            cells.draw(&mut buffer, &TileContent::Summary, content, Color::Reset);
            let painted = (0..height)
                .rev()
                .find(|&y| !buffer_line(&buffer, y).is_empty())
                .map_or(0, |y| y + 1);
            assert_eq!(
                reported,
                painted,
                "height={height}: {}",
                buffer_rows(&buffer).join("\n")
            );
        }
    }

    #[test]
    fn a_long_cut_drops_the_space_before_its_mark() {
        let mut row = same_second("/work/ci", 1_425_874);
        row.process.command =
            CommandText::of("cargo", &["nextest", "run", "--locked", "--offline"]);
        let rows = [&row];
        let area = Rect::new(0, 0, 62, 3);
        let grid = FrameTableColumnGrid::of(&rows, TableKind::Command, ProcessTree::Long);
        let layout =
            CellTableLayout::for_table(&grid, area, Color::Reset, &rows, PinnedGroup::Unpinned);
        let mut line = Line::from("cargo nextest run --locked --offline");
        mark_cut_line(&mut line, 28);
        let text: String = line
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect();
        assert_eq!(text, "cargo nextest run --locked…");
        let mut buffer = Buffer::empty(area);
        frame_draw_process_table(
            &mut buffer,
            area,
            &rows,
            Color::Reset,
            PinnedGroup::Unpinned,
            &grid,
        );
        let command = layout.command_area(area);
        let rendered = table_column_text(&buffer, &layout, COMMAND_COLUMN, 2);
        assert!(command.width > 0);
        if rendered.ends_with(ELISION) {
            assert!(
                !rendered
                    .trim_end_matches(ELISION)
                    .ends_with(char::is_whitespace),
                "{rendered}"
            );
        }
    }

    #[test]
    fn a_long_ci_cell_draws_its_dispatched_nextests_runs() {
        let shim = [
            "/bin/sh",
            "/tools/bin/cargo",
            "nextest",
            "run",
            "--profile",
            "ci",
            "--all-features",
            "--workspace",
            "--tests",
        ]
        .map(OsString::from);
        let nextest = [
            "cargo-nextest",
            "nextest",
            "run",
            "--profile",
            "ci",
            "--all-features",
            "--workspace",
            "--tests",
        ]
        .map(OsString::from);
        let records = ProcessObservations::new(
            [
                (172_043, 170_718, "sh", shim.as_slice()),
                (172_044, 172_043, "sh", shim.as_slice()),
                (172_047, 172_044, "cargo-nextest", nextest.as_slice()),
            ]
            .map(|(pid, parent, name, argv)| {
                let mut process = ProcessObservation::cargo(pid, argv);
                process.name = ProcessField::Observed(OsStr::new(name));
                process.parent = ProcessField::Observed(Pid::from_u32(parent));
                process
            }),
        );
        let groups = CensusSequence::default().sample(&records);
        let mut roster = Roster::new();
        roster.observe(groups, Instant::now());
        let stats = SccacheStats::default();
        let cells = Cells::for_frame(&roster, &[], ProcessTree::Long, &stats, 50);
        let area = Rect::new(0, 0, 200, 50);
        let (grid, buffer) = settled_buffer_with_cells(&cells, area);
        let placement = grid
            .placements(
                area,
                TileGrowth {
                    initial_rows:  12,
                    fill:          TileFill::Redistribute,
                    widen_summary: false,
                },
            )
            .into_iter()
            .find(|placement| matches!(placement.content, TileContent::Group(_)))
            .expect("CI cell");
        let text = placement_text(&buffer, &placement);
        let header = text
            .lines()
            .find(|line| line.contains("command"))
            .expect("table header");
        let runs = header.find("runs").expect("runs header");
        let row = text
            .lines()
            .find(|line| line.split_whitespace().next() == Some("172047"))
            .expect("dispatched process row");
        assert!(
            row.contains("[ci] cargo nextest run --profile ci --all-features --workspace --tests"),
            "{text}"
        );
        assert_eq!(row[runs..].trim(), "2", "{text}");
        assert!(
            !text.contains("172043") && !text.contains("172044"),
            "{text}"
        );
    }

    #[test]
    fn progress_in_the_summary_does_not_tighten_the_cells_gap() {
        let mut app = subcommand_frame(ProcessTree::Short, &["clippy", "--locked"], 2_593_000);
        let groups = app
            .roster
            .groups()
            .iter()
            .map(|group| {
                let mut lead = group.lead.process.clone();
                lead.state = CaptureLookup::Registered(CaptureRead::Progress(compiling(999, 1000)));
                let mut scan = cargo_group(lead);
                scan.rest = group
                    .rows()
                    .skip(1)
                    .map(|row| row.process.clone())
                    .collect();
                scan
            })
            .collect();
        app.roster.observe(groups, Instant::now());
        let stats = SccacheStats::default();
        let cells = Cells::for_frame(&app.roster, &[], ProcessTree::Short, &stats, 120);
        let area = Rect::new(0, 0, 46, 30);
        let mut allocations = vec![(TileContent::Summary, area)];
        allocations.extend(
            app.roster
                .groups()
                .iter()
                .map(|group| (TileContent::Group(group.id.clone()), Rect::new(0, 0, 46, 8))),
        );
        cells.prepare_frame(|| allocations.clone());
        for (content, area) in allocations {
            let mut buffer = Buffer::empty(area);
            cells.draw(&mut buffer, &content, area, Color::Reset);
            let lines = buffer_rows(&buffer);
            match content {
                TileContent::Summary => assert!(
                    lines.iter().any(|line| line.contains("99.9%")),
                    "{lines:#?}"
                ),
                TileContent::Group(_) => {
                    let header = lines
                        .iter()
                        .find(|line| line.contains("parent"))
                        .expect("parent header");
                    assert_eq!(header.find("command"), Some(19), "{header}");
                },
                TileContent::Empty(_) => {},
            }
            let (grid, expected_gap) = match content {
                TileContent::Summary => (&cells.grids.summary.0, 1),
                TileContent::Group(_) => (&cells.grids.commands.0, 2),
                TileContent::Empty(_) => continue,
            };
            assert_eq!(
                CellTableLayout::from_grid(grid, area, Color::Reset).column_spacing,
                expected_gap,
                "{lines:#?}"
            );
        }
    }

    #[test]
    fn long_cuts_join_the_mark_in_state_and_height_cut_frames() {
        let mut failures = Vec::new();
        for (blocked, width, height) in [(true, 62, 3), (false, 88, 4)] {
            let mut first = same_second("/work/ci", 2_209_338);
            first.process.command = CommandText::of(
                "cargo",
                &[
                    "clippy",
                    "--workspace",
                    "--all-targets",
                    "--all-features",
                    "--locked",
                    "--offline",
                    "--config",
                    "build.target-dir=/tmp/very-long-command-tail/with-more-components",
                ],
            );
            first.process.parent = VisibleParent::Ancestor(2_209_148);
            if !blocked {
                first.process.state =
                    CaptureLookup::Registered(CaptureRead::Progress(compiling(999, 1000)));
                first.process.resource_use.compiler = CompilerObservation::Running(Compiler {
                    name:  "sccache",
                    count: 1,
                });
                first.process.managed = Measurement::Reading(1);
            }
            let mut neighbour = same_second("/work/other", 3_043_326);
            if blocked {
                neighbour.process.state =
                    CaptureLookup::Registered(CaptureRead::Progress(RunState::Blocked));
            }
            let rows = [&first, &neighbour];
            let mut grid = FrameTableColumnGrid::of(&rows, TableKind::Command, ProcessTree::Long);
            if !blocked {
                // Another table's reading reserves this alignment slot in longcut.
                let mut slots = grid.slots.clone();
                slots.push(STATE_COLUMN);
                slots.sort_unstable();
                grid.set_frame_slots(slots);
            }
            let area = Rect::new(0, 0, width, height);
            let layout = CellTableLayout::for_table(
                &grid,
                area,
                Color::Reset,
                &[&first],
                PinnedGroup::Unpinned,
            );
            let command = layout.command_area(area);
            let mut buffer = Buffer::empty(area);
            frame_draw_process_table(
                &mut buffer,
                area,
                &[&first],
                Color::Reset,
                PinnedGroup::Unpinned,
                &grid,
            );
            let line = buffer_line(&buffer, height - 1);
            let edge = command.right() - 1;
            if buffer[(edge, height - 1)].symbol() != ELISION
                || buffer[(edge - 1, height - 1)].symbol() == " "
                || line.matches(ELISION).count() != 1
            {
                failures.push(format!("blocked={blocked}: {line}"));
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[test]
    fn a_cut_off_cell_keeps_its_release_subcommand_at_sixty_eight_columns() {
        let mut roster =
            Roster::for_test(SessionColumnWidths::for_machine(8, 8 * BYTES_PER_GIBIBYTE));
        // cutfoot has 68 content cells inside its two border cells.
        let groups = [
            (
                2_098_105,
                2_098_060,
                vec!["nextest", "run", "--test-threads", "32", "--no-fail-fast"],
            ),
            (
                2_751_807,
                2_751_058,
                vec!["nextest", "run", "--test-threads", "5", "--no-fail-fast"],
            ),
            (3_647_638, 3_647_533, vec!["handler-p4"]),
            (3_749_738, 3_749_198, vec!["build", "--release", "--locked"]),
        ]
        .into_iter()
        .map(|(pid, parent, arguments)| {
            let mut lead = invocation(pid, &arguments);
            lead.parent = VisibleParent::Ancestor(parent);
            lead.resource_use.cpu = Measurement::Reading(200.0);
            lead.resource_use.memory = Measurement::Reading(0);
            lead.duration = "03:46:52".to_string();
            lead.path = format!("~/work/{pid}");
            lead.directory_identity = WorkingDirectoryIdentity::Absolute(lead.path.clone().into());
            let mut group = cargo_group(lead);
            group.rest = (1..10)
                .map(|offset| {
                    let mut child = invocation(
                        pid + offset,
                        &["check", "--locked", "--workspace", "--all-targets"],
                    );
                    child.path = group.lead.path.clone();
                    child.directory_identity = group.lead.directory_identity.clone();
                    child
                })
                .collect();
            group
        })
        .collect();
        roster.observe(groups, Instant::now());
        let stats = SccacheStats::default();
        let cells = Cells::for_frame(&roster, &[], ProcessTree::Long, &stats, 32);
        let allocations: Vec<_> = roster
            .groups()
            .iter()
            .map(|group| (TileContent::Group(group.id.clone()), Rect::new(0, 0, 68, 3)))
            .collect();
        cells.prepare_frame(|| allocations.clone());
        let (content, area) = allocations.last().expect("release cell");
        let mut buffer = Buffer::empty(*area);
        cells.draw(&mut buffer, content, *area, Color::Reset);
        let text = buffer_rows(&buffer).join("\n");
        assert!(text.contains("[release] cargo build"), "{text}");
        assert!(
            text.contains("start") && text.contains("dur") && text.contains("mem"),
            "{text}"
        );
        assert!(text.contains("pid     parent"), "{text}");
    }

    #[test]
    fn blocked_frames_keep_ordinary_cell_gaps_at_two_hundred_columns() {
        let app = subcommand_frame(ProcessTree::Short, &["clippy", "--locked"], 4_180_000);
        let stats = SccacheStats::default();
        let cells = Cells::for_frame(&app.roster, &[], ProcessTree::Short, &stats, 120);
        let area = Rect::new(0, 0, 198, 8);
        let mut allocations = vec![(TileContent::Summary, area)];
        allocations.extend(
            app.roster
                .groups()
                .iter()
                .map(|group| (TileContent::Group(group.id.clone()), area)),
        );
        cells.prepare_frame(|| allocations.clone());
        let mut saw_blocked = false;
        for (content, area) in allocations {
            let TileContent::Group(_) = content else {
                continue;
            };
            let mut buffer = Buffer::empty(area);
            cells.draw(&mut buffer, &content, area, Color::Reset);
            let lines = buffer_rows(&buffer);
            saw_blocked |= lines.iter().any(|line| line.contains("blocked"));
            let header = lines
                .iter()
                .find(|line| line.contains("parent"))
                .expect("cell header");
            assert!(header.starts_with(" pid      parent"), "{header}");
            assert_eq!(
                CellTableLayout::from_grid(&cells.grids.commands.0, area, Color::Reset)
                    .column_spacing,
                2,
                "{lines:#?}"
            );
        }
        assert!(saw_blocked);
    }

    #[test]
    fn a_long_cut_command_reaches_its_column_edge_with_one_mark() {
        for arguments in [
            &["build", "--release", "--locked", "--offline"][..],
            &["nextest", "run", "--locked", "--offline"][..],
            &["mend", "--all-targets", "--workspace", "--fix"][..],
        ] {
            let mut first = same_second("/work", 1_958_592);
            first.process.command = CommandText::of("cargo", arguments);
            first.process.parent = VisibleParent::Ancestor(1_958_095);
            let mut child = same_second("/work", 1_958_609);
            child.process.parent = VisibleParent::Invocation {
                id:  first.process.invocation_id.clone(),
                pid: first.process.pid,
            };
            let rows = [&first, &child];
            let grid = FrameTableColumnGrid::of(&rows, TableKind::Command, ProcessTree::Long);
            grid.row_end.set(TableRowEnd::Reserved);
            let area = Rect::new(0, 0, 62, 3);
            let mut buffer = Buffer::empty(area);
            frame_draw_process_table(
                &mut buffer,
                area,
                &rows,
                Color::Reset,
                PinnedGroup::Unpinned,
                &grid,
            );
            let line = buffer_line(&buffer, 2);
            assert_eq!(line.matches(ELISION).count(), 1, "{line}");
            let edge = usize::from(area.width - CONTENT_RIGHT_INSET - 1);
            assert_eq!(line.chars().nth(edge), Some('…'), "{line}");
        }
    }

    #[test]
    fn a_forty_column_summary_wraps_the_command_beside_its_progress() {
        let mut row = same_second("~/rust/obsidian_knife", 2_373_911);
        row.process.command = CommandText::of("cargo", &["build", "--release", "--locked"]);
        row.process.state = CaptureLookup::Registered(CaptureRead::Progress(compiling(98, 100)));
        let rows = [&row];
        let grid = FrameTableColumnGrid::of(&rows, TableKind::Summary, ProcessTree::Short);
        for width in [38, 46] {
            let area = Rect::new(0, 0, width, 8);
            let mut buffer = Buffer::empty(area);
            frame_draw_process_table(
                &mut buffer,
                area,
                &rows,
                Color::Reset,
                PinnedGroup::Unpinned,
                &grid,
            );
            let lines = buffer_rows(&buffer);
            assert!(lines[0].contains("state"), "{width}: {lines:#?}");
            assert!(
                lines[2].contains("2373911") && lines[2].contains("98%"),
                "{width}: {lines:#?}"
            );
            if width == 38 {
                assert!(lines[2].contains("[release] cargo"), "{lines:#?}");
                assert!(lines[3].trim().starts_with("build"), "{lines:#?}");
            }
        }
    }

    #[test]
    fn a_summary_keeps_progress_when_no_command_word_fits() {
        let mut row = same_second("~/project", 2_373_911);
        row.process.command = CommandText::of("cargo", &["build", "--release"]);
        row.process.state = CaptureLookup::Registered(CaptureRead::Progress(compiling(98, 100)));
        for tree in [ProcessTree::Short, ProcessTree::Long] {
            let grid = FrameTableColumnGrid::of(&[&row], TableKind::Summary, tree);
            let area = Rect::new(0, 0, 18, 5);
            let mut buffer = Buffer::empty(area);
            frame_draw_process_table(
                &mut buffer,
                area,
                &[&row],
                Color::Reset,
                PinnedGroup::Unpinned,
                &grid,
            );
            let lines = buffer_rows(&buffer);
            assert_eq!(
                lines[0].split_whitespace().collect::<Vec<_>>(),
                ["pid", "state"]
            );
            assert!(
                lines[2].contains("2373911") && lines[2].contains("98%"),
                "{lines:#?}"
            );
        }
    }

    fn assert_progress_summary(failures: &mut Vec<String>) {
        let mut app = hidden_state_frame(ProcessTree::Short);
        app.loaded_config.config.tiles.view = TileView::Summary;
        let area = Rect::new(0, 0, 48, 120);
        let mut terminal = Terminal::new(TestBackend::new(48, 120)).unwrap();
        terminal
            .draw(|frame| {
                draw_panes(frame, &mut app, area, TileGridContents::Shown);
            })
            .unwrap();
        app.tiles.settle_for_test();
        terminal
            .draw(|frame| {
                draw_panes(frame, &mut app, area, TileGridContents::Shown);
            })
            .unwrap();
        let text = buffer_rows(terminal.backend().buffer()).join("\n");
        if !text.contains("50%") || !text.contains("blocked") {
            failures.push(format!("48-column summary loses progress: {text}"));
        }
    }

    fn assert_shot_column_fixture(fixture: ShotColumnFixture) -> Vec<String> {
        let ShotColumnFixture {
            tree,
            arguments,
            pid,
            blocked,
            shot_width,
            height,
        } = fixture;
        let mut failures = Vec::new();
        let mut app = subcommand_frame(tree, arguments, pid);
        let groups = app
            .roster
            .groups()
            .iter()
            .map(|group| {
                let mut rows = group.rows().take(4).map(|row| row.process.clone());
                CargoGroup {
                    lead:     rows.next().expect("lead"),
                    rest:     rows.collect(),
                    ancestry: AncestryWalk::Ended(Vec::new()),
                }
            })
            .collect();
        app.roster.observe(groups, Instant::now());
        if !blocked {
            let groups = app
                .roster
                .groups()
                .iter()
                .map(|group| {
                    let mut rows = group.rows().map(|row| {
                        let mut process = row.process.clone();
                        process.state = CaptureLookup::Unregistered;
                        process
                    });
                    CargoGroup {
                        lead:     rows.next().expect("lead"),
                        rest:     rows.collect(),
                        ancestry: AncestryWalk::Ended(Vec::new()),
                    }
                })
                .collect();
            app.roster.observe(groups, Instant::now());
        }
        {
            let stats = SccacheStats::default();
            let cells = Cells::for_frame(&app.roster, &[], tree, &stats, height);
            cells.grids.commands.0.row_end.set(TableRowEnd::Reserved);
            cells.grids.summary.0.row_end.set(TableRowEnd::Reserved);
            assert_main_state_columns(&cells, shot_width, &mut failures);
            assert_frame_column_priority(&cells.grids, tree, pid, &mut failures);
            assert_shot_frame_columns(&cells, tree, arguments, pid, shot_width);
        }
        if shot_width == 48 {
            let texts = state_frame_text_when_settled(&mut app, shot_width, height);
            assert_eq!(texts.len(), 8);
            for text in texts {
                let header = text.lines().next().expect("header");
                assert!(
                    header.contains("parent") && !header.contains("state"),
                    "{text}"
                );
            }
        }
        failures
    }

    fn assert_duration_and_long_tail_columns(failures: &mut Vec<String>) {
        for tree in [ProcessTree::Short, ProcessTree::Long] {
            let mut first = same_second("/work/release", 1_012_329);
            first.process.parent = VisibleParent::Ancestor(1_012_300);
            first.process.resource_use.cpu = Measurement::Reading(6400.0);
            first.process.command = CommandText::of("cargo", &["build", "--release", "--locked"]);
            first.process.resource_use.memory = Measurement::Reading(BYTES_PER_GIBIBYTE);
            let rows = [&first];
            let grid = FrameTableColumnGrid::of(&rows, TableKind::Command, tree);
            grid.row_end.set(TableRowEnd::Reserved);
            let area = Rect::new(0, 0, 62, 4);
            let mut buffer = Buffer::empty(area);
            frame_draw_process_table(
                &mut buffer,
                area,
                &rows,
                Color::Reset,
                PinnedGroup::Unpinned,
                &grid,
            );
            let header = buffer_line(&buffer, 0);
            if !header.contains("dur") {
                failures.push(format!("64-column {tree:?} cell loses duration: {header}"));
            }
        }
        let mut lead = same_second("/work/ci", 895_427);
        lead.process.command = CommandText::of(
            "cargo",
            &[
                "nextest",
                "run",
                "--profile",
                "ci",
                "--workspace",
                "--all-features",
                "--no-fail-fast",
                "--test-threads",
                "12",
            ],
        );
        lead.process.resource_use.compiler = CompilerObservation::Running(Compiler {
            name:  "rustc",
            count: 1,
        });
        lead.process.managed = Measurement::Reading(5);
        lead.process.state = CaptureLookup::Registered(CaptureRead::Progress(RunState::Blocked));
        let mut child = same_second("/work/ci", 1_012_329);
        child.process.command = CommandText::of("cargo", &["check", "--locked"]);
        lead.process.parent = VisibleParent::Ancestor(894_913);
        lead.process.resource_use.memory = Measurement::Reading(BYTES_PER_GIBIBYTE);
        lead.process.resource_use.cpu = Measurement::Reading(6400.0);
        child.process.parent = VisibleParent::Ancestor(1_012_300);
        child.process.resource_use.compiler = CompilerObservation::Running(Compiler {
            name:  "sccache",
            count: 1,
        });
        let rows = [&lead, &child];
        let grid = FrameTableColumnGrid::of(&rows, TableKind::Command, ProcessTree::Long);
        grid.row_end.set(TableRowEnd::Reserved);
        let area = Rect::new(0, 0, 88, 4);
        let mut buffer = Buffer::empty(area);
        frame_draw_process_table(
            &mut buffer,
            area,
            &rows,
            Color::Reset,
            PinnedGroup::Unpinned,
            &grid,
        );
        let text = buffer_rows(&buffer).join("\n");
        let header = buffer_line(&buffer, 0);
        let first_row = buffer_line(&buffer, 2);
        if !header.contains("runs")
            || !header.contains("compiler")
            || !first_row.contains("rustc×1")
            || first_row.split_whitespace().last() != Some("5")
        {
            failures.push(format!("Long argument tails displace counts: {text}"));
        }
    }

    #[test]
    fn shot_frames_keep_columns_in_the_same_order_at_every_width() {
        let mut failures: Vec<_> = std::thread::scope(|scope| {
            let runs = shot_column_fixtures()
                .map(|fixture| scope.spawn(move || assert_shot_column_fixture(fixture)));
            runs.into_iter()
                .flat_map(|run| run.join().expect("shot fixture"))
                .collect()
        });
        assert_progress_summary(&mut failures);
        assert_duration_and_long_tail_columns(&mut failures);
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[test]
    fn blocked_clippy_keeps_its_subcommand_before_start_and_duration() {
        let mut app = subcommand_frame(
            ProcessTree::Short,
            &["clippy", "--locked", "--offline", "-j", "1"],
            860_000,
        );
        let texts = state_frame_text_when_settled(&mut app, 64, 50);
        let blocked = texts
            .iter()
            .find(|text| text.contains("860100"))
            .expect("blocked");
        assert!(blocked.contains("cargo clippy"), "{blocked}");
        let release = texts
            .iter()
            .find(|text| text.contains("860200"))
            .expect("release");
        assert!(release.contains("[release] cargo build"), "{release}");
    }

    #[test]
    fn long_blocked_clippy_keeps_its_subcommand_before_optional_columns() {
        let mut app = subcommand_frame(
            ProcessTree::Long,
            &["clippy", "--locked", "--offline", "-j", "1"],
            870_000,
        );
        let texts = state_frame_text_when_settled(&mut app, 64, 50);
        let blocked = texts
            .iter()
            .find(|text| text.contains("870100"))
            .expect("blocked");
        assert!(blocked.contains("cargo clippy"), "{blocked}");
        let release = texts
            .iter()
            .find(|text| text.contains("870200"))
            .expect("release");
        assert!(release.contains("[release] cargo build"), "{release}");
    }

    #[test]
    fn seven_digit_pids_leave_room_for_install_before_cpu_and_memory() {
        let mut app = subcommand_frame(
            ProcessTree::Short,
            &["install", "cargo-nextest", "--locked"],
            3_910_000,
        );
        let texts = state_frame_text_when_settled(&mut app, 54, 50);
        let blocked = texts
            .iter()
            .find(|text| text.contains("3910100"))
            .expect("blocked");
        assert!(blocked.contains("cargo install"), "{blocked}");
        let release = texts
            .iter()
            .find(|text| text.contains("3910200"))
            .expect("release");
        assert!(release.contains("[release] cargo build"), "{release}");
    }

    #[test]
    fn a_narrow_release_row_keeps_build_before_optional_columns() {
        for width in [40, 48] {
            let mut app = subcommand_frame(ProcessTree::Short, &["clippy", "--locked"], 4_180_000);
            let texts = state_frame_text_when_settled(&mut app, width, 50);
            let release = texts
                .iter()
                .find(|text| text.contains("4180200"))
                .expect("release");
            assert!(
                release.contains("[release] cargo build"),
                "width {width}: {release}"
            );
        }
    }

    #[test]
    fn a_narrow_blocked_frame_keeps_parent_before_the_state_slot() {
        let mut app = subcommand_frame(ProcessTree::Short, &["clippy", "--locked"], 3_910_000);
        let texts = state_frame_text_when_settled(&mut app, 48, 120);
        assert_eq!(texts.len(), 8);
        for text in &texts {
            let header = text
                .lines()
                .find(|line| line.contains("command"))
                .expect(text);
            assert!(header.contains("parent"), "{text}");
            assert!(!header.contains("state"), "{text}");
        }
        let children = texts
            .iter()
            .find(|text| text.contains("3910001"))
            .expect("children");
        let header = children
            .lines()
            .find(|line| line.contains("parent"))
            .expect(children);
        let parent_at = header.find("parent").expect(children);
        let child = children
            .lines()
            .find(|line| line.contains("3910001"))
            .expect(children);
        assert!(child[parent_at..].starts_with("3910000"), "{children}");
    }

    fn assert_long_frame_keeps_memory(blocked: bool) {
        let mut app = subcommand_frame(
            ProcessTree::Long,
            &[
                "clippy",
                "--locked",
                "--offline",
                "--config",
                "build.target-dir=/tmp/long-argument-tail",
            ],
            3_920_000,
        );
        if !blocked {
            let groups = app
                .roster
                .groups()
                .iter()
                .map(|group| {
                    let mut processes = group.rows().map(|row| {
                        let mut process = row.process.clone();
                        process.state = CaptureLookup::Unregistered;
                        if process.pid == 3_920_100 {
                            process.command = CommandText::of(
                                "cargo",
                                &[
                                    "install",
                                    "cargo-nextest-with-extra-components",
                                    "--locked",
                                    "--offline",
                                ],
                            );
                        }
                        process
                    });
                    CargoGroup {
                        lead:     processes.next().expect("lead"),
                        rest:     processes.collect(),
                        ancestry: AncestryWalk::Ended(Vec::new()),
                    }
                })
                .collect();
            app.roster.observe(groups, Instant::now());
        }
        let texts = state_frame_text_when_settled(&mut app, 64, 50);
        assert_eq!(texts.len(), 8);
        for text in &texts {
            let header = text
                .lines()
                .find(|line| line.contains("command"))
                .expect(text);
            for title in ["parent", "cpu", "mem"] {
                assert!(header.contains(title), "{text}");
            }
            let name = if text.contains("3920200") {
                "[release] cargo build"
            } else if !blocked && text.contains("3920100") {
                "cargo install"
            } else {
                "cargo clippy"
            };
            assert!(text.contains(name), "{text}");
        }
        assert!(texts.iter().any(|text| text.contains('…')), "{texts:#?}");
    }

    #[test]
    fn long_argument_tails_yield_before_memory_in_a_stateless_frame() {
        assert_long_frame_keeps_memory(false);
    }

    #[test]
    fn long_argument_tails_yield_before_memory_in_a_blocked_frame() {
        assert_long_frame_keeps_memory(true);
    }

    #[test]
    fn runs_titles_align_when_only_one_table_has_a_state_value() {
        let mut app = subcommand_frame(
            ProcessTree::Short,
            &["clippy", "--locked", "--offline", "-j", "1"],
            970_000,
        );
        let texts = state_frame_text_when_settled(&mut app, 200, 50);
        assert_eq!(texts.len(), 8);
        let offsets: Vec<_> = texts
            .iter()
            .map(|text| {
                let header = text
                    .lines()
                    .find(|line| line.contains("command"))
                    .expect(text);
                ["cpu", "mem", "runs"].map(|title| header.find(title).expect(text))
            })
            .collect();
        assert!(
            offsets.iter().all(|offset| *offset == offsets[0]),
            "{texts:#?}"
        );
    }

    #[derive(Eq, PartialEq)]
    pub(super) struct TableLayoutQuery {
        invocations: Vec<InvocationId>,
        width:       u16,
    }

    thread_local! {
        pub(super) static ROW_USE_LAYOUTS: RefCell<Vec<TableLayoutQuery>> = const { RefCell::new(Vec::new()) };
        pub(super) static LIMIT_ROW_USE_LAYOUTS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }

    pub(super) fn record_row_use_layout(rows: &[&TrackedRow], width: u16) {
        LIMIT_ROW_USE_LAYOUTS.with(|limit| {
            if limit.get() {
                ROW_USE_LAYOUTS.with(|layouts| {
                    let mut layouts = layouts.borrow_mut();
                    let key = TableLayoutQuery {
                        invocations: rows
                            .iter()
                            .map(|row| row.process.invocation_id.clone())
                            .collect(),
                        width,
                    };
                    if u32::try_from(rows.len()) == Ok(LARGE_TABLE_TEST_ROWS) {
                        assert!(
                            !layouts.contains(&key),
                            "repeated table layout for {} rows at width {width}",
                            rows.len()
                        );
                    }
                    layouts.push(key);
                });
            }
        });
    }

    #[test]
    fn a_six_hundred_row_cell_prepares_each_table_width_once() {
        let mut roster = Roster::default();
        let mut group = cargo_group(invocation(
            LARGE_TABLE_TEST_FIRST_PID,
            &LARGE_TABLE_TEST_COMMAND,
        ));
        group.rest = (1..LARGE_TABLE_TEST_ROWS)
            .map(|index| {
                invocation(
                    LARGE_TABLE_TEST_FIRST_PID + index,
                    &LARGE_TABLE_TEST_COMMAND,
                )
            })
            .collect();
        roster.observe(vec![group], Instant::now());
        let stats = SccacheStats::default();
        let area = LARGE_TABLE_TEST_AREA;
        let cells = Cells::for_frame(
            &roster,
            &[],
            ProcessTree::Short,
            &stats,
            area.height.saturating_sub(3),
        );
        let mut grid = tui_pane::TileGrid::new();
        grid.set_view(TileView::Cells);
        let growth = tui_pane::TileGrowth::default();
        grid.set_layout(area, growth);
        let widths = [
            (TileContent::Summary, area.width - 2),
            (
                TileContent::Group(roster.groups()[0].id.clone()),
                area.width - 2,
            ),
        ];
        grid.sync(&cells.demands(&widths), growth);
        grid.settle_for_test();
        let mut buffer = Buffer::empty(area);
        ROW_USE_LAYOUTS.with(|layouts| layouts.borrow_mut().clear());
        LIMIT_ROW_USE_LAYOUTS.with(|limit| limit.set(true));
        tui_pane::draw_tile_grid(
            &mut buffer,
            &mut grid,
            area,
            tui_pane::TileGrowth::default(),
            TileGridContents::Shown,
            &cells,
        );
        let content = TileContent::Group(roster.groups()[0].id.clone());
        for height in 0..=area.height {
            let probe = Rect::new(0, 0, area.width - 2, height);
            cells.rows_drawn(&content, probe);
            cells.rows_kept(&content, probe);
        }
        LIMIT_ROW_USE_LAYOUTS.with(|limit| limit.set(false));
        ROW_USE_LAYOUTS.with(|layouts| {
            assert!(
                layouts
                    .borrow()
                    .iter()
                    .any(|layout| u32::try_from(layout.invocations.len())
                        == Ok(LARGE_TABLE_TEST_ROWS))
            );
        });
        assert!(
            buffer_rows(&buffer)
                .join("")
                .contains(&(LARGE_TABLE_TEST_FIRST_PID + 1).to_string())
        );
    }

    #[test]
    fn a_zero_width_table_skips_process_layout() {
        let mut roster = Roster::default();
        roster.observe(
            vec![cargo_group(invocation(
                LARGE_TABLE_TEST_FIRST_PID,
                &LARGE_TABLE_TEST_COMMAND,
            ))],
            Instant::now(),
        );
        let stats = SccacheStats::default();
        let cells = Cells::new(&roster, &[], ProcessTree::Short, &stats);
        let content = TileContent::Group(roster.groups()[0].id.clone());
        ROW_USE_LAYOUTS.with(|layouts| layouts.borrow_mut().clear());
        LIMIT_ROW_USE_LAYOUTS.with(|limit| limit.set(true));
        assert_eq!(
            cells.rows_drawn(&content, Rect::new(0, 0, 0, LARGE_TABLE_TEST_AREA.height)),
            TABLE_HEADER_HEIGHT
        );
        LIMIT_ROW_USE_LAYOUTS.with(|limit| limit.set(false));
        ROW_USE_LAYOUTS.with(|layouts| assert!(layouts.borrow().is_empty()));
    }

    fn state_frame_text(app: &mut App, width: u16, height: u16) -> Vec<String> {
        let area = Rect::new(0, 0, width, height);
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
        for _ in 0..12 {
            app.tiles.settle_for_test();
            terminal
                .draw(|frame| {
                    draw_panes(frame, app, area, TileGridContents::Shown);
                })
                .expect("frame");
        }
        state_frame_cells(app, area, terminal.backend().buffer())
    }

    fn state_frame_text_when_settled(app: &mut App, width: u16, height: u16) -> Vec<String> {
        let area = Rect::new(0, 0, width, height);
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
        let mut previous = Buffer::empty(area);
        for _ in 0..12 {
            app.tiles.settle_for_test();
            terminal
                .draw(|frame| {
                    draw_panes(frame, app, area, TileGridContents::Shown);
                })
                .expect("frame");
            let current = terminal.backend().buffer();
            if !app.tiles.tick() && *current == previous {
                return state_frame_cells(app, area, current);
            }
            previous = current.clone();
        }
        panic!("install frame did not settle");
    }

    fn state_frame_cells(app: &App, area: Rect, buffer: &Buffer) -> Vec<String> {
        app.tiles
            .placements(area, app.loaded_config.config.tiles.growth())
            .iter()
            .filter(|cell| matches!(cell.content, TileContent::Group(_)))
            .map(|cell| placement_text(buffer, cell))
            .collect()
    }

    fn assert_hidden_state_frame_keeps_memory(tree: ProcessTree) {
        let mut app = hidden_state_frame(tree);
        let texts = state_frame_text(&mut app, 66, 50);
        assert_eq!(texts.len(), 8);
        let mut offsets = Vec::new();
        for text in &texts {
            let header = text
                .lines()
                .find(|line| line.contains("command"))
                .expect(text);
            offsets.push(["cpu", "mem"].map(|title| header.find(title).expect(text)));
            let process = text
                .lines()
                .find(|line| line.trim_start().starts_with("316"))
                .expect(text);
            let memory_at = header.find("mem").expect(text);
            let memory = process[memory_at..memory_at + 5].trim();
            assert!(
                memory.ends_with('G')
                    && memory.starts_with(|character: char| character.is_ascii_digit()),
                "{text}"
            );
            assert!(
                !header.contains("start") && !header.contains("dur"),
                "{text}"
            );
            let subcommand = if text.contains("3160000") || text.contains("3160100") {
                "clippy"
            } else if text.contains("3160200") {
                "build"
            } else {
                "check"
            };
            assert!(
                text.contains("cargo") && text.split_whitespace().any(|word| word == subcommand),
                "{text}"
            );
            if tree == ProcessTree::Long {
                let command_at = header.find("command").expect(text);
                for line in text.lines().skip_while(|line| *line != header).skip(2) {
                    if line.contains("content rows:") {
                        break;
                    }
                    let command: String = line.chars().skip(command_at).collect();
                    let cut = command.contains(ELISION);
                    if cut {
                        assert_eq!(command.matches(ELISION).count(), 1, "{text}");
                    }
                    let prefix = command.trim_end().trim_end_matches(ELISION);
                    let words: Vec<_> = prefix.split_whitespace().collect();
                    for (position, word) in words.iter().enumerate() {
                        let arguments = [
                            "cargo",
                            "clippy",
                            "build",
                            "check",
                            "[release]",
                            "--release",
                            "--locked",
                            "--offline",
                            "-j",
                            "1",
                        ];
                        assert!(
                            arguments.contains(word)
                                || (cut
                                    && position + 1 == words.len()
                                    && arguments.iter().any(|argument| argument.starts_with(word))),
                            "split argument {word:?}: {text}"
                        );
                    }
                }
            }
        }
        assert!(
            offsets.iter().all(|offset| *offset == offsets[0]),
            "{texts:#?}"
        );
        let last_process_row = texts[0].lines().rev().nth(1).expect("row above foot");
        assert_eq!(
            last_process_row.chars().rev().nth(1),
            ELISION.chars().next(),
            "{texts:#?}"
        );
        assert!(
            texts.iter().any(|text| text.contains("[release]")),
            "{texts:#?}"
        );
    }

    #[test]
    fn hidden_short_state_tables_keep_memory_before_argument_tails() {
        assert_hidden_state_frame_keeps_memory(ProcessTree::Short);
    }

    #[test]
    fn hidden_long_state_tables_keep_memory_before_argument_tails() {
        assert_hidden_state_frame_keeps_memory(ProcessTree::Long);
    }

    #[test]
    fn a_narrow_frame_keeps_parent_before_its_state_reading() {
        let mut app = hidden_state_frame(ProcessTree::Short);
        let texts = state_frame_text(&mut app, 48, 120);
        let blocked = texts
            .iter()
            .find(|text| text.contains("3160100"))
            .expect("blocked cell");
        assert!(!blocked.contains("blocked"), "{blocked}");
        let blocked_header = blocked
            .lines()
            .find(|line| line.contains("command"))
            .expect(blocked);
        assert!(
            !blocked_header.contains("state") && blocked_header.contains("parent"),
            "{blocked}"
        );
        let mut resource_offsets = Vec::new();
        for text in &texts {
            let header = text
                .lines()
                .find(|line| line.contains("command"))
                .expect(text);
            if !text.contains("3160100") {
                assert!(
                    header.contains("parent") && !header.contains("state"),
                    "{text}"
                );
            }
            assert_eq!(
                header.find("command"),
                blocked_header.find("command"),
                "{text}"
            );
            for title in ["cpu", "mem"] {
                if let Some(offset) = header.find(title) {
                    resource_offsets.push((title, offset));
                }
            }
        }
        for title in ["cpu", "mem"] {
            let offsets: Vec<_> = resource_offsets
                .iter()
                .filter(|(name, _)| *name == title)
                .map(|(_, offset)| *offset)
                .collect();
            assert!(
                offsets.iter().all(|offset| Some(offset) == offsets.first()),
                "{texts:#?}"
            );
        }
    }

    #[test]
    fn a_short_install_tail_never_raises_the_state_frames_command_minimum() {
        let mut app = hidden_state_frame_with_arguments(
            ProcessTree::Short,
            &["install", "cargo-nextest", "--locked"],
        );
        let groups = app
            .roster
            .groups()
            .iter()
            .map(|group| {
                let mut rows = group.rows().map(|row| {
                    let mut process = row.process.clone();
                    process.command =
                        CommandText::of("cargo", &["install", "cargo-nextest", "--locked"]);
                    process
                });
                CargoGroup {
                    lead:     rows.next().expect("lead"),
                    rest:     rows.collect(),
                    ancestry: AncestryWalk::Ended(Vec::new()),
                }
            })
            .collect();
        app.roster.observe(groups, Instant::now());
        for width in 54..=56 {
            let texts = state_frame_text_when_settled(&mut app, width, 50);
            let blocked = texts
                .iter()
                .find(|text| text.contains("3160100"))
                .expect("blocked cell");
            let header = blocked
                .lines()
                .find(|line| line.contains("command"))
                .expect(blocked);
            assert!(header.contains("state"), "width {width}: {blocked}");
            assert!(
                blocked.contains("blocked") && blocked.contains("cargo"),
                "width {width}: {blocked}"
            );
            let command_at = header.find("command").expect(blocked);
            let commands: Vec<_> = blocked
                .lines()
                .skip_while(|line| *line != header)
                .skip(2)
                .take_while(|line| !line.contains("content rows:"))
                .map(|line| line.chars().skip(command_at).collect::<String>())
                .filter(|line| !line.trim().is_empty())
                .collect();
            assert!(
                commands.iter().any(|command| command.contains("install")),
                "width {width}: missing subcommand: {blocked}"
            );
            for command in &commands {
                for word in command.split_whitespace() {
                    assert!(
                        ["cargo", "install", "cargo-nextest", ELISION].contains(&word),
                        "width {width}: split word {word:?}: {blocked}"
                    );
                }
            }
            assert!(
                commands
                    .join(" ")
                    .trim_end()
                    .ends_with(crate::constants::ARGUMENT_OMISSION_MARK),
                "width {width}: missing argument mark: {blocked}"
            );
        }
    }

    #[test]
    fn stateless_tables_keep_memory_beside_a_blocked_install() {
        let mut app = hidden_state_frame_with_arguments(
            ProcessTree::Short,
            &["install", "cargo-nextest", "--locked"],
        );
        for width in [53, 57, 65, 67] {
            let texts = state_frame_text_when_settled(&mut app, width, 50);
            assert_eq!(texts.len(), 8);
            let mut memory_offsets = Vec::new();
            let mut cpu_offsets = Vec::new();
            for text in &texts {
                let header = text
                    .lines()
                    .find(|line| line.contains("command"))
                    .expect(text);
                let memory_at = header.find("mem");
                memory_offsets.push(memory_at);
                let process = text
                    .lines()
                    .find(|line| line.trim_start().starts_with("316"))
                    .expect(text);
                if let Some(memory_at) = memory_at {
                    let memory: String = process.chars().skip(memory_at).take(5).collect();
                    assert!(memory.trim().ends_with('G'), "{text}");
                }
                if width >= 65 {
                    assert!(memory_at.is_some(), "{text}");
                }
                if let Some(offset) = header.find("cpu") {
                    cpu_offsets.push(offset);
                }
                if text.contains("3160100") {
                    assert!(
                        header.contains("state") && text.contains("blocked"),
                        "{text}"
                    );
                } else {
                    assert!(!header.contains("state"), "{text}");
                }
            }
            assert!(
                memory_offsets
                    .iter()
                    .all(|offset| *offset == memory_offsets[0]),
                "{texts:#?}"
            );
            assert!(
                cpu_offsets
                    .iter()
                    .all(|offset| Some(offset) == cpu_offsets.first()),
                "{texts:#?}"
            );
        }
    }

    #[test]
    fn resource_columns_align_beside_a_blocked_test_at_threshold_widths() {
        let mut app = hidden_state_frame_with_arguments(ProcessTree::Short, &["test"]);
        let groups = app
            .roster
            .groups()
            .iter()
            .map(|group| {
                let mut rows = group.rows().map(|row| {
                    let mut process = row.process.clone();
                    process.command = CommandText::of("cargo", &["test"]);
                    process
                });
                CargoGroup {
                    lead:     rows.next().expect("lead"),
                    rest:     rows.collect(),
                    ancestry: AncestryWalk::Ended(Vec::new()),
                }
            })
            .collect();
        app.roster.observe(groups, Instant::now());
        for width in [48, 49, 50, 51, 52] {
            let grids = FrameColumnGrids::new(&app.roster, &[], ProcessTree::Short);
            let grid = &grids.commands.0;
            grid.row_end.set(TableRowEnd::Reserved);
            grid.frame_row_end.set(TableRowEnd::Reserved);
            let area = Rect::new(0, 0, width, 4);
            let headers: Vec<_> = app
                .roster
                .groups()
                .iter()
                .map(|group| {
                    let rows: Vec<_> = group.rows().collect();
                    let mut buffer = Buffer::empty(area);
                    frame_draw_process_table(
                        &mut buffer,
                        area,
                        &rows,
                        Color::Reset,
                        PinnedGroup::Lead(&group.lead.process),
                        grid,
                    );
                    buffer_line(&buffer, 0)
                })
                .collect();
            for title in ["mem", "cpu"] {
                let offsets: Vec<_> = headers.iter().map(|header| header.find(title)).collect();
                assert!(
                    offsets.iter().all(|offset| *offset == offsets[0]),
                    "width {width}, {title}: {headers:#?}"
                );
                if width == 52 {
                    assert!(offsets[0].is_some(), "{title}: {headers:#?}");
                }
            }
        }
    }

    fn nested_progress_app(blocked: bool) -> App {
        let mut app = App::new_for_test().expect("test app");
        app.loaded_config.config.tiles.view = TileView::Cells;
        let mut groups = Vec::new();
        for (pid, subcommand, child_command) in
            [(810_000, "clippy", "check"), (820_000, "nextest", "test")]
        {
            let mut lead = invocation(pid, &[subcommand, "run", "--workspace"]);
            lead.path = format!("~/work-{pid}");
            lead.directory_identity =
                WorkingDirectoryIdentity::Absolute(format!("/test-home/work-{pid}").into());
            lead.resource_use.memory = Measurement::Reading(BYTES_PER_GIBIBYTE);
            lead.state = CaptureLookup::Registered(CaptureRead::Progress(
                if blocked && subcommand == "clippy" {
                    RunState::Blocked
                } else {
                    compiling(837, 1000)
                },
            ));
            let mut child = invocation(pid + 1, &[child_command, "--workspace"]);
            child.path = lead.path.clone();
            child.directory_identity = lead.directory_identity.clone();
            child.parent = VisibleParent::Invocation {
                id: lead.invocation_id.clone(),
                pid,
            };
            child.capture_membership = CaptureMembership::EnclosingUnconfirmed;
            child.state = lead.state.clone();
            child.resource_use.memory = Measurement::Reading(BYTES_PER_GIBIBYTE);
            groups.push(CargoGroup {
                lead,
                rest: vec![child],
                ancestry: AncestryWalk::Ended(vec![]),
            });
        }
        let mut other = invocation(830_000, &["nextest", "run", "--workspace"]);
        other.resource_use.memory = Measurement::Reading(BYTES_PER_GIBIBYTE);
        groups.push(cargo_group(other));
        app.roster.observe(groups, Instant::now());
        app
    }

    fn draw_nested_progress(app: &mut App) -> Terminal<TestBackend> {
        let area = Rect::new(0, 0, 64, 50);
        let mut terminal =
            Terminal::new(TestBackend::new(area.width, area.height)).expect("terminal");
        for _ in 0..3 {
            app.tiles.settle_for_test();
            terminal
                .draw(|frame| {
                    draw_panes(frame, app, area, TileGridContents::Shown);
                })
                .expect("frame");
        }
        terminal
    }

    #[test]
    fn own_account_nested_cells_keep_their_heading_readings_and_memory() {
        let mut app = nested_progress_app(false);
        let terminal = draw_nested_progress(&mut app);
        let placements = app.tiles.placements(
            Rect::new(0, 0, 64, 50),
            app.loaded_config.config.tiles.growth(),
        );
        for pid in [810_000, 820_000] {
            let cell = placements
                .iter()
                .find(|placement| {
                    placement.content == TileContent::Group(InvocationId::for_test(pid))
                })
                .expect("nested cell");
            let text = placement_text(terminal.backend().buffer(), cell);
            assert_eq!(text.matches("83.7%").count(), 1, "{text}");
            assert!(text.lines().nth(1).unwrap().contains("83.7%"), "{text}");
            assert!(text.lines().next().unwrap().contains("mem"), "{text}");
            for line in text.lines().filter(|line| {
                line.trim_start().starts_with(&pid.to_string())
                    || line.trim_start().starts_with(&(pid + 1).to_string())
            }) {
                assert!(!line.contains("83.7%"), "{text}");
            }
        }
    }

    #[test]
    fn a_blocked_nested_cell_keeps_its_neighbours_resource_offsets() {
        let mut app = nested_progress_app(true);
        let terminal = draw_nested_progress(&mut app);
        let placements = app.tiles.placements(
            Rect::new(0, 0, 64, 50),
            app.loaded_config.config.tiles.growth(),
        );
        let text_for = |pid| {
            let cell = placements
                .iter()
                .find(|placement| {
                    placement.content == TileContent::Group(InvocationId::for_test(pid))
                })
                .expect("cell");
            placement_text(terminal.backend().buffer(), cell)
        };
        let blocked = text_for(810_000);
        let other = text_for(830_000);
        for title in ["cpu", "mem"] {
            let offset = blocked
                .lines()
                .next()
                .unwrap()
                .find(title)
                .expect("resource title");
            assert_eq!(
                Some(offset),
                other.lines().next().unwrap().find(title),
                "{blocked}\n{other}"
            );
        }
        assert!(blocked.contains("blocked"), "{blocked}");
        assert!(blocked.contains("1.0G"), "{blocked}");
    }

    #[test]
    fn settled_nested_cells_keep_identical_frames() {
        let mut app = nested_progress_app(true);
        let mut terminal = draw_nested_progress(&mut app);
        app.tiles.settle_for_test();
        terminal
            .draw(|frame| {
                draw_panes(
                    frame,
                    &mut app,
                    Rect::new(0, 0, 64, 50),
                    TileGridContents::Shown,
                );
            })
            .expect("settled frame");
        let settled = terminal.backend().buffer().clone();
        for _ in 0..17 {
            terminal
                .draw(|frame| {
                    draw_panes(
                        frame,
                        &mut app,
                        Rect::new(0, 0, 64, 50),
                        TileGridContents::Shown,
                    );
                })
                .expect("repeated frame");
            assert!(!app.tiles.tick(), "unchanged rows queued motion");
            assert_eq!(terminal.backend().buffer(), &settled);
        }
    }

    #[test]
    fn a_summary_reading_stands_only_beside_its_own_row() {
        let first = same_second("~/project", 123_456);
        let mut second = same_second("~/project", 234_567);
        second.process.state = CaptureLookup::Registered(CaptureRead::Progress(compiling(79, 100)));
        let buffer = narrow_table_buffer(&[&first, &second], TableKind::Summary, 160, 8);
        assert!(!buffer_line(&buffer, 1).contains('%'));
        assert!(!buffer_line(&buffer, 2).contains("79%"));
        assert!(buffer_line(&buffer, 3).contains("79%"));
    }

    #[test]
    fn a_single_command_summary_keeps_its_reading_at_forty_columns() {
        let first = row(Some(compiling(93, 100)));
        let buffer = narrow_table_buffer(&[&first], TableKind::Summary, 36, 5);
        assert!(buffer_line(&buffer, 0).contains("state"));
        assert!(!buffer_line(&buffer, 1).contains('%'));
        assert!(buffer_line(&buffer, 2).contains("93%"));
    }

    #[test]
    fn summary_readings_and_their_title_share_one_right_edge() {
        let mut first = same_second("~/project", 123_456);
        first.process.state =
            CaptureLookup::Registered(CaptureRead::Progress(compiling(661, 1000)));
        let mut second = same_second("~/project", 234_567);
        second.process.state = CaptureLookup::Registered(CaptureRead::Progress(compiling(35, 100)));
        let mut third = same_second("~/project", 345_678);
        third.process.state = CaptureLookup::Registered(CaptureRead::Progress(compiling(7, 100)));
        let buffer = narrow_table_buffer(&[&first, &second, &third], TableKind::Summary, 160, 8);
        let header = buffer_line(&buffer, 0);
        let edge = header.find("state").unwrap() + "state".len() - 1;
        for y in 2..=4 {
            assert_eq!(buffer_line(&buffer, y).rfind('%'), Some(edge));
        }
    }

    #[test]
    fn a_column_wide_word_wraps_whole_through_process_row() {
        let source = TrackedRow::from(invocation(500, &["test(rear_jack_pixels)"]));
        let area = Rect::new(0, 0, 160, 8);
        let mut layout = CellTableLayout::of(
            &[&source],
            TableKind::Command,
            area,
            Color::Reset,
            ProcessTree::Long,
        );
        let index = layout
            .columns
            .iter()
            .position(|&column| column == COMMAND_COLUMN)
            .unwrap();
        layout.column_widths[index] = 22;
        let drawn = process_row(&source, &layout);
        let mut buffer = Buffer::empty(Rect::new(0, 0, 22, 8));
        Paragraph::new(drawn.cells[COMMAND_COLUMN].clone()).render(buffer.area, &mut buffer);
        assert_eq!(buffer_line(&buffer, 1).trim(), "test(rear_jack_pixels)");
    }

    #[test]
    fn an_unavailable_cpu_cell_draws_no_placeholder() {
        let mut source = row(None);
        source.process.resource_use.cpu =
            Measurement::Unavailable(MeasurementAbsence::FirstObservation);
        let buffer = measurement_row_buffer(&source, TableKind::Command);
        assert_eq!(
            cpu_cell_text(&buffer, TABLE_HEADER_HEIGHT + GROUP_HEADER_HEIGHT),
            ""
        );
    }

    #[test]
    fn a_short_ancestry_marks_hidden_levels_without_marking_arguments() {
        for (ancestors, expected) in [
            (vec![ancestor(100, "claude session")], "100 claude"),
            (
                vec![
                    ancestor(100, "claude session"),
                    ancestor(101, "zsh"),
                    ancestor(102, "bash"),
                ],
                "100 claude …",
            ),
        ] {
            let roster = roster_with_ancestry(
                invocation(500, &["check", "--workspace"]),
                vec![],
                ancestors,
            );
            let area = Rect::new(0, 0, 160, 12);
            let mut buffer = Buffer::empty(area);
            draw_group(
                &mut buffer,
                &roster,
                &InvocationId::for_test(500),
                area,
                Color::Reset,
                &[],
                ProcessTree::Short,
            );
            assert_eq!(buffer_line(&buffer, 0).trim(), expected);
            assert!(
                buffer_rows(&buffer)
                    .iter()
                    .any(|line| line.contains("cargo check …"))
            );
        }
    }

    #[test]
    fn a_cell_without_an_ancestry_row_names_its_parent_in_the_table() {
        let mut process = invocation(500, &["check"]);
        process.parent = VisibleParent::Ancestor(102);
        let roster = roster_with_ancestry(
            process,
            vec![],
            vec![ancestor(100, "claude"), ancestor(102, "bash")],
        );
        let area = Rect::new(0, 0, 160, 3);
        let mut buffer = Buffer::empty(area);
        draw_group(
            &mut buffer,
            &roster,
            &InvocationId::for_test(500),
            area,
            Color::Reset,
            &[],
            ProcessTree::Short,
        );
        assert!(buffer_line(&buffer, 0).contains("parent"));
        assert!(buffer_line(&buffer, 2).contains("102"));
    }

    #[test]
    fn a_long_ancestry_cut_fills_its_last_row_before_its_mark() {
        let ancestors = vec![ancestor(
            100,
            "runner /tmp/long-path/component/remaining-component/final",
        )];
        let area = Rect::new(0, 0, 44, 1);
        let mut buffer = Buffer::empty(area);
        draw_ancestry(
            &mut buffer,
            area,
            &ancestors,
            0,
            Color::Reset,
            AncestryFoot::Other,
            0,
        );
        let line = buffer_line(&buffer, 0);
        let mark = line.find(ELISION).unwrap();
        assert!(!line[..mark].ends_with(' '), "{line}");
        assert_eq!(mark, usize::from(cleared(area).width) - 1, "{line}");
    }

    #[test]
    fn a_short_summary_puts_its_hidden_rows_mark_at_the_row_end() {
        let first = same_second("~/project", 123_456);
        let second = same_second("~/project", 234_567);
        let area = Rect::new(0, 0, 160, 3);
        let mut buffer = Buffer::empty(area);
        draw_process_table(
            &mut buffer,
            area,
            &[&first, &second],
            TableKind::Summary,
            Color::Reset,
            PinnedGroup::Unpinned,
            ProcessTree::Short,
        );
        let line = buffer_line(&buffer, 2);
        assert_eq!(
            buffer[(cleared(area).right() - 1, 2)].symbol(),
            ELISION,
            "{line}"
        );
        assert_eq!(
            buffer[(cleared(area).right() - 1, 2)].fg,
            cut_mark_ink(Color::Reset, 0)
        );
    }

    #[test]
    fn a_mixed_account_narrow_summary_keeps_each_directory_segment() {
        let mut app = App::new_for_test().expect("test app");
        app.loaded_config.config.tiles.view = TileView::Summary;
        let mut context = capture_context();
        context.account.name = AccountName::Resolved("natepiano".into());
        let groups = [
            (500, "mat-alone", context.clone()),
            (501, "check-alone", context.clone()),
            (499, "hana", {
                context.account.uid += 1;
                context.account.name = AccountName::Resolved("hana-linux-1".into());
                context
            }),
        ]
        .into_iter()
        .map(|(pid, segment, context)| {
            let mut process = invocation(pid, &["check"]);
            process.path = format!("~/long-directory-name/{segment}");
            process.directory_identity = WorkingDirectoryIdentity::Absolute(
                format!("/home/long-directory-name/{segment}").into(),
            );
            process.provenance = RowProvenance::Direct(context);
            cargo_group(process)
        })
        .collect();
        app.roster.observe(groups, Instant::now());
        let area = Rect::new(0, 0, 22, 24);
        let mut terminal = Terminal::new(TestBackend::new(22, 25)).expect("terminal");
        terminal
            .draw(|frame| {
                draw_panes(frame, &mut app, area, TileGridContents::Shown);
            })
            .expect("initial draw");
        app.tiles.settle_for_test();
        terminal
            .draw(|frame| {
                draw_panes(frame, &mut app, area, TileGridContents::Shown);
            })
            .expect("settled draw");
        let lines = buffer_rows(terminal.backend().buffer());
        for segment in ["mat-alone", "check-alone", "hana"] {
            assert!(
                lines
                    .iter()
                    .any(|line| line.trim_matches([' ', '│']) == format!("…/{segment}")),
                "{lines:#?}"
            );
        }
        assert!(
            !lines.iter().any(|line| line.contains("[natepiano] …")),
            "{lines:#?}"
        );
    }

    #[test]
    fn a_last_short_summary_row_keeps_whole_words_before_its_mark() {
        let mut app = App::new_for_test().expect("test app");
        app.loaded_config.config.tiles.view = TileView::Summary;
        app.roster.observe(
            (1_234_567..1_234_570)
                .map(|pid| cargo_group(invocation(pid, &["nextest", "run", "--tests"])))
                .collect(),
            Instant::now(),
        );
        let area = Rect::new(0, 0, 32, 6);
        let mut terminal = Terminal::new(TestBackend::new(32, 7)).expect("terminal");
        terminal
            .draw(|frame| {
                draw_panes(frame, &mut app, area, TileGridContents::Shown);
            })
            .expect("initial draw");
        app.tiles.settle_for_test();
        terminal
            .draw(|frame| {
                draw_panes(frame, &mut app, area, TileGridContents::Shown);
            })
            .expect("settled draw");
        let lines = buffer_rows(terminal.backend().buffer());
        let command_start = lines[1]
            .chars()
            .position(|character| character == 'c')
            .expect("command title");
        assert_eq!(
            usize::from(area.right() - 3) - command_start - 1,
            18,
            "18-cell command column"
        );
        let (y, line) = lines
            .iter()
            .enumerate()
            .find(|(_, line)| line.contains("cargo nextest"))
            .expect("last command row");
        assert!(line.contains("cargo nextest …"), "{lines:#?}");
        assert_eq!(line.matches(ELISION).count(), 1, "{line}");
        assert_eq!(
            terminal.backend().buffer()[(area.right() - 3, u16::try_from(y).expect("row"))]
                .symbol(),
            " ",
            "{line}"
        );
        assert!(
            !lines.iter().any(|line| line.contains("run…")),
            "{lines:#?}"
        );
    }

    #[test]
    fn a_table_drops_one_accounts_tags_together_and_keeps_other_accounts() {
        let mut long = row_at("~/long-directory-name/project", None);
        long.process.provenance = RowProvenance::Direct(capture_context());
        let mut short = row_at("~/short", None);
        short.process.pid += 1;
        short.process.provenance = long.process.provenance.clone();
        let mut other = row_at("~/other", None);
        other.process.pid += 2;
        let mut context = capture_context();
        context.account.uid += 1;
        context.account.name = AccountName::Resolved("second".to_string());
        other.process.provenance = RowProvenance::Direct(context);
        let buffer = narrow_table_buffer(&[&long, &short, &other], TableKind::Summary, 36, 12);
        let lines = buffer_rows(&buffer);
        assert!(
            lines.iter().any(|line| line.trim() == "~/short"),
            "{lines:#?}"
        );
        assert!(
            lines.iter().any(|line| line.trim() == "[second] ~/other"),
            "{lines:#?}"
        );
        assert!(
            !lines.iter().any(|line| line.contains("[runner-one]")),
            "{lines:#?}"
        );
    }

    #[test]
    fn an_unreadable_heading_keeps_its_account_beside_a_readable_one_that_drops_it() {
        let mut readable = row_at("~/long-directory-name/project", None);
        readable.process.provenance = RowProvenance::Direct(capture_context());
        let mut unavailable = row_at(UNRESOLVED_PATH, None);
        unavailable.process.pid += 1;
        unavailable.process.directory_identity = WorkingDirectoryIdentity::Unavailable;
        unavailable.process.provenance = readable.process.provenance.clone();
        let buffer = narrow_table_buffer(&[&readable, &unavailable], TableKind::Summary, 36, 8);
        let lines = buffer_rows(&buffer);
        assert!(
            lines.iter().any(|line| line.trim() == "[runner-one]"),
            "{lines:#?}"
        );
        assert!(
            !lines.iter().any(|line| line.contains("[runner-one] ~/")),
            "{lines:#?}"
        );
    }

    #[test]
    fn a_cut_prefix_before_a_whole_path_segment_keeps_its_slash() {
        let source = row_at("~/long/hana_catalyst-lfo", None);
        for width in 17..=19 {
            let buffer = narrow_table_buffer(&[&source], TableKind::Summary, width, 5);
            let heading = buffer_line(&buffer, 1);
            if heading.trim().ends_with("hana_catalyst-lfo") {
                assert!(heading.trim().starts_with("…/"), "{width}: {heading}");
            }
            if width == 19 {
                assert_eq!(heading.trim(), "…/hana_catalyst-lfo");
            }
        }
    }

    #[test]
    fn a_toast_leaves_the_summary_foot_drawn() {
        let mut app = App::new_for_test().expect("test app");
        app.started = Instant::now();
        let mut process = invocation(500, &["check"]);
        process.resource_use.memory = Measurement::Reading(BYTES_PER_GIBIBYTE);
        process.subtree_resource_use.memory = process.resource_use.memory;
        app.roster = roster_of(process, vec![]);
        let keymap = std::rc::Rc::clone(&app.keymap);
        let mut terminal = Terminal::new(TestBackend::new(64, 24)).expect("terminal");
        terminal
            .draw(|frame| draw(frame, &mut app, &keymap))
            .expect("bare frame");
        let bare = terminal.backend().buffer().clone();
        app.framework.toasts.push_persistent(
            "toast title",
            "the toast's body",
            ToastStyle::Normal,
            ToastActivation::Informational,
            8,
        );
        terminal
            .draw(|frame| draw(frame, &mut app, &keymap))
            .expect("toast frame");
        let with_toast = terminal.backend().buffer();
        let foot = buffer_rows(&bare)
            .iter()
            .position(|line| line.contains("content rows:"))
            .expect("summary count");
        assert!(buffer_line(&bare, u16::try_from(foot).unwrap()).contains("mem"));
        let card = app.framework.toasts.hits()[0].card_rect;
        assert!(
            usize::from(card.bottom()) < foot,
            "card {card:?}, foot {foot}"
        );
        assert!(
            usize::from(card.bottom().saturating_add(1)) <= foot,
            "cleared gap reaches the foot"
        );
        for x in 0..64 {
            assert_eq!(
                with_toast[(x, u16::try_from(foot).unwrap())],
                bare[(x, u16::try_from(foot).unwrap())],
                "foot cell {x}"
            );
        }
    }

    #[test]
    fn a_toast_uses_cells_below_a_six_row_summary_without_clearing_its_foot() {
        let mut app = App::new_for_test().expect("test app");
        app.started = Instant::now();
        let mut process = invocation(500, &["check"]);
        let arguments: Vec<_> = std::iter::once("check")
            .chain(
                std::iter::repeat_n(["--color", "very_long_configuration_argument"], 40).flatten(),
            )
            .collect();
        process.command = CommandText::of("cargo", &arguments);
        app.tree = ProcessTree::Long;
        process.resource_use.memory = Measurement::Reading(BYTES_PER_GIBIBYTE);
        process.subtree_resource_use.memory = process.resource_use.memory;
        app.roster = roster_of(process, vec![]);
        app.loaded_config.config.tiles.view = TileView::Cells;
        let keymap = std::rc::Rc::clone(&app.keymap);
        let mut terminal = Terminal::new(TestBackend::new(64, 24)).expect("terminal");
        terminal
            .draw(|frame| draw(frame, &mut app, &keymap))
            .expect("bare frame");
        app.tiles.settle_for_test();
        terminal
            .draw(|frame| draw(frame, &mut app, &keymap))
            .expect("settled frame");
        let bare = terminal.backend().buffer().clone();
        let foot = buffer_rows(&bare)
            .iter()
            .position(|line| line.contains("content rows:"))
            .expect("summary foot");
        assert_eq!(foot, 4, "{:?}", buffer_rows(&bare));
        app.framework.toasts.push_persistent(
            "toast title",
            "body",
            ToastStyle::Normal,
            ToastActivation::Informational,
            3,
        );
        terminal
            .draw(|frame| draw(frame, &mut app, &keymap))
            .expect("toast frame");
        let card = app
            .framework
            .toasts
            .hits()
            .first()
            .expect("visible toast")
            .card_rect;
        assert_eq!(terminal.backend().buffer()[(card.x, card.y)].symbol(), "┌");
        assert!(buffer_line(&bare, u16::try_from(foot).unwrap()).contains("mem"));
        for x in 0..64 {
            assert_eq!(
                terminal.backend().buffer()[(x, u16::try_from(foot).unwrap())],
                bare[(x, u16::try_from(foot).unwrap())]
            );
        }
    }

    #[test]
    fn a_toast_outside_the_summary_columns_keeps_its_original_rect() {
        let mut app = App::new_for_test().expect("test app");
        app.started = Instant::now();
        app.roster = directory_rich_roster(&[1; 5]);
        app.loaded_config.config.tiles.view = TileView::Cells;
        app.framework.toasts.set_settings(
            tui_pane::ToastSettings::from_table(
                &"[toasts]\nplacement = 'bottom_right'\nwidth = 32\n"
                    .parse()
                    .expect("toast settings table"),
            )
            .expect("toast settings"),
        );
        app.framework.toasts.push_persistent(
            "toast title",
            "body",
            ToastStyle::Normal,
            ToastActivation::Informational,
            3,
        );
        let keymap = std::rc::Rc::clone(&app.keymap);
        let mut terminal = Terminal::new(TestBackend::new(200, 60)).expect("terminal");
        terminal
            .draw(|frame| draw(frame, &mut app, &keymap))
            .expect("frame");
        app.tiles.settle_for_test();
        terminal
            .draw(|frame| draw(frame, &mut app, &keymap))
            .expect("settled frame");
        let actual = app.framework.toasts.hits()[0].card_rect;
        terminal
            .draw(|frame| {
                tui_pane::render_toasts(
                    frame,
                    &mut app.framework,
                    tui_pane::frame_inner(Rect::new(0, 0, 200, 59)),
                );
            })
            .expect("toast alone");
        let expected = app.framework.toasts.hits()[0].card_rect;
        assert_eq!(actual, expected);
        assert_eq!(
            terminal.backend().buffer()[(actual.x, actual.y)].symbol(),
            "┌"
        );
    }

    #[test]
    fn hidden_grid_contents_leave_the_toast_at_its_unrestricted_rect() {
        let mut app = App::new_for_test().expect("test app");
        app.roster = directory_rich_roster(&[1; 2]);
        app.loaded_config.config.tiles.view = TileView::Cells;
        app.framework.toasts.push_persistent(
            "title",
            "body",
            ToastStyle::Normal,
            ToastActivation::Informational,
            3,
        );
        let area = Rect::new(0, 0, 64, 23);
        let mut terminal = Terminal::new(TestBackend::new(64, 24)).expect("terminal");
        terminal
            .draw(|frame| {
                draw_panes(frame, &mut app, area, TileGridContents::Shown);
            })
            .expect("initial frame");
        app.tiles.settle_for_test();
        terminal
            .draw(|frame| {
                let frames = draw_panes(frame, &mut app, area, TileGridContents::Hidden);
                assert!(frames.0.is_empty(), "hidden summaries are not painted");
                let allowed = app
                    .framework
                    .toasts
                    .area_avoiding_frames(tui_pane::frame_inner(area), &frames.0);
                tui_pane::render_toasts(frame, &mut app.framework, allowed);
            })
            .expect("hidden contents");
        let actual = app.framework.toasts.hits()[0].card_rect;
        assert_eq!(
            terminal.backend().buffer()[(actual.x, actual.y)].symbol(),
            "┌"
        );
        terminal
            .draw(|frame| {
                tui_pane::render_toasts(frame, &mut app.framework, tui_pane::frame_inner(area));
            })
            .expect("toast alone");
        assert_eq!(actual, app.framework.toasts.hits()[0].card_rect);
    }

    #[test]
    fn an_empty_compiler_and_runs_slot_returns_a_cut_commands_last_argument() {
        let mut app = App::new_for_test().expect("test app");
        app.tree = ProcessTree::Long;
        app.loaded_config.config.tiles.view = TileView::Cells;
        app.tiles.set_min_tile_height(8);
        let mut first = invocation(500, &["check"]);
        first.resource_use.compiler = CompilerObservation::Running(Compiler {
            name:  "sccache",
            count: 15,
        });
        first.managed = Measurement::Reading(1000);
        let mut second = invocation(
            600,
            &[
                "nextest",
                "run",
                "--cargo-profile",
                "release",
                "-p",
                "cargo-tile",
                "--run-ignored",
                "only",
                "-E",
                "test(panic93_sweep_sizes)",
                "an_additional_argument_that_takes_one_more_command_line_further",
                "--no-capture",
            ],
        );
        second.resource_use.compiler = CompilerObservation::None;
        second.managed = Measurement::Unavailable(MeasurementAbsence::FirstObservation);
        app.roster.observe(
            vec![
                CargoGroup {
                    lead:     first,
                    rest:     (501..521).map(|pid| invocation(pid, &["check"])).collect(),
                    ancestry: AncestryWalk::Ended(vec![]),
                },
                CargoGroup {
                    lead:     second,
                    rest:     vec![],
                    ancestry: AncestryWalk::Ended(vec![]),
                },
            ],
            Instant::now(),
        );
        let area = Rect::new(0, 0, 126, 22);
        let mut terminal = Terminal::new(TestBackend::new(126, 23)).expect("terminal");
        terminal
            .draw(|frame| {
                draw_panes(frame, &mut app, area, TileGridContents::Shown);
            })
            .expect("initial draw");
        app.tiles.settle_for_test();
        terminal
            .draw(|frame| {
                draw_panes(frame, &mut app, area, TileGridContents::Shown);
            })
            .expect("settled draw");
        let placements = app
            .tiles
            .placements(area, app.loaded_config.config.tiles.growth());
        let second = placements
            .iter()
            .find(|placement| {
                placement.content == TileContent::Group(app.roster.groups()[1].id.clone())
            })
            .expect("second cell");
        let text = placement_text(terminal.backend().buffer(), second);
        assert!(text.contains("--no-capture"), "{text}");
        assert!(
            !text.contains("compiler") && !text.contains("runs"),
            "{text}"
        );
        let first = placements
            .iter()
            .find(|placement| {
                placement.content == TileContent::Group(app.roster.groups()[0].id.clone())
            })
            .expect("first cell");
        let text = placement_text(terminal.backend().buffer(), first);
        assert!(text.contains("compiler") && text.contains("runs"), "{text}");
    }

    #[test]
    fn reserved_long_columns_stay_settled_on_repeated_frames() {
        let mut app = App::new_for_test().expect("test app");
        app.tree = ProcessTree::Long;
        app.loaded_config.config.tiles.view = TileView::Cells;
        let groups = (0..8)
            .map(|index| {
                let pid = 500 + index * 100;
                let mut lead = invocation(
                    pid,
                    &[
                        "check",
                        "--color",
                        "a_long_argument_with_several_components_and_a_long_final_component",
                        "a_long_argument_with_several_components_and_a_long_final_component",
                    ],
                );
                lead.path = format!("~/work-{index}");
                lead.directory_identity =
                    WorkingDirectoryIdentity::Absolute(format!("/test-home/work-{index}").into());
                CargoGroup {
                    lead,
                    rest: (pid + 1..pid + 2)
                        .map(|child| invocation(child, &["check"]))
                        .collect(),
                    ancestry: AncestryWalk::Ended(vec![]),
                }
            })
            .collect();
        app.roster.observe(groups, Instant::now());
        let area = Rect::new(0, 0, 126, 66);
        let mut terminal =
            Terminal::new(TestBackend::new(area.width, area.height + 1)).expect("terminal");
        for _ in 0..3 {
            app.tiles.settle_for_test();
            terminal
                .draw(|frame| {
                    draw_panes(frame, &mut app, area, TileGridContents::Shown);
                })
                .expect("settling draw");
        }
        app.tiles.settle_for_test();
        terminal
            .draw(|frame| {
                draw_panes(frame, &mut app, area, TileGridContents::Shown);
            })
            .expect("settled draw");
        let settled = terminal.backend().buffer().clone();
        for _ in 0..3 {
            FRAME_DEMAND_QUERIES.with(|queries| queries.set(0));
            terminal
                .draw(|frame| {
                    draw_panes(frame, &mut app, area, TileGridContents::Shown);
                })
                .expect("repeated draw");
            FRAME_DEMAND_QUERIES
                .with(|queries| assert_eq!(queries.get(), 2, "each probe width is measured once"));
            assert!(
                !app.tiles.tick(),
                "unchanged contents queued another motion"
            );
            assert_eq!(terminal.backend().buffer(), &settled);
        }
    }

    #[test]
    fn a_whole_one_row_cell_keeps_its_release_subcommand_beside_a_cut_table() {
        let mut app = App::new_for_test().expect("test app");
        app.loaded_config.config.tiles.view = TileView::Cells;
        app.tiles.set_min_tile_height(4);
        app.loaded_config.config.tiles.initial_rows =
            toml::from_str::<crate::config::TilesConfig>("initial_rows = 2")
                .expect("tile config")
                .initial_rows;
        let mut whole = invocation(1_234_567, &["build", "--release", "--tests"]);
        whole.parent = VisibleParent::Ancestor(1_234_566);
        whole.state = CaptureLookup::Registered(CaptureRead::Progress(RunState::Blocked));
        let mut first = invocation(1_000_000, &["check"]);
        first.state = CaptureLookup::Registered(CaptureRead::Progress(RunState::Blocked));
        app.roster.observe(
            vec![
                CargoGroup {
                    lead:     first,
                    rest:     (1_000_001..1_000_031)
                        .map(|pid| {
                            let mut child = invocation(pid, &["check"]);
                            child.state = CaptureLookup::Registered(CaptureRead::Progress(
                                compiling(65, 100),
                            ));
                            child
                        })
                        .collect(),
                    ancestry: AncestryWalk::Ended(vec![]),
                },
                cargo_group(whole),
                cargo_group(invocation(1_234_568, &["check"])),
                cargo_group(invocation(1_234_569, &["check"])),
            ],
            Instant::now(),
        );
        let area = Rect::new(0, 0, 84, 24);
        let mut terminal = Terminal::new(TestBackend::new(84, 25)).expect("terminal");
        terminal
            .draw(|frame| {
                draw_panes(frame, &mut app, area, TileGridContents::Shown);
            })
            .expect("initial draw");
        app.tiles.settle_for_test();
        terminal
            .draw(|frame| {
                draw_panes(frame, &mut app, area, TileGridContents::Shown);
            })
            .expect("settled draw");
        let placements = app
            .tiles
            .placements(area, app.loaded_config.config.tiles.growth());
        let whole = placements
            .iter()
            .find(|placement| {
                placement.content == TileContent::Group(InvocationId::for_test(1_234_567))
            })
            .expect("one-row cell");
        let text = placement_text(terminal.backend().buffer(), whole);
        assert!(text.contains("[release] cargo build …"), "{text}");
        assert_eq!(whole.frame.inner().height, 4, "{text}");
        let cut = placements
            .iter()
            .find(|placement| {
                placement.content == TileContent::Group(InvocationId::for_test(1_000_000))
            })
            .expect("cut cell");
        assert!(!placement_text(terminal.backend().buffer(), cut).contains("1000030"));
    }

    #[test]
    fn hiding_and_whole_cells_measure_the_same_reserved_column_ends() {
        let mut app = App::new_for_test().expect("test app");
        app.tree = ProcessTree::Long;
        app.loaded_config.config.tiles.view = TileView::Cells;
        let make_lead = |pid| {
            let mut process = invocation(
                pid,
                &[
                    "check",
                    "--color",
                    "a_long_argument_with_several_components_and_a_long_final_component",
                    "a_long_argument_with_several_components_and_a_long_final_component",
                    "a_long_argument_with_several_components_and_a_long_final_component",
                ],
            );
            process.resource_use.compiler = CompilerObservation::Running(Compiler {
                name:  "sccache",
                count: 15,
            });
            process
        };
        app.roster.observe(
            vec![
                CargoGroup {
                    lead:     make_lead(500),
                    rest:     (501..551)
                        .map(|pid| {
                            let mut child = invocation(pid, &["check"]);
                            child.resource_use.compiler = CompilerObservation::None;
                            child
                        })
                        .collect(),
                    ancestry: AncestryWalk::Ended(vec![]),
                },
                CargoGroup {
                    lead:     make_lead(600),
                    rest:     vec![],
                    ancestry: AncestryWalk::Ended(vec![]),
                },
            ],
            Instant::now(),
        );
        let area = Rect::new(0, 0, 126, 50);
        let mut terminal = Terminal::new(TestBackend::new(126, 51)).expect("terminal");
        terminal
            .draw(|frame| {
                draw_panes(frame, &mut app, area, TileGridContents::Shown);
            })
            .expect("initial draw");
        app.tiles.settle_for_test();
        terminal
            .draw(|frame| {
                draw_panes(frame, &mut app, area, TileGridContents::Shown);
            })
            .expect("settled draw");
        let placements = app
            .tiles
            .placements(area, app.loaded_config.config.tiles.growth());
        let mut compiler_columns = Vec::new();
        for group in app.roster.groups() {
            let placement = placements
                .iter()
                .find(|placement| placement.content == TileContent::Group(group.id.clone()))
                .expect("command cell");
            let rect = placement.frame.inner();
            let header = buffer_line(terminal.backend().buffer(), rect.y);
            compiler_columns.push(header.find("compiler").unwrap_or_else(|| {
                panic!(
                    "compiler column: {}",
                    placement_text(terminal.backend().buffer(), placement)
                )
            }));
            let text = placement_text(terminal.backend().buffer(), placement);
            assert!(text.contains(&group.lead.process.pid.to_string()), "{text}");
        }
        assert_eq!(compiler_columns[0], compiler_columns[1]);
        let first = placements
            .iter()
            .find(|placement| {
                placement.content == TileContent::Group(app.roster.groups()[0].id.clone())
            })
            .expect("hiding cell");
        assert!(!placement_text(terminal.backend().buffer(), first).contains("550"));
    }

    #[test]
    fn a_short_profiled_command_keeps_its_subcommand_and_argument_mark_whole() {
        for (profile, pid) in [("p19b", 1_234_567), ("release", 12_345)] {
            let source = TrackedRow::from(invocation(pid, &["build", "--profile", profile]));
            let mut first = TrackedRow::from(invocation(pid + 1, &["check"]));
            first.process.state =
                CaptureLookup::Registered(CaptureRead::Progress(compiling(661, 1000)));
            let mut second = TrackedRow::from(invocation(pid + 2, &["check"]));
            second.process.state =
                CaptureLookup::Registered(CaptureRead::Progress(compiling(661, 1000)));
            let area = Rect::new(0, 0, 38, 8);
            let mut buffer = Buffer::empty(area);
            draw_process_table(
                &mut buffer,
                area,
                &[&source, &first, &second],
                TableKind::Summary,
                Color::Reset,
                PinnedGroup::Unpinned,
                ProcessTree::Short,
            );
            assert!(
                buffer_line(&buffer, 2).contains(&format!("[{profile}] cargo build …")),
                "{:?}",
                buffer_rows(&buffer)
            );
        }
    }

    #[test]
    fn a_shared_state_slot_is_titled_in_each_table_that_keeps_it() {
        let waiting = row(Some(RunState::Blocked));
        let mut running = row(None);
        running.process.pid += 1;
        let grid =
            FrameTableColumnGrid::of(&[&waiting, &running], TableKind::Command, ProcessTree::Long);
        let area = Rect::new(0, 0, 160, 6);
        let mut headers = Vec::new();
        for source in [&waiting, &running] {
            let mut buffer = Buffer::empty(area);
            frame_draw_process_table(
                &mut buffer,
                area,
                &[source],
                Color::Reset,
                PinnedGroup::Unpinned,
                &grid,
            );
            headers.push(buffer_line(&buffer, 0));
        }
        assert!(headers[0].contains("state"), "{headers:#?}");
        assert!(!headers[1].contains("state"), "{headers:#?}");
        for title in ["pid", "parent", "start", "dur", "cpu", "mem"] {
            assert_eq!(
                headers[0].find(title),
                headers[1].find(title),
                "{headers:#?}"
            );
        }
    }

    #[test]
    fn an_empty_state_slot_keeps_the_command_aligned_with_stateful_tables() {
        let waiting = row(Some(RunState::Blocked));
        let mut running = row(None);
        running.process.pid += 1;
        let first = row(Some(compiling(1, 2)));
        let second = row(Some(compiling(1, 2)));
        let mut grid = FrameTableColumnGrid::of(
            &[&waiting, &running, &first, &second],
            TableKind::Command,
            ProcessTree::Long,
        );
        grid.set_frame_slots(vec![PID_COLUMN, STATE_COLUMN, COMMAND_COLUMN]);
        let area = Rect::new(0, 0, 40, 6);
        let mut headers = Vec::new();
        for source in [&waiting, &running] {
            let mut buffer = Buffer::empty(area);
            frame_draw_process_table(
                &mut buffer,
                area,
                &[source],
                Color::Reset,
                PinnedGroup::Unpinned,
                &grid,
            );
            headers.push(buffer_line(&buffer, 0));
        }
        assert!(headers[0].contains("state"));
        assert!(!headers[1].contains("state"));
        assert_eq!(headers[0].find("pid"), headers[1].find("pid"));
        let waiting_command = headers[0].find("command").unwrap();
        let running_command = headers[1].find("command").unwrap();
        assert_eq!(running_command, waiting_command, "{headers:#?}");
        let mut long_running = running;
        long_running.process.command = CommandText::of("a_long_session_runner_program", &[]);
        let mut long_waiting = waiting;
        long_waiting.process.command = long_running.process.command.clone();
        let long_grid = FrameTableColumnGrid::of(
            &[&long_waiting, &long_running, &first, &second],
            TableKind::Command,
            ProcessTree::Long,
        );
        let long_area = Rect::new(0, 0, 62, 6);
        let mut long_buffer = Buffer::empty(long_area);
        frame_draw_process_table(
            &mut long_buffer,
            long_area,
            &[&long_running],
            Color::Reset,
            PinnedGroup::Unpinned,
            &long_grid,
        );
        let header = buffer_line(&long_buffer, 0);
        assert!(
            header.contains("cpu") && header.contains("mem") && header.contains("dur"),
            "{:?}",
            buffer_rows(&long_buffer)
        );
        for title in ["start", "state"] {
            assert!(!header.contains(title), "{:?}", buffer_rows(&long_buffer));
        }
    }

    #[test]
    fn each_short_ancestry_mark_reads_as_what_it_marks() {
        for (walk, name_cut, hidden, walk_cut) in [
            (
                AncestryWalk::Ended(vec![ancestor(
                    100,
                    "/opt/very_long_session_runner_executable_name",
                )]),
                true,
                false,
                false,
            ),
            (
                AncestryWalk::Ended(vec![
                    ancestor(100, "claude session"),
                    ancestor(101, "zsh"),
                    ancestor(102, "bash"),
                ]),
                false,
                true,
                false,
            ),
            (
                AncestryWalk::LimitCut(vec![ancestor(100, "claude session")]),
                false,
                false,
                true,
            ),
            (
                AncestryWalk::LimitCut(vec![
                    ancestor(100, "/opt/very_long_session_runner_executable_name"),
                    ancestor(101, "zsh"),
                    ancestor(102, "bash"),
                ]),
                true,
                true,
                true,
            ),
        ] {
            let mut group = cargo_group(invocation(500, &["check", "--workspace"]));
            group.ancestry = walk;
            let mut roster = Roster::new();
            roster.observe(vec![group], Instant::now());
            let area = Rect::new(0, 0, 40, 12);
            let mut buffer = Buffer::empty(area);
            draw_group(
                &mut buffer,
                &roster,
                &InvocationId::for_test(500),
                area,
                Color::Reset,
                &[],
                ProcessTree::Short,
            );
            let line = buffer_line(&buffer, 0);
            let text = line.trim();
            assert_eq!(text.starts_with("… "), walk_cut, "{line}");
            assert_eq!(text.ends_with(" …"), hidden, "{line}");
            if name_cut {
                let name = if hidden {
                    text.strip_suffix(" …").expect("hidden-level mark")
                } else {
                    text
                };
                assert!(name.ends_with(ELISION));
                assert!(!name.strip_suffix(ELISION).expect("name cut").ends_with(' '));
            }
            for (x, cell) in (0..area.width).map(|x| (x, &buffer[(x, 0)])) {
                if cell.symbol() == ELISION {
                    assert_eq!(cell.fg, cut_mark_ink(Color::Reset, 0), "mark at {x}");
                }
            }
        }
    }

    #[test]
    fn a_kept_ellipsis_word_is_not_an_argument_omission() {
        let mut group = cargo_group(invocation(500, &["check"]));
        group.ancestry = AncestryWalk::Ended(vec![ancestor(100, "cargo …")]);
        let mut roster = Roster::new();
        roster.observe(vec![group], Instant::now());
        let area = Rect::new(0, 0, 80, 12);
        let mut buffer = Buffer::empty(area);
        draw_group(
            &mut buffer,
            &roster,
            &InvocationId::for_test(500),
            area,
            Color::Reset,
            &[],
            ProcessTree::Short,
        );
        assert_eq!(buffer_line(&buffer, 0).trim(), "100 cargo …");
    }

    #[test]
    fn a_summarys_separators_take_a_column_mates_spare_row() {
        for (commands, height, whole) in [(3, 27, true), (4, 35, true), (4, 33, false)] {
            let roster = directory_rich_roster(&vec![1; commands]);
            let area = Rect::new(0, 0, 40, height);
            let (grid, buffer) = settled_cells_buffer(&roster, area);
            let growth = TileGrowth {
                initial_rows:  12,
                fill:          TileFill::Redistribute,
                widen_summary: false,
            };
            let placements = grid.placements(area, growth);
            let summary = placements
                .iter()
                .find(|placement| placement.content == TileContent::Summary)
                .expect("summary");
            let inner = summary.frame.inner();
            let content = Rect {
                height: inner.height.saturating_sub(1),
                ..inner
            };
            let use_rows = content_row_use(
                &roster,
                &TileContent::Summary,
                content,
                &hidden_when_idle(),
                ProcessTree::Long,
            );
            let spare = placements.iter().any(|placement| {
                matches!(placement.content, TileContent::Group(_))
                    && has_blank_row_above_foot(&buffer, placement)
            });
            if whole {
                assert_eq!(
                    use_rows.separator_rows_yielded,
                    0,
                    "{commands}/{height}: {:?}",
                    buffer_rows(&buffer)
                );
                assert!(
                    !spare,
                    "a cell keeps a spare row: {:?}",
                    buffer_rows(&buffer)
                );
            }
            assert!(
                !(use_rows.separator_rows_yielded > 0 && spare),
                "commands={commands} height={height} yielded={} {:?}",
                use_rows.separator_rows_yielded,
                buffer_rows(&buffer)
            );
        }
    }

    fn reading_scope_roster() -> Roster {
        let mut parent = invocation(500, &["check"]);
        parent.resource_use.cpu = Measurement::Reading(10.0);
        parent.resource_use.memory = Measurement::Reading(BYTES_PER_GIBIBYTE);
        let mut child = invocation(501, &["check"]);
        child.parent = VisibleParent::Invocation {
            id:  parent.invocation_id.clone(),
            pid: parent.pid,
        };
        child.resource_use.cpu = Measurement::Reading(90.0);
        child.resource_use.memory = Measurement::Reading(2 * BYTES_PER_GIBIBYTE);
        child.resource_use.compiler = CompilerObservation::Running(Compiler {
            name:  "sccache",
            count: 6,
        });
        roster_of(parent, vec![child])
    }

    fn reading_scope_buffer(roster: &Roster, summary: bool, height: u16) -> Buffer {
        let area = Rect::new(0, 0, 200, height);
        let mut buffer = Buffer::empty(area);
        let grids = FrameColumnGrids::new(roster, &[], ProcessTree::Long);
        let rows: Vec<_> = if summary {
            vec![&roster.groups()[0].lead]
        } else {
            roster.groups()[0].rows().collect()
        };
        let grid = if summary {
            &grids.summary.0
        } else {
            &grids.commands.0
        };
        frame_draw_process_table(
            &mut buffer,
            area,
            &rows,
            Color::Reset,
            PinnedGroup::Unpinned,
            grid,
        );
        buffer
    }

    /// Keep the finished row in the roster while the current scan replaces its resources.
    fn cpu_turnover_roster(
        lead: CargoProcess,
        live: Vec<CargoProcess>,
        finished: CargoProcess,
    ) -> Roster {
        let mut initial = live.clone();
        initial.push(finished);
        let mut group = cargo_group(lead);
        group.rest = initial;
        let mut roster = Roster::new();
        let now = Instant::now();
        roster.observe(vec![group.clone()], now);
        group.rest = live;
        roster.observe(vec![group], now + Duration::from_millis(1));
        assert!(roster.groups()[0].rows().any(TrackedRow::is_ended));
        roster
    }

    #[test]
    fn a_mend_cell_keeps_live_compiler_cpu_while_finished_children_fade() {
        let mut mend = invocation(1_008_494, &["mend", "--all-targets"]);
        mend.resource_use.cpu = Measurement::Reading(0.0);
        mend.resource_use.memory = Measurement::Reading(0);
        let mut check = invocation(1_008_717, &["check", "--workspace"]);
        check.parent = VisibleParent::Invocation {
            id:  mend.invocation_id.clone(),
            pid: mend.pid,
        };
        check.resource_use.cpu = Measurement::Reading(1586.0);
        check.resource_use.memory = Measurement::Reading(3 * BYTES_PER_GIBIBYTE);
        check.resource_use.compiler = CompilerObservation::Running(Compiler {
            name:  "rustc",
            count: 7,
        });
        let mut finished = invocation(1_008_718, &["test", "--no-run"]);
        finished.parent = VisibleParent::Invocation {
            id:  check.invocation_id.clone(),
            pid: check.pid,
        };
        finished.resource_use.cpu = Measurement::Unavailable(MeasurementAbsence::FirstObservation);
        finished.resource_use.memory = Measurement::Reading(BYTES_PER_GIBIBYTE);
        let roster = cpu_turnover_roster(mend, vec![check], finished);
        let buffer = reading_scope_buffer(&roster, false, 3);
        let line = buffer_line(&buffer, 2);
        assert!(line.contains("1008494"), "{line}");
        assert!(line.contains("1586%"), "{line}");
        assert!(line.contains("3.0G"), "{line}");
        assert!(line.contains("rustc×7"), "{line}");
    }

    #[test]
    fn a_companion_nextest_keeps_its_cpu_while_an_unmeasured_child_fades() {
        for cpu in [0.0, 1.0] {
            let mut nextest = invocation(1_155_700, &["nextest", "run", "--locked"]);
            nextest.resource_use.cpu = Measurement::Reading(cpu);
            let mut child = invocation(1_155_701, &["test", "--no-run"]);
            child.parent = VisibleParent::Invocation {
                id:  nextest.invocation_id.clone(),
                pid: nextest.pid,
            };
            child.resource_use.cpu = Measurement::Unavailable(MeasurementAbsence::FirstObservation);
            let roster = cpu_turnover_roster(nextest, Vec::new(), child);
            let buffer = reading_scope_buffer(&roster, false, 3);
            let line = buffer_line(&buffer, 2);
            assert!(line.contains("1155700"), "{line}");
            assert!(line.contains(&format!("{cpu}%")), "{line}");
        }
    }

    #[test]
    fn a_row_reading_sums_its_known_contributions() {
        let roster = reading_scope_roster();
        let buffer = reading_scope_buffer(&roster, true, 8);
        let line = buffer_line(&buffer, 2);
        assert!(line.contains("100%"), "{line}");
        assert!(line.contains("3.0G"), "{line}");
    }

    #[test]
    fn a_cut_table_moves_hidden_rows_use_into_their_drawn_ancestor() {
        let roster = reading_scope_roster();
        let whole = reading_scope_buffer(&roster, false, 8);
        let cut = reading_scope_buffer(&roster, false, 3);
        assert!(buffer_line(&whole, 2).contains("10%"));
        assert!(buffer_line(&whole, 3).contains("90%"));
        assert!(
            buffer_line(&cut, 2).contains("100%"),
            "{:?}",
            buffer_rows(&cut)
        );
        assert!(buffer_line(&cut, 2).contains("3.0G"));
        assert!(!buffer_line(&whole, 2).contains("sccache"));
        assert!(buffer_line(&whole, 3).contains("sccache×6"));
        assert!(buffer_line(&cut, 2).contains("sccache×6"));
    }

    #[test]
    fn a_cut_table_keeps_the_whole_combined_compiler_count() {
        let mut parent = invocation(500, &["build"]);
        parent.resource_use.compiler = CompilerObservation::Running(Compiler {
            name:  "sccache",
            count: 9,
        });
        let mut child = invocation(501, &["check"]);
        child.parent = VisibleParent::Invocation {
            id:  parent.invocation_id.clone(),
            pid: parent.pid,
        };
        child.resource_use.compiler = CompilerObservation::Running(Compiler {
            name:  "sccache",
            count: 6,
        });
        parent.subtree_resource_use =
            InvocationResourceUse::sum([parent.resource_use.clone(), child.resource_use.clone()]);
        let roster = roster_of(parent, vec![child]);
        let buffer = reading_scope_buffer(&roster, false, 3);
        assert!(
            buffer_line(&buffer, 2).contains("sccache×15"),
            "{:?}",
            buffer_rows(&buffer)
        );
    }

    #[test]
    fn unavailable_resource_cells_draw_only_spaces() {
        for reason in [
            MeasurementAbsence::FirstObservation,
            MeasurementAbsence::ReadFailed,
            MeasurementAbsence::Unproven,
        ] {
            let mut absent = invocation(500, &["build"]);
            absent.resource_use = InvocationResourceUse {
                cpu:      Measurement::Unavailable(reason),
                memory:   Measurement::Unavailable(reason),
                compiler: CompilerObservation::Unknown,
            };
            absent.managed = Measurement::Unavailable(reason);
            let mut known = invocation(501, &["build"]);
            known.resource_use.compiler = CompilerObservation::Running(Compiler {
                name:  "sccache",
                count: 6,
            });
            known.managed = Measurement::Reading(7);
            let rows = [TrackedRow::from(absent), TrackedRow::from(known)];
            let buffer = narrow_table_buffer(&[&rows[0], &rows[1]], TableKind::Command, 180, 6);
            let header = buffer_line(&buffer, 0);
            let text: String = (0..buffer.area.width)
                .map(|x| buffer[(x, 2)].symbol())
                .collect();
            for (label, next) in [("mem", "command"), ("compiler", "runs")] {
                let start = header.find(label).expect("column title");
                let end = header.find(next).expect("next title");
                assert_eq!(text[start..end].trim(), "", "{label}: {text}");
            }
            let start = header.find("runs").expect("runs title");
            assert_eq!(text[start..].trim(), "", "{text}");
        }
    }

    #[test]
    fn an_unreadable_summary_foot_keeps_its_count_without_memory() {
        let mut process = invocation(500, &["check"]);
        process.resource_use.memory = Measurement::Unavailable(MeasurementAbsence::ReadFailed);
        process.subtree_resource_use.memory = process.resource_use.memory;
        let roster = roster_of(process, vec![]);
        let (_, buffer) = settled_cells_buffer(&roster, Rect::new(0, 0, 64, 24));
        let lines = buffer_rows(&buffer);
        let foot = lines
            .iter()
            .find(|line| line.contains("content rows:"))
            .expect("foot");
        assert!(!foot.contains("mem"), "{foot}");
    }

    #[test]
    fn a_height_cut_keeps_its_hidden_childs_compiler_column() {
        let roster = reading_scope_roster();
        let stats = SccacheStats::default();
        let cells = Cells::for_frame(&roster, &[], ProcessTree::Long, &stats, 3);
        let area = Rect::new(0, 0, 200, 3);
        let mut buffer = Buffer::empty(area);
        cells.draw(
            &mut buffer,
            &TileContent::Group(InvocationId::for_test(500)),
            area,
            Color::Reset,
        );
        let lines = buffer_rows(&buffer);
        assert!(
            lines.iter().any(|line| line.contains("compiler")),
            "{lines:#?}"
        );
        assert!(
            lines.iter().any(|line| line.contains("sccache×6")),
            "{lines:#?}"
        );
    }

    #[test]
    fn an_empty_state_slot_reserves_width_beside_a_stateless_commands_name() {
        let mut waiting = row(Some(compiling(1, 2)));
        let mut running = row(None);
        waiting.process.command = CommandText::of("cargo", &["check"]);
        running.process.command = CommandText::of("cargo", &["a_much_longer_command_name"]);
        running.process.pid += 1;
        let mut other = row(Some(compiling(1, 2)));
        other.process.pid += 2;
        other.process.invocation_id = InvocationId::for_test(other.process.pid);
        let mut grid = FrameTableColumnGrid::of(
            &[&waiting, &running, &other],
            TableKind::Command,
            ProcessTree::Long,
        );
        grid.set_frame_slots(vec![PID_COLUMN, STATE_COLUMN, COMMAND_COLUMN]);
        let area = Rect::new(0, 0, 52, 8);
        let mut headers = Vec::new();
        for source in [&waiting, &running] {
            let mut buffer = Buffer::empty(area);
            frame_draw_process_table(
                &mut buffer,
                area,
                &[source],
                Color::Reset,
                PinnedGroup::Unpinned,
                &grid,
            );
            headers.push(buffer_line(&buffer, 0));
        }
        assert!(headers[0].contains("state"), "{headers:#?}");
        assert!(!headers[1].contains("state"), "{headers:#?}");
        assert_eq!(
            headers[0].find("pid"),
            headers[1].find("pid"),
            "{headers:#?}"
        );
        assert_eq!(
            headers[1].find("command").expect("stateless command"),
            headers[0].find("command").expect("reading command"),
            "{headers:#?}"
        );
    }

    #[test]
    fn a_narrow_summary_keeps_readings_after_the_command_leaves() {
        let mut first = same_second("~/project", 3_053_262);
        first.process.command = CommandText::of("cargo", &["nextest", "run", "--workspace"]);
        first.process.state = CaptureLookup::Registered(CaptureRead::Progress(compiling(52, 100)));
        let mut second = same_second("~/project", 3_070_134);
        second.process.command = CommandText::of("cargo", &["clippy", "--workspace"]);
        second.process.state = CaptureLookup::Registered(CaptureRead::Progress(RunState::Blocked));
        let buffer = narrow_table_buffer(&[&first, &second], TableKind::Summary, 20, 12);
        let lines = buffer_rows(&buffer);
        assert_eq!(
            lines[0].split_whitespace().collect::<Vec<_>>(),
            ["pid", "state"]
        );
        for (pid, reading) in [("3053262", "52%"), ("3070134", "blocked")] {
            assert!(
                lines
                    .iter()
                    .any(|line| line.contains(pid) && line.contains(reading)),
                "{lines:#?}"
            );
        }
    }

    #[test]
    fn a_narrow_long_summary_keeps_readings_after_the_command_leaves() {
        let mut first = same_second("~/project", 1_770_300);
        first.process.command = CommandText::of("cargo", &["build", "--release"]);
        first.process.state =
            CaptureLookup::Registered(CaptureRead::Progress(compiling(520, 1000)));
        let mut second = same_second("~/project", 1_770_301);
        second.process.command = CommandText::of("cargo", &["clippy", "--workspace"]);
        second.process.state = CaptureLookup::Registered(CaptureRead::Progress(RunState::Blocked));
        let extras: Vec<_> = (0..12)
            .map(|offset| same_second("~/project", 1_770_302 + offset))
            .collect();
        let rows: Vec<_> = [&first, &second].into_iter().chain(extras.iter()).collect();
        let buffer = narrow_table_buffer(&rows, TableKind::Summary, 18, 12);
        let lines = buffer_rows(&buffer);
        assert_eq!(
            lines[0].split_whitespace().collect::<Vec<_>>(),
            ["pid", "state"]
        );
        for (pid, reading) in [("1770300", "52.0%"), ("1770301", "blocked")] {
            assert!(
                lines
                    .iter()
                    .any(|line| line.contains(pid) && line.contains(reading)),
                "{lines:#?}"
            );
        }
    }

    #[test]
    fn a_summary_alone_uses_its_last_row_for_a_separator_before_its_foot() {
        let roster = directory_rich_roster(&[1; 26]);
        let stats = SccacheStats::new();
        let cells = Cells::for_frame(&roster, &[], ProcessTree::Short, &stats, 46);
        let area = Rect::new(0, 0, 40, 49);
        let mut buffer = Buffer::empty(area);
        let mut grid = TileGrid::new();
        grid.set_view(TileView::Summary);
        tui_pane::draw_tile_grid(
            &mut buffer,
            &mut grid,
            area,
            TileGrowth::default(),
            TileGridContents::Shown,
            &cells,
        );
        let lines = buffer_rows(&buffer);
        assert!(lines[47].contains("content rows:"), "{lines:#?}");
        assert!(!lines[46].trim_matches([' ', '│']).is_empty(), "{lines:#?}");
        assert_eq!(
            (2..47)
                .filter(|&y| lines[y].trim_matches([' ', '│']).is_empty())
                .count(),
            1,
            "{lines:#?}"
        );
        assert_eq!(
            lines.iter().filter(|line| line.contains("~/work-")).count(),
            22,
            "{lines:#?}"
        );
    }

    #[test]
    fn an_uncut_table_uses_its_full_width_with_a_later_grid_slot() {
        let mut first = row(None);
        first.process.command = CommandText::of("cargo", &["check", "abcdefghijklmnopqrstuv"]);
        first.process.resource_use.compiler = CompilerObservation::Running(Compiler {
            name:  "sccache",
            count: 15,
        });
        let rows = [&first];
        let mut grid = FrameTableColumnGrid::of(&rows, TableKind::Command, ProcessTree::Long);
        grid.slots = vec![PID_COLUMN, COMMAND_COLUMN, COMPILER_COLUMN];
        let area = Rect::new(0, 0, 28, 20);
        let mut actual = Buffer::empty(area);
        frame_draw_process_table(
            &mut actual,
            area,
            &rows,
            Color::Reset,
            PinnedGroup::Unpinned,
            &grid,
        );
        grid.slots.pop();
        let mut expected = Buffer::empty(area);
        frame_draw_process_table(
            &mut expected,
            area,
            &rows,
            Color::Reset,
            PinnedGroup::Unpinned,
            &grid,
        );
        assert_eq!(buffer_rows(&actual), buffer_rows(&expected));
    }

    #[test]
    fn a_cut_table_reserves_its_mark_beside_whole_rightmost_values() {
        for compiler in [true, false] {
            let mut first = invocation(500, &["check"]);
            first.resource_use.compiler = CompilerObservation::Running(Compiler {
                name:  "sccache",
                count: 15,
            });
            first.managed = Measurement::Reading(1000);
            let mut second = first.clone();
            second.pid += 1;
            second.invocation_id = InvocationId::for_test(second.pid);
            let rows = [TrackedRow::from(first), TrackedRow::from(second)];
            let mut grid = FrameTableColumnGrid::of(
                &[&rows[0], &rows[1]],
                TableKind::Command,
                ProcessTree::Long,
            );
            grid.slots = if compiler {
                vec![PID_COLUMN, COMMAND_COLUMN, COMPILER_COLUMN]
            } else {
                vec![PID_COLUMN, COMMAND_COLUMN, MANAGED_COLUMN]
            };
            let value = if compiler { "sccache×15" } else { "1000" };
            for width in 24..=34 {
                let area = Rect::new(0, 0, width, 3);
                let mut buffer = Buffer::empty(area);
                frame_draw_process_table(
                    &mut buffer,
                    area,
                    &[&rows[0], &rows[1]],
                    Color::Reset,
                    PinnedGroup::Unpinned,
                    &grid,
                );
                let line = buffer_line(&buffer, 2);
                let header = buffer_line(&buffer, 0);
                if header.contains(if compiler { "compiler" } else { "runs" }) {
                    assert!(line.contains(value), "width {width}: {line}");
                    assert_eq!(
                        buffer[(cleared(area).right() - 1, 2)].symbol(),
                        ELISION,
                        "{line}"
                    );
                }
            }
        }
    }

    #[test]
    fn an_unavailable_row_keeps_its_own_absence_reason() {
        let mut parent = invocation(500, &["build"]);
        parent.resource_use.cpu = Measurement::Unavailable(MeasurementAbsence::ReadFailed);
        let mut child = invocation(501, &["check"]);
        child.parent = VisibleParent::Invocation {
            id:  parent.invocation_id.clone(),
            pid: parent.pid,
        };
        child.resource_use.cpu = Measurement::Unavailable(MeasurementAbsence::FirstObservation);
        let contributions = vec![
            InvocationContribution::from(&child),
            InvocationContribution::from(&parent),
        ];
        let row = TrackedRow::from(parent);
        let readings = row_resource_readings(&[&row], &contributions);
        assert_eq!(
            readings[0].resources.cpu,
            Measurement::Unavailable(MeasurementAbsence::ReadFailed)
        );
    }

    #[test]
    fn an_unavailable_contributor_leaves_the_row_reading_unavailable() {
        let mut parent = invocation(500, &["check"]);
        parent.resource_use.cpu = Measurement::Reading(10.0);
        let mut child = invocation(501, &["check"]);
        child.parent = VisibleParent::Invocation {
            id:  parent.invocation_id.clone(),
            pid: parent.pid,
        };
        child.resource_use.cpu = Measurement::Unavailable(MeasurementAbsence::ReadFailed);
        let roster = roster_of(parent, vec![child]);
        let buffer = reading_scope_buffer(&roster, false, 3);
        assert!(
            !buffer_line(&buffer, 2).contains("10%"),
            "{:?}",
            buffer_rows(&buffer)
        );
    }

    #[test]
    fn nested_unrowed_descendants_add_to_their_nearest_drawn_row() {
        let mut parent = invocation(500, &["check"]);
        parent.resource_use.cpu = Measurement::Reading(10.0);
        let mut child = invocation(501, &["check"]);
        child.resource_use.cpu = Measurement::Reading(20.0);
        child.parent = VisibleParent::Invocation {
            id:  parent.invocation_id.clone(),
            pid: parent.pid,
        };
        let mut nested = invocation(502, &["check"]);
        nested.resource_use.cpu = Measurement::Reading(30.0);
        nested.parent = VisibleParent::Invocation {
            id:  child.invocation_id.clone(),
            pid: child.pid,
        };
        let roster = roster_of(parent, vec![child, nested]);
        let buffer = reading_scope_buffer(&roster, false, 4);
        assert!(buffer_line(&buffer, 2).contains("10%"));
        assert!(
            buffer_line(&buffer, 3).contains("50%"),
            "{:?}",
            buffer_rows(&buffer)
        );
    }

    #[test]
    fn a_pid_reads_one_value_where_both_tables_draw_its_rows() {
        let roster = reading_scope_roster();
        let summary = reading_scope_buffer(&roster, true, 3);
        let cell = reading_scope_buffer(&roster, false, 3);
        for buffer in [&summary, &cell] {
            assert!(
                buffer_line(buffer, 2).contains("100%"),
                "{:?}",
                buffer_rows(buffer)
            );
            assert!(buffer_line(buffer, 2).contains("3.0G"));
        }
    }

    #[test]
    fn a_summary_row_reads_its_subtree_while_its_cell_reads_its_own_share() {
        let roster = reading_scope_roster();
        let summary = reading_scope_buffer(&roster, true, 8);
        let cell = reading_scope_buffer(&roster, false, 8);
        assert!(
            buffer_line(&summary, 2).contains("100%"),
            "{:?}",
            buffer_rows(&summary)
        );
        assert!(buffer_line(&cell, 2).contains("10%"));
        assert!(buffer_line(&cell, 3).contains("90%"));
    }

    #[test]
    fn a_driver_and_its_child_name_the_dispatched_pid_in_the_buffer() {
        let shim = [OsString::from("cargo"), "p20driver".into()];
        let driver = [OsString::from("/tools/cargo-p20driver"), "p20driver".into()];
        let child = [
            OsString::from("cargo-nextest"),
            "nextest".into(),
            "run".into(),
        ];
        let records = ProcessObservations::new(
            [
                (100, 1, "cargo", shim.as_slice()),
                (200, 100, "cargo-p20driver", driver.as_slice()),
                (300, 200, "cargo-nextest", child.as_slice()),
            ]
            .map(|(pid, parent, name, argv)| {
                let mut record = ProcessObservation::cargo(pid, argv);
                record.name = ProcessField::Observed(OsStr::new(name));
                record.parent = ProcessField::Observed(sysinfo::Pid::from_u32(parent));
                record
            }),
        );
        let mut roster = Roster::new();
        roster.observe(CensusSequence::default().sample(&records), Instant::now());
        let hidden = vec!["p20driver".to_string()];
        let area = Rect::new(0, 0, 160, 12);
        let mut buffer = Buffer::empty(area);
        draw_group(
            &mut buffer,
            &roster,
            &roster.groups()[0].id,
            area,
            Color::Reset,
            &hidden,
            ProcessTree::Long,
        );
        let lines = buffer_rows(&buffer);
        assert!(
            lines
                .iter()
                .any(|line| line.contains("200 cargo p20driver")),
            "{lines:#?}"
        );
        let line = lines
            .iter()
            .find(|line| line.contains("300") && line.contains("nextest"))
            .expect("child");
        assert!(line.contains("200"), "{line}");
    }

    #[test]
    fn a_parent_pid_without_a_table_row_uses_plain_ink() {
        let mut lead = invocation(500, &[PORT_SUBCOMMAND_NAME]);
        lead.managed = Measurement::Reading(1);
        let mut child = invocation(501, &["check"]);
        child.parent = VisibleParent::Invocation {
            id:  lead.invocation_id.clone(),
            pid: lead.pid,
        };
        let roster = roster_of(lead, vec![child]);
        let source = roster.groups()[0].rows().nth(1).expect("child row");
        assert!(matches!(source.parent_family(), ParentFamily::Member(_)));
        let area = Rect::new(0, 0, 160, 6);
        let mut buffer = Buffer::empty(area);
        draw_process_table(
            &mut buffer,
            area,
            &[source],
            TableKind::Command,
            Color::Reset,
            PinnedGroup::Unpinned,
            ProcessTree::Long,
        );
        let line = buffer_line(&buffer, 2);
        let x = u16::try_from(line.find("500").expect("parent pid")).unwrap();
        assert_eq!(buffer[(x, 2)].fg, text_default());
    }

    #[test]
    fn a_short_release_command_wraps_beside_its_summary_reading() {
        for (done, total) in [(661, 1000), (1000, 1000)] {
            let mut source = TrackedRow::from(invocation(123_456, &["build", "--release"]));
            source.process.state =
                CaptureLookup::Registered(CaptureRead::Progress(compiling(done, total)));
            let area = Rect::new(0, 0, 38, 8);
            let mut buffer = Buffer::empty(area);
            draw_process_table(
                &mut buffer,
                area,
                &[&source],
                TableKind::Summary,
                Color::Reset,
                PinnedGroup::Unpinned,
                ProcessTree::Short,
            );
            let lines = buffer_rows(&buffer);
            assert!(lines[0].contains("state"), "{lines:#?}");
            let first = &lines[2];
            assert!(first.contains("123456"), "{lines:#?}");
            if done == total {
                assert!(first.contains("100.0%"), "{lines:#?}");
                assert!(first.contains("[release] cargo"), "{lines:#?}");
                assert!(!first.contains("build"), "{lines:#?}");
                assert!(lines[3].trim().starts_with("build …"), "{lines:#?}");
            } else {
                assert!(first.contains("66.1%"), "{lines:#?}");
                assert!(first.contains("[release] cargo build …"), "{lines:#?}");
            }
        }
    }

    #[test]
    fn a_hidden_driver_keeps_the_parent_before_its_child_under_a_cut() {
        let driver = invocation(4100, &[PORT_SUBCOMMAND_NAME]);
        let mut parent = invocation(4200, &["nextest", "run"]);
        parent.parent = VisibleParent::Invocation {
            id:  driver.invocation_id.clone(),
            pid: driver.pid,
        };
        let mut child = invocation(4300, &["check"]);
        child.parent = VisibleParent::Invocation {
            id:  parent.invocation_id.clone(),
            pid: parent.pid,
        };
        let roster = roster_with_ancestry(driver, vec![child, parent], vec![]);
        for height in [3, 12] {
            let area = Rect::new(0, 0, 160, height);
            let mut buffer = Buffer::empty(area);
            draw_group(
                &mut buffer,
                &roster,
                &InvocationId::for_test(4100),
                area,
                Color::Reset,
                &hidden_when_idle(),
                ProcessTree::Long,
            );
            let lines = buffer_rows(&buffer);
            let parent_row = lines
                .iter()
                .position(|line| line.contains("cargo nextest run"));
            let child_row = lines.iter().position(|line| line.contains("cargo check"));
            assert!(parent_row.is_some(), "{height}: {lines:#?}");
            if height == 3 {
                assert!(child_row.is_none(), "{lines:#?}");
                let y = u16::try_from(parent_row.unwrap()).unwrap();
                assert_eq!(
                    buffer[(cell_width(SECTION_ITEM_INDENT), y)].fg,
                    text_default()
                );
            } else {
                assert!(
                    parent_row.unwrap() < child_row.expect("child is drawn"),
                    "{lines:#?}"
                );
            }
        }
    }

    #[test]
    fn a_long_short_view_runner_stays_on_one_drawn_row() {
        let roster = roster_with_ancestry(
            invocation(4100, &["check"]),
            vec![],
            vec![ancestor(
                1_234_567,
                "/opt/very_long_session_runner_executable_name",
            )],
        );
        let area = Rect::new(0, 0, 40, 20);
        let mut buffer = Buffer::empty(area);
        draw_group(
            &mut buffer,
            &roster,
            &InvocationId::for_test(4100),
            area,
            Color::Reset,
            &[],
            ProcessTree::Short,
        );
        let lines = buffer_rows(&buffer);
        let header = lines
            .iter()
            .position(|line| line.contains("pid"))
            .expect("table header");
        let ancestry = lines[..header]
            .iter()
            .filter(|line| !line.trim().is_empty())
            .collect::<Vec<_>>();
        assert_eq!(ancestry.len(), 1, "{lines:#?}");
        assert!(ancestry[0].contains("1234567"), "{lines:#?}");
        assert!(ancestry[0].trim_end().ends_with(ELISION), "{lines:#?}");
        let marker = area.width - CONTENT_RIGHT_INSET - 1;
        assert_eq!(buffer[(marker, 0)].symbol(), ELISION);
        assert_eq!(buffer[(marker, 0)].fg, cut_mark_ink(Color::Reset, 0));
        assert!(!ancestry[0].contains("/opt"), "{lines:#?}");
        let table_rows = frame_table_height(
            &roster.groups()[0].rows().collect::<Vec<_>>(),
            area.width,
            PinnedGroup::Lead(&roster.groups()[0].lead.process),
            &FrameColumnGrids::new(&roster, &[], ProcessTree::Short)
                .commands
                .0,
        );
        assert_eq!(
            group_height(&roster.groups()[0], area.width, &[], ProcessTree::Short),
            table_rows + 2
        );
    }

    #[test]
    fn a_narrow_inline_ancestry_mark_keeps_its_faded_ink() {
        let ancestors = vec![ancestor(1_234_567, "root"), ancestor(7_654_321, "runner")];
        for width in [12, 18, 30] {
            let area = Rect::new(0, 0, width, 2);
            let mut buffer = Buffer::empty(area);
            let ground = pane_background(true);
            draw_ancestry(
                &mut buffer,
                area,
                &ancestors,
                160,
                ground,
                AncestryFoot::Other,
                1,
            );
            let marker = (0..width)
                .find(|&x| buffer[(x, 0)].symbol() == ELISION)
                .expect("cut mark");
            assert_eq!(
                buffer[(marker, 0)].fg,
                cut_mark_ink(ground, 160),
                "width {width}"
            );
        }
    }

    #[test]
    fn a_narrow_summary_keeps_each_commands_progress_reading() {
        let mut first = invocation(123_456, &["build", "--release"]);
        first.state = CaptureLookup::Registered(CaptureRead::Progress(compiling(84, 100)));
        let mut second = invocation(234_567, &["build", "--release"]);
        second.state = CaptureLookup::Registered(CaptureRead::Progress(compiling(58, 100)));
        let mut roster = Roster::new();
        roster.observe(
            vec![cargo_group(first), cargo_group(second)],
            Instant::now(),
        );
        let sccache = SccacheStats::new();
        for tree in [ProcessTree::Short, ProcessTree::Long] {
            let area = Rect::new(0, 0, 46, 16);
            let cells = Cells::for_frame(&roster, &[], tree, &sccache, area.height);
            let mut buffer = Buffer::empty(area);
            cells.draw(&mut buffer, &TileContent::Summary, area, Color::Reset);
            let text = buffer_rows(&buffer).join("\n");
            assert_eq!(text.matches("84%").count(), 1, "{tree:?}: {text}");
            assert_eq!(text.matches("58%").count(), 1, "{tree:?}: {text}");
            for reading in ["84%", "58%"] {
                let (y, line) = buffer_rows(&buffer)
                    .into_iter()
                    .enumerate()
                    .find(|(_, line)| line.contains(reading))
                    .unwrap();
                let x = u16::try_from(line.find(reading).unwrap()).unwrap();
                assert_ne!(buffer[(x, u16::try_from(y).unwrap())].fg, Color::Reset);
            }
        }
    }

    #[test]
    fn a_cut_command_chain_leaves_the_summarys_foot_measured() {
        let mut group = cargo_group(invocation(500, &["check"]));
        group.ancestry = AncestryWalk::LimitCut(vec![ancestor(100, "claude")]);
        let mut roster = Roster::new();
        roster.observe(vec![group], Instant::now());
        let sccache = SccacheStats::new();
        for tree in [ProcessTree::Short, ProcessTree::Long] {
            let cells = Cells::new(&roster, &[], tree, &sccache);
            let area = Rect::new(0, 0, 160, 30);
            let (grid, buffer) = settled_buffer_with_cells(&cells, area);
            for placement in grid.placements(
                area,
                TileGrowth {
                    initial_rows:  12,
                    fill:          TileFill::Redistribute,
                    widen_summary: false,
                },
            ) {
                let inner = placement.frame.inner();
                let y = inner.bottom() - 1;
                let line: String = (inner.left()..inner.right())
                    .map(|x| buffer[(x, y)].symbol())
                    .collect();
                let byte = line.find("content rows: ").unwrap() + "content rows: ".len();
                let x = inner.left() + u16::try_from(line[..byte].chars().count()).unwrap();
                let expected = match placement.content {
                    TileContent::Summary => tui_pane::success_color(),
                    TileContent::Group(_) => tui_pane::error_color(),
                    TileContent::Empty(_) => continue,
                };
                assert_eq!(buffer[(x, y)].fg, expected, "{tree:?}: {line}");
            }
        }
    }

    #[test]
    fn two_commands_in_one_directory_show_their_own_progress() {
        let first = row(Some(compiling(84, 100)));
        let mut second = row(Some(compiling(58, 100)));
        second.process.pid += 1;
        let buffer = narrow_table_buffer(&[&first, &second], TableKind::Summary, 160, 5);
        assert!(!buffer_line(&buffer, 1).contains('%'));
        assert!(buffer_line(&buffer, 2).contains("84%"));
        assert!(buffer_line(&buffer, 3).contains("58%"));
    }

    #[test]
    fn one_visible_command_keeps_its_reading_when_the_other_cannot_fit_the_frame() {
        let mut first = invocation(500, &["check"]);
        first.state = CaptureLookup::Registered(CaptureRead::Progress(compiling(84, 100)));
        let mut second = invocation(501, &["check"]);
        second.state = CaptureLookup::Registered(CaptureRead::Progress(compiling(58, 100)));
        let mut roster = Roster::new();
        roster.observe(
            vec![cargo_group(first), cargo_group(second)],
            Instant::now(),
        );
        let sccache = SccacheStats::new();
        let cells = Cells::for_frame(&roster, &[], ProcessTree::Long, &sccache, 3);
        let area = Rect::new(0, 0, 160, 3);
        let mut buffer = Buffer::empty(area);
        cells.draw(&mut buffer, &TileContent::Summary, area, Color::Reset);
        assert!(!buffer_line(&buffer, 1).contains('%'));
        assert!(buffer_line(&buffer, 2).contains("84%"));
        assert!(!buffer_line(&buffer, 2).contains("58%"));
    }

    #[test]
    fn independent_command_cells_show_each_reading_once() {
        let mut first = invocation(500, &["check"]);
        first.state = CaptureLookup::Registered(CaptureRead::Progress(compiling(84, 100)));
        let mut second = invocation(501, &["check"]);
        second.state = CaptureLookup::Registered(CaptureRead::Progress(compiling(58, 100)));
        let mut roster = Roster::new();
        roster.observe(
            vec![cargo_group(first), cargo_group(second)],
            Instant::now(),
        );
        let sccache = SccacheStats::new();
        let cells = Cells::new(&roster, &[], ProcessTree::Long, &sccache);
        for (group, reading) in roster.groups().iter().zip(["84%", "58%"]) {
            let area = Rect::new(0, 0, 160, 6);
            let mut buffer = Buffer::empty(area);
            cells.draw(
                &mut buffer,
                &TileContent::Group(group.id.clone()),
                area,
                Color::Reset,
            );
            let text = buffer_rows(&buffer).join("\n");
            assert_eq!(text.matches(reading).count(), 1);
        }
    }

    #[test]
    fn a_chain_cut_by_the_walk_limit_opens_with_the_cut_mark() {
        for tree in [ProcessTree::Short, ProcessTree::Long] {
            for levels in [chain(6), vec![ancestor(3334, "claude")]] {
                let mut group = cargo_group(invocation(500, &["check"]));
                group.ancestry = AncestryWalk::LimitCut(levels);
                let mut roster = Roster::new();
                roster.observe(vec![group], Instant::now());
                let area = Rect::new(0, 0, 160, 20);
                let mut buffer = Buffer::empty(area);
                draw_group(
                    &mut buffer,
                    &roster,
                    &roster.groups()[0].id,
                    area,
                    Color::Reset,
                    &[],
                    tree,
                );
                assert!(buffer_line(&buffer, 0).trim().starts_with("… "));
            }
        }
    }

    #[test]
    fn a_cut_summary_has_no_state_slot_for_hidden_readings() {
        let visible = same_second("~/project", 123_456);
        let mut first_hidden = same_second("~/project", 234_567);
        first_hidden.process.state =
            CaptureLookup::Registered(CaptureRead::Progress(compiling(84, 100)));
        let mut second_hidden = same_second("~/project", 345_678);
        second_hidden.process.state =
            CaptureLookup::Registered(CaptureRead::Progress(compiling(58, 100)));
        let mut baseline_first = first_hidden.clone();
        baseline_first.process.state = CaptureLookup::Unregistered;
        let mut baseline_second = second_hidden.clone();
        baseline_second.process.state = CaptureLookup::Unregistered;
        let mut actual_headers = Vec::new();
        let mut expected_headers = Vec::new();
        for width in [200, 40] {
            let buffer = narrow_table_buffer(
                &[&visible, &first_hidden, &second_hidden],
                TableKind::Summary,
                width,
                3,
            );
            let baseline = narrow_table_buffer(
                &[&visible, &baseline_first, &baseline_second],
                TableKind::Summary,
                width,
                3,
            );
            actual_headers.push(buffer_line(&buffer, 0));
            expected_headers.push(buffer_line(&baseline, 0));
            assert!(!buffer_line(&buffer, 0).contains("state"));
            assert!(buffer_line(&buffer, 2).contains("123456"));
        }
        assert_eq!(actual_headers, expected_headers);
    }

    #[test]
    fn a_narrow_summary_titles_the_state_slot_of_its_drawn_reading() {
        let mut first = same_second("~/project", 123_456);
        first.process.command = CommandText::of("cargo", &["build", "--release"]);
        first.process.state = CaptureLookup::Registered(CaptureRead::Progress(compiling(84, 100)));
        let mut hidden = same_second("~/project", 234_567);
        hidden.process.command = first.process.command.clone();
        hidden.process.state = CaptureLookup::Registered(CaptureRead::Progress(compiling(58, 100)));
        let buffer = narrow_table_buffer(&[&first, &hidden], TableKind::Summary, 40, 3);
        let header = buffer_line(&buffer, 0);
        let value = buffer_line(&buffer, 2);
        let x = u16::try_from(header.find("state").expect("state title")).unwrap();
        assert!(value.contains("84%"), "{value}");
        assert_eq!(buffer[(x, 0)].fg, label_color());
        let reading = u16::try_from(value.find("84%").unwrap()).unwrap();
        assert_eq!(buffer[(reading, 2)].fg, success_color());
    }

    fn seven_digit_progress_summary(width: u16, tree: ProcessTree) -> Buffer {
        let mut first = invocation(1_727_353, &["build"]);
        first.state = CaptureLookup::Registered(CaptureRead::Progress(compiling(7, 100)));
        let mut second = invocation(1_727_354, &["check"]);
        second.state = CaptureLookup::Registered(CaptureRead::Progress(compiling(65, 100)));
        let third = invocation(1_727_355, &["test"]);
        let mut roster = Roster::new();
        roster.observe(
            vec![cargo_group(first), cargo_group(second), cargo_group(third)],
            Instant::now(),
        );
        let sccache = SccacheStats::new();
        let area = Rect::new(1, 1, width - 2, 8);
        let cells = Cells::for_frame(&roster, &[], tree, &sccache, area.height);
        let mut buffer = Buffer::empty(Rect::new(0, 0, width, 10));
        cells.draw(&mut buffer, &TileContent::Summary, area, Color::Reset);
        buffer
    }

    #[test]
    fn a_sixteen_column_summary_identifies_every_row_by_pid() {
        for tree in [ProcessTree::Short, ProcessTree::Long] {
            let buffer = seven_digit_progress_summary(16, tree);
            let header = buffer_line(&buffer, 1);
            assert_eq!(header.trim(), "pid", "{tree:?}: {header}");
            let x = u16::try_from(header.find("pid").expect("pid title")).unwrap();
            assert_eq!(buffer[(x, 1)].fg, label_color());
            let lines = buffer_rows(&buffer);
            for pid in ["1727353", "1727354", "1727355"] {
                assert_eq!(
                    lines.iter().filter(|line| line.contains(pid)).count(),
                    1,
                    "{tree:?}: {lines:#?}"
                );
            }
        }
    }

    #[test]
    fn a_summary_keeps_pid_at_the_reading_column_boundary() {
        for tree in [ProcessTree::Short, ProcessTree::Long] {
            for width in 11..=17 {
                let buffer = seven_digit_progress_summary(width, tree);
                let header = buffer_line(&buffer, 1);
                assert!(header.contains("pid"), "{tree:?} at {width}: {header}");
                assert_eq!(header.contains("state"), width == 17, "{header}");
                let lines = buffer_rows(&buffer);
                for pid in ["1727353", "1727354", "1727355"] {
                    let y = lines
                        .iter()
                        .position(|line| line.contains(pid))
                        .expect("whole pid");
                    if width == 17 {
                        let reading = match pid {
                            "1727353" => "7%",
                            "1727354" => "65%",
                            _ => continue,
                        };
                        let x =
                            u16::try_from(lines[y].find(reading).expect("own reading")).unwrap();
                        assert_eq!(buffer[(x, u16::try_from(y).unwrap())].fg, success_color());
                    } else {
                        assert!(!lines[y].contains('%'), "{}", lines[y]);
                    }
                }
            }
        }
    }

    #[test]
    fn a_cut_ancestrys_root_stays_put_and_children_start_after_their_parent() {
        let ancestors: Vec<_> = (0..6)
            .map(|level| ancestor(1_000_000 + level, "runner"))
            .collect();
        for budget in 2..=4 {
            let area = Rect::new(0, 0, 80, budget);
            let mut cut = Buffer::empty(area);
            draw_ancestry(
                &mut cut,
                area,
                &ancestors,
                160,
                pane_background(true),
                AncestryFoot::Other,
                0,
            );
            let whole_area = Rect::new(0, 0, 80, budget + 1);
            let mut whole = Buffer::empty(whole_area);
            draw_ancestry(
                &mut whole,
                whole_area,
                &ancestors[..usize::from(budget)],
                160,
                pane_background(true),
                AncestryFoot::Other,
                0,
            );
            let pid_column = |buffer: &Buffer, y| {
                (0..area.width)
                    .find(|&x| {
                        buffer[(x, y)]
                            .symbol()
                            .chars()
                            .any(|ch| ch.is_ascii_digit())
                    })
                    .expect("ancestor pid")
            };
            assert_eq!(pid_column(&cut, 0), pid_column(&whole, 0));
            assert_eq!(pid_column(&cut, 0), cell_width(SECTION_HEADER_INDENT));
            for y in 1..budget {
                let first = (0..area.width)
                    .find(|&x| cut[(x, y)].symbol() != " ")
                    .expect("ancestor level");
                assert!(first > pid_column(&cut, y - 1), "{}", buffer_line(&cut, y));
                if cut[(first, y)].symbol() == ELISION {
                    assert_eq!(pid_column(&cut, y), first + cell_width(ELISION) + 1);
                    assert_eq!(cut[(first, y)].fg, cut_mark_ink(pane_background(true), 160));
                }
            }
        }
    }

    #[test]
    fn a_cut_ancestry_keeps_each_pids_depth_and_inline_mark() {
        let ancestors: Vec<_> = (0..6)
            .map(|level| ancestor(1_000_000 + level, "runner"))
            .collect();
        for budget in 2..=4 {
            let area = Rect::new(0, 0, 80, budget);
            let mut buffer = Buffer::empty(area);
            assert_eq!(
                draw_ancestry(
                    &mut buffer,
                    area,
                    &ancestors,
                    160,
                    pane_background(true),
                    AncestryFoot::Other,
                    0,
                ),
                budget
            );
            let mut previous = None;
            for y in 0..budget {
                let line = buffer_line(&buffer, y);
                let x = (0..area.width)
                    .find(|&x| {
                        buffer[(x, y)]
                            .symbol()
                            .chars()
                            .any(|ch| ch.is_ascii_digit())
                    })
                    .expect("ancestor pid");
                if let Some(parent_x) = previous {
                    let mark_cells = if line.contains(ELISION) {
                        cell_width(ELISION) + 1
                    } else {
                        0
                    };
                    assert_eq!(
                        x,
                        parent_x + cell_width(ANCESTRY_LEVEL_INDENT) + mark_cells,
                        "{line}"
                    );
                }
                if line.contains(ELISION) {
                    assert_eq!(buffer[(x - 2, y)].symbol(), ELISION);
                    assert_eq!(buffer[(x - 1, y)].symbol(), " ");
                    assert_eq!(
                        buffer[(x - 2, y)].fg,
                        cut_mark_ink(pane_background(true), 160)
                    );
                }
                previous = Some(x);
            }
        }
    }

    #[test]
    fn a_marked_ancestrys_wrapped_lines_keep_their_commands_indent() {
        let walk = AncestryWalk::LimitCut(vec![
            ancestor(100, &"root ".repeat(8)),
            ancestor(200, &"middle ".repeat(8)),
            ancestor(300, "nearest"),
        ]);
        let source = AncestrySource::from(&walk);
        let area = Rect::new(0, 0, 30, 20);
        let plan = source.plan(area, 0);
        let expected = source.height(&plan.levels, area.width);
        let mut buffer = Buffer::empty(area);
        let used = draw_ancestry_source(
            &mut buffer,
            area,
            source,
            160,
            pane_background(true),
            AncestryFoot::Other,
            0,
        );
        assert_eq!(usize::from(used), expected + usize::from(plan.gap));
        let mut command_start = 0;
        for y in 0..u16::try_from(expected).unwrap() {
            let first = (0..area.width)
                .find(|&x| buffer[(x, y)].symbol() != " ")
                .unwrap();
            let pid = ["100", "200", "300"].into_iter().find_map(|pid| {
                let line = buffer_line(&buffer, y);
                line.find(pid)
                    .map(|byte| u16::try_from(line[..byte].chars().count()).unwrap())
            });
            if let Some(pid_start) = pid {
                command_start = pid_start + 4;
            } else {
                assert_eq!(first, command_start);
                assert_eq!(
                    buffer[(first, y)].fg,
                    cut_mark_ink(pane_background(true), 160)
                );
            }
        }
    }

    #[test]
    fn a_summarys_foot_counts_content_without_surrendered_separators() {
        let roster = directory_rich_roster(&[1, 1, 1, 1]);
        let sccache = SccacheStats::new();
        for (height, ink) in [(12, success_color()), (11, tui_pane::error_color())] {
            let area = Rect::new(0, 0, 162, height);
            let cells = Cells::new(&roster, &[], ProcessTree::Long, &sccache);
            let mut grid = TileGrid::new();
            grid.set_view(TileView::Summary);
            let growth = TileGrowth {
                initial_rows:  12,
                fill:          TileFill::Redistribute,
                widen_summary: false,
            };
            let mut buffer = Buffer::empty(area);
            tui_pane::draw_tile_grid(
                &mut buffer,
                &mut grid,
                area,
                growth,
                TileGridContents::Shown,
                &cells,
            );
            grid.settle_for_test();
            buffer = Buffer::empty(area);
            tui_pane::draw_tile_grid(
                &mut buffer,
                &mut grid,
                area,
                growth,
                TileGridContents::Shown,
                &cells,
            );
            let y = height - 2;
            let foot = buffer_line(&buffer, y);
            let byte = foot.find("content rows: ").expect("summary foot") + "content rows: ".len();
            let x = u16::try_from(foot[..byte].chars().count()).unwrap();
            assert_eq!(buffer[(x, y)].fg, ink, "{foot}");
            let content = buffer_rows(&buffer)[..usize::from(y)].join("\n");
            if height == 12 {
                for pid in [4100, 4200, 4300, 4400] {
                    assert!(content.contains(&pid.to_string()), "{content}");
                }
                assert!(!content.contains(ELISION), "{content}");
            } else {
                assert!(content.contains(ELISION), "{content}");
            }
        }
    }

    #[test]
    fn a_summary_keeps_only_the_latest_separators_that_fit() {
        let rows: Vec<_> = (0..4)
            .map(|index| same_second(&format!("~/project-{index}"), 500 + index))
            .collect();
        let refs: Vec<_> = rows.iter().collect();
        for (height, headings) in [
            (11, vec![1, 3, 6, 9]),
            (10, vec![1, 3, 5, 8]),
            (12, vec![1, 4, 7, 10]),
        ] {
            let area = Rect::new(0, 0, 162, height);
            let mut grid = FrameTableColumnGrid::of(&refs, TableKind::Summary, ProcessTree::Long);
            grid.separation = DirectorySeparation::YieldEarliest;
            let mut buffer = Buffer::empty(area);
            frame_draw_process_table(
                &mut buffer,
                area,
                &refs,
                Color::Reset,
                PinnedGroup::Unpinned,
                &grid,
            );
            for (index, y) in headings.into_iter().enumerate() {
                assert!(
                    buffer_line(&buffer, y).contains(&format!("project-{index}")),
                    "{:?}",
                    buffer_rows(&buffer)
                );
            }
        }
    }

    #[test]
    fn the_summary_gives_its_spare_row_to_a_cell_hiding_an_ancestry_level() {
        let roster = tiled_roster(3, &chain(6));
        let area = Rect::new(0, 0, 40, 28);
        let (grid, buffer) = settled_cells_buffer(&roster, area);
        assert_eq!(
            blank_rows_beside_hidden_content(&roster, &grid, &buffer, area),
            Vec::<TileContent>::new()
        );
    }

    #[test]
    fn a_summary_can_lend_separators_without_losing_any_process() {
        let roster = directory_rich_roster(&[1, 6, 1, 1]);
        let sccache = SccacheStats::new();
        let cells = Cells::new(&roster, &[], ProcessTree::Long, &sccache);
        for (height, drawn) in [(12, 12), (11, 11), (10, 10), (9, 9)] {
            let use_rows = cells.row_use(&TileContent::Summary, Rect::new(0, 0, 160, height));
            assert_eq!(use_rows.drawn, drawn);
            assert_eq!(use_rows.kept, 9);
        }
    }

    #[test]
    fn cpu_and_mem_are_sized_for_the_machine() {
        let widths = SessionColumnWidths::for_machine(16, 128 * BYTES_PER_GIBIBYTE);
        assert_eq!(widths.cpu, cell_width("1600%"));
        assert_eq!(widths.memory, cell_width("128.0G"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn processor_capacity_excludes_the_aggregate_and_unrelated_statistics() {
        assert_eq!(
            processor_rows("cpu 1 2 3\ncpu0 1 2 3\ncpu7 1 2 3\nintr 4\nprocesses 8\ncpu_other 9\n"),
            2
        );
        assert_eq!(processor_rows("cpu 1 2 3\nintr 4\n"), 0);
    }

    #[test]
    fn the_short_ancestry_links_only_a_parent_it_draws() {
        for shell_between in [false, true] {
            let mut lead = invocation(500, &["check"]);
            lead.parent = VisibleParent::Ancestor(if shell_between { 200 } else { 100 });
            let mut ancestors = vec![ancestor(100, "claude")];
            if shell_between {
                ancestors.push(shell(200, "bash"));
            }
            let roster = roster_with_ancestry(lead, vec![], ancestors);
            let sccache = SccacheStats::new();
            for tree in [ProcessTree::Short, ProcessTree::Long] {
                let cells = Cells::new(&roster, &[], tree, &sccache);
                let group = &roster.groups()[0];
                let area = Rect::new(0, 0, 160, 10);
                let mut buffer = Buffer::empty(area);
                cells.draw(
                    &mut buffer,
                    &TileContent::Group(group.id.clone()),
                    area,
                    Color::Reset,
                );
                let rows = buffer_rows(&buffer);
                let header_y = rows
                    .iter()
                    .position(|line| line.contains("pid") && line.contains("command"))
                    .unwrap();
                let row = &rows[header_y + 2];
                if tree == ProcessTree::Short && shell_between {
                    assert!(!row.contains("200"), "{rows:#?}");
                } else {
                    assert!(
                        row.contains(if shell_between { "200" } else { "100" }),
                        "{rows:#?}"
                    );
                }
            }
        }
    }

    #[test]
    fn a_walk_cut_chain_reads_cut_in_its_foot() {
        let now = Instant::now();
        let mut group = cargo_group(invocation(500, &["check"]));
        group.ancestry = AncestryWalk::LimitCut(vec![ancestor(100, "claude")]);
        let mut roster = Roster::new();
        roster.observe(vec![group], now);
        roster.observe(vec![], now);
        roster.advance(
            now + std::time::Duration::from_secs(1),
            std::time::Duration::from_secs(4),
        );
        let sccache = SccacheStats::new();
        let area = Rect::new(0, 0, 160, 30);
        let growth = TileGrowth {
            initial_rows:  12,
            fill:          TileFill::Redistribute,
            widen_summary: false,
        };
        for tree in [ProcessTree::Short, ProcessTree::Long] {
            let cells = Cells::new(&roster, &[], tree, &sccache);
            let (mut grid, _) = settled_buffer_with_cells(&cells, area);
            for focus in [1, 2] {
                grid.focus_cell(focus);
                grid.settle_for_test();
                let mut buffer = Buffer::empty(area);
                tui_pane::draw_tile_grid(
                    &mut buffer,
                    &mut grid,
                    area,
                    growth,
                    TileGridContents::Shown,
                    &cells,
                );
                for placement in grid.placements(area, growth) {
                    if !placement.frame.is_focused() {
                        continue;
                    }
                    let inner = placement.frame.inner();
                    let y = inner.bottom() - 1;
                    let line: String = (inner.left()..inner.right())
                        .map(|x| buffer[(x, y)].symbol())
                        .collect();
                    let byte = line.find("content rows: ").unwrap() + "content rows: ".len();
                    let x = inner.left() + u16::try_from(line[..byte].chars().count()).unwrap();
                    let CellReadoutFade::Fading(progress) = cells.readout_fade(&placement.content)
                    else {
                        panic!("finished cell fades")
                    };
                    assert_eq!(
                        buffer[(x, y)].fg,
                        blend_color(
                            match placement.content {
                                TileContent::Group(_) => tui_pane::error_color(),
                                TileContent::Summary | TileContent::Empty(_) => {
                                    tui_pane::success_color()
                                },
                            },
                            pane_background(true),
                            progress.amount()
                        )
                    );
                    if matches!(placement.content, TileContent::Group(_)) {
                        assert!(placement_text(&buffer, &placement).contains("… 100 claude"));
                    }
                }
            }
        }
    }

    #[test]
    fn a_summed_memory_reading_widens_once_and_never_narrows() {
        let mut widths = SessionColumnWidths::for_machine(16, 16 * BYTES_PER_GIBIBYTE);
        let initial = widths.memory;
        for bytes in [100 * BYTES_PER_GIBIBYTE, BYTES_PER_GIBIBYTE] {
            let mut process = invocation(500, &["check"]);
            process.resource_use.memory = Measurement::Reading(bytes);
            widths.observe(&process);
            assert_eq!(widths.memory, initial + 1);
        }
    }

    #[test]
    fn the_short_ancestry_is_the_nearest_session_runner() {
        for (chain, expected) in [
            (
                vec![
                    ancestor(519_409, "/nix/store/runner/Runner.Listener"),
                    ancestor(
                        1_556_662,
                        "/nix/store/runner/Runner.Worker spawnclient 166 175",
                    ),
                    shell(1_558_245, "/nix/store/bash/bin/bash"),
                ],
                "1556662 Runner.Worker",
            ),
            (vec![ancestor(11, "claude"), shell(12, "bash")], "11 claude"),
            (
                vec![
                    ancestor(11, "ghostty"),
                    ancestor(13, "/bin/env X=1"),
                    ancestor(14, "/bin/nohup cargo"),
                    shell(12, "zsh"),
                ],
                "11 ghostty",
            ),
        ] {
            let roster = roster_with_ancestry(invocation(20, &["check"]), vec![], chain);
            let ancestors = drawn_ancestry(&roster.groups()[0], false, ProcessTree::Short);
            assert_eq!(ancestors.as_slice().len(), 1);
            assert_eq!(
                format!(
                    "{} {}",
                    ancestors.as_slice()[0].pid,
                    ancestors.as_slice()[0].command
                ),
                expected
            );
        }
    }

    #[test]
    fn the_long_ancestry_keeps_every_level() {
        let chain = vec![
            ancestor(519_409, "/nix/store/runner/Runner.Listener"),
            ancestor(
                1_556_662,
                "/nix/store/runner/Runner.Worker spawnclient 166 175",
            ),
            shell(1_558_245, "/nix/store/bash/bin/bash"),
        ];
        let roster = roster_with_ancestry(invocation(20, &["check"]), vec![], chain.clone());
        assert_eq!(
            drawn_ancestry(&roster.groups()[0], false, ProcessTree::Long).as_slice(),
            chain.as_slice()
        );
    }

    #[test]
    fn a_short_view_program_is_named_by_its_file_name() {
        let row = TrackedRow::from(CargoProcess {
            command: CommandText::of("/nix/store/cargo/bin/cargo", &["check", "--workspace"]),
            ..invocation(20, &["check"])
        });
        let area = Rect::new(0, 0, 160, 5);
        let mut buffer = Buffer::empty(area);
        draw_process_table(
            &mut buffer,
            area,
            &[&row],
            TableKind::Command,
            Color::Reset,
            PinnedGroup::Unpinned,
            ProcessTree::Short,
        );
        assert!(buffer_line(&buffer, 2).contains("cargo check …"));
        assert!(!buffer_rows(&buffer).join("\n").contains("/nix/store"));
        assert_eq!(
            shortened(&ancestor(12, "/nix/store/runner/Runner.Worker spawnclient")).command,
            "Runner.Worker"
        );
    }

    #[test]
    fn a_value_changing_length_moves_no_column() {
        let mut process = invocation(20, &["check"]);
        let mut roster = Roster::for_test(SessionColumnWidths::for_machine(
            16,
            16 * BYTES_PER_GIBIBYTE,
        ));
        let mut headers = Vec::new();
        for (cpu, memory) in [
            ("765%", 9 * BYTES_PER_GIBIBYTE),
            ("1017%", 10 * BYTES_PER_GIBIBYTE),
            ("9%", BYTES_PER_GIBIBYTE),
        ] {
            process.resource_use.cpu =
                Measurement::Reading(cpu.trim_end_matches('%').parse::<f32>().unwrap());
            process.resource_use.memory = Measurement::Reading(memory);
            roster.observe(vec![cargo_group(process.clone())], Instant::now());
            let grids = FrameColumnGrids::new(&roster, &[], ProcessTree::Long);
            let rows = [&roster.groups()[0].lead];
            let mut frame_headers = Vec::new();
            for grid in [&grids.summary.0, &grids.commands.0] {
                let area = Rect::new(0, 0, 200, 5);
                let mut buffer = Buffer::empty(area);
                frame_draw_process_table(
                    &mut buffer,
                    area,
                    &rows,
                    Color::Reset,
                    PinnedGroup::Unpinned,
                    grid,
                );
                let header = buffer_line(&buffer, 0);
                frame_headers.push(
                    TABLE_HEADERS
                        .iter()
                        .map(|name| header.find(name))
                        .collect::<Vec<_>>(),
                );
            }
            headers.push(frame_headers);
        }
        assert_eq!(headers[0], headers[1]);
        assert_eq!(headers[1], headers[2]);
    }

    #[test]
    fn single_and_many_row_frames_reserve_the_same_command_mark_cell() {
        let first = tight_command_row(500, 400, &["check"]);
        let second = tight_command_row(501, 400, &["check"]);
        let mut headers = Vec::new();
        for rows in [vec![&first], vec![&first, &second]] {
            for height in [3, 6] {
                headers.push(buffer_line(
                    &narrow_table_buffer(&rows, TableKind::Command, 160, height),
                    0,
                ));
            }
        }
        assert!(headers.iter().all(|header| header == &headers[0]));
    }

    #[test]
    fn a_summary_sizes_columns_without_the_command_cells_parent_slot() {
        let row = tight_command_row(500, 400, &["check"]);
        let buffer = narrow_table_buffer(&[&row], TableKind::Summary, 30, 4);
        let header = buffer_line(&buffer, 0);
        assert!(header.contains("cpu"), "{header}");
        assert!(header.contains("mem"), "{header}");
    }

    #[test]
    fn a_wide_short_summary_keeps_measurements_beside_nextest() {
        let roster = roster_of(
            invocation(
                500,
                &[
                    "nextest",
                    "run",
                    "--test-threads",
                    "32",
                    "--bin",
                    "hana",
                    "-E",
                    "package(hana) & (test(arrange_glyph) | …)",
                ],
            ),
            Vec::new(),
        );
        let buffer = summary_alone_buffer_in_tree(&roster, 198, ProcessTree::Short);
        assert_eq!(buffer.area.width, 200);
        let header_y = (0..buffer.area.height)
            .find(|&y| buffer_line(&buffer, y).contains("command"))
            .expect("the summary has a table header");
        let header = buffer_line(&buffer, header_y);
        for label in ["start", "dur", "cpu", "mem"] {
            let x = header
                .find(label)
                .expect("the measurement column is visible");
            let x = u16::try_from(header[..x].chars().count()).expect("header fits the buffer");
            assert_eq!(
                buffer[(x, header_y)].fg,
                blend_color(label_color(), pane_background(false), 0)
            );
        }
        let command_y = (header_y + 1..buffer.area.height)
            .find(|&y| buffer_line(&buffer, y).contains("cargo nextest"))
            .expect("the summary draws its command");
        let command = buffer_line(&buffer, command_y);
        let byte_x = command.find("cargo").expect("the command names cargo");
        let x = u16::try_from(command[..byte_x].chars().count()).expect("command fits the buffer");
        let name = "cargo nextest run …";
        let drawn: String = (x..x + cell_width(name))
            .map(|column| buffer[(column, command_y)].symbol())
            .collect();
        assert_eq!(drawn, name);
        assert_eq!(buffer[(x, command_y)].fg, text_default());
        assert_eq!(buffer[(x + 6, command_y)].fg, success_color());
        assert_eq!(buffer[(x + cell_width(name), command_y)].symbol(), " ");
        assert!(
            !command.contains("32") && !command.contains("hana") && !command.contains("package(")
        );
        assert_eq!(
            (0..buffer.area.height)
                .filter(|&y| buffer_line(&buffer, y).contains("nextest"))
                .count(),
            1
        );
    }

    #[test]
    fn a_summary_row_past_the_frames_height_sizes_no_column() {
        let mut first = invocation(500, &["check"]);
        first.path = "~/first".to_string();
        let mut last = invocation(501, &["twentycharacternamexx"]);
        last.path = "~/last".to_string();
        last.directory_identity = WorkingDirectoryIdentity::Absolute("/last".into());
        let mut roster = Roster::new();
        roster.observe(vec![cargo_group(first), cargo_group(last)], Instant::now());
        let area = Rect::new(0, 0, 40, 3);
        let mut buffer = Buffer::empty(area);
        draw_summary(
            &mut buffer,
            &roster,
            area,
            Color::Reset,
            &[],
            ProcessTree::Long,
        );
        let header = buffer_line(&buffer, 0);
        assert!(header.contains("cpu"), "{header}");
        assert!(header.contains("mem"), "{header}");
    }

    #[test]
    fn a_summary_row_inside_the_frames_height_still_sizes_its_columns() {
        let mut first = invocation(500, &["check"]);
        first.path = "~/first".to_string();
        let mut last = invocation(501, &["twentycharacternamexx"]);
        last.path = "~/last".to_string();
        last.directory_identity = WorkingDirectoryIdentity::Absolute("/last".into());
        let mut roster = Roster::for_test(SessionColumnWidths::for_machine(
            16,
            16 * BYTES_PER_GIBIBYTE,
        ));
        roster.observe(vec![cargo_group(first), cargo_group(last)], Instant::now());
        let area = Rect::new(0, 0, 40, 5);
        let mut buffer = Buffer::empty(area);
        draw_summary(
            &mut buffer,
            &roster,
            area,
            Color::Reset,
            &[],
            ProcessTree::Long,
        );
        assert!(!buffer_line(&buffer, 0).contains("cpu"));
        assert!(!buffer_line(&buffer, 0).contains("mem"));
    }

    #[test]
    fn long_rows_keep_profile_arguments_beside_their_label() {
        for (arguments, label, full) in [
            (
                vec!["build", "--release", "--locked"],
                "[release]",
                "build --release --locked",
            ),
            (
                vec!["build", "--profile", "p19c", "--config", "x"],
                "[p19c]",
                "build --profile p19c --config x",
            ),
            (
                vec!["build", "--profile=p19c"],
                "[p19c]",
                "build --profile=p19c",
            ),
        ] {
            let row = TrackedRow::from(invocation(500, &arguments));
            for tree in [ProcessTree::Long, ProcessTree::Short] {
                for kind in [TableKind::Summary, TableKind::Command] {
                    let area = Rect::new(0, 0, 200, 5);
                    let mut buffer = Buffer::empty(area);
                    draw_process_table(
                        &mut buffer,
                        area,
                        &[&row],
                        kind,
                        Color::Reset,
                        PinnedGroup::Unpinned,
                        tree,
                    );
                    let line = buffer_line(&buffer, 2);
                    let name = match tree {
                        ProcessTree::Long => format!("{label} cargo {full}"),
                        ProcessTree::Short => format!("{label} cargo build …"),
                    };
                    let x = u16::try_from(line.find(label).expect("the row has its profile label"))
                        .expect("the row fits the buffer");
                    let drawn: String = (x..x + cell_width(&name))
                        .map(|column| buffer[(column, 2)].symbol())
                        .collect();
                    assert_eq!(drawn, name, "{tree:?}/{kind:?}");
                    assert_eq!(
                        buffer[(x, 2)].fg,
                        cut_mark_style(Color::Reset, 0)
                            .fg
                            .expect("the mark has ink")
                    );
                    let program_x = x + cell_width(label) + 1;
                    assert_eq!(buffer[(program_x, 2)].fg, text_default());
                    assert_eq!(buffer[(program_x + 6, 2)].fg, success_color());
                    assert_eq!(buffer[(x + cell_width(&name), 2)].symbol(), " ");
                }
            }
        }
    }

    #[test]
    fn default_and_child_profiles_have_no_profile_label() {
        for arguments in [
            vec!["run", "--", "--release"],
            vec!["run", "--", "-r"],
            vec!["build", "--profile", "dev"],
            vec!["build"],
        ] {
            let row = TrackedRow::from(invocation(500, &arguments));
            for tree in [ProcessTree::Short, ProcessTree::Long] {
                for kind in [TableKind::Summary, TableKind::Command] {
                    let area = Rect::new(0, 0, 160, 5);
                    let mut buffer = Buffer::empty(area);
                    draw_process_table(
                        &mut buffer,
                        area,
                        &[&row],
                        kind,
                        Color::Reset,
                        PinnedGroup::Unpinned,
                        tree,
                    );
                    let command = buffer_line(&buffer, 2);
                    assert!(!command.contains("[release]") && !command.contains("[dev]"));
                    match tree {
                        ProcessTree::Short => assert!(!command.contains("--profile")),
                        ProcessTree::Long => assert!(command.contains(&arguments.join(" "))),
                    }
                }
            }
        }
    }

    #[test]
    fn a_profile_stays_on_the_visible_line_of_a_cut_command() {
        let row = TrackedRow::from(invocation(
            500,
            &[
                "build",
                "--release",
                "--features",
                "one/two/three/four/five/six/seven/eight",
            ],
        ));
        for tree in [ProcessTree::Short, ProcessTree::Long] {
            for kind in [TableKind::Summary, TableKind::Command] {
                let area = Rect::new(0, 0, 40, 3);
                let mut buffer = Buffer::empty(area);
                draw_process_table(
                    &mut buffer,
                    area,
                    &[&row],
                    kind,
                    Color::Reset,
                    PinnedGroup::Unpinned,
                    tree,
                );
                let command = buffer_line(&buffer, 2);
                assert!(command.contains("[release]"), "{command}");
                assert!(command.contains(ELISION), "{command}");
            }
        }
    }

    #[test]
    fn a_driver_keeps_its_profile_at_the_foot_of_its_chain() {
        let roster = roster_of(
            invocation(500, &[PORT_SUBCOMMAND_NAME, "--release"]),
            vec![invocation(501, &["build"])],
        );
        let sccache = SccacheStats::new();
        let hidden = hidden_when_idle();
        for tree in [ProcessTree::Short, ProcessTree::Long] {
            let cells = Cells::new(&roster, &hidden, tree, &sccache);
            let area = Rect::new(0, 0, 160, 16);
            let mut buffer = Buffer::empty(area);
            cells.draw(
                &mut buffer,
                &TileContent::Group(roster.groups()[0].id.clone()),
                area,
                Color::Reset,
            );
            assert_eq!(
                buffer_rows(&buffer).join("\n").matches("[release]").count(),
                1
            );
        }
    }

    #[test]
    fn cargo_profiles_survive_a_cut_command_in_both_views() {
        for arguments in [
            vec!["build", "--release"],
            vec!["build", "-r"],
            vec!["build", "--profile", "release"],
            vec!["build", "--profile=release"],
            vec!["build", "--profile", "ci"],
        ] {
            let profile = if arguments.contains(&"ci") {
                "[ci]"
            } else {
                "[release]"
            };
            let row = TrackedRow::from(invocation(500, &arguments));
            for tree in [ProcessTree::Short, ProcessTree::Long] {
                for kind in [TableKind::Summary, TableKind::Command] {
                    let area = Rect::new(0, 0, 40, 3);
                    let mut buffer = Buffer::empty(area);
                    draw_process_table(
                        &mut buffer,
                        area,
                        &[&row],
                        kind,
                        Color::Reset,
                        PinnedGroup::Unpinned,
                        tree,
                    );
                    assert!(
                        buffer_line(&buffer, 2).contains(profile),
                        "{kind:?}/{tree:?}: {:?}",
                        buffer_line(&buffer, 2)
                    );
                }
            }
        }
    }

    #[test]
    fn a_heading_does_not_repeat_its_cells_progress() {
        let first = row(Some(compiling(998, 1000)));
        let mut enclosed = row_at("~/second", Some(compiling(998, 1000)));
        enclosed.process.provenance = RowProvenance::Enclosing(capture_context());
        let buffer = narrow_table_buffer(&[&first, &enclosed], TableKind::Command, 160, 8);
        assert_eq!(buffer_rows(&buffer).join("\n").matches("99.8%").count(), 1);
    }

    #[test]
    fn an_ancestry_cut_is_marked_inline_at_every_budget() {
        let ancestors = chain(6);
        for budget in 1..=4 {
            let levels = ancestry_fit(&ancestors, budget, |levels| ancestry_height(levels, 80));
            assert_eq!(levels.len(), budget);
            if budget >= 2 {
                assert_eq!(drawn(&levels)[0], Some(0));
            }
            let lines: Vec<_> = levels
                .iter()
                .enumerate()
                .flat_map(|(depth, level)| {
                    ancestry_lines(
                        *level,
                        depth,
                        80,
                        text_default(),
                        secondary_text_color(),
                        &AncestryIndentation::of(&levels),
                    )
                })
                .collect();
            let marked: Vec<_> = lines
                .iter()
                .filter(|line| line.to_string().contains(ELISION))
                .collect();
            assert_eq!(marked.len(), 1);
            assert!(
                marked[0]
                    .to_string()
                    .trim()
                    .starts_with(&format!("{ELISION} "))
            );
            assert_ne!(marked[0].to_string().trim(), ELISION);
        }
    }

    #[test]
    fn a_two_row_ancestry_keeps_the_wrapped_root_and_nearest_pid() {
        let ancestry = vec![ancestor(100, &"root ".repeat(20)), ancestor(200, "nearest")];
        let area = Rect::new(0, 0, 40, 5);
        let mut buffer = Buffer::empty(area);
        draw_ancestry(
            &mut buffer,
            area,
            &ancestry,
            0,
            Color::Reset,
            AncestryFoot::Other,
            3,
        );
        assert!(buffer_line(&buffer, 0).contains("100"));
        assert!(buffer_line(&buffer, 0).ends_with(ELISION));
        assert!(buffer_line(&buffer, 1).contains("200 nearest"));
    }

    #[test]
    fn the_ancestry_mark_takes_its_rows_faded_ink() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 80, 2));
        let area = buffer.area;
        draw_ancestry(
            &mut buffer,
            area,
            &chain(6),
            160,
            pane_background(true),
            AncestryFoot::Other,
            1,
        );
        let marker = (0..area.width)
            .find(|&x| buffer[(x, 0)].symbol() == ELISION)
            .unwrap();
        assert_eq!(
            buffer[(marker, 0)].fg,
            blend_color(secondary_text_color(), pane_background(true), 160)
        );
    }

    #[test]
    fn a_cut_ancestry_gives_its_gap_row_to_one_more_level() {
        let ancestors = chain(6);
        for available in 1..=3 {
            let area = Rect::new(0, 0, 80, available + 3);
            let mut buffer = Buffer::empty(area);
            let used = draw_ancestry(
                &mut buffer,
                area,
                &ancestors,
                0,
                pane_background(false),
                AncestryFoot::Other,
                3,
            );
            assert_eq!(used, available);
            assert_ne!(buffer_line(&buffer, used - 1), "");
        }
    }

    #[test]
    fn ancestry_fitting_measures_every_retained_form_in_rows() {
        for (heights, budget, expected) in [
            (
                vec![2, 3, 1, 1, 1],
                6,
                vec![Some(0), Some(2), Some(3), Some(4)],
            ),
            (
                vec![1, 1, 2, 1, 1],
                5,
                vec![Some(0), Some(2), Some(3), Some(4)],
            ),
        ] {
            let ancestors = chain(5);
            let levels = ancestry_fit(&ancestors, budget, |levels| {
                drawn(levels)
                    .iter()
                    .map(|pid| pid.map_or(1, |pid| heights[usize::try_from(pid).unwrap()]))
                    .sum()
            });
            assert_eq!(drawn(&levels), expected);
        }
        for (commands, budget, expected_height) in [
            (
                vec![
                    "root ".repeat(7),
                    "middle ".repeat(9),
                    "leaf".to_string(),
                    "leaf".to_string(),
                    "leaf".to_string(),
                ],
                6,
                5,
            ),
            (
                vec![
                    "root".to_string(),
                    "middle".to_string(),
                    "wrapped ".repeat(5),
                    "leaf".to_string(),
                    "leaf".to_string(),
                ],
                5,
                5,
            ),
        ] {
            let ancestry: Vec<_> = commands
                .iter()
                .enumerate()
                .map(|(pid, command)| ancestor(u32::try_from(pid).unwrap(), command))
                .collect();
            let levels = ancestry_fit(&ancestry, budget, |levels| ancestry_height(levels, 30));
            assert_eq!(drawn(&levels), vec![Some(0), Some(2), Some(3), Some(4)]);
            assert_eq!(ancestry_height(&levels, 30), expected_height);
        }
    }

    #[test]
    fn a_heading_drops_its_tag_before_cutting_its_path() {
        for (path, expected) in [
            (
                "~/rust/tool-based-ui-frame-time",
                "~/rust/tool-based-ui-frame-time",
            ),
            (
                "~/very-long-parent/tool-based-ui-geometry-material",
                "…/tool-based-ui-geometry-material",
            ),
        ] {
            let mut row = row_at(path, None);
            row.process.provenance = RowProvenance::Direct(capture_context());
            let rows = [&row];
            let groups = group_by_path(&rows, PinnedGroup::Unpinned);
            assert_eq!(groups[0].fitted_heading(38), expected);
        }
    }

    #[test]
    fn a_heading_that_fits_without_its_tag_is_drawn_whole() {
        let mut row = row_at("~/rust/tool-based-ui-frame-time", None);
        row.process.provenance = RowProvenance::Direct(capture_context());
        let rows = [&row];
        assert_eq!(
            group_by_path(&rows, PinnedGroup::Unpinned)[0].fitted_heading(38),
            row.process.path
        );
    }

    #[test]
    fn a_cell_cut_to_one_row_shows_its_own_command() {
        let lead = invocation(500, &["nextest", "run"]);
        let mut child = invocation(400, &["check"]);
        child.parent = VisibleParent::Invocation {
            id:  lead.invocation_id.clone(),
            pid: lead.pid,
        };
        let roster = roster_with_ancestry(lead, vec![child], vec![]);
        let area = Rect::new(0, 0, 160, 3);
        let mut buffer = Buffer::empty(area);
        draw_contents(
            &mut buffer,
            &roster,
            &TileContent::Group(roster.groups()[0].id.clone()),
            area,
            Color::Reset,
            &[],
            ProcessTree::Long,
        );
        assert!(buffer_line(&buffer, 2).contains("500"));
        assert!(buffer_line(&buffer, 2).contains("cargo nextest run"));
    }

    #[test]
    fn a_parent_precedes_its_child_inside_an_unpinned_directory() {
        let lead = row_at("~/lead", None);
        let mut parent = same_second("~/other", 500);
        parent.process.started = RunStart::Known(200);
        let mut child = same_second("~/other", 400);
        child.process.parent = VisibleParent::Invocation {
            id:  parent.process.invocation_id.clone(),
            pid: parent.process.pid,
        };
        let groups = group_by_path(&[&lead, &child, &parent], PinnedGroup::Lead(&lead.process));
        assert_eq!(
            groups[1]
                .rows
                .iter()
                .map(|row| row.process.pid)
                .collect::<Vec<_>>(),
            [500, 400]
        );
    }

    #[test]
    fn the_hidden_rows_mark_is_drawn_in_the_marks_ink() {
        let first = row(None);
        let second = same_second(GAUGE_PATH, 500);
        let buffer = narrow_table_buffer(&[&first, &second], TableKind::Command, 100, 3);
        let x = (0..buffer.area.width)
            .find(|&x| buffer[(x, 2)].symbol() == ELISION)
            .unwrap();
        assert_eq!(buffer[(x, 2)].fg, secondary_text_color());
    }

    #[test]
    fn an_ordinary_short_command_keeps_nextest_and_its_action_in_ten_cells() {
        let mut row = row(None);
        row.process.command = CommandText::of("cargo", &["nextest", "run", "--workspace"]);
        let rows = [&row];
        let area = Rect::new(0, 0, 40, 10);
        let mut layout = CellTableLayout::of(
            &rows,
            TableKind::Command,
            area,
            Color::Reset,
            ProcessTree::Short,
        );
        layout.columns = vec![COMMAND_COLUMN];
        layout.column_widths = vec![10];
        let values = ProcessRowValues::of(&row, layout.detail, layout.tree);
        let command = command_cell(&row, &layout, values.arguments, Style::default());
        let mut buffer = Buffer::empty(Rect::new(0, 0, 10, 3));
        let area = buffer.area;
        Paragraph::new(command).render(area, &mut buffer);
        assert_eq!(buffer_line(&buffer, 0).trim_end(), "cargo");
        assert_eq!(buffer_line(&buffer, 1).trim_end(), "nextest");
        assert_eq!(buffer_line(&buffer, 2).trim_end(), "run …");
    }

    #[test]
    fn a_whole_short_word_leaves_its_argument_mark_on_the_next_line() {
        let name = CommandText::of("abcdefghijklmnop", &["--argument"]).named();
        let text = wrapped_short_command(vec![Span::raw("abcdefghijklmnop")], 16, name.arguments);
        let mut buffer = Buffer::empty(Rect::new(0, 0, 16, 2));
        let area = buffer.area;
        Paragraph::new(text).render(area, &mut buffer);
        assert_eq!(buffer_line(&buffer, 0), "abcdefghijklmnop");
        assert_eq!(
            buffer_line(&buffer, 1),
            crate::constants::ARGUMENT_OMISSION_MARK
        );
        assert_eq!(buffer[(0, 1)].symbol(), " ");
    }

    #[test]
    fn the_summary_title_has_a_blank_cell_after_it() {
        let buffer = summary_alone_buffer(&Roster::new(), 38);
        assert!(buffer_line(&buffer, 0).starts_with("┌ summary ─"));
    }

    #[test]
    fn a_parent_pid_loses_its_family_colour_while_its_child_row_is_cut() {
        let mut lead = invocation(500, &["build"]);
        lead.managed = Measurement::Reading(1);
        let mut child = invocation(501, &["check"]);
        child.parent = VisibleParent::Invocation {
            id:  lead.invocation_id.clone(),
            pid: 500,
        };
        let roster = roster_of(lead, vec![child]);
        let group = &roster.groups()[0];
        let rows: Vec<_> = group.rows().collect();
        for (height, ink) in [
            (3, text_default()),
            (
                4,
                match group.lead.family() {
                    FamilyHead::Heads(index) => theme::family_color(index),
                    FamilyHead::NoChildren => panic!("family"),
                },
            ),
        ] {
            let buffer = narrow_table_buffer(&rows, TableKind::Command, 160, height);
            let x = u16::try_from(buffer_line(&buffer, 2).find("500").unwrap()).unwrap();
            assert_eq!(buffer[(x, 2)].fg, ink);
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
        let layout = CellTableLayout::of(
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
        row.process.resource_use.cpu = Measurement::Reading(1177.0);
        row.process.resource_use.memory = Measurement::Reading(BYTES_PER_GIBIBYTE * 2);
        row.process.resource_use.compiler = CompilerObservation::Running(Compiler {
            name:  COMPILER_PROCESS_NAMES[0],
            count: 1,
        });
        row.process.managed = Measurement::Reading(1);
        row.process.command = CommandText::of("cargo", arguments);
        row
    }

    #[test]
    fn stacked_command_cells_share_one_column_grid() {
        let mut narrow = invocation(41, &["check", "--features", "one,two,three,four,five,six"]);
        narrow.resource_use.compiler = CompilerObservation::Running(Compiler {
            name:  COMPILER_PROCESS_NAMES[1],
            count: 1,
        });
        narrow.managed = Measurement::Reading(1);
        let mut wide = invocation(1_234_567, &["check"]);
        wide.resource_use.cpu = Measurement::Reading(1192.0);
        wide.resource_use.compiler = CompilerObservation::Running(Compiler {
            name:  COMPILER_PROCESS_NAMES[0],
            count: 123,
        });
        wide.managed = Measurement::Reading(123);
        let mut roster = Roster::new();
        roster.observe(vec![cargo_group(narrow), cargo_group(wide)], Instant::now());
        let sccache = SccacheStats::new();
        let cells = Cells::new(&roster, &[], ProcessTree::Long, &sccache);
        for width in [64, 200] {
            let mut buffer = Buffer::empty(Rect::new(0, 0, width, 8));
            for (index, group) in roster.groups().iter().enumerate() {
                cells.draw(
                    &mut buffer,
                    &TileContent::Group(group.id.clone()),
                    Rect::new(0, u16::try_from(index).unwrap() * 4, width, 4),
                    Color::Reset,
                );
            }
            let widths: Vec<_> = roster
                .groups()
                .iter()
                .map(|group| (TileContent::Group(group.id.clone()), width))
                .collect();
            let demands = cells.demands(&widths);
            for demand in &demands.groups {
                let area = Rect::new(0, 0, width, 32);
                let mut whole = Buffer::empty(area);
                let content = TileContent::Group(demand.id.clone());
                cells.draw(&mut whole, &content, area, Color::Reset);
                assert_eq!(filled_rows(&whole), demand.rows);
                assert_eq!(usize::from(cells.rows_drawn(&content, area)), demand.rows);
            }
            let first = buffer_line(&buffer, 0);
            let second = buffer_line(&buffer, 4);
            for header in TABLE_HEADERS {
                assert_eq!(
                    first.find(header),
                    second.find(header),
                    "{width}: {first:?}, {second:?}"
                );
            }
        }
    }

    #[test]
    fn compiler_and_runs_follow_the_command_column() {
        let row = tight_command_row(359_829, 359_762, &["check"]);
        let rows = [&row];
        let area = Rect::new(0, 0, 200, 4);
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
        let header = buffer_line(&buffer, 0);
        let command = header.find("command").unwrap();
        let compiler = header.find("compiler").unwrap();
        assert_eq!(
            compiler - command,
            "cargo check".len()
                + usize::from(TABLE_CONTINUATION_CLEARANCE)
                + usize::from(TABLE_COLUMN_SPACING),
            "{header:?}"
        );
        assert_eq!(
            header.find("runs").unwrap() - compiler,
            usize::from(cell_width(&format!("{}×1", COMPILER_PROCESS_NAMES[0])))
                + usize::from(TABLE_COLUMN_SPACING)
        );
    }

    #[test]
    fn the_longest_command_keeps_room_for_the_hidden_rows_mark() {
        let mut lead = tight_command_row(359_829, 359_762, &["check"]);
        lead.process.path = "/workspace/project".to_string();
        let mut later = lead.clone();
        later.process.pid += 1;
        later.process.invocation_id = InvocationId::for_test(later.process.pid);
        let rows = [&lead, &later];
        let area = Rect::new(0, 0, 200, 3);
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
        let header = buffer_line(&buffer, 0);
        let command = header.find("command").unwrap();
        let compiler = header.find("compiler").unwrap();
        let line = buffer_line(&buffer, 2);
        assert_eq!(&line[command..command + "cargo check".len()], "cargo check");
        assert_eq!(
            buffer[(u16::try_from(command + "cargo check".len()).unwrap(), 2)].symbol(),
            " "
        );
        let marker = area.right() - CONTENT_RIGHT_INSET - 1;
        assert_eq!(buffer[(marker, 2)].symbol(), ELISION);
        assert!(usize::from(marker) > compiler);
    }

    #[test]
    fn no_content_row_touches_the_right_frame() {
        let mut lead = invocation(
            4100,
            &["check", "--features", "one,two,three,four,five,six"],
        );
        lead.state = CaptureLookup::Registered(CaptureRead::Progress(compiling(998, 1000)));
        let mut roster = Roster::new();
        roster.observe(
            vec![
                CargoGroup {
                    lead,
                    rest: vec![invocation(4101, &["check"])],
                    ancestry: crate::census::scan::AncestryWalk::Ended(vec![ancestor(
                        40,
                        "agent with a very long command line that wraps across the cell",
                    )]),
                },
                cargo_group(invocation(4200, &["check"])),
            ],
            Instant::now(),
        );
        let sccache = SccacheStats::new();
        let cells = Cells::new(&roster, &[], ProcessTree::Long, &sccache);
        for width in [38, 62, 198] {
            for content in [
                TileContent::Summary,
                TileContent::Group(roster.groups()[0].id.clone()),
            ] {
                for height in [3, 4, 8, 16] {
                    let area = Rect::new(0, 0, width, height);
                    let mut buffer = Buffer::empty(area);
                    cells.draw(&mut buffer, &content, area, Color::Reset);
                    for y in area.top()..area.bottom() {
                        assert_eq!(
                            buffer[(area.right() - 1, y)].symbol(),
                            " ",
                            "{width}, {content:?}, row {y}: {:?}",
                            buffer_line(&buffer, y)
                        );
                    }
                    if height == 3 {
                        assert!(
                            buffer_rows(&buffer)
                                .iter()
                                .any(|line| line.contains(ELISION))
                        );
                    }
                    if height == 16 && matches!(content, TileContent::Group(_)) {
                        assert!(
                            buffer_rows(&buffer)
                                .iter()
                                .any(|line| line.contains("agent"))
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn a_column_whose_only_value_is_hidden_draws_no_header() {
        let lead = same_second("/workspace/project", 4100);
        let mut later = same_second("/workspace/project", 4101);
        later.process.managed = Measurement::Reading(1);
        later.process.resource_use.compiler = CompilerObservation::Unknown;
        later.process.state = CaptureLookup::Registered(CaptureRead::Progress(RunState::Blocked));
        let mut visible = same_second("/workspace/visible", 4200);
        visible.process.managed = Measurement::Reading(1);
        visible.process.resource_use.compiler = CompilerObservation::Unknown;
        visible.process.state = CaptureLookup::Registered(CaptureRead::Progress(RunState::Blocked));
        let mut roster = Roster::new();
        roster.observe(
            vec![
                CargoGroup {
                    lead:     lead.process,
                    rest:     vec![later.process],
                    ancestry: crate::census::scan::AncestryWalk::Ended(Vec::new()),
                },
                cargo_group(visible.process),
            ],
            Instant::now(),
        );
        let sccache = SccacheStats::new();
        let cells = Cells::new(&roster, &[], ProcessTree::Long, &sccache);
        let mut buffer = Buffer::empty(Rect::new(0, 0, 200, 8));
        for (index, group) in roster.groups().iter().enumerate() {
            cells.draw(
                &mut buffer,
                &TileContent::Group(group.id.clone()),
                Rect::new(0, u16::try_from(index).unwrap() * 4, 200, 3),
                Color::Reset,
            );
        }
        let hidden = buffer_line(&buffer, 0);
        let visible = buffer_line(&buffer, 4);
        assert!(!hidden.contains("state"), "{hidden:?}");
        assert!(visible.contains("state"), "{visible:?}");
        for title in ["pid", "parent", "start", "dur", "cpu", "mem"] {
            assert_eq!(
                hidden.find(title),
                visible.find(title),
                "{hidden:?} {visible:?}"
            );
        }
        for column in [COMPILER_COLUMN, MANAGED_COLUMN] {
            assert!(!hidden.contains(TABLE_HEADERS[column]), "{hidden:?}");
            if let Some(slot) = visible.find(TABLE_HEADERS[column]) {
                for offset in 0..TABLE_HEADERS[column].len() {
                    assert_eq!(
                        buffer[(u16::try_from(slot + offset).unwrap(), 0)].symbol(),
                        " "
                    );
                }
            } else {
                assert_eq!(column, COMPILER_COLUMN);
            }
        }
        for column in [
            PID_COLUMN,
            PARENT_COLUMN,
            START_COLUMN,
            DURATION_COLUMN,
            CPU_COLUMN,
            MEMORY_COLUMN,
        ] {
            assert_eq!(
                hidden.find(TABLE_HEADERS[column]),
                visible.find(TABLE_HEADERS[column])
            );
        }
        assert_eq!(
            hidden.find("command").expect("hidden command"),
            visible.find("command").expect("visible command")
        );
        assert!(buffer_line(&buffer, 2).contains("4100"));
        assert!(!buffer_line(&buffer, 2).contains("4101"));
    }

    #[test]
    fn columns_drop_before_a_command_name_wraps() {
        let mut row = row(None);
        row.process.command = CommandText::of("cargo", &["clippy"]);
        row.process.pid = 1_359_829;

        for cpu in ["12%", "123456789012345678901234567890%"] {
            row.process.resource_use.cpu =
                Measurement::Reading(cpu.trim_end_matches('%').parse::<f32>().unwrap());
            let rows = [&row];
            for width in [38, 46] {
                let mut headers = Vec::new();
                for kind in [TableKind::Summary, TableKind::Command] {
                    let area = Rect::new(0, 0, width, 6);
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
                    assert!(
                        buffer_line(&buffer, 2).contains("cargo clippy"),
                        "{width}: {:?}",
                        buffer_rows(&buffer)
                    );
                    assert_eq!(buffer_line(&buffer, 3), "");
                    headers.push(buffer_line(&buffer, 0));
                }
                for column in [
                    PID_COLUMN,
                    START_COLUMN,
                    DURATION_COLUMN,
                    CPU_COLUMN,
                    MEMORY_COLUMN,
                    COMMAND_COLUMN,
                ] {
                    assert_eq!(
                        headers[0].contains(TABLE_HEADERS[column]),
                        headers[1].contains(TABLE_HEADERS[column]),
                        "{width}, {cpu}, {headers:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn a_whole_command_keeps_a_blank_cell_before_the_hidden_rows_mark() {
        let mut lead = same_second("/workspace/project", 410);
        lead.process.command = CommandText::of("cargo", &["check"]);
        let mut later = lead.clone();
        later.process.pid = 411;
        later.process.invocation_id = InvocationId::for_test(411);
        let rows = [&lead, &later];
        let area = Rect::new(0, 0, 38, 3);
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
        let line = buffer_line(&buffer, 2);
        let command = line.find("cargo check").unwrap();
        let marker = line.find(ELISION).unwrap();
        assert!(marker > command + "cargo check".len(), "{line:?}");
        assert!(
            line[command + "cargo check".len()..marker]
                .chars()
                .all(|character| character == ' ')
        );
        assert_eq!(
            marker,
            usize::from(area.right() - CONTENT_RIGHT_INSET - 1),
            "{line:?}"
        );
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
    fn table_column_text(
        buffer: &Buffer,
        layout: &CellTableLayout,
        column: usize,
        y: u16,
    ) -> String {
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
        assert_eq!(summary_foot_text(memory_total(&roster).foot()), "");
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
            (Measurement::Unavailable(MeasurementAbsence::Unproven), ""),
            (
                Measurement::Reading(BYTES_PER_GIBIBYTE * 1234 / 100),
                "12.3G",
            ),
        ] {
            let mut row = row(None);
            row.process.resource_use.memory = memory;

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
            row.process.resource_use.cpu = census::scan::aggregate(&shares, std::iter::once(pid));

            for kind in [TableKind::Command, TableKind::Summary] {
                let buffer = measurement_row_buffer(&row, kind);

                assert_eq!(
                    cpu_cell_text(&buffer, TABLE_HEADER_HEIGHT + GROUP_HEADER_HEIGHT),
                    "",
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
            row.process.resource_use.cpu = census::scan::aggregate(&shares, std::iter::once(pid));

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
        build.process.resource_use.memory = Measurement::Reading(322_122_547);
        let mut test = row(None);
        test.process.pid = 41_234;
        test.process.invocation_id = InvocationId::for_test(41_234);
        test.process.start = "11:05".to_string();
        test.process.duration = "00:08".to_string();
        test.process.resource_use.cpu = Measurement::Reading(5.0);
        test.process.resource_use.memory = Measurement::Reading(BYTES_PER_GIBIBYTE * 124 / 10);
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
    fn an_unknown_compiler_observation_keeps_its_cell_empty() {
        let mut row = row(None);
        row.process.resource_use.compiler = CompilerObservation::Unknown;

        let text = measurement_row_text(&row, TableKind::Command);

        assert!(text.ends_with("cargo build"), "{text:?}");
        assert_eq!(
            usize::from(cell_width(
                &ProcessRowValues::of(&row, SummaryDetail::Full, ProcessTree::Long).cells
                    [COMPILER_COLUMN]
            )),
            0
        );
    }

    #[test]
    fn an_unknown_managed_count_keeps_its_cell_empty() {
        let mut row = row(None);
        row.process.managed = Measurement::Unavailable(MeasurementAbsence::Unproven);

        let text = measurement_row_text(&row, TableKind::Command);

        assert!(text.ends_with("cargo build"), "{text:?}");
    }

    /// A registration cannot supply process measurements in either table layout.
    #[test]
    fn all_unproven_measurements_render_in_command_and_summary_views() {
        let mut row = row(None);
        row.process.resource_use.cpu = Measurement::Unavailable(MeasurementAbsence::Unproven);
        row.process.resource_use.compiler = CompilerObservation::Unknown;
        row.process.managed = Measurement::Unavailable(MeasurementAbsence::Unproven);

        for kind in [TableKind::Command, TableKind::Summary] {
            let buffer = measurement_row_buffer(&row, kind);
            let header = buffer_line(&buffer, 0);
            let text = buffer_line(&buffer, TABLE_HEADER_HEIGHT + GROUP_HEADER_HEIGHT);
            for column in [CPU_COLUMN, MEMORY_COLUMN, COMPILER_COLUMN, MANAGED_COLUMN] {
                let Some(start) = header.find(TABLE_HEADERS[column]) else {
                    assert!(matches!(column, COMPILER_COLUMN | MANAGED_COLUMN));
                    continue;
                };
                let end = TABLE_HEADERS[column + 1..]
                    .iter()
                    .filter_map(|label| header.find(label))
                    .min()
                    .unwrap_or(text.len());
                let expected = "";
                assert_eq!(text[start..end].trim(), expected, "{kind:?}");
            }
        }
    }

    #[test]
    fn observed_idle_compilers_and_zero_managed_runs_keep_their_cells_empty() {
        let row = row(None);

        let text = measurement_row_text(&row, TableKind::Command);

        assert!(text.ends_with("cargo build"), "{text:?}");
        assert_eq!(
            usize::from(cell_width(
                &ProcessRowValues::of(&row, SummaryDetail::Full, ProcessTree::Long).cells
                    [COMPILER_COLUMN]
            )),
            0
        );
        assert_eq!(managed_text(&row.process), "");
    }

    #[test]
    fn observed_compilers_and_managed_runs_keep_their_counts() {
        let mut row = row(None);
        let driver = COMPILER_PROCESS_NAMES[0];
        row.process.resource_use.compiler = CompilerObservation::Running(Compiler {
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
            path:                 path.to_string(),
            directory_identity:   WorkingDirectoryIdentity::Absolute(
                Path::new("/test-home").join(path),
            ),
            pid:                  41233,
            invocation_id:        InvocationId::for_test(41233),
            capture_membership:   CaptureMembership::Outside,
            provenance:           RowProvenance::Uncaptured(ProcessOwner::Unavailable),
            parent:               VisibleParent::None,
            start:                "11:04".to_string(),
            started:              RunStart::Known(started),
            duration:             "00:18".to_string(),
            resource_use:         InvocationResourceUse {
                cpu:      Measurement::Reading(12.0),
                memory:   Measurement::Unavailable(MeasurementAbsence::Unproven),
                compiler: CompilerObservation::None,
            },
            subtree_resource_use: InvocationResourceUse {
                cpu:      Measurement::Reading(12.0),
                memory:   Measurement::Unavailable(MeasurementAbsence::Unproven),
                compiler: CompilerObservation::None,
            },
            state:                state.map_or(CaptureLookup::Unregistered, |state| {
                CaptureLookup::Registered(CaptureRead::Progress(state))
            }),
            managed:              Measurement::Reading(0),
            nested:               false,
            command:              CommandText::of("cargo", &["build"]),
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
            vec![Some(0), Some(3), Some(4), Some(5)],
        );
    }

    /// Under three rows there is no room for two ends and a separate
    /// elision between them, so the cut and the nearest ancestor survive.
    #[test]
    fn a_short_ancestry_keeps_its_cut_mark_and_nearest_ancestor() {
        let ancestry = chain(6);
        assert_eq!(
            drawn(&ancestry_levels(&ancestry, 2)),
            vec![Some(0), Some(5)]
        );
        assert_eq!(drawn(&ancestry_levels(&ancestry, 1)), vec![Some(5)]);
    }

    /// One available ancestry row combines the cut mark with the nearest
    /// ancestor rather than showing a bare root and dropping the leaf.
    #[test]
    fn one_ancestry_row_draws_the_marked_nearest_ancestor() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 24, 2));
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

        assert_eq!(used, 1, "the cut ancestry yields its gap");
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
            &AncestryIndentation::WholeChain,
        );
        let text: String = lines[0]
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect();

        assert_eq!(ancestry_room(&ancestor, 0, width), 7);
        assert_eq!(
            ancestry_rows(
                AncestryLevel::Ancestor(&ancestor),
                0,
                width,
                &AncestryIndentation::WholeChain
            ),
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
                    indent_width
                        .saturating_add(cell_width(pid))
                        .saturating_add(CONTENT_RIGHT_INSET)
                        <= width,
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
            format!("      ~/.claude/lo{ELISION}")
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
            resource_use: InvocationResourceUse {
                cpu:      Measurement::Reading(12.0),
                memory:   Measurement::Unavailable(MeasurementAbsence::Unproven),
                compiler: CompilerObservation::None,
            },
            subtree_resource_use: InvocationResourceUse {
                cpu:      Measurement::Reading(12.0),
                memory:   Measurement::Unavailable(MeasurementAbsence::Unproven),
                compiler: CompilerObservation::None,
            },
            state: CaptureLookup::Unregistered,
            managed: Measurement::Reading(0),
            nested: false,
            command: CommandText::of("cargo", arguments),
        }
    }

    /// A process with the group-total memory used by summary-foot tests.
    fn memory_invocation(pid: u32, memory: Measurement<u64>) -> CargoProcess {
        let mut process = invocation(pid, &["build"]);
        process.resource_use.memory = memory;
        process.subtree_resource_use.memory = memory;
        process
    }

    /// One group with `lead` and no child invocations.
    fn cargo_group(lead: CargoProcess) -> CargoGroup {
        CargoGroup {
            lead,
            rest: Vec::new(),
            ancestry: AncestryWalk::Ended(Vec::new()),
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
        work.resource_use.memory = Measurement::Reading(BYTES_PER_GIBIBYTE);
        work.subtree_resource_use.memory = Measurement::Reading(BYTES_PER_GIBIBYTE * 1234 / 100);
        let lead = invocation(4100, &[PORT_SUBCOMMAND_NAME]);

        let mut nested = invocation(4300, &["check"]);
        nested.parent = VisibleParent::Invocation {
            id:  work.invocation_id.clone(),
            pid: work.pid,
        };
        nested.resource_use.memory =
            Measurement::Reading(BYTES_PER_GIBIBYTE * 1234 / 100 - BYTES_PER_GIBIBYTE);
        nested.nested = true;
        let buffer = measurement_group_buffer(lead, vec![work, nested], TableKind::Summary);

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
        lead.resource_use.cpu = census::scan::aggregate(&shares, members.into_iter());
        let rest = members[1..]
            .iter()
            .map(|pid| {
                let mut child = invocation(pid.as_u32(), &["build"]);
                child.resource_use.cpu = shares[pid];
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
                        "",
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
                ancestry: crate::census::scan::AncestryWalk::Ended(ancestry),
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
            vec![Some(6218), Some(24101)],
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

        assert_eq!(used, 2, "the root and marked nearest level");
        assert_eq!(buffer_line(&buffer, 0), " 0 sh");
        assert_eq!(buffer_line(&buffer, 1), "  … 23 uv run");
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
        assert!(drawn.contains("18581 node …"), "{drawn}");
        assert!(
            !drawn.contains("6218") && !drawn.contains("24101"),
            "{drawn}"
        );
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

        assert_eq!(buffer_line(&buffer, 0), " 596633 sh");
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

        let pinned = group_by_path(&rows, PinnedGroup::Lead(&lead.process));

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
        let groups = group_by_path(&rows, PinnedGroup::Lead(&current.process));
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
        registration.process.resource_use.cpu =
            Measurement::Unavailable(MeasurementAbsence::Unproven);
        registration.process.resource_use.compiler = CompilerObservation::Unknown;
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
        process.process.resource_use.cpu = Measurement::Reading(12.0);
        process.process.resource_use.compiler = CompilerObservation::None;
        process.process.managed = Measurement::Reading(0);
        let pinned = PinnedGroup::Lead(&registration.process);
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
        assert_eq!(group.fitted_heading(24), format!("{ELISION}/component"));
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
            format!("{ELISION}ool-based-ui-frame-time")
        );

        let row = row_at("/very-long-parent/tool-based-ui-frame-time", None);
        let rows = [&row];
        let groups = group_by_path(&rows, PinnedGroup::Unpinned);
        assert_eq!(
            groups[0].fitted_heading(25),
            format!("{ELISION}ool-based-ui-frame-time")
        );
    }

    #[test]
    fn a_wide_directory_tail_fits_its_cell_budget() {
        for (path, room) in [
            (format!("/{}", "界".repeat(6)), 10),
            (format!("/{}AB", "界".repeat(17)), 36),
        ] {
            let row = row_at(&path, None);
            let rows = [&row];
            let groups = group_by_path(&rows, PinnedGroup::Unpinned);
            let tail = groups[0].tail_heading("", room);
            assert!(tail.starts_with(ELISION), "{tail:?}");
            assert!(cell_width(&tail) <= room, "{room}: {tail:?}");
            assert!(path.ends_with(tail.strip_prefix(ELISION).unwrap()));
        }
    }

    #[test]
    fn a_wide_directory_heading_keeps_its_marked_tail_and_right_clearance() {
        let path = format!("/{}AB", "界".repeat(17));
        let row = row_at(&path, None);
        let rows = [&row];
        let groups = group_by_path(&rows, PinnedGroup::Unpinned);
        let area = Rect::new(0, 0, 38, 6);
        let room = cleared(area)
            .width
            .saturating_sub(cell_width(SECTION_HEADER_INDENT));
        let fitted = groups[0].fitted_heading(room);
        for kind in [TableKind::Summary, TableKind::Command] {
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
            let mut heading = String::new();
            let mut x = area.left();
            while x < area.right() {
                let symbol = buffer[(x, TABLE_HEADER_HEIGHT)].symbol();
                heading.push_str(symbol);
                x = x.saturating_add(cell_width(symbol).max(1));
            }
            let heading = heading.trim_end();
            assert_eq!(heading.trim(), fitted, "{kind:?}: {heading:?}");
            assert!(heading.contains(ELISION), "{heading:?}");
            assert!(heading.ends_with("AB"), "{heading:?}");
            assert_eq!(
                buffer[(area.right() - 1, TABLE_HEADER_HEIGHT)].symbol(),
                " "
            );
        }
    }

    #[test]
    fn a_cut_line_keeps_wide_graphemes_within_its_cell_budget() {
        let text = "❤\u{fe0f}".repeat(6);
        let mut line = Line::from(text);
        mark_cut_line(&mut line, 10);
        assert!(UnicodeWidthStr::width(&line) <= 10, "{line:?}");
        assert!(line.to_string().ends_with(ELISION));
    }

    #[test]
    fn summary_elision_keeps_wide_graphemes_within_its_cell_budget() {
        let text = "❤\u{fe0f}".repeat(6);
        let fitted = elide_summary_end(&text, 10);
        assert!(cell_width(&fitted) <= 10, "{fitted:?}");
        assert!(fitted.ends_with(ELISION));
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

    #[test]
    fn an_unreadable_directory_heading_keeps_only_its_account() {
        let mut row = owned(
            crate::constants::UNRESOLVED_PATH,
            40,
            1000,
            AccountName::Resolved("hana-linux-2".into()),
        );
        row.process.directory_identity = WorkingDirectoryIdentity::Unavailable;
        for room in [14, 38, 120] {
            for kind in [TableKind::Command, TableKind::Summary] {
                let area = Rect::new(
                    0,
                    0,
                    room + cell_width(SECTION_HEADER_INDENT) + CONTENT_RIGHT_INSET,
                    6,
                );
                let mut buffer = Buffer::empty(area);
                draw_process_table(
                    &mut buffer,
                    area,
                    &[&row],
                    kind,
                    Color::Reset,
                    PinnedGroup::Unpinned,
                    ProcessTree::Long,
                );
                assert_eq!(
                    buffer_line(&buffer, TABLE_HEADER_HEIGHT).trim(),
                    "[hana-linux-2]"
                );
                assert_eq!(
                    buffer[(cell_width(SECTION_HEADER_INDENT), TABLE_HEADER_HEIGHT)].fg,
                    accent_color()
                );
            }
        }
        row.process.provenance = RowProvenance::Uncaptured(ProcessOwner::Unavailable);
        for kind in [TableKind::Command, TableKind::Summary] {
            let buffer = measurement_row_buffer(&row, kind);
            assert_eq!(
                buffer_line(&buffer, TABLE_HEADER_HEIGHT).trim(),
                "unavailable"
            );
        }
    }

    #[test]
    fn a_narrow_unreadable_directory_heading_marks_its_accounts_tail() {
        let mut row = owned(
            crate::constants::UNRESOLVED_PATH,
            40,
            1000,
            AccountName::Resolved("hana-linux-2".into()),
        );
        row.process.directory_identity = WorkingDirectoryIdentity::Unavailable;
        for (room, expected) in [(8, "[hana-l…"), (13, "[hana-linux-…")] {
            for kind in [TableKind::Command, TableKind::Summary] {
                let area = Rect::new(
                    0,
                    0,
                    room + cell_width(SECTION_HEADER_INDENT) + CONTENT_RIGHT_INSET,
                    6,
                );
                let mut buffer = Buffer::empty(area);
                draw_process_table(
                    &mut buffer,
                    area,
                    &[&row],
                    kind,
                    Color::Reset,
                    PinnedGroup::Unpinned,
                    ProcessTree::Long,
                );
                let heading = buffer_line(&buffer, TABLE_HEADER_HEIGHT);
                assert_eq!(heading.trim(), expected, "{kind:?}, {room}: {heading}");
                assert_eq!(
                    buffer[(
                        cell_width(SECTION_HEADER_INDENT) + room - 1,
                        TABLE_HEADER_HEIGHT
                    )]
                        .fg,
                    accent_color()
                );
            }
        }
    }

    /// A runner's unreadable directory heads as its account; a registered build
    /// names the checkout supplied by its capture.
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
            (&unread, "[runner-one]"),
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
        let pinned = PinnedGroup::Lead(&lead.process);
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
        let pinned = PinnedGroup::Lead(&second.process);
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
        let layout = CellTableLayout::of(
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
        let layout = CellTableLayout::of(
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
            0
        );
        for row in &rows {
            assert_eq!(state_cell(row, &layout).to_string().trim(), "50%");
        }
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

        let groups = group_by_path(&rows, PinnedGroup::Lead(&ordinary.process));

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
        row.process.resource_use.compiler = CompilerObservation::Running(Compiler {
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
        let layout = CellTableLayout::of(
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
            interior_width
                .saturating_add(cell_width(SECTION_ITEM_INDENT))
                .saturating_add(CONTENT_RIGHT_INSET),
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
        let grid = FrameTableColumnGrid::of(&row_refs, kind, ProcessTree::Long);
        let full_width =
            table_width_with_spacing(&grid.constraints(&grid.slots), TIGHT_TABLE_COLUMN_SPACING)
                .saturating_add(grid.rightmost_value_clearance());
        for interior_width in interior_widths {
            let area = Rect::new(0, 0, interior_width + indent + CONTENT_RIGHT_INSET, 8);
            let layout =
                CellTableLayout::of(&row_refs, kind, area, Color::Reset, ProcessTree::Long);
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
        summary_alone_buffer_in_tree(roster, inner_width, ProcessTree::Long)
    }

    fn summary_alone_buffer_in_tree(
        roster: &Roster,
        inner_width: u16,
        tree: ProcessTree,
    ) -> Buffer {
        let area = Rect::new(0, 0, inner_width.saturating_add(2), 8);
        let mut buffer = Buffer::empty(area);
        let mut grid = tui_pane::TileGrid::new();
        grid.set_min_tile_width(crate::constants::MIN_CELL_WIDTH);
        let sccache = SccacheStats::new();
        let hidden_when_idle = hidden_when_idle();
        let cells = Cells::new(roster, &hidden_when_idle, tree, &sccache);
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

    #[test]
    fn a_finished_commands_cell_reports_the_fade_its_contents_use() {
        for hidden in [Vec::new(), hidden_when_idle()] {
            let now = Instant::now();
            let lead = invocation(4100, &[PORT_SUBCOMMAND_NAME]);
            let child = invocation(4200, &["build"]);
            let mut roster = Roster::new();
            roster.observe(
                vec![CargoGroup {
                    lead:     lead.clone(),
                    rest:     vec![child],
                    ancestry: crate::census::scan::AncestryWalk::Ended(Vec::new()),
                }],
                now,
            );
            roster.observe(vec![cargo_group(lead)], now);
            roster.advance(
                now + std::time::Duration::from_secs(1),
                std::time::Duration::from_secs(crate::constants::DEFAULT_FADE_SECONDS),
            );
            let sccache = SccacheStats::new();
            let cells = Cells::new(&roster, &hidden, ProcessTree::Long, &sccache);
            let group = &roster.groups()[0];
            assert!(group.rows().skip(1).any(|row| row.faded() > 0));
            assert_eq!(
                cells.readout_fade(&TileContent::Group(group.id.clone())),
                CellReadoutFade::Unfaded,
                "a live lead holds the cell ink"
            );
            roster.observe(Vec::new(), now + std::time::Duration::from_secs(1));
            roster.advance(
                now + std::time::Duration::from_secs(2),
                std::time::Duration::from_secs(crate::constants::DEFAULT_FADE_SECONDS),
            );
            let group = &roster.groups()[0];
            let rows: Vec<_> = group
                .rows()
                .skip(usize::from(group.leads_as_ancestor(&hidden)))
                .collect();
            let fade = heading_fade(&rows).min(group.lead.faded());
            assert!(fade > 0);
            let cells = Cells::new(&roster, &hidden, ProcessTree::Long, &sccache);
            assert_eq!(
                cells.readout_fade(&TileContent::Group(group.id.clone())),
                CellReadoutFade::Fading(CellFadeProgress::new(fade))
            );
            assert_eq!(
                cells.readout_fade(&TileContent::Empty(3)),
                CellReadoutFade::Unfaded
            );
        }
    }

    #[test]
    fn the_summary_foot_fades_with_its_rows() {
        let now = Instant::now();
        let mut driver = invocation(4100, &[PORT_SUBCOMMAND_NAME]);
        driver.resource_use.memory = Measurement::Reading(BYTES_PER_GIBIBYTE);
        driver.subtree_resource_use.memory = driver.resource_use.memory;
        let mut roster = Roster::new();
        roster.observe(
            vec![CargoGroup {
                lead:     driver.clone(),
                rest:     vec![invocation(4200, &["build"])],
                ancestry: crate::census::scan::AncestryWalk::Ended(Vec::new()),
            }],
            now,
        );
        roster.observe(vec![cargo_group(driver.clone())], now);
        roster.advance(
            now + std::time::Duration::from_secs(1),
            std::time::Duration::from_secs(crate::constants::DEFAULT_FADE_SECONDS),
        );
        let hidden = hidden_when_idle();
        let rows = summary_rows(&roster, &hidden);
        let fade = heading_fade(&rows.iter().map(AsRef::as_ref).collect::<Vec<_>>());
        assert!(fade > 0, "promoted rows fade while the driver stays live");
        let sccache = SccacheStats::new();
        let cells = Cells::new(&roster, &hidden, ProcessTree::Long, &sccache);
        assert_eq!(
            cells.readout_fade(&TileContent::Summary),
            CellReadoutFade::Fading(CellFadeProgress::new(fade))
        );
        let area = Rect::new(0, 0, 200, 20);
        let (grid, buffer) = settled_buffer_with_cells(&cells, area);
        let summary = grid
            .placements(area, TileGrowth::default())
            .into_iter()
            .find(|cell| cell.content == TileContent::Summary)
            .expect("summary is drawn");
        let inner = summary.frame.inner();
        let foot_y = inner.bottom() - 1;
        let SummaryFoot::Text(foot) = cells.summary_foot() else {
            panic!("the summary reports its memory total");
        };
        let mut x = inner.x + 1;
        for span in foot.spans {
            let ink = span.style.fg.or(foot.style.fg).unwrap_or_else(text_default);
            for _ in span.content.chars() {
                assert_eq!(
                    buffer[(x, foot_y)].fg,
                    blend_color(ink, pane_background(summary.frame.is_focused()), fade)
                );
                x += 1;
            }
        }
        roster.observe(
            vec![CargoGroup {
                lead:     driver,
                rest:     vec![invocation(4300, &["check"])],
                ancestry: crate::census::scan::AncestryWalk::Ended(Vec::new()),
            }],
            now + std::time::Duration::from_secs(1),
        );
        let cells = Cells::new(&roster, &hidden, ProcessTree::Long, &sccache);
        assert_eq!(
            cells.readout_fade(&TileContent::Summary),
            CellReadoutFade::Unfaded
        );
    }

    /// Production cells with the trait's every-row default retained.
    struct CellsWithoutRowReport<'a>(Cells<'a>);

    impl TileCells<InvocationId> for CellsWithoutRowReport<'_> {
        fn prepare_frame(&self, allocations: impl FnOnce() -> Vec<(TileContent, Rect)>) {
            self.0.prepare_frame(allocations);
        }

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
                    ancestry: AncestryWalk::Ended(ancestry.to_vec()),
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
                    ancestry: AncestryWalk::Ended(ancestry.to_vec()),
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
                ancestry: crate::census::scan::AncestryWalk::Ended(vec![ancestor(7000, "cargo")]),
            },
            CargoGroup {
                lead: recipient,
                rest,
                ancestry: crate::census::scan::AncestryWalk::Ended(Vec::new()),
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
                    ancestry: AncestryWalk::Ended(Vec::new()),
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
        let mut former = Cells::new(roster, &hidden_when_idle, ProcessTree::Long, &sccache);
        former.grids.summary.0.separation = DirectorySeparation::KeepTogether;
        let without_report = CellsWithoutRowReport(former);
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
    fn a_separator_loan_paints_the_recipients_new_process_rows() {
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
        for (lead, child) in [(4200, 4201), (4400, 4402), (4600, 4602)] {
            let group = roster
                .groups()
                .iter()
                .find(|group| group.lead.process.pid == lead)
                .expect("recipient group");
            let placement = placements
                .iter()
                .find(|placement| placement.content == TileContent::Group(group.id.clone()))
                .expect("recipient frame");
            let rect = placement.frame.inner();
            let text: String = (rect.top()..rect.bottom())
                .flat_map(|y| (rect.left()..rect.right()).map(move |x| (x, y)))
                .map(|position| buffer[position].symbol())
                .collect();
            assert!(text.contains(&child.to_string()), "{lead}: {text}");
        }
    }

    #[test]
    fn a_near_full_summary_yields_separators_for_hidden_process_rows() {
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
            .iter()
            .find(|placement| placement.content == TileContent::Summary)
            .expect("the summary is placed");
        let widths = [(TileContent::Summary, summary.frame.inner().width)];
        let demands = tile_demands(&roster, &widths, &hidden_when_idle(), ProcessTree::Long);

        assert_eq!(demands.summary, 18, "{}", buffer_rows(&buffer).join("\n"));
        assert_eq!(
            blank_content_row_count(&buffer, summary),
            0,
            "{}",
            buffer_rows(&buffer).join("\n")
        );
        assert_eq!(placement_content_rows(summary), 13);
        let summary_rect = summary.frame.inner();
        let summary_text: String = (summary_rect.top()..summary_rect.bottom())
            .flat_map(|y| (summary_rect.left()..summary_rect.right()).map(move |x| (x, y)))
            .map(|position| buffer[position].symbol())
            .collect();
        for pid in [4100, 4200, 4300, 4400, 4500, 4600] {
            assert!(summary_text.contains(&pid.to_string()), "{summary_text}");
        }
        let visible: Vec<_> = placements
            .iter()
            .filter_map(|placement| {
                let TileContent::Group(ref id) = placement.content else {
                    return None;
                };
                let group = roster
                    .groups()
                    .iter()
                    .find(|group| &group.id == id)
                    .expect("group");
                let rect = placement.frame.inner();
                let text: String = (rect.top()..rect.bottom())
                    .flat_map(|y| (rect.left()..rect.right()).map(move |x| (x, y)))
                    .map(|position| buffer[position].symbol())
                    .collect();
                Some(
                    group
                        .rows()
                        .filter(|row| text.contains(&row.process.pid.to_string()))
                        .map(|row| row.process.pid)
                        .collect::<Vec<_>>(),
                )
            })
            .collect();
        assert_eq!(
            visible,
            vec![
                vec![4100],
                vec![4200, 4201],
                vec![4300],
                vec![4400, 4401, 4402],
                vec![4500],
                vec![4600, 4601, 4602]
            ]
        );
        assert!(!has_blank_row_above_foot(&buffer, summary));
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
        let area = Rect::new(0, 0, 126, 60);
        let target = TileContent::Group(roster.groups()[3].id.clone());
        let (production, without_report) = row_report_comparison(&roster, area);
        let former =
            blank_rows_beside_hidden_content(&roster, &without_report.0, &without_report.1, area);
        let repaired =
            blank_rows_beside_hidden_content(&roster, &production.0, &production.1, area);

        assert!(
            former.contains(&target),
            "former: {former:?}; repaired: {repaired:?}"
        );
        assert!(!repaired.contains(&target));
        let placement = production
            .0
            .placements(
                area,
                TileGrowth {
                    initial_rows:  12,
                    fill:          TileFill::Redistribute,
                    widen_summary: false,
                },
            )
            .into_iter()
            .find(|placement| placement.content == target)
            .expect("recipient cell");
        let rect = placement.frame.inner();
        let text: String = (rect.top()..rect.bottom())
            .flat_map(|y| (rect.left()..rect.right()).map(move |x| (x, y)))
            .map(|position| production.1[position].symbol())
            .collect();
        assert!(text.contains("4400"), "{text}");
        assert!(!text.contains("4401"), "{text}");
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
        lead.resource_use.memory = Measurement::Reading(BYTES_PER_GIBIBYTE);
        lead.subtree_resource_use.memory = lead.resource_use.memory;
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
            let marked_note = matches!(empty_note.strip_suffix(ELISION), Some(prefix)
                if !prefix.trim().is_empty() && NO_PROCESSES_NOTE.starts_with(prefix));
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
            let layout = CellTableLayout::of(
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
            vec![
                MANAGED_COLUMN,
                COMPILER_COLUMN,
                START_COLUMN,
                DURATION_COLUMN,
                MEMORY_COLUMN,
                CPU_COLUMN,
                STATE_COLUMN,
                PARENT_COLUMN,
                COMMAND_COLUMN,
                PID_COLUMN,
            ]
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
        let layout = CellTableLayout::of(
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

        assert_eq!(command_lines, ["cargo metadata"]);
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

    #[test]
    fn the_command_column_is_capped_by_the_frames_longest_command() {
        let mut row = long_row();
        row.process.resource_use.memory = Measurement::Reading(BYTES_PER_GIBIBYTE * 124 / 10);
        row.process.resource_use.compiler = CompilerObservation::Running(Compiler {
            name:  COMPILER_PROCESS_NAMES[0],
            count: 1,
        });
        row.process.managed = Measurement::Reading(1);
        let rows = [&row];
        let command_width = command_line_width(&row, SummaryDetail::Full, ProcessTree::Long);
        for width in [85, 200] {
            let area = Rect::new(0, 0, width, 16);
            let layout = CellTableLayout::of(
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
            let header = buffer_line(&buffer, 0);
            let command_start = header.find(TABLE_HEADERS[COMMAND_COLUMN]).unwrap();
            let compiler_start = header.find(TABLE_HEADERS[COMPILER_COLUMN]).unwrap();
            let drawn_width = compiler_start - command_start - usize::from(layout.column_spacing);
            if width == 200 {
                assert_eq!(
                    drawn_width,
                    usize::from(command_width + TABLE_CONTINUATION_CLEARANCE)
                );
            } else {
                assert!(drawn_width < usize::from(command_width + TABLE_CONTINUATION_CLEARANCE));
            }
            assert!(
                drawn_width
                    >= usize::from(command_line_width(
                        &row,
                        SummaryDetail::Full,
                        ProcessTree::Short
                    ))
            );
        }
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
        let area = Rect::new(0, 0, 75, 8);
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
        let area = Rect::new(0, 0, 80, 3);
        let mut buffer = Buffer::empty(area);
        let mut row = long_row();
        row.process.resource_use.compiler = CompilerObservation::Running(Compiler {
            name:  COMPILER_PROCESS_NAMES[0],
            count: 1,
        });
        row.process.managed = Measurement::Reading(1);
        let rows = [&row];
        let layout = CellTableLayout::of(
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
        row.process.command = CommandText::of("cargo", &["界", "alpha/beta", "gamma", "delta"]);
        let rows = [&row];
        let layout = CellTableLayout::of(
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
                CellTableLayout::of(
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
        let layout = CellTableLayout::of(
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
        lead.process.resource_use.compiler = CompilerObservation::Running(Compiler {
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
        let layout = CellTableLayout::of(
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
            PinnedGroup::Lead(&lead.process),
            ProcessTree::Long,
        );

        let command = table_column_text(&buffer, &layout, COMMAND_COLUMN, 5);
        assert!(command.starts_with("cargo check"), "{command:?}");
        assert_eq!(
            buffer[(area.right() - CONTENT_RIGHT_INSET - 1, 5)].symbol(),
            ELISION
        );
        assert!(
            command["cargo check".len()..]
                .chars()
                .all(|character| character == ' ')
        );
        assert!(buffer_line(&buffer, 4).contains("/workspace/later"));
        assert_ne!(buffer_line(&buffer, 5), "");
    }

    #[test]
    fn a_whole_longest_command_marks_later_rows_without_using_a_free_row() {
        let area = Rect::new(0, 0, 62, 4);
        let mut buffer = Buffer::empty(area);
        let mut lead = same_second("/workspace/project", 2_398_793);
        lead.process.parent = VisibleParent::Ancestor(2_398_735);
        lead.process.resource_use.cpu = Measurement::Reading(1192.0);
        lead.process.resource_use.memory = Measurement::Reading(BYTES_PER_GIBIBYTE * 98 / 10);
        lead.process.command = CommandText::of("cargo", &["nextest", "run"]);
        lead.process.managed = Measurement::Reading(2);
        let later = same_second("/workspace/later", 2_614_582);
        let rows = [&lead, &later];
        let layout = CellTableLayout::of(
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
            PinnedGroup::Lead(&lead.process),
            ProcessTree::Long,
        );

        assert_eq!(
            layout.column_width(COMMAND_COLUMN),
            cell_width("cargo nextest run").saturating_add(TABLE_CONTINUATION_CLEARANCE)
        );
        assert_eq!(
            table_column_text(&buffer, &layout, COMMAND_COLUMN, 2).trim_end(),
            "cargo nextest run"
        );
        assert_eq!(
            buffer[(area.right() - CONTENT_RIGHT_INSET - 1, 2)].symbol(),
            ELISION
        );
        assert_eq!(
            table_column_text(&buffer, &layout, COMMAND_COLUMN, 3).trim(),
            ""
        );
    }

    #[test]
    fn a_whole_longest_command_keeps_its_last_letter_when_no_row_is_free() {
        let area = Rect::new(0, 0, 62, 3);
        let mut buffer = Buffer::empty(area);
        let mut lead = same_second("/workspace/project", 2_398_793);
        lead.process.parent = VisibleParent::Ancestor(2_398_735);
        lead.process.resource_use.cpu = Measurement::Reading(1192.0);
        lead.process.resource_use.memory = Measurement::Reading(BYTES_PER_GIBIBYTE * 98 / 10);
        lead.process.command = CommandText::of("cargo", &["nextest", "run"]);
        lead.process.managed = Measurement::Reading(2);
        let later = same_second("/workspace/later", 2_614_582);
        let rows = [&lead, &later];
        let layout = CellTableLayout::of(
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
            PinnedGroup::Lead(&lead.process),
            ProcessTree::Long,
        );

        assert_eq!(
            layout.column_width(COMMAND_COLUMN),
            cell_width("cargo nextest run").saturating_add(TABLE_CONTINUATION_CLEARANCE)
        );
        assert_eq!(
            table_column_text(&buffer, &layout, COMMAND_COLUMN, 2).trim_end(),
            "cargo nextest run"
        );
        assert_eq!(
            buffer[(area.right() - CONTENT_RIGHT_INSET - 1, 2)].symbol(),
            ELISION
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

        let marked = buffer_line(&buffer, 5);
        assert!(!marked.contains("project-2"), "{marked:?}");
        assert!(
            buffer_rows(&buffer)
                .iter()
                .all(|line| !line.contains("project-2"))
        );
        assert!(marked.ends_with(ELISION), "{marked:?}");
        assert!(buffer_line(&buffer, 4).contains("project-1"));
        assert_eq!(buffer_line(&buffer, 3), "");
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
        let layout = CellTableLayout::of(
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
        assert!(table_column_text(&buffer, &layout, COMMAND_COLUMN, 7).starts_with("cargo"));
        assert_eq!(
            buffer[(area.right() - CONTENT_RIGHT_INSET - 1, 7)].symbol(),
            ELISION
        );
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
        let layout = CellTableLayout::of(
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
        assert!(table_column_text(&buffer, &layout, COMMAND_COLUMN, 6).starts_with("cargo"));
        assert_eq!(
            buffer[(area.right() - CONTENT_RIGHT_INSET - 1, 6)].symbol(),
            ELISION
        );
        assert!(!drawn.contains("project-3"), "{drawn}");
    }

    #[test]
    fn a_summary_keeps_its_command_name_whole_at_the_foot() {
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
        assert!(last.ends_with("cargo clippy"), "{last:?}");
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
        let title = SUMMARY_CELL_TITLE;
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
                "┌ summary ─────────────────────────────────────────────────┐",
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
                "│   view              < cells >                            │",
                "│   widen summary     < false >                            │",
                "│   fade seconds      < 3 >                                │",
                "│ Capture:                                                 │",
                "└──────────────────────────────────────────────────────────┘",
            ],
        ),
        (
            Some(GlobalAction::OpenKeymap),
            [
                "┌ summary ─────────────────────────────────────────────────┐",
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
                "┌ summary ─────────────────────────────────────────────────┐",
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
            ToastActivation::Informational,
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
            ToastActivation::Informational,
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
        const TOAST_LINES: usize = 6;
        for (opener, mut expected) in FRAME_TAIL_CASES {
            match opener {
                None => {
                    expected[10] = "│ └──────────────────────────────────────────────────────┘ │";
                    expected[11] = "│                                                          │";
                },
                Some(GlobalAction::OpenGlobalShortcuts) => {
                    expected[10] = "│ └──│   Focus the tile to the left        left      │───┘ │";
                    expected[11] = "│    │   Focus the tile to the right       right     │     │";
                },
                _ => {},
            }
            if opener.is_none() {
                expected[9] = "│ └──────────────────────────────────────────────────────┘ │";
                expected[10] = "│                                                          │";
                expected[11] = "│                              content rows: 0  r/c: 11/58 │";
            }
            if matches!(opener, Some(GlobalAction::OpenGlobalShortcuts)) {
                expected[9] = "│ └──│   Focus the tile below              down      │───┘ │";
                expected[10] = "│    │   Focus the tile to the left        left      │     │";
                expected[11] = "│    │   Focus the tile to the right       right     │1/58 │";
            }
            let mut app = App::new_for_test().expect("test app should build");
            app.started = Instant::now();
            app.framework.toasts.push_persistent(
                "toast title",
                "the toast's body",
                ToastStyle::Normal,
                ToastActivation::Informational,
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
