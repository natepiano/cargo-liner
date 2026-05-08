//! `Toasts<Ctx>`: transient notification stack.
//!
//! Lives behind [`Framework::toasts`](crate::Framework). The stack is
//! Tab-focusable when non-empty, mirroring the binary's existing
//! `PaneId::Toasts` arm. Unlike [`KeymapPane`](crate::KeymapPane) and
//! [`SettingsPane`](crate::SettingsPane), [`Toasts`] is **not** an
//! overlay — toasts land silently and only become a focus target when
//! [`Self::has_active`] returns `true`.

use core::marker::PhantomData;

use crossterm::event::KeyCode;

use crate::Action;
use crate::AppContext;
use crate::BarRegion;
use crate::BarSlot;
use crate::Bindings;
use crate::KeyBind;
use crate::KeyOutcome;
use crate::Mode;

crate::action_enum! {
    /// Actions reachable on the toast stack's local bar.
    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    pub enum ToastsAction {
        /// Pop the top toast from the stack.
        Dismiss => ("dismiss", "dismiss", "Dismiss top toast");
    }
}

/// Framework-owned transient notification stack.
///
/// Held inline on [`Framework<Ctx>`](crate::Framework) and reached via
/// `framework.toasts`. The framework's
/// [`dismiss`](crate::Framework::dismiss) chain pops the top toast when
/// [`FocusedPane::Framework(Toasts)`](crate::FocusedPane) is the
/// current focus.
pub struct Toasts<Ctx: AppContext> {
    stack: Vec<String>,
    _ctx:  PhantomData<fn(&mut Ctx)>,
}

impl<Ctx: AppContext> Toasts<Ctx> {
    /// Empty toast stack.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            stack: Vec::new(),
            _ctx:  PhantomData,
        }
    }

    /// Push a toast onto the top of the stack.
    pub fn push(&mut self, message: impl Into<String>) { self.stack.push(message.into()); }

    /// Pop the most-recently pushed toast. Returns `true` if a toast
    /// was popped, `false` when the stack was already empty.
    pub fn try_pop_top(&mut self) -> bool { self.stack.pop().is_some() }

    /// Whether the stack has any active toasts. Drives the
    /// `is_pane_tabbable(PaneId::Toasts)` gate in
    /// [`pane_order`](crate::Framework::pane_order) walks (Phase 12+).
    #[must_use]
    pub const fn has_active(&self) -> bool { !self.stack.is_empty() }

    /// Default key bindings for the toast stack's local actions.
    #[must_use]
    pub fn defaults() -> Bindings<ToastsAction> {
        crate::bindings! {
            KeyCode::Esc => ToastsAction::Dismiss,
        }
    }

    /// Consume one keypress. Returns [`KeyOutcome::Consumed`] when the
    /// key matches a [`ToastsAction`] binding (and the action fires);
    /// [`KeyOutcome::Unhandled`] otherwise so the dispatcher continues
    /// down its chain. Toasts is the only framework pane whose
    /// `handle_key` can return [`KeyOutcome::Unhandled`].
    pub fn handle_key(&mut self, _ctx: &mut Ctx, bind: &KeyBind) -> KeyOutcome {
        let map = Self::defaults().into_scope_map();
        if let Some(action) = map.action_for(bind) {
            match action {
                ToastsAction::Dismiss => {
                    let _ = self.try_pop_top();
                    return KeyOutcome::Consumed;
                },
            }
        }
        KeyOutcome::Unhandled
    }

    /// Toasts is always a [`Mode::Static`] pane — there is nothing to
    /// scroll and no text-entry state.
    #[must_use]
    pub const fn mode(&self, _ctx: &Ctx) -> Mode<Ctx> { Mode::Static }

    /// Bar slots for the toast stack's local actions. The bar renderer
    /// (Phase 12) consults this when
    /// [`FocusedPane::Framework(Toasts)`](crate::FocusedPane) is the
    /// current focus.
    #[must_use]
    pub fn bar_slots(&self, _ctx: &Ctx) -> Vec<(BarRegion, BarSlot<ToastsAction>)> {
        ToastsAction::ALL
            .iter()
            .copied()
            .map(|a| (BarRegion::PaneAction, BarSlot::Single(a)))
            .collect()
    }
}

impl<Ctx: AppContext> Default for Toasts<Ctx> {
    fn default() -> Self { Self::new() }
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

    use super::Toasts;
    use crate::AppContext;
    use crate::FocusedPane;
    use crate::Framework;
    use crate::KeyBind;
    use crate::KeyOutcome;
    use crate::Mode;

    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    enum TestPaneId {
        Foo,
    }

    struct TestApp {
        framework: Framework<Self>,
    }

    impl AppContext for TestApp {
        type AppPaneId = TestPaneId;

        fn framework(&self) -> &Framework<Self> { &self.framework }
        fn framework_mut(&mut self) -> &mut Framework<Self> { &mut self.framework }
    }

    fn fresh_app() -> TestApp {
        TestApp {
            framework: Framework::new(FocusedPane::App(TestPaneId::Foo)),
        }
    }

    #[test]
    fn new_is_empty_and_not_active() {
        let toasts: Toasts<TestApp> = Toasts::new();
        assert!(!toasts.has_active());
    }

    #[test]
    fn push_then_pop_round_trips() {
        let mut toasts: Toasts<TestApp> = Toasts::new();
        toasts.push("hello");
        assert!(toasts.has_active());
        assert!(toasts.try_pop_top());
        assert!(!toasts.has_active());
    }

    #[test]
    fn try_pop_top_on_empty_returns_false() {
        let mut toasts: Toasts<TestApp> = Toasts::new();
        assert!(!toasts.try_pop_top());
    }

    #[test]
    fn handle_key_dismiss_pops_top_and_returns_consumed() {
        let mut toasts: Toasts<TestApp> = Toasts::new();
        toasts.push("hello");
        let mut app = fresh_app();
        let outcome = toasts.handle_key(&mut app, &KeyBind::from(KeyCode::Esc));
        assert_eq!(outcome, KeyOutcome::Consumed);
        assert!(!toasts.has_active());
    }

    #[test]
    fn handle_key_unmatched_returns_unhandled() {
        let mut toasts: Toasts<TestApp> = Toasts::new();
        toasts.push("hello");
        let mut app = fresh_app();
        let outcome = toasts.handle_key(&mut app, &KeyBind::from('z'));
        assert_eq!(outcome, KeyOutcome::Unhandled);
        assert!(toasts.has_active(), "non-matching key must not pop");
    }

    #[test]
    fn mode_is_always_static() {
        let toasts: Toasts<TestApp> = Toasts::new();
        let app = fresh_app();
        assert!(matches!(toasts.mode(&app), Mode::Static));
    }
}
