//! Frame rendering: the app's panes, the framework status line along the
//! bottom, and whichever framework overlay is open above them.

#[cfg(target_os = "linux")]
mod linux_processors;

mod invocation_frame;

use std::borrow::Cow;
use std::cell::Cell;
use std::cell::OnceCell;
use std::cell::RefCell;
use std::collections::HashMap;
use std::num::NonZeroU16;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::LazyLock;
use std::sync::OnceLock;
use std::time::Duration;

use invocation_frame::DepartureSite;
use invocation_frame::EndedCellContent;
use invocation_frame::FrameInvocationPlacement;
use invocation_frame::InvocationCellId;
use invocation_frame::InvocationProcessRows;
use invocation_frame::PaintedProcessRowSpan;
use invocation_frame::PlacementTrial;
use invocation_frame::ProcessRowPaint;
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
#[cfg(test)]
use ratatui::widgets::Row;
#[cfg(test)]
use ratatui::widgets::Table;
use ratatui::widgets::Widget;
use tui_pane::AdmissibleCell;
use tui_pane::AttractWork;
use tui_pane::BarPalette;
use tui_pane::CellContentExtent;
use tui_pane::CellDeparture;
use tui_pane::CellFadeProgress;
use tui_pane::CellReadoutFade;
use tui_pane::FrameLayoutPreparation;
use tui_pane::FramePlacement;
use tui_pane::Keymap;
use tui_pane::NotAdmissible;
use tui_pane::PaintedRowSpan;
use tui_pane::PaneFrameLabel;
use tui_pane::PresentedRowSpan;
use tui_pane::SECTION_HEADER_INDENT;
use tui_pane::SECTION_ITEM_INDENT;
use tui_pane::ScanIndicator;
use tui_pane::StatusLine;
use tui_pane::StatusLineGlobal;
use tui_pane::StatusLineNote;
use tui_pane::SummaryDemand;
use tui_pane::SummaryFoot;
use tui_pane::Theme;
use tui_pane::TileCells;
use tui_pane::TileGridContents;
#[cfg(test)]
use tui_pane::TileGrowth;
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
use crate::app::ArgumentDetail;
use crate::app::ProcessHierarchy;
use crate::census::command_text::CommandWord;
use crate::census::command_text::CommandWordDisplay;
use crate::config::DefaultArguments;
use crate::constants::ALL_ARGUMENTS_NOTE_LABEL;
use crate::constants::CARGO_RELEASE_PROFILE;
use crate::constants::CARGO_TOOLCHAIN_SELECTOR;
use crate::constants::COMMAND_WORD_SEPARATOR;
use crate::constants::NEXTEST_SUBCOMMAND_NAME;

/// A frame's independent hierarchy and argument choices with checked defaults.
#[derive(Clone, Debug, Eq, PartialEq)]
struct CommandView {
    hierarchy: ProcessHierarchy,
    arguments: ArgumentDetail,
    defaults:  DefaultArguments,
}

impl CommandView {
    fn new(
        hierarchy: ProcessHierarchy,
        arguments: ArgumentDetail,
        defaults: &DefaultArguments,
    ) -> Self {
        Self {
            hierarchy,
            arguments,
            defaults: defaults.clone(),
        }
    }

    #[cfg(test)]
    fn short() -> &'static Self {
        static VIEW: LazyLock<CommandView> = LazyLock::new(|| {
            CommandView::new(
                ProcessHierarchy::Shown,
                ArgumentDetail::Default,
                &DefaultArguments::default(),
            )
        });
        &VIEW
    }

    fn long() -> &'static Self {
        static VIEW: LazyLock<CommandView> = LazyLock::new(|| {
            CommandView::new(
                ProcessHierarchy::Shown,
                ArgumentDetail::All,
                &DefaultArguments::default(),
            )
        });
        &VIEW
    }
}

