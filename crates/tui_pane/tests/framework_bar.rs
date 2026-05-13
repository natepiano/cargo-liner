//! Integration tests for the framework's bar renderer.
//!
//! Drives [`tui_pane::render_status_bar`] from outside the crate to
//! lock the public path. Browse / focused-Toasts coverage live here;
//! the in-crate unit tests in `bar/tests.rs` cover overlay edit
//! states (Awaiting / Conflict / Editing) that need the `cfg(test)`
//! `for_test_*` constructors.

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    reason = "tests should panic on unexpected values"
)]

use crossterm::event::KeyCode;
use ratatui::text::Span;
use tui_pane::AppContext;
use tui_pane::BarPalette;
use tui_pane::Bindings;
use tui_pane::FocusedPane;
use tui_pane::Framework;
use tui_pane::FrameworkFocusId;
use tui_pane::GlobalAction;
use tui_pane::Globals;
use tui_pane::Keymap;
use tui_pane::Navigation;
use tui_pane::Pane;
use tui_pane::Shortcuts;
use tui_pane::StatusLineGlobal;
use tui_pane::render_status_bar;
use tui_pane::status_line_global_spans;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum AppPaneId {
    Project,
}

tui_pane::action_enum! {
    /// Test-only project pane actions.
    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    pub enum ProjectAction {
        /// Activate the selected row.
        Activate => ("activate", "go",     "Activate row");
        /// Refresh the pane.
        Refresh  => ("refresh",  "refresh","Refresh");
    }
}

tui_pane::action_enum! {
    /// Test-only navigation actions.
    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    pub enum NavAction {
        /// Move up.
        Up    => ("up",    "up",    "Up");
        /// Move down.
        Down  => ("down",  "down",  "Down");
        /// Move left.
        Left  => ("left",  "left",  "Left");
        /// Move right.
        Right => ("right", "right", "Right");
        /// Jump to the start.
        Home  => ("home",  "home",  "Home");
        /// Jump to the end.
        End   => ("end",   "end",   "End");
    }
}

tui_pane::action_enum! {
    /// Test-only app-global actions.
    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    pub enum AppGlobalAction {
        /// Open the finder.
        Find => ("find", "find", "Find");
    }
}

struct App {
    framework: Framework<Self>,
}

impl AppContext for App {
    type AppPaneId = AppPaneId;
    type ToastAction = tui_pane::NoToastAction;

    fn framework(&self) -> &Framework<Self> { &self.framework }
    fn framework_mut(&mut self) -> &mut Framework<Self> { &mut self.framework }
}

struct ProjectPane;

impl Pane<App> for ProjectPane {
    const APP_PANE_ID: AppPaneId = AppPaneId::Project;
}

impl Shortcuts<App> for ProjectPane {
    type Actions = ProjectAction;

    const SCOPE_NAME: &'static str = "project";

    fn defaults() -> Bindings<Self::Actions> {
        tui_pane::bindings! {
            KeyCode::Enter => ProjectAction::Activate,
            'r' => ProjectAction::Refresh,
        }
    }

    fn dispatcher() -> fn(Self::Actions, &mut App) {
        |_action, _ctx| { /* no-op */ }
    }
}

struct AppNav;

impl Navigation<App> for AppNav {
    type Actions = NavAction;

    const DOWN: Self::Actions = NavAction::Down;
    const END: Self::Actions = NavAction::End;
    const HOME: Self::Actions = NavAction::Home;
    const LEFT: Self::Actions = NavAction::Left;
    const RIGHT: Self::Actions = NavAction::Right;
    const UP: Self::Actions = NavAction::Up;

    fn defaults() -> Bindings<Self::Actions> {
        tui_pane::bindings! {
            KeyCode::Up    => NavAction::Up,
            KeyCode::Down  => NavAction::Down,
            KeyCode::Left  => NavAction::Left,
            KeyCode::Right => NavAction::Right,
            KeyCode::Home  => NavAction::Home,
            KeyCode::End   => NavAction::End,
        }
    }

    fn dispatcher() -> fn(Self::Actions, FocusedPane<AppPaneId>, &mut App) {
        |_action, _focused, _ctx| { /* no-op */ }
    }
}

struct AppGlobals;

impl Globals<App> for AppGlobals {
    type Actions = AppGlobalAction;

