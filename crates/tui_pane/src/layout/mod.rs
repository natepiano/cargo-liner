mod column_widths;
mod viewport;

pub use column_widths::ColumnSpec;
pub use column_widths::ColumnWidths;
use ratatui::layout::Constraint;
use ratatui::layout::Rect;
pub use viewport::Viewport;
pub use viewport::ViewportOverflow;
pub use viewport::render_overflow_affordance;

/// Axis length spec for a pane in a grid layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaneAxisSize {
    /// Fixed length in cells.
    Fixed(u16),
    /// Flex-fill with relative weight.
    Fill(u16),
}

/// Width + height spec for a single pane.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PaneSizeSpec {
    /// Width spec along the horizontal axis.
    pub width:  PaneAxisSize,
    /// Height spec along the vertical axis.
    pub height: PaneAxisSize,
}

impl PaneSizeSpec {
    /// Spec that fills the available space on both axes with equal
    /// weight.
    #[must_use]
    pub const fn fill() -> Self {
        Self {
            width:  PaneAxisSize::Fill(1),
            height: PaneAxisSize::Fill(1),
        }
    }
}

/// Placement of one pane inside a grid layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PanePlacement<Id> {
    /// Pane identifier.
    pub pane:     Id,
    /// Top row this pane occupies.
    pub row:      usize,
    /// Left column this pane occupies.
    pub col:      usize,
    /// Number of rows this pane spans.
    pub row_span: usize,
    /// Number of columns this pane spans.
    pub col_span: usize,
}

/// Grid layout: a list of pane placements.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaneGridLayout<Id> {
    /// Placements in arbitrary order.
    pub placements: Vec<PanePlacement<Id>>,
}

impl<Id: Copy> PaneGridLayout<Id> {
    /// Tab order derived from placements (row-major, then column).
    #[must_use]
    pub fn tab_order(self) -> Vec<Id> {
        let mut placements = self.placements;
        placements.sort_by_key(|placement| (placement.row, placement.col));
        placements
            .into_iter()
            .map(|placement| placement.pane)
            .collect()
    }
}

/// One pane after layout resolution: its identifier and rendered rect.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResolvedPane<Id> {
    /// Pane identifier.
    pub pane: Id,
    /// Rendered rect for the pane.
    pub area: Rect,
}

/// Layout resolved to concrete `Rect`s.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ResolvedPaneLayout<Id> {
    /// Resolved placements in arbitrary order.
    pub panes: Vec<ResolvedPane<Id>>,
}

impl<Id> ResolvedPaneLayout<Id> {
    /// Construct from a vec of resolved placements.
    #[must_use]
    pub const fn new(panes: Vec<ResolvedPane<Id>>) -> Self { Self { panes } }
}

impl<Id: Copy + Eq> ResolvedPaneLayout<Id> {
    /// Look up the rect for `pane`. Returns [`Rect::ZERO`] when absent.
    #[must_use]
    pub fn area(&self, pane: Id) -> Rect {
        self.panes
            .iter()
            .find(|resolved| resolved.pane == pane)
            .map_or(Rect::ZERO, |resolved| resolved.area)
    }
}

/// Convert a slice of [`PaneAxisSize`] into ratatui [`Constraint`]s.
#[must_use]
pub fn constraints_for_sizes(sizes: &[PaneAxisSize]) -> Vec<Constraint> {
    sizes
        .iter()
        .map(|size| match size {
            PaneAxisSize::Fixed(length) => Constraint::Length(*length),
            PaneAxisSize::Fill(weight) => Constraint::Fill(*weight),
        })
        .collect()
}
