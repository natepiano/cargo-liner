//! Progress over a set of expected identities.

use std::collections::HashSet;
use std::hash::Hash;
use std::time::Duration;
use std::time::Instant;

use super::Percentage;
use super::PhaseCompletion;
use super::PhaseLifecycle;
use super::ProgressState;

/// The expected-key set of a [`KeyedPhase`], modeled so the
/// "stabilized but unknown" combination is unrepresentable.
///
/// `Unknown` means the row is omitted — no denominator is known yet.
/// `Growing` means the denominator is known but still resolving, so the
/// row renders `Waiting` and the percentage would jump backward if shown.
/// `Stable` means the denominator is final and the row renders a bar (an
/// empty `Stable` set renders 100%).
#[derive(Debug, Default)]
pub enum Denominator<K: Eq + Hash> {
    /// No denominator has been discovered.
    #[default]
    Unknown,
    /// Expected identities are still being discovered.
    Growing(HashSet<K>),
    /// Final expected identities.
    Stable(HashSet<K>),
}

impl<K: Eq + Hash> Denominator<K> {
    /// The expected key set when one is known; `None` while `Unknown`.
    #[must_use]
    pub const fn keys(&self) -> Option<&HashSet<K>> {
        match self {
            Self::Unknown => None,
            Self::Growing(keys) | Self::Stable(keys) => Some(keys),
        }
    }

    /// Number of expected keys; `0` while `Unknown`.
    #[must_use]
    pub fn expected_count(&self) -> usize { self.keys().map_or(0, HashSet::len) }

    /// `true` when no denominator is known yet — the row is omitted.
    #[must_use]
    pub const fn is_unknown(&self) -> bool { matches!(self, Self::Unknown) }

    /// `true` while the denominator is still resolving — the row renders
    /// `Waiting` rather than a (backward-jumping) bar.
    #[must_use]
    pub const fn is_growing(&self) -> bool { matches!(self, Self::Growing(_)) }

    /// Freeze a `Growing` denominator into its final `Stable` set. A no-op
    /// for `Unknown` / already-`Stable`; idempotent, so a debounced caller
    /// can invoke it repeatedly.
    pub fn stabilize(&mut self) {
        if let Self::Growing(keys) = self {
            *self = Self::Stable(std::mem::take(keys));
        }
    }

    /// Insert a key, transitioning `Unknown` to a `Stable` set first.
    /// Returns `true` when the key was newly inserted. Used by phases whose
    /// denominator accrues incrementally (repo while `Growing`, lint).
    pub fn insert(&mut self, key: K) -> bool {
        match self {
            Self::Growing(keys) | Self::Stable(keys) => keys.insert(key),
            Self::Unknown => {
                let mut keys = HashSet::new();
                let inserted = keys.insert(key);
                *self = Self::Stable(keys);
                inserted
            },
        }
    }
}

/// A phase that tracks a finite set of identities. The denominator is a
/// [`Denominator`]: `Unknown` omits the row, `Stable` renders a bar.
#[derive(Debug)]
pub struct KeyedPhase<K: Eq + Hash> {
    /// Expected identities and their stabilization state.
    pub expected:   Denominator<K>,
    /// Identities whose results have arrived.
    pub seen:       HashSet<K>,
    /// Current completion or failure outcome.
    pub lifecycle:  PhaseLifecycle,
    /// Stamped when the row first becomes visible (first progress, or —
    /// for a lazily-populated phase — when `expected` first becomes
    /// known). Drives the minimum-visible floor.
    pub first_seen: Option<Instant>,
}

impl<K: Eq + Hash> Default for KeyedPhase<K> {
    fn default() -> Self {
        Self {
            expected:   Denominator::Unknown,
            seen:       HashSet::new(),
            lifecycle:  PhaseLifecycle::Running,
            first_seen: None,
        }
    }
}

impl<K: Eq + Hash> KeyedPhase<K> {
    /// Length of the expected set; `0` when the denominator is `Unknown`.
    #[must_use]
    pub fn expected_len(&self) -> usize { self.expected.expected_count() }

    /// Stamp `first_seen` the first time the row becomes visible. Idempotent.
    pub fn stamp_first_seen(&mut self, now: Instant) { self.first_seen.get_or_insert(now); }

