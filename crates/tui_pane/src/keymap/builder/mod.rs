//! `KeymapBuilder<Ctx, State>`: typestate construction surface for
//! [`Keymap<Ctx>`](super::Keymap).
//!
//! Two states:
//!
//! - [`Configuring`]: settings phase. Settings methods (`config_path`, `load_toml`, `vim_mode`,
//!   `on_quit`, `on_restart`, `dismiss_fallback`, `register_navigation`, `register_globals`,
//!   `register_settings_overlay`, `register_keymap_overlay`) are reachable here only.
//! - [`Registering`]: panes phase. Entered on the first [`KeymapBuilder::register`] call. Settings
//!   methods drop off the type — the compiler enforces "settings before panes" at compile time.
//!   `build_into(&mut Framework<Ctx>)` is the production finalizer.

use core::any::Any;
use core::any::type_name;
use core::marker::PhantomData;
use std::collections::HashMap;
use std::collections::HashSet;
use std::io::ErrorKind;
use std::path::PathBuf;

use toml::Table;

use super::Bindings;
use super::Globals;
use super::Keymap;
use super::Navigation;
use super::Shortcuts;
use super::load::KeymapError;
use super::runtime_scope;
use super::runtime_scope::PaneScope;
use super::runtime_scope::RuntimeScope;
use super::scope_map::ScopeMap;
use super::vim::VimMode;
use crate::AppContext;
use crate::Framework;
use crate::KeymapPane;
use crate::KeymapPaneAction;
use crate::Pane;
use crate::SettingsPane;
use crate::SettingsPaneAction;
use crate::TabStop;
use crate::framework::ModeQuery;

mod finalize;
mod overlay;
mod registration;

use finalize::finalize;
use overlay::action_key_set;
use overlay::apply_toml_overlay;
use overlay::apply_toml_overlay_with_peer;
use overlay::framework_global_action_key_set;
use registration::apply_vim_navigation_extras;
use registration::build_pane_bindings;

/// `Box<dyn Any>`-erased typed singleton. The builder stores the
/// `ScopeMap<X::Actions>` from a `Navigation` / `Globals` impl behind
/// this so [`Keymap`] can hold heterogeneous singletons in one field.
type ErasedSingleton = Box<dyn Any>;

/// `<N>`/`<G>`-monomorphized renderer that materializes a scope's
/// bar slots without naming the action enum. Captured at `register_*`
/// time and copied onto [`Keymap`] for the bar to read.
type ScopeRenderFn<Ctx> = fn(&Keymap<Ctx>) -> Vec<super::RenderedSlot>;

struct PaneRegistration<Ctx: AppContext> {
    id:         Ctx::AppPaneId,
    mode_query: ModeQuery<Ctx>,
    tab_stop:   TabStop<Ctx>,
}

/// Marker: builder is in the settings phase. Consumes settings
/// chained methods (`config_path`, etc.). Transitions to
/// [`Registering`] on the first [`KeymapBuilder::register`] call.
pub struct Configuring;

/// Marker: builder is in the panes phase. Settings methods are no
/// longer reachable on the type.
pub struct Registering;

