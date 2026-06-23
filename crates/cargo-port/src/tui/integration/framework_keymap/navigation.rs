use super::Action;
use super::App;
use super::AppGlobalAction;
use super::AppPaneId;
use super::Bindings;
use super::FocusedPane;
use super::Globals;
use super::KeyBind;
use super::NavAction;
use super::Navigation;
use super::input;
use super::panes;
use super::sccache;

/// `Navigation<App>` host. Zero-sized because the framework only needs
/// the type; navigation defaults / dispatch are static methods.
pub struct AppNavigation;

impl Navigation<App> for AppNavigation {
    const SECTION_NAME: &'static str = "List Navigation";

    fn dispatcher() -> fn(NavAction, FocusedPane<AppPaneId>, &mut App) {
        panes::dispatch_navigation_action
    }
}

impl Globals<App> for AppGlobalAction {
    type Actions = Self;

    const SECTION_NAME: &'static str = "Global Shortcuts";

    fn render_order() -> &'static [Self::Actions] { Self::ALL }

    fn defaults() -> Bindings<Self::Actions> {
        tui_pane::bindings! {
            'y'                  => Self::Copy,
            '/'                  => Self::Find,
            'e'                  => Self::OpenEditor,
            't'                  => Self::OpenTerminal,
            KeyBind::ctrl('r')   => Self::Rescan,
            'c'                  => Self::Clean,
            'S'                  => Self::SccacheStats,
            ' '                  => Self::PauseLint,
        }
    }

    fn dispatcher() -> fn(Self::Actions, &mut App) { dispatch_app_global }
}

pub(super) fn dispatch_app_global(action: AppGlobalAction, app: &mut App) {
    match action {
        AppGlobalAction::Copy => app.copy_focused_selection(),
        AppGlobalAction::Find => input::open_finder(app),
        AppGlobalAction::OpenEditor => input::open_in_editor(app),
        AppGlobalAction::OpenTerminal => input::open_terminal(app),
        AppGlobalAction::Rescan => app.rescan(),
        AppGlobalAction::Clean => panes::request_clean(app),
        AppGlobalAction::SccacheStats => sccache::open_sccache_stats_overlay(app),
        AppGlobalAction::PauseLint => app.toggle_lint_pause(),
    }
}
