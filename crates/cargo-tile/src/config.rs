//! `cargo-tile` configuration: the `[appearance]` section of
//! `<os config dir>/cargo-tile/config.toml`, plus the sibling paths for
//! the keymap file and the themes directory.

use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use serde::Deserialize;
use serde::Serialize;

use crate::constants::CONFIG_DIRNAME;
use crate::constants::CONFIG_FILENAME;
use crate::constants::DEFAULT_CAPTURE_AUTO_INSTALL;
use crate::constants::DEFAULT_DARK_THEME;
use crate::constants::DEFAULT_EXCLUDED;
use crate::constants::DEFAULT_FADE_SECONDS;
use crate::constants::DEFAULT_HIDDEN_WHEN_IDLE;
use crate::constants::DEFAULT_INITIAL_ROWS;
use crate::constants::DEFAULT_ITERM2_PROFILE;
use crate::constants::DEFAULT_LIGHT_THEME;
use crate::constants::FAVORITES_FILENAME;
use crate::constants::KEYMAP_FILENAME;
use crate::constants::MAX_FADE_SECONDS;
use crate::constants::MIN_INITIAL_ROWS;
use crate::constants::THEMES_DIRNAME;

/// Which appearance the app resolves at startup and which theme id
/// serves each one. Theme ids name a variant from
/// [`crate::theme`]'s registry: one of the app's own built-ins, or one
/// declared in a `themes/*.toml` file.
#[derive(Debug, Deserialize, Serialize)]
#[serde(default)]
pub(crate) struct AppearanceConfig {
    /// `auto` follows the terminal, `light` and `dark` pin one.
    pub(crate) mode:           String,
    /// Theme id used when the resolved appearance is light.
    pub(crate) light_theme:    String,
    /// Theme id used when the resolved appearance is dark.
    pub(crate) dark_theme:     String,
    /// iTerm2 profile the session adopts while the app runs, switched
    /// back to the one it came in on at exit. Empty leaves the session
    /// alone, and so does every terminal that is not iTerm2.
    pub(crate) iterm2_profile: String,
}

impl Default for AppearanceConfig {
    fn default() -> Self {
        Self {
            mode:           "auto".to_string(),
            light_theme:    DEFAULT_LIGHT_THEME.to_string(),
            dark_theme:     DEFAULT_DARK_THEME.to_string(),
            iterm2_profile: DEFAULT_ITERM2_PROFILE.to_string(),
        }
    }
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
    pub(crate) initial_rows: usize,
    /// Seconds a finished row stays on screen, greyed, before it and any
    /// cell it leaves empty go. Read through
    /// [`TilesConfig::fade`], which enforces the ceiling.
    pub(crate) fade_seconds: u64,
}

impl Default for TilesConfig {
    fn default() -> Self {
        Self {
            initial_rows: DEFAULT_INITIAL_ROWS,
            fade_seconds: DEFAULT_FADE_SECONDS,
        }
    }
}

impl TilesConfig {
    /// Rows the single column grows to, never below one.
    ///
    /// Clamped on read rather than at load so a hand-edited zero in
    /// `config.toml` is corrected rather than rejected -- the file keeps
    /// what was typed, the grid stays laid out.
    pub(crate) fn initial_rows(&self) -> usize { self.initial_rows.max(MIN_INITIAL_ROWS) }

    /// How long a finished row lingers before the display lets go of it,
    /// clamped on read for the same reason as
    /// [`initial_rows`](Self::initial_rows).
    pub(crate) fn fade(&self) -> Duration {
        Duration::from_secs(self.fade_seconds.min(MAX_FADE_SECONDS))
    }
}

/// Whether the grid stands the capture shim up on its own.
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
    pub(crate) appearance: AppearanceConfig,
    /// `[capture]` — whether the grid installs the shim itself.
    pub(crate) capture:    CaptureConfig,
    /// `[commands]` — which commands the grid holds back while idle.
    pub(crate) commands:   CommandsConfig,
    /// `[tiles]` — how the tile grid grows.
    pub(crate) tiles:      TilesConfig,
}

