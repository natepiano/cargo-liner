mod action;
mod body;
mod commands;
mod format;
mod ids;
mod item;
mod lifecycle;
mod navigation;
mod render;
mod running_tracker;
mod settings;
mod slots;
mod toast;
mod view;

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "tests should fail loudly on unexpected values"
)]
mod tests;

use std::time::Duration;

pub use action::ToastsAction;
pub use body::ToastBody;
pub use body::toast_body_width;
pub use format::format_toast_items;
pub use ids::ToastId;
pub use ids::ToastTaskId;
pub use item::TrackedItem;
pub use item::TrackedItemKey;
pub use item::TrackedItemView;
pub use render::ToastsRenderCtx;
pub use running_tracker::RunningTracker;
pub use settings::ToastDuration;
pub use settings::ToastPlacement;
pub use settings::ToastSettings;
pub(crate) use settings::remove_legacy_toast_keys;
pub use toast::Toast;
use toast::ToastLifetime;
pub use toast::ToastStyle;
pub use view::ToastHit;
pub use view::ToastHitbox;
pub use view::ToastView;

use crate::AppContext;
use crate::Viewport;

/// Result of handling a focused toast key.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ToastCommand<A> {
    /// No toast action fired.
    None,
    /// The focused toast requested its action payload.
    Activate(A),
}

struct ToastSpec<Ctx: AppContext> {
    title:              String,
    body:               ToastBody,
    style:              ToastStyle,
    lifetime:           ToastLifetime,
    action:             Option<Ctx::ToastAction>,
    min_interior_lines: usize,
    item_linger:        Duration,
}

/// Outcome of [`Toasts::reactivate_task`].
///
/// Replaces a plain `bool` so callers can distinguish "no toast
/// for this task — create one" from "user dismissed this toast
/// — leave it alone." `bool` returns conflated those cases and
/// caused user-dismissed toasts to be re-created on the next
/// tracker poll.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReactivateOutcome {
    /// No toast registered for this task id. Caller should
    /// create a fresh toast for the tracker.
    NotFound,
    /// Toast existed and was returned to
    /// [`toast::ToastPhase::Visible`] with task status reset to
    /// `Running`.
    Revived,
    /// Toast existed but its dismissal is
    /// [`toast::ToastDismissal::ClosedByUser`]. Caller should neither
    /// touch the toast nor create a replacement — the user
    /// closed it, and the underlying tracker work continues
    /// without UI surface.
    DismissedByUser,
}

/// Framework-owned toast manager.
pub struct Toasts<Ctx: AppContext> {
    next_id:      u64,
    entries:      Vec<Toast<Ctx>>,
    /// Viewport used when focus is on the Toasts framework pane.
    pub viewport: Viewport,
    hits:         Vec<ToastHitbox>,
    settings:     ToastSettings,
}

impl<Ctx: AppContext> Default for Toasts<Ctx> {
    fn default() -> Self { Self::new() }
}

impl<Ctx: AppContext> Toasts<Ctx> {
    /// Create an empty toast manager with default settings.
    #[must_use]
    pub fn new() -> Self { Self::with_settings(ToastSettings::default()) }

    /// Create an empty toast manager with explicit settings.
    #[must_use]
    pub fn with_settings(settings: ToastSettings) -> Self {
        Self {
            next_id: 1,
            entries: Vec::new(),
            viewport: Viewport::default(),
            hits: Vec::new(),
            settings,
        }
    }

    /// Borrow the toast settings.
    #[must_use]
    pub const fn settings(&self) -> &ToastSettings { &self.settings }

    /// Mutably borrow the toast settings.
    pub const fn settings_mut(&mut self) -> &mut ToastSettings { &mut self.settings }

    /// Replace the toast settings.
    pub const fn set_settings(&mut self, settings: ToastSettings) { self.settings = settings; }

    fn sync_viewport_len(&mut self) {
        let len = self.active_now().len();
        self.viewport.set_len(len);
        if len == 0 {
            self.viewport.set_pos(0);
        } else if self.viewport.pos() >= len {
            self.viewport.set_pos(len - 1);
        }
    }
}
