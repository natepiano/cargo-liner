//! The tile grid, keyed by a group type with no values.
//!
//! The grid itself is [`tui_pane::TileGrid`]. cargo-handler has nothing
//! yet that claims a cell of its own, so the id is [`NoGroup`], which
//! states in the type that no cell holds a group: every cell is the
//! summary or an empty cell opened with `+`.

/// The group a cell would draw. Uninhabited: nothing claims a cell yet.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum NoGroup {}

/// The grid of cells, the summary first.
pub(crate) type TileGrid = tui_pane::TileGrid<NoGroup>;
/// What one cell of the grid is showing.
pub(crate) type TileContent = tui_pane::TileContent<NoGroup>;
/// What every cell of the grid is asking for.
pub(crate) type TileDemands = tui_pane::TileDemands<NoGroup>;
