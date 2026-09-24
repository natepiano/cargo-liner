//! The tile grid, keyed by the invocation each command's cell draws.
//!
//! The grid itself is [`tui_pane::TileGrid`]; these aliases bind it to
//! [`InvocationId`] so the rest of the app names it without a type
//! argument.

use crate::census::InvocationId;

/// The grid of cells, one per running command after the summary.
pub(crate) type TileGrid = tui_pane::TileGrid<InvocationId>;
/// What one cell of the grid is showing.
pub(crate) type TileContent = tui_pane::TileContent<InvocationId>;
/// What every cell of the grid is asking for, as one scan left it.
pub(crate) type TileDemands = tui_pane::TileDemands<InvocationId>;
/// One command group's claim on its column.
pub(crate) type TileDemand = tui_pane::TileDemand<InvocationId>;
