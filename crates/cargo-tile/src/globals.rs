//! The app-globals scope: this app's global shortcuts, the ones the
//! framework does not already own.
//!
//! [`tui_pane::GlobalAction`] owns quit, restart, pane cycling, and the
//! settings / keymap / shortcut overlays — those need no registration
//! here. This scope is for the shortcuts *this* app adds on top: the two
//! that open and close cells, and the four arrows that move the focus
//! ring between them. The framework picks up the
//! rest from the registration in [`crate::keymap`]: TOML loading, the
//! status-line slots, and the rows in the keymap overlay.
//!
//! Two of them are not about the grid at all: `f` holds the whole
//! display still, which is what makes a screen that repaints four times
//! a second readable, and `a` draws the attract screen over the grid
//! whether or not anything is running.
//!
//! To add another, give the enum a variant, bind a default key in
//! [`Globals::defaults`], and handle it in [`dispatch`].

use crossterm::event::KeyCode;
use tui_pane::Bindings;
use tui_pane::Globals;

use crate::app::App;
use crate::constants::APP_GLOBALS_SECTION;
use crate::tiles::Direction;

tui_pane::action_enum! {
    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    pub(crate) enum AppGlobalAction {
        AddTile    => ("add_tile",    "Add a tile");
        RemoveTile => ("remove_tile", "Remove an empty tile");
        FocusLeft  => ("focus_left",  "Focus the tile to the left");
        FocusRight => ("focus_right", "Focus the tile to the right");
        FocusUp    => ("focus_up",    "Focus the tile above");
        FocusDown  => ("focus_down",  "Focus the tile below");
        Freeze     => ("freeze",      "Freeze the display");
        Attract    => ("attract",     "Show the attract screen");
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
            KeyCode::Left => Self::FocusLeft,
            KeyCode::Right => Self::FocusRight,
            KeyCode::Up => Self::FocusUp,
            KeyCode::Down => Self::FocusDown,
            'f' => Self::Freeze,
            'a' => Self::Attract,
        }
    }

    fn dispatcher() -> fn(Self::Actions, &mut App) { dispatch }
}

/// Run one app-global action.
fn dispatch(action: AppGlobalAction, app: &mut App) {
    let initial_rows = app.loaded_config.config.tiles.initial_rows();
    match action {
        AppGlobalAction::AddTile => app.tiles.add(initial_rows),
        AppGlobalAction::RemoveTile => app.tiles.remove(),
        AppGlobalAction::FocusLeft => app.tiles.focus_step(Direction::Left, initial_rows),
        AppGlobalAction::FocusRight => app.tiles.focus_step(Direction::Right, initial_rows),
        AppGlobalAction::FocusUp => app.tiles.focus_step(Direction::Up, initial_rows),
        AppGlobalAction::FocusDown => app.tiles.focus_step(Direction::Down, initial_rows),
        AppGlobalAction::Freeze => app.updates = app.updates.toggled(),
        AppGlobalAction::Attract => app.attract.toggle(),
    }
}
