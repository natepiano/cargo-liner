//! [`TileGrowth`]: the settings that decide how many cells each column
//! of the grid holds, and how wide the summary is.

use crate::InitialRows;
use crate::TileFill;

/// How a [`super::TileGrid`] grows as cells open.
///
/// How tall its first column gets before the grid arranges itself into
/// a square, how the cells spread over the columns from there, and
/// whether the summary reaches across columns for the room its widest
/// line wants.
///
/// The app builds one from its `[tiles]` table and hands it to every
/// grid call that lays cells out, so a setting stepped in the settings
/// overlay reaches the next frame without the grid keeping a copy of
/// the config. [`Self::default`] is what an app's defaults give.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TileGrowth {
    /// Rows the grid grows to in a single column; see [`InitialRows`].
    /// Below [`super::MIN_INITIAL_ROWS`] it reads as that floor.
    pub initial_rows:  usize,
    /// How the cells spread over the columns; see [`TileFill`].
    pub fill:          TileFill,
    /// Whether the summary widens past its own column when its widest
    /// line does not fit there.
    ///
    /// The summary keeps the height its own column gives it and takes
    /// the top of each column it widens over, one column at a time,
    /// until its widest line fits -- [`super::TileDemands::summary_width`]
    /// -- or it reaches the right edge. The cells of a column it widens
    /// over divide what is left below it. A column whose cells would no
    /// longer fit below the summary stops it there, the same as the
    /// edge.
    pub widen_summary: bool,
}

impl Default for TileGrowth {
    fn default() -> Self {
        Self {
            initial_rows:  InitialRows::default().get(),
            fill:          TileFill::default(),
            widen_summary: false,
        }
    }
}
