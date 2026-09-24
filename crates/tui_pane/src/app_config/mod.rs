//! An app's `config.toml`, plus the sibling paths for the keymap file
//! and the themes directory.
//!
//! [`AppIdentity`] names the directory under the OS config root the
//! files live in and the `[appearance]` defaults the app's own themes
//! answer to. [`LoadedConfig`] reads the app's own serde config struct
//! from that directory, writes back a file that does not yet spell out
//! every setting, and saves edits. [`AppearanceConfig`] is the
//! `[appearance]` table that struct carries, and [`install_theme`]
//! resolves it against the app's built-in themes plus the user's
//! `themes/` directory and installs the result process-wide.
//! [`InitialRows`] is the one `[tiles]` key the framework owns; the
//! app keeps declaring the table around it.
//!
//! Distinct from `settings_store`, whose `SettingsStore` round-trips a
//! `toml::Table`: this path serializes the app's struct directly, so
//! the file keeps the struct's declaration order.

mod appearance;
mod constants;
mod identity;
mod initial_rows;
mod loaded;
mod theme_install;

pub use appearance::AppearanceConfig;
pub use identity::AppIdentity;
pub use initial_rows::InitialRows;
pub use loaded::LoadedConfig;
pub use theme_install::install_theme;
pub(crate) use theme_install::resolve_appearance;
