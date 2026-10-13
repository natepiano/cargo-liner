use super::ToastId;
use super::Toasts;
use crate::AppContext;
use crate::CycleDirection;
use crate::KeyOutcome;
use crate::ListNavigation;
use crate::PaneFocusState;

/// Focus of the toast pane and the card selected inside it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToastFocus {
    /// The toast pane does not hold focus.
    PaneUnfocused,
    /// The toast pane holds focus without a selected card.
    PaneFocusedWithoutToast,
    /// The toast pane holds focus on this card.
    PaneFocusedOn(ToastId),
}

impl<Ctx: AppContext> Toasts<Ctx> {
    /// Return toast focus using the embedding's current pane focus.
    #[must_use]
    pub fn focused_id(&self, pane: PaneFocusState) -> ToastFocus { self.focused_toast_id(pane) }

    /// Return toast focus using the embedding's current pane focus.
    #[must_use]
    pub fn focused_toast_id(&self, pane: PaneFocusState) -> ToastFocus {
        if pane != PaneFocusState::Active {
            return ToastFocus::PaneUnfocused;
        }
        self.active_now()
            .get(self.viewport.pos())
            .map_or(ToastFocus::PaneFocusedWithoutToast, |toast| {
                ToastFocus::PaneFocusedOn(toast.id())
            })
    }

    /// Move the toast viewport to the first toast when one exists.
    pub fn reset_to_first(&mut self) {
        if self.has_active() {
            self.viewport.set_pos(0);
        }
    }

    /// Move the toast viewport to the last toast when one exists.
    pub fn reset_to_last(&mut self) {
        let len = self.active_now().len();
        if len > 0 {
            self.viewport.set_pos(len - 1);
        }
    }

    /// Apply list navigation while focus is on the Toasts pane.
    pub fn on_navigation(&mut self, nav: ListNavigation) -> KeyOutcome {
        let len = self.active_now().len();
        if len == 0 {
            return KeyOutcome::Unhandled;
        }
        self.viewport.set_len(len);
        match nav {
            ListNavigation::Up => self.viewport.up(),
            ListNavigation::Down => self.viewport.down(),
            ListNavigation::Home => self.viewport.home(),
            ListNavigation::End => self.viewport.end(),
            ListNavigation::PageUp => self.viewport.page_up(),
            ListNavigation::PageDown => self.viewport.page_down(),
            ListNavigation::HalfPageUp => self.viewport.half_page_up(),
            ListNavigation::HalfPageDown => self.viewport.half_page_down(),
        }
        KeyOutcome::Consumed
    }

    /// Consume a focus-cycle step as toast scrolling when possible.
    pub fn try_consume_cycle_step(&mut self, direction: CycleDirection) -> bool {
        let len = self.active_now().len();
        self.viewport.set_len(len);
        match direction {
            CycleDirection::Next if self.viewport.pos() + 1 < len => {
                self.viewport.down();
                true
            },
            CycleDirection::Prev if self.viewport.pos() > 0 => {
                self.viewport.up();
                true
            },
            CycleDirection::Next | CycleDirection::Prev => false,
        }
    }
}
