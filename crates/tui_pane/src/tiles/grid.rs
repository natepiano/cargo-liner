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

use std::cell::OnceCell;
use std::cell::RefCell;
use std::cmp::Ordering;
use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::collections::HashMap;
use std::collections::VecDeque;
use std::fmt::Debug;
use std::num::NonZeroU16;
use std::sync::OnceLock;
#[cfg(test)]
use std::sync::atomic::AtomicUsize;
#[cfg(test)]
use std::sync::atomic::Ordering as AtomicOrdering;
use std::time::Instant;

use ratatui::layout::Layout;
use ratatui::layout::Position;
use ratatui::layout::Rect;

use super::action::TileAction;
use super::constants::MIN_INITIAL_ROWS;
use super::constants::MIN_TILE_HEIGHT;
use super::constants::MOVING_HEIGHT_PROBE_STEP;
use super::constants::PROGRESS_SCALE;
use super::constants::SUMMARY_SLOT_SHORTFALL_DIVISOR;
use super::constants::TABLE_CELL;
use super::draw;
use super::growth::TileGrowth;
use super::settings::CellRowDemand;
use super::settings::SummaryRowAllocation;
use super::settings::TileSettings;
use super::settings::TileWidthFloor;
use crate::CycleDirection;
use crate::PaneAxisSize;
use crate::PaneFrame;
use crate::ResolvedPane;
use crate::ResolvedPaneLayout;
use crate::TileFill;
use crate::TileView;
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
    /// Stable cell identity carried through movement and removal.
    identity:               CellIdentity<Id>,
    /// What and where the piece draws.
    pub(super) placement:   TilePlacement<Id>,
    /// Whether this piece draws the cell's name.
    pub(super) name:        PieceName,
    /// How this piece participates in motion-height probing.
    pub(super) height_snap: PieceHeightSnap,
}

#[cfg(test)]
impl<Id: Clone> TilePiece<Id> {
    /// A hand-built paint fixture, with the same identity as its content.
    pub(super) fn for_test(content: TileContent<Id>, frame: PaneFrame, name: PieceName) -> Self {
        let identity = match &content {
            TileContent::Summary => CellIdentity::Summary,
            TileContent::Group(id) => CellIdentity::Slot(Slot::Group(id.clone())),
            TileContent::Empty(index) => {
                CellIdentity::Slot(Slot::Empty(u64::try_from(*index).unwrap_or(u64::MAX)))
            },
        };
        Self {
            identity,
            placement: TilePlacement { content, frame },
            name,
            height_snap: PieceHeightSnap::Settled,
        }
    }
}

/// How one visible piece participates in this frame's height search.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum PieceHeightSnap {
    /// A settled piece has no moving height.
    Settled,
    /// Search between the last drawn height and the destination height.
    Search {
        /// Framed height at the start of the motion.
        start: u16,
        /// Framed height at the end of the motion.
        end:   u16,
    },
    /// A piece leaving with its column keeps its starting height.
    KeepStartingHeight,
}

/// One frame's cell pieces and the fixed column frames they move inside.
pub(super) struct TileDrawing<Id> {
    /// Every visible piece, in cell order.
    pub(super) pieces:       Vec<TilePiece<Id>>,
    /// Each column's band at the same point in the transition.
    pub(super) column_bands: Vec<Rect>,
}

/// One answer from a cell at a content-area size considered this frame.
#[derive(Clone, Debug, Eq, PartialEq)]
struct FrameContentRowUsage {
    /// Width of the content area inspected.
    width:    u16,
    /// Height of the content area inspected.
    height:   u16,
    /// Rows the content draws there.
    drawn:    u16,
    /// Whether the allocation is filled or contains the whole value.
    fit:      FrameHeightFit,
    /// Rows retained before a column mate may use the rest.
    retained: OnceLock<u16>,
}

impl FrameContentRowUsage {
    fn retained_rows(&self) -> RetainedRows {
        self.retained
            .get()
            .map_or(RetainedRows::Unmeasured, |rows| {
                RetainedRows::Measured(*rows)
            })
    }
}

/// Whether an allocation has a filled span within its retained height bounds.
enum FilledSpan {
    /// The content fills this span or is whole there.
    Fits(u16),
    /// No filled span fits the permitted interval.
    NoFit,
}

/// Largest filled span no greater than the requested one.
fn measured_filled_span<Id: Clone + Eq>(
    row_usage: &mut Vec<FrameCellRowUsage<Id>>,
    content: &TileContent<Id>,
    rect: Rect,
    mut span: u16,
    minimum: u16,
    rows_drawn: &mut impl FnMut(&TileContent<Id>, Rect) -> u16,
    rows_kept: &mut impl FnMut(&TileContent<Id>, Rect) -> u16,
) -> FilledSpan {
    while span >= minimum {
        let area = draw::content_area(frame_inner(Rect {
            height: span.saturating_add(1),
            ..rect
        }));
        measure_frame_row_use(row_usage, content, area, rows_drawn, rows_kept);
        if frame_height_fit(row_usage, content, area) != FrameHeightFit::LeavesGap {
            return FilledSpan::Fits(span);
        }
        let next = frame_painted_rows(row_usage, content, area)
            .saturating_add(span.saturating_sub(area.height));
        if next >= span {
            break;
        }
        span = next;
    }
    FilledSpan::NoFit
}

/// Whether this allocation has supplied its retained-row answer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RetainedRows {
    /// A motion-only probe has not needed the answer.
    Unmeasured,
    /// The settled layout gathered the answer with its painted rows.
    Measured(u16),
}

/// What the current frame's content does with one candidate height.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FrameHeightFit {
    /// Content reaches the allocation's last row.
    Filled,
    /// Content ends sooner because the whole value is visible.
    Whole,
    /// Content leaves a gap while later rows remain hidden.
    LeavesGap,
}

/// Row-usage probes for one cell during the current frame.
#[derive(Clone, Debug, Eq, PartialEq)]
struct FrameCellRowUsage<Id> {
    /// The content whose allocations were inspected.
    content: TileContent<Id>,
    /// Answers at only the sizes this frame considered.
    rows:    FrameCellRowAnswers,
}

/// Frame answers indexed by width and height, retaining only requested sizes.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct FrameCellRowAnswers {
    first_width:  FrameWidthRowAnswers,
    other_widths: HashMap<u16, FrameHeightRowAnswers>,
    answers:      Vec<FrameContentRowUsage>,
}

/// The first measured width avoids a map lookup on the common frame path.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
enum FrameWidthRowAnswers {
    #[default]
    Unmeasured,
    Measured {
        width: u16,
        rows:  FrameHeightRowAnswers,
    },
}

/// Index of a fully measured allocation in the frame's answer sequence.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum FrameRowIndex {
    #[default]
    Unmeasured,
    Measured(usize),
}

/// Finite heights are indexed directly; the whole-content probe stands apart.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct FrameHeightRowAnswers {
    finite: Vec<FrameRowIndex>,
    whole:  FrameRowIndex,
}

impl FrameHeightRowAnswers {
    fn at(&self, height: u16) -> FrameRowIndex {
        if height == u16::MAX {
            self.whole
        } else {
            self.finite
                .get(usize::from(height))
                .copied()
                .unwrap_or_default()
        }
    }

    fn record(&mut self, height: u16, index: usize) {
        if height == u16::MAX {
            self.whole = FrameRowIndex::Measured(index);
        } else {
            let height = usize::from(height);
            if self.finite.len() <= height {
                self.finite.resize(height + 1, FrameRowIndex::Unmeasured);
            }
            self.finite[height] = FrameRowIndex::Measured(index);
        }
    }
}

/// Whether this frame has measured a content allocation.
enum FrameRowAnswer<'a> {
    Unmeasured,
    Measured(&'a FrameContentRowUsage),
}

impl FrameCellRowAnswers {
    fn index(&self, area: Rect) -> FrameRowIndex {
        match &self.first_width {
            FrameWidthRowAnswers::Measured { width, rows } if *width == area.width => {
                rows.at(area.height)
            },
            _ => self
                .other_widths
                .get(&area.width)
                .map_or(FrameRowIndex::Unmeasured, |rows| rows.at(area.height)),
        }
    }

    fn answer(&self, area: Rect) -> FrameRowAnswer<'_> {
        match self.index(area) {
            FrameRowIndex::Unmeasured => FrameRowAnswer::Unmeasured,
            FrameRowIndex::Measured(index) => FrameRowAnswer::Measured(&self.answers[index]),
        }
    }

    fn record(&mut self, answer: FrameContentRowUsage) -> usize {
        let index = self.answers.len();
        if self.first_width == FrameWidthRowAnswers::Unmeasured {
            self.first_width = FrameWidthRowAnswers::Measured {
                width: answer.width,
                rows:  FrameHeightRowAnswers::default(),
            };
        }
        let rows = match &mut self.first_width {
            FrameWidthRowAnswers::Measured { width, rows } if *width == answer.width => rows,
            _ => self.other_widths.entry(answer.width).or_default(),
        };
        rows.record(answer.height, index);
        self.answers.push(answer);
        index
    }

    fn values(&self) -> impl Iterator<Item = &FrameContentRowUsage> { self.answers.iter() }
}

/// Which row-usage pass controls a moving frame's divider placement.
enum HeightSnap<'a, Id> {
    /// Retain the interpolated dividers while gathering probe areas.
    Off,
    /// Snap with this frame's answers without reversing an earlier frame.
    Frame {
        /// Content use at the candidate sizes probed for this frame.
        row_usage: &'a [FrameCellRowUsage<Id>],
        /// How earlier selected heights restrict this drawing pass.
        path:      PieceHeightPath<'a>,
    },
}

/// How selected piece heights constrain one divider search.
enum PieceHeightPath<'a> {
    /// Move from the preceding frame's heights toward the destination.
    AdvanceFrom(&'a [u16]),
    /// Reproduce the heights selected for the frame being drawn.
    DrawSelected(&'a [u16]),
}

impl<Id: Eq> HeightSnap<'_, Id> {
    /// Whether a measured content step cannot give up a single row.
    fn has_content_step_gap(&self, content: &TileContent<Id>) -> bool {
        let Self::Frame { row_usage, .. } = self else {
            return false;
        };
        row_usage.iter().any(|cell| {
            &cell.content == content
                && cell
                    .rows
                    .values()
                    .any(|rows| rows.fit == FrameHeightFit::LeavesGap)
        })
    }

    /// How `piece` uses the content rows inside `rect`.
    fn fit(&self, piece: &BandPiece<Id>, rect: Rect) -> FrameHeightFit {
        if matches!(piece.drawn.content, TileContent::Empty(_)) {
            return FrameHeightFit::Whole;
        }
        let Self::Frame { row_usage, .. } = self else {
            return FrameHeightFit::Filled;
        };
        let area = draw::content_area(frame_inner(rect));
        frame_height_fit(row_usage, &piece.drawn.content, area)
    }

    /// Whether `piece` fills `rect` or is whole inside it.
    #[cfg(test)]
    fn accepts(&self, piece: &BandPiece<Id>, rect: Rect) -> bool {
        self.fit(piece, rect) != FrameHeightFit::LeavesGap
    }

    /// Rows a whole piece can hand to a cutting mate.
    fn spare_rows(&self, piece: &BandPiece<Id>, rect: Rect) -> u16 {
        let Self::Frame { row_usage, .. } = self else {
            return 0;
        };
        let area = draw::content_area(frame_inner(rect));
        if matches!(piece.drawn.content, TileContent::Empty(_)) {
            return area.height;
        }
        match frame_retained_rows(row_usage, &piece.drawn.content, area) {
            RetainedRows::Unmeasured => 0,
            RetainedRows::Measured(kept) => area.height.saturating_sub(kept),
        }
    }

    /// The preceding frame's height for this ordered piece.
    fn previous_height(&self, index: usize) -> Option<u16> {
        let Self::Frame { path, .. } = self else {
            return None;
        };
        match path {
            PieceHeightPath::AdvanceFrom(heights) | PieceHeightPath::DrawSelected(heights) => {
                heights.get(index).copied()
            },
        }
    }

    /// Height this pass must reproduce, when the frame has been prepared.
    fn selected_height(&self, index: usize) -> Option<u16> {
        let Self::Frame {
            path: PieceHeightPath::DrawSelected(heights),
            ..
        } = self
        else {
            return None;
        };
        heights.get(index).copied()
    }
}

/// Piece heights already selected during one running motion.
#[derive(Default)]
struct MotionHeightPath {
    /// Grid area whose moving pieces these heights describe.
    area:          Rect,
    /// Column-growth rule whose moving pieces these heights describe.
    growth:        TileGrowth,
    /// Last progress value whose heights were recorded.
    progress:      u32,
    /// Framed heights in the stable order produced by the motion.
    piece_heights: Vec<u16>,
}

/// A content retarget's filled endpoint, retained until its inputs change.
#[derive(Default)]
enum RetargetedMotionDestination {
    /// The ordinary row allocator supplies the destination.
    #[default]
    Unconstrained,
    /// Filled geometry preserves the heights already drawn by unchanged cells.
    Filled {
        area:    Rect,
        growth:  TileGrowth,
        rows:    Vec<CellRowDemand>,
        columns: Vec<Rect>,
        span:    usize,
        panes:   Vec<ResolvedPane<usize>>,
        row_use: Vec<DestinationRowUse>,
    },
}

/// Painted and retained rows that keep one destination allocation current.
#[derive(Clone, Copy, Eq, PartialEq)]
struct DestinationRowUse {
    drawn: u16,
    kept:  u16,
}

/// Read a destination's row answers from the current frame's shared probes.
fn measure_destination_rows<Id: Clone + Eq>(
    panes: &[ResolvedPane<usize>],
    contents: &[TileContent<Id>],
    row_usage: &mut Vec<FrameCellRowUsage<Id>>,
    rows_drawn: &mut impl FnMut(&TileContent<Id>, Rect) -> u16,
    rows_kept: &mut impl FnMut(&TileContent<Id>, Rect) -> u16,
) -> Vec<DestinationRowUse> {
    panes
        .iter()
        .filter_map(|pane| {
            let content = contents.get(pane.pane.saturating_sub(TABLE_CELL))?;
            let area = draw::content_area(frame_inner(pane.area));
            let kept = measure_frame_row_use(row_usage, content, area, rows_drawn, rows_kept);
            Some(DestinationRowUse {
                drawn: frame_painted_rows(row_usage, content, area),
                kept,
            })
        })
        .collect()
}

/// Span limits that keep an unchanged piece moving in its drawn direction.
#[derive(Clone, Copy)]
struct MotionSpanBounds {
    /// Least permitted span, including a growing piece's last drawn allocation.
    smallest: u16,
    /// Greatest permitted span, including a shrinking piece's last drawn allocation.
    largest:  u16,
}

/// Whether a new destination needs constraints from unchanged moving pieces.
enum DestinationHeightBounds {
    /// The ordinary destination already preserves each drawn direction.
    Ordinary,
    /// Every pane's remaining span interval when a neighbour must keep its rows.
    Constrained(Vec<MotionSpanBounds>),
}

/// Content and span constraints shared by a retargeted endpoint's columns.
struct RetargetedHeightConstraints<'a, Id> {
    /// Every pane's content in logical cell order.
    contents:        &'a [TileContent<Id>],
    /// Current content and frame demands in logical cell order.
    row_demands:     &'a [CellRowDemand],
    /// Which logical cells have changed their exact row demand.
    changed_demands: &'a [bool],
    /// Remaining span bounds in resolved pane order.
    bounds:          &'a [MotionSpanBounds],
}

/// Whether every column can fit the retarget's height constraints.
#[derive(Clone, Copy, Eq, PartialEq)]
enum FilledColumnFit {
    /// The shared allocations fill each column within its bounds.
    Fits,
    /// No filled allocation meets the height constraints.
    NoFit,
}

/// Share a retarget's column rows while keeping every permitted content step.
fn fill_retargeted_columns<Id: Clone + Eq>(
    grid: &Grid,
    panes: &mut [ResolvedPane<usize>],
    constraints: &RetargetedHeightConstraints<'_, Id>,
    row_usage: &mut Vec<FrameCellRowUsage<Id>>,
    rows_drawn: &mut impl FnMut(&TileContent<Id>, Rect) -> u16,
    rows_kept: &mut impl FnMut(&TileContent<Id>, Rect) -> u16,
) -> FilledColumnFit {
    let contents = constraints.contents;
    let bounds = constraints.bounds;
    for column in &grid.columns {
        let members = panes
            .iter()
            .enumerate()
            .filter_map(|(index, pane)| (pane.area.x == column.x).then_some(index))
            .collect::<Vec<_>>();
        let total = column.height.saturating_sub(1);
        let ColumnHeightBalance::SpareRows(free) = release_retargeted_rows(
            &members,
            panes,
            constraints,
            total,
            row_usage,
            rows_drawn,
            rows_kept,
        ) else {
            return FilledColumnFit::NoFit;
        };
        if free > 0 {
            let mut recipients = members.clone();
            recipients.sort_by_key(|&index| {
                let pane = &panes[index];
                pane.area.height.saturating_add(free)
                    > constraints.row_demands[pane.pane.saturating_sub(TABLE_CELL)].exact_framed()
            });
            let recipient = recipients.iter().copied().find(|&index| {
                let pane = &panes[index];
                let content = &contents[pane.pane.saturating_sub(TABLE_CELL)];
                let span = pane.area.height.saturating_sub(1).saturating_add(free);
                if span > bounds[index].largest {
                    return false;
                }
                let area = draw::content_area(frame_inner(Rect {
                    height: span.saturating_add(1),
                    ..pane.area
                }));
                measure_frame_row_use(row_usage, content, area, rows_drawn, rows_kept);
                frame_height_fit(row_usage, content, area) != FrameHeightFit::LeavesGap
            });
            let Some(recipient) = recipient else {
                return FilledColumnFit::NoFit;
            };
            panes[recipient].area.height = panes[recipient].area.height.saturating_add(free);
        }
        let mut top = column.top();
        for index in members {
            panes[index].area.y = top;
            top = top.saturating_add(panes[index].area.height.saturating_sub(1));
        }
    }
    FilledColumnFit::Fits
}

/// Rows available after the permitted donors have supplied a column.
enum ColumnHeightBalance {
    /// Rows left for a piece that can fill them.
    SpareRows(u16),
    /// The bounds leave more height in use than the column has.
    InsufficientRoom,
}

/// Release rows from filled donors within their remaining motion bounds.
fn release_retargeted_rows<Id: Clone + Eq>(
    members: &[usize],
    panes: &mut [ResolvedPane<usize>],
    constraints: &RetargetedHeightConstraints<'_, Id>,
    total: u16,
    row_usage: &mut Vec<FrameCellRowUsage<Id>>,
    rows_drawn: &mut impl FnMut(&TileContent<Id>, Rect) -> u16,
    rows_kept: &mut impl FnMut(&TileContent<Id>, Rect) -> u16,
) -> ColumnHeightBalance {
    let contents = constraints.contents;
    let bounds = constraints.bounds;
    let changed = constraints.changed_demands;
    let used = members
        .iter()
        .map(|&index| panes[index].area.height.saturating_sub(1))
        .sum::<u16>();
    let mut excess = used.saturating_sub(total);
    let mut donors = members.to_vec();
    donors.sort_by_key(|&index| !changed[panes[index].pane.saturating_sub(TABLE_CELL)]);
    for index in donors {
        if excess == 0 {
            break;
        }
        let pane = &mut panes[index];
        let content = &contents[pane.pane.saturating_sub(TABLE_CELL)];
        let span = pane.area.height.saturating_sub(1);
        let target = span.saturating_sub(excess).max(bounds[index].smallest);
        let next = measured_filled_span(
            row_usage,
            content,
            pane.area,
            target,
            bounds[index].smallest,
            rows_drawn,
            rows_kept,
        );
        let FilledSpan::Fits(next) = next else {
            continue;
        };
        pane.area.height = next.saturating_add(1);
        excess = excess.saturating_sub(span.saturating_sub(next));
    }
    let used = members
        .iter()
        .map(|&index| panes[index].area.height.saturating_sub(1))
        .sum::<u16>();
    if used > total {
        ColumnHeightBalance::InsufficientRoom
    } else {
        ColumnHeightBalance::SpareRows(total.saturating_sub(used))
    }
}

/// The active motion's clock, compared without waiting in render tests.
#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum MotionTiming {
    /// No motion is running.
    Settled,
    /// The start and duration that a content retarget keeps.
    Active { started: Instant, millis: u64 },
}

/// Cell geometry from the last settled frame that drew the cells view.
#[derive(Default)]
enum LastSettledCellGeometry {
    /// No settled cells frame has supplied geometry for a later motion.
    #[default]
    NoCellsFrame,
    /// Geometry that a settled cells frame drew in these bounds.
    Drawn {
        /// Bounds the geometry fills.
        area:   Rect,
        /// Column rule used to resolve the geometry.
        growth: TileGrowth,
        /// Every settled cell and column rect.
        grid:   Grid,
    },
}

/// How one measured content allocation fits, with an unmeasured size rejected.
fn frame_height_fit<Id: Eq>(
    usage: &[FrameCellRowUsage<Id>],
    content: &TileContent<Id>,
    area: Rect,
) -> FrameHeightFit {
    match usage
        .iter()
        .find(|cell| &cell.content == content)
        .map(|cell| cell.rows.answer(area))
    {
        Some(FrameRowAnswer::Measured(rows)) => rows.fit,
        _ => FrameHeightFit::LeavesGap,
    }
}

/// The retained-row answer for one measured content allocation.
fn frame_retained_rows<Id: Eq>(
    usage: &[FrameCellRowUsage<Id>],
    content: &TileContent<Id>,
    area: Rect,
) -> RetainedRows {
    match usage
        .iter()
        .find(|cell| &cell.content == content)
        .map(|cell| cell.rows.answer(area))
    {
        Some(FrameRowAnswer::Measured(rows)) => rows.retained_rows(),
        _ => RetainedRows::Unmeasured,
    }
}

/// Painted rows at one recorded allocation, keeping unmeasured rows in place.
fn frame_painted_rows<Id: Eq>(
    usage: &[FrameCellRowUsage<Id>],
    content: &TileContent<Id>,
    area: Rect,
) -> u16 {
    match usage
        .iter()
        .find(|cell| &cell.content == content)
        .map(|cell| cell.rows.answer(area))
    {
        Some(FrameRowAnswer::Measured(rows)) => rows.drawn,
        _ => area.height,
    }
}

/// Frame-cache index for a content identity, retaining stable cell order.
fn frame_cell_index<Id: Clone + Eq>(
    usage: &mut Vec<FrameCellRowUsage<Id>>,
    content: &TileContent<Id>,
) -> usize {
    usage
        .iter()
        .position(|cell| &cell.content == content)
        .unwrap_or_else(|| {
            usage.push(FrameCellRowUsage {
                content: content.clone(),
                rows:    FrameCellRowAnswers::default(),
            });
            usage.len() - 1
        })
}

/// Record one content-area answer, reusing an answer already gathered this frame.
#[cfg(test)]
fn measure_frame_row_usage<Id: Clone + Eq>(
    usage: &mut Vec<FrameCellRowUsage<Id>>,
    content: &TileContent<Id>,
    area: Rect,
    rows_drawn: &mut impl FnMut(&TileContent<Id>, Rect) -> u16,
) -> u16 {
    let index = frame_cell_index(usage, content);
    measured_content_rows(&mut usage[index].rows, content, area, rows_drawn).drawn
}

/// Supply one stored answer, including a whole-content answer when needed.
fn measured_content_rows<'a, Id>(
    answers: &'a mut FrameCellRowAnswers,
    content: &TileContent<Id>,
    area: Rect,
    rows_drawn: &mut impl FnMut(&TileContent<Id>, Rect) -> u16,
) -> &'a FrameContentRowUsage {
    let index = match answers.index(area) {
        FrameRowIndex::Measured(index) => index,
        FrameRowIndex::Unmeasured => {
            let drawn = rows_drawn(content, area).min(area.height);
            let fit = if drawn == area.height {
                FrameHeightFit::Filled
            } else if area.height == u16::MAX {
                FrameHeightFit::Whole
            } else {
                let whole_area = Rect {
                    height: u16::MAX,
                    ..area
                };
                let whole = match answers.answer(whole_area) {
                    FrameRowAnswer::Measured(rows) => rows.drawn,
                    FrameRowAnswer::Unmeasured => {
                        let whole = rows_drawn(content, whole_area);
                        answers.record(FrameContentRowUsage {
                            width:    area.width,
                            height:   u16::MAX,
                            drawn:    whole,
                            fit:      FrameHeightFit::Whole,
                            retained: OnceLock::new(),
                        });
                        whole
                    },
                };
                if drawn >= whole {
                    FrameHeightFit::Whole
                } else {
                    FrameHeightFit::LeavesGap
                }
            };
            answers.record(FrameContentRowUsage {
                width: area.width,
                height: area.height,
                drawn,
                fit,
                retained: OnceLock::new(),
            })
        },
    };
    &answers.answers[index]
}

/// Record painted and retained rows together, reusing this frame's answer.
fn measure_frame_row_use<Id: Clone + Eq>(
    usage: &mut Vec<FrameCellRowUsage<Id>>,
    content: &TileContent<Id>,
    area: Rect,
    rows_drawn: &mut impl FnMut(&TileContent<Id>, Rect) -> u16,
    rows_kept: &mut impl FnMut(&TileContent<Id>, Rect) -> u16,
) -> u16 {
    let index = frame_cell_index(usage, content);
    measure_cell_retained_rows(&mut usage[index], area, rows_drawn, rows_kept)
}

/// Record both purposes directly through a known frame cell index.
fn measure_cell_retained_rows<Id>(
    cell: &mut FrameCellRowUsage<Id>,
    area: Rect,
    rows_drawn: &mut impl FnMut(&TileContent<Id>, Rect) -> u16,
    rows_kept: &mut impl FnMut(&TileContent<Id>, Rect) -> u16,
) -> u16 {
    let rows = measured_content_rows(&mut cell.rows, &cell.content, area, rows_drawn);
    *rows
        .retained
        .get_or_init(|| rows_kept(&cell.content, area).min(area.height))
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
    /// The cell whose contents this piece paints, independent of its number.
    identity: CellIdentity<Id>,
    /// What the cell draws.
    content:  TileContent<Id>,
    /// Whether the cell holds focus, which is what lights its border.
    focused:  bool,
}

/// A painted cell's identity, including empty cells whose numbers can change.
#[derive(Clone)]
enum CellIdentity<Id> {
    Summary,
    Slot(Slot<Id>),
}

/// Click geometry from the most recent grid paint, before any focus change.
enum PaintedCells<Id> {
    Undrawn,
    Drawn(Vec<PaintedCell<Id>>),
}

struct PaintedCell<Id> {
    identity: CellIdentity<Id>,
    frame:    PaneFrame,
}

/// Whether the painted cell still owns a slot that can receive input.
pub(super) enum CellHit<Id> {
    Missed,
    Retired,
    Current {
        index:   usize,
        content: TileContent<Id>,
        frame:   PaneFrame,
    },
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
pub enum FocusLocation {
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

/// Whether a step's depth has seen the widths its destination draws at.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DestinationDepthStatus {
    /// The depth came from demands measured for this arrangement.
    Resolved,
    /// A key used the preceding frame's demands and awaits its first render.
    AwaitingMeasuredWidths,
}

/// Whether one demand sync belongs to the frame being drawn.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DemandSync {
    /// A scan independent of the current render.
    Independent,
    /// Demands measured at the widths this render will draw.
    CurrentRender,
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
    /// Row demands each cell holds, in cell order and the summary's first.
    rows:          Vec<CellRowDemand>,
    /// Which cell holds the ring, or whether that slot is outside the layout.
    focused:       FocusLocation,
    /// Columns the summary asks to reach across, its own included; see
    /// [`summary_span`]. Held with the rows so the summary widening or
    /// narrowing is a change the grid travels through like any other.
    summary_span:  usize,
    /// Positions occupied by the summary, including its first.
    summary_depth: usize,
}

impl HeldCellLayout {
    /// Whether changing from `other` needs the grid's resize motion.
    fn needs_motion_from(&self, other: &Self) -> bool {
        self.focused != other.focused
            || self.summary_span != other.summary_span
            || self.summary_depth != other.summary_depth
            || self.rows.len() != other.rows.len()
            || self
                .rows
                .iter()
                .zip(&other.rows)
                .any(|(current, held)| current.stepped_framed() != held.stepped_framed())
    }
}

/// The arrangement a transition is moving away from.
struct Transition<Id> {
    /// The cells as they stood before the change.
    from:         Vec<Slot<Id>>,
    /// What those cells were drawn at. Snapshotted rather than
    /// recomputed, because by the time a transition starts the demands
    /// it is moving away from have already been replaced by the ones it
    /// is moving toward.
    held:         HeldCellLayout,
    /// Exact cell geometry drawn before the motion began.
    source:       Box<Grid>,
    /// When the motion began.
    started:      Instant,
    /// How long this one step runs for.
    millis:       u64,
    /// Whether the destination depth has seen its drawn widths.
    depth_status: DestinationDepthStatus,
}

/// One arrangement waiting its turn, and how long the move into it
/// takes once it comes up.
struct Step<Id> {
    /// The cells as this step leaves them.
    slots:        Vec<Slot<Id>>,
    /// Summary depth at the end of this step.
    depth:        usize,
    /// How long the move into `slots` runs for.
    millis:       u64,
    /// Whether the destination depth has seen its drawn widths.
    depth_status: DestinationDepthStatus,
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
    slots:                  Vec<Slot<Id>>,
    /// Summary depth of `slots`.
    depth:                  usize,
    /// Arrangements waiting their turn, each one cell's move from the
    /// one before it. Only ever one of them is in flight, which is what
    /// makes a change propagate through the grid instead of happening
    /// to every cell at once.
    pending:                VecDeque<Step<Id>>,
    /// What each cell is currently drawn at, which is what a change
    /// animates away from. Empty until the first scan settles it.
    held:                   HeldCellLayout,
    /// What every cell is asking for, as the last scan left it.
    demands:                TileDemands<Id>,
    /// Whether the geometry is settled or playing a timed move.
    motion:                 GridMotion<Id>,
    /// The rect the last frame laid out, so [`Self::add`] can tell
    /// whether the cells it would create still fit on screen.
    area:                   Rect,
    /// How the grid grows, as the last frame had it. Retained so a
    /// mouse click can resolve the same geometry the frame drew without
    /// the caller carrying the settings to every hit test.
    growth:                 TileGrowth,
    /// The view drawing, hit-testing and focus movement share until a
    /// settled frame commits another one.
    held_display:           GridDisplay,
    /// Identity for the next cell opened or emptied.
    next_slot:              u64,
    /// The cell the focus ring is on.
    focus:                  Focus<Id>,
    /// The sizes and timings the grid is laid out and animated with.
    settings:               TileSettings,
    /// Summary clip rectangles painted by the last grid draw.
    painted_summary_frames: Vec<Rect>,
    /// Stable identities and geometry retained from the frame input refers to.
    painted_cells:          PaintedCells<Id>,
    /// Content-row usage at every size inspected during the current frame.
    frame_row_usage:        Vec<FrameCellRowUsage<Id>>,
    /// Cell geometry drawn by the last settled frame.
    last_settled_geometry:  LastSettledCellGeometry,
    /// One-way heights selected by the current motion's earlier frames.
    motion_heights:         MotionHeightPath,
    /// Filled destination geometry constrained by a content change in flight.
    retargeted_destination: RetargetedMotionDestination,
    /// Admission is checked once until an arrangement or its rules change.
    layout_admission:       OnceLock<CheckedLayoutAdmission>,
    #[cfg(test)]
    admission_checks:       AtomicUsize,
}

impl<Id: Clone + Eq + Debug> Default for TileGrid<Id> {
    fn default() -> Self { Self::new() }
}

impl<Id: Clone + Eq + Debug> TileGrid<Id> {
    /// A grid holding nothing but the summary.
    #[must_use]
    pub fn new() -> Self {
        Self {
            slots:                         Vec::new(),
            depth:                         1,
            pending:                       VecDeque::new(),
            held:                          HeldCellLayout {
                rows:          Vec::new(),
                focused:       FocusLocation::Departing,
                summary_span:  1,
                summary_depth: 1,
            },
            demands:                       TileDemands::default(),
            motion:                        GridMotion::Settled,
            area:                          Rect::ZERO,
            growth:                        TileGrowth::default(),
            held_display:                  GridDisplay::Cells,
            next_slot:                     0,
            focus:                         Focus::Summary,
            settings:                      TileSettings::default(),
            painted_summary_frames:        Vec::new(),
            painted_cells:                 PaintedCells::Undrawn,
            frame_row_usage:               Vec::new(),
            last_settled_geometry:         LastSettledCellGeometry::default(),
            motion_heights:                MotionHeightPath::default(),
            retargeted_destination:        RetargetedMotionDestination::default(),
            layout_admission:              OnceLock::new(),
            #[cfg(test)]
            admission_checks:              AtomicUsize::new(0),
        }
    }

    /// Sets the core width, including its frame. The floor is required by default.
    pub fn set_min_tile_width(&mut self, width: u16) {
        let width = width.max(super::constants::MIN_TILE_WIDTH);
        self.update_layout_rules(
            self.area,
            EndpointLayoutRules {
                min_tile_width: width,
                ..EndpointLayoutRules::new(self.growth, &self.settings)
            },
        );
    }

    /// Sets whether a terminal below the core width keeps a compressed column.
    pub fn set_tile_width_floor(&mut self, floor: TileWidthFloor) {
        self.update_layout_rules(
            self.area,
            EndpointLayoutRules {
                width_floor: floor,
                ..EndpointLayoutRules::new(self.growth, &self.settings)
            },
        );
    }

    /// Sets the rule that chooses command cells or the summary alone.
    ///
    /// A motion retains the view it began with. A settled grid applies
    /// a view at once.
    pub const fn set_view(&mut self, view: TileView) {
        self.settings.view = view;
        if !matches!(self.motion, GridMotion::Settled) {
            return;
        }
        match view {
            TileView::Summary => self.held_display = GridDisplay::SummaryAlone,
            TileView::Cells => self.held_display = GridDisplay::Cells,
        }
    }

    /// Sets the shortest height a tile may occupy, including its frame.
    pub fn set_min_tile_height(&mut self, height: u16) {
        let height = height.max(super::constants::MIN_TILE_HEIGHT);
        self.update_layout_rules(
            self.area,
            EndpointLayoutRules {
                min_tile_height: height,
                ..EndpointLayoutRules::new(self.growth, &self.settings)
            },
        );
    }

