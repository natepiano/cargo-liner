use super::CellIdentity;
use super::Duration;
use super::PaneFrame;
use super::Rect;
#[cfg(test)]
use super::Slot;
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

/// The current slot that holds a requested content identity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CellIndex {
    /// The content holds this one-based grid index.
    Current(usize),
    /// No current slot holds the content.
    Missing,
}

/// One cell as a single frame should draw it.
///
/// A cell crossing between columns is drawn as two of these -- the piece
/// leaving the old column and the piece arriving in the new one -- which
/// is what makes it read as sliding off one column's edge and back in at
/// the next.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TilePlacement<Id> {
    /// What the cell draws.
    pub content: TileContent<Id>,
    /// Where the cell's box sits this frame, and how far it is cut off
    /// -- the framework's own account of a moving pane, which both
    /// [`crate::draw_clipped`] and [`crate::GridLines`] read.
    pub frame:   PaneFrame,
}

/// A cell travels between two allocations using the grid's motion settings.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CellTravel {
    /// The last painted allocation when this travel began.
    pub from:    Rect,
    /// The allocation this frame requests.
    pub to:      Rect,
    /// Time advanced by the application's live poll clock.
    pub elapsed: Duration,
}

/// Which copy of a cell draws its name while the cell crosses columns.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::tiles) enum PieceName {
    /// This piece draws the cell's title or empty-cell number.
    Shown,
    /// The cell's other piece draws its name.
    OnTheOtherPiece,
}

/// One visible piece and the name role its placed geometry gives it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::tiles) struct TilePiece<Id> {
    /// Stable cell identity carried through movement and removal.
    pub(super) identity:              CellIdentity<Id>,
    /// What and where the piece draws.
    pub(in crate::tiles) placement:   TilePlacement<Id>,
    /// Whether this piece draws the cell's name.
    pub(in crate::tiles) name:        PieceName,
    /// How this piece participates in motion-height probing.
    pub(in crate::tiles) height_snap: PieceHeightSnap,
}

#[cfg(test)]
impl<Id: Clone> TilePiece<Id> {
    /// A hand-built paint fixture, with the same identity as its content.
    pub(in crate::tiles) fn for_test(
        content: TileContent<Id>,
        frame: PaneFrame,
        name: PieceName,
    ) -> Self {
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
pub(in crate::tiles) enum PieceHeightSnap {
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
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::tiles) struct TileDrawing<Id> {
    /// Every visible piece, in cell order.
    pub(in crate::tiles) pieces:       Vec<TilePiece<Id>>,
    /// Each column's band at the same point in the transition.
    pub(in crate::tiles) column_bands: Vec<Rect>,
}

impl<Id> TileDrawing<Id> {
    /// The placements carried by this frame's pieces.
    #[cfg(test)]
    pub(in crate::tiles) fn placements(&self) -> impl Iterator<Item = &TilePlacement<Id>> {
        self.pieces.iter().map(|piece| &piece.placement)
    }
}
