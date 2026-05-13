//! Integration tests for the framework-owned typed toasts manager,
//! the split overlay/focus id enums, and the virtual focus cycle.
//!
//! Driven through the public surface (`Keymap::dispatch_framework_global`
//! → `dispatch_global` → `focus_step` / `dismiss_chain`) so the tests
//! exercise the same paths the binary will hit.

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    reason = "tests should panic on unexpected values"
)]

use std::cell::Cell;

use crossterm::event::KeyCode;
use tui_pane::AppContext;
use tui_pane::Bindings;
use tui_pane::FocusedPane;
use tui_pane::Framework;
use tui_pane::FrameworkFocusId;
use tui_pane::GlobalAction;
use tui_pane::Globals;
use tui_pane::Keymap;
use tui_pane::ListNavigation;
use tui_pane::Navigation;
use tui_pane::Pane;
use tui_pane::Shortcuts;

tui_pane::action_enum! {
    /// Test-only foo pane actions.
    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    pub enum FooAction {
        /// Activate the selected row.
        Activate => ("activate", "go", "Activate row");
    }
}

tui_pane::action_enum! {
    /// Test-only navigation actions.
    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    pub enum NavAction {
        /// Move up.
        Up    => ("up",    "up",    "Move up");
        /// Move down.
        Down  => ("down",  "down",  "Move down");
        /// Move left.
        Left  => ("left",  "left",  "Move left");
        /// Move right.
        Right => ("right", "right", "Move right");
        /// Jump to the start.
        Home  => ("home",  "home",  "Jump to start");
        /// Jump to the end.
        End   => ("end",   "end",   "Jump to end");
    }
}

tui_pane::action_enum! {
    /// Test-only app-global actions.
    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    pub enum AppGlobalAction {
        /// Open the finder.
        Find => ("find", "find", "Open find");
    }
}

/// Test-only pane identifier.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum TestPaneId {
    /// First fixture pane.
    Foo,
    /// Second fixture pane.
    Bar,
}

/// Test-only `AppContext` implementation that counts `set_focus` calls.
pub struct TestApp {
    framework:           Framework<Self>,
    /// Number of times the test set focus on this app.
    pub set_focus_calls: Cell<u32>,
}

impl AppContext for TestApp {
    type AppPaneId = TestPaneId;
    type ToastAction = tui_pane::NoToastAction;

    fn framework(&self) -> &Framework<Self> { &self.framework }
    fn framework_mut(&mut self) -> &mut Framework<Self> { &mut self.framework }
    fn set_focus(&mut self, focus: FocusedPane<Self::AppPaneId>) {
        self.set_focus_calls.set(self.set_focus_calls.get() + 1);
        self.framework.set_focused(focus);
    }
}

/// Test-only pane fixture bound to [`TestPaneId::Foo`].
pub struct FooPane;
impl Pane<TestApp> for FooPane {
    const APP_PANE_ID: TestPaneId = TestPaneId::Foo;
}
impl Shortcuts<TestApp> for FooPane {
    type Actions = FooAction;
    const SCOPE_NAME: &'static str = "foo";
    fn defaults() -> Bindings<Self::Actions> {
        tui_pane::bindings! { KeyCode::Enter => FooAction::Activate }
    }
    fn dispatcher() -> fn(Self::Actions, &mut TestApp) { |_a, _c| {} }
}

/// Test-only pane fixture bound to [`TestPaneId::Bar`].
pub struct BarPane;
impl Pane<TestApp> for BarPane {
    const APP_PANE_ID: TestPaneId = TestPaneId::Bar;
}
impl Shortcuts<TestApp> for BarPane {
    type Actions = FooAction;
    const SCOPE_NAME: &'static str = "bar";
    fn defaults() -> Bindings<Self::Actions> { FooPane::defaults() }
    fn dispatcher() -> fn(Self::Actions, &mut TestApp) { FooPane::dispatcher() }
}

/// Test-only navigation fixture.
pub struct AppNav;
impl Navigation<TestApp> for AppNav {
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

    fn dispatcher() -> fn(Self::Actions, FocusedPane<TestPaneId>, &mut TestApp) { |_a, _f, _c| {} }
}

