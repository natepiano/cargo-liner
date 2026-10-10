//! [`TileView`]: the `tiles.view` setting.

use serde::Deserialize;
use serde::Serialize;

use crate::app_settings::SettingStep;

/// `tiles.view`: when the tile grid shows command cells beside its
/// summary.
///
/// Spelled `auto`, `summary` and `cells` in `config.toml`.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TileView {
    /// Show command cells only while the summary draws as many rows
    /// beside them as it draws alone.
    #[default]
    Auto,
    /// Give the whole grid area to the summary.
    Summary,
    /// Show command cells whenever each one fits at its configured
    /// floor.
    Cells,
}

impl TileView {
    /// The spelling `config.toml` and the settings overlay use.
    #[must_use]
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Summary => "summary",
            Self::Cells => "cells",
        }
    }

    /// Move one step through the view choices, wrapping at both ends.
    pub(crate) const fn step(&mut self, step: SettingStep) {
        *self = match (*self, step) {
            (Self::Auto, SettingStep::Next) | (Self::Cells, SettingStep::Prev) => Self::Summary,
            (Self::Summary, SettingStep::Next) | (Self::Auto, SettingStep::Prev) => Self::Cells,
            (Self::Cells, SettingStep::Next) | (Self::Summary, SettingStep::Prev) => Self::Auto,
        };
    }
}
