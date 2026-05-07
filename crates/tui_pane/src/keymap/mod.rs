//! Keymap: types and traits for binding keys to actions.

mod action_enum;
mod bindings;
mod builder;
mod erased_scope;
mod global_action;
mod globals;
mod key_bind;
mod key_outcome;
mod load;
mod navigation;
mod scope_map;
mod shortcuts;
mod vim;

use core::any::TypeId;
use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;

pub use action_enum::Action;
pub use bindings::Bindings;
pub use builder::KeymapBuilder;
pub use erased_scope::ErasedScope;
pub use erased_scope::RenderedSlot;
pub use global_action::GlobalAction;
pub use globals::Globals;
pub use key_bind::KeyBind;
pub use key_bind::KeyInput;
pub use key_bind::KeyParseError;
pub use key_outcome::KeyOutcome;
pub use load::KeymapError;
pub use navigation::Navigation;
pub use scope_map::ScopeMap;
pub use shortcuts::Shortcuts;
pub use vim::VimMode;

use crate::AppContext;
use crate::Pane;

/// The keymap container: anchor for every binding the framework
/// resolves at runtime.
///
/// Built with [`Keymap::<Ctx>::builder()`]; the canonical entry point
/// returns a [`KeymapBuilder<Ctx>`] for incremental registration of
/// pane scopes, navigation, and globals (full body lands in Phase 10).
///
/// Holds two lookups for each registered pane scope:
///
/// - **`TypeId`-keyed:** [`Self::scope_for`] for callers that already have the type parameter
///   (pane-internal callers, dispatcher walks).
/// - **`AppPaneId`-keyed:** [`Self::scope_for_app_pane`] for callers that hold only a
///   [`FocusedPane`](crate::FocusedPane) and never a typed `P` (bar renderer in Phase 12, input
///   dispatcher in Phase 11).
///
/// Framework panes are not stored in this map — they are special-cased
/// by the [`FocusedPane::Framework`](crate::FocusedPane::Framework)
/// arms in callers (see Phase 11).
pub struct Keymap<Ctx: AppContext + 'static> {
    by_type:     HashMap<TypeId, Box<dyn ErasedScope<Ctx>>>,
    by_pane_id:  HashMap<Ctx::AppPaneId, TypeId>,
    config_path: Option<PathBuf>,
}

impl<Ctx: AppContext + 'static> Keymap<Ctx> {
    /// Canonical entry point for assembling a keymap. The full builder
    /// surface lands in Phase 10; the skeleton here lets Phase 9
    /// callers wire the registry and exercise the lookup paths.
    #[must_use]
    pub fn builder() -> KeymapBuilder<Ctx> { KeymapBuilder::new() }

    /// Empty keymap, no scopes registered. `pub(super)` because only
    /// [`KeymapBuilder::build`] (sibling) constructs one.
    pub(super) fn new(config_path: Option<PathBuf>) -> Self {
        Self {
            by_type: HashMap::new(),
            by_pane_id: HashMap::new(),
            config_path,
        }
    }

    /// Insert one scope under a `TypeId`. `pub(super)` so only the
    /// builder writes.
    pub(super) fn insert_scope_raw(&mut self, type_id: TypeId, scope: Box<dyn ErasedScope<Ctx>>) {
        self.by_type.insert(type_id, scope);
    }

    /// Index a `TypeId` under an `AppPaneId`. `pub(super)` so only the
    /// builder writes.
    pub(super) fn insert_pane_id_raw(&mut self, pane_id: Ctx::AppPaneId, type_id: TypeId) {
        self.by_pane_id.insert(pane_id, type_id);
    }

    /// Path to the config file the loader read (or would read), if
    /// any. `None` when the binary registered no config path or the
    /// file is missing — the loader treats a missing file as "use
    /// defaults" and returns `Ok`.
    #[must_use]
    pub fn config_path(&self) -> Option<&Path> { self.config_path.as_deref() }

    /// `TypeId<P>`-keyed scope lookup. Used by code that already has
    /// the type parameter — pane-internal callers, dispatcher walks.
    ///
    /// Returns a sealed [`ErasedScope`] trait object. External crates
    /// can name the trait but cannot implement it, so the only way to
    /// produce one is via [`KeymapBuilder::register`].
    #[must_use]
    pub fn scope_for<P: Shortcuts<Ctx>>(&self) -> Option<&dyn ErasedScope<Ctx>> {
        self.by_type.get(&TypeId::of::<P>()).map(Box::as_ref)
    }

    /// `AppPaneId`-keyed scope lookup. Used by callers that hold a
    /// [`FocusedPane`](crate::FocusedPane) but never a typed `P` — the
    /// bar renderer (Phase 12) and the input dispatcher (Phase 11).
    #[must_use]
    pub fn scope_for_app_pane(&self, id: Ctx::AppPaneId) -> Option<&dyn ErasedScope<Ctx>> {
        let type_id = self.by_pane_id.get(&id)?;
        self.by_type.get(type_id).map(Box::as_ref)
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

    use super::Bindings;
    use super::KeyOutcome;
    use super::Keymap;
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
    fn empty_keymap_has_no_scopes() {
        let keymap: Keymap<TestApp> = Keymap::new(None);
        assert!(keymap.scope_for::<FooPane>().is_none());
        assert!(keymap.scope_for_app_pane(TestPaneId::Foo).is_none());
        assert!(keymap.config_path().is_none());
    }

    #[test]
    fn registered_scope_is_findable_by_typeid_and_app_pane_id() {
        let keymap = Keymap::<TestApp>::builder()
            .register::<FooPane>(FooPane)
            .build()
            .expect("build keymap with one scope");

        assert!(keymap.scope_for::<FooPane>().is_some());
        assert!(keymap.scope_for_app_pane(TestPaneId::Foo).is_some());
        assert!(keymap.scope_for_app_pane(TestPaneId::Bar).is_none());
    }

    #[test]
    fn dispatch_through_app_pane_lookup() {
        let keymap = Keymap::<TestApp>::builder()
            .register::<FooPane>(FooPane)
            .build()
            .expect("build keymap with one scope");

        let scope = keymap
            .scope_for_app_pane(TestPaneId::Foo)
            .expect("scope must exist for Foo");
        let mut app = fresh_app();
        let outcome = scope.dispatch_key(&KeyCode::Enter.into(), &mut app);
        assert_eq!(outcome, KeyOutcome::Consumed);
    }
}
