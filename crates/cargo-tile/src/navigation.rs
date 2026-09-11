//! The app-wide navigation scope.
//!
//! The framework asks every app that registers pane shortcuts for one
//! of these, and it is what makes the movement keys rebindable: the
//! action set and the default keys are framework-owned
//! ([`NavAction`]), and this module supplies only the routing.
//!
//! cargo-tile has exactly one list of its own to move through -- the
//! settings overlay. The tile grid is not a list: it is a grid of
//! cells the app globals walk, and the keymap and global-shortcuts
//! overlays are run entirely by the framework, which reads their keys
//! before this scope is ever consulted. So the routing here is short,
//! and it answers for the surface that is open rather than for the
//! focused pane: an overlay draws over the grid without taking focus
//! from it, so `focused` would name the grid either way.

use tui_pane::FocusedPane;
use tui_pane::FrameworkOverlayId;
use tui_pane::NavAction;
use tui_pane::Navigation;

use crate::app::App;
use crate::app::AppPaneId;
use crate::constants::NAVIGATION_SECTION;
use crate::settings;
use crate::settings::Step;

/// `Navigation<App>` host. Zero-sized; the trait is all statics.
pub(crate) struct AppNavigation;

impl Navigation<App> for AppNavigation {
    const SECTION_NAME: &'static str = NAVIGATION_SECTION;

    fn dispatcher() -> fn(NavAction, FocusedPane<AppPaneId>, &mut App) {
        |action, _focused, app| {
            if app.framework.overlay() == Some(FrameworkOverlayId::Settings) {
                settings_overlay(action, app);
            }
        }
    }
}

/// Move through settings rows and their wrapped lines, or step a value.
///
/// Left and right are the value, not the selection -- a settings row
/// has no column to move into, so the sideways keys are free to mean
/// "the previous choice" and "the next one".
fn settings_overlay(action: NavAction, app: &mut App) {
    match action {
        NavAction::Left => settings::cycle(app, Step::Prev),
        NavAction::Right => settings::cycle(app, Step::Next),
        _ => app.framework.settings_pane.navigate(action),
    }
}
