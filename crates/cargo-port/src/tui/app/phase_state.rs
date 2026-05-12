use std::collections::HashSet;
use std::hash::Hash;
use std::time::Instant;

use tui_pane::ToastTaskId;
use tui_pane::TrackedItem;
use tui_pane::TrackedItemKey;

/// A phase that tracks a finite set of identities. `expected = None` is the
/// "Unknown" state (nothing has been recorded yet); `Some(HashSet::new())` is
/// "initialized, nothing to do." The distinction matters: an Unknown phase
/// cannot be `is_complete`, but an initialized-empty one can.
#[derive(Debug)]
pub struct KeyedPhase<K: Eq + Hash> {
    pub expected:    Option<HashSet<K>>,
    pub seen:        HashSet<K>,
    pub complete_at: Option<Instant>,
    pub toast:       Option<ToastTaskId>,
}

impl<K: Eq + Hash> Default for KeyedPhase<K> {
    fn default() -> Self {
        Self {
            expected:    None,
            seen:        HashSet::new(),
            complete_at: None,
            toast:       None,
        }
    }
}

impl<K: Eq + Hash> KeyedPhase<K> {
    /// Length of the expected set; `0` when `expected` is `None`.
    pub(super) fn expected_len(&self) -> usize { self.expected.as_ref().map_or(0, HashSet::len) }

    /// Returns the expected set, initializing it to empty if `None`. Used by
    /// call sites that incrementally insert (e.g. `repo.expected.insert(id)`)
    /// where the set may not have been initialized yet.
    pub(super) fn ensure_expected(&mut self) -> &mut HashSet<K> {
        self.expected.get_or_insert_with(HashSet::new)
    }

    /// Reset all fields and install a fresh expected set in one shot.
    pub(super) fn reset_with_expected(&mut self, expected: HashSet<K>) {
        self.expected = Some(expected);
        self.seen.clear();
        self.complete_at = None;
        self.toast = None;
    }

    /// Build tracked items from this phase's expected/seen sets.
    /// Already-seen keys are pre-marked as completed (renderer shows
    /// them with strikethrough); pending keys get `started_at = now`
    /// so they render with a live spinner + ticking duration that
    /// freezes when the item completes.
    pub(super) fn tracked_items<F>(&self, label_fn: F) -> Vec<TrackedItem>
    where
        for<'a> &'a K: Into<TrackedItemKey>,
        F: Fn(&K) -> String,
    {
        let now = Instant::now();
        let empty = HashSet::new();
        let expected = self.expected.as_ref().unwrap_or(&empty);
        expected
            .iter()
            .map(|k| {
                let is_seen = self.seen.contains(k);
                TrackedItem {
                    label:        label_fn(k),
                    key:          k.into(),
                    started_at:   if is_seen { None } else { Some(now) },
                    completed_at: if is_seen { Some(now) } else { None },
                }
            })
            .collect()
    }
}

/// A phase that tracks cardinality only — no per-item identity. Parallel to
/// `KeyedPhase` but with `usize` counters instead of `HashSet`.
#[derive(Debug, Default)]
pub struct CountedPhase {
    pub expected:    Option<usize>,
    pub seen:        usize,
    pub complete_at: Option<Instant>,
    pub toast:       Option<ToastTaskId>,
}

/// Shared completion behavior for keyed and counted phases. Two
/// implementations — the minimum that justifies a trait under the style
/// guide. Having the trait lets callers write `phase.complete_once(now)`
/// without caring which kind of phase they hold.
pub(super) trait PhaseCompletion {
    /// `true` once `expected` is set and `seen` has caught up to it.
    fn is_complete(&self) -> bool;

    fn complete_at(&self) -> Option<Instant>;

    fn mark_complete_at(&mut self, now: Instant);

    fn take_toast(&mut self) -> Option<ToastTaskId>;

    /// Mark complete exactly once. Returns `true` on the transition from
    /// "in progress" to "complete", `false` otherwise (already complete, or
    /// still unfinished). Idempotent.
    fn complete_once(&mut self, now: Instant) -> bool {
        if self.complete_at().is_some() || !self.is_complete() {
            return false;
        }
        self.mark_complete_at(now);
        true
    }
}

impl<K: Eq + Hash> PhaseCompletion for KeyedPhase<K> {
    fn is_complete(&self) -> bool {
        matches!(&self.expected, Some(expected) if self.seen.len() >= expected.len())
    }

    fn complete_at(&self) -> Option<Instant> { self.complete_at }

    fn mark_complete_at(&mut self, now: Instant) { self.complete_at = Some(now); }

    fn take_toast(&mut self) -> Option<ToastTaskId> { self.toast.take() }
}

impl PhaseCompletion for CountedPhase {
    fn is_complete(&self) -> bool {
        matches!(self.expected, Some(expected) if self.seen >= expected)
    }

    fn complete_at(&self) -> Option<Instant> { self.complete_at }

    fn mark_complete_at(&mut self, now: Instant) { self.complete_at = Some(now); }

    fn take_toast(&mut self) -> Option<ToastTaskId> { self.toast.take() }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
#[allow(
    clippy::unwrap_used,
    reason = "tests should panic on unexpected values"
)]
#[allow(clippy::panic, reason = "tests should panic on unexpected values")]
mod tests {
    use std::time::Duration;

    use super::*;

    fn instant_at(offset_ms: u64) -> Instant { Instant::now() + Duration::from_millis(offset_ms) }

    #[test]
    fn keyed_unknown_vs_initialized_empty() {
        let mut phase: KeyedPhase<i32> = KeyedPhase::default();
        assert!(
            !phase.is_complete(),
            "Unknown (expected=None) is never complete"
        );
        phase.expected = Some(HashSet::new());
        assert!(phase.is_complete(), "initialized-empty is complete");
    }

    #[test]
    fn keyed_complete_when_all_seen() {
        let mut phase: KeyedPhase<i32> = KeyedPhase::default();
        phase.reset_with_expected([1, 2, 3].into_iter().collect());
        assert!(!phase.is_complete());
        phase.seen.insert(1);
        phase.seen.insert(2);
        assert!(!phase.is_complete());
        phase.seen.insert(3);
        assert!(phase.is_complete());
    }

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
    fn take_toast_on_empty_stays_none() {
        let mut phase: KeyedPhase<i32> = KeyedPhase::default();
        assert!(phase.take_toast().is_none());
        assert!(phase.toast.is_none(), "take on empty leaves empty");
    }

    #[test]
    fn keyed_ensure_expected_initializes_unknown() {
        let mut phase: KeyedPhase<i32> = KeyedPhase::default();
        assert!(phase.expected.is_none());
        phase.ensure_expected().insert(42);
        assert_eq!(phase.expected_len(), 1);
        assert!(phase.expected.as_ref().unwrap().contains(&42));
    }

    #[test]
    fn keyed_reset_clears_seen_and_timestamp() {
        let mut phase: KeyedPhase<i32> = KeyedPhase::default();
        phase.seen.insert(7);
        phase.complete_at = Some(instant_at(0));
        phase.reset_with_expected(std::iter::once(1).collect());
        assert!(phase.seen.is_empty());
        assert!(phase.complete_at.is_none());
        assert_eq!(phase.expected_len(), 1);
    }
}
