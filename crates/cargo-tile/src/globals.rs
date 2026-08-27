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
//! or not anything is running, and `p` says how much of each command a
//! cell spells out. Three more belong to the attract screen's saved
//! favorites: `ctrl-s` saves the parameters on screen now, `ctrl-o`
//! opens the saved list, and `m` shows one of them at random.
//!
//! To add another, give the enum a variant, bind a default key in
//! [`Globals::defaults`], and handle it in [`dispatch`].

use std::rc::Rc;
use std::time::Duration;
use std::time::Instant;

use crossterm::event::KeyCode;
use tui_pane::Bindings;
use tui_pane::Globals;
use tui_pane::KeyBind;

use crate::app::App;
use crate::attract::AttractMode;
use crate::constants::APP_GLOBALS_SECTION;
use crate::favorites;
use crate::favorites::FavoriteRows;
use crate::favorites::FavoriteSettings;
use crate::favorites::FavoritesFileState;
use crate::favorites::FavoritesMutation;
use crate::favorites::FavoritesRetryInstruction;
use crate::favorites::ResolvedBinding;
use crate::favorites_overlay::report_closed_overlay_adjustment;
use crate::random;
use crate::random::EmptyIndexDomain;
use crate::random::NonZeroIndexBound;
use crate::tiles::Direction;

const FAVORITE_TOAST_MIN_INTERIOR_LINES: usize = 1;
const FAVORITE_TOAST_VISIBLE: Duration = Duration::from_secs(5);

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
        AppGlobalAction::AddTile => app.tiles.add(initial_rows),
        AppGlobalAction::RemoveTile => app.tiles.remove(),
        AppGlobalAction::FocusLeft => app.tiles.focus_step(Direction::Left, initial_rows),
        AppGlobalAction::FocusRight => app.tiles.focus_step(Direction::Right, initial_rows),
        AppGlobalAction::FocusUp => app.tiles.focus_step(Direction::Up, initial_rows),
        AppGlobalAction::FocusDown => app.tiles.focus_step(Direction::Down, initial_rows),
        AppGlobalAction::Freeze => app.updates = app.updates.toggled(),
        AppGlobalAction::Attract => app.attract.toggle(),
        AppGlobalAction::ProcessTree => app.tree = app.tree.toggled(),
        AppGlobalAction::SaveFavorite => save_favorite(app),
        AppGlobalAction::OpenFavorites => {
            let keymap = Rc::clone(&app.keymap);
            app.favorites_overlay.open(&keymap);
        },
        AppGlobalAction::RandomFavorite => show_random_favorite(app),
    }
}

/// Load the current favorites file and show one recognized row at random.
fn show_random_favorite(app: &mut App) {
    show_random_favorite_with(app, favorites::load, random::clock_seed);
}

fn show_random_favorite_with(
    app: &mut App,
    load: impl FnOnce() -> FavoritesFileState,
    seed: impl FnOnce() -> u64,
) {
    let state = load();
    if let FavoritesFileState::Loaded { rows, .. } = &state
        && let Ok(settings) = draw_recognized_settings(rows, seed())
    {
        let outcome = app.attract.apply_favorite(settings);
        app.attract.request_show();
        report_closed_overlay_adjustment(app, outcome);
        return;
    }
    let keymap = Rc::clone(&app.keymap);
    app.favorites_overlay.open_file_state(state, &keymap);
}

fn draw_recognized_settings(
    rows: &FavoriteRows,
    seed: u64,
) -> Result<FavoriteSettings, EmptyIndexDomain> {
    let bound = NonZeroIndexBound::try_from_len(rows.recognized().count())?;
    let index = random::bounded_index(seed, bound);
    rows.recognized()
        .nth(index)
        .map(|favorite| favorite.settings)
        .ok_or(EmptyIndexDomain)
}

/// Persist the selected attract parameters and show the result.
fn save_favorite(app: &mut App) {
    let result = favorites::push(app.attract.favorite_settings());
    let (title, body) = match result {
        Ok(favorite) => (
            "Favorite saved",
            format!("{} parameters saved", mode_label(favorite.settings.mode())),
        ),
        Err(error) => {
            let binding = app
                .keymap
                .globals::<AppGlobalAction>()
                .and_then(|scope| scope.key_for(AppGlobalAction::SaveFavorite))
                .cloned();
            let retry = FavoritesRetryInstruction::Press(ResolvedBinding::for_action(
                "save_favorite",
                binding,
            ));
            (
                "Favorite not saved",
                favorites::favorite_refusal_message(FavoritesMutation::Save, &retry, &error),
            )
        },
    };
    let pushed_at = Instant::now();
    let toast_id = app.framework.toasts.push_timed(
        title,
        body.as_str(),
        FAVORITE_TOAST_VISIBLE,
        FAVORITE_TOAST_MIN_INTERIOR_LINES,
    );
    app.schedule_timed_toast(
        toast_id,
        pushed_at,
        FAVORITE_TOAST_VISIBLE,
        &body,
        FAVORITE_TOAST_MIN_INTERIOR_LINES,
    );
}

