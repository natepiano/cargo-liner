//! [`VisualDeadline`]: the earliest moment the app has asked the loop
//! to wake for without an event behind it.

use std::time::Duration;
use std::time::Instant;

use crate::ToastVisualDeadline;

/// Earliest app-owned visual transition that can require another frame.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VisualDeadline {
    /// No app-owned transition is waiting on time alone.
    NoVisualChangeScheduled,
    /// Wake no later than this instant.
    At(Instant),
}

impl VisualDeadline {
    /// Whichever of the two falls first, where a side with nothing
    /// scheduled never wins.
    #[must_use]
    pub fn earlier(self, other: Self) -> Self {
        match (self, other) {
            (Self::NoVisualChangeScheduled, deadline)
            | (deadline, Self::NoVisualChangeScheduled) => deadline,
            (Self::At(left), Self::At(right)) => Self::At(left.min(right)),
        }
    }

    /// Cut `wait` short so the loop wakes no later than this deadline.
    pub(super) fn limit_wait(self, now: Instant, wait: Duration) -> Duration {
        match self {
            Self::NoVisualChangeScheduled => wait,
            Self::At(deadline) => wait.min(deadline.saturating_duration_since(now)),
        }
    }
}

impl From<ToastVisualDeadline> for VisualDeadline {
    fn from(toast_visual_deadline: ToastVisualDeadline) -> Self {
        match toast_visual_deadline {
            ToastVisualDeadline::NoVisualChangeScheduled => Self::NoVisualChangeScheduled,
            ToastVisualDeadline::At(deadline) => Self::At(deadline),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;
    use std::time::Instant;

    use super::VisualDeadline;

    #[test]
    fn earlier_takes_the_first_instant_and_skips_an_empty_side() {
        let now = Instant::now();
        let soon = VisualDeadline::At(now + Duration::from_millis(5));
        let later = VisualDeadline::At(now + Duration::from_millis(50));
        assert_eq!(soon.earlier(later), soon);
        assert_eq!(later.earlier(soon), soon);
        assert_eq!(
            VisualDeadline::NoVisualChangeScheduled.earlier(later),
            later
        );
        assert_eq!(
            later.earlier(VisualDeadline::NoVisualChangeScheduled),
            later
        );
        assert_eq!(
            VisualDeadline::NoVisualChangeScheduled
                .earlier(VisualDeadline::NoVisualChangeScheduled),
            VisualDeadline::NoVisualChangeScheduled
        );
    }

    #[test]
    fn a_deadline_shortens_the_wait_and_never_lengthens_it() {
        let now = Instant::now();
        let wait = Duration::from_millis(8);
        assert_eq!(
            VisualDeadline::NoVisualChangeScheduled.limit_wait(now, wait),
            wait
        );
        assert_eq!(
            VisualDeadline::At(now + Duration::from_millis(3)).limit_wait(now, wait),
            Duration::from_millis(3)
        );
        assert_eq!(
            VisualDeadline::At(now + Duration::from_millis(30)).limit_wait(now, wait),
            wait
        );
        assert_eq!(
            VisualDeadline::At(now).limit_wait(now + Duration::from_millis(1), wait),
            Duration::ZERO
        );
    }
}
