//! `Toasts<Ctx>`: framework-owned typed notification manager.
//!
//! Lives behind [`Framework::toasts`](crate::Framework). The stack is
//! Tab-focusable when [`Toasts::has_active`] returns `true`, mirroring
//! the binary's existing `PaneId::Toasts` arm. Unlike
//! [`KeymapPane`](crate::KeymapPane) and [`SettingsPane`](crate::SettingsPane),
//! [`Toasts`] is **not** an overlay — toasts land silently and only
//! become a focus target when at least one is active.
//!
//! Phase 12 owns the data model only: id allocation, a viewport cursor
//! for focused-toast navigation, three styles, the public push /
//! dismiss / navigation surface, and the consume-while-scrollable
//! cycle-step hook. Lifecycle (timed / task / persistent), tracked
//! items, and rendering land in Phase 22 when cargo-port's
//! `ToastManager` migrates onto this type.

use core::marker::PhantomData;

use crate::Action;
use crate::AppContext;
use crate::BarRegion;
use crate::BarSlot;
use crate::Bindings;
use crate::CycleDirection;
use crate::KeyBind;
use crate::KeyOutcome;
use crate::ListNavigation;
use crate::Mode;

/// Actions reachable on the toast stack's local bar.
///
/// Empty in Phase 12 — toast dismiss flows through
/// [`GlobalAction::Dismiss`](crate::GlobalAction::Dismiss), and
/// focus-internal navigation flows through the app's
/// [`Navigation`](crate::Navigation) scope translated to
/// [`ListNavigation`]. Phase 20 adds `Activate` for tracked-item
/// activation.
///
/// Hand-rolled because [`crate::action_enum!`] requires ≥1 variant;
/// every method on the empty enum is unreachable, expressed via the
/// exhaustive `match self {}` form.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ToastsAction {}

impl Action for ToastsAction {
    const ALL: &'static [Self] = &[];

    fn toml_key(self) -> &'static str { match self {} }
    fn bar_label(self) -> &'static str { match self {} }
    fn description(self) -> &'static str { match self {} }
    fn from_toml_key(_key: &str) -> Option<Self> { None }
}

impl core::fmt::Display for ToastsAction {
    fn fmt(&self, _f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        // ToastsAction has no variants; this method cannot be called.
        Ok(())
    }
}

/// Stable handle for a toast in the framework's manager.
///
/// Returned by [`Toasts::push`] / [`Toasts::push_styled`]. Pass to
/// [`Toasts::dismiss`] to remove the matching entry. The handle stays
/// valid until the toast is removed; the manager never reuses an id.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ToastId(u64);

/// Visual severity of a toast.
///
/// Closed enum so the renderer (Phase 22) maps each variant to its
/// color in one place.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ToastStyle {
    /// Default informational toast.
    Normal,
    /// Warning — non-fatal but worth attention.
    Warning,
    /// Error — something failed.
    Error,
}

/// One typed notification record.
///
/// Generic over `Ctx` from Phase 12 so future phases (Phase 20 adds
/// `action`, Phase 22 adds the lifecycle / tracked-item fields) can
/// extend the field set without renaming. The type signature does not
/// change across phases.
pub struct Toast<Ctx: AppContext> {
    id:    ToastId,
    title: String,
    body:  String,
    style: ToastStyle,
    _ctx:  PhantomData<fn(&Ctx)>,
}

impl<Ctx: AppContext> Toast<Ctx> {
    /// The toast's stable handle.
    #[must_use]
    pub const fn id(&self) -> ToastId { self.id }

    /// The toast's title line.
    #[must_use]
    pub fn title(&self) -> &str { &self.title }

    /// The toast's body text.
    #[must_use]
    pub fn body(&self) -> &str { &self.body }

    /// The toast's visual severity.
    #[must_use]
    pub const fn style(&self) -> ToastStyle { self.style }
}

/// Framework-owned typed notification manager.
///
/// Held inline on [`Framework<Ctx>`](crate::Framework) and reached via
/// `framework.toasts`. The framework's
/// [`dismiss_framework`](crate::Framework::dismiss_framework) chain
/// pops the focused toast when
/// [`FocusedPane::Framework(FrameworkFocusId::Toasts)`](crate::FocusedPane)
/// is the current focus.
pub struct Toasts<Ctx: AppContext> {
    entries: Vec<Toast<Ctx>>,
    /// Index of the focused toast in [`Self::entries`]. Saturated to
    /// `entries.len() - 1` on every mutation; meaningful only while
    /// [`Self::has_active`] returns `true`.
    cursor:  usize,
    next_id: u64,
    _ctx:    PhantomData<fn(&mut Ctx)>,
}

