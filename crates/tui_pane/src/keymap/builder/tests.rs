use std::fs;
use std::process;

use crossterm::event::KeyCode;

use super::Keymap;
use super::VimMode;
use crate::AppContext;
use crate::FocusedPane;
use crate::Framework;
use crate::FrameworkFocusId;
use crate::FrameworkOverlayId;
use crate::GlobalAction;
use crate::KeyBind;
use crate::KeymapPaneAction;
use crate::Pane;
use crate::SettingsPaneAction;
use crate::TabStop;
use crate::keymap::Bindings;
use crate::keymap::Globals;
use crate::keymap::KeyOutcome;
use crate::keymap::KeymapError;
use crate::keymap::Navigation;
use crate::keymap::Shortcuts;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum TestPaneId {
    Foo,
    Bar,
    Baz,
    Excluded,
    Hidden,
}

const ORDERED_BAR_TAB_ORDER: i16 = 10;
const HIDDEN_TAB_ORDER: i16 = 15;
const ORDERED_FOO_TAB_ORDER: i16 = 20;
const ORDERED_BAZ_TAB_ORDER: i16 = 30;

crate::action_enum! {
    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    pub enum FooAction {
        Activate => ("activate", "go", "Activate row");
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
        Find => ("find", "find", "Open find");
    }
}

struct TestApp {
    framework: Framework<Self>,
    quits:     u32,
    restarts:  u32,
    dismisses: u32,
}

impl AppContext for TestApp {
    type AppPaneId = TestPaneId;
    type ToastAction = crate::NoToastAction;

    fn framework(&self) -> &Framework<Self> { &self.framework }
    fn framework_mut(&mut self) -> &mut Framework<Self> { &mut self.framework }
}

fn fresh_app() -> TestApp {
    TestApp {
        framework: Framework::new(FocusedPane::App(TestPaneId::Foo)),
        quits:     0,
        restarts:  0,
        dismisses: 0,
    }
}

struct FooPane;

impl Pane<TestApp> for FooPane {
    const APP_PANE_ID: TestPaneId = TestPaneId::Foo;
}

impl Shortcuts<TestApp> for FooPane {
    type Actions = FooAction;

    const SCOPE_NAME: &'static str = "foo";

    fn defaults() -> Bindings<Self::Actions> {
        crate::bindings! { KeyCode::Enter => FooAction::Activate }
    }

    fn dispatcher() -> fn(Self::Actions, &mut TestApp) { dispatch_noop }
}

fn dispatch_noop(_: FooAction, _: &mut TestApp) {}

fn never_tabbable(_: &TestApp) -> bool { false }

struct BarPane;

impl Pane<TestApp> for BarPane {
    const APP_PANE_ID: TestPaneId = TestPaneId::Bar;
}

impl Shortcuts<TestApp> for BarPane {
    type Actions = FooAction;

    const SCOPE_NAME: &'static str = "bar";

    fn defaults() -> Bindings<Self::Actions> { FooPane::defaults() }

    fn dispatcher() -> fn(Self::Actions, &mut TestApp) { dispatch_noop }
}

struct OrderedFooPane;

impl Pane<TestApp> for OrderedFooPane {
    const APP_PANE_ID: TestPaneId = TestPaneId::Foo;

    fn tab_stop() -> TabStop<TestApp> { TabStop::always(ORDERED_FOO_TAB_ORDER) }
}

impl Shortcuts<TestApp> for OrderedFooPane {
    type Actions = FooAction;

    const SCOPE_NAME: &'static str = "ordered_foo";

    fn defaults() -> Bindings<Self::Actions> { FooPane::defaults() }

    fn dispatcher() -> fn(Self::Actions, &mut TestApp) { dispatch_noop }
}

struct OrderedBarPane;

impl Pane<TestApp> for OrderedBarPane {
    const APP_PANE_ID: TestPaneId = TestPaneId::Bar;

    fn tab_stop() -> TabStop<TestApp> { TabStop::always(ORDERED_BAR_TAB_ORDER) }
}

impl Shortcuts<TestApp> for OrderedBarPane {
    type Actions = FooAction;

    const SCOPE_NAME: &'static str = "ordered_bar";

    fn defaults() -> Bindings<Self::Actions> { FooPane::defaults() }

    fn dispatcher() -> fn(Self::Actions, &mut TestApp) { dispatch_noop }
}

struct OrderedBazPane;

impl Pane<TestApp> for OrderedBazPane {
    const APP_PANE_ID: TestPaneId = TestPaneId::Baz;

    fn tab_stop() -> TabStop<TestApp> { TabStop::always(ORDERED_BAZ_TAB_ORDER) }
}

impl Shortcuts<TestApp> for OrderedBazPane {
    type Actions = FooAction;

    const SCOPE_NAME: &'static str = "ordered_baz";

    fn defaults() -> Bindings<Self::Actions> { FooPane::defaults() }

    fn dispatcher() -> fn(Self::Actions, &mut TestApp) { dispatch_noop }
}

