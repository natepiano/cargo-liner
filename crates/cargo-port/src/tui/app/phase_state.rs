use std::collections::HashSet;
use std::hash::Hash;
use std::time::Duration;
use std::time::Instant;

use crate::project::AbsolutePath;

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
    #[default]
    Unknown,
    Growing(HashSet<K>),
    Stable(HashSet<K>),
}

impl<K: Eq + Hash> Denominator<K> {
    /// The expected key set when one is known; `None` while `Unknown`.
    pub const fn keys(&self) -> Option<&HashSet<K>> {
        match self {
            Self::Unknown => None,
            Self::Growing(keys) | Self::Stable(keys) => Some(keys),
        }
    }

    /// Number of expected keys; `0` while `Unknown`.
    pub fn len(&self) -> usize { self.keys().map_or(0, HashSet::len) }

    /// `true` when no denominator is known yet — the row is omitted.
    pub const fn is_unknown(&self) -> bool { matches!(self, Self::Unknown) }

    /// `true` while the denominator is still resolving — the row renders
    /// `Waiting` rather than a (backward-jumping) bar.
    pub const fn is_growing(&self) -> bool { matches!(self, Self::Growing(_)) }

    /// Freeze a `Growing` denominator into its final `Stable` set. A no-op
    /// for `Unknown` / already-`Stable`; idempotent, so a debounced caller
    /// can invoke it repeatedly.
    pub(super) fn stabilize(&mut self) {
        if let Self::Growing(keys) = self {
            *self = Self::Stable(std::mem::take(keys));
        }
    }

    /// Insert a key, transitioning `Unknown` to a `Stable` set first.
    /// Returns `true` when the key was newly inserted. Used by phases whose
    /// denominator accrues incrementally (repo while `Growing`, lint).
    pub(super) fn insert(&mut self, key: K) -> bool {
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
    pub expected:    Denominator<K>,
    pub seen:        HashSet<K>,
    pub complete_at: Option<Instant>,
    /// Stamped when the row first becomes visible (first progress, or —
    /// for a lazily-populated phase — when `expected` first becomes
    /// known). Drives the minimum-visible floor.
    pub first_seen:  Option<Instant>,
    /// Set once the phase reaches a terminal failure (fetch error, metadata
    /// error, or timeout). A failed row renders a stalled marker and no
    /// longer holds the panel open; the reason drives a separate toast.
    pub failure:     Option<FailureReason>,
}

impl<K: Eq + Hash> Default for KeyedPhase<K> {
    fn default() -> Self {
        Self {
            expected:    Denominator::Unknown,
            seen:        HashSet::new(),
            complete_at: None,
            first_seen:  None,
            failure:     None,
        }
    }
}

impl<K: Eq + Hash> KeyedPhase<K> {
    /// Length of the expected set; `0` when the denominator is `Unknown`.
    pub(super) fn expected_len(&self) -> usize { self.expected.len() }

    /// Stamp `first_seen` the first time the row becomes visible. Idempotent.
    pub(super) fn stamp_first_seen(&mut self, now: Instant) { self.first_seen.get_or_insert(now); }

    /// A deterministic not-yet-seen expected key, formatted via `label` — the
    /// "currently working on / next" detail for a slow row. Picks the
    /// lexically smallest label so it does not flicker frame to frame.
    pub(super) fn pending_sample<F: Fn(&K) -> String>(&self, label: F) -> Option<String> {
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
    pub(super) fn time_out(&mut self, now: Instant, timeout: Duration) -> Option<Duration> {
        if self.failure.is_some() || self.complete_at.is_some() || self.expected.is_unknown() {
            return None;
        }
        let elapsed = now.duration_since(self.first_seen?);
        if elapsed <= timeout {
            return None;
        }
        self.failure = Some(FailureReason::Timeout(elapsed));
        Some(elapsed)
    }

    /// Reset all fields and install a fresh `Stable` expected set in one shot.
    pub(super) fn reset_with_expected(&mut self, expected: HashSet<K>) {
        self.expected = Denominator::Stable(expected);
        self.seen.clear();
        self.complete_at = None;
        self.first_seen = None;
        self.failure = None;
    }

    /// Reset all fields to the `Unknown` denominator — the row stays
    /// omitted until work is queued. Used by lazily-populated phases (lint).
    pub(super) fn reset_unknown(&mut self) {
        self.expected = Denominator::Unknown;
        self.seen.clear();
        self.complete_at = None;
        self.first_seen = None;
        self.failure = None;
    }

    /// Reset all fields to a `Growing` (empty) denominator — the row renders
    /// `Waiting` until the denominator stabilizes. Used by the repo phase,
    /// whose GitHub set accrues as git remotes resolve.
    pub(super) fn reset_growing(&mut self) {
        self.expected = Denominator::Growing(HashSet::new());
        self.seen.clear();
        self.complete_at = None;
        self.first_seen = None;
        self.failure = None;
    }
}

/// A phase that tracks cardinality only — no per-item identity. Parallel to
/// `KeyedPhase` but with `usize` counters instead of `HashSet`.
#[derive(Debug, Default)]
pub struct CountedPhase {
    pub expected:    Option<usize>,
    pub seen:        usize,
    pub complete_at: Option<Instant>,
}

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
    pub complete_at:   Option<Instant>,
    pub first_seen:    Option<Instant>,
    pub failure:       Option<FailureReason>,
}

impl Default for LanguagePhase {
    fn default() -> Self {
        Self {
            expected:      Denominator::Unknown,
            seen:          HashSet::new(),
            work_expected: 0,
            work_seen:     0,
            complete_at:   None,
            first_seen:    None,
            failure:       None,
        }
    }
}

impl LanguagePhase {
    pub(super) fn expected_len(&self) -> usize { self.expected.len() + self.work_expected }

