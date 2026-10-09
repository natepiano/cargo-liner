//! The tile grid: how many cells the pane holds, where each one sits,
//! and the motion from one arrangement to the next.
//!
//! Cells are numbered from one and fill column by column. Cell one is
//! the summary table; each of the rest belongs to one running command,
//! or stands empty waiting for one. [`columns`] says how many cells each
//! column holds and [`shares`] says how a column divides between them;
//! both are pure, so the arrangement at any count is a test rather than
//! something to squint at on screen.
//!
//! A column divides evenly until something forces it not to. Each cell
//! asks for the rows its contents need through [`cell_wants`], and
//! [`shares`] hands room off the cells that are not using theirs and
//! onto the ones that have run out, from either side. A cell keeps what
//! it needs and no more, so every row it is not using is one a
//! neighbour can have; the summary, then the focus ring, is served
//! first out of them. A column where everything fits is divided evenly,
//! because nothing in it is asking for the room going spare.
//!
//! What a cell holds is sticky. Its contents shrinking hands nothing
//! back, because nothing else wants the room; the room returns when
//! another cell encroaches -- any cell asking for more than it holds,
//! or a cell joining or leaving -- at which point every cell drops to
//! what it is actually showing and the whole grid is divided again.
//! [`HeldCellLayout`] is that state and [`TileGrid::settled_held`] is the rule;
//! [`shares`] itself stays a pure function of what it is handed.
//!
//! The demand is deliberately coarse. Measured in rows, a column would
//! re-divide on every scan that rewrapped a command line; rounded up to
//! a whole [`super::constants::TILE_DEMAND_STEP`] it moves on the step
//! from a quiet cell to a busy one, which is what keeps a change
//! something the grid travels through rather than something it twitches
//! on.
//!
//! Splitting a rect is the framework's job, not this module's:
//! [`constraints_for_sizes`] turns per-column and per-row shares into
//! ratatui constraints and ratatui's solver tiles the rect exactly, and
//! the result is a [`ResolvedPaneLayout`] keyed by cell number -- the
//! same type [`crate::render_panes`] walks. What is left here is only
//! what the framework has no opinion about: how many columns there are,
//! how tall each one is, and how a cell travels when that changes.
//!
//! Neighbours share a border, so the rects handed out overlap by the one
//! line between them rather than sitting flush. [`crate::GridLines`]
//! draws that line once for both.

use std::cmp::Ordering;
use std::cmp::Reverse;
use std::collections::VecDeque;
use std::fmt::Debug;
use std::time::Instant;

use ratatui::layout::Layout;
use ratatui::layout::Position;
use ratatui::layout::Rect;

use super::action::TileAction;
use super::constants::MIN_INITIAL_ROWS;
use super::constants::PROGRESS_SCALE;
use super::constants::TABLE_CELL;
use super::draw;
use super::growth::TileGrowth;
use super::settings::TileSettings;
use crate::CycleDirection;
use crate::PaneAxisSize;
use crate::PaneFrame;
use crate::ResolvedPane;
use crate::ResolvedPaneLayout;
use crate::TileFill;
use crate::constraints_for_sizes;
use crate::frame_inner;
use crate::share_borders;

/// One group's claim on its column.
///
/// `Id` is the app's identity for the group a cell draws. The grid only
/// clones and compares it, so any stable key the app already holds will
/// do.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TileDemand<Id> {
    /// The group's identity, the same key [`TileContent::Group`] carries.
    pub id:   Id,
    /// Rows the group's cell would draw given all the room it wants,
    /// which the app measures at the width
    /// [`TileCells::demands`](crate::TileCells::demands) is handed for
    /// the cell.
    pub rows: usize,
}

/// What every cell of the grid is asking for, as one scan left it.
///
/// Kept apart from the arrangement on purpose. The arrangement says
/// which slot sits at which cell and is what the motion is keyed on;
/// this says how much of its column each of them takes, and the two
/// change on different events -- a command starting or finishing moves
/// cells, while a command merely running more invocations than before
/// only re-divides the column it is already in.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TileDemands<Id> {
    /// Rows the summary cell would draw given all the room it wants.
    pub summary:       usize,
    /// Columns the summary's widest line takes, inside the cell's
    /// borders. Read only under [`TileGrowth::widen_summary`], where the
    /// summary widens across columns until it holds this many; zero
    /// asks for no more than its own column.
    pub summary_width: u16,
    /// Every group that gets a cell, in cell order.
    pub groups:        Vec<TileDemand<Id>>,
}

impl<Id> Default for TileDemands<Id> {
    fn default() -> Self {
        Self {
            summary:       0,
            summary_width: 0,
            groups:        Vec::new(),
        }
    }
}

impl<Id: Clone + Eq + Debug> TileDemands<Id> {
    /// Rows the cell drawing `id` is asking for, and none when this
    /// scan no longer carries that group.
    pub(super) fn rows_for(&self, id: &Id) -> usize {
        self.groups
            .iter()
            .find(|demand| &demand.id == id)
            .map_or(0, |demand| demand.rows)
    }

    /// The identity of every group that gets a cell, in order.
    fn ids(&self) -> Vec<Id> { self.groups.iter().map(|demand| demand.id.clone()).collect() }

    /// Whether this scan still carries `id`, which is what tells a cell
    /// closing to make way for the order apart from one whose command
    /// has finished.
    fn holds(&self, id: &Id) -> bool { self.groups.iter().any(|demand| &demand.id == id) }
}

/// What a cell is showing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TileContent<Id> {
    /// The summary table, which every grid opens with.
    Summary,
    /// One command's tile, keyed by the group whose rows it draws.
    Group(Id),
    /// A cell opened with `+` that no command has claimed, carrying the
    /// number it currently sits at.
    Empty(usize),
}

/// One cell as a single frame should draw it.
///
/// A cell crossing between columns is drawn as two of these -- the piece
/// leaving the old column and the piece arriving in the new one -- which
/// is what makes it read as sliding off one column's edge and back in at
/// the next.
pub struct TilePlacement<Id> {
    /// What the cell draws.
    pub content: TileContent<Id>,
    /// Where the cell's box sits this frame, and how far it is cut off
    /// -- the framework's own account of a moving pane, which both
    /// [`crate::draw_clipped`] and [`crate::GridLines`] read.
    pub frame:   PaneFrame,
}

/// Which copy of a cell draws its name while the cell crosses columns.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum PieceName {
    /// This piece draws the cell's title or empty-cell number.
    Shown,
    /// The cell's other piece draws its name.
    OnTheOtherPiece,
}

/// One visible piece and the name role its placed geometry gives it.
pub(super) struct TilePiece<Id> {
    /// What and where the piece draws.
    pub(super) placement: TilePlacement<Id>,
    /// Whether this piece draws the cell's name.
    pub(super) name:      PieceName,
}

/// One frame's cell pieces and the fixed column frames they move inside.
pub(super) struct TileDrawing<Id> {
    /// Every visible piece, in cell order.
    pub(super) pieces:       Vec<TilePiece<Id>>,
    /// Each column's band at the same point in the transition.
    pub(super) column_bands: Vec<Rect>,
}

impl<Id> TileDrawing<Id> {
    /// The placements carried by this frame's pieces.
    #[cfg(test)]
    pub(super) fn placements(&self) -> impl Iterator<Item = &TilePlacement<Id>> {
        self.pieces.iter().map(|piece| &piece.placement)
    }
}

/// A cell after the summary.
///
/// The identity here is what makes the motion work. A cell is animated
/// from where its slot stood to where the same slot stands now, so a
/// command finishing in the middle of the grid draws every cell after it
/// travelling one place forward, rather than the contents jumping
/// between cells that stayed put.
#[derive(Clone, Debug, Eq, PartialEq)]
enum Slot<Id> {
    /// The tile drawing one command's rows.
    Group(Id),
    /// A cell opened with `+` and not yet claimed. The number is only
    /// an identity -- it is never shown, and no two empties share one --
    /// so an empty cell can be told from its neighbour while the grid
    /// closes up around them.
    Empty(u64),
}

/// The cell holding focus.
///
/// Focus is held by identity rather than by cell number for the same
/// reason the cells are: the command a developer is watching keeps the
/// ring as the grid closes up around it.
#[derive(Clone, Debug, Eq, PartialEq)]
enum Focus<Id> {
    /// The summary, where focus starts and where it falls back to.
    Summary,
    /// One of the cells after it.
    Cell(Slot<Id>),
}

/// One step of the focus ring, named for the arrow that asks for it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Direction {
    /// Toward the column on the left.
    Left,
    /// Toward the column on the right.
    Right,
    /// Toward the cell above, within one column.
    Up,
    /// Toward the cell below, within one column.
    Down,
}

/// A cell's content and focus appearance, apart from its geometry.
#[derive(Clone)]
struct CellAppearance<Id> {
    /// What the cell draws.
    content: TileContent<Id>,
    /// Whether the cell holds focus, which is what lights its border.
    focused: bool,
}

impl<Id: Clone> CellAppearance<Id> {
    /// This cell drawn at `frame`, focus carried onto it.
    fn at(&self, frame: PaneFrame) -> TilePlacement<Id> {
        TilePlacement {
            content: self.content.clone(),
            frame:   frame.with_focus(self.focused),
        }
    }
}

/// Where one cell stands at the two ends of a transition.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CellTransition {
    /// The cell remains, though its number and column may change.
    PresentAtBothEnds {
        /// Its one-based cell number before the move.
        before_cell: usize,
        /// Its one-based cell number after the move.
        after_cell:  usize,
    },
    /// The cell joins the grid at this one-based cell number.
    Arriving {
        /// Its cell number after the move.
        after_cell: usize,
    },
    /// The cell leaves from this one-based cell number.
    Departing {
        /// Its cell number before the move.
        before_cell: usize,
    },
}

/// The outer row a piece enters or leaves through.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BandEdge {
    /// The first frame row in the column band.
    Top,
    /// The last frame row in the column band.
    Bottom,
}

/// How one ordered piece occupies a column band through a move.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BandPieceMotion {
    /// The piece has a cell at both ends.
    Resident {
        /// Its rect before the move.
        before: Rect,
        /// Its rect after the move.
        after:  Rect,
    },
    /// The piece grows in from one outer row.
    Entering {
        /// The row it grows from.
        edge:    BandEdge,
        /// Its rect after the move.
        after:   Rect,
        /// Whether its contents slide through the edge while it grows.
        sliding: bool,
    },
    /// The piece shrinks onto one outer row.
    Leaving {
        /// Its rect before the move.
        before:  Rect,
        /// The row it shrinks onto.
        edge:    BandEdge,
        /// Whether its contents slide through the edge while it shrinks.
        sliding: bool,
    },
    /// The piece keeps its starting rows while its whole column closes.
    ClosingWithColumn {
        /// Its rect before the move.
        before: Rect,
    },
}

/// One cell piece assigned to the column topology it moves within.
struct BandPiece<Id> {
    /// Its zero-based column.
    column:   usize,
    /// How its two row boundaries move.
    motion:   BandPieceMotion,
    /// How far this piece's row boundary has moved.
    progress: u32,
    /// What the piece draws.
    drawn:    CellAppearance<Id>,
}

/// Where the focus ring is located within the current arrangement.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum FocusLocation {
    /// The ring sits on this one-based cell number, including the summary.
    Cell(usize),
    /// The focused slot is outside the arrangement while it moves.
    #[default]
    Departing,
}

/// Which row receives spare height first within one column.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ColumnFocus {
    /// This zero-based row belongs to the focused cell.
    Row(usize),
    /// The focus ring sits outside this column.
    Outside,
}

/// Whether cell geometry is settled or moving from a previous arrangement.
enum GridMotion<Id> {
    /// The current layout is fully drawn at its destination.
    Settled,
    /// A timed move retains the layout it started from.
    Moving(Transition<Id>),
}

/// Which view of the grid the last laid-out area can show readably.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum GridDisplay {
    /// Every held cell fits at the configured floor.
    Cells,
    /// The summary fills the area while the held cells wait off-screen.
    SummaryAlone,
}

/// What one arrangement is drawn at: the rows every cell is holding
/// and which of them has the focus ring.
///
/// A cell holds the rows it grew into rather than the rows it is using.
/// Its contents shrinking hands nothing back, because nothing else is
/// asking for the room; the moment something is -- any cell asking for
/// more than it holds, or a cell joining or leaving the grid -- every
/// cell drops back to what it is actually showing and the columns are
/// divided again from there. That is the whole of the stickiness, and
/// it lives here rather than in [`shares`] so the division itself stays
/// a pure function of what it is handed.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct HeldCellLayout {
    /// Rows each cell holds, in cell order and the summary's first.
    rows:          Vec<u16>,
    /// Which cell holds the ring, or whether that slot is outside the layout.
    focused:       FocusLocation,
    /// Columns the summary asks to reach across, its own included; see
    /// [`summary_span`]. Held with the rows so the summary widening or
    /// narrowing is a change the grid travels through like any other.
    summary_span:  usize,
    /// Positions occupied by the summary, including its first.
    summary_depth: usize,
}

/// The arrangement a transition is moving away from.
struct Transition<Id> {
    /// The cells as they stood before the change.
    from:    Vec<Slot<Id>>,
    /// What those cells were drawn at. Snapshotted rather than
    /// recomputed, because by the time a transition starts the demands
    /// it is moving away from have already been replaced by the ones it
    /// is moving toward.
    held:    HeldCellLayout,
    /// When the motion began.
    started: Instant,
    /// How long this one step runs for.
    millis:  u64,
}

/// One arrangement waiting its turn, and how long the move into it
/// takes once it comes up.
struct Step<Id> {
    /// The cells as this step leaves them.
    slots:  Vec<Slot<Id>>,
    /// Summary depth at the end of this step.
    depth:  usize,
    /// How long the move into `slots` runs for.
    millis: u64,
}

/// The grid's cells and the transition it is playing, if any.
///
/// Cell one is the summary and is always there; every other cell draws
/// one group, keyed by the app's `Id`, or stands empty. The app feeds
/// the grid what each cell is asking for through [`Self::sync`], draws
/// what [`Self::placements`] hands back, and moves it on with
/// [`Self::tick`] and [`Self::apply`].
pub struct TileGrid<Id> {
    /// Cells after the summary, in cell order. The summary is cell one
    /// and is not held here, so the grid never falls below one cell.
    slots:     Vec<Slot<Id>>,
    /// Summary depth of `slots`.
    depth:     usize,
    /// Arrangements waiting their turn, each one cell's move from the
    /// one before it. Only ever one of them is in flight, which is what
    /// makes a change propagate through the grid instead of happening
    /// to every cell at once.
    pending:   VecDeque<Step<Id>>,
    /// What each cell is currently drawn at, which is what a change
    /// animates away from. Empty until the first scan settles it.
    held:      HeldCellLayout,
    /// What every cell is asking for, as the last scan left it.
    demands:   TileDemands<Id>,
    /// Whether the geometry is settled or playing a timed move.
    motion:    GridMotion<Id>,
    /// The rect the last frame laid out, so [`Self::add`] can tell
    /// whether the cells it would create still fit on screen.
    area:      Rect,
    /// How the grid grows, as the last frame had it. Retained so a
    /// mouse click can resolve the same geometry the frame drew without
    /// the caller carrying the settings to every hit test.
    growth:    TileGrowth,
    /// Identity for the next cell opened or emptied.
    next_slot: u64,
    /// The cell the focus ring is on.
    focus:     Focus<Id>,
    /// The sizes and timings the grid is laid out and animated with.
    settings:  TileSettings,
}

impl<Id: Clone + Eq + Debug> Default for TileGrid<Id> {
    fn default() -> Self { Self::new() }
}

impl<Id: Clone + Eq + Debug> TileGrid<Id> {
    /// A grid holding nothing but the summary.
    #[must_use]
    pub fn new() -> Self {
        Self {
            slots:     Vec::new(),
            depth:     1,
            pending:   VecDeque::new(),
            held:      HeldCellLayout {
                rows:          Vec::new(),
                focused:       FocusLocation::Departing,
                summary_span:  1,
                summary_depth: 1,
            },
            demands:   TileDemands::default(),
            motion:    GridMotion::Settled,
            area:      Rect::ZERO,
            growth:    TileGrowth::default(),
            next_slot: 0,
            focus:     Focus::Summary,
            settings:  TileSettings::default(),
        }
    }

    /// Sets the narrowest width a tile may occupy, including its frame.
    pub fn set_min_tile_width(&mut self, width: u16) {
        self.settings.min_tile_width = width.max(super::constants::MIN_TILE_WIDTH);
    }

    /// What the grid is drawn at, worked out fresh whenever no settled
    /// frame has left an answer behind -- a grid that has never been
    /// synced, or one whose cell count has just changed under the
    /// snapshot.
    fn drawn_held(&self) -> HeldCellLayout {
        if self.held.rows.len() == self.count() {
            return self.held.clone();
        }
        HeldCellLayout {
            rows:          cell_wants(&self.demands, &self.slots, &self.settings),
            focused:       self.focused_cell(),
            summary_span:  self.wanted_span(),
            summary_depth: self.depth,
        }
    }

    /// What the grid would be drawn at once this scan is folded in.
    ///
    /// The cells keep the rows they hold while every one of them is
    /// asking for no more than it already has. A single cell asking for
    /// more is the encroachment the whole grid is re-divided on, so
    /// every cell drops back to what it is showing at once rather than
    /// the short one taking its room from whichever neighbour happens
    /// to be next to it.
    fn settled_held(&self) -> HeldCellLayout {
        let wants = cell_wants(&self.demands, &self.slots, &self.settings);
        let focused = self.focused_cell();
        let summary_span = self.wanted_span();
        if wants.len() != self.held.rows.len()
            || self.depth != self.held.summary_depth
            || wants
                .iter()
                .zip(&self.held.rows)
                .any(|(want, held)| want > held)
        {
            return HeldCellLayout {
                rows: wants,
                focused,
                summary_span,
                summary_depth: self.depth,
            };
        }
        HeldCellLayout {
            rows: self.held.rows.clone(),
            focused,
            summary_span,
            summary_depth: self.depth,
        }
    }

    /// Columns the summary asks to reach across at the current cell
    /// count, as the last frame's rect and settings have it.
    fn wanted_span(&self) -> usize {
        if !self.growth.widen_summary {
            return 1;
        }
        summary_span(
            self.area,
            column_layout(self.count(), self.depth, self.growth)
                .widths
                .len(),
            self.demands.summary_width,
        )
    }

    /// Cells the grid holds, the summary included.
    const fn count(&self) -> usize { self.slots.len() + TABLE_CELL }

    /// The interior width of the cell drawing each thing on screen.
    ///
    /// The cell count and resolved summary depth settle the columns
    /// before any column is divided vertically. That lets the app
    /// measure a cell's contents at the width they will be wrapped to
    /// and hand the result back as the demand this same grid divides by.
    pub(super) fn content_widths(
        &self,
        area: Rect,
        growth: TileGrowth,
    ) -> Vec<(TileContent<Id>, u16)> {
        let ColumnLayout {
            widths: held,
            summary_depth: depth,
        } = column_layout(self.count(), self.drawn_held().summary_depth, growth);
        let opened: Vec<Rect> = Layout::horizontal(constraints_for_sizes(&fills(held.len())))
            .split(area)
            .to_vec();
        let mut widths = Vec::with_capacity(self.count());
        for (column, &cells_here) in held.iter().enumerate() {
            let inner = opened
                .get(column)
                .map_or(0, |&rect| frame_inner(share_borders(rect, area)).width);
            widths.extend(std::iter::repeat_n(inner, cells_here));
        }
        if depth > 1 {
            widths.drain(1..depth);
        }
        // The summary alone can reach past its own column, which only
        // the laid-out grid knows.
        let summary = Grid::new(area, &self.drawn_held(), growth, &self.settings)
            .cell(TABLE_CELL)
            .map(|rect| frame_inner(rect).width);
        cells(&self.slots)
            .into_iter()
            .zip(widths)
            .map(|((content, _), width)| {
                let width = match content {
                    TileContent::Summary => summary.unwrap_or(width),
                    TileContent::Group(_) | TileContent::Empty(_) => width,
                };
                (content, width)
            })
            .collect()
    }

    /// Record the geometry the pane just laid out.
    pub const fn set_layout(&mut self, area: Rect, growth: TileGrowth) {
        self.area = area;
        self.growth = growth;
    }

    /// Take the grid to its next step once the one in flight has run
    /// its course, reporting whether it still wants repainting.
    ///
    /// One step at a time, always. A scan that closes two cells and
    /// opens a third is three steps, and they play in order rather than
    /// at once -- three separate travels are followable where three
    /// overlaid on each other are not.
    pub fn tick(&mut self) -> bool {
        if matches!(self.motion, GridMotion::Settled) {
            return false;
        }
        if self.progress() >= PROGRESS_SCALE {
            self.advance();
        }
        true
    }

