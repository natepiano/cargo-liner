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
//! steps and then hands to [`apply_settings`], one the user types in,
//! or a read-only row. Stepping writes `config.toml` and swaps the
//! active theme in place. A typed row is the one way into a text
//! editor: Enter opens it on the row's current text, Enter again hands
//! the text to [`SettingsHost::commit_setting_text`], and Esc leaves the
//! setting as it was. A typed list reads through [`list_display`],
//! opens its editor on [`join_list`], and commits through
//! [`parse_list`].
//!
//! [`draw_settings`] fits the popup to the widest row and draws it;
//! [`SettingsNavigation`] is the navigation scope that moves through
//! the rows, with the sideways keys stepping the selected value.

mod constants;
mod draw;
mod list;
mod navigation;
mod rows;
mod step;

pub use constants::LIST_SEPARATOR;
pub use draw::draw_settings;
pub use list::join_list;
pub use list::list_display;
pub use list::parse_list;
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
