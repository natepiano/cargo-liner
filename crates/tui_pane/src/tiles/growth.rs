//! [`TileGrowth`]: the settings that decide how many cells each column
//! of the grid holds.

use crate::InitialRows;
use crate::TileFill;

/// How a [`super::TileGrid`] grows as cells open: how tall its first
/// column gets before the grid arranges itself into a square, and how
/// the cells spread over the columns from there.
///
/// The app builds one from its `[tiles]` table and hands it to every
/// grid call that lays cells out, so a setting stepped in the settings
/// overlay reaches the next frame without the grid keeping a copy of
/// the config. [`Self::default`] is what an app's defaults give.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TileGrowth {
    /// Rows the grid grows to in a single column; see [`InitialRows`].
    /// Below [`super::MIN_INITIAL_ROWS`] it reads as that floor.
    pub initial_rows: usize,
    /// How the cells spread over the columns; see [`TileFill`].
    pub fill:         TileFill,
}

impl Default for TileGrowth {
    fn default() -> Self {
        Self {
            initial_rows: InitialRows::default().get(),
            fill:         TileFill::default(),
        }
    }
}