impl<Ctx: AppContext> Toasts<Ctx> {
    /// Empty manager.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            entries: Vec::new(),
            cursor:  0,
            next_id: 0,
            _ctx:    PhantomData,
        }
    }

    /// Push a [`ToastStyle::Normal`] toast and return its handle.
    pub fn push(&mut self, title: impl Into<String>, body: impl Into<String>) -> ToastId {
        self.push_styled(title, body, ToastStyle::Normal)
    }

    /// Push a styled toast and return its handle.
    pub fn push_styled(
        &mut self,
        title: impl Into<String>,
        body: impl Into<String>,
        style: ToastStyle,
    ) -> ToastId {
        let id = ToastId(self.next_id);
        self.next_id = self.next_id.wrapping_add(1);
        self.entries.push(Toast {
            id,
            title: title.into(),
            body: body.into(),
            style,
            _ctx: PhantomData,
        });
        id
    }

    /// Remove the toast with the given id. Returns `true` if a toast
    /// was removed, `false` if no toast had that id.
    pub fn dismiss(&mut self, id: ToastId) -> bool {
        let Some(idx) = self.entries.iter().position(|t| t.id == id) else {
            return false;
        };
        self.entries.remove(idx);
        self.clamp_cursor();
        true
    }

    /// Remove the focused toast. Returns `true` if a toast was removed,
    /// `false` if the manager was empty.
    pub fn dismiss_focused(&mut self) -> bool {
        if self.entries.is_empty() {
            return false;
        }
        let idx = self.cursor.min(self.entries.len() - 1);
        self.entries.remove(idx);
        self.clamp_cursor();
        true
    }

    /// Handle of the currently focused toast, or `None` when the
    /// manager is empty.
    #[must_use]
    pub fn focused_id(&self) -> Option<ToastId> {
        self.entries
            .get(self.cursor.min(self.entries.len().saturating_sub(1)))
            .map(|t| t.id)
    }

    /// Whether the manager has any active toasts. Drives the
    /// `is_pane_tabbable(Toasts)` gate in
    /// [`focus_cycle`](crate::Framework::pane_order) walks.
    #[must_use]
    pub const fn has_active(&self) -> bool { !self.entries.is_empty() }

    /// Active toasts, oldest first.
    #[must_use]
    pub fn active(&self) -> &[Toast<Ctx>] { &self.entries }

    /// Move the viewport to the first toast — called by `focus_step`
    /// on a `Next`-direction entry into Toasts focus.
    pub const fn reset_to_first(&mut self) { self.cursor = 0; }

    /// Move the viewport to the last toast — called by `focus_step`
    /// on a `Prev`-direction entry into Toasts focus.
    pub const fn reset_to_last(&mut self) { self.cursor = self.entries.len().saturating_sub(1); }

    /// Resolved-nav entry point. Dispatch translates the app's resolved
    /// navigation action via [`Navigation::list_navigation`](crate::Navigation::list_navigation)
    /// (default impl matches the action against the trait's
    /// [`UP`](crate::Navigation::UP) / [`DOWN`](crate::Navigation::DOWN) /
    /// [`HOME`](crate::Navigation::HOME) / [`END`](crate::Navigation::END)
    /// constants) before calling this method. Pure pane-local mutation
    /// (viewport cursor); no `&mut Ctx` borrow needed.
    pub fn on_navigation(&mut self, nav: ListNavigation) -> KeyOutcome {
        if self.entries.is_empty() {
            return KeyOutcome::Unhandled;
        }
        let last = self.entries.len() - 1;
        match nav {
            ListNavigation::Up => self.cursor = self.cursor.saturating_sub(1),
            ListNavigation::Down => self.cursor = (self.cursor + 1).min(last),
            ListNavigation::Home => self.cursor = 0,
            ListNavigation::End => self.cursor = last,
        }
        KeyOutcome::Consumed
    }

    /// Pre-globals hook. Dispatch calls this when the inbound key maps
    /// to [`GlobalAction::NextPane`](crate::GlobalAction::NextPane) /
    /// [`GlobalAction::PrevPane`](crate::GlobalAction::PrevPane) and
    /// Toasts is focused. Returns `true` when there is internal scroll
    /// room (consumes the key, blocks the cycle advance). Mirrors
    /// cargo-port's existing "Tab scrolls within the toast list before
    /// advancing focus" behavior, but driven by the keymap entry for
    /// `NextPane`, not literal `Tab` — so a rebound `NextPane` keeps
    /// the consume-while-scrollable behavior.
    pub const fn try_consume_cycle_step(&mut self, direction: CycleDirection) -> bool {
        if self.entries.is_empty() {
            return false;
        }
        let last = self.entries.len() - 1;
        match direction {
            CycleDirection::Next if self.cursor < last => {
                self.cursor += 1;
                true
            },
            CycleDirection::Prev if self.cursor > 0 => {
                self.cursor -= 1;
                true
            },
            _ => false,
        }
    }

    /// No-op wrapper retained for tests that drive raw key dispatch.
    /// The Phase 12 production path uses [`Self::on_navigation`] +
    /// [`Self::try_consume_cycle_step`]; toast dismiss flows through
    /// [`GlobalAction::Dismiss`](crate::GlobalAction::Dismiss) and
    /// never reaches this method.
    pub const fn handle_key(&mut self, _bind: &KeyBind) -> KeyOutcome { KeyOutcome::Unhandled }

    /// Focused Toasts is [`Mode::Navigable`] — the bar shows the nav
    /// row, the (empty in Phase 12) `PaneAction` row, and the global
    /// strip.
    #[must_use]
    pub const fn mode(&self, _ctx: &Ctx) -> Mode<Ctx> { Mode::Navigable }

    /// Default key bindings for the toast pane's local actions. Empty
    /// in Phase 12 because [`ToastsAction`] is empty; kept as a method
    /// so the bar adapter (Phase 13) can call it uniformly across
    /// framework panes.
    #[must_use]
    pub const fn defaults() -> Bindings<ToastsAction> { Bindings::new() }

    /// Bar slots for the toast pane's local actions. Empty in Phase 12;
    /// Phase 20 adds `Activate`.
    #[must_use]
    pub fn bar_slots(&self, _ctx: &Ctx) -> Vec<(BarRegion, BarSlot<ToastsAction>)> {
        ToastsAction::ALL
            .iter()
            .copied()
            .map(|a| (BarRegion::PaneAction, BarSlot::Single(a)))
            .collect()
    }

    /// Saturate [`Self::cursor`] to the last valid index after a
    /// removal.
    const fn clamp_cursor(&mut self) {
        if self.entries.is_empty() {
            self.cursor = 0;
        } else if self.cursor >= self.entries.len() {
            self.cursor = self.entries.len() - 1;
        }
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
    use super::CycleDirection;
    use super::ListNavigation;
    use super::ToastStyle;
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
        assert!(toasts.focused_id().is_none());
    }

    #[test]
    fn push_returns_unique_ids_and_marks_active() {
        let mut toasts: Toasts<TestApp> = Toasts::new();
        let a = toasts.push("A", "body-a");
        let b = toasts.push("B", "body-b");
        assert_ne!(a, b);
        assert!(toasts.has_active());
        assert_eq!(toasts.active().len(), 2);
    }

    #[test]
    fn push_styled_records_style() {
        let mut toasts: Toasts<TestApp> = Toasts::new();
        let _ = toasts.push_styled("err", "boom", ToastStyle::Error);
        assert_eq!(toasts.active()[0].style(), ToastStyle::Error);
    }

    #[test]
    fn dismiss_by_id_removes_target_and_keeps_others() {
        let mut toasts: Toasts<TestApp> = Toasts::new();
        let a = toasts.push("A", "");
        let b = toasts.push("B", "");
        assert!(toasts.dismiss(a));
        assert_eq!(toasts.active().len(), 1);
        assert_eq!(toasts.active()[0].id(), b);
    }

    #[test]
    fn dismiss_unknown_id_returns_false() {
        let mut toasts: Toasts<TestApp> = Toasts::new();
        let a = toasts.push("A", "");
        assert!(toasts.dismiss(a));
        assert!(!toasts.dismiss(a));
    }

    #[test]
    fn dismiss_focused_pops_at_cursor() {
        let mut toasts: Toasts<TestApp> = Toasts::new();
        let _ = toasts.push("A", "");
        let b = toasts.push("B", "");
        let _ = toasts.on_navigation(ListNavigation::End);
        assert_eq!(toasts.focused_id(), Some(b));
        assert!(toasts.dismiss_focused());
        assert_eq!(toasts.active().len(), 1);
    }

    #[test]
    fn dismiss_focused_on_empty_returns_false() {
        let mut toasts: Toasts<TestApp> = Toasts::new();
        assert!(!toasts.dismiss_focused());
    }

    #[test]
    fn on_navigation_walks_cursor_within_bounds() {
        let mut toasts: Toasts<TestApp> = Toasts::new();
        let a = toasts.push("A", "");
        let _ = toasts.push("B", "");
        let c = toasts.push("C", "");
        assert_eq!(toasts.focused_id(), Some(a));
        assert_eq!(
            toasts.on_navigation(ListNavigation::Down),
            KeyOutcome::Consumed
        );
        assert_eq!(
            toasts.on_navigation(ListNavigation::Down),
            KeyOutcome::Consumed
        );
        assert_eq!(toasts.focused_id(), Some(c));
        // saturates at end
        assert_eq!(
            toasts.on_navigation(ListNavigation::Down),
            KeyOutcome::Consumed
        );
        assert_eq!(toasts.focused_id(), Some(c));
        assert_eq!(
            toasts.on_navigation(ListNavigation::Up),
            KeyOutcome::Consumed
        );
        assert_eq!(
            toasts.on_navigation(ListNavigation::Home),
            KeyOutcome::Consumed
        );
        assert_eq!(toasts.focused_id(), Some(a));
        assert_eq!(
            toasts.on_navigation(ListNavigation::End),
            KeyOutcome::Consumed
        );
        assert_eq!(toasts.focused_id(), Some(c));
    }

    #[test]
    fn on_navigation_on_empty_returns_unhandled() {
        let mut toasts: Toasts<TestApp> = Toasts::new();
        assert_eq!(
            toasts.on_navigation(ListNavigation::Down),
            KeyOutcome::Unhandled,
        );
    }

    #[test]
    fn try_consume_cycle_step_consumes_when_scroll_room() {
        let mut toasts: Toasts<TestApp> = Toasts::new();
        let _ = toasts.push("A", "");
        let _ = toasts.push("B", "");
        // cursor at 0; Next has room, returns true
        assert!(toasts.try_consume_cycle_step(CycleDirection::Next));
        // cursor at 1; Next has no more room, returns false
        assert!(!toasts.try_consume_cycle_step(CycleDirection::Next));
        // cursor at 1; Prev has room, returns true
        assert!(toasts.try_consume_cycle_step(CycleDirection::Prev));
        // cursor at 0; Prev has no room, returns false
        assert!(!toasts.try_consume_cycle_step(CycleDirection::Prev));
    }

    #[test]
    fn try_consume_cycle_step_on_empty_returns_false() {
        let mut toasts: Toasts<TestApp> = Toasts::new();
        assert!(!toasts.try_consume_cycle_step(CycleDirection::Next));
        assert!(!toasts.try_consume_cycle_step(CycleDirection::Prev));
    }

    #[test]
    fn reset_to_first_and_last_set_cursor() {
        let mut toasts: Toasts<TestApp> = Toasts::new();
        let a = toasts.push("A", "");
        let _ = toasts.push("B", "");
        let c = toasts.push("C", "");
        toasts.reset_to_last();
        assert_eq!(toasts.focused_id(), Some(c));
        toasts.reset_to_first();
        assert_eq!(toasts.focused_id(), Some(a));
    }

    #[test]
    fn handle_key_is_unhandled() {
        let mut toasts: Toasts<TestApp> = Toasts::new();
        let _ = toasts.push("A", "");
        let _app = fresh_app();
        assert_eq!(
            toasts.handle_key(&KeyBind::from('z')),
            KeyOutcome::Unhandled,
        );
    }

    #[test]
    fn mode_is_navigable() {
        let toasts: Toasts<TestApp> = Toasts::new();
        let app = fresh_app();
        assert!(matches!(toasts.mode(&app), Mode::Navigable));
    }

    #[test]
    fn bar_slots_is_empty_in_phase_12() {
        let toasts: Toasts<TestApp> = Toasts::new();
        let app = fresh_app();
        assert!(toasts.bar_slots(&app).is_empty());
    }
}
