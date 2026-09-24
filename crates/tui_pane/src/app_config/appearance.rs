//! [`AppearanceConfig`]: the `[appearance]` table of an app's
//! `config.toml`.

use std::fmt;
use std::fmt::Formatter;
use std::marker::PhantomData;

use serde::Deserialize;
use serde::Serialize;

use super::constants::DEFAULT_APPEARANCE_MODE;
use super::identity::AppIdentity;

/// Which appearance the app resolves at startup and which theme id
/// serves each one.
///
/// Theme ids name a variant in the registry
/// [`install_theme`](crate::install_theme) builds: one of the app's own
/// built-ins, or one declared in a `themes/*.toml` file.
///
/// `I` supplies the defaults a missing key takes. The table is
/// `#[serde(default)]`, so a file that leaves a key out gets it from
/// [`Self::default`] as it is read, and a file restated by
/// [`LoadedConfig`](crate::LoadedConfig) spells out the app's own
/// defaults rather than a framework-wide one.
#[derive(Deserialize, Serialize)]
#[serde(default, bound = "")]
pub struct AppearanceConfig<I: AppIdentity> {
    /// `auto` follows the terminal, `light` and `dark` pin one.
    pub mode:           String,
    /// Theme id used when the resolved appearance is light.
    pub light_theme:    String,
    /// Theme id used when the resolved appearance is dark.
    pub dark_theme:     String,
    /// iTerm2 profile the session adopts while the app runs, switched
    /// back to the one it came in on at exit. Empty leaves the session
    /// alone, and so does every terminal that is not iTerm2.
    pub iterm2_profile: String,
    /// Ties the defaults to `I` without owning one, so the table is
    /// `Send` and `Sync` whatever `I` is.
    #[serde(skip)]
    identity:           PhantomData<fn() -> I>,
}

impl<I: AppIdentity> Default for AppearanceConfig<I> {
    fn default() -> Self {
        Self {
            mode:           DEFAULT_APPEARANCE_MODE.to_string(),
            light_theme:    I::DEFAULT_LIGHT_THEME.to_string(),
            dark_theme:     I::DEFAULT_DARK_THEME.to_string(),
            iterm2_profile: I::DEFAULT_ITERM2_PROFILE.to_string(),
            identity:       PhantomData,
        }
    }
}

impl<I: AppIdentity> fmt::Debug for AppearanceConfig<I> {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AppearanceConfig")
            .field("mode", &self.mode)
            .field("light_theme", &self.light_theme)
            .field("dark_theme", &self.dark_theme)
            .field("iterm2_profile", &self.iterm2_profile)
            .finish_non_exhaustive()
    }
}
