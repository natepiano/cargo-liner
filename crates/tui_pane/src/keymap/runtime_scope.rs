//! Runtime-scope vtable for per-pane keymap operations.
//!
//! [`Keymap<Ctx>`](super::Keymap) stores one trait object per
//! registered pane behind [`RuntimeScope<Ctx>`]. The trait is
//! `pub(crate)` — only the keymap and its builder name it. Public
//! callers reach pane operations through the convenience methods on
//! [`Keymap`](super::Keymap) ([`dispatch_app_pane`](super::Keymap::dispatch_app_pane),
//! [`render_app_pane_bar_slots`](super::Keymap::render_app_pane_bar_slots),
//! [`key_for_toml_key`](super::Keymap::key_for_toml_key)).
//!
//! Each trait method is a complete pane operation: typed access to
//! `P::Actions` happens **inside** [`PaneScope<Ctx, P>`]'s impl, where
//! `P: Shortcuts<Ctx>` is in scope. The trait surface itself stays
//! type-parameter-free so the keymap can hold heterogeneous panes in
//! one map.

use super::Action;
use super::KeyBind;
use super::KeyOutcome;
use super::ScopeMap;
use super::Shortcuts;
use crate::AppContext;
use crate::BarRegion;
use crate::ShortcutState;
use crate::Visibility;

/// Crate-private vtable for per-pane keymap operations.
pub(crate) trait RuntimeScope<Ctx: AppContext>: 'static {
    /// Resolve `bind` to an action and call the pane's dispatcher.
    /// Returns [`KeyOutcome::Consumed`] on a hit; [`KeyOutcome::Unhandled`]
    /// when no binding matches.
    fn dispatch_key(&self, bind: &KeyBind, ctx: &mut Ctx) -> KeyOutcome;

    /// Bar slots already reduced to label + key + state + visibility.
    /// Slots with [`Visibility::Hidden`] or no bound key are dropped
    /// from the returned `Vec`.
    fn render_bar_slots(&self, ctx: &Ctx) -> Vec<RenderedSlot>;

    /// Reverse lookup: TOML key string → bound [`KeyBind`].
    /// Returns `None` if `key` does not name an action in this scope's
    /// action enum, or if the named action has no binding.
    fn key_for_toml_key(&self, key: &str) -> Option<KeyBind>;
}

/// One bar slot, fully resolved for the renderer.
///
/// Pre-resolves everything the typed scope used to expose piecemeal so
/// no typed action enum has to cross the trait. Hidden slots and
/// unbound slots are dropped before this struct is built; `visibility`
/// is preserved on the struct so renderers can still distinguish
/// current visible-ness without requiring another lookup.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct RenderedSlot {
    /// Which bar region this slot belongs to.
    pub region:     BarRegion,
    /// The action's bar label (e.g. `"activate"`).
    pub label:      &'static str,
    /// The currently bound key.
    pub key:        KeyBind,
    /// Active vs greyed-out.
    pub state:      ShortcutState,
    /// Always [`Visibility::Visible`] in the returned `Vec` (hidden
    /// slots are dropped); kept on the struct for renderer
    /// uniformity.
    pub visibility: Visibility,
}

/// The single implementor of [`RuntimeScope<Ctx>`]. Captures the
/// typed pane and its bindings at registration time so the impl can
/// call `P::dispatcher()`, `P::Actions::from_toml_key`, `P::bar_slots`,
/// etc. without leaking `P` into the trait.
pub(super) struct PaneScope<Ctx: AppContext + 'static, P: Shortcuts<Ctx>> {
    pub(super) pane:     P,
    pub(super) bindings: ScopeMap<P::Actions>,
}

