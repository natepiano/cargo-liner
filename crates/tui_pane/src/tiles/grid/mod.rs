//! The tile grid: how many cells the pane holds, where each one sits,
//! and the motion from one arrangement to the next.
//!
//! Cells are numbered from one and fill column by column. Cell one is
//! the summary table; each of the rest belongs to one running command,
//! or stands empty waiting for one. [`allocation::columns`] says how many cells each
//! column holds and [`allocation::shares`] says how a column divides between them;
//! both are pure, so the arrangement at any count is a test rather than
//! something to squint at on screen.
//!
//! A column divides evenly until something forces it not to. Each cell
//! asks for the rows its contents need through [`allocation::cell_wants`], and
//! [`allocation::shares`] hands room off the cells that are not using theirs and
//! onto the ones that have run out, from either side. A cell keeps what
//! it needs and no more, so every row it is not using is one a
//! neighbour can have; the summary, then the focus ring, is served
//! first out of them. A column where everything fits is divided evenly,
//! because nothing in it is asking for the room going spare.
//!
//! What a cell holds is sticky. Its contents shrinking hands nothing
//! back, because nothing else wants the room; the room returns when
//! another cell encroaches -- any cell asking for more than it holds,
//! or a cell joining or leaving -- at which point every cell drops to
//! what it is actually showing and the whole grid is divided again.
//! [`arrangement::HeldCellLayout`] is that state and [`TileGrid::settled_held`] is the rule;
//! [`allocation::shares`] itself stays a pure function of what it is handed.
//!
//! The demand is deliberately coarse. Measured in rows, a column would
//! re-divide on every scan that rewrapped a command line; rounded up to
//! a whole [`super::constants::TILE_DEMAND_STEP`] it moves on the step
//! from a quiet cell to a busy one, which is what keeps a change
//! something the grid travels through rather than something it twitches
//! on.
//!
//! Splitting a rect is the framework's job, not this module's:
//! [`constraints_for_sizes`] turns per-column and per-row shares into
//! ratatui constraints and ratatui's solver tiles the rect exactly, and
//! the result is a [`ResolvedPaneLayout`] keyed by cell number -- the
//! same type [`crate::render_panes`] walks. What is left here is only
//! what the framework has no opinion about: how many columns there are,
//! how tall each one is, and how a cell travels when that changes.
//!
//! Neighbours share a border, so the rects handed out overlap by the one
//! line between them rather than sitting flush. [`crate::GridLines`]
//! draws that line once for both.

mod allocation;
mod arrangement;
mod bands;
mod column_order;
mod demands;
mod heights;
mod layout;
mod motion;
mod pieces;
mod rows;
mod selection;
mod tile_grid;
mod visual;

use std::cell::OnceCell;
use std::cell::RefCell;
use std::cmp::Ordering;
use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::collections::HashMap;
use std::collections::VecDeque;
use std::fmt::Debug;
use std::num::NonZeroU16;
use std::sync::Arc;
use std::sync::OnceLock;
#[cfg(test)]
use std::sync::atomic::AtomicUsize;
#[cfg(test)]
use std::sync::atomic::Ordering as AtomicOrdering;
use std::time::Duration;
use std::time::Instant;

