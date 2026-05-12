//! `Shortcuts<Ctx>`: per-pane scope trait.
//!
//! State-bearing scope — one impl per app pane type. Drives bar
//! rendering, keymap-overlay help, and per-action dispatch. Pane
//! identity and the input-mode query live on the
//! [`Pane<Ctx>`](crate::Pane) supertrait, not here.

use super::Action;
use super::Bindings;
use super::KeyBind;
use crate::AppContext;
use crate::BarRegion;
use crate::BarSlot;
use crate::Pane;
use crate::ShortcutState;
use crate::Visibility;

/// Per-pane scope: state-bearing, one impl per pane type.
///
/// Each implementor declares its action-enum [`Self::Actions`] and a
/// stable [`Self::SCOPE_NAME`] for TOML. Methods cover default
/// bindings, bar rendering, per-action visibility / state overrides,
/// vim-mode extras, and the free-function dispatcher. Pane identity
/// (`APP_PANE_ID`) and input-mode query (`mode`) come from
/// [`Pane<Ctx>`](crate::Pane).
///
/// `'static` is inherited from [`Pane<Ctx>`](crate::Pane); the
/// framework keys its registries on `TypeId<P>` and stores `fn`
/// pointers, both of which require `'static`.
pub trait Shortcuts<Ctx: AppContext>: Pane<Ctx> {
    /// The pane's action enum.
    type Actions: Action;

    /// TOML table name for this scope (e.g. `"project_list"`). Must be
    /// stable — TOML files are user-edited.
    const SCOPE_NAME: &'static str;

    /// Default keybindings. No framework default — every pane declares
    /// its own keys.
    fn defaults() -> Bindings<Self::Actions>;

    /// Per-action show/hide. Default [`Visibility::Visible`]. Override
    /// when a slot drops from the bar based on pane state — e.g.
    /// `Activate` returning [`Visibility::Hidden`] when no row is
    /// selected. Distinct from [`Self::state`], which keeps the slot
    /// in the bar but grays it out.
    fn visibility(&self, _action: Self::Actions, _ctx: &Ctx) -> Visibility { Visibility::Visible }

    /// Per-action enabled / disabled status. Default
    /// [`ShortcutState::Enabled`]. Override when the action is visible
    /// but inert (e.g. an action grayed out when no target exists).
    fn state(&self, _action: Self::Actions, _ctx: &Ctx) -> ShortcutState { ShortcutState::Enabled }

    /// Bar slot layout. Default: one
    /// `(BarRegion::PaneAction, BarSlot::Single(action))` per
    /// [`Action::ALL`] in declaration order. Override to introduce
    /// `Paired` slots, route into [`BarRegion::Nav`], or omit
    /// data-dependent slots.
    fn bar_slots(&self, _ctx: &Ctx) -> Vec<(BarRegion, BarSlot<Self::Actions>)> {
        Self::Actions::ALL
            .iter()
            .copied()
            .map(|a| (BarRegion::PaneAction, BarSlot::Single(a)))
            .collect()
    }

    /// Pane actions that gain a vim binding when
    /// [`VimMode::Enabled`](crate::VimMode::Enabled). Default empty.
    /// Consumed by the keymap builder, which appends each
    /// `(action, key)` pair to the pane's scope after the TOML overlay.
    #[must_use]
    fn vim_extras() -> &'static [(Self::Actions, KeyBind)] { &[] }

    /// Free-function dispatcher. The framework calls
    /// `Self::dispatcher()(action, ctx)` while holding `&mut Ctx`.
    /// Implementations navigate from the `Ctx` root rather than
    /// holding a `&mut self` borrow — the framework never owns a typed
    /// `&PaneStruct` at dispatch time.
    fn dispatcher() -> fn(Self::Actions, &mut Ctx);
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
    use crate::Mode;
    use crate::Pane;
    use crate::ShortcutState;
    use crate::Visibility;
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
        framework:    Framework<Self>,
        app_settings: (),
    }

    impl AppContext for TestApp {
        type AppPaneId = TestPaneId;
        type AppSettings = ();
        type ToastAction = crate::NoToastAction;

        fn framework(&self) -> &Framework<Self> { &self.framework }
        fn framework_mut(&mut self) -> &mut Framework<Self> { &mut self.framework }
        fn app_settings(&self) -> &Self::AppSettings { &self.app_settings }
        fn app_settings_mut(&mut self) -> &mut Self::AppSettings { &mut self.app_settings }
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
            |_action, _ctx| { /* no-op for the test impl */ }
        }
    }

    fn fresh_app() -> TestApp {
        TestApp {
            framework:    Framework::new(FocusedPane::App(TestPaneId::Foo)),
            app_settings: (),
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
    fn default_visibility_returns_visible() {
        let pane = FooPane;
        let app = fresh_app();
        assert_eq!(
            pane.visibility(FooAction::Activate, &app),
            Visibility::Visible,
        );
        assert_eq!(pane.visibility(FooAction::Clean, &app), Visibility::Visible);
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
    fn default_mode_returns_navigable() {
        let app = fresh_app();
        let query: fn(&TestApp) -> Mode<TestApp> = <FooPane as Pane<TestApp>>::mode();
        assert!(matches!(query(&app), Mode::Navigable));
    }

    #[test]
    fn text_input_mode_carries_handler() {
        fn no_op(_key: crate::KeyBind, _ctx: &mut TestApp) {}
        let mode: Mode<TestApp> = Mode::TextInput(no_op);
        assert!(matches!(mode, Mode::TextInput(_)));
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
        assert_eq!(<FooPane as Pane<TestApp>>::APP_PANE_ID, TestPaneId::Foo);
    }
}