struct ExcludedPane;

impl Pane<TestApp> for ExcludedPane {
    const APP_PANE_ID: TestPaneId = TestPaneId::Excluded;

    fn tab_stop() -> TabStop<TestApp> { TabStop::never() }
}

impl Shortcuts<TestApp> for ExcludedPane {
    type Actions = FooAction;

    const SCOPE_NAME: &'static str = "excluded";

    fn defaults() -> Bindings<Self::Actions> { FooPane::defaults() }

    fn dispatcher() -> fn(Self::Actions, &mut TestApp) { dispatch_noop }
}

struct HiddenPane;

impl Pane<TestApp> for HiddenPane {
    const APP_PANE_ID: TestPaneId = TestPaneId::Hidden;

    fn tab_stop() -> TabStop<TestApp> { TabStop::ordered(HIDDEN_TAB_ORDER, never_tabbable) }
}

impl Shortcuts<TestApp> for HiddenPane {
    type Actions = FooAction;

    const SCOPE_NAME: &'static str = "hidden";

    fn defaults() -> Bindings<Self::Actions> { FooPane::defaults() }

    fn dispatcher() -> fn(Self::Actions, &mut TestApp) { dispatch_noop }
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

fn fresh_builder_singletons() -> super::KeymapBuilder<TestApp, super::Configuring> {
    Keymap::<TestApp>::builder()
        .register_navigation::<AppNav>()
        .expect("nav register must succeed")
        .register_globals::<AppGlobals>()
        .expect("globals register must succeed")
}

#[test]
fn empty_builder_produces_empty_keymap() {
    let keymap = Keymap::<TestApp>::builder()
        .build()
        .expect("empty build must succeed");
    let mut app = fresh_app();
    assert_eq!(
        keymap.dispatch_app_pane(TestPaneId::Foo, &KeyCode::Enter.into(), &mut app),
        KeyOutcome::Unhandled,
    );
    assert!(keymap.config_path().is_none());
}

#[test]
fn register_inserts_scope_under_app_pane_id() {
    let keymap = fresh_builder_singletons()
        .register::<FooPane>(FooPane)
        .build()
        .expect("build must succeed");
    let mut app = fresh_app();
    let outcome = keymap.dispatch_app_pane(TestPaneId::Foo, &KeyCode::Enter.into(), &mut app);
    assert_eq!(outcome, KeyOutcome::Consumed);
    let other = keymap.dispatch_app_pane(TestPaneId::Bar, &KeyCode::Enter.into(), &mut app);
    assert_eq!(other, KeyOutcome::Unhandled);
}

#[test]
fn config_path_round_trips() {
    let path = std::path::PathBuf::from("/tmp/keymap.toml");
    let keymap = fresh_builder_singletons()
        .config_path(path.clone())
        .register::<FooPane>(FooPane)
        .build()
        .expect("build must succeed");
    assert_eq!(keymap.config_path(), Some(path.as_path()));
}

#[test]
fn registered_scope_dispatches_keys_through_keymap() {
    let keymap = fresh_builder_singletons()
        .register::<FooPane>(FooPane)
        .build()
        .expect("build must succeed");
    let mut app = fresh_app();
    let outcome = keymap.dispatch_app_pane(TestPaneId::Foo, &KeyCode::Enter.into(), &mut app);
    assert_eq!(outcome, KeyOutcome::Consumed);
}

#[test]
fn navigation_missing_when_panes_registered_without_nav() {
    let err = Keymap::<TestApp>::builder()
        .register_globals::<AppGlobals>()
        .expect("globals register must succeed")
        .register::<FooPane>(FooPane)
        .build()
        .expect_err("navigation missing must surface");
    assert!(matches!(err, KeymapError::NavigationMissing));
}

#[test]
fn globals_missing_when_panes_registered_without_globals() {
    let err = Keymap::<TestApp>::builder()
        .register_navigation::<AppNav>()
        .expect("nav register must succeed")
        .register::<FooPane>(FooPane)
        .build()
        .expect_err("globals missing must surface");
    assert!(matches!(err, KeymapError::GlobalsMissing));
}

#[test]
fn duplicate_scope_surfaces_from_build() {
    struct OtherFoo;
    impl Pane<TestApp> for OtherFoo {
        const APP_PANE_ID: TestPaneId = TestPaneId::Foo;
    }
    impl Shortcuts<TestApp> for OtherFoo {
        type Actions = FooAction;
        const SCOPE_NAME: &'static str = "other_foo";
        fn defaults() -> Bindings<Self::Actions> { FooPane::defaults() }
        fn dispatcher() -> fn(Self::Actions, &mut TestApp) { FooPane::dispatcher() }
    }

    let err = fresh_builder_singletons()
        .register::<FooPane>(FooPane)
        .register::<OtherFoo>(OtherFoo)
        .build()
        .expect_err("duplicate must surface");
    let KeymapError::DuplicateScope { type_name } = err else {
        panic!("expected DuplicateScope, got {err:?}");
    };
    assert!(
        type_name.contains("OtherFoo"),
        "type_name should name the offender, got: {type_name}",
    );
}

#[test]
fn on_quit_hook_fires_on_global_action_quit() {
    fn bump_quits(ctx: &mut TestApp) { ctx.quits += 1; }
    let keymap = fresh_builder_singletons()
        .on_quit(bump_quits)
        .register::<FooPane>(FooPane)
        .build()
        .expect("build must succeed");
    let mut app = fresh_app();
    keymap.dispatch_framework_global(GlobalAction::Quit, &mut app);
    assert!(app.framework().quit_requested());
    assert_eq!(app.quits, 1);
}

#[test]
fn on_restart_hook_fires_on_global_action_restart() {
    fn bump_restarts(ctx: &mut TestApp) { ctx.restarts += 1; }
    let keymap = fresh_builder_singletons()
        .on_restart(bump_restarts)
        .register::<FooPane>(FooPane)
        .build()
        .expect("build must succeed");
    let mut app = fresh_app();
    keymap.dispatch_framework_global(GlobalAction::Restart, &mut app);
    assert!(app.framework().restart_requested());
    assert_eq!(app.restarts, 1);
}

#[test]
fn dismiss_fallback_fires_on_global_action_dismiss() {
    fn handle_dismiss(ctx: &mut TestApp) -> bool {
        ctx.dismisses += 1;
        true
    }
    let keymap = fresh_builder_singletons()
        .dismiss_fallback(handle_dismiss)
        .register::<FooPane>(FooPane)
        .build()
        .expect("build must succeed");
    let mut app = fresh_app();
    keymap.dispatch_framework_global(GlobalAction::Dismiss, &mut app);
    assert_eq!(app.dismisses, 1);
}

#[test]
fn vim_mode_appends_hjkl_to_navigation() {
    let keymap = Keymap::<TestApp>::builder()
        .vim_mode(VimMode::Enabled)
        .register_navigation::<AppNav>()
        .expect("nav register must succeed")
        .register_globals::<AppGlobals>()
        .expect("globals register must succeed")
        .register::<FooPane>(FooPane)
        .build()
        .expect("build must succeed");
    let nav = keymap
        .navigation::<AppNav>()
        .expect("nav must be registered");
    assert_eq!(nav.action_for(&KeyBind::from('h')), Some(NavAction::Left));
    assert_eq!(nav.action_for(&KeyBind::from('j')), Some(NavAction::Down));
    assert_eq!(nav.action_for(&KeyBind::from('k')), Some(NavAction::Up));
    assert_eq!(nav.action_for(&KeyBind::from('l')), Some(NavAction::Right));
}

#[test]
fn vim_mode_preserves_arrow_primaries_for_navigation() {
    let keymap = Keymap::<TestApp>::builder()
        .vim_mode(VimMode::Enabled)
        .register_navigation::<AppNav>()
        .expect("nav register must succeed")
        .register_globals::<AppGlobals>()
        .expect("globals register must succeed")
        .register::<FooPane>(FooPane)
        .build()
        .expect("build must succeed");
    let nav = keymap
        .navigation::<AppNav>()
        .expect("nav must be registered");
    assert_eq!(
        nav.key_for(NavAction::Up),
        Some(&KeyBind::from(KeyCode::Up))
    );
}

#[test]
fn build_into_populates_framework_pane_metadata() {
    let mut framework = Framework::<TestApp>::new(FocusedPane::App(TestPaneId::Foo));
    let _keymap = fresh_builder_singletons()
        .register::<FooPane>(FooPane)
        .build_into(&mut framework)
        .expect("build_into must succeed");
    let app = TestApp {
        framework,
        quits: 0,
        restarts: 0,
        dismisses: 0,
    };
    assert!(app.framework().focused_pane_mode(&app).is_some());
}

#[test]
fn register_chains_in_registering_state() {
    struct OtherPane;
    impl Pane<TestApp> for OtherPane {
        const APP_PANE_ID: TestPaneId = TestPaneId::Bar;
    }
    impl Shortcuts<TestApp> for OtherPane {
        type Actions = FooAction;
        const SCOPE_NAME: &'static str = "other";
        fn defaults() -> Bindings<Self::Actions> { FooPane::defaults() }
        fn dispatcher() -> fn(Self::Actions, &mut TestApp) { FooPane::dispatcher() }
    }

    let keymap = fresh_builder_singletons()
        .register::<FooPane>(FooPane)
        .register::<OtherPane>(OtherPane)
        .build()
        .expect("build must succeed");
    let mut app = fresh_app();
    assert_eq!(
        keymap.dispatch_app_pane(TestPaneId::Foo, &KeyCode::Enter.into(), &mut app),
        KeyOutcome::Consumed,
    );
    assert_eq!(
        keymap.dispatch_app_pane(TestPaneId::Bar, &KeyCode::Enter.into(), &mut app),
        KeyOutcome::Consumed,
    );
}

#[test]
fn toml_overlay_replaces_pane_action_keys() {
    let dir = std::env::temp_dir();
    let path = dir.join(format!("tui_pane_test_{}.toml", std::process::id()));
    std::fs::write(&path, "[foo]\nactivate = \"x\"\n").expect("write toml");
    let keymap = Keymap::<TestApp>::builder()
        .load_toml(path.clone())
        .expect("load_toml must succeed")
        .register_navigation::<AppNav>()
        .expect("nav register must succeed")
        .register_globals::<AppGlobals>()
        .expect("globals register must succeed")
        .register::<FooPane>(FooPane)
        .build()
        .expect("build must succeed");
    let mut app = fresh_app();
    assert_eq!(
        keymap.dispatch_app_pane(TestPaneId::Foo, &KeyBind::from('x'), &mut app),
        KeyOutcome::Consumed,
    );
    assert_eq!(
        keymap.dispatch_app_pane(TestPaneId::Foo, &KeyCode::Enter.into(), &mut app),
        KeyOutcome::Unhandled,
    );
    let _ = std::fs::remove_file(&path);
}

#[test]
fn toml_overlay_array_form_binds_multiple_keys() {
    let dir = std::env::temp_dir();
    let path = dir.join(format!("tui_pane_test_array_{}.toml", std::process::id()));
    std::fs::write(&path, "[foo]\nactivate = [\"x\", \"y\"]\n").expect("write toml");
    let keymap = Keymap::<TestApp>::builder()
        .load_toml(path.clone())
        .expect("load_toml must succeed")
        .register_navigation::<AppNav>()
        .expect("nav register must succeed")
        .register_globals::<AppGlobals>()
        .expect("globals register must succeed")
        .register::<FooPane>(FooPane)
        .build()
        .expect("build must succeed");
    let mut app = fresh_app();
    assert_eq!(
        keymap.dispatch_app_pane(TestPaneId::Foo, &KeyBind::from('x'), &mut app),
        KeyOutcome::Consumed,
    );
    assert_eq!(
        keymap.dispatch_app_pane(TestPaneId::Foo, &KeyBind::from('y'), &mut app),
        KeyOutcome::Consumed,
    );
    let _ = std::fs::remove_file(&path);
}

#[test]
fn toml_overlay_array_in_array_duplicate_rejected_at_build() {
    // Cross-action collision in the [foo] table — the same key
    // `x` is bound twice in the array for the same action.
    let dir = std::env::temp_dir();
    let path = dir.join(format!("tui_pane_test_dup_{}.toml", std::process::id()));
    std::fs::write(&path, "[foo]\nactivate = [\"x\", \"x\"]\n").expect("write toml");
    let result = Keymap::<TestApp>::builder()
        .load_toml(path.clone())
        .expect("load_toml must succeed")
        .register_navigation::<AppNav>()
        .expect("nav register must succeed")
        .register_globals::<AppGlobals>()
        .expect("globals register must succeed")
        .register::<FooPane>(FooPane)
        .build();
    assert!(matches!(result, Err(KeymapError::InArrayDuplicate { .. })));
    let _ = std::fs::remove_file(&path);
}

#[test]
fn toml_unknown_scope_surfaces_at_build() {
    let dir = std::env::temp_dir();
    let path = dir.join(format!("tui_pane_test_uscope_{}.toml", std::process::id()));
    std::fs::write(&path, "[mystery]\nactivate = \"x\"\n").expect("write toml");
    let result = Keymap::<TestApp>::builder()
        .load_toml(path.clone())
        .expect("load_toml must succeed")
        .register_navigation::<AppNav>()
        .expect("nav register must succeed")
        .register_globals::<AppGlobals>()
        .expect("globals register must succeed")
        .register::<FooPane>(FooPane)
        .build();
    assert!(matches!(result, Err(KeymapError::UnknownScope { .. })));
    let _ = std::fs::remove_file(&path);
}

#[test]
fn vim_mode_treats_shift_letter_as_distinct_from_bare_letter() {
    struct ShiftKNav;
    impl Navigation<TestApp> for ShiftKNav {
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
                KeyBind::shift('K') => NavAction::Right,
            }
        }

