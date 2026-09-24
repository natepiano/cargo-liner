//! The sizes and timings a tile grid is laid out and animated with.

use super::constants::FOCUS_ANIMATION_MILLIS;
use super::constants::MAX_PENDING_STEPS;
use super::constants::MIN_STEP_MILLIS;
use super::constants::MIN_TILE_HEIGHT;
use super::constants::MIN_TILE_WIDTH;
use super::constants::TILE_ANIMATION_MILLIS;
use super::constants::TILE_BORDER_ROWS;
use super::constants::TILE_DEMAND_STEP;

/// The sizes and timings a [`super::TileGrid`] lays its cells out and
/// animates them with. Every grid runs on [`Self::default`], which
/// reads the constants in [`super::constants`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct TileSettings {
    /// Rows one cell standing alone needs; see [`MIN_TILE_HEIGHT`].
    pub(super) min_tile_height:        u16,
    /// Columns one cell standing alone needs; see [`MIN_TILE_WIDTH`].
    pub(super) min_tile_width:         u16,
    /// Rows a cell spends on its own border; see [`TILE_BORDER_ROWS`].
    pub(super) border_rows:            u16,
    /// Rows of content one step of demand is worth; see
    /// [`TILE_DEMAND_STEP`].
    pub(super) demand_step:            usize,
    /// How long one change to the grid takes; see
    /// [`TILE_ANIMATION_MILLIS`].
    pub(super) animation_millis:       u64,
    /// Floor on one step of a ripple; see [`MIN_STEP_MILLIS`].
    pub(super) min_step_millis:        u64,
    /// How long the resize a step of the focus ring asks for takes; see
    /// [`FOCUS_ANIMATION_MILLIS`].
    pub(super) focus_animation_millis: u64,
    /// Steps the grid queues before it settles the rest in one move;
    /// see [`MAX_PENDING_STEPS`].
    pub(super) max_pending_steps:      usize,
}

impl Default for TileSettings {
    fn default() -> Self {
        Self {
            min_tile_height:        MIN_TILE_HEIGHT,
            min_tile_width:         MIN_TILE_WIDTH,
            border_rows:            TILE_BORDER_ROWS,
            demand_step:            TILE_DEMAND_STEP,
            animation_millis:       TILE_ANIMATION_MILLIS,
            min_step_millis:        MIN_STEP_MILLIS,
            focus_animation_millis: FOCUS_ANIMATION_MILLIS,
            max_pending_steps:      MAX_PENDING_STEPS,
        }
    }
}

impl TileSettings {
    /// The rows a cell drawing `rows` of content asks its column for:
    /// the content rounded up to a whole [`Self::demand_step`], plus the
    /// rows its own border costs.
    pub(super) fn demanded_rows(&self, rows: usize) -> u16 {
        let stepped = rows
            .div_ceil(self.demand_step)
            .saturating_mul(self.demand_step);
        u16::try_from(stepped)
            .unwrap_or(u16::MAX)
            .saturating_add(self.border_rows)
    }
}