/// Builder for [`Keymap<Ctx>`].
///
/// Constructed via [`Keymap::<Ctx>::builder()`]. Type parameter
/// `State` is one of [`Configuring`] (default) or [`Registering`];
/// the first [`Self::register`] call transitions the type.
///
/// `build()` returns [`Result<Keymap<Ctx>, KeymapError>`] so loader
/// and validation failures surface uniformly.
///
/// # Compile-time ordering
///
/// Settings methods are not callable on a builder in the
/// [`Registering`] state — the type system enforces "settings before
/// panes":
///
/// ```compile_fail
/// fn check<Ctx: tui_pane::AppContext + 'static>(
///     b: tui_pane::KeymapBuilder<Ctx, tui_pane::Registering>,
///     path: std::path::PathBuf,
/// ) {
///     // ERROR: no method `config_path` on `KeymapBuilder<Ctx, Registering>`.
///     let _ = b.config_path(path);
/// }
/// ```
pub struct KeymapBuilder<Ctx: AppContext + 'static, State = Configuring> {
    scopes:                HashMap<Ctx::AppPaneId, Box<dyn RuntimeScope<Ctx>>>,
    pane_registrations:    Vec<PaneRegistration<Ctx>>,
    registered_scopes:     HashSet<&'static str>,
    duplicate_scope:       Option<&'static str>,
    config_path:           Option<PathBuf>,
    toml_table:            Option<Table>,
    vim_mode:              VimMode,
    on_quit:               Option<fn(&mut Ctx)>,
    on_restart:            Option<fn(&mut Ctx)>,
    dismiss_fallback:      Option<fn(&mut Ctx) -> bool>,
    navigation_scope:      Option<ErasedSingleton>,
    navigation_scope_name: Option<&'static str>,
    /// `N`-monomorphized renderer captured at
    /// [`Self::register_navigation`] time; copied onto the keymap in
    /// [`finalize`]. The bar uses
    /// [`Keymap::render_navigation_slots`] without naming `N`.
    navigation_render_fn:  Option<ScopeRenderFn<Ctx>>,
    globals_scope:         Option<ErasedSingleton>,
    globals_scope_name:    Option<&'static str>,
    globals_action_keys:   Option<HashSet<&'static str>>,
    /// `G`-monomorphized renderer captured at
    /// [`Self::register_globals`] time. See
    /// [`Self::navigation_render_fn`].
    globals_render_fn:     Option<ScopeRenderFn<Ctx>>,
    settings_overlay:      Option<ScopeMap<SettingsPaneAction>>,
    keymap_overlay:        Option<ScopeMap<KeymapPaneAction>>,
    deferred_error:        Option<KeymapError>,
    _state:                PhantomData<State>,
}

