//! Phase 13 framework-pane bar snapshot tests.
//!
//! Covers the framework's overlay panes (Keymap / Settings) in every
//! `EditState` reachable through the Phase 13 test scaffolding plus
//! focused-Toasts. App-pane snapshots land in Phase 14 once the app's
//! `Shortcuts<App>` impls exist.

use std::path::PathBuf;

use crossterm::event::KeyCode;
use ratatui::text::Span;

use super::BarPalette;
use super::StatusBar;
use super::render as render_inner;
use crate::AppContext;
use crate::Bindings;
use crate::FocusedPane;
use crate::Framework;
use crate::FrameworkFocusId;
use crate::FrameworkOverlayId;
use crate::Globals;
use crate::KeyBind;
use crate::KeymapPane;
use crate::Mode;
use crate::Navigation;
use crate::Pane;
use crate::SettingsPane;
use crate::Shortcuts;
use crate::keymap::Keymap;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum TestPaneId {
    Foo,
    Bar,
}

crate::action_enum! {
    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    pub enum FooAction {
        Activate => ("activate", "go",     "Activate row");
        Clean    => ("clean",    "clean",  "Clean target");
    }
}

crate::action_enum! {
    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    pub enum NavAction {
        Up    => ("up",    "up",    "Move up");
        Down  => ("down",  "down",  "Move down");
        Left  => ("left",  "left",  "Move left");
        Right => ("right", "right", "Move right");
        Home  => ("home",  "home",  "Jump to start");
        End   => ("end",   "end",   "Jump to end");
    }
}

crate::action_enum! {
    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    pub enum AppGlobalAction {
        Find => ("find", "find", "Open finder");
    }
}

struct TestApp {
    framework: Framework<Self>,
}

impl AppContext for TestApp {
    type AppPaneId = TestPaneId;

    fn framework(&self) -> &Framework<Self> { &self.framework }
    fn framework_mut(&mut self) -> &mut Framework<Self> { &mut self.framework }
}

struct FooPane;

impl Pane<TestApp> for FooPane {
    const APP_PANE_ID: TestPaneId = TestPaneId::Foo;
}

impl Shortcuts<TestApp> for FooPane {
    type Actions = FooAction;

    const SCOPE_NAME: &'static str = "foo";

    fn defaults() -> Bindings<Self::Actions> {
        crate::bindings! {
            KeyCode::Enter => FooAction::Activate,
            'c' => FooAction::Clean,
        }
    }

    fn dispatcher() -> fn(Self::Actions, &mut TestApp) {
        |_action, _ctx| { /* no-op */ }
    }
}

/// Static-mode pane: returns [`Mode::Static`] from `Pane::mode()`.
/// Used to assert that `Nav` is suppressed but `PaneAction` and
/// `Global` still render.
struct StaticPane;

impl Pane<TestApp> for StaticPane {
    const APP_PANE_ID: TestPaneId = TestPaneId::Bar;

    fn mode() -> fn(&TestApp) -> Mode<TestApp> { |_ctx| Mode::Static }
}

impl Shortcuts<TestApp> for StaticPane {
    type Actions = FooAction;

    const SCOPE_NAME: &'static str = "bar";

    fn defaults() -> Bindings<Self::Actions> {
        crate::bindings! { 'a' => FooAction::Activate }
    }

    fn dispatcher() -> fn(Self::Actions, &mut TestApp) {
        |_action, _ctx| { /* no-op */ }
    }
}

/// TextInput-mode pane: returns [`Mode::TextInput(...)`]. Used to
/// assert that every region is suppressed.
struct TextInputPane;

impl Pane<TestApp> for TextInputPane {
    const APP_PANE_ID: TestPaneId = TestPaneId::Bar;

    fn mode() -> fn(&TestApp) -> Mode<TestApp> { |_ctx| Mode::TextInput(text_input_handler) }
}

const fn text_input_handler(_bind: KeyBind, _ctx: &mut TestApp) {}

impl Shortcuts<TestApp> for TextInputPane {
    type Actions = FooAction;

    const SCOPE_NAME: &'static str = "bar";

    fn defaults() -> Bindings<Self::Actions> {
        crate::bindings! { KeyCode::Esc => FooAction::Clean }
    }

    fn dispatcher() -> fn(Self::Actions, &mut TestApp) {
        |_action, _ctx| { /* no-op */ }
    }
}

struct AppNav;

impl Navigation<TestApp> for AppNav {
    type Actions = NavAction;

    const DOWN: Self::Actions = NavAction::Down;
    const END: Self::Actions = NavAction::End;
    const HOME: Self::Actions = NavAction::Home;
    const LEFT: Self::Actions = NavAction::Left;
    const RIGHT: Self::Actions = NavAction::Right;
    const UP: Self::Actions = NavAction::Up;