impl<Ctx: AppContext + 'static, P: Shortcuts<Ctx>> RuntimeScope<Ctx> for PaneScope<Ctx, P> {
    fn dispatch_key(&self, bind: &KeyBind, ctx: &mut Ctx) -> KeyOutcome {
        self.bindings
            .action_for(bind)
            .map_or(KeyOutcome::Unhandled, |action| {
                P::dispatcher()(action, ctx);
                KeyOutcome::Consumed
            })
    }

    fn render_bar_slots(&self, ctx: &Ctx) -> Vec<RenderedSlot> {
        self.pane
            .bar_slots(ctx)
            .into_iter()
            .filter_map(|(region, slot)| {
                let action = slot.primary();
                let visibility = self.pane.visibility(action, ctx);
                if matches!(visibility, Visibility::Hidden) {
                    return None;
                }
                let key = self.bindings.key_for(action).copied()?;
                Some(RenderedSlot {
                    region,
                    label: action.bar_label(),
                    key,
                    state: self.pane.state(action, ctx),
                    visibility,
                })
            })
            .collect()
    }

    fn key_for_toml_key(&self, key: &str) -> Option<KeyBind> {
        let action = P::Actions::from_toml_key(key)?;
        self.bindings.key_for(action).copied()
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
    use core::sync::atomic::AtomicUsize;
    use core::sync::atomic::Ordering;

    use crossterm::event::KeyCode;

    use super::PaneScope;
    use super::RenderedSlot;
    use super::RuntimeScope;
    use crate::AppContext;
    use crate::BarRegion;
    use crate::BarSlot;
    use crate::FocusedPane;
    use crate::Framework;
    use crate::Pane;
    use crate::ShortcutState;
    use crate::Visibility;
    use crate::keymap::Bindings;
    use crate::keymap::KeyBind;
    use crate::keymap::KeyOutcome;
    use crate::keymap::Shortcuts;

    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    enum TestPaneId {
        Foo,
    }

    crate::action_enum! {
        #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
        pub enum FooAction {
            Activate => ("activate", "go",    "Activate row");
            Clean    => ("clean",    "clean", "Clean target");
        }
    }

    struct TestApp {
        framework:  Framework<Self>,
        dispatched: AtomicUsize,
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
            |_action, ctx| {
                ctx.dispatched.fetch_add(1, Ordering::SeqCst);
            }
        }
    }

    fn fresh_app() -> TestApp {
        TestApp {
            framework:  Framework::new(FocusedPane::App(TestPaneId::Foo)),
            dispatched: AtomicUsize::new(0),
        }
    }

    fn fresh_scope() -> PaneScope<TestApp, FooPane> {
        PaneScope {
            pane:     FooPane,
            bindings: FooPane::defaults().into_scope_map(),
        }
    }

    #[test]
    fn dispatch_consumed_on_match_and_calls_dispatcher() {
        let scope = fresh_scope();
        let mut app = fresh_app();
        let outcome = scope.dispatch_key(&KeyCode::Enter.into(), &mut app);
        assert_eq!(outcome, KeyOutcome::Consumed);
        assert_eq!(app.dispatched.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn dispatch_unhandled_on_miss_does_not_call_dispatcher() {
        let scope = fresh_scope();
        let mut app = fresh_app();
        let outcome = scope.dispatch_key(&'z'.into(), &mut app);
        assert_eq!(outcome, KeyOutcome::Unhandled);
        assert_eq!(app.dispatched.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn render_bar_slots_resolves_label_key_and_state() {
        let scope = fresh_scope();
        let app = fresh_app();
        let slots = scope.render_bar_slots(&app);
        assert_eq!(slots.len(), 2);
        assert_eq!(
            slots[0],
            RenderedSlot {
                region:     BarRegion::PaneAction,
                label:      "go",
                key:        KeyBind::from(KeyCode::Enter),
                state:      ShortcutState::Enabled,
                visibility: Visibility::Visible,
            },
        );
        assert_eq!(
            slots[1],
            RenderedSlot {
                region:     BarRegion::PaneAction,
                label:      "clean",
                key:        KeyBind::from('c'),
                state:      ShortcutState::Enabled,
                visibility: Visibility::Visible,
            },
        );
    }

    #[test]
    fn render_bar_slots_drops_hidden_slots() {
        struct HidesActivate;
        impl Pane<TestApp> for HidesActivate {
            const APP_PANE_ID: TestPaneId = TestPaneId::Foo;
        }
        impl Shortcuts<TestApp> for HidesActivate {
            type Actions = FooAction;
            const SCOPE_NAME: &'static str = "hides";
            fn defaults() -> Bindings<Self::Actions> { FooPane::defaults() }
            fn dispatcher() -> fn(Self::Actions, &mut TestApp) { FooPane::dispatcher() }
            fn visibility(&self, action: Self::Actions, _ctx: &TestApp) -> Visibility {
                match action {
                    FooAction::Activate => Visibility::Hidden,
                    FooAction::Clean => Visibility::Visible,
                }
            }
        }

        let scope = PaneScope {
            pane:     HidesActivate,
            bindings: HidesActivate::defaults().into_scope_map(),
        };
        let app = fresh_app();
        let slots = scope.render_bar_slots(&app);
        assert_eq!(slots.len(), 1);
        assert_eq!(slots[0].label, "clean");
    }

    #[test]
    fn render_bar_slots_uses_paired_primary_for_lookup() {
        struct PairedPane;
        impl Pane<TestApp> for PairedPane {
            const APP_PANE_ID: TestPaneId = TestPaneId::Foo;
        }
        impl Shortcuts<TestApp> for PairedPane {
            type Actions = FooAction;
            const SCOPE_NAME: &'static str = "paired";
            fn defaults() -> Bindings<Self::Actions> { FooPane::defaults() }
            fn dispatcher() -> fn(Self::Actions, &mut TestApp) { FooPane::dispatcher() }
            fn bar_slots(&self, _ctx: &TestApp) -> Vec<(BarRegion, BarSlot<Self::Actions>)> {
                vec![(
                    BarRegion::PaneAction,
                    BarSlot::Paired(FooAction::Activate, FooAction::Clean, "/"),
                )]
            }
        }

        let scope = PaneScope {
            pane:     PairedPane,
            bindings: PairedPane::defaults().into_scope_map(),
        };
        let app = fresh_app();
        let slots = scope.render_bar_slots(&app);
        assert_eq!(slots.len(), 1);
        assert_eq!(slots[0].label, "go");
        assert_eq!(slots[0].key, KeyBind::from(KeyCode::Enter));
    }

    #[test]
    fn key_for_toml_key_round_trips_known_actions() {
        let scope = fresh_scope();
        assert_eq!(
            scope.key_for_toml_key("activate"),
            Some(KeyBind::from(KeyCode::Enter)),
        );
        assert_eq!(scope.key_for_toml_key("clean"), Some(KeyBind::from('c')));
    }

    #[test]
    fn key_for_toml_key_unknown_action_returns_none() {
        let scope = fresh_scope();
        assert!(scope.key_for_toml_key("frobnicate").is_none());
    }

    #[test]
    fn dispatch_through_trait_object() {
        let scope = fresh_scope();
        let erased: &dyn RuntimeScope<TestApp> = &scope;
        let mut app = fresh_app();
        let outcome = erased.dispatch_key(&KeyCode::Enter.into(), &mut app);
        assert_eq!(outcome, KeyOutcome::Consumed);
    }
}