impl<Ctx: AppContext + 'static> KeymapBuilder<Ctx, Configuring> {
    /// Empty builder.
    pub(super) fn new() -> Self {
        Self {
            scopes:                HashMap::new(),
            pane_registrations:    Vec::new(),
            registered_scopes:     HashSet::new(),
            duplicate_scope:       None,
            config_path:           None,
            toml_table:            None,
            vim_mode:              VimMode::Disabled,
            on_quit:               None,
            on_restart:            None,
            dismiss_fallback:      None,
            navigation_scope:      None,
            navigation_scope_name: None,
            navigation_render_fn:  None,
            globals_scope:         None,
            globals_scope_name:    None,
            globals_action_keys:   None,
            globals_render_fn:     None,
            settings_overlay:      None,
            keymap_overlay:        None,
            deferred_error:        None,
            _state:                PhantomData,
        }
    }

    /// Override the config path the loader will read.
    #[must_use]
    pub fn config_path(mut self, path: PathBuf) -> Self {
        self.config_path = Some(path);
        self
    }

    /// Read and parse the keymap TOML file at `path`. The parsed table
    /// is stored on the builder; subsequent `register*` calls overlay
    /// each scope's overrides onto its defaults.
    ///
    /// A missing file is treated as "use defaults" — `Ok(self)` is
    /// returned with no overlay table set. Parse failures and
    /// non-`NotFound` I/O errors surface as [`KeymapError`].
    ///
    /// Also records the path on the builder, equivalent to calling
    /// [`Self::config_path`] with the same path.
    ///
    /// # Errors
    ///
    /// Returns [`KeymapError::Io`] on a read failure other than
    /// `NotFound`, or [`KeymapError::Parse`] on a TOML syntax error.
    pub fn load_toml(mut self, path: PathBuf) -> Result<Self, KeymapError> {
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(err) if err.kind() == ErrorKind::NotFound => {
                self.config_path = Some(path);
                return Ok(self);
            },
            Err(err) => return Err(KeymapError::Io(err)),
        };
        let table: Table = toml::from_str(&text)?;
        self.toml_table = Some(table);
        self.config_path = Some(path);
        Ok(self)
    }

    /// Set the [`VimMode`] flag. When [`VimMode::Enabled`], each
    /// subsequent `register*` call appends vim navigation extras after
    /// applying TOML overrides.
    #[must_use]
    pub const fn vim_mode(mut self, mode: VimMode) -> Self {
        self.vim_mode = mode;
        self
    }

    /// Register a hook called after [`crate::GlobalAction::Quit`]
    /// flips
    /// `framework.quit_requested`. The hook can rely on
    /// `ctx.framework().quit_requested() == true`.
    #[must_use]
    pub const fn on_quit(mut self, hook: fn(&mut Ctx)) -> Self {
        self.on_quit = Some(hook);
        self
    }

    /// Register a hook called after [`crate::GlobalAction::Restart`]
    /// flips
    /// `framework.restart_requested`.
    #[must_use]
    pub const fn on_restart(mut self, hook: fn(&mut Ctx)) -> Self {
        self.on_restart = Some(hook);
        self
    }

    /// Register a fallback the framework calls when its own dismiss
    /// chain matches nothing. Returns `true` if the binary handled the
    /// dismiss (so the dispatcher can stop), `false` otherwise.
    #[must_use]
    pub const fn dismiss_fallback(mut self, hook: fn(&mut Ctx) -> bool) -> Self {
        self.dismiss_fallback = Some(hook);
        self
    }

    /// Register the [`Navigation`] singleton. Eagerly collapses
    /// [`N::defaults()`](Navigation::defaults) (with TOML and vim
    /// extras overlay) into a [`ScopeMap<N::Actions>`].
    ///
    /// # Errors
    ///
    /// Returns [`KeymapError`] on TOML parse / validation failures
    /// inside the `[N::SCOPE_NAME]` table.
    pub fn register_navigation<N: Navigation<Ctx>>(mut self) -> Result<Self, KeymapError> {
        let defaults = N::defaults();
        let scope_name = <N as Navigation<Ctx>>::SCOPE_NAME;
        let mut bindings =
            apply_toml_overlay::<N::Actions>(scope_name, defaults, self.toml_table.as_ref())?;
        if matches!(self.vim_mode, VimMode::Enabled) {
            apply_vim_navigation_extras::<Ctx, N>(&mut bindings);
        }
        let scope_map: ScopeMap<N::Actions> = bindings.into_scope_map();
        self.navigation_scope = Some(Box::new(scope_map));
        self.navigation_scope_name = Some(scope_name);
        self.navigation_render_fn = Some(runtime_scope::render_navigation_slots::<Ctx, N>);
        self.registered_scopes.insert(scope_name);
        Ok(self)
    }

    /// Register the [`Globals`] singleton. Eagerly collapses
    /// [`G::defaults()`](Globals::defaults) (with TOML overlay from
    /// the shared `[global]` table) into a [`ScopeMap<G::Actions>`].
    ///
    /// # Errors
    ///
    /// Returns [`KeymapError`] on TOML parse / validation failures
    /// inside the `[G::SCOPE_NAME]` table.
    pub fn register_globals<G: Globals<Ctx>>(mut self) -> Result<Self, KeymapError> {
        let defaults = G::defaults();
        let scope_name = <G as Globals<Ctx>>::SCOPE_NAME;
        let framework_keys = framework_global_action_key_set();
        let peer_keys = (scope_name == "global").then_some(&framework_keys);
        let bindings = apply_toml_overlay_with_peer::<G::Actions>(
            scope_name,
            defaults,
            self.toml_table.as_ref(),
            peer_keys,
        )?;
        let scope_map: ScopeMap<G::Actions> = bindings.into_scope_map();
        self.globals_scope = Some(Box::new(scope_map));
        self.globals_scope_name = Some(scope_name);
        if scope_name == "global" {
            self.globals_action_keys = Some(action_key_set::<G::Actions>());
        }
        self.globals_render_fn = Some(runtime_scope::render_app_globals_slots::<Ctx, G>);
        self.registered_scopes.insert(scope_name);
        Ok(self)
    }

    /// Register the framework-owned settings overlay scope. This
    /// makes `[settings]` a known TOML table and applies its overrides
    /// to the settings overlay bindings.
    ///
    /// # Errors
    ///
    /// Returns [`KeymapError`] on TOML parse / validation failures
    /// inside the `[settings]` table.
    pub fn register_settings_overlay(mut self) -> Result<Self, KeymapError> {
        let bindings = apply_toml_overlay::<SettingsPaneAction>(
            "settings",
            SettingsPane::defaults(),
            self.toml_table.as_ref(),
        )?;
        self.settings_overlay = Some(bindings.into_scope_map());
        self.registered_scopes.insert("settings");
        Ok(self)
    }

    /// Register the framework-owned keymap overlay scope. This makes
    /// `[keymap]` a known TOML table and applies its overrides to the
    /// keymap overlay bindings.
    ///
    /// # Errors
    ///
    /// Returns [`KeymapError`] on TOML parse / validation failures
    /// inside the `[keymap]` table.
    pub fn register_keymap_overlay(mut self) -> Result<Self, KeymapError> {
        let bindings = apply_toml_overlay::<KeymapPaneAction>(
            "keymap",
            KeymapPane::defaults(),
            self.toml_table.as_ref(),
        )?;
        self.keymap_overlay = Some(bindings.into_scope_map());
        self.registered_scopes.insert("keymap");
        Ok(self)
    }

    /// Register a [`Shortcuts<Ctx>`] impl. Eagerly collapses
    /// [`P::defaults()`](Shortcuts::defaults) (with TOML and vim
    /// extras overlay) into a [`ScopeMap<P::Actions>`] and stores the
    /// typed pane behind a [`RuntimeScope`] trait object keyed on
    /// `P::APP_PANE_ID`. Transitions the builder to [`Registering`].
    ///
    /// Errors during overlay are deferred until [`Self::build`] /
    /// [`Self::build_into`] so the chain stays a `Self`-returning
    /// flow. Phase 9 / Phase 10's overlay logic does not currently
    /// emit deferred errors per pane; that becomes relevant only when
    /// the loader's validation surface widens.
    ///
    /// Duplicate `APP_PANE_ID`s — two distinct `P` types claiming the
    /// same id — are recorded and surfaced as
    /// [`KeymapError::DuplicateScope`] from `build` / `build_into`.
    pub fn register<P: Shortcuts<Ctx>>(self, pane: P) -> KeymapBuilder<Ctx, Registering> {
        let mut next = transition::<Ctx>(self);
        next.insert_pane::<P>(pane);
        next
    }

    /// Finalize the builder with no scopes registered. Returns the
    /// built [`Keymap<Ctx>`].
    ///
    /// # Errors
    ///
    /// Returns [`KeymapError::NavigationMissing`] /
    /// [`KeymapError::GlobalsMissing`] when the matching singleton
    /// was not registered. Loader / overlay errors propagate from the
    /// `register*` methods that ran earlier.
    pub fn build(self) -> Result<Keymap<Ctx>, KeymapError> { finalize(self) }
}

