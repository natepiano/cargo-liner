//! Per-project sync-state tracking for the "Sync changes" task toast.
//!
//! Two pieces of per-project state:
//! 1. `toast_readiness`: set once the first GitHub fetch for the project's repo completes. Until
//!    then, sync-state changes are treated as startup noise and never surface as a toast.
//! 2. `last_seen`: the most recent `(ahead, behind)` value observed for the project's primary
//!    remote. [`Baseline`] makes the three states explicit: `Unseen`, `Seen(None)` (remote-less),
//!    and `Seen(Some((a, b)))`.
//!
//! The single live "Sync changes" task toast's id is also held here
//! so transitions within the linger window accumulate as tracked
//! items on one toast rather than spawning N stacked cards.

use std::collections::HashMap;

use tui_pane::ToastTaskId;

use crate::constants::IN_SYNC;
use crate::constants::SYNC_DOWN;
use crate::constants::SYNC_UP;
use crate::project::AbsolutePath;

#[derive(Default)]
pub struct SyncTracker {
    entries:       HashMap<AbsolutePath, SyncEntry>,
    current_toast: Option<ToastTaskId>,
    next_item_seq: u64,
}

#[derive(Default)]
struct SyncEntry {
    toast_readiness: SyncToastReadiness,
    last_seen:       Baseline,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum SyncToastReadiness {
    Ready,
    #[default]
    Waiting,
}

impl SyncToastReadiness {
    const fn is_ready(self) -> bool { matches!(self, Self::Ready) }
}

/// Three-state baseline for the per-project ahead/behind value.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
enum Baseline {
    /// No observation has been recorded yet — first `observe` call
    /// for this project just stashes the value.
    #[default]
    Unseen,
    /// At least one observation has been recorded; the inner `Option`
    /// distinguishes "no remote / no remote-tracking branch" (`None`)
    /// from a concrete `(ahead, behind)` tuple.
    Seen(Option<(usize, usize)>),
}

/// A flipped sync value worth surfacing.
pub struct SyncTransition {
    pub previous: Option<(usize, usize)>,
    pub current:  Option<(usize, usize)>,
}

impl SyncTracker {
    /// Mark a project eligible for sync-change toasts. Called once per
    /// project, the first time that project's GitHub fetch completes.
    /// Seeding the baseline is a separate step ([`Self::seed_baseline`]) so
    /// a project whose git info is still loading becomes eligible without
    /// recording a premature "no remote" baseline that a later
    /// `CheckoutInfo` would flip to "in sync".
    pub fn mark_eligible(&mut self, path: AbsolutePath) {
        self.entries.entry(path).or_default().toast_readiness = SyncToastReadiness::Ready;
    }

    /// Seed a project's baseline from a fully-resolved ahead/behind value,
    /// but only if no observation has been recorded yet. A `None` here is a
    /// real "no remote-tracking branch", not git info that is still loading.
    pub fn seed_baseline(&mut self, path: AbsolutePath, current: Option<(usize, usize)>) {
        let entry = self.entries.entry(path).or_default();
        if matches!(entry.last_seen, Baseline::Unseen) {
            entry.last_seen = Baseline::Seen(current);
        }
    }

    /// Record `current` and return `Some` if the value flipped versus
    /// the prior observation AND the project is eligible. Always
    /// updates the stored baseline.
    pub fn observe(
        &mut self,
        path: AbsolutePath,
        current: Option<(usize, usize)>,
    ) -> Option<SyncTransition> {
        let entry = self.entries.entry(path).or_default();
        let previous = entry.last_seen;
        entry.last_seen = Baseline::Seen(current);
        match (entry.toast_readiness.is_ready(), previous) {
            (true, Baseline::Seen(prev)) if prev != current => Some(SyncTransition {
                previous: prev,
                current,
            }),
            _ => None,
        }
    }

    pub const fn current_toast(&self) -> Option<ToastTaskId> { self.current_toast }

    pub const fn set_current_toast(&mut self, id: Option<ToastTaskId>) { self.current_toast = id; }

