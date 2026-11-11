//! [`VisualDeadline`]: the earliest moment the app has asked the loop
//! to wake for without an event behind it. `StatusDeadline` supplies the
//! status line's whole-second deadline.

use std::time::Duration;
use std::time::Instant;

use super::app::Repaint;
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
    pub fn earlier(self, other_visual_deadline: Self) -> Self {
        match (self, other_visual_deadline) {
            (Self::NoVisualChangeScheduled, visual_deadline)
            | (visual_deadline, Self::NoVisualChangeScheduled) => visual_deadline,
            (Self::At(left_instant), Self::At(right)) => Self::At(left_instant.min(right)),
        }
    }

    /// Cut `wait` short so the loop wakes no later than this deadline.
    pub(super) fn limit_wait(self, now: Instant, wait: Duration) -> Duration {
        match self {
            Self::NoVisualChangeScheduled => wait,
            Self::At(deadline_instant) => wait.min(deadline_instant.saturating_duration_since(now)),
        }
    }
}

impl From<ToastVisualDeadline> for VisualDeadline {
    fn from(toast_visual_deadline: ToastVisualDeadline) -> Self {
        match toast_visual_deadline {
            ToastVisualDeadline::NoVisualChangeScheduled => Self::NoVisualChangeScheduled,
            ToastVisualDeadline::At(deadline_instant) => Self::At(deadline_instant),
        }
    }
}

/// The next whole second of the framework uptime, when the status line changes.
pub(crate) struct StatusDeadline {
    next: Instant,
}

impl StatusDeadline {
    pub(crate) fn new(start: Instant) -> Self {
        Self {
            next: start + Duration::from_secs(1),
        }
    }

    pub(crate) const fn visual_deadline(&self) -> VisualDeadline { VisualDeadline::At(self.next) }

    /// Move to the next whole second after `now` when the deadline has passed.
    pub(crate) fn take_tick(&mut self, now: Instant) -> Repaint {
        if now < self.next {
            return Repaint::NotNeeded;
        }
        let seconds = now.duration_since(self.next).as_secs() + 1;
        self.next += Duration::from_secs(seconds);
        Repaint::Needed
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;
    use std::time::Instant;

    use super::Repaint;
    use super::StatusDeadline;
    use super::VisualDeadline;

    #[test]
    fn the_status_deadline_falls_on_each_uptime_second() {
        let start = Instant::now();
        let mut status = StatusDeadline::new(start);
        for second in 1..=4 {
            let due = start + Duration::from_secs(second);
            let before =
                start + Duration::from_secs(second).saturating_sub(Duration::from_nanos(1));
            assert_eq!(status.visual_deadline(), VisualDeadline::At(due));
            assert_eq!(status.take_tick(before), Repaint::NotNeeded);
            assert_eq!(
                status.take_tick(due + Duration::from_millis(3)),
                Repaint::Needed
            );
            assert_eq!(
                status.take_tick(due + Duration::from_millis(3)),
                Repaint::NotNeeded
            );
        }
    }

    #[test]
    fn earlier_takes_the_first_instant_and_skips_an_empty_side() {
        let now = Instant::now();
        let soon_visual_deadline = VisualDeadline::At(now + Duration::from_millis(5));
        let later_visual_deadline = VisualDeadline::At(now + Duration::from_millis(50));
        assert_eq!(
            soon_visual_deadline.earlier(later_visual_deadline),
            soon_visual_deadline
        );
        assert_eq!(
            later_visual_deadline.earlier(soon_visual_deadline),
            soon_visual_deadline
        );
        assert_eq!(
            VisualDeadline::NoVisualChangeScheduled.earlier(later_visual_deadline),
            later_visual_deadline
        );
        assert_eq!(
            later_visual_deadline.earlier(VisualDeadline::NoVisualChangeScheduled),
            later_visual_deadline
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
        let wait_duration = Duration::from_millis(8);
        assert_eq!(
            VisualDeadline::NoVisualChangeScheduled.limit_wait(now, wait_duration),
            wait_duration
        );
        assert_eq!(
            VisualDeadline::At(now + Duration::from_millis(3)).limit_wait(now, wait_duration),
            Duration::from_millis(3)
        );
        assert_eq!(
            VisualDeadline::At(now + Duration::from_millis(30)).limit_wait(now, wait_duration),
            wait_duration
        );
        assert_eq!(
            VisualDeadline::At(now).limit_wait(now + Duration::from_millis(1), wait_duration),
            Duration::ZERO
        );
    }
}