    fn defaults() -> Bindings<Self::Actions> {
        crate::bindings! {
            KeyCode::Up    => NavAction::Up,
            KeyCode::Down  => NavAction::Down,
            KeyCode::Left  => NavAction::Left,
            KeyCode::Right => NavAction::Right,
            KeyCode::Home  => NavAction::Home,
            KeyCode::End   => NavAction::End,
        }
    }

    fn dispatcher() -> fn(Self::Actions, FocusedPane<TestPaneId>, &mut TestApp) {
        |_action, _focused, _ctx| { /* no-op */ }
    }
}

struct AppGlobals;

impl Globals<TestApp> for AppGlobals {
    type Actions = AppGlobalAction;

    fn render_order() -> &'static [Self::Actions] { &[AppGlobalAction::Find] }

    fn defaults() -> Bindings<Self::Actions> {
        crate::bindings! { 'f' => AppGlobalAction::Find }
    }

    fn dispatcher() -> fn(Self::Actions, &mut TestApp) {
        |_action, _ctx| { /* no-op */ }
    }
}

fn fresh_app(initial: FocusedPane<TestPaneId>) -> TestApp {
    TestApp {
        framework: Framework::new(initial),
    }
}

/// Build a keymap that registers `FooPane` (Navigable), navigation,
/// and globals. Uses `build_into` so the framework's per-pane mode
/// registry is populated.
fn build_keymap_with_foo(framework: &mut Framework<TestApp>) -> Keymap<TestApp> {
    Keymap::<TestApp>::builder()
        .register_navigation::<AppNav>()
        .expect("register_navigation")
        .register_globals::<AppGlobals>()
        .expect("register_globals")
        .register::<FooPane>(FooPane)
        .build_into(framework)
        .expect("build_into")
}

/// Same as [`build_keymap_with_foo`] but registers a static-mode pane
/// as the second registered pane.
fn build_keymap_with_foo_and_static(framework: &mut Framework<TestApp>) -> Keymap<TestApp> {
    Keymap::<TestApp>::builder()
        .register_navigation::<AppNav>()
        .expect("register_navigation")
        .register_globals::<AppGlobals>()
        .expect("register_globals")
        .register::<FooPane>(FooPane)
        .register::<StaticPane>(StaticPane)
        .build_into(framework)
        .expect("build_into")
}

/// Same as [`build_keymap_with_foo`] but registers a text-input-mode
/// pane.
fn build_keymap_with_foo_and_textinput(framework: &mut Framework<TestApp>) -> Keymap<TestApp> {
    Keymap::<TestApp>::builder()
        .register_navigation::<AppNav>()
        .expect("register_navigation")
        .register_globals::<AppGlobals>()
        .expect("register_globals")
        .register::<FooPane>(FooPane)
        .register::<TextInputPane>(TextInputPane)
        .build_into(framework)
        .expect("build_into")
}

/// Test-only wrapper that drives [`render`](super::render) with the
/// theme-neutral [`BarPalette::default`]. Phase 13 / Phase 14 in-crate
/// tests assert on `Span::content` and don't inspect styling — the
/// default palette keeps every span unstyled.
#[allow(
    clippy::trivially_copy_pass_by_ref,
    reason = "matches `render_inner`'s signature 1:1; rewriting call sites \
              would mean `&FocusedPane::App(...)` becomes `FocusedPane::App(...)` \
              everywhere in the file"
)]
fn render(
    focused: &FocusedPane<TestPaneId>,
    ctx: &TestApp,
    keymap: &Keymap<TestApp>,
    framework: &Framework<TestApp>,
) -> StatusBar {
    let palette = BarPalette::default();
    render_inner(focused, ctx, keymap, framework, &palette)
}