    /// Mint a unique sequence number for the next tracked item key so
    /// repeated transitions for the same project don't dedupe against
    /// each other inside one toast.
    pub const fn next_item_seq(&mut self) -> u64 {
        let seq = self.next_item_seq;
        self.next_item_seq = self.next_item_seq.wrapping_add(1);
        seq
    }
}

/// Render `acme: ↓3 ──▶︎ in sync` for one transition.
pub fn format_transition(name: &str, transition: &SyncTransition) -> String {
    format!(
        "{name}: {} ──▶︎ {}",
        format_sync(transition.previous),
        format_sync(transition.current)
    )
}

fn format_sync(value: Option<(usize, usize)>) -> String {
    match value {
        Some((0, 0)) => format!("{IN_SYNC} in sync"),
        Some((a, 0)) => format!("{SYNC_UP}{a}"),
        Some((0, b)) => format!("{SYNC_DOWN}{b}"),
        Some((a, b)) => format!("{SYNC_UP}{a}{SYNC_DOWN}{b}"),
        None => "no remote".to_string(),
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn path(name: &str) -> AbsolutePath {
        AbsolutePath::from(PathBuf::from(format!("/tmp/{name}")))
    }

    #[test]
    fn observe_returns_none_before_eligibility() {
        let mut tracker = SyncTracker::default();
        assert!(tracker.observe(path("a"), Some((1, 0))).is_none());
        assert!(tracker.observe(path("a"), Some((2, 0))).is_none());
    }

    #[test]
    fn seeded_baseline_then_first_change_toasts() {
        let mut tracker = SyncTracker::default();
        tracker.seed_baseline(path("a"), Some((1, 0)));
        tracker.mark_eligible(path("a"));
        assert!(
            tracker.observe(path("a"), Some((1, 0))).is_none(),
            "same value, no toast"
        );
        let transition = tracker
            .observe(path("a"), Some((2, 0)))
            .expect("changed value while eligible");
        assert_eq!(transition.previous, Some((1, 0)));
        assert_eq!(transition.current, Some((2, 0)));
    }

    #[test]
    fn back_to_in_sync_is_a_transition() {
        let mut tracker = SyncTracker::default();
        tracker.seed_baseline(path("a"), Some((3, 0)));
        tracker.mark_eligible(path("a"));
        let transition = tracker
            .observe(path("a"), Some((0, 0)))
            .expect("3-ahead → in sync should toast");
        assert_eq!(transition.previous, Some((3, 0)));
        assert_eq!(transition.current, Some((0, 0)));
    }

    #[test]
    fn no_remote_to_remote_is_a_transition() {
        let mut tracker = SyncTracker::default();
        // A genuinely-resolved "no remote-tracking branch" baseline.
        tracker.seed_baseline(path("a"), None);
        tracker.mark_eligible(path("a"));
        let transition = tracker
            .observe(path("a"), Some((0, 1)))
            .expect("None → behind 1 should toast");
        assert_eq!(transition.previous, None);
        assert_eq!(transition.current, Some((0, 1)));
    }

    #[test]
    fn eligible_without_baseline_seeds_silently_on_first_observation() {
        // The startup race: the project becomes eligible (GitHub fetch
        // completed) while git info is still loading, so no baseline is
        // seeded. The first resolved observation must establish the
        // baseline silently — not toast a spurious "no remote → in sync".
        let mut tracker = SyncTracker::default();
        tracker.mark_eligible(path("a"));
        assert!(
            tracker.observe(path("a"), Some((0, 0))).is_none(),
            "first resolved observation seeds the baseline, no toast"
        );
        let transition = tracker
            .observe(path("a"), Some((0, 1)))
            .expect("a later real change still toasts");
        assert_eq!(transition.previous, Some((0, 0)));
        assert_eq!(transition.current, Some((0, 1)));
    }

    #[test]
    fn seeding_a_second_time_does_not_re_seed_baseline() {
        let mut tracker = SyncTracker::default();
        tracker.seed_baseline(path("a"), Some((1, 0)));
        tracker.mark_eligible(path("a"));
        // Observe a change so last_seen advances to (2, 0)
        let _ = tracker.observe(path("a"), Some((2, 0)));
        // A second seed (shouldn't happen in practice but defensive):
        tracker.seed_baseline(path("a"), Some((9, 9)));
        let transition = tracker
            .observe(path("a"), Some((3, 0)))
            .expect("baseline preserved across re-seeding");
        assert_eq!(
            transition.previous,
            Some((2, 0)),
            "must compare against last observed, not the re-seed value"
        );
    }

    #[test]
    fn next_item_seq_is_monotonic() {
        let mut tracker = SyncTracker::default();
        assert_eq!(tracker.next_item_seq(), 0);
        assert_eq!(tracker.next_item_seq(), 1);
        assert_eq!(tracker.next_item_seq(), 2);
    }

    #[test]
    fn format_transition_renders_full_arrow() {
        let t = SyncTransition {
            previous: Some((3, 0)),
            current:  Some((0, 0)),
        };
        assert_eq!(format_transition("acme", &t), "acme: ↑3 ──▶︎ ☑️ in sync");
    }
}
