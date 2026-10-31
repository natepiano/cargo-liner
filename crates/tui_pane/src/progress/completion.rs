//! Shared phase lifecycle and completion protocol.

use std::time::Duration;
use std::time::Instant;

use super::Percentage;

/// A startup phase has one outcome at a time. A timeout means slow work,
/// so its late result may still complete; other failures are final until reset.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum PhaseLifecycle {
    /// Work has not reached a terminal outcome.
    #[default]
    Running,
    /// Work completed at the recorded instant.
    Completed {
        /// First completion timestamp.
        at: Instant,
    },
    /// Work failed; only timeouts can later complete.
    Failed {
        /// Cause of this failure.
        reason: FailureReason,
    },
}

impl PhaseLifecycle {
    const fn complete_once(&mut self, at: Instant) -> bool {
        match self {
            Self::Running
            | Self::Failed {
                reason: FailureReason::Timeout(_),
            } => {
                *self = Self::Completed { at };
                true
            },
            Self::Completed { .. } | Self::Failed { .. } => false,
        }
    }

    /// Fail running work without overwriting an existing outcome.
    pub const fn fail(&mut self, reason: FailureReason) {
        match self {
            Self::Running => *self = Self::Failed { reason },
            Self::Completed { .. } | Self::Failed { .. } => {},
        }
    }

    /// New obligations reopen a completed row, but never erase a failure.
    pub const fn reopen(&mut self) {
        match self {
            Self::Completed { .. } => *self = Self::Running,
            Self::Running | Self::Failed { .. } => {},
        }
    }

    /// Fail running work strictly after its timeout, returning elapsed time on transition.
    pub fn time_out(
        &mut self,
        now: Instant,
        first_seen: Option<Instant>,
        timeout: Duration,
    ) -> Option<Duration> {
        match self {
            Self::Running => {
                let elapsed = now.duration_since(first_seen?);
                if elapsed <= timeout {
                    return None;
                }
                self.fail(FailureReason::Timeout(elapsed));
                Some(elapsed)
            },
            Self::Completed { .. } | Self::Failed { .. } => None,
        }
    }
}

/// Why a phase failed.
///
/// Timeouts can recover when work lands; other reasons
/// remain final until reset. A failed row renders a stalled
/// marker; the reason is carried for the accompanying warning toast and for
/// logging. (`cargo metadata` errors are not represented here — that handler
/// already toasts and counts the workspace as done, so the row never stalls.)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureReason {
    /// A remote service rate-limited the work.
    RateLimited,
    /// Retrieval failed permanently until reset.
    FetchError,
    /// Work exceeded its deadline after this elapsed time.
    Timeout(Duration),
}

/// The render state of one startup row.
///
/// `Active` carries a clamped
/// percentage; `CompleteHeld` is 100% held open by the minimum-visible
/// floor; `Waiting` is an indeterminate denominator; `Failed` is a
/// stall (recoverable for timeouts).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgressState {
    /// Determinate progress at the given percentage.
    Active(Percentage),
    /// Complete work held visible until its minimum duration elapses.
    CompleteHeld,
    /// The denominator is still resolving.
    Waiting,
    /// Work failed or timed out.
    Failed,
}

/// Shared completion and panel-gating behavior for keyed, counted and
/// language phases. Callers can track each kind through the same lifecycle.
pub trait PhaseCompletion {
    /// `true` once `expected` is set and `seen` has caught up to it.
    fn is_complete(&self) -> bool;

    /// Current phase outcome.
    fn lifecycle(&self) -> PhaseLifecycle;

    /// Mutable phase outcome for completion transitions.
    fn lifecycle_mut(&mut self) -> &mut PhaseLifecycle;

    /// First completion instant, when complete.
    fn complete_at(&self) -> Option<Instant> {
        match self.lifecycle() {
            PhaseLifecycle::Completed { at } => Some(at),
            PhaseLifecycle::Running | PhaseLifecycle::Failed { .. } => None,
        }
    }

    /// `true` when this phase renders no row (no denominator known yet),
    /// so it never holds the dismissal gate.
    fn is_omitted(&self) -> bool;

    /// When the row first became visible — drives the minimum-visible floor.
    fn first_seen(&self) -> Option<Instant>;

    /// `true` while the phase is failed, including a recoverable timeout.
    fn is_failed(&self) -> bool { matches!(self.lifecycle(), PhaseLifecycle::Failed { .. }) }

    /// The row's render state, or `None` when the row is omitted.
    fn progress_state(&self, now: Instant, min_visible: Duration) -> Option<ProgressState>;

    /// Complete finished work once, including late work after a timeout.
    /// Returns `false` for unfinished work, an existing completion, or a
    /// final failure. The first completion timestamp is preserved.
    fn complete_once(&mut self, now: Instant) -> bool {
        if !self.is_complete() {
            return false;
        }
        self.lifecycle_mut().complete_once(now)
    }

