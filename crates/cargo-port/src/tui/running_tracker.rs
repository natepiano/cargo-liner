//! Generic in-flight tracker: a `HashMap<K, Instant>` of running
//! work paired with the single sticky `ToastTaskId` that displays
//! "N <thing> running."
//!
//! Used by the [`Lint`](super::lint_state::Lint) subsystem
//! (`RunningTracker<AbsolutePath>`), GitHub repo fetches
//! (`RunningTracker<OwnerRepo>`), and `Inflight.clean`. Not used
//! for CI: `Ci.fetch_toast` is fire-once (consumed via
//! `take_fetch_toast` at fetch completion) rather than
//! sticky-during-flight, and `Ci.fetch_tracker` is a bare
//! `HashSet<AbsolutePath>` with no started-at timestamp.
//!
//! The tracker only owns state — it does not drive the toast itself.
//! Callers sync the toast via `App::sync_tracked_path_toast` (and its
//! GitHub counterpart), reading the `running` / `toast` fields directly.

use std::borrow::Borrow;
use std::collections::HashMap;
use std::hash::Hash;
use std::time::Instant;

use super::toasts::ToastTaskId;
use super::toasts::TrackedItem;
use super::toasts::TrackedItemKey;

pub struct RunningTracker<K: Eq + Hash> {
    pub running: HashMap<K, Instant>,
    pub toast:   Option<ToastTaskId>,
}

impl<K: Eq + Hash> Default for RunningTracker<K> {
    fn default() -> Self { Self::new() }
}

impl<K: Eq + Hash> RunningTracker<K> {
    pub fn new() -> Self {
        Self {
            running: HashMap::new(),
            toast:   None,
        }
    }

    /// Test-only today; non-test callers materialize a
    /// `Vec<TrackedItem>` and check that for emptiness instead.
    #[cfg(test)]
    pub fn is_empty(&self) -> bool { self.running.is_empty() }

    /// Insert `k` with `started`. Returns `true` when the key was
    /// not previously running (the run is new), `false` when it was
    /// already in flight (the timestamp is overwritten).
    pub fn insert(&mut self, k: K, started: Instant) -> bool {
        self.running.insert(k, started).is_none()
    }

    pub fn remove<Q>(&mut self, k: &Q) -> Option<Instant>
    where
        K: Borrow<Q>,
        Q: ?Sized + Eq + Hash,
    {
        self.running.remove(k)
    }

    pub fn clear(&mut self) {
        self.running.clear();
        self.toast = None;
    }

    /// Collect this tracker into the data the toast helper needs:
    /// the current toast slot plus a `TrackedItem` per running key.
    /// Done as a non-`&mut` reader so the borrow on the
    /// subsystem-owned tracker is released before the caller hands
    /// the items to the toast-sync sink (which takes `&mut self`).
    pub fn items_for_toast<F>(&self, label_fn: F) -> (Option<ToastTaskId>, Vec<TrackedItem>)
    where
        for<'a> &'a K: Into<TrackedItemKey>,
        F: Fn(&K) -> String,
    {
        let items = self
            .running
            .iter()
            .map(|(k, &started)| TrackedItem {
                label:        label_fn(k),
                key:          k.into(),
                started_at:   Some(started),
                completed_at: None,
            })
            .collect();
        (self.toast, items)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_is_empty_with_no_toast() {
        let t: RunningTracker<String> = RunningTracker::new();
        assert!(t.is_empty());
        assert!(t.toast.is_none());
        assert!(t.running.is_empty());
    }

    #[test]
    fn insert_returns_true_for_new_key_false_for_existing() {
        let mut t: RunningTracker<String> = RunningTracker::new();
        let now = Instant::now();
        assert!(t.insert("a".into(), now));
        assert!(!t.insert("a".into(), now));
        assert!(t.insert("b".into(), now));
        assert_eq!(t.running.len(), 2);
    }

    #[test]
    fn remove_returns_started_instant_when_present() {
        let mut t: RunningTracker<String> = RunningTracker::new();
        let now = Instant::now();
        t.insert("a".into(), now);
        assert_eq!(t.remove("a"), Some(now));
        assert!(t.remove("a").is_none());
        assert!(t.is_empty());
    }

    #[test]
    fn toast_round_trip() {
        let mut t: RunningTracker<String> = RunningTracker::new();
        t.toast = Some(ToastTaskId(42));
        assert_eq!(t.toast, Some(ToastTaskId(42)));
        t.toast = None;
        assert!(t.toast.is_none());
    }

    #[test]
    fn clear_drops_running_and_toast() {
        let mut t: RunningTracker<String> = RunningTracker::new();
        t.insert("a".into(), Instant::now());
        t.toast = Some(ToastTaskId(1));
        t.clear();
        assert!(t.is_empty());
        assert!(t.toast.is_none());
    }
}
