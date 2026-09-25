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
use tui_pane::apply_settings;
use tui_pane::join_list;
use tui_pane::list_display;
use tui_pane::parse_list;
use tui_pane::step_framework_setting;

use crate::app::App;
use crate::config::CargoHandler;
use crate::constants::MACHINES_SETTINGS_SECTION;
use crate::constants::REMOTE_MACHINES_LABEL;
use crate::constants::TILES_SETTINGS_SECTION;

/// cargo-handler's own rows; the framework steps the rest.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AppSetting {
    /// `machines.remote`, typed in as a list of ssh host names.
    RemoteMachines,
}

/// Build the settings rows for the current frame.
pub(crate) fn rows(app: &App) -> SettingsRows<AppSetting> {
    let config = &app.loaded_config.config;
    let mut out = SettingsRows::new();

    out.appearance(&config.appearance);

    out.section(TILES_SETTINGS_SECTION);
    out.initial_rows(config.tiles.initial_rows);

    out.section(MACHINES_SETTINGS_SECTION);
    out.text(
        AppSetting::RemoteMachines,
        REMOTE_MACHINES_LABEL,
        list_display(&config.machines.remote),
    );

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
        Some(
            SettingTarget::App(AppSetting::RemoteMachines)
            | SettingTarget::AppText(_)
            | SettingTarget::ReadOnly,
        )
        | None => {},
    }
}

/// The remote list as the text its editor opens on, when the selection
/// is the typed `remote` row.
pub(crate) fn selected_text(app: &App) -> Option<String> {
    remote_row_selected(app).then(|| join_list(&app.loaded_config.config.machines.remote))
}

/// Replace the remote list with what was typed, write `config.toml`,
/// and apply it: the scheduler probes the new list from its next round.
pub(crate) fn commit_text(app: &mut App, text: &str) {
    if !remote_row_selected(app) {
        return;
    }
    set_remote_machines(app, parse_list(text));
    apply_settings(&mut app.loaded_config, &mut app.startup_note);
}

/// Replace the remote list in the config, in the scheduler's copy, and
/// in the census, which forgets the answers of hosts taken out.
fn set_remote_machines(app: &mut App, hosts: Vec<String>) {
    app.remote_machines.replace(hosts.clone());
    app.census.retain_remotes(&hosts);
    app.loaded_config.config.machines.remote = hosts;
}

/// Whether the settings selection is on the typed `remote` row.
fn remote_row_selected(app: &App) -> bool {
    let selection = app.framework.settings_pane.viewport().pos();
    rows(app).target(selection) == Some(SettingTarget::AppText(AppSetting::RemoteMachines))
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

#[cfg(test)]
#[expect(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use super::*;
    use crate::census::CensusUpdate;
    use crate::census::MachineState;

    /// A typed list reaches the config, the scheduler's copy and the
    /// census, which drops the answer of a host taken out.
    #[test]
    fn a_new_remote_list_reaches_the_config_the_scheduler_and_the_census() {
        let mut app = App::new_for_test().expect("test app should build");
        app.census.apply(CensusUpdate::Remote {
            host:  "studio".to_string(),
            state: MachineState::Failed("timed out".to_string()),
        });
        let hosts = parse_list("mac, pi");

        set_remote_machines(&mut app, hosts.clone());

        assert_eq!(app.loaded_config.config.machines.remote, hosts);
        assert_eq!(app.remote_machines.snapshot(), hosts);
        let configured = ["studio".to_string()];
        assert_eq!(
            app.census.machines(&configured)[1].state,
            &MachineState::Scanning
        );
    }
}
