//! The sizes and timings a tile grid is laid out and animated with.

use super::constants::FOCUS_ANIMATION_MILLIS;
use super::constants::MAX_PENDING_STEPS;
use super::constants::MIN_STEP_MILLIS;
use super::constants::MIN_TILE_HEIGHT;
use super::constants::MIN_TILE_WIDTH;
use super::constants::TILE_ANIMATION_MILLIS;
use super::constants::TILE_BORDER_ROWS;
use super::constants::TILE_DEMAND_STEP;

/// The framed rows a cell uses now and the stepped demand that controls
/// layout motion.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct CellRowDemand {
    /// Rows the current content and frame use.
    exact_framed:   u16,
    /// Rows after content demand is rounded to a layout step.
    stepped_framed: u16,
}

impl CellRowDemand {
    /// A test or retained layout demand whose exact and stepped rows agree.
    pub(super) const fn uniform(framed: u16) -> Self {
        Self {
            exact_framed:   framed,
            stepped_framed: framed,
        }
    }

    /// Rows the current content and frame use.
    pub(super) const fn exact_framed(self) -> u16 { self.exact_framed }

    /// Rows that control layout depth and motion.
    pub(super) const fn stepped_framed(self) -> u16 { self.stepped_framed }

    /// Keep this layout step while accepting a new exact demand.
    pub(super) const fn with_exact_from(self, current: Self) -> Self {
        Self {
            exact_framed: current.exact_framed,
            ..self
        }
    }
}

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
    /// The exact framed rows for `rows` of content and the layout demand after
    /// rounding that content up to a whole [`Self::demand_step`].
    pub(super) fn demanded_rows(&self, rows: usize) -> CellRowDemand {
        let exact = u16::try_from(rows)
            .unwrap_or(u16::MAX)
            .saturating_add(self.border_rows);
        let stepped = rows
            .div_ceil(self.demand_step)
            .saturating_mul(self.demand_step);
        let stepped = u16::try_from(stepped)
            .unwrap_or(u16::MAX)
            .saturating_add(self.border_rows);
        CellRowDemand {
            exact_framed:   exact,
            stepped_framed: stepped,
        }
    }
}