    pub(super) fn stamp_first_seen(&mut self, now: Instant) { self.first_seen.get_or_insert(now); }

    pub(super) fn reset_with_expected_roots(&mut self, expected: HashSet<AbsolutePath>) {
        self.expected = Denominator::Stable(expected);
        self.seen.clear();
        self.work_expected = 0;
        self.work_seen = 0;
        self.complete_at = None;
        self.first_seen = None;
        self.failure = None;
    }

    pub(super) const fn add_work_expected(&mut self, units: usize) {
        if units == 0 {
            return;
        }
        self.work_expected = self.work_expected.saturating_add(units);
        self.complete_at = None;
    }

    pub(super) const fn add_work_seen(&mut self, units: usize) {
        self.work_seen = self.work_seen.saturating_add(units);
    }

    pub(super) fn time_out(&mut self, now: Instant, timeout: Duration) -> Option<Duration> {
        if self.failure.is_some() || self.complete_at.is_some() || self.expected.is_unknown() {
            return None;
        }
        let elapsed = now.duration_since(self.first_seen?);
        if elapsed <= timeout {
            return None;
        }
        self.failure = Some(FailureReason::Timeout(elapsed));
        Some(elapsed)
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

/// A bar fraction clamped to `0..=100`, computed once from a subset count
/// so the renderer never re-divides and can never exceed the bar width.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Percentage(u8);

impl Percentage {
    /// An empty (0%) percentage.
    pub(super) const fn empty() -> Self { Self(0) }

    /// A complete (100%) percentage.
    pub(super) const fn full() -> Self { Self(100) }

    /// Percentage of `expected` keys that have been seen. An empty
    /// `expected` set renders 100; `seen >= expected` clamps to 100.
    pub(super) fn from_fraction(seen: usize, expected: usize) -> Self {
        if expected == 0 || seen >= expected {
            return Self(100);
        }
        // seen < expected and expected > 0, so the quotient is in 0..100.
        Self(u8::try_from(seen * 100 / expected).unwrap_or(100))
    }

    pub(super) const fn get(self) -> u8 { self.0 }
}

/// Why a phase reached a terminal failure. A failed row renders a stalled
/// marker; the reason is carried for the accompanying warning toast and for
/// logging. (`cargo metadata` errors are not represented here — that handler
/// already toasts and counts the workspace as done, so the row never stalls.)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureReason {
    RateLimited,
    FetchError,
    Timeout(Duration),
}

/// The render state of one startup row. `Progress` carries a clamped
/// percentage; `CompleteHeld` is 100% held open by the minimum-visible
/// floor; `Waiting` is an indeterminate denominator; `Failed` is a
/// terminal stall.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ProgressState {
    Progress(Percentage),
    CompleteHeld,
    Waiting,
    Failed,
}

/// One labeled row of the startup panel.
#[derive(Debug, Clone)]
pub(super) struct ProgressRow {
    pub label:  &'static str,
    pub state:  ProgressState,
    /// The item the row is currently working on (or about to), shown only
    /// once the row has been slow enough to warrant it. `None` keeps the row
    /// terse.
    pub detail: Option<String>,
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

    /// `true` when this phase renders no row (no denominator known yet),
    /// so it never holds the dismissal gate.
    fn is_omitted(&self) -> bool;

    /// When the row first became visible — drives the minimum-visible floor.
    fn first_seen(&self) -> Option<Instant>;

