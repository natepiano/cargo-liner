//! `cargo-tile` configuration: the sections of
//! `<os config dir>/cargo-tile/config.toml`, and the identity
//! `tui_pane` finds that file, the keymap file and the themes directory
//! by.

use std::time::Duration;

use serde::Deserialize;
use serde::Serialize;
use tui_pane::AppConfig;
use tui_pane::AppIdentity;
use tui_pane::AppearanceConfig;
use tui_pane::InitialRows;

use crate::constants::BINARY_NAME;
use crate::constants::CONFIG_DIRNAME;
use crate::constants::DEFAULT_CAPTURE_AUTO_INSTALL;
use crate::constants::DEFAULT_DARK_THEME;
use crate::constants::DEFAULT_EXCLUDED;
use crate::constants::DEFAULT_FADE_SECONDS;
use crate::constants::DEFAULT_HIDDEN_WHEN_IDLE;
use crate::constants::DEFAULT_LIGHT_THEME;
use crate::constants::MAX_FADE_SECONDS;

/// cargo-tile's identity for the framework's config and runner paths.
pub(crate) enum CargoTile {}

impl AppIdentity for CargoTile {
    const BINARY_NAME: &'static str = BINARY_NAME;
    const CONFIG_DIRNAME: &'static str = CONFIG_DIRNAME;
    const DEFAULT_LIGHT_THEME: &'static str = DEFAULT_LIGHT_THEME;
    const DEFAULT_DARK_THEME: &'static str = DEFAULT_DARK_THEME;
}

/// Which commands the grid holds back until they have work under them,
/// and which it never watches at all.
#[derive(Debug, Deserialize, Serialize)]
#[serde(default)]
pub(crate) struct CommandsConfig {
    /// Cargo subcommands the scan drops on sight, so they reach neither
    /// the summary nor the grid. For commands that are cargo by
    /// spelling and not by purpose: a hook firing `cargo berth` four
    /// times a second opens and closes a cell for each one, and the
    /// cell has nothing to draw because the command compiles nothing.
    /// Stronger than [`hidden_when_idle`](Self::hidden_when_idle),
    /// which keeps the summary line -- an excluded command is not
    /// tracked at all.
    pub(crate) excluded:         Vec<String>,
    /// Cargo subcommands that earn a cell of their own only while they
    /// are driving other cargo invocations. A terminal UI reached as a
    /// subcommand -- `cargo port` -- is open all day and compiles
    /// nothing on its own, so a cell for it while it sits there holds
    /// one row that says no more than the summary's line for it already
    /// does. It gets its cell the moment it starts an invocation, with
    /// that invocation under it. The summary line is never held back.
    pub(crate) hidden_when_idle: Vec<String>,
}

impl Default for CommandsConfig {
    fn default() -> Self {
        Self {
            excluded:         DEFAULT_EXCLUDED
                .iter()
                .map(|subcommand| (*subcommand).to_string())
                .collect(),
            hidden_when_idle: DEFAULT_HIDDEN_WHEN_IDLE
                .iter()
                .map(|subcommand| (*subcommand).to_string())
                .collect(),
        }
    }
}

/// How the tile grid grows.
#[derive(Debug, Deserialize, Serialize)]
#[serde(default)]
pub(crate) struct TilesConfig {
    /// Rows the grid grows to in a single column before it starts
    /// arranging itself into a square. Read through
    /// [`TilesConfig::initial_rows`], which enforces the floor.
    pub(crate) initial_rows: InitialRows,
    /// Seconds a finished row stays on screen, greyed, before it and any
    /// cell it leaves empty go. Read through
    /// [`TilesConfig::fade`], which enforces the ceiling.
    pub(crate) fade_seconds: u64,
}

impl Default for TilesConfig {
    fn default() -> Self {
        Self {
            initial_rows: InitialRows::default(),
            fade_seconds: DEFAULT_FADE_SECONDS,
        }
    }
}

impl TilesConfig {
    /// Rows the single column grows to, never below one; see
    /// [`InitialRows::get`].
    pub(crate) fn initial_rows(&self) -> usize { self.initial_rows.get() }

    /// How long a finished row lingers before the display lets go of it,
    /// clamped on read for the same reason as
    /// [`initial_rows`](Self::initial_rows).
    pub(crate) fn fade(&self) -> Duration {
        Duration::from_secs(self.fade_seconds.min(MAX_FADE_SECONDS))
    }
}

