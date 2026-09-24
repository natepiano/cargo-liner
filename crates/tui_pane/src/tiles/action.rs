//! The grid actions an app's keys ask for.

/// One thing a key asks the tile grid to do, carried out by
/// [`TileGrid::apply`](super::TileGrid::apply).
///
/// An app keeps its own variants for these in its globals enum, so the
/// keymap table they live in, their names and their default keys stay
/// the app's; each of those arms hands the matching action here.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TileAction {
    /// Open an empty cell at the end, unless the grid it would make no
    /// longer fits.
    Add,
    /// Close an empty cell: the focused one when focus is on one,
    /// otherwise the last one [`Self::Add`] opened. A cell drawing a
    /// group and the summary are never closed this way.
    Remove,
    /// Move focus one column to the left, staying put at the edge.
    FocusLeft,
    /// Move focus one column to the right, staying put at the edge.
    FocusRight,
    /// Move focus one cell up within its column, staying put at the top.
    FocusUp,
    /// Move focus one cell down within its column, staying put at the
    /// bottom.
    FocusDown,
}
