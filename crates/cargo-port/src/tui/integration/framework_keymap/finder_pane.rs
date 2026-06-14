use super::App;
use super::AppPaneId;
use super::Bindings;
use super::FinderAction;
use super::KeyBind;
use super::KeyOutcome;
use super::Mode;
use super::Pane;
use super::Rc;
use super::Shortcuts;
use super::TabStop;
use super::finder;

/// `Pane<App>` + `Shortcuts<App>` host for the Finder overlay.
///
/// Mode flips with `app.overlays.is_finder_open()`:
/// - open  → `Mode::TextInput(finder_keys)` — character keys go to the embedded handler, which
///   dispatches Finder actions through the framework keymap before falling back to text entry.
/// - closed → `Mode::Navigable` (default Browse-style behaviour, though while closed the Finder
///   pane never actually has focus).
pub struct FinderPane;

impl Pane<App> for FinderPane {
    const APP_PANE_ID: AppPaneId = AppPaneId::Finder;

    fn mode() -> fn(&App) -> Mode<App> {
        |ctx| {
            if ctx.overlays.is_finder_open() {
                Mode::TextInput(finder_keys)
            } else {
                Mode::Navigable
            }
        }
    }

    fn tab_stop() -> TabStop<App> { TabStop::never() }
}

impl Shortcuts<App> for FinderPane {
    type Actions = FinderAction;

    const SCOPE_NAME: &'static str = "finder";
    const SECTION_NAME: &'static str = "Finder";

    fn defaults() -> Bindings<Self::Actions> {
        tui_pane::bindings! {
            crossterm::event::KeyCode::Enter => FinderAction::Activate,
            crossterm::event::KeyCode::Esc   => FinderAction::Cancel,
        }
    }

    fn dispatcher() -> fn(Self::Actions, &mut App) { finder::dispatch_finder_action }
}

/// `Mode::TextInput` handler for the Finder. Routes a single keypress
/// through the Finder pane scope first, then falls back to text entry
/// and result-list navigation for keys that are not Finder actions.
fn finder_keys(bind: KeyBind, app: &mut App) {
    let keymap = Rc::clone(&app.framework_keymap);
    match keymap.dispatch_app_pane(FinderPane::APP_PANE_ID, &bind, app) {
        KeyOutcome::Consumed => {},
        KeyOutcome::Unhandled => finder::handle_finder_text_key(app, bind.code),
    }
}
