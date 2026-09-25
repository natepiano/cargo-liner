//! The app-globals scope: this app's global shortcuts, the ones the
//! framework does not already own.
//!
//! [`tui_pane::GlobalAction`] owns quit, restart, pane cycling, and the
//! settings / keymap / shortcut overlays — those need no registration
//! here. This scope is for the shortcuts *this* app adds on top: the two
//! that open and close cells, and the four arrows that move the focus
//! ring between them. The framework picks up the rest from the
//! registration in [`crate::keymap`]: TOML loading, the status-line
//! slots, and the rows in the keymap overlay.
//!
//! The rest belong to the attract screen: `a` draws it over the grid,
//! `r` draws a fresh one at random, and `u` restores the configuration
//! that replacement displaced. Three more belong to its saved
//! favorites: `ctrl-s` saves the parameters on screen now, `ctrl-o`
//! opens the saved list, and `m` shows one of them at random.
//!
//! To add another, give the enum a variant, bind a default key in
//! [`Globals::defaults`], and handle it in [`dispatch`].

use crossterm::event::KeyCode;
use tui_pane::Bindings;
use tui_pane::Globals;
use tui_pane::KeyBind;
use tui_pane::TileAction;

use crate::app::App;
use crate::constants::APP_GLOBALS_SECTION;

tui_pane::action_enum! {
    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    pub(crate) enum AppGlobalAction {
        AddTile    => ("add_tile",    "Add a tile");
        RemoveTile => ("remove_tile", "Remove an empty tile");
        FocusLeft  => ("focus_left",  "Focus the tile to the left");
        FocusRight => ("focus_right", "Focus the tile to the right");
        FocusUp    => ("focus_up",    "Focus the tile above");
        FocusDown  => ("focus_down",  "Focus the tile below");
        Attract    => ("attract",     "Show the attract screen");
        RandomizeAttract => ("randomize_attract", "Randomize the attract screen");
        UndoAttractReplacement => ("undo_attract_replacement", "Undo attract replacement");
        SaveFavorite => ("save_favorite", "Save attract parameters");
        OpenFavorites => ("open_favorites", "Open attract favorites");
        RandomFavorite => ("random_favorite", "Show a random favorite");
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
            'a' => Self::Attract,
            'r' => Self::RandomizeAttract,
            'u' => Self::UndoAttractReplacement,
            KeyBind::ctrl('s') => Self::SaveFavorite,
            KeyBind::ctrl('o') => Self::OpenFavorites,
            'm' => Self::RandomFavorite,
        }
    }

    fn dispatcher() -> fn(Self::Actions, &mut App) { dispatch }
}

/// Run one app-global action.
fn dispatch(action: AppGlobalAction, app: &mut App) {
    let initial_rows = app.loaded_config.config.tiles.initial_rows();
    match action {
        AppGlobalAction::AddTile => app.tiles.apply(TileAction::Add, initial_rows),
        AppGlobalAction::RemoveTile => app.tiles.apply(TileAction::Remove, initial_rows),
        AppGlobalAction::FocusLeft => app.tiles.apply(TileAction::FocusLeft, initial_rows),
        AppGlobalAction::FocusRight => app.tiles.apply(TileAction::FocusRight, initial_rows),
        AppGlobalAction::FocusUp => app.tiles.apply(TileAction::FocusUp, initial_rows),
        AppGlobalAction::FocusDown => app.tiles.apply(TileAction::FocusDown, initial_rows),
        AppGlobalAction::Attract => app.attract.toggle(),
        AppGlobalAction::RandomizeAttract => app.attract.randomize(),
        AppGlobalAction::UndoAttractReplacement => tui_pane::undo_attract_replacement(app),
        AppGlobalAction::SaveFavorite => tui_pane::save_favorite(app),
        AppGlobalAction::OpenFavorites => tui_pane::open_favorites(app),
        AppGlobalAction::RandomFavorite => tui_pane::show_random_favorite(app),
    }
}

#[cfg(test)]
#[expect(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::time::Instant;

    use tui_pane::KeyBind;

    use super::*;

    /// Every default key reaches its action, read out of the table the
    /// keymap is built from. The arrows and `+` `-` share the scope with
    /// the attract and favorites keys, so a key added over one of them
    /// would take a tile away instead.
    #[test]
    fn each_default_key_maps_to_its_action() {
        let scope = AppGlobalAction::defaults().into_scope_map();
        let expected = [
            (KeyBind::from('+'), AppGlobalAction::AddTile),
            (KeyBind::from('-'), AppGlobalAction::RemoveTile),
            (KeyBind::from(KeyCode::Left), AppGlobalAction::FocusLeft),
            (KeyBind::from(KeyCode::Right), AppGlobalAction::FocusRight),
            (KeyBind::from(KeyCode::Up), AppGlobalAction::FocusUp),
            (KeyBind::from(KeyCode::Down), AppGlobalAction::FocusDown),
            (KeyBind::from('a'), AppGlobalAction::Attract),
            (KeyBind::from('r'), AppGlobalAction::RandomizeAttract),
            (KeyBind::from('u'), AppGlobalAction::UndoAttractReplacement),
            (KeyBind::ctrl('s'), AppGlobalAction::SaveFavorite),
            (KeyBind::ctrl('o'), AppGlobalAction::OpenFavorites),
            (KeyBind::from('m'), AppGlobalAction::RandomFavorite),
        ];

        for (bind, action) in expected {
            assert_eq!(scope.action_for(&bind), Some(action), "{bind:?}");
        }
        assert_eq!(
            expected.len(),
            <AppGlobalAction as tui_pane::Action>::ALL.len()
        );
    }

    /// `r` asks for the attract screen as well as drawing a new one, so
    /// the draw is seen.
    #[test]
    fn randomizing_requests_the_attract_screen() {
        let mut app = App::new_for_test().expect("test app should build");
        assert!(!app.attract.asked_for());

        dispatch(AppGlobalAction::RandomizeAttract, &mut app);

        assert!(app.attract.asked_for());
    }

    /// With no replacement to undo, `u` says so rather than doing
    /// nothing silently.
    #[test]
    fn undo_with_nothing_to_undo_says_so() {
        let mut app = App::new_for_test().expect("test app should build");

        dispatch(AppGlobalAction::UndoAttractReplacement, &mut app);

        let toasts = app.framework.toasts.active_views(Instant::now());
        assert_eq!(toasts.len(), 1);
        assert_eq!(toasts[0].title(), "Nothing to undo");
    }
}
