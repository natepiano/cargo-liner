//! The favorites globals an app's own globals scope dispatches to:
//! save the attract parameters on screen, open the saved list, and show
//! one saved favorite at random.

use super::FavoritesHost;
use super::bindings;
use super::report_closed_overlay_adjustment;
use crate::AttractSettings;
use crate::FavoriteRows;
use crate::FavoriteSaveOutcome;
use crate::FavoritesFileState;
use crate::FavoritesMutation;
use crate::FavoritesRetryInstruction;
use crate::attract;
use crate::attract::EmptyIndexDomain;
use crate::attract::NOTICE_TOAST_MIN_INTERIOR_LINES;
use crate::attract::NOTICE_TOAST_VISIBLE;
use crate::attract::NonZeroIndexBound;

/// Persist the attract parameters on screen now and say how it went in
/// a toast.
///
/// A refusal names the key that retries it, found by the TOML name
/// `save_favorite` (see [`FavoritesHost`]).
pub fn save_favorite<A: FavoritesHost>(app: &mut A) {
    let settings = app.attract_mut().current_settings();
    let attract_mode = settings.mode();
    let result = crate::push_favorite::<A::Identity>(settings);
    let (title, body) = match result {
        Ok(FavoriteSaveOutcome::Added) => (
            "Favorite added",
            format!(
                "{} parameters added to favorites",
                attract::mode_label(attract_mode)
            ),
        ),
        Ok(FavoriteSaveOutcome::Refreshed) => (
            "Favorite refreshed",
            format!(
                "{} parameters were already saved, so the existing favorite's timestamp was refreshed rather than a second row being added",
                attract::mode_label(attract_mode)
            ),
        ),
        Err(error) => {
            let keymap = app.keymap();
            let retry = FavoritesRetryInstruction::Press(bindings::resolve_global_binding(
                &keymap,
                "save_favorite",
            ));
            (
                "Favorite not saved",
                crate::favorite_refusal_message(FavoritesMutation::Save, &retry, &error),
            )
        },
    };
    app.framework_mut().toasts.push_timed(
        title,
        body.as_str(),
        NOTICE_TOAST_VISIBLE,
        NOTICE_TOAST_MIN_INTERIOR_LINES,
    );
}

/// Load the favorites file and open the overlay on what it holds.
pub fn open_favorites<A: FavoritesHost>(app: &mut A) {
    let current_parameters = app.attract_mut().current_settings().into();
    let keymap = app.keymap();
    app.favorites_overlay_mut()
        .open(&keymap, current_parameters);
}

/// Load the favorites file and show one recognized row at random.
///
/// A file with no recognized row to draw opens the overlay on it
/// instead, so the reason is on screen.
pub fn show_random_favorite<A: FavoritesHost>(app: &mut A) {
    show_random_favorite_with(
        app,
        crate::load_favorites::<A::Identity>,
        attract::clock_seed,
    );
}

/// Open the overlay on `state` as a file load would, with the app's
/// keymap and the attract parameters on screen now.
#[doc(hidden)]
pub fn open_favorites_on_state_for_test<A: FavoritesHost>(app: &mut A, state: FavoritesFileState) {
    open_on_state(app, state);
}

fn show_random_favorite_with<A: FavoritesHost>(
    app: &mut A,
    load: impl FnOnce() -> FavoritesFileState,
    seed: impl FnOnce() -> u64,
) {
    let state = load();
    if let FavoritesFileState::Loaded { rows, .. } = &state
        && let Ok(settings) = draw_recognized_settings(rows, seed())
    {
        let outcome = app.attract_mut().apply_settings(settings);
        app.attract_mut().request_show();
        report_closed_overlay_adjustment(app, outcome);
        return;
    }
    open_on_state(app, state);
}

fn open_on_state<A: FavoritesHost>(app: &mut A, state: FavoritesFileState) {
    let current_parameters = app.attract_mut().current_settings().into();
    let keymap = app.keymap();
    app.favorites_overlay_mut()
        .open_file_state(state, current_parameters, &keymap);
}

