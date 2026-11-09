//! Drawing the tile grid: each cell's contents, the readout along its
//! foot, and the borders over the top of them.
//!
//! [`TileCells`] is the app's part -- what each cell asks for and what
//! goes inside it. Everything else is here: the readout row every cell
//! reserves, the number an empty cell carries, and the one pass of
//! [`GridLines`] that draws every border the cells share.

mod contents;
mod fade;
mod foot;
mod measurements;
mod paint;
mod rows;

use std::error::Error;
use std::fmt::Debug;
use std::fmt::Display;
use std::fmt::Formatter;
use std::num::NonZeroUsize;

pub use contents::TileCells;
pub use contents::TileGridContents;
pub use fade::CellFadeProgress;
pub use fade::CellReadoutFade;
use fade::CellReadoutInk;
pub use foot::SummaryFoot;
use foot::SummaryFootDrawOutcome;
use foot::draw_summary_foot;
pub(super) use measurements::TileMeasurements;
pub(super) use measurements::add_readout_rows;
#[cfg(test)]
use measurements::measurement_widths;
pub(super) use measurements::prepare_covered_grid;
use measurements::prepare_grid;
#[cfg(test)]
use paint::draw_cell;
pub(super) use paint::draw_placements;
pub use paint::draw_tile_cell;
pub use paint::draw_tile_grid;
pub(super) use paint::has_visible_body_row;
pub(super) use paint::name_row_is_visible;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Paragraph;
pub use rows::CellContentExtent;
use rows::CellReadout;
use rows::CellRowsReadout;
use rows::ReadoutRow;
pub use rows::RowExceedsTotal;
pub use rows::RowsReadout;
pub use rows::SelectedRow;
pub use rows::ShownRows;
pub(super) use rows::content_area;
use rows::draw_rows_readout;
use rows::draw_rows_readout_after_foot;
use rows::readout_row;
use rows::rows_readout_height;
#[cfg(test)]
use rows::rows_readout_lines;

use super::constants::TILE_FOOT_GAP;
use super::constants::TILE_FOOT_LEFT_INSET;
use super::constants::TILE_NUMBER_INDENT;
use super::constants::TILE_ROWS_CELL_LABEL;
use super::constants::TILE_ROWS_CELL_SEPARATOR;
use super::constants::TILE_ROWS_CONTENT_LABEL;
use super::constants::TILE_ROWS_READOUT_HEIGHT;
use super::constants::TILE_ROWS_RIGHT_INSET;
use super::constants::TILE_ROWS_WIDTH_LABEL;
use super::grid::GridDisplay;
use super::grid::PieceName;
use super::grid::TileContent;
use super::grid::TileDemands;
use super::grid::TileDrawing;
use super::grid::TileGrid;
use super::growth::TileGrowth;
use crate::GridLines;
use crate::PaneBorders;
use crate::PaneChrome;
use crate::PaneFrame;
use crate::PaneFrameLabel;
#[cfg(test)]
use crate::blend_color;
use crate::default_pane_chrome;
use crate::draw_clipped;
use crate::error_color;
use crate::frame_inner;
use crate::label_color;
use crate::screen_ground;
use crate::success_color;
use crate::text_default;
use crate::theme;
use crate::theme::BlendRounding;
use crate::title_color;

#[cfg(test)]
mod tests;