        fn dispatcher() -> fn(Self::Actions, FocusedPane<TestPaneId>, &mut TestApp) {
            |_action, _focused, _ctx| { /* no-op */ }
        }
    }

    let keymap = Keymap::<TestApp>::builder()
        .vim_mode(VimMode::Enabled)
        .register_navigation::<ShiftKNav>()
        .expect("nav register must succeed")
        .register_globals::<AppGlobals>()
        .expect("globals register must succeed")
        .register::<FooPane>(FooPane)
        .build()
        .expect("build must succeed");

    let nav = keymap
        .navigation::<ShiftKNav>()
        .expect("nav must be registered");
    assert_eq!(nav.action_for(&KeyBind::from('k')), Some(NavAction::Up));
    assert_eq!(
        nav.action_for(&KeyBind::shift('K')),
        Some(NavAction::Right),
        "Shift+K still binds the original action — vim's bare k is distinct on (code, mods)",
    );
}

#[test]
fn cross_action_collision_in_toml_surfaces_at_build() {
    let dir = std::env::temp_dir();
    let path = dir.join(format!("tui_pane_test_xcoll_{}.toml", std::process::id()));
    std::fs::write(&path, "[navigation]\nup = \"x\"\ndown = \"x\"\n").expect("write toml");
    let result = Keymap::<TestApp>::builder()
        .load_toml(path.clone())
        .expect("load_toml must succeed")
        .register_navigation::<AppNav>();
    let _ = std::fs::remove_file(&path);
    match result {
        Err(KeymapError::CrossActionCollision { .. }) => {},
        Err(other) => panic!("expected CrossActionCollision, got {other:?}"),
        Ok(_) => panic!("expected CrossActionCollision, got Ok"),
    }
}

