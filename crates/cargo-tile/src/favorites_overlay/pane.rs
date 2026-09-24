//! Keymap host, action set, and dispatcher for the favorites modal.

use std::mem;

use crossterm::event::KeyCode;
use tui_pane::Bindings;
use tui_pane::Mode;
use tui_pane::Pane;
use tui_pane::Shortcuts;
use tui_pane::TabStop;

use super::FavoritesOverlayActionOutcome;
use super::close_overlay;
use super::constants::FAVORITES_SCOPE;
use super::constants::FAVORITES_SECTION;
use super::report_application_outcome;
use crate::app::App;
use crate::app::AppPaneId;

tui_pane::action_enum! {
    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    pub(crate) enum FavoritesOverlayAction {
        SelectPrevious => ("select_previous", "Select the previous favorite");
        SelectNext => ("select_next", "Select the next favorite");
        PageColumnsLeft => ("page_columns_left", "Show the previous parameter column");
        PageColumnsRight => ("page_columns_right", "Show the next parameter column");
        Load => ("load", "Load the selected favorite");
        Delete => ("delete", "Delete the selected favorite");
        Close => ("close", "Close favorites");
    }
}

/// Keymap host for the app-owned favorites modal.
pub(crate) struct FavoritesOverlayPane;

impl Pane<App> for FavoritesOverlayPane {
    const APP_PANE_ID: AppPaneId = AppPaneId::Favorites;

    fn mode() -> fn(&App) -> Mode<App> { |_app| Mode::Static }

    fn tab_stop() -> TabStop<App> { TabStop::never() }
}

impl Shortcuts<App> for FavoritesOverlayPane {
    type Actions = FavoritesOverlayAction;

    const SCOPE_NAME: &'static str = FAVORITES_SCOPE;
    const SECTION_NAME: &'static str = FAVORITES_SECTION;

    fn defaults() -> Bindings<Self::Actions> {
        tui_pane::bindings! {
            [KeyCode::Up, 'k'] => FavoritesOverlayAction::SelectPrevious,
            [KeyCode::Down, 'j'] => FavoritesOverlayAction::SelectNext,
            [KeyCode::Left, 'h'] => FavoritesOverlayAction::PageColumnsLeft,
            [KeyCode::Right, 'l'] => FavoritesOverlayAction::PageColumnsRight,
            KeyCode::Enter => FavoritesOverlayAction::Load,
            'x' => FavoritesOverlayAction::Delete,
            KeyCode::Esc => FavoritesOverlayAction::Close,
        }
    }

    fn dispatcher() -> fn(Self::Actions, &mut App) { dispatch }
}

pub(super) fn dispatch(action: FavoritesOverlayAction, app: &mut App) {
    let mut overlay = mem::take(&mut app.favorites_overlay);
    match overlay.handle_action(action) {
        FavoritesOverlayActionOutcome::Quiet => {},
        FavoritesOverlayActionOutcome::Load(settings) => {
            let application = app.attract.apply_settings(settings);
            close_overlay(&mut overlay, app);
            app.attract.request_show();
            report_application_outcome(&mut overlay, app, application);
        },
        FavoritesOverlayActionOutcome::Close => close_overlay(&mut overlay, app),
    }
    app.favorites_overlay = overlay;
}

