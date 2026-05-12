//! Keymap: types and traits for binding keys to actions.

mod action_enum;
mod bindings;
mod builder;
mod global_action;
mod globals;
mod key_bind;
mod key_outcome;
mod load;
mod navigation;
mod runtime_scope;
mod scope_map;
mod shortcuts;
mod vim;

use core::fmt::Formatter;
use std::any::Any;
use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;

pub use action_enum::Action;
pub use bindings::Bindings;
pub use builder::Configuring;
pub use builder::KeymapBuilder;
#[allow(
    unused_imports,
    reason = "re-exported at crate root for callers naming the typestate"
)]
pub use builder::Registering;
pub use global_action::GlobalAction;
pub use globals::Globals;
pub use key_bind::KeyBind;
pub use key_bind::KeyInput;
pub use key_bind::KeyParseError;
pub use key_outcome::KeyOutcome;
pub use load::KeymapError;
pub use navigation::Navigation;
pub use runtime_scope::RenderedSlot;
pub use scope_map::ScopeMap;
pub use shortcuts::Shortcuts;
pub use vim::VimMode;

use self::runtime_scope::RuntimeScope;
use crate::AppContext;
use crate::BarRegion;
use crate::KeymapPane;
use crate::KeymapPaneAction;
use crate::SettingsPane;
use crate::SettingsPaneAction;
use crate::SettingsRegistry;
use crate::framework;

/// `<N>`/`<G>`-monomorphized renderer the bar reads to materialize
/// bar slots without naming the action enum. Mirror of
/// [`builder::ScopeRenderFn`](self::builder::ScopeRenderFn) — the
/// keymap and the builder both store one of these per scope.
type ScopeRenderFn<Ctx> = fn(&Keymap<Ctx>) -> Vec<RenderedSlot>;

/// The keymap container: anchor for every binding the framework
/// resolves at runtime.
///
/// Built with [`Self::builder`], which returns a
/// [`KeymapBuilder<Ctx, Configuring>`]. Settings phase first
/// (`config_path`, plus Phase 10's `load_toml` / `vim_mode`); on the
/// first [`KeymapBuilder::register`] call the type transitions to
/// [`KeymapBuilder<Ctx, Registering>`] and settings methods drop off
/// the type.
///
/// One [`Ctx::AppPaneId`](AppContext::AppPaneId)-keyed scope per
/// registered pane. Public callers reach pane operations through
/// [`Self::dispatch_app_pane`], [`Self::render_app_pane_bar_slots`],
/// [`Self::key_for_toml_key`], and [`Self::is_key_bound_to_toml_key`] — the underlying
/// [`RuntimeScope`](self::runtime_scope::RuntimeScope) trait is
/// crate-private.
///
/// Framework panes are not stored in this map — they are special-cased
/// by [`FocusedPane::Framework`](crate::FocusedPane::Framework) arms in
/// callers (see Phase 11).
pub struct Keymap<Ctx: AppContext + 'static> {
    scopes:                HashMap<Ctx::AppPaneId, Box<dyn RuntimeScope<Ctx>>>,
    navigation:            Option<Box<dyn Any>>,
    /// Monomorphized renderer for the navigation scope. Each
    /// [`KeymapBuilder::register_navigation::<N>`](crate::KeymapBuilder::register_navigation)
    /// call sets this to the `N`-specialized free fn in
    /// [`runtime_scope::render_navigation_slots`]. The bar renderer
    /// reads it via [`Self::render_navigation_slots`] without naming
    /// `N`.
    navigation_render_fn:  Option<ScopeRenderFn<Ctx>>,
    globals:               Option<Box<dyn Any>>,
    /// Monomorphized renderer for the app-globals scope. See
    /// [`Self::navigation_render_fn`].
    app_globals_render_fn: Option<ScopeRenderFn<Ctx>>,
    framework_globals:     ScopeMap<GlobalAction>,
    settings_overlay:      ScopeMap<SettingsPaneAction>,
    overlay_keymap_scope:  ScopeMap<KeymapPaneAction>,
    settings:              Option<SettingsRegistry<Ctx>>,
    on_quit:               Option<fn(&mut Ctx)>,
    on_restart:            Option<fn(&mut Ctx)>,
    dismiss_fallback:      Option<fn(&mut Ctx) -> bool>,
    config_path:           Option<PathBuf>,
}

