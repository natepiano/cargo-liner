use crossterm::event::KeyCode;
use ratatui::layout::Position;

use super::ToastActivation;
use super::ToastCommand;
use super::ToastFocus;
use super::ToastHit;
use super::ToastHitbox;
use super::Toasts;
use super::ToastsAction;
use crate::AppContext;
use crate::BarRegion;
use crate::BarSlot;
use crate::Bindings;
use crate::Hittable;
use crate::KeyBind;
use crate::KeyOutcome;
use crate::Mode;
use crate::PaneFocusState;
use crate::keymap::Action;

impl<Ctx: AppContext> Toasts<Ctx> {
    /// Return the Toasts pane mode.
    #[must_use]
    pub const fn mode(&self, _: &Ctx) -> Mode<Ctx> { Mode::Navigable }

    /// Return default Toasts-pane bindings.
    #[must_use]
    pub fn defaults() -> Bindings<ToastsAction> {
        crate::bindings! {
            KeyCode::Enter => ToastsAction::Activate,
        }
    }

    /// Return status-bar slots for the Toasts pane.
    #[must_use]
    pub fn bar_slots(&self, _: &Ctx) -> Vec<(BarRegion, BarSlot<ToastsAction>)> {
        ToastsAction::ALL
            .iter()
            .copied()
            .map(|toasts_action| (BarRegion::PaneAction, BarSlot::Single(toasts_action)))
            .collect()
    }

    /// Return hitboxes from the last toast render pass.
    #[must_use]
    pub fn hits(&self) -> &[ToastHitbox] { &self.hits }

    /// Replace hitboxes from the latest toast render pass.
    pub fn set_hits(&mut self, hits: Vec<ToastHitbox>) { self.hits = hits; }

    /// Handle a key and return both key outcome and toast action command.
    pub fn handle_key_command(
        &mut self,
        key_bind: &KeyBind,
        pane_focus_state: PaneFocusState,
    ) -> (KeyOutcome, ToastCommand<Ctx::ToastAction>) {
        let scope_map = Self::defaults().into_scope_map();
        if scope_map.action_for(key_bind) != Some(ToastsAction::Activate) {
            return (KeyOutcome::Unhandled, ToastCommand::None);
        }

        let ToastFocus::PaneFocusedOn(toast_id) = self.focused_toast_id(pane_focus_state) else {
            return (KeyOutcome::Unhandled, ToastCommand::None);
        };
        let Some(toast) = self.entries.iter().find(|toast| toast.id == toast_id) else {
            return (KeyOutcome::Unhandled, ToastCommand::None);
        };
        let ToastActivation::Opens(toast_action) = toast.action.clone() else {
            return (KeyOutcome::Unhandled, ToastCommand::None);
        };
        (KeyOutcome::Consumed, ToastCommand::Activate(toast_action))
    }

    /// Handle a key and return only the key outcome.
    pub fn handle_key(&mut self, key_bind: &KeyBind, pane: PaneFocusState) -> KeyOutcome {
        self.handle_key_command(key_bind, pane).0
    }
}

impl<Ctx: AppContext> Hittable<ToastHit> for Toasts<Ctx> {
    /// Walk hitboxes top-to-bottom (latest-rendered first) and return
    /// the matching toast hit. The close button takes priority over
    /// the card body within a single toast.
    fn hit_test_at(&self, position: Position) -> Option<ToastHit> {
        for toast_hitbox in self.hits.iter().rev() {
            if toast_hitbox.close_rect.contains(position) {
                return Some(ToastHit::Close(toast_hitbox.id));
            }
            if toast_hitbox.card_rect.contains(position) {
                return Some(ToastHit::Card(toast_hitbox.id));
            }
        }
        None
    }
}