#[test]
fn global_toml_overlay_overrides_framework_globals() {
    let dir = std::env::temp_dir();
    let path = dir.join(format!("tui_pane_test_global_{}.toml", std::process::id()));
    std::fs::write(&path, "[global]\nquit = \"z\"\n").expect("write toml");
    let keymap = Keymap::<TestApp>::builder()
        .load_toml(path.clone())
        .expect("load_toml must succeed")
        .build()
        .expect("build must succeed");
    let _ = std::fs::remove_file(&path);

    assert_eq!(
        keymap.framework_globals().action_for(&KeyBind::from('z')),
        Some(GlobalAction::Quit),
    );
    assert_eq!(
        keymap.framework_globals().action_for(&KeyBind::from('q')),
        None,
        "default 'q' must be replaced by the user override",
    );
}

#[test]
fn shared_global_table_applies_framework_and_app_keys() {
    let dir = std::env::temp_dir();
    let path = dir.join(format!(
        "tui_pane_test_shared_global_{}.toml",
        process::id()
    ));
    fs::write(
        &path,
        "[global]\nquit = \"z\"\nsettings = \"F2\"\nfind = \"?\"\n",
    )
    .expect("write toml");
    let keymap = Keymap::<TestApp>::builder()
        .load_toml(path.clone())
        .expect("load_toml must succeed")
        .register_globals::<AppGlobals>()
        .expect("app globals must skip framework-owned keys")
        .build()
        .expect("framework globals must skip app-owned keys");
    let _ = std::fs::remove_file(&path);

    assert_eq!(
        keymap.framework_globals().action_for(&KeyBind::from('z')),
        Some(GlobalAction::Quit),
    );
    assert_eq!(
        keymap
            .framework_globals()
            .action_for(&KeyBind::from(KeyCode::F(2))),
        Some(GlobalAction::OpenSettings),
    );
    let app_globals = keymap
        .globals::<AppGlobals>()
        .expect("app globals must be registered");
    assert_eq!(
        app_globals.action_for(&KeyBind::from('?')),
        Some(AppGlobalAction::Find),
    );
}

