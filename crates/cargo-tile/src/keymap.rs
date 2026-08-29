//! Keymap assembly — the `cargo-tile` counterpart of cargo-port's
//! `build_framework_keymap`.
//!
//! The framework globals — `s` settings, ctrl-k keymap, `?` shortcuts,
//! `q` quit, `R` restart, `x` dismiss, Tab pane cycling — come from
//! [`tui_pane::GlobalAction`]'s defaults and need no registration.
//! `register_navigation` folds in the movement scope (see
//! [`crate::navigation`]), `register_globals` this app's own globals
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

use crate::app::App;
use crate::app::AppPaneId;
use crate::attract::MovingBandPane;
use crate::attract::MovingTextPane;
use crate::attract::PixelatePane;
use crate::favorites_overlay::FavoritesOverlayPane;
use crate::globals::AppGlobalAction;
use crate::navigation::AppNavigation;

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
        _ => FrameworkGlobalShortcutVisibility::Shown,
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
        .register_navigation::<AppNavigation>()?
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