    /// Give every running command a cell and take back the cells whose
    /// command has gone.
    ///
    /// A command arriving takes the first cell `+` opened and nothing has
    /// claimed, so a developer who made room in advance sees the build
    /// land in it. With no empty cell waiting it opens one of its own. A
    /// command leaving closes its cell wherever that sat, and the grid
    /// comes together over it.
    ///
    /// The cells read in the order `ids` gives, which is the order every
    /// table on the screen reads in: oldest work first. That order is not
    /// fixed once a cell is open -- a driver's cell sorts by the earliest
    /// work still under it, so the last of a queue finishing carries the
    /// cell past whatever started in between. Reaching the new order by
    /// moving cells drew the two of them travelling through each other,
    /// which is the one motion the grid never makes. So a cell the order
    /// has overtaken closes instead, and comes back in behind the cell
    /// that passed it; every cell after it does the same, which is what
    /// carries the change down the rest of the grid a cell at a time.
    ///
    /// The comparison is against where the grid is *headed* rather than
    /// where it stands, so a scan arriving mid-travel adds to the queue
    /// instead of starting the same change over.
    ///
    /// A scan that moves no cell can still re-divide a column: what the
    /// cells are asking for is part of the arrangement, so a command
    /// that has taken on enough new invocations to want another unit of
    /// its column travels there the same way a cell opening does.
    pub fn sync(&mut self, demands: &TileDemands<Id>, growth: TileGrowth) {
        self.demands = demands.clone();
        self.growth = growth;
        let mut arrangement = self.target();
        let mut steps: Vec<Vec<Slot<Id>>> = Vec::new();
        let live = demands.ids();
        while let Some(index) = arrangement.iter().position(|slot| match slot {
            Slot::Group(id) => !live.contains(id),
            Slot::Empty(_) => false,
        }) {
            Self::close(&mut arrangement, index, &mut steps);
        }
        // Worked out against the cells the departed have already given
        // back, so a command arriving in the same scan another leaves
        // takes the room that leaving freed.
        let ids = self.admitted(&arrangement, live, growth);
        // Every cell from the first one out of order onward goes, and
        // the cells before it are the run the grid gets to keep.
        let standing = kept(&arrangement, &ids);
        let mut reopening: Vec<Id> = Vec::new();
        while let Some((index, id)) =
            arrangement
                .iter()
                .enumerate()
                .find_map(|(index, slot)| match slot {
                    Slot::Group(id) if !ids.iter().take(standing).any(|held| held == id) => {
                        Some((index, id.clone()))
                    },
                    _ => None,
                })
        {
            reopening.push(id);
            Self::close(&mut arrangement, index, &mut steps);
        }
        for id in ids {
            if arrangement
                .iter()
                .any(|slot| matches!(slot, Slot::Group(held) if held == &id))
            {
                continue;
            }
            // Behind every cell already carrying a command, because the
            // ones still standing are exactly the commands that sort
            // ahead of this one. An empty cell keeps its place: it is a
            // place the reader made, and only a command genuinely
            // arriving claims one -- a cell that closed to let the order
            // through brings its own back rather than eating it.
            let behind = arrangement
                .iter()
                .rposition(|slot| matches!(*slot, Slot::Group(_)))
                .map_or(0, |index| index.saturating_add(1));
            let claimed = if reopening.contains(&id) {
                None
            } else {
                arrangement
                    .iter()
                    .skip(behind)
                    .position(|slot| matches!(*slot, Slot::Empty(_)))
                    .map(|offset| behind.saturating_add(offset))
            };
            match claimed {
                Some(index) => {
                    arrangement[index] = Slot::Group(id);
                    steps.push(arrangement.clone());
                },
                // Refusing beats a cell too small to read: the command
                // is still in the summary, which is never crowded out.
                None if self.fits(arrangement.len() + TABLE_CELL + 1, growth) => {
                    arrangement.insert(behind, Slot::Group(id));
                    steps.push(arrangement.clone());
                },
                None => (),
            }
        }
        // Nothing moved, but the cells standing still may have changed
        // what they are asking for. Re-dividing the column is a step
        // like any other, and queued as one so it travels rather than
        // snapping.
        let wanted_depth = summary_depth(
            self.area,
            &cell_wants(&self.demands, &arrangement, &self.settings),
            growth,
            &self.settings,
        );
        if steps.is_empty()
            && wanted_depth == self.target_depth()
            && matches!(self.motion, GridMotion::Settled)
            && self.settled_held() != self.held
        {
            steps.push(self.slots.clone());
        }
        self.queue_with_depth(steps, wanted_depth);
    }

    /// Which of `live` the grid can actually give a cell to: the ones
    /// already holding one, plus as many of the rest as there is room
    /// to open.
    ///
    /// Settled before anything closes, because a command the grid has
    /// no room for is left out of the order entirely. Left in, it would
    /// stop the order at its own place and send every cell behind it
    /// through a close and an open to make way for a cell that never
    /// opens.
    fn admitted(&self, arrangement: &[Slot<Id>], live: Vec<Id>, growth: TileGrowth) -> Vec<Id> {
        let mut spare = arrangement
            .iter()
            .filter(|slot| matches!(**slot, Slot::Empty(_)))
            .count();
        let mut count = arrangement.len();
        live.into_iter()
            .filter(|id| {
                if arrangement
                    .iter()
                    .any(|slot| matches!(slot, Slot::Group(held) if held == id))
                {
                    return true;
                }
                if spare > 0 {
                    spare -= 1;
                    return true;
                }
                if self.fits(count + TABLE_CELL + 1, growth) {
                    count += 1;
                    return true;
                }
                false
            })
            .collect()
    }

    /// Push the step that closes the cell at `index`.
    ///
    /// The cell goes and the grid comes together over the space it
    /// held: what stood above it and what stood below meet, every cell
    /// after it moving up one place at once. Keying the cells by slot
    /// is what makes that a single travel each rather than contents
    /// shuffling between cells that never moved.
    ///
    /// The hole is not walked to the end of the grid a neighbour at a
    /// time, which is what this used to do. Every swap along the way
    /// drew the hole travelling down through the cell travelling up --
    /// two boxes crossing, where the eye was looking for one closing.
    fn close(arrangement: &mut Vec<Slot<Id>>, index: usize, steps: &mut Vec<Vec<Slot<Id>>>) {
        let _ = arrangement.remove(index);
        steps.push(arrangement.clone());
    }

    /// Open an empty cell at the end, unless the grid it would make no
    /// longer fits.
    ///
    /// Refusing beats filling the pane with cells too small to carry a
    /// border and a row: the grid stops growing at the point the
    /// terminal stops being able to show it.
    fn add(&mut self, growth: TileGrowth) {
        let mut arrangement = self.target();
        if !self.fits(arrangement.len() + TABLE_CELL + 1, growth) {
            return;
        }
        arrangement.push(Slot::Empty(self.next_slot));
        self.next_slot = self.next_slot.saturating_add(1);
        self.queue(vec![arrangement]);
    }

    /// Close an empty cell: the focused one when focus is on one,
    /// otherwise the last one `+` opened.
    ///
    /// Only an empty cell goes. The cells carrying commands are the
    /// display itself, so `-` undoes `+` rather than hiding a running
    /// build, and the summary is never removable at all. Falling back to
    /// the last empty cell is what keeps `+` and `-` a pair from the
    /// summary, where focus starts and where it returns.
    ///
    /// The cell leaves the way a finished command's does, through
    /// [`Self::close`], so taking one out of the middle closes the grid
    /// over it rather than leaving a hole behind to travel.
    fn remove(&mut self) {
        let mut arrangement = self.target();
        let Some(index) = self.removable(&arrangement) else {
            return;
        };
        let mut steps = Vec::new();
        Self::close(&mut arrangement, index, &mut steps);
        self.queue(steps);
    }

    /// Which cell `-` takes out of `arrangement`, or `None` when it
    /// holds no empty one.
    fn removable(&self, arrangement: &[Slot<Id>]) -> Option<usize> {
        if let Focus::Cell(slot @ Slot::Empty(_)) = &self.focus
            && let Some(index) = arrangement.iter().position(|held| held == slot)
        {
            return Some(index);
        }
        arrangement
            .iter()
            .rposition(|slot| matches!(*slot, Slot::Empty(_)))
    }

    /// The arrangement the grid is headed for: the last step queued, or
    /// what it is showing when nothing is queued.
    fn target(&self) -> Vec<Slot<Id>> {
        self.pending
            .back()
            .map_or_else(|| self.slots.clone(), |step| step.slots.clone())
    }

    /// Summary depth at the end of the queued arrangement.
    fn target_depth(&self) -> usize { self.pending.back().map_or(self.depth, |step| step.depth) }

    /// Queue `steps`, starting the first one when nothing is in flight.
    ///
    /// One scan's worth of change is spread over
    /// [`TileSettings::animation_millis`] however many steps it takes,
    /// down to [`TileSettings::min_step_millis`] a step, so a single cell
    /// closing keeps the full unhurried travel and a burst of them runs
    /// at one steady pace instead of dragging. Past
    /// [`TileSettings::max_pending_steps`] the grid
    /// gives the sequence up and settles the rest in one move: a whole
    /// suite finishing at once would take longer to play through than
    /// anyone would watch.
    fn queue(&mut self, steps: Vec<Vec<Slot<Id>>>) {
        let final_slots = steps.last().cloned().unwrap_or_else(|| self.target());
        let wants = cell_wants(&self.demands, &final_slots, &self.settings);
        let wanted = summary_depth(self.area, &wants, self.growth, &self.settings);
        self.queue_with_depth(steps, wanted);
    }

    /// Queue arrangements using the summary depth already chosen for their end.
    fn queue_with_depth(&mut self, steps: Vec<Vec<Slot<Id>>>, wanted: usize) {
        let start_slots = self.target();
        let final_slots = steps.last().cloned().unwrap_or_else(|| self.target());
        let headed = self.target_depth();
        let mut depth = headed;
        let mut moves = Vec::new();
        while depth > wanted {
            depth -= 1;
            moves.push((start_slots.clone(), depth));
        }
        for slots in steps {
            moves.push((slots, depth));
        }
        while depth < wanted {
            depth += 1;
            moves.push((final_slots.clone(), depth));
        }
        moves.retain(|(slots, depth)| {
            queued_cells_keep_endpoint_columns(
                &start_slots,
                headed,
                &final_slots,
                wanted,
                slots,
                *depth,
                self.growth,
            )
        });
        self.queue_steps(moves);
    }

    /// Queue arrangements paired with their summary depth.
    fn queue_steps(&mut self, steps: Vec<(Vec<Slot<Id>>, usize)>) {
        if steps.is_empty() {
            return;
        }
        let millis = u64::try_from(steps.len())
            .ok()
            .and_then(|count| self.settings.animation_millis.checked_div(count))
            .unwrap_or(self.settings.animation_millis)
            .max(self.settings.min_step_millis);
        self.pending
            .extend(steps.into_iter().map(|(slots, depth)| Step {
                slots,
                depth,
                millis,
            }));
        if self.pending.len() > self.settings.max_pending_steps
            && let Some(last) = self.pending.pop_back()
        {
            self.pending.clear();
            self.pending.push_back(Step {
                millis: self.settings.animation_millis,
                ..last
            });
        }
        if matches!(self.motion, GridMotion::Settled) {
            self.advance();
        }
    }

    /// Move into the next queued step, or settle when there is none.
    ///
    /// The units the step leaves are worked out here rather than by the
    /// caller, so a step that only re-divides a column and a step that
    /// moves cells land the same way: the arrangement being left keeps
    /// the units it was drawn at, and the one arriving takes whatever
    /// the current demands and focus come to.
    fn advance(&mut self) {
        let drawn_held = self.drawn_held();
        let Some(step) = self.pending.pop_front() else {
            self.motion = GridMotion::Settled;
            self.held = drawn_held;
            return;
        };
        let previous = std::mem::replace(&mut self.slots, step.slots);
        self.depth = step.depth;
        self.settle_focus(&previous);
        self.held = self.settled_held();
        self.motion = GridMotion::Moving(Transition {
            from:    previous,
            held:    drawn_held,
            started: Instant::now(),
            millis:  step.millis,
        });
    }

    /// Re-divide the column focus just landed in or left, when the
    /// change asks for one.
    ///
    /// Queued at [`TileSettings::focus_animation_millis`] rather than through
    /// [`Self::queue`]: the ring is a key the developer is already
    /// pressing again, and the full travel a cell opening gets would
    /// leave the grid settling behind them.
    fn resize_for_focus(&mut self) {
        if matches!(self.motion, GridMotion::Moving(_)) || self.settled_held() == self.held {
            return;
        }
        self.pending.push_back(Step {
            slots:  self.slots.clone(),
            depth:  self.depth,
            millis: self.settings.focus_animation_millis,
        });
        self.advance();
    }

    /// Keep the ring on a cell that is still there and hand it back to
    /// the summary when it is not.
    ///
    /// The one cell that changes identity without going anywhere is an
    /// empty one a command has just claimed. The developer opened that
    /// cell to watch for exactly this, so the ring stays on it; every
    /// other way a slot can vanish is the cell itself leaving.
    ///
    /// Except a cell that closed to let the order through, which is
    /// coming back a step or two later. The command is still running
    /// and still the one being watched, so the ring waits for it rather
    /// than dropping to the summary and making the developer find it
    /// again. [`Self::focused_cell`] already answers `FocusLocation::Departing` while the
    /// slot is off the grid, so nothing is drawn holding it meanwhile.
    fn settle_focus(&mut self, previous: &[Slot<Id>]) {
        let Focus::Cell(slot) = &self.focus else {
            return;
        };
        if self.slots.contains(slot) {
            return;
        }
        if let Slot::Group(id) = slot
            && self.demands.holds(id)
        {
            return;
        }
        self.focus = previous
            .iter()
            .position(|held| held == slot)
            .and_then(|index| self.slots.get(index))
            .filter(|taken| claims(slot, taken))
            .map_or(Focus::Summary, |taken| Focus::Cell(taken.clone()));
    }

    /// Play every queued step at once, for tests that care where the
    /// grid ends up rather than how it gets there -- a render golden
    /// among them, since each step otherwise runs for as long as its
    /// travel takes on the wall clock.
    #[doc(hidden)]
    pub fn settle_for_test(&mut self) {
        while !self.pending.is_empty() {
            self.advance();
        }
        self.motion = GridMotion::Settled;
    }

    /// Make a multi-step test transition run directly between its endpoints.
    #[cfg(test)]
    pub(super) const fn collapse_queued_changes_for_test(&mut self) {
        self.settings.max_pending_steps = 1;
    }

    /// Move focus one cell in `direction`, staying put at the edges.
    ///
    /// The grid is ragged -- a column can hold fewer cells than the one
    /// beside it -- so a sideways step keeps the row it can and lands
    /// on the bottom cell of a shorter column rather than refusing to
    /// move at all.
    ///
    /// A summary widened over the next columns heads each of them as
    /// well as its own, so up from the top of one of them reaches it,
    /// and a sideways step from it goes on to the first column it does
    /// not cover.
    fn focus_step(&mut self, direction: Direction, growth: TileGrowth) {
        if self.display() == GridDisplay::SummaryAlone {
            return;
        }
        let FocusLocation::Cell(cell) = self.focused_cell() else {
            return;
        };
        let lanes = Grid::new(self.area, &self.drawn_held(), growth, &self.settings).lanes();
        let Some((column, row)) = lanes.iter().enumerate().find_map(|(column, lane)| {
            lane.iter()
                .position(|&held| held == cell)
                .map(|row| (column, row))
        }) else {
            return;
        };
        let landing = |column: usize, row: usize| {
            lanes
                .get(column)
                .and_then(|lane| lane.get(row.min(lane.len().saturating_sub(1))))
                .copied()
                .filter(|&landed| landed != cell)
        };
        let next = match direction {
            Direction::Left => (0..column).rev().find_map(|to| landing(to, row)),
            Direction::Right => {
                (column.saturating_add(1)..lanes.len()).find_map(|to| landing(to, row))
            },
            Direction::Up => (0..row).rev().find_map(|above| landing(column, above)),
            Direction::Down => (row.saturating_add(1)..lanes[column].len())
                .find_map(|below| landing(column, below)),
        };
        let Some(next) = next else {
            return;
        };
        self.focus = self.focus_at(next);
        self.resize_for_focus();
    }

    /// Take the focus ring one cell along the grid's own order, the
    /// way Tab asks for it, wrapping at either end.
    ///
    /// Where the arrows read the grid as rows and columns -- a column
    /// to the left, the cell above -- this reads it as the list it is
    /// numbered as: the summary, then every command's cell in turn,
    /// then back to the summary. Reports whether it took the step, so
    /// the framework knows the key was spent here rather than on the
    /// pane cycle.
    pub fn cycle_focus(&mut self, direction: CycleDirection) -> bool {
        if self.display() == GridDisplay::SummaryAlone {
            return false;
        }
        let last = self.count();
        let FocusLocation::Cell(cell) = self.focused_cell() else {
            return false;
        };
        let next = match direction {
            CycleDirection::Next if cell >= last => TABLE_CELL,
            CycleDirection::Next => cell.saturating_add(1),
            CycleDirection::Prev if cell <= TABLE_CELL => last,
            CycleDirection::Prev => cell.saturating_sub(1),
        };
        self.focus = self.focus_at(next);
        self.resize_for_focus();
        true
    }

    /// The cell number focus rests on, or its departure from the arrangement.
    fn focused_cell(&self) -> FocusLocation {
        match &self.focus {
            Focus::Summary => FocusLocation::Cell(TABLE_CELL),
            Focus::Cell(slot) => {
                cell_of(&self.slots, slot).map_or(FocusLocation::Departing, FocusLocation::Cell)
            },
        }
    }

    /// What focus becomes on landing at cell `index`, left where it is
    /// when the grid has no such cell.
    fn focus_at(&self, index: usize) -> Focus<Id> {
        if index == TABLE_CELL {
            return Focus::Summary;
        }
        index
            .checked_sub(TABLE_CELL + 1)
            .and_then(|position| self.slots.get(position))
            .map_or_else(|| self.focus.clone(), |slot| Focus::Cell(slot.clone()))
    }

    /// Put focus on cell `index`, leaving it where it is when the grid
    /// has no such cell.
    pub fn focus_cell(&mut self, index: usize) {
        if self.display() == GridDisplay::SummaryAlone {
            return;
        }
        self.focus = self.focus_at(index);
        self.resize_for_focus();
    }

    /// The cell `pos` lands in, or `None` when the point is outside the
    /// grid.
    ///
    /// Answered against the settled grid rather than against a
    /// transition in flight: a cell mid-travel is a transient, and
    /// clicking one is asking for where it is going.
    pub fn cell_at(&self, pos: Position) -> Option<usize> {
        if self.display() == GridDisplay::SummaryAlone {
            return self.area.contains(pos).then_some(TABLE_CELL);
        }
        let grid = Grid::new(self.area, &self.drawn_held(), self.growth, &self.settings);
        grid.resolved
            .panes
            .iter()
            .find(|resolved| resolved.area.contains(pos))
            .map(|resolved| resolved.pane)
    }

    /// Whether every cell of a `count`-cell grid would be big enough to
    /// draw in the rect the last frame used.
    fn fits(&self, count: usize, growth: TileGrowth) -> bool {
        fits(self.area, count, growth, &self.settings)
    }

    /// Whether every arrangement that may still be drawn fits in `area`.
    pub(super) fn holds_in(&self, area: Rect, growth: TileGrowth) -> bool {
        let arrangement_fits = |slots: &[Slot<Id>], depth: usize| {
            let positions = slots
                .len()
                .saturating_add(TABLE_CELL)
                .saturating_add(depth.saturating_sub(1));
            fits(area, positions, growth, &self.settings)
        };
        arrangement_fits(&self.slots, self.depth)
            && match &self.motion {
                GridMotion::Settled => true,
                GridMotion::Moving(transition) => {
                    arrangement_fits(&transition.from, transition.held.summary_depth)
                },
            }
            && self
                .pending
                .iter()
                .all(|step| arrangement_fits(&step.slots, step.depth))
    }

    /// The one display state shared by drawing, hit-testing and focus movement.
    pub(super) fn display(&self) -> GridDisplay {
        if self.holds_in(self.area, self.growth) {
            GridDisplay::Cells
        } else {
            GridDisplay::SummaryAlone
        }
    }

    /// How far through the current transition the grid is, on the
    /// [`PROGRESS_SCALE`] scale. A settled grid is fully through.
    fn progress(&self) -> u32 {
        let GridMotion::Moving(transition) = &self.motion else {
            return PROGRESS_SCALE;
        };
        let elapsed = transition.started.elapsed().as_millis();
        let total = u128::from(transition.millis);
        if total == 0 || elapsed >= total {
            return PROGRESS_SCALE;
        }
        u32::try_from(elapsed * u128::from(PROGRESS_SCALE) / total).unwrap_or(PROGRESS_SCALE)
    }

    /// Every piece to draw this frame, in cell order.
    pub fn placements(&self, area: Rect, growth: TileGrowth) -> Vec<TilePlacement<Id>> {
        self.drawing(area, growth)
            .pieces
            .into_iter()
            .map(|piece| piece.placement)
            .collect()
    }

    /// Every piece and column frame to draw at one reading of the
    /// transition clock.
    pub(super) fn drawing(&self, area: Rect, growth: TileGrowth) -> TileDrawing<Id> {
        self.drawing_at(area, growth, self.progress())
    }

    /// The summary filling an area whose cells do not fit.
    fn summary_alone_drawing(area: Rect) -> TileDrawing<Id> {
        TileDrawing {
            pieces:       vec![TilePiece {
                placement: TilePlacement {
                    content: TileContent::Summary,
                    frame:   PaneFrame::new(area).with_focus(true),
                },
                name:      PieceName::Shown,
            }],
            column_bands: Vec::new(),
        }
    }

    /// The grid's pieces after its current arrangement has settled.
    fn settled_drawing(&self, settled: Grid) -> TileDrawing<Id> {
        let focused = self.focused_cell();
        let pieces = cells(&self.slots)
            .into_iter()
            .filter_map(|(content, index)| {
                Some(TilePiece {
                    placement: TilePlacement {
                        content,
                        frame: PaneFrame::new(settled.cell(index)?)
                            .with_focus(focused == FocusLocation::Cell(index)),
                    },
                    name:      PieceName::Shown,
                })
            })
            .collect();
        TileDrawing {
            pieces,
            column_bands: settled.columns,
        }
    }

    /// Every piece and column frame at an exact point in the transition.
    pub(super) fn drawing_at(&self, area: Rect, growth: TileGrowth, raw: u32) -> TileDrawing<Id> {
        if !self.holds_in(area, growth) {
            return Self::summary_alone_drawing(area);
        }
        let settled = Grid::new(area, &self.drawn_held(), growth, &self.settings);
        let GridMotion::Moving(transition) = &self.motion else {
            return self.settled_drawing(settled);
        };

        let progress = eased(raw);
        let before = Grid::new(area, &transition.held, growth, &self.settings);
        let column_bands = column_bands(&before, &settled, progress);
        let turns = Turns::of(&before, &settled);
        let mut pieces = Vec::new();
        // The summary keeps cell one throughout, but the grid around it
        // resizes, so it still has somewhere to travel.
        moving_cell(
            &before,
            &settled,
            CellTransition::PresentAtBothEnds {
                before_cell: TABLE_CELL,
                after_cell:  TABLE_CELL,
            },
            eased(turns.summary(raw)),
            CellAppearance {
                content: TileContent::Summary,
                focused: self.focus == Focus::Summary,
            },
            &mut pieces,
        );
        for slot in union(&transition.from, &self.slots) {
            // A slot handed over in place is not a cell arriving and a
            // cell leaving, however it looks in the arrangement: the
            // cell stands where it stood and only its contents changed.
            // Taken at face value the pair would draw the old one
            // collapsing while a new one rose out of the column floor to
            // meet it, which is motion the grid never made.
            let (old, new) = (
                cell_of(&transition.from, &slot),
                cell_of(&self.slots, &slot),
            );
            let transition = match (old, new) {
                (None, Some(index))
                    if index
                        .checked_sub(TABLE_CELL + 1)
                        .and_then(|position| transition.from.get(position))
                        .is_some_and(|held| claims(held, &slot)) =>
                {
                    CellTransition::PresentAtBothEnds {
                        before_cell: index,
                        after_cell:  index,
                    }
                },
                (Some(index), None)
                    if index
                        .checked_sub(TABLE_CELL + 1)
                        .and_then(|position| self.slots.get(position))
                        .is_some_and(|taken| {
                            claims(&slot, taken) && !transition.from.contains(taken)
                        }) =>
                {
                    continue;
                },
                (Some(before_cell), Some(after_cell)) => CellTransition::PresentAtBothEnds {
                    before_cell,
                    after_cell,
                },
                (None, Some(after_cell)) => CellTransition::Arriving { after_cell },
                (Some(before_cell), None) => CellTransition::Departing { before_cell },
                (None, None) => continue,
            };
            let content = content_of(&slot, new.or(old).unwrap_or(TABLE_CELL));
            let piece_progress = if turns.covers(&before, &settled, transition) {
                eased(turns.covered(raw))
            } else {
                progress
            };
            moving_cell(
                &before,
                &settled,
                transition,
                piece_progress,
                CellAppearance {
                    content,
                    focused: self.focus == Focus::Cell(slot),
                },
                &mut pieces,
            );
        }
        let pieces = place_band_pieces(&before, &settled, &column_bands, pieces);
        TileDrawing {
            pieces,
            column_bands,
        }
    }

