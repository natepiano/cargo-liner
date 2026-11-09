use super::Arc;
use super::PaneFrame;
use super::Rect;
use super::Slot;
use super::TileContent;
use super::TileDrawing;
use super::TilePlacement;
/// A cell's content and focus appearance, apart from its geometry.
#[derive(Clone)]
pub(super) struct CellAppearance<Id> {
    /// The cell whose contents this piece paints, independent of its number.
    pub(super) identity: CellIdentity<Id>,
    /// What the cell draws.
    pub(super) content:  TileContent<Id>,
    /// Whether the cell holds focus, which is what lights its border.
    pub(super) focused:  bool,
}

/// A painted cell's identity, including empty cells whose numbers can change.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum CellIdentity<Id> {
    Summary,
    Slot(Slot<Id>),
}

/// The background mode recorded by a body paint.
#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum BackgroundTransparency {
    Transparent,
    Opaque,
}

impl BackgroundTransparency {
    pub(super) fn current() -> Self {
        if crate::transparent_background() {
            Self::Transparent
        } else {
            Self::Opaque
        }
    }
}

/// The focused pane tint recorded by a body paint.
#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum FocusedPaneTint {
    Enabled,
    Disabled,
}

impl FocusedPaneTint {
    pub(super) fn current() -> Self {
        if crate::focused_pane_tint_enabled() {
            Self::Enabled
        } else {
            Self::Disabled
        }
    }
}

/// Geometry and appearance from one body paint. Input reads these same pieces.
pub(super) struct GridVisualState<Id> {
    pub(super) drawing:     TileDrawing<Id>,
    pub(super) contents:    crate::TileGridContents,
    pub(super) theme:       Arc<crate::Theme>,
    pub(super) transparent: BackgroundTransparency,
    pub(super) tint:        FocusedPaneTint,
}

/// Only a body paint can replace this record.
pub(super) enum LastPainted<Id> {
    NeverPainted,
    Painted {
        visual:         GridVisualState<Id>,
        summary_frames: Vec<Rect>,
    },
}

/// Whether the painted cell still owns a slot that can receive input.
pub(in crate::tiles) enum CellHit<Id> {
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
    pub(super) fn at(&self, frame: PaneFrame) -> TilePlacement<Id> {
        TilePlacement {
            content: self.content.clone(),
            frame:   frame.with_focus(self.focused),
        }
    }
}

/// Which view of the grid the last laid-out area can show readably.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::tiles) enum GridDisplay {
    /// Every held cell fits at the configured floor.
    Cells,
    /// The summary fills the area while the held cells wait off-screen.
    SummaryAlone,
}
