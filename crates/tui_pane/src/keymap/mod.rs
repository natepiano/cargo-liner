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

use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;

pub use action_enum::Action;
pub use bindings::Bindings;
pub use builder::Configuring;
pub use builder::KeymapBuilder;
#[allow(unused_imports, reason = "re-exported at crate root for callers naming the typestate")]
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
/// and [`Self::key_for_toml_key`] — the underlying
/// [`RuntimeScope`](self::runtime_scope::RuntimeScope) trait is
/// crate-private.
///
/// Framework panes are not stored in this map — they are special-cased
/// by [`FocusedPane::Framework`](crate::FocusedPane::Framework) arms in
/// callers (see Phase 11).
pub struct Keymap<Ctx: AppContext + 'static> {
    scopes:      HashMap<Ctx::AppPaneId, Box<dyn RuntimeScope<Ctx>>>,
    config_path: Option<PathBuf>,
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
            config_path,
        }
    }

    /// Insert one scope under its `AppPaneId`. `pub(super)` so only
    /// the builder writes.
    pub(super) fn insert_scope(
        &mut self,
        id: Ctx::AppPaneId,
        scope: Box<dyn RuntimeScope<Ctx>>,
    ) {
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
    pub fn render_app_pane_bar_slots(
        &self,
        id: Ctx::AppPaneId,
        ctx: &Ctx,
    ) -> Vec<RenderedSlot> {
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
    use super::KeyBind;
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
            Activate => ("activate", "go",    "Activate row");
            Clean    => ("clean",    "clean", "Clean target");
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

    fn fresh_app() -> TestApp {
        TestApp {
            framework: Framework::new(FocusedPane::App(TestPaneId::Foo)),
        }
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
        assert!(keymap.key_for_toml_key(TestPaneId::Foo, "activate").is_none());
        assert!(keymap.config_path().is_none());
    }

    #[test]
    fn registered_scope_dispatches_keys() {
        let keymap = Keymap::<TestApp>::builder()
            .register::<FooPane>(FooPane)
            .build()
            .expect("build keymap with one scope");

        let mut app = fresh_app();
        let outcome =
            keymap.dispatch_app_pane(TestPaneId::Foo, &KeyCode::Enter.into(), &mut app);
        assert_eq!(outcome, KeyOutcome::Consumed);
    }

    #[test]
    fn unregistered_pane_id_returns_unhandled() {
        let keymap = Keymap::<TestApp>::builder()
            .register::<FooPane>(FooPane)
            .build()
            .expect("build keymap with one scope");

        let mut app = fresh_app();
        let outcome =
            keymap.dispatch_app_pane(TestPaneId::Bar, &KeyCode::Enter.into(), &mut app);
        assert_eq!(outcome, KeyOutcome::Unhandled);
    }

    #[test]
    fn render_app_pane_bar_slots_resolves_through_keymap() {
        let keymap = Keymap::<TestApp>::builder()
            .register::<FooPane>(FooPane)
            .build()
            .expect("build keymap with one scope");
        let slots = keymap.render_app_pane_bar_slots(TestPaneId::Foo, &fresh_app());
        let labels: Vec<&'static str> = slots.iter().map(|s| s.label).collect();
        assert_eq!(labels, vec!["go", "clean"]);
    }

    #[test]
    fn render_app_pane_bar_slots_empty_for_unregistered_pane() {
        let keymap = Keymap::<TestApp>::builder()
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
        let keymap = Keymap::<TestApp>::builder()
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
        assert!(keymap.key_for_toml_key(TestPaneId::Foo, "frobnicate").is_none());
        assert!(keymap.key_for_toml_key(TestPaneId::Bar, "activate").is_none());
    }
}