use crate::app_global_action::AppGlobalAction;
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
use crate::census::scan::PublishedInvocation;
use crate::constants::ACCOUNT_HEADING_CLOSE;
use crate::constants::ACCOUNT_HEADING_OPEN;
use crate::constants::ANCESTRY_GAP_HEIGHT;
use crate::constants::ANCESTRY_LEVEL_INDENT;
use crate::constants::ANCESTRY_MIN_COMMAND_WIDTH;
use crate::constants::APP_NAME;
use crate::constants::APP_VERSION;
use crate::constants::ATTRACT_NOTE_LABEL;
use crate::constants::BYTES_PER_GIBIBYTE;
use crate::constants::CELL_FRAME_CHROME_ROWS;
use crate::constants::COMMAND_COLUMN;
use crate::constants::COMMAND_FIRST_CONTENT_ROWS;
use crate::constants::COMPILER_COLUMN;
use crate::constants::CONTENT_RIGHT_INSET;
use crate::constants::CPU_COLUMN;
use crate::constants::DEFAULT_FADE_SECONDS;
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
#[cfg(test)]
use crate::roster::Roster;
use crate::roster::TrackedGroup;
use crate::roster::TrackedRow;
use crate::roster::VisibleRoster;
use crate::sccache::LabelRunKind;
use crate::sccache::SccacheStats;
use crate::settings;
use crate::theme;
use crate::tiles::TileContent;
use crate::tiles::TileDemand;
use crate::tiles::TileDemands;
#[cfg(test)]
use crate::tiles::TileGrid;

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

    let roster = app
        .roster
        .visible(&app.loaded_config.config.commands.hidden);
    let work = if roster.groups().is_empty() {
        AttractWork::Idle
    } else {
        AttractWork::Running
    };
    let updates = app.updates;
    let mut summary_frames = DrawnSummaryFrames(Vec::new());
    draw_attract_layers(frame, app, body, work, updates, |frame, app, contents| {
        summary_frames = draw_visible_panes(frame, app, body, contents, roster);
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
#[cfg(test)]
fn draw_panes(
    frame: &mut Frame,
    app: &mut App,
    area: Rect,
    contents: TileGridContents,
) -> DrawnSummaryFrames {
    let roster = app
        .roster
        .visible(&app.loaded_config.config.commands.hidden);
    draw_visible_panes(frame, app, area, contents, roster)
}

fn draw_visible_panes(
    frame: &mut Frame,
    app: &mut App,
    area: Rect,
    contents: TileGridContents,
    roster: VisibleRoster,
) -> DrawnSummaryFrames {
    let growth = app.loaded_config.config.tiles.growth();
    let view = CommandView::new(
        app.tree,
        app.arguments,
        &app.loaded_config.config.commands.default_arguments,
    );
    let cells = Cells::for_frame(
        roster,
        &view,
        &app.sccache,
        area.height.saturating_sub(CELL_FRAME_CHROME_ROWS),
    )
    .with_fade(app.loaded_config.config.tiles.fade());
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

/// `Cells` records the summary frames painted by `draw_tile_grid`.
struct DrawnSummaryFrames(Vec<Rect>);

/// Preparation is reusable only for the same allocation and frame policy.
#[derive(Default)]
enum FramePreparation {
    #[default]
    Unprepared,
    Allocated(Vec<(TileContent, Rect)>),
    Committed,
}

/// What the tile grid's cells hold, borrowed out of [`App`] for one frame.
struct Cells<'a> {
    preparation:         RefCell<FramePreparation>,
    placement:           RefCell<FrameInvocationPlacement>,
    trial:               RefCell<PlacementTrial>,
    /// Every command group, which the summary gathers and each group's
    /// own cell draws.
    roster:              VisibleRoster,
    process_rows:        InvocationProcessRows,
    /// Configured departure duration shared with the grid lifecycle.
    fade:                Duration,
    /// What sccache reports, written along the summary cell's top border.
    sccache:             &'a SccacheStats,
    /// Content-row answers already computed during this draw.
    row_uses:            RefCell<HashMap<u16, PreparedWidthRows>>,
    /// Candidate membership can change without changing the frame's row values.
    candidate_summaries: RefCell<Vec<PreparedCandidateSummary>>,
    painted:             RefCell<Vec<PaintedRowSpan<InvocationId>>>,
    cell_receipts:       RefCell<HashMap<CellRowIdentity, Vec<PaintedProcessRowSpan>>>,
    /// Intrinsic column inputs shared by every allocation in this frame.
    grids:               FrameColumnGrids,
}

impl<'a> Cells<'a> {
    #[cfg(test)]
    fn new(
        roster: &Roster,
        hidden_commands: &[String],
        view: &CommandView,
        sccache: &'a SccacheStats,
    ) -> Self {
        Self::for_frame(
            roster.clone().visible(hidden_commands),
            view,
            sccache,
            u16::MAX,
        )
    }

    /// Cell provider with an empty cache for this draw.
    fn for_frame(
        roster: VisibleRoster,
        view: &CommandView,
        sccache: &'a SccacheStats,
        height: u16,
    ) -> Self {
        Self {
            sccache,
            process_rows: InvocationProcessRows::from(&roster),
            placement: RefCell::new(FrameInvocationPlacement::listed(&roster)),
            trial: RefCell::new(PlacementTrial::from(FrameInvocationPlacement::listed(
                &roster,
            ))),
            preparation: RefCell::new(FramePreparation::Unprepared),
            fade: Duration::from_secs(DEFAULT_FADE_SECONDS),
            row_uses: RefCell::new(HashMap::new()),
            candidate_summaries: RefCell::new(Vec::new()),
            painted: RefCell::new(Vec::new()),
            cell_receipts: RefCell::new(HashMap::new()),
            grids: FrameColumnGrids::for_frame(&roster, view, height),
            roster,
        }
    }

    fn prepare_command_form(&self, allocations: &[(TileContent, Rect)]) {
        let grid = &self.grids.commands.0;
        if matches!(
            *grid.command_layout.borrow(),
            CommandFrameLayout::Committed(_)
        ) {
            return;
        }
        let Some(width) = allocations
            .iter()
            .filter_map(|(content, area)| {
                matches!(content, TileContent::Group(_)).then_some(indented(*area).width)
            })
            .min()
        else {
            return;
        };
        let next = CommandFrameLayout::Prepared(CommandHeaderForm {
            columns: grid.fit_command_columns(width).columns,
        });
        if *grid.command_layout.borrow() != next {
            grid.command_layout.replace(next);
            grid.prepared_plans.borrow_mut().clear();
            self.row_uses.borrow_mut().clear();
            *self.preparation.borrow_mut() = FramePreparation::Unprepared;
        }
    }

    fn clear_summary_plans(&self) {
        if !matches!(*self.preparation.borrow(), FramePreparation::Committed) {
            *self.preparation.borrow_mut() = FramePreparation::Unprepared;
        }
        for rows in self.row_uses.borrow_mut().values_mut() {
            rows.summary.clear();
        }
        self.grids.summary.0.prepared_plans.borrow_mut().clear();
    }

    fn reset_row_policy(&self) {
        self.grids.summary.0.row_end.set(TableRowEnd::Unreserved);
        let commands = &self.grids.commands.0;
        commands.row_end.set(TableRowEnd::Unreserved);
        commands.cell_gap.set(CellTableGap::Unprepared);
        commands.whole_short_tables.borrow_mut().clear();
    }

    fn frame_layout_policy(&self) -> FrameLayoutPolicy {
        let grid = &self.grids.commands.0;
        FrameLayoutPolicy {
            header:         grid.command_layout.borrow().clone(),
            summary_end:    self.grids.summary.0.row_end.get(),
            command_end:    grid.row_end.get(),
            gap:            grid.cell_gap.get(),
            whole_commands: grid.whole_short_tables.borrow().clone(),
        }
    }

    /// Painter fixtures provide their allocation below the checked grid boundary.
    #[cfg(test)]
    fn draw(&self, buffer: &mut Buffer, content: &TileContent, inner: Rect, ground: Color) {
        match content {
            TileContent::Group(id) => {
                let placement = FrameInvocationPlacement::painter_fixture(&self.roster, id);
                self.paint_cell(buffer, content, inner, ground, &placement);
            },
            _ => self.paint_cell(buffer, content, inner, ground, &self.placement.borrow()),
        }
    }

    fn paint_cell(
        &self,
        buffer: &mut Buffer,
        content: &TileContent,
        inner: Rect,
        ground: Color,
        placement: &FrameInvocationPlacement,
    ) {
        let grid = match content {
            TileContent::Summary => &self.grids.summary.0,
            TileContent::Group(_) | TileContent::Empty(_) => &self.grids.commands.0,
        };
        grid.painted_rows.borrow_mut().clear();
        match content {
            TileContent::Summary => frame_draw_summary(
                buffer,
                &SummaryRows::of(&self.process_rows, placement),
                inner,
                ground,
                grid,
            ),
            TileContent::Group(id) => {
                let assignment = placement.assignment(id);
                let rows = self.process_rows.assigned(&assignment);
                if let CellDeparture::Fading(end) = self.departure(id) {
                    let ended = EndedCellContent::new(rows, &end, self.fade);
                    self.paint_ended_cell(buffer, id, inner, ground, &ended);
                } else {
                    frame_draw_group_rows(
                        buffer,
                        &self.roster,
                        id,
                        inner,
                        ground,
                        grid,
                        &rows.values(),
                    );
                }
            },
            TileContent::Empty(_) => {},
        }
        let spans = grid
            .painted_rows
            .borrow()
            .iter()
            .filter_map(|span| self.process_rows.span(span, process_row_site(content)))
            .collect();
        self.cell_receipts
            .borrow_mut()
            .insert(CellRowIdentity::from(content), spans);
    }

    /// Compose retained values through the current columns and keep the first pid visible.
    fn paint_ended_cell(
        &self,
        buffer: &mut Buffer,
        id: &InvocationId,
        inner: Rect,
        background: Color,
        ended: &EndedCellContent,
    ) {
        let content = TileContent::Group(id.clone());
        let grid = &self.grids.commands.0;
        let rows = ended.values();
        let layout = CellTableLayout::for_table_slots(
            grid,
            Rect {
                height: u16::MAX,
                ..inner
            },
            background,
            &rows,
        );
        let first_row = rows
            .first()
            .map_or(1, |row| process_row_lines(row, &layout));
        let first = self
            .first_content_rows_at(&content, inner)
            .saturating_add(first_row.saturating_sub(1));
        let full = Rect {
            height: inner.height.max(first),
            ..inner
        };
        let offset = full.height.saturating_sub(inner.height);
        let mut composed = Buffer::empty(full);
        composed.set_style(full, Style::default().bg(background));
        frame_draw_group_rows(
            &mut composed,
            &self.roster,
            id,
            full,
            background,
            grid,
            &rows,
        );
        for y in inner.y..inner.bottom() {
            for x in inner.x..inner.right() {
                buffer[(x, y)] = composed[(x, y + offset)].clone();
            }
        }
        let visible = Rect {
            y: inner.y.saturating_add(offset),
            ..inner
        };
        grid.painted_rows.borrow_mut().retain_mut(|painted| {
            let area = painted.span.area().intersection(visible);
            let projected = Rect {
                y: area.y.saturating_sub(offset),
                ..area
            };
            let Ok(span) = PaintedRowSpan::new(painted.span.id().clone(), projected) else {
                return false;
            };
            painted.span = span;
            true
        });
    }

    /// Use the configured fade for each tracked group departure.
    const fn with_fade(mut self, fade: Duration) -> Self {
        self.fade = fade;
        self
    }

    /// Reuse the width-only layout across all heights considered in this draw.
    fn prepared_rows<T>(
        &self,
        content: &TileContent,
        width: u16,
        read: impl FnOnce(&PreparedCellRows) -> T,
    ) -> T {
        let grid = match content {
            TileContent::Summary => &self.grids.summary.0,
            TileContent::Group(_) | TileContent::Empty(_) => &self.grids.commands.0,
        };
        let row_end = grid.row_end.get();
        let frame_row_end = grid.frame_row_end.get();
        let gap = grid.cell_gap.get();
        let retained = match content {
            TileContent::Group(id) => grid
                .retained_ancestry
                .borrow()
                .get(id)
                .copied()
                .map_or(RetainedPrefix::Absent, RetainedPrefix::from),
            TileContent::Summary | TileContent::Empty(_) => RetainedPrefix::Absent,
        };
        {
            let layouts = self.row_uses.borrow();
            if let Some(prepared) = layouts.get(&width).and_then(|rows| {
                rows.for_content(content).iter().find(|prepared| {
                    prepared.policy.row_end == row_end
                        && prepared.policy.frame_row_end == frame_row_end
                        && prepared.policy.gap == gap
                        && prepared.policy.retained == retained
                        && prepared.policy.whole_short == *grid.whole_short_tables.borrow()
                })
            }) {
                return read(&prepared.rows);
            }
        }
        let mut layouts = self.row_uses.borrow_mut();
        let variants = layouts.entry(width).or_default().for_content_mut(content);
        let policy = PreparedRowPolicy {
            row_end,
            frame_row_end,
            gap,
            retained,
            whole_short: grid.whole_short_tables.borrow().clone(),
        };
        let rows = PreparedCellRows::new(
            &self.roster,
            content,
            width,
            &self.grids,
            || self.trial.borrow().summary_rows(&self.process_rows),
            &self.process_rows,
        );
        let index = variants.len();
        variants.push(PreparedRowsForPolicy { policy, rows });
        read(&variants[index].rows)
    }

    /// One shared column gap is settled after every command table has been measured.
    fn prepare_table_spacing(&self, allocations: &[(TileContent, Rect)]) {
        let grid = &self.grids.commands.0;
        let spacing = allocations
            .iter()
            .filter(|(content, _)| matches!(content, TileContent::Group(_)))
            .map(|(_, area)| grid.cell_column_spacing(indented(*area).width))
            .min()
            .unwrap_or(TABLE_COLUMN_SPACING);
        grid.cell_gap.set(CellTableGap::Settled(spacing));
    }

    /// Painted and retained rows at one height, from the prepared width layout.
    fn row_use(&self, content: &TileContent, area: Rect) -> GroupRowUse {
        self.prepared_rows(content, area.width, |rows| rows.at(area.height))
    }

    fn candidate_summary_rows(&self, placement: &FrameInvocationPlacement, area: Rect) -> u16 {
        let grid = &self.grids.summary.0;
        let inputs = CandidateSummaryInputs {
            placement: placement.clone(),
            width:     area.width,
            policy:    PreparedRowPolicy::of(grid, RetainedPrefix::Absent),
        };
        let mut summaries = self.candidate_summaries.borrow_mut();
        if let Some(summary) = summaries.iter().find(|summary| summary.inputs == inputs) {
            return summary.rows.at(area.height).drawn;
        }
        let selection = SummaryRows::of(&self.process_rows, placement);
        let rows = PreparedSummaryRows::new(&selection, inputs.width, grid);
        let drawn = rows.at(area.height).drawn;
        summaries.push(PreparedCandidateSummary { inputs, rows });
        drawn
    }
}

impl TileCells<InvocationId> for Cells<'_> {
    fn required_commands(&self) -> Vec<InvocationId> {
        self.roster
            .groups()
            .iter()
            .filter(|group| !group.lead.is_ended())
            .map(|group| group.id.clone())
            .collect()
    }

    fn placement_trial(&self, placement: &FramePlacement<InvocationId>) {
        self.trial
            .replace(PlacementTrial::from(FrameInvocationPlacement::checked(
                &self.roster,
                placement,
            )));
        self.clear_summary_plans();
    }

    fn frame_placement(&self, placement: &FramePlacement<InvocationId>) {
        let placement = FrameInvocationPlacement::checked(&self.roster, placement);
        if !self.trial.borrow().agrees_with(&placement) {
            self.clear_summary_plans();
        }
        self.trial.replace(PlacementTrial::from(placement.clone()));
        self.placement.replace(placement);
    }

    fn summary_demand_for(
        &self,
        placement: &FramePlacement<InvocationId>,
        _: u16,
    ) -> SummaryDemand<InvocationId> {
        let placement = FrameInvocationPlacement::checked(&self.roster, placement);
        SummaryRows::of(&self.process_rows, &placement).demand()
    }

    fn summary_rows_for(&self, placement: &FramePlacement<InvocationId>, area: Rect) -> u16 {
        let placement = FrameInvocationPlacement::checked(&self.roster, placement);
        self.candidate_summary_rows(&placement, area)
    }

    fn freeze_painted_frame(&self) {
        let mut layout = self.grids.commands.0.command_layout.borrow_mut();
        if let CommandFrameLayout::Prepared(form) = &*layout {
            *layout = CommandFrameLayout::Committed(form.clone());
        }
        self.preparation.replace(FramePreparation::Committed);
    }

    fn admission(
        &self,
        id: &InvocationId,
    ) -> Result<AdmissibleCell<InvocationId>, NotAdmissible<InvocationId>> {
        let group = self
            .roster
            .groups()
            .iter()
            .find(|group| &group.id == id)
            .ok_or_else(|| NotAdmissible::MissingEvidence(id.clone()))?;
        let live = group
            .lead
            .admissible()
            .map_err(|_| NotAdmissible::MissingEvidence(id.clone()))?;
        Ok(AdmissibleCell::new(id.clone(), group.previous_ids(), live))
    }

    fn painted_rows(&self, content: &TileContent, _: Rect) -> Vec<PaintedRowSpan<InvocationId>> {
        let site = process_row_site(content);
        self.cell_receipts
            .borrow()
            .get(&CellRowIdentity::from(content))
            .into_iter()
            .flatten()
            .filter_map(|span| span.adapter(&site))
            .collect()
    }

    fn presented_rows(&self, content: &TileContent, rows: &[PresentedRowSpan<InvocationId>]) {
        let receipts = self.cell_receipts.borrow();
        let sources = receipts.get(&CellRowIdentity::from(content));
        for row in rows {
            self.painted.borrow_mut().push(row.visible().clone());
            if let Some(source) = sources
                .into_iter()
                .flatten()
                .find(|source| source.matches_source(row.source()))
            {
                self.process_rows.presented(source, row.visible().clone());
            }
        }
    }

    fn ended_members(&self, id: &InvocationId) -> Vec<InvocationId> {
        self.roster
            .groups()
            .iter()
            .find(|group| &group.id == id)
            .map_or_else(Vec::new, |group| {
                group
                    .rows()
                    .map(|row| row.process.invocation_id.clone())
                    .collect()
            })
    }

    fn summary_title(&self) -> &str { SUMMARY_CELL_TITLE }

    fn retain_content_prefix(&self, content: &TileContent, rows: u16) {
        if let TileContent::Group(id) = content {
            let previous = self
                .grids
                .commands
                .0
                .retained_ancestry
                .borrow_mut()
                .insert(id.clone(), rows);
            if previous != Some(rows) {
                *self.preparation.borrow_mut() = FramePreparation::Unprepared;
            }
        }
    }

    fn prepare_painted_frame(
        &self,
        placement: &FramePlacement<InvocationId>,
        allocations: impl FnOnce() -> Vec<(TileContent, Rect)>,
    ) -> FrameLayoutPreparation {
        let next = FrameInvocationPlacement::checked(&self.roster, placement);
        let placement_changed = !self.trial.borrow().agrees_with(&next);
        if placement_changed {
            self.trial.replace(PlacementTrial::from(next));
            self.clear_summary_plans();
        }
        let before = self.frame_layout_policy();
        self.prepare_frame(allocations);
        let after = self.frame_layout_policy();
        match (before == after, placement_changed) {
            (false, _) => FrameLayoutPreparation::MeasureAgain,
            (true, true) => FrameLayoutPreparation::MeasureSummaryAgain,
            (true, false) => FrameLayoutPreparation::Ready,
        }
    }

    fn prepare_frame(&self, allocations: impl FnOnce() -> Vec<(TileContent, Rect)>) {
        if matches!(*self.preparation.borrow(), FramePreparation::Committed) {
            return;
        }
        let allocations = allocations();
        self.prepare_command_form(&allocations);
        if matches!(&*self.preparation.borrow(), FramePreparation::Allocated(previous) if *previous == allocations)
        {
            return;
        }
        self.reset_row_policy();
        let mut summary_cut = false;
        let mut commands_cut = false;
        for (content, area) in &allocations {
            let area = *area;
            match content {
                TileContent::Summary => {
                    let selection = self.trial.borrow().summary_rows(&self.process_rows);
                    let rows = selection.listed().to_vec();
                    if !selection.listed().is_empty()
                        && usize::from(area.height)
                            < frame_table_height(
                                &rows,
                                area.width,
                                PinnedGroup::Unpinned,
                                &self.grids.summary.0,
                            )
                            .saturating_sub(
                                group_by_path(&rows, PinnedGroup::Unpinned)
                                    .len()
                                    .saturating_sub(1)
                                    .saturating_mul(usize::from(GROUP_GAP_HEIGHT)),
                            )
                    {
                        continue;
                    }
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

                    let rows = self.process_rows.measured(id);
                    let grid = &self.grids.commands.0;
                    let demand = frame_table_height(
                        &rows,
                        area.width,
                        PinnedGroup::Lead(&group.lead.process),
                        grid,
                    );
                    let ancestry = drawn_ancestry(group, &grid.view);
                    let used = ancestry_source_row_use(
                        AncestrySource::for_group(&ancestry, group, &grid.view),
                        area,
                        grid.ancestry_table_budget(id, area.height, demand),
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
                    if !cut && grid.view.arguments == ArgumentDetail::Default {
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
        self.prepare_table_spacing(&allocations);
        *self.preparation.borrow_mut() = FramePreparation::Allocated(allocations);
    }

    fn summary_foot(&self) -> SummaryFoot { memory_total(&self.roster).foot() }

    fn content_extent(&self, content: &TileContent) -> CellContentExtent {
        let cut = match content {
            TileContent::Group(id) => self.roster.groups().iter().any(|group| {
                &group.id == id
                    && self.grids.commands.0.view.hierarchy == ProcessHierarchy::Shown
                    && matches!(group.ancestry(), AncestryWalk::LimitCut(_))
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
        self.prepared_rows(content, area.width, PreparedCellRows::demand)
            .saturating_sub(usize::from(
                self.row_use(content, area).separator_rows_yielded,
            ))
    }

    fn readout_fade(&self, content: &TileContent) -> CellReadoutFade {
        let faded = match content {
            TileContent::Summary => {
                let rows = self.trial.borrow().listed(&self.process_rows);
                heading_fade(&rows)
            },
            TileContent::Group(id) => {
                let Some(group) = self.roster.groups().iter().find(|group| &group.id == id) else {
                    return CellReadoutFade::Unfaded;
                };
                let rows = self.process_rows.measured(id);
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
        let summary = self.trial.borrow().listed(&self.process_rows);
        let summary_grid = FrameTableColumnGrid::with_parent_links(
            &summary,
            TableKind::Summary,
            &self.grids.summary.0.view,
            AncestryParentLinks::default(),
            invocation_progress_rows(&summary, TableKind::Summary),
            Rc::clone(&self.grids.summary.0.contributions),
        );

        TileDemands {
            summary:       self
                .trial
                .borrow()
                .summary_rows(&self.process_rows)
                .demand(),
            summary_width: frame_summary_width(&summary, &summary_grid),
            groups:        self
                .roster
                .tiled_ids()
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

    fn first_content_rows(&self, content: &TileContent) -> u16 {
        match content {
            TileContent::Group(_) => COMMAND_FIRST_CONTENT_ROWS,
            TileContent::Summary | TileContent::Empty(_) => 1,
        }
    }

    fn first_content_rows_at(&self, content: &TileContent, area: Rect) -> u16 {
        self.prepared_rows(content, area.width, |rows| match rows {
            PreparedCellRows::Command {
                ancestry,
                table_demand,
                ..
            } => ancestry
                .at(area.height, *table_demand)
                .drawn
                .saturating_add(TABLE_HEADER_HEIGHT)
                .saturating_add(GROUP_HEADER_HEIGHT)
                .saturating_add(1),
            PreparedCellRows::Summary(_) | PreparedCellRows::Empty => 1,
        })
    }

    fn content_prefix_rows_at(&self, content: &TileContent, area: Rect) -> u16 {
        self.prepared_rows(content, area.width, |rows| match rows {
            PreparedCellRows::Command {
                ancestry,
                table_demand,
                ..
            } => {
                let rows = ancestry.at(area.height, *table_demand);
                rows.drawn.saturating_sub(rows.gap)
            },
            PreparedCellRows::Summary(_) | PreparedCellRows::Empty => 0,
        })
    }

    fn rows_drawn(&self, content: &TileContent, area: Rect) -> u16 {
        self.row_use(content, area).drawn
    }

    fn rows_kept(&self, content: &TileContent, area: Rect) -> u16 {
        self.row_use(content, area).kept
    }

    fn draw(&self, buffer: &mut Buffer, content: &TileContent, inner: Rect, ground: Color) {
        self.paint_cell(buffer, content, inner, ground, &self.placement.borrow());
    }

    fn summary_labels(&self, rect: Rect) -> Vec<PaneFrameLabel> {
        sccache_label(self.sccache, rect)
    }

    fn departure(&self, id: &InvocationId) -> CellDeparture {
        if let Some(group) = self.roster.groups().iter().find(|group| &group.id == id) {
            return match group.departure(self.fade) {
                CellDeparture::Fading(end) => CellDeparture::Fading(end.with_row_values()),
                state => state,
            };
        }
        self.roster
            .groups()
            .iter()
            .flat_map(TrackedGroup::rows)
            .find(|row| &row.process.invocation_id == id)
            .map_or(CellDeparture::Immediate, |row| match row.publication() {
                PublishedInvocation::Ended(end) => CellDeparture::Fading(end.departure(self.fade)),
                _ => CellDeparture::Live,
            })
    }
}

fn process_row_site(content: &TileContent) -> DepartureSite {
    match content {
        TileContent::Group(id) => DepartureSite::Cell(InvocationCellId(id.clone())),
        TileContent::Summary | TileContent::Empty(_) => DepartureSite::Summary,
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

/// Width probes borrow invocation keys; cache misses can copy the identity.
#[derive(Default)]
struct PreparedWidthRows {
    summary:  Vec<PreparedRowsForPolicy>,
    commands: HashMap<InvocationId, Vec<PreparedRowsForPolicy>>,
    empty:    Vec<PreparedRowsForPolicy>,
}

impl PreparedWidthRows {
    fn for_content(&self, content: &TileContent) -> &[PreparedRowsForPolicy] {
        match content {
            TileContent::Summary => &self.summary,
            TileContent::Group(id) => self.commands.get(id).map_or(&[], Vec::as_slice),
            TileContent::Empty(_) => &self.empty,
        }
    }

    fn for_content_mut(&mut self, content: &TileContent) -> &mut Vec<PreparedRowsForPolicy> {
        match content {
            TileContent::Summary => &mut self.summary,
            TileContent::Group(id) => self.commands.entry(id.clone()).or_default(),
            TileContent::Empty(_) => &mut self.empty,
        }
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum RetainedPrefix {
    Absent,
    Rows(NonZeroU16),
}

impl From<u16> for RetainedPrefix {
    fn from(rows: u16) -> Self { NonZeroU16::new(rows).map_or(Self::Absent, Self::Rows) }
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum ParentColumn {
    Shown,
    Hidden,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum SummaryForm {
    Detailed,
    Compact,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum HeaderLabel {
    Hidden,
    Shown,
}

impl HeaderLabel {
    const fn include(&mut self, shows_value: bool) {
        if shows_value {
            *self = Self::Shown;
        }
    }
}

/// `PreparedRowPolicy` keys `PreparedWidthRows` by table form and retained ancestry.
#[derive(Eq, PartialEq)]
struct PreparedRowPolicy {
    row_end:       TableRowEnd,
    frame_row_end: TableRowEnd,
    gap:           CellTableGap,
    retained:      RetainedPrefix,
    whole_short:   Vec<WholeShortCommandTable>,
}

impl PreparedRowPolicy {
    fn of(grid: &FrameTableColumnGrid, retained: RetainedPrefix) -> Self {
        Self {
            row_end: grid.row_end.get(),
            frame_row_end: grid.frame_row_end.get(),
            gap: grid.cell_gap.get(),
            retained,
            whole_short: grid.whole_short_tables.borrow().clone(),
        }
    }
}

/// `PreparedWidthRows::commands` caches `Cells` measurements by membership, width
/// and the summary policy selected by `FrameInvocationPlacement::checked`.
#[derive(Eq, PartialEq)]
struct CandidateSummaryInputs {
    placement: FrameInvocationPlacement,
    width:     u16,
    policy:    PreparedRowPolicy,
}

struct PreparedCandidateSummary {
    inputs: CandidateSummaryInputs,
    rows:   PreparedSummaryRows,
}

/// One cell's prepared rows under a settled frame policy.
struct PreparedRowsForPolicy {
    policy: PreparedRowPolicy,
    rows:   PreparedCellRows,
}

/// Prepared ancestry and table measurements for one frame cell.
enum PreparedCellRows {
    Empty,
    Summary(PreparedSummaryRows),
    Command {
        ancestry:     PreparedAncestryRows,
        tables:       PreparedTableRows,
        table_demand: usize,
    },
}

impl PreparedCellRows {
    fn new<'a>(
        roster: &VisibleRoster,
        content: &TileContent,
        width: u16,
        grids: &FrameColumnGrids,
        summary: impl FnOnce() -> SummaryRows<'a>,
        process_rows: &InvocationProcessRows,
    ) -> Self {
        match content {
            TileContent::Summary => {
                let summary = summary();
                Self::Summary(PreparedSummaryRows::new(&summary, width, &grids.summary.0))
            },
            TileContent::Group(id) => {
                let Some(group) = roster.groups().iter().find(|group| &group.id == id) else {
                    return Self::Empty;
                };
                let grid = &grids.commands.0;

                let rows = process_rows.measured(id);
                let walk = drawn_ancestry(group, &grid.view);
                let mut ancestry = PreparedAncestryRows::new(
                    AncestrySource::for_group(&walk, group, &grid.view),
                    width,
                );
                ancestry.retained = grid
                    .retained_ancestry
                    .borrow()
                    .get(id)
                    .copied()
                    .map_or(RetainedPrefix::Absent, RetainedPrefix::from);
                let pinned = PinnedGroup::Lead(&group.lead.process);
                // Shown ancestry keeps the same parent-column policy even when
                // no ancestry row fits. Hidden ancestry always uses the table alone.
                let tables = PreparedTableRows::new(
                    &rows,
                    width,
                    pinned,
                    &table_grid_after_ancestry(grid, 0),
                );
                let table_demand = tables.demand();
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
                let table = tables.at(height.saturating_sub(ancestry.drawn));
                GroupRowUse::with_ancestry(&ancestry, table.drawn)
            },
        }
    }
}

/// Preserve a painted prefix when it still leaves the table's first process row.
fn retained_ancestry_table_budget(height: u16, table: usize, retained: RetainedPrefix) -> usize {
    let required = COMMAND_FIRST_CONTENT_ROWS;
    if let RetainedPrefix::Rows(retained) = retained
        && height >= required.saturating_add(retained.get())
    {
        table.min(usize::from(height.saturating_sub(retained.get())))
    } else {
        table
    }
}

/// Heights of every retained ancestry form at this width.
struct PreparedAncestryRows {
    heights:  Vec<usize>,
    retained: RetainedPrefix,
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
        Self {
            heights,
            retained: RetainedPrefix::Absent,
        }
    }

    fn at(&self, height: u16, table: usize) -> AncestryRowUse {
        let table = retained_ancestry_table_budget(height, table, self.retained);
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
                gap:      0,
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
        AncestryRowUse {
            drawn,
            required,
            gap,
        }
    }
}

/// Painted and retained rows at any height of one solved table width.
enum TableHeightUse {
    EmptyTable,
    HeaderOnly,
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
            Self::HeaderOnly | Self::NoReadableColumns => usize::from(TABLE_HEADER_HEIGHT),
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
            Self::HeaderOnly | Self::NoReadableColumns => {
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
    compact_rows: Option<CompactSummaryRows>,
    layouts:      Vec<SummaryRowsAtHeight>,
    separation:   DirectorySeparation,
}

/// Compact command rows protect the listed set until every detailed row fits.
struct CompactSummaryRows {
    rows:            u16,
    headings:        u16,
    detailed_height: usize,
}

/// The first allocation height that uses these summary command widths.
struct SummaryRowsAtHeight {
    height: u16,
    rows:   TableHeightUse,
}

impl PreparedSummaryRows {
    fn new(selection: &SummaryRows<'_>, width: u16, grid: &FrameTableColumnGrid) -> Self {
        let combined = selection.listed().to_vec();
        let rows = combined.as_slice();
        let groups = group_by_path(rows, PinnedGroup::Unpinned);
        let compact_rows = (!selection.listed().is_empty()).then(|| CompactSummaryRows {
            rows:            TABLE_HEADER_HEIGHT
                .saturating_add(u16::try_from(rows.len()).unwrap_or(u16::MAX)),
            headings:        u16::try_from(groups.len()).unwrap_or(u16::MAX),
            detailed_height: frame_table_height(rows, width, PinnedGroup::Unpinned, grid)
                .saturating_sub(
                    groups
                        .len()
                        .saturating_sub(1)
                        .saturating_mul(usize::from(GROUP_GAP_HEIGHT)),
                ),
        });
        let prepare = |height| {
            let area = Rect::new(0, 0, width, height);
            let layout = CellTableLayout::for_table_slots(grid, area, Color::Reset, rows);
            SummaryRowsAtHeight {
                height,
                rows: match (selection, rows.is_empty(), width) {
                    (SummaryRows::NoListedInvocations, _, _) => TableHeightUse::HeaderOnly,
                    (_, true, _) => TableHeightUse::EmptyTable,
                    (_, false, 0) => TableHeightUse::NoReadableColumns,
                    _ => TableHeightUse::new(&groups, &layout),
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
            compact_rows,
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
        if let Some(compact) = &self.compact_rows
            && usize::from(height) < compact.detailed_height
        {
            let drawn = height.min(compact.rows.saturating_add(compact.headings));
            return GroupRowUse {
                drawn,
                kept: drawn,
                separator_rows_yielded: 0,
            };
        }
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
            let area = Rect::new(0, 0, width, u16::MAX);
            let layout = CellTableLayout::for_table_slots(grid, area, pane_background(false), rows);
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
    hidden_commands: &[String],
    view: &CommandView,
) -> u16 {
    content_row_use(roster, content, area, hidden_commands, view).drawn
}

/// Painted and retained rows for one cell allocation.
#[cfg(test)]
fn content_row_use(
    roster: &Roster,
    content: &TileContent,
    area: Rect,
    hidden_commands: &[String],
    view: &CommandView,
) -> GroupRowUse {
    let stats = SccacheStats::default();
    let cells = Cells::new(roster, hidden_commands, view, &stats);
    cells.prepare_frame(|| vec![(content.clone(), area)]);
    cells.row_use(content, area)
}

/// Rows one cell retains before lending to a column mate.
#[cfg(test)]
fn content_rows_kept(
    roster: &Roster,
    content: &TileContent,
    area: Rect,
    hidden_commands: &[String],
    view: &CommandView,
) -> u16 {
    content_row_use(roster, content, area, hidden_commands, view).kept
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
    hidden_commands: &[String],
    view: &CommandView,
) -> TileDemands {
    frame_tile_demands(
        roster,
        widths,
        hidden_commands,
        &FrameColumnGrids::new(roster, hidden_commands, view),
    )
}

#[cfg(test)]
fn frame_tile_demands(
    roster: &Roster,
    widths: &[(TileContent, u16)],
    hidden_commands: &[String],
    grids: &FrameColumnGrids,
) -> TileDemands {
    let visible = roster.clone().visible(hidden_commands);
    let roster = &visible;
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
    let summary = summary_rows(roster);
    TileDemands {
        summary:       SummaryRows::of(
            &InvocationProcessRows::from(roster),
            &FrameInvocationPlacement::listed(roster),
        )
        .demand(),
        summary_width: frame_summary_width(&summary, &grids.summary.0),
        groups:        roster
            .tiled_ids()
            .into_iter()
            .filter_map(|id| roster.groups().iter().find(|group| group.id == id))
            .map(|group| {
                let width = width_of(TileContent::Group(group.id.clone()));
                TileDemand {
                    id:   group.id.clone(),
                    rows: frame_group_height(group, width, &grids.commands.0),
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
/// given is the answer.
#[cfg(test)]
fn group_height(group: &TrackedGroup, width: u16, view: &CommandView) -> usize {
    let rows: Vec<_> = group.rows().collect();
    let grid = FrameTableColumnGrid::of(&rows, TableKind::Command, view);
    frame_group_height(group, width, &grid)
}

#[cfg(test)]
fn frame_group_height(group: &TrackedGroup, width: u16, grid: &FrameTableColumnGrid) -> usize {
    let rows: Vec<&TrackedRow> = group.rows().collect();
    // The same steps [`frame_draw_group`] draws, measured the same way: a
    // command wrapping down the cell wants the rows it wraps onto, and
    // asking for one row a level would leave the block short of what it
    // would have drawn with the room.
    let ancestry = drawn_ancestry(group, &grid.view);
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
        AncestrySource::for_group(&ancestry, group, &grid.view),
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
    let layout = CellTableLayout::for_table_slots(grid, area, pane_background(false), rows);
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

/// Admit roster cells through the production measurements for cross-module tests.
#[cfg(test)]
pub(crate) fn draw_grid_for_test(
    grid: &mut TileGrid,
    roster: &Roster,
    area: Rect,
    growth: TileGrowth,
) {
    let stats = SccacheStats::default();
    let cells = Cells::new(roster, &[], CommandView::long(), &stats);
    let mut buffer = Buffer::empty(area);
    tui_pane::draw_tile_grid(
        &mut buffer,
        grid,
        area,
        growth,
        TileGridContents::Shown,
        &cells,
    );
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
                CommandView::long(),
            );
        },
    );
}

/// Draw cell contents through the same visible roster boundary as a frame.
#[cfg(test)]
fn draw_contents(
    buffer: &mut Buffer,
    roster: &Roster,
    content: &TileContent,
    inner: Rect,
    ground: Color,
    hidden_commands: &[String],
    view: &CommandView,
) {
    let visible = roster.clone().visible(hidden_commands);
    let roster = &visible;
    frame_draw_contents(
        buffer,
        roster,
        content,
        inner,
        ground,
        &FrameColumnGrids::for_frame(roster, view, u16::MAX),
        &FrameInvocationPlacement::listed(roster),
    );
}

#[cfg(test)]
fn frame_draw_contents(
    buffer: &mut Buffer,
    roster: &VisibleRoster,
    content: &TileContent,
    inner: Rect,
    ground: Color,
    grids: &FrameColumnGrids,
    placement: &FrameInvocationPlacement,
) {
    match content {
        TileContent::Summary => frame_draw_summary(
            buffer,
            &SummaryRows::of(&InvocationProcessRows::from(roster), placement),
            inner,
            ground,
            &grids.summary.0,
        ),
        TileContent::Group(id) => {
            frame_draw_group(buffer, roster, id, inner, ground, &grids.commands.0);
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
    hidden_commands: &[String],
    view: &CommandView,
) {
    let visible = roster.clone().visible(hidden_commands);
    let roster = &visible;
    let grids = FrameColumnGrids::for_frame(roster, view, inner.height);
    frame_draw_summary(
        buffer,
        &SummaryRows::of(
            &InvocationProcessRows::from(roster),
            &FrameInvocationPlacement::listed(roster),
        ),
        inner,
        ground,
        &grids.summary.0,
    );
}

/// Only listed invocations and their summary departures enter this plan.
enum SummaryRows<'a> {
    EmptyRoster,
    NoListedInvocations,
    Listed {
        rows:   Vec<&'a TrackedRow>,
        owners: Vec<InvocationId>,
    },
}

impl<'a> SummaryRows<'a> {
    fn of(rows: &'a InvocationProcessRows, placement: &FrameInvocationPlacement) -> Self {
        let mut listed = rows.listed_with_owners(placement);
        listed.sort_by_key(|(row, _)| row.process.started);
        if !listed.is_empty() {
            Self::Listed {
                rows:   listed.iter().map(|(row, _)| *row).collect(),
                owners: listed.into_iter().map(|(_, owner)| owner).collect(),
            }
        } else if rows.is_empty() {
            Self::EmptyRoster
        } else {
            Self::NoListedInvocations
        }
    }

    fn listed(&self) -> &[&'a TrackedRow] {
        match self {
            Self::Listed { rows, .. } => rows,
            Self::EmptyRoster | Self::NoListedInvocations => &[],
        }
    }

    fn demand(&self) -> SummaryDemand<InvocationId> {
        let header = usize::from(TABLE_HEADER_HEIGHT)
            + if matches!(self, Self::EmptyRoster) {
                usize::from(GROUP_HEADER_HEIGHT)
            } else {
                0
            };
        let owners = match self {
            Self::Listed { owners, .. } => owners.clone(),
            Self::EmptyRoster | Self::NoListedInvocations => Vec::new(),
        };
        SummaryDemand::new(header, owners)
    }
}

/// A terminal below the summary minimum reserves its last row for the omitted count.
enum SummaryFit<'a, 'row> {
    Whole(&'a [&'row TrackedRow]),
    Clipped {
        shown:   &'a [&'row TrackedRow],
        omitted: usize,
    },
}

impl<'a, 'row> SummaryFit<'a, 'row> {
    fn of(rows: &'a [&'row TrackedRow], available: u16) -> Self {
        if available == 0 {
            Self::Whole(&rows[..0])
        } else if rows.len() <= usize::from(available) {
            Self::Whole(rows)
        } else {
            let shown = &rows[..usize::from(available.saturating_sub(1))];
            Self::Clipped {
                shown,
                omitted: rows.len() - shown.len(),
            }
        }
    }

    const fn shown(&self) -> &'a [&'row TrackedRow] {
        match self {
            Self::Whole(rows) | Self::Clipped { shown: rows, .. } => rows,
        }
    }
}

fn frame_draw_summary(
    buffer: &mut Buffer,
    selection: &SummaryRows<'_>,
    inner: Rect,
    ground: Color,
    grid: &FrameTableColumnGrid,
) {
    if matches!(selection, SummaryRows::NoListedInvocations) {
        paint_summary_header_without_rows(buffer, inner, ground, grid);
        return;
    }
    let rows = selection.listed().to_vec();
    let groups = group_by_path(&rows, PinnedGroup::Unpinned);
    if summary_form(&rows, inner, groups.len(), grid) == SummaryForm::Detailed {
        frame_draw_process_table(buffer, inner, &rows, ground, PinnedGroup::Unpinned, grid);
        return;
    }
    let header_height = TABLE_HEADER_HEIGHT.min(inner.height.saturating_sub(1));
    let available = inner.height.saturating_sub(header_height);
    let fit = SummaryFit::of(&rows, available);
    let visible = fit.shown();
    // Compact rows have no mandatory directory heading. Size their optional
    // columns from the rows that fit, independently of the detailed-table plan.
    let layout = CellTableLayout::for_table_slots(
        grid,
        Rect {
            height: u16::MAX,
            ..inner
        },
        ground,
        visible,
    );
    let PreparedCompactSummary {
        rows: prepared,
        plan,
    } = PreparedCompactSummary::new(visible, available, &layout, grid);
    let area = indented(inner);
    let headings = TableAccountHeadings::of(
        &groups,
        cleared(inner)
            .width
            .saturating_sub(cell_width(SECTION_HEADER_INDENT)),
    );
    let mut spare_headings = match &fit {
        SummaryFit::Whole(_) => {
            available.saturating_sub(u16::try_from(prepared.len()).unwrap_or(u16::MAX))
        },
        SummaryFit::Clipped { .. } => 0,
    };
    let mut headed = Vec::new();
    let mut top = area.y.saturating_add(header_height);
    for row in prepared {
        if spare_headings > 0
            && let Some((index, group)) = groups.iter().enumerate().find(|(_, group)| {
                group
                    .rows
                    .iter()
                    .any(|source| std::ptr::eq(*source, row.source))
            })
            && !headed.contains(&index)
        {
            draw_path_group(
                buffer,
                Rect {
                    y: top,
                    height: 1,
                    ..inner
                },
                group,
                &layout,
                Vec::new(),
                &headings,
            );
            headed.push(index);
            spare_headings -= 1;
            top = top.saturating_add(1);
        }
        paint_compact_summary_row(
            buffer,
            Rect {
                y: top,
                height: 1,
                ..area
            },
            row,
            &layout,
            grid,
        );
        top = top.saturating_add(1);
    }
    paint_table_header(
        buffer,
        Rect {
            height: header_height,
            ..area
        },
        &layout,
        heading_fade(&rows),
        std::array::from_fn(|column| plan.labels_column(column)),
    );
    if let SummaryFit::Clipped { omitted, .. } = fit {
        paint_summary_overflow(buffer, inner, top, omitted, &layout, heading_fade(&rows));
    }
}

fn paint_summary_overflow(
    buffer: &mut Buffer,
    inner: Rect,
    top: u16,
    omitted: usize,
    layout: &CellTableLayout,
    faded: u8,
) {
    mark_table_continuation(
        buffer,
        inner,
        layout,
        TableContinuation::RowsWereOmitted,
        faded,
    );
    Line::from(format!("{omitted} more"))
        .style(
            Style::default()
                .fg(secondary_text_color())
                .bg(layout.ground),
        )
        .render(
            Rect {
                y: top,
                height: inner.height.min(1),
                ..indented(inner)
            },
            buffer,
        );
    if top < inner.bottom() {
        let right = inner
            .right()
            .saturating_sub(CONTENT_RIGHT_INSET)
            .saturating_sub(1);
        buffer[(right, top)].set_symbol(ELISION);
    }
}

fn summary_form(
    rows: &[&TrackedRow],
    inner: Rect,
    group_count: usize,
    grid: &FrameTableColumnGrid,
) -> SummaryForm {
    if rows.is_empty()
        || usize::from(inner.height)
            >= frame_table_height(rows, inner.width, PinnedGroup::Unpinned, grid).saturating_sub(
                group_count
                    .saturating_sub(1)
                    .saturating_mul(usize::from(GROUP_GAP_HEIGHT)),
            )
    {
        SummaryForm::Detailed
    } else {
        SummaryForm::Compact
    }
}

fn paint_summary_header_without_rows(
    buffer: &mut Buffer,
    inner: Rect,
    background: Color,
    grid: &FrameTableColumnGrid,
) {
    let layout = CellTableLayout::for_table_slots(grid, inner, background, &[]);
    paint_table_header(
        buffer,
        indented(inner),
        &layout,
        0,
        [HeaderLabel::Shown; TABLE_HEADERS.len()],
    );
}

/// Paint and receipt use the same one-row compact allocation.
fn paint_compact_summary_row(
    buffer: &mut Buffer,
    area: Rect,
    row: LaidOutProcessRow<'_>,
    layout: &CellTableLayout,
    grid: &FrameTableColumnGrid,
) {
    if let Ok(span) = PaintedRowSpan::new(row.source.process.invocation_id.clone(), area) {
        grid.painted_rows.borrow_mut().push(ProcessRowPaint {
            pid: row.source.process.pid,
            span,
        });
    }
    paint_process_rows(buffer, area, vec![row], layout);
}

/// Compact summary rows and their attributed readings share one visibility plan.
struct PreparedCompactSummary<'a> {
    rows: Vec<LaidOutProcessRow<'a>>,
    plan: ProcessTablePlan<'a>,
}

impl<'a> PreparedCompactSummary<'a> {
    fn new(
        rows: &[&'a TrackedRow],
        available: u16,
        layout: &CellTableLayout,
        grid: &FrameTableColumnGrid,
    ) -> Self {
        let prepared: Vec<_> = rows
            .iter()
            .take(usize::from(available))
            .map(|row| {
                let mut row = process_row(row, layout);
                row.cells[COMMAND_COLUMN] = Text::from(row.cut_command_continuation(1));
                row.lines = 1;
                row
            })
            .collect();
        let row_visibility = prepared.iter().fold(
            [HeaderLabel::Hidden; TABLE_HEADERS.len()],
            |mut seen, row| {
                for (index, cell) in row.cells.iter().enumerate() {
                    seen[index].include(!cell.to_string().is_empty());
                }
                seen
            },
        );
        let mut plan = ProcessTablePlan {
            separators:                  TableSeparators {
                first:  0,
                groups: 0,
            },
            visible_group_count:         0,
            visible_rows:                PlannedVisibleRows::ProcessRows(
                prepared.iter().map(|row| row.source).collect(),
            ),
            body_rows_drawn:             u16::try_from(prepared.len()).unwrap_or(u16::MAX),
            body_rows_required:          u16::try_from(prepared.len()).unwrap_or(u16::MAX),
            header_and_value_visibility: row_visibility,
        };
        let mut drawn_groups = [prepared];
        apply_planned_row_readings(&mut drawn_groups, &mut plan, layout, grid);
        let [prepared] = drawn_groups;
        Self {
            rows: prepared,
            plan,
        }
    }
}

/// One lead per visible command group, shared by measurement and drawing.
fn summary_rows(roster: &VisibleRoster) -> Vec<&TrackedRow> {
    roster.groups().iter().map(|group| &group.lead).collect()
}

/// The summary's memory total: every running command's group total.
fn memory_total(roster: &VisibleRoster) -> SummaryMemoryTotal {
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

/// One visible command's ancestry and invocation table.
#[cfg(test)]
fn draw_group(
    buffer: &mut Buffer,
    roster: &Roster,
    id: &InvocationId,
    inner: Rect,
    ground: Color,
    hidden_commands: &[String],
    view: &CommandView,
) {
    let visible = roster.clone().visible(hidden_commands);
    let roster = &visible;
    frame_draw_group(
        buffer,
        roster,
        id,
        inner,
        ground,
        &FrameColumnGrids::for_frame(roster, view, u16::MAX)
            .commands
            .0,
    );
}

#[cfg(test)]
fn frame_draw_group(
    buffer: &mut Buffer,
    roster: &VisibleRoster,
    id: &InvocationId,
    inner: Rect,
    ground: Color,
    grid: &FrameTableColumnGrid,
) {
    let rows = InvocationProcessRows::from(roster);
    frame_draw_group_rows(buffer, roster, id, inner, ground, grid, &rows.measured(id));
}

fn frame_draw_group_rows(
    buffer: &mut Buffer,
    roster: &VisibleRoster,
    id: &InvocationId,
    inner: Rect,
    ground: Color,
    grid: &FrameTableColumnGrid,
    rows: &[&TrackedRow],
) {
    let Some(group) = roster.groups().iter().find(|group| &group.id == id) else {
        return;
    };
    let ancestry = drawn_ancestry(group, &grid.view);
    // The chain and table fade together when their command finishes.
    let faded = group_fade(group, rows);
    // The same measurement [`frame_group_height`] made when the cell
    // asked for its room, so the block is given back exactly what the
    // ask left it.
    let table_rows = frame_table_height(
        rows,
        inner.width,
        PinnedGroup::Lead(&group.lead.process),
        grid,
    );
    let used = draw_ancestry_source(
        buffer,
        inner,
        AncestrySource::for_group(&ancestry, group, &grid.view),
        faded,
        ground,
        grid.ancestry_table_budget(id, inner.height, table_rows),
    );
    let table = Rect {
        y: inner.y.saturating_add(used),
        height: inner.height.saturating_sub(used),
        ..inner
    };
    // The command's directory stays above its invocation table.
    let table_grid = table_grid_after_ancestry(grid, used);
    frame_draw_process_table(
        buffer,
        table,
        rows,
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
    if rows_drawn == 0 && grid.view.hierarchy == ProcessHierarchy::Hidden {
        let mut table_grid = grid.clone();
        table_grid.parent_links = AncestryParentLinks::default();
        table_grid.parent_content = ParentColumnContent::NamesParents;
        Cow::Owned(table_grid)
    } else {
        Cow::Borrowed(grid)
    }
}

/// The steps above the group's table, omitting pass-through ancestors.
///
/// Read here rather than off [`crate::roster::TrackedGroup::ancestry`]
/// directly so the cell's height is asked for against the steps it will
/// actually draw. Counting the raw chain asked for rows that
/// [`carried`] then dropped.
///
/// The hidden view reserves no ancestry rows; argument detail applies equally
/// to ancestors and command rows when the hierarchy is shown.
fn drawn_ancestry(group: &TrackedGroup, view: &CommandView) -> AncestryWalk<Ancestor> {
    let chain = match view.hierarchy {
        ProcessHierarchy::Shown => carried(group.ancestry().as_slice().to_vec())
            .into_iter()
            .map(|mut ancestor| {
                ancestor.command = match view.arguments {
                    ArgumentDetail::Default => {
                        ancestor.command.with_default_arguments(&view.defaults)
                    },
                    ArgumentDetail::All => ancestor.command,
                };
                ancestor
            })
            .collect(),
        ProcessHierarchy::Hidden => return AncestryWalk::Ended(Vec::new()),
    };
    match group.ancestry() {
        AncestryWalk::Ended(_) => AncestryWalk::Ended(chain),
        AncestryWalk::LimitCut(_) => AncestryWalk::LimitCut(chain),
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
#[cfg(test)]
fn draw_ancestry(
    buffer: &mut Buffer,
    area: Rect,
    ancestry: &[Ancestor],
    faded: u8,
    ground: Color,
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
        table,
    )
}

fn draw_ancestry_source(
    buffer: &mut Buffer,
    area: Rect,
    source: AncestrySource<'_>,
    faded: u8,
    ground: Color,
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
    let indentation = AncestryIndentation::of(&levels);
    let mut blocks: Vec<Vec<Line<'static>>> = levels
        .iter()
        .enumerate()
        .map(|(level, ancestor)| {
            if matches!(source.wrapping, AncestryWrapping::SingleRow) {
                vec![short_ancestry_line(
                    *ancestor,
                    level,
                    area.width,
                    pid,
                    command,
                    &indentation,
                    source.visibility,
                )]
            } else {
                ancestry_lines(*ancestor, level, area.width, pid, command, &indentation)
            }
        })
        .collect();
    let mut remaining = budget.max(1);
    let count = blocks.len();
    for (index, lines) in blocks.iter_mut().enumerate() {
        let reserved = count.saturating_sub(index).saturating_sub(1);
        let kept = lines.len().min(remaining.saturating_sub(reserved));
        if lines.len() > kept {
            let prefix = lines[0]
                .spans
                .iter()
                .take(3)
                .map(|span| cell_width(&span.content))
                .fold(0, u16::saturating_add);
            let ancestor = match levels[index] {
                AncestryLevel::Ancestor(ancestor)
                | AncestryLevel::AncestorAfterElision(ancestor) => ancestor,
            };
            let wrapped = wrapped_command(
                ancestor_command_words(ancestor, Style::default().fg(command)),
                area.width.saturating_sub(prefix),
                Style::default().fg(command),
            )
            .into_rows(kept);
            for (index, (line, text)) in lines.iter_mut().zip(wrapped).enumerate() {
                line.spans.truncate(if index == 0 { 3 } else { 1 });
                // Continuation lines have only their hanging indentation span.
                line.spans.extend(text.spans);
            }
            lines.truncate(kept);
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
    let name = elide_summary_end(&ancestor.command.display_line(), room);
    Line::from(vec![
        Span::styled(indent, Style::default().fg(command)),
        Span::styled(label, Style::default().fg(pid)),
        Span::raw(separator),
        Span::styled(name, Style::default().fg(command)),
        Span::styled(suffix, Style::default().fg(command)),
    ])
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
    let TableContinuation::CommandWasCut { line } = continuation else {
        return;
    };
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
    fn for_tree(walk: &'a AncestryWalk<Ancestor>, view: &CommandView) -> Self {
        let mut source = Self::from(walk);
        if view.hierarchy == ProcessHierarchy::Hidden && source.levels.len() == 1 {
            source.wrapping = AncestryWrapping::SingleRow;
        }
        source
    }

    fn for_group(
        walk: &'a AncestryWalk<Ancestor>,
        group: &TrackedGroup,
        view: &CommandView,
    ) -> Self {
        let mut source = Self::for_tree(walk, view);
        let hidden = view.hierarchy == ProcessHierarchy::Hidden
            && walk.as_slice().last().is_some_and(|kept| {
                group
                    .ancestry()
                    .as_slice()
                    .last()
                    .is_some_and(|last| last.pid != kept.pid)
            });
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
    /// Separator rows that do not carry ancestry content.
    gap:      u16,
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
            gap:      0,
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
    AncestryRowUse {
        drawn,
        required,
        gap: plan.gap,
    }
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
    wrapped_command(
        ancestor_command_words(ancestor, Style::default()),
        room,
        cut_mark_style(Color::Reset, 0),
    )
    .lines
    .len()
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
        ancestor_command_words(ancestor, Style::default().fg(command)),
        room,
        Style::default().fg(command),
    );
    wrapped
        .into_text()
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

impl<'a> PathGroup<'a> {
    /// Measure this group's process rows with the shared frame columns.
    fn measured_rows(&self, layout: &CellTableLayout) -> Vec<LaidOutProcessRow<'a>> {
        self.rows
            .iter()
            .map(|row| measured_process_row(row, layout))
            .collect()
    }
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

/// Text and command-name demand before any cell width is assigned.
struct ProcessRowValues {
    /// Column metadata; numeric display text is deferred during row selection.
    cells: [String; TABLE_HEADERS.len()],
}

impl ProcessRowValues {
    /// Read every value independently of the eventual viewport.
    fn of(source: &TrackedRow, view: &CommandView) -> Self {
        let process = &source.process;
        let command = displayed_command(&process.command, view);
        let compiler = match &process.resource_use.compiler {
            CompilerObservation::Unknown | CompilerObservation::None => String::new(),
            CompilerObservation::Running(compiler) => {
                format!("{}×{}", compiler.name, compiler.count)
            },
        };
        let cpu = cpu_text(process.resource_use.cpu);
        let memory = memory_text(process.resource_use.memory);
        let state = if state_width(&process.state) > 0 {
            STATE_BLOCKED.to_string()
        } else {
            String::new()
        };
        Self {
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
        }
    }
}

/// Metadata widths held for the lifetime of the terminal session.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
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
    if let Some(cores) = linux_processors::online_processors() {
        return cores.get();
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

/// Intrinsic inputs for the command tables of one immutable frame.
struct FrameCommandColumnGrid(FrameTableColumnGrid);

/// Intrinsic inputs for the summary's distinct column set in one frame.
struct FrameSummaryColumnGrid(FrameTableColumnGrid);

/// The two independent grids read by demands, drawing and row probes.
struct FrameColumnGrids {
    /// Every row belonging to a command cell participates in this grid.
    commands: FrameCommandColumnGrid,
    /// Only visible summary rows participate in this grid.
    summary:  FrameSummaryColumnGrid,
}

impl FrameColumnGrids {
    #[cfg(test)]
    fn new(roster: &Roster, hidden_commands: &[String], view: &CommandView) -> Self {
        Self::for_frame(&roster.clone().visible(hidden_commands), view, u16::MAX)
    }

    /// Fit intrinsic widths once over the rows this frame can possibly draw.
    fn for_frame(roster: &VisibleRoster, view: &CommandView, height: u16) -> Self {
        let mut command_rows = Vec::new();
        let mut command_tables = Vec::new();
        let mut command_progress = Vec::new();
        let mut parent_links = AncestryParentLinks::default();
        for group in roster.groups() {
            let rows: Vec<_> = group.rows().collect();
            command_progress.extend(invocation_progress_rows(&rows, TableKind::Command));
            if view.hierarchy == ProcessHierarchy::Shown {
                let ancestry = drawn_ancestry(group, view);
                for row in &rows {
                    if let VisibleParent::Ancestor(pid) = row.process.parent
                        && ancestry
                            .as_slice()
                            .last()
                            .is_none_or(|ancestor| ancestor.pid != pid)
                    {
                        Rc::make_mut(&mut parent_links.hidden)
                            .push(row.process.invocation_id.clone());
                    }
                }
            }
            let groups = group_by_path(&rows, PinnedGroup::Lead(&group.lead.process));
            let possible = possible_frame_rows(&groups, height);
            command_tables.push(possible.clone());
            command_rows.extend(possible);
        }
        let summary = summary_rows(roster);
        let summary_progress = invocation_progress_rows(&summary, TableKind::Summary);
        let summary_groups = group_by_path(&summary, PinnedGroup::Unpinned);
        let summary = possible_frame_rows(&summary_groups, height);
        let contributions = Rc::new(IndexedResourceReadings::new(
            roster
                .groups()
                .iter()
                .flat_map(TrackedGroup::rows)
                .map(InvocationContribution::from)
                .collect(),
        ));
        let mut commands = FrameTableColumnGrid::with_parent_links(
            &command_rows,
            TableKind::Command,
            view,
            parent_links,
            command_progress,
            Rc::clone(&contributions),
        );
        let mut summary = FrameTableColumnGrid::with_parent_links(
            &summary,
            TableKind::Summary,
            view,
            AncestryParentLinks::default(),
            summary_progress,
            Rc::clone(&contributions),
        );
        summary.layouts = Rc::clone(&commands.layouts);
        let family_inks = FrameFamilyInks::shared();
        let heights = PreparedCommandHeights::next_frame();
        let styled = PreparedStyledRows::next_frame();
        for grid in [&mut commands, &mut summary] {
            grid.prepared_rows.borrow_mut().heights = Rc::clone(&heights);
            grid.prepared_rows.borrow_mut().by_pid = Rc::clone(&styled);
            grid.family_inks = Rc::clone(&family_inks);
            grid.intrinsic_widths[CPU_COLUMN] =
                grid.intrinsic_widths[CPU_COLUMN].max(roster.column_widths().cpu);
            grid.intrinsic_widths[MEMORY_COLUMN] =
                grid.intrinsic_widths[MEMORY_COLUMN].max(roster.column_widths().memory);
            grid.intrinsic_widths[COMPILER_COLUMN] =
                grid.intrinsic_widths[COMPILER_COLUMN].max(roster.column_widths().compiler);
        }
        if !roster.groups().is_empty() {
            summary.separation = DirectorySeparation::YieldEarliest;
        }
        commands.fit_planned_compilers(&command_rows);
        commands.cell_gap.set(CellTableGap::Unprepared);
        commands.table_budgets = Rc::new(
            command_tables
                .iter()
                .map(|rows| {
                    let local = commands.for_visible_rows(rows);
                    TableWidthBudget::of(rows, local.state_column, view)
                })
                .collect(),
        );
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

/// Parent links whose ancestry endpoint remains visible in this view view.
#[derive(Clone, Default)]
struct AncestryParentLinks {
    hidden: Rc<Vec<InvocationId>>,
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
#[derive(Clone, PartialEq)]
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
#[derive(Clone)]
struct RowResourceReading {
    invocation: InvocationId,
    pid:        u32,
    resources:  InvocationResourceUse,
}

/// Immutable frame contributions indexed once, with attribution shared by identical row sets.
struct IndexedResourceReadings {
    contributions: Vec<InvocationContribution>,
    ancestry:      OnceCell<ContributionAncestry>,
    readings:      RefCell<Vec<AttributedRowSet>>,
}

/// Resource readings cached for an ordered set of visible invocation contributions.
struct AttributedRowSet {
    contributions: Vec<InvocationContribution>,
    readings:      Rc<Vec<RowResourceReading>>,
}

/// Parent relationships are needed only when this frame grid attributes resources.
struct ContributionAncestry {
    ancestors:   HashMap<(InvocationId, u32), usize>,
    descendants: HashMap<(InvocationId, u32), Vec<usize>>,
}

impl ContributionAncestry {
    fn new(contributions: &[InvocationContribution]) -> Self {
        let ancestors: HashMap<_, _> = contributions
            .iter()
            .enumerate()
            .map(|(index, row)| ((row.invocation.clone(), row.pid), index))
            .collect();
        let mut descendants: HashMap<_, Vec<_>> = HashMap::new();
        for (index, row) in contributions.iter().enumerate() {
            let mut parent = &row.parent;
            let mut visited = Vec::new();
            while let VisibleParent::Invocation { id, pid } = parent {
                let key = (id.clone(), *pid);
                if visited.contains(&key) {
                    break;
                }
                visited.push(key.clone());
                descendants.entry(key.clone()).or_default().push(index);
                let Some(&ancestor) = ancestors.get(&key) else {
                    break;
                };
                parent = &contributions[ancestor].parent;
            }
        }
        Self {
            ancestors,
            descendants,
        }
    }
}

impl IndexedResourceReadings {
    const fn new(contributions: Vec<InvocationContribution>) -> Self {
        Self {
            contributions,
            ancestry: OnceCell::new(),
            readings: RefCell::new(Vec::new()),
        }
    }

    fn for_rows(&self, rows: &[&TrackedRow]) -> Rc<Vec<RowResourceReading>> {
        if let Some(known) = self.readings.borrow().iter().find(|known| {
            known.contributions.len() == rows.len()
                && known.contributions.iter().zip(rows).all(|(known, row)| {
                    known.invocation == row.process.invocation_id
                        && known.pid == row.process.pid
                        && known.parent == row.process.parent
                        && known.resources == row.process.resource_use
                        && known.ended == row.is_ended()
                })
        }) {
            return Rc::clone(&known.readings);
        }
        let ancestry = self
            .ancestry
            .get_or_init(|| ContributionAncestry::new(&self.contributions));
        let key: Vec<_> = rows
            .iter()
            .copied()
            .map(InvocationContribution::from)
            .collect();
        let indices: HashMap<_, _> = rows
            .iter()
            .enumerate()
            .map(|(index, row)| ((&row.process.invocation_id, row.process.pid), index))
            .collect();
        let mut assigned: Vec<_> = rows
            .iter()
            .map(|row| {
                vec![(
                    sysinfo::Pid::from_u32(row.process.pid),
                    row.process.resource_use.clone(),
                )]
            })
            .collect();
        let mut candidates = Vec::new();
        for row in rows {
            if let Some(descendants) = ancestry
                .descendants
                .get(&(row.process.invocation_id.clone(), row.process.pid))
            {
                for &index in descendants {
                    if !candidates.contains(&index) {
                        candidates.push(index);
                    }
                }
            }
        }
        candidates.sort_unstable();
        for index in candidates {
            let contribution = &self.contributions[index];
            if indices.contains_key(&(&contribution.invocation, contribution.pid)) {
                continue;
            }
            let mut parent = &contribution.parent;
            for _ in 0..=self.contributions.len() {
                let VisibleParent::Invocation { id, pid } = parent else {
                    break;
                };
                if let Some(&index) = indices.get(&(id, *pid)) {
                    if contribution.ended == rows[index].is_ended() {
                        assigned[index].push((
                            sysinfo::Pid::from_u32(contribution.pid),
                            contribution.resources.clone(),
                        ));
                    }
                    break;
                }
                let Some(&ancestor) = ancestry.ancestors.get(&(id.clone(), *pid)) else {
                    break;
                };
                parent = &self.contributions[ancestor].parent;
            }
        }
        let readings = Rc::new(
            rows.iter()
                .zip(assigned)
                .map(|(row, resources)| RowResourceReading {
                    invocation: row.process.invocation_id.clone(),
                    pid:        row.process.pid,
                    resources:  census::sum_resource_use_once_per_pid(resources),
                })
                .collect(),
        );
        self.readings.borrow_mut().push(AttributedRowSet {
            contributions: key,
            readings:      Rc::clone(&readings),
        });
        readings
    }
}

/// Standalone attribution for resource regression fixtures.
#[cfg(test)]
fn row_resource_readings(
    rows: &[&TrackedRow],
    contributions: &[InvocationContribution],
) -> Vec<RowResourceReading> {
    IndexedResourceReadings::new(contributions.to_vec())
        .for_rows(rows)
        .as_ref()
        .clone()
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

/// Inputs that determine one frame's column reductions.
#[derive(Eq, PartialEq)]
struct ColumnFitInputs {
    width:         u16,
    state:         TableStateColumn,
    budget_states: Vec<TableStateColumn>,
    slots:         Vec<usize>,
    empty_tail:    Vec<usize>,
    intrinsic:     [u16; TABLE_HEADERS.len()],
    values:        [u16; TABLE_HEADERS.len()],
    command:       u16,
    subcommand:    u16,
    title:         u16,
    clearance:     u16,
    row_end:       TableRowEnd,
    parent:        ParentColumnContent,
    arguments:     ArgumentDetail,
}

/// Columns and widths selected together by the frame's reduction policy.
#[derive(Clone)]
struct FittedTableColumns {
    columns:     Vec<usize>,
    constraints: Vec<Constraint>,
    value_width: u16,
}

/// One field set is shared by all command cells, including departures.
#[derive(Clone, Eq, PartialEq)]
struct CommandHeaderForm {
    columns: Vec<usize>,
}

impl CommandHeaderForm {
    fn columns(&self) -> &[usize] { &self.columns }
}

/// Width probes cannot replace a committed frame's field set.
#[derive(Clone, Eq, PartialEq)]
enum CommandFrameLayout {
    Unprepared,
    Prepared(CommandHeaderForm),
    Committed(CommandHeaderForm),
}

/// These row policies must stay fixed between the capacity proof and paint.
#[derive(Eq, PartialEq)]
struct FrameLayoutPolicy {
    header:         CommandFrameLayout,
    summary_end:    TableRowEnd,
    command_end:    TableRowEnd,
    gap:            CellTableGap,
    whole_commands: Vec<WholeShortCommandTable>,
}

/// Complete column slots and intrinsic widths before a cell drops or spaces them.
#[derive(Clone)]
struct FrameTableColumnGrid {
    /// Palette comparisons are shared by all parent and pid spans in this frame.
    family_inks:              Rc<RefCell<FrameFamilyInks>>,
    /// One gap for the command cells, independent of the summary's gap.
    cell_gap:                 Rc<Cell<CellTableGap>>,
    /// Shared row-end cells reserved only after a table omits process rows.
    row_end:                  Cell<TableRowEnd>,
    /// The frame decision remains shared when one whole Short table uses its edge.
    frame_row_end:            Rc<Cell<TableRowEnd>>,
    /// Fully drawn Short tables that need the edge cells for a whole command name.
    whole_short_tables:       Rc<RefCell<Vec<WholeShortCommandTable>>>,
    layouts:                  Rc<RefCell<Vec<SolvedTableColumns>>>,
    column_fits:              Rc<RefCell<Vec<(ColumnFitInputs, FittedTableColumns)>>>,
    command_layout:           Rc<RefCell<CommandFrameLayout>>,
    painted_rows:             Rc<RefCell<Vec<ProcessRowPaint>>>,
    retained_ancestry:        Rc<RefCell<HashMap<InvocationId, u16>>>,
    prepared_rows:            Rc<RefCell<PreparedProcessRows>>,
    prepared_plans:           Rc<RefCell<Vec<CachedTablePlan>>>,
    /// Disjoint resources stay available even when a height cut hides their row.
    contributions:            Rc<IndexedResourceReadings>,
    /// The frame-wide presence of optional slots, independent of visible rows.
    slots:                    Vec<usize>,
    /// Whether this table can lend directory separators independently.
    separation:               DirectorySeparation,
    /// Whether the parent slot names anything in this frame's row set.
    parent_content:           ParentColumnContent,
    /// Tree-selected ancestry links used by both values and width fitting.
    parent_links:             AncestryParentLinks,
    /// Invocations whose directory has more than one independent reading.
    row_progress:             Rc<Vec<InvocationId>>,
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
    table_budgets:            Rc<Vec<TableWidthBudget>>,
    /// Empty columns after the command can lend their cells to its argument tail.
    empty_tail_columns:       Vec<usize>,
    /// A progress reading has no removable state-column handle.
    state_column:             TableStateColumn,
    /// Summary and command cells use separate grids.
    kind:                     TableKind,
    /// Detail used to read the row texts that fitted this grid.
    view:                     CommandView,
}

impl FrameTableColumnGrid {
    fn ancestry_table_budget(&self, id: &InvocationId, height: u16, table: usize) -> usize {
        let retained = self
            .retained_ancestry
            .borrow()
            .get(id)
            .copied()
            .map_or(RetainedPrefix::Absent, RetainedPrefix::from);
        retained_ancestry_table_budget(height, table, retained)
    }

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
        for reading in self.contributions.for_rows(rows).iter() {
            if let CompilerObservation::Running(compiler) = &reading.resources.compiler {
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
    fn of(rows: &[&TrackedRow], kind: TableKind, view: &CommandView) -> Self {
        Self::with_parent_links(
            rows,
            kind,
            view,
            AncestryParentLinks::default(),
            invocation_progress_rows(rows, kind),
            Rc::new(IndexedResourceReadings::new(
                rows.iter()
                    .copied()
                    .map(InvocationContribution::from)
                    .collect(),
            )),
        )
    }

    fn with_parent_links(
        rows: &[&TrackedRow],
        kind: TableKind,
        view: &CommandView,
        parent_links: AncestryParentLinks,
        row_progress: Vec<InvocationId>,
        contributions: Rc<IndexedResourceReadings>,
    ) -> Self {
        let values: Vec<_> = rows
            .iter()
            .map(|row| {
                let mut values = ProcessRowValues::of(row, view);
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
        if view.arguments == ArgumentDetail::Default {
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
            family_inks: FrameFamilyInks::shared(),
            cell_gap: Rc::new(std::cell::Cell::new(CellTableGap::Independent)),
            row_end: std::cell::Cell::new(TableRowEnd::Unresolved),
            frame_row_end: Rc::new(std::cell::Cell::new(TableRowEnd::Unresolved)),
            whole_short_tables: std::rc::Rc::new(RefCell::new(Vec::new())),
            layouts: std::rc::Rc::new(RefCell::new(Vec::new())),
            column_fits: Rc::new(RefCell::new(Vec::new())),
            command_layout: Rc::new(RefCell::new(CommandFrameLayout::Unprepared)),
            painted_rows: Rc::new(RefCell::new(Vec::new())),
            retained_ancestry: Rc::new(RefCell::new(HashMap::new())),
            prepared_rows: std::rc::Rc::new(RefCell::new(PreparedProcessRows::default())),
            prepared_plans: std::rc::Rc::new(RefCell::new(Vec::new())),
            contributions,
            row_progress: Rc::new(row_progress),
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
            table_budgets: Rc::new(vec![TableWidthBudget::of(rows, state_column, view)]),
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
            view: view.clone(),
        }
    }

    /// Choose slot ownership before running the shared width policy.
    fn for_visible_rows(&self, rows: &[&TrackedRow]) -> Cow<'_, Self> {
        let empty_tail_columns: &[usize] = if rows.iter().all(|row| {
            !matches!(
                row.process.resource_use.compiler,
                CompilerObservation::Running(_)
            ) && !matches!(row.process.managed, Measurement::Reading(count) if count > 0)
        }) {
            &[COMPILER_COLUMN, MANAGED_COLUMN]
        } else {
            &[]
        };
        let has_progress = rows
            .iter()
            .any(|row| self.row_progress.contains(&row.process.invocation_id));
        let filled = rows.iter().any(|row| {
            state_width(&row.process.state) > 0
                || self.row_progress.contains(&row.process.invocation_id)
        });
        let state_column = if has_progress {
            TableStateColumn::ProgressReading
        } else if filled {
            TableStateColumn::FilledValue
        } else if self.kind == TableKind::Summary || !self.slots.contains(&STATE_COLUMN) {
            TableStateColumn::Absent
        } else {
            TableStateColumn::BlankAlignment
        };
        let removes_state =
            state_column == TableStateColumn::Absent && self.slots.contains(&STATE_COLUMN);
        let changes_budget = self.kind == TableKind::Summary
            && (self.table_budgets.len() != 1
                || self.table_budgets[0].state_column != state_column);
        if self.empty_tail_columns == empty_tail_columns
            && self.state_column == state_column
            && !removes_state
            && !changes_budget
        {
            return Cow::Borrowed(self);
        }
        let mut local = self.clone();
        local.empty_tail_columns = empty_tail_columns.to_vec();
        local.state_column = state_column;
        if removes_state {
            local.slots.retain(|column| *column != STATE_COLUMN);
        }
        if self.kind == TableKind::Summary {
            let mut budget = self.table_budgets[0].clone();
            budget.state_column = local.state_column;
            local.table_budgets = Rc::new(vec![budget]);
        }
        Cow::Owned(local)
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
        if self.kind == TableKind::Command
            && let CommandFrameLayout::Prepared(form) | CommandFrameLayout::Committed(form) =
                &*self.command_layout.borrow()
        {
            let columns = form.columns().to_vec();
            return FittedTableColumns {
                constraints: self.constraints(&columns),
                value_width: width.saturating_sub(self.allocation_clearance(&columns)),
                columns,
            };
        }
        let inputs = ColumnFitInputs {
            width,
            state: self.state_column,
            budget_states: self
                .table_budgets
                .iter()
                .map(|budget| budget.state_column)
                .collect(),
            slots: self.slots.clone(),
            empty_tail: self.empty_tail_columns.clone(),
            intrinsic: self.intrinsic_widths,
            values: self.value_widths,
            command: self.longest_command_width,
            subcommand: self.longest_subcommand_width,
            title: self.command_title_width,
            clearance: self.continuation_clearance,
            row_end: self.row_end.get(),
            parent: self.parent_content,
            arguments: self.view.arguments,
        };
        if let Some((_, fitted)) = self
            .column_fits
            .borrow()
            .iter()
            .find(|(key, _)| *key == inputs)
        {
            return fitted.clone();
        }
        let fitted = self.fit_command_columns(width);
        self.column_fits.borrow_mut().push((inputs, fitted.clone()));
        fitted
    }

    /// Solve once for each distinct column policy in this frame.
    fn fit_command_columns(&self, width: u16) -> FittedTableColumns {
        let mut budgets: Vec<TableWidthBudget> = Vec::new();
        for table in self.table_budgets.iter() {
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
            if self.view.arguments == ArgumentDetail::All && column == COMMAND_COLUMN {
                0
            } else {
                self.trailing_column_clearance(column)
            }
        })
    }

    /// Choose this table's gap after its column reductions.
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
        let mut states = Vec::new();
        for table in self.table_budgets.iter() {
            let index = states
                .iter()
                .position(|(state, _)| *state == table.state_column)
                .unwrap_or_else(|| {
                    let mut local = self.clone();
                    local.state_column = table.state_column;
                    let index = states.len();
                    states.push((table.state_column, local.columns_for_command_name(width)));
                    index
                });
            let fitted = &states[index].1;
            let Some(position) = fitted
                .columns
                .iter()
                .position(|&column| column == COMMAND_COLUMN)
            else {
                continue;
            };
            // Argument tails have already yielded to the subcommand in Short view.
            let command_width = match self.view.arguments {
                ArgumentDetail::Default => table.short.min(table.subcommand),
                ArgumentDetail::All => table.full.saturating_sub(TABLE_CONTINUATION_CLEARANCE),
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
        if self.view.arguments == ArgumentDetail::All
            && matches!(column, COMPILER_COLUMN | MANAGED_COLUMN)
        {
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
                    || (self.view.arguments == ArgumentDetail::All
                        && last == COMMAND_COLUMN
                        && has_memory)))
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
    fn of(rows: &[&TrackedRow], state_column: TableStateColumn, view: &CommandView) -> Self {
        Self {
            short: rows
                .iter()
                .map(|row| cell_width(&displayed_command(&row.process.command, view)))
                .max()
                .unwrap_or_default(),
            full: rows
                .iter()
                .map(|row| {
                    cell_width(&displayed_command(
                        &row.process.command,
                        CommandView::long(),
                    ))
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
fn frame_slot_columns(values: &[ProcessRowValues], kind: TableKind) -> Vec<usize> {
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
        .map(|row| cell_width(&displayed_command(&row.process.command, &grid.view)))
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
            remaining = remaining.saturating_sub(process_row_lines(row, &layout));
        }
    }
    false
}

/// Every input read by the table width solver.
#[derive(Clone, Eq, PartialEq)]
struct TableLayoutInputs {
    width:          u16,
    columns:        Vec<usize>,
    constraints:    Vec<Constraint>,
    column_spacing: u16,
}

/// A width solution and the complete inputs that produced it.
#[derive(Clone)]
struct SolvedTableColumns {
    inputs: TableLayoutInputs,
    widths: Vec<u16>,
}

impl TableLayoutInputs {
    fn of(grid: &FrameTableColumnGrid, width: u16) -> Self {
        let width = indented(Rect::new(0, 0, width, 0)).width;
        let FittedTableColumns {
            columns,
            mut constraints,
            value_width: table_width,
            ..
        } = grid.columns_for_command_name(width);
        let column_spacing = match grid.kind {
            TableKind::Summary => grid.preferred_column_spacing(width),
            TableKind::Command => match grid.cell_gap.get() {
                CellTableGap::Independent => grid.preferred_column_spacing(width),
                CellTableGap::Unprepared => grid.cell_column_spacing(width),
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
        Self {
            width: table_width,
            columns,
            constraints,
            column_spacing,
        }
    }
}

impl SolvedTableColumns {
    fn new(inputs: TableLayoutInputs) -> Self {
        let widths = solved_column_widths(inputs.width, &inputs.constraints, inputs.column_spacing);
        Self { inputs, widths }
    }
}

/// Wrapped rows indexed by pid before checking full invocation identity and layout.
#[derive(Default)]
struct PreparedProcessRows {
    by_pid:  Rc<RefCell<PreparedStyledRows>>,
    heights: Rc<RefCell<PreparedCommandHeights>>,
}

/// Retain styled rows only while the current or preceding frame uses them.
#[derive(Default)]
struct PreparedStyledRows {
    previous: HashMap<u32, Vec<Rc<CachedProcessRow>>>,
    current:  HashMap<u32, Vec<Rc<CachedProcessRow>>>,
}

impl PreparedStyledRows {
    fn next_frame() -> Rc<RefCell<Self>> {
        thread_local! {
            static ROWS: Rc<RefCell<PreparedStyledRows>> = Rc::new(RefCell::new(PreparedStyledRows::default()));
        }
        ROWS.with(|shared| {
            let mut rows = shared.borrow_mut();
            rows.previous = std::mem::take(&mut rows.current);
            Rc::clone(shared)
        })
    }

    fn get(
        &mut self,
        row: &TrackedRow,
        layout: &CellTableLayout,
        parent_column: ParentColumn,
    ) -> Option<Rc<CachedProcessRow>> {
        let matches = |cached: &&Rc<CachedProcessRow>| cached.matches(row, layout, parent_column);
        if let Some(cached) = self
            .current
            .get(&row.process.pid)
            .and_then(|rows| rows.iter().find(matches))
        {
            return Some(Rc::clone(cached));
        }
        let cached = self
            .previous
            .get(&row.process.pid)
            .and_then(|rows| rows.iter().find(matches))
            .cloned()?;
        self.current
            .entry(row.process.pid)
            .or_default()
            .push(Rc::clone(&cached));
        Some(cached)
    }
}

/// Only the preceding and current frame's wrapping inputs remain cached.
#[derive(Default)]
struct PreparedCommandHeights {
    previous: HashMap<u16, Vec<PreparedCommandHeight>>,
    current:  HashMap<u16, Vec<PreparedCommandHeight>>,
}

impl PreparedCommandHeights {
    fn next_frame() -> Rc<RefCell<Self>> {
        thread_local! {
            static HEIGHTS: Rc<RefCell<PreparedCommandHeights>> = Rc::new(RefCell::new(PreparedCommandHeights::default()));
        }
        HEIGHTS.with(|shared| {
            let mut heights = shared.borrow_mut();
            heights.previous = std::mem::take(&mut heights.current);
            Rc::clone(shared)
        })
    }
}

/// Wrapping depends on command text and width, independently of row inks.
#[derive(Clone)]
struct PreparedCommandHeight {
    command: CommandText,
    view:    CommandView,
    lines:   u16,
}

/// A cached row's colours depend on its invocation's lifetime.
#[derive(Clone, Copy, Eq, PartialEq)]
enum ProcessRowLifetime {
    Live,
    Ended,
}

impl ProcessRowLifetime {
    const fn of(row: &TrackedRow) -> Self {
        if row.is_ended() {
            Self::Ended
        } else {
            Self::Live
        }
    }
}

/// A cached row either owns a displayed progress reading or has none.
#[derive(Clone, Copy, Eq, PartialEq)]
enum ProcessRowProgress {
    Displayed,
    Absent,
}

impl ProcessRowProgress {
    fn of(row: &TrackedRow, layout: &CellTableLayout) -> Self {
        if layout.row_progress.contains(&row.process.invocation_id) {
            Self::Displayed
        } else {
            Self::Absent
        }
    }
}

/// Styled row cells prepared once for one solved layout.
#[derive(Clone)]
struct CachedProcessRow {
    process:       CargoProcess,
    faded:         u8,
    cells:         [Text<'static>; TABLE_HEADERS.len()],
    lines:         u16,
    widths:        Vec<u16>,
    columns:       Vec<usize>,
    ground:        Color,
    parent_column: ParentColumn,
    lifetime:      ProcessRowLifetime,
    progress:      ProcessRowProgress,
    view:          CommandView,
    theme:         Arc<Theme>,
}

impl CachedProcessRow {
    fn matches(
        &self,
        row: &TrackedRow,
        layout: &CellTableLayout,
        parent_column: ParentColumn,
    ) -> bool {
        self.process == row.process
            && self.faded == row.faded()
            && self.widths == layout.column_widths
            && self.columns == layout.columns
            && self.ground == layout.ground
            && self.parent_column == parent_column
            && self.lifetime == ProcessRowLifetime::of(row)
            && self.progress == ProcessRowProgress::of(row, layout)
            && self.view == layout.view
            && Arc::ptr_eq(&self.theme, &tui_pane::theme())
    }
}

/// Resolved family colors remain valid only for the complete active theme.
#[derive(Default)]
struct FrameFamilyInks {
    theme:  Option<Arc<Theme>>,
    colors: HashMap<usize, Color>,
}

impl FrameFamilyInks {
    fn shared() -> Rc<RefCell<Self>> {
        thread_local! {
            static FAMILY_INKS: Rc<RefCell<FrameFamilyInks>> = Rc::new(RefCell::new(FrameFamilyInks::default()));
        }
        FAMILY_INKS.with(Rc::clone)
    }

    fn color(&mut self, index: usize) -> Color {
        let active = tui_pane::theme();
        if self
            .theme
            .as_ref()
            .is_none_or(|theme| !Arc::ptr_eq(theme, &active))
        {
            self.colors.clear();
            self.theme = Some(active);
        }
        *self
            .colors
            .entry(index)
            .or_insert_with(|| theme::family_color(index))
    }
}

/// The command cells' gap is settled before any cell in their frame is painted.
#[derive(Clone, Copy, Eq, PartialEq)]
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
#[derive(Clone, Eq, PartialEq)]
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
    let mut local = grid.for_visible_rows(rows);
    let grid = local.as_ref();
    if grid.row_end.get() == TableRowEnd::Reserved
        && grid.whole_short_tables.borrow().iter().any(|table| {
            table.width == width
                && table.rows.len() == rows.len()
                && table.rows.iter().zip(rows).all(|((id, pid), row)| {
                    *id == row.process.invocation_id && *pid == row.process.pid
                })
        })
    {
        local.to_mut().row_end.set(TableRowEnd::Unreserved);
    }
    local
}

/// How one cell's tables are laid out, settled once for the whole cell.
///
/// Every group in the cell is drawn against this, which is what keeps
/// the tables lined up down the cell instead of each one fitting itself
/// and the columns stepping in and out as the eye moves between them.
#[derive(Clone)]
struct CellTableLayout {
    family_inks:    Rc<RefCell<FrameFamilyInks>>,
    #[cfg(test)]
    table_width:    u16,
    painted_rows:   Rc<RefCell<Vec<ProcessRowPaint>>>,
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
    row_progress:   Rc<Vec<InvocationId>>,
    /// Ancestry links retained in the frame's selected view view.
    parent_links:   AncestryParentLinks,
    /// How much of each row's command line the cell prints.

    /// Whether a row spells out its whole command line or only the
    /// name of what runs.
    view:           CommandView,
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
        view: &CommandView,
    ) -> Self {
        Self::from_grid(&FrameTableColumnGrid::of(rows, kind, view), area, ground)
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
        let local;
        let grid = if grid.kind == TableKind::Summary {
            let visible =
                possible_frame_rows(&group_by_path(rows, PinnedGroup::Unpinned), area.height);
            local = grid.for_visible_rows(&visible);
            &local
        } else {
            grid.as_ref()
        };
        Self::from_grid(grid, area, ground)
    }

    /// Lay out one frame grid at this cell's actual inner width.
    fn from_grid(grid: &FrameTableColumnGrid, area: Rect, ground: Color) -> Self {
        let inputs = TableLayoutInputs::of(grid, area.width);
        let cached = grid
            .layouts
            .borrow()
            .iter()
            .find(|cached| cached.inputs == inputs)
            .cloned();
        let solved = cached.unwrap_or_else(|| {
            let solved = SolvedTableColumns::new(inputs);
            grid.layouts.borrow_mut().push(solved.clone());
            solved
        });
        let TableLayoutInputs {
            columns,
            constraints,
            column_spacing,
            ..
        } = solved.inputs;
        let column_widths = solved.widths;
        Self {
            family_inks: Rc::clone(&grid.family_inks),
            #[cfg(test)]
            table_width: area.width,
            painted_rows: Rc::clone(&grid.painted_rows),
            prepared_rows: std::rc::Rc::clone(&grid.prepared_rows),
            prepared_plans: std::rc::Rc::clone(&grid.prepared_plans),
            kind: grid.kind,
            row_progress: grid.row_progress.clone(),
            parent_links: grid.parent_links.clone(),
            constraints,
            columns,
            column_widths,
            column_spacing,

            view: grid.view.clone(),
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
    let PlannedVisibleRows::ProcessRows(visible) = &plan.visible_rows else {
        return;
    };
    let readings = grid.contributions.for_rows(visible);
    let family_links = AllocationFamilyLinks::from_plan(plan);
    let shows_cpu = layout.columns.contains(&CPU_COLUMN);
    let shows_memory = layout.columns.contains(&MEMORY_COLUMN);
    let shows_compiler = layout.columns.contains(&COMPILER_COLUMN);
    let shows_parent = layout.columns.contains(&PARENT_COLUMN);
    let shows_pid = layout.columns.contains(&PID_COLUMN);
    for row in drawn_groups.iter_mut().flatten().filter(|row| {
        visible
            .iter()
            .any(|source| std::ptr::eq(*source, row.source))
    }) {
        if let Some(reading) = readings.iter().find(|reading| {
            reading.invocation == row.source.process.invocation_id
                && reading.pid == row.source.process.pid
        }) {
            if shows_cpu || shows_memory {
                let muted = Style::default().fg(layout.ink(label_color(), row.source.faded()));
                if shows_cpu {
                    row.cells[CPU_COLUMN] =
                        Text::from(Span::styled(cpu_text(reading.resources.cpu), muted));
                }
                if shows_memory {
                    row.cells[MEMORY_COLUMN] =
                        Text::from(Span::styled(memory_text(reading.resources.memory), muted));
                }
            }
            if shows_compiler {
                row.cells[COMPILER_COLUMN] = Text::from(compiler_reading(
                    &reading.resources.compiler,
                    row.source,
                    layout,
                ));
                plan.header_and_value_visibility[COMPILER_COLUMN].include(
                    row.cells[COMPILER_COLUMN]
                        .lines
                        .iter()
                        .any(|line| !line.to_string().is_empty()),
                );
            }
        }
        if shows_parent {
            let style = parent_style(row.source, layout, &family_links);
            for line in &mut row.cells[PARENT_COLUMN].lines {
                for span in &mut line.spans {
                    span.style = style;
                }
            }
        }
        if shows_pid {
            let style = pid_style(row.source, layout, &family_links);
            for line in &mut row.cells[PID_COLUMN].lines {
                for span in &mut line.spans {
                    span.style = style;
                }
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
    view: &CommandView,
) {
    frame_draw_process_table(
        buffer,
        area,
        rows,
        ground,
        pinned,
        &FrameTableColumnGrid::of(rows, kind, view),
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
        .map(|group| group.measured_rows(&layout))
        .collect();
    let available_height = area.height.saturating_sub(TABLE_HEADER_HEIGHT);
    let mut plan = planned_table_rows(&drawn_groups, available_height, grid.separation, &layout);
    if let PlannedVisibleRows::ProcessRows(visible) = &plan.visible_rows {
        for row in drawn_groups.iter_mut().flatten() {
            if visible
                .iter()
                .any(|source| std::ptr::eq(*source, row.source))
            {
                *row = process_row(row.source, &layout);
            }
        }
    }
    let headings = TableAccountHeadings::of(
        &groups[..plan.visible_group_count],
        cleared(area)
            .width
            .saturating_sub(cell_width(SECTION_HEADER_INDENT)),
    );
    apply_planned_row_readings(&mut drawn_groups, &mut plan, &layout, grid);
    paint_table_header(
        buffer,
        header_area,
        &layout,
        faded,
        std::array::from_fn(|column| plan.labels_column(column)),
    );
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
    header_and_value_visibility: [HeaderLabel; TABLE_HEADERS.len()],
}

impl ProcessTablePlan<'_> {
    /// A present slot labels itself only when a drawn row supplies its value.
    const fn labels_column(&self, column: usize) -> HeaderLabel {
        if matches!(column, STATE_COLUMN | COMPILER_COLUMN | MANAGED_COLUMN) {
            match &self.visible_rows {
                PlannedVisibleRows::NoProcessRows => HeaderLabel::Hidden,
                PlannedVisibleRows::ProcessRows(rows) => {
                    if rows.is_empty() {
                        HeaderLabel::Hidden
                    } else {
                        self.header_and_value_visibility[column]
                    }
                },
            }
        } else {
            HeaderLabel::Shown
        }
    }
}

/// Viewport plan shared by row-use probes and paint at one solved width.
#[derive(Clone)]
struct CachedTablePlan {
    rows:                Vec<Vec<(InvocationId, u32)>>,
    column_widths:       Vec<u16>,
    columns:             Vec<usize>,
    height:              u16,
    separation:          DirectorySeparation,
    visible:             Vec<usize>,
    separators:          TableSeparators,
    visible_group_count: usize,
    drawn:               u16,
    required:            u16,
    labels:              [HeaderLabel; TABLE_HEADERS.len()],
}

fn planned_table_rows<'a>(
    groups: &[Vec<LaidOutProcessRow<'a>>],
    height: u16,
    separation: DirectorySeparation,
    layout: &CellTableLayout,
) -> ProcessTablePlan<'a> {
    if let Some(cached) = layout.prepared_plans.borrow().iter().find(|cached| {
        cached.rows.len() == groups.len()
            && cached.rows.iter().zip(groups).all(|(saved, group)| {
                saved.len() == group.len()
                    && saved.iter().zip(group).all(|((id, pid), row)| {
                        (id, pid) == (&row.source.process.invocation_id, &row.source.process.pid)
                    })
            })
            && cached.column_widths == layout.column_widths
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
        rows: groups
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
            .collect(),
        column_widths: layout.column_widths.clone(),
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
    let mut header_and_value_visibility = [HeaderLabel::Hidden; TABLE_HEADERS.len()];
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
                header_and_value_visibility[column].include(
                    row.cells[column]
                        .lines
                        .iter()
                        .any(|line| line.spans.iter().any(|span| !span.content.is_empty())),
                );
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
        /// The final row, already cut from parsed units with its neutral mark.
        line: Line<'static>,
    },
    /// The final visible command is whole, but later process rows did not fit.
    RowsWereOmitted,
}

/// Classify a vertical cut and ask the wrapper for its final visible command row.
fn table_continuation(rows: &[LaidOutProcessRow<'_>], visible_lines: u16) -> TableContinuation {
    let mut used = 0u16;
    for row in rows {
        if used >= visible_lines {
            return TableContinuation::RowsWereOmitted;
        }
        if used.saturating_add(row.lines) > visible_lines {
            let first_hidden_line = usize::from(visible_lines.saturating_sub(used));
            return TableContinuation::CommandWasCut {
                line: row.cut_command_continuation(first_hidden_line),
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
    let mut top = area.y.saturating_add(GROUP_HEADER_HEIGHT);
    let bottom = top.saturating_add(table_height);
    for row in &rows {
        let height = row.lines.min(bottom.saturating_sub(top));
        if let Ok(span) = PaintedRowSpan::new(
            row.source.process.invocation_id.clone(),
            Rect::new(indented(area).x, top, indented(area).width, height),
        ) {
            layout.painted_rows.borrow_mut().push(ProcessRowPaint {
                pid: row.source.process.pid,
                span,
            });
        }
        top = top.saturating_add(height);
    }
    paint_process_rows(
        buffer,
        Rect {
            y: area.y.saturating_add(GROUP_HEADER_HEIGHT),
            height: table_height,
            ..indented(area)
        },
        rows,
        layout,
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
fn longest_command_width(rows: &[ProcessRowValues]) -> u16 {
    rows.iter()
        .map(|row| cell_width(&row.cells[COMMAND_COLUMN]))
        .max()
        .unwrap_or_default()
}

/// The executable spelling selected by the current view view.
fn displayed_program<'a>(command: &'a CommandText, view: &CommandView) -> &'a str {
    match view.arguments {
        ArgumentDetail::Default => command.program_name(),
        ArgumentDetail::All => &command.program,
    }
}

/// Leading profile label retained when the command's tail is cut.
fn profile_label(command: &CommandText) -> String {
    match command.profile() {
        CargoProfile::Default => String::new(),
        CargoProfile::Release => {
            format!("{PROFILE_LABEL_OPEN}{CARGO_RELEASE_PROFILE}{PROFILE_LABEL_CLOSE}")
        },
        CargoProfile::Named(name) => format!("{PROFILE_LABEL_OPEN}{name}{PROFILE_LABEL_CLOSE}"),
    }
}

/// All words painted by the command column, including its profile.
fn displayed_command(command: &CommandText, view: &CommandView) -> String {
    let arguments = match view.arguments {
        ArgumentDetail::Default => command.selected_arguments(&view.defaults),
        ArgumentDetail::All => command.line(),
    };
    [
        profile_label(command),
        displayed_program(command, view).to_string(),
        arguments,
    ]
    .into_iter()
    .filter(|part| !part.is_empty())
    .collect::<Vec<_>>()
    .join(COMMAND_WORD_SEPARATOR)
}

/// Only the toolchain, profile and subcommand set the resource-column budget.
fn displayed_subcommand_name(command: &CommandText) -> CommandName {
    let named = command.named();
    let mut words = named.words.split_whitespace().peekable();
    let mut kept = vec![profile_label(command), command.program_name().to_string()];
    while words
        .peek()
        .is_some_and(|word| word.starts_with(CARGO_TOOLCHAIN_SELECTOR))
    {
        if let Some(selector) = words.next() {
            kept.push(selector.to_string());
        }
    }
    if let Some(subcommand) = words.next() {
        kept.push(subcommand.to_string());
        if subcommand == NEXTEST_SUBCOMMAND_NAME
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

/// The parsed word fixes both its wrap boundary and its permitted cut.
enum StyledCommandWord {
    /// An independent argv entry may use path, punctuation and whitespace breaks.
    Independent(Span<'static>),
    /// A cut must reach the value; it may never leave this flag by itself.
    BoundValue {
        flag:  Span<'static>,
        value: Span<'static>,
    },
}

impl StyledCommandWord {
    fn argument(word: &CommandWord, style: Style) -> Self {
        match word.display_word() {
            CommandWordDisplay::Independent(text) => {
                Self::Independent(Span::styled(text.into_owned(), style))
            },
            CommandWordDisplay::FlagWithValue { flag, value } => Self::BoundValue {
                flag:  Span::styled(flag.to_owned(), style),
                value: Span::styled(value.to_owned(), style),
            },
        }
    }

    fn width(&self) -> u16 {
        match self {
            Self::Independent(span) => cell_width(&span.content),
            Self::BoundValue { flag, value } => cell_width(&flag.content)
                .saturating_add(cell_width(COMMAND_WORD_SEPARATOR))
                .saturating_add(cell_width(&value.content)),
        }
    }

    fn write_to(self, line: &mut Line<'static>) {
        match self {
            Self::Independent(span) => line.spans.push(span),
            Self::BoundValue { flag, value } => {
                line.spans.push(flag);
                line.spans.push(Span::raw(COMMAND_WORD_SEPARATOR));
                line.spans.push(value);
            },
        }
    }
}

/// Ancestors keep the same parsed argv units as process rows.
fn ancestor_command_words(ancestor: &Ancestor, style: Style) -> Vec<StyledCommandWord> {
    let command = &ancestor.command;
    let mut words = vec![StyledCommandWord::Independent(Span::styled(
        command.program.clone(),
        style,
    ))];
    words.extend(
        command
            .words()
            .iter()
            .map(|word| StyledCommandWord::argument(word, style)),
    );
    words
}

/// A parsed unit or source fragment, with its original boundary before it.
struct JoinedCommandWord {
    join: CommandContinuationJoin,
    word: StyledCommandWord,
}

/// Wrapped units retain their boundaries until the row budget is known.
struct WrappedCommand {
    lines: Vec<WrappedCommandLine>,
    width: u16,
    cut:   Style,
}

/// One line's units; no painted cells are needed to decide a later height cut.
struct WrappedCommandLine {
    words: Vec<JoinedCommandWord>,
}

impl WrappedCommand {
    fn into_text(self) -> Text<'static> { Text::from(self.into_rows(usize::MAX)) }

    /// Build the last row from its units with the mark already reserved.
    fn into_rows(mut self, budget: usize) -> Vec<Line<'static>> {
        if budget == 0 {
            return Vec::new();
        }
        let cut_words = if self.lines.len() > budget {
            self.lines
                .split_off(budget - 1)
                .into_iter()
                .flat_map(|line| line.words)
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        let mut lines: Vec<_> = self
            .lines
            .into_iter()
            .map(|line| command_unit_line(line.words, self.width, self.cut, CommandRowEnd::Whole))
            .collect();
        if !cut_words.is_empty() {
            lines.push(command_unit_line(
                cut_words,
                self.width,
                self.cut,
                CommandRowEnd::Cut,
            ));
        }
        lines
    }
}

/// Whether the row budget requires a mark even when the next unit starts offscreen.
#[derive(Clone, Copy)]
enum CommandRowEnd {
    Whole,
    Cut,
}

/// Wrap parsed units; independent oversized entries split at their own source boundaries.
fn wrapped_command(words: Vec<StyledCommandWord>, width: u16, cut: Style) -> WrappedCommand {
    let mut lines = Vec::new();
    let mut line = Vec::new();
    let mut used: u16 = 0;
    for unit in command_wrap_units(words, width) {
        let separator = if line.is_empty() {
            0
        } else {
            unit.join.width()
        };
        let needed = unit.word.width();
        if !line.is_empty() && used.saturating_add(separator).saturating_add(needed) > width {
            lines.push(WrappedCommandLine {
                words: std::mem::take(&mut line),
            });
            used = 0;
        }
        if !line.is_empty() {
            used = used.saturating_add(unit.join.width());
        }
        used = used.saturating_add(needed);
        line.push(unit);
    }
    if !line.is_empty() {
        lines.push(WrappedCommandLine { words: line });
    }
    WrappedCommand { lines, width, cut }
}

/// Read whitespace boundaries from the source, before the shared punctuation wrapper.
fn command_wrap_units(words: Vec<StyledCommandWord>, width: u16) -> Vec<JoinedCommandWord> {
    let mut units = Vec::new();
    for word in words {
        match word {
            StyledCommandWord::Independent(span) if cell_width(&span.content) > width => {
                for source_word in span.content.split_whitespace() {
                    let parts = tui_pane::wrapped(
                        vec![Span::styled(source_word.to_owned(), span.style)],
                        width,
                    );
                    for (index, part) in parts.lines.into_iter().enumerate() {
                        units.push(JoinedCommandWord {
                            join: if index == 0 {
                                CommandContinuationJoin::WordSeparator
                            } else {
                                CommandContinuationJoin::Direct
                            },
                            word: StyledCommandWord::Independent(Span::styled(
                                part.to_string(),
                                span.style,
                            )),
                        });
                    }
                }
            },
            word => units.push(JoinedCommandWord {
                join: CommandContinuationJoin::WordSeparator,
                word,
            }),
        }
    }
    units
}

impl CommandContinuationJoin {
    const fn width(self) -> u16 {
        match self {
            Self::Direct => 0,
            Self::WordSeparator => 1,
        }
    }
}

/// A cut prefix either includes a value or omits the entire bound unit.
enum CutCommandUnit {
    Prefix(StyledCommandWord),
    Omitted,
}

/// The only command cut owner: parsed units and a cell budget produce the row.
fn command_unit_line(
    words: Vec<JoinedCommandWord>,
    width: u16,
    cut: Style,
    end: CommandRowEnd,
) -> Line<'static> {
    let total = words.iter().enumerate().fold(0u16, |used, (index, unit)| {
        used.saturating_add(if index == 0 { 0 } else { unit.join.width() })
            .saturating_add(unit.word.width())
    });
    let needs_mark = matches!(end, CommandRowEnd::Cut) || total > width;
    let room = width.saturating_sub(if needs_mark { cell_width(ELISION) } else { 0 });
    let mut used: u16 = 0;
    let mut line = Line::default();
    let mut words = words.into_iter().peekable();
    while let Some(unit) = words.next() {
        if !line.spans.is_empty() && matches!(unit.join, CommandContinuationJoin::WordSeparator) {
            if used >= room {
                break;
            }
            line.spans.push(Span::raw(COMMAND_WORD_SEPARATOR));
            used = used.saturating_add(1);
        }
        let available = room.saturating_sub(used);
        let boundary = if needs_mark {
            words.peek().map_or(0, |next| next.join.width())
        } else {
            0
        };
        if unit.word.width().saturating_add(boundary) <= available {
            used = used.saturating_add(unit.word.width());
            unit.word.write_to(&mut line);
        } else {
            let prefix_room = if unit.word.width() <= available {
                available.saturating_sub(boundary)
            } else {
                available
            };
            if let CutCommandUnit::Prefix(word) = cut_command_unit(unit.word, prefix_room) {
                word.write_to(&mut line);
            }
            break;
        }
    }
    if needs_mark && width >= cell_width(ELISION) {
        line.spans.push(Span::styled(ELISION, cut));
    }
    line
}

/// Grapheme-safe prefixes keep their original whitespace, including a cut separator.
fn command_span_prefix(span: &Span<'static>, room: u16) -> Span<'static> {
    let mut used: usize = 0;
    let text: String = span
        .styled_graphemes(Style::default())
        .take_while(|grapheme| {
            let cells = UnicodeWidthStr::width(grapheme.symbol);
            if used.saturating_add(cells) > usize::from(room) {
                return false;
            }
            used += cells;
            true
        })
        .map(|grapheme| grapheme.symbol)
        .collect();
    Span::styled(text, span.style)
}

fn cut_command_unit(word: StyledCommandWord, room: u16) -> CutCommandUnit {
    match word {
        StyledCommandWord::Independent(span) => CutCommandUnit::Prefix(
            StyledCommandWord::Independent(command_span_prefix(&span, room)),
        ),
        StyledCommandWord::BoundValue { flag, value } => {
            let value_room = room
                .saturating_sub(cell_width(&flag.content))
                .saturating_sub(1);
            let value = command_span_prefix(&value, value_room);
            if value.content.trim().is_empty() {
                CutCommandUnit::Omitted
            } else {
                CutCommandUnit::Prefix(StyledCommandWord::BoundValue { flag, value })
            }
        },
    }
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
fn command_line_width(row: &TrackedRow, view: &CommandView) -> u16 {
    cell_width(&displayed_command(&row.process.command, view))
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
const fn numeric_column(column: usize) -> bool {
    matches!(column, CPU_COLUMN | MEMORY_COLUMN | STATE_COLUMN)
}

#[cfg(test)]
fn column_header(layout: &CellTableLayout, faded: u8, plan: &ProcessTablePlan<'_>) -> Row<'static> {
    let style = column_header_style(layout, faded);
    Row::new(
        TABLE_HEADERS
            .iter()
            .enumerate()
            .filter(|(column, _)| layout.columns.contains(column))
            .map(|(column, label)| {
                let line = Line::from(Span::styled(
                    if plan.labels_column(column) == HeaderLabel::Shown {
                        (*label).to_string()
                    } else {
                        String::new()
                    },
                    style,
                ));
                if numeric_column(column) {
                    line.right_aligned()
                } else {
                    line
                }
            }),
    )
}

/// Column positions solved once at a stable origin, shared by every row.
/// Row heights and absolute screen positions do not require another solver pass.
fn painted_table_columns(area: Rect, layout: &CellTableLayout) -> Vec<(usize, Rect)> {
    let gaps = u32::try_from(layout.columns.len().saturating_sub(1))
        .unwrap_or(u32::MAX)
        .saturating_mul(u32::from(layout.column_spacing));
    let used = layout
        .column_widths
        .iter()
        .map(|width| u32::from(*width))
        .sum::<u32>();
    if used.saturating_add(gaps) <= u32::from(area.width) {
        let mut left = area.x;
        return layout
            .columns
            .iter()
            .copied()
            .zip(&layout.column_widths)
            .map(|(column, width)| {
                let rect = Rect::new(left, area.y, *width, area.height);
                left = left
                    .saturating_add(*width)
                    .saturating_add(layout.column_spacing);
                (column, rect)
            })
            .collect();
    }
    let columns = Layout::horizontal(layout.constraints.iter().copied())
        .flex(Flex::Start)
        .spacing(layout.column_spacing)
        .split(Rect::new(0, 0, area.width, 1));
    layout
        .columns
        .iter()
        .copied()
        .zip(columns.iter())
        .map(|(column, rect)| {
            (
                column,
                Rect::new(
                    area.x.saturating_add(rect.x),
                    area.y,
                    rect.width,
                    area.height,
                ),
            )
        })
        .collect()
}

/// Draw the labels at exactly the positions used by the prepared process cells.
fn paint_table_header(
    buffer: &mut Buffer,
    area: Rect,
    layout: &CellTableLayout,
    faded: u8,
    visible: [HeaderLabel; TABLE_HEADERS.len()],
) {
    let style = column_header_style(layout, faded);
    for (column, area) in painted_table_columns(area, layout) {
        let line = Line::from(Span::styled(
            if layout.kind == TableKind::Command || visible[column] == HeaderLabel::Shown {
                TABLE_HEADERS[column]
            } else {
                ""
            },
            style,
        ));
        if numeric_column(column) {
            line.right_aligned()
        } else {
            line
        }
        .render(area, buffer);
    }
}

/// Paint prewrapped rows into their solved columns without laying out a new table.
fn paint_process_rows(
    buffer: &mut Buffer,
    area: Rect,
    rows: Vec<LaidOutProcessRow<'_>>,
    layout: &CellTableLayout,
) {
    let columns = painted_table_columns(area, layout);
    let mut top = area.y;
    for row in rows {
        let height = row.lines.min(area.bottom().saturating_sub(top));
        if height == 0 {
            break;
        }
        for (column, column_area) in &columns {
            let cell = &row.cells[*column];
            let cell_area = Rect {
                y: top,
                height,
                ..*column_area
            };
            if numeric_column(*column) {
                cell.clone().right_aligned().render(cell_area, buffer);
            } else {
                cell.render(cell_area, buffer);
            }
        }
        top = top.saturating_add(height);
    }
}

/// Style shared by the column labels and their no-room stand-in.
fn column_header_style(layout: &CellTableLayout, faded: u8) -> Style {
    Style::default().fg(layout.ink(label_color(), faded))
}

/// One process row prepared for measurement and table rendering.
struct LaidOutProcessRow<'a> {
    /// Cells in canonical column order, filtered only when the table takes them.
    cells:         [Text<'static>; TABLE_HEADERS.len()],
    /// Stable invocation identity for viewport planning.
    source:        &'a TrackedRow,
    /// Lines the row occupies. The command is the one cell that ever
    /// asks for more than one, and it asks for as many as its line
    /// wrapped to.
    lines:         u16,
    /// Solved width used by both wrapping and height-cut continuation.
    command_width: u16,
    /// Ground used to retain the row's faded styles in a budgeted cut.
    ground:        Color,

    /// Display mode used to prepare the command cell.
    view: CommandView,
}

/// Measure every row before preparing the values selected for paint.
fn measured_process_row<'a>(
    row: &'a TrackedRow,
    layout: &CellTableLayout,
) -> LaidOutProcessRow<'a> {
    let mut cells = std::array::from_fn(|_| Text::default());
    cells[STATE_COLUMN] = Text::from(state_cell(row, layout));
    cells[MANAGED_COLUMN] = Text::from(managed_text(&row.process));
    LaidOutProcessRow {
        cells,
        source: row,
        lines: process_row_lines(row, layout),
        command_width: layout.column_width(COMMAND_COLUMN),
        ground: layout.ground,
        view: layout.view.clone(),
    }
}

impl LaidOutProcessRow<'_> {
    /// Turn the prepared cells into the columns selected for this table.
    #[cfg(test)]
    fn into_table_row(self, layout: &CellTableLayout) -> Row<'static> {
        Row::new(
            self.cells
                .into_iter()
                .enumerate()
                .filter(|(column, _)| layout.columns.contains(column))
                .map(|(column, cell)| {
                    if numeric_column(column) {
                        cell.right_aligned()
                    } else {
                        cell
                    }
                }),
        )
        .height(self.lines)
    }

    /// The wrapper produces the cut row directly from the selected units.
    fn cut_command_continuation(&self, visible_lines: usize) -> Line<'static> {
        let words = styled_process_command_words(self.source, &self.view, self.ground);
        wrapped_command(
            words,
            self.command_width,
            cut_mark_style(self.ground, self.source.faded()),
        )
        .into_rows(visible_lines)
        .pop()
        .unwrap_or_default()
    }
}

/// Selection and wrapping share the exact styled parsed-word list.
fn process_command_words(
    command: &CommandText,
    view: &CommandView,
    program: Style,
    arguments: Style,
    ground: Color,
    faded: u8,
) -> Vec<StyledCommandWord> {
    let mut spans = Vec::new();
    let profile = profile_label(command);
    if !profile.is_empty() {
        spans.push(StyledCommandWord::Independent(Span::styled(
            profile,
            cut_mark_style(ground, faded),
        )));
    }
    spans.push(StyledCommandWord::Independent(Span::styled(
        displayed_program(command, view).to_string(),
        program,
    )));
    let words = match view.arguments {
        ArgumentDetail::All => command.words().iter().collect(),
        ArgumentDetail::Default => command.selected_words(&view.defaults),
    };
    spans.extend(
        words
            .into_iter()
            .map(|word| StyledCommandWord::argument(word, arguments)),
    );
    spans
}

fn command_cell(row: &TrackedRow, layout: &CellTableLayout) -> Text<'static> {
    let command_width = layout.column_width(COMMAND_COLUMN);
    if command_width == 0 {
        Text::default()
    } else {
        let spans = styled_process_command_words(row, &layout.view, layout.ground);
        wrapped_command(
            spans,
            command_width,
            cut_mark_style(layout.ground, row.faded()),
        )
        .into_text()
    }
}

/// Style selection is identical for measured rows and their budgeted cut rows.
fn styled_process_command_words(
    row: &TrackedRow,
    view: &CommandView,
    ground: Color,
) -> Vec<StyledCommandWord> {
    let muted = Style::default().fg(blend_color(label_color(), ground, row.faded()));
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
    process_command_words(&process.command, view, program, arguments, ground, faded)
}

/// Read a prepared height without cloning the row's styled cells.
fn process_row_lines(row: &TrackedRow, layout: &CellTableLayout) -> u16 {
    let width = layout.column_width(COMMAND_COLUMN);
    let shared = Rc::clone(&layout.prepared_rows.borrow().heights);
    let mut heights = shared.borrow_mut();
    if let Some(height) = heights.current.get(&width).and_then(|heights| {
        heights
            .iter()
            .find(|height| height.command == row.process.command && height.view == layout.view)
    }) {
        return height.lines;
    }
    if let Some(height) = heights
        .previous
        .get(&width)
        .and_then(|heights| {
            heights
                .iter()
                .find(|height| height.command == row.process.command && height.view == layout.view)
        })
        .cloned()
    {
        let lines = height.lines;
        heights.current.entry(width).or_default().push(height);
        return lines;
    }
    let lines = u16::try_from(command_cell(row, layout).height())
        .unwrap_or(u16::MAX)
        .max(1);
    heights
        .current
        .entry(width)
        .or_default()
        .push(PreparedCommandHeight {
            command: row.process.command.clone(),
            view: layout.view.clone(),
            lines,
        });
    lines
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
#[derive(Clone, Copy)]
enum ProcessColumn {
    Pid,
    Parent,
    Start,
    Duration,
    Cpu,
    Memory,
    State,
    Command,
    Compiler,
    Managed,
}

impl ProcessColumn {
    const ALL: [Self; TABLE_HEADERS.len()] = [
        Self::Pid,
        Self::Parent,
        Self::Start,
        Self::Duration,
        Self::Cpu,
        Self::Memory,
        Self::State,
        Self::Command,
        Self::Compiler,
        Self::Managed,
    ];
}

fn process_row<'a>(row: &'a TrackedRow, layout: &CellTableLayout) -> LaidOutProcessRow<'a> {
    let parent_column = if layout
        .parent_links
        .hidden
        .contains(&row.process.invocation_id)
    {
        ParentColumn::Hidden
    } else {
        ParentColumn::Shown
    };
    let shared = Rc::clone(&layout.prepared_rows.borrow().by_pid);
    let cached = shared.borrow_mut().get(row, layout, parent_column);
    if let Some(cached) = cached {
        return LaidOutProcessRow {
            cells:         cached.cells.clone(),
            lines:         cached.lines,
            source:        row,
            command_width: layout.column_width(COMMAND_COLUMN),
            ground:        layout.ground,
            view:          layout.view.clone(),
        };
    }

    let process = &row.process;
    let faded = row.faded();
    let muted = Style::default().fg(layout.ink(label_color(), faded));
    let mut command = command_cell(row, layout);
    let lines = u16::try_from(command.height()).unwrap_or(u16::MAX).max(1);
    let cells = std::array::from_fn(|column| {
        if !layout.columns.contains(&column) {
            return Text::default();
        }
        match ProcessColumn::ALL[column] {
            ProcessColumn::Pid => Text::from(Span::styled(
                number_text_if_fits(process.pid.to_string(), layout.column_width(PID_COLUMN)),
                pid_style(row, layout, &AllocationFamilyLinks::NoDrawnLinks),
            )),
            ProcessColumn::Parent => Text::from(Span::styled(
                number_text_if_fits(
                    layout.parent_links.text(process),
                    layout.column_width(PARENT_COLUMN),
                ),
                parent_style(row, layout, &AllocationFamilyLinks::NoDrawnLinks),
            )),
            ProcessColumn::Start => Text::from(Span::styled(process.start.clone(), muted)),
            ProcessColumn::Duration => Text::from(Span::styled(process.duration.clone(), muted)),
            ProcessColumn::State => Text::from(state_cell(row, layout)),
            ProcessColumn::Command => std::mem::take(&mut command),
            ProcessColumn::Managed => Text::from(Span::styled(managed_text(process), muted)),
            ProcessColumn::Cpu | ProcessColumn::Memory | ProcessColumn::Compiler => Text::default(),
        }
    });
    shared
        .borrow_mut()
        .current
        .entry(row.process.pid)
        .or_default()
        .push(Rc::new(CachedProcessRow {
            process: process.clone(),
            faded,
            cells: cells.clone(),
            lines,
            widths: layout.column_widths.clone(),
            columns: layout.columns.clone(),
            ground: layout.ground,
            parent_column,
            lifetime: ProcessRowLifetime::of(row),
            progress: ProcessRowProgress::of(row, layout),
            view: layout.view.clone(),
            theme: tui_pane::theme(),
        }));
    LaidOutProcessRow {
        cells,
        lines,
        source: row,
        command_width: layout.column_width(COMMAND_COLUMN),
        ground: layout.ground,
        view: layout.view.clone(),
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
        FamilyHead::Heads(family) => layout.family_inks.borrow_mut().color(family),
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
        ParentFamily::Member(family) => layout.family_inks.borrow_mut().color(family),
        ParentFamily::NoFamily => text_default(),
    };
    Style::default().fg(layout.ink(color, row.faded()))
}

/// Solved widths for every constraint, matching [`ratatui::widgets::Table`]'s layout.
fn solved_column_widths(width: u16, constraints: &[Constraint], column_spacing: u16) -> Vec<u16> {
    if constraints.is_empty() {
        return Vec::new();
    }
    let fixed: Option<Vec<_>> = constraints
        .iter()
        .map(|constraint| match constraint {
            Constraint::Length(width) => Some(*width),
            _ => None,
        })
        .collect();
    if let Some(widths) = fixed {
        let gaps = u32::try_from(widths.len().saturating_sub(1))
            .unwrap_or(u32::MAX)
            .saturating_mul(u32::from(column_spacing));
        let used = widths.iter().map(|width| u32::from(*width)).sum::<u32>();
        if used.saturating_add(gaps) <= u32::from(width) {
            return widths;
        }
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
    FrameTableColumnGrid::of(rows, kind, CommandView::long()).slots
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
    if matches!(
        compiler,
        CompilerObservation::Unknown | CompilerObservation::None
    ) {
        return Line::default();
    }
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
    let globals = [
        StatusLineGlobal::app(AppGlobalAction::ProcessTree),
        StatusLineGlobal::app(AppGlobalAction::Arguments),
        StatusLineGlobal::global_shortcuts_help(),
    ];
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
    if app.tree == ProcessHierarchy::Shown {
        notes.push(StatusLineNote::flag(PROCESS_TREE_NOTE_LABEL));
    }
    if app.arguments == ArgumentDetail::All {
        notes.push(StatusLineNote::flag(ALL_ARGUMENTS_NOTE_LABEL));
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
    hidden_commands: &[String],
) -> Vec<(u32, Measurement<String>)> {
    let visible = roster.clone().visible(hidden_commands);
    let roster = &visible;
    let summary = summary_rows(roster);
    let rows = summary;
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
mod tests;

#[cfg(test)]
mod status_bar_contrast_tests;