/// Test-only globals fixture.
pub struct AppGlobals;
impl Globals<TestApp> for AppGlobals {
    type Actions = AppGlobalAction;
    fn render_order() -> &'static [Self::Actions] { &[AppGlobalAction::Find] }
    fn defaults() -> Bindings<Self::Actions> {
        tui_pane::bindings! { 'f' => AppGlobalAction::Find }
    }
    fn dispatcher() -> fn(Self::Actions, &mut TestApp) { |_a, _c| {} }
}

fn build_with_panes() -> (TestApp, Keymap<TestApp>) {
    let mut framework = Framework::<TestApp>::new(FocusedPane::App(TestPaneId::Foo));
    let keymap = Keymap::<TestApp>::builder()
        .register_navigation::<AppNav>()
        .expect("nav register")
        .register_globals::<AppGlobals>()
        .expect("globals register")
        .register::<FooPane>(FooPane)
        .register::<BarPane>(BarPane)
        .build_into(&mut framework)
        .expect("build_into");
    let app = TestApp {
        framework,
        set_focus_calls: Cell::new(0),
    };
    (app, keymap)
}

fn build_no_panes() -> (TestApp, Keymap<TestApp>) {
    let framework = Framework::<TestApp>::new(FocusedPane::App(TestPaneId::Foo));
    let keymap = Keymap::<TestApp>::builder().build().expect("empty build");
    let app = TestApp {
        framework,
        set_focus_calls: Cell::new(0),
    };
    (app, keymap)
}

#[test]
fn pane_order_empty_and_toasts_active_cycles_to_toasts() {
    let (mut app, keymap) = build_no_panes();
    app.framework.toasts.push("hi", "body");

    keymap.dispatch_framework_global(GlobalAction::NextPane, &mut app);
    assert_eq!(
        app.framework().focused(),
        &FocusedPane::Framework(FrameworkFocusId::Toasts),
    );
}

#[test]
fn toasts_inactive_while_focused_next_moves_to_app_pane() {
    let (mut app, keymap) = build_with_panes();
    app.framework.toasts.push("hi", "");
    app.set_focus(FocusedPane::Framework(FrameworkFocusId::Toasts));
    // dismiss the only toast → manager empty, focus stale on Toasts
    let _ = app.framework.toasts.dismiss_focused();

    keymap.dispatch_framework_global(GlobalAction::NextPane, &mut app);
    assert_eq!(
        app.framework().focused(),
        &FocusedPane::App(TestPaneId::Foo),
        "next from stale-Toasts focus must land on the first app pane",
    );
}

#[test]
fn prev_from_first_app_lands_on_toasts_when_active() {
    let (mut app, keymap) = build_with_panes();
    app.framework.toasts.push("hi", "");
    // focus is Foo (first)
    keymap.dispatch_framework_global(GlobalAction::PrevPane, &mut app);
    assert_eq!(
        app.framework().focused(),
        &FocusedPane::Framework(FrameworkFocusId::Toasts),
    );
}

#[test]
fn next_from_last_app_lands_on_toasts_when_active_then_wraps() {
    let (mut app, keymap) = build_with_panes();
    app.framework.toasts.push("hi", "");
    app.set_focus(FocusedPane::App(TestPaneId::Bar));

    keymap.dispatch_framework_global(GlobalAction::NextPane, &mut app);
    assert_eq!(
        app.framework().focused(),
        &FocusedPane::Framework(FrameworkFocusId::Toasts),
    );
    keymap.dispatch_framework_global(GlobalAction::NextPane, &mut app);
    assert_eq!(
        app.framework().focused(),
        &FocusedPane::App(TestPaneId::Foo),
        "cycle wraps from Toasts back to first app pane",
    );
}

#[test]
fn dismiss_focused_toast_removes_it_and_reconciles_focus() {
    let (mut app, keymap) = build_with_panes();
    app.framework.toasts.push("hi", "");
    app.set_focus(FocusedPane::Framework(FrameworkFocusId::Toasts));

    keymap.dispatch_framework_global(GlobalAction::Dismiss, &mut app);
    assert!(!app.framework.toasts.has_active());
    assert_eq!(
        app.framework().focused(),
        &FocusedPane::App(TestPaneId::Foo),
        "after dismiss empties toasts, focus moves to first app pane",
    );
}