/// How the grid installs the capture shim. The former `roots` key no longer
/// exists; old entries are ignored because every account uses the shared parent.
#[derive(Debug, Deserialize, Serialize)]
#[serde(default)]
pub(crate) struct CaptureConfig {
    /// Put the capture shim in front of every toolchain's cargo when
    /// the grid opens, and bring an installed one up to date. On, the
    /// grid reports progress from its first launch and repairs itself
    /// after `rustup update`; off, the shim is only ever touched by
    /// `cargo tile install` and `cargo tile uninstall`. The grid never
    /// takes the shim out on its own either way.
    pub(crate) auto_install: bool,
}

impl Default for CaptureConfig {
    fn default() -> Self {
        Self {
            auto_install: DEFAULT_CAPTURE_AUTO_INSTALL,
        }
    }
}

/// Parsed `config.toml`. Every section defaults, so a missing file and
/// an empty file behave the same.
#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub(crate) struct Config {
    /// `[appearance]` — theme selection.
    pub(crate) appearance: AppearanceConfig<CargoTile>,
    /// `[capture]` — automatic shim installation.
    pub(crate) capture:    CaptureConfig,
    /// `[commands]` — which commands the grid holds back while idle.
    pub(crate) commands:   CommandsConfig,
    /// `[tiles]` — how the tile grid grows.
    pub(crate) tiles:      TilesConfig,
}

impl AppConfig for Config {
    type Identity = CargoTile;

    fn appearance(&self) -> &AppearanceConfig<CargoTile> { &self.appearance }

    fn appearance_mut(&mut self) -> &mut AppearanceConfig<CargoTile> { &mut self.appearance }

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

    /// [`LoadedConfig::load`] writes the file back whenever it does not already hold
    /// every setting, so a file that does hold them must serialize to
    /// itself -- otherwise every startup rewrites the config, for good.
    #[test]
    fn a_complete_config_is_left_alone() {
        let complete =
            toml::to_string_pretty(&Config::default()).expect("a config should serialize");
        assert_eq!(round_trip(&complete), complete);
    }

    /// The case the write exists for: a file written before a section
    /// existed comes back carrying it.
    #[test]
    fn a_config_missing_a_section_gains_it() {
        let old = "[appearance]\nmode = \"dark\"\n";
        let restated = round_trip(old);
        assert_ne!(restated, old);
        assert!(restated.contains("[capture]"));
        assert!(!restated.contains("roots ="));
        assert!(restated.contains("[commands]"));
        assert!(restated.contains("[tiles]"));
        // What the file did say survives the rewrite; only what it left
        // out is filled in.
        assert!(restated.contains("mode = \"dark\""));
    }

    /// Every key the default config writes, in the order and spelling
    /// `config.toml` has always carried. A change here rewrites every
    /// user's file on their next startup.
    #[test]
    fn the_default_config_serializes_to_the_known_file() {
        let expected = "\
[appearance]
mode = \"auto\"
light_theme = \"Default Light\"
dark_theme = \"Default Dark\"
iterm2_profile = \"cargo-tile\"

[capture]
auto_install = true

[commands]
excluded = [\"berth\"]
hidden_when_idle = [\"port\"]

[tiles]
initial_rows = 4
fade_seconds = 3
";
        let written =
            toml::to_string_pretty(&Config::default()).expect("a config should serialize");
        assert_eq!(written, expected);
    }

    /// A file that leaves `iterm2_profile` and the theme ids out gets the
    /// app's own defaults for them when it is restated.
    #[test]
    fn a_config_missing_iterm2_profile_restates_the_binary_name() {
        let restated = round_trip("[appearance]\nmode = \"dark\"\n");
        assert!(restated.contains("iterm2_profile = \"cargo-tile\""));
        assert!(restated.contains("light_theme = \"Default Light\""));
    }

    #[test]
    fn obsolete_roots_are_ignored_and_auto_install_is_preserved() {
        for roots in ["[]", "[\"/old/captures\"]", "42"] {
            let text = format!("[capture]\nauto_install = false\nroots = {roots}\n");
            let config: Config = toml::from_str(&text).expect("obsolete keys should be ignored");
            assert!(!config.capture.auto_install);
            let restated = round_trip(&text);
            assert!(!restated.contains("roots"));
            assert_eq!(round_trip(&restated), restated);
        }
    }
}
