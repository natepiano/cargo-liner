//! A grid of bordered cells that opens, closes and re-divides with
//! animated motion.
//!
//! Cell one is a summary every grid opens with; each cell after it
//! draws one group the app names by its own `Id`, or stands empty.
//! [`TileGrid`] holds the arrangement and hands back where every cell is
//! drawn this frame, [`TileDemands`] is what the app tells it each cell
//! is asking for, and [`TileAction`] is what the app's keys ask it to
//! do. [`draw_tile_grid`] draws it, asking the app's [`TileCells`] what
//! goes in each cell. The layout rules and the motion are described in
//! `grid.rs`.

mod action;
mod constants;
mod draw;
mod grid;
mod settings;

pub use action::TileAction;
pub use constants::MIN_INITIAL_ROWS;
pub use constants::TABLE_CELL;
pub use constants::TILE_ROWS_CONTENT_LABEL;
pub use draw::TileCells;
pub use draw::TileGridContents;
pub use draw::draw_tile_cell;
pub use draw::draw_tile_grid;
pub use grid::TileContent;
pub use grid::TileDemand;
pub use grid::TileDemands;
pub use grid::TileGrid;
pub use grid::TilePlacement;
