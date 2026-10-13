use ratatui::layout::Rect;

use super::ToastActivation;
use super::ToastId;
use super::ToastStyle;
use super::TrackedItemView;
use super::body::ToastRenderBody;

/// Whether a toast's body is live or fading after its task finished.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ToastLingerState {
    /// The body uses its live style.
    Live,
    /// The completed task's body fades through its linger period.
    Lingering {
        /// Fade progress from 0.0 to 1.0.
        progress: f32,
    },
}

/// Whether a toast has a closing countdown to display.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToastCountdownState {
    /// The toast has no timed closing policy.
    NoCountdown,
    /// The toast's closing deadline has not passed.
    CountingDown {
        /// Seconds until closing, rounded up.
        remaining_secs: u64,
    },
    /// The closing deadline has passed; no countdown is displayed.
    Expired,
}

/// Render-ready view of a toast.
#[derive(Clone, Debug)]
pub struct ToastView {
    pub(super) id:             ToastId,
    pub(super) title:          String,
    pub(super) body:           String,
    pub(super) render_body:    ToastRenderBody,
    pub(super) style:          ToastStyle,
    pub(super) activation:     ToastActivation<()>,
    pub(super) linger:         ToastLingerState,
    pub(super) countdown:      ToastCountdownState,
    pub(super) tracked_items:  Vec<TrackedItemView>,
    pub(super) min_height:     u16,
    pub(super) desired_height: u16,
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
    pub const fn has_action(&self) -> bool { matches!(self.activation, ToastActivation::Opens(())) }

    /// Return whether this toast's body is live or lingering.
    #[must_use]
    pub const fn linger(&self) -> ToastLingerState { self.linger }

    /// Return this toast's closing countdown state.
    #[must_use]
    pub const fn countdown(&self) -> ToastCountdownState { self.countdown }

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
