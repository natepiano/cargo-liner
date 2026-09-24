//! The settings overlay an app built on the framework shows under `s`:
//! the rows it lists, the stepping that edits them, and the drawing.
//!
//! The app composes its rows with [`SettingsRows`], calling the
//! framework's builders for the rows the framework owns (the three
//! `[appearance]` steppers, `tiles.initial_rows`, the Files paths and
//! the Notices) and adding its own sections, steppers and read-only
//! values between them. Each selectable row carries a
//! [`SettingTarget`]: a [`FrameworkSetting`] the framework steps with
//! [`step_framework_setting`], one of the app's own settings the app
//! steps and then hands to [`apply_settings`], or a read-only row.
//! Stepping writes `config.toml` and swaps the active theme in place.
//! Nothing here opens a text editor, so the overlay never has a mode
//! the user has to type their way out of.
//!
//! [`draw_settings`] fits the popup to the widest row and draws it;
//! [`SettingsNavigation`] is the navigation scope that moves through
//! the rows, with the sideways keys stepping the selected value.

mod constants;
mod draw;
mod navigation;
mod rows;
mod step;

pub use draw::draw_settings;
pub use navigation::SettingsHost;
pub use navigation::SettingsNavigation;
pub use rows::SettingTarget;
pub use rows::SettingsRows;
pub use step::AppConfig;
pub use step::FrameworkSetting;
pub use step::SettingStep;
pub use step::apply_settings;
pub use step::step_framework_setting;
pub use step::stepped;
