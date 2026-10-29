//! Progress over an expected count.

use std::time::Duration;
use std::time::Instant;

use super::PhaseCompletion;
use super::PhaseLifecycle;
use super::ProgressState;

/// A phase that tracks cardinality only — no per-item identity. Parallel to
/// [`super::KeyedPhase`] but with `usize` counters instead of a set.
#[derive(Debug, Default)]
pub struct CountedPhase {
    /// Expected total, or no known count yet.
    pub expected:  Option<usize>,
    /// Number of observed results.
    pub seen:      usize,
    /// Current completion or failure outcome.
    pub lifecycle: PhaseLifecycle,
}

impl PhaseCompletion for CountedPhase {
    fn is_complete(&self) -> bool {
        matches!(self.expected, Some(expected) if self.seen >= expected)
    }

    fn lifecycle(&self) -> PhaseLifecycle { self.lifecycle }

    fn lifecycle_mut(&mut self) -> &mut PhaseLifecycle { &mut self.lifecycle }

    // `CountedPhase` tracks internal cardinality only — it never renders a
    // startup row, so it is always omitted and yields no progress state.
    fn is_omitted(&self) -> bool { true }

    fn first_seen(&self) -> Option<Instant> { None }

    fn progress_state(&self, _: Instant, _: Duration) -> Option<ProgressState> { None }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use super::*;

    #[test]
    fn counted_complete_when_seen_reaches_expected() {
        let mut phase = CountedPhase::default();
        assert!(
            !phase.is_complete(),
            "Unknown (expected=None) is never complete"
        );
        phase.expected = Some(3);
        assert!(!phase.is_complete());
        phase.seen = 2;
        assert!(!phase.is_complete());
        phase.seen = 3;
        assert!(phase.is_complete());
        phase.seen = 4;
        assert!(phase.is_complete(), "overshoot stays complete");
    }

    #[test]
    fn counted_initialized_zero_of_zero_is_complete() {
        let phase = CountedPhase {
            expected: Some(0),
            ..CountedPhase::default()
        };
        assert!(phase.is_complete());
    }

    #[test]
    fn counted_completion_keeps_its_first_timestamp() {
        let mut phase = CountedPhase {
            expected: Some(1),
            ..CountedPhase::default()
        };
        assert!(!phase.complete_once(Instant::now()));
        phase.seen = 1;
        let now = Instant::now();
        assert!(phase.complete_once(now));
        assert!(!phase.complete_once(now + Duration::from_secs(1)));
        assert_eq!(phase.lifecycle, PhaseLifecycle::Completed { at: now });
    }
}
