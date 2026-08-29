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
mod tests {
    use tui_pane::KeyBind;

    use super::*;

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