impl<Ctx: AppContext + 'static> Keymap<Ctx> {
    /// Canonical entry point for assembling a keymap. Returns the
    /// builder in [`Configuring`] state — settings methods first,
    /// then [`KeymapBuilder::register`] transitions to [`Registering`].
    #[must_use]
    pub fn builder() -> KeymapBuilder<Ctx, Configuring> { KeymapBuilder::new() }

    /// Empty keymap, no scopes registered. `pub(super)` because only
    /// [`KeymapBuilder::build`] (sibling) constructs one.
    pub(super) fn new(config_path: Option<PathBuf>) -> Self {
        Self {
            scopes: HashMap::new(),
            navigation: None,
            navigation_render_fn: None,
            globals: None,
            app_globals_render_fn: None,
            framework_globals: GlobalAction::defaults().into_scope_map(),
            settings_overlay: SettingsPane::<Ctx>::defaults().into_scope_map(),
            overlay_keymap_scope: KeymapPane::<Ctx>::defaults().into_scope_map(),
            settings: None,
            on_quit: None,
            on_restart: None,
            dismiss_fallback: None,
            config_path,
        }
    }

    /// `pub(super)` because only [`KeymapBuilder::build`] (sibling)
    /// constructs one.
    pub(super) fn set_navigation(&mut self, scope_map: Box<dyn Any>) {
        self.navigation = Some(scope_map);
    }

    /// `pub(super)` because only [`KeymapBuilder::build`] (sibling)
    /// stores it, alongside [`Self::set_navigation`].
    pub(super) const fn set_navigation_render_fn(&mut self, render: ScopeRenderFn<Ctx>) {
        self.navigation_render_fn = Some(render);
    }

    /// `pub(super)` because only [`KeymapBuilder::build`] (sibling)
    /// constructs one.
    pub(super) fn set_globals(&mut self, scope_map: Box<dyn Any>) {
        self.globals = Some(scope_map);
    }

    /// `pub(super)` because only [`KeymapBuilder::build`] (sibling)
    /// stores it, alongside [`Self::set_globals`].
    pub(super) const fn set_app_globals_render_fn(&mut self, render: ScopeRenderFn<Ctx>) {
        self.app_globals_render_fn = Some(render);
    }

    /// `pub(super)` because only [`KeymapBuilder::build`] (sibling)
    /// constructs one.
    pub(super) fn set_framework_globals(&mut self, map: ScopeMap<GlobalAction>) {
        self.framework_globals = map;
    }

    /// `pub(super)` because only [`KeymapBuilder::build`] (sibling)
    /// overlays user TOML onto the framework-owned settings scope.
    pub(super) fn set_settings_overlay(&mut self, map: ScopeMap<SettingsPaneAction>) {
        self.settings_overlay = map;
    }

    /// `pub(super)` because only [`KeymapBuilder::build`] (sibling)
    /// overlays user TOML onto the framework-owned keymap scope.
    pub(super) fn set_keymap_overlay(&mut self, map: ScopeMap<KeymapPaneAction>) {
        self.overlay_keymap_scope = map;
    }

    /// `pub(super)` because only [`KeymapBuilder::build`] (sibling)
    /// constructs one.
    pub(super) fn set_settings(&mut self, settings: SettingsRegistry<Ctx>) {
        self.settings = Some(settings);
    }

    /// `pub(super)` because only [`KeymapBuilder::build`] (sibling)
    /// constructs one.
    pub(super) const fn set_on_quit(&mut self, hook: fn(&mut Ctx)) { self.on_quit = Some(hook); }

    /// `pub(super)` because only [`KeymapBuilder::build`] (sibling)
    /// constructs one.
    pub(super) const fn set_on_restart(&mut self, hook: fn(&mut Ctx)) {
        self.on_restart = Some(hook);
    }

    /// `pub(super)` because only [`KeymapBuilder::build`] (sibling)
    /// constructs one.
    pub(super) const fn set_dismiss_fallback(&mut self, hook: fn(&mut Ctx) -> bool) {
        self.dismiss_fallback = Some(hook);
    }

    /// Insert one scope under its `AppPaneId`. `pub(super)` so only
    /// the builder writes.
    pub(super) fn insert_scope(&mut self, id: Ctx::AppPaneId, scope: Box<dyn RuntimeScope<Ctx>>) {
        self.scopes.insert(id, scope);
    }

    /// Path to the config file the loader read (or would read), if
    /// any. `None` when the binary registered no config path or the
    /// file is missing — the loader treats a missing file as "use
    /// defaults" and returns `Ok`.
    #[must_use]
    pub fn config_path(&self) -> Option<&Path> { self.config_path.as_deref() }

    /// Resolve `bind` to an action in the scope registered for `id`
    /// and call its dispatcher. Returns [`KeyOutcome::Unhandled`] if
    /// no scope is registered for `id` or no binding matches; the
    /// caller continues its dispatch chain (globals, dismiss,
    /// fallback) on `Unhandled`.
    pub fn dispatch_app_pane(
        &self,
        id: Ctx::AppPaneId,
        bind: &KeyBind,
        ctx: &mut Ctx,
    ) -> KeyOutcome {
        self.scopes
            .get(&id)
            .map_or(KeyOutcome::Unhandled, |s| s.dispatch_key(bind, ctx))
    }

    /// Bar slots for the scope registered for `id`, fully resolved
    /// (label / key / state / visibility). Returns an empty `Vec` if
    /// no scope is registered.
    #[must_use]
    pub fn render_app_pane_bar_slots(&self, id: Ctx::AppPaneId, ctx: &Ctx) -> Vec<RenderedSlot> {
        self.scopes
            .get(&id)
            .map_or_else(Vec::new, |s| s.render_bar_slots(ctx))
    }

    /// Reverse lookup: TOML action key string → currently bound
    /// [`KeyBind`] in the scope registered for `id`. Returns `None`
    /// if no scope is registered for `id`, the action name is not
    /// recognized, or the named action has no binding.
    #[must_use]
    pub fn key_for_toml_key(&self, id: Ctx::AppPaneId, action: &str) -> Option<KeyBind> {
        self.scopes.get(&id)?.key_for_toml_key(action)
    }

    /// Reverse lookup: TOML action key string → every currently bound
    /// [`KeyBind`] in the scope registered for `id`. Returns an empty
    /// vector if no scope is registered for `id`, the action name is
    /// not recognized, or the named action has no binding.
    #[must_use]
    pub fn keys_for_toml_key(&self, id: Ctx::AppPaneId, action: &str) -> Vec<KeyBind> {
        self.scopes
            .get(&id)
            .map_or_else(Vec::new, |scope| scope.keys_for_toml_key(action))
    }

    /// Reverse lookup predicate: returns `true` when `bind` is one of
    /// the keys currently bound to the TOML action key string in the
    /// scope registered for `id`.
    ///
    /// Unlike [`Self::key_for_toml_key`], this checks every binding
    /// for the action, not just the primary key rendered in compact
    /// UI. Structural input checks should use this predicate so TOML
    /// arrays like `cancel = ["Esc", "q"]` work for every key.
    #[must_use]
    pub fn is_key_bound_to_toml_key(
        &self,
        id: Ctx::AppPaneId,
        action: &str,
        bind: &KeyBind,
    ) -> bool {
        self.scopes
            .get(&id)
            .is_some_and(|scope| scope.is_key_bound_to_toml_key(action, bind))
    }

    /// The framework-globals scope ([`GlobalAction`] →
    /// [`KeyBind`]). Built from
    /// [`GlobalAction::defaults`](GlobalAction::defaults) plus any
    /// `[global]` overrides at builder time.
    #[must_use]
    pub const fn framework_globals(&self) -> &ScopeMap<GlobalAction> { &self.framework_globals }

    /// Resolved bindings for the framework-owned settings overlay.
    ///
    /// Defaults are always present; calling
    /// [`KeymapBuilder::register_settings_overlay`] additionally
    /// applies user TOML from `[settings]`.
    #[must_use]
    pub const fn settings_overlay(&self) -> &ScopeMap<SettingsPaneAction> { &self.settings_overlay }

    /// Resolved bindings for the framework-owned keymap overlay.
    ///
    /// Defaults are always present; calling
    /// [`KeymapBuilder::register_keymap_overlay`] additionally
    /// applies user TOML from `[keymap]`.
    #[must_use]
    pub const fn keymap_overlay(&self) -> &ScopeMap<KeymapPaneAction> { &self.overlay_keymap_scope }

    /// Bar slots for the navigation scope, fully resolved (label,
    /// key, region tagged [`BarRegion::Nav`](crate::BarRegion::Nav)).
    /// Returns an empty `Vec` when no navigation impl was registered.
    ///
    /// The bar renderer reads this and partitions by region without
    /// naming the binary's `<N>` — the `N`-monomorphized renderer was
    /// stored at [`KeymapBuilder::register_navigation`](crate::KeymapBuilder::register_navigation)
    /// time.
    #[must_use]
    pub fn render_navigation_slots(&self) -> Vec<RenderedSlot> {
        self.navigation_render_fn
            .map_or_else(Vec::new, |render| render(self))
    }

    /// Bar slots for the app-globals scope, fully resolved (label,
    /// key, region tagged [`BarRegion::Global`](crate::BarRegion::Global)).
    /// Returns an empty `Vec` when no globals impl was registered.
    ///
    /// Mirrors [`Self::render_navigation_slots`].
    #[must_use]
    pub fn render_app_globals_slots(&self) -> Vec<RenderedSlot> {
        self.app_globals_render_fn
            .map_or_else(Vec::new, |render| render(self))
    }

    /// Bar slots for the framework-globals scope ([`GlobalAction`]
    /// variants in [`GlobalAction::ALL`] order, region tagged
    /// [`BarRegion::Global`](crate::BarRegion::Global)). Always
    /// available — the framework-globals scope ships fully populated.
    ///
    /// `BarRegion::Global` covers the full strip the bar emits at the
    /// far right; the `bar` renderer is free to split out
    /// [`GlobalAction::NextPane`] / [`GlobalAction::PrevPane`] into
    /// its own pane-cycle slot inside the nav region — that slot is
    /// rendered by direct lookup against
    /// [`Self::framework_globals`], not by filtering this `Vec`.
    #[must_use]
    pub fn render_framework_globals_slots(&self) -> Vec<RenderedSlot> {
        self::runtime_scope::slots_from_scope(
            BarRegion::Global,
            GlobalAction::ALL,
            &self.framework_globals,
        )
    }

    /// Typed singleton getter for the registered [`Navigation`] impl.
    ///
    /// Returns `None` when [`KeymapBuilder::register_navigation`] was
    /// not called, or when the caller asks for a `N` that does not
    /// match the type the builder stored. The builder rejects
    /// missing-navigation builds with [`KeymapError::NavigationMissing`]
    /// for any non-empty pane set, so production callers can rely on
    /// `Some(_)`.
    #[must_use]
    pub fn navigation<N: Navigation<Ctx>>(&self) -> Option<&ScopeMap<N::Actions>> {
        self.navigation
            .as_ref()
            .and_then(|stored| stored.downcast_ref::<ScopeMap<N::Actions>>())
    }

    /// Typed singleton getter for the registered [`Globals`] impl.
    ///
    /// Returns `None` for the same reasons as [`Self::navigation`].
    #[must_use]
    pub fn globals<G: Globals<Ctx>>(&self) -> Option<&ScopeMap<G::Actions>> {
        self.globals
            .as_ref()
            .and_then(|stored| stored.downcast_ref::<ScopeMap<G::Actions>>())
    }

    /// Borrow the binary's settings registry, if one was supplied via
    /// [`KeymapBuilder::with_settings`].
    #[must_use]
    pub const fn settings(&self) -> Option<&SettingsRegistry<Ctx>> { self.settings.as_ref() }

    /// `pub(crate)` so [`crate::framework::dispatch_global`] can read
    /// the hook without widening the public surface.
    pub(crate) const fn on_quit_hook(&self) -> Option<fn(&mut Ctx)> { self.on_quit }

    /// `pub(crate)` so [`crate::framework::dispatch_global`] can read
    /// the hook without widening the public surface.
    pub(crate) const fn on_restart_hook(&self) -> Option<fn(&mut Ctx)> { self.on_restart }

    /// `pub(crate)` so [`crate::framework::dispatch_global`] can read
    /// the hook without widening the public surface.
    pub(crate) const fn dismiss_fallback_hook(&self) -> Option<fn(&mut Ctx) -> bool> {
        self.dismiss_fallback
    }

    /// Dispatch one [`GlobalAction`] through the framework's built-in
    /// behavior. Phase 11's input dispatcher calls this on every
    /// framework-global hit.
    pub fn dispatch_framework_global(&self, action: GlobalAction, ctx: &mut Ctx) {
        framework::dispatch_global(action, self, ctx);
    }
}

