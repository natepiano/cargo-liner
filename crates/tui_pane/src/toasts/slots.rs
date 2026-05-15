use crossterm::event::KeyCode;
use ratatui::layout::Position;

use super::ToastCommand;
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
use crate::keymap::Action;

impl<Ctx: AppContext> Toasts<Ctx> {
    /// Return the Toasts pane mode.
    pub const fn mode(&self, _ctx: &Ctx) -> Mode<Ctx> { Mode::Navigable }

    /// Return default Toasts-pane bindings.
    #[must_use]
    pub fn defaults() -> Bindings<ToastsAction> {
        crate::bindings! {
            KeyCode::Enter => ToastsAction::Activate,
        }
    }

    /// Return status-bar slots for the Toasts pane.
    pub fn bar_slots(&self, _ctx: &Ctx) -> Vec<(BarRegion, BarSlot<ToastsAction>)> {
        ToastsAction::ALL
            .iter()
            .copied()
            .map(|action| (BarRegion::PaneAction, BarSlot::Single(action)))
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
        bind: &KeyBind,
    ) -> (KeyOutcome, ToastCommand<Ctx::ToastAction>) {
        let scope = Self::defaults().into_scope_map();
        if scope.action_for(bind) != Some(ToastsAction::Activate) {
            return (KeyOutcome::Unhandled, ToastCommand::None);
        }

        let Some(id) = self.focused_toast_id() else {
            return (KeyOutcome::Unhandled, ToastCommand::None);
        };
        let Some(action) = self
            .entries
            .iter()
            .find(|toast| toast.id == id)
            .and_then(|toast| toast.action.clone())
        else {
            return (KeyOutcome::Unhandled, ToastCommand::None);
        };
        (KeyOutcome::Consumed, ToastCommand::Activate(action))
    }

    /// Handle a key and return only the key outcome.
    pub fn handle_key(&mut self, bind: &KeyBind) -> KeyOutcome { self.handle_key_command(bind).0 }
}

impl<Ctx: AppContext> Hittable<ToastHit> for Toasts<Ctx> {
    /// Walk hitboxes top-to-bottom (latest-rendered first) and return
    /// the matching toast hit. The close button takes priority over
    /// the card body within a single toast.
    fn hit_test_at(&self, pos: Position) -> Option<ToastHit> {
        for hit in self.hits.iter().rev() {
            if hit.close_rect.contains(pos) {
                return Some(ToastHit::Close(hit.id));
            }
            if hit.card_rect.contains(pos) {
                return Some(ToastHit::Card(hit.id));
            }
        }
        None
    }
}
