//! `KeymapBuilder<Ctx, State>`: typestate construction surface for
//! [`Keymap<Ctx>`](super::Keymap).
//!
//! Two states:
//!
//! - [`Configuring`]: settings phase. Only state where settings
//!   methods (`config_path` now; Phase 10's `load_toml`, `vim_mode`,
//!   `with_settings`, `on_quit`, `on_restart`, `dismiss_fallback`)
//!   are reachable.
//! - [`Registering`]: panes phase. Entered on the first
//!   [`KeymapBuilder::register`] call. Settings methods drop off the
//!   type — the compiler enforces "settings before panes" at compile
//!   time.

use core::marker::PhantomData;
use std::collections::HashMap;
use std::path::PathBuf;

use super::Keymap;
use super::Shortcuts;
use super::load::KeymapError;
use super::runtime_scope::PaneScope;
use super::runtime_scope::RuntimeScope;
use crate::AppContext;

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
/// `build()` returns [`Result<Keymap<Ctx>, KeymapError>`] so loader /
/// validation failures (Phase 10+) surface uniformly.
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
    scopes:      HashMap<Ctx::AppPaneId, Box<dyn RuntimeScope<Ctx>>>,
    config_path: Option<PathBuf>,
    _state:      PhantomData<State>,
}

impl<Ctx: AppContext + 'static> KeymapBuilder<Ctx, Configuring> {
    /// Empty builder.
    pub(super) fn new() -> Self {
        Self {
            scopes:      HashMap::new(),
            config_path: None,
            _state:      PhantomData,
        }
    }

    /// Override the config path the loader will read.
    #[must_use]
    pub fn config_path(mut self, path: PathBuf) -> Self {
        self.config_path = Some(path);
        self
    }

    /// Register a [`Shortcuts<Ctx>`] impl. Eagerly collapses
    /// [`P::defaults()`](Shortcuts::defaults) into a
    /// [`ScopeMap<P::Actions>`](super::ScopeMap) and stores the typed
    /// pane behind a [`RuntimeScope`] trait object keyed on
    /// `P::APP_PANE_ID`. Transitions the builder to [`Registering`].
    #[must_use]
    pub fn register<P: Shortcuts<Ctx>>(
        mut self,
        pane: P,
    ) -> KeymapBuilder<Ctx, Registering> {
        self.scopes.insert(P::APP_PANE_ID, make_pane_scope::<Ctx, P>(pane));
        KeymapBuilder {
            scopes:      self.scopes,
            config_path: self.config_path,
            _state:      PhantomData,
        }
    }

    /// Finalize the builder with no scopes registered. Returns the
    /// built [`Keymap<Ctx>`].
    ///
    /// # Errors
    ///
    /// Returns a [`KeymapError`] when the loader (Phase 10+) fails —
    /// I/O, TOML parse, unknown action / scope, duplicate or colliding
    /// bindings, or a key string that fails [`KeyBind::parse`](super::KeyBind::parse).
    /// Phase 9 always returns `Ok` because no loader has been wired
    /// yet.
    pub fn build(self) -> Result<Keymap<Ctx>, KeymapError> {
        Ok(finalize(self.scopes, self.config_path))
    }
}

impl<Ctx: AppContext + 'static> KeymapBuilder<Ctx, Registering> {
    /// Register an additional [`Shortcuts<Ctx>`] impl. Same body as
    /// the [`Configuring`]-state form, but stays in [`Registering`].
    #[must_use]
    pub fn register<P: Shortcuts<Ctx>>(mut self, pane: P) -> Self {
        self.scopes.insert(P::APP_PANE_ID, make_pane_scope::<Ctx, P>(pane));
        self
    }

    /// Finalize the builder. Returns the built [`Keymap<Ctx>`].
    ///
    /// # Errors
    ///
    /// Returns a [`KeymapError`] when the loader (Phase 10+) fails —
    /// I/O, TOML parse, unknown action / scope, duplicate or colliding
    /// bindings, or a key string that fails [`KeyBind::parse`](super::KeyBind::parse).
    /// Phase 9 always returns `Ok`.
    pub fn build(self) -> Result<Keymap<Ctx>, KeymapError> {
        Ok(finalize(self.scopes, self.config_path))
    }
}

fn make_pane_scope<Ctx: AppContext + 'static, P: Shortcuts<Ctx>>(
    pane: P,
) -> Box<dyn RuntimeScope<Ctx>> {
    Box::new(PaneScope {
        pane,
        bindings: P::defaults().into_scope_map(),
    })
}

fn finalize<Ctx: AppContext + 'static>(
    scopes: HashMap<Ctx::AppPaneId, Box<dyn RuntimeScope<Ctx>>>,
    config_path: Option<PathBuf>,
) -> Keymap<Ctx> {
    let mut keymap = Keymap::<Ctx>::new(config_path);
    for (id, scope) in scopes {
        keymap.insert_scope(id, scope);
    }
    keymap
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use crossterm::event::KeyCode;

    use super::Keymap;
    use crate::AppContext;
    use crate::FocusedPane;
    use crate::Framework;
    use crate::Pane;
    use crate::keymap::Bindings;
    use crate::keymap::KeyOutcome;
    use crate::keymap::Shortcuts;

    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    enum TestPaneId {
        Foo,
        Bar,
    }

    crate::action_enum! {
        #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
        pub enum FooAction {
            Activate => ("activate", "go", "Activate row");
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
            crate::bindings! { KeyCode::Enter => FooAction::Activate }
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
        let keymap = Keymap::<TestApp>::builder()
            .register::<FooPane>(FooPane)
            .build()
            .expect("build must succeed");
        let mut app = fresh_app();
        let outcome =
            keymap.dispatch_app_pane(TestPaneId::Foo, &KeyCode::Enter.into(), &mut app);
        assert_eq!(outcome, KeyOutcome::Consumed);
        let other =
            keymap.dispatch_app_pane(TestPaneId::Bar, &KeyCode::Enter.into(), &mut app);
        assert_eq!(other, KeyOutcome::Unhandled);
    }

    #[test]
    fn config_path_round_trips() {
        let path = std::path::PathBuf::from("/tmp/keymap.toml");
        let keymap = Keymap::<TestApp>::builder()
            .config_path(path.clone())
            .register::<FooPane>(FooPane)
            .build()
            .expect("build must succeed");
        assert_eq!(keymap.config_path(), Some(path.as_path()));
    }

    #[test]
    fn registered_scope_dispatches_keys_through_keymap() {
        let keymap = Keymap::<TestApp>::builder()
            .register::<FooPane>(FooPane)
            .build()
            .expect("build must succeed");
        let mut app = fresh_app();
        let outcome =
            keymap.dispatch_app_pane(TestPaneId::Foo, &KeyCode::Enter.into(), &mut app);
        assert_eq!(outcome, KeyOutcome::Consumed);
    }

    #[test]
    fn register_chains_in_registering_state() {
        // Two distinct panes; second register stays in Registering.
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

        let keymap = Keymap::<TestApp>::builder()
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
}