    /// Choose whether the summary keeps its measured rows before the tiles.
    pub fn set_summary_row_allocation(&mut self, allocation: SummaryRowAllocation) {
        self.update_layout_rules(
            self.area,
            EndpointLayoutRules {
                summary_allocation: allocation,
                ..EndpointLayoutRules::new(self.growth, &self.settings)
            },
        );
    }

    /// Probe destination row steps, then nearby heights this moving frame may use.
    pub(super) fn measure_row_steps(
        &mut self,
        area: Rect,
        growth: TileGrowth,
        progress: u32,
        mut rows_drawn: impl FnMut(&TileContent<Id>, Rect) -> u16,
        mut rows_kept: impl FnMut(&TileContent<Id>, Rect) -> u16,
    ) {
        self.frame_row_usage.clear();
        if !self.holds_in(area, growth) {
            if matches!(self.motion, GridMotion::Settled) {
                self.held_display = GridDisplay::SummaryAlone;
                self.last_settled_geometry = LastSettledCellGeometry::NoCellsFrame;
            }
            return;
        }
        let contents: Vec<TileContent<Id>> = cells(&self.slots)
            .into_iter()
            .map(|(content, _)| content)
            .collect();
        let mut frame_row_usage: Vec<_> = contents
            .iter()
            .cloned()
            .map(|content| FrameCellRowUsage {
                content,
                rows: FrameCellRowAnswers::default(),
            })
            .collect();
        let held = self.drawn_held();
        let cells_grid = Grid::with_row_use(
            area,
            &held,
            growth,
            &self.settings,
            &mut |index, area, purpose| {
                contents.get(index).map_or(area.height, |_| {
                    let kept = measure_cell_retained_rows(
                        &mut frame_row_usage[index],
                        area,
                        &mut rows_drawn,
                        &mut rows_kept,
                    );
                    match purpose {
                        RowUsePurpose::RetainContent => kept,
                        RowUsePurpose::PaintContent => {
                            match frame_row_usage[index].rows.answer(area) {
                                FrameRowAnswer::Measured(rows) => rows.drawn,
                                FrameRowAnswer::Unmeasured => area.height,
                            }
                        },
                    }
                })
            },
        );
        let cells_grid = self.bound_retargeted_destination(
            cells_grid,
            &contents,
            &mut frame_row_usage,
            &mut rows_drawn,
            &mut rows_kept,
        );
        self.frame_row_usage = frame_row_usage;
        self.commit_display();
        if matches!(self.motion, GridMotion::Settled) {
            self.last_settled_geometry = if self.held_display == GridDisplay::Cells {
                LastSettledCellGeometry::Drawn {
                    area,
                    growth,
                    grid: cells_grid,
                }
            } else {
                LastSettledCellGeometry::NoCellsFrame
            };
            self.motion_heights = MotionHeightPath::default();
            return;
        }
        if self.held_display != GridDisplay::Cells || !matches!(self.motion, GridMotion::Moving(_))
        {
            self.motion_heights = MotionHeightPath::default();
            return;
        }
        if progress >= PROGRESS_SCALE {
            self.motion_heights = MotionHeightPath::default();
            return;
        }
        self.measure_moving_height_rows(area, growth, progress, &mut rows_drawn, &mut rows_kept);
    }

    /// Keep a filled content retarget from reversing unchanged resident heights.
    fn bound_retargeted_destination(
        &mut self,
        mut grid: Grid,
        contents: &[TileContent<Id>],
        row_usage: &mut Vec<FrameCellRowUsage<Id>>,
        rows_drawn: &mut impl FnMut(&TileContent<Id>, Rect) -> u16,
        rows_kept: &mut impl FnMut(&TileContent<Id>, Rect) -> u16,
    ) -> Grid {
        if matches!(
            self.retargeted_destination,
            RetargetedMotionDestination::Filled { .. }
        ) {
            let ordinary_panes = grid.resolved.panes.clone();
            if self.apply_retargeted_destination(&mut grid) {
                let measured = measure_destination_rows(
                    &grid.resolved.panes,
                    contents,
                    row_usage,
                    rows_drawn,
                    rows_kept,
                );
                if let RetargetedMotionDestination::Filled { row_use, .. } =
                    &self.retargeted_destination
                    && *row_use == measured
                {
                    return grid;
                }
                grid.resolved = ResolvedPaneLayout::new(ordinary_panes);
            }
            self.retargeted_destination = RetargetedMotionDestination::Unconstrained;
        }
        let GridMotion::Moving(transition) = &self.motion else {
            return grid;
        };
        if self.motion_heights.piece_heights.is_empty() || grid.columns != transition.source.columns
        {
            return grid;
        }
        let wants = cell_wants(&self.demands, &self.slots, &self.settings);
        let changed = contents
            .iter()
            .enumerate()
            .map(|(index, content)| {
                let source_cell = match content {
                    TileContent::Summary => Some(TABLE_CELL),
                    TileContent::Group(id) => cell_of(&transition.from, &Slot::Group(id.clone())),
                    TileContent::Empty(_) => None,
                };
                source_cell
                    .and_then(|cell| transition.held.rows.get(cell.saturating_sub(TABLE_CELL)))
                    .is_some_and(|source| source.exact_framed() != wants[index].exact_framed())
            })
            .collect::<Vec<_>>();
        if !changed.iter().any(|&changed| changed) {
            return grid;
        }
        let mut panes = grid.resolved.panes.clone();
        let DestinationHeightBounds::Constrained(bounds) =
            self.bound_resident_heights(&grid, contents, &changed, row_usage, &mut panes)
        else {
            return grid;
        };
        let constraints = RetargetedHeightConstraints {
            contents,
            row_demands: &wants,
            changed_demands: &changed,
            bounds: &bounds,
        };
        if fill_retargeted_columns(
            &grid,
            &mut panes,
            &constraints,
            row_usage,
            rows_drawn,
            rows_kept,
        ) == FilledColumnFit::NoFit
        {
            return grid;
        }
        grid.resolved = ResolvedPaneLayout::new(panes.clone());
        let row_use = measure_destination_rows(&panes, contents, row_usage, rows_drawn, rows_kept);
        self.retargeted_destination = RetargetedMotionDestination::Filled {
            area: grid.area,
            growth: self.growth,
            rows: wants,
            columns: grid.columns.clone(),
            span: grid.span,
            panes,
            row_use,
        };
        grid
    }

    /// Restrict resident heights to their drawn direction and empty height floor.
    fn bound_resident_heights(
        &self,
        grid: &Grid,
        contents: &[TileContent<Id>],
        changed: &[bool],
        row_usage: &[FrameCellRowUsage<Id>],
        panes: &mut [ResolvedPane<usize>],
    ) -> DestinationHeightBounds {
        let GridMotion::Moving(transition) = &self.motion else {
            return DestinationHeightBounds::Ordinary;
        };
        let drawing = self.drawing_at_with_height_snap(
            grid.area,
            self.growth,
            self.motion_heights.progress,
            &HeightSnap::Off,
        );
        let mut bounds = vec![
            MotionSpanBounds {
                smallest: grid.rules.min_tile_height.saturating_sub(1),
                largest:  grid.area.height.saturating_sub(1),
            };
            panes.len()
        ];
        let mut constrained = false;
        for (index, pane) in panes.iter_mut().enumerate() {
            let Some(content) = contents.get(pane.pane.saturating_sub(TABLE_CELL)) else {
                continue;
            };
            let empty = match content {
                TileContent::Summary => false,
                TileContent::Group(id) => {
                    !self.demands.holds(id)
                        && row_usage.iter().any(|cell| {
                            &cell.content == content
                                && cell.rows.values().any(|rows| {
                                    rows.height > 0 && rows.fit == FrameHeightFit::Whole
                                })
                                && cell.rows.values().all(|rows| rows.drawn == 0)
                        })
                },
                TileContent::Empty(_) => true,
            };
            if changed[pane.pane.saturating_sub(TABLE_CELL)] && !empty {
                continue;
            }
            let source_cell = match content {
                TileContent::Summary => Some(TABLE_CELL),
                TileContent::Group(id) => cell_of(&transition.from, &Slot::Group(id.clone())),
                TileContent::Empty(cell) => cell
                    .checked_sub(TABLE_CELL + 1)
                    .and_then(|index| self.slots.get(index))
                    .and_then(|slot| cell_of(&transition.from, slot)),
            };
            let Some(source) = source_cell.and_then(|cell| transition.source.cell(cell)) else {
                continue;
            };
            let Some(previous) = drawing
                .pieces
                .iter()
                .position(|piece| &piece.placement.content == content)
                .and_then(|piece| self.motion_heights.piece_heights.get(piece))
                .copied()
            else {
                continue;
            };
            let previous_span = previous
                .saturating_sub(1)
                .max(bounds[index].smallest)
                .min(bounds[index].largest);
            if empty {
                bounds[index].largest = previous_span;
            } else {
                match previous.cmp(&source.height) {
                    Ordering::Less => bounds[index].largest = previous_span,
                    Ordering::Greater => bounds[index].smallest = previous_span,
                    Ordering::Equal => {},
                }
            }
            let span = pane.area.height.saturating_sub(1);
            let retained = span.clamp(bounds[index].smallest, bounds[index].largest);
            constrained |= retained != span;
            pane.area.height = retained.saturating_add(1);
        }
        if constrained {
            DestinationHeightBounds::Constrained(bounds)
        } else {
            DestinationHeightBounds::Ordinary
        }
    }

    /// Apply a filled endpoint only while its layout inputs still agree.
    fn apply_retargeted_destination(&self, grid: &mut Grid) -> bool {
        let RetargetedMotionDestination::Filled {
            area,
            growth,
            rows,
            columns,
            span,
            panes,
            ..
        } = &self.retargeted_destination
        else {
            return false;
        };
        if *area != grid.area
            || *growth != self.growth
            || *rows != cell_wants(&self.demands, &self.slots, &self.settings)
            || *columns != grid.columns
            || *span != grid.span
            || panes
                .iter()
                .any(|pane| pane.area.height < grid.rules.min_tile_height)
        {
            return false;
        }
        grid.resolved = ResolvedPaneLayout::new(panes.clone());
        true
    }

    /// Probe the one-way candidates used to place this moving frame.
    fn measure_moving_height_rows(
        &mut self,
        area: Rect,
        growth: TileGrowth,
        progress: u32,
        rows_drawn: &mut impl FnMut(&TileContent<Id>, Rect) -> u16,
        rows_kept: &mut impl FnMut(&TileContent<Id>, Rect) -> u16,
    ) {
        let drawing = self.drawing_at_with_height_snap(area, growth, progress, &HeightSnap::Off);
        if self.motion_heights.area != area
            || self.motion_heights.growth != growth
            || progress < self.motion_heights.progress
            || self.motion_heights.piece_heights.len() != drawing.pieces.len()
        {
            self.motion_heights = MotionHeightPath::default();
        }
        let probes = drawing
            .pieces
            .into_iter()
            .enumerate()
            .filter_map(|(index, piece)| match piece.height_snap {
                PieceHeightSnap::Search { start, end } => Some((
                    index,
                    piece.placement.content,
                    piece.placement.frame.clip(),
                    start,
                    end,
                )),
                PieceHeightSnap::Settled | PieceHeightSnap::KeepStartingHeight => None,
            })
            .collect::<Vec<_>>();
        let mut frame_row_usage = std::mem::take(&mut self.frame_row_usage);
        for (index, content, rect, start, end) in probes {
            let previous = self
                .motion_heights
                .piece_heights
                .get(index)
                .copied()
                .unwrap_or(start);
            let mut lower = previous.min(end);
            let upper = previous.max(end);
            let interpolated = rect.height.clamp(lower, upper);
            let previous_area = draw::content_area(frame_inner(Rect {
                height: previous,
                ..rect
            }));
            measure_frame_row_use(
                &mut frame_row_usage,
                &content,
                previous_area,
                rows_drawn,
                rows_kept,
            );
            if frame_height_fit(&frame_row_usage, &content, previous_area)
                == FrameHeightFit::LeavesGap
            {
                let filled = frame_painted_rows(&frame_row_usage, &content, previous_area)
                    .saturating_add(previous.saturating_sub(previous_area.height));
                lower = lower.min(filled);
            }
            let probe_from = if frame_height_fit(&frame_row_usage, &content, previous_area)
                == FrameHeightFit::Whole
            {
                previous
            } else {
                interpolated
            };
            let toward_end = match probe_from.cmp(&end) {
                Ordering::Less => probe_from.saturating_add(MOVING_HEIGHT_PROBE_STEP),
                Ordering::Equal => probe_from,
                Ordering::Greater => probe_from.saturating_sub(MOVING_HEIGHT_PROBE_STEP),
            }
            .clamp(lower, upper);
            for height in [interpolated, previous, toward_end, end] {
                let area = draw::content_area(frame_inner(Rect { height, ..rect }));
                let drawn = measure_frame_row_use(
                    &mut frame_row_usage,
                    &content,
                    area,
                    rows_drawn,
                    rows_kept,
                );
                if frame_height_fit(&frame_row_usage, &content, area) == FrameHeightFit::Filled {
                    continue;
                }
                let frame_rows = height.saturating_sub(area.height);
                let filled_height = drawn.saturating_add(frame_rows).clamp(lower, upper);
                let filled_area = draw::content_area(frame_inner(Rect {
                    height: filled_height,
                    ..rect
                }));
                measure_frame_row_use(
                    &mut frame_row_usage,
                    &content,
                    filled_area,
                    rows_drawn,
                    rows_kept,
                );
            }
        }
        self.frame_row_usage = frame_row_usage;
        self.select_moving_heights(area, growth, progress);
    }

    /// Select and retain heights for this measured motion frame.
    fn select_moving_heights(&mut self, area: Rect, growth: TileGrowth, progress: u32) {
        let previous_heights = self.motion_heights.piece_heights.clone();
        let height_snap = HeightSnap::Frame {
            row_usage: &self.frame_row_usage,
            path:      PieceHeightPath::AdvanceFrom(&previous_heights),
        };
        let drawing = self.drawing_at_with_height_snap(area, growth, progress, &height_snap);
        self.motion_heights = MotionHeightPath {
            area,
            growth,
            progress,
            piece_heights: drawing
                .pieces
                .iter()
                .map(|piece| piece.placement.frame.clip().height)
                .collect(),
        };
    }

    /// Commit the view for a settled frame whose cells fit the area.
    ///
    /// Only the area decides: rows a cell cannot show go to its hidden
    /// count rather than switching the view.
    const fn commit_display(&mut self) {
        if !matches!(self.motion, GridMotion::Settled) {
            return;
        }
        self.held_display = match self.settings.view {
            TileView::Summary => GridDisplay::SummaryAlone,
            TileView::Cells => GridDisplay::Cells,
        };
    }

    /// The stored retained-row answer, or every row when unmeasured.
    fn frame_rows_kept(&self, content: &TileContent<Id>, area: Rect) -> u16 {
        match frame_retained_rows(&self.frame_row_usage, content, area) {
            RetainedRows::Unmeasured => area.height,
            RetainedRows::Measured(kept) => kept,
        }
    }

    /// Resolve the destination with the step probes gathered for it.
    fn resolved_grid(&self, area: Rect, held: &HeldCellLayout, growth: TileGrowth) -> Grid {
        self.resolved_grid_for(area, &self.slots, held, growth)
    }

