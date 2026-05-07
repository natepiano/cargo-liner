//! `KeymapBuilder<Ctx>`: incremental construction surface for
//! [`Keymap<Ctx>`](super::Keymap).
//!
//! Phase 9 ships the skeleton plus the two registration entry points
//! ([`Self::register`] for full pane scopes, [`Self::register_app_pane`]
//! for pane-id-only registration). The full body — TOML overlay, vim
//! extras, settings registry, lifecycle hooks — lands in Phase 10.

use core::any::TypeId;
use std::collections::HashMap;
use std::path::PathBuf;

use super::Keymap;
use super::Pane;
use super::Shortcuts;
use super::erased_scope::ConcreteScope;
use super::erased_scope::ErasedScope;
use super::load::KeymapError;
use crate::AppContext;

/// Builder for [`Keymap<Ctx>`]. Constructed via
/// [`Keymap::<Ctx>::builder()`]. `build()` returns
/// [`Result<Keymap<Ctx>, KeymapError>`] so loader / validation
/// failures surface uniformly.
pub struct KeymapBuilder<Ctx: AppContext + 'static> {
    scopes:      HashMap<TypeId, Box<dyn ErasedScope<Ctx>>>,
    by_pane_id:  HashMap<Ctx::AppPaneId, TypeId>,
    config_path: Option<PathBuf>,
}

impl<Ctx: AppContext + 'static> KeymapBuilder<Ctx> {
    /// Empty builder.
    pub(super) fn new() -> Self {
        Self {
            scopes:      HashMap::new(),
            by_pane_id:  HashMap::new(),
            config_path: None,
        }
    }

    /// Register a [`Shortcuts<Ctx>`] impl: stores the typed bindings
    /// behind a [`ConcreteScope`] so the resulting [`Keymap`] can
    /// dispatch keys, render bar slots, and reverse-lookup TOML keys
    /// without ever holding the typed `<P>` parameter again.
    #[must_use]
    pub fn register<P: Shortcuts<Ctx>>(mut self, pane: P) -> Self {
        let bindings = P::defaults().into_scope_map();
        let scope: Box<dyn ErasedScope<Ctx>> =
            Box::new(ConcreteScope::<Ctx, P>::new(pane, bindings));
        let type_id = TypeId::of::<P>();
        self.by_pane_id.insert(P::APP_PANE_ID, type_id);
        self.scopes.insert(type_id, scope);
        self
    }

    /// Register a pane that has identity but no [`Shortcuts<Ctx>`] impl
    /// — the framework records `P::APP_PANE_ID` and `P::mode()` but no
    /// scope bindings. Used by text-input panes and any future
    /// non-shortcut pane registrations.
    ///
    /// Phase 9 stores nothing: the registry slot for this path lands
    /// when [`Framework::register_app_pane`](crate::Framework) gains
    /// the `mode_queries` map (later phase).
    #[must_use]
    pub fn register_app_pane<P: Pane<Ctx>>(self) -> Self {
        let _ = P::APP_PANE_ID;
        let _ = P::mode();
        self
    }

    /// Override the config path the loader will read.
    #[must_use]
    pub fn config_path(mut self, path: PathBuf) -> Self {
        self.config_path = Some(path);
        self
    }

    /// Finalize the builder. Returns the built [`Keymap<Ctx>`] or the
    /// first loader / validation error encountered.
    ///
    /// Phase 9 implementation is pure construction (no TOML overlay
    /// yet); the [`Result`] return type is in place so Phase 10 can
    /// wire loader failures without a signature change.
    ///
    /// # Errors
    ///
    /// Returns a [`KeymapError`] when the loader (Phase 10+) fails —
    /// I/O, TOML parse, unknown action / scope, duplicate or colliding
    /// bindings, or a key string that fails [`KeyBind::parse`]. Phase 9
    /// always returns `Ok` because no loader has been wired yet.
    pub fn build(self) -> Result<Keymap<Ctx>, KeymapError> {
        let mut keymap = Keymap::<Ctx>::new(self.config_path);
        for (type_id, scope) in self.scopes {
            keymap.insert_scope_raw(type_id, scope);
        }
        for (pane_id, type_id) in self.by_pane_id {
            keymap.insert_pane_id_raw(pane_id, type_id);
        }
        Ok(keymap)
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
        assert!(keymap.scope_for::<FooPane>().is_none());
        assert!(keymap.scope_for_app_pane(TestPaneId::Foo).is_none());
        assert!(keymap.config_path().is_none());
    }

    #[test]
    fn register_inserts_scope_findable_both_ways() {
        let keymap = Keymap::<TestApp>::builder()
            .register::<FooPane>(FooPane)
            .build()
            .expect("build must succeed");
        assert!(keymap.scope_for::<FooPane>().is_some());
        assert!(keymap.scope_for_app_pane(TestPaneId::Foo).is_some());
    }

    #[test]
    fn register_app_pane_does_not_add_a_scope() {
        let keymap = Keymap::<TestApp>::builder()
            .register_app_pane::<FooPane>()
            .build()
            .expect("build must succeed");
        assert!(keymap.scope_for::<FooPane>().is_none());
        assert!(keymap.scope_for_app_pane(TestPaneId::Foo).is_none());
    }

    #[test]
    fn config_path_round_trips() {
        let path = std::path::PathBuf::from("/tmp/keymap.toml");
        let keymap = Keymap::<TestApp>::builder()
            .config_path(path.clone())
            .build()
            .expect("build must succeed");
        assert_eq!(keymap.config_path(), Some(path.as_path()));
    }

    #[test]
    fn registered_scope_dispatches_keys() {
        let keymap = Keymap::<TestApp>::builder()
            .register::<FooPane>(FooPane)
            .build()
            .expect("build must succeed");
        let scope = keymap
            .scope_for_app_pane(TestPaneId::Foo)
            .expect("scope must be present");
        let mut app = fresh_app();
        let outcome = scope.dispatch_key(&KeyCode::Enter.into(), &mut app);
        assert_eq!(outcome, KeyOutcome::Consumed);
    }
}