    /// Carry out one grid action: open or close a cell, or move the
    /// focus ring a step. `growth` decides how many cells each column
    /// holds, which decides whether a new cell still fits and where a
    /// step of the ring lands.
    pub fn apply(&mut self, action: TileAction, growth: TileGrowth) {
        self.growth = growth;
        match action {
            TileAction::Add => self.add(growth),
            TileAction::Remove => self.remove(),
            TileAction::FocusLeft => self.focus_step(Direction::Left, growth),
            TileAction::FocusRight => self.focus_step(Direction::Right, growth),
            TileAction::FocusUp => self.focus_step(Direction::Up, growth),
            TileAction::FocusDown => self.focus_step(Direction::Down, growth),
        }
    }
}

/// What each slot draws and the cell it sits at, the summary first.
fn cells<Id: Clone>(slots: &[Slot<Id>]) -> Vec<(TileContent<Id>, usize)> {
    let mut out = vec![(TileContent::Summary, TABLE_CELL)];
    out.extend(slots.iter().enumerate().map(|(position, slot)| {
        let index = position + TABLE_CELL + 1;
        (content_of(slot, index), index)
    }));
    out
}

/// The cell `slot` sits at, or `None` when this arrangement has no such
/// slot.
fn cell_of<Id: Eq>(slots: &[Slot<Id>], slot: &Slot<Id>) -> Option<usize> {
    slots
        .iter()
        .position(|held| held == slot)
        .map(|position| position + TABLE_CELL + 1)
}

/// What a slot draws once it knows the cell it landed at.
fn content_of<Id: Clone>(slot: &Slot<Id>, index: usize) -> TileContent<Id> {
    match slot {
        Slot::Group(id) => TileContent::Group(id.clone()),
        Slot::Empty(_) => TileContent::Empty(index),
    }
}

/// Whether `taken` is a command landing in the empty cell `held` stood
/// for, which is the one way a slot changes identity without moving.
const fn claims<Id>(held: &Slot<Id>, taken: &Slot<Id>) -> bool {
    matches!((held, taken), (Slot::Empty(_), Slot::Group(_)))
}

/// Every slot either arrangement holds, in the order each end gives it.
fn union<Id: Clone + Eq>(from: &[Slot<Id>], to: &[Slot<Id>]) -> Vec<Slot<Id>> {
    let mut out = to.to_vec();
    for (position, slot) in from.iter().enumerate() {
        if to.contains(slot) {
            continue;
        }
        let next_survivor = from[position.saturating_add(1)..]
            .iter()
            .find(|following| to.contains(following));
        let insert_at = next_survivor
            .and_then(|following| out.iter().position(|placed| placed == following))
            .unwrap_or(out.len());
        out.insert(insert_at, slot.clone());
    }
    out
}

/// How many of `ids` can stand where they already do: the longest run
/// from the front of the order that `arrangement` already holds in that
/// same order.
///
/// Everything past that run is what closes. Counted from the front
/// rather than as the largest run that happens to be in order anywhere,
/// because a cell only ever comes back in *behind* the cells still
/// standing -- so the ones that stay have to be the front of the order
/// rather than a stretch out of its middle.
fn kept<Id: Eq>(arrangement: &[Slot<Id>], ids: &[Id]) -> usize {
    let mut cursor = 0_usize;
    let mut count = 0_usize;
    for id in ids {
        let Some(offset) = arrangement
            .iter()
            .skip(cursor)
            .position(|slot| matches!(slot, Slot::Group(held) if held == id))
        else {
            break;
        };
        cursor = cursor.saturating_add(offset).saturating_add(1);
        count = count.saturating_add(1);
    }
    count
}

/// One arrangement resolved against a rect: the rect every cell holds,
/// and the rect every column holds.
///
/// Columns are resolved one at a time rather than through
/// [`crate::PaneGridLayout`] because the grid is ragged -- its columns
/// need not hold the same number of cells -- and that type's placements
/// describe a uniform grid.
struct Grid {
    /// The rect the whole grid fills.
    area:     Rect,
    /// Rows in each column, left to right.
    widths:   Vec<usize>,
    /// Positions represented by the summary, including its first.
    depth:    usize,
    /// Where each cell sits, keyed by cell number.
    resolved: ResolvedPaneLayout<usize>,
    /// The rect each column's cells divide: the column's whole height,
    /// or what the summary leaves below it in a column it widens over.
    columns:  Vec<Rect>,
    /// Columns the summary covers, its own included.
    span:     usize,
}

/// Columns and summary depth that account for exactly the logical cells.
struct ColumnLayout {
    widths:        Vec<usize>,
    summary_depth: usize,
}

/// Use the deepest held summary that still fits in column zero.
fn column_layout(count: usize, held_depth: usize, growth: TileGrowth) -> ColumnLayout {
    let mut depth = held_depth.max(1);
    loop {
        let widths = columns(count + depth - 1, growth);
        if depth == 1 || widths.first().is_some_and(|&height| depth <= height) {
            return ColumnLayout {
                widths,
                summary_depth: depth,
            };
        }
        depth -= 1;
    }
}

impl Grid {
    /// Resolve the cells `held` describes against `area`.
    ///
    /// The first column is divided before the rest, because the summary
    /// at its top settles how far it reaches: it keeps the height its
    /// own column gives it and widens over as many of the next columns
    /// as `held` asks and [`reach`] allows, each of which then divides
    /// only what is left below it.
    fn new(area: Rect, held: &HeldCellLayout, growth: TileGrowth, settings: &TileSettings) -> Self {
        let count = held.rows.len();
        let ColumnLayout {
            widths,
            summary_depth: depth,
        } = column_layout(count, held.summary_depth, growth);
        let opened: Vec<Rect> = Layout::horizontal(constraints_for_sizes(&fills(widths.len())))
            .split(area)
            .to_vec();
        let mut bands = opened.clone();
        let mut span = 1;
        let mut panes = Vec::with_capacity(count);
        let mut position = TABLE_CELL;
        for (column, &height) in widths.iter().enumerate() {
            let opens_at = position;
            let ends_at = opens_at.saturating_add(height);
            let column_head = if column == 0 {
                ColumnHead::Summary(depth)
            } else {
                ColumnHead::Cell
            };
            let (first_cell, pieces) = match column_head {
                ColumnHead::Summary(depth) => (TABLE_CELL, height - depth + 1),
                ColumnHead::Cell => (opens_at - depth + 1, height),
            };
            let first = first_cell - TABLE_CELL;
            let last = (first + pieces).min(held.rows.len());
            let wants = held.rows.get(first..last).unwrap_or(&[]);
            // The ring is held against the whole grid; a column divides
            // itself, so it is told only about the one cell of its own
            // that carries it.
            let focused = match held.focused {
                FocusLocation::Cell(cell) => cell
                    .checked_sub(first_cell)
                    .filter(|at| *at < pieces)
                    .map_or(ColumnFocus::Outside, ColumnFocus::Row),
                FocusLocation::Departing => ColumnFocus::Outside,
            };
            position = ends_at;
            let Some(&band) = bands.get(column) else {
                continue;
            };
            for (row, &cell) in Layout::vertical(constraints_for_sizes(&shares(
                wants,
                band.height,
                column_head,
                focused,
                settings.min_tile_height,
            )))
            .split(band)
            .iter()
            .enumerate()
            {
                let index = first_cell + row;
                let cell = if index == TABLE_CELL {
                    span = reach(&opened, &widths, cell, held.summary_span, settings);
                    for (band, column) in bands.iter_mut().zip(&opened).take(span).skip(1) {
                        *band = below(*column, cell);
                    }
                    widened(cell, opened.get(span.saturating_sub(1)).copied())
                } else {
                    cell
                };
                panes.push(ResolvedPane {
                    pane: index,
                    area: share_borders(cell, area),
                });
            }
        }
        Self {
            area,
            widths,
            depth,
            resolved: ResolvedPaneLayout::new(panes),
            columns: bands
                .iter()
                .map(|&column| share_borders(column, area))
                .collect(),
            span,
        }
    }

    /// The cells each column reads down, for the arrows: its own, with
    /// the summary heading every column it widens over as well as its
    /// own.
    fn lanes(&self) -> Vec<Vec<usize>> {
        let mut opens_at = TABLE_CELL;
        self.widths
            .iter()
            .enumerate()
            .map(|(column, &height)| {
                let own = opens_at..opens_at.saturating_add(height);
                opens_at = own.end;
                let under_summary = column > 0 && column < self.span;
                std::iter::once(TABLE_CELL)
                    .filter(|_| under_summary)
                    .chain(own.map(|position| {
                        if position <= self.depth {
                            TABLE_CELL
                        } else {
                            position - self.depth + 1
                        }
                    }))
                    .collect()
            })
            .collect()
    }

    /// Where cell `index` sits, or `None` when the grid has no such cell.
    fn cell(&self, index: usize) -> Option<Rect> {
        self.resolved
            .panes
            .iter()
            .find(|resolved| resolved.pane == index)
            .map(|resolved| resolved.area)
    }

    /// The column cell `index` falls in, or `None` when the grid has no
    /// such cell.
    fn column_of(&self, index: usize) -> Option<usize> {
        let slot = if index == TABLE_CELL {
            TABLE_CELL
        } else {
            index + self.depth - 1
        };
        position(&self.widths, slot).map(|(column, _)| column)
    }

    /// The rect column `column` occupies, full height.
    #[cfg(test)]
    fn column_rect(&self, column: usize) -> Rect {
        self.columns.get(column).copied().unwrap_or(self.area)
    }
}

/// Columns the summary asks to cover, its own included, for its widest
/// line to fit: the fewest of `columns` equal columns across `area` whose
/// combined interior is at least `width` wide, or all of them when even
/// that is too narrow.
fn summary_span(area: Rect, columns: usize, width: u16) -> usize {
    let opened = Layout::horizontal(constraints_for_sizes(&fills(columns))).split(area);
    let Some(&first) = opened.first() else {
        return 1;
    };
    opened
        .iter()
        .position(|column| {
            let covered = widened(first, Some(*column));
            frame_inner(share_borders(covered, area)).width >= width
        })
        .map_or(opened.len(), |last| last.saturating_add(1))
}

/// Columns the summary covers once `summary`, its rect in the first
/// column, has its height: as many of the `wanted` as `opened` holds,
/// stopping at the first column whose cells would no longer fit below
/// it at [`TileSettings::min_tile_height`] -- the same floor that stops
/// the grid growing a cell it cannot show.
fn reach(
    opened: &[Rect],
    widths: &[usize],
    summary: Rect,
    wanted: usize,
    settings: &TileSettings,
) -> usize {
    let mut span = 1;
    while span < wanted.min(opened.len()) {
        let (Some(column), Some(&cells)) = (opened.get(span), widths.get(span)) else {
            break;
        };
        let room = column.bottom().saturating_sub(summary.bottom());
        let cells = u16::try_from(cells).unwrap_or(u16::MAX);
        if room < shared_run(cells, settings.min_tile_height) {
            break;
        }
        span += 1;
    }
    span
}

/// `summary` carried right to the far edge of `last`, the last column it
/// covers; left as it is without one.
fn widened(summary: Rect, last: Option<Rect>) -> Rect {
    last.map_or(summary, |last| Rect {
        width: last.right().saturating_sub(summary.x),
        ..summary
    })
}

/// What `column` has left below `summary` for its own cells.
const fn below(column: Rect, summary: Rect) -> Rect {
    Rect {
        y: summary.bottom(),
        height: column.bottom().saturating_sub(summary.bottom()),
        ..column
    }
}

/// Rows in each column, left to right, for a grid of `count` cells.
///
/// Two regimes. Up to `growth.initial_rows` the grid is a single
/// column, one cell taller each time: three things on a display read
/// down one column rather than spread across two. Past that the grid
/// opens as many columns as the smallest square holding the cells
/// needs -- `ceil_sqrt` rows to a column, filled a column at a time --
/// and `growth.fill` says how the cells spread over them.
///
/// Crossing into the second regime rearranges what is already on the
/// screen rather than appending to it. At three initial rows the fourth
/// cell turns a column of three into a two-by-two, and the tenth turns
/// a full three-by-three into three columns of four. Between those
/// points [`TileFill::AddNew`] moves nothing at all: the cell arriving
/// lands at the foot of the last column, or opens the next one alone.
/// [`TileFill::Redistribute`] keeps the columns within one cell of each
/// other instead, the taller ones first: the thirteenth cell at three
/// initial rows opens a fourth column and deals the thirteen out as
/// four, three, three and three rather than standing in the fourth
/// column alone.
///
/// Nothing here is remembered, so the grid comes back down through the
/// arrangements it went up through -- take the fourth cell away at
/// three initial rows and the two-by-two is a column of three again.
fn columns(count: usize, growth: TileGrowth) -> Vec<usize> {
    let rows = growth.initial_rows.max(MIN_INITIAL_ROWS);
    if count == 0 {
        return Vec::new();
    }
    // The height every column but the last stands at under `AddNew`:
    // the whole grid while it is still growing to `initial_rows`, the
    // side of the smallest square holding it once it is past them.
    let tallest = if count <= rows {
        count
    } else {
        ceil_sqrt(count)
    };
    let opened = count.div_ceil(tallest);
    match growth.fill {
        TileFill::AddNew => {
            let filled = opened.saturating_sub(1);
            let mut widths = vec![tallest; filled];
            widths.push(count - tallest * filled);
            widths
        },
        TileFill::Redistribute => {
            let (shortest, taller) = (count / opened, count % opened);
            (0..opened)
                .map(|column| shortest + usize::from(column < taller))
                .collect()
        },
    }
}

/// Smallest `side` with `side * side >= value`.
const fn ceil_sqrt(value: usize) -> usize {
    let mut side = 1;
    while side * side < value {
        side += 1;
    }
    side
}

/// `count` equal shares of one axis.
fn fills(count: usize) -> Vec<PaneAxisSize> { vec![PaneAxisSize::Fill(1); count] }

/// What occupies the first piece of a divided column.
#[derive(Clone, Copy)]
enum ColumnHead {
    Summary(usize),
    Cell,
}

/// One column's cells as axis lengths: the even division, then room
/// moved off the cells that are not using theirs and onto the cells
/// that have run out of it.
///
/// The even division is the starting point rather than the answer, and
/// a cell only gives room up when another cell is actually short of it.
/// A column of cells all asking for less than their share is divided
/// evenly, because nothing in it is asking for the room going spare --
/// which is what keeps a cell that grew standing where it grew until
/// something else needs the space.
///
/// A cell holds on to what its contents need and hands over the rest,
/// so a summary showing six rows in a column of eighty gives up the
/// other seventy-odd rather than half its share. `floor` -- the grid's
/// [`TileSettings::min_tile_height`] -- is the one floor: below it a
/// cell has stopped reading as a cell at all.
/// There is no ceiling -- a cell can only ever take what its neighbours
/// are not using, and they are not using it precisely because they do
/// not need it.
///
/// Under [`ColumnHead::Summary`] the summary's shortfall is served first
/// out of the room going spare, so the depth it needs never depends on
/// where the focus is.
///
/// `focused` names the cell holding the focus ring, which is served
/// next out of what is left. That settles nothing while there is enough
/// to go round -- every short cell is filled either way -- and decides
/// it where there is not: the cell being watched gets what it asked for
/// and the others divide what is left.
fn shares(
    wants: &[u16],
    height: u16,
    head: ColumnHead,
    focused: ColumnFocus,
    floor: u16,
) -> Vec<PaneAxisSize> {
    let count = wants.len();
    if count == 0 {
        return Vec::new();
    }
    let mut weights = vec![1; count];
    if let ColumnHead::Summary(depth) = head {
        weights[0] = u16::try_from(depth).unwrap_or(u16::MAX);
    }
    let base = apportion(&weights, height);
    let want: Vec<u16> = wants
        .iter()
        .map(|&asked| asked.max(floor).min(height))
        .collect();

    let mut short: Vec<u16> = want
        .iter()
        .zip(&base)
        .map(|(asked, share)| asked.saturating_sub(*share))
        .collect();
    let spare: Vec<u16> = base
        .iter()
        .zip(&want)
        .map(|(share, asked)| share.saturating_sub(*asked))
        .collect();

    // The summary first when it heads the column, then the focus ring,
    // then the rest of the short cells over whatever they left, each in
    // proportion to how short it is.
    let mut room = spare.iter().copied().fold(0, u16::saturating_add);
    let mut taken = vec![0; count];
    if matches!(head, ColumnHead::Summary(_)) {
        let served = short[0].min(room);
        room = room.saturating_sub(served);
        short[0] = 0;
        taken[0] = served;
    }
    if let ColumnFocus::Row(at) = focused {
        let served = short.get(at).copied().unwrap_or_default().min(room);
        room = room.saturating_sub(served);
        if let Some(cell) = short.get_mut(at) {
            *cell = 0;
        }
        if let Some(cell) = taken.get_mut(at) {
            *cell = cell.saturating_add(served);
        }
    }
    let wanted = short.iter().copied().fold(0, u16::saturating_add);
    for (cell, part) in taken.iter_mut().zip(apportion(&short, wanted.min(room))) {
        *cell = cell.saturating_add(part);
    }

    let moved = taken.iter().copied().fold(0, u16::saturating_add);
    if moved == 0 {
        return base.into_iter().map(PaneAxisSize::Fixed).collect();
    }
    let given = apportion(&spare, moved);
    base.into_iter()
        .enumerate()
        .map(|(index, share)| {
            let settled = share
                .saturating_add(taken.get(index).copied().unwrap_or_default())
                .saturating_sub(given.get(index).copied().unwrap_or_default());
            PaneAxisSize::Fixed(settled)
        })
        .collect()
}

/// Whether the positions of a grid retain a readable minimum size.
fn fits(area: Rect, count: usize, growth: TileGrowth, settings: &TileSettings) -> bool {
    let widths = columns(count, growth);
    let (Ok(opened), Some(&tallest)) = (u16::try_from(widths.len()), widths.iter().max()) else {
        return false;
    };
    let Ok(tallest) = u16::try_from(tallest) else {
        return false;
    };
    area.width >= shared_run(opened, settings.min_tile_width)
        && area.height >= shared_run(tallest, settings.min_tile_height)
}

/// Rows the first column allocates to a summary of `depth` positions.
fn summary_share(wants: &[u16], height: u16, depth: usize, growth: TileGrowth, floor: u16) -> u16 {
    let Some(&positions) = columns(wants.len() + depth - 1, growth).first() else {
        return 0;
    };
    let pieces = positions.saturating_sub(depth).saturating_add(1);
    match shares(
        &wants[..pieces.min(wants.len())],
        height,
        ColumnHead::Summary(depth),
        ColumnFocus::Outside,
        floor,
    )
    .first()
    {
        Some(PaneAxisSize::Fixed(rows)) => *rows,
        _ => 0,
    }
}

/// Fewest summary positions that hold its current demand within the fit limit.
fn summary_depth(area: Rect, wants: &[u16], growth: TileGrowth, settings: &TileSettings) -> usize {
    let Some(&asked) = wants.first() else {
        return 1;
    };
    let mut depth = 1;
    loop {
        if summary_share(wants, area.height, depth, growth, settings.min_tile_height) >= asked {
            return depth;
        }
        let next = depth + 1;
        if next > columns(wants.len() + next - 1, growth)[0]
            || !fits(area, wants.len() + next - 1, growth, settings)
        {
            return depth;
        }
        depth = next;
    }
}

/// `total` split between `weights` in proportion, the units integer
/// division leaves over going to the largest remainders so the parts
/// always add back up to `total` exactly.
fn apportion(weights: &[u16], total: u16) -> Vec<u16> {
    let sum: u64 = weights.iter().copied().map(u64::from).sum();
    if sum == 0 {
        return vec![0; weights.len()];
    }
    let total = u64::from(total);
    let scaled: Vec<u64> = weights
        .iter()
        .map(|&weight| total * u64::from(weight))
        .collect();
    let mut parts: Vec<u64> = scaled.iter().map(|value| value / sum).collect();
    let mut spare = total.saturating_sub(parts.iter().sum::<u64>());
    let mut order: Vec<usize> = (0..weights.len()).collect();
    order.sort_by_key(|&index| Reverse(scaled[index] % sum));
    for index in order {
        if spare == 0 {
            break;
        }
        if let Some(part) = parts.get_mut(index) {
            *part += 1;
            spare -= 1;
        }
    }
    parts
        .into_iter()
        .map(|part| u16::try_from(part).unwrap_or(u16::MAX))
        .collect()
}

/// The rows every cell is asking for, the summary's first.
///
/// What a cell asks for is its contents rounded up to a whole
/// [`TileSettings::demand_step`], plus the rows its own border costs.
/// An empty cell asks for nothing beyond its border, which [`shares`]
/// takes as a cell with room to spare rather than one to shrink.
fn cell_wants<Id: Clone + Eq + Debug>(
    demands: &TileDemands<Id>,
    slots: &[Slot<Id>],
    settings: &TileSettings,
) -> Vec<u16> {
    let mut wants = Vec::with_capacity(slots.len() + TABLE_CELL);
    wants.push(settings.demanded_rows(demands.summary));
    wants.extend(slots.iter().map(|slot| match slot {
        Slot::Group(id) => settings.demanded_rows(demands.rows_for(id)),
        Slot::Empty(_) => settings.demanded_rows(0),
    }));
    wants
}

/// Cells a run of `count` tiles needs along one axis once neighbours
/// share a border: one leading line and its content per tile, then a
/// single line closing the run.
const fn shared_run(count: u16, min_tile: u16) -> u16 {
    count
        .saturating_mul(min_tile.saturating_sub(1))
        .saturating_add(1)
}