fn draw_recognized_settings(
    rows: &FavoriteRows,
    seed: u64,
) -> Result<AttractSettings, EmptyIndexDomain> {
    let bound = NonZeroIndexBound::try_from_len(rows.recognized().count())?;
    let index = attract::bounded_index(seed, bound);
    rows.recognized()
        .nth(index)
        .map(|favorite| favorite.settings)
        .ok_or(EmptyIndexDomain)
}

#[cfg(test)]
#[expect(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::collections::HashSet;
    use std::fs;
    use std::io::ErrorKind;
    use std::path::Path;
    use std::path::PathBuf;
    use std::time::Duration;
    use std::time::Instant;

    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::layout::Rect;
    use tempfile::TempDir;

    use super::*;
    use crate::Action;
    use crate::AttractGridPresentation;
    use crate::AttractVisibilityInstruction;
    use crate::FavoritesMutationError;
    use crate::KeyBind;
    use crate::KeySequence;
    use crate::ResolvedBinding;
    use crate::ToastVisualDeadline;
    use crate::Updates;
    use crate::favorites_overlay::test_app;
    use crate::favorites_overlay::test_app::TestApp;
    use crate::favorites_overlay::test_app::TestGlobalAction;

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
            rows: crate::parse_favorite_rows_for_test(text)
                .expect("favorites fixture should parse"),
        }
    }

    fn load_test_path(path: &Path) -> FavoritesFileState {
        match fs::read_to_string(path) {
            Ok(text) => loaded_state(path, &text),
            Err(error) if error.kind() == ErrorKind::NotFound => FavoritesFileState::Missing {
                path: path.to_path_buf(),
            },
            Err(error) => FavoritesFileState::Unreadable {
                path:  path.to_path_buf(),
                error: error.to_string(),
            },
        }
    }

    fn rendered_overlay(app: &mut TestApp) -> String {
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

    fn moving_band_settings() -> AttractSettings {
        draw_recognized_settings(
            &crate::parse_favorite_rows_for_test(MOVING_BAND_ROW)
                .expect("favorites fixture should parse"),
            0,
        )
        .expect("fixture should contain a recognized favorite")
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
            let mut app = TestApp::new_for_test().expect("test app should build");
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
    fn failed_random_favorite_loads_preserve_the_existing_undo_point() {
        let path = PathBuf::from("/tmp/favorites.toml");
        let cases = [
            FavoritesFileState::Missing { path: path.clone() },
            loaded_state(&path, UNRECOGNIZED_ROW),
            FavoritesFileState::Unparseable {
                path,
                error: "bad TOML".to_string(),
            },
        ];

        for state in cases {
            let mut app = TestApp::new_for_test().expect("test app should build");
            app.attract.record_terminal_resize(Rect::new(0, 0, 80, 24));
            let before = app.attract.current_settings();
            app.attract.apply_settings(moving_band_settings());

            show_random_favorite_with(&mut app, || state, || 0);
            app.favorites_overlay = crate::favorites_overlay::FavoritesOverlay::default();
            crate::undo_attract_replacement(&mut app);

            assert_eq!(app.attract.current_settings(), before);
        }
    }

    #[test]
    fn a_later_press_observes_a_favorite_saved_after_the_first_load() {
        let directory = TempDir::new().expect("temporary directory should be created");
        let path = directory.path().join("favorites.toml");
        let mut app = TestApp::new_for_test().expect("test app should build");

        show_random_favorite_with(&mut app, || load_test_path(&path), || 0);
        assert!(app.favorites_overlay.is_open());

        app.favorites_overlay = crate::favorites_overlay::FavoritesOverlay::default();
        fs::write(&path, MOVING_BAND_ROW).expect("another process should save a favorite");
        show_random_favorite_with(&mut app, || load_test_path(&path), || 0);

        assert!(!app.favorites_overlay.is_open());
        assert!(app.attract.asked_for());
        assert_eq!(
            app.attract.current_settings(),
            draw_recognized_settings(
                &crate::parse_favorite_rows_for_test(MOVING_BAND_ROW)
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
        let mut app = TestApp::new_for_test().expect("test app should build");
        app.attract.record_terminal_resize(Rect::new(0, 0, 80, 24));
        let initial_settings = app.attract.current_settings();
        app.attract.apply_settings(initial_settings);
        app.attract.request_show();
        app.attract.toggle();

        let before_configuration = app.attract.configuration();
        assert_eq!(
            before_configuration.visibility_instruction(),
            AttractVisibilityInstruction::Hide
        );
        assert_eq!(
            before_configuration.grid_presentation(),
            AttractGridPresentation::ReplacesGrid
        );
        show_random_favorite_with(&mut app, || load_test_path(&path), || 0);

        assert!(app.framework.toasts.active_now().is_empty());
        assert_eq!(
            app.framework
                .toasts
                .next_visual_change_deadline(Instant::now()),
            ToastVisualDeadline::NoVisualChangeScheduled
        );
        assert_eq!(
            fs::read(&path).expect("favorites fixture should remain readable"),
            before
        );

        crate::undo_attract_replacement(&mut app);
        assert_eq!(app.attract.configuration(), before_configuration);
    }

    #[test]
    fn an_adjusted_random_favorite_schedules_a_lowercase_warning_while_frozen() {
        let oversized = MOVING_BAND_ROW.replace("width = 10", "width = 10000");
        let directory = TempDir::new().expect("temporary directory should be created");
        let path = directory.path().join("favorites.toml");
        fs::write(&path, &oversized).expect("favorites fixture should be written");
        let file_before = fs::read(&path).expect("favorites fixture should be readable");
        let mut app = TestApp::new_for_test().expect("test app should build");
        app.updates = Updates::Frozen;
        app.attract.record_terminal_resize(Rect::new(0, 0, 10, 5));
        let before_settings = app.attract.current_settings();
        let now = Instant::now();

        show_random_favorite_with(&mut app, || load_test_path(&path), || 0);

        let toasts = app.framework.toasts.active_views(Instant::now());
        assert_eq!(toasts.len(), 1);
        assert_eq!(toasts[0].title(), "Favorite adjusted");
        assert!(toasts[0].body().contains("width 10000 ->"));
        assert!(!toasts[0].body().contains("MovingBand"));
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
        assert_eq!(
            fs::read(&path).expect("favorites fixture should remain readable"),
            file_before
        );

        crate::undo_attract_replacement(&mut app);
        assert_eq!(app.attract.current_settings(), before_settings);
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
            .map(|error| crate::favorite_refusal_message(FavoritesMutation::Save, &retry, error))
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

    /// A save refusal can't be produced without writing the real favorites file, so this pins
    /// the retry text `save_favorite` builds: the binding read by its type agrees with the
    /// binding read by its TOML name, which is how `save_favorite` and the overlay's prompts
    /// both read it, and a rebound key reaches the toast.
    #[test]
    fn save_refusal_toast_names_the_bound_key() {
        let keymap = test_app::keymap_from("[global]\nsave_favorite = \"y\"\n");
        let typed = keymap
            .globals::<TestGlobalAction>()
            .and_then(|scope| scope.key_for(TestGlobalAction::SaveFavorite))
            .cloned();
        let by_name = TestGlobalAction::from_toml_key("save_favorite").and_then(|action| {
            keymap
                .globals::<TestGlobalAction>()
                .and_then(|scope| scope.key_for(action))
                .cloned()
        });
        assert_eq!(typed, by_name);

        let retry =
            FavoritesRetryInstruction::Press(ResolvedBinding::for_action("save_favorite", typed));
        let message = crate::favorite_refusal_message(
            FavoritesMutation::Save,
            &retry,
            &FavoritesMutationError::LockUnavailable {
                path:  PathBuf::from("/tmp/favorites.lock"),
                error: "held".to_string(),
            },
        );

        assert!(
            message.contains("press y to try again"),
            "{message:?} should name the rebound key"
        );
    }
}