    /// A deterministic not-yet-seen expected key, formatted via `label` — the
    /// "currently working on / next" detail for a slow row. Picks the
    /// lexically smallest label so it does not flicker frame to frame.
    #[must_use]
    pub fn pending_sample<F: Fn(&K) -> String>(&self, label: F) -> Option<String> {
        let expected = self.expected.keys()?;
        expected
            .iter()
            .filter(|key| !self.seen.contains(*key))
            .map(label)
            .min()
    }

    /// Mark the phase failed by timeout when it has been visible longer than
    /// `timeout` without completing or failing. Returns the elapsed time on
    /// the transition (so the caller can toast it once), `None` otherwise.
    pub fn time_out(&mut self, now: Instant, timeout: Duration) -> Option<Duration> {
        if self.expected.is_unknown() {
            return None;
        }
        self.lifecycle.time_out(now, self.first_seen, timeout)
    }

    /// Reset all fields and install a fresh `Stable` expected set in one shot.
    pub fn reset_with_expected(&mut self, expected: HashSet<K>) {
        self.expected = Denominator::Stable(expected);
        self.seen.clear();
        self.lifecycle = PhaseLifecycle::Running;
        self.first_seen = None;
    }

    /// Reset all fields to the `Unknown` denominator — the row stays
    /// omitted until work is queued. Used by lazily-populated phases (lint).
    pub fn reset_unknown(&mut self) {
        self.expected = Denominator::Unknown;
        self.seen.clear();
        self.lifecycle = PhaseLifecycle::Running;
        self.first_seen = None;
    }

    /// Reset all fields to a `Growing` (empty) denominator — the row renders
    /// `Waiting` until the denominator stabilizes. Used by the repo phase,
    /// whose GitHub set accrues as git remotes resolve.
    pub fn reset_growing(&mut self) {
        self.expected = Denominator::Growing(HashSet::new());
        self.seen.clear();
        self.lifecycle = PhaseLifecycle::Running;
        self.first_seen = None;
    }
}

impl<K: Eq + Hash> PhaseCompletion for KeyedPhase<K> {
    fn is_complete(&self) -> bool {
        // `expected.is_subset(&self.seen)` — every expected key must
        // actually be in `seen`, not just enough total entries. The
        // disk-usage batch path inserts a tree's root plus all
        // nested entries (workspace members, etc.) into `seen`, so
        // a length-only comparison would mark the phase complete
        // long before every expected top-level project was seen,
        // jumping the panel row to 100% prematurely.
        matches!(self.expected.keys(), Some(expected) if expected.is_subset(&self.seen))
    }

    fn lifecycle(&self) -> PhaseLifecycle { self.lifecycle }

    fn lifecycle_mut(&mut self) -> &mut PhaseLifecycle { &mut self.lifecycle }

    fn is_omitted(&self) -> bool { self.expected.is_unknown() }

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
        if self.expected.is_unknown() {
            return None;
        }
        if self.expected.is_growing() {
            return Some(ProgressState::Waiting);
        }
        let expected = self.expected.keys()?;
        let done = expected.iter().filter(|k| self.seen.contains(k)).count();
        let percentage = Percentage::from_fraction(done, expected.len());
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
    fn instant_at(offset_ms: u64) -> Instant { Instant::now() + Duration::from_millis(offset_ms) }

