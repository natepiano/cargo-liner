//! [`TileView`]: the `tiles.view` setting.

use serde::Deserialize;
use serde::Serialize;

use crate::app_settings::SettingStep;

/// `tiles.view`: when the tile grid shows command cells beside its
/// summary.
///
/// Spelled `summary` and `cells` in `config.toml`.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TileView {
    /// Give the whole grid area to the summary.
    Summary,
    /// Show command cells whenever each one fits at its configured
    /// floor.
    #[default]
    #[serde(alias = "auto")]
    Cells,
}

impl TileView {
    /// The spelling `config.toml` and the settings overlay use.
    #[must_use]
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Summary => "summary",
            Self::Cells => "cells",
        }
    }

    /// Move one step through the view choices, wrapping at both ends.
    pub(crate) const fn step(&mut self, step: SettingStep) {
        *self = match (*self, step) {
            (Self::Cells, SettingStep::Next | SettingStep::Prev) => Self::Summary,
            (Self::Summary, SettingStep::Next | SettingStep::Prev) => Self::Cells,
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_saved_auto_view_loads_as_cells_and_saves_as_cells()
    -> Result<(), Box<dyn std::error::Error>> {
        #[derive(Deserialize, Serialize)]
        struct Tiles {
            view: TileView,
        }
        let tiles: Tiles = toml::from_str("view = \"auto\"")?;
        assert_eq!(tiles.view, TileView::Cells);
        assert_eq!(toml::to_string(&tiles)?, "view = \"cells\"\n");
        Ok(())
    }

    #[test]
    fn the_view_choices_wrap_between_summary_and_cells_in_both_directions() {
        for direction in [SettingStep::Next, SettingStep::Prev] {
            let mut view = TileView::Cells;
            view.step(direction);
            assert_eq!(view, TileView::Summary);
            view.step(direction);
            assert_eq!(view, TileView::Cells);
        }
    }
}
