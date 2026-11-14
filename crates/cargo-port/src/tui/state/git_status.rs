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

/// The "Git status changes" task toast id that the tracker retains.
///
/// `Tracked` means an id is retained, not that the framework toast is still live.
#[derive(Clone, Copy, Default)]
pub(in crate::tui) enum GitStatusToastTracking {
    /// No toast id is retained.
    #[default]
    Untracked,
    /// The id of the last toast this tracker created.
    Tracked(ToastTaskId),
}

/// The outcome of one `GitStatusTracker::observe` call.
pub(in crate::tui) enum GitStatusObservation {
    /// The first observation for this path seeded its baseline.
    BaselineSeeded,
    /// The value equals the prior observation.
    Unchanged,
    /// The value differs from the prior observation.
    Changed(GitStatusTransition),
}

#[derive(Default)]
pub(in crate::tui) struct GitStatusTracker {
    entries:       HashMap<AbsolutePath, Baseline>,
    current_toast: GitStatusToastTracking,
    next_item_seq: u64,
}

/// A flipped git-status value worth surfacing.
pub(in crate::tui) struct GitStatusTransition {
    pub previous: GitStatus,
    pub current:  GitStatus,
}

impl GitStatusTracker {
    /// Record `current` and compare it with the prior observation. The first
    /// call for a given path returns `BaselineSeeded` and just seeds the baseline.
    pub(in crate::tui) fn observe(
        &mut self,
        path: AbsolutePath,
        current: GitStatus,
    ) -> GitStatusObservation {
        match self.entries.insert(path, Baseline::Seen(current)) {
            None | Some(Baseline::Unseen) => GitStatusObservation::BaselineSeeded,
            Some(Baseline::Seen(previous)) if previous == current => {
                GitStatusObservation::Unchanged
            },
            Some(Baseline::Seen(previous)) => {
                GitStatusObservation::Changed(GitStatusTransition { previous, current })
            },
        }
    }

    pub(in crate::tui) const fn current_toast(&self) -> GitStatusToastTracking {
        self.current_toast
    }

    pub(in crate::tui) const fn set_current_toast(&mut self, id: ToastTaskId) {
        self.current_toast = GitStatusToastTracking::Tracked(id);
    }

    /// Return a new sequence number for the next tracked item key, so that
    /// repeated transitions for one project stay separate items in one toast.
    pub(in crate::tui) const fn next_item_seq(&mut self) -> u64 {
        let seq = self.next_item_seq;
        self.next_item_seq = self.next_item_seq.wrapping_add(1);
        seq
    }
}

/// Render `acme: ● modified ──▶︎ ✓ clean` for one transition.
pub(in crate::tui) fn format_transition(name: &str, transition: &GitStatusTransition) -> String {
    format!(
        "{name}: {} ──▶︎ {}",
        transition.previous.label_with_icon(),
        transition.current.label_with_icon()
    )
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn path(name: &str) -> AbsolutePath {
        AbsolutePath::from(PathBuf::from(format!("/tmp/{name}")))
    }

    #[test]
    fn first_observation_seeds_baseline_silently() {
        let mut tracker = GitStatusTracker::default();
        assert!(matches!(
            tracker.observe(path("a"), GitStatus::Clean),
            GitStatusObservation::BaselineSeeded
        ));
    }

    #[test]
    fn same_value_does_not_toast() {
        let mut tracker = GitStatusTracker::default();
        let _ = tracker.observe(path("a"), GitStatus::Clean);
        assert!(matches!(
            tracker.observe(path("a"), GitStatus::Clean),
            GitStatusObservation::Unchanged
        ));
    }

    #[test]
    fn changed_value_toasts() {
        let mut tracker = GitStatusTracker::default();
        let _ = tracker.observe(path("a"), GitStatus::Clean);
        assert!(
            matches!(
                tracker.observe(path("a"), GitStatus::Modified),
                GitStatusObservation::Changed(GitStatusTransition {
                    previous: GitStatus::Clean,
                    current:  GitStatus::Modified,
                })
            ),
            "clean → modified should toast"
        );
    }

    #[test]
    fn back_to_clean_is_a_transition() {
        let mut tracker = GitStatusTracker::default();
        let _ = tracker.observe(path("a"), GitStatus::Modified);
        assert!(
            matches!(
                tracker.observe(path("a"), GitStatus::Clean),
                GitStatusObservation::Changed(GitStatusTransition {
                    previous: GitStatus::Modified,
                    current:  GitStatus::Clean,
                })
            ),
            "modified → clean should toast"
        );
    }

    #[test]
    fn per_path_baselines_are_independent() {
        let mut tracker = GitStatusTracker::default();
        let _ = tracker.observe(path("a"), GitStatus::Clean);
        let _ = tracker.observe(path("b"), GitStatus::Modified);
        // `GitStatusTracker::observe` sees b's unchanged `GitStatus::Modified` baseline, so no
        // toast.
        assert!(matches!(
            tracker.observe(path("b"), GitStatus::Modified),
            GitStatusObservation::Unchanged
        ));
        // `GitStatusTracker::observe` sees a change to `GitStatus::Untracked`, so a toasts.
        assert!(
            matches!(
                tracker.observe(path("a"), GitStatus::Untracked),
                GitStatusObservation::Changed(GitStatusTransition {
                    previous: GitStatus::Clean,
                    current:  GitStatus::Untracked,
                })
            ),
            "a flipped"
        );
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
