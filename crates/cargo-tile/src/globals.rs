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
//! Some are not about the grid at all: `f` holds the whole display
//! still, which is what makes a screen that repaints four times a
//! second readable, `a` draws the attract screen over the grid whether
//! or not anything is running, `r` draws a fresh attract screen at
//! random, `u` restores the attract configuration that replacement
//! displaced, and `p` says how much of each command a cell spells out.
//! Three more belong to the attract screen's saved favorites: `ctrl-s`
//! saves the parameters on screen now, `ctrl-o` opens the saved list,
//! and `m` shows one of them at random.
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
        Freeze     => ("freeze",      "Freeze the display");
        Attract    => ("attract",     "Show the attract screen");
        RandomizeAttract => ("randomize_attract", "Randomize the attract screen");
        UndoAttractReplacement => ("undo_attract_replacement", "Undo attract replacement");
        ProcessTree => ("process_tree", "Show whole command lines");
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
            'f' => Self::Freeze,
            'a' => Self::Attract,
            'r' => Self::RandomizeAttract,
            'u' => Self::UndoAttractReplacement,
            'p' => Self::ProcessTree,
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
        AppGlobalAction::Freeze => app.updates = app.updates.toggled(),
        AppGlobalAction::Attract => app.attract.toggle(),
        AppGlobalAction::RandomizeAttract => app.attract.randomize(),
        AppGlobalAction::UndoAttractReplacement => tui_pane::undo_attract_replacement(app),
        AppGlobalAction::ProcessTree => app.tree = app.tree.toggled(),
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
    use std::time::Duration;
    use std::time::Instant;

    use ratatui::layout::Rect;
    use tui_pane::AttractSettings;
    use tui_pane::KeyBind;
    use tui_pane::ToastVisualDeadline;
    use tui_pane::Updates;

    use super::*;
    use crate::app::ProcessTree;

    const MOVING_BAND_ROW: &str = r#"