#[test]
fn entering_toasts_with_next_calls_reset_to_first() {
    let (mut app, keymap) = build_with_panes();
    let a = app.framework.toasts.push("A", "");
    let _ = app.framework.toasts.push("B", "");
    let _ = app.framework.toasts.push("C", "");
    // walk cursor away from 0
    let _ = app.framework.toasts.on_navigation(ListNavigation::Down);
    let _ = app.framework.toasts.on_navigation(ListNavigation::Down);

    // current focus is Foo; Next-Next from Foo lands on Bar then Toasts
    keymap.dispatch_framework_global(GlobalAction::NextPane, &mut app);
    keymap.dispatch_framework_global(GlobalAction::NextPane, &mut app);
    assert_eq!(
        app.framework().focused(),
        &FocusedPane::Framework(FrameworkFocusId::Toasts),
    );
    assert_eq!(
        app.framework.toasts.focused_id(),
        Some(a),
        "Next-direction entry resets cursor to first toast",
    );
}

#[test]
fn entering_toasts_with_prev_calls_reset_to_last() {
    let (mut app, keymap) = build_with_panes();
    let _ = app.framework.toasts.push("A", "");
    let _ = app.framework.toasts.push("B", "");
    let c = app.framework.toasts.push("C", "");
    // cursor starts at 0

    keymap.dispatch_framework_global(GlobalAction::PrevPane, &mut app);
    assert_eq!(
        app.framework().focused(),
        &FocusedPane::Framework(FrameworkFocusId::Toasts),
    );
    assert_eq!(
        app.framework.toasts.focused_id(),
        Some(c),
        "Prev-direction entry resets cursor to last toast",
    );
}

#[test]
fn dismiss_chain_closes_overlay_when_no_focused_toast() {
    let (mut app, keymap) = build_no_panes();
    keymap.dispatch_framework_global(GlobalAction::OpenKeymap, &mut app);
    assert!(app.framework().overlay().is_some());

    keymap.dispatch_framework_global(GlobalAction::Dismiss, &mut app);
    assert!(app.framework().overlay().is_none());
}

#[test]
fn dismiss_chain_falls_through_to_fallback_when_neither_fires() {
    fn handler(ctx: &mut TestApp) -> bool {
        ctx.set_focus_calls.set(99);
        true
    }

    let mut framework = Framework::<TestApp>::new(FocusedPane::App(TestPaneId::Foo));
    let keymap = Keymap::<TestApp>::builder()
        .register_navigation::<AppNav>()
        .expect("nav")
        .register_globals::<AppGlobals>()
        .expect("globals")
        .dismiss_fallback(handler)
        .register::<FooPane>(FooPane)
        .build_into(&mut framework)
        .expect("build_into");
    let mut app = TestApp {
        framework,
        set_focus_calls: Cell::new(0),
    };

    keymap.dispatch_framework_global(GlobalAction::Dismiss, &mut app);
    assert_eq!(
        app.set_focus_calls.get(),
        99,
        "fallback fires when no overlay is open and Toasts is not focused",
    );
}

#[test]
fn focus_changes_route_through_app_context_set_focus() {
    let (mut app, keymap) = build_with_panes();
    app.framework.toasts.push("hi", "");
    let before = app.set_focus_calls.get();

    keymap.dispatch_framework_global(GlobalAction::NextPane, &mut app);
    assert!(
        app.set_focus_calls.get() > before,
        "NextPane must route through ctx.set_focus(...)",
    );
}

#[test]
fn dismiss_chain_reconcile_routes_through_set_focus() {
    let (mut app, keymap) = build_with_panes();
    app.framework.toasts.push("hi", "");
    app.set_focus(FocusedPane::Framework(FrameworkFocusId::Toasts));
    let before = app.set_focus_calls.get();

    keymap.dispatch_framework_global(GlobalAction::Dismiss, &mut app);
    assert!(
        app.set_focus_calls.get() > before,
        "post-dismiss reconciliation must call ctx.set_focus(...)",
    );
}
