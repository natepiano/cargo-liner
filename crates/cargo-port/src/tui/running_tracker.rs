//! Generic in-flight tracker: a `HashMap<K, Instant>` of running
//! work paired with the single sticky `ToastTaskId` that displays
//! "N <thing> running."
//!
//! Phase 12.1 of the App-API extraction (see `docs/app-api.md`).
//! Extracted from the `running_paths` / `running_toast` field pair
//! that the [`Lint`](super::lint_state::Lint) subsystem held
//! after Phase 11.4a. Phase 12.2 adopts it for GitHub repo fetches
//! (`RunningTracker<OwnerRepo>`); Phase 13 may adopt it for CI.
//!
//! The tracker only owns state — it does not drive the toast itself.
//! Callers sync the toast via `App::sync_tracked_path_toast` (and its
//! GitHub counterpart), reading [`Self::running_map`] /
//! [`Self::toast`] and writing back through [`Self::set_toast`].
//! Phase 12.2 will retype the helper to take `&RunningTracker<K>`
//! directly.

use std::borrow::Borrow;
use std::collections::HashMap;
use std::hash::Hash;
use std::time::Instant;

use crate::tui::toasts::ToastTaskId;

pub struct RunningTracker<K: Eq + Hash> {
    running: HashMap<K, Instant>,
    toast:   Option<ToastTaskId>,
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

    /// Cfg-test today; non-test callers use [`Self::running_map`] +
    /// `HashMap::is_empty`. Phase 12.2 retypes
    /// `App::sync_tracked_path_toast` to take `&RunningTracker<K>`
    /// directly and drops the `#[cfg(test)]` gate.
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

    /// Borrow the underlying map. Required by
    /// `App::sync_tracked_path_toast` until Phase 12.2 retypes the
    /// helper to take `&RunningTracker<K>` directly.
    pub const fn running_map(&self) -> &HashMap<K, Instant> { &self.running }

    pub const fn toast(&self) -> Option<ToastTaskId> { self.toast }

    pub const fn set_toast(&mut self, t: Option<ToastTaskId>) { self.toast = t; }

    pub fn clear(&mut self) {
        self.running.clear();
        self.toast = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_is_empty_with_no_toast() {
        let t: RunningTracker<String> = RunningTracker::new();
        assert!(t.is_empty());
        assert!(t.toast().is_none());
        assert!(t.running_map().is_empty());
    }

    #[test]
    fn insert_returns_true_for_new_key_false_for_existing() {
        let mut t: RunningTracker<String> = RunningTracker::new();
        let now = Instant::now();
        assert!(t.insert("a".into(), now));
        assert!(!t.insert("a".into(), now));
        assert!(t.insert("b".into(), now));
        assert_eq!(t.running_map().len(), 2);
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
        t.set_toast(Some(ToastTaskId(42)));
        assert_eq!(t.toast(), Some(ToastTaskId(42)));
        t.set_toast(None);
        assert!(t.toast().is_none());
    }

    #[test]
    fn clear_drops_running_and_toast() {
        let mut t: RunningTracker<String> = RunningTracker::new();
        t.insert("a".into(), Instant::now());
        t.set_toast(Some(ToastTaskId(1)));
        t.clear();
        assert!(t.is_empty());
        assert!(t.toast().is_none());
    }
}