impl<Ctx: AppContext + 'static> KeymapBuilder<Ctx, Registering> {
    /// Register an additional [`Shortcuts<Ctx>`] impl. Same body as
    /// the [`Configuring`]-state form, but stays in [`Registering`].
    #[must_use]
    pub fn register<P: Shortcuts<Ctx>>(mut self, pane: P) -> Self {
        self.insert_pane::<P>(pane);
        self
    }

    /// Finalize the builder. Returns the built [`Keymap<Ctx>`].
    ///
    /// Production code should call [`Self::build_into`] instead so
    /// the framework's per-pane mode-query and tab-stop registries are
    /// populated.
    ///
    /// # Errors
    ///
    /// Same as [`KeymapBuilder::<Configuring>::build`].
    pub fn build(self) -> Result<Keymap<Ctx>, KeymapError> { finalize(self) }

    /// Production finalizer. Builds the [`Keymap<Ctx>`] *and* writes
    /// the registered `(AppPaneId, mode_fn)` pairs into the
    /// framework's per-pane registries so
    /// [`Framework::focused_pane_mode`](crate::Framework::focused_pane_mode)
    /// can answer for every registered pane and focus cycling can
    /// read each pane's tab-stop metadata.
    ///
    /// # Errors
    ///
    /// Same as [`Self::build`].
    pub fn build_into(self, framework: &mut Framework<Ctx>) -> Result<Keymap<Ctx>, KeymapError> {
        for registration in &self.pane_registrations {
            framework.register_app_pane(
                registration.id,
                registration.mode_query,
                registration.tab_stop,
            );
        }
        finalize(self)
    }
}