    const MIN_VISIBLE: Duration = Duration::from_millis(400);
    #[test]
    fn keyed_unknown_vs_initialized_empty() {
        let mut phase: KeyedPhase<i32> = KeyedPhase::default();
        assert!(
            !phase.is_complete(),
            "Unknown denominator is never complete"
        );
        phase.expected = Denominator::Stable(HashSet::new());
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
    fn denominator_insert_initializes_unknown() {
        let mut phase: KeyedPhase<i32> = KeyedPhase::default();
        assert!(phase.expected.is_unknown());
        assert!(phase.expected.insert(42), "first insert is new");
        assert_eq!(phase.expected_len(), 1);
        assert!(phase.expected.keys().unwrap().contains(&42));
    }

    #[test]
    fn keyed_reset_clears_seen_and_timestamp() {
        let mut phase: KeyedPhase<i32> = KeyedPhase::default();
        phase.seen.insert(7);
        phase.lifecycle = PhaseLifecycle::Completed { at: instant_at(0) };
        phase.reset_with_expected(std::iter::once(1).collect());
        assert!(phase.seen.is_empty());
        assert!(phase.complete_at().is_none());
        assert_eq!(phase.expected_len(), 1);
    }

    #[test]
    fn keyed_timeout_completes_when_late_work_lands() {
        let mut phase = KeyedPhase::default();
        phase.reset_with_expected(HashSet::from([1]));
        let start = Instant::now();
        phase.stamp_first_seen(start);
        let late = start + Duration::from_secs(2);
        assert!(phase.time_out(late, Duration::from_secs(1)).is_some());
        phase.seen.insert(1);
        assert!(phase.complete_once(late));
        assert!(!phase.is_failed());
        assert_eq!(phase.lifecycle, PhaseLifecycle::Completed { at: late });
        assert_eq!(
            phase.progress_state(late, MIN_VISIBLE),
            Some(ProgressState::Active(Percentage::full()))
        );
    }

    #[test]
    fn progress_state_omits_unknown_row() {
        let phase: KeyedPhase<i32> = KeyedPhase::default();
        assert!(
            phase.progress_state(Instant::now(), MIN_VISIBLE).is_none(),
            "an Unknown denominator omits the row"
        );
    }

    #[test]
    fn progress_state_reports_partial_fraction() {
        let mut phase: KeyedPhase<i32> = KeyedPhase::default();
        phase.reset_with_expected([1, 2, 3, 4].into_iter().collect());
        phase.stamp_first_seen(Instant::now());
        phase.seen.insert(1);
        assert_eq!(
            phase.progress_state(Instant::now(), MIN_VISIBLE),
            Some(ProgressState::Active(Percentage::from_fraction(1, 4)))
        );
    }

    #[test]
    fn growing_denominator_renders_waiting_until_stable() {
        let mut phase: KeyedPhase<i32> = KeyedPhase::default();
        phase.reset_growing();
        phase.stamp_first_seen(Instant::now());
        assert!(phase.expected.insert(1));
        assert!(phase.expected.insert(2));
        assert_eq!(
            phase.progress_state(Instant::now(), MIN_VISIBLE),
            Some(ProgressState::Waiting),
            "a growing denominator renders Waiting, never a regressing bar"
        );
        assert!(
            !phase.gate_satisfied(Instant::now(), MIN_VISIBLE),
            "a waiting row holds the panel open"
        );
        phase.seen.insert(1);
        phase.expected.stabilize();
        let state = phase
            .progress_state(Instant::now(), MIN_VISIBLE)
            .expect("row");
        assert_eq!(
            state,
            ProgressState::Active(Percentage::from_fraction(1, 2)),
            "once stable the row renders a determinate bar"
        );
    }

    #[test]
    fn timeout_marks_failed_and_releases_the_gate() {
        let mut phase: KeyedPhase<i32> = KeyedPhase::default();
        phase.reset_with_expected(std::iter::once(1).collect());
        let start = instant_at(0);
        phase.stamp_first_seen(start);
        let timeout = Duration::from_mins(2);
        assert!(
            phase.time_out(start, timeout).is_none(),
            "no timeout before the deadline"
        );
        let after = start + timeout + Duration::from_secs(1);
        assert!(
            phase.time_out(after, timeout).is_some(),
            "times out past the deadline"
        );
        assert!(
            phase.time_out(after, timeout).is_none(),
            "already failed — no second transition"
        );
        assert!(phase.is_failed() && phase.is_terminal());
        assert_eq!(
            phase.progress_state(after, MIN_VISIBLE),
            Some(ProgressState::Failed)
        );
        assert!(
            phase.gate_satisfied(after, MIN_VISIBLE),
            "a failed row never holds the panel open"
        );
    }

    #[test]
    fn complete_phase_does_not_time_out() {
        let mut phase: KeyedPhase<i32> = KeyedPhase::default();
        phase.reset_with_expected(HashSet::new());
        let start = instant_at(0);
        phase.stamp_first_seen(start);
        assert!(phase.complete_once(start));
        let after = start + Duration::from_mins(10);
        assert!(
            phase.time_out(after, Duration::from_mins(2)).is_none(),
            "a completed phase is immune to the timeout"
        );
        assert!(!phase.is_failed());
    }

    #[test]
    fn omitted_phase_does_not_time_out() {
        let mut phase: KeyedPhase<i32> = KeyedPhase::default();
        phase.stamp_first_seen(instant_at(0));
        let after = instant_at(0) + Duration::from_mins(10);
        assert!(
            phase.time_out(after, Duration::from_mins(2)).is_none(),
            "an omitted (Unknown) phase never times out"
        );
    }
}
