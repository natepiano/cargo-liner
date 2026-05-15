use super::ToastId;
use super::ToastView;
use super::Toasts;
use crate::AppContext;
use crate::CycleDirection;
use crate::KeyOutcome;
use crate::ListNavigation;

impl<Ctx: AppContext> Toasts<Ctx> {
    /// Return the focused toast identifier.
    #[must_use]
    pub fn focused_id(&self) -> Option<ToastId> { self.focused_toast_id() }

    /// Return the focused toast identifier.
    pub fn focused_toast_id(&self) -> Option<ToastId> {
        self.active_now()
            .get(self.viewport.pos())
            .map(ToastView::id)
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
