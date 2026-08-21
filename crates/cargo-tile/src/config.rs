//! `cargo-tile` configuration: the `[appearance]` section of
//! `<os config dir>/cargo-tile/config.toml`, plus the sibling paths for
//! the keymap file and the themes directory.

use std::fs;
use std::path::PathBuf;

use serde::Deserialize;
use serde::Serialize;
use tui_pane::BUILTIN_DARK_NAME;
use tui_pane::BUILTIN_LIGHT_NAME;

use crate::constants::CONFIG_DIRNAME;
use crate::constants::CONFIG_FILENAME;
use crate::constants::KEYMAP_FILENAME;
use crate::constants::THEMES_DIRNAME;

/// Which appearance the app resolves at startup and which theme id
/// serves each one. Theme ids name a variant from
/// [`crate::theme`]'s registry: a built-in, or one declared in a
/// `themes/*.toml` file.
#[derive(Debug, Deserialize, Serialize)]
#[serde(default)]
pub(crate) struct AppearanceConfig {
    /// `auto` follows the terminal, `light` and `dark` pin one.
    pub(crate) mode:        String,
    /// Theme id used when the resolved appearance is light.
    pub(crate) light_theme: String,
    /// Theme id used when the resolved appearance is dark.
    pub(crate) dark_theme:  String,
}

impl Default for AppearanceConfig {
    fn default() -> Self {
        Self {
            mode:        "auto".to_string(),
            light_theme: BUILTIN_LIGHT_NAME.to_string(),
            dark_theme:  BUILTIN_DARK_NAME.to_string(),
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
        |config| LoadedConfig {
            config,
            error: None,
        },
    )
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

/// `<os config dir>/cargo-tile/keymap.toml`.
pub(crate) fn keymap_path() -> Option<PathBuf> {
    config_root().map(|dir| dir.join(KEYMAP_FILENAME))
}

/// `<os config dir>/cargo-tile/themes`.
pub(crate) fn themes_dir() -> Option<PathBuf> { config_root().map(|dir| dir.join(THEMES_DIRNAME)) }

/// `<os config dir>/cargo-tile`. `None` on platforms where the OS
/// config directory cannot be resolved.
fn config_root() -> Option<PathBuf> { dirs::config_dir().map(|dir| dir.join(CONFIG_DIRNAME)) }