    /// Resolve one endpoint arrangement with this pass's retained rows.
    fn resolved_grid_for(
        &self,
        area: Rect,
        slots: &[Slot<Id>],
        held: &HeldCellLayout,
        growth: TileGrowth,
    ) -> Grid {
        let contents: Vec<TileContent<Id>> = cells(slots)
            .into_iter()
            .map(|(content, _)| content)
            .collect();
        let mut grid = Grid::with_row_use(
            area,
            held,
            growth,
            &self.settings,
            &mut |index, area, purpose| {
                contents
                    .get(index)
                    .map_or(area.height, |content| match purpose {
                        RowUsePurpose::RetainContent => self.frame_rows_kept(content, area),
                        RowUsePurpose::PaintContent => {
                            frame_painted_rows(&self.frame_row_usage, content, area)
                        },
                    })
            },
        );
        if slots == self.slots && self.apply_retargeted_destination(&mut grid) {
            return grid;
        }
        grid
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
                .any(|(want, held)| want.stepped_framed() > held.stepped_framed())
        {
            return HeldCellLayout {
                rows: wants,
                focused,
                summary_span,
                summary_depth: self.depth,
            };
        }
        let rows = self
            .held
            .rows
            .iter()
            .zip(wants)
            .map(|(&held, current)| held.with_exact_from(current))
            .collect();
        HeldCellLayout {
            rows,
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
        match column_layout(
            self.count(),
            self.depth,
            self.growth,
            self.area,
            &self.settings,
        ) {
            ColumnPlan::Columns(layout) => summary_span(
                self.area,
                layout.columns.widths.len(),
                self.demands.summary_width,
            ),
            ColumnPlan::InsufficientWidth | ColumnPlan::InsufficientRows => 1,
        }
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
        let held = self.demand_measurement_held();
        let ColumnPlan::Columns(layout) = column_layout(
            self.count(),
            held.summary_depth,
            growth,
            area,
            &self.settings,
        ) else {
            return vec![(TileContent::Summary, frame_inner(area).width)];
        };
        let ColumnPlan::Columns(layout) = layout.for_summary(area, &held, &self.settings) else {
            return vec![(TileContent::Summary, frame_inner(area).width)];
        };
        let ColumnLayout {
            widths: column_heights,
            summary_depth: depth,
            ..
        } = layout.columns;
        let opened: Vec<Rect> =
            Layout::horizontal(constraints_for_sizes(&fills(column_heights.len())))
                .split(area)
                .to_vec();
        let mut widths = Vec::with_capacity(self.count());
        for (column, &cells_here) in column_heights.iter().enumerate() {
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
        let summary = self
            .resolved_grid(area, &held, growth)
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

    /// Existing allocation layout used to prepare widths before the demand pass.
    pub(super) fn preparation_allocations(
        &self,
        area: Rect,
        growth: TileGrowth,
    ) -> Vec<(TileContent<Id>, Rect)> {
        if self.display() == GridDisplay::SummaryAlone {
            return vec![(TileContent::Summary, draw::content_area(frame_inner(area)))];
        }
        let layout = self.resolved_grid(area, &self.demand_measurement_held(), growth);
        cells(&self.slots)
            .into_iter()
            .filter_map(|(content, index)| {
                layout
                    .cell(index)
                    .map(|rect| (content, draw::content_area(frame_inner(rect))))
            })
            .collect()
    }

    /// Destination allocations with enough rows for either endpoint of a move.
    /// Source geometry is retained for the whole transition, so intermediate
    /// heights cannot change a provider's measured visible slice.
    pub(super) fn motion_preparation_allocations(
        &self,
        area: Rect,
        growth: TileGrowth,
    ) -> Vec<(TileContent<Id>, Rect)> {
        let mut allocations = self.preparation_allocations(area, growth);
        if let GridMotion::Moving(transition) = &self.motion {
            let source_cells = cells(&transition.from);
            for (content, allocation) in &mut allocations {
                if let Some((_, index)) = source_cells.iter().find(|(source, _)| source == content)
                    && let Some(source) = transition.source.cell(*index)
                {
                    let source = draw::content_area(frame_inner(source));
                    allocation.height = allocation.height.max(source.height);
                }
            }
        }
        allocations
    }

    /// Destination state used to ask cells for their wrapped row demands.
    fn demand_measurement_held(&self) -> HeldCellLayout {
        let mut held = self.drawn_held();
        if matches!(
            self.motion,
            GridMotion::Moving(Transition {
                depth_status: DestinationDepthStatus::AwaitingMeasuredWidths,
                ..
            })
        ) {
            held.summary_depth = 1;
        }
        held
    }

    /// Record the geometry the pane just laid out.
    pub fn set_layout(&mut self, area: Rect, growth: TileGrowth) {
        self.update_layout_rules(area, EndpointLayoutRules::new(growth, &self.settings));
    }

    /// The only rule-writing boundary also retires geometry made under the old rules.
    fn update_layout_rules(&mut self, area: Rect, rules: EndpointLayoutRules) {
        if self.area != area || EndpointLayoutRules::new(self.growth, &self.settings) != rules {
            self.area = area;
            self.growth = rules.growth;
            self.settings.min_tile_width = rules.min_tile_width;
            self.settings.min_tile_height = rules.min_tile_height;
            self.settings.tile_width_floor = rules.width_floor;
            self.settings.summary_row_allocation = rules.summary_allocation;
            self.replan_layout_endpoints();
        }
    }

    /// A changed allocation cannot inherit geometry measured under the old floors.
    /// Keep the requested final cell order, then resolve it through the checked planner.
    fn replan_layout_endpoints(&mut self) {
        self.layout_admission.take();
        let depth = self.target_depth();
        self.complete_arrangement(self.target(), depth);
        self.pending.clear();
        self.motion = GridMotion::Settled;
        self.last_settled_geometry = LastSettledCellGeometry::NoCellsFrame;
        self.frame_row_usage.clear();
        self.motion_heights = MotionHeightPath::default();
        self.retargeted_destination = RetargetedMotionDestination::Unconstrained;
        let rows = cell_wants(&self.demands, &self.slots, &self.settings);
        self.depth = depth;
        self.held = HeldCellLayout {
            rows,
            focused: self.focused_cell(),
            summary_span: self.wanted_span(),
            summary_depth: self.depth,
        };
        self.commit_display();
    }

    /// Record one drawn layout, completing a move that began in another area.
    pub(super) fn set_draw_layout(&mut self, area: Rect, growth: TileGrowth) {
        let area_changed = self.area != area;
        self.set_layout(area, growth);
        if area_changed {
            self.retargeted_destination = RetargetedMotionDestination::Unconstrained;
        }
        if area_changed && matches!(self.motion, GridMotion::Moving(_)) {
            self.advance();
        }
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
        self.sync_demands(demands, growth, DemandSync::Independent);
    }

    /// Fold demands measured by the current render into its destination.
    pub(super) fn sync_rendered_demands(&mut self, demands: &TileDemands<Id>, growth: TileGrowth) {
        self.sync_demands(demands, growth, DemandSync::CurrentRender);
    }

    /// Reconcile one demand observation with the headed arrangement.
    fn sync_demands(
        &mut self,
        demands: &TileDemands<Id>,
        growth: TileGrowth,
        demand_sync: DemandSync,
    ) {
        if self.demands != *demands {
            self.layout_admission.take();
            self.demands = demands.clone();
        }
        self.set_layout(self.area, growth);
        if demand_sync == DemandSync::CurrentRender {
            self.fold_rendered_claims();
        }
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
                    let empty = arrangement[index].clone();
                    arrangement[index] = Slot::Group(id.clone());
                    if demand_sync != DemandSync::CurrentRender
                        || !self.claim_moving_empty(&empty, &id)
                    {
                        steps.push(arrangement.clone());
                    }
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
        if steps.is_empty() && matches!(self.motion, GridMotion::Moving(_)) {
            if demand_sync == DemandSync::CurrentRender {
                self.resolve_key_destination_depth();
            }
            return;
        }
        let settled = self.settled_held();
        if steps.is_empty()
            && wanted_depth == self.target_depth()
            && matches!(self.motion, GridMotion::Settled)
            && settled.needs_motion_from(&self.held)
        {
            steps.push(self.slots.clone());
        } else if steps.is_empty()
            && wanted_depth == self.target_depth()
            && matches!(self.motion, GridMotion::Settled)
        {
            if self.held != settled {
                self.layout_admission.take();
            }
            self.held = settled;
        }
        self.queue_with_depth(steps, wanted_depth, DestinationDepthStatus::Resolved);
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
            .filter(|slot| matches!(slot, Slot::Empty(_)))
            .count();
        let mut count = spare;
        let mut retained = Vec::new();
        let all_fit = self.fits(arrangement.len() + TABLE_CELL, growth);
        for slot in arrangement {
            if let Slot::Group(id) = slot
                && (all_fit || self.fits(count + TABLE_CELL + 1, growth))
            {
                retained.push(id);
                count += 1;
            }
        }
        live.into_iter()
            .filter(|id| {
                if retained.contains(&id) {
                    return true;
                }
                if arrangement
                    .iter()
                    .any(|slot| matches!(slot, Slot::Group(held) if held == id))
                {
                    return false;
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
        if self.open_waiting_commands_in_current_motion() {
            return;
        }
        let mut arrangement = self.target();
        if !self.fits(arrangement.len() + TABLE_CELL + 1, growth) {
            return;
        }
        arrangement.push(Slot::Empty(self.next_slot));
        self.next_slot = self.next_slot.saturating_add(1);
        self.queue(vec![arrangement]);
    }

    /// A requested opening serves commands already waiting for their cells.
    fn open_waiting_commands_in_current_motion(&mut self) -> bool {
        if !matches!(self.motion, GridMotion::Moving(_)) || self.pending.is_empty() {
            return false;
        }
        let target = self.target();
        if target.len() <= self.slots.len() {
            return false;
        }
        let mut preceding = &self.slots;
        for step in &self.pending {
            if !preceding.iter().all(|slot| step.slots.contains(slot))
                || step
                    .slots
                    .iter()
                    .any(|slot| !preceding.contains(slot) && matches!(slot, Slot::Empty(_)))
            {
                return false;
            }
            preceding = &step.slots;
        }
        let progress = self.motion_heights.progress;
        let drawn = self.drawing_at(self.area, self.growth, progress);
        self.pending.clear();
        self.complete_arrangement(target, self.depth);
        self.resolve_key_destination_depth();
        if !self.motion_heights.piece_heights.is_empty() {
            let next = self.drawing_at_with_height_snap(
                self.area,
                self.growth,
                progress,
                &HeightSnap::Off,
            );
            self.motion_heights.piece_heights = next
                .pieces
                .iter()
                .map(|piece| {
                    let placement = &piece.placement;
                    drawn
                        .pieces
                        .iter()
                        .map(|piece| &piece.placement)
                        .find(|previous| {
                            previous.content == placement.content
                                && previous.frame.clip().x == placement.frame.clip().x
                        })
                        .map_or(1, |previous| previous.frame.clip().height)
                })
                .collect();
        }
        true
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
        self.queue_with_depth(
            steps,
            wanted,
            DestinationDepthStatus::AwaitingMeasuredWidths,
        );
    }

    /// Queue arrangements with the summary depth each one ends at.
    ///
    /// A slot move and its depth change share one step. With no slot move,
    /// each depth unit remains its own step.
    fn queue_with_depth(
        &mut self,
        steps: Vec<Vec<Slot<Id>>>,
        wanted: usize,
        final_depth_status: DestinationDepthStatus,
    ) {
        let start_slots = self.target();
        let final_slots = steps.last().cloned().unwrap_or_else(|| self.target());
        let headed = self.target_depth();
        let mut depth = headed;
        let mut moves = Vec::new();
        if steps.is_empty() {
            while depth != wanted {
                if depth > wanted {
                    depth -= 1;
                } else {
                    depth += 1;
                }
                moves.push((start_slots.clone(), depth, DestinationDepthStatus::Resolved));
            }
        } else {
            let step_count = steps.len();
            for (index, slots) in steps.into_iter().enumerate() {
                let final_step = index + 1 == step_count;
                depth = if final_step {
                    wanted
                } else {
                    summary_depth(
                        self.area,
                        &cell_wants(&self.demands, &slots, &self.settings),
                        self.growth,
                        &self.settings,
                    )
                };
                let depth_status = if final_step {
                    final_depth_status
                } else {
                    DestinationDepthStatus::Resolved
                };
                moves.push((slots, depth, depth_status));
            }
        }
        moves.retain(|(slots, depth, _)| {
            queued_cells_keep_endpoint_columns(
                &QueuedArrangement {
                    slots:         &start_slots,
                    summary_depth: headed,
                },
                &QueuedArrangement {
                    slots:         &final_slots,
                    summary_depth: wanted,
                },
                &QueuedArrangement {
                    slots,
                    summary_depth: *depth,
                },
                self.growth,
                self.area,
                &self.settings,
            )
        });
        self.queue_steps(moves);
    }

    /// Give the active and queued destinations their current rendered demands.
    fn resolve_key_destination_depth(&mut self) {
        let wanted = summary_depth(
            self.area,
            &cell_wants(&self.demands, &self.slots, &self.settings),
            self.growth,
            &self.settings,
        );
        let GridMotion::Moving(transition) = &mut self.motion else {
            return;
        };
        transition.depth_status = DestinationDepthStatus::Resolved;
        if self.depth != wanted {
            self.layout_admission.take();
        }
        self.depth = wanted;
        let held = self.settled_held();
        if self.held != held {
            self.layout_admission.take();
        }
        self.held = held;
        for step in &mut self.pending {
            let depth = summary_depth(
                self.area,
                &cell_wants(&self.demands, &step.slots, &self.settings),
                self.growth,
                &self.settings,
            );
            if step.depth != depth {
                self.layout_admission.take();
                step.depth = depth;
            }
        }
    }

    /// Fold measured empty-cell handovers into the motion already drawing them.
    fn fold_rendered_claims(&mut self) {
        if !matches!(self.motion, GridMotion::Moving(_)) {
            return;
        }
        let mut preceding = &self.slots;
        let mut claimed_steps = Vec::new();
        let mut handovers = Vec::new();
        for (index, step) in self.pending.iter().enumerate() {
            let only_claims = preceding.len() == step.slots.len()
                && preceding
                    .iter()
                    .zip(&step.slots)
                    .all(|(before, after)| before == after || claims(before, after));
            if only_claims {
                let start = handovers.len();
                for (before, after) in preceding.iter().zip(&step.slots) {
                    if let (Slot::Empty(_), Slot::Group(id)) = (before, after) {
                        handovers.push((before.clone(), id.clone()));
                    }
                }
                if start < handovers.len() {
                    claimed_steps.push(index);
                }
            }
            preceding = &step.slots;
        }
        for (empty, id) in handovers {
            self.claim_moving_empty(&empty, &id);
        }
        let mut index = 0;
        self.pending.retain(|_| {
            let keep = !claimed_steps.contains(&index);
            index += 1;
            keep
        });
    }

    /// Replace the same empty identity wherever a running motion carries it.
    fn claim_moving_empty(&mut self, empty: &Slot<Id>, id: &Id) -> bool {
        let GridMotion::Moving(transition) = &mut self.motion else {
            return false;
        };
        self.layout_admission.take();
        let claim = |slots: &mut [Slot<Id>]| {
            for slot in slots {
                if slot == empty {
                    *slot = Slot::Group(id.clone());
                }
            }
        };
        claim(&mut transition.from);
        claim(&mut self.slots);
        for step in &mut self.pending {
            claim(&mut step.slots);
        }
        if self.focus == Focus::Cell(empty.clone()) {
            self.focus = Focus::Cell(Slot::Group(id.clone()));
        }
        true
    }

    /// Queue arrangements paired with their summary depth.
    fn queue_steps(&mut self, steps: Vec<(Vec<Slot<Id>>, usize, DestinationDepthStatus)>) {
        if steps.is_empty() {
            return;
        }
        self.layout_admission.take();
        let millis = u64::try_from(steps.len())
            .ok()
            .and_then(|count| self.settings.animation_millis.checked_div(count))
            .unwrap_or(self.settings.animation_millis)
            .max(self.settings.min_step_millis);
        self.pending
            .extend(steps.into_iter().map(|(slots, depth, depth_status)| Step {
                slots,
                depth,
                millis,
                depth_status,
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
        self.layout_admission.take();
        let drawn_held = self.drawn_held();
        let Some(step) = self.pending.pop_front() else {
            self.motion_heights = MotionHeightPath::default();
            self.motion = GridMotion::Settled;
            self.held = drawn_held;
            self.last_settled_geometry = LastSettledCellGeometry::NoCellsFrame;
            return;
        };
        let was_settled = matches!(self.motion, GridMotion::Settled);
        let last_settled_geometry = std::mem::take(&mut self.last_settled_geometry);
        let source = match (was_settled, last_settled_geometry) {
            (true, LastSettledCellGeometry::Drawn { area, growth, grid })
                if area == self.area
                    && growth == self.growth
                    && grid.is_current_endpoint(self.area, self.growth, &self.settings) =>
            {
                grid
            },
            _ => self.resolved_grid(self.area, &drawn_held, self.growth),
        };
        let source = self.last_drawn_motion_source(source, &step.slots);
        self.motion_heights = MotionHeightPath::default();
        let previous = self.complete_arrangement(step.slots, step.depth);
        self.motion = GridMotion::Moving(Transition {
            from:         previous,
            held:         drawn_held,
            source:       Box::new(source),
            started:      Instant::now(),
            millis:       step.millis,
            depth_status: step.depth_status,
        });
    }

    /// Install an arrival and settle its identity-dependent state against the slots it left.
    fn complete_arrangement(&mut self, slots: Vec<Slot<Id>>, depth: usize) -> Vec<Slot<Id>> {
        self.layout_admission.take();
        self.retargeted_destination = RetargetedMotionDestination::Unconstrained;
        let previous = std::mem::replace(&mut self.slots, slots);
        self.depth = depth;
        self.settle_focus(&previous);
        self.held = self.settled_held();
        previous
    }

    /// Keep the drawn heights when a queued key follows a column's motion.
    fn last_drawn_motion_source(&self, mut source: Grid, next_slots: &[Slot<Id>]) -> Grid {
        if self.motion_heights.piece_heights.is_empty()
            || self.motion_heights.area != self.area
            || self.motion_heights.growth != self.growth
        {
            return source;
        }
        let drawing = self.drawing_at(self.area, self.growth, self.motion_heights.progress);
        if drawing.column_bands != source.columns {
            return source;
        }
        let contents = cells(&self.slots);
        let mut panes = Vec::with_capacity(contents.len());
        for (content, pane) in &contents {
            let mut pieces = drawing
                .pieces
                .iter()
                .filter(|piece| &piece.placement.content == content);
            let Some(piece) = pieces.next() else {
                return source;
            };
            let frame = piece.placement.frame;
            let survives = pane
                .checked_sub(TABLE_CELL + 1)
                .and_then(|index| self.slots.get(index))
                .is_none_or(|slot| next_slots.contains(slot));
            if pieces.next().is_some()
                || frame.shift() != 0
                || (survives && frame.clip().height < source.rules.min_tile_height)
                || source.cell(*pane).is_none_or(|endpoint| {
                    frame.clip().x != endpoint.x || frame.clip().width != endpoint.width
                })
            {
                return source;
            }
            panes.push(ResolvedPane {
                pane: *pane,
                area: frame.clip(),
            });
        }
        if drawing.pieces.iter().any(|piece| {
            !contents
                .iter()
                .any(|(content, _)| content == &piece.placement.content)
                && draw::has_visible_body_row(piece.placement.frame)
        }) {
            return source;
        }
        source.resolved = ResolvedPaneLayout::new(panes);
        source
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
            slots:        self.slots.clone(),
            depth:        self.depth,
            millis:       self.settings.focus_animation_millis,
            depth_status: DestinationDepthStatus::Resolved,
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
        self.layout_admission.take();
        while !self.pending.is_empty() {
            self.advance();
        }
        self.motion = GridMotion::Settled;
        self.last_settled_geometry = LastSettledCellGeometry::NoCellsFrame;
        self.motion_heights = MotionHeightPath::default();
    }

    /// Make a multi-step test transition run directly between its endpoints.
    #[cfg(test)]
    pub(super) const fn collapse_queued_changes_for_test(&mut self) {
        self.settings.max_pending_steps = 1;
    }

    /// Complete only the active motion so tests can inspect each queued arrival.
    #[cfg(test)]
    pub(super) fn finish_motion_for_test(&mut self) { self.advance(); }

    /// Read a motion's clock without sampling elapsed time.
    #[cfg(test)]
    pub(super) const fn motion_timing_for_test(&self) -> MotionTiming {
        match &self.motion {
            GridMotion::Settled => MotionTiming::Settled,
            GridMotion::Moving(transition) => MotionTiming::Active {
                started: transition.started,
                millis:  transition.millis,
            },
        }
    }

    /// Exact framed summary demand at the active destination.
    #[cfg(test)]
    pub(super) fn destination_summary_demand_for_test(&self) -> u16 {
        self.held
            .rows
            .first()
            .copied()
            .unwrap_or_default()
            .exact_framed()
    }

    /// Retain a destination measured by the current render fixture.
    #[cfg(test)]
    pub(super) fn retain_measured_destination_for_test(&mut self) {
        let grid = self.resolved_grid(self.area, &self.drawn_held(), self.growth);
        let contents = cells(&self.slots);
        let row_use = grid
            .resolved
            .panes
            .iter()
            .map(|pane| {
                let content = &contents[pane.pane.saturating_sub(TABLE_CELL)].0;
                let area = draw::content_area(frame_inner(pane.area));
                DestinationRowUse {
                    drawn: frame_painted_rows(&self.frame_row_usage, content, area),
                    kept:  self.frame_rows_kept(content, area),
                }
            })
            .collect();
        self.retargeted_destination = RetargetedMotionDestination::Filled {
            area: grid.area,
            growth: self.growth,
            rows: cell_wants(&self.demands, &self.slots, &self.settings),
            columns: grid.columns,
            span: grid.span,
            panes: grid.resolved.panes,
            row_use,
        };
    }

    /// Whether a filled destination remains retained for this motion.
    #[cfg(test)]
    pub(super) const fn has_retargeted_destination_for_test(&self) -> bool {
        matches!(
            self.retargeted_destination,
            RetargetedMotionDestination::Filled { .. }
        )
    }

    /// Record a settled one-column fixture's shared spans.
    #[cfg(test)]
    pub(super) fn record_settled_spans_for_test(&mut self, spans: &[u16]) {
        let mut grid = self.resolved_grid(self.area, &self.drawn_held(), self.growth);
        let mut top = self.area.top();
        let panes = spans
            .iter()
            .enumerate()
            .map(|(index, &span)| {
                let area = Rect {
                    y: top,
                    height: span.saturating_add(1),
                    ..self.area
                };
                top = top.saturating_add(span);
                ResolvedPane {
                    pane: index.saturating_add(TABLE_CELL),
                    area,
                }
            })
            .collect();
        assert_eq!(top, self.area.bottom().saturating_sub(1));
        grid.resolved = ResolvedPaneLayout::new(panes);
        self.last_settled_geometry = LastSettledCellGeometry::Drawn {
            area: self.area,
            growth: self.growth,
            grid,
        };
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
        let lanes = self
            .resolved_grid(self.area, &self.drawn_held(), growth)
            .lanes();
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
    pub fn focused_cell(&self) -> FocusLocation {
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
        let grid = self.resolved_grid(self.area, &self.drawn_held(), self.growth);
        grid.resolved
            .panes
            .iter()
            .find(|resolved| resolved.area.contains(pos))
            .map(|resolved| resolved.pane)
    }

    /// Whether every cell of a `count`-cell grid would be big enough to
    /// draw in the rect the last frame used.
    fn fits(&self, count: usize, growth: TileGrowth) -> bool {
        if !fits(self.area, count, growth, &self.settings) {
            return false;
        }
        if self.settings.summary_row_allocation == SummaryRowAllocation::Shared {
            return true;
        }
        let ColumnPlan::Columns(plan) = column_layout(count, 1, growth, self.area, &self.settings)
        else {
            return false;
        };
        let mut rows = vec![self.settings.demanded_rows(0); count];
        rows[0] = self.settings.demanded_rows(self.demands.summary);
        let held = HeldCellLayout {
            rows,
            focused: FocusLocation::Departing,
            summary_span: if growth.widen_summary {
                summary_span(
                    self.area,
                    plan.columns.widths.len(),
                    self.demands.summary_width,
                )
            } else {
                1
            },
            summary_depth: 1,
        };
        matches!(
            plan.for_summary(self.area, &held, &self.settings),
            ColumnPlan::Columns(_)
        )
    }

    /// Whether every arrangement that may still be drawn fits in `area`.
    pub(super) fn holds_in(&self, area: Rect, growth: TileGrowth) -> bool {
        let admission = self
            .layout_admission
            .get_or_init(|| self.check_admission(area, growth));
        if admission.area == area
            && admission.rules == EndpointLayoutRules::new(growth, &self.settings)
        {
            admission.fits
        } else {
            self.check_admission(area, growth).fits
        }
    }

    /// Stamp a changed arrangement once, preserving concurrent read-only access.
    fn check_admission(&self, area: Rect, growth: TileGrowth) -> CheckedLayoutAdmission {
        #[cfg(test)]
        self.admission_checks.fetch_add(1, AtomicOrdering::Relaxed);
        CheckedLayoutAdmission {
            area,
            rules: EndpointLayoutRules::new(growth, &self.settings),
            fits: self.check_arrangements(area, growth),
        }
    }

    /// Validate changed arrangements, borrowing held row demands when available.
    fn check_arrangements(&self, area: Rect, growth: TileGrowth) -> bool {
        let arrangement_fits = |slots: &[Slot<Id>], depth: usize, held: &HeldCellLayout| {
            let ColumnPlan::Columns(plan) = column_layout(
                slots.len() + TABLE_CELL,
                depth,
                growth,
                area,
                &self.settings,
            ) else {
                return false;
            };
            let measured;
            let rows = if held.rows.len() == slots.len() + TABLE_CELL {
                &held.rows
            } else {
                measured = cell_wants(&self.demands, slots, &self.settings);
                &measured
            };
            let span = if growth.widen_summary {
                summary_span(area, plan.columns.widths.len(), self.demands.summary_width)
                    .max(held.summary_span)
            } else {
                1
            };
            matches!(
                plan.for_summary_rows(area, rows, span, &self.settings),
                ColumnPlan::Columns(_)
            )
        };
        arrangement_fits(&self.slots, self.depth, &self.held)
            && match &self.motion {
                GridMotion::Settled => true,
                GridMotion::Moving(transition) => {
                    transition
                        .source
                        .is_current_endpoint(area, growth, &self.settings)
                        && arrangement_fits(
                            &transition.from,
                            transition.held.summary_depth,
                            &transition.held,
                        )
                },
            }
            && self
                .pending
                .iter()
                .all(|step| arrangement_fits(&step.slots, step.depth, &HeldCellLayout::default()))
    }

    /// The one display state shared by drawing, hit-testing and focus movement.
    pub(super) fn display(&self) -> GridDisplay {
        if self.holds_in(self.area, self.growth) {
            self.held_display
        } else {
            GridDisplay::SummaryAlone
        }
    }

    /// How far through the current transition the grid is, on the
    /// [`PROGRESS_SCALE`] scale. A settled grid is fully through.
    pub(super) fn progress(&self) -> u32 {
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

    /// Summary clip rectangles painted by the most recent `draw_tile_grid` call.
    #[must_use]
    pub fn painted_summary_frames(&self) -> &[Rect] { &self.painted_summary_frames }

    pub(super) fn record_painted_summary_frames(&mut self, frames: Vec<Rect>) {
        self.painted_summary_frames = frames;
    }

    /// Retain exactly the pieces that the grid painted, in paint order.
    pub(super) fn record_painted_cells(&mut self, pieces: &[TilePiece<Id>]) {
        self.painted_cells = PaintedCells::Drawn(
            pieces
                .iter()
                .filter(|piece| draw::has_visible_body_row(piece.placement.frame))
                .map(|piece| PaintedCell {
                    identity: piece.identity.clone(),
                    frame:    piece.placement.frame,
                })
                .collect(),
        );
    }

    /// Resolve the topmost painted piece before letting input change focus.
    pub(super) fn cell_hit(&self, pos: Position) -> CellHit<Id> {
        let PaintedCells::Drawn(pieces) = &self.painted_cells else {
            return self
                .cell_at(pos)
                .and_then(|index| {
                    let grid = self.resolved_grid(self.area, &self.drawn_held(), self.growth);
                    let frame = PaneFrame::new(if self.display() == GridDisplay::SummaryAlone {
                        self.area
                    } else {
                        grid.cell(index)?
                    });
                    let content = cells(&self.slots)
                        .into_iter()
                        .find_map(|(content, cell)| (cell == index).then_some(content))?;
                    Some(CellHit::Current {
                        index,
                        content,
                        frame,
                    })
                })
                .unwrap_or(CellHit::Missed);
        };
        let Some(piece) = pieces
            .iter()
            .rev()
            .find(|piece| piece.frame.clip().contains(pos))
        else {
            return CellHit::Missed;
        };
        let index = match &piece.identity {
            CellIdentity::Summary => TABLE_CELL,
            CellIdentity::Slot(slot) => {
                let Some(index) = cell_of(&self.slots, slot) else {
                    return CellHit::Retired;
                };
                index
            },
        };
        let content = match &piece.identity {
            CellIdentity::Summary => TileContent::Summary,
            CellIdentity::Slot(slot) => content_of(slot, index),
        };
        CellHit::Current {
            index,
            content,
            frame: piece.frame,
        }
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
                identity:    CellIdentity::Summary,
                placement:   TilePlacement {
                    content: TileContent::Summary,
                    frame:   PaneFrame::new(area).with_focus(true),
                },
                name:        PieceName::Shown,
                height_snap: PieceHeightSnap::Settled,
            }],
            column_bands: Vec::new(),
        }
    }

    /// The grid's pieces after its current arrangement has settled.
    fn settled_drawing(&self, settled: &Grid) -> TileDrawing<Id> {
        let focused = self.focused_cell();
        let pieces = cells(&self.slots)
            .into_iter()
            .filter_map(|(content, index)| {
                Some(TilePiece {
                    identity:    if index == TABLE_CELL {
                        CellIdentity::Summary
                    } else {
                        CellIdentity::Slot(self.slots[index - TABLE_CELL - 1].clone())
                    },
                    placement:   TilePlacement {
                        content,
                        frame: PaneFrame::new(settled.cell(index)?)
                            .with_focus(focused == FocusLocation::Cell(index)),
                    },
                    name:        PieceName::Shown,
                    height_snap: PieceHeightSnap::Settled,
                })
            })
            .collect();
        TileDrawing {
            pieces,
            column_bands: settled.columns.clone(),
        }
    }

    /// Both sides of each change to a fixed provider's integer geometry inputs.
    #[cfg(test)]
    pub(in crate::tiles) fn distinct_motion_steps_for_test(
        &self,
        area: Rect,
        growth: TileGrowth,
    ) -> Vec<u32> {
        let destination = self.resolved_grid(area, &self.drawn_held(), growth);
        let mut previous = None;
        let mut steps = vec![0, PROGRESS_SCALE];
        for raw in 0..=PROGRESS_SCALE {
            let drawing = self.drawing_from_destination(area, raw, &HeightSnap::Off, &destination);
            let geometry = drawing
                .pieces
                .iter()
                .map(|piece| {
                    let placement = &piece.placement;
                    (
                        placement.content.clone(),
                        placement.frame.rect(),
                        placement.frame.clip(),
                        placement.frame.shift(),
                    )
                })
                .collect::<Vec<_>>();
            if previous.as_ref() != Some(&geometry) {
                steps.extend([raw.saturating_sub(1), raw]);
                previous = Some(geometry);
            }
        }
        steps.sort_unstable();
        steps.dedup();
        steps
    }

    /// Every piece and column frame at an exact point in the transition.
    pub(super) fn drawing_at(&self, area: Rect, growth: TileGrowth, raw: u32) -> TileDrawing<Id> {
        let height_snap = if raw >= PROGRESS_SCALE || self.motion_heights.piece_heights.is_empty() {
            HeightSnap::Off
        } else if raw == self.motion_heights.progress {
            HeightSnap::Frame {
                row_usage: &self.frame_row_usage,
                path:      PieceHeightPath::DrawSelected(&self.motion_heights.piece_heights),
            }
        } else {
            HeightSnap::Frame {
                row_usage: &self.frame_row_usage,
                path:      PieceHeightPath::AdvanceFrom(&self.motion_heights.piece_heights),
            }
        };
        self.drawing_at_with_height_snap(area, growth, raw, &height_snap)
    }

    /// Draw one exact motion point with or without its content-aware height pass.
    fn drawing_at_with_height_snap(
        &self,
        area: Rect,
        growth: TileGrowth,
        raw: u32,
        height_snap: &HeightSnap<'_, Id>,
    ) -> TileDrawing<Id> {
        if self.held_display == GridDisplay::SummaryAlone || !self.holds_in(area, growth) {
            return Self::summary_alone_drawing(area);
        }
        let settled = self.resolved_grid(area, &self.drawn_held(), growth);
        self.drawing_from_destination(area, raw, height_snap, &settled)
    }

    /// Use the destination already resolved with this measurement pass's rows.
    fn drawing_from_destination(
        &self,
        area: Rect,
        raw: u32,
        height_snap: &HeightSnap<'_, Id>,
        destination: &Grid,
    ) -> TileDrawing<Id> {
        let GridMotion::Moving(transition) = &self.motion else {
            return self.settled_drawing(destination);
        };
        let before = transition.source.as_ref();
        if !before.is_current_endpoint(area, self.growth, &self.settings) {
            return self.settled_drawing(destination);
        }

        let progress = eased(raw);
        let column_bands = column_bands(before, destination, progress);
        let turns = Turns::of(before, destination);
        let mut pieces = Vec::new();
        // The summary keeps cell one throughout, but the grid around it
        // resizes, so it still has somewhere to travel.
        moving_cell(
            before,
            destination,
            CellTransition::PresentAtBothEnds {
                before_cell: TABLE_CELL,
                after_cell:  TABLE_CELL,
            },
            eased(turns.summary(raw)),
            CellAppearance {
                identity: CellIdentity::Summary,
                content:  TileContent::Summary,
                focused:  self.focus == Focus::Summary,
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
            let piece_progress = if turns.covers(before, destination, transition) {
                eased(turns.covered(raw))
            } else {
                progress
            };
            moving_cell(
                before,
                destination,
                transition,
                piece_progress,
                CellAppearance {
                    identity: CellIdentity::Slot(slot.clone()),
                    content,
                    focused: self.focus == Focus::Cell(slot),
                },
                &mut pieces,
            );
        }
        let pieces = place_band_pieces(before, destination, &column_bands, pieces, height_snap);
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
        self.set_layout(self.area, growth);
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
    /// Floor and allocation rules under which this endpoint was checked.
    rules:    EndpointLayoutRules,
}

/// Geometry rules that must still hold before a cached endpoint can be reused.
#[derive(Clone, Copy, Eq, PartialEq)]
struct EndpointLayoutRules {
    growth:             TileGrowth,
    min_tile_width:     u16,
    min_tile_height:    u16,
    width_floor:        TileWidthFloor,
    summary_allocation: SummaryRowAllocation,
}

/// Admission of the held, moving and queued arrangements under one rule stamp.
struct CheckedLayoutAdmission {
    area:  Rect,
    rules: EndpointLayoutRules,
    fits:  bool,
}

impl EndpointLayoutRules {
    const fn new(growth: TileGrowth, settings: &TileSettings) -> Self {
        Self {
            growth,
            min_tile_width: settings.min_tile_width,
            min_tile_height: settings.min_tile_height,
            width_floor: settings.tile_width_floor,
            summary_allocation: settings.summary_row_allocation,
        }
    }
}

/// Choose retained content for donors and painted content for receiving steps.
#[derive(Clone, Copy)]
enum RowUsePurpose {
    RetainContent,
    PaintContent,
}

/// Geometry needed to turn one proposed share into its content area.
struct RowProbeLayout<'a> {
    area:         Rect,
    band:         Rect,
    first_cell:   usize,
    cells:        usize,
    opened:       &'a [Rect],
    widths:       &'a [usize],
    summary_span: usize,
    settings:     &'a TileSettings,
}

impl RowProbeLayout<'_> {
    /// The content area a row would receive at this disjoint height.
    fn content_area(&self, row: usize, rows: u16) -> Rect {
        let last = row.saturating_add(1) == self.cells;
        let y = if last {
            self.band.bottom().saturating_sub(rows)
        } else {
            self.band.y
        };
        let mut cell = Rect {
            x: self.band.x,
            y,
            width: self.band.width,
            height: rows,
        };
        if self.first_cell.saturating_add(row) == TABLE_CELL {
            let reached = match self.settings.summary_row_allocation {
                SummaryRowAllocation::Shared => reach(
                    self.opened,
                    self.widths,
                    cell,
                    self.summary_span,
                    self.settings,
                ),
                SummaryRowAllocation::BeforeTiles => self.summary_span,
            };
            cell = widened(cell, self.opened.get(reached.saturating_sub(1)).copied());
        }
        draw::content_area(frame_inner(share_borders(cell, self.area)))
    }
}

/// A checked arrangement, or the floor that prevents it from being drawn.
enum ColumnPlan {
    Columns(RowAdmissibleColumnLayout),
    InsufficientWidth,
    InsufficientRows,
}

/// A proposed distribution of logical positions, before its row budget is checked.
struct ColumnLayout {
    widths:        Vec<usize>,
    summary_depth: usize,
    summary_span:  usize,
}

/// Every physical column can hold its residents at the configured height floor.
struct RowAdmissibleColumnLayout {
    columns: ColumnLayout,
}

impl ColumnLayout {
    /// Count the summary once, however many logical positions it occupies.
    fn residents(&self, column: usize) -> usize {
        if column == 0 {
            self.widths[0] - self.summary_depth + 1
        } else {
            self.widths[column]
        }
    }

    /// The only boundary that admits a proposed column layout for drawing.
    fn checked(self, area: Rect, settings: &TileSettings, summary_rows: u16) -> ColumnPlan {
        let opened =
            Layout::horizontal(constraints_for_sizes(&fills(self.widths.len()))).split(area);
        if area.width == 0
            || opened.iter().any(|&column| {
                share_borders(column, area).width < settings.min_tile_width
                    && !(self.widths.len() == 1
                        && settings.tile_width_floor == TileWidthFloor::CompressSingleColumn)
            })
        {
            return ColumnPlan::InsufficientWidth;
        }
        let gap = usize::from(settings.min_tile_height - 1);
        for column in 0..self.widths.len() {
            let residents = self.residents(column);
            let (height, reservation, mates) = if column == 0 {
                // Keep held growth positions and any larger content reservation.
                // The last piece owns its closing row; other pieces share it.
                let content_gap = summary_rows.saturating_sub(u16::from(residents == 1));
                (
                    area.height,
                    usize::from(content_gap).max(self.summary_depth * gap),
                    residents - 1,
                )
            } else {
                let height = if column < self.summary_span {
                    area.height.saturating_sub(summary_rows)
                } else {
                    area.height
                };
                (height, 0, residents)
            };
            let available = usize::from(height.saturating_sub(1));
            if height == 0 || reservation > available || mates > (available - reservation) / gap {
                return ColumnPlan::InsufficientRows;
            }
        }
        ColumnPlan::Columns(RowAdmissibleColumnLayout { columns: self })
    }
}

/// Use the deepest held summary whose physical residents fit the viewport.
fn column_layout(
    count: usize,
    held_depth: usize,
    growth: TileGrowth,
    area: Rect,
    settings: &TileSettings,
) -> ColumnPlan {
    for depth in (1..=held_depth.max(1)).rev() {
        let positions = count.saturating_add(depth - 1);
        let natural = columns(positions, growth);
        if depth > 1 && natural.first().is_none_or(|&height| depth > height) {
            continue;
        }
        let Some(opened) = (1..=natural.len()).rev().find(|&candidate| {
            let columns = Layout::horizontal(constraints_for_sizes(&fills(candidate))).split(area);
            columns
                .iter()
                .all(|&column| share_borders(column, area).width >= settings.min_tile_width)
                || (candidate == 1
                    && area.width > 0
                    && settings.tile_width_floor == TileWidthFloor::CompressSingleColumn)
        }) else {
            return ColumnPlan::InsufficientWidth;
        };
        let widths = if opened == natural.len() {
            natural
        } else {
            let low = positions / opened;
            let extra = positions % opened;
            (0..opened)
                .map(|column| low + usize::from(column < extra))
                .collect()
        };
        if depth > widths[0] {
            continue;
        }
        return ColumnLayout {
            widths,
            summary_depth: depth,
            summary_span: 1,
        }
        .checked(area, settings, 0);
    }
    ColumnPlan::InsufficientRows
}

impl RowAdmissibleColumnLayout {
    /// Recheck each redistributed candidate, including bands below the summary.
    fn for_summary(self, area: Rect, held: &HeldCellLayout, settings: &TileSettings) -> ColumnPlan {
        self.for_summary_rows(area, &held.rows, held.summary_span, settings)
    }

    /// Borrow the summary reservation without copying the held row record.
    fn for_summary_rows(
        self,
        area: Rect,
        rows: &[CellRowDemand],
        summary_span: usize,
        settings: &TileSettings,
    ) -> ColumnPlan {
        let columns = self.columns;
        let initial_columns = columns.widths.len();
        if settings.summary_row_allocation != SummaryRowAllocation::BeforeTiles {
            return ColumnLayout {
                summary_span: summary_span.min(initial_columns).max(1),
                ..columns
            }
            .checked(area, settings, 0);
        }
        let positions = columns.widths.iter().sum::<usize>();
        for count in (1..=initial_columns).rev() {
            let widths = if count == initial_columns {
                columns.widths.clone()
            } else {
                let low = positions / count;
                let extra = positions % count;
                (0..count)
                    .map(|column| low + usize::from(column < extra))
                    .collect()
            };
            let depth = columns.summary_depth.min(widths[0]);
            let candidate = ColumnLayout {
                widths,
                summary_depth: depth,
                summary_span: summary_span
                    .saturating_mul(count)
                    .div_ceil(initial_columns)
                    .max(1),
            };
            let wants = &rows[..candidate.residents(0).min(rows.len())];
            let shared = shares(
                wants,
                area.height,
                ColumnHead::Summary(depth),
                ColumnFocus::Outside,
                settings.min_tile_height,
            );
            let reserved =
                summary_before_tiles(wants, area.height, settings.min_tile_height, shared);
            let summary_rows = reserved.first().map_or(0, |size| match size {
                PaneAxisSize::Fixed(rows) => *rows,
                PaneAxisSize::Fill(_) => 0,
            });
            let plan = candidate.checked(area, settings, summary_rows);
            if matches!(plan, ColumnPlan::Columns(_)) {
                return plan;
            }
        }
        ColumnPlan::InsufficientRows
    }
}

/// Content readings used to divide one grid into shared columns.
enum GridRowReadings<'a> {
    #[cfg(test)]
    Intrinsic,
    Measured(&'a mut dyn FnMut(usize, Rect, RowUsePurpose) -> u16),
}

impl GridRowReadings<'_> {
    fn column_shares(
        &mut self,
        wants: &[CellRowDemand],
        probe: &RowProbeLayout<'_>,
        head: ColumnHead,
        focused: ColumnFocus,
    ) -> Vec<PaneAxisSize> {
        let shared = match self {
            #[cfg(test)]
            Self::Intrinsic => shares(
                wants,
                probe.band.height,
                head,
                focused,
                probe.settings.min_tile_height,
            ),
            Self::Measured(rows_use) => {
                let first = probe.first_cell - TABLE_CELL;
                let mut unused_rows = |row: usize, rows: u16, purpose: RowUsePurpose| {
                    let available = probe.content_area(row, rows);
                    let kept = rows_use(first.saturating_add(row), available, purpose)
                        .min(available.height);
                    available.height.saturating_sub(kept)
                };
                shares_with_directional_row_usage(
                    wants,
                    probe.band.height,
                    head,
                    focused,
                    probe.settings.min_tile_height,
                    &mut unused_rows,
                )
            },
        };
        if probe.settings.summary_row_allocation == SummaryRowAllocation::BeforeTiles
            && matches!(head, ColumnHead::Summary(_))
            && !wants.is_empty()
        {
            summary_before_tiles(
                wants,
                probe.band.height,
                probe.settings.min_tile_height,
                shared,
            )
        } else {
            shared
        }
    }
}

impl Grid {
    /// Construction checked the bands once; unchanged rules preserve that guarantee.
    fn is_current_endpoint(&self, area: Rect, growth: TileGrowth, settings: &TileSettings) -> bool {
        self.area == area && self.rules == EndpointLayoutRules::new(growth, settings)
    }

    /// Resolve the cells `held` describes against `area`.
    ///
    /// The first column is divided before the rest, because the summary
    /// at its top settles how far it reaches: it keeps the height its
    /// own column gives it and widens over as many of the next columns
    /// as `held` asks and [`reach`] allows, each of which then divides
    /// only what is left below it.
    #[cfg(test)]
    fn new(area: Rect, held: &HeldCellLayout, growth: TileGrowth, settings: &TileSettings) -> Self {
        Self::with_row_readings(area, held, growth, settings, GridRowReadings::Intrinsic)
    }

    /// Resolve cells while moving room only in steps their contents retain.
    fn with_row_use(
        area: Rect,
        held: &HeldCellLayout,
        growth: TileGrowth,
        settings: &TileSettings,
        rows_use: &mut impl FnMut(usize, Rect, RowUsePurpose) -> u16,
    ) -> Self {
        Self::with_row_readings(
            area,
            held,
            growth,
            settings,
            GridRowReadings::Measured(rows_use),
        )
    }

    /// Resolve measured content or intrinsic demands at the same column geometry.
    fn with_row_readings(
        area: Rect,
        held: &HeldCellLayout,
        growth: TileGrowth,
        settings: &TileSettings,
        mut readings: GridRowReadings<'_>,
    ) -> Self {
        let count = held.rows.len();
        let ColumnPlan::Columns(layout) =
            column_layout(count, held.summary_depth, growth, area, settings)
        else {
            return Self::summary_only(area, growth, settings);
        };
        let ColumnPlan::Columns(layout) = layout.for_summary(area, held, settings) else {
            return Self::summary_only(area, growth, settings);
        };
        let ColumnLayout {
            widths,
            summary_depth: depth,
            summary_span: planned_span,
        } = layout.columns;
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
            let focused = column_focus(held.focused, first_cell, pieces);
            position = ends_at;
            let Some(&band) = bands.get(column) else {
                continue;
            };
            let probe = RowProbeLayout {
                area,
                band,
                first_cell,
                cells: wants.len(),
                opened: &opened,
                widths: &widths,
                summary_span: planned_span,
                settings,
            };
            let shares = readings.column_shares(wants, &probe, column_head, focused);
            for (row, &cell) in Layout::vertical(constraints_for_sizes(&shares))
                .split(band)
                .iter()
                .enumerate()
            {
                let index = first_cell + row;
                let cell = if index == TABLE_CELL {
                    span = reach(&opened, &widths, cell, planned_span, settings);
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
            rules: EndpointLayoutRules::new(growth, settings),
        }
    }

    /// No column arrangement meets the configured floors in this area.
    fn summary_only(area: Rect, growth: TileGrowth, settings: &TileSettings) -> Self {
        Self {
            area,
            widths: vec![1],
            depth: 1,
            resolved: ResolvedPaneLayout::new(vec![ResolvedPane {
                pane: TABLE_CELL,
                area,
            }]),
            columns: Vec::new(),
            span: 1,
            rules: EndpointLayoutRules::new(growth, settings),
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

/// The focus ring's row within one column.
fn column_focus(focused: FocusLocation, first_cell: usize, pieces: usize) -> ColumnFocus {
    match focused {
        FocusLocation::Cell(cell) => cell
            .checked_sub(first_cell)
            .filter(|at| *at < pieces)
            .map_or(ColumnFocus::Outside, ColumnFocus::Row),
        FocusLocation::Departing => ColumnFocus::Outside,
    }
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
    wants: &[CellRowDemand],
    height: u16,
    head: ColumnHead,
    focused: ColumnFocus,
    floor: u16,
) -> Vec<PaneAxisSize> {
    solve_column_shares(
        wants,
        height,
        head,
        focused,
        floor,
        &mut DemandOnlyColumnRows {
            exact: disjoint_demands(
                wants.iter().map(|demand| demand.exact_framed()),
                height,
                floor,
            ),
        },
    )
}

/// Keep the shared allocation unless the summary needs more rows than its share.
fn summary_before_tiles(
    wants: &[CellRowDemand],
    height: u16,
    floor: u16,
    shared: Vec<PaneAxisSize>,
) -> Vec<PaneAxisSize> {
    let exact = disjoint_demands(
        wants.iter().map(|demand| demand.exact_framed()),
        height,
        floor,
    );
    if shared
        .first()
        .is_some_and(|share| matches!(share, PaneAxisSize::Fixed(rows) if *rows >= exact[0]))
    {
        return shared;
    }
    let reserved = exact[0].min(height);
    let mut rows = vec![PaneAxisSize::Fixed(reserved)];
    if wants.len() > 1 {
        let remaining = height.saturating_sub(reserved);
        rows.extend(shares(
            &wants[1..],
            remaining,
            ColumnHead::Cell,
            ColumnFocus::Outside,
            floor,
        ));
    }
    rows
}

/// Row-use readings available to a column's shared allocation solver.
trait ColumnRowReadings {
    fn unused(&mut self, index: usize, rows: u16, purpose: RowUsePurpose) -> u16;
}

/// Intrinsic content fills each allocation only up to its exact demand.
struct DemandOnlyColumnRows {
    exact: Vec<u16>,
}

impl ColumnRowReadings for DemandOnlyColumnRows {
    fn unused(&mut self, index: usize, rows: u16, _: RowUsePurpose) -> u16 {
        rows.saturating_sub(self.exact[index])
    }
}

/// Whether a column has already asked for one content allocation.
#[derive(Clone, Copy)]
enum ColumnRowAnswer {
    Unmeasured,
    Measured(u16),
}

/// Lazily measured content use, indexed by tile, height and purpose.
struct ColumnRowUse<'m, F> {
    measure: &'m mut F,
    answers: Vec<Vec<[ColumnRowAnswer; 2]>>,
}

impl<'m, F: FnMut(usize, u16, RowUsePurpose) -> u16> ColumnRowUse<'m, F> {
    fn new(count: usize, measure: &'m mut F) -> Self {
        Self {
            measure,
            answers: vec![Vec::new(); count],
        }
    }
}

impl<F: FnMut(usize, u16, RowUsePurpose) -> u16> ColumnRowReadings for ColumnRowUse<'_, F> {
    fn unused(&mut self, index: usize, rows: u16, purpose: RowUsePurpose) -> u16 {
        let purpose_index = match purpose {
            RowUsePurpose::RetainContent => 0,
            RowUsePurpose::PaintContent => 1,
        };
        let answers = &mut self.answers[index];
        if answers.len() <= usize::from(rows) {
            answers.resize(usize::from(rows) + 1, [ColumnRowAnswer::Unmeasured; 2]);
        }
        let answer = &mut answers[usize::from(rows)][purpose_index];
        match *answer {
            ColumnRowAnswer::Measured(unused) => unused,
            ColumnRowAnswer::Unmeasured => {
                let unused = (self.measure)(index, rows, purpose);
                *answer = ColumnRowAnswer::Measured(unused);
                unused
            },
        }
    }
}

fn shares_with_directional_row_usage(
    wants: &[CellRowDemand],
    height: u16,
    head: ColumnHead,
    focused: ColumnFocus,
    floor: u16,
    unused_rows: &mut impl FnMut(usize, u16, RowUsePurpose) -> u16,
) -> Vec<PaneAxisSize> {
    let mut measured = ColumnRowUse::new(wants.len(), unused_rows);
    solve_column_shares(wants, height, head, focused, floor, &mut measured)
}

fn solve_column_shares(
    wants: &[CellRowDemand],
    height: u16,
    head: ColumnHead,
    focused: ColumnFocus,
    floor: u16,
    measured: &mut impl ColumnRowReadings,
) -> Vec<PaneAxisSize> {
    let count = wants.len();
    if count == 0 {
        return Vec::new();
    }
    let mut weights = vec![1; count];
    if let ColumnHead::Summary(depth) = head {
        weights[0] = u16::try_from(depth).unwrap_or(u16::MAX);
    }
    let base = apportion_shared_run(&weights, height);
    let want = disjoint_demands(
        wants.iter().map(|demand| demand.stepped_framed()),
        height,
        floor,
    );
    let exact = disjoint_demands(
        wants.iter().map(|demand| demand.exact_framed()),
        height,
        floor,
    );
    let mut minimum = disjoint_demands(std::iter::repeat_n(floor, count), height, floor);

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
    let given = apportion(&spare, moved);
    let mut settled: Vec<u16> = base
        .into_iter()
        .enumerate()
        .map(|(index, share)| {
            share
                .saturating_add(taken.get(index).copied().unwrap_or_default())
                .saturating_sub(given.get(index).copied().unwrap_or_default())
        })
        .collect();
    if let ColumnHead::Summary(depth) = head {
        let positions = count.saturating_add(depth.saturating_sub(1));
        let held_summary = settled[0];
        lend_to_summary(&mut settled, &exact, height, positions, &minimum);
        if settled[0] > held_summary
            && settled[0] >= exact[0]
            && measured.unused(0, settled[0], RowUsePurpose::PaintContent) == 0
        {
            minimum[0] = minimum[0].max(exact[0].saturating_sub(measured.unused(
                0,
                exact[0],
                RowUsePurpose::RetainContent,
            )));
        }
    }
    serve_directional_shortfalls(&mut settled, &exact, &minimum, measured);
    settled.into_iter().map(PaneAxisSize::Fixed).collect()
}

/// Convert framed demands into disjoint layout rows. Every piece except the
/// last receives its closing frame row from [`share_borders`].
fn disjoint_demands(framed: impl IntoIterator<Item = u16>, height: u16, floor: u16) -> Vec<u16> {
    let mut rows: Vec<u16> = framed
        .into_iter()
        .map(|asked| asked.max(floor).min(height).saturating_sub(1))
        .collect();
    if height > 0
        && let Some(last) = rows.last_mut()
    {
        *last = last.saturating_add(1);
    }
    rows
}

/// Give a nearby summary shortfall rows from its tallest eligible mate.
fn lend_to_summary(
    settled: &mut [u16],
    wants: &[u16],
    height: u16,
    positions: usize,
    floors: &[u16],
) {
    let Some((&held, &asked)) = settled.first().zip(wants.first()) else {
        return;
    };
    let short = asked.saturating_sub(held);
    let framed_slot = apportion_shared_run(&vec![1; positions], height)
        .first()
        .copied()
        .unwrap_or_default()
        .saturating_add(u16::from(height > 0 && settled.len() > 1));
    let available = settled
        .iter()
        .zip(floors)
        .skip(1)
        .map(|(rows, floor)| rows.saturating_sub(*floor))
        .fold(0, u16::saturating_add);
    if short == 0 || short > framed_slot / SUMMARY_SLOT_SHORTFALL_DIVISOR || available < short {
        return;
    }
    for _ in 0..short {
        let Some(donor) = (1..settled.len())
            .filter(|&index| settled[index] > floors[index])
            .max_by_key(|&index| {
                settled[index].saturating_add(u16::from(index + 1 < settled.len()))
            })
        else {
            return;
        };
        settled[0] = settled[0].saturating_add(1);
        settled[donor] = settled[donor].saturating_sub(1);
    }
}

/// A donor's fixed, nonempty supply of rows that preserve protected content.
#[derive(Eq, Ord, PartialEq, PartialOrd)]
struct DonorSupply {
    framed_height: u16,
    index:         usize,
    remaining:     NonZeroU16,
}

/// A recipient step that increases retained content, or preserves it and paints more.
struct RecipientContentGain {
    index:      usize,
    rows:       u16,
    protection: RowUsePurpose,
}

/// Whether a candidate allocation increases the column's content measure.
enum RecipientRowStep {
    NoContentGain,
    Gain(RecipientContentGain),
}

impl RecipientRowStep {
    fn measure(
        index: usize,
        rows: u16,
        painted: u16,
        retained: u16,
        measured: &mut impl ColumnRowReadings,
    ) -> Self {
        let next_painted =
            rows.saturating_sub(measured.unused(index, rows, RowUsePurpose::PaintContent));
        if next_painted <= painted {
            return Self::NoContentGain;
        }
        let next_retained =
            rows.saturating_sub(measured.unused(index, rows, RowUsePurpose::RetainContent));
        let protection = match next_retained.cmp(&retained) {
            Ordering::Greater => RowUsePurpose::RetainContent,
            Ordering::Equal => RowUsePurpose::PaintContent,
            Ordering::Less => return Self::NoContentGain,
        };
        Self::Gain(RecipientContentGain {
            index,
            rows,
            protection,
        })
    }
}

/// A content gain funded entirely by the plan's fixed donor supplies.
enum ColumnTransfer<'a> {
    Unfunded,
    Funded {
        gain:          RecipientContentGain,
        donor_heights: &'a [u16],
    },
}

impl ColumnTransfer<'_> {
    const fn apply(self, settled: &mut [u16]) -> bool {
        match self {
            Self::Unfunded => false,
            Self::Funded {
                gain,
                donor_heights,
            } => {
                settled.copy_from_slice(donor_heights);
                settled[gain.index] = gain.rows;
                true
            },
        }
    }
}

/// An ordered sequence of donor rows, spending only its initial supplies.
struct ColumnDonations {
    heights:   Vec<u16>,
    available: BinaryHeap<DonorSupply>,
    donated:   u16,
}

impl ColumnDonations {
    fn new(
        recipient: usize,
        settled: &[u16],
        exact: &[u16],
        minimum: &[u16],
        protection: RowUsePurpose,
        measured: &mut impl ColumnRowReadings,
    ) -> Self {
        let mut available = BinaryHeap::new();
        for index in 0..settled.len() {
            if index == recipient {
                continue;
            }
            let free = directional_spare_rows(index, settled, exact, minimum, protection, measured);
            if let Some(remaining) = NonZeroU16::new(free) {
                available.push(DonorSupply {
                    framed_height: settled[index]
                        .saturating_add(u16::from(index + 1 < settled.len())),
                    index,
                    remaining,
                });
            }
        }
        Self {
            heights: settled.to_vec(),
            available,
            donated: 0,
        }
    }

    fn transfer(&mut self, gain: RecipientContentGain) -> ColumnTransfer<'_> {
        let rows = gain.rows.saturating_sub(self.heights[gain.index]);
        while self.donated < rows {
            let Some(mut donor) = self.available.pop() else {
                return ColumnTransfer::Unfunded;
            };
            self.heights[donor.index] -= 1;
            self.donated += 1;
            if let Some(remaining) = NonZeroU16::new(donor.remaining.get() - 1) {
                donor.remaining = remaining;
                donor.framed_height -= 1;
                self.available.push(donor);
            }
        }
        ColumnTransfer::Funded {
            gain,
            donor_heights: &self.heights,
        }
    }
}

fn serve_directional_shortfalls(
    settled: &mut [u16],
    exact: &[u16],
    minimum: &[u16],
    measured: &mut impl ColumnRowReadings,
) {
    loop {
        let mut recipients: Vec<usize> = (0..settled.len())
            .filter(|&index| settled[index] < exact[index])
            .collect();
        recipients
            .sort_by_key(|&index| Reverse((exact[index].saturating_sub(settled[index]), index)));
        let mut moved = false;
        for recipient in recipients {
            let held = settled[recipient];
            let painted =
                held.saturating_sub(measured.unused(recipient, held, RowUsePurpose::PaintContent));
            let retained =
                held.saturating_sub(measured.unused(recipient, held, RowUsePurpose::RetainContent));
            let available = (0..settled.len())
                .filter(|&index| index != recipient)
                .map(|index| {
                    donation_row_limit(
                        index,
                        settled,
                        exact,
                        minimum,
                        RowUsePurpose::RetainContent,
                        measured,
                    )
                })
                .fold(0, u16::saturating_add);
            let retained_donations = OnceCell::new();
            let painted_donations = OnceCell::new();
            for step in 1..=exact[recipient].saturating_sub(held).min(available) {
                let rows = held.saturating_add(step);
                let RecipientRowStep::Gain(gain) =
                    RecipientRowStep::measure(recipient, rows, painted, retained, measured)
                else {
                    continue;
                };
                let plan = match gain.protection {
                    RowUsePurpose::RetainContent => &retained_donations,
                    RowUsePurpose::PaintContent => &painted_donations,
                };
                let mut donations = plan
                    .get_or_init(|| {
                        RefCell::new(ColumnDonations::new(
                            recipient,
                            settled,
                            exact,
                            minimum,
                            gain.protection,
                            measured,
                        ))
                    })
                    .borrow_mut();
                if donations.transfer(gain).apply(settled) {
                    moved = true;
                    break;
                }
            }
            if moved {
                break;
            }
        }
        if !moved {
            return;
        }
    }
}

/// Consecutive removable rows whose lower allocations keep protected content.
/// Paint-only gains also preserve donor retained content, so every transfer
/// increases (total retained use, total painted use) in lexicographic order.
fn directional_spare_rows(
    index: usize,
    settled: &[u16],
    exact: &[u16],
    minimum: &[u16],
    protection: RowUsePurpose,
    measured: &mut impl ColumnRowReadings,
) -> u16 {
    let rows = settled[index];
    let limit = donation_row_limit(index, settled, exact, minimum, protection, measured);
    if limit == 0 {
        return 0;
    }
    let used = rows.saturating_sub(measured.unused(index, rows, protection));
    let retained = match protection {
        RowUsePurpose::RetainContent => used,
        RowUsePurpose::PaintContent => {
            rows.saturating_sub(measured.unused(index, rows, RowUsePurpose::RetainContent))
        },
    };
    (1..=limit)
        .take_while(|&donated| {
            let lower = rows - donated;
            lower.saturating_sub(measured.unused(index, lower, protection)) == used
                && (matches!(protection, RowUsePurpose::RetainContent)
                    || lower.saturating_sub(measured.unused(
                        index,
                        lower,
                        RowUsePurpose::RetainContent,
                    )) == retained)
        })
        .last()
        .unwrap_or_default()
}

/// An upper bound on donor rows before lower allocations are measured.
fn donation_row_limit(
    index: usize,
    settled: &[u16],
    exact: &[u16],
    minimum: &[u16],
    protection: RowUsePurpose,
    measured: &mut impl ColumnRowReadings,
) -> u16 {
    let rows = settled[index];
    let used = rows.saturating_sub(measured.unused(index, rows, protection));
    rows.saturating_sub(exact[index].min(used).max(minimum[index]))
}

/// Whether the positions of a grid retain a readable minimum size.
fn fits(area: Rect, count: usize, growth: TileGrowth, settings: &TileSettings) -> bool {
    matches!(
        column_layout(count, 1, growth, area, settings),
        ColumnPlan::Columns(_)
    )
}

/// Rows the first column allocates to a summary of `depth` positions.
fn summary_share(
    wants: &[CellRowDemand],
    height: u16,
    depth: usize,
    growth: TileGrowth,
    floor: u16,
    area: Rect,
    settings: &TileSettings,
) -> u16 {
    let ColumnPlan::Columns(plan) = column_layout(wants.len(), depth, growth, area, settings)
    else {
        return 0;
    };
    let pieces = plan.columns.residents(0);
    let shares = shares(
        &wants[..pieces.min(wants.len())],
        height,
        ColumnHead::Summary(plan.columns.summary_depth),
        ColumnFocus::Outside,
        floor,
    );
    match shares.first() {
        Some(PaneAxisSize::Fixed(rows)) => {
            rows.saturating_add(u16::from(height > 0 && shares.len() > 1))
        },
        _ => 0,
    }
}

/// Fewest summary positions that hold its current demand within the fit limit.
fn summary_depth(
    area: Rect,
    wants: &[CellRowDemand],
    growth: TileGrowth,
    settings: &TileSettings,
) -> usize {
    let Some(asked) = wants.first().map(|demand| demand.stepped_framed()) else {
        return 1;
    };
    let stepped: Vec<CellRowDemand> = wants
        .iter()
        .map(|demand| CellRowDemand::uniform(demand.stepped_framed()))
        .collect();
    let mut depth = 1;
    loop {
        if summary_share(
            &stepped,
            area.height,
            depth,
            growth,
            settings.min_tile_height,
            area,
            settings,
        ) >= asked
        {
            return depth;
        }
        let next = depth + 1;
        let ColumnPlan::Columns(plan) = column_layout(wants.len(), next, growth, area, settings)
        else {
            return depth;
        };
        if next > plan.columns.summary_depth {
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

/// Disjoint layout sizes for panes whose neighbouring frames share one row.
///
/// Every pane except the last receives its lower frame row later through
/// [`share_borders`]. The last pane owns the column's closing row, so it is
/// reserved before the rest is divided and restored to the last share.
fn apportion_shared_run(weights: &[u16], total: u16) -> Vec<u16> {
    let mut shares = apportion(weights, total.saturating_sub(1));
    if total > 0
        && let Some(last) = shares.last_mut()
    {
        *last = last.saturating_add(1);
    }
    shares
}

/// The exact and stepped framed-row demands for every cell, the summary's
/// first. An empty cell asks for nothing beyond its border, which [`shares`]
/// takes as a cell with room to spare rather than one to shrink.
fn cell_wants<Id: Clone + Eq + Debug>(
    demands: &TileDemands<Id>,
    slots: &[Slot<Id>],
    settings: &TileSettings,
) -> Vec<CellRowDemand> {
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
    let turns = Turns::of(before, after);
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
            let vertical_progress = match turns {
                Turns::Widening { from, to } if (from..to).contains(&column) => {
                    first_half(progress)
                },
                Turns::Narrowing { from, to } if (from..to).contains(&column) => {
                    second_half(progress)
                },
                Turns::Together | Turns::Widening { .. } | Turns::Narrowing { .. } => progress,
            };
            let (top, bottom) = if column < after.columns.len() {
                (
                    lerp(from.top(), to.top(), vertical_progress),
                    lerp(from.bottom(), to.bottom(), vertical_progress),
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
fn place_band_pieces<Id: Clone + Eq>(
    before: &Grid,
    after: &Grid,
    bands: &[Rect],
    pieces: Vec<BandPiece<Id>>,
    height_snap: &HeightSnap<'_, Id>,
) -> Vec<TilePiece<Id>> {
    let mut rects = vec![Rect::ZERO; pieces.len()];
    let mut height_snaps = vec![PieceHeightSnap::Settled; pieces.len()];
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
        let search = BandHeightSearch {
            before_grid: before,
            after_grid: after,
            pieces: &pieces,
            in_band: &in_band,
            band,
            min_tile_height: after.rules.min_tile_height,
            height_snap,
        };
        let dividers = band_piece_dividers(&search, from_band, to_band);
        let mut before_top = from_band.top();
        let mut after_top = to_band.top();
        for (position, &index) in in_band.iter().enumerate() {
            let piece = &pieces[index];
            let divider = dividers[position];
            let bottom = dividers[position + 1];
            rects[index] = band_piece_rect(before, after, piece, band, divider, bottom);
            let last = position.saturating_add(1) == in_band.len();
            let before_bottom = if last {
                from_band.bottom().saturating_sub(1)
            } else {
                piece.motion.before_bottom(from_band, before_top)
            };
            let after_bottom = if last {
                to_band.bottom().saturating_sub(1)
            } else {
                piece.motion.after_bottom(to_band, after_top)
            };
            height_snaps[index] =
                if matches!(piece.motion, BandPieceMotion::ClosingWithColumn { .. }) {
                    PieceHeightSnap::KeepStartingHeight
                } else {
                    let start = before_bottom.saturating_sub(before_top).saturating_add(1);
                    let end = after_bottom.saturating_sub(after_top).saturating_add(1);
                    PieceHeightSnap::Search { start, end }
                };
            before_top = before_bottom;
            after_top = after_bottom;
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
        .zip(height_snaps)
        .map(|(((piece, frame), name), height_snap)| TilePiece {
            identity: piece.drawn.identity.clone(),
            placement: piece.drawn.at(frame),
            name,
            height_snap,
        })
        .collect()
}

/// The framed rect one pair of shared dividers gives `piece`.
fn band_piece_rect<Id>(
    before_grid: &Grid,
    after_grid: &Grid,
    piece: &BandPiece<Id>,
    band: Rect,
    top: u16,
    bottom: u16,
) -> Rect {
    let mut rect = Rect::new(
        band.x,
        top,
        band.width,
        bottom.saturating_sub(top).saturating_add(1),
    );
    if matches!(piece.drawn.content, TileContent::Summary)
        && (before_grid.span > 1 || after_grid.span > 1)
        && let BandPieceMotion::Resident { before, after } = piece.motion
    {
        let horizontal = lerp_rect(before, after, piece.progress);
        rect.x = horizontal.x;
        rect.width = horizontal.width;
    }
    rect
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

/// Inputs shared throughout one band's height search.
struct BandHeightSearch<'a, Id> {
    /// The arrangement from which the pieces move.
    before_grid:     &'a Grid,
    /// The arrangement toward which the pieces move.
    after_grid:      &'a Grid,
    /// Every piece across all bands.
    pieces:          &'a [BandPiece<Id>],
    /// Indices of the pieces resident in this band.
    in_band:         &'a [usize],
    /// The band's interpolated bounds.
    band:            Rect,
    /// The framed height retained by resident pieces.
    min_tile_height: u16,
    /// Row-use answers gathered for the current frame.
    height_snap:     &'a HeightSnap<'a, Id>,
}

/// One unit of a piece's endpoint height change.
struct PieceHeightChange {
    /// Piece position within its band.
    position: usize,
    /// This unit's one-based place in the piece's total change.
    step:     u16,
    /// Units this piece changes across the motion.
    total:    u16,
}

/// Order height changes proportionally while keeping each piece one-way.
fn ordered_height_changes(before: &[u16], after: &[u16], growing: bool) -> Vec<PieceHeightChange> {
    let mut changes = Vec::new();
    for (position, (&before, &after)) in before.iter().zip(after).enumerate() {
        let changes_in_direction = if growing {
            after > before
        } else {
            before > after
        };
        if !changes_in_direction {
            continue;
        }
        let total = before.abs_diff(after);
        changes.extend((1..=total).map(|step| PieceHeightChange {
            position,
            step,
            total,
        }));
    }
    changes.sort_by(|left, right| {
        (u32::from(left.step) * u32::from(right.total))
            .cmp(&(u32::from(right.step) * u32::from(left.total)))
            .then(left.position.cmp(&right.position))
    });
    changes
}

/// Shared dividers whose piece heights advance through one fixed path.
fn monotone_band_dividers<Id: Eq>(
    pieces: &[BandPiece<Id>],
    in_band: &[usize],
    before: &[u16],
    after: &[u16],
    height_snap: &HeightSnap<'_, Id>,
) -> Vec<u16> {
    let before_spans = before
        .windows(2)
        .map(|pair| pair[1].saturating_sub(pair[0]))
        .collect::<Vec<_>>();
    let after_spans = after
        .windows(2)
        .map(|pair| pair[1].saturating_sub(pair[0]))
        .collect::<Vec<_>>();
    let growing = ordered_height_changes(&before_spans, &after_spans, true);
    let shrinking = ordered_height_changes(&before_spans, &after_spans, false);
    // Keep the probe pass's candidate sizes; combine fractional rows only
    // after those probes show that the content steps have no gaps.
    let round_each_piece = matches!(height_snap, HeightSnap::Off)
        || in_band
            .iter()
            .any(|&index| height_snap.has_content_step_gap(&pieces[index].drawn.content));
    let grown = before_spans
        .iter()
        .zip(&after_spans)
        .zip(in_band)
        .map(|((&before, &after), &index)| {
            let change = after.saturating_sub(before);
            if round_each_piece {
                u32::from(lerp(0, change, pieces[index].progress)).saturating_mul(PROGRESS_SCALE)
            } else {
                u32::from(change).saturating_mul(pieces[index].progress)
            }
        })
        .fold(0_u32, u32::saturating_add);
    let shrunk = before_spans
        .iter()
        .zip(&after_spans)
        .zip(in_band)
        .map(|((&before, &after), &index)| {
            let change = before.saturating_sub(after);
            if round_each_piece {
                u32::from(lerp(0, change, pieces[index].progress)).saturating_mul(PROGRESS_SCALE)
            } else {
                u32::from(change).saturating_mul(pieces[index].progress)
            }
        })
        .fold(0_u32, u32::saturating_add);
    let transfers = usize::try_from(grown.min(shrunk) / PROGRESS_SCALE).unwrap_or(usize::MAX);
    let mut spans = before_spans;
    for change in growing.iter().take(transfers) {
        spans[change.position] = spans[change.position].saturating_add(1);
    }
    for change in shrinking.iter().take(transfers) {
        spans[change.position] = spans[change.position].saturating_sub(1);
    }
    let mut dividers = Vec::with_capacity(spans.len().saturating_add(1));
    dividers.push(before.first().copied().unwrap_or_default());
    for span in spans {
        let next = dividers
            .last()
            .copied()
            .unwrap_or_default()
            .saturating_add(span);
        dividers.push(next);
    }
    dividers
}

/// Inclusive row dividers shared by every ordered piece in one band.
fn band_piece_dividers<Id: Eq>(
    search: &BandHeightSearch<'_, Id>,
    from_band: Rect,
    to_band: Rect,
) -> Vec<u16> {
    let BandHeightSearch {
        pieces,
        in_band,
        band,
        min_tile_height,
        ..
    } = search;
    let mut before = vec![from_band.top()];
    let mut after = vec![to_band.top()];
    for &index in *in_band {
        let motion = pieces[index].motion;
        before.push(motion.before_bottom(from_band, before[before.len() - 1]));
        after.push(motion.after_bottom(to_band, after[after.len() - 1]));
    }
    before[in_band.len()] = from_band.bottom().saturating_sub(1);
    after[in_band.len()] = to_band.bottom().saturating_sub(1);

    let floor = band.bottom().saturating_sub(1);
    let same_band = from_band.top() == to_band.top()
        && from_band.bottom().saturating_sub(1) == to_band.bottom().saturating_sub(1);
    let mut dividers = if same_band {
        monotone_band_dividers(pieces, in_band, &before, &after, search.height_snap)
    } else {
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
        dividers
    };
    if let Some(last) = dividers.last_mut() {
        *last = (*last).min(floor);
    }
    let resident_gap = min_tile_height.saturating_sub(1);
    let minimum_gaps = in_band
        .iter()
        .map(|&index| {
            u16::from(matches!(
                pieces[index].motion,
                BandPieceMotion::Resident { .. }
            ))
            .saturating_mul(resident_gap)
        })
        .collect::<Vec<_>>();
    let mut latest = vec![floor; dividers.len()];
    for position in (0..in_band.len()).rev() {
        latest[position] = latest[position + 1].saturating_sub(minimum_gaps[position]);
    }
    for position in 1..in_band.len() {
        let earliest = dividers[position - 1].saturating_add(minimum_gaps[position - 1]);
        dividers[position] = dividers[position].clamp(earliest, latest[position]);
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
    let HeightSnap::Frame { row_usage, .. } = search.height_snap else {
        return dividers;
    };
    if row_usage.is_empty() {
        return dividers;
    }
    snapped_band_piece_dividers(search, &before, &after, &dividers)
}

/// One candidate for all shared dividers reached so far in a band.
#[derive(Clone)]
struct BandDividerCandidate {
    /// Inclusive divider rows from the band's top through this candidate.
    dividers:      Vec<u16>,
    /// Pieces whose content reaches their last available row.
    filled_pieces: usize,
    /// Rows whole pieces can hand to another piece in their band.
    whole_spare:   u16,
    /// Total distance from the independently interpolated dividers.
    distance:      u32,
}

impl BandDividerCandidate {
    /// The spare-row relationship represented by this partial candidate.
    const fn spare_state(&self) -> CandidateSpareState {
        match (self.filled_pieces > 0, self.whole_spare > 0) {
            (false, false) => CandidateSpareState::Neutral,
            (true, false) => CandidateSpareState::FilledAllocation,
            (false, true) => CandidateSpareState::WholeSpare,
            (true, true) => CandidateSpareState::SpareBesideFilledAllocation,
        }
    }

    /// Add one piece's semantic fit to this candidate.
    fn record(&mut self, fit: PieceCandidateFit) {
        self.filled_pieces = self.filled_pieces.saturating_add(fit.filled_count());
        self.whole_spare = self.whole_spare.saturating_add(fit.whole_spare());
    }
}

/// Whether a partial divider candidate can still give every spare row away.
#[derive(Clone, Copy, Eq, PartialEq)]
enum CandidateSpareState {
    /// No filled allocation or spare whole piece has appeared yet.
    Neutral,
    /// A filled allocation has appeared without a whole piece holding spare rows.
    FilledAllocation,
    /// A whole piece has spare rows, with no filled allocation yet.
    WholeSpare,
    /// A whole piece holds spare rows beside a filled allocation.
    SpareBesideFilledAllocation,
}

/// How a divider search ranks transferable rows.
#[derive(Clone, Copy, Eq, PartialEq)]
enum SpareRowPreference<'a> {
    /// Keep candidates that give every whole piece's spare rows to cutting mates.
    GiveToCuttingMate {
        /// Opening and closing spans selected by the nearest filled search.
        motion_dividers: &'a [u16],
    },
    /// Retain the prior nearest-candidate behavior.
    Nearest,
}

impl SpareRowPreference<'_> {
    /// Prefer fewer transferable spare rows before distance from the motion.
    const fn score(self, candidate: &BandDividerCandidate) -> (u16, u32) {
        let spare = match self {
            Self::GiveToCuttingMate { .. } if candidate.filled_pieces > 0 => candidate.whole_spare,
            Self::GiveToCuttingMate { .. } | Self::Nearest => 0,
        };
        (spare, candidate.distance)
    }

    /// State that must remain distinct while candidates share an ending divider.
    const fn selection_state(self, candidate: &BandDividerCandidate) -> CandidateSpareState {
        match self {
            Self::GiveToCuttingMate { .. } => candidate.spare_state(),
            Self::Nearest => CandidateSpareState::Neutral,
        }
    }
}

/// Whether jointly filled allocations exist along the remaining motion.
enum FilledBandPath {
    /// Every piece is filled or whole at these shared dividers.
    Filled(Vec<u16>),
    /// No jointly filled allocation is currently reachable.
    NoFilledPath,
}

/// Whether one piece can use a proposed divider pair.
#[derive(Clone, Copy, Eq, PartialEq)]
enum PieceCandidateFit {
    /// The divider pair is outside the piece's one-way geometry.
    InvalidGeometry,
    /// The measured content reaches the proposed allocation's last row.
    Filled,
    /// The whole value fits, with rows it does not retain for itself.
    Whole { spare_rows: u16 },
    /// Content fills the span or fits whole in a piece following its slide.
    SlidingAllocation,
    /// The measured content leaves a gap in the proposed allocation.
    LeavesGap,
}

impl PieceCandidateFit {
    /// One filled allocation added to the candidate's spare-row state.
    fn filled_count(self) -> usize { usize::from(matches!(self, Self::Filled)) }

    /// Spare rows held by a whole piece.
    const fn whole_spare(self) -> u16 {
        match self {
            Self::Whole { spare_rows } => spare_rows,
            Self::InvalidGeometry | Self::Filled | Self::SlidingAllocation | Self::LeavesGap => 0,
        }
    }

    /// Whether this fit meets the motion snap's content rule.
    const fn meets_required_fit(self) -> bool {
        matches!(
            self,
            Self::Filled | Self::Whole { .. } | Self::SlidingAllocation
        )
    }
}

/// Pick the nearest jointly valid set of shared dividers in one band.
fn snapped_band_piece_dividers<Id: Eq>(
    search: &BandHeightSearch<'_, Id>,
    before: &[u16],
    after: &[u16],
    interpolated: &[u16],
) -> Vec<u16> {
    let nearest = match nearest_band_piece_dividers(
        search,
        before,
        after,
        interpolated,
        SpareRowPreference::Nearest,
    ) {
        FilledBandPath::Filled(dividers) => dividers,
        FilledBandPath::NoFilledPath => {
            let mut held = vec![search.band.top()];
            for (position, &index) in search.in_band.iter().enumerate() {
                let span = search.height_snap.previous_height(index).map_or_else(
                    || before[position + 1].saturating_sub(before[position]),
                    |height| height.saturating_sub(1),
                );
                held.push(held[position].saturating_add(span));
            }
            if held.last().copied() == Some(search.band.bottom().saturating_sub(1)) {
                return held;
            }
            let mut heights = vec![0; search.pieces.len()];
            for (position, &index) in search.in_band.iter().enumerate() {
                heights[index] = interpolated[position + 1]
                    .saturating_sub(interpolated[position])
                    .saturating_add(1);
            }
            let height_snap = match search.height_snap {
                HeightSnap::Off => HeightSnap::Off,
                HeightSnap::Frame { row_usage, .. } => HeightSnap::Frame {
                    row_usage,
                    path: PieceHeightPath::AdvanceFrom(&heights),
                },
            };
            let current_band = BandHeightSearch {
                height_snap: &height_snap,
                ..*search
            };
            match nearest_band_piece_dividers(
                &current_band,
                before,
                after,
                interpolated,
                SpareRowPreference::Nearest,
            ) {
                FilledBandPath::Filled(dividers) => dividers,
                FilledBandPath::NoFilledPath => interpolated.to_vec(),
            }
        },
    };
    if !search.in_band.iter().any(|&index| {
        let progress = search.pieces[index].progress;
        progress < PROGRESS_SCALE
    }) {
        return nearest;
    }
    match nearest_band_piece_dividers(
        search,
        before,
        after,
        interpolated,
        SpareRowPreference::GiveToCuttingMate {
            motion_dividers: &nearest,
        },
    ) {
        FilledBandPath::Filled(dividers) => dividers,
        FilledBandPath::NoFilledPath => nearest,
    }
}

/// Find the nearest shared dividers within every piece's remaining path.
fn nearest_band_piece_dividers<Id: Eq>(
    search: &BandHeightSearch<'_, Id>,
    before: &[u16],
    after: &[u16],
    interpolated: &[u16],
    spare_rows: SpareRowPreference<'_>,
) -> FilledBandPath {
    let in_band = search.in_band;
    let Some(&top) = interpolated.first() else {
        return FilledBandPath::NoFilledPath;
    };
    let Some(&bottom) = interpolated.last() else {
        return FilledBandPath::NoFilledPath;
    };
    let mut candidates = vec![BandDividerCandidate {
        dividers:      vec![top],
        filled_pieces: 0,
        whole_spare:   0,
        distance:      0,
    }];
    for (position, &interpolated_divider) in
        interpolated.iter().enumerate().take(in_band.len()).skip(1)
    {
        let bounds = (0..position).try_fold((top, top), |(lower, upper), piece| {
            let (piece_lower, piece_upper) =
                piece_span_bounds(search, before, after, piece, spare_rows)?;
            Some((
                lower.saturating_add(piece_lower),
                upper.saturating_add(piece_upper),
            ))
        });
        let Some((lower, upper)) = bounds else {
            return FilledBandPath::NoFilledPath;
        };
        let lower = lower.min(bottom);
        let upper = upper.min(bottom);
        let mut next = Vec::new();
        for divider in lower..=upper {
            let mut at_divider = Vec::new();
            for candidate in &candidates {
                let fit = snapped_piece_fit(
                    search,
                    before,
                    after,
                    position - 1,
                    *candidate.dividers.last().unwrap_or(&top),
                    divider,
                    spare_rows,
                );
                if !fit.meets_required_fit() {
                    continue;
                }
                let mut candidate = candidate.clone();
                candidate.dividers.push(divider);
                candidate.record(fit);
                candidate.distance = candidate
                    .distance
                    .saturating_add(u32::from(divider.abs_diff(interpolated_divider)));
                let state = spare_rows.selection_state(&candidate);
                let replace = at_divider
                    .iter()
                    .position(|held: &BandDividerCandidate| {
                        spare_rows.selection_state(held) == state
                    })
                    .is_none_or(|held| {
                        spare_rows.score(&candidate) < spare_rows.score(&at_divider[held])
                    });
                if replace {
                    at_divider.retain(|held| spare_rows.selection_state(held) != state);
                    at_divider.push(candidate);
                }
            }
            next.extend(at_divider);
        }
        if next.is_empty() {
            return FilledBandPath::NoFilledPath;
        }
        candidates = next;
    }
    candidates
        .into_iter()
        .filter_map(|mut candidate| {
            let fit = snapped_piece_fit(
                search,
                before,
                after,
                in_band.len().saturating_sub(1),
                *candidate.dividers.last().unwrap_or(&top),
                bottom,
                spare_rows,
            );
            if !fit.meets_required_fit() {
                return None;
            }
            candidate.record(fit);
            Some(candidate)
        })
        .min_by_key(|candidate| spare_rows.score(candidate))
        .map(|mut candidate| {
            candidate.dividers.push(bottom);
            candidate.dividers
        })
        .map_or(FilledBandPath::NoFilledPath, FilledBandPath::Filled)
}

/// Framed-span interval one piece may use on its remaining path.
fn piece_span_bounds<Id: Eq>(
    search: &BandHeightSearch<'_, Id>,
    before: &[u16],
    after: &[u16],
    position: usize,
    spare_rows: SpareRowPreference<'_>,
) -> Option<(u16, u16)> {
    let &index = search.in_band.get(position)?;
    let piece = &search.pieces[index];
    let before_span = before
        .get(position + 1)?
        .saturating_sub(*before.get(position)?);
    let after_span = after
        .get(position + 1)?
        .saturating_sub(*after.get(position)?);
    let previous_span = search
        .height_snap
        .previous_height(index)
        .map_or(before_span, |height| height.saturating_sub(1));
    let mut lower = previous_span.min(after_span);
    let mut upper = previous_span.max(after_span);
    let previous_rect = band_piece_rect(
        search.before_grid,
        search.after_grid,
        piece,
        search.band,
        search.band.top(),
        search.band.top().saturating_add(previous_span),
    );
    if search.height_snap.fit(piece, previous_rect) == FrameHeightFit::LeavesGap
        && let HeightSnap::Frame { row_usage, .. } = search.height_snap
    {
        let area = draw::content_area(frame_inner(previous_rect));
        let filled = frame_painted_rows(row_usage, &piece.drawn.content, area)
            .saturating_add(previous_span.saturating_sub(area.height));
        lower = lower.min(filled);
    }
    if let Some(selected) = search.height_snap.selected_height(index) {
        let selected = selected.saturating_sub(1);
        lower = lower.max(selected);
        upper = upper.min(selected);
    }
    if let SpareRowPreference::GiveToCuttingMate { motion_dividers } = spare_rows {
        let nearest = motion_dividers
            .get(position + 1)?
            .saturating_sub(*motion_dividers.get(position)?);
        if matches!(
            piece.motion,
            BandPieceMotion::Entering { .. } | BandPieceMotion::Leaving { .. }
        ) {
            lower = lower.max(nearest);
            upper = upper.min(nearest);
        }
    }
    match piece.motion {
        BandPieceMotion::Resident { .. } => {
            lower = lower.max(search.min_tile_height.saturating_sub(1));
        },
        BandPieceMotion::ClosingWithColumn { .. } => {
            lower = lower.max(before_span);
            upper = upper.min(before_span);
        },
        BandPieceMotion::Leaving { .. } if piece.progress < PROGRESS_SCALE => {
            lower = lower.max(before_span.min(MIN_TILE_HEIGHT.saturating_sub(1)));
        },
        BandPieceMotion::Entering { .. } | BandPieceMotion::Leaving { .. } => {},
    }
    (lower <= upper).then_some((lower, upper))
}

/// How one candidate piece keeps its motion and fills its content area.
fn snapped_piece_fit<Id: Eq>(
    search: &BandHeightSearch<'_, Id>,
    before: &[u16],
    after: &[u16],
    position: usize,
    top: u16,
    bottom: u16,
    spare_rows: SpareRowPreference<'_>,
) -> PieceCandidateFit {
    let Some(&index) = search.in_band.get(position) else {
        return PieceCandidateFit::InvalidGeometry;
    };
    let piece = &search.pieces[index];
    let span = bottom.saturating_sub(top);
    let Some((lower, upper)) = piece_span_bounds(search, before, after, position, spare_rows)
    else {
        return PieceCandidateFit::InvalidGeometry;
    };
    if bottom < top || span < lower || span > upper {
        return PieceCandidateFit::InvalidGeometry;
    }
    // A collapsed piece shares one divider with its neighbour. Taking any
    // rows requires room for a body row between the two edges.
    if span > 0 && span.saturating_add(1) < MIN_TILE_HEIGHT {
        return PieceCandidateFit::InvalidGeometry;
    }
    let rect = band_piece_rect(
        search.before_grid,
        search.after_grid,
        piece,
        search.band,
        top,
        bottom,
    );
    let retained_span = if search.height_snap.previous_height(index).is_some() {
        lower.max(after[position + 1].saturating_sub(after[position]))
    } else {
        lower
    };
    match search.height_snap.fit(piece, rect) {
        FrameHeightFit::Filled | FrameHeightFit::Whole
            if !matches!(piece.motion, BandPieceMotion::Resident { .. }) =>
        {
            PieceCandidateFit::SlidingAllocation
        },
        FrameHeightFit::Filled => PieceCandidateFit::Filled,
        FrameHeightFit::Whole => PieceCandidateFit::Whole {
            spare_rows: search
                .height_snap
                .spare_rows(piece, rect)
                .min(span.saturating_sub(retained_span)),
        },
        FrameHeightFit::LeavesGap => PieceCandidateFit::LeavesGap,
    }
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
    area: Rect,
    settings: &TileSettings,
) -> Option<usize> {
    let cell = slots
        .iter()
        .position(|held| held == slot)?
        .saturating_add(TABLE_CELL + 1);
    let ColumnPlan::Columns(layout) =
        column_layout(slots.len() + TABLE_CELL, depth, growth, area, settings)
    else {
        return None;
    };
    let laid_out = cell.saturating_add(layout.columns.summary_depth.saturating_sub(1));
    position(&layout.columns.widths, laid_out).map(|(column, _)| column)
}

/// Cell order and summary depth at one step of a queued movement.
struct QueuedArrangement<'a, Id> {
    slots:         &'a [Slot<Id>],
    summary_depth: usize,
}

/// Whether every surviving cell uses one of its two endpoint columns.
fn queued_cells_keep_endpoint_columns<Id: Eq>(
    start: &QueuedArrangement<'_, Id>,
    finish: &QueuedArrangement<'_, Id>,
    candidate: &QueuedArrangement<'_, Id>,
    growth: TileGrowth,
    area: Rect,
    settings: &TileSettings,
) -> bool {
    start
        .slots
        .iter()
        .filter(|slot| finish.slots.contains(slot) && candidate.slots.contains(slot))
        .all(|slot| {
            let start_column = queued_slot_column(
                start.slots,
                start.summary_depth,
                slot,
                growth,
                area,
                settings,
            );
            let final_column = queued_slot_column(
                finish.slots,
                finish.summary_depth,
                slot,
                growth,
                area,
                settings,
            );
            let candidate_column = queued_slot_column(
                candidate.slots,
                candidate.summary_depth,
                slot,
                growth,
                area,
                settings,
            );
            candidate_column == start_column || candidate_column == final_column
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
    use std::ops::RangeInclusive;

    use ratatui::buffer::Buffer;
    use ratatui::layout::Margin;
    use ratatui::style::Color;

    use super::*;
    use crate::TileGridContents;
    use crate::tiles::constants::LARGE_CONTENT_TEST_CELLS;
    use crate::tiles::constants::LARGE_CONTENT_TEST_HEIGHT;
    use crate::tiles::constants::LARGE_CONTENT_TEST_INITIAL_ROWS;
    use crate::tiles::constants::LARGE_CONTENT_TEST_ROW_STEP;
    use crate::tiles::constants::LARGE_CONTENT_TEST_ROWS;
    use crate::tiles::constants::LARGE_CONTENT_TEST_WIDTHS;
    use crate::tiles::constants::MIN_TILE_HEIGHT;
    use crate::tiles::constants::SEVEN_CELL_MOVING_ROW_PROBE_COUNT;
    use crate::tiles::constants::SEVEN_CELL_SETTLED_DRAWN_PROBE_COUNT;
    use crate::tiles::constants::SEVEN_CELL_SETTLED_KEPT_PROBE_COUNT;
    use crate::tiles::constants::TILE_BORDER_ROWS;
    use crate::tiles::constants::TILE_DEMAND_STEP;
    use crate::tiles::constants::TILE_ROWS_READOUT_HEIGHT;

    mod motion_rules {
        use super::*;
        use crate::TileCells;
        use crate::tiles::constants::COLUMN_FOOT_TEST_AREA;
        use crate::tiles::constants::COLUMN_FOOT_TEST_MIN_HEIGHT;
        use crate::tiles::constants::COMMAND_OPENING_TEST_SPANS;
        use crate::tiles::constants::CONTENT_CHANGE_TEST_COMMAND_ROWS;
        use crate::tiles::constants::CONTENT_CHANGE_TEST_PROGRESS;
        use crate::tiles::constants::CONTENT_CHANGE_TEST_PROJECT_ROWS;
        use crate::tiles::constants::CONTENT_CHANGE_TEST_ROW;
        use crate::tiles::constants::EMPTY_DEPARTURE_TEST_SPANS;
        use crate::tiles::constants::LENDING_TEST_SUMMARY_TITLE;
        use crate::tiles::constants::RESIDENT_EMPTY_TEST_SPANS;
        use crate::tiles::constants::RESIDENT_EMPTY_TEST_START_SPANS;
        use crate::tiles::constants::SLIDING_TEST_INITIAL_ROWS;
        use crate::tiles::constants::SLIDING_TEST_INTERVALS;
        use crate::tiles::constants::SUMMARY_STEP_TEST_ENTRY_ROWS;
        use crate::tiles::constants::SUMMARY_STEP_TEST_HEADER_ROWS;

        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        enum MotionRuleKey {
            ResidentEmpty,
            CommandOpening,
            EmptyDeparture,
        }

        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        enum FixturePieceRole {
            Resident,
            EnteringClaimedCommand,
            EnteringQueuedCommand,
            EnteringEmpty,
            LeavingEmpty,
        }

        struct MotionRuleCells {
            entries:           u16,
            commands:          u32,
            departed_commands: u32,
        }

        impl TileCells<u32> for MotionRuleCells {
            fn summary_title(&self) -> &'static str { LENDING_TEST_SUMMARY_TITLE }

            fn demands(&self, _: &[(TileContent<u32>, u16)]) -> TileDemands<u32> {
                TileDemands {
                    summary:       usize::from(self.entries * CONTENT_CHANGE_TEST_PROJECT_ROWS),
                    summary_width: 0,
                    groups:        (self.departed_commands + 1..=self.commands)
                        .map(|id| TileDemand {
                            id,
                            rows: usize::from(CONTENT_CHANGE_TEST_COMMAND_ROWS),
                        })
                        .collect(),
                }
            }

            fn rows_drawn(&self, content: &TileContent<u32>, area: Rect) -> u16 {
                match content {
                    TileContent::Summary => {
                        let whole = self.entries * CONTENT_CHANGE_TEST_PROJECT_ROWS;
                        if area.height >= whole {
                            whole
                        } else {
                            let header = area.height.min(SUMMARY_STEP_TEST_HEADER_ROWS);
                            header
                                + (area.height.saturating_sub(header)
                                    / SUMMARY_STEP_TEST_ENTRY_ROWS)
                                    .min(self.entries)
                                    * SUMMARY_STEP_TEST_ENTRY_ROWS
                        }
                    },
                    TileContent::Group(id) if *id <= self.departed_commands => 0,
                    TileContent::Group(_) => area.height.min(CONTENT_CHANGE_TEST_COMMAND_ROWS),
                    TileContent::Empty(_) => 0,
                }
            }

            fn draw(&self, buffer: &mut Buffer, content: &TileContent<u32>, area: Rect, _: Color) {
                for y in area.top()..area.top() + self.rows_drawn(content, area) {
                    buffer[(area.x, y)].set_symbol(CONTENT_CHANGE_TEST_ROW);
                }
            }
        }

        fn motion_rule_frame(
            grid: &mut TileGrid<u32>,
            cells: &MotionRuleCells,
            raw: u32,
        ) -> (TileDrawing<u32>, Buffer) {
            let area = COLUMN_FOOT_TEST_AREA;
            let growth = redistribute(SLIDING_TEST_INITIAL_ROWS);
            let widths = grid.content_widths(area, growth);
            let mut demands = cells.demands(&widths);
            demands.summary += usize::from(TILE_ROWS_READOUT_HEIGHT);
            for group in &mut demands.groups {
                group.rows += usize::from(TILE_ROWS_READOUT_HEIGHT);
            }
            if cells.departed_commands > 0 {
                grid.demands = demands.clone();
                grid.resolve_key_destination_depth();
            } else {
                grid.sync_rendered_demands(&demands, growth);
            }
            grid.measure_row_steps(
                area,
                growth,
                raw,
                |content, area| cells.rows_drawn(content, area),
                |content, area| cells.rows_kept(content, area),
            );
            let drawing = grid.drawing_at(area, growth, raw);
            let mut buffer = Buffer::empty(area);
            draw::draw_placements(
                &mut buffer,
                area,
                &drawing,
                &demands,
                &widths,
                TileGridContents::Shown,
                cells,
            );
            (drawing, buffer)
        }

        fn motion_rule_fixture(key: MotionRuleKey) -> (TileGrid<u32>, MotionRuleCells) {
            let growth = redistribute(SLIDING_TEST_INITIAL_ROWS);
            let (entries, commands, slots, spans) = match key {
                MotionRuleKey::ResidentEmpty => (
                    5,
                    5,
                    vec![
                        Slot::Group(1),
                        Slot::Group(2),
                        Slot::Group(3),
                        Slot::Group(4),
                        Slot::Group(5),
                        Slot::Empty(1),
                        Slot::Empty(2),
                    ],
                    RESIDENT_EMPTY_TEST_START_SPANS.as_slice(),
                ),
                MotionRuleKey::CommandOpening => (
                    2,
                    1,
                    vec![Slot::Group(1)],
                    COMMAND_OPENING_TEST_SPANS.as_slice(),
                ),
                MotionRuleKey::EmptyDeparture => (
                    4,
                    3,
                    vec![
                        Slot::Empty(0),
                        Slot::Group(1),
                        Slot::Group(2),
                        Slot::Group(3),
                    ],
                    EMPTY_DEPARTURE_TEST_SPANS.as_slice(),
                ),
            };
            let mut cells = MotionRuleCells {
                entries,
                commands,
                departed_commands: u32::from(key == MotionRuleKey::ResidentEmpty),
            };
            let mut grid = TileGrid::new();
            grid.set_view(TileView::Cells);
            grid.set_min_tile_width(COLUMN_FOOT_TEST_AREA.width);
            grid.set_min_tile_height(COLUMN_FOOT_TEST_MIN_HEIGHT);
            grid.set_layout(COLUMN_FOOT_TEST_AREA, growth);
            grid.slots = slots;
            grid.next_slot = 3;
            motion_rule_frame(&mut grid, &cells, PROGRESS_SCALE);
            grid.settle_for_test();
            motion_rule_frame(&mut grid, &cells, PROGRESS_SCALE);
            grid.record_settled_spans_for_test(spans);
            match key {
                MotionRuleKey::ResidentEmpty => {
                    let mut target = grid.slots.clone();
                    target.push(Slot::Empty(3));
                    grid.queue_with_depth(
                        vec![target],
                        grid.depth,
                        DestinationDepthStatus::AwaitingMeasuredWidths,
                    );
                    cells.entries = 4;
                    cells.departed_commands = 1;
                    let mut heights = RESIDENT_EMPTY_TEST_SPANS
                        .iter()
                        .map(|span| span + 1)
                        .collect::<Vec<_>>();
                    heights.push(1);
                    grid.motion_heights = MotionHeightPath {
                        area: COLUMN_FOOT_TEST_AREA,
                        growth,
                        progress: CONTENT_CHANGE_TEST_PROGRESS[0],
                        piece_heights: heights,
                    };
                },
                MotionRuleKey::CommandOpening => {
                    cells.commands = 2;
                    let mut demands = cells.demands(&[]);
                    demands.summary += usize::from(TILE_ROWS_READOUT_HEIGHT);
                    for group in &mut demands.groups {
                        group.rows += usize::from(TILE_ROWS_READOUT_HEIGHT);
                    }
                    grid.sync(&demands, growth);
                },
                MotionRuleKey::EmptyDeparture => {
                    grid.focus = Focus::Cell(Slot::Empty(0));
                    grid.apply(TileAction::Remove, growth);
                },
            }
            (grid, cells)
        }

        fn fixture_role(key: MotionRuleKey, piece: &TilePiece<u32>) -> FixturePieceRole {
            match piece.height_snap {
                PieceHeightSnap::Search { start: 1, .. } => match (key, &piece.placement.content) {
                    (MotionRuleKey::CommandOpening, TileContent::Group(2)) => {
                        FixturePieceRole::EnteringClaimedCommand
                    },
                    (MotionRuleKey::CommandOpening, TileContent::Group(3)) => {
                        FixturePieceRole::EnteringQueuedCommand
                    },
                    _ => FixturePieceRole::EnteringEmpty,
                },
                PieceHeightSnap::Search { end: 1, .. } => FixturePieceRole::LeavingEmpty,
                _ => FixturePieceRole::Resident,
            }
        }

        #[derive(Clone, Copy, Eq, PartialEq)]
        enum MotionRuleAssertions {
            All,
            ResidentAllocation,
            EnteringSlide,
            LeavingSlide,
        }

        #[derive(Clone, Copy)]
        struct MotionRuleFrame<'a> {
            key:      MotionRuleKey,
            checks:   MotionRuleAssertions,
            raw:      u32,
            cells:    &'a MotionRuleCells,
            drawing:  &'a TileDrawing<u32>,
            endpoint: &'a TileDrawing<u32>,
            buffer:   &'a Buffer,
        }

        impl MotionRuleFrame<'_> {
            fn assert_content(&self, tile: &TilePiece<u32>) {
                let Self {
                    key,
                    raw,
                    cells,
                    buffer,
                    ..
                } = *self;
                let piece = &tile.placement;
                let role = fixture_role(key, tile);
                let clip = piece.frame.clip();
                let area = draw::content_area(frame_inner(clip));
                let rows = cells.rows_drawn(&piece.content, area);
                let whole = cells.rows_drawn(
                    &piece.content,
                    Rect {
                        height: u16::MAX,
                        ..area
                    },
                );
                assert!(
                    rows == area.height || rows == whole,
                    "{key:?} {raw}: {role:?} {:?} has a blank row in {clip:?}",
                    piece.content
                );
                if rows < whole && area.height > 0 {
                    assert_eq!(
                        buffer[(area.x, area.bottom() - 1)].symbol(),
                        CONTENT_CHANGE_TEST_ROW
                    );
                }
            }

            fn assert_resident_spare_rows(&self, tile: &TilePiece<u32>) {
                let Self {
                    key,
                    checks,
                    raw,
                    cells,
                    drawing,
                    endpoint,
                    ..
                } = *self;
                let piece = &tile.placement;
                let role = fixture_role(key, tile);
                let clip = piece.frame.clip();
                let area = draw::content_area(frame_inner(clip));
                let rows = cells.rows_drawn(&piece.content, area);
                if matches!(
                    checks,
                    MotionRuleAssertions::All | MotionRuleAssertions::ResidentAllocation
                ) && role == FixturePieceRole::Resident
                    && clip.height > COLUMN_FOOT_TEST_MIN_HEIGHT
                    && rows < area.height
                {
                    let end = endpoint
                        .placements()
                        .find(|end| end.content == piece.content)
                        .map_or(0, |end| end.frame.clip().height);
                    if clip.height > end
                        || raw == PROGRESS_SCALE
                        || (key == MotionRuleKey::ResidentEmpty
                            && piece.content == TileContent::Group(1))
                    {
                        for mate in drawing
                            .pieces
                            .iter()
                            .filter(|mate| {
                                fixture_role(key, mate) == FixturePieceRole::Resident
                                    && mate.placement.content != piece.content
                            })
                            .map(|mate| &mate.placement)
                        {
                            let mate_area = draw::content_area(frame_inner(mate.frame.clip()));
                            let next = cells.rows_drawn(
                                &mate.content,
                                Rect {
                                    height: mate_area.height + 1,
                                    ..mate_area
                                },
                            );
                            assert!(
                                next <= cells.rows_drawn(&mate.content, mate_area),
                                "{key:?} {raw}: {:?} holds spare beside {:?}; {:?}",
                                piece.content,
                                mate.content,
                                drawing
                                    .placements()
                                    .map(|p| (&p.content, p.frame.clip()))
                                    .collect::<Vec<_>>()
                            );
                        }
                    }
                }
            }

            fn record_heights(
                &self,
                tile: &TilePiece<u32>,
                paths: &mut Vec<(TileContent<u32>, FixturePieceRole, Vec<u16>)>,
            ) {
                let Self {
                    key, checks, raw, ..
                } = *self;
                let piece = &tile.placement;
                let role = fixture_role(key, tile);
                let clip = piece.frame.clip();
                let height = clip.height;
                if let Some((_, _, heights)) = paths
                    .iter_mut()
                    .find(|(content, held_role, _)| *content == piece.content && *held_role == role)
                {
                    let previous = heights.last().copied().unwrap_or(height);
                    let step = if previous <= 1 || key == MotionRuleKey::ResidentEmpty {
                        SUMMARY_STEP_TEST_ENTRY_ROWS
                    } else {
                        MOVING_HEIGHT_PROBE_STEP
                    };
                    match role {
                        FixturePieceRole::EnteringClaimedCommand
                        | FixturePieceRole::EnteringQueuedCommand
                        | FixturePieceRole::EnteringEmpty
                            if matches!(
                                checks,
                                MotionRuleAssertions::All | MotionRuleAssertions::EnteringSlide
                            ) =>
                        {
                            assert!(
                                height >= previous && height - previous <= step,
                                "{key:?} {raw}: {role:?} steps {previous} -> {height}"
                            );
                        },
                        FixturePieceRole::LeavingEmpty
                            if matches!(
                                checks,
                                MotionRuleAssertions::All | MotionRuleAssertions::LeavingSlide
                            ) =>
                        {
                            assert!(
                                height <= previous,
                                "{key:?} {raw}: leaving piece grows {previous} -> {height}"
                            );
                        },
                        _ => {},
                    }
                    heights.push(height);
                } else {
                    if matches!(
                        role,
                        FixturePieceRole::EnteringClaimedCommand
                            | FixturePieceRole::EnteringQueuedCommand
                            | FixturePieceRole::EnteringEmpty
                    ) && matches!(
                        checks,
                        MotionRuleAssertions::All | MotionRuleAssertions::EnteringSlide
                    ) {
                        assert!(
                            height <= MIN_TILE_HEIGHT,
                            "{key:?} {raw}: {role:?} appears at {height}"
                        );
                    }
                    paths.push((piece.content.clone(), role, vec![height]));
                }
                if matches!(
                    checks,
                    MotionRuleAssertions::All | MotionRuleAssertions::LeavingSlide
                ) && role == FixturePieceRole::LeavingEmpty
                    && raw < PROGRESS_SCALE
                {
                    assert!(
                        draw::has_visible_body_row(piece.frame),
                        "{key:?} {raw}: leaving cell vanishes before its clock ends"
                    );
                }
            }
        }

        fn assert_motion_rules(key: MotionRuleKey, checks: MotionRuleAssertions) {
            let (mut grid, mut cells) = motion_rule_fixture(key);
            let clock = grid.motion_timing_for_test();
            let mut paths = Vec::<(TileContent<u32>, FixturePieceRole, Vec<u16>)>::new();
            let mut last = Vec::new();
            for sample in 0..=SLIDING_TEST_INTERVALS {
                let start = if key == MotionRuleKey::ResidentEmpty {
                    CONTENT_CHANGE_TEST_PROGRESS[0]
                } else {
                    0
                };
                let raw = start + sample * (PROGRESS_SCALE - start) / SLIDING_TEST_INTERVALS;
                match key {
                    MotionRuleKey::ResidentEmpty if raw >= PROGRESS_SCALE / 2 => {
                        cells.entries = 3;
                        cells.departed_commands = 2;
                    },
                    MotionRuleKey::CommandOpening if raw >= PROGRESS_SCALE / 10 => {
                        cells.entries = 3;
                    },
                    _ => {},
                }
                if key == MotionRuleKey::CommandOpening && sample == SLIDING_TEST_INTERVALS / 8 {
                    cells.commands = 3;
                    let widths = grid.content_widths(COLUMN_FOOT_TEST_AREA, grid.growth);
                    let mut demands = cells.demands(&widths);
                    demands.summary += usize::from(TILE_ROWS_READOUT_HEIGHT);
                    for group in &mut demands.groups {
                        group.rows += usize::from(TILE_ROWS_READOUT_HEIGHT);
                    }
                    grid.sync(&demands, grid.growth);
                    grid.apply(TileAction::Add, grid.growth);
                }
                let (drawing, buffer) = motion_rule_frame(&mut grid, &cells, raw);
                assert_eq!(clock, grid.motion_timing_for_test(), "{key:?} {raw}");
                let endpoint = grid.drawing_at(COLUMN_FOOT_TEST_AREA, grid.growth, PROGRESS_SCALE);
                let frame = MotionRuleFrame {
                    key,
                    checks,
                    raw,
                    cells: &cells,
                    drawing: &drawing,
                    endpoint: &endpoint,
                    buffer: &buffer,
                };
                for tile in &drawing.pieces {
                    frame.assert_content(tile);
                    frame.assert_resident_spare_rows(tile);
                    frame.record_heights(tile, &mut paths);
                }
                last = drawing
                    .placements()
                    .filter(|p| draw::has_visible_body_row(p.frame))
                    .map(|p| (p.content.clone(), p.frame.clip()))
                    .collect();
            }
            for (content, role, mut heights) in paths {
                if role != FixturePieceRole::Resident
                    || !matches!(
                        checks,
                        MotionRuleAssertions::All | MotionRuleAssertions::ResidentAllocation
                    )
                {
                    continue;
                }
                heights.dedup();
                assert!(
                    heights.windows(3).all(|h| (h[0] < h[1]) == (h[1] < h[2])),
                    "{key:?}: resident {content:?} reverses {heights:?}"
                );
            }
            grid.finish_motion_for_test();
            let (settled, buffer) = motion_rule_frame(&mut grid, &cells, PROGRESS_SCALE);
            let frame = MotionRuleFrame {
                key,
                checks,
                raw: PROGRESS_SCALE,
                cells: &cells,
                drawing: &settled,
                endpoint: &settled,
                buffer: &buffer,
            };
            for tile in &settled.pieces {
                frame.assert_content(tile);
                frame.assert_resident_spare_rows(tile);
            }
            assert_eq!(
                last,
                settled
                    .placements()
                    .filter(|p| draw::has_visible_body_row(p.frame))
                    .map(|p| (p.content.clone(), p.frame.clip()))
                    .collect::<Vec<_>>()
            );
            assert!(!grid.tick(), "{key:?} has a second motion");
        }

        #[test]
        fn a_resident_empty_cell_takes_no_row_while_mates_cut() {
            assert_motion_rules(
                MotionRuleKey::ResidentEmpty,
                MotionRuleAssertions::ResidentAllocation,
            );
        }
        #[test]
        fn a_command_cell_slides_in_when_its_content_arrives_mid_slide() {
            assert_motion_rules(
                MotionRuleKey::CommandOpening,
                MotionRuleAssertions::EnteringSlide,
            );
        }
        #[test]
        fn an_empty_cell_slides_out_across_a_summary_depth_step() {
            assert_motion_rules(
                MotionRuleKey::EmptyDeparture,
                MotionRuleAssertions::LeavingSlide,
            );
        }
        #[test]
        fn every_frame_of_a_key_keeps_the_motion_rules() {
            for key in [
                MotionRuleKey::ResidentEmpty,
                MotionRuleKey::CommandOpening,
                MotionRuleKey::EmptyDeparture,
            ] {
                assert_motion_rules(key, MotionRuleAssertions::All);
            }
        }
    }

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
    fn demanded_rows(rows: usize) -> u16 {
        TileSettings::default().demanded_rows(rows).stepped_framed()
    }

    /// One demand whose exact and stepped framed rows agree.
    const fn row_demand(framed: u16) -> CellRowDemand { CellRowDemand::uniform(framed) }

    /// Demands whose exact and stepped framed rows agree.
    fn row_demands(framed: &[u16]) -> Vec<CellRowDemand> {
        framed.iter().copied().map(row_demand).collect()
    }

    /// `count` cells with nothing to show, which every column divides
    /// evenly -- the geometry and motion tests are written against it.
    fn even(count: usize) -> HeldCellLayout {
        HeldCellLayout {
            rows:          vec![row_demand(demanded_rows(0)); count],
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
                        // No core-width column fits, even after reducing the column count.
                        width: TEST_TILE_WIDTH - 1,
                        ..area
                    },
                    count,
                    growth,
                    &settings,
                ),
                "{count} below the column floor"
            );
            let layout = match column_layout(count, 1, growth, area, &settings) {
                ColumnPlan::Columns(layout) => Some(layout),
                ColumnPlan::InsufficientWidth | ColumnPlan::InsufficientRows => None,
            }
            .expect("the positive threshold admits columns");
            let opened =
                Layout::horizontal(constraints_for_sizes(&fills(layout.columns.widths.len())))
                    .split(area);
            assert!(
                opened
                    .iter()
                    .all(|&column| share_borders(column, area).width >= TEST_TILE_WIDTH)
            );
        }
    }

    #[test]
    fn the_framework_floor_still_fits_three_columns_in_twenty_two() {
        let area = Rect::new(0, 0, 22, u16::MAX);
        assert!(fits(area, 9, redistribute(1), &TileSettings::default()));
    }

    #[test]
    fn a_column_at_the_exact_floor_draws_no_piece_under_it() {
        let floor = MIN_TILE_HEIGHT.saturating_mul(2);
        let settings = TileSettings {
            min_tile_height: floor,
            ..TileSettings::default()
        };
        for pieces in [2_u16, 3] {
            for extra in [0, 1] {
                let area = Rect::new(
                    0,
                    0,
                    TEST_WIDTH,
                    shared_run(pieces, floor).saturating_add(extra),
                );
                let grid = Grid::new(
                    area,
                    &even(usize::from(pieces)),
                    add_new(usize::from(pieces)),
                    &settings,
                );
                for cell in TABLE_CELL..TABLE_CELL + usize::from(pieces) {
                    let rect = grid.cell(cell).expect("the column holds each piece");
                    assert!(
                        rect.height >= floor,
                        "{pieces} pieces with {extra} extra rows gave cell {cell} {rect:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn a_widened_summary_leaves_no_piece_under_the_floor() {
        let floor = MIN_TILE_HEIGHT.saturating_mul(2);
        let settings = TileSettings {
            min_tile_height: floor,
            ..TileSettings::default()
        };
        let area = Rect::new(0, 0, TEST_WIDTH, shared_run(2, floor).saturating_mul(2));
        let held = HeldCellLayout {
            summary_span: 2,
            ..even(4)
        };
        let grid = Grid::new(area, &held, widening(3), &settings);

        assert_eq!(grid.span, 2);
        for cell in TABLE_CELL..TABLE_CELL + held.rows.len() {
            let rect = grid.cell(cell).expect("the widened grid holds each piece");
            assert!(
                rect.height >= floor,
                "cell {cell} is under the floor: {rect:?}"
            );
        }
    }

    /// Paint the wide, short right band that is admissible only at the old floor.
    fn painted_widened_floor_fixture() -> (TileGrid<u32>, Rect, TileGrowth, TileDemands<u32>) {
        let area = Rect::new(0, 0, 80, 47);
        let growth = widening(4);
        let mut grid = TileGrid::<u32>::new();
        grid.set_min_tile_width(40);
        grid.set_min_tile_height(3);
        grid.set_layout(area, growth);
        let demands = TileDemands {
            summary:       33,
            summary_width: 80,
            groups:        (1..=9).map(|id| TileDemand { id, rows: 3 }).collect(),
        };
        grid.sync(&demands, growth);
        grid.settle_for_test();
        grid.measure_row_steps(
            area,
            growth,
            PROGRESS_SCALE,
            |_, content| content.height,
            |_, content| content.height,
        );
        assert!(
            matches!(&grid.last_settled_geometry, LastSettledCellGeometry::Drawn { grid, .. } if grid.span == 2)
        );
        (grid, area, growth, demands)
    }

    fn demand_resize_motion_fixture() -> (TileGrid<u32>, Rect, TileGrowth, TileDemands<u32>) {
        let area = Rect::new(0, 0, 200, 50);
        let growth = redistribute(4);
        let mut grid = TileGrid::<u32>::new();
        grid.set_layout(area, growth);
        let mut demands = TileDemands {
            summary:       3,
            summary_width: 0,
            groups:        (1..=3).map(|id| TileDemand { id, rows: 3 }).collect(),
        };
        grid.sync(&demands, growth);
        grid.settle_for_test();
        grid.measure_row_steps(
            area,
            growth,
            PROGRESS_SCALE,
            |_, area| area.height,
            |_, area| area.height,
        );
        demands.groups[0].rows = 30;
        grid.sync(&demands, growth);
        assert!(matches!(grid.motion, GridMotion::Moving(_)));
        (grid, area, growth, demands)
    }

    fn assert_growth_keeps_groups_visible(grid: &TileGrid<u32>, area: Rect, growth: TileGrowth) {
        for progress in [0, PROGRESS_SCALE / 2, PROGRESS_SCALE] {
            let drawing = grid.drawing_at(area, growth, progress);
            for id in 1..=3 {
                assert!(
                    drawing.pieces.iter().any(|piece| {
                        piece.placement.content == TileContent::Group(id)
                            && piece.placement.frame.clip().height >= grid.settings.min_tile_height
                    }),
                    "growth change hides group {id} at progress {progress}"
                );
            }
        }
    }

    #[test]
    fn a_growth_change_during_demand_motion_keeps_every_group_visible() {
        let (mut grid, area, _, demands) = demand_resize_motion_fixture();
        let growth = redistribute(1);
        grid.sync(&demands, growth);
        grid.set_layout(area, growth);
        assert_growth_keeps_groups_visible(&grid, area, growth);
    }

    #[test]
    fn an_action_growth_change_replans_the_motion_endpoints() {
        let (mut grid, area, _, _) = demand_resize_motion_fixture();
        let growth = redistribute(1);
        grid.apply(TileAction::FocusDown, growth);
        assert_growth_keeps_groups_visible(&grid, area, growth);
    }

    #[test]
    fn a_layout_growth_change_replans_the_motion_endpoints() {
        let (mut grid, area, _, _) = demand_resize_motion_fixture();
        let growth = redistribute(1);
        grid.set_layout(area, growth);
        assert_growth_keeps_groups_visible(&grid, area, growth);
    }

    #[test]
    fn unchanged_motion_frames_reuse_endpoint_admission() {
        let (mut grid, area, growth, demands) = demand_resize_motion_fixture();
        assert!(grid.holds_in(area, growth));
        let checks = grid.admission_checks.load(AtomicOrdering::Relaxed);
        for progress in [0, PROGRESS_SCALE / 2, PROGRESS_SCALE] {
            grid.sync_rendered_demands(&demands, growth);
            grid.set_layout(area, growth);
            assert_eq!(grid.display(), GridDisplay::Cells);
            assert_growth_keeps_groups_visible(&grid, area, growth);
            grid.measure_row_steps(
                area,
                growth,
                progress,
                |_, area| area.height,
                |_, area| area.height,
            );
        }
        assert_eq!(
            grid.admission_checks.load(AtomicOrdering::Relaxed),
            checks,
            "unchanged frames must not validate admission again"
        );
        grid.set_min_tile_height(6);
        assert!(grid.holds_in(area, growth));
        assert_eq!(
            grid.admission_checks.load(AtomicOrdering::Relaxed),
            checks + 1,
            "changed rules need one new admission check"
        );
    }

    #[test]
    fn a_floor_change_rechecks_the_painted_widened_summary_endpoint() {
        let (mut grid, area, growth, demands) = painted_widened_floor_fixture();
        grid.set_min_tile_height(6);
        grid.sync(&demands, growth);
        grid.measure_row_steps(
            area,
            growth,
            PROGRESS_SCALE / 2,
            |_, content| content.height,
            |_, content| content.height,
        );
        let drawing = grid.drawing_at(area, growth, PROGRESS_SCALE / 2);
        assert!(!drawing.pieces.is_empty());
        for piece in &drawing.pieces {
            assert!(piece.placement.frame.clip().height >= 6);
        }
    }

    #[derive(Clone, Copy)]
    enum MotionLayoutChange {
        HeightFloor,
        GridArea,
    }

    /// Fill every motion cache and queue another endpoint before changing its inputs.
    fn assert_layout_change_discards_old_motion_geometry(change: MotionLayoutChange) {
        let (mut grid, area, growth, mut demands) = painted_widened_floor_fixture();
        demands.groups[0].rows = 12;
        grid.sync(&demands, growth);
        grid.measure_row_steps(
            area,
            growth,
            PROGRESS_SCALE / 2,
            |_, content| content.height,
            |_, content| content.height,
        );
        assert!(matches!(grid.motion, GridMotion::Moving(_)));
        assert_ne!(grid.frame_row_usage, [] as [FrameCellRowUsage<u32>; 0]);
        assert_ne!(grid.motion_heights.piece_heights, [] as [u16; 0]);
        grid.retain_measured_destination_for_test();
        assert!(grid.has_retargeted_destination_for_test());
        grid.apply(TileAction::Add, growth);
        assert!(!grid.pending.is_empty());
        let target = grid.target();
        let (area, floor) = match change {
            MotionLayoutChange::HeightFloor => {
                grid.set_min_tile_height(6);
                (area, 6)
            },
            MotionLayoutChange::GridArea => {
                let resized = Rect::new(2, 1, 80, 39);
                grid.set_draw_layout(resized, growth);
                (resized, 3)
            },
        };
        assert_eq!(
            grid.slots, target,
            "the requested cell order survives replanning"
        );
        assert!(grid.pending.is_empty());
        assert!(matches!(grid.motion, GridMotion::Settled));
        assert_eq!(grid.frame_row_usage, [] as [FrameCellRowUsage<u32>; 0]);
        assert_eq!(grid.motion_heights.piece_heights, [] as [u16; 0]);
        assert!(!grid.has_retargeted_destination_for_test());
        assert!(matches!(
            grid.last_settled_geometry,
            LastSettledCellGeometry::NoCellsFrame
        ));
        grid.measure_row_steps(
            area,
            growth,
            PROGRESS_SCALE / 2,
            |_, content| content.height,
            |_, content| content.height,
        );
        for progress in [0, PROGRESS_SCALE / 2, PROGRESS_SCALE] {
            let drawing = grid.drawing_at(area, growth, progress);
            assert!(!drawing.pieces.is_empty());
            for piece in &drawing.pieces {
                let frame = piece.placement.frame.clip();
                assert!(frame.height >= floor, "changed layout floor: {frame:?}");
                assert_eq!(frame.intersection(area), frame);
            }
        }
    }

    #[test]
    fn a_floor_change_discards_queued_endpoints_and_measured_motion_heights() {
        assert_layout_change_discards_old_motion_geometry(MotionLayoutChange::HeightFloor);
    }

    #[test]
    fn an_area_change_discards_queued_endpoints_and_measured_motion_heights() {
        assert_layout_change_discards_old_motion_geometry(MotionLayoutChange::GridArea);
    }

    #[test]
    fn width_limited_columns_keep_the_exact_resident_capacity() {
        let area = Rect::new(0, 0, 80, 47);
        let settings = TileSettings {
            min_tile_width: 40,
            min_tile_height: 6,
            ..TileSettings::default()
        };
        assert!(fits(area, 18, redistribute(4), &settings));
        assert!(!fits(area, 19, redistribute(4), &settings));
        assert!(!fits(
            Rect::new(0, 0, 40, u16::MAX),
            2,
            redistribute(4),
            &TileSettings {
                min_tile_height: u16::MAX,
                ..settings
            }
        ));
    }

    #[test]
    fn summary_redistribution_checks_the_remaining_resident_band() {
        let settings = TileSettings {
            min_tile_width: 40,
            min_tile_height: 6,
            summary_row_allocation: SummaryRowAllocation::BeforeTiles,
            ..TileSettings::default()
        };
        let held = HeldCellLayout {
            rows: row_demands(&[40; 16]),
            summary_span: 2,
            ..even(16)
        };
        for (height, fits_summary) in [(47, false), (83, true)] {
            let area = Rect::new(0, 0, 80, height);
            let plan = match column_layout(16, 1, redistribute(4), area, &settings) {
                ColumnPlan::Columns(plan) => plan.for_summary(area, &held, &settings),
                unavailable => unavailable,
            };
            assert_eq!(matches!(plan, ColumnPlan::Columns(_)), fits_summary);
            if fits_summary {
                let grid = Grid::new(area, &held, redistribute(4), &settings);
                assert_eq!(grid.span, 2);
                assert_eq!(grid.resolved.panes.len(), 16);
                assert!(grid.resolved.panes.iter().all(|pane| pane.area.height >= 6));
            }
        }
    }

    #[test]
    fn width_limited_admission_keeps_resident_motion_within_the_band_budget() {
        let area = Rect::new(0, 0, 80, 47);
        let growth = TileGrowth {
            initial_rows:  4,
            fill:          TileFill::Redistribute,
            widen_summary: false,
        };
        let mut grid = TileGrid::<u32>::new();
        grid.set_min_tile_width(40);
        grid.set_min_tile_height(6);
        grid.set_layout(area, growth);
        let mut demands = TileDemands {
            summary:       0,
            summary_width: 0,
            groups:        (1..=19).map(|id| TileDemand { id, rows: 1 }).collect(),
        };
        grid.sync(&demands, growth);
        grid.settle_for_test();
        demands.groups[0].rows = 12;
        grid.sync(&demands, growth);
        let progress = PROGRESS_SCALE / 2;
        grid.measure_row_steps(
            area,
            growth,
            progress,
            |_, content| content.height,
            |_, content| content.height,
        );
        let drawing = grid.drawing_at_with_height_snap(area, growth, progress, &HeightSnap::Off);
        assert!(!drawing.pieces.is_empty());
        for piece in &drawing.pieces {
            let frame = piece.placement.frame.clip();
            assert!(
                frame.height >= 6,
                "a resident must keep its framed floor: {frame:?}"
            );
            assert!(
                frame.top() >= area.top() && frame.bottom() <= area.bottom(),
                "a resident must stay within its band: {frame:?}"
            );
        }
        assert!(
            grid.slots.len() + TABLE_CELL <= 18,
            "two 47-row columns admit at most nine six-row residents each"
        );
        assert_eq!(grid.demands.groups.len(), 19);
    }

    #[test]
    fn no_resident_piece_is_under_the_floor_in_flight() {
        let floor = MIN_TILE_HEIGHT.saturating_mul(2);
        let growth = add_new(4);
        let area = Rect::new(0, 0, TEST_WIDTH, shared_run(4, floor));
        let moving = |from: Vec<Slot<u32>>, to: Vec<Slot<u32>>| {
            let mut grid = TileGrid::new();
            grid.set_min_tile_height(floor);
            grid.set_layout(area, growth);
            grid.held = even(from.len() + TABLE_CELL);
            grid.slots = from;
            grid.queue(vec![to]);
            grid
        };
        let opening = moving(
            vec![Slot::Group(1), Slot::Group(2)],
            vec![Slot::Group(1), Slot::Group(99), Slot::Group(2)],
        );
        let closing = moving(
            vec![Slot::Group(1), Slot::Group(99), Slot::Group(2)],
            vec![Slot::Group(1), Slot::Group(2)],
        );

        for (label, grid) in [("opening", opening), ("closing", closing)] {
            for raw in motion_steps() {
                let drawing = grid.drawing_at(area, growth, raw);
                for resident in [
                    TileContent::Summary,
                    TileContent::Group(1),
                    TileContent::Group(2),
                ] {
                    let pieces = drawing
                        .pieces
                        .iter()
                        .filter(|piece| piece.placement.content == resident)
                        .collect::<Vec<_>>();
                    assert_eq!(
                        pieces.len(),
                        1,
                        "{label} at {raw} draws one resident {resident:?}"
                    );
                    let rect = pieces[0].placement.frame.clip();
                    assert!(
                        rect.height >= floor,
                        "{label} at {raw} draws {resident:?} under the floor: {rect:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn a_shrinking_summary_that_widens_keeps_resident_pieces_at_the_floor() {
        let floor = 6;
        let area = Rect::new(0, 0, 80, 21);
        let growth = widening(3);
        let slots = vec![
            Slot::Group(1),
            Slot::Group(2),
            Slot::Group(3),
            Slot::Group(4),
        ];
        let before = HeldCellLayout {
            rows:          row_demands(&[11, 6, 6, 6, 6]),
            focused:       FocusLocation::Departing,
            summary_span:  1,
            summary_depth: 1,
        };
        let mut grid = TileGrid::new();
        grid.set_min_tile_height(floor);
        grid.set_layout(area, growth);
        grid.slots = slots.clone();
        grid.held = HeldCellLayout {
            rows:          row_demands(&[6, 6, 10, 6, 6]),
            focused:       FocusLocation::Departing,
            summary_span:  2,
            summary_depth: 1,
        };
        let source = Grid::new(area, &before, growth, &grid.settings);
        grid.motion = GridMotion::Moving(Transition {
            from:         slots,
            held:         before,
            source:       Box::new(source),
            started:      Instant::now(),
            millis:       1,
            depth_status: DestinationDepthStatus::Resolved,
        });

        for raw in motion_steps().chain([PROGRESS_SCALE / 2]) {
            let drawing = grid.drawing_at(area, growth, raw);
            for piece in &drawing.pieces {
                assert!(
                    piece.placement.frame.clip().height >= floor,
                    "at {raw}, {:?} is under the floor: {:?}",
                    piece.placement.content,
                    piece.placement.frame.clip()
                );
            }
        }
    }

    #[test]
    fn fits_holds_at_the_height_floor_and_fails_one_row_under() {
        let floor = MIN_TILE_HEIGHT.saturating_mul(2);
        let mut grid = TileGrid::<u32>::new();
        grid.set_min_tile_height(0);
        assert_eq!(grid.settings.min_tile_height, MIN_TILE_HEIGHT);
        grid.set_min_tile_height(floor);
        let growth = add_new(3);
        let height = shared_run(3, floor);

        assert!(fits(
            Rect::new(0, 0, TEST_WIDTH, height),
            3,
            growth,
            &grid.settings
        ));
        assert!(!fits(
            Rect::new(0, 0, TEST_WIDTH, height.saturating_sub(1)),
            3,
            growth,
            &grid.settings
        ));
    }

    #[test]
    fn holds_in_counts_the_headed_summary_depth() {
        let growth = redistribute(1);
        let mut grid = TileGrid::<u32>::new();
        grid.set_min_tile_width(TEST_TILE_WIDTH);
        grid.slots = vec![Slot::Empty(0), Slot::Empty(1), Slot::Empty(2)];
        grid.depth = 2;
        // Two columns hold two floor-height cells each, but not the extra summary position.
        let area = Rect::new(0, 0, 80, 5);

        assert!(fits(area, grid.count(), growth, &grid.settings));
        assert!(!grid.holds_in(area, growth));
    }

    struct ViewProbeCells {
        summary_demand: usize,
        summary_drawn:  u16,
        group_demand:   usize,
        group_drawn:    u16,
    }

    impl crate::TileCells<u32> for ViewProbeCells {
        fn summary_title(&self) -> &'static str { "summary" }

        fn demands(&self, _: &[(TileContent<u32>, u16)]) -> TileDemands<u32> {
            TileDemands {
                summary:       self.summary_demand,
                summary_width: 0,
                groups:        vec![TileDemand {
                    id:   1,
                    rows: self.group_demand,
                }],
            }
        }

        fn rows_drawn(&self, content: &TileContent<u32>, area: Rect) -> u16 {
            match content {
                TileContent::Summary => self.summary_drawn.min(area.height),
                TileContent::Group(_) => self.group_drawn.min(area.height),
                TileContent::Empty(_) => 0,
            }
        }

        fn draw(&self, _: &mut Buffer, _: &TileContent<u32>, _: Rect, _: Color) {}
    }

    struct SameFrameSummaryCells {
        alone_rows: usize,
    }

    impl crate::TileCells<u32> for SameFrameSummaryCells {
        fn summary_title(&self) -> &'static str { "summary" }

        fn demands(&self, widths: &[(TileContent<u32>, u16)]) -> TileDemands<u32> {
            let summary_width = widths
                .iter()
                .find_map(|(content, width)| (*content == TileContent::Summary).then_some(*width))
                .unwrap_or_default();
            let summary = if summary_width < TEST_TILE_WIDTH {
                5
            } else {
                self.alone_rows
            };
            TileDemands {
                summary,
                summary_width: 0,
                groups: (1..=5).map(|id| TileDemand { id, rows: 1 }).collect(),
            }
        }

        fn rows_drawn(&self, content: &TileContent<u32>, area: Rect) -> u16 {
            match content {
                TileContent::Summary if area.width < TEST_TILE_WIDTH => 5,
                TileContent::Summary => u16::try_from(self.alone_rows)
                    .unwrap_or(u16::MAX)
                    .min(area.height),
                TileContent::Group(_) | TileContent::Empty(_) => area.height,
            }
        }

        fn draw(&self, _: &mut Buffer, _: &TileContent<u32>, _: Rect, _: Color) {}
    }

    struct WidthDependentSummaryCells;

    impl crate::TileCells<u32> for WidthDependentSummaryCells {
        fn summary_title(&self) -> &'static str { "summary" }

        fn demands(&self, widths: &[(TileContent<u32>, u16)]) -> TileDemands<u32> {
            let summary_width = widths
                .iter()
                .find_map(|(content, width)| (*content == TileContent::Summary).then_some(*width))
                .unwrap_or_default();
            TileDemands {
                summary:       if summary_width < TEST_TILE_WIDTH {
                    30
                } else {
                    5
                },
                summary_width: 0,
                groups:        (1..=5).map(|id| TileDemand { id, rows: 1 }).collect(),
            }
        }

        fn rows_drawn(&self, content: &TileContent<u32>, area: Rect) -> u16 {
            match content {
                TileContent::Summary if area.width < TEST_TILE_WIDTH => 30.min(area.height),
                TileContent::Summary => 5.min(area.height),
                TileContent::Group(_) => 1.min(area.height),
                TileContent::Empty(_) => 0,
            }
        }

        fn draw(&self, _: &mut Buffer, _: &TileContent<u32>, _: Rect, _: Color) {}
    }

    fn draw_view_frame(
        grid: &mut TileGrid<u32>,
        cells: &impl crate::TileCells<u32>,
        area: Rect,
        growth: TileGrowth,
    ) {
        let mut buffer = ratatui::buffer::Buffer::empty(area);
        crate::draw_tile_grid(
            &mut buffer,
            grid,
            area,
            growth,
            TileGridContents::Shown,
            cells,
        );
    }

    #[test]
    fn the_cells_view_keeps_cells_while_the_summary_draws_fewer_rows_beside_them() {
        let area = Rect::new(0, 0, TEST_TILE_WIDTH.saturating_mul(2), 20);
        let growth = TileGrowth::default();
        let cells = SameFrameSummaryCells { alone_rows: 8 };
        let mut grid = TileGrid::<u32>::new();
        grid.set_min_tile_width(TEST_TILE_WIDTH);
        draw_view_frame(&mut grid, &cells, area, growth);
        grid.settle_for_test();
        draw_view_frame(&mut grid, &cells, area, growth);

        assert_eq!(grid.display(), GridDisplay::Cells);
    }

    #[test]
    fn the_cells_view_returns_while_the_summary_is_cut_beside_them() {
        let area = Rect::new(0, 0, TEST_WIDTH, 18);
        let growth = TileGrowth::default();
        let cells = ViewProbeCells {
            summary_demand: 11,
            summary_drawn:  6,
            group_demand:   11,
            group_drawn:    6,
        };
        let mut grid = TileGrid::<u32>::new();
        grid.set_view(TileView::Summary);
        draw_view_frame(&mut grid, &cells, area, growth);
        grid.settle_for_test();
        draw_view_frame(&mut grid, &cells, area, growth);
        assert_eq!(grid.display(), GridDisplay::SummaryAlone);

        grid.set_view(TileView::Cells);
        draw_view_frame(&mut grid, &cells, area, growth);

        assert_eq!(grid.display(), GridDisplay::Cells);
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
        held.rows[0] = row_demand(TEST_HEIGHT);
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
        let wants = row_demands(wants);
        shares(
            &wants,
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

    /// Heights the frame draws from one column's disjoint layout rows.
    fn framed_column_rows(wants: &[u16], focused: ColumnFocus) -> Vec<u16> {
        let mut rows = column_rows(wants, focused);
        let shared = rows.len().saturating_sub(1);
        for row in rows.iter_mut().take(shared) {
            *row = row.saturating_add(1);
        }
        rows
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
        let rows = framed_column_rows(&[u16::MAX, 0, 0, 0], ColumnFocus::Outside);
        let shared = u16::try_from(rows.len().saturating_sub(1)).unwrap_or(u16::MAX);
        for quiet in &rows[1..] {
            assert_eq!(
                *quiet, MIN_TILE_HEIGHT,
                "the quiet cells keep only what a cell can be read at: {rows:?}"
            );
        }
        assert_eq!(
            rows.iter().sum::<u16>().saturating_sub(shared),
            TEST_HEIGHT,
            "and each shared row is counted by both neighbours"
        );
    }

    /// No cell is pushed under [`MIN_TILE_HEIGHT`], however hard its
    /// neighbours are asking.
    #[test]
    fn no_cell_is_pushed_below_what_a_cell_can_be_read_at() {
        let rows = framed_column_rows(&[u16::MAX, u16::MAX, 0, 0], ColumnFocus::Outside);
        assert!(
            rows.iter().all(|&cell| cell >= MIN_TILE_HEIGHT),
            "the floor holds for every cell: {rows:?}"
        );
    }

    #[test]
    fn no_piece_gets_a_row_past_its_ask_while_a_column_mate_is_short() {
        let settings = TileSettings::default();
        let wants = [
            settings.demanded_rows(1),
            settings.demanded_rows(100),
            settings.demanded_rows(12),
        ];
        let mut rows: Vec<u16> = shares(
            &wants,
            TEST_HEIGHT,
            ColumnHead::Cell,
            ColumnFocus::Outside,
            settings.min_tile_height,
        )
        .into_iter()
        .map(|size| match size {
            PaneAxisSize::Fixed(rows) => rows,
            PaneAxisSize::Fill(weight) => weight,
        })
        .collect();
        let shared = rows.len().saturating_sub(1);
        for rows in rows.iter_mut().take(shared) {
            *rows = rows.saturating_add(1);
        }

        assert_eq!(rows, vec![3, 25, 14]);
    }

    #[test]
    fn an_unusable_row_moves_when_its_cell_is_furthest_below_demand() {
        let wants = row_demands(&[100, 20, 20]);
        let rows = shares_with_directional_row_usage(
            &wants,
            TEST_HEIGHT,
            ColumnHead::Cell,
            ColumnFocus::Outside,
            MIN_TILE_HEIGHT,
            &mut |index, rows, _| u16::from(index == 0 && rows == 13),
        );

        assert_eq!(
            rows,
            vec![
                PaneAxisSize::Fixed(12),
                PaneAxisSize::Fixed(13),
                PaneAxisSize::Fixed(15),
            ]
        );
    }

    #[test]
    fn a_loaned_summary_yields_separators_to_hidden_process_rows() {
        let wants = row_demands(&[12, 14]);
        let rows = shares_with_directional_row_usage(
            &wants,
            21,
            ColumnHead::Summary(1),
            ColumnFocus::Outside,
            MIN_TILE_HEIGHT,
            &mut |index, rows, purpose| {
                let used = match (index, purpose) {
                    (0, RowUsePurpose::RetainContent) => rows.min(9),
                    (0, RowUsePurpose::PaintContent) => rows.min(11),
                    _ => rows.min(14),
                };
                rows.saturating_sub(used)
            },
        );
        assert_eq!(rows, vec![PaneAxisSize::Fixed(9), PaneAxisSize::Fixed(12)]);
    }

    /// Row-use readings from a startup column whose transfers repeated.
    #[derive(Default)]
    struct StartupColumnRows {
        readings: usize,
    }

    impl ColumnRowReadings for StartupColumnRows {
        fn unused(&mut self, index: usize, rows: u16, purpose: RowUsePurpose) -> u16 {
            self.readings += 1;
            assert!(self.readings <= 512, "startup transfers do not finish");
            let reading = match (index, rows) {
                (0, 25..=31) => Some((rows - 25, 0)),
                (0, 32) => Some((7, 0)),
                (1, 6..=8) | (2, 5..=7) | (3, 6..=7) => Some((0, 0)),
                (3, 8 | 10) => Some((1, 1)),
                (3, 9) => Some((2, 1)),
                _ => None,
            };
            let (retained, painted) = reading.expect("the startup fixture defines this allocation");
            match purpose {
                RowUsePurpose::RetainContent => retained,
                RowUsePurpose::PaintContent => painted,
            }
        }
    }

    #[test]
    fn a_donor_whose_row_use_jumps_as_it_shrinks_keeps_its_used_content() {
        let exact = [32, 8, 7, 10];
        let minimum = [5, 5, 5, 6];
        let mut measured = StartupColumnRows::default();
        for settled in [[29, 6, 5, 9], [30, 6, 5, 8], [28, 6, 5, 10]] {
            for protection in [RowUsePurpose::PaintContent, RowUsePurpose::RetainContent] {
                for index in [0, 3] {
                    let held = settled[index];
                    let used = held - measured.unused(index, held, protection);
                    let spare = directional_spare_rows(
                        index,
                        &settled,
                        &exact,
                        &minimum,
                        protection,
                        &mut measured,
                    );
                    for donated in 1..=spare {
                        let lower = held - donated;
                        assert_eq!(
                            lower - measured.unused(index, lower, protection),
                            used,
                            "tile {index} loses content at {held}->{lower}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn a_startup_column_with_nonmonotone_row_use_finishes_sharing() {
        for mut settled in [[29, 6, 5, 9], [30, 6, 5, 8], [28, 6, 5, 10]] {
            let mut measured = StartupColumnRows::default();
            serve_directional_shortfalls(
                &mut settled,
                &[32, 8, 7, 10],
                &[5, 5, 5, 6],
                &mut measured,
            );
            assert_eq!(settled, [25, 7, 7, 10]);
        }
    }

    #[test]
    fn a_summary_separator_uses_a_mates_unpainted_spare_row() {
        let mut shares = [10, 11];
        let mut unused = |index, rows: u16, purpose| {
            let used = match (index, purpose) {
                (0, RowUsePurpose::RetainContent) => rows.min(9),
                (0, RowUsePurpose::PaintContent) => rows.min(11),
                _ => rows.min(10),
            };
            rows.saturating_sub(used)
        };
        let mut measured = ColumnRowUse::new(shares.len(), &mut unused);
        serve_directional_shortfalls(&mut shares, &[11, 14], &[2, 3], &mut measured);
        assert_eq!(shares, [11, 10]);
    }

    /// Unused rows from the measured five-cell column at each considered height.
    const fn measured_column_unused(index: usize, rows: u16, _: RowUsePurpose) -> u16 {
        match (index, rows) {
            (0 | 2 | 3, 6) => 1,
            _ => 0,
        }
    }

    #[test]
    fn a_freed_row_goes_to_a_mate_whose_next_step_it_completes() {
        let mut settled = [6, 6, 6, 5, 6];
        let exact = [17, 11, 15, 15, 12];
        let minimum = [5, 5, 5, 5, 6];

        let mut unused = measured_column_unused;
        let mut measured = ColumnRowUse::new(settled.len(), &mut unused);
        serve_directional_shortfalls(&mut settled, &exact, &minimum, &mut measured);

        assert_eq!(settled, [7, 6, 5, 5, 6]);
    }

    #[test]
    fn a_cell_made_whole_gives_no_row_away() {
        let mut settled = [6, 6, 6, 5, 6];
        let exact = [17, 11, 15, 15, 12];
        let minimum = [5, 5, 5, 5, 6];

        let mut unused = measured_column_unused;
        let mut measured = ColumnRowUse::new(settled.len(), &mut unused);
        serve_directional_shortfalls(&mut settled, &exact, &minimum, &mut measured);

        assert_eq!(settled[0], 7);
    }

    #[test]
    fn a_freed_row_stays_when_no_mates_next_step_fits() {
        let mut settled = [6, 5, 5];
        let exact = [12, 12, 12];
        let minimum = [5, 5, 5];
        let mut unused = |index, rows, _| match (index, rows) {
            (0..=2, 6) => 1,
            _ => 0,
        };

        let mut measured = ColumnRowUse::new(settled.len(), &mut unused);
        serve_directional_shortfalls(&mut settled, &exact, &minimum, &mut measured);

        assert_eq!(settled, [6, 5, 5]);
    }

    #[test]
    fn an_exact_demand_change_redivides_without_motion() {
        let growth = add_new(3);
        let demands = |small| TileDemands {
            summary:       small,
            summary_width: 0,
            groups:        vec![
                TileDemand { id: 1, rows: 100 },
                TileDemand { id: 2, rows: 12 },
            ],
        };
        let mut grid = TileGrid::new();
        grid.set_layout(test_area(), growth);
        grid.sync(&demands(1), growth);
        grid.settle_for_test();
        let before = Grid::new(test_area(), &grid.held, growth, &grid.settings)
            .cell(TABLE_CELL)
            .expect("the summary is placed")
            .height;

        grid.sync(&demands(2), growth);

        assert!(matches!(grid.motion, GridMotion::Settled));
        let after = Grid::new(test_area(), &grid.held, growth, &grid.settings)
            .cell(TABLE_CELL)
            .expect("the summary is placed")
            .height;
        assert_eq!((before, after), (3, 4));
    }

    #[test]
    fn a_shrinking_cell_updates_its_exact_demand_across_a_step_boundary() {
        let growth = add_new(3);
        let demands = |last| TileDemands {
            summary:       1,
            summary_width: 0,
            groups:        vec![
                TileDemand { id: 1, rows: 100 },
                TileDemand {
                    id:   2,
                    rows: last,
                },
            ],
        };
        let mut grid = TileGrid::new();
        grid.set_layout(test_area(), growth);
        grid.sync(&demands(4), growth);
        grid.settle_for_test();
        let heights = |grid: &TileGrid<u32>| {
            let resolved = Grid::new(test_area(), &grid.held, growth, &grid.settings);
            (TABLE_CELL..=TABLE_CELL + 2)
                .map(|cell| {
                    resolved
                        .cell(cell)
                        .expect("the column holds the cell")
                        .height
                })
                .collect::<Vec<_>>()
        };

        assert_eq!(heights(&grid), vec![3, 33, 6]);
        grid.sync(&demands(3), growth);
        grid.settle_for_test();

        assert_eq!(heights(&grid), vec![3, 34, 5]);
        assert_eq!(
            grid.held.rows[2].stepped_framed(),
            demanded_rows(4),
            "the held layout retains the earlier step"
        );
        assert_eq!(
            grid.held.rows[2].exact_framed(),
            TileSettings::default().demanded_rows(3).exact_framed(),
            "the held layout accepts the current exact demand"
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
        assert!(
            level[1].abs_diff(level[2]) <= 1,
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
            let demands = row_demands(&asked);
            assert_eq!(
                shares(
                    &demands,
                    TEST_HEIGHT,
                    ColumnHead::Cell,
                    ColumnFocus::Row(1),
                    MIN_TILE_HEIGHT
                ),
                shares(
                    &demands,
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
        let demands = row_demands(&asked);
        assert_eq!(
            shares(
                &demands,
                TEST_HEIGHT,
                ColumnHead::Cell,
                ColumnFocus::Row(1),
                MIN_TILE_HEIGHT
            ),
            shares(
                &demands,
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
        let grown: Vec<u16> = grid
            .held
            .rows
            .iter()
            .map(|demand| demand.stepped_framed())
            .collect();

        grid.sync(&quiet(&[7]), redistribute(4));
        grid.settle_for_test();

        assert_eq!(
            grid.held
                .rows
                .iter()
                .map(|demand| demand.stepped_framed())
                .collect::<Vec<_>>(),
            grown,
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
            grid.held.rows[1].stepped_framed(),
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
            rows:          row_demands(&rows),
            focused:       FocusLocation::Cell(2 + TABLE_CELL),
            summary_span:  1,
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
        // No core-width tile fits, so only the summary can be placed.
        let narrow_area = Rect::new(0, 0, TEST_TILE_WIDTH - 1, TEST_HEIGHT);
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
            identity: CellIdentity::Summary,
            content:  TileContent::Summary,
            focused:  false,
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
        place_band_pieces(before, after, &bands, pieces, &HeightSnap::Off)
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
        let placed = place_band_pieces(&before, &after, &bands, pieces, &HeightSnap::Off);

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
        let closed = place_band_pieces(&before, &after, &bands, pieces, &HeightSnap::Off)
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
        let area = Rect {
            height: TEST_HEIGHT.saturating_add(1),
            ..test_area()
        };
        for (before, after, start_depth, final_depth) in [
            (&[1, 2][..], &[1, 99, 2][..], 2, 3),
            (&[1, 99, 2][..], &[1, 2][..], 3, 2),
        ] {
            let demands = |ids: &[u32]| TileDemands {
                summary:       37,
                summary_width: 0,
                groups:        ids.iter().map(|&id| TileDemand { id, rows: 0 }).collect(),
            };
            let mut grid = seeded_grid();
            grid.set_layout(area, growth);
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
                        &QueuedArrangement {
                            slots:         &start,
                            summary_depth: start_depth,
                        },
                        &QueuedArrangement {
                            slots:         &finish,
                            summary_depth: final_depth,
                        },
                        &QueuedArrangement {
                            slots,
                            summary_depth: depth
                        },
                        growth,
                        area,
                        &grid.settings,
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
        assert_eq!(grid.target_depth(), 1);

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

    /// A key fixture whose summary needs two positions before `+` and one
    /// after it. The command remains short of its exact row demand throughout.
    fn depth_changing_key_grid() -> (TileGrid<u32>, Rect, TileGrowth) {
        let area = test_area();
        let growth = add_new(4);
        let demands = TileDemands {
            summary:       usize::from(TEST_HEIGHT) * 3 / 4,
            summary_width: 0,
            groups:        vec![TileDemand {
                id:   1,
                rows: usize::from(TEST_HEIGHT),
            }],
        };
        let mut grid = TileGrid::new();
        grid.set_view(TileView::Cells);
        grid.set_layout(area, growth);
        grid.sync(&demands, growth);
        grid.settle_for_test();
        assert_eq!(grid.depth, 2);
        (grid, area, growth)
    }

    #[test]
    fn a_key_that_changes_slots_and_depth_queues_one_step() {
        let (mut grid, _, growth) = depth_changing_key_grid();
        for (action, final_depth) in [(TileAction::Add, 1), (TileAction::Remove, 2)] {
            assert!(matches!(grid.motion, GridMotion::Settled));
            grid.apply(action, growth);
            assert!(matches!(grid.motion, GridMotion::Moving(_)));
            assert_eq!(grid.depth, final_depth);
            assert!(grid.pending.is_empty());
            grid.advance();
        }
    }

    /// Apply one key and inspect every point of every step it queues.
    fn assert_key_keeps_unchanged_heights_one_way(
        grid: &mut TileGrid<u32>,
        area: Rect,
        growth: TileGrowth,
        action: TileAction,
    ) {
        let contents = [TileContent::Summary, TileContent::Group(1)];
        let mut paths = vec![Vec::new(); contents.len()];
        grid.apply(action, growth);
        while matches!(grid.motion, GridMotion::Moving(_)) {
            for raw in 0..=PROGRESS_SCALE {
                let drawing = grid.drawing_at(area, growth, raw);
                for (content, path) in contents.iter().zip(&mut paths) {
                    let mut pieces = drawing
                        .placements()
                        .filter(|placement| &placement.content == content);
                    let piece = pieces.next().expect("the unchanged cell remains visible");
                    assert!(
                        pieces.next().is_none(),
                        "the unchanged cell stays in its band"
                    );
                    path.push(piece.frame.clip().height);
                }
            }
            grid.advance();
        }
        for (content, heights) in contents.iter().zip(paths) {
            let (&start, &end) = heights
                .first()
                .zip(heights.last())
                .expect("the key queues a moving frame");
            let one_way = if start <= end {
                heights.windows(2).all(|pair| pair[0] <= pair[1])
            } else {
                heights.windows(2).all(|pair| pair[0] >= pair[1])
            };
            assert!(one_way, "{content:?} reverses through {heights:?}");
        }
    }

    #[test]
    fn no_unchanged_cell_reverses_height_across_one_key() {
        let (mut grid, area, growth) = depth_changing_key_grid();
        assert_key_keeps_unchanged_heights_one_way(&mut grid, area, growth, TileAction::Add);
        assert_key_keeps_unchanged_heights_one_way(&mut grid, area, growth, TileAction::Remove);
    }

    /// Render the new widths, then inspect the one motion the key started.
    fn assert_width_changing_key_moves_once(
        grid: &mut TileGrid<u32>,
        cells: &WidthDependentSummaryCells,
        area: Rect,
        growth: TileGrowth,
        action: TileAction,
        expected_depth: usize,
    ) {
        grid.apply(action, growth);
        assert!(matches!(grid.motion, GridMotion::Moving(_)));
        draw_view_frame(grid, cells, area, growth);
        assert!(grid.pending.is_empty(), "the key has no second motion");
        assert_eq!(grid.depth, expected_depth);

        let contents = [TileContent::Summary, TileContent::Group(1)];
        let mut paths = vec![Vec::new(); contents.len()];
        for raw in grid.distinct_motion_steps_for_test(area, growth) {
            grid.measure_row_steps(
                area,
                growth,
                raw,
                |content, area| crate::TileCells::rows_drawn(cells, content, area),
                |content, area| crate::TileCells::rows_kept(cells, content, area),
            );
            let drawing = grid.drawing_at(area, growth, raw);
            for (content, path) in contents.iter().zip(&mut paths) {
                let height = drawing
                    .placements()
                    .find(|placement| &placement.content == content)
                    .map(|placement| placement.frame.clip().height)
                    .expect("the unchanged cell remains visible");
                path.push(height);
            }
        }
        for (content, heights) in contents.iter().zip(paths) {
            let (&start, &end) = heights
                .first()
                .zip(heights.last())
                .expect("the key draws one motion");
            let one_way = if start <= end {
                heights.windows(2).all(|pair| pair[0] <= pair[1])
            } else {
                heights.windows(2).all(|pair| pair[0] >= pair[1])
            };
            assert!(one_way, "{content:?} reverses through {heights:?}");
        }

        grid.advance();
        assert!(matches!(grid.motion, GridMotion::Settled));
        assert_eq!(grid.depth, expected_depth);
        draw_view_frame(grid, cells, area, growth);
    }

    #[test]
    fn a_key_whose_new_widths_change_demands_still_moves_once() {
        let area = Rect::new(0, 0, TEST_TILE_WIDTH.saturating_mul(3), TEST_HEIGHT);
        let growth = TileGrowth::default();
        let cells = WidthDependentSummaryCells;
        let mut grid = TileGrid::new();
        grid.set_min_tile_width(TEST_TILE_WIDTH);
        grid.set_min_tile_height(6);
        draw_view_frame(&mut grid, &cells, area, growth);
        grid.settle_for_test();
        draw_view_frame(&mut grid, &cells, area, growth);
        assert_eq!(grid.count(), 6);
        assert_eq!(grid.depth, 1);

        assert_width_changing_key_moves_once(&mut grid, &cells, area, growth, TileAction::Add, 2);
        assert_width_changing_key_moves_once(
            &mut grid,
            &cells,
            area,
            growth,
            TileAction::Remove,
            1,
        );
    }

    /// Demands that keep every stepped fixture's content incomplete.
    fn tall_demands(ids: &[u32]) -> TileDemands<u32> {
        let rows = usize::from(TEST_HEIGHT).saturating_mul(2);
        TileDemands {
            summary:       rows,
            summary_width: 0,
            groups:        ids.iter().map(|&id| TileDemand { id, rows }).collect(),
        }
    }

    /// A third resident enters one column whose other pieces both shrink.
    fn stepped_height_motion() -> (TileGrid<u32>, Rect, TileGrowth) {
        let area = test_area();
        let growth = redistribute(usize::from(TEST_HEIGHT));
        let mut grid = TileGrid::new();
        grid.set_view(TileView::Cells);
        grid.set_layout(area, growth);
        grid.sync(&tall_demands(&[1]), growth);
        grid.settle_for_test();
        grid.collapse_queued_changes_for_test();
        grid.sync(&tall_demands(&[1, 2]), growth);
        (grid, area, growth)
    }

    /// One single-column motion with exact framed endpoint heights.
    fn exact_height_motion(before: &[u16], after: &[u16]) -> (TileGrid<u32>, Rect, TileGrowth) {
        let cells = before.len();
        let height = before
            .iter()
            .copied()
            .reduce(|sum, height| sum.saturating_add(height).saturating_sub(1))
            .unwrap_or_default();
        assert_eq!(
            height,
            after
                .iter()
                .copied()
                .reduce(|sum, height| sum.saturating_add(height).saturating_sub(1))
                .unwrap_or_default()
        );
        let area = Rect::new(0, 0, TEST_WIDTH, height);
        let growth = redistribute(cells);
        let slots = (1..u32::try_from(cells).unwrap_or(u32::MAX))
            .map(Slot::Group)
            .collect::<Vec<_>>();
        let mut grid = TileGrid::new();
        grid.set_view(TileView::Cells);
        grid.set_layout(area, growth);
        grid.slots = slots.clone();
        grid.held = HeldCellLayout {
            rows:          row_demands(after),
            focused:       FocusLocation::Departing,
            summary_span:  1,
            summary_depth: 1,
        };
        let before = HeldCellLayout {
            rows:          row_demands(before),
            focused:       FocusLocation::Departing,
            summary_span:  1,
            summary_depth: 1,
        };
        let source = Grid::new(area, &before, growth, &grid.settings);
        grid.motion = GridMotion::Moving(Transition {
            from:         slots,
            held:         before,
            source:       Box::new(source),
            started:      Instant::now(),
            millis:       1,
            depth_status: DestinationDepthStatus::Resolved,
        });
        (grid, area, growth)
    }

    /// One single-column add or remove motion with exact endpoint heights.
    fn exact_slot_height_motion(
        before: &[u16],
        after: &[u16],
        before_slots: Vec<Slot<u32>>,
        after_slots: Vec<Slot<u32>>,
    ) -> (TileGrid<u32>, Rect, TileGrowth) {
        let source_height = before
            .iter()
            .copied()
            .reduce(|sum, height| sum.saturating_add(height).saturating_sub(1))
            .unwrap_or_default();
        let destination_height = after
            .iter()
            .copied()
            .reduce(|sum, height| sum.saturating_add(height).saturating_sub(1))
            .unwrap_or_default();
        assert_eq!(source_height, destination_height);
        assert_eq!(before.len(), before_slots.len().saturating_add(1));
        assert_eq!(after.len(), after_slots.len().saturating_add(1));
        let area = Rect::new(0, 0, TEST_WIDTH, source_height);
        let growth = redistribute(before.len().max(after.len()));
        let source_held = HeldCellLayout {
            rows:          row_demands(before),
            focused:       FocusLocation::Departing,
            summary_span:  1,
            summary_depth: 1,
        };
        let destination_held = HeldCellLayout {
            rows:          row_demands(after),
            focused:       FocusLocation::Departing,
            summary_span:  1,
            summary_depth: 1,
        };
        let source = Grid::new(area, &source_held, growth, &TileSettings::default());
        let mut grid = TileGrid::new();
        grid.set_view(TileView::Cells);
        grid.set_layout(area, growth);
        grid.slots = after_slots;
        grid.held = destination_held;
        grid.motion = GridMotion::Moving(Transition {
            from:         before_slots,
            held:         source_held,
            source:       Box::new(source),
            started:      Instant::now(),
            millis:       1,
            depth_status: DestinationDepthStatus::Resolved,
        });
        (grid, area, growth)
    }

    #[test]
    fn a_previous_empty_height_below_the_destination_floor_keeps_ordered_bounds() {
        let (mut grid, area, growth) =
            exact_slot_height_motion(&[9, 3], &[6, 6], vec![Slot::Empty(0)], vec![Slot::Empty(0)]);
        let previous = grid.drawing_at_with_height_snap(area, growth, 0, &HeightSnap::Off);
        let heights = previous
            .placements()
            .map(|placement| placement.frame.clip().height)
            .collect::<Vec<_>>();
        assert_eq!(heights, [9, 3]);
        grid.motion_heights = MotionHeightPath {
            area,
            growth,
            progress: 0,
            piece_heights: heights,
        };
        grid.set_min_tile_height(6);
        grid.demands = TileDemands {
            summary:       100,
            summary_width: 0,
            groups:        Vec::new(),
        };

        grid.measure_row_steps(
            area,
            growth,
            1,
            |content, area| match content {
                TileContent::Empty(_) => 0,
                _ => area.height,
            },
            |content, area| match content {
                TileContent::Empty(_) => 0,
                _ => area.height,
            },
        );

        let destination = grid.resolved_grid(area, &grid.drawn_held(), growth);
        assert_eq!(destination.rules.min_tile_height, 6);
        assert_eq!(destination.resolved.panes.len(), 2);
        for pane in &destination.resolved.panes {
            assert!((destination.rules.min_tile_height..=area.height).contains(&pane.area.height));
        }
        assert_eq!(destination.resolved.panes[1].area.height, 6);
    }

    /// Draw the exact-height fixture with every candidate height filled.
    fn filled_height_drawing(
        grid: &mut TileGrid<u32>,
        area: Rect,
        growth: TileGrowth,
        raw: u32,
    ) -> TileDrawing<u32> {
        grid.measure_row_steps(
            area,
            growth,
            raw,
            |_, area| area.height,
            |_, area| area.height,
        );
        grid.drawing_at(area, growth, raw)
    }

    /// Record one settled frame and return its framed height by content.
    fn recorded_settled_heights(
        grid: &mut TileGrid<u32>,
        area: Rect,
        growth: TileGrowth,
    ) -> Vec<(TileContent<u32>, u16)> {
        grid.measure_row_steps(
            area,
            growth,
            PROGRESS_SCALE,
            |content, area| match content {
                TileContent::Empty(_) => 0,
                TileContent::Summary | TileContent::Group(_) => area.height,
            },
            |content, area| match content {
                TileContent::Empty(_) => 0,
                TileContent::Summary | TileContent::Group(_) => area.height,
            },
        );
        grid.drawing_at(area, growth, PROGRESS_SCALE)
            .placements()
            .map(|placement| (placement.content.clone(), placement.frame.clip().height))
            .collect()
    }

    /// One content's framed height in a drawing.
    fn piece_height(drawing: &TileDrawing<u32>, content: &TileContent<u32>) -> Option<u16> {
        drawing
            .placements()
            .find(|placement| &placement.content == content)
            .map(|placement| placement.frame.clip().height)
    }

    #[test]
    fn a_departure_uses_recorded_heights_when_its_rows_drop_to_zero() {
        let area = test_area();
        let growth = redistribute(4);
        let mut grid = TileGrid::new();
        grid.set_view(TileView::Cells);
        grid.set_layout(area, growth);
        grid.sync(&tall_demands(&[1, 2, 3]), growth);
        grid.settle_for_test();
        let recorded = recorded_settled_heights(&mut grid, area, growth);
        grid.sync(&tall_demands(&[1, 3]), growth);

        let remaining = [
            TileContent::Summary,
            TileContent::Group(1),
            TileContent::Group(3),
        ];
        let mut paths = vec![Vec::new(); remaining.len()];
        for raw in motion_steps() {
            grid.measure_row_steps(
                area,
                growth,
                raw,
                |content, area| match content {
                    TileContent::Group(2) | TileContent::Empty(_) => 0,
                    TileContent::Summary | TileContent::Group(_) => area.height,
                },
                |content, area| match content {
                    TileContent::Group(2) | TileContent::Empty(_) => 0,
                    TileContent::Summary | TileContent::Group(_) => area.height,
                },
            );
            let drawing = grid.drawing_at(area, growth, raw);
            if raw == 0 {
                for (content, height) in &recorded {
                    assert_eq!(piece_height(&drawing, content), Some(*height));
                }
            }
            for (content, heights) in remaining.iter().zip(&mut paths) {
                heights.push(
                    piece_height(&drawing, content)
                        .expect("each remaining piece stays in the drawing"),
                );
            }
        }

        for (content, heights) in remaining.iter().zip(paths) {
            let start = recorded
                .iter()
                .find_map(|(recorded, height)| (recorded == content).then_some(*height))
                .expect("the settled frame contains every remaining piece");
            let end = heights.last().copied().unwrap_or(start);
            let one_way = if start <= end {
                heights.iter().all(|height| *height >= start)
                    && heights.windows(2).all(|pair| pair[0] <= pair[1])
            } else {
                heights.iter().all(|height| *height <= start)
                    && heights.windows(2).all(|pair| pair[0] >= pair[1])
            };
            assert!(
                one_way,
                "{content:?} reverses from {start} through {heights:?}"
            );
        }
    }

    /// Check the first moving frame against the settled frame before one key.
    fn assert_key_motion_starts_one_way(
        grid: &mut TileGrid<u32>,
        area: Rect,
        growth: TileGrowth,
        action: TileAction,
    ) {
        let recorded = recorded_settled_heights(grid, area, growth);
        grid.apply(action, growth);
        let raw = PROGRESS_SCALE / 20;
        grid.measure_row_steps(
            area,
            growth,
            raw,
            |content, area| match content {
                TileContent::Group(1) | TileContent::Empty(_) => 0,
                TileContent::Summary | TileContent::Group(_) => area.height,
            },
            |content, area| match content {
                TileContent::Group(1) | TileContent::Empty(_) => 0,
                TileContent::Summary | TileContent::Group(_) => area.height,
            },
        );
        let first = grid.drawing_at(area, growth, raw);
        let destination = grid.drawing_at(area, growth, PROGRESS_SCALE);
        for (content, start) in recorded {
            let Some(first) = piece_height(&first, &content) else {
                continue;
            };
            let Some(end) = piece_height(&destination, &content) else {
                continue;
            };
            assert!(
                (start.min(end)..=start.max(end)).contains(&first),
                "{content:?} moves from {start} away from its {end} destination to {first}"
            );
        }
    }

    #[test]
    fn seven_cell_plus_and_minus_start_every_piece_one_way() {
        let area = Rect::new(0, 0, 200, 50);
        let growth = redistribute(12);
        let mut grid = TileGrid::new();
        grid.set_view(TileView::Cells);
        grid.set_min_tile_width(40);
        grid.set_min_tile_height(6);
        grid.set_layout(area, growth);
        grid.sync(&tall_demands(&[1, 2, 3, 4, 5, 6]), growth);
        grid.settle_for_test();

        assert_key_motion_starts_one_way(&mut grid, area, growth, TileAction::Add);
        grid.settle_for_test();
        assert_key_motion_starts_one_way(&mut grid, area, growth, TileAction::Remove);
    }

    fn assert_piece_heights_move_one_way(before: &[u16], after: &[u16]) {
        let (mut grid, area, growth) = exact_height_motion(before, after);
        let mut heights = vec![Vec::new(); before.len()];
        for raw in single_column_height_steps(&grid.drawing_at(area, growth, 0)) {
            let drawing = filled_height_drawing(&mut grid, area, growth, raw);
            for (piece, heights) in drawing.placements().zip(&mut heights) {
                heights.push(piece.frame.clip().height);
            }
        }
        for ((&start, &finish), heights) in before.iter().zip(after).zip(heights) {
            assert_eq!(heights.first().copied(), Some(start));
            assert_eq!(heights.last().copied(), Some(finish));
            let moves_one_way = if start <= finish {
                heights.windows(2).all(|pair| pair[0] <= pair[1])
            } else {
                heights.windows(2).all(|pair| pair[0] >= pair[1])
            };
            assert!(
                moves_one_way,
                "{start} to {finish} reverses through {heights:?}"
            );
        }
    }

    #[test]
    fn filled_motion_sources_begin_at_their_settled_heights() {
        for (before, after) in [
            (&[6, 6, 9][..], &[8, 7, 6][..]),
            (&[6, 19, 9][..], &[19, 8, 7][..]),
            (&[6, 6, 10][..], &[7, 8, 7][..]),
        ] {
            let (mut grid, area, growth) = exact_height_motion(before, after);
            let drawing = filled_height_drawing(&mut grid, area, growth, 0);

            for (piece, &settled_height) in drawing.pieces.iter().zip(before) {
                assert_eq!(piece.placement.frame.clip().height, settled_height);
                assert_eq!(
                    piece.height_snap,
                    PieceHeightSnap::Search {
                        start: settled_height,
                        end:   match piece.height_snap {
                            PieceHeightSnap::Search { end, .. } => end,
                            PieceHeightSnap::Settled | PieceHeightSnap::KeepStartingHeight =>
                                settled_height,
                        },
                    }
                );
            }
        }
    }

    /// Draw and inspect a motion whose contents fill selected framed heights.
    fn assert_snapped_heights_move_one_way(
        before: &[u16],
        after: &[u16],
        valid_heights: &[&[u16]],
    ) {
        let (mut grid, area, growth) = exact_height_motion(before, after);
        let mut piece_heights = vec![Vec::new(); before.len()];
        for raw in single_column_height_steps(&grid.drawing_at(area, growth, 0)) {
            grid.measure_row_steps(
                area,
                growth,
                raw,
                |content, area| {
                    if area.height == u16::MAX {
                        return area.height;
                    }
                    let index = match content {
                        TileContent::Summary => 0,
                        TileContent::Group(id) => usize::try_from(*id).unwrap_or(usize::MAX),
                        TileContent::Empty(_) => return area.height,
                    };
                    let framed = area
                        .height
                        .saturating_add(TILE_BORDER_ROWS)
                        .saturating_add(TILE_ROWS_READOUT_HEIGHT);
                    if valid_heights[index].contains(&framed) {
                        area.height
                    } else {
                        area.height.saturating_sub(1)
                    }
                },
                |_, area| area.height,
            );
            let drawing = grid.drawing_at(area, growth, raw);
            for (index, placement) in drawing.placements().enumerate() {
                let height = placement.frame.clip().height;
                assert!(
                    valid_heights[index].contains(&height),
                    "piece {index} uses unfilled height {height} at {raw}"
                );
                piece_heights[index].push(height);
            }
        }
        for (index, ((&start, &end), heights)) in
            before.iter().zip(after).zip(piece_heights).enumerate()
        {
            assert_eq!(heights.first().copied(), Some(start));
            assert_eq!(heights.last().copied(), Some(end));
            let one_way = if start <= end {
                heights.windows(2).all(|pair| pair[0] <= pair[1])
            } else {
                heights.windows(2).all(|pair| pair[0] >= pair[1])
            };
            assert!(one_way, "piece {index} reverses through {heights:?}");
        }
    }

    #[test]
    fn mixed_height_changes_move_each_piece_one_way() {
        assert_piece_heights_move_one_way(&[6, 6, 9], &[8, 7, 6]);
    }

    #[test]
    fn a_large_height_shrink_moves_one_way() {
        assert_piece_heights_move_one_way(&[6, 19, 9], &[19, 8, 7]);
    }

    #[test]
    fn three_snapped_mates_move_one_way() {
        assert_snapped_heights_move_one_way(
            &[6, 6, 10],
            &[7, 8, 7],
            &[&[6, 7], &[6, 7, 8], &[7, 9, 10]],
        );
    }

    #[test]
    fn mixed_snapped_height_changes_move_one_way() {
        assert_snapped_heights_move_one_way(
            &[6, 6, 9],
            &[8, 7, 6],
            &[&[6, 8], &[6, 7], &[6, 7, 9]],
        );
    }

    /// Painted rows at every height of the endpoint-and-midpoint fixture.
    fn endpoint_painted_rows(endpoints: &[(u16, u16)], height: u16) -> Vec<Vec<u16>> {
        endpoints
            .iter()
            .map(|&(start, end)| {
                let filled = [start, u16::midpoint(start, end), end];
                (0..=height)
                    .map(|height| {
                        let framed = height
                            .saturating_add(TILE_BORDER_ROWS)
                            .saturating_add(TILE_ROWS_READOUT_HEIGHT);
                        if filled.contains(&framed) {
                            height
                        } else {
                            height.saturating_sub(1)
                        }
                    })
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>()
    }

    /// Both sides of every individual or aggregate integer height transfer.
    fn single_column_height_steps(drawing: &TileDrawing<u32>) -> Vec<u32> {
        let mut changes = Vec::new();
        let (mut grown, mut shrunk) = (0_u16, 0_u16);
        for piece in &drawing.pieces {
            if let PieceHeightSnap::Search { start, end } = piece.height_snap {
                changes.push(start.abs_diff(end));
                grown = grown.saturating_add(end.saturating_sub(start));
                shrunk = shrunk.saturating_add(start.saturating_sub(end));
            }
        }
        changes.extend([grown, shrunk]);
        changes.sort_unstable();
        changes.dedup();
        let mut prior = vec![0; changes.len()];
        let mut steps = vec![0, PROGRESS_SCALE];
        for raw in 1..=PROGRESS_SCALE {
            let heights = changes
                .iter()
                .map(|&change| lerp(0, change, eased(raw)))
                .collect::<Vec<_>>();
            if heights != prior {
                steps.extend([raw.saturating_sub(1), raw]);
                prior = heights;
            }
        }
        steps.sort_unstable();
        steps.dedup();
        steps
    }

    /// The endpoint fixture's painted rows, including its unbounded probe.
    fn endpoint_rows_drawn(
        content: &TileContent<u32>,
        area: Rect,
        painted_rows: &[Vec<u16>],
    ) -> u16 {
        if area.height == u16::MAX {
            return TEST_HEIGHT;
        }
        let index = match content {
            TileContent::Summary => 0,
            TileContent::Group(id) => usize::try_from(*id).unwrap_or(usize::MAX),
            TileContent::Empty(_) => return 0,
        };
        painted_rows
            .get(index)
            .and_then(|rows| rows.get(usize::from(area.height)))
            .copied()
            .unwrap_or_else(|| area.height.saturating_sub(1))
    }

    #[test]
    fn unchanged_mates_stay_between_their_endpoints_during_plus_and_minus_motion() {
        let fewer = [19, 7, 8, 8, 8, 7];
        let more = [15, 9, 10, 6, 6, 6, 6];
        let groups = (1..=5).map(Slot::Group).collect::<Vec<_>>();
        let mut added = groups.clone();
        added.push(Slot::Empty(0));
        for (before, after, before_slots, after_slots) in [
            (&fewer[..], &more[..], groups.clone(), added.clone()),
            (&more[..], &fewer[..], added, groups),
        ] {
            let (mut grid, area, growth) =
                exact_slot_height_motion(before, after, before_slots, after_slots);
            let endpoints = before
                .iter()
                .copied()
                .zip(after.iter().copied())
                .collect::<Vec<_>>();
            let painted_rows = endpoint_painted_rows(&endpoints, area.height);
            let mut prior = before.to_vec();
            let mut heights = Vec::with_capacity(before.len().max(after.len()));
            let opening = grid.drawing_at_with_height_snap(area, growth, 0, &HeightSnap::Off);
            assert_eq!(opening.column_bands, [area]);
            let steps = single_column_height_steps(&opening);
            let mut raw = 0;
            loop {
                grid.measure_row_steps(
                    area,
                    growth,
                    raw,
                    |content, probe_area| endpoint_rows_drawn(content, probe_area, &painted_rows),
                    |_, area| area.height,
                );
                if raw == PROGRESS_SCALE {
                    assert_eq!(grid.motion_heights.piece_heights, [] as [u16; 0]);
                } else {
                    assert_eq!(grid.motion_heights.progress, raw);
                    assert_ne!(grid.motion_heights.piece_heights, [] as [u16; 0]);
                }
                let destination = grid.resolved_grid(area, &grid.drawn_held(), growth);
                assert_eq!(destination.columns, [area]);
                assert_eq!(
                    destination
                        .resolved
                        .panes
                        .iter()
                        .map(|pane| pane.area.height)
                        .collect::<Vec<_>>(),
                    after
                );
                heights.clear();
                heights.extend(
                    grid.drawing_at(area, growth, raw)
                        .placements()
                        .map(|placement| placement.frame.clip().height),
                );
                for (index, (&height, &(start, end))) in heights.iter().zip(&endpoints).enumerate()
                {
                    assert!(
                        (start.min(end)..=start.max(end)).contains(&height),
                        "piece {index} leaves {start}..={end} at {raw}: {heights:?}"
                    );
                    let moved_toward_end = if start <= end {
                        height >= prior[index]
                    } else {
                        height <= prior[index]
                    };
                    assert!(
                        moved_toward_end,
                        "piece {index} moves away from {end} at {raw}: {prior:?} to {heights:?}"
                    );
                }
                if raw == PROGRESS_SCALE {
                    break;
                }
                // Stable probes repeat until the next integer transfer;
                // changing heights still advance on consecutive frames.
                raw = if heights == prior {
                    steps
                        .iter()
                        .copied()
                        .find(|&step| step > raw)
                        .unwrap_or(PROGRESS_SCALE)
                } else {
                    raw.saturating_add(1)
                };
                std::mem::swap(&mut prior, &mut heights);
            }
        }
    }

    #[test]
    fn selected_piece_heights_are_not_reversed_by_absolute_divider_limits() {
        let (mut grid, area, growth) = exact_height_motion(&[8, 8, 8, 8], &[10, 6, 10, 6]);
        let raw = PROGRESS_SCALE / 2;
        let selected = vec![10, 8, 8, 6];
        let unsnapped = grid.drawing_at_with_height_snap(area, growth, raw, &HeightSnap::Off);
        let mut row_usage = Vec::new();
        for (piece, &height) in unsnapped.pieces.iter().zip(&selected) {
            let rect = Rect {
                height,
                ..piece.placement.frame.clip()
            };
            let content_area = draw::content_area(frame_inner(rect));
            measure_frame_row_usage(
                &mut row_usage,
                &piece.placement.content,
                content_area,
                &mut |_, area| area.height,
            );
        }
        grid.frame_row_usage = row_usage;
        grid.motion_heights = MotionHeightPath {
            area,
            growth,
            progress: raw,
            piece_heights: selected.clone(),
        };

        let actual = grid
            .drawing_at(area, growth, raw)
            .placements()
            .map(|placement| placement.frame.clip().height)
            .collect::<Vec<_>>();

        assert_eq!(actual, selected);
    }

    #[test]
    fn a_shrinking_piece_skips_the_height_that_leaves_a_row_unused() {
        let (mut grid, area, growth) = exact_height_motion(&[11, 11, 11, 13], &[15, 7, 15, 9]);
        let raw = PROGRESS_SCALE * 3 / 4;
        let selected = vec![15, 11, 11, 9];
        let unsnapped = grid.drawing_at_with_height_snap(area, growth, raw, &HeightSnap::Off);
        let mut row_usage = Vec::new();
        for (piece_index, piece) in unsnapped.pieces.iter().enumerate() {
            for height in 9..=13 {
                let rect = Rect {
                    height,
                    ..piece.placement.frame.clip()
                };
                let content_area = draw::content_area(frame_inner(rect));
                measure_frame_row_usage(
                    &mut row_usage,
                    &piece.placement.content,
                    content_area,
                    &mut |_, area| {
                        if piece_index == 3 && height == 10 {
                            area.height.saturating_sub(1)
                        } else {
                            area.height
                        }
                    },
                );
            }
        }
        grid.frame_row_usage = row_usage;
        grid.motion_heights = MotionHeightPath {
            area,
            growth,
            progress: raw,
            piece_heights: selected,
        };

        let drawing = grid.drawing_at(area, growth, raw);
        let shrinking = &drawing.pieces[3];
        let rect = shrinking.placement.frame.clip();
        let content_area = draw::content_area(frame_inner(rect));

        assert_eq!(rect.height, 9);
        assert_ne!(
            frame_height_fit(
                &grid.frame_row_usage,
                &shrinking.placement.content,
                content_area,
            ),
            FrameHeightFit::LeavesGap
        );

        let (mut grid, area, growth) = exact_height_motion(&[11, 11, 11, 13], &[15, 7, 15, 9]);
        let mut heights = Vec::new();
        for raw in single_column_height_steps(&grid.drawing_at(area, growth, 0)) {
            grid.measure_row_steps(
                area,
                growth,
                raw,
                |content, area| {
                    let framed = area
                        .height
                        .saturating_add(TILE_BORDER_ROWS)
                        .saturating_add(TILE_ROWS_READOUT_HEIGHT);
                    if content == &TileContent::Group(3) && framed == 10 {
                        area.height.saturating_sub(1)
                    } else {
                        area.height
                    }
                },
                |_, area| area.height,
            );
            let drawing = grid.drawing_at(area, growth, raw);
            let shrinking = drawing
                .placements()
                .find(|placement| placement.content == TileContent::Group(3))
                .expect("the shrinking resident remains in the drawing");
            let rect = shrinking.frame.clip();
            if raw < PROGRESS_SCALE {
                let content_area = draw::content_area(frame_inner(rect));
                assert_ne!(
                    frame_height_fit(&grid.frame_row_usage, &shrinking.content, content_area,),
                    FrameHeightFit::LeavesGap,
                    "the shrinking resident leaves a gap at {raw}: {rect:?}"
                );
            }
            heights.push(rect.height);
        }
        assert!(heights.windows(2).all(|pair| pair[0] >= pair[1]));
        for expected in [13, 12, 11, 9] {
            assert!(heights.contains(&expected), "missing height {expected}");
        }
        assert!(!heights.contains(&10));
    }

    #[test]
    fn a_resize_during_motion_draws_every_column_inside_the_new_area() {
        let old_area = Rect::new(0, 0, 80, 30);
        let new_area = Rect::new(10, 5, 60, 22);
        let growth = redistribute(1);
        let slots = (1..=3).map(Slot::Group).collect::<Vec<_>>();
        let source_held = HeldCellLayout {
            rows:          row_demands(&[8, 8, 8, 8]),
            focused:       FocusLocation::Departing,
            summary_span:  1,
            summary_depth: 1,
        };
        let destination_held = HeldCellLayout {
            rows: row_demands(&[10, 6, 10, 6]),
            ..source_held
        };
        let source = Grid::new(old_area, &source_held, growth, &TileSettings::default());
        let mut grid = TileGrid::new();
        grid.set_view(TileView::Cells);
        grid.set_layout(old_area, growth);
        grid.slots = slots.clone();
        grid.held = destination_held;
        grid.motion = GridMotion::Moving(Transition {
            from:         slots,
            held:         source_held,
            source:       Box::new(source),
            started:      Instant::now(),
            millis:       1,
            depth_status: DestinationDepthStatus::Resolved,
        });
        let _ = filled_height_drawing(
            &mut grid,
            old_area,
            growth,
            PROGRESS_SCALE.saturating_div(4),
        );

        grid.set_layout(new_area, growth);
        for raw in PROGRESS_SCALE.saturating_div(4)..=PROGRESS_SCALE {
            let drawing = filled_height_drawing(&mut grid, new_area, growth, raw);

            assert_eq!(drawing.column_bands.len(), 2);
            for band in &drawing.column_bands {
                assert_eq!(
                    (band.top(), band.bottom()),
                    (new_area.top(), new_area.bottom()),
                    "column outer rows at {raw}"
                );
                assert!(band.left() >= new_area.left(), "column left at {raw}");
                assert!(band.right() <= new_area.right(), "column right at {raw}");
            }
            for placement in drawing.placements() {
                let clip = placement.frame.clip();
                assert!(clip.left() >= new_area.left(), "piece left at {raw}");
                assert!(clip.right() <= new_area.right(), "piece right at {raw}");
                assert!(clip.top() >= new_area.top(), "piece top at {raw}");
                assert!(clip.bottom() <= new_area.bottom(), "piece bottom at {raw}");
            }
        }
    }

    #[test]
    fn returning_to_a_motion_source_area_keeps_the_resized_destination() {
        let source_area = test_area();
        let resized_area = Rect::new(3, 2, TEST_WIDTH - 6, TEST_HEIGHT - 4);
        let growth = redistribute(4);
        let mut grid = synced_motion(&[7, 8, 9], &[7, 9], growth);
        let partial = grid.drawing_at(source_area, growth, PROGRESS_SCALE / 2);
        assert!(
            partial
                .placements()
                .any(|placement| placement.content == TileContent::Group(8)),
            "the departed piece is still visible before the resize"
        );
        assert!(
            grid.progress() < PROGRESS_SCALE,
            "the clock is still running"
        );

        grid.set_draw_layout(resized_area, growth);
        for raw in [0, PROGRESS_SCALE / 2, PROGRESS_SCALE] {
            let drawing = grid.drawing_at(resized_area, growth, raw);
            assert!(
                drawing
                    .placements()
                    .all(|placement| placement.content != TileContent::Group(8)),
                "the departed piece returns in the resized area at {raw}"
            );
            assert_eq!(
                endpoint_rects(&drawing),
                settled_rects(
                    &Grid::new(resized_area, &grid.drawn_held(), growth, &grid.settings),
                    grid.count(),
                ),
                "resized destination at {raw}"
            );
        }

        grid.set_draw_layout(source_area, growth);
        for raw in [0, PROGRESS_SCALE / 2, PROGRESS_SCALE] {
            let drawing = grid.drawing_at(source_area, growth, raw);
            assert!(
                drawing
                    .placements()
                    .all(|placement| placement.content != TileContent::Group(8)),
                "the departed piece returns in the source area at {raw}"
            );
            assert_eq!(
                endpoint_rects(&drawing),
                settled_rects(
                    &Grid::new(source_area, &grid.drawn_held(), growth, &grid.settings),
                    grid.count(),
                ),
                "source-area destination at {raw}"
            );
        }
    }

    #[test]
    fn an_every_other_height_shrink_moves_one_way() {
        assert_snapped_heights_move_one_way(
            &[6, 19, 9],
            &[19, 8, 7],
            &[
                &[6, 8, 10, 12, 14, 16, 18, 19],
                &[8, 10, 12, 14, 16, 18, 19],
                &[7, 9],
            ],
        );
    }

    #[test]
    fn a_height_search_reaches_the_nearest_valid_height_each_way() {
        let (mut grid, area, growth) = exact_height_motion(&[19, 27], &[27, 19]);
        let raw = PROGRESS_SCALE / 2;
        grid.measure_row_steps(
            area,
            growth,
            raw,
            |_, area| match area.height {
                0..=16 => area.height,
                17..=23 => 16,
                24.. => 24,
            },
            |_, area| area.height,
        );
        let heights = grid
            .drawing_at(area, growth, raw)
            .placements()
            .map(|placement| placement.frame.clip().height)
            .collect::<Vec<_>>();

        assert!(
            heights.iter().all(|height| [19, 27].contains(height)),
            "the midpoint uses only the nearest valid endpoint heights: {heights:?}"
        );
    }

    #[test]
    fn a_painted_optional_row_fills_a_moving_height() {
        let content = TileContent::<u32>::Summary;
        let rect = Rect::new(0, 0, 40, 5);
        let area = draw::content_area(frame_inner(rect));
        let mut usage = Vec::new();
        measure_frame_row_usage(&mut usage, &content, area, &mut |_, _| 2);
        let piece = BandPiece {
            column:   0,
            motion:   BandPieceMotion::Resident {
                before: rect,
                after:  rect,
            },
            progress: PROGRESS_SCALE / 2,
            drawn:    CellAppearance {
                identity: match &content {
                    TileContent::Group(id) => CellIdentity::Slot(Slot::Group(*id)),
                    _ => CellIdentity::Summary,
                },
                content,
                focused: false,
            },
        };

        assert_eq!(area.height, 2);
        assert_eq!(
            frame_height_fit(&usage, &piece.drawn.content, area),
            FrameHeightFit::Filled
        );
        assert!(
            HeightSnap::Frame {
                row_usage: &usage,
                path:      PieceHeightPath::AdvanceFrom(&[]),
            }
            .accepts(&piece, rect)
        );
    }

    #[test]
    fn nonmonotone_starting_dividers_keep_every_piece_inside_its_band() {
        let band = Rect::new(43, 79, 40, 14);
        let rects = [
            Rect::new(43, 79, 40, 12),
            Rect::new(43, 85, 40, 2),
            Rect::new(43, 86, 40, 7),
        ];
        let pieces: Vec<_> = rects
            .iter()
            .enumerate()
            .map(|(index, &rect)| BandPiece {
                column:   0,
                motion:   BandPieceMotion::Resident {
                    before: rect,
                    after:  rect,
                },
                progress: 0,
                drawn:    CellAppearance {
                    identity: CellIdentity::Slot(Slot::Group(index)),
                    content:  TileContent::Group(index),
                    focused:  false,
                },
            })
            .collect();
        let held = HeldCellLayout {
            rows: row_demands(&[4; 4]),
            ..HeldCellLayout::default()
        };
        let grid = Grid::new(band, &held, TileGrowth::default(), &TileSettings::default());
        let indices = [0, 1, 2];
        let search = BandHeightSearch {
            before_grid: &grid,
            after_grid: &grid,
            pieces: &pieces,
            in_band: &indices,
            band,
            min_tile_height: 1,
            height_snap: &HeightSnap::Off,
        };
        let dividers = band_piece_dividers(&search, band, band);
        for (position, piece) in pieces.iter().enumerate() {
            let rect = band_piece_rect(
                &grid,
                &grid,
                piece,
                band,
                dividers[position],
                dividers[position + 1],
            );
            let frame = piece_frame(piece.motion, rect, band);
            assert_eq!(rect.intersection(band), rect, "{dividers:?}: {rect:?}");
            assert_eq!(frame.clip().intersection(band), frame.clip());
        }
    }

    /// Draw one fixed progress point after probing a two-row content step.
    fn stepped_height_drawing(
        grid: &mut TileGrid<u32>,
        area: Rect,
        growth: TileGrowth,
        raw: u32,
        calls: &mut usize,
    ) -> TileDrawing<u32> {
        grid.measure_row_steps(
            area,
            growth,
            raw,
            |content, area| {
                *calls = calls.saturating_add(1);
                let fills = match content {
                    TileContent::Summary | TileContent::Group(_) => area.height % 2 == 1,
                    TileContent::Empty(_) => true,
                };
                if fills {
                    area.height
                } else {
                    area.height.saturating_sub(1)
                }
            },
            |_, area| area.height,
        );
        grid.drawing_at(area, growth, raw)
    }

    /// Whether the two-row fixture fills this framed height.
    fn stepped_height_fills(content: &TileContent<u32>, rect: Rect) -> bool {
        let area = draw::content_area(frame_inner(rect));
        area.is_empty()
            || match content {
                TileContent::Summary | TileContent::Group(_) => area.height % 2 == 1,
                TileContent::Empty(_) => true,
            }
    }

    #[test]
    fn a_growing_resident_uses_the_current_frames_whole_row_count() {
        let content = TileContent::<u32>::Summary;
        let rect = Rect::new(0, 0, 40, 10);
        let area = draw::content_area(frame_inner(rect));
        let mut usage = Vec::new();
        measure_frame_row_usage(&mut usage, &content, area, &mut |_, area| {
            if area.height == u16::MAX { 9 } else { 6 }
        });
        let piece = BandPiece {
            column:   0,
            motion:   BandPieceMotion::Resident {
                before: Rect::new(0, 0, 40, 9),
                after:  Rect::new(0, 0, 40, 12),
            },
            progress: PROGRESS_SCALE / 2,
            drawn:    CellAppearance {
                identity: match &content {
                    TileContent::Group(id) => CellIdentity::Slot(Slot::Group(*id)),
                    _ => CellIdentity::Summary,
                },
                content,
                focused: false,
            },
        };

        assert!(
            !HeightSnap::Frame {
                row_usage: &usage,
                path:      PieceHeightPath::AdvanceFrom(&[]),
            }
            .accepts(&piece, rect)
        );
    }

    #[test]
    fn a_widening_resident_uses_its_current_widths_whole_row_count() {
        let content = TileContent::<u32>::Summary;
        let rect = Rect::new(0, 0, 40, 10);
        let area = draw::content_area(frame_inner(rect));
        let mut usage = Vec::new();
        measure_frame_row_usage(&mut usage, &content, area, &mut |_, area| {
            if area.height == u16::MAX { 9 } else { 6 }
        });
        let piece = BandPiece {
            column:   0,
            motion:   BandPieceMotion::Resident {
                before: Rect::new(0, 0, 30, 10),
                after:  Rect::new(0, 0, 50, 10),
            },
            progress: PROGRESS_SCALE / 2,
            drawn:    CellAppearance {
                identity: match &content {
                    TileContent::Group(id) => CellIdentity::Slot(Slot::Group(*id)),
                    _ => CellIdentity::Summary,
                },
                content,
                focused: false,
            },
        };

        assert!(
            !HeightSnap::Frame {
                row_usage: &usage,
                path:      PieceHeightPath::AdvanceFrom(&[]),
            }
            .accepts(&piece, rect)
        );
    }

    #[test]
    fn a_departing_piece_with_no_content_can_collapse() {
        let content = TileContent::Group(1);
        let rect = Rect::new(0, 0, 40, 6);
        let area = draw::content_area(frame_inner(rect));
        let mut usage = Vec::new();
        measure_frame_row_usage(&mut usage, &content, area, &mut |_, _| 0);
        let piece = BandPiece {
            column:   0,
            motion:   BandPieceMotion::Leaving {
                before:  Rect::new(0, 0, 40, 12),
                edge:    BandEdge::Bottom,
                sliding: false,
            },
            progress: PROGRESS_SCALE / 2,
            drawn:    CellAppearance {
                identity: match &content {
                    TileContent::Group(id) => CellIdentity::Slot(Slot::Group(*id)),
                    _ => CellIdentity::Summary,
                },
                content,
                focused: false,
            },
        };

        assert!(
            HeightSnap::Frame {
                row_usage: &usage,
                path:      PieceHeightPath::AdvanceFrom(&[]),
            }
            .accepts(&piece, rect)
        );
    }

    #[test]
    fn a_moving_piece_snaps_to_a_height_its_content_fills() {
        let (mut grid, area, growth) = stepped_height_motion();
        let mut calls = 0;
        for raw in 0..=PROGRESS_SCALE {
            let drawing = stepped_height_drawing(&mut grid, area, growth, raw, &mut calls);
            for placement in drawing.placements() {
                assert!(
                    stepped_height_fills(&placement.content, placement.frame.clip()),
                    "{:?} is not filled at {raw}",
                    placement.frame.clip()
                );
            }
        }
    }

    #[test]
    fn a_whole_moving_piece_gives_spare_rows_to_a_cutting_mate() {
        let compact_height = MIN_TILE_HEIGHT.saturating_mul(2);
        let expanded_height = compact_height.saturating_add(4);
        let (grid, area, growth) = exact_height_motion(
            &[compact_height, expanded_height],
            &[expanded_height, compact_height],
        );
        let whole_rows = draw::content_area(frame_inner(Rect {
            height: compact_height,
            ..area
        }))
        .height;

        for raw in 1..PROGRESS_SCALE {
            let unsnapped = grid.drawing_at_with_height_snap(area, growth, raw, &HeightSnap::Off);
            let mut row_usage = Vec::new();
            for piece in &unsnapped.pieces {
                for height in compact_height..=expanded_height {
                    let content_area = draw::content_area(frame_inner(Rect {
                        height,
                        ..piece.placement.frame.clip()
                    }));
                    measure_frame_row_use(
                        &mut row_usage,
                        &piece.placement.content,
                        content_area,
                        &mut |content, area| match content {
                            TileContent::Summary => whole_rows.min(area.height),
                            TileContent::Group(_) | TileContent::Empty(_) => area.height,
                        },
                        &mut |content, area| match content {
                            TileContent::Summary => whole_rows.min(area.height),
                            TileContent::Group(_) | TileContent::Empty(_) => area.height,
                        },
                    );
                }
            }
            let height_snap = HeightSnap::Frame {
                row_usage: &row_usage,
                path:      PieceHeightPath::AdvanceFrom(&[]),
            };
            let drawing = grid.drawing_at_with_height_snap(area, growth, raw, &height_snap);
            let summary = drawing
                .placements()
                .find(|placement| placement.content == TileContent::Summary)
                .expect("the summary remains in the band");
            let summary_area = draw::content_area(frame_inner(summary.frame.clip()));
            assert_eq!(
                summary_area.height, whole_rows,
                "the summary keeps a spare row while its mate cuts at {raw}"
            );
        }
    }

    #[test]
    fn a_summary_shrink_starts_filled_and_never_reverses() {
        let (mut grid, area, growth) = exact_height_motion(&[10, 8], &[6, 12]);
        let mut heights = Vec::new();
        for raw in 0..=PROGRESS_SCALE {
            grid.measure_row_steps(
                area,
                growth,
                raw,
                |content, area| match content {
                    TileContent::Summary if area.height == u16::MAX => 15,
                    TileContent::Summary if [5, 7].contains(&area.height) => {
                        area.height.saturating_sub(1)
                    },
                    TileContent::Summary | TileContent::Group(_) => area.height,
                    TileContent::Empty(_) => 0,
                },
                |content, area| match content {
                    TileContent::Summary if [5, 7].contains(&area.height) => {
                        area.height.saturating_sub(1)
                    },
                    TileContent::Summary | TileContent::Group(_) => area.height,
                    TileContent::Empty(_) => 0,
                },
            );
            let drawing = grid.drawing_at(area, growth, raw);
            let summary = drawing
                .pieces
                .iter()
                .find(|piece| piece.placement.content == TileContent::Summary)
                .expect("the summary remains in the shrinking band");
            let content_height =
                draw::content_area(frame_inner(summary.placement.frame.clip())).height;
            let rows = if [5, 7].contains(&content_height) {
                content_height.saturating_sub(1)
            } else {
                content_height
            };
            assert_eq!(
                rows, content_height,
                "summary leaves a row at progress {raw}"
            );
            heights.push(summary.placement.frame.clip().height);
        }
        assert_eq!(heights.first().copied(), Some(9));
        assert!(
            heights.windows(2).all(|pair| pair[0] >= pair[1]),
            "the shrinking summary reverses through {heights:?}"
        );
    }

    #[test]
    fn a_content_change_keeps_residents_at_their_last_filled_heights() {
        let (mut grid, area, growth) = exact_height_motion(&[19, 27], &[27, 19]);
        let mut last_heights = vec![19, 27];
        for raw in 0..=PROGRESS_SCALE {
            let valid_heights = (PROGRESS_SCALE / 2..PROGRESS_SCALE)
                .contains(&raw)
                .then(|| last_heights.clone());
            grid.measure_row_steps(
                area,
                growth,
                raw,
                |content, area| {
                    if area.height == u16::MAX {
                        return TEST_HEIGHT;
                    }
                    let Some(valid_heights) = &valid_heights else {
                        return area.height;
                    };
                    let index = match content {
                        TileContent::Summary => 0,
                        TileContent::Group(id) => usize::try_from(*id).unwrap_or(usize::MAX),
                        TileContent::Empty(_) => return 0,
                    };
                    let framed = area
                        .height
                        .saturating_add(TILE_BORDER_ROWS)
                        .saturating_add(TILE_ROWS_READOUT_HEIGHT);
                    if valid_heights.get(index).copied() == Some(framed) {
                        area.height
                    } else {
                        area.height.saturating_sub(1)
                    }
                },
                |_, area| area.height,
            );
            let drawing = grid.drawing_at(area, growth, raw);
            let heights = drawing
                .placements()
                .map(|placement| placement.frame.clip().height)
                .collect::<Vec<_>>();
            if let Some(valid_heights) = valid_heights {
                assert_eq!(
                    heights, valid_heights,
                    "a resident leaves its last filled height at {raw}"
                );
            }
            last_heights = heights;
        }
    }

    #[test]
    fn a_snapped_height_moves_one_way_through_a_motion() {
        let (mut grid, area, growth) = stepped_height_motion();
        let mut calls = 0;
        let contents = [
            TileContent::Summary,
            TileContent::Group(1),
            TileContent::Group(2),
        ];
        let mut heights = vec![Vec::new(); contents.len()];
        for raw in 0..=PROGRESS_SCALE {
            let drawing = stepped_height_drawing(&mut grid, area, growth, raw, &mut calls);
            for (content, heights) in contents.iter().zip(&mut heights) {
                if let Some(placement) = drawing
                    .placements()
                    .find(|placement| &placement.content == content)
                {
                    heights.push(placement.frame.clip().height);
                }
            }
        }
        for (content, heights) in contents.iter().zip(heights) {
            let Some((&start, &finish)) = heights.first().zip(heights.last()) else {
                continue;
            };
            let one_way = if start <= finish {
                heights.windows(2).all(|pair| pair[0] <= pair[1])
            } else {
                heights.windows(2).all(|pair| pair[0] >= pair[1])
            };
            assert!(one_way, "{content:?} reverses through {heights:?}");
            let start_endpoint = grid
                .drawing_at(area, growth, 0)
                .placements()
                .find(|placement| &placement.content == content)
                .map(|placement| placement.frame.clip().height);
            let finish_endpoint = grid
                .drawing_at(area, growth, PROGRESS_SCALE)
                .placements()
                .find(|placement| &placement.content == content)
                .map(|placement| placement.frame.clip().height);
            if let (Some(start_endpoint), Some(finish_endpoint)) = (start_endpoint, finish_endpoint)
            {
                assert_eq!(start, start_endpoint);
                assert_eq!(finish, finish_endpoint);
            }
        }
    }

    #[test]
    fn a_column_still_tiles_its_band_while_pieces_snap() {
        let (mut grid, area, growth) = stepped_height_motion();
        let mut calls = 0;
        for raw in 0..=PROGRESS_SCALE {
            let drawing = stepped_height_drawing(&mut grid, area, growth, raw, &mut calls);
            for &band in &drawing.column_bands {
                let pieces = drawing
                    .placements()
                    .map(|placement| placement.frame.clip())
                    .filter(|piece| piece.x == band.x && piece.width == band.width)
                    .collect::<Vec<_>>();
                assert_eq!(pieces.first().map(|piece| piece.top()), Some(band.top()));
                assert_eq!(
                    pieces.last().map(|piece| piece.bottom()),
                    Some(band.bottom())
                );
                assert!(
                    pieces
                        .windows(2)
                        .all(|pair| pair[0].bottom().saturating_sub(1) == pair[1].top())
                );
            }
        }
    }

    #[test]
    fn two_stepped_mates_both_stand_at_heights_they_fill() {
        let (mut grid, area, growth) = stepped_height_motion();
        let mut calls = 0;
        for raw in 0..=PROGRESS_SCALE {
            let drawing = stepped_height_drawing(&mut grid, area, growth, raw, &mut calls);
            for content in [TileContent::Summary, TileContent::Group(1)] {
                let rect = drawing
                    .placements()
                    .find(|placement| placement.content == content)
                    .map(|placement| placement.frame.clip())
                    .expect("both resident mates remain in the band");
                assert!(
                    rect.height >= MIN_TILE_HEIGHT,
                    "{content:?} at {raw}: {rect:?}"
                );
                assert!(
                    stepped_height_fills(&content, rect),
                    "{content:?} at {raw}: {rect:?}"
                );
            }
        }
    }

    #[test]
    fn arriving_and_leaving_pieces_snap_without_a_floor() {
        let (mut arriving, area, growth) = stepped_height_motion();
        let mut calls = 0;
        let mut arrival_below_floor = false;
        for raw in arriving.distinct_motion_steps_for_test(area, growth) {
            let drawing = stepped_height_drawing(&mut arriving, area, growth, raw, &mut calls);
            let rect = drawing
                .placements()
                .find(|placement| placement.content == TileContent::Group(2))
                .map(|placement| placement.frame.clip());
            assert!(rect.is_some(), "the arrival has one piece at {raw}");
            let rect = rect.unwrap_or_default();
            arrival_below_floor |= rect.height < MIN_TILE_HEIGHT;
            assert!(
                stepped_height_fills(&TileContent::Group(2), rect),
                "arrival at {raw}: {rect:?}"
            );
        }
        assert!(
            arrival_below_floor,
            "the arrival may collapse under the floor"
        );

        let mut leaving = TileGrid::new();
        leaving.set_view(TileView::Cells);
        leaving.set_layout(area, growth);
        leaving.sync(&tall_demands(&[1, 2]), growth);
        leaving.settle_for_test();
        leaving.collapse_queued_changes_for_test();
        leaving.sync(&tall_demands(&[1]), growth);
        let mut departure_below_floor = false;
        for raw in leaving.distinct_motion_steps_for_test(area, growth) {
            let drawing = stepped_height_drawing(&mut leaving, area, growth, raw, &mut calls);
            let rect = drawing
                .placements()
                .find(|placement| placement.content == TileContent::Group(2))
                .map(|placement| placement.frame.clip())
                .expect("the departure has one piece");
            departure_below_floor |= rect.height < MIN_TILE_HEIGHT;
            assert!(
                stepped_height_fills(&TileContent::Group(2), rect),
                "departure at {raw}: {rect:?}"
            );
        }
        assert!(
            departure_below_floor,
            "the departure may collapse under the floor"
        );

        let before = [1, 2, 3, 4, 5, 6];
        let after = [1, 2, 3, 4, 6];
        let closing_growth = redistribute(3);
        let mut closing = synced_motion(&before, &after, closing_growth);
        let start = closing.drawing_at(area, closing_growth, 0);
        let column = start.column_bands.len().saturating_sub(1);
        let start_band = start.column_bands[column];
        let starting_rows = start
            .placements()
            .filter(|placement| {
                let clip = placement.frame.clip();
                clip.x == start_band.x && clip.width == start_band.width
            })
            .map(|placement| {
                let clip = placement.frame.clip();
                (clip.y, clip.height)
            })
            .collect::<Vec<_>>();
        let middle = stepped_height_drawing(
            &mut closing,
            area,
            closing_growth,
            PROGRESS_SCALE / 2,
            &mut calls,
        );
        let middle_band = middle.column_bands[column];
        let middle_rows = middle
            .placements()
            .filter(|placement| {
                let clip = placement.frame.clip();
                clip.x == middle_band.x && clip.width == middle_band.width
            })
            .map(|placement| {
                let clip = placement.frame.clip();
                (clip.y, clip.height)
            })
            .collect::<Vec<_>>();
        assert_eq!(
            middle_rows, starting_rows,
            "the closing column keeps its rows"
        );
    }

    #[test]
    fn a_large_cell_column_measures_each_allocation_once() {
        for width in LARGE_CONTENT_TEST_WIDTHS {
            for rows in LARGE_CONTENT_TEST_ROWS {
                let area = Rect::new(0, 0, width, LARGE_CONTENT_TEST_HEIGHT);
                let held = HeldCellLayout {
                    rows:          row_demands(&[rows; LARGE_CONTENT_TEST_CELLS]),
                    focused:       FocusLocation::Departing,
                    summary_span:  1,
                    summary_depth: 1,
                };
                let mut queries = Vec::new();
                let grid = Grid::with_row_use(
                    area,
                    &held,
                    add_new(LARGE_CONTENT_TEST_INITIAL_ROWS),
                    &TileSettings::default(),
                    &mut |index, area, purpose| {
                        let retained = matches!(purpose, RowUsePurpose::RetainContent);
                        let key = (index, area.width, area.height, retained);
                        assert!(
                            !queries.contains(&key),
                            "repeated column allocation: {key:?}"
                        );
                        queries.push(key);
                        if index == 0 {
                            area.height.min(rows) / LARGE_CONTENT_TEST_ROW_STEP
                                * LARGE_CONTENT_TEST_ROW_STEP
                        } else {
                            area.height.min(1)
                        }
                    },
                );
                assert_eq!(grid.resolved.panes.len(), held.rows.len());
                assert!(
                    (0..held.rows.len())
                        .all(|index| queries.iter().any(|(known, ..)| *known == index))
                );
            }
        }
    }

    #[test]
    fn a_seven_cell_moving_frame_limits_row_probes() {
        let area = Rect::new(0, 0, 200, 50);
        let growth = redistribute(12);
        let demands = |ids: RangeInclusive<u32>| TileDemands {
            summary:       100,
            summary_width: 0,
            groups:        ids.map(|id| TileDemand { id, rows: 100 }).collect(),
        };
        let mut grid = TileGrid::new();
        grid.set_view(TileView::Cells);
        grid.set_min_tile_width(40);
        grid.set_min_tile_height(6);
        grid.set_layout(area, growth);
        grid.sync(&demands(1..=5), growth);
        grid.settle_for_test();
        grid.collapse_queued_changes_for_test();
        grid.sync(&demands(1..=6), growth);
        let endpoint_areas = [0, PROGRESS_SCALE]
            .into_iter()
            .flat_map(|raw| grid.drawing_at(area, growth, raw).pieces)
            .map(|piece| {
                let area = draw::content_area(frame_inner(piece.placement.frame.clip()));
                (piece.placement.content, area.width, area.height)
            })
            .collect::<Vec<_>>();
        let mut drawn_calls = 0_usize;
        let mut kept_calls = 0_usize;
        let raw = PROGRESS_SCALE / 2;
        grid.measure_row_steps(
            area,
            growth,
            raw,
            |content, area| {
                drawn_calls += 1;
                if area.height == u16::MAX {
                    return match content {
                        TileContent::Summary => MIN_TILE_HEIGHT
                            .saturating_sub(TILE_BORDER_ROWS)
                            .saturating_sub(TILE_ROWS_READOUT_HEIGHT),
                        TileContent::Group(_) | TileContent::Empty(_) => area.height,
                    };
                }
                if endpoint_areas.iter().any(|(endpoint, width, height)| {
                    endpoint == content && *width == area.width && *height == area.height
                }) {
                    area.height
                } else {
                    area.height.saturating_sub(1)
                }
            },
            |_, area| {
                kept_calls += 1;
                area.height
            },
        );
        let _ = grid.drawing_at(area, growth, raw);
        assert_eq!(
            drawn_calls.saturating_add(kept_calls),
            SEVEN_CELL_MOVING_ROW_PROBE_COUNT
        );
    }

    #[test]
    fn a_settled_frame_measures_each_cell_size_once_and_requests_no_frame() {
        let area = Rect::new(0, 0, 200, 50);
        let growth = redistribute(12);
        let demands = TileDemands {
            summary:       100,
            summary_width: 0,
            groups:        (1..=6).map(|id| TileDemand { id, rows: 100 }).collect(),
        };
        let mut grid = TileGrid::new();
        grid.set_view(TileView::Cells);
        grid.set_min_tile_width(40);
        grid.set_min_tile_height(6);
        grid.set_layout(area, growth);
        grid.sync(&demands, growth);
        grid.settle_for_test();

        let mut drawn = Vec::new();
        let mut kept = Vec::new();
        grid.measure_row_steps(
            area,
            growth,
            PROGRESS_SCALE,
            |content, area| {
                drawn.push((content.clone(), area));
                area.height
            },
            |content, area| {
                kept.push((content.clone(), area));
                area.height
            },
        );
        assert!(kept.iter().all(|probe| drawn.contains(probe)));
        assert!(
            drawn
                .iter()
                .enumerate()
                .all(|(index, probe)| !drawn[index + 1..].contains(probe))
        );
        assert!(
            kept.iter()
                .enumerate()
                .all(|(index, probe)| !kept[index + 1..].contains(probe))
        );
        assert_eq!(drawn.len(), SEVEN_CELL_SETTLED_DRAWN_PROBE_COUNT);
        assert_eq!(kept.len(), SEVEN_CELL_SETTLED_KEPT_PROBE_COUNT);

        grid.sync(&demands, growth);
        drawn.clear();
        kept.clear();
        grid.measure_row_steps(
            area,
            growth,
            PROGRESS_SCALE,
            |content, area| {
                drawn.push((content.clone(), area));
                area.height
            },
            |content, area| {
                kept.push((content.clone(), area));
                area.height
            },
        );

        assert!(kept.iter().all(|probe| drawn.contains(probe)));
        assert!(
            drawn
                .iter()
                .enumerate()
                .all(|(index, probe)| !drawn[index + 1..].contains(probe))
        );
        assert!(
            kept.iter()
                .enumerate()
                .all(|(index, probe)| !kept[index + 1..].contains(probe))
        );
        assert_eq!(drawn.len(), SEVEN_CELL_SETTLED_DRAWN_PROBE_COUNT);
        assert_eq!(kept.len(), SEVEN_CELL_SETTLED_KEPT_PROBE_COUNT);
        assert!(matches!(grid.motion, GridMotion::Settled));
        assert!(!grid.tick());
    }

    #[test]
    fn changed_retained_rows_update_the_next_settled_transfer() {
        let area = Rect::new(0, 0, TEST_WIDTH, 29);
        let growth = add_new(5);
        let mut grid = TileGrid::new();
        grid.set_view(TileView::Cells);
        grid.set_layout(area, growth);
        grid.slots = (1..=4).map(Slot::Group).collect();
        grid.held = HeldCellLayout {
            rows:          row_demands(&[17, 11, 15, 15, 12]),
            focused:       FocusLocation::Departing,
            summary_span:  1,
            summary_depth: 1,
        };
        grid.measure_row_steps(
            area,
            growth,
            PROGRESS_SCALE,
            |_, area| area.height,
            |_, area| area.height,
        );
        let before = grid
            .resolved_grid(area, &grid.drawn_held(), growth)
            .cell(TABLE_CELL)
            .expect("the first pass places the summary")
            .height;

        grid.measure_row_steps(
            area,
            growth,
            PROGRESS_SCALE,
            |_, area| area.height,
            |content, area| match content {
                TileContent::Summary | TileContent::Group(2 | 3) if area.height == 4 => {
                    area.height.saturating_sub(1)
                },
                TileContent::Summary | TileContent::Group(_) | TileContent::Empty(_) => area.height,
            },
        );
        let after = grid
            .resolved_grid(area, &grid.drawn_held(), growth)
            .cell(TABLE_CELL)
            .expect("the refreshed pass places the summary")
            .height;

        assert!(after > before, "the summary receives a newly freed row");
        assert!(!grid.tick());
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
    fn a_motion_frame_applies_unused_rows_to_its_own_arrangement() {
        let area = Rect::new(0, 0, TEST_WIDTH, 40);
        let growth = redistribute(4);
        let demands = |groups: &[u32]| TileDemands {
            summary:       100,
            summary_width: 0,
            groups:        groups
                .iter()
                .map(|&id| TileDemand { id, rows: 100 })
                .collect(),
        };
        let mut grid = TileGrid::new();
        grid.set_layout(area, growth);
        grid.sync(&demands(&[1, 2, 3]), growth);
        grid.settle_for_test();
        grid.sync(&demands(&[1, 3]), growth);
        let GridMotion::Moving(transition) = &grid.motion else {
            assert!(
                matches!(&grid.motion, GridMotion::Moving(_)),
                "the removed cell starts one motion"
            );
            return;
        };
        let source = Grid::new(area, &transition.held, growth, &grid.settings)
            .cell(TABLE_CELL)
            .expect("the source places the summary");

        grid.measure_row_steps(
            area,
            growth,
            PROGRESS_SCALE / 2,
            |content, area| match content {
                TileContent::Group(3) => area.height.saturating_sub(1),
                TileContent::Summary | TileContent::Group(_) | TileContent::Empty(_) => area.height,
            },
            |content, area| match content {
                TileContent::Group(3) => area.height.saturating_sub(1),
                TileContent::Summary | TileContent::Group(_) | TileContent::Empty(_) => area.height,
            },
        );

        let destination = grid
            .resolved_grid(area, &grid.drawn_held(), growth)
            .cell(TABLE_CELL)
            .expect("the destination places the summary");
        let summary_at = |raw| {
            grid.drawing_at(area, growth, raw)
                .pieces
                .into_iter()
                .find(|piece| piece.placement.content == TileContent::Summary)
                .map(|piece| piece.placement.frame.rect())
                .expect("the frame places the summary")
        };

        assert_eq!(summary_at(0), source);
        assert_eq!(summary_at(PROGRESS_SCALE), destination);
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
    fn a_resize_during_queued_closings_settles_focus_and_keeps_navigation_working() {
        let growth = redistribute(4);
        let mut grid = seeded_grid();
        grid.sync(&quiet(&[1, 2, 3]), growth);
        grid.settle_for_test();
        grid.focus_cell(TABLE_CELL + 3);
        grid.settle_for_test();

        grid.sync(&quiet(&[1]), growth);
        assert!(matches!(grid.motion, GridMotion::Moving(_)));
        assert!(!grid.pending.is_empty());
        assert_eq!(grid.focus, Focus::Cell(Slot::Group(3)));

        let area = Rect {
            width: grid.area.width - 1,
            ..grid.area
        };
        grid.set_layout(area, growth);
        assert_eq!(shown(&grid), vec![TileContent::Group(1)]);
        assert_eq!(
            grid.focus,
            Focus::Summary,
            "the removed group releases focus"
        );
        assert_eq!(grid.held.focused, FocusLocation::Cell(TABLE_CELL));
        assert!(grid.pending.is_empty());
        assert!(matches!(grid.motion, GridMotion::Settled));

        assert!(
            grid.cycle_focus(CycleDirection::Next),
            "Tab reaches a live cell"
        );
        assert_eq!(grid.focus, Focus::Cell(Slot::Group(1)));
        grid.apply(TileAction::FocusUp, growth);
        assert_eq!(grid.focus, Focus::Summary, "arrows reach the summary");
        grid.apply(TileAction::FocusDown, growth);
        assert_eq!(grid.focus, Focus::Cell(Slot::Group(1)));
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

        fn held_at_depth(rows: &[u16], summary_depth: usize) -> HeldCellLayout {
            HeldCellLayout {
                rows: row_demands(rows),
                focused: FocusLocation::Cell(TABLE_CELL),
                summary_span: 1,
                summary_depth,
            }
        }

        fn deep_grid() -> TileGrid<u32> {
            let mut grid = seeded_grid();
            let mut demands = busy(&[(7, 7), (8, 7), (9, 7)]);
            demands.summary = 15;
            grid.sync(&demands, redistribute(4));
            grid.settle_for_test();
            grid
        }

        fn evidence_grid(summary: usize) -> TileGrid<u32> {
            let growth = redistribute(4);
            let area = Rect {
                height: 47,
                ..test_area()
            };
            let demands = TileDemands {
                summary,
                summary_width: 0,
                groups: (1..=10).map(|id| TileDemand { id, rows: 10 }).collect(),
            };
            let mut grid = TileGrid::new();
            grid.set_min_tile_height(6);
            grid.set_layout(area, growth);
            grid.sync(&demands, growth);
            grid.settle_for_test();
            grid
        }

        #[test]
        fn a_summary_one_row_short_takes_it_from_its_mates() {
            let grid = evidence_grid(12);
            let area = Rect {
                height: 47,
                ..test_area()
            };
            let settled = Grid::new(area, &grid.held, redistribute(4), &grid.settings);
            let rows: Vec<u16> = (TABLE_CELL..=TABLE_CELL + 3)
                .map(|cell| {
                    settled
                        .cell(cell)
                        .expect("the first column holds four pieces")
                        .height
                })
                .collect();

            assert_eq!(grid.depth, 1);
            assert_eq!(rows, vec![14, 12, 12, 12]);
            assert!(
                rows.iter()
                    .all(|&height| height >= grid.settings.min_tile_height)
            );
        }

        #[test]
        fn a_summary_far_short_still_takes_a_second_position() {
            let grid = evidence_grid(20);
            let slot = apportion_shared_run(&[1; 4], 47)[0].saturating_add(1);
            let asked = grid.held.rows[0].exact_framed();

            assert_eq!(asked, slot.saturating_add(slot.saturating_mul(3) / 4));
            assert_eq!(grid.depth, 2);
        }

        #[test]
        fn summary_depth_never_falls_as_the_summary_asks_for_more() {
            let growth = redistribute(4);
            for command_count in [6, 9, 10, 13] {
                let mut previous = 1;
                let mut grid = TileGrid::new();
                grid.set_layout(test_area(), growth);
                for summary in 1..=usize::from(TEST_HEIGHT) {
                    let demands = TileDemands {
                        summary,
                        summary_width: 0,
                        groups: (1..=command_count)
                            .map(|id| TileDemand { id, rows: 10 })
                            .collect(),
                    };
                    grid.sync(&demands, growth);
                    grid.settle_for_test();
                    assert!(
                        grid.depth >= previous,
                        "{command_count} commands fell from {previous} to {} at {summary}",
                        grid.depth
                    );
                    previous = grid.depth;
                }
            }
        }

        #[test]
        fn no_mate_goes_under_the_minimum_to_feed_the_summary() {
            let floor = 6;
            let settings = TileSettings {
                min_tile_height: floor,
                ..TileSettings::default()
            };
            let held = held_at_depth(&[8, u16::MAX, u16::MAX], 1);
            for (height, expected) in [(18, [8, 6, 6]), (19, [8, 7, 6])] {
                let area = Rect::new(0, 0, TEST_WIDTH, height);
                let grid = Grid::new(area, &held, add_new(3), &settings);
                let rows: Vec<u16> = (TABLE_CELL..=TABLE_CELL + 2)
                    .map(|cell| {
                        grid.cell(cell)
                            .expect("the column holds three pieces")
                            .height
                    })
                    .collect();

                assert_eq!(rows, expected);
                assert!(rows[1] >= floor, "the shared-border mate is {rows:?}");
                assert!(rows[2] >= floor, "the bottom mate is {rows:?}");
            }
        }

        #[test]
        fn the_summary_counts_as_the_fewest_cells_that_hold_it() {
            let settings = TileSettings::default();
            for (asked, depth, rows) in [
                (9, 1, 11),
                (14, 1, 14),
                (30, 2, 30),
                (35, 2, 35),
                (45, 3, 40),
            ] {
                let wants = row_demands(&[asked, 10, 10, 10]);
                assert_eq!(
                    summary_depth(test_area(), &wants, redistribute(4), &settings),
                    depth
                );
                assert_eq!(
                    summary_share(
                        &wants,
                        TEST_HEIGHT,
                        depth,
                        redistribute(4),
                        MIN_TILE_HEIGHT,
                        test_area(),
                        &TileSettings::default()
                    ),
                    rows
                );
            }

            let idle = row_demands(&[30, 3, 3, 3]);
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
            demands.summary = 36;
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
                let held = held_at_depth(&[10; 4], depth);
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
            let held = held_at_depth(&[30, 10, 10, 10], 2);
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
            let held = held_at_depth(&[30, 10, 10, 10], 7);
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
            let wants = row_demands(&[u16::MAX, MIN_TILE_HEIGHT]);
            assert!(fits(area, 2, growth, &settings));
            assert!(!fits(area, 3, growth, &settings));
            assert_eq!(summary_depth(area, &wants, growth, &settings), 1);
            assert!(
                summary_share(
                    &wants,
                    area.height,
                    1,
                    growth,
                    MIN_TILE_HEIGHT,
                    area,
                    &TileSettings::default()
                ) < wants[0].stepped_framed()
            );
        }

        #[test]
        fn the_summary_is_served_before_the_focus_ring() {
            let wants = row_demands(&[26, 35, 0]);
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
            assert_eq!(outside[0], PaneAxisSize::Fixed(25));
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
            let starting_depth = grid.depth;

            let growth = redistribute(4);
            grid.apply(TileAction::Add, growth);
            let expected = summary_depth(
                test_area(),
                &cell_wants(&grid.demands, &grid.target(), &grid.settings),
                growth,
                &grid.settings,
            );
            assert!(expected < starting_depth);
            assert_eq!(grid.depth, expected);
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
            demands.summary = 36;
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
                &held_at_depth(&[10; 4], 1),
                redistribute(4),
                &settings,
            );
            let after = Grid::new(
                area,
                &held_at_depth(&[30, 10, 10, 10], 2),
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
                        identity: CellIdentity::Slot(Slot::Empty(2)),
                        content:  TileContent::Empty(2),
                        focused:  false,
                    },
                    &mut pieces,
                );
                let placed = place_band_pieces(&before, &after, &bands, pieces, &HeightSnap::Off);
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
                demands.summary = 15;
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
            demands.summary = 15;
            demands.summary_width = u16::MAX;
            grid.sync(&demands, growth);
            grid.settle_for_test();

            let layout = Grid::new(test_area(), &grid.held, growth, &grid.settings);
            assert_eq!(grid.depth, 2);
            assert_eq!(layout.widths, vec![3, 2]);
            assert_eq!(grid.held.summary_span, 2);
            assert_eq!(layout.span, 2);

            demands.summary = 36;
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
            let resolved = match column_layout(
                capped.count(),
                capped.depth,
                growth,
                capped.area,
                &capped.settings,
            ) {
                ColumnPlan::Columns(layout) => Some(layout),
                ColumnPlan::InsufficientWidth | ColumnPlan::InsufficientRows => None,
            }
            .expect("the test area admits columns");
            assert_eq!(resolved.columns.widths.len(), 1);
            assert!(resolved.columns.summary_depth < capped.depth);
            assert_eq!(capped.wanted_span(), resolved.columns.widths.len());
        }
    }
}
