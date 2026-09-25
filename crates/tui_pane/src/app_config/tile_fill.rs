//! [`TileFill`]: the `tiles.fill` setting.

use serde::Deserialize;
use serde::Serialize;

use super::constants::TILE_FILLS;
use crate::app_settings;
use crate::app_settings::SettingStep;

/// `tiles.fill`: how the tile grid spreads its cells over its columns
/// once it holds more than one.
///
/// Either way the grid opens the same number of columns for the same
/// number of cells; the two differ only in how many cells each column
/// holds. Spelled `add_new` and `redistribute` in `config.toml`.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TileFill {
    /// Every column but the last stands at full height, and the last
    /// holds what is left over. The cell that opens a new column stands
    /// in it alone, the whole height of the grid.
    AddNew,
    /// The cells are dealt over the columns as evenly as they go: each
    /// column holds `cells / columns`, and the first `cells % columns`
    /// columns hold one more. The cell that opens a new column moves
    /// cells over from the columns before it, so no column stands more
    /// than one cell shorter than another.
    #[default]
    Redistribute,
}

impl TileFill {
    /// The spelling `config.toml` and the settings overlay use.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AddNew => "add_new",
            Self::Redistribute => "redistribute",
        }
    }

    /// Move one step through [`TILE_FILLS`], wrapping at both ends.
    pub(crate) fn step(&mut self, step: SettingStep) {
        let choices: Vec<String> = TILE_FILLS
            .iter()
            .map(|fill| fill.as_str().to_string())
            .collect();
        let next = app_settings::stepped(&choices, self.as_str(), step);
        *self = TILE_FILLS
            .into_iter()
            .find(|fill| fill.as_str() == next)
            .unwrap_or_default();
    }
}

#[cfg(test)]
#[expect(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use serde::Deserialize;
    use serde::Serialize;

    use super::TileFill;
    use crate::app_settings::SettingStep;

    /// A `[tiles]` table holding nothing but the setting, the way an
    /// app's own table carries it.
    #[derive(Debug, Default, Deserialize, Serialize)]
    #[serde(default)]
    struct Tiles {
        /// The setting under test.
        fill: TileFill,
    }

    /// The setting as a `[tiles]` table spelling `text` parses it.
    fn parsed(text: &str) -> TileFill {
        toml::from_str::<Tiles>(text)
            .expect("a table the test wrote should parse")
            .fill
    }

    /// Both spellings parse to their own variant.
    #[test]
    fn both_spellings_parse_from_toml() {
        assert_eq!(parsed("fill = \"add_new\"\n"), TileFill::AddNew);
        assert_eq!(parsed("fill = \"redistribute\"\n"), TileFill::Redistribute);
    }

    /// A table that leaves the key out gets the default, which is
    /// redistribute.
    #[test]
    fn a_missing_key_falls_back_to_redistribute() {
        assert_eq!(parsed(""), TileFill::Redistribute);
        assert_eq!(TileFill::default(), TileFill::Redistribute);
    }

    /// A value that is neither spelling is a parse error, so a typo
    /// leaves the file for its author to fix.
    #[test]
    fn an_unknown_spelling_does_not_parse() {
        assert!(toml::from_str::<Tiles>("fill = \"addnew\"\n").is_err());
    }

    /// The file and the settings overlay spell each variant the same
    /// way, so a value read off the overlay can be typed into the file.
    #[test]
    fn the_overlay_spells_each_value_the_way_the_file_does() {
        for fill in [TileFill::AddNew, TileFill::Redistribute] {
            let written = toml::to_string(&Tiles { fill }).expect("a table should serialize");
            assert_eq!(written, format!("fill = \"{}\"\n", fill.as_str()));
        }
    }

    /// Two values, so either direction moves to the other one.
    #[test]
    fn stepping_wraps_between_the_two_values() {
        let mut fill = TileFill::Redistribute;
        fill.step(SettingStep::Next);
        assert_eq!(fill, TileFill::AddNew);
        fill.step(SettingStep::Next);
        assert_eq!(fill, TileFill::Redistribute);
        fill.step(SettingStep::Prev);
        assert_eq!(fill, TileFill::AddNew);
    }
}
