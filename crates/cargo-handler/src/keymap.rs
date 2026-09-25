//! Keymap assembly.
//!
//! The framework globals — `s` settings, ctrl-k keymap, `?` shortcuts,
//! `q` quit, `R` restart, `x` dismiss, Tab pane cycling — come from
//! [`tui_pane::GlobalAction`]'s defaults and need no registration.
//! `register_navigation` folds in the movement scope
//! ([`SettingsNavigation`], which moves through the settings overlay),
//! `register_globals` this app's own globals scope (see
//! [`crate::globals`]), `register_overlay` the overlay-local scope, and
//! `register_pane` installs the grid in the framework's pane registry.
//! `keymap.toml`, when present, overrides any of it.
//!
//! The attract panes and the favorites overlay register through
//! `register` rather than `register_pane` because they carry keys of
//! their own. An attract pane is a scope without a rectangle: the
//! attract screen draws over the whole terminal, and the registration
//! exists so its keys are bindable separately from every other scope.
//! See [`tui_pane::Attract`].

use std::path::PathBuf;

use tui_pane::FavoritesOverlayPane;
use tui_pane::Framework;
use tui_pane::FrameworkGlobalShortcutPresentation;
use tui_pane::FrameworkGlobalShortcutVisibility;
use tui_pane::GlobalAction;
use tui_pane::Keymap;
use tui_pane::KeymapError;
use tui_pane::MovingBandPane;
use tui_pane::MovingTextPane;
use tui_pane::PixelatePane;
use tui_pane::SettingsNavigation;
use tui_pane::TileGridPane;

use crate::app::App;
use crate::globals::AppGlobalAction;

/// Which framework globals the `?` overlay lists. `x` dismisses
/// nothing this app shows, so its row stays out of the overlay while
/// the key keeps its binding.
const fn framework_global_shortcut_visibility(
    action: GlobalAction,
) -> FrameworkGlobalShortcutVisibility {
    match action {
        GlobalAction::Dismiss => FrameworkGlobalShortcutVisibility::Hidden,
        GlobalAction::Quit
        | GlobalAction::Restart
        | GlobalAction::NextPane
        | GlobalAction::PrevPane
        | GlobalAction::OpenKeymap
        | GlobalAction::OpenSettings
        | GlobalAction::OpenGlobalShortcuts => FrameworkGlobalShortcutVisibility::Shown,
    }
}

/// Assemble the keymap and install its pane registry on `framework`.
///
/// Built in [`ignore_unknown_entries`](tui_pane::KeymapBuilder::ignore_unknown_entries)
/// mode so a stale `keymap.toml` entry is skipped rather than failing
/// startup.
pub(crate) fn build_keymap(
    framework: &mut Framework<App>,
    keymap_path: Option<PathBuf>,
) -> Result<Keymap<App>, KeymapError> {
    let mut builder = Keymap::builder()
        .ignore_unknown_entries()
        .framework_global_shortcut_presentation(FrameworkGlobalShortcutPresentation::new(
            framework_global_shortcut_visibility,
        ));
    if let Some(path) = keymap_path {
        builder = builder.config_path(path.clone());
        if path.is_file() {
            builder = builder.load_toml(path)?;
        }
    }
    builder
        .register_navigation::<SettingsNavigation<App>>()?
        .register_globals::<AppGlobalAction>()?
        .register_overlay()?
        .register_pane::<TileGridPane<App>>()
        .register(MovingBandPane::<App>::default())
        .register(MovingTextPane::<App>::default())
        .register(PixelatePane::<App>::default())
        .register(FavoritesOverlayPane::<App>::default())
        .build_into(framework)
}

