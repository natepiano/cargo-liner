use ratatui::layout::Rect;
use ratatui::style::Color;

use super::ToastId;
use super::ToastStyle;
use super::TrackedItemView;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ToastActionState {
    Available,
    None,
}

impl ToastActionState {
    const fn has_action(self) -> bool { matches!(self, Self::Available) }
}

impl From<bool> for ToastActionState {
    fn from(has_action: bool) -> Self {
        if has_action {
            Self::Available
        } else {
            Self::None
        }
    }
}

/// Render-ready view of a toast.
#[derive(Clone, Debug)]
pub struct ToastView {
    pub(super) id:               ToastId,
    pub(super) title:            String,
    pub(super) body:             String,
    /// One foreground color per body line, when the toast body carries them
    /// (the multi-row startup panel). `None` falls back to the uniform body
    /// style.
    pub(super) body_line_colors: Option<Vec<Color>>,
    pub(super) style:            ToastStyle,
    pub(super) action_state:     ToastActionState,
    pub(super) linger_progress:  Option<f32>,
    pub(super) remaining_secs:   Option<u64>,
    pub(super) tracked_items:    Vec<TrackedItemView>,
    pub(super) min_height:       u16,
    pub(super) desired_height:   u16,
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

    /// Per-line body foreground colors, when the body carries them.
    #[must_use]
    pub(super) fn body_line_colors(&self) -> Option<&[Color]> { self.body_line_colors.as_deref() }

    /// Return this toast's style.
    #[must_use]
    pub const fn style(&self) -> ToastStyle { self.style }

    /// Return whether Enter can activate an action for this toast.
    #[must_use]
    pub const fn has_action(&self) -> bool { self.action_state.has_action() }

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

/// Result of `Hittable::hit_test_at` on the toast stack.
///
/// The close-button rectangle takes priority over the card body so a
/// click on the X never accidentally fires the card-card behavior.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToastHit {
    /// Click landed on a toast's close button.
    Close(ToastId),
    /// Click landed on a toast's card body (not the close button).
    Card(ToastId),
}