#[test]
fn shared_global_table_still_rejects_truly_unknown_actions() {
    let dir = std::env::temp_dir();
    let path = dir.join(format!(
        "tui_pane_test_shared_global_unknown_{}.toml",
        std::process::id()
    ));
    std::fs::write(&path, "[global]\nbogus_action = \"z\"\n").expect("write toml");
    let result = Keymap::<TestApp>::builder()
        .load_toml(path.clone())
        .expect("load_toml must succeed")
        .register_globals::<AppGlobals>();
    let _ = std::fs::remove_file(&path);

    assert!(
        matches!(result, Err(KeymapError::UnknownAction { .. })),
        "truly unknown shared-global action must still error",
    );
}

#[test]
fn app_global_key_errors_without_registered_app_globals_peer() {
    let dir = std::env::temp_dir();
    let path = dir.join(format!(
        "tui_pane_test_global_no_peer_{}.toml",
        std::process::id()
    ));
    std::fs::write(&path, "[global]\nfind = \"?\"\n").expect("write toml");
    let result = Keymap::<TestApp>::builder()
        .load_toml(path.clone())
        .expect("load_toml must succeed")
        .build();
    let _ = std::fs::remove_file(&path);

    assert!(
        matches!(result, Err(KeymapError::UnknownAction { .. })),
        "framework globals stay strict when no app-globals peer is registered",
    );
}

#[test]
fn settings_overlay_toml_rebinds_registered_scope() {
    let dir = std::env::temp_dir();
    let path = dir.join(format!(
        "tui_pane_test_settings_overlay_{}.toml",
        std::process::id()
    ));
    std::fs::write(&path, "[settings]\nstart_edit = \"F2\"\n").expect("write toml");
    let keymap = Keymap::<TestApp>::builder()
        .load_toml(path.clone())
        .expect("load_toml must succeed")
        .register_settings_overlay()
        .expect("settings overlay must register")
        .build()
        .expect("build must succeed");
    let _ = std::fs::remove_file(&path);

    assert_eq!(
        keymap
            .settings_overlay()
            .action_for(&KeyBind::from(KeyCode::F(2))),
        Some(SettingsPaneAction::StartEdit),
    );
    assert_eq!(
        keymap
            .settings_overlay()
            .action_for(&KeyBind::from(KeyCode::Enter)),
        None,
        "TOML replaces the action's default binding",
    );
}