#[cfg(test)]
#[expect(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use tui_pane::AttractMode;
    use tui_pane::FocusedPane;
    use tui_pane::Framework;
    use tui_pane::Keymap;
    use tui_pane::KeymapHelpRowKind;
    use tui_pane::KeymapPane;

    use super::build_keymap;
    use crate::app::App;
    use crate::app::AppPaneId;

    /// The keymap the app starts with, built from the defaults alone.
    fn default_keymap() -> Keymap<App> {
        let mut framework = Framework::new(FocusedPane::App(AppPaneId::Main));
        build_keymap(&mut framework, None).expect("the app's keymap must assemble")
    }

    /// Every registration the app makes has to agree with the
    /// framework's rules about what a complete keymap holds, and
    /// nothing but assembling one says whether it does. Registering a
    /// pane's shortcuts is what makes a navigation scope mandatory.
    #[test]
    fn the_app_assembles_a_keymap_with_a_navigation_scope() {
        let keymap = default_keymap();

        assert!(
            keymap.navigation().is_some(),
            "the settings overlay moves on the navigation scope"
        );
    }

    /// `keymap.toml` is hand-edited, so the table each attract mode's
    /// keys and the favorites overlay's keys are read from keeps its
    /// name wherever the scope is declared.
    #[test]
    fn attract_and_favorites_scopes_keep_their_table_names() {
        let keymap = default_keymap();
        let table_names = [
            AttractMode::MovingBand,
            AttractMode::MovingText,
            AttractMode::Pixelate,
        ]
        .map(|mode| keymap.scope_toml_name_for(AppPaneId::Attract(mode)));

        assert_eq!(
            table_names,
            [
                Some("attract_moving_band"),
                Some("attract_moving_text"),
                Some("attract_pixelate"),
            ]
        );
        assert_eq!(
            keymap.scope_toml_name_for(AppPaneId::Favorites),
            Some("favorites")
        );
    }

    #[test]
    fn the_shortcut_overlay_omits_dismiss() {
        let keymap = default_keymap();

        assert!(
            keymap
                .global_shortcut_rows()
                .iter()
                .all(|row| row.action != "dismiss"),
            "the compact shortcut overlay must omit the inactive x Dismiss row"
        );
    }

    /// Section names, TOML tables, action keys, default binds and their
    /// order, captured from the first build: cargo-tile's rows without
    /// its `freeze` and `process_tree` globals.
    #[test]
    fn keymap_overlay_rows_keep_their_order() {
        let app = App::new_for_test().expect("test app should build");
        let rows: Vec<String> = KeymapPane::ordered_help_rows(&app, &app.keymap)
            .iter()
            .map(|row| match row.row_kind {
                KeymapHelpRowKind::Header => format!("[{}] {}", row.section, row.scope),
                KeymapHelpRowKind::Action => format!(
                    "{}.{} = {}",
                    row.scope,
                    row.action,
                    row.bind
                        .as_ref()
                        .map_or_else(String::new, ToString::to_string)
                ),
            })
            .collect();
        assert_eq!(rows, EXPECTED_OVERLAY_ROWS);
    }

    /// The captured overlay rows.
    const EXPECTED_OVERLAY_ROWS: [&str; 89] = [
        "[Global Navigation] global",
        "global.next_pane = tab",
        "global.prev_pane = shift-tab",
        "[Global Shortcuts] global",
        "global.add_tile = +",
        "global.dismiss = x",
        "global.focus_up = up",
        "global.focus_down = down",
        "global.focus_left = left",
        "global.focus_right = right",
        "global.open_favorites = ctrl-o",
        "global.open_keymap = ctrl-k",
        "global.open_settings = s",
        "global.quit = q",
        "global.randomize_attract = r",
        "global.remove_tile = -",
        "global.restart = R",
        "global.save_favorite = ctrl-s",
        "global.random_favorite = m",
        "global.open_global_shortcuts = ?",
        "global.attract = a",
        "global.undo_attract_replacement = u",
        "[Navigation] navigation",
        "navigation.half_page_down = ",
        "navigation.half_page_up = ",
        "navigation.end = end",
        "navigation.home = home",
        "navigation.down = down",
        "navigation.left = left",
        "navigation.right = right",
        "navigation.up = up",
        "navigation.page_down = pagedown",
        "navigation.page_up = pageup",
        "[Attract: Moving Band] attract_moving_band",
        "attract_moving_band.cycle_fraying = v",
        "attract_moving_band.tail_faster = ]",
        "attract_moving_band.tail_slower = [",
        "attract_moving_band.travel_down = down",
        "attract_moving_band.travel_left = left",
        "attract_moving_band.travel_right = right",
        "attract_moving_band.travel_up = up",
        "attract_moving_band.show_moving_band = 1",
        "attract_moving_band.show_moving_text = 2",
        "attract_moving_band.show_pixelate = 3",
        "attract_moving_band.slower = <",
        "attract_moving_band.faster = >",
        "attract_moving_band.thinner = -",
        "attract_moving_band.wider = +",
        "[Attract: Moving Text] attract_moving_text",
        "attract_moving_text.spread_narrower = [",
        "attract_moving_text.cycle_drift = v",
        "attract_moving_text.travel_down = down",
        "attract_moving_text.travel_left = left",
        "attract_moving_text.travel_right = right",
        "attract_moving_text.travel_up = up",
        "attract_moving_text.cycle_fill = t",
        "attract_moving_text.spread_wider = ]",
        "attract_moving_text.show_moving_band = 1",
        "attract_moving_text.show_moving_text = 2",
        "attract_moving_text.show_pixelate = 3",
        "attract_moving_text.slower = <",
        "attract_moving_text.faster = >",
        "[Attract: Pixelate] attract_pixelate",
        "attract_pixelate.cycle_resolve = v",
        "attract_pixelate.coarser = +",
        "attract_pixelate.sharper = -",
        "attract_pixelate.wave_narrower = [",
        "attract_pixelate.wave_wider = ]",
        "attract_pixelate.cycle_fill = t",
        "attract_pixelate.show_moving_band = 1",
        "attract_pixelate.show_moving_text = 2",
        "attract_pixelate.show_pixelate = 3",
        "attract_pixelate.slower = <",
        "attract_pixelate.faster = >",
        "attract_pixelate.sweep_down = down",
        "attract_pixelate.sweep_left = left",
        "attract_pixelate.sweep_right = right",
        "attract_pixelate.sweep_up = up",
        "[Favorites] favorites",
        "favorites.close = escape",
        "favorites.delete = x",
        "favorites.load = enter",
        "favorites.select_next = down",
        "favorites.select_previous = up",
        "favorites.page_columns_right = right",
        "favorites.page_columns_left = left",
        "[Overlay] overlay",
        "overlay.cancel = escape",
        "overlay.start_edit = enter",
    ];
}