impl<Ctx: AppContext + 'static> core::fmt::Debug for Keymap<Ctx> {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Keymap")
            .field("scopes", &self.scopes.len())
            .field("config_path", &self.config_path)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::env;
    use std::fs;
    use std::process;
    use std::time::SystemTime;
    use std::time::UNIX_EPOCH;

    use crossterm::event::KeyCode;

    use super::Bindings;
    use super::Globals;
    use super::KeyBind;
    use super::KeyOutcome;
    use super::Keymap;
    use super::KeymapBuilder;
    use super::Navigation;
    use super::Shortcuts;
    use crate::AppContext;
    use crate::FocusedPane;
    use crate::Framework;
    use crate::Pane;

    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    enum TestPaneId {
        Foo,
        Bar,
    }

    crate::action_enum! {
        #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
        pub enum FooAction {
            Activate => ("activate", "go",    "Activate row");
            Clean    => ("clean",    "clean", "Clean target");
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
    }

    impl AppContext for TestApp {
        type AppPaneId = TestPaneId;
        type ToastAction = crate::NoToastAction;

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

    fn fresh_app() -> TestApp {
        TestApp {
            framework: Framework::new(FocusedPane::App(TestPaneId::Foo)),
        }
    }

    fn fresh_builder() -> KeymapBuilder<TestApp, super::builder::Configuring> {
        Keymap::<TestApp>::builder()
            .register_navigation::<AppNav>()
            .expect("nav register must succeed")
            .register_globals::<AppGlobals>()
            .expect("globals register must succeed")
    }

    #[test]
    fn empty_keymap_dispatches_unhandled_for_any_pane() {
        let keymap: Keymap<TestApp> = Keymap::new(None);
        let mut app = fresh_app();
        assert_eq!(
            keymap.dispatch_app_pane(TestPaneId::Foo, &KeyCode::Enter.into(), &mut app),
            KeyOutcome::Unhandled,
        );
        assert!(
            keymap
                .render_app_pane_bar_slots(TestPaneId::Foo, &fresh_app())
                .is_empty(),
        );
        assert!(
            keymap
                .key_for_toml_key(TestPaneId::Foo, "activate")
                .is_none()
        );
        assert!(keymap.config_path().is_none());
    }

    #[test]
    fn registered_scope_dispatches_keys() {
        let keymap = fresh_builder()
            .register::<FooPane>(FooPane)
            .build()
            .expect("build keymap with one scope");

        let mut app = fresh_app();
        let outcome = keymap.dispatch_app_pane(TestPaneId::Foo, &KeyCode::Enter.into(), &mut app);
        assert_eq!(outcome, KeyOutcome::Consumed);
    }

    #[test]
    fn unregistered_pane_id_returns_unhandled() {
        let keymap = fresh_builder()
            .register::<FooPane>(FooPane)
            .build()
            .expect("build keymap with one scope");

        let mut app = fresh_app();
        let outcome = keymap.dispatch_app_pane(TestPaneId::Bar, &KeyCode::Enter.into(), &mut app);
        assert_eq!(outcome, KeyOutcome::Unhandled);
    }

    #[test]
    fn render_app_pane_bar_slots_resolves_through_keymap() {
        let keymap = fresh_builder()
            .register::<FooPane>(FooPane)
            .build()
            .expect("build keymap with one scope");
        let slots = keymap.render_app_pane_bar_slots(TestPaneId::Foo, &fresh_app());
        let labels: Vec<&'static str> = slots.iter().map(|s| s.label).collect();
        assert_eq!(labels, vec!["go", "clean"]);
    }

    #[test]
    fn render_app_pane_bar_slots_empty_for_unregistered_pane() {
        let keymap = fresh_builder()
            .register::<FooPane>(FooPane)
            .build()
            .expect("build keymap with one scope");
        assert!(
            keymap
                .render_app_pane_bar_slots(TestPaneId::Bar, &fresh_app())
                .is_empty(),
        );
    }

    #[test]
    fn key_for_toml_key_round_trips_through_keymap() {
        let keymap = fresh_builder()
            .register::<FooPane>(FooPane)
            .build()
            .expect("build keymap with one scope");

        assert_eq!(
            keymap.key_for_toml_key(TestPaneId::Foo, "activate"),
            Some(KeyCode::Enter.into()),
        );
        assert_eq!(
            keymap.key_for_toml_key(TestPaneId::Foo, "clean"),
            Some(KeyBind::from('c')),
        );
        assert!(
            keymap
                .key_for_toml_key(TestPaneId::Foo, "frobnicate")
                .is_none()
        );
        assert!(
            keymap
                .key_for_toml_key(TestPaneId::Bar, "activate")
                .is_none()
        );
    }

    #[test]
    fn is_key_bound_to_toml_key_checks_all_bindings() {
        let path = env::temp_dir().join(format!(
            "tui-pane-keymap-{}-{}.toml",
            process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time after epoch")
                .as_nanos(),
        ));
        fs::write(&path, "[foo]\nactivate = [\"Enter\", \"q\"]\n").expect("write keymap toml");

        let keymap = Keymap::<TestApp>::builder()
            .load_toml(path.clone())
            .expect("load toml")
            .register_navigation::<AppNav>()
            .expect("nav register must succeed")
            .register_globals::<AppGlobals>()
            .expect("globals register must succeed")
            .register::<FooPane>(FooPane)
            .build()
            .expect("build keymap with one scope");
        fs::remove_file(path).expect("remove keymap toml");

        assert_eq!(
            keymap.key_for_toml_key(TestPaneId::Foo, "activate"),
            Some(KeyCode::Enter.into()),
        );
        assert!(keymap.is_key_bound_to_toml_key(
            TestPaneId::Foo,
            "activate",
            &KeyCode::Enter.into(),
        ));
        assert!(keymap.is_key_bound_to_toml_key(TestPaneId::Foo, "activate", &KeyBind::from('q'),));
        assert!(
            !keymap.is_key_bound_to_toml_key(TestPaneId::Foo, "activate", &KeyBind::from('c'),)
        );
        assert!(!keymap.is_key_bound_to_toml_key(
            TestPaneId::Foo,
            "frobnicate",
            &KeyBind::from('q'),
        ));
        assert!(
            !keymap.is_key_bound_to_toml_key(TestPaneId::Bar, "activate", &KeyBind::from('q'),)
        );
    }
}