#[test]
fn keymap_overlay_toml_rebinds_registered_scope() {
    let dir = std::env::temp_dir();
    let path = dir.join(format!(
        "tui_pane_test_keymap_overlay_{}.toml",
        std::process::id()
    ));
    std::fs::write(&path, "[keymap]\ncancel = \"F3\"\n").expect("write toml");
    let keymap = Keymap::<TestApp>::builder()
        .load_toml(path.clone())
        .expect("load_toml must succeed")
        .register_keymap_overlay()
        .expect("keymap overlay must register")
        .build()
        .expect("build must succeed");
    let _ = std::fs::remove_file(&path);

    assert_eq!(
        keymap
            .keymap_overlay()
            .action_for(&KeyBind::from(KeyCode::F(3))),
        Some(KeymapPaneAction::Cancel),
    );
}

#[test]
fn known_overlay_unknown_action_errors() {
    let dir = std::env::temp_dir();
    let path = dir.join(format!(
        "tui_pane_test_settings_unknown_action_{}.toml",
        std::process::id()
    ));
    std::fs::write(&path, "[settings]\nbogus_action = \"x\"\n").expect("write toml");
    let result = Keymap::<TestApp>::builder()
        .load_toml(path.clone())
        .expect("load_toml must succeed")
        .register_settings_overlay();
    let _ = std::fs::remove_file(&path);

    assert!(matches!(result, Err(KeymapError::UnknownAction { .. })));
}

#[test]
fn unknown_overlay_table_still_errors_when_known_overlays_registered() {
    let dir = std::env::temp_dir();
    let path = dir.join(format!(
        "tui_pane_test_unknown_overlay_scope_{}.toml",
        std::process::id()
    ));
    std::fs::write(&path, "[bogus_overlay]\nfoo = \"x\"\n").expect("write toml");
    let result = Keymap::<TestApp>::builder()
        .load_toml(path.clone())
        .expect("load_toml must succeed")
        .register_settings_overlay()
        .expect("settings overlay must register")
        .register_keymap_overlay()
        .expect("keymap overlay must register")
        .build();
    let _ = std::fs::remove_file(&path);

    assert!(matches!(result, Err(KeymapError::UnknownScope { .. })));
}

#[test]
fn next_pane_and_prev_pane_walk_registered_panes_with_wrap() {
    let mut framework = Framework::<TestApp>::new(FocusedPane::App(TestPaneId::Foo));
    let keymap = fresh_builder_singletons()
        .register::<FooPane>(FooPane)
        .register::<BarPane>(BarPane)
        .build_into(&mut framework)
        .expect("build_into must succeed");
    let mut app = TestApp {
        framework,
        quits: 0,
        restarts: 0,
        dismisses: 0,
    };

    keymap.dispatch_framework_global(GlobalAction::NextPane, &mut app);
    assert_eq!(
        app.framework().focused(),
        &FocusedPane::App(TestPaneId::Bar),
    );
    keymap.dispatch_framework_global(GlobalAction::NextPane, &mut app);
    assert_eq!(
        app.framework().focused(),
        &FocusedPane::App(TestPaneId::Foo),
        "next-pane wraps from the last pane to the first",
    );

    keymap.dispatch_framework_global(GlobalAction::PrevPane, &mut app);
    assert_eq!(
        app.framework().focused(),
        &FocusedPane::App(TestPaneId::Bar),
        "prev-pane wraps from the first pane to the last",
    );
    keymap.dispatch_framework_global(GlobalAction::PrevPane, &mut app);
    assert_eq!(
        app.framework().focused(),
        &FocusedPane::App(TestPaneId::Foo),
    );
}

#[test]
fn explicit_tab_stops_drive_next_prev_order() {
    let mut framework = Framework::<TestApp>::new(FocusedPane::App(TestPaneId::Bar));
    let keymap = fresh_builder_singletons()
        .register::<OrderedFooPane>(OrderedFooPane)
        .register::<OrderedBazPane>(OrderedBazPane)
        .register::<OrderedBarPane>(OrderedBarPane)
        .build_into(&mut framework)
        .expect("build_into must succeed");
    let mut app = TestApp {
        framework,
        quits: 0,
        restarts: 0,
        dismisses: 0,
    };

    keymap.dispatch_framework_global(GlobalAction::NextPane, &mut app);
    assert_eq!(
        app.framework().focused(),
        &FocusedPane::App(TestPaneId::Foo),
        "explicit order must beat registration order",
    );
    keymap.dispatch_framework_global(GlobalAction::NextPane, &mut app);
    assert_eq!(
        app.framework().focused(),
        &FocusedPane::App(TestPaneId::Baz),
    );
    keymap.dispatch_framework_global(GlobalAction::PrevPane, &mut app);
    assert_eq!(
        app.framework().focused(),
        &FocusedPane::App(TestPaneId::Foo),
    );
}