use allocation::ColumnHead;
#[cfg(test)]
use allocation::ColumnRowUse;
#[cfg(test)]
use allocation::apportion_shared_run;
use allocation::cell_wants;
use allocation::column_focus;
use allocation::columns;
use allocation::fills;
use allocation::fits;
#[cfg(test)]
use allocation::serve_directional_shortfalls;
use allocation::shared_run;
use allocation::shares;
use allocation::shares_with_directional_row_usage;
use allocation::summary_before_tiles;
use allocation::summary_depth;
use allocation::summary_share;
use arrangement::ColumnFocus;
use arrangement::DemandSync;
use arrangement::Direction;
use arrangement::Focus;
pub use arrangement::FocusLocation;
use arrangement::HeldCellLayout;
use arrangement::QueuedArrangement;
use arrangement::RowDemandChange;
use arrangement::Slot;
use arrangement::cell_of;
use arrangement::cells;
use arrangement::claims;
use arrangement::content_of;
use arrangement::kept;
use arrangement::position;
use arrangement::queued_cells_keep_endpoint_columns;
use arrangement::union;
#[cfg(test)]
use bands::BandEdge;
#[cfg(test)]
use bands::BandHeightSearch;
use bands::BandPiece;
#[cfg(test)]
use bands::BandPieceMotion;
use bands::CellTransition;
use bands::Turns;
#[cfg(test)]
use bands::band_piece_dividers;
#[cfg(test)]
use bands::band_piece_rect;
#[cfg(test)]
use bands::column_band;
use bands::column_bands;
use bands::moving_cell;
#[cfg(test)]
use bands::piece_frame;
use bands::place_band_pieces;
use column_order::ColumnRanks;
pub use column_order::TileColumnOrder;
pub use demands::TileDemand;
pub use demands::TileDemands;
use heights::DestinationHeightBounds;
#[cfg(test)]
use heights::DestinationRowUse;
use heights::FilledColumnFit;
use heights::HeightSnap;
use heights::LastSettledCellGeometry;
use heights::MotionHeightPath;
use heights::MotionSpanBounds;
use heights::PieceHeightPath;
use heights::RetargetedHeightConstraints;
use heights::RetargetedMotionDestination;
use heights::fill_retargeted_columns;
use heights::measure_destination_rows;
use layout::CheckedLayoutAdmission;
use layout::ColumnLayout;
use layout::ColumnPlan;
use layout::EndpointLayoutRules;
use layout::Grid;
use layout::RowUsePurpose;
use layout::candidate_content_widths;
use layout::column_layout;
use layout::summary_span;
use motion::DestinationDepthStatus;
use motion::GridMotion;
use motion::MotionClock;
#[cfg(test)]
pub(super) use motion::MotionTiming;
use motion::eased;
use motion::lerp;
use motion::lerp_rect;
use motion::travel_progress;
pub use pieces::CellIndex;
pub use pieces::CellTravel;
pub(super) use pieces::PieceHeightSnap;
pub(super) use pieces::PieceName;
pub use pieces::TileContent;
pub(super) use pieces::TileDrawing;
pub(super) use pieces::TilePiece;
pub use pieces::TilePlacement;
use ratatui::buffer::Buffer;
use ratatui::layout::Layout;
use ratatui::layout::Position;
use ratatui::layout::Rect;
use rows::FilledSpan;
use rows::FrameCellRowAnswers;
use rows::FrameCellRowUsage;
use rows::FrameHeightFit;
use rows::FrameRowAnswer;
use rows::RetainedRows;
use rows::frame_height_fit;
use rows::frame_painted_rows;
use rows::frame_retained_rows;
use rows::measure_cell_retained_rows;
#[cfg(test)]
use rows::measure_frame_row_usage;
use rows::measure_frame_row_use;
use rows::measured_filled_span;
use selection::CandidateDepthSelection;
use selection::CandidateMeasurementFreshness;
pub(super) use selection::ColumnPreparation;
use selection::LayoutDemandMeasurement;
use selection::LayoutSelection;
use selection::LayoutSelectionInputs;
pub(super) use selection::SelectedLayoutMeasurements;
use selection::SelectedLayoutPlan;
pub use tile_grid::TileGrid;
use tile_grid::Transition;
use visual::BackgroundTransparency;
use visual::CellAppearance;
pub(super) use visual::CellHit;
use visual::CellIdentity;
use visual::FocusedPaneTint;
use visual::FullScreenPiece;
pub(super) use visual::GridDisplay;
use visual::GridVisualState;
use visual::LastPainted;

use super::action::TileAction;
use super::constants;
use super::constants::MIN_INITIAL_ROWS;
use super::constants::MIN_TILE_HEIGHT;
use super::constants::MOVING_HEIGHT_PROBE_STEP;
use super::constants::PROGRESS_SCALE;
use super::constants::SUMMARY_SLOT_SHORTFALL_DIVISOR;
use super::constants::TABLE_CELL;
use super::draw;
use super::draw::TileMeasurements;
use super::growth::TileGrowth;
use super::host;
use super::settings::CellRowDemand;
use super::settings::SummaryRowAllocation;
use super::settings::TileSettings;
use super::settings::TileWidthFloor;
use crate::CycleDirection;
use crate::PaneAxisSize;
use crate::PaneFrame;
use crate::Repaint;
use crate::ResolvedPane;
use crate::ResolvedPaneLayout;
use crate::TileFill;
use crate::TileGridContents;
use crate::TileView;
use crate::constraints_for_sizes;
use crate::frame_inner;
use crate::share_borders;

#[cfg(test)]
mod tests;

#[cfg(test)]
#[path = "../drawn_hits_tests.rs"]
mod drawn_hits_tests;