/// Render every `Span`'s content into one string, with no styling.
fn flatten(spans: &[Span<'static>]) -> String {
    let mut s = String::new();
    for span in spans {
        s.push_str(&span.content);
    }
    s
}

fn flatten_bar(bar: &StatusBar) -> (String, String, String) {
    (
        flatten(&bar.nav),
        flatten(&bar.pane_action),
        flatten(&bar.global),
    )
}

// ── Per-region rule fixture ──────────────────────────────────────────

#[test]
fn navigable_mode_renders_every_region() {
    let mut app = fresh_app(FocusedPane::App(TestPaneId::Foo));
    let keymap = build_keymap_with_foo(&mut app.framework);
    let bar = render(
        &FocusedPane::App(TestPaneId::Foo),
        &app,
        &keymap,
        app.framework(),
    );
    let (nav, pane_action, global) = flatten_bar(&bar);
    assert!(!nav.is_empty(), "Nav region must render in Navigable mode");
    assert!(
        !pane_action.is_empty(),
        "PaneAction region must render in Navigable mode",
    );
    assert!(
        !global.is_empty(),
        "Global region must render in Navigable mode",
    );
}

#[test]
fn static_mode_suppresses_nav_only() {
    let mut app = fresh_app(FocusedPane::App(TestPaneId::Bar));
    let keymap = build_keymap_with_foo_and_static(&mut app.framework);
    let bar = render(
        &FocusedPane::App(TestPaneId::Bar),
        &app,
        &keymap,
        app.framework(),
    );
    let (nav, pane_action, global) = flatten_bar(&bar);
    assert!(nav.is_empty(), "Nav must be suppressed in Static mode");
    assert!(!pane_action.is_empty(), "PaneAction renders in Static mode");
    assert!(!global.is_empty(), "Global renders in Static mode");
}

#[test]
fn textinput_mode_suppresses_every_region() {
    let mut app = fresh_app(FocusedPane::App(TestPaneId::Bar));
    let keymap = build_keymap_with_foo_and_textinput(&mut app.framework);
    let bar = render(
        &FocusedPane::App(TestPaneId::Bar),
        &app,
        &keymap,
        app.framework(),
    );
    assert!(
        bar.is_empty(),
        "Every region must be suppressed in TextInput mode (got {bar:?})",
    );
}

#[test]
fn unregistered_pane_id_renders_empty_bar() {
    // Nothing registered; the focus is on `Foo` but no scope answers
    // `mode_queries`, so `focused_pane_mode` returns `None`.
    let app = fresh_app(FocusedPane::App(TestPaneId::Foo));
    let keymap: Keymap<TestApp> = Keymap::new(None);
    let bar = render(
        &FocusedPane::App(TestPaneId::Foo),
        &app,
        &keymap,
        app.framework(),
    );
    assert!(bar.is_empty(), "None mode → empty bar (got {bar:?})");
}

// ── Framework overlay panes ──────────────────────────────────────────

#[test]
fn keymap_browse_renders_pane_actions_and_globals() {
    let mut app = fresh_app(FocusedPane::App(TestPaneId::Foo));
    let keymap = build_keymap_with_foo(&mut app.framework);
    app.framework_mut().dispatch_global_for_test_open_keymap();
    let focused = *app.framework().focused();
    let bar = render(&focused, &app, &keymap, app.framework());
    let (nav, pane_action, global) = flatten_bar(&bar);
    // Browse → Mode::Navigable → Nav, PaneAction, Global all render.
    assert!(
        !pane_action.is_empty(),
        "Keymap Browse must show local actions"
    );
    assert!(
        pane_action.contains("edit"),
        "got pane_action={pane_action:?}"
    );
    assert!(
        pane_action.contains("save"),
        "got pane_action={pane_action:?}"
    );
    assert!(
        pane_action.contains("cancel"),
        "got pane_action={pane_action:?}"
    );
    assert!(!global.is_empty(), "Keymap Browse must show globals");
    // Nav region: pane-cycle row from framework globals (NextPane/PrevPane)
    // — even though no app pane is in focus, the cycle row resolves
    // through the framework_globals scope.
    assert!(
        !nav.is_empty(),
        "Keymap Browse must show pane-cycle / nav row"
    );
}

#[test]
fn keymap_awaiting_text_input_suppresses_every_region() {
    let mut app = fresh_app(FocusedPane::App(TestPaneId::Foo));
    let keymap = build_keymap_with_foo(&mut app.framework);
    app.framework_mut().dispatch_global_for_test_open_keymap();
    // Replace the framework's keymap pane with one in Awaiting.
    app.framework_mut().keymap_pane =
        KeymapPane::<TestApp>::for_test_awaiting(Some(PathBuf::from("/tmp/keys.toml")));
    let focused = *app.framework().focused();
    let bar = render(&focused, &app, &keymap, app.framework());
    assert!(
        bar.is_empty(),
        "Keymap Awaiting (TextInput mode) suppresses every region (got {bar:?})",
    );
}

#[test]
fn keymap_conflict_renders_pane_actions_and_globals() {
    let mut app = fresh_app(FocusedPane::App(TestPaneId::Foo));
    let keymap = build_keymap_with_foo(&mut app.framework);
    app.framework_mut().dispatch_global_for_test_open_keymap();
    app.framework_mut().keymap_pane =
        KeymapPane::<TestApp>::for_test_conflict(Some(PathBuf::from("/tmp/keys.toml")));
    let focused = *app.framework().focused();
    let bar = render(&focused, &app, &keymap, app.framework());
    let (nav, pane_action, global) = flatten_bar(&bar);
    // Conflict → Mode::Static → Nav suppressed; PaneAction + Global render.
    assert!(
        nav.is_empty(),
        "Conflict mode suppresses Nav (got nav={nav:?})"
    );
    assert!(
        !pane_action.is_empty(),
        "Conflict shows local Cancel/Save (got pane_action={pane_action:?})",
    );
    assert!(
        !global.is_empty(),
        "Conflict shows globals (got global={global:?})"
    );
}

#[test]
fn settings_browse_renders_pane_actions_and_globals() {
    let mut app = fresh_app(FocusedPane::App(TestPaneId::Foo));
    let keymap = build_keymap_with_foo(&mut app.framework);
    app.framework_mut().dispatch_global_for_test_open_settings();
    let focused = *app.framework().focused();
    let bar = render(&focused, &app, &keymap, app.framework());
    let (_, pane_action, global) = flatten_bar(&bar);
    assert!(pane_action.contains("edit"));
    assert!(pane_action.contains("save"));
    assert!(pane_action.contains("cancel"));
    assert!(!global.is_empty());
}

#[test]
fn settings_editing_text_input_suppresses_every_region() {
    let mut app = fresh_app(FocusedPane::App(TestPaneId::Foo));
    let keymap = build_keymap_with_foo(&mut app.framework);
    app.framework_mut().dispatch_global_for_test_open_settings();
    app.framework_mut().settings_pane =
        SettingsPane::<TestApp>::for_test_editing(Some(PathBuf::from("/tmp/settings.toml")));
    let focused = *app.framework().focused();
    let bar = render(&focused, &app, &keymap, app.framework());
    assert!(
        bar.is_empty(),
        "Settings Editing (TextInput mode) suppresses every region (got {bar:?})",
    );
}

// ── Focused Toasts ───────────────────────────────────────────────────

#[test]
fn focused_toasts_renders_nav_and_globals_no_pane_actions() {
    let mut app = fresh_app(FocusedPane::Framework(FrameworkFocusId::Toasts));
    let keymap = build_keymap_with_foo(&mut app.framework);
    let _ = app.framework_mut().toasts.push("Build done", "ok");
    let focused = FocusedPane::Framework(FrameworkFocusId::Toasts);
    let bar = render(&focused, &app, &keymap, app.framework());
    let (nav, pane_action, global) = flatten_bar(&bar);
    // Toasts focus → Mode::Navigable → Nav + Global render; PaneAction
    // is empty because ToastsAction has no variants in Phase 12.
    assert!(!nav.is_empty(), "Toasts must show nav (got nav={nav:?})");
    assert!(
        pane_action.is_empty(),
        "Toasts has no PaneAction in Phase 12 (got pane_action={pane_action:?})",
    );
    assert!(
        !global.is_empty(),
        "Toasts must show globals (got global={global:?})"
    );
    // The focused-Toasts dismiss path goes through `GlobalAction::Dismiss`,
    // which renders by its bar_label (`dismiss`).
    assert!(global.contains("dismiss"), "got global={global:?}");
}

#[test]
fn nav_row_renders_resolved_keys_via_display_short() {
    let mut app = fresh_app(FocusedPane::App(TestPaneId::Foo));
    let keymap = build_keymap_with_foo(&mut app.framework);
    let bar = render(
        &FocusedPane::App(TestPaneId::Foo),
        &app,
        &keymap,
        app.framework(),
    );
    let nav = flatten(&bar.nav);
    // Default nav binds Up/Down to KeyCode::Up/Down → renders as ↑ /
    // ↓ via `display_short`. The pane-cycle row uses Tab.
    assert!(
        nav.contains('↑'),
        "default nav must show ↑ glyph (got {nav:?})"
    );
    assert!(
        nav.contains('↓'),
        "default nav must show ↓ glyph (got {nav:?})"
    );
    // Phase 19 widens this to a full rebind regression suite.
}

// ── Helpers ──────────────────────────────────────────────────────────

trait FrameworkTestHelpers {
    fn dispatch_global_for_test_open_keymap(&mut self);
    fn dispatch_global_for_test_open_settings(&mut self);
}

impl FrameworkTestHelpers for Framework<TestApp> {
    fn dispatch_global_for_test_open_keymap(&mut self) {
        self.set_overlay_for_test(FrameworkOverlayId::Keymap);
    }

    fn dispatch_global_for_test_open_settings(&mut self) {
        self.set_overlay_for_test(FrameworkOverlayId::Settings);
    }
}
