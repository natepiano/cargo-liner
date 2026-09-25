//! cargo-handler's rows in the framework settings overlay, and the
//! stepping that edits them.
//!
//! The framework owns every row here: the `[appearance]` steppers,
//! `initial rows`, the Files paths and the Notices section. This module
//! places them. Every stepper walks its allowed values on
//! Left/Right/Enter, writes `config.toml`, and swaps the active theme
//! in place. Every other row reports state and is inert.

use tui_pane::SettingStep;
use tui_pane::SettingTarget;
use tui_pane::SettingsRows;
use tui_pane::step_framework_setting;

use crate::app::App;
use crate::config::CargoHandler;
use crate::constants::TILES_SETTINGS_SECTION;

/// cargo-handler's own stepper rows. Uninhabited: the framework steps
/// every row the overlay has.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AppSetting {}

/// Build the settings rows for the current frame.
pub(crate) fn rows(app: &App) -> SettingsRows<AppSetting> {
    let config = &app.loaded_config.config;
    let mut out = SettingsRows::new();

    out.appearance(&config.appearance);

    out.section(TILES_SETTINGS_SECTION);
    out.initial_rows(config.tiles.initial_rows);

    out.files::<CargoHandler>();

    out.notices(&notices(app));
    out
}

/// Step the selected row's value, then persist and apply the result.
///
/// A read-only row is a no-op, so the keys stay harmless everywhere in
/// the overlay.
pub(crate) fn cycle(app: &mut App, step: SettingStep) {
    let selection = app.framework.settings_pane.viewport().pos();
    match rows(app).target(selection) {
        Some(SettingTarget::Framework(setting)) => {
            step_framework_setting(setting, step, &mut app.loaded_config, &mut app.startup_note);
        },
        Some(SettingTarget::App(setting)) => match setting {},
        Some(SettingTarget::ReadOnly) | None => {},
    }
}

/// Outstanding startup notices, kept visible after their toasts
/// disappear: the theme note, then the config error.
fn notices(app: &App) -> Vec<(&'static str, &str)> {
    let mut notices = Vec::new();
    if let Some(note) = &app.startup_note {
        notices.push(("theme", note.as_str()));
    }
    if let Some(error) = &app.loaded_config.error {
        notices.push(("config", error.as_str()));
    }
    notices
}