    fn render_order() -> &'static [Self::Actions] { &[AppGlobalAction::Find] }

    fn defaults() -> Bindings<Self::Actions> {
        tui_pane::bindings! { 'f' => AppGlobalAction::Find }
    }

    fn dispatcher() -> fn(Self::Actions, &mut App) {
        |_action, _ctx| { /* no-op */ }
    }
}

fn fresh(initial: FocusedPane<AppPaneId>) -> (App, Keymap<App>) {
    let mut app = App {
        framework: Framework::new(initial),
    };
    let keymap = Keymap::<App>::builder()
        .register_navigation::<AppNav>()
        .expect("register_navigation")
        .register_globals::<AppGlobals>()
        .expect("register_globals")
        .register::<ProjectPane>(ProjectPane)
        .build_into(&mut app.framework)
        .expect("build_into");
    (app, keymap)
}

fn flatten(spans: &[Span<'static>]) -> String {
    let mut s = String::new();
    for span in spans {
        s.push_str(&span.content);
    }
    s
}

#[test]
fn render_status_bar_navigable_pane_shows_every_region() {
    let (app, keymap) = fresh(FocusedPane::App(AppPaneId::Project));
    let palette = BarPalette::default();
    let bar = render_status_bar(
        &FocusedPane::App(AppPaneId::Project),
        &app,
        &keymap,
        app.framework(),
        &palette,
    );
    let nav = flatten(&bar.nav);
    let pane_action = flatten(&bar.pane_action);
    let global = flatten(&bar.global);

    assert!(
        nav.contains("Tab"),
        "nav must show Tab pane-cycle (got {nav:?})"
    );
    assert!(nav.contains('↑'), "nav must show ↑ glyph (got {nav:?})");
    assert!(
        pane_action.contains("go"),
        "pane action must show project's Activate label (got {pane_action:?})",
    );
    assert!(
        pane_action.contains("refresh"),
        "pane action must show Refresh (got {pane_action:?})",
    );
    assert!(
        global.contains("find"),
        "global must show app-globals Find (got {global:?})",
    );
    assert!(
        global.contains("quit"),
        "global must show framework's Quit (got {global:?})",
    );
}

#[test]
fn status_line_global_spans_follow_supplied_policy_order() {
    let (_app, keymap) = fresh(FocusedPane::App(AppPaneId::Project));
    let globals = [
        StatusLineGlobal::app(AppGlobalAction::Find),
        StatusLineGlobal::framework(GlobalAction::OpenSettings),
        StatusLineGlobal::framework(GlobalAction::Quit),
    ];

    let spans =
        status_line_global_spans::<App, AppGlobals>(&keymap, &globals, &BarPalette::default());
    let global = flatten(&spans);

    assert_contains_in_order(&global, &["find", "settings", "quit"]);
    assert!(
        !global.contains("dismiss"),
        "status-line policy omitted Dismiss, so renderer must not add it (got {global:?})",
    );
}

#[test]
fn render_status_bar_focused_toasts_renders_dismiss_in_global() {
    let (mut app, keymap) = fresh(FocusedPane::Framework(FrameworkFocusId::Toasts));
    let _ = app.framework_mut().toasts.push("Build done", "ok");
    let palette = BarPalette::default();
    let bar = render_status_bar(
        &FocusedPane::Framework(FrameworkFocusId::Toasts),
        &app,
        &keymap,
        app.framework(),
        &palette,
    );
    let pane_action = flatten(&bar.pane_action);
    let global = flatten(&bar.global);

    assert!(
        pane_action.contains("Enter open"),
        "Toast focus must show ToastsAction::Activate (got {pane_action:?})",
    );
    assert!(
        global.contains("dismiss"),
        "Toast focus must show GlobalAction::Dismiss (got {global:?})",
    );
}

fn assert_contains_in_order(text: &str, needles: &[&str]) {
    let mut start = 0;
    for needle in needles {
        let Some(offset) = text[start..].find(needle) else {
            panic!("expected {needle:?} after byte {start} in {text:?}");
        };
        start += offset + needle.len();
    }
}

#[test]
fn status_bar_default_is_empty() {
    let bar = tui_pane::StatusBar::default();
    assert!(bar.is_empty());
}
