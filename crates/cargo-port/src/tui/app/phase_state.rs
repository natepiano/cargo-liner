use std::collections::HashSet;
use std::time::Duration;
use std::time::Instant;

pub use tui_pane::CountedPhase;
pub use tui_pane::Denominator;
pub use tui_pane::FailureReason;
pub use tui_pane::KeyedPhase;
pub(super) use tui_pane::Percentage;
pub(super) use tui_pane::PhaseCompletion;
pub use tui_pane::PhaseLifecycle;
pub(super) use tui_pane::ProgressRow;
pub(super) use tui_pane::ProgressState;

use crate::project::AbsolutePath;

/// Startup language scanning has two kinds of work:
///
/// - project roots, where identity matters because final stats must land for every startup root;
///   and
/// - scan-work progress, where only cardinality matters and storing every path would flood the UI
///   queue.
#[derive(Debug)]
pub struct LanguagePhase {
    pub expected:      Denominator<AbsolutePath>,
    pub seen:          HashSet<AbsolutePath>,
    pub work_expected: usize,
    pub work_seen:     usize,
    pub lifecycle:     PhaseLifecycle,
    pub first_seen:    Option<Instant>,
}

impl Default for LanguagePhase {
    fn default() -> Self {
        Self {
            expected:      Denominator::Unknown,
            seen:          HashSet::new(),
            work_expected: 0,
            work_seen:     0,
            lifecycle:     PhaseLifecycle::Running,
            first_seen:    None,
        }
    }
}

impl LanguagePhase {
    pub(super) fn expected_len(&self) -> usize {
        self.expected.expected_count() + self.work_expected
    }

    pub(super) fn stamp_first_seen(&mut self, now: Instant) { self.first_seen.get_or_insert(now); }

    pub(super) fn reset_with_expected_roots(&mut self, expected: HashSet<AbsolutePath>) {
        self.expected = Denominator::Stable(expected);
        self.seen.clear();
        self.work_expected = 0;
        self.work_seen = 0;
        self.lifecycle = PhaseLifecycle::Running;
        self.first_seen = None;
    }

    pub(super) const fn add_work_expected(&mut self, units: usize) {
        if units == 0 {
            return;
        }
        self.work_expected = self.work_expected.saturating_add(units);
        self.lifecycle.reopen();
    }

    pub(super) const fn add_work_seen(&mut self, units: usize) {
        self.work_seen = self.work_seen.saturating_add(units);
    }

    pub(super) fn time_out(&mut self, now: Instant, timeout: Duration) -> Option<Duration> {
        if self.expected.is_unknown() {
            return None;
        }
        self.lifecycle.time_out(now, self.first_seen, timeout)
    }

    fn root_progress(&self) -> Option<(usize, usize)> {
        let expected = self.expected.keys()?;
        let done = expected.iter().filter(|k| self.seen.contains(*k)).count();
        Some((done, expected.len()))
    }

    fn work_progress(&self) -> (usize, usize) {
        (self.work_seen.min(self.work_expected), self.work_expected)
    }
}

impl PhaseCompletion for LanguagePhase {
    fn is_complete(&self) -> bool {
        let roots_complete =
            matches!(self.expected.keys(), Some(expected) if expected.is_subset(&self.seen));
        roots_complete && self.work_seen >= self.work_expected
    }

    fn lifecycle(&self) -> PhaseLifecycle { self.lifecycle }

    fn lifecycle_mut(&mut self) -> &mut PhaseLifecycle { &mut self.lifecycle }

    fn is_omitted(&self) -> bool { self.expected.is_unknown() && self.work_expected == 0 }

    fn first_seen(&self) -> Option<Instant> { self.first_seen }

    fn progress_state(&self, now: Instant, min_visible: Duration) -> Option<ProgressState> {
        match self.lifecycle {
            PhaseLifecycle::Failed { .. } => return Some(ProgressState::Failed),
            PhaseLifecycle::Completed { .. } => {
                return Some(if self.min_visible_elapsed(now, min_visible) {
                    ProgressState::Active(Percentage::full())
                } else {
                    ProgressState::CompleteHeld
                });
            },
            PhaseLifecycle::Running => {},
        }
        if self.expected.is_unknown() && self.work_expected == 0 {
            return None;
        }
        if self.expected.is_growing() {
            return Some(ProgressState::Waiting);
        }
        let (root_seen, root_expected) = self.root_progress().unwrap_or((0, 0));
        let (work_seen, work_expected) = self.work_progress();
        let percentage =
            Percentage::from_fraction(root_seen + work_seen, root_expected + work_expected);
        Some(ProgressState::Active(percentage))
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use super::*;
    const MIN_VISIBLE: Duration = Duration::from_millis(400);
    #[test]
    fn language_timeout_completes_when_late_work_lands() {
        let mut phase = LanguagePhase::default();
        phase.reset_with_expected_roots(HashSet::new());
        phase.add_work_expected(1);
        let start = Instant::now();
        phase.stamp_first_seen(start);
        let late = start + Duration::from_secs(2);
        assert!(phase.time_out(late, Duration::from_secs(1)).is_some());
        phase.add_work_seen(1);
        assert!(phase.complete_once(late));
        assert!(!phase.is_failed());
        assert_eq!(phase.lifecycle, PhaseLifecycle::Completed { at: late });
        assert_eq!(
            phase.progress_state(late, MIN_VISIBLE),
            Some(ProgressState::Active(Percentage::full()))
        );
    }

    #[test]
    fn reset_starts_a_fresh_lifecycle_after_failure() {
        let mut keyed: KeyedPhase<i32> = KeyedPhase::default();
        for reset in [
            KeyedPhase::reset_unknown,
            KeyedPhase::reset_growing,
            |phase: &mut KeyedPhase<i32>| phase.reset_with_expected(HashSet::from([1])),
        ] {
            keyed.lifecycle.fail(FailureReason::FetchError);
            reset(&mut keyed);
            assert_eq!(keyed.lifecycle, PhaseLifecycle::Running);
        }
        let mut language = LanguagePhase::default();
        language.lifecycle.fail(FailureReason::FetchError);
        language.add_work_expected(1);
        language.add_work_seen(1);
        assert!(!language.complete_once(Instant::now()));
        assert_eq!(
            language.lifecycle,
            PhaseLifecycle::Failed {
                reason: FailureReason::FetchError,
            }
        );
        language.reset_with_expected_roots(HashSet::new());
        assert_eq!(language.lifecycle, PhaseLifecycle::Running);
    }
}
