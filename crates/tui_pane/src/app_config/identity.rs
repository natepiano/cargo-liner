//! [`AppIdentity`]: who an app is, for everything the framework keys by
//! name.

use std::path::PathBuf;

use super::constants::CONFIG_FILENAME;
use super::constants::KEYMAP_FILENAME;
use super::constants::THEMES_DIRNAME;
use crate::SettingsFileSpec;

/// Who an app is, for everything the framework keys by name.
///
/// The directory its files live in under the OS config root, the name
/// it reports errors and restarts under, and the `[appearance]`
/// defaults its own theme built-ins answer to.
///
/// Implemented on a type that exists only to carry these names, which
/// is then the type parameter of [`AppearanceConfig`](crate::AppearanceConfig)
/// and of [`LoadedConfig::load`](crate::LoadedConfig::load).
pub trait AppIdentity: 'static {
    /// The binary's own name: the prefix on every message the runner
    /// prints, and the executable a restart falls back to when the
    /// running one cannot be resolved.
    const BINARY_NAME: &'static str;
    /// Directory under the OS config root holding `config.toml`,
    /// `keymap.toml` and `themes/`.
    const CONFIG_DIRNAME: &'static str;
    /// `appearance.light_theme` default: the id of one of the app's
    /// built-in light variants.
    const DEFAULT_LIGHT_THEME: &'static str;
    /// `appearance.dark_theme` default: the id of one of the app's
    /// built-in dark variants.
    const DEFAULT_DARK_THEME: &'static str;
    /// `appearance.iterm2_profile` default. Sharing the binary's name
    /// keeps the pairing obvious from the iTerm2 side.
    const DEFAULT_ITERM2_PROFILE: &'static str = Self::BINARY_NAME;

    /// `<os config dir>/<CONFIG_DIRNAME>/config.toml`. `None` on
    /// platforms where the OS config directory cannot be resolved.
    #[must_use]
    fn config_path() -> Option<PathBuf> {
        SettingsFileSpec::new(Self::CONFIG_DIRNAME, CONFIG_FILENAME).resolved_path()
    }

    /// `<os config dir>/<CONFIG_DIRNAME>/keymap.toml`.
    #[must_use]
    fn keymap_path() -> Option<PathBuf> {
        SettingsFileSpec::new(Self::CONFIG_DIRNAME, KEYMAP_FILENAME).resolved_path()
    }

    /// `<os config dir>/<CONFIG_DIRNAME>/themes`.
    #[must_use]
    fn themes_dir() -> Option<PathBuf> {
        SettingsFileSpec::new(Self::CONFIG_DIRNAME, THEMES_DIRNAME).resolved_path()
    }
}
