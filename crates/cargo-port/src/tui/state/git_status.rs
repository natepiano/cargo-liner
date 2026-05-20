//! Per-project git-status tracking for the "Git status changes" task toast.
//!
//! Mirrors [`super::sync`] but without the eligibility gate: `GitStatus` is
//! purely local (no remote refresh has to land first), so the first
//! observation just seeds the baseline silently and any later observation
//! whose value differs surfaces a transition.
//!
//! The single live "Git status changes" task toast's id is also held here so
//! transitions within the linger window accumulate as tracked items on one
//! toast rather than spawning N stacked cards.

use std::collections::HashMap;

use tui_pane::ToastTaskId;

use crate::project::AbsolutePath;
use crate::project::GitStatus;

#[derive(Default)]
pub struct GitStatusTracker {
    entries:       HashMap<AbsolutePath, Baseline>,
    current_toast: Option<ToastTaskId>,
    next_item_seq: u64,
}

/// Two-state baseline for the per-project `GitStatus`.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
enum Baseline {
    /// No observation has been recorded yet — first `observe` call for this
    /// project just stashes the value.
    #[default]
    Unseen,
    /// At least one observation has been recorded.
    Seen(GitStatus),
}

/// A flipped git-status value worth surfacing.
pub struct GitStatusTransition {
    pub previous: GitStatus,
    pub current:  GitStatus,
}

impl GitStatusTracker {
    /// Record `current` and return `Some` if the value flipped versus the
    /// prior observation. The first call for a given path always returns
    /// `None` and just seeds the baseline.
    pub fn observe(
        &mut self,
        path: AbsolutePath,
        current: GitStatus,
    ) -> Option<GitStatusTransition> {
        let previous = self.entries.insert(path, Baseline::Seen(current));
        match previous {
            Some(Baseline::Seen(prev)) if prev != current => Some(GitStatusTransition {
                previous: prev,
                current,
            }),
            _ => None,
        }
    }

    pub const fn current_toast(&self) -> Option<ToastTaskId> { self.current_toast }

    pub const fn set_current_toast(&mut self, id: Option<ToastTaskId>) { self.current_toast = id; }

    /// Mint a unique sequence number for the next tracked item key so
    /// repeated transitions for the same project don't dedupe against each
    /// other inside one toast.
    pub const fn next_item_seq(&mut self) -> u64 {
        let seq = self.next_item_seq;
        self.next_item_seq = self.next_item_seq.wrapping_add(1);
        seq
    }
}

/// Render `acme: ● modified ──▶︎ ✓ clean` for one transition.
pub fn format_transition(name: &str, transition: &GitStatusTransition) -> String {
    format!(
        "{name}: {} ──▶︎ {}",
        transition.previous.label_with_icon(),
        transition.current.label_with_icon()
    )
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
    fn first_observation_seeds_baseline_silently() {
        let mut tracker = GitStatusTracker::default();
        assert!(tracker.observe(path("a"), GitStatus::Clean).is_none());
    }

    #[test]
    fn same_value_does_not_toast() {
        let mut tracker = GitStatusTracker::default();
        let _ = tracker.observe(path("a"), GitStatus::Clean);
        assert!(tracker.observe(path("a"), GitStatus::Clean).is_none());
    }

    #[test]
    fn changed_value_toasts() {
        let mut tracker = GitStatusTracker::default();
        let _ = tracker.observe(path("a"), GitStatus::Clean);
        let transition = tracker
            .observe(path("a"), GitStatus::Modified)
            .expect("clean → modified should toast");
        assert_eq!(transition.previous, GitStatus::Clean);
        assert_eq!(transition.current, GitStatus::Modified);
    }

    #[test]
    fn back_to_clean_is_a_transition() {
        let mut tracker = GitStatusTracker::default();
        let _ = tracker.observe(path("a"), GitStatus::Modified);
        let transition = tracker
            .observe(path("a"), GitStatus::Clean)
            .expect("modified → clean should toast");
        assert_eq!(transition.previous, GitStatus::Modified);
        assert_eq!(transition.current, GitStatus::Clean);
    }

    #[test]
    fn per_path_baselines_are_independent() {
        let mut tracker = GitStatusTracker::default();
        let _ = tracker.observe(path("a"), GitStatus::Clean);
        let _ = tracker.observe(path("b"), GitStatus::Modified);
        // Same value as b's seed — no toast for b.
        assert!(tracker.observe(path("b"), GitStatus::Modified).is_none());
        // Different value from a's seed — toast for a.
        let transition = tracker
            .observe(path("a"), GitStatus::Untracked)
            .expect("a flipped");
        assert_eq!(transition.previous, GitStatus::Clean);
        assert_eq!(transition.current, GitStatus::Untracked);
    }

    #[test]
    fn next_item_seq_is_monotonic() {
        let mut tracker = GitStatusTracker::default();
        assert_eq!(tracker.next_item_seq(), 0);
        assert_eq!(tracker.next_item_seq(), 1);
        assert_eq!(tracker.next_item_seq(), 2);
    }

    #[test]
    fn format_transition_uses_label_with_icon() {
        let t = GitStatusTransition {
            previous: GitStatus::Modified,
            current:  GitStatus::Clean,
        };
        let rendered = format_transition("acme", &t);
        assert!(rendered.starts_with("acme: "));
        assert!(rendered.contains("modified"));
        assert!(rendered.contains("clean"));
        assert!(rendered.contains(" ──▶︎ "));
    }
}
