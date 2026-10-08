//! cargo-handler's rows in the framework settings overlay, and the
//! stepping that edits them.
//!
//! The framework owns every row here but `remote`: the `[appearance]`
//! steppers, `initial rows`, `fill`, `widen summary`, the Files paths
//! and the Notices section. This module places them. Every stepper
//! walks its allowed values on Left/Right/Enter, writes `config.toml`,
//! and swaps the active theme in place. Every other row reports state
//! and is inert.

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
    out.tile_fill(config.tiles.fill);
    out.widen_summary(config.tiles.widen_summary);

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
    use tui_pane::Appearance;
    use tui_pane::FrameworkSetting;
    use tui_pane::TileFill;

    use super::*;
    use crate::census::CensusUpdate;
    use crate::census::MachineState;

    #[test]
    fn applying_an_appearance_replaces_the_theme_notice() {
        let mut app = App::new_for_test().expect("test app should build");
        app.startup_note = Some("earlier notice".to_string());
        app.loaded_config.config.appearance.light_theme = "Missing Light".to_string();

        app.apply_system_appearance(Appearance::Light);

        assert_eq!(
            app.startup_note.as_deref(),
            Some("theme `Missing Light` not found — using a built-in")
        );
    }

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

    /// The value the `fill` row shows.
    fn fill_row_value(app: &App) -> String {
        rows(app)
            .rows()
            .iter()
            .find(|row| row.label == "fill")
            .map(|row| row.value.clone())
            .expect("the settings list a fill row")
    }

    /// Stepping the `fill` row walks the config between its two values
    /// and the row follows it. The save goes to the test build's fixed
    /// config root, never the user's file.
    #[test]
    fn stepping_the_fill_row_walks_both_values() {
        let mut app = App::new_for_test().expect("test app should build");
        let settings = rows(&app);
        let selection = (0..settings.rows().len())
            .find(|&selection| {
                settings.target(selection)
                    == Some(SettingTarget::Framework(FrameworkSetting::TileFill))
            })
            .expect("the settings list a fill row");
        app.framework
            .settings_pane
            .viewport_mut()
            .set_pos(selection);
        assert_eq!(app.loaded_config.config.tiles.fill, TileFill::Redistribute);
        assert_eq!(fill_row_value(&app), "redistribute");

        cycle(&mut app, SettingStep::Next);
        assert_eq!(app.loaded_config.config.tiles.fill, TileFill::AddNew);
        assert_eq!(fill_row_value(&app), "add_new");
        assert_eq!(
            app.loaded_config.config.tiles.growth().fill,
            TileFill::AddNew
        );

        cycle(&mut app, SettingStep::Prev);
        assert_eq!(app.loaded_config.config.tiles.fill, TileFill::Redistribute);
        assert_eq!(fill_row_value(&app), "redistribute");
    }

    /// Stepping the `transparent` row flips the config, the row, and
    /// whether the screen is left see-through. The save goes to the
    /// test build's fixed config root, never the user's file.
    #[test]
    fn stepping_the_transparent_row_paints_the_screen_solid_and_back() {
        let mut app = App::new_for_test().expect("test app should build");
        let settings = rows(&app);
        let selection = (0..settings.rows().len())
            .find(|&selection| {
                settings.target(selection)
                    == Some(SettingTarget::Framework(FrameworkSetting::Transparent))
            })
            .expect("the settings list a transparent row");
        app.framework
            .settings_pane
            .viewport_mut()
            .set_pos(selection);
        let row_value = |app: &App| {
            rows(app)
                .rows()
                .iter()
                .find(|row| row.label == "transparent")
                .map(|row| row.value.clone())
                .expect("the settings list a transparent row")
        };
        assert!(app.loaded_config.config.appearance.transparent);
        assert_eq!(row_value(&app), "true");

        cycle(&mut app, SettingStep::Next);
        assert!(!app.loaded_config.config.appearance.transparent);
        assert_eq!(row_value(&app), "false");
        assert!(!tui_pane::transparent_background());

        cycle(&mut app, SettingStep::Prev);
        assert!(app.loaded_config.config.appearance.transparent);
        assert_eq!(row_value(&app), "true");
        assert!(tui_pane::transparent_background());
    }
}
