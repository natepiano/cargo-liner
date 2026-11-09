use super::CellAppearance;
use super::FrameHeightFit;
use super::Grid;
use super::HeightSnap;
use super::MIN_TILE_HEIGHT;
use super::Ordering;
use super::PROGRESS_SCALE;
use super::PaneFrame;
use super::PieceHeightPath;
use super::PieceHeightSnap;
use super::PieceName;
use super::Rect;
use super::TileContent;
use super::TilePiece;
use super::draw;
use super::frame_inner;
use super::frame_painted_rows;
use super::lerp;
use super::lerp_rect;
/// Where one cell stands at the two ends of a transition.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum CellTransition {
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
pub(super) enum BandEdge {
    /// The first frame row in the column band.
    Top,
    /// The last frame row in the column band.
    Bottom,
}

/// How one ordered piece occupies a column band through a move.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum BandPieceMotion {
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
pub(super) struct BandPiece<Id> {
    /// Its zero-based column.
    pub(super) column:   usize,
    /// How its two row boundaries move.
    pub(super) motion:   BandPieceMotion,
    /// How far this piece's row boundary has moved.
    pub(super) progress: u32,
    /// What the piece draws.
    pub(super) drawn:    CellAppearance<Id>,
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
pub(super) enum Turns {
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
    pub(super) fn of(before: &Grid, after: &Grid) -> Self {
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
    pub(super) fn summary(self, progress: u32) -> u32 {
        match self {
            Self::Together => progress,
            Self::Widening { .. } => second_half(progress),
            Self::Narrowing { .. } => first_half(progress),
        }
    }

    /// How far through their turn the cells under the summary's moving
    /// edge are at `progress`.
    pub(super) fn covered(self, progress: u32) -> u32 {
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
    pub(super) fn covers(self, before: &Grid, after: &Grid, transition: CellTransition) -> bool {
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
pub(super) fn moving_cell<Id: Clone>(
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
pub(super) fn column_bands(before: &Grid, after: &Grid, progress: u32) -> Vec<Rect> {
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
pub(super) fn column_band(before: &Grid, after: &Grid, column: usize, progress: u32) -> Rect {
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
    pub(super) const fn row(self, band: Rect) -> u16 {
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
pub(super) fn place_band_pieces<Id: Clone + Eq>(
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
            min_tile_height: after.rules.min_tile_height.get(),
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
pub(super) fn band_piece_rect<Id>(
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
pub(super) struct BandHeightSearch<'a, Id> {
    /// The arrangement from which the pieces move.
    pub(super) before_grid:     &'a Grid,
    /// The arrangement toward which the pieces move.
    pub(super) after_grid:      &'a Grid,
    /// Every piece across all bands.
    pub(super) pieces:          &'a [BandPiece<Id>],
    /// Indices of the pieces resident in this band.
    pub(super) in_band:         &'a [usize],
    /// The band's interpolated bounds.
    pub(super) band:            Rect,
    /// The framed height retained by resident pieces.
    pub(super) min_tile_height: u16,
    /// Row-use answers gathered for the current frame.
    pub(super) height_snap:     &'a HeightSnap<'a, Id>,
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
pub(super) fn band_piece_dividers<Id: Eq>(
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
    pub(super) fn record(&mut self, fit: PieceCandidateFit) {
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
pub(super) fn piece_frame(motion: BandPieceMotion, rect: Rect, band: Rect) -> PaneFrame {
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