const fn mode_label(attract_mode: AttractMode) -> &'static str {
    match attract_mode {
        AttractMode::MovingBand => "Moving band",
        AttractMode::MovingText => "Moving text",
        AttractMode::Pixelate => "Pixelate",
    }
}

#[cfg(test)]
#[expect(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::collections::HashSet;
    use std::fs;
    use std::io;
    use std::path::Path;
    use std::path::PathBuf;
    use std::time::Duration;

    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::layout::Rect;
    use tempfile::TempDir;
    use tui_pane::KeyBind;
    use tui_pane::KeySequence;

    use super::*;
    use crate::app::ProcessTree;
    use crate::app::Updates;
    use crate::favorites::FavoritesMutationError;
    use crate::favorites::parse_rows_for_overlay_test;
    use crate::terminal::VisualDeadline;
    use crate::terminal::VisualFrameRequest;

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

    const UNRECOGNIZED_ROW: &str = r#"
[[favorite]]
id = "01a03f62-9c14-7b41-8a02-1de4c7c9b334"
saved = "2026-08-26T14:31:05-07:00"
mode = "future_mode"
"#;

    fn loaded_state(path: impl Into<PathBuf>, text: &str) -> FavoritesFileState {
        FavoritesFileState::Loaded {
            path: path.into(),
            rows: parse_rows_for_overlay_test(text).expect("favorites fixture should parse"),
        }
    }

    fn load_test_path(path: &Path) -> FavoritesFileState {
        match fs::read_to_string(path) {
            Ok(text) => loaded_state(path, &text),
            Err(error) if error.kind() == io::ErrorKind::NotFound => FavoritesFileState::Missing {
                path: path.to_path_buf(),
            },
            Err(error) => FavoritesFileState::Unreadable {
                path:  path.to_path_buf(),
                error: error.to_string(),
            },
        }
    }

    fn rendered_overlay(app: &mut App) -> String {
        let mut terminal =
            Terminal::new(TestBackend::new(100, 20)).expect("test terminal should build");
        terminal
            .draw(|frame| app.favorites_overlay.render(frame))
            .expect("favorites overlay should render");
        let buffer = terminal.backend().buffer();
        (buffer.area.y..buffer.area.bottom())
            .map(|y| {
                (buffer.area.x..buffer.area.right()).fold(String::new(), |mut line, x| {
                    line.push_str(buffer[(x, y)].symbol());
                    line
                })
            })
            .collect::<Vec<_>>()
            .join("\n")
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

    #[test]
    fn every_non_loadable_state_opens_the_existing_overlay_position() {
        let path = PathBuf::from("/tmp/favorites.toml");
        let cases = [
            (
                FavoritesFileState::Missing { path: path.clone() },
                "No favorites saved",
            ),
            (loaded_state(&path, ""), "No favorites saved"),
            (
                loaded_state(&path, UNRECOGNIZED_ROW),
                "mode = \"future_mode\" is not recognized",
            ),
            (
                FavoritesFileState::LocationUnavailable,
                "location unavailable",
            ),
            (
                FavoritesFileState::Unparseable {
                    path:  path.clone(),
                    error: "bad TOML".to_string(),
                },
                "bad TOML",
            ),
            (
                FavoritesFileState::Unreadable {
                    path,
                    error: "permission denied".to_string(),
                },
                "permission denied",
            ),
        ];

        for (state, expected) in cases {
            let mut app = App::new_for_test().expect("test app should build");
            show_random_favorite_with(&mut app, || state, || 0);
            let rendered = rendered_overlay(&mut app);

            assert!(app.favorites_overlay.is_open());
            assert!(
                rendered.contains(expected),
                "{rendered:?} should contain {expected:?}"
            );
            if expected.contains("future_mode") {
                assert!(!rendered.contains("No favorites saved"));
            }
        }
    }

    #[test]
    fn a_later_press_observes_a_favorite_saved_after_the_first_load() {
        let directory = TempDir::new().expect("temporary directory should be created");
        let path = directory.path().join("favorites.toml");
        let mut app = App::new_for_test().expect("test app should build");

        show_random_favorite_with(&mut app, || load_test_path(&path), || 0);
        assert!(app.favorites_overlay.is_open());

        app.favorites_overlay = crate::favorites_overlay::FavoritesOverlay::default();
        fs::write(&path, MOVING_BAND_ROW).expect("another process should save a favorite");
        show_random_favorite_with(&mut app, || load_test_path(&path), || 0);

        assert!(!app.favorites_overlay.is_open());
        assert!(app.attract.asked_for());
        assert_eq!(
            app.attract.favorite_settings(),
            draw_recognized_settings(
                &parse_rows_for_overlay_test(MOVING_BAND_ROW)
                    .expect("favorites fixture should parse"),
                0,
            )
            .expect("fixture has a recognized row")
        );
    }

    #[test]
    fn applying_a_favorite_is_silent_when_exact_and_never_rewrites_the_file() {
        let directory = TempDir::new().expect("temporary directory should be created");
        let path = directory.path().join("favorites.toml");
        fs::write(&path, MOVING_BAND_ROW).expect("favorites fixture should be written");
        let before = fs::read(&path).expect("favorites fixture should be readable");
        let mut app = App::new_for_test().expect("test app should build");
        app.attract.record_terminal_resize(Rect::new(0, 0, 80, 24));

        show_random_favorite_with(&mut app, || load_test_path(&path), || 0);

        assert!(app.framework.toasts.active_now().is_empty());
        assert_eq!(
            app.toast_visual_deadline(Instant::now(), Duration::from_millis(8)),
            VisualDeadline::NoVisualChangeScheduled
        );
        assert_eq!(
            fs::read(&path).expect("favorites fixture should remain readable"),
            before
        );
    }

    #[test]
    fn an_adjusted_random_favorite_schedules_a_lowercase_warning_while_frozen() {
        let oversized = MOVING_BAND_ROW.replace("width = 10", "width = 10000");
        let mut app = App::new_for_test().expect("test app should build");
        app.updates = Updates::Frozen;
        app.attract.record_terminal_resize(Rect::new(0, 0, 10, 5));
        let now = Instant::now();

        show_random_favorite_with(
            &mut app,
            || loaded_state("/tmp/favorites.toml", &oversized),
            || 0,
        );

        let toasts = app.framework.toasts.active_views(Instant::now());
        assert_eq!(toasts.len(), 1);
        assert_eq!(toasts[0].title(), "Favorite adjusted");
        assert!(toasts[0].body().contains("width 10000 ->"));
        assert!(!toasts[0].body().contains("MovingBand"));
        assert!(matches!(
            app.toast_visual_deadline(now, Duration::from_millis(8)),
            VisualDeadline::At(_)
        ));

        let at_expiry = now + Duration::from_secs(30);
        app.framework.toasts.prune(at_expiry);
        assert_eq!(
            app.toast_visual_frame_request(at_expiry),
            VisualFrameRequest::Requested
        );
        let after_exit = at_expiry + Duration::from_secs(30);
        app.framework.toasts.prune(after_exit);
        assert_eq!(
            app.toast_visual_frame_request(after_exit),
            VisualFrameRequest::Requested
        );
        assert!(app.framework.toasts.active_views(after_exit).is_empty());
    }

    #[test]
    fn every_favorite_refusal_names_a_distinct_cause() {
        let path = PathBuf::from("/tmp/favorites.toml");
        let errors = [
            FavoritesMutationError::LocationUnavailable,
            FavoritesMutationError::Unparseable {
                path:  path.clone(),
                error: "bad TOML".to_string(),
            },
            FavoritesMutationError::Unreadable {
                path:  path.clone(),
                error: "permission denied".to_string(),
            },
            FavoritesMutationError::LockUnavailable {
                path:  path.clone(),
                error: "favorites are in use".to_string(),
            },
            FavoritesMutationError::WriteFailed {
                path,
                error: "disk is read-only".to_string(),
            },
        ];
        let retry = FavoritesRetryInstruction::Press(ResolvedBinding::for_action(
            "save_favorite",
            Some(KeySequence::from(KeyBind::ctrl('s'))),
        ));
        let messages: Vec<String> = errors
            .iter()
            .map(|error| {
                favorites::favorite_refusal_message(FavoritesMutation::Save, &retry, error)
            })
            .collect();
        let distinct: HashSet<&str> = messages.iter().map(String::as_str).collect();

        assert_eq!(distinct.len(), errors.len());
        for (message, cause) in messages.iter().zip([
            "no OS config directory",
            "unparseable favorites",
            "cannot read favorites",
            "cannot acquire favorites lock",
            "cannot write favorites",
        ]) {
            assert!(message.contains(cause), "{message:?} should name {cause:?}");
        }
        assert!(messages[3].contains("press ⌃s to try again"));
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