/// Which way a step moves the summary's far edge, which decides the
/// order it and the cells below it move in.
///
/// Moving together, the summary's edge sweeps across a column whose top
/// cell has not yet moved out of its way, and the two are drawn over
/// each other for the length of the step. So they take turns. Widening,
/// the cells it is about to cover move down over the first half of the
/// step and the summary spreads over the room they leave in the second;
/// narrowing, the summary draws back first and the cells rise into what
/// it gave up.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Turns {
    /// The summary covers the same columns at both ends of the step.
    Together,
    /// The summary covers more columns at the end: columns `from..to`
    /// are the ones it widens over.
    Widening {
        /// The first column it newly covers.
        from: usize,
        /// One past the last column it newly covers.
        to:   usize,
    },
    /// The summary covers fewer columns at the end: columns `from..to`
    /// are the ones it draws back from.
    Narrowing {
        /// The first column it no longer covers.
        from: usize,
        /// One past the last column it no longer covers.
        to:   usize,
    },
}

impl Turns {
    /// The turns a step from `before` to `after` takes.
    fn of(before: &Grid, after: &Grid) -> Self {
        match before.span.cmp(&after.span) {
            Ordering::Equal => Self::Together,
            Ordering::Less => Self::Widening {
                from: before.span,
                to:   after.span,
            },
            Ordering::Greater => Self::Narrowing {
                from: after.span,
                to:   before.span,
            },
        }
    }

    /// How far through its own turn the summary is at `progress`.
    fn summary(self, progress: u32) -> u32 {
        match self {
            Self::Together => progress,
            Self::Widening { .. } => second_half(progress),
            Self::Narrowing { .. } => first_half(progress),
        }
    }

    /// How far through their turn the cells under the summary's moving
    /// edge are at `progress`.
    fn covered(self, progress: u32) -> u32 {
        match self {
            Self::Together => progress,
            Self::Widening { .. } => first_half(progress),
            Self::Narrowing { .. } => second_half(progress),
        }
    }

    /// Whether the cell travelling `old` to `new` stands in one of the
    /// columns the summary's edge crosses, at both ends of the step. A
    /// cell changing columns is moving for its own reasons, on the
    /// step's own clock.
    fn covers(self, before: &Grid, after: &Grid, transition: CellTransition) -> bool {
        let (Self::Widening { from, to } | Self::Narrowing { from, to }) = self else {
            return false;
        };
        let CellTransition::PresentAtBothEnds {
            before_cell,
            after_cell,
        } = transition
        else {
            return false;
        };
        let column = before.column_of(before_cell);
        column.is_some_and(|column| (from..to).contains(&column))
            && column == after.column_of(after_cell)
    }
}

/// `progress` through the first half of a step, done at the midpoint.
fn first_half(progress: u32) -> u32 { progress.saturating_mul(2).min(PROGRESS_SCALE) }

/// `progress` through the second half of a step, not begun until the
/// midpoint.
fn second_half(progress: u32) -> u32 {
    progress
        .saturating_mul(2)
        .saturating_sub(PROGRESS_SCALE)
        .min(PROGRESS_SCALE)
}

/// Resolve one cell into the ordered column pieces it occupies.
fn moving_cell<Id: Clone>(
    before: &Grid,
    after: &Grid,
    transition: CellTransition,
    progress: u32,
    drawn: CellAppearance<Id>,
    out: &mut Vec<BandPiece<Id>>,
) {
    match transition {
        CellTransition::PresentAtBothEnds {
            before_cell,
            after_cell,
        } => {
            let (Some(from), Some(to)) = (before.cell(before_cell), after.cell(after_cell)) else {
                return;
            };
            let (Some(from_column), Some(to_column)) =
                (before.column_of(before_cell), after.column_of(after_cell))
            else {
                return;
            };
            if from_column == to_column {
                out.push(BandPiece {
                    column: from_column,
                    motion: BandPieceMotion::Resident {
                        before: from,
                        after:  to,
                    },
                    progress,
                    drawn,
                });
                return;
            }
            wrapping_cell(
                after,
                (from_column, to_column),
                (from, to),
                progress,
                drawn,
                out,
            );
        },
        CellTransition::Arriving { after_cell } => {
            let (Some(after_rect), Some(column)) =
                (after.cell(after_cell), after.column_of(after_cell))
            else {
                return;
            };
            out.push(BandPiece {
                column,
                motion: BandPieceMotion::Entering {
                    edge:    BandEdge::Bottom,
                    after:   after_rect,
                    sliding: false,
                },
                progress,
                drawn,
            });
        },
        CellTransition::Departing { before_cell } => {
            let (Some(before_rect), Some(column)) =
                (before.cell(before_cell), before.column_of(before_cell))
            else {
                return;
            };
            out.push(BandPiece {
                column,
                motion: if column < after.columns.len() {
                    BandPieceMotion::Leaving {
                        before:  before_rect,
                        edge:    BandEdge::Bottom,
                        sliding: false,
                    }
                } else {
                    BandPieceMotion::ClosingWithColumn {
                        before: before_rect,
                    }
                },
                progress,
                drawn,
            });
        },
    }
}

/// Push the two pieces a cell crossing columns draws as.
///
/// The cell slides out of one column and back in at the other, against
/// the direction the grid is snaking: moving one column left it leaves
/// over the top and returns from the bottom, and moving right it does
/// the reverse. Each piece is clipped to its own column, so neither is
/// ever seen outside one.
///
/// The two pieces join their bands' ordered topology later, because a
/// step that changes the cell count re-divides the columns while the
/// cell crosses between them.
fn wrapping_cell<Id: Clone>(
    after: &Grid,
    (from_column, to_column): (usize, usize),
    (from, to): (Rect, Rect),
    progress: u32,
    drawn: CellAppearance<Id>,
    out: &mut Vec<BandPiece<Id>>,
) {
    let (leaving_edge, entering_edge) = if to_column < from_column {
        (BandEdge::Top, BandEdge::Bottom)
    } else {
        (BandEdge::Bottom, BandEdge::Top)
    };
    out.push(BandPiece {
        column: from_column,
        motion: if from_column < after.columns.len() {
            BandPieceMotion::Leaving {
                before:  from,
                edge:    leaving_edge,
                sliding: true,
            }
        } else {
            BandPieceMotion::ClosingWithColumn { before: from }
        },
        progress,
        drawn: drawn.clone(),
    });
    out.push(BandPiece {
        column: to_column,
        motion: BandPieceMotion::Entering {
            edge:    entering_edge,
            after:   to,
            sliding: true,
        },
        progress,
        drawn,
    });
}

/// The horizontal band a column stands in partway through a step.
///
/// Every divider is interpolated once and used as both bands' shared
/// edge. An absent band is one line on the area's last drawable column.
fn column_bands(before: &Grid, after: &Grid, progress: u32) -> Vec<Rect> {
    let count = before.columns.len().max(after.columns.len());
    let mut dividers: Vec<u16> = (0..=count)
        .map(|index| {
            lerp(
                column_divider(before, index),
                column_divider(after, index),
                progress,
            )
        })
        .collect();
    for index in (0..count).rev() {
        if dividers[index + 1].saturating_sub(dividers[index]) == 1 {
            dividers[index] = dividers[index + 1];
        }
    }
    (0..count)
        .map(|column| {
            let from = column_endpoint(before, column);
            let to = column_endpoint(after, column);
            let (top, bottom) = if column < after.columns.len() {
                (
                    lerp(from.top(), to.top(), progress),
                    lerp(from.bottom(), to.bottom(), progress),
                )
            } else {
                (from.top(), from.bottom())
            };
            Rect {
                x:      dividers[column],
                y:      top,
                width:  dividers[column + 1]
                    .saturating_sub(dividers[column])
                    .saturating_add(1),
                height: bottom.saturating_sub(top),
            }
        })
        .collect()
}

/// One interpolated column band, for geometry helpers and tests.
#[cfg(test)]
fn column_band(before: &Grid, after: &Grid, column: usize, progress: u32) -> Rect {
    column_bands(before, after, progress)
        .get(column)
        .copied()
        .unwrap_or_else(|| column_endpoint(after, column))
}

/// One end's band, with an absent column collapsed on the last screen column.
fn column_endpoint(grid: &Grid, column: usize) -> Rect {
    grid.columns.get(column).copied().unwrap_or_else(|| Rect {
        x: grid.area.right().saturating_sub(1),
        width: u16::from(!grid.area.is_empty()),
        ..grid.area
    })
}

/// Divider `index` at one settled end of a column layout.
fn column_divider(grid: &Grid, index: usize) -> u16 {
    if index == 0 {
        return grid
            .columns
            .first()
            .map_or_else(|| grid.area.left(), |band| band.left());
    }
    grid.columns.get(index - 1).map_or_else(
        || grid.area.right().saturating_sub(1),
        |band| band.right().saturating_sub(1),
    )
}

/// `rect` moved into `band`'s columns, the rows it covers untouched.
const fn banded(rect: Rect, band: Rect) -> Rect {
    Rect {
        x: band.x,
        width: band.width,
        ..rect
    }
}

impl BandEdge {
    /// The inclusive divider row this edge names in `band`.
    const fn row(self, band: Rect) -> u16 {
        match self {
            Self::Top => band.top(),
            Self::Bottom => band.bottom().saturating_sub(1),
        }
    }

    /// Shift that rests `rect`'s moving outer border on `clip`'s divider.
    fn shift_to_clip(self, rect: Rect, clip: Rect) -> i32 {
        match self {
            Self::Top => i32::from(clip.bottom()) - i32::from(rect.bottom()),
            Self::Bottom => i32::from(clip.top()) - i32::from(rect.top()),
        }
    }
}

impl BandPieceMotion {
    /// This piece's lower inclusive divider before the move.
    const fn before_bottom(self, band: Rect, previous: u16) -> u16 {
        match self {
            Self::Resident { before, .. }
            | Self::Leaving { before, .. }
            | Self::ClosingWithColumn { before } => before.bottom().saturating_sub(1),
            Self::Entering {
                edge,
                sliding: true,
                ..
            } => edge.row(band),
            Self::Entering { sliding: false, .. } => previous,
        }
    }

    /// This piece's lower inclusive divider after the move.
    const fn after_bottom(self, band: Rect, previous: u16) -> u16 {
        match self {
            Self::Resident { after, .. } | Self::Entering { after, .. } => {
                after.bottom().saturating_sub(1)
            },
            Self::ClosingWithColumn { before } => before.bottom().saturating_sub(1),
            Self::Leaving {
                edge,
                sliding: true,
                ..
            } => edge.row(band),
            Self::Leaving { sliding: false, .. } => previous,
        }
    }
}

/// Turn ordered band pieces into placements using one divider per boundary.
fn place_band_pieces<Id: Clone>(
    before: &Grid,
    after: &Grid,
    bands: &[Rect],
    pieces: Vec<BandPiece<Id>>,
) -> Vec<TilePiece<Id>> {
    let mut rects = vec![Rect::ZERO; pieces.len()];
    let from_bands = column_bands(before, after, 0);
    let to_bands = column_bands(before, after, PROGRESS_SCALE);
    for (column, &band) in bands.iter().enumerate() {
        let in_band: Vec<usize> = pieces
            .iter()
            .enumerate()
            .filter_map(|(index, piece)| (piece.column == column).then_some(index))
            .collect();
        let from_band = from_bands[column];
        let to_band = to_bands[column];
        let dividers = band_piece_dividers(&pieces, &in_band, from_band, to_band, band);
        for (position, &index) in in_band.iter().enumerate() {
            let piece = &pieces[index];
            let divider = dividers[position];
            let bottom = dividers[position + 1];
            let mut rect = Rect::new(
                band.x,
                divider,
                band.width,
                bottom.saturating_sub(divider).saturating_add(1),
            );
            if matches!(piece.drawn.content, TileContent::Summary)
                && (before.span > 1 || after.span > 1)
                && let BandPieceMotion::Resident { before, after } = piece.motion
            {
                let horizontal = lerp_rect(before, after, piece.progress);
                rect.x = horizontal.x;
                rect.width = horizontal.width;
            }
            rects[index] = rect;
        }
    }
    let frames = pieces
        .iter()
        .zip(&rects)
        .map(|(piece, &rect)| piece_frame(piece.motion, rect, bands[piece.column]))
        .collect::<Vec<_>>();
    let names = (0..pieces.len())
        .map(|index| piece_name(index, &pieces, &frames))
        .collect::<Vec<_>>();
    pieces
        .into_iter()
        .zip(frames)
        .zip(names)
        .map(|((piece, frame), name)| TilePiece {
            placement: piece.drawn.at(frame),
            name,
        })
        .collect()
}

/// Which of a crossing cell's adjacent pieces owns its name.
///
/// [`wrapping_cell`] adds the leaving piece immediately before the
/// entering one. A single-piece cell therefore never delegates its
/// name, while a crossing delegates only once the arriving piece can
/// draw the row on which an empty cell carries its number. If neither
/// piece can draw that row, the entering piece takes over once its
/// border has a row above the column floor, preserving the title.
fn piece_name<Id>(index: usize, pieces: &[BandPiece<Id>], frames: &[PaneFrame]) -> PieceName {
    let entering = matches!(
        pieces[index].motion,
        BandPieceMotion::Entering { sliding: true, .. }
    );
    let followed_by_entering = pieces.get(index + 1).is_some_and(|piece| {
        matches!(
            piece.motion,
            BandPieceMotion::Entering { sliding: true, .. }
        )
    });
    let (leaving_frame, entering_frame) = if entering {
        let Some(leaving) = index.checked_sub(1).and_then(|leaving| frames.get(leaving)) else {
            return PieceName::Shown;
        };
        (*leaving, frames[index])
    } else if followed_by_entering {
        (frames[index], frames[index + 1])
    } else {
        return PieceName::Shown;
    };
    let entering_draws = draw::name_row_is_visible(entering_frame)
        || (!draw::name_row_is_visible(leaving_frame) && entering_frame.clip().height >= 2);
    if entering == entering_draws {
        PieceName::Shown
    } else {
        PieceName::OnTheOtherPiece
    }
}

/// Inclusive row dividers shared by every ordered piece in one band.
fn band_piece_dividers<Id>(
    pieces: &[BandPiece<Id>],
    in_band: &[usize],
    from_band: Rect,
    to_band: Rect,
    band: Rect,
) -> Vec<u16> {
    let mut before = vec![from_band.top()];
    let mut after = vec![to_band.top()];
    for &index in in_band {
        let motion = pieces[index].motion;
        before.push(motion.before_bottom(from_band, before[before.len() - 1]));
        after.push(motion.after_bottom(to_band, after[after.len() - 1]));
    }
    before[in_band.len()] = from_band.bottom().saturating_sub(1);
    after[in_band.len()] = to_band.bottom().saturating_sub(1);

    let floor = band.bottom().saturating_sub(1);
    let mut dividers = vec![band.top()];
    for (position, &index) in in_band.iter().enumerate() {
        let bottom = if position + 1 == in_band.len() {
            floor
        } else {
            lerp(
                before[position + 1],
                after[position + 1],
                pieces[index].progress,
            )
            .clamp(dividers[dividers.len() - 1], floor)
        };
        dividers.push(bottom);
    }
    for position in (0..in_band.len()).rev() {
        if dividers[position + 1].saturating_sub(dividers[position]) != 1 {
            continue;
        }
        let collapses_at_top = matches!(
            pieces[in_band[position]].motion,
            BandPieceMotion::Entering {
                edge: BandEdge::Top,
                ..
            } | BandPieceMotion::Leaving {
                edge: BandEdge::Top,
                ..
            }
        );
        if collapses_at_top || position == 0 {
            dividers[position + 1] = dividers[position];
        } else {
            dividers[position] = dividers[position + 1];
        }
    }
    // A topward merge can leave the next pair one row apart after that
    // pair was already visited. Sweep those chained gaps toward the top;
    // changing a later divider cannot reopen an earlier pair.
    for position in 0..in_band.len() {
        if dividers[position + 1].saturating_sub(dividers[position]) == 1 {
            dividers[position + 1] = dividers[position];
        }
    }
    dividers
}

/// Draw one band piece in the rect its shared dividers assign it.
fn piece_frame(motion: BandPieceMotion, rect: Rect, band: Rect) -> PaneFrame {
    match motion {
        BandPieceMotion::Resident { .. }
        | BandPieceMotion::Entering { sliding: false, .. }
        | BandPieceMotion::Leaving { sliding: false, .. }
        | BandPieceMotion::ClosingWithColumn { .. } => PaneFrame::new(rect),
        BandPieceMotion::Entering {
            edge,
            after,
            sliding: true,
        } => {
            let base = banded(after, band);
            let shift = edge.shift_to_clip(base, rect);
            PaneFrame::shifted(base, shift, rect)
        },
        BandPieceMotion::Leaving {
            before,
            edge,
            sliding: true,
        } => {
            let base = banded(before, band);
            let shift = edge.shift_to_clip(base, rect);
            PaneFrame::shifted(base, shift, rect)
        },
    }
}

/// The column and row a one-based cell index falls in.
fn position(widths: &[usize], index: usize) -> Option<(usize, usize)> {
    let mut seen = 0;
    for (column, &height) in widths.iter().enumerate() {
        if index <= seen + height {
            return Some((column, index.checked_sub(seen + 1)?));
        }
        seen += height;
    }
    None
}

/// The column `slot` occupies in one queued arrangement.
fn queued_slot_column<Id: Eq>(
    slots: &[Slot<Id>],
    depth: usize,
    slot: &Slot<Id>,
    growth: TileGrowth,
) -> Option<usize> {
    let cell = slots
        .iter()
        .position(|held| held == slot)?
        .saturating_add(TABLE_CELL + 1);
    let layout = column_layout(slots.len() + TABLE_CELL, depth, growth);
    let laid_out = cell.saturating_add(layout.summary_depth.saturating_sub(1));
    position(&layout.widths, laid_out).map(|(column, _)| column)
}

/// Whether every surviving cell uses one of its two endpoint columns.
fn queued_cells_keep_endpoint_columns<Id: Eq>(
    start_slots: &[Slot<Id>],
    start_depth: usize,
    final_slots: &[Slot<Id>],
    final_depth: usize,
    candidate_slots: &[Slot<Id>],
    candidate_depth: usize,
    growth: TileGrowth,
) -> bool {
    start_slots
        .iter()
        .filter(|slot| final_slots.contains(slot) && candidate_slots.contains(slot))
        .all(|slot| {
            let start = queued_slot_column(start_slots, start_depth, slot, growth);
            let finish = queued_slot_column(final_slots, final_depth, slot, growth);
            let candidate = queued_slot_column(candidate_slots, candidate_depth, slot, growth);
            candidate == start || candidate == finish
        })
}

/// Interpolate each edge of `from` toward `to`.
fn lerp_rect(from: Rect, to: Rect, progress: u32) -> Rect {
    let left = lerp(from.x, to.x, progress);
    let top = lerp(from.y, to.y, progress);
    let right = lerp(from.right(), to.right(), progress);
    let bottom = lerp(from.bottom(), to.bottom(), progress);
    Rect {
        x:      left,
        y:      top,
        width:  right.saturating_sub(left),
        height: bottom.saturating_sub(top),
    }
}

/// `from` moved toward `to` by `progress`, on integers throughout.
fn lerp(from: u16, to: u16, progress: u32) -> u16 {
    let (from, to) = (u32::from(from), u32::from(to));
    let value = if to >= from {
        from + (to - from) * progress / PROGRESS_SCALE
    } else {
        from - (from - to) * progress / PROGRESS_SCALE
    };
    u16::try_from(value).unwrap_or(u16::MAX)
}

/// Ease progress in and out, so a transition starts and lands gently
/// rather than stopping dead at both ends.
///
/// Half smoothstep, half linear. Smoothstep alone peaks at one and a
/// half times linear speed, and everything it borrows for that middle it
/// takes from the two ends -- which matters here because the grid moves
/// in whole cells. A tail that flat leaves the last cell of travel
/// waiting several frames longer than the ones before it, so the motion
/// reads as stopping and then jumping into place. Averaging with linear
/// pulls the peak down to one and a quarter, and the slowest step ends
/// up around twice the fastest rather than an order of magnitude.
fn eased(progress: u32) -> u32 {
    let scale = u64::from(PROGRESS_SCALE);
    let progress = u64::from(progress.min(PROGRESS_SCALE));
    let smoothstep =
        (3 * progress * progress * scale - 2 * progress * progress * progress) / (scale * scale);
    u32::try_from(u64::midpoint(smoothstep, progress)).unwrap_or(PROGRESS_SCALE)
}