#[cfg(test)]
#[expect(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::path::PathBuf;
    use std::rc::Rc;

    use crossterm::event::KeyEvent;
    use crossterm::event::KeyModifiers;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use tui_pane::FavoritesFileState;
    use tui_pane::KeyBind;

    use super::*;
    use crate::favorites_overlay::tests::RECOGNIZED_ROWS_TWO;

    /// What one key press leaves behind in the modal and the attract screen.
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    struct KeyEffect {
        open:              bool,
        position:          usize,
        column_page:       usize,
        delete_armed:      bool,
        attract_requested: bool,
    }

    /// Open the two-row file on a fresh app, draw it narrow enough to page its columns, press
    /// `prelude` and then `code` through the full key ladder, and read back what changed.
    fn effect_of(prelude: &[KeyCode], code: KeyCode) -> KeyEffect {
        let mut app = App::new_for_test().expect("test app should build");
        let rows = tui_pane::parse_favorite_rows_for_test(RECOGNIZED_ROWS_TWO)
            .expect("favorites fixture should parse");
        let current_parameters = app.attract.current_settings().into();
        let keymap = Rc::clone(&app.keymap);
        app.favorites_overlay.open_file_state(
            FavoritesFileState::Loaded {
                path: PathBuf::from("/tmp/favorites.toml"),
                rows,
            },
            current_parameters,
            &keymap,
        );
        let mut terminal =
            Terminal::new(TestBackend::new(46, 20)).expect("test terminal should build");
        terminal
            .draw(|frame| app.favorites_overlay.render(frame))
            .expect("favorites overlay should render");

        for pressed in prelude.iter().copied().chain([code]) {
            tui_pane::dispatch_key(&mut app, KeyEvent::new(pressed, KeyModifiers::NONE));
        }
        KeyEffect {
            open:              app.favorites_overlay.is_open(),
            position:          app.favorites_overlay.viewport.pos(),
            column_page:       app.favorites_overlay.horizontal_column_page,
            delete_armed:      app
                .favorites_overlay
                .deletion_confirmation_is_armed_for_test(),
            attract_requested: app.attract.asked_for(),
        }
    }

    /// Every default key reaches the open modal ahead of the app globals that bind the same keys,
    /// and each lands on its own action. Up and Left start one step in, so their step back shows.
    #[test]
    fn favorites_pane_key_dispatch_is_pinned() {
        let resting = KeyEffect {
            open:              true,
            position:          0,
            column_page:       0,
            delete_armed:      false,
            attract_requested: false,
        };
        let one_down = KeyEffect {
            position: 1,
            ..resting
        };
        let one_page_right = KeyEffect {
            column_page: 1,
            ..resting
        };
        let closed = KeyEffect {
            open: false,
            ..resting
        };
        let cases: [(&[KeyCode], KeyCode, KeyEffect); 11] = [
            (&[KeyCode::Down], KeyCode::Up, resting),
            (&[KeyCode::Down], KeyCode::Char('k'), resting),
            (&[], KeyCode::Down, one_down),
            (&[], KeyCode::Char('j'), one_down),
            (&[KeyCode::Right], KeyCode::Left, resting),
            (&[KeyCode::Right], KeyCode::Char('h'), resting),
            (&[], KeyCode::Right, one_page_right),
            (&[], KeyCode::Char('l'), one_page_right),
            (
                &[],
                KeyCode::Enter,
                KeyEffect {
                    attract_requested: true,
                    ..closed
                },
            ),
            (
                &[],
                KeyCode::Char('x'),
                KeyEffect {
                    delete_armed: true,
                    ..resting
                },
            ),
            (&[], KeyCode::Esc, closed),
        ];
        for (prelude, code, expected) in cases {
            assert_eq!(
                effect_of(prelude, code),
                expected,
                "{code:?} after {prelude:?}"
            );
        }
    }

    #[test]
    fn modal_scope_includes_load_and_delete() {
        let scope = FavoritesOverlayPane::defaults().into_scope_map();
        let cases = [
            (
                KeyBind::from(KeyCode::Up),
                FavoritesOverlayAction::SelectPrevious,
            ),
            (KeyBind::from('k'), FavoritesOverlayAction::SelectPrevious),
            (
                KeyBind::from(KeyCode::Down),
                FavoritesOverlayAction::SelectNext,
            ),
            (KeyBind::from('j'), FavoritesOverlayAction::SelectNext),
            (
                KeyBind::from(KeyCode::Left),
                FavoritesOverlayAction::PageColumnsLeft,
            ),
            (KeyBind::from('h'), FavoritesOverlayAction::PageColumnsLeft),
            (
                KeyBind::from(KeyCode::Right),
                FavoritesOverlayAction::PageColumnsRight,
            ),
            (KeyBind::from('l'), FavoritesOverlayAction::PageColumnsRight),
            (KeyBind::from(KeyCode::Enter), FavoritesOverlayAction::Load),
            (KeyBind::from('x'), FavoritesOverlayAction::Delete),
            (KeyBind::from(KeyCode::Esc), FavoritesOverlayAction::Close),
        ];
        for (binding, action) in cases {
            assert_eq!(scope.action_for(&binding), Some(action));
        }
    }
}
