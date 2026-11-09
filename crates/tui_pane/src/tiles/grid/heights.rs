use super::BandPiece;
use super::CellRowDemand;
use super::FilledSpan;
use super::FrameCellRowUsage;
use super::FrameHeightFit;
use super::Grid;
use super::Rect;
use super::ResolvedPane;
use super::RetainedRows;
use super::TABLE_CELL;
use super::TileContent;
use super::TileGrowth;
use super::draw;
use super::frame_height_fit;
use super::frame_inner;
use super::frame_painted_rows;
use super::frame_retained_rows;
use super::measure_frame_row_use;
use super::measured_filled_span;
/// Which row-usage pass controls a moving frame's divider placement.
pub(super) enum HeightSnap<'a, Id> {
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
pub(super) enum PieceHeightPath<'a> {
    /// Move from the preceding frame's heights toward the destination.
    AdvanceFrom(&'a [u16]),
    /// Reproduce the heights selected for the frame being drawn.
    DrawSelected(&'a [u16]),
}

impl<Id: Eq> HeightSnap<'_, Id> {
    /// Whether a measured content step cannot give up a single row.
    pub(super) fn has_content_step_gap(&self, content: &TileContent<Id>) -> bool {
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
    pub(super) fn fit(&self, piece: &BandPiece<Id>, rect: Rect) -> FrameHeightFit {
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
    pub(super) fn accepts(&self, piece: &BandPiece<Id>, rect: Rect) -> bool {
        self.fit(piece, rect) != FrameHeightFit::LeavesGap
    }

    /// Rows a whole piece can hand to a cutting mate.
    pub(super) fn spare_rows(&self, piece: &BandPiece<Id>, rect: Rect) -> u16 {
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
    pub(super) fn previous_height(&self, index: usize) -> Option<u16> {
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
    pub(super) fn selected_height(&self, index: usize) -> Option<u16> {
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
pub(super) struct MotionHeightPath {
    /// Grid area whose moving pieces these heights describe.
    pub(super) area:          Rect,
    /// Column-growth rule whose moving pieces these heights describe.
    pub(super) growth:        TileGrowth,
    /// Last progress value whose heights were recorded.
    pub(super) progress:      u32,
    /// Framed heights in the stable order produced by the motion.
    pub(super) piece_heights: Vec<u16>,
}

/// A content retarget's filled endpoint, retained until its inputs change.
#[derive(Clone, Default, Eq, PartialEq)]
pub(super) enum RetargetedMotionDestination {
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
pub(super) struct DestinationRowUse {
    pub(super) drawn: u16,
    pub(super) kept:  u16,
}

/// Read a destination's row answers from the current frame's shared probes.
pub(super) fn measure_destination_rows<Id: Clone + Eq>(
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
pub(super) struct MotionSpanBounds {
    /// Least permitted span, including a growing piece's last drawn allocation.
    pub(super) smallest: u16,
    /// Greatest permitted span, including a shrinking piece's last drawn allocation.
    pub(super) largest:  u16,
}

/// Whether a new destination needs constraints from unchanged moving pieces.
pub(super) enum DestinationHeightBounds {
    /// The ordinary destination already preserves each drawn direction.
    Ordinary,
    /// Every pane's remaining span interval when a neighbour must keep its rows.
    Constrained(Vec<MotionSpanBounds>),
}

/// Content and span constraints shared by a retargeted endpoint's columns.
pub(super) struct RetargetedHeightConstraints<'a, Id> {
    /// Every pane's content in logical cell order.
    pub(super) contents:        &'a [TileContent<Id>],
    /// Current content and frame demands in logical cell order.
    pub(super) row_demands:     &'a [CellRowDemand],
    /// Which logical cells have changed their exact row demand.
    pub(super) changed_demands: &'a [bool],
    /// Remaining span bounds in resolved pane order.
    pub(super) bounds:          &'a [MotionSpanBounds],
}

/// Whether every column can fit the retarget's height constraints.
#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum FilledColumnFit {
    /// The shared allocations fill each column within its bounds.
    Fits,
    /// No filled allocation meets the height constraints.
    NoFit,
}

/// Share a retarget's column rows while keeping every permitted content step.
pub(super) fn fill_retargeted_columns<Id: Clone + Eq>(
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

/// Cell geometry from the last settled frame that drew the cells view.
#[derive(Default)]
pub(super) enum LastSettledCellGeometry {
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