#[cfg(test)]
#[expect(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use ratatui::layout::Margin;

    use super::*;
    use crate::tiles::constants::MIN_TILE_HEIGHT;
    use crate::tiles::constants::TILE_BORDER_ROWS;
    use crate::tiles::constants::TILE_DEMAND_STEP;

    /// The pid both invocations of the reused-pid test share.
    const TEST_PID: u32 = 42;
    /// The lifetime that tells the replacement apart from the original,
    /// whose lifetime is its pid.
    const TEST_REPLACEMENT_LIFETIME: u64 = 100_002;

    /// A replacement cannot take the old invocation's cell or focus identity.
    #[test]
    fn reused_pids_keep_distinct_tiles_and_focus() {
        let first = (TEST_PID, u64::from(TEST_PID));
        let replacement = (TEST_PID, TEST_REPLACEMENT_LIFETIME);
        let mut demands = TileDemands {
            summary:       0,
            summary_width: 0,
            groups:        vec![TileDemand {
                id:   first,
                rows: 0,
            }],
        };
        let mut grid = seeded_grid();
        grid.sync(&demands, redistribute(MIN_INITIAL_ROWS));
        grid.settle_for_test();
        grid.focus_cell(TABLE_CELL + 1);
        demands.groups.push(TileDemand {
            id:   replacement,
            rows: 0,
        });

        grid.sync(&demands, redistribute(MIN_INITIAL_ROWS));
        grid.settle_for_test();

        assert_eq!(
            shown(&grid),
            vec![TileContent::Group(first), TileContent::Group(replacement)]
        );
        assert_eq!(grid.focus, Focus::Cell(Slot::Group(first)));
        demands.groups.remove(0);
        grid.sync(&demands, redistribute(MIN_INITIAL_ROWS));
        grid.settle_for_test();
        assert_eq!(shown(&grid), vec![TileContent::Group(replacement)]);
        assert_eq!(grid.focus, Focus::Summary);
    }

    /// Width of the rect the placement tests lay their grids out in.
    const TEST_WIDTH: u16 = 80;
    /// Height of the rect the placement tests lay their grids out in.
    const TEST_HEIGHT: u16 = 40;
    /// App-level floor used by the tile-width tests.
    const TEST_TILE_WIDTH: u16 = 40;

    /// The rect the placement tests lay their grids out in.
    const fn test_area() -> Rect { Rect::new(0, 0, TEST_WIDTH, TEST_HEIGHT) }

    /// The rows a cell drawing `rows` of content asks for under the
    /// default settings.
    fn demanded_rows(rows: usize) -> u16 { TileSettings::default().demanded_rows(rows) }

    /// `count` cells with nothing to show, which every column divides
    /// evenly -- the geometry and motion tests are written against it.
    fn even(count: usize) -> HeldCellLayout {
        HeldCellLayout {
            rows:          vec![demanded_rows(0); count],
            focused:       FocusLocation::Departing,
            summary_span:  1,
            summary_depth: 1,
        }
    }

    /// What one scan of `ids` demands, every cell asking for the floor
    /// so the arrangement is all that moves.
    fn quiet<Id: Clone>(ids: &[Id]) -> TileDemands<Id> {
        TileDemands {
            summary:       0,
            summary_width: 0,
            groups:        ids
                .iter()
                .map(|id| TileDemand {
                    id:   id.clone(),
                    rows: 0,
                })
                .collect(),
        }
    }

    /// A tally with one entry per cell of [`test_area`].
    fn blank_tally() -> Vec<u32> { vec![0; usize::from(TEST_WIDTH) * usize::from(TEST_HEIGHT)] }

    /// Count every cell of [`test_area`] that `rect` covers.
    fn paint(tally: &mut [u32], rect: Rect) {
        for y in rect.top()..rect.bottom() {
            for x in rect.left()..rect.right() {
                let index = usize::from(y) * usize::from(TEST_WIDTH) + usize::from(x);
                if let Some(cell) = tally.get_mut(index) {
                    *cell += 1;
                }
            }
        }
    }

    /// How a test grid grows at `initial_rows` under
    /// [`TileFill::AddNew`].
    const fn add_new(initial_rows: usize) -> TileGrowth {
        TileGrowth {
            initial_rows,
            fill: TileFill::AddNew,
            widen_summary: false,
        }
    }

    /// How a test grid grows at `initial_rows` under
    /// [`TileFill::Redistribute`], the fill an app gets by default.
    const fn redistribute(initial_rows: usize) -> TileGrowth {
        TileGrowth {
            initial_rows,
            fill: TileFill::Redistribute,
            widen_summary: false,
        }
    }

    /// How a test grid grows at `initial_rows` with the summary widening.
    const fn widening(initial_rows: usize) -> TileGrowth {
        TileGrowth {
            widen_summary: true,
            ..redistribute(initial_rows)
        }
    }

    #[test]
    fn the_configured_tile_floor_fits_each_column_count_at_its_shared_width() {
        let growth = redistribute(1);
        let settings = TileSettings {
            min_tile_width: TEST_TILE_WIDTH,
            ..TileSettings::default()
        };
        for (count, width) in [(25, 196), (16, 157), (9, 118), (4, 79), (1, 40)] {
            let area = Rect::new(0, 0, width, u16::MAX);
            assert!(fits(area, count, growth, &settings), "{count} at {width}");
            assert!(
                !fits(
                    Rect {
                        width: width - 1,
                        ..area
                    },
                    count,
                    growth,
                    &settings,
                ),
                "{count} below {width}"
            );
        }
    }

    #[test]
    fn the_framework_floor_still_fits_three_columns_in_twenty_two() {
        let area = Rect::new(0, 0, 22, u16::MAX);
        assert!(fits(area, 9, redistribute(1), &TileSettings::default()));
    }

    #[test]
    fn holds_in_counts_the_headed_summary_depth() {
        let growth = redistribute(1);
        let mut grid = TileGrid::<u32>::new();
        grid.set_min_tile_width(TEST_TILE_WIDTH);
        grid.slots = vec![
            Slot::Empty(0),
            Slot::Empty(1),
            Slot::Empty(2),
            Slot::Empty(3),
            Slot::Empty(4),
        ];
        grid.depth = 2;
        let area = Rect::new(0, 0, 79, u16::MAX);

        assert!(fits(area, grid.count(), growth, &grid.settings));
        assert!(!grid.holds_in(area, growth));
    }

    #[test]
    fn a_click_anywhere_in_the_summary_alone_picks_the_summary() {
        let growth = redistribute(4);
        let mut grid = seeded_grid();
        grid.set_min_tile_width(TEST_TILE_WIDTH);
        grid.sync(&quiet(&[7]), growth);
        grid.settle_for_test();
        grid.focus_cell(TABLE_CELL + 1);
        let held_focus = grid.focus.clone();
        grid.set_layout(Rect::new(0, 0, TEST_TILE_WIDTH - 1, TEST_HEIGHT), growth);

        for point in [
            Position::new(0, 0),
            Position::new(TEST_TILE_WIDTH - 2, TEST_HEIGHT - 1),
        ] {
            assert_eq!(grid.cell_at(point), Some(TABLE_CELL));
            grid.focus_cell(TABLE_CELL);
            assert_eq!(grid.focus, held_focus);
        }
    }

    #[test]
    fn focus_returns_to_its_cell_with_the_cells() {
        let growth = redistribute(4);
        let mut grid = seeded_grid();
        grid.set_min_tile_width(TEST_TILE_WIDTH);
        grid.sync(&quiet(&[7]), growth);
        grid.settle_for_test();
        grid.focus_cell(TABLE_CELL + 1);
        let held_focus = grid.focus.clone();
        grid.set_layout(Rect::new(0, 0, TEST_TILE_WIDTH - 1, TEST_HEIGHT), growth);

        assert!(!grid.cycle_focus(CycleDirection::Next));
        grid.focus_step(Direction::Up, growth);
        assert_eq!(grid.focus, held_focus);

        grid.set_layout(Rect::new(0, 0, TEST_WIDTH, TEST_HEIGHT), growth);
        let focused = grid
            .drawing_at(
                Rect::new(0, 0, TEST_WIDTH, TEST_HEIGHT),
                growth,
                PROGRESS_SCALE,
            )
            .pieces
            .into_iter()
            .find(|piece| piece.placement.frame.is_focused())
            .map(|piece| piece.placement.content);
        assert_eq!(focused, Some(TileContent::Group(7)));
    }

    /// A scan of `groups` quiet cells whose summary asks for `width`
    /// columns across.
    fn wide_summary(groups: &[u32], width: u16) -> TileDemands<u32> {
        TileDemands {
            summary_width: width,
            ..quiet(groups)
        }
    }

    /// The summary asks for the fewest columns whose interior holds its
    /// widest line, and for all of them when none does.
    #[test]
    fn the_summary_asks_for_the_fewest_columns_that_hold_its_width() {
        let own = frame_inner(share_borders(
            Rect::new(0, 0, TEST_WIDTH / 2, TEST_HEIGHT),
            test_area(),
        ))
        .width;
        assert_eq!(summary_span(test_area(), 2, 0), 1);
        assert_eq!(summary_span(test_area(), 2, own), 1);
        assert_eq!(summary_span(test_area(), 2, own + 1), 2);
        assert_eq!(summary_span(test_area(), 2, u16::MAX), 2);
        assert_eq!(summary_span(test_area(), 1, u16::MAX), 1);
    }

    /// Widened, the summary keeps the height its own column gives it and
    /// the next column's cells divide what is left below it.
    #[test]
    fn a_widened_summary_keeps_its_height_and_the_next_column_starts_below_it() {
        let settings = TileSettings::default();
        let narrow = Grid::new(test_area(), &even(4), widening(3), &settings);
        let held = HeldCellLayout {
            summary_span: 2,
            ..even(4)
        };
        let wide = Grid::new(test_area(), &held, widening(3), &settings);
        let cell = |grid: &Grid, index: usize| {
            grid.cell(index)
                .expect("a four-cell grid holds cells one to three")
        };
        let was = cell(&narrow, TABLE_CELL);
        let summary = cell(&wide, TABLE_CELL);
        let under = cell(&wide, TABLE_CELL + 1);
        let beside = cell(&wide, TABLE_CELL + 2);
        assert_eq!(wide.span, 2);
        assert_eq!((summary.y, summary.height), (was.y, was.height));
        assert_eq!((summary.x, summary.right()), (0, TEST_WIDTH));
        assert_eq!(
            beside.y, under.y,
            "the next column starts where the summary ends"
        );
        assert_eq!(beside.x, wide.column_rect(1).x);
    }

    /// A column whose cells would not fit below the summary stops it,
    /// the same as the right edge does.
    #[test]
    fn a_column_with_no_room_below_the_summary_stops_it() {
        let settings = TileSettings::default();
        let mut held = HeldCellLayout {
            summary_span: 2,
            ..even(4)
        };
        assert_eq!(columns(4, widening(3)), vec![2, 2]);
        assert_eq!(
            Grid::new(test_area(), &held, widening(3), &settings).span,
            2
        );
        held.rows[0] = TEST_HEIGHT;
        assert_eq!(
            Grid::new(test_area(), &held, widening(3), &settings).span,
            1
        );
    }

    /// The grid widens the summary only while the setting is on.
    #[test]
    fn the_summary_widens_only_when_the_setting_is_on() {
        for (growth, span) in [(redistribute(3), 1), (widening(3), 2)] {
            let mut grid = TileGrid::new();
            grid.set_layout(test_area(), growth);
            grid.sync(&wide_summary(&[1, 2, 3], u16::MAX), growth);
            grid.settle_for_test();
            assert_eq!(grid.held.summary_span, span, "{growth:?}");
            let width = grid.content_widths(test_area(), growth)[0].1;
            let expected = if span == 1 {
                TEST_WIDTH / 2 - 1
            } else {
                TEST_WIDTH - 2
            };
            assert_eq!(width, expected, "{growth:?}");
        }
    }

    /// Up from the top of a column the summary covers reaches the
    /// summary, and a sideways step from it passes over what it covers.
    #[test]
    fn the_arrows_treat_a_widened_summary_as_heading_what_it_covers() {
        let growth = widening(3);
        let mut grid = TileGrid::new();
        grid.set_layout(test_area(), growth);
        grid.sync(&wide_summary(&[1, 2, 3], u16::MAX), growth);
        grid.settle_for_test();
        grid.focus_cell(TABLE_CELL + 2);
        grid.apply(TileAction::FocusUp, growth);
        assert_eq!(grid.focus, Focus::Summary);
        grid.apply(TileAction::FocusRight, growth);
        assert_eq!(grid.focus, Focus::Summary, "nothing stands right of it");
        grid.apply(TileAction::FocusDown, growth);
        assert_eq!(grid.focus, Focus::Cell(Slot::Group(1)));
        grid.focus_cell(TABLE_CELL + 3);
        grid.apply(TileAction::FocusLeft, growth);
        assert_eq!(grid.focus, Focus::Cell(Slot::Group(1)));
    }

    /// Rows per column for every count up to `count`, so a walk through
    /// the sequence reads as one table.
    fn walk(count: usize, growth: TileGrowth) -> Vec<Vec<usize>> {
        (1..=count).map(|n| columns(n, growth)).collect()
    }

    /// Both fills, for the rules that hold whichever one is set.
    const FILLS: [TileFill; 2] = [TileFill::AddNew, TileFill::Redistribute];

    /// Below the initial rows there is one column, whichever the fill.
    #[test]
    fn the_first_column_fills_before_a_second_one_opens() {
        for fill in FILLS {
            let growth = TileGrowth {
                initial_rows: 4,
                fill,
                widen_summary: false,
            };
            assert_eq!(walk(4, growth), vec![vec![1], vec![2], vec![3], vec![4]]);
        }
    }

    /// The cell past the initial rows does not extend the column and
    /// does not open a column of one beside it. It rearranges the whole
    /// grid into the smallest square that holds the cells.
    #[test]
    fn the_cell_past_the_initial_rows_rearranges_the_whole_grid() {
        assert_eq!(columns(4, add_new(3)), vec![2, 2]);
        assert_eq!(columns(5, add_new(4)), vec![3, 2]);
        assert_eq!(columns(9, add_new(8)), vec![3, 3, 3]);
    }

    /// The whole walk at three initial rows: a column growing to three,
    /// then the square it rearranges into and fills a column at a time.
    #[test]
    fn the_columns_fill_the_smallest_square_that_holds_them() {
        assert_eq!(
            walk(9, add_new(3)),
            vec![
                vec![1],
                vec![2],
                vec![3],
                vec![2, 2],
                vec![3, 2],
                vec![3, 3],
                vec![3, 3, 1],
                vec![3, 3, 2],
                vec![3, 3, 3],
            ]
        );
    }

    /// The same walk under redistribute: the cell that opens the third
    /// column takes one from each of the first two, so it does not
    /// stand alone.
    #[test]
    fn redistribute_keeps_the_columns_within_one_cell_of_each_other() {
        assert_eq!(
            walk(9, redistribute(3)),
            vec![
                vec![1],
                vec![2],
                vec![3],
                vec![2, 2],
                vec![3, 2],
                vec![3, 3],
                vec![3, 2, 2],
                vec![3, 3, 2],
                vec![3, 3, 3],
            ]
        );
    }

    /// A full square rearranges again rather than opening a column
    /// beside itself: the tenth cell at three initial rows makes every
    /// column four tall instead of standing alone in a fourth column.
    #[test]
    fn a_full_square_grows_a_row_before_it_grows_a_column() {
        assert_eq!(columns(10, add_new(3)), vec![4, 4, 2]);
        assert_eq!(columns(12, add_new(3)), vec![4, 4, 4]);
        assert_eq!(columns(13, add_new(3)), vec![4, 4, 4, 1]);
        assert_eq!(columns(16, add_new(3)), vec![4, 4, 4, 4]);
        assert_eq!(columns(17, add_new(3)), vec![5, 5, 5, 2]);
    }

    /// Twelve cells stand as three columns of four either way. The
    /// thirteenth opens a fourth column: add new stands it there alone,
    /// and redistribute deals the thirteen out as four, three, three and
    /// three. The fourteenth fills the fourth column one more under add
    /// new, and makes the second column four tall under redistribute.
    #[test]
    fn twelve_thirteen_and_fourteen_cells_under_each_fill() {
        assert_eq!(columns(12, add_new(4)), vec![4, 4, 4]);
        assert_eq!(columns(13, add_new(4)), vec![4, 4, 4, 1]);
        assert_eq!(columns(14, add_new(4)), vec![4, 4, 4, 2]);

        assert_eq!(columns(12, redistribute(4)), vec![4, 4, 4]);
        assert_eq!(columns(13, redistribute(4)), vec![4, 3, 3, 3]);
        assert_eq!(columns(14, redistribute(4)), vec![4, 4, 3, 3]);
    }

    /// The fill decides how many cells each column holds and never how
    /// many columns there are.
    #[test]
    fn both_fills_open_the_same_number_of_columns() {
        for initial_rows in 1..=6 {
            for count in 1..=60 {
                assert_eq!(
                    columns(count, add_new(initial_rows)).len(),
                    columns(count, redistribute(initial_rows)).len(),
                    "count {count} at {initial_rows} initial rows"
                );
            }
        }
    }

    /// Under redistribute no column holds more than one cell over any
    /// other, and the taller columns stand to the left of the shorter.
    #[test]
    fn redistributed_columns_differ_by_at_most_one_taller_first() {
        for initial_rows in 1..=6 {
            for count in 1..=60 {
                let widths = columns(count, redistribute(initial_rows));
                let (Some(&first), Some(&last)) = (widths.first(), widths.last()) else {
                    continue;
                };
                assert!(
                    first - last <= 1,
                    "count {count} at {initial_rows} initial rows: {widths:?}"
                );
                assert!(
                    widths.windows(2).all(|pair| pair[0] >= pair[1]),
                    "count {count} at {initial_rows} initial rows: {widths:?}"
                );
            }
        }
    }

    /// The grid comes back down through the arrangements it went up
    /// through, because the answer is a function of the count and
    /// nothing else. Taking the fourth cell away leaves a column of
    /// three rather than a two-by-two with a hole in it.
    #[test]
    fn the_grid_comes_back_down_the_way_it_went_up() {
        for fill in FILLS {
            let growth = TileGrowth {
                initial_rows: 3,
                fill,
                widen_summary: false,
            };
            assert_eq!(columns(5, growth), vec![3, 2]);
            assert_eq!(columns(4, growth), vec![2, 2]);
            assert_eq!(columns(3, growth), vec![3]);
        }
    }

    #[test]
    fn the_fifth_column_opens_once_every_column_stands_five_tall() {
        assert_eq!(columns(21, add_new(4)), vec![5, 5, 5, 5, 1]);
        assert_eq!(columns(25, add_new(4)), vec![5, 5, 5, 5, 5]);
    }

    #[test]
    fn the_square_after_five_grows_the_same_way() {
        assert_eq!(columns(26, add_new(4)), vec![6, 6, 6, 6, 2]);
        assert_eq!(columns(30, add_new(4)), vec![6, 6, 6, 6, 6]);
        assert_eq!(columns(31, add_new(4)), vec![6, 6, 6, 6, 6, 1]);
        assert_eq!(columns(36, add_new(4)), vec![6, 6, 6, 6, 6, 6]);
    }

    #[test]
    fn every_arrangement_holds_exactly_the_cells_asked_for() {
        for fill in FILLS {
            for initial_rows in 1..=6 {
                for count in 1..=60 {
                    let growth = TileGrowth {
                        initial_rows,
                        fill,
                        widen_summary: false,
                    };
                    assert_eq!(
                        columns(count, growth).iter().sum::<usize>(),
                        count,
                        "count {count} at {initial_rows} initial rows, {fill:?}"
                    );
                }
            }
        }
    }

    /// Between one rearrangement and the next, adding a cell under add
    /// new moves nothing already on the screen: it lands at the foot of
    /// the last column or opens the next one. The rearrangements are
    /// the only motion, which is what makes them worth watching.
    #[test]
    fn nothing_already_placed_moves_until_the_grid_rearranges() {
        for initial_rows in 1..=6 {
            for count in 1..60 {
                let before = columns(count, add_new(initial_rows));
                let after = columns(count + 1, add_new(initial_rows));
                // The first column stands at the height every column but
                // the last is filled to, so a change there is the
                // rearrangement itself.
                if before.first() != after.first() {
                    continue;
                }
                for index in 1..=count {
                    assert_eq!(
                        position(&before, index),
                        position(&after, index),
                        "cell {index} moved going from {count} to {} at {initial_rows} initial \
                         rows",
                        count + 1
                    );
                }
            }
        }
    }

    #[test]
    fn initial_rows_below_one_is_treated_as_one() {
        for fill in FILLS {
            assert_eq!(
                columns(
                    3,
                    TileGrowth {
                        initial_rows: 0,
                        fill,
                        widen_summary: false,
                    }
                ),
                columns(
                    3,
                    TileGrowth {
                        initial_rows: 1,
                        fill,
                        widen_summary: false,
                    }
                )
            );
        }
    }

    /// What one scan of `(id, rows)` pairs demands, so a test can say
    /// which cells are busy and which are idle.
    fn busy(groups: &[(u32, usize)]) -> TileDemands<u32> {
        TileDemands {
            summary:       0,
            summary_width: 0,
            groups:        groups
                .iter()
                .map(|&(id, rows)| TileDemand { id, rows })
                .collect(),
        }
    }

    /// The rows one column hands each of its cells, top to bottom.
    ///
    /// The allocation rather than the rects it becomes: neighbouring
    /// cells share a border line, so the rects overlap by one and no
    /// longer add up to the column.
    fn column_rows(wants: &[u16], focused: ColumnFocus) -> Vec<u16> {
        shares(
            wants,
            TEST_HEIGHT,
            ColumnHead::Cell,
            focused,
            MIN_TILE_HEIGHT,
        )
        .into_iter()
        .map(|size| match size {
            PaneAxisSize::Fixed(rows) => rows,
            PaneAxisSize::Fill(weight) => weight,
        })
        .collect()
    }

    /// The whole point of dividing a column by demand: a cell whose
    /// content does not fit its even share is drawn taller than the
    /// idle cells beside it.
    #[test]
    fn a_cell_that_has_run_out_of_room_takes_what_an_idle_one_is_not_using() {
        let share = TEST_HEIGHT / 4;
        let rows = column_rows(
            &[demanded_rows(usize::from(share) * 2), 0, 0, 0],
            ColumnFocus::Outside,
        );
        assert!(
            rows[0] > share,
            "the cell that ran out of room is given more than its share: {rows:?}"
        );
        assert!(
            rows[1].abs_diff(rows[3]) <= 1,
            "the idle cells give up the same amount as each other, give or take the row \
             integer division leaves over: {rows:?}"
        );
    }

    /// Room reaches a cell from either side of it. The rule this
    /// replaced could only pass space downward, so a cell at the top of
    /// a column could never draw on a quiet one below it.
    #[test]
    fn room_reaches_a_cell_from_either_side_of_it() {
        // Four cells into forty rows, so every cell's share -- and so
        // every cell's ceiling -- is the same and the two runs are
        // comparable.
        let asked = demanded_rows(usize::from(TEST_HEIGHT));
        let third = column_rows(&[0, 0, asked, 0], ColumnFocus::Outside);
        assert!(
            third[2] > third[1] && third[2] > third[3],
            "the cell draws on the neighbours above it as well as below: {third:?}"
        );
        let first = column_rows(&[asked, 0, 0, 0], ColumnFocus::Outside);
        assert_eq!(
            third[2], first[0],
            "and takes the same room wherever in the column it sits"
        );
    }

    /// Nothing is asking for the room going spare, so nothing clever
    /// happens: the column is divided evenly.
    #[test]
    fn a_column_where_everything_fits_divides_evenly() {
        assert_eq!(
            column_rows(&[0, 0, 0, 0], ColumnFocus::Outside),
            apportion(&[1, 1, 1, 1], TEST_HEIGHT)
        );
    }

    /// A cell hands over every row its contents are not using, not
    /// some fraction of its share: a quiet cell beside a busy one is
    /// left at what it is showing.
    #[test]
    fn a_cell_gives_up_every_row_its_contents_do_not_need() {
        let rows = column_rows(&[u16::MAX, 0, 0, 0], ColumnFocus::Outside);
        for quiet in &rows[1..] {
            assert_eq!(
                *quiet, MIN_TILE_HEIGHT,
                "the quiet cells keep only what a cell can be read at: {rows:?}"
            );
        }
        assert_eq!(
            rows.iter().sum::<u16>(),
            TEST_HEIGHT,
            "and the column is still filled exactly"
        );
    }

    /// No cell is pushed under [`MIN_TILE_HEIGHT`], however hard its
    /// neighbours are asking.
    #[test]
    fn no_cell_is_pushed_below_what_a_cell_can_be_read_at() {
        let rows = column_rows(&[u16::MAX, u16::MAX, 0, 0], ColumnFocus::Outside);
        assert!(
            rows.iter().all(|&cell| cell >= MIN_TILE_HEIGHT),
            "the floor holds for every cell: {rows:?}"
        );
    }

    /// Demand is quantised, so a cell has to gain a whole
    /// [`TILE_DEMAND_STEP`] before it asks for anything more.
    #[test]
    fn demand_holds_still_until_it_has_a_whole_step_more_to_ask_for() {
        assert_eq!(demanded_rows(1), demanded_rows(TILE_DEMAND_STEP));
        assert!(demanded_rows(TILE_DEMAND_STEP + 1) > demanded_rows(TILE_DEMAND_STEP));
        assert_eq!(
            demanded_rows(0),
            TILE_BORDER_ROWS,
            "an empty cell asks for nothing beyond its border"
        );
    }

    /// Where the room does not go round, the cell being watched is the
    /// one that gets it and the rest divide what is left.
    #[test]
    fn the_focused_cell_wins_the_room_that_does_not_go_round() {
        let share = TEST_HEIGHT / 4;
        let asked = [0, share * 2, share * 2, 0];
        let level = column_rows(&asked, ColumnFocus::Outside);
        let watched = column_rows(&asked, ColumnFocus::Row(1));
        assert_eq!(
            level[1], level[2],
            "unwatched, the two short cells divide it between them: {level:?}"
        );
        assert!(
            watched[1] > watched[2],
            "watched, the ring is served first: {watched:?}"
        );
        assert_eq!(
            watched.iter().sum::<u16>(),
            TEST_HEIGHT,
            "and the column is still filled exactly"
        );
    }

    /// Focus settles nothing while there is enough room to go round:
    /// every short cell is filled whether it is watched or not.
    #[test]
    fn focus_changes_nothing_while_there_is_room_to_go_round() {
        let share = TEST_HEIGHT / 4;
        for asked in [vec![demanded_rows(0); 4], vec![0, share + 1, share + 1, 0]] {
            assert_eq!(
                shares(
                    &asked,
                    TEST_HEIGHT,
                    ColumnHead::Cell,
                    ColumnFocus::Row(1),
                    MIN_TILE_HEIGHT
                ),
                shares(
                    &asked,
                    TEST_HEIGHT,
                    ColumnHead::Cell,
                    ColumnFocus::Outside,
                    MIN_TILE_HEIGHT
                ),
                "nothing is competing, so the ring changes nothing: {asked:?}"
            );
        }
    }

    /// Focus is a claim on the room going spare rather than a right to
    /// it: a column where every cell has run out hands it nothing.
    #[test]
    fn focus_takes_nothing_from_a_column_that_has_no_room_to_spare() {
        let asked = vec![u16::MAX; 3];
        assert_eq!(
            shares(
                &asked,
                TEST_HEIGHT,
                ColumnHead::Cell,
                ColumnFocus::Row(1),
                MIN_TILE_HEIGHT
            ),
            shares(
                &asked,
                TEST_HEIGHT,
                ColumnHead::Cell,
                ColumnFocus::Outside,
                MIN_TILE_HEIGHT
            ),
            "every cell is short, so the ring changes nothing"
        );
    }

    /// A cell keeps the rows it grew into once its content shrinks:
    /// nothing else is asking for the room, so handing it back would be
    /// a resize with nothing to show for it.
    #[test]
    fn a_cell_keeps_what_it_grew_to_while_nothing_encroaches() {
        let mut grid = seeded_grid();
        grid.sync(&busy(&[(7, usize::from(TEST_HEIGHT))]), redistribute(4));
        grid.settle_for_test();
        let grown = grid.held.clone();

        grid.sync(&quiet(&[7]), redistribute(4));
        grid.settle_for_test();

        assert_eq!(
            grid.held, grown,
            "its rows went away and it stands where it grew to"
        );
    }

    /// Another cell encroaching is what hands the room back. The whole
    /// grid drops to what its cells are showing and divides again.
    #[test]
    fn a_cell_hands_its_room_back_once_another_encroaches() {
        let mut grid = seeded_grid();
        grid.sync(&busy(&[(7, usize::from(TEST_HEIGHT))]), redistribute(4));
        grid.settle_for_test();
        grid.sync(&quiet(&[7]), redistribute(4));
        grid.settle_for_test();
        let grown = grid.held.clone();

        grid.sync(
            &busy(&[(7, 0), (8, usize::from(TEST_HEIGHT))]),
            redistribute(4),
        );
        grid.settle_for_test();

        assert_ne!(
            grid.held, grown,
            "the cell arriving is the encroachment the grid re-divides on"
        );
        assert_eq!(
            grid.held.rows[1],
            demanded_rows(0),
            "and the cell that had grown is back to what it is showing"
        );
    }

    /// A scan that moves no cell but changes what one is asking for
    /// still travels: the column re-divides over the animation rather
    /// than snapping to the new split.
    #[test]
    fn a_column_re_dividing_travels_the_way_a_cell_moving_does() {
        let mut grid = seeded_grid();
        grid.sync(&quiet(&[7]), redistribute(4));
        grid.settle_for_test();
        assert!(
            matches!(grid.motion, GridMotion::Settled),
            "the arrangement has settled"
        );

        grid.sync(&busy(&[(7, usize::from(TEST_HEIGHT))]), redistribute(4));

        let transition = match &grid.motion {
            GridMotion::Moving(transition) => Some(transition),
            GridMotion::Settled => None,
        }
        .expect("the new demand starts a transition");
        assert_eq!(
            transition.from, grid.slots,
            "no cell moved -- the same slots stand at the same numbers"
        );
        assert_ne!(
            transition.held, grid.held,
            "but the column is divided differently at either end of it"
        );
    }

    /// Divided by demand or evenly, the cells still tile their area
    /// exactly.
    #[test]
    fn a_column_divided_by_demand_covers_its_area_the_way_an_even_one_does() {
        let mut covered = blank_tally();
        let mut interiors = blank_tally();
        let share = usize::from(TEST_HEIGHT / 4);
        let rows = vec![
            demanded_rows(share * 2),
            0,
            demanded_rows(share + 1),
            0,
            demanded_rows(share * 2),
        ];
        let held = HeldCellLayout {
            rows,
            focused: FocusLocation::Cell(2 + TABLE_CELL),
            summary_span: 1,
            summary_depth: 1,
        };
        let grid = Grid::new(
            test_area(),
            &held,
            redistribute(4),
            &TileSettings::default(),
        );
        for index in 1..=held.rows.len() {
            let rect = grid.cell(index).expect("cell is in the grid");
            paint(&mut covered, rect);
            paint(&mut interiors, rect.inner(Margin::new(1, 1)));
        }
        assert!(
            covered.iter().all(|hits| *hits >= 1),
            "the division leaves a gap"
        );
        assert!(
            interiors.iter().all(|hits| *hits <= 1),
            "the division draws two cells into the same place"
        );
    }

    /// A second column starts on the same screen column the first one
    /// ends on. That single shared line is what the grid is drawn from.
    #[test]
    fn a_column_starts_where_the_one_before_it_ends() {
        let grid = Grid::new(
            test_area(),
            &even(5),
            redistribute(4),
            &TileSettings::default(),
        );
        let first = grid.cell(1).expect("cell is in the grid");
        let second = grid.cell(5).expect("cell is in the grid");
        assert_eq!(first.right() - 1, second.left());
    }

    /// A stacked cell starts on the row the one above it ends on.
    #[test]
    fn a_row_starts_where_the_one_above_it_ends() {
        let grid = Grid::new(
            test_area(),
            &even(2),
            redistribute(4),
            &TileSettings::default(),
        );
        let first = grid.cell(1).expect("cell is in the grid");
        let second = grid.cell(2).expect("cell is in the grid");
        assert_eq!(first.bottom() - 1, second.top());
    }

    #[test]
    fn a_settled_grid_places_one_piece_per_cell() {
        let placements = TileGrid::<u32>::new().placements(test_area(), redistribute(4));
        assert_eq!(placements.len(), 1);
        assert_eq!(placements[0].content, TileContent::Summary);
        assert_eq!(placements[0].frame.rect(), test_area());
    }

    #[test]
    fn placements_are_the_summary_alone_while_the_cells_are_hidden() {
        let growth = redistribute(4);
        let wide_area = Rect::new(0, 0, 200, TEST_HEIGHT);
        let narrow_area = Rect::new(0, 0, 100, TEST_HEIGHT);
        let mut grid = TileGrid::new();
        grid.set_min_tile_width(TEST_TILE_WIDTH);
        grid.set_layout(wide_area, growth);
        grid.sync(&quiet(&[1, 2, 3, 4, 5, 6]), growth);
        grid.settle_for_test();
        assert_eq!(grid.drawing(wide_area, growth).column_bands.len(), 3);
        assert_eq!(grid.drawing(narrow_area, growth).column_bands, Vec::new());

        let placements = grid.placements(narrow_area, growth);
        assert_eq!(placements.len(), 1);
        assert_eq!(placements[0].content, TileContent::Summary);
        assert_eq!(placements[0].frame.rect(), narrow_area);
        assert_eq!(placements[0].frame.clip(), narrow_area);
        assert!(placements[0].frame.is_focused());
    }

    #[test]
    fn the_table_cell_is_never_removed() {
        let mut grid = TileGrid::<u32>::new();
        grid.remove();
        assert_eq!(grid.count(), 1);
    }

    #[test]
    fn a_grid_with_no_room_refuses_to_grow() {
        let mut grid = TileGrid::<u32>::new();
        grid.set_layout(Rect::new(0, 0, 4, 2), redistribute(4));
        grid.add(redistribute(4));
        assert_eq!(grid.count(), 1);
    }

    /// Cell 5 leaves the top of column two and arrives at the bottom of
    /// column one, so mid-transition it is drawn twice -- once in each.
    #[test]
    fn a_cell_changing_column_is_drawn_in_both() {
        let area = test_area();
        let growth = redistribute(4);
        let before = (1..=15).collect::<Vec<_>>();
        let after = (1..=16).collect::<Vec<_>>();
        let grid = synced_motion(&before, &after, growth);
        let drawing = grid.drawing_at(area, growth, PROGRESS_SCALE / 2);
        let pieces = drawing
            .placements()
            .filter(|placement| placement.content == TileContent::Group(4))
            .collect::<Vec<_>>();
        assert_eq!(pieces.len(), 2);
        let leaving = pieces
            .iter()
            .find(|placement| placement.frame.shift() < 0)
            .expect("the piece leaving travels upward");
        let arriving = pieces
            .iter()
            .find(|placement| placement.frame.shift() > 0)
            .expect("the piece arriving is still below");
        assert_eq!(
            (leaving.frame.clip().x, leaving.frame.clip().width),
            (drawing.column_bands[1].x, drawing.column_bands[1].width),
            "the leaving piece stays in its old column"
        );
        assert_eq!(
            (arriving.frame.clip().x, arriving.frame.clip().width),
            (drawing.column_bands[0].x, drawing.column_bands[0].width),
            "the arriving piece stands in its new column"
        );
        assert_eq!(
            i32::from(leaving.frame.rect().bottom()) + leaving.frame.shift(),
            i32::from(leaving.frame.clip().bottom()),
            "the leaving contents share the top-edge divider"
        );
        assert_eq!(
            i32::from(arriving.frame.rect().top()) + arriving.frame.shift(),
            i32::from(arriving.frame.clip().top()),
            "the arriving contents share the bottom-edge divider"
        );
    }

    /// One cell as the wrapping tests draw it; nothing about the motion
    /// depends on what it holds.
    fn drawn() -> CellAppearance<u32> {
        CellAppearance {
            content: TileContent::Summary,
            focused: false,
        }
    }

    /// Place one cell motion through the same shared-band path as a frame.
    fn placed_motion(
        before: &Grid,
        after: &Grid,
        transition: CellTransition,
        progress: u32,
        drawn: CellAppearance<u32>,
    ) -> Vec<TilePlacement<u32>> {
        let bands: Vec<Rect> = (0..before.columns.len().max(after.columns.len()))
            .map(|column| column_band(before, after, column, progress))
            .collect();
        let mut pieces = Vec::new();
        moving_cell(before, after, transition, progress, drawn, &mut pieces);
        place_band_pieces(before, after, &bands, pieces)
            .into_iter()
            .map(|piece| piece.placement)
            .collect()
    }

    /// The pile of lines a closing column used to leave behind. A cell
    /// crossing columns was drawn in the columns as they stood before
    /// the step, so while the grid re-divided itself around it the two
    /// pieces sat across the cells that had already widened past them --
    /// their borders standing inside another cell rather than on a
    /// boundary. A wrapping piece shares its edges with whatever stays
    /// put in the column it is in.
    #[test]
    fn a_wrapping_cell_shares_its_column_edges_with_the_cells_that_stay() {
        let area = test_area();
        let before = Grid::new(area, &even(7), add_new(3), &TileSettings::default());
        let after = Grid::new(area, &even(6), add_new(3), &TileSettings::default());
        assert_eq!(before.widths, vec![3, 3, 1]);
        assert_eq!(after.widths, vec![3, 3], "the last column goes too");
        let half = PROGRESS_SCALE / 2;

        // Cell four leaves the head of column two for the foot of
        // column one, while cell five stays in column two throughout.
        let bands: Vec<Rect> = (0..before.columns.len().max(after.columns.len()))
            .map(|column| column_band(&before, &after, column, half))
            .collect();
        let mut pieces = Vec::new();
        moving_cell(
            &before,
            &after,
            CellTransition::PresentAtBothEnds {
                before_cell: 4,
                after_cell:  3,
            },
            half,
            drawn(),
            &mut pieces,
        );
        moving_cell(
            &before,
            &after,
            CellTransition::PresentAtBothEnds {
                before_cell: 5,
                after_cell:  4,
            },
            half,
            drawn(),
            &mut pieces,
        );
        let placed = place_band_pieces(&before, &after, &bands, pieces);

        let column = |placement: &TilePlacement<u32>| {
            let rect = placement.frame.rect();
            (rect.x, rect.width)
        };
        assert_eq!(
            column(&placed[0].placement),
            column(&placed[2].placement),
            "the piece on its way out stands in the column as it is now"
        );
    }

    /// A cell whose column goes with it is squeezed off the right edge
    /// by the columns widening behind it, rather than left standing
    /// where its column used to be.
    #[test]
    fn a_wrapping_cell_leaving_with_its_column_is_squeezed_off_the_edge() {
        let area = test_area();
        let before = Grid::new(area, &even(7), add_new(3), &TileSettings::default());
        let after = Grid::new(area, &even(6), add_new(3), &TileSettings::default());
        let half = PROGRESS_SCALE / 2;

        let out = placed_motion(
            &before,
            &after,
            CellTransition::PresentAtBothEnds {
                before_cell: 7,
                after_cell:  6,
            },
            half,
            drawn(),
        );

        let leaving = out[0].frame.rect();
        assert!(
            leaving.width < before.column_rect(2).width,
            "the column it stands in is being taken away"
        );
        assert_eq!(leaving.right(), area.right(), "against the right edge");
        assert_eq!(out[0].frame.clip(), leaving, "and it is cut off there");
    }

    /// Closing the one cell a column holds takes the column with it, so
    /// the cell is squeezed off the right edge by the columns widening
    /// behind it -- one line sweeping sideways, not a second one
    /// sliding down at the same time.
    #[test]
    fn a_cell_taking_its_column_with_it_closes_off_the_right_edge() {
        let area = test_area();
        let before = Grid::new(area, &even(7), add_new(3), &TileSettings::default());
        let after = Grid::new(area, &even(6), add_new(3), &TileSettings::default());
        assert_eq!(
            before.widths,
            vec![3, 3, 1],
            "cell seven is a column of one"
        );
        let closed = column_band(&before, &after, 2, PROGRESS_SCALE);
        assert_eq!(closed.width, 1, "only its outer line remains");
        assert_eq!(closed.x, area.right() - 1, "on the last drawable column");
        assert_eq!(
            (closed.y, closed.height),
            (area.y, area.height),
            "and travels nowhere vertically"
        );
    }

    /// Closing the last cell of a column the grid keeps drops it onto
    /// that column's floor, which the cell above expands down onto.
    #[test]
    fn a_cell_leaving_a_column_that_stays_closes_onto_its_floor() {
        let area = test_area();
        let before = Grid::new(area, &even(6), redistribute(4), &TileSettings::default());
        let after = Grid::new(area, &even(5), redistribute(4), &TileSettings::default());
        assert_eq!(before.widths, vec![3, 3], "cell six sits under cell five");
        let bands = column_bands(&before, &after, PROGRESS_SCALE);
        let mut pieces = Vec::new();
        for cell in 1..=5 {
            moving_cell(
                &before,
                &after,
                CellTransition::PresentAtBothEnds {
                    before_cell: cell,
                    after_cell:  cell,
                },
                PROGRESS_SCALE,
                drawn(),
                &mut pieces,
            );
        }
        moving_cell(
            &before,
            &after,
            CellTransition::Departing { before_cell: 6 },
            PROGRESS_SCALE,
            drawn(),
            &mut pieces,
        );
        let closed = place_band_pieces(&before, &after, &bands, pieces)
            .last()
            .map(|piece| piece.placement.frame.clip())
            .unwrap_or_default();
        assert_eq!(closed.height, 1, "only its edge remains");
        assert_eq!(
            closed.y,
            after.column_rect(1).bottom() - 1,
            "against the column floor"
        );
        assert_eq!(
            (closed.x, closed.width),
            (after.column_rect(1).x, after.column_rect(1).width)
        );
    }

    /// A grid the tests can grow without a terminal under it.
    fn seeded_grid<Id: Clone + Eq + Debug>() -> TileGrid<Id> {
        let mut grid = TileGrid::new();
        grid.set_layout(test_area(), redistribute(4));
        grid
    }

    #[test]
    fn every_queued_state_keeps_surviving_cells_in_an_endpoint_column() {
        let growth = redistribute(2);
        for (before, after, start_depth, final_depth) in [
            (&[1, 2][..], &[1, 99, 2][..], 2, 3),
            (&[1, 99, 2][..], &[1, 2][..], 3, 2),
        ] {
            let demands = |ids: &[u32]| TileDemands {
                summary:       35,
                summary_width: 0,
                groups:        ids.iter().map(|&id| TileDemand { id, rows: 0 }).collect(),
            };
            let mut grid = seeded_grid();
            grid.set_layout(test_area(), growth);
            grid.sync(&demands(before), growth);
            grid.settle_for_test();
            assert_eq!(grid.depth, start_depth);
            let start = grid.slots.clone();

            grid.sync(&demands(after), growth);
            let finish = grid.target();
            assert_eq!(grid.target_depth(), final_depth);
            assert_eq!(
                finish,
                after.iter().copied().map(Slot::Group).collect::<Vec<_>>()
            );
            for (slots, depth) in std::iter::once((&grid.slots, grid.depth))
                .chain(grid.pending.iter().map(|step| (&step.slots, step.depth)))
            {
                assert!(
                    queued_cells_keep_endpoint_columns(
                        &start,
                        start_depth,
                        &finish,
                        final_depth,
                        slots,
                        depth,
                        growth,
                    ),
                    "{before:?}@{start_depth} -> {after:?}@{final_depth} queued {slots:?}@{depth}"
                );
            }
        }
    }

    #[test]
    fn adding_the_tenth_live_cell_never_draws_a_survivor_in_a_third_column() {
        let area = Rect::new(0, 0, 200, 50);
        let growth = TileGrowth::default();
        let demands = TileDemands {
            summary:       13,
            summary_width: 0,
            groups:        (1..=8).map(|id| TileDemand { id, rows: 13 }).collect(),
        };
        let mut grid = TileGrid::new();
        grid.set_layout(area, growth);
        grid.sync(&demands, growth);
        grid.settle_for_test();
        assert_eq!(grid.count(), 9);
        assert_eq!(grid.depth, 1);

        grid.apply(TileAction::Add, growth);
        assert_eq!(grid.target().len() + TABLE_CELL, 10);
        assert_eq!(grid.target_depth(), 2);

        let mut steps = 0;
        while let GridMotion::Moving(transition) = &grid.motion {
            steps += 1;
            let before = Grid::new(area, &transition.held, growth, &grid.settings);
            let after_held = grid.drawn_held();
            let after = Grid::new(area, &after_held, growth, &grid.settings);
            let survivors = transition
                .from
                .iter()
                .filter(|slot| grid.slots.contains(slot))
                .filter_map(|slot| match slot {
                    Slot::Group(id) => Some((*id, slot)),
                    Slot::Empty(_) => None,
                })
                .collect::<Vec<_>>();
            for raw in motion_steps() {
                let drawing = grid.drawing_at(area, growth, raw);
                for &(id, slot) in &survivors {
                    let Some(before_cell) = cell_of(&transition.from, slot) else {
                        continue;
                    };
                    let Some(after_cell) = cell_of(&grid.slots, slot) else {
                        continue;
                    };
                    let endpoint_columns =
                        [before.column_of(before_cell), after.column_of(after_cell)];
                    for placement in drawing
                        .placements()
                        .filter(|placement| placement.content == TileContent::Group(id))
                    {
                        let clip = placement.frame.clip();
                        assert!(
                            endpoint_columns.into_iter().flatten().any(|column| {
                                drawing.column_bands.get(column).is_some_and(|band| {
                                    clip.x == band.x && clip.width == band.width
                                })
                            }),
                            "step {steps} at raw {raw} draws {id} in {clip:?}, outside endpoint columns {endpoint_columns:?}"
                        );
                    }
                }
            }
            grid.advance();
        }
        assert!(steps > 0, "adding the tenth cell queues motion");
    }

    /// Twenty-four exact points including both ends of a motion.
    fn motion_steps() -> impl Iterator<Item = u32> {
        (0..24).map(|step| step * PROGRESS_SCALE / 23)
    }

    /// A settled four-cell column beginning its two-column opening.
    fn opening_column() -> TileGrid<u32> {
        let growth = redistribute(4);
        let mut grid = seeded_grid();
        grid.sync(&quiet(&[1, 2, 3]), growth);
        grid.settle_for_test();
        grid.add(growth);
        grid
    }

    #[test]
    fn column_bands_tile_the_area_through_a_transition() {
        let growth = redistribute(4);
        let opening = opening_column();
        let mut closing = opening_column();
        closing.settle_for_test();
        closing.remove();

        for grid in [&opening, &closing] {
            for raw in motion_steps() {
                let bands = grid.drawing_at(test_area(), growth, raw).column_bands;
                assert_eq!(
                    bands.first().map(|band| band.left()),
                    Some(test_area().left())
                );
                assert_eq!(
                    bands.last().map(|band| band.right() - 1),
                    Some(test_area().right() - 1)
                );
                for pair in bands.windows(2) {
                    assert_eq!(pair[0].right() - 1, pair[1].left());
                }
            }
        }
    }

    /// Every placement assigned to a band shares both outside rows and
    /// every divider between them with the piece beside it.
    fn assert_shared_piece_rows(grid: &TileGrid<u32>, growth: TileGrowth) {
        for raw in motion_steps() {
            let drawing = grid.drawing_at(test_area(), growth, raw);
            for &band in &drawing.column_bands {
                let pieces: Vec<Rect> = drawing
                    .placements()
                    .map(|placement| placement.frame.clip())
                    .filter(|rect| rect.x == band.x && rect.width == band.width)
                    .collect();
                assert!(!pieces.is_empty(), "column {band:?} has no pieces at {raw}");
                assert_eq!(
                    pieces[0].top(),
                    band.top(),
                    "first boundary at {raw}: {band:?}, {pieces:?}"
                );
                assert_eq!(
                    pieces.last().map(|piece| piece.bottom()),
                    Some(band.bottom())
                );
                for pair in pieces.windows(2) {
                    assert_eq!(
                        pair[0].bottom() - 1,
                        pair[1].top(),
                        "shared boundary at {raw}: {band:?}, {pieces:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn pieces_in_a_column_share_their_separators_through_a_transition() {
        let growth = redistribute(4);
        let opening = opening_column();
        assert_shared_piece_rows(&opening, growth);

        let mut closing = opening_column();
        closing.settle_for_test();
        closing.remove();
        assert_shared_piece_rows(&closing, growth);

        let mut growing = seeded_grid();
        growing.sync(&quiet(&[1]), growth);
        growing.settle_for_test();
        growing.sync(&busy(&[(1, usize::from(TEST_HEIGHT))]), growth);
        assert_shared_piece_rows(&growing, growth);

        let mut crossing = seeded_grid();
        let before: Vec<u32> = (1..=15).collect();
        let after: Vec<u32> = (1..=16).collect();
        crossing.sync(&quiet(&before), growth);
        crossing.settle_for_test();
        crossing.sync(&quiet(&after), growth);
        assert_shared_piece_rows(&crossing, growth);
    }

    /// Settled cell rects in screen order, with collapsed motion pieces omitted.
    fn endpoint_rects(drawing: &TileDrawing<u32>) -> Vec<Rect> {
        let mut rects = drawing
            .placements()
            .map(|placement| placement.frame.clip())
            .filter(|rect| rect.width > 1 && rect.height > 1)
            .collect::<Vec<_>>();
        rects.sort_by_key(|rect| (rect.x, rect.y, rect.width, rect.height));
        rects
    }

    /// Every settled rect at one end of a moving grid.
    fn settled_rects(grid: &Grid, count: usize) -> Vec<Rect> {
        let mut rects = (TABLE_CELL..=count)
            .filter_map(|cell| grid.cell(cell))
            .collect::<Vec<_>>();
        rects.sort_by_key(|rect| (rect.x, rect.y, rect.width, rect.height));
        rects
    }

    /// Check both exact ends after exercising every standard motion snapshot.
    fn assert_settled_motion_endpoints(label: &str, grid: &TileGrid<u32>, growth: TileGrowth) {
        assert!(
            matches!(grid.motion, GridMotion::Moving(_)),
            "{label} is moving"
        );
        let GridMotion::Moving(transition) = &grid.motion else {
            return;
        };
        let before = Grid::new(test_area(), &transition.held, growth, &grid.settings);
        let after_held = grid.drawn_held();
        let after = Grid::new(test_area(), &after_held, growth, &grid.settings);
        let drawings = motion_steps()
            .map(|raw| grid.drawing_at(test_area(), growth, raw))
            .collect::<Vec<_>>();
        assert_eq!(drawings.len(), 24);
        assert_eq!(
            endpoint_rects(&drawings[0]),
            settled_rects(&before, transition.held.rows.len()),
            "{label} starts on its before rects"
        );
        assert_eq!(
            endpoint_rects(&drawings[drawings.len() - 1]),
            settled_rects(&after, after_held.rows.len()),
            "{label} ends on its after rects"
        );
    }

    /// A public sync from `before` to `after`, paused on its first step.
    fn synced_motion(before: &[u32], after: &[u32], growth: TileGrowth) -> TileGrid<u32> {
        let mut grid = seeded_grid();
        grid.set_layout(test_area(), growth);
        grid.sync(&quiet(before), growth);
        grid.settle_for_test();
        grid.sync(&quiet(after), growth);
        grid
    }

    #[test]
    fn motion_endpoints_keep_every_present_cell_in_its_settled_rect() {
        let one_column = redistribute(4);
        let middle_departure = synced_motion(&[7, 8, 9], &[7, 9], one_column);
        assert_settled_motion_endpoints("middle departure", &middle_departure, one_column);

        let mut middle_arrival = synced_motion(&[7, 9], &[7, 9], one_column);
        middle_arrival.add(one_column);
        middle_arrival.settle_for_test();
        middle_arrival.add(one_column);
        middle_arrival.settle_for_test();
        middle_arrival.sync(&quiet(&[7, 8, 9]), one_column);
        assert_settled_motion_endpoints("middle arrival", &middle_arrival, one_column);

        let end_departure = synced_motion(&[7, 8, 9], &[7, 8], one_column);
        assert_settled_motion_endpoints("end departure", &end_departure, one_column);
        let end_arrival = synced_motion(&[7, 8], &[7, 8, 9], one_column);
        assert_settled_motion_endpoints("end arrival", &end_arrival, one_column);

        let crossing_growth = redistribute(3);
        let crossing = synced_motion(&[7, 8], &[7, 8, 9], crossing_growth);
        assert_settled_motion_endpoints("column crossing", &crossing, crossing_growth);
    }

    #[test]
    fn a_cell_claimed_during_an_opening_stays_in_its_transition_column() {
        let growth = redistribute(4);
        let mut grid = opening_column();
        let opening_from = match &grid.motion {
            GridMotion::Moving(transition) => Some(transition.from.clone()),
            GridMotion::Settled => None,
        };
        assert!(opening_from.is_some(), "the column opening is in flight");
        let opening_from = opening_from.unwrap_or_default();

        grid.sync(&quiet(&[1, 2, 3, 99]), growth);
        let still_from = match &grid.motion {
            GridMotion::Moving(transition) => Some(transition.from.clone()),
            GridMotion::Settled => None,
        };
        assert!(
            still_from.is_some(),
            "claiming the cell preserves the opening"
        );
        let still_from = still_from.unwrap_or_default();
        assert_eq!(still_from, opening_from);
        for raw in motion_steps() {
            assert!(
                grid.drawing_at(test_area(), growth, raw)
                    .placements()
                    .all(|placement| placement.content != TileContent::Group(99)),
                "the claim waits for the opening at {raw}"
            );
        }

        grid.advance();
        for raw in motion_steps() {
            let claimed = grid
                .drawing_at(test_area(), growth, raw)
                .placements()
                .filter(|placement| placement.content == TileContent::Group(99))
                .count();
            assert_eq!(claimed, 1, "one claimed piece is drawn at {raw}");
        }
    }

    /// What each cell after the summary is showing, settled.
    fn shown<Id: Clone + Eq + Debug>(grid: &TileGrid<Id>) -> Vec<TileContent<Id>> {
        cells(&grid.slots)
            .into_iter()
            .skip(1)
            .map(|(content, _)| content)
            .collect()
    }

    #[test]
    fn a_command_arriving_opens_its_own_cell() {
        let mut grid = seeded_grid();
        grid.sync(&quiet(&[7]), redistribute(4));
        grid.settle_for_test();
        assert_eq!(shown(&grid), vec![TileContent::Group(7)]);
    }

    /// A developer who pressed `+` made room deliberately, so the next
    /// build lands in it rather than opening another cell beside it.
    #[test]
    fn a_command_arriving_takes_the_first_empty_cell() {
        let mut grid = seeded_grid();
        grid.add(redistribute(4));
        grid.add(redistribute(4));
        grid.sync(&quiet(&[7]), redistribute(4));
        grid.settle_for_test();

        assert_eq!(shown(&grid).len(), 2, "no third cell opened");
        assert_eq!(shown(&grid)[0], TileContent::Group(7));
        assert_eq!(shown(&grid)[1], TileContent::Empty(3));
    }

    /// A cell is placed where there is room for it, which need not be
    /// where it belongs. Whatever order the cells were placed in, they
    /// end up standing in the order the demands name them -- otherwise
    /// a command that opened its cell early sits above one that started
    /// before it, for as long as both live.
    #[test]
    fn the_cells_stand_in_the_order_they_are_demanded_in() {
        let mut grid = seeded_grid();
        grid.sync(&quiet(&[8]), redistribute(4));
        grid.settle_for_test();
        grid.sync(&quiet(&[7, 8]), redistribute(4));
        grid.settle_for_test();

        assert_eq!(
            shown(&grid),
            vec![TileContent::Group(7), TileContent::Group(8),]
        );
    }

    /// Only the cells carrying commands are re-ordered. An empty cell
    /// is a place the reader made with `+`, and nothing shuffles
    /// through it.
    #[test]
    fn re_ordering_leaves_an_empty_cell_where_it_was_made() {
        let mut grid = seeded_grid();
        grid.sync(&quiet(&[8]), redistribute(4));
        grid.settle_for_test();
        grid.add(redistribute(4));
        grid.add(redistribute(4));
        grid.sync(&quiet(&[7, 8]), redistribute(4));
        grid.settle_for_test();

        let shown = shown(&grid);
        assert_eq!(shown[0], TileContent::Group(7));
        assert_eq!(shown[1], TileContent::Group(8));
        assert!(
            matches!(shown[2], TileContent::Empty(_)),
            "the empty cell stayed last: {shown:?}"
        );
    }

    /// The cells after a departed one each move one place forward,
    /// which is the travel the closing animates.
    #[test]
    fn a_command_leaving_the_middle_moves_the_rest_forward() {
        let mut grid = seeded_grid();
        grid.sync(&quiet(&[7, 8, 9]), redistribute(4));
        grid.settle_for_test();
        grid.sync(&quiet(&[7, 9]), redistribute(4));
        grid.settle_for_test();

        assert_eq!(
            shown(&grid),
            vec![TileContent::Group(7), TileContent::Group(9)]
        );
    }

    /// The complaint this closing answers: a hole walking to the end of
    /// the grid meant every step drew it travelling one way through the
    /// cell travelling the other, two boxes crossing in the space one
    /// was closing over. Mid-transition each survivor keeps one piece
    /// and the departing cell has the one piece that collapses.
    #[test]
    fn one_departing_piece_is_in_flight_while_the_grid_closes() {
        let mut grid = seeded_grid();
        grid.sync(&quiet(&[7, 8, 9]), redistribute(4));
        grid.settle_for_test();

        grid.sync(&quiet(&[7, 9]), redistribute(4));
        let placements = grid.placements(test_area(), redistribute(4));
        assert_eq!(
            placements.len(),
            4,
            "the summary, two survivors and one departing command"
        );
    }

    /// A command leaving the middle takes its cell with it in one
    /// step: nothing is left behind to walk to the end of the grid.
    #[test]
    fn a_command_leaving_the_middle_closes_the_grid_in_one_step() {
        let mut grid = seeded_grid();
        grid.sync(&quiet(&[7, 8, 9]), redistribute(4));
        grid.settle_for_test();

        grid.sync(&quiet(&[7, 9]), redistribute(4));
        assert_eq!(
            shown(&grid),
            vec![TileContent::Group(7), TileContent::Group(9)],
            "the cell above it and the cell below it come together"
        );
        grid.advance();
        assert!(
            matches!(grid.motion, GridMotion::Settled),
            "and there is no second step for a hole to travel through"
        );
    }

    /// The order a cell stands in is not settled once it is open. A
    /// driver's cell sorts by the earliest work still under it, so the
    /// last of a queue finishing carries the cell past whatever started
    /// in between. Reaching the new order by moving the two cells drew
    /// them travelling through each other; the overtaken one closes
    /// instead and comes back in behind the cell that passed it.
    #[test]
    fn a_cell_the_order_overtakes_closes_and_comes_back_in_behind() {
        let mut grid = seeded_grid();
        grid.sync(&quiet(&[7, 8]), redistribute(4));
        grid.settle_for_test();

        grid.sync(&quiet(&[8, 7]), redistribute(4));
        assert_eq!(
            shown(&grid),
            vec![TileContent::Group(8)],
            "the overtaken cell closes on its own"
        );
        grid.advance();
        assert_eq!(
            shown(&grid),
            vec![TileContent::Group(8), TileContent::Group(7)],
            "and opens again behind the one that passed it"
        );
        grid.advance();
        assert!(
            matches!(grid.motion, GridMotion::Settled),
            "a close and an open, and nothing else"
        );
    }

    /// A cell only ever comes back in behind the cells still standing,
    /// so a change in the middle of the order carries down everything
    /// after it: each cell behind the overtaken one closes too, and they
    /// return in the order they now read in.
    #[test]
    fn the_order_changing_cascades_down_the_rest_of_the_grid() {
        let mut grid = seeded_grid();
        grid.sync(&quiet(&[7, 8, 9]), redistribute(4));
        grid.settle_for_test();

        grid.sync(&quiet(&[8, 7, 9]), redistribute(4));
        assert_eq!(
            shown(&grid),
            vec![TileContent::Group(8), TileContent::Group(9)],
            "the overtaken cell goes first"
        );
        grid.advance();
        assert_eq!(
            shown(&grid),
            vec![TileContent::Group(8)],
            "then the cell behind it, which has nowhere left to come back in"
        );
        grid.advance();
        assert_eq!(
            shown(&grid),
            vec![TileContent::Group(8), TileContent::Group(7)],
            "the overtaken cell returns behind the one that passed it"
        );
        grid.advance();
        assert_eq!(
            shown(&grid),
            vec![
                TileContent::Group(8),
                TileContent::Group(7),
                TileContent::Group(9),
            ],
            "and the last one behind that"
        );
    }

    /// The complaint this answers: two cells exchanging places were
    /// drawn each travelling through the other, borders crossing where
    /// the eye was following one box. Every step of a reorder now adds
    /// or takes away a single cell, so whatever survives a step stands
    /// in the order it already stood in.
    #[test]
    fn no_step_of_a_reorder_draws_two_cells_crossing() {
        let mut grid = seeded_grid();
        grid.sync(&quiet(&[7, 8, 9, 10]), redistribute(4));
        grid.settle_for_test();

        grid.sync(&quiet(&[9, 7, 10, 8]), redistribute(4));
        let mut crossed = Vec::new();
        while let GridMotion::Moving(transition) = &grid.motion {
            let (from, to) = (transition.from.clone(), grid.slots.clone());
            let left: Vec<Slot<u32>> = from
                .iter()
                .filter(|slot| to.contains(slot))
                .cloned()
                .collect();
            let landed: Vec<Slot<u32>> = to
                .iter()
                .filter(|slot| from.contains(slot))
                .cloned()
                .collect();
            if left != landed {
                crossed.push((from, to));
            }
            grid.advance();
        }
        assert!(crossed.is_empty(), "cells crossed: {crossed:?}");
        assert_eq!(
            shown(&grid),
            vec![
                TileContent::Group(9),
                TileContent::Group(7),
                TileContent::Group(10),
                TileContent::Group(8),
            ],
            "and the grid still arrives at the order it was asked for"
        );
    }

    /// A reorder takes no cell off the grid: whatever closed to let the
    /// order through opens its own cell again rather than claiming the
    /// one a reader made with `+`.
    #[test]
    fn a_reorder_leaves_the_cell_count_where_it_found_it() {
        let mut grid = seeded_grid();
        grid.sync(&quiet(&[7, 8]), redistribute(4));
        grid.settle_for_test();
        grid.add(redistribute(4));
        grid.settle_for_test();

        grid.sync(&quiet(&[8, 7]), redistribute(4));
        grid.settle_for_test();
        let shown = shown(&grid);
        assert_eq!(shown.len(), 3, "the grid is the size it was: {shown:?}");
        assert_eq!(shown[0], TileContent::Group(8));
        assert_eq!(shown[1], TileContent::Group(7));
        assert!(
            matches!(shown[2], TileContent::Empty(_)),
            "and the reader's cell is still there: {shown:?}"
        );
    }

    /// The ring waits for a cell that closed to let the order through.
    /// Dropping it to the summary would make the developer find the
    /// command again over a change they never asked for.
    #[test]
    fn focus_waits_for_a_cell_that_closed_to_let_the_order_through() {
        let mut grid = seeded_grid();
        grid.sync(&quiet(&[7, 8]), redistribute(4));
        grid.settle_for_test();
        grid.focus_step(Direction::Down, redistribute(4));
        grid.settle_for_test();
        assert_eq!(grid.focus, Focus::Cell(Slot::Group(7)));

        grid.sync(&quiet(&[8, 7]), redistribute(4));
        assert_eq!(
            grid.focus,
            Focus::Cell(Slot::Group(7)),
            "the ring stays with the command while its cell is off the grid"
        );
        assert_eq!(
            grid.focused_cell(),
            FocusLocation::Departing,
            "and no cell is drawn holding it meanwhile"
        );

        grid.settle_for_test();
        assert_eq!(
            grid.focused_cell(),
            FocusLocation::Cell(TABLE_CELL + 2),
            "it comes back with the cell, one place further down"
        );
    }

    /// Two commands ending in the same scan do not go together. The
    /// grid closes the first cell, plays that travel out, and only then
    /// closes the next -- taking them in the order they stand in, which
    /// is the order they arrived in. Two cells closing over each other
    /// at once is a pile of lines rather than a grid coming together.
    #[test]
    fn commands_ending_together_close_one_cell_at_a_time() {
        let mut grid = seeded_grid();
        grid.sync(&quiet(&[7, 8, 9]), redistribute(4));
        grid.settle_for_test();

        grid.sync(&quiet(&[9]), redistribute(4));
        assert_eq!(
            shown(&grid),
            vec![TileContent::Group(8), TileContent::Group(9)],
            "the oldest of them goes first, on its own"
        );
        assert!(
            matches!(grid.motion, GridMotion::Moving(_)),
            "and its travel is in flight"
        );

        grid.advance();
        assert_eq!(
            shown(&grid),
            vec![TileContent::Group(9)],
            "the next one follows once that travel is done"
        );
    }

    /// Every piece leaves with a closing column at its starting rows,
    /// including when a wide summary has shortened the column.
    #[test]
    fn no_piece_in_a_closing_column_changes_height() {
        let area = test_area();
        let before = [1, 2, 3, 4, 5, 6];
        let after = [1, 2, 3, 4, 6];
        let ordinary_growth = redistribute(3);
        let ordinary = synced_motion(&before, &after, ordinary_growth);

        let covered_growth = widening(3);
        let mut covered = seeded_grid();
        covered.set_layout(area, covered_growth);
        covered.sync(&wide_summary(&before, u16::MAX), covered_growth);
        covered.settle_for_test();
        covered.sync(&wide_summary(&after, u16::MAX), covered_growth);

        for (label, grid, growth, shortened) in [
            ("full-height", ordinary, ordinary_growth, false),
            ("under the summary", covered, covered_growth, true),
        ] {
            let start = grid.drawing_at(area, growth, 0);
            assert_eq!(start.column_bands.len(), 3, "{label} column count");
            let column = start.column_bands.len() - 1;
            let start_band = start.column_bands[column];
            assert_eq!(
                start_band.top() > area.top(),
                shortened,
                "{label} starting top"
            );
            let starting_rows = start
                .placements()
                .filter(|placement| {
                    let clip = placement.frame.clip();
                    clip.x == start_band.x && clip.width == start_band.width
                })
                .map(|placement| {
                    let rect = placement.frame.rect();
                    let clip = placement.frame.clip();
                    (
                        placement.content.clone(),
                        rect.y,
                        rect.height,
                        clip.y,
                        clip.height,
                        placement.frame.shift(),
                    )
                })
                .collect::<Vec<_>>();
            assert!(starting_rows.len() > 1, "{label} has stacked pieces");

            for raw in motion_steps() {
                let drawing = grid.drawing_at(area, growth, raw);
                let band = drawing.column_bands[column];
                let rows = drawing
                    .placements()
                    .filter(|placement| {
                        let clip = placement.frame.clip();
                        clip.x == band.x && clip.width == band.width
                    })
                    .map(|placement| {
                        let rect = placement.frame.rect();
                        let clip = placement.frame.clip();
                        (
                            placement.content.clone(),
                            rect.y,
                            rect.height,
                            clip.y,
                            clip.height,
                            placement.frame.shift(),
                        )
                    })
                    .collect::<Vec<_>>();
                assert_eq!(rows, starting_rows, "{label} at raw {raw}");
                assert_eq!(
                    (band.top(), band.bottom()),
                    (start_band.top(), start_band.bottom()),
                    "{label} band at raw {raw}"
                );
            }
        }
    }

    /// The same cell in a column the grid keeps does slide off its
    /// edge, which is the snake every other close is drawn as.
    #[test]
    fn a_cell_leaving_a_column_that_stays_slides_off_its_edge() {
        let area = test_area();
        let growth = redistribute(4);
        let before = (1..=15).collect::<Vec<_>>();
        let after = (1..=16).collect::<Vec<_>>();
        let grid = synced_motion(&before, &after, growth);
        let drawing = grid.drawing_at(area, growth, PROGRESS_SCALE / 2);
        let leaving = drawing
            .placements()
            .filter(|placement| placement.content == TileContent::Group(4))
            .find(|placement| placement.frame.shift() < 0)
            .expect("the piece leaving travels upward");

        assert_eq!(
            i32::from(leaving.frame.rect().bottom()) + leaving.frame.shift(),
            i32::from(leaving.frame.clip().bottom()),
            "its bottom border stays on the divider it leaves through"
        );
    }

    #[test]
    fn a_middle_departure_starts_on_its_before_rows_between_its_neighbours() {
        let mut grid = seeded_grid();
        grid.sync(&quiet(&[7, 8, 9]), redistribute(4));
        grid.settle_for_test();
        grid.sync(&quiet(&[7, 9]), redistribute(4));

        let drawing = grid.drawing_at(test_area(), redistribute(4), 0);
        let placement = |content| {
            drawing
                .placements()
                .find(|placement| placement.content == content)
                .expect("the transition still draws this command")
        };
        let above = placement(TileContent::Group(7));
        let survivor = placement(TileContent::Group(9));
        let departing = placement(TileContent::Group(8));
        let band = drawing
            .column_bands
            .first()
            .copied()
            .expect("the transition has a column");

        assert_eq!(
            departing.frame.clip().top(),
            above.frame.clip().bottom() - 1,
            "the departure shares its upper separator"
        );
        assert_eq!(
            departing.frame.clip().bottom() - 1,
            survivor.frame.clip().top(),
            "the departure shares its lower separator"
        );
        assert_eq!(
            survivor.frame.clip().bottom(),
            band.bottom(),
            "the last survivor reaches the column floor"
        );
        assert_eq!(
            departing.frame.clip(),
            Grid::new(
                test_area(),
                &even(4),
                redistribute(4),
                &TileSettings::default(),
            )
            .cell(3)
            .expect("the before grid has the departing cell"),
            "the departing piece begins on its settled before rect"
        );
    }

    #[test]
    fn removing_refuses_to_close_a_cell_holding_a_command() {
        let mut grid = seeded_grid();
        grid.sync(&quiet(&[7]), redistribute(4));
        grid.settle_for_test();
        grid.remove();
        grid.settle_for_test();
        assert_eq!(shown(&grid), vec![TileContent::Group(7)]);
    }

    #[test]
    fn removing_closes_the_last_cell_that_plus_opened() {
        let mut grid = seeded_grid();
        grid.sync(&quiet(&[7]), redistribute(4));
        grid.add(redistribute(4));
        grid.remove();
        grid.settle_for_test();
        assert_eq!(shown(&grid), vec![TileContent::Group(7)]);
    }

    #[test]
    fn the_summary_holds_focus_to_begin_with() {
        assert_eq!(TileGrid::<u32>::new().focus, Focus::Summary);
    }

    #[test]
    fn focus_walks_the_grid_and_stops_at_its_edges() {
        let mut grid = seeded_grid();
        grid.sync(&quiet(&[7, 8]), redistribute(4));
        grid.settle_for_test();

        grid.focus_step(Direction::Down, redistribute(4));
        assert_eq!(grid.focused_cell(), FocusLocation::Cell(2));
        grid.focus_step(Direction::Down, redistribute(4));
        assert_eq!(grid.focused_cell(), FocusLocation::Cell(3));
        grid.focus_step(Direction::Down, redistribute(4));
        assert_eq!(
            grid.focused_cell(),
            FocusLocation::Cell(3),
            "the last cell is the floor"
        );
        grid.focus_step(Direction::Up, redistribute(4));
        grid.focus_step(Direction::Up, redistribute(4));
        assert_eq!(grid.focus, Focus::Summary, "and the summary is the ceiling");
    }

    /// Tab reads the grid as the list it is numbered as, and wraps
    /// rather than stopping where the arrows stop.
    #[test]
    fn tab_walks_every_cell_and_comes_back_round() {
        let mut grid = seeded_grid();
        grid.sync(&quiet(&[7, 8]), redistribute(4));
        grid.settle_for_test();

        assert!(grid.cycle_focus(CycleDirection::Next));
        assert_eq!(grid.focused_cell(), FocusLocation::Cell(2));
        assert!(grid.cycle_focus(CycleDirection::Next));
        assert_eq!(grid.focused_cell(), FocusLocation::Cell(3));
        assert!(grid.cycle_focus(CycleDirection::Next));
        assert_eq!(
            grid.focus,
            Focus::Summary,
            "the last cell wraps to the summary"
        );
        assert!(grid.cycle_focus(CycleDirection::Prev));
        assert_eq!(
            grid.focused_cell(),
            FocusLocation::Cell(3),
            "and back the other way"
        );
    }

    /// Focus is held by identity, so the cell it is on keeps it while
    /// the grid closes up around it.
    #[test]
    fn focus_follows_a_cell_through_a_closing() {
        let mut grid = seeded_grid();
        grid.sync(&quiet(&[7, 8, 9]), redistribute(4));
        grid.settle_for_test();
        grid.focus_step(Direction::Down, redistribute(4));
        grid.focus_step(Direction::Down, redistribute(4));
        grid.focus_step(Direction::Down, redistribute(4));
        assert_eq!(grid.focus, Focus::Cell(Slot::Group(9)));

        grid.sync(&quiet(&[7, 9]), redistribute(4));
        grid.settle_for_test();
        assert_eq!(grid.focus, Focus::Cell(Slot::Group(9)));
        assert_eq!(
            grid.focused_cell(),
            FocusLocation::Cell(3),
            "one place forward"
        );
    }

    #[test]
    fn focus_falls_back_to_the_summary_when_its_command_ends() {
        let mut grid = seeded_grid();
        grid.sync(&quiet(&[7]), redistribute(4));
        grid.settle_for_test();
        grid.focus_step(Direction::Down, redistribute(4));
        assert_eq!(grid.focus, Focus::Cell(Slot::Group(7)));

        grid.sync(&quiet(&[]), redistribute(4));
        grid.settle_for_test();
        assert_eq!(grid.focus, Focus::Summary);
    }

    /// The developer opened the cell to watch for exactly this, so the
    /// ring stays on it when a command lands there.
    #[test]
    fn focus_stays_on_an_empty_cell_a_command_claims() {
        let mut grid = seeded_grid();
        grid.add(redistribute(4));
        grid.settle_for_test();
        grid.focus_step(Direction::Down, redistribute(4));
        assert!(matches!(grid.focus, Focus::Cell(Slot::Empty(_))));

        grid.sync(&quiet(&[7]), redistribute(4));
        grid.settle_for_test();
        assert_eq!(grid.focus, Focus::Cell(Slot::Group(7)));
    }

    /// A click is the other way onto a cell, and lands on the same ring
    /// the arrows walk.
    #[test]
    fn a_click_inside_a_cell_takes_the_focus_ring() {
        let mut grid = seeded_grid();
        grid.sync(&quiet(&[7, 8]), redistribute(4));
        grid.settle_for_test();

        let second = Grid::new(
            test_area(),
            &even(grid.count()),
            redistribute(4),
            &TileSettings::default(),
        )
        .cell(2)
        .expect("the grid has three cells");
        let inside = Position::new(second.x + second.width / 2, second.y + second.height / 2);
        let index = grid.cell_at(inside).expect("the point is inside cell two");
        grid.focus_cell(index);

        assert_eq!(grid.focus, Focus::Cell(Slot::Group(7)));
    }

    #[test]
    fn a_click_outside_the_grid_finds_no_cell() {
        let grid = seeded_grid::<u32>();
        assert_eq!(grid.cell_at(Position::new(TEST_WIDTH, TEST_HEIGHT)), None);
    }

    /// `-` takes out the cell the ring is on when `+` opened it, rather
    /// than the last one opened.
    #[test]
    fn minus_takes_out_the_focused_empty_cell() {
        let mut grid = seeded_grid::<u32>();
        grid.add(redistribute(4));
        grid.add(redistribute(4));
        grid.settle_for_test();
        let first = grid.slots[0].clone();
        grid.focus_step(Direction::Down, redistribute(4));
        assert_eq!(grid.focus, Focus::Cell(first.clone()));

        grid.remove();
        grid.settle_for_test();
        assert_eq!(grid.slots.len(), 1, "one cell went");
        assert_ne!(grid.slots[0], first, "and it was the focused one");
        assert_eq!(grid.focus, Focus::Summary, "the ring falls back");
    }

    /// A cell carrying a command is the display itself, so `-` leaves it
    /// where it is and takes the last empty cell instead.
    #[test]
    fn minus_leaves_a_running_command_alone() {
        let mut grid = seeded_grid();
        grid.sync(&quiet(&[7]), redistribute(4));
        grid.add(redistribute(4));
        grid.settle_for_test();
        grid.focus_step(Direction::Down, redistribute(4));
        assert_eq!(grid.focus, Focus::Cell(Slot::Group(7)));

        grid.remove();
        grid.settle_for_test();
        assert_eq!(
            shown(&grid),
            vec![TileContent::Group(7)],
            "the command keeps its cell and the empty one goes"
        );
    }

    #[test]
    fn minus_with_no_empty_cell_changes_nothing() {
        let mut grid = seeded_grid();
        grid.sync(&quiet(&[7]), redistribute(4));
        grid.settle_for_test();

        grid.remove();
        grid.settle_for_test();
        assert_eq!(shown(&grid), vec![TileContent::Group(7)]);
    }

    #[test]
    fn easing_pins_both_ends() {
        assert_eq!(eased(0), 0);
        assert_eq!(eased(PROGRESS_SCALE), PROGRESS_SCALE);
        assert!(eased(PROGRESS_SCALE / 2).abs_diff(PROGRESS_SCALE / 2) <= 1);
    }

    mod summary_depth {
        use super::*;

        fn held_at_depth(rows: Vec<u16>, summary_depth: usize) -> HeldCellLayout {
            HeldCellLayout {
                rows,
                focused: FocusLocation::Cell(TABLE_CELL),
                summary_span: 1,
                summary_depth,
            }
        }

        fn deep_grid() -> TileGrid<u32> {
            let mut grid = seeded_grid();
            let mut demands = busy(&[(7, 7), (8, 7), (9, 7)]);
            demands.summary = 12;
            grid.sync(&demands, redistribute(4));
            grid.settle_for_test();
            grid
        }

        #[test]
        fn the_summary_counts_as_the_fewest_cells_that_hold_it() {
            let settings = TileSettings::default();
            for (asked, depth, rows) in [
                (9, 1, 10),
                (14, 2, 27),
                (30, 2, 30),
                (35, 3, 40),
                (45, 3, 40),
            ] {
                let wants = [asked, 10, 10, 10];
                assert_eq!(
                    summary_depth(test_area(), &wants, redistribute(4), &settings),
                    depth
                );
                assert_eq!(
                    summary_share(&wants, TEST_HEIGHT, depth, redistribute(4), MIN_TILE_HEIGHT),
                    rows
                );
            }

            let idle = [30, 3, 3, 3];
            assert_eq!(
                summary_depth(test_area(), &idle, redistribute(4), &settings),
                1,
                "ordinary rebalancing lets the idle commands lend the summary their spare rows"
            );
        }

        #[test]
        fn the_summary_gives_a_cell_back_once_it_fits_in_fewer() {
            let mut grid = seeded_grid();
            let mut demands = busy(&[(7, 7), (8, 7), (9, 7)]);
            demands.summary = 33;
            grid.sync(&demands, redistribute(4));
            grid.settle_for_test();
            assert_eq!(grid.depth, 3);
            assert_eq!(grid.held.summary_depth, 3);

            demands.summary = 6;
            grid.sync(&demands, redistribute(4));
            grid.settle_for_test();
            assert_eq!(grid.depth, 1);
            assert_eq!(grid.held.summary_depth, 1);
        }

        #[test]
        fn a_deeper_summary_pushes_every_cell_one_place_on() {
            let settings = TileSettings::default();
            let growth = redistribute(4);
            for (depth, widths) in [(1, vec![4]), (2, vec![3, 2]), (3, vec![3, 3])] {
                let held = held_at_depth(vec![10; 4], depth);
                let grid = Grid::new(test_area(), &held, growth, &settings);
                assert_eq!(grid.widths, widths);
                assert_eq!(grid.column_of(1), Some(0));
                for cell in 2..=held.rows.len() {
                    assert_eq!(
                        position(&grid.widths, cell + depth - 1).map(|(column, _)| column),
                        grid.column_of(cell),
                        "logical cell {cell} follows its position at depth {depth}"
                    );
                    assert!(grid.cell(cell).is_some());
                }
            }
        }

        #[test]
        fn a_deeper_summary_tiles_its_column() {
            let held = held_at_depth(vec![30, 10, 10, 10], 2);
            let grid = Grid::new(
                test_area(),
                &held,
                redistribute(4),
                &TileSettings::default(),
            );
            let mut covered = blank_tally();
            let mut interiors = blank_tally();
            for cell in 1..=held.rows.len() {
                let rect = grid.cell(cell).expect("logical cell is in the grid");
                paint(&mut covered, rect);
                paint(&mut interiors, rect.inner(Margin::new(1, 1)));
            }
            assert!(
                covered.iter().all(|&hits| hits >= 1),
                "the grid leaves no gap"
            );
            assert!(
                interiors.iter().all(|&hits| hits <= 1),
                "interiors never overlap"
            );
            let summary = grid.cell(1).expect("summary is in the grid");
            let below = grid.cell(2).expect("cell two is below it");
            assert_eq!(summary.bottom() - 1, below.top());
        }

        #[test]
        fn a_capped_summary_places_every_cell_without_an_empty_column() {
            let held = held_at_depth(vec![30, 10, 10, 10], 7);
            let grid = Grid::new(
                test_area(),
                &held,
                redistribute(4),
                &TileSettings::default(),
            );
            assert_eq!(grid.depth, 3);
            assert_eq!(grid.widths, vec![3, 3]);
            assert_eq!(
                grid.widths.iter().sum::<usize>(),
                held.rows.len() + grid.depth - 1
            );
            for column in 0..grid.widths.len() {
                assert!(
                    (1..=held.rows.len()).any(|cell| grid.column_of(cell) == Some(column)),
                    "column {column} has a cell"
                );
            }

            let mut covered = blank_tally();
            let mut interiors = blank_tally();
            for cell in 1..=held.rows.len() {
                let rect = grid.cell(cell).expect("every logical cell is placed");
                paint(&mut covered, rect);
                paint(&mut interiors, rect.inner(Margin::new(1, 1)));
            }
            assert!(
                covered.iter().all(|&hits| hits >= 1),
                "no area is left empty"
            );
            assert!(
                interiors.iter().all(|&hits| hits <= 1),
                "interiors do not overlap"
            );
        }

        #[test]
        fn widths_follow_capped_summary_positions_before_the_next_sync() {
            let mut grid = seeded_grid();
            let mut demands = busy(&[(7, 7)]);
            demands.summary = 200;
            grid.sync(&demands, redistribute(8));
            grid.settle_for_test();
            assert_eq!(grid.held.summary_depth, 7);

            let growth = redistribute(4);
            let layout = Grid::new(test_area(), &grid.drawn_held(), growth, &grid.settings);
            assert!(layout.depth < grid.held.summary_depth);
            let widths = grid.content_widths(test_area(), growth);
            assert_eq!(widths.len(), grid.count());
            for ((_, width), cell) in widths.into_iter().zip(1..=grid.count()) {
                let rect = layout
                    .cell(cell)
                    .expect("cell is placed under the new growth");
                assert_eq!(
                    width,
                    frame_inner(rect).width,
                    "cell {cell} is measured where it draws"
                );
            }
        }

        #[test]
        fn the_summary_never_takes_the_grid_past_fits() {
            let area = Rect::new(0, 0, TEST_WIDTH, 5);
            let growth = redistribute(4);
            let settings = TileSettings::default();
            let wants = [u16::MAX, MIN_TILE_HEIGHT];
            assert!(fits(area, 2, growth, &settings));
            assert!(!fits(area, 3, growth, &settings));
            assert_eq!(summary_depth(area, &wants, growth, &settings), 1);
            assert!(summary_share(&wants, area.height, 1, growth, MIN_TILE_HEIGHT) < wants[0]);
        }

        #[test]
        fn the_summary_is_served_before_the_focus_ring() {
            let wants = [26, 35, 0];
            let outside = shares(
                &wants,
                TEST_HEIGHT,
                ColumnHead::Summary(2),
                ColumnFocus::Outside,
                MIN_TILE_HEIGHT,
            );
            let focused = shares(
                &wants,
                TEST_HEIGHT,
                ColumnHead::Summary(2),
                ColumnFocus::Row(1),
                MIN_TILE_HEIGHT,
            );
            assert_eq!(outside[0], PaneAxisSize::Fixed(26));
            assert_eq!(
                focused[0], outside[0],
                "focus does not take the summary's spare rows"
            );
        }

        #[test]
        fn moving_focus_never_changes_the_summary_depth() {
            let mut grid = deep_grid();
            assert_eq!(grid.depth, 2);
            for cell in [2, 3, 4, 1] {
                grid.focus_cell(cell);
                grid.settle_for_test();
                assert_eq!(grid.depth, 2, "focus on cell {cell} keeps the depth");
                assert_eq!(grid.held.summary_depth, 2);
            }
        }

        #[test]
        fn the_arrows_move_by_position_beside_a_deep_summary() {
            let mut grid = deep_grid();
            grid.focus_step(Direction::Down, redistribute(4));
            assert_eq!(grid.focused_cell(), FocusLocation::Cell(2));
            grid.focus_step(Direction::Up, redistribute(4));
            assert_eq!(grid.focused_cell(), FocusLocation::Cell(1));
            grid.focus_cell(4);
            grid.focus_step(Direction::Left, redistribute(4));
            assert_eq!(grid.focused_cell(), FocusLocation::Cell(1));
            grid.focus_cell(3);
            grid.focus_step(Direction::Left, redistribute(4));
            assert_eq!(grid.focused_cell(), FocusLocation::Cell(1));
        }

        #[test]
        fn a_click_on_any_part_of_a_deep_summary_focuses_it() {
            let mut grid = deep_grid();
            let rect = Grid::new(test_area(), &grid.held, redistribute(4), &grid.settings)
                .cell(1)
                .expect("summary is in the grid");
            let mut shallow = grid.held.clone();
            shallow.summary_depth = 1;
            let second = Grid::new(test_area(), &shallow, redistribute(4), &grid.settings)
                .cell(2)
                .expect("cell two follows a one-position summary");
            let lower = second.top() + 1;
            assert!(lower < second.bottom());
            for y in [rect.top() + 1, lower] {
                grid.focus_cell(2);
                let point = Position::new(rect.left() + 1, y);
                let clicked = grid
                    .cell_at(point)
                    .expect("both parts of the summary are clickable");
                assert_eq!(clicked, 1);
                grid.focus_cell(clicked);
                assert_eq!(grid.focus, Focus::Summary);
            }
        }

        #[test]
        fn adding_under_new_growth_queues_its_summary_depth() {
            let mut grid = seeded_grid();
            let mut demands = busy(&[(7, 7)]);
            demands.summary = 200;
            grid.sync(&demands, redistribute(8));
            grid.settle_for_test();
            assert_eq!(grid.depth, 7);

            let growth = redistribute(4);
            grid.apply(TileAction::Add, growth);
            let expected = summary_depth(
                test_area(),
                &cell_wants(&grid.demands, &grid.target(), &grid.settings),
                growth,
                &grid.settings,
            );
            assert!(expected < grid.depth);
            assert_eq!(grid.target_depth(), expected);
        }

        #[test]
        fn cell_numbers_hold_while_the_summary_deepens() {
            let mut grid = seeded_grid();
            let mut demands = busy(&[(7, 7), (8, 7)]);
            grid.sync(&demands, redistribute(4));
            grid.settle_for_test();
            grid.add(redistribute(4));
            grid.settle_for_test();
            assert_eq!(grid.depth, 1);
            let before = shown(&grid);
            assert_eq!(before[2], TileContent::Empty(4));
            demands.summary = 33;
            grid.sync(&demands, redistribute(4));
            grid.settle_for_test();
            assert_eq!(grid.depth, 3);
            assert_eq!(shown(&grid), before);
            assert_eq!(
                cells(&grid.slots)
                    .into_iter()
                    .map(|(_, cell)| cell)
                    .collect::<Vec<_>>(),
                vec![1, 2, 3, 4]
            );
            grid.focus_cell(2);
            assert_eq!(grid.focus, Focus::Cell(Slot::Group(7)));
        }

        #[test]
        fn deepening_travels_the_way_a_closing_cell_does_in_reverse() {
            let area = test_area();
            let settings = TileSettings::default();
            let before = Grid::new(
                area,
                &held_at_depth(vec![10; 4], 1),
                redistribute(4),
                &settings,
            );
            let after = Grid::new(
                area,
                &held_at_depth(vec![30, 10, 10, 10], 2),
                redistribute(4),
                &settings,
            );
            for progress in [
                0,
                PROGRESS_SCALE / 4,
                PROGRESS_SCALE / 2,
                PROGRESS_SCALE * 3 / 4,
                PROGRESS_SCALE,
            ] {
                let bands: Vec<Rect> = (0..before.columns.len().max(after.columns.len()))
                    .map(|column| column_band(&before, &after, column, progress))
                    .collect();
                let mut pieces = Vec::new();
                moving_cell(
                    &before,
                    &after,
                    CellTransition::PresentAtBothEnds {
                        before_cell: 1,
                        after_cell:  1,
                    },
                    progress,
                    drawn(),
                    &mut pieces,
                );
                moving_cell(
                    &before,
                    &after,
                    CellTransition::PresentAtBothEnds {
                        before_cell: 2,
                        after_cell:  2,
                    },
                    progress,
                    CellAppearance {
                        content: TileContent::Empty(2),
                        focused: false,
                    },
                    &mut pieces,
                );
                let placed = place_band_pieces(&before, &after, &bands, pieces);
                let summary = &placed[0].placement;
                let second = &placed[1].placement;
                assert_eq!(summary.frame.rect().bottom() - 1, second.frame.rect().top());
            }
        }

        #[test]
        fn a_steady_summary_queues_nothing() {
            for (growth, span) in [(redistribute(4), 1), (widening(4), 2)] {
                let mut grid = seeded_grid();
                let mut demands = busy(&[(7, 7), (8, 7), (9, 7)]);
                demands.summary = 12;
                demands.summary_width = u16::MAX;
                grid.sync(&demands, growth);
                grid.settle_for_test();
                assert_eq!(grid.depth, 2);
                assert_eq!(grid.held.summary_span, span);
                assert!(grid.pending.is_empty());

                grid.sync(&demands, growth);
                assert!(grid.pending.is_empty(), "{growth:?}");
                assert!(matches!(grid.motion, GridMotion::Settled));
            }
        }

        #[test]
        fn a_deeper_summary_widens_over_the_columns_its_depth_opens() {
            let growth = widening(4);
            let mut grid = seeded_grid();
            let mut demands = busy(&[(7, 7), (8, 7), (9, 7)]);
            demands.summary = 12;
            demands.summary_width = u16::MAX;
            grid.sync(&demands, growth);
            grid.settle_for_test();

            let layout = Grid::new(test_area(), &grid.held, growth, &grid.settings);
            assert_eq!(grid.depth, 2);
            assert_eq!(layout.widths, vec![3, 2]);
            assert_eq!(grid.held.summary_span, 2);
            assert_eq!(layout.span, 2);

            demands.summary = 33;
            grid.sync(&demands, growth);
            grid.settle_for_test();
            let layout = Grid::new(test_area(), &grid.held, growth, &grid.settings);
            assert_eq!(grid.depth, 3);
            assert_eq!(layout.widths, vec![3, 3]);
            assert_eq!(grid.held.summary_span, 2);
            assert_eq!(layout.span, 1, "the next column needs room for its cells");

            let mut capped = seeded_grid();
            let mut demands = busy(&[(7, 7)]);
            demands.summary = 200;
            demands.summary_width = u16::MAX;
            capped.sync(&demands, widening(8));
            capped.settle_for_test();
            assert_eq!(capped.depth, 7);
            capped.set_layout(test_area(), growth);
            let resolved = column_layout(capped.count(), capped.depth, growth);
            assert_eq!(resolved.widths.len(), 1);
            assert!(resolved.summary_depth < capped.depth);
            assert_eq!(capped.wanted_span(), resolved.widths.len());
        }
    }
}
