//! `Shortcuts<Ctx>`: per-pane scope trait.
//!
//! State-bearing scope — one impl per app pane type. Drives bar
//! rendering, keymap-overlay help, and per-action dispatch. The
//! framework keys its per-pane query registry (e.g.
//! `input_mode_queries`) on
//! [`AppContext::AppPaneId`](crate::AppContext::AppPaneId), which is
//! why each impl declares its [`Self::APP_PANE_ID`] variant.

use crate::AppContext;
use crate::BarRegion;
use crate::BarSlot;
use crate::InputMode;
use crate::ShortcutState;
use crate::keymap::Action;
use crate::keymap::Bindings;
use crate::keymap::KeyBind;

/// Per-pane scope: state-bearing, one impl per pane type.
///
/// Each implementor declares its action-enum [`Self::Variant`], a
/// stable [`Self::SCOPE_NAME`] for TOML, and a [`Self::APP_PANE_ID`]
/// the framework uses to key its per-pane registries. Methods cover
/// default bindings, bar rendering, label/state overrides, input-mode
/// query, vim-mode extras, and the free-function dispatcher.
///
/// `'static` is required because the framework keys its registries on
/// `TypeId<P>` and stores `fn` pointers — both demand `'static`. Pane
/// instances live on the binary's `App`; the trait impl itself never
/// holds borrowed data.
pub trait Shortcuts<Ctx: AppContext>: 'static {
    /// The pane's action enum.
    type Variant: Action;

    /// TOML table name for this scope (e.g. `"project_list"`). Must be
    /// stable — TOML files are user-edited.
    const SCOPE_NAME: &'static str;

    /// Stable per-pane identity used by the framework's per-pane
    /// query registry (e.g. `input_mode_queries`). The trait covers
    /// app panes only — framework panes (Keymap, Settings, Toasts) are
    /// special-cased — so the variant is always an `AppPaneId`.
    const APP_PANE_ID: Ctx::AppPaneId;

    /// Default keybindings. No framework default — every pane declares
    /// its own keys.
    fn defaults() -> Bindings<Self::Variant>;

    /// Per-action bar label. `None` hides the slot. Default returns
    /// `Some(action.bar_label())` (the static label declared in
    /// `action_enum!`). Override only when the label depends on pane
    /// state.
    fn label(&self, action: Self::Variant, _ctx: &Ctx) -> Option<&'static str> {
        Some(action.bar_label())
    }

    /// Per-action enabled / disabled status. Default
    /// [`ShortcutState::Enabled`]. Override when the action is visible
    /// but inert (e.g. an action grayed out when no target exists).
    fn state(&self, _action: Self::Variant, _ctx: &Ctx) -> ShortcutState { ShortcutState::Enabled }

    /// Bar slot layout. Default: one
    /// `(BarRegion::PaneAction, BarSlot::Single(action))` per
    /// [`Action::ALL`] in declaration order. Override to introduce
    /// `Paired` slots, route into [`BarRegion::Nav`], or omit
    /// data-dependent slots.
    fn bar_slots(&self, _ctx: &Ctx) -> Vec<(BarRegion, BarSlot<Self::Variant>)> {
        Self::Variant::ALL
            .iter()
            .copied()
            .map(|a| (BarRegion::PaneAction, BarSlot::Single(a)))
            .collect()
    }

    /// Pane's current input mode (Navigable / Static / TextInput).
    /// Drives bar-region suppression and the structural Esc gate.
    ///
    /// Returns a `fn(&Ctx) -> InputMode` so the framework can store
    /// the pointer in `Framework<Ctx>::input_mode_queries`, keyed by
    /// `AppPaneId`, populated at `register::<P>()` time. The framework
    /// holds `&Ctx` and an `AppPaneId` at query time, never a typed
    /// `&PaneStruct`, so the closure does the navigation from `Ctx` to
    /// whatever pane state determines the mode.
    ///
    /// Default returns [`InputMode::Navigable`]. Panes whose mode
    /// varies with `Ctx` state override.
    fn input_mode() -> fn(&Ctx) -> InputMode { |_ctx| InputMode::Navigable }

    /// Pane actions that gain a vim binding when
    /// [`VimMode::Enabled`](crate::VimMode::Enabled). Default empty.
    /// Consumed by the keymap builder, which appends each
    /// `(action, key)` pair to the pane's scope after the TOML overlay.
    fn vim_extras() -> &'static [(Self::Variant, KeyBind)] { &[] }

    /// Free-function dispatcher. The framework calls
    /// `Self::dispatcher()(action, ctx)` while holding `&mut Ctx`.
    /// Implementations navigate from the `Ctx` root rather than
    /// holding a `&mut self` borrow — the framework never owns a typed
    /// `&PaneStruct` at dispatch time.
    fn dispatcher() -> fn(Self::Variant, &mut Ctx);
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

    use super::Shortcuts;
    use crate::AppContext;
    use crate::BarRegion;
    use crate::BarSlot;
    use crate::FocusedPane;
    use crate::Framework;
    use crate::InputMode;
    use crate::ShortcutState;
    use crate::keymap::Bindings;

    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    enum TestPaneId {
        Foo,
    }

    crate::action_enum! {
        #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
        pub enum FooAction {
            Activate => ("activate", "activate", "Activate row");
            Clean    => ("clean",    "clean",    "Clean target dir");
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

    impl Shortcuts<TestApp> for FooPane {
        type Variant = FooAction;

        const APP_PANE_ID: TestPaneId = TestPaneId::Foo;
        const SCOPE_NAME: &'static str = "foo";

        fn defaults() -> Bindings<Self::Variant> {
            crate::bindings! {
                KeyCode::Enter => FooAction::Activate,
                'c' => FooAction::Clean,
            }
        }

        fn dispatcher() -> fn(Self::Variant, &mut TestApp) {
            |_action, _ctx| { /* no-op for the test impl */ }
        }
    }

    fn fresh_app() -> TestApp {
        TestApp {
            framework: Framework::new(FocusedPane::App(TestPaneId::Foo)),
        }
    }

    #[test]
    fn defaults_round_trip_through_scope_map() {
        let map = FooPane::defaults().into_scope_map();
        assert_eq!(
            map.action_for(&KeyCode::Enter.into()),
            Some(FooAction::Activate),
        );
        assert_eq!(map.action_for(&'c'.into()), Some(FooAction::Clean));
    }

    #[test]
    fn default_label_returns_action_bar_label() {
        let pane = FooPane;
        let app = fresh_app();
        assert_eq!(pane.label(FooAction::Activate, &app), Some("activate"));
        assert_eq!(pane.label(FooAction::Clean, &app), Some("clean"));
    }

    #[test]
    fn default_state_is_enabled() {
        let pane = FooPane;
        let app = fresh_app();
        assert_eq!(
            pane.state(FooAction::Activate, &app),
            ShortcutState::Enabled
        );
    }

    #[test]
    fn default_bar_slots_emit_one_pane_action_per_variant_in_declaration_order() {
        let pane = FooPane;
        let app = fresh_app();
        assert_eq!(
            pane.bar_slots(&app),
            vec![
                (BarRegion::PaneAction, BarSlot::Single(FooAction::Activate)),
                (BarRegion::PaneAction, BarSlot::Single(FooAction::Clean)),
            ],
        );
    }

    #[test]
    fn default_input_mode_returns_navigable() {
        let app = fresh_app();
        let query: fn(&TestApp) -> InputMode = FooPane::input_mode();
        assert_eq!(query(&app), InputMode::Navigable);
    }

    #[test]
    fn default_vim_extras_is_empty() {
        assert!(FooPane::vim_extras().is_empty());
    }

    #[test]
    fn dispatcher_is_callable() {
        let mut app = fresh_app();
        let dispatch: fn(FooAction, &mut TestApp) = FooPane::dispatcher();
        dispatch(FooAction::Activate, &mut app);
    }

    #[test]
    fn const_identifiers_are_reachable() {
        assert_eq!(<FooPane as Shortcuts<TestApp>>::SCOPE_NAME, "foo");
        assert_eq!(
            <FooPane as Shortcuts<TestApp>>::APP_PANE_ID,
            TestPaneId::Foo,
        );
    }
}
