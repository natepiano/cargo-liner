//! The app-globals scope: this app's global shortcuts, the ones the
//! framework does not already own.
//!
//! [`tui_pane::GlobalAction`] owns quit, restart, pane cycling, and the
//! settings / keymap / shortcut overlays — those need no registration
//! here. This scope is for the shortcuts *this* app adds on top, and it
//! holds the two that drive the tile grid. The framework picks up the
//! rest from the registration in [`crate::keymap`]: TOML loading, the
//! status-line slots, and the rows in the keymap overlay.
//!
//! To add another, give the enum a variant, bind a default key in
//! [`Globals::defaults`], and handle it in [`dispatch`].

use tui_pane::Bindings;
use tui_pane::Globals;

use crate::app::App;
use crate::constants::APP_GLOBALS_SECTION;

tui_pane::action_enum! {
    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    pub(crate) enum AppGlobalAction {
        AddTile    => ("add_tile",    "Add a tile");
        RemoveTile => ("remove_tile", "Remove a tile");
    }
}

impl Globals<App> for AppGlobalAction {
    type Actions = Self;

    const SECTION_NAME: &'static str = APP_GLOBALS_SECTION;

    fn render_order() -> &'static [Self::Actions] { <Self as tui_pane::Action>::ALL }

    fn defaults() -> Bindings<Self::Actions> {
        tui_pane::bindings! {
            '+' => Self::AddTile,
            '-' => Self::RemoveTile,
        }
    }

    fn dispatcher() -> fn(Self::Actions, &mut App) { dispatch }
}

/// Run one app-global action.
fn dispatch(action: AppGlobalAction, app: &mut App) {
    match action {
        AppGlobalAction::AddTile => {
            let initial_rows = app.loaded_config.config.tiles.initial_rows();
            app.tiles.add(initial_rows);
        },
        AppGlobalAction::RemoveTile => app.tiles.remove(),
    }
}