    /// `true` once the phase has reached a terminal outcome — complete or
    /// failed. Used to release downstream phases (e.g. repo waits on git).
    fn is_terminal(&self) -> bool {
        match self.lifecycle() {
            PhaseLifecycle::Running => false,
            PhaseLifecycle::Completed { .. } | PhaseLifecycle::Failed { .. } => true,
        }
    }

    /// `true` once the minimum-visible floor has elapsed: `now` is at or
    /// past `max(complete_at, first_seen + min_visible)`. A phase with no
    /// `first_seen` (it failed before first progress, or never rendered)
    /// uses `complete_at` alone.
    fn min_visible_elapsed(&self, now: Instant, min_visible: Duration) -> bool {
        let floor = match (self.complete_at(), self.first_seen()) {
            (Some(complete_at), Some(first_seen)) => complete_at.max(first_seen + min_visible),
            (Some(complete_at), None) => complete_at,
            (None, Some(first_seen)) => first_seen + min_visible,
            (None, None) => return true,
        };
        now >= floor
    }

    /// `true` when this phase no longer holds the startup panel open: it is
    /// omitted, failed (terminal), or complete and past its minimum-visible
    /// floor.
    fn gate_satisfied(&self, now: Instant, min_visible: Duration) -> bool {
        if self.is_omitted() {
            return true;
        }
        match self.lifecycle() {
            PhaseLifecycle::Running => false,
            PhaseLifecycle::Completed { .. } => self.min_visible_elapsed(now, min_visible),
            PhaseLifecycle::Failed { .. } => true,
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::collections::HashSet;

    use super::*;
    use crate::progress::CountedPhase;
    use crate::progress::KeyedPhase;
    fn instant_at(offset_ms: u64) -> Instant { Instant::now() + Duration::from_millis(offset_ms) }

    const MIN_VISIBLE: Duration = Duration::from_millis(400);
    #[test]
    fn complete_once_transitions_only_once() {
        let mut phase: KeyedPhase<i32> = KeyedPhase::default();
        phase.reset_with_expected(HashSet::new());
        let first = instant_at(0);
        let second = instant_at(10);
        assert!(phase.complete_once(first));
        assert_eq!(phase.complete_at(), Some(first));
        assert!(
            !phase.complete_once(second),
            "already complete, no transition"
        );
        assert_eq!(
            phase.complete_at(),
            Some(first),
            "timestamp not overwritten"
        );
    }

    #[test]
    fn complete_once_noop_when_not_complete() {
        let mut phase = CountedPhase {
            expected: Some(2),
            seen: 1,
            ..CountedPhase::default()
        };
        assert!(!phase.complete_once(instant_at(0)));
        assert!(phase.complete_at().is_none());
    }

    #[test]
    fn fetch_failure_stays_failed_when_late_work_lands() {
        for reason in [FailureReason::FetchError, FailureReason::RateLimited] {
            let mut phase = KeyedPhase::default();
            phase.reset_with_expected(HashSet::from([1]));
            phase.lifecycle.fail(reason);
            phase.seen.insert(1);
            let late = Instant::now();
            assert!(!phase.complete_once(late));
            assert_eq!(phase.lifecycle, PhaseLifecycle::Failed { reason });
            assert_eq!(phase.complete_at(), None);
            assert_eq!(
                phase.progress_state(late, MIN_VISIBLE),
                Some(ProgressState::Failed)
            );
        }
    }

    #[test]
    fn complete_row_held_until_min_visible_then_full() {
        let mut phase: KeyedPhase<i32> = KeyedPhase::default();
        phase.reset_with_expected(HashSet::new());
        let start = instant_at(0);
        phase.stamp_first_seen(start);
        assert!(phase.complete_once(start));
        assert_eq!(
            phase.progress_state(start, MIN_VISIBLE),
            Some(ProgressState::CompleteHeld),
            "a row that completes instantly is held full within the floor"
        );
        assert!(
            !phase.gate_satisfied(start, MIN_VISIBLE),
            "the floor keeps the row gating the panel"
        );
        let after = start + MIN_VISIBLE + Duration::from_millis(1);
        assert_eq!(
            phase.progress_state(after, MIN_VISIBLE),
            Some(ProgressState::Active(Percentage::full())),
            "past the floor the row renders a full bar"
        );
        assert!(
            phase.gate_satisfied(after, MIN_VISIBLE),
            "past the floor the row no longer gates"
        );
    }

    #[test]
    fn gate_satisfied_for_omitted_but_open_for_incomplete() {
        let omitted: KeyedPhase<i32> = KeyedPhase::default();
        assert!(
            omitted.gate_satisfied(Instant::now(), MIN_VISIBLE),
            "an omitted row never holds the panel"
        );
        let mut running: KeyedPhase<i32> = KeyedPhase::default();
        running.reset_with_expected(std::iter::once(1).collect());
        assert!(
            !running.gate_satisfied(Instant::now(), MIN_VISIBLE),
            "an incomplete row holds the panel open"
        );
    }
}