#[test]
fn never_and_false_predicate_panes_are_skipped() {
    let mut framework = Framework::<TestApp>::new(FocusedPane::App(TestPaneId::Foo));
    let keymap = fresh_builder_singletons()
        .register::<FooPane>(FooPane)
        .register::<ExcludedPane>(ExcludedPane)
        .register::<HiddenPane>(HiddenPane)
        .register::<BarPane>(BarPane)
        .build_into(&mut framework)
        .expect("build_into must succeed");
    let mut app = TestApp {
        framework,
        quits: 0,
        restarts: 0,
        dismisses: 0,
    };

    keymap.dispatch_framework_global(GlobalAction::NextPane, &mut app);
    assert_eq!(
        app.framework().focused(),
        &FocusedPane::App(TestPaneId::Bar),
    );
    keymap.dispatch_framework_global(GlobalAction::PrevPane, &mut app);
    assert_eq!(
        app.framework().focused(),
        &FocusedPane::App(TestPaneId::Foo),
    );
}

#[test]
fn stale_focus_next_uses_first_live_tab_stop() {
    let mut framework = Framework::<TestApp>::new(FocusedPane::App(TestPaneId::Hidden));
    let keymap = fresh_builder_singletons()
        .register::<FooPane>(FooPane)
        .register::<BarPane>(BarPane)
        .build_into(&mut framework)
        .expect("build_into must succeed");
    let mut app = TestApp {
        framework,
        quits: 0,
        restarts: 0,
        dismisses: 0,
    };

    keymap.dispatch_framework_global(GlobalAction::NextPane, &mut app);
    assert_eq!(
        app.framework().focused(),
        &FocusedPane::App(TestPaneId::Foo),
    );
}

#[test]
fn stale_focus_prev_uses_last_live_tab_stop() {
    let mut framework = Framework::<TestApp>::new(FocusedPane::App(TestPaneId::Hidden));
    let keymap = fresh_builder_singletons()
        .register::<FooPane>(FooPane)
        .register::<BarPane>(BarPane)
        .build_into(&mut framework)
        .expect("build_into must succeed");
    let mut app = TestApp {
        framework,
        quits: 0,
        restarts: 0,
        dismisses: 0,
    };

    keymap.dispatch_framework_global(GlobalAction::PrevPane, &mut app);
    assert_eq!(
        app.framework().focused(),
        &FocusedPane::App(TestPaneId::Bar),
    );
}

#[test]
fn dismissed_toast_reconciles_to_first_live_app_tab_stop() {
    let mut framework = Framework::<TestApp>::new(FocusedPane::Framework(FrameworkFocusId::Toasts));
    framework.toasts.push("one", "body");
    let keymap = fresh_builder_singletons()
        .register::<OrderedFooPane>(OrderedFooPane)
        .register::<OrderedBarPane>(OrderedBarPane)
        .build_into(&mut framework)
        .expect("build_into must succeed");
    let mut app = TestApp {
        framework,
        quits: 0,
        restarts: 0,
        dismisses: 0,
    };

    keymap.dispatch_framework_global(GlobalAction::Dismiss, &mut app);
    assert_eq!(
        app.framework().focused(),
        &FocusedPane::App(TestPaneId::Bar),
        "empty toast focus must reconcile to the first live app tab stop",
    );
}

#[test]
fn active_toasts_append_after_app_tab_stops_and_reset_on_entry() {
    let mut framework = Framework::<TestApp>::new(FocusedPane::App(TestPaneId::Foo));
    let first = framework.toasts.push("one", "body");
    let second = framework.toasts.push("two", "body");
    let keymap = fresh_builder_singletons()
        .register::<OrderedBarPane>(OrderedBarPane)
        .register::<OrderedFooPane>(OrderedFooPane)
        .build_into(&mut framework)
        .expect("build_into must succeed");
    let mut app = TestApp {
        framework,
        quits: 0,
        restarts: 0,
        dismisses: 0,
    };

    keymap.dispatch_framework_global(GlobalAction::NextPane, &mut app);
    assert_eq!(
        app.framework().focused(),
        &FocusedPane::Framework(FrameworkFocusId::Toasts),
    );
    assert_eq!(app.framework().toasts.focused_id(), Some(first));

    app.set_focus(FocusedPane::App(TestPaneId::Bar));
    keymap.dispatch_framework_global(GlobalAction::PrevPane, &mut app);
    assert_eq!(
        app.framework().focused(),
        &FocusedPane::Framework(FrameworkFocusId::Toasts),
    );
    assert_eq!(app.framework().toasts.focused_id(), Some(second));
}