/// A load attempt: the config that will be used, plus the parse error
/// that made it fall back to defaults.
pub(crate) struct LoadedConfig {
    /// The config the app runs with.
    pub(crate) config: Config,
    /// Parse failure text, surfaced in the settings overlay.
    pub(crate) error:  Option<String>,
}

/// Read `config.toml`, falling back to defaults when it is absent.
///
/// A parse error is not fatal: the app runs on defaults and reports the
/// error through [`LoadedConfig::error`], because the terminal is not
/// yet in raw mode here and a panic would leave nothing on screen.
pub(crate) fn load() -> LoadedConfig {
    let Some(path) = config_path() else {
        return LoadedConfig {
            config: Config::default(),
            error:  None,
        };
    };
    let Ok(text) = fs::read_to_string(&path) else {
        return LoadedConfig {
            config: Config::default(),
            error:  None,
        };
    };
    toml::from_str(&text).map_or_else(
        |error| LoadedConfig {
            config: Config::default(),
            error:  Some(error.to_string()),
        },
        |config| {
            let error = restate(&config, &text);
            LoadedConfig { config, error }
        },
    )
}

/// Write a parsed config back over the file it came from when the file
/// does not already say the same thing.
///
/// A file written before a setting existed does not mention it, which
/// leaves that setting editable only by someone who already knows its
/// name. Writing the parsed config back spells out every section at its
/// default, so the file lists the whole of what can be set. It is a
/// no-op once the file holds everything, and it is only reached on a
/// file that parsed -- one with a typo in it is left alone for its
/// author to fix rather than overwritten.
fn restate(config: &Config, text: &str) -> Option<String> {
    match toml::to_string_pretty(config) {
        Ok(restated) if restated == text => None,
        Ok(_) => save(config),
        Err(error) => Some(error.to_string()),
    }
}

/// Write `config.toml`, creating the config directory when it is
/// missing.
///
/// Returns the failure text for the settings overlay rather than an
/// error type: every caller renders it, none of them recover.
pub(crate) fn save(config: &Config) -> Option<String> {
    let Some(path) = config_path() else {
        return Some(format!(
            "no OS config directory: cannot write {CONFIG_FILENAME}"
        ));
    };
    let text = match toml::to_string_pretty(config) {
        Ok(text) => text,
        Err(error) => return Some(error.to_string()),
    };
    if let Some(parent) = path.parent()
        && let Err(error) = fs::create_dir_all(parent)
    {
        return Some(format!("{}: {error}", parent.display()));
    }
    fs::write(&path, text)
        .err()
        .map(|error| format!("{}: {error}", path.display()))
}

/// `<os config dir>/cargo-tile/config.toml`.
pub(crate) fn config_path() -> Option<PathBuf> {
    config_root().map(|dir| dir.join(CONFIG_FILENAME))
}

/// `<os config dir>/cargo-tile/favorites.toml`.
pub(crate) fn favorites_path() -> Option<PathBuf> {
    config_root().map(|dir| dir.join(FAVORITES_FILENAME))
}

/// `<os config dir>/cargo-tile/keymap.toml`.
pub(crate) fn keymap_path() -> Option<PathBuf> {
    config_root().map(|dir| dir.join(KEYMAP_FILENAME))
}

/// `<os config dir>/cargo-tile/themes`.
pub(crate) fn themes_dir() -> Option<PathBuf> { config_root().map(|dir| dir.join(THEMES_DIRNAME)) }

/// `<os config dir>/cargo-tile`. `None` on platforms where the OS
/// config directory cannot be resolved.
fn config_root() -> Option<PathBuf> { dirs::config_dir().map(|dir| dir.join(CONFIG_DIRNAME)) }

#[cfg(test)]
#[expect(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use super::*;

    /// What [`restate`] compares against, for a config that has been
    /// through the file and back.
    fn round_trip(text: &str) -> String {
        let config: Config = toml::from_str(text).expect("a config the test wrote should parse");
        toml::to_string_pretty(&config).expect("a config should serialize")
    }

    /// [`load`] writes the file back whenever it does not already hold
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
        assert!(restated.contains("[commands]"));
        assert!(restated.contains("[tiles]"));
        // What the file did say survives the rewrite; only what it left
        // out is filled in.
        assert!(restated.contains("mode = \"dark\""));
    }
}
