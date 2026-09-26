//! `cargo-handler` configuration: the sections of
//! `<os config dir>/cargo-handler/config.toml`, and the identity
//! `tui_pane` finds that file, the keymap file, the favorites file and
//! the themes directory by.

#[cfg(test)]
use std::path::Path;
#[cfg(test)]
use std::path::PathBuf;

use serde::Deserialize;
use serde::Serialize;
use tui_pane::AppConfig;
use tui_pane::AppIdentity;
use tui_pane::AppearanceConfig;
use tui_pane::InitialRows;
use tui_pane::TileFill;
use tui_pane::TileGrowth;

use crate::constants::BINARY_NAME;
use crate::constants::CONFIG_DIRNAME;
use crate::constants::DEFAULT_DARK_THEME;
use crate::constants::DEFAULT_LIGHT_THEME;
use crate::constants::DEFAULT_WIDEN_SUMMARY;
#[cfg(test)]
use crate::constants::TEST_CONFIG_ROOT;

/// cargo-handler's identity for the framework's config and runner paths.
pub(crate) enum CargoHandler {}

/// In a test build, `config.toml`, `themes/` and `keymap.toml` resolve
/// under `/<config>` instead of the OS config directory. The
/// settings overlay is as wide as its widest row, which is one of these
/// paths, so only a fixed root draws the same overlay on every machine.
/// It also keeps a test from writing the user's own config or keymap.
impl AppIdentity for CargoHandler {
    const BINARY_NAME: &'static str = BINARY_NAME;
    const CONFIG_DIRNAME: &'static str = CONFIG_DIRNAME;
    const DEFAULT_LIGHT_THEME: &'static str = DEFAULT_LIGHT_THEME;
    const DEFAULT_DARK_THEME: &'static str = DEFAULT_DARK_THEME;

    #[cfg(test)]
    fn config_path() -> Option<PathBuf> { Some(test_config_path("config.toml")) }

    #[cfg(test)]
    fn keymap_path() -> Option<PathBuf> { Some(test_config_path("keymap.toml")) }

    #[cfg(test)]
    fn themes_dir() -> Option<PathBuf> { Some(test_config_path("themes")) }
}

/// Where a test build finds `name`: in cargo-handler's directory under
/// [`TEST_CONFIG_ROOT`].
#[cfg(test)]
fn test_config_path(name: &str) -> PathBuf {
    Path::new(TEST_CONFIG_ROOT).join(CONFIG_DIRNAME).join(name)
}

/// How the tile grid grows.
#[derive(Debug, Deserialize, Serialize)]
#[serde(default)]
pub(crate) struct TilesConfig {
    /// Rows the grid grows to in a single column before it starts
    /// arranging itself into a square. Read through
    /// [`TilesConfig::growth`], which enforces the floor.
    pub(crate) initial_rows:  InitialRows,
    /// How the cells spread over the columns once there is more than
    /// one: `add_new` or `redistribute`.
    pub(crate) fill:          TileFill,
    /// Whether the summary widens over the next columns when its widest
    /// line does not fit its own; see [`TileGrowth::widen_summary`].
    pub(crate) widen_summary: bool,
}

impl Default for TilesConfig {
    fn default() -> Self {
        Self {
            initial_rows:  InitialRows::default(),
            fill:          TileFill::default(),
            widen_summary: DEFAULT_WIDEN_SUMMARY,
        }
    }
}

impl TilesConfig {
    /// The grid's growth: the single column's rows, never below one
    /// (see [`InitialRows::get`]), the fill, and whether the summary
    /// widens.
    pub(crate) fn growth(&self) -> TileGrowth {
        TileGrowth {
            initial_rows:  self.initial_rows.get(),
            fill:          self.fill,
            widen_summary: self.widen_summary,
        }
    }
}

/// The machines the summary lists besides this one.
#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub(crate) struct MachinesConfig {
    /// ssh host names, each probed for its agents, listed in this
    /// order after this machine.
    pub(crate) remote: Vec<String>,
}