impl<Ctx: AppContext + 'static, State> KeymapBuilder<Ctx, State> {
    /// Insert one pane scope. Records the `(id, mode_fn)` pair for
    /// `build_into` and the `SCOPE_NAME` for cross-scope validation.
    /// Detects `APP_PANE_ID` duplicates and stashes the offender's
    /// type name for `build` / `build_into` to surface as
    /// [`KeymapError::DuplicateScope`].
    fn insert_pane<P: Shortcuts<Ctx>>(&mut self, pane: P) {
        if self.scopes.contains_key(&P::APP_PANE_ID) && self.duplicate_scope.is_none() {
            self.duplicate_scope = Some(type_name::<P>());
        }
        let bindings = match build_pane_bindings::<Ctx, P>(self.toml_table.as_ref(), self.vim_mode)
        {
            Ok(b) => b,
            Err(err) => {
                if self.deferred_error.is_none() {
                    self.deferred_error = Some(err);
                }
                Bindings::new()
            },
        };
        let scope: Box<dyn RuntimeScope<Ctx>> = Box::new(PaneScope {
            pane,
            bindings: bindings.into_scope_map(),
        });
        self.scopes.insert(P::APP_PANE_ID, scope);
        self.pane_registrations.push(PaneRegistration {
            id:         P::APP_PANE_ID,
            mode_query: <P as Pane<Ctx>>::mode(),
            tab_stop:   <P as Pane<Ctx>>::tab_stop(),
        });
        self.registered_scopes
            .insert(<P as Shortcuts<Ctx>>::SCOPE_NAME);
    }
}

/// Move from the [`Configuring`] type to [`Registering`]. Field-by-
/// field move so the typestate transition is purely a type change at
/// runtime.
fn transition<Ctx: AppContext + 'static>(
    src: KeymapBuilder<Ctx, Configuring>,
) -> KeymapBuilder<Ctx, Registering> {
    KeymapBuilder {
        scopes:                src.scopes,
        pane_registrations:    src.pane_registrations,
        registered_scopes:     src.registered_scopes,
        duplicate_scope:       src.duplicate_scope,
        config_path:           src.config_path,
        toml_table:            src.toml_table,
        vim_mode:              src.vim_mode,
        on_quit:               src.on_quit,
        on_restart:            src.on_restart,
        dismiss_fallback:      src.dismiss_fallback,
        navigation_scope:      src.navigation_scope,
        navigation_scope_name: src.navigation_scope_name,
        navigation_render_fn:  src.navigation_render_fn,
        globals_scope:         src.globals_scope,
        globals_scope_name:    src.globals_scope_name,
        globals_action_keys:   src.globals_action_keys,
        globals_render_fn:     src.globals_render_fn,
        settings_overlay:      src.settings_overlay,
        keymap_overlay:        src.keymap_overlay,
        deferred_error:        src.deferred_error,
        _state:                PhantomData,
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
        let mut framework =
            Framework::<TestApp>::new(FocusedPane::Framework(FrameworkFocusId::Toasts));
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
        let mut framework =
            Framework::<TestApp>::new(FocusedPane::Framework(FrameworkFocusId::Toasts));
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
}
