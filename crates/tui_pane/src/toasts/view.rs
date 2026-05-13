use ratatui::layout::Rect;

use super::ToastId;
use super::ToastStyle;
use super::TrackedItemView;

/// Render-ready view of a toast.
#[derive(Clone, Debug)]
pub struct ToastView {
    pub(super) id:              ToastId,
    pub(super) title:           String,
    pub(super) body:            String,
    pub(super) style:           ToastStyle,
    pub(super) has_action:      bool,
    pub(super) linger_progress: Option<f32>,
    pub(super) remaining_secs:  Option<u64>,
    pub(super) tracked_items:   Vec<TrackedItemView>,
    pub(super) min_height:      u16,
    pub(super) desired_height:  u16,
}

impl ToastView {
    /// Return this toast's identifier.
    #[must_use]
    pub const fn id(&self) -> ToastId { self.id }

    /// Return this toast's title.
    #[must_use]
    pub fn title(&self) -> &str { &self.title }

    /// Return this toast's body text.
    #[must_use]
    pub fn body(&self) -> &str { &self.body }

    /// Return this toast's style.
    #[must_use]
    pub const fn style(&self) -> ToastStyle { self.style }

    /// Return whether Enter can activate an action for this toast.
    #[must_use]
    pub const fn has_action(&self) -> bool { self.has_action }

    /// Return task linger progress from 0.0 to 1.0, if finished.
    #[must_use]
    pub const fn linger_progress(&self) -> Option<f32> { self.linger_progress }

    /// Return remaining seconds for a timed toast, if applicable.
    #[must_use]
    pub const fn remaining_secs(&self) -> Option<u64> { self.remaining_secs }

    /// Return the tracked items rendered in this toast.
    #[must_use]
    pub fn tracked_items(&self) -> &[TrackedItemView] { &self.tracked_items }

    /// Return the minimum card height needed to display this toast.
    #[must_use]
    pub const fn min_height(&self) -> u16 { self.min_height }

    /// Return the desired card height when space is available.
    #[must_use]
    pub const fn desired_height(&self) -> u16 { self.desired_height }
}

/// Click target geometry for one rendered toast.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ToastHitbox {
    /// Toast hit by this geometry.
    pub id:         ToastId,
    /// Full toast-card rectangle.
    pub card_rect:  Rect,
    /// Close-button rectangle.
    pub close_rect: Rect,
}
