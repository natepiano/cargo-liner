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

use crate::constants::BINARY_NAME;
use crate::constants::CONFIG_DIRNAME;
use crate::constants::DEFAULT_DARK_THEME;
use crate::constants::DEFAULT_LIGHT_THEME;
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
#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub(crate) struct TilesConfig {
    /// Rows the grid grows to in a single column before it starts
    /// arranging itself into a square. Read through
    /// [`TilesConfig::initial_rows`], which enforces the floor.
    pub(crate) initial_rows: InitialRows,
}

impl TilesConfig {
    /// Rows the single column grows to, never below one; see
    /// [`InitialRows::get`].
    pub(crate) fn initial_rows(&self) -> usize { self.initial_rows.get() }
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
}

impl AppConfig for Config {
    type Identity = CargoHandler;

    fn appearance(&self) -> &AppearanceConfig<CargoHandler> { &self.appearance }

    fn appearance_mut(&mut self) -> &mut AppearanceConfig<CargoHandler> { &mut self.appearance }

    fn initial_rows_mut(&mut self) -> &mut InitialRows { &mut self.tiles.initial_rows }
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
}