/// Parsed `config.toml`. Every section defaults, so a missing file and
/// an empty file behave the same.
#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub(crate) struct Config {
    /// `[appearance]` — theme selection.
    pub(crate) appearance: AppearanceConfig<CargoHandler>,
    /// `[tiles]` — how the tile grid grows.
    pub(crate) tiles:      TilesConfig,
    /// `[machines]` — the remote machines the summary lists.
    pub(crate) machines:   MachinesConfig,
}

impl AppConfig for Config {
    type Identity = CargoHandler;

    fn appearance(&self) -> &AppearanceConfig<CargoHandler> { &self.appearance }

    fn appearance_mut(&mut self) -> &mut AppearanceConfig<CargoHandler> { &mut self.appearance }

    fn initial_rows_mut(&mut self) -> &mut InitialRows { &mut self.tiles.initial_rows }

    fn tile_fill_mut(&mut self) -> &mut TileFill { &mut self.tiles.fill }

    fn widen_summary_mut(&mut self) -> &mut bool { &mut self.tiles.widen_summary }
}

/// `config.toml` as loaded, with whatever went wrong reading or
/// writing it.
pub(crate) type LoadedConfig = tui_pane::LoadedConfig<Config>;

#[cfg(test)]
#[expect(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use super::*;

    /// What [`LoadedConfig::load`] compares against, for a config that
    /// has been through the file and back.
    fn round_trip(text: &str) -> String {
        let config: Config = toml::from_str(text).expect("a config the test wrote should parse");
        toml::to_string_pretty(&config).expect("a config should serialize")
    }

    /// Every key the default config writes, in the order and spelling
    /// `config.toml` carries. A change here rewrites every user's file
    /// on their next startup.
    #[test]
    fn the_default_config_serializes_to_the_known_file() {
        let expected = "\
[appearance]
mode = \"auto\"
light_theme = \"Default Light\"
dark_theme = \"Default Dark\"
iterm2_profile = \"cargo-handler\"

[tiles]
initial_rows = 4
fill = \"redistribute\"
widen_summary = true

[machines]
remote = []
";
        let written =
            toml::to_string_pretty(&Config::default()).expect("a config should serialize");
        assert_eq!(written, expected);
    }

    /// [`LoadedConfig::load`] writes the file back whenever it does not
    /// already hold every setting, so a file that does hold them must
    /// serialize to itself -- otherwise every startup rewrites the
    /// config, for good.
    #[test]
    fn a_complete_config_is_left_alone() {
        let complete =
            toml::to_string_pretty(&Config::default()).expect("a config should serialize");
        assert_eq!(round_trip(&complete), complete);
    }

    /// A file that leaves `iterm2_profile`, the theme ids and `[tiles]`
    /// out gets the app's own defaults for them when it is restated,
    /// and keeps what it did say.
    #[test]
    fn a_config_missing_keys_restates_the_binary_name() {
        let restated = round_trip("[appearance]\nmode = \"dark\"\n");
        assert!(restated.contains("mode = \"dark\""));
        assert!(restated.contains("iterm2_profile = \"cargo-handler\""));
        assert!(restated.contains("light_theme = \"Default Light\""));
        assert!(restated.contains("[tiles]"));
    }

    /// The fill a config spelling `text` loads with.
    fn parsed_fill(text: &str) -> TileFill {
        toml::from_str::<Config>(text)
            .expect("a config the test wrote should parse")
            .tiles
            .fill
    }

    /// `tiles.fill` parses in both spellings, and a `[tiles]` table
    /// that leaves it out gets redistribute.
    #[test]
    fn tile_fill_parses_both_values_and_falls_back_to_redistribute() {
        assert_eq!(
            parsed_fill("[tiles]\nfill = \"add_new\"\n"),
            TileFill::AddNew
        );
        assert_eq!(
            parsed_fill("[tiles]\nfill = \"redistribute\"\n"),
            TileFill::Redistribute
        );
        assert_eq!(
            parsed_fill("[tiles]\ninitial_rows = 3\n"),
            TileFill::Redistribute
        );
    }
}