    /// `true` once the phase has reached a terminal failure.
    fn is_failed(&self) -> bool;

    /// The row's render state, or `None` when the row is omitted.
    fn progress_state(&self, now: Instant, min_visible: Duration) -> Option<ProgressState>;

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

    /// `true` once the phase has reached a terminal outcome — complete or
    /// failed. Used to release downstream phases (e.g. repo waits on git).
    fn is_terminal(&self) -> bool { self.complete_at().is_some() || self.is_failed() }

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
        self.is_omitted()
            || self.is_failed()
            || (self.complete_at().is_some() && self.min_visible_elapsed(now, min_visible))
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

    fn complete_at(&self) -> Option<Instant> { self.complete_at }

    fn mark_complete_at(&mut self, now: Instant) { self.complete_at = Some(now); }

    fn is_omitted(&self) -> bool { self.expected.is_unknown() }

    fn first_seen(&self) -> Option<Instant> { self.first_seen }

    fn is_failed(&self) -> bool { self.failure.is_some() }

    fn progress_state(&self, now: Instant, min_visible: Duration) -> Option<ProgressState> {
        if self.failure.is_some() {
            return Some(ProgressState::Failed);
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
        if self.is_complete() && !self.min_visible_elapsed(now, min_visible) {
            return Some(ProgressState::CompleteHeld);
        }
        Some(ProgressState::Progress(percentage))
    }
}

impl PhaseCompletion for CountedPhase {
    fn is_complete(&self) -> bool {
        matches!(self.expected, Some(expected) if self.seen >= expected)
    }

    fn complete_at(&self) -> Option<Instant> { self.complete_at }

    fn mark_complete_at(&mut self, now: Instant) { self.complete_at = Some(now); }

    // `CountedPhase` tracks internal cardinality only — it never renders a
    // startup row, so it is always omitted and yields no progress state.
    fn is_omitted(&self) -> bool { true }

    fn first_seen(&self) -> Option<Instant> { None }

    fn is_failed(&self) -> bool { false }

    fn progress_state(&self, _: Instant, _: Duration) -> Option<ProgressState> { None }
}

impl PhaseCompletion for LanguagePhase {
    fn is_complete(&self) -> bool {
        let roots_complete =
            matches!(self.expected.keys(), Some(expected) if expected.is_subset(&self.seen));
        roots_complete && self.work_seen >= self.work_expected
    }

    fn complete_at(&self) -> Option<Instant> { self.complete_at }

    fn mark_complete_at(&mut self, now: Instant) { self.complete_at = Some(now); }

    fn is_omitted(&self) -> bool { self.expected.is_unknown() && self.work_expected == 0 }

    fn first_seen(&self) -> Option<Instant> { self.first_seen }

    fn is_failed(&self) -> bool { self.failure.is_some() }

    fn progress_state(&self, now: Instant, min_visible: Duration) -> Option<ProgressState> {
        if self.failure.is_some() {
            return Some(ProgressState::Failed);
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
        if self.is_complete() && !self.min_visible_elapsed(now, min_visible) {
            return Some(ProgressState::CompleteHeld);
        }
        Some(ProgressState::Progress(percentage))
    }
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
mod tests {
    use std::time::Duration;

    use super::*;

    fn instant_at(offset_ms: u64) -> Instant { Instant::now() + Duration::from_millis(offset_ms) }

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
        phase.complete_at = Some(instant_at(0));
        phase.reset_with_expected(std::iter::once(1).collect());
        assert!(phase.seen.is_empty());
        assert!(phase.complete_at.is_none());
        assert_eq!(phase.expected_len(), 1);
    }

    const MIN_VISIBLE: Duration = Duration::from_millis(400);

    #[test]
    fn percentage_from_fraction_clamps_and_handles_empty() {
        assert_eq!(
            Percentage::from_fraction(0, 0).get(),
            100,
            "empty expected renders 100%"
        );
        assert_eq!(
            Percentage::from_fraction(5, 3).get(),
            100,
            "overshoot clamps to 100%"
        );
        assert_eq!(Percentage::from_fraction(0, 4).get(), 0);
        assert_eq!(Percentage::from_fraction(1, 4).get(), 25);
        assert_eq!(
            Percentage::from_fraction(3, 8).get(),
            37,
            "fractional percent floors"
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
            Some(ProgressState::Progress(Percentage::from_fraction(1, 4)))
        );
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
            Some(ProgressState::Progress(Percentage::full())),
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
            ProgressState::Progress(Percentage::from_fraction(1, 2)),
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
