//! Keymap assembly — the `cargo-tile` counterpart of cargo-port's
//! `build_framework_keymap`.
//!
//! The framework globals — `s` settings, ctrl-k keymap, `?` shortcuts,
//! `q` quit, `R` restart, `x` dismiss, Tab pane cycling — come from
//! [`tui_pane::GlobalAction`]'s defaults and need no registration.
//! `register_navigation` folds in the movement scope
//! ([`SettingsNavigation`], which moves through the settings overlay),
//! `register_globals` this app's own globals
//! scope (empty in the template, see [`crate::globals`]),
//! `register_overlay` the overlay-local scope, and `register_pane`
//! installs each app pane in the framework's pane registry.
//! `keymap.toml`, when present, overrides any of it.
//!
//! [`MovingBandPane`] registers through `register` rather than
//! `register_pane` because it carries keys of its own. It is a scope
//! without a rectangle: the attract screen draws over the whole
//! terminal, and the registration exists so its keys are bindable
//! separately from every other scope. See [`crate::attract`].

use std::path::PathBuf;

use tui_pane::CycleDirection;
use tui_pane::Framework;
use tui_pane::FrameworkGlobalShortcutPresentation;
use tui_pane::FrameworkGlobalShortcutVisibility;
use tui_pane::GlobalAction;
use tui_pane::Keymap;
use tui_pane::KeymapError;
use tui_pane::Mode;
use tui_pane::Pane;
use tui_pane::SettingsNavigation;

use crate::app::App;
use crate::app::AppPaneId;
use crate::attract::MovingBandPane;
use crate::attract::MovingTextPane;
use crate::attract::PixelatePane;
use crate::favorites_overlay::FavoritesOverlayPane;
use crate::globals::AppGlobalAction;

/// `Pane<App>` host for the main content pane. No pane-local shortcuts
/// yet, so it registers through `register_pane` rather than `register`.
struct MainPane;

impl Pane<App> for MainPane {
    const APP_PANE_ID: AppPaneId = AppPaneId::Main;

    /// The grid is not a list. Its cells are walked by the app globals
    /// and by Tab, so the status line's navigation region has nothing
    /// to say about it, and `Static` is what keeps that region off.
    fn mode() -> fn(&App) -> Mode<App> { |_app| Mode::Static }

    /// Tab walks the tile grid rather than the framework's pane cycle.
    /// The app registers one pane and puts every command in a cell
    /// inside it, so the cells are what a developer means by "the next
    /// one" -- and the step never falls through, because there is no
    /// second pane behind the grid to fall through to.
    fn cycle_step() -> Option<fn(&mut App, CycleDirection) -> bool> {
        Some(|app, direction| app.tiles.cycle_focus(direction))
    }
}

const fn cargo_tile_framework_global_shortcut_visibility(
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
            cargo_tile_framework_global_shortcut_visibility,
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
        .register_pane::<MainPane>()
        .register(MovingBandPane)
        .register(MovingTextPane)
        .register(PixelatePane)
        .register(FavoritesOverlayPane)
        .build_into(framework)
}

#[cfg(test)]
#[expect(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use tui_pane::FocusedPane;
    use tui_pane::Framework;

    use super::build_keymap;
    use crate::app::AppPaneId;
    use crate::attract::AttractMode;

    /// Every registration the app makes has to agree with the
    /// framework's rules about what a complete keymap holds, and
    /// nothing but assembling one says whether it does. Registering a
    /// pane's shortcuts is what first made a navigation scope
    /// mandatory, and the app was shipped without one -- a failure the
    /// binary could only report by refusing to start.
    #[test]
    fn the_app_assembles_a_keymap_the_framework_accepts() {
        let mut framework = Framework::new(FocusedPane::App(AppPaneId::Main));
        let keymap = build_keymap(&mut framework, None).expect("the app's keymap must assemble");
        assert!(
            keymap.navigation().is_some(),
            "the settings overlay moves on the navigation scope"
        );
        for mode in [
            AttractMode::MovingBand,
            AttractMode::MovingText,
            AttractMode::Pixelate,
        ] {
            assert!(
                keymap
                    .scope_toml_name_for(AppPaneId::Attract(mode))
                    .is_some(),
                "{mode:?} must be rebindable under a table of its own"
            );
        }
        assert!(
            keymap
                .global_shortcut_rows()
                .iter()
                .all(|row| row.action != "dismiss"),
            "the compact shortcut overlay must omit cargo-tile's inactive x Dismiss row"
        );
    }
}

#[cfg(test)]
#[expect(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod overlay_tests {
    use tui_pane::KeymapHelpRowKind;
    use tui_pane::KeymapPane;

    use crate::app::App;

    /// Section names, TOML tables, action keys, default binds and their
    /// order, captured from the build before the navigation scope moved
    /// into `tui_pane`.
    #[test]
    fn keymap_overlay_rows_keep_their_order() {
        let app = App::new_for_test().expect("test app");
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
        assert_eq!(rows, EXPECTED);
    }

    /// The captured overlay rows.
    const EXPECTED: [&str; 91] = [
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
        "global.freeze = f",
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
        "global.process_tree = p",
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