#[test]
fn focused_toasts_scroll_before_advancing_cycle() {
    let mut framework = Framework::<TestApp>::new(FocusedPane::Framework(FrameworkFocusId::Toasts));
    let first = framework.toasts.push("one", "body");
    let second = framework.toasts.push("two", "body");
    let keymap = fresh_builder_singletons()
        .register::<FooPane>(FooPane)
        .register::<BarPane>(BarPane)
        .build_into(&mut framework)
        .expect("build_into must succeed");
    let mut app = TestApp {
        framework,
        quits: 0,
        restarts: 0,
        dismisses: 0,
    };

    assert_eq!(app.framework().toasts.focused_id(), Some(first));
    keymap.dispatch_framework_global(GlobalAction::NextPane, &mut app);
    assert_eq!(
        app.framework().focused(),
        &FocusedPane::Framework(FrameworkFocusId::Toasts),
    );
    assert_eq!(app.framework().toasts.focused_id(), Some(second));

    keymap.dispatch_framework_global(GlobalAction::NextPane, &mut app);
    assert_eq!(
        app.framework().focused(),
        &FocusedPane::App(TestPaneId::Foo),
        "NextPane advances out of Toasts after the last toast",
    );

    app.set_focus(FocusedPane::Framework(FrameworkFocusId::Toasts));
    app.framework_mut().toasts.reset_to_last();
    keymap.dispatch_framework_global(GlobalAction::PrevPane, &mut app);
    assert_eq!(
        app.framework().focused(),
        &FocusedPane::Framework(FrameworkFocusId::Toasts),
    );
    assert_eq!(app.framework().toasts.focused_id(), Some(first));
}

#[test]
fn open_keymap_and_open_settings_open_framework_overlays() {
    let keymap = Keymap::<TestApp>::builder()
        .build()
        .expect("empty build must succeed");
    let mut app = fresh_app();
    let initial_focus = *app.framework().focused();

    keymap.dispatch_framework_global(GlobalAction::OpenKeymap, &mut app);
    assert_eq!(app.framework().overlay(), Some(FrameworkOverlayId::Keymap));
    assert_eq!(*app.framework().focused(), initial_focus);

    keymap.dispatch_framework_global(GlobalAction::OpenSettings, &mut app);
    assert_eq!(
        app.framework().overlay(),
        Some(FrameworkOverlayId::Settings)
    );
    assert_eq!(*app.framework().focused(), initial_focus);

    keymap.dispatch_framework_global(GlobalAction::Dismiss, &mut app);
    assert_eq!(app.framework().overlay(), None);
    assert_eq!(*app.framework().focused(), initial_focus);
}

#[test]
fn invalid_binding_in_toml_surfaces_at_build() {
    let dir = std::env::temp_dir();
    let path = dir.join(format!("tui_pane_test_bad_{}.toml", std::process::id()));
    std::fs::write(&path, "[foo]\nactivate = \"Bogus+nonsense\"\n").expect("write toml");
    let result = Keymap::<TestApp>::builder()
        .load_toml(path.clone())
        .expect("load_toml must succeed")
        .register_navigation::<AppNav>()
        .expect("nav register must succeed")
        .register_globals::<AppGlobals>()
        .expect("globals register must succeed")
        .register::<FooPane>(FooPane)
        .build();
    let _ = std::fs::remove_file(&path);
    let err = result.expect_err("invalid binding must surface");
    assert!(
        matches!(err, KeymapError::InvalidBinding { .. }),
        "expected InvalidBinding, got {err:?}",
    );
}

#[test]
fn unknown_action_in_toml_surfaces_at_build() {
    let dir = std::env::temp_dir();
    let path = dir.join(format!("tui_pane_test_uact_{}.toml", std::process::id()));
    std::fs::write(&path, "[foo]\nfrobnicate = \"x\"\n").expect("write toml");
    let result = Keymap::<TestApp>::builder()
        .load_toml(path.clone())
        .expect("load_toml must succeed")
        .register_navigation::<AppNav>()
        .expect("nav register must succeed")
        .register_globals::<AppGlobals>()
        .expect("globals register must succeed")
        .register::<FooPane>(FooPane)
        .build();
    let _ = std::fs::remove_file(&path);
    let err = result.expect_err("unknown action must surface");
    assert!(
        matches!(err, KeymapError::UnknownAction { .. }),
        "expected UnknownAction, got {err:?}",
    );
}

#[test]
fn load_toml_missing_file_treated_as_no_overlay() {
    let path = std::env::temp_dir().join("tui_pane_does_not_exist.toml");
    let _ = std::fs::remove_file(&path);
    let keymap = Keymap::<TestApp>::builder()
        .load_toml(path)
        .expect("missing file must yield Ok")
        .register_navigation::<AppNav>()
        .expect("nav register must succeed")
        .register_globals::<AppGlobals>()
        .expect("globals register must succeed")
        .register::<FooPane>(FooPane)
        .build()
        .expect("build must succeed");
    let mut app = fresh_app();
    assert_eq!(
        keymap.dispatch_app_pane(TestPaneId::Foo, &KeyCode::Enter.into(), &mut app),
        KeyOutcome::Consumed,
    );
}