[[favorite]]
id = "01a03f60-9c14-7b41-8a02-1de4c7c9b332"
saved = "2026-08-26T11:02:44-07:00"
mode = "moving_band"
direction = "left"
width = 10
speed = 32
tail_speed = 72
fraying = "leading"
"#;

    fn moving_band_settings() -> AttractSettings {
        tui_pane::parse_favorite_rows_for_test(MOVING_BAND_ROW)
            .expect("favorites fixture should parse")
            .recognized()
            .next()
            .expect("fixture should contain a recognized favorite")
            .settings
    }

    fn assert_only_toast_animates_and_expires(app: &mut App, expected_title: &str) {
        let now = Instant::now();
        let toasts = app.framework.toasts.active_views(now);
        assert_eq!(toasts.len(), 1);
        assert_eq!(toasts[0].title(), expected_title);
        assert!(matches!(
            app.framework.toasts.next_visual_change_deadline(now),
            ToastVisualDeadline::At(_)
        ));

        let at_expiry = now + Duration::from_secs(30);
        app.framework.toasts.prune(at_expiry);
        assert!(matches!(
            app.framework.toasts.next_visual_change_deadline(at_expiry),
            ToastVisualDeadline::At(_)
        ));
        let after_exit = at_expiry + Duration::from_secs(30);
        app.framework.toasts.prune(after_exit);
        assert_eq!(
            app.framework.toasts.next_visual_change_deadline(after_exit),
            ToastVisualDeadline::NoVisualChangeScheduled
        );
        assert!(app.framework.toasts.active_views(after_exit).is_empty());
    }

    /// `p` reaches the process-tree toggle, read out of the table the
    /// keymap is actually built from. It shares the scope with the four
    /// arrows and with `+` and `-`, so a key added over one of those
    /// would take a tile away instead.
    #[test]
    fn p_toggles_the_process_tree() {
        let scope = AppGlobalAction::defaults().into_scope_map();

        assert_eq!(
            scope.action_for(&KeyBind::from('p')),
            Some(AppGlobalAction::ProcessTree),
        );
    }

    #[test]
    fn r_randomizes_the_attract_screen() {
        let scope = AppGlobalAction::defaults().into_scope_map();

        assert_eq!(
            scope.action_for(&KeyBind::from('r')),
            Some(AppGlobalAction::RandomizeAttract),
        );
    }

    #[test]
    fn u_undoes_the_latest_attract_replacement() {
        let scope = AppGlobalAction::defaults().into_scope_map();

        assert_eq!(
            scope.action_for(&KeyBind::from('u')),
            Some(AppGlobalAction::UndoAttractReplacement),
        );
    }

    #[test]
    fn exact_hidden_restore_confirms_and_expires_while_frozen() {
        let mut app = App::new_for_test().expect("test app should build");
        app.updates = Updates::Frozen;
        app.attract.record_terminal_resize(Rect::new(0, 0, 80, 24));
        let before = app.attract.current_settings();
        app.attract.apply_settings(moving_band_settings());

        dispatch(AppGlobalAction::UndoAttractReplacement, &mut app);

        assert_eq!(app.attract.current_settings(), before);
        assert!(
            !app.attract.showing(),
            "the exact restore remains fully hidden"
        );
        let toasts = app.framework.toasts.active_views(Instant::now());
        assert_eq!(toasts.len(), 1);
        assert_eq!(toasts[0].body(), "Moving text configuration restored");
        assert_only_toast_animates_and_expires(&mut app, "Attract restored");
    }

    #[test]
    fn adjusted_restore_names_the_moved_parameter_sets_and_expires_while_frozen() {
        let mut app = App::new_for_test().expect("test app should build");
        app.updates = Updates::Frozen;
        app.attract.record_terminal_resize(Rect::new(0, 0, 80, 24));
        app.attract.apply_settings(moving_band_settings());
        app.attract.record_terminal_resize(Rect::new(0, 0, 4, 3));

        dispatch(AppGlobalAction::UndoAttractReplacement, &mut app);

        let toasts = app.framework.toasts.active_views(Instant::now());
        assert_eq!(toasts.len(), 1);
        assert!(toasts[0].body().contains("moving band"));
        assert_only_toast_animates_and_expires(&mut app, "Attract restored with adjustments");
    }

    #[test]
    fn unavailable_undo_says_so_and_expires_while_frozen() {
        let mut app = App::new_for_test().expect("test app should build");
        app.updates = Updates::Frozen;

        dispatch(AppGlobalAction::UndoAttractReplacement, &mut app);

        assert_only_toast_animates_and_expires(&mut app, "Nothing to undo");
    }

    #[test]
    fn randomizing_always_requests_the_attract_screen() {
        let mut app = App::new_for_test().expect("test app should build");
        let mut configurations = Vec::new();

        assert!(!app.attract.asked_for());
        for _ in 0..64 {
            dispatch(AppGlobalAction::RandomizeAttract, &mut app);
            assert!(app.attract.asked_for());
            configurations.push(app.attract.current_settings());
        }
        configurations.dedup();
        // A configuration spans a mode and wide numeric parameter ranges. Requiring
        // 64 nanosecond-seeded draws to all match makes a chance failure negligible.
        assert!(configurations.len() > 1);

        app.attract.toggle();
        assert!(!app.attract.asked_for());
        dispatch(AppGlobalAction::RandomizeAttract, &mut app);
        assert!(app.attract.asked_for());
    }

    #[test]
    fn control_s_saves_a_favorite() {
        let scope = AppGlobalAction::defaults().into_scope_map();

        assert_eq!(
            scope.action_for(&KeyBind::ctrl('s')),
            Some(AppGlobalAction::SaveFavorite),
        );
    }

    #[test]
    fn control_o_opens_favorites() {
        let scope = AppGlobalAction::defaults().into_scope_map();

        assert_eq!(
            scope.action_for(&KeyBind::ctrl('o')),
            Some(AppGlobalAction::OpenFavorites),
        );
    }

    #[test]
    fn m_loads_a_random_favorite() {
        let scope = AppGlobalAction::defaults().into_scope_map();

        assert_eq!(
            scope.action_for(&KeyBind::from('m')),
            Some(AppGlobalAction::RandomFavorite),
        );
    }

    /// The display starts short and the key walks between the two. A
    /// grid opening on the whole chain spends half of every cell on
    /// something that has not changed since the command began.
    #[test]
    fn the_tree_starts_short_and_the_key_walks_both_ways() {
        let tree = ProcessTree::default();

        assert_eq!(tree, ProcessTree::Short);
        assert_eq!(tree.toggled(), ProcessTree::Long);
        assert_eq!(tree.toggled().toggled(), ProcessTree::Short);
    }
}
