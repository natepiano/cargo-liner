//! Per-project sync-state tracking for the "Sync changes" task toast.
//!
//! Two pieces of per-project state:
//! 1. `eligible`: flipped true once the first GitHub fetch for the project's repo completes. Until
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
    eligible:  bool,
    last_seen: Baseline,
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
    /// Mark a project eligible for sync-change toasts and seed its
    /// baseline if not yet observed. Called once per project, the first
    /// time that project's GitHub fetch completes.
    pub fn mark_eligible(&mut self, path: AbsolutePath, current: Option<(usize, usize)>) {
        let entry = self.entries.entry(path).or_default();
        entry.eligible = true;
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
        match (entry.eligible, previous) {
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
    clippy::unwrap_used,
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
    fn mark_eligible_seeds_baseline_then_first_change_toasts() {
        let mut tracker = SyncTracker::default();
        tracker.mark_eligible(path("a"), Some((1, 0)));
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
        tracker.mark_eligible(path("a"), Some((3, 0)));
        let transition = tracker
            .observe(path("a"), Some((0, 0)))
            .expect("3-ahead → in sync should toast");
        assert_eq!(transition.previous, Some((3, 0)));
        assert_eq!(transition.current, Some((0, 0)));
    }

    #[test]
    fn no_remote_to_remote_is_a_transition() {
        let mut tracker = SyncTracker::default();
        tracker.mark_eligible(path("a"), None);
        let transition = tracker
            .observe(path("a"), Some((0, 1)))
            .expect("None → behind 1 should toast");
        assert_eq!(transition.previous, None);
        assert_eq!(transition.current, Some((0, 1)));
    }

    #[test]
    fn mark_eligible_a_second_time_does_not_re_seed_baseline() {
        let mut tracker = SyncTracker::default();
        tracker.mark_eligible(path("a"), Some((1, 0)));
        // Observe a change so last_seen advances to (2, 0)
        let _ = tracker.observe(path("a"), Some((2, 0)));
        // Re-eligibility (shouldn't happen in practice but defensive):
        tracker.mark_eligible(path("a"), Some((9, 9)));
        let transition = tracker
            .observe(path("a"), Some((3, 0)))
            .expect("baseline preserved across re-eligibility");
        assert_eq!(
            transition.previous,
            Some((2, 0)),
            "must compare against last observed, not the re-eligibility seed"
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
