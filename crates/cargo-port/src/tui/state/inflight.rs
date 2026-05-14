//! The `Inflight` subsystem.
//!
//! Owns App's in-flight bookkeeping:
//! - clean: in-flight cargo clean paths plus the running-clean toast slot
//! - `pending_cleans`, `pending_ci_fetch`, `pending_example_run`
//! - `example_running`, `example_child`, `example_output`
//!
//! Lint lifecycle (`runtime`, running paths, toast) lives on
//! [`Lint`](super::state::Lint); CI fetch lifecycle lives on
//! [`Ci`](super::state::Ci).

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::Mutex;

use tui_pane::RunningTracker;

use crate::project::AbsolutePath;
use crate::tui::app::PendingClean;
use crate::tui::panes::PendingCiFetch;
use crate::tui::panes::PendingExampleRun;

/// Owns App's in-flight bookkeeping. App holds a single
/// `inflight: Inflight`.
pub struct Inflight {
    /// In-flight cargo clean state — same lifecycle as
    /// `Lint::running` and `Github::running`.
    clean:               RunningTracker<AbsolutePath>,
    pending_cleans:      VecDeque<PendingClean>,
    pending_ci_fetch:    Option<PendingCiFetch>,
    pending_example_run: Option<PendingExampleRun>,
    example_running:     Option<String>,
    example_child:       Arc<Mutex<Option<u32>>>,
    example_output:      Vec<String>,
}

impl Inflight {
    pub fn new() -> Self {
        Self {
            clean:               RunningTracker::new(),
            pending_cleans:      VecDeque::new(),
            pending_ci_fetch:    None,
            pending_example_run: None,
            example_running:     None,
            example_child:       Arc::new(Mutex::new(None)),
            example_output:      Vec::new(),
        }
    }

    // ── running clean tracker ───────────────────────────────────────

    pub const fn clean(&self) -> &RunningTracker<AbsolutePath> { &self.clean }

    pub const fn clean_mut(&mut self) -> &mut RunningTracker<AbsolutePath> { &mut self.clean }

    // ── pending queues ──────────────────────────────────────────────

    pub const fn pending_cleans_mut(&mut self) -> &mut VecDeque<PendingClean> {
        &mut self.pending_cleans
    }

    pub fn set_pending_ci_fetch(&mut self, fetch: PendingCiFetch) {
        self.pending_ci_fetch = Some(fetch);
    }

    /// Test-only inspection accessor — production paths consume
    /// the slot via [`Self::take_pending_ci_fetch`].
    #[cfg(test)]
    pub const fn pending_ci_fetch_ref(&self) -> Option<&PendingCiFetch> {
        self.pending_ci_fetch.as_ref()
    }

    pub const fn take_pending_ci_fetch(&mut self) -> Option<PendingCiFetch> {
        self.pending_ci_fetch.take()
    }

    pub fn clear_pending_ci_fetch(&mut self) { self.pending_ci_fetch = None; }

    pub fn set_pending_example_run(&mut self, run: PendingExampleRun) {
        self.pending_example_run = Some(run);
    }

    pub const fn take_pending_example_run(&mut self) -> Option<PendingExampleRun> {
        self.pending_example_run.take()
    }

    // ── example runner ──────────────────────────────────────────────

    pub fn example_running(&self) -> Option<&str> { self.example_running.as_deref() }

    pub fn set_example_running(&mut self, running: Option<String>) {
        self.example_running = running;
    }

    pub fn example_child(&self) -> Arc<Mutex<Option<u32>>> { Arc::clone(&self.example_child) }

    pub fn example_output(&self) -> &[String] { &self.example_output }

    pub const fn example_output_mut(&mut self) -> &mut Vec<String> { &mut self.example_output }

    pub fn set_example_output(&mut self, output: Vec<String>) { self.example_output = output; }

    pub const fn example_output_is_empty(&self) -> bool { self.example_output.is_empty() }

    pub fn apply_example_progress(&mut self, line: String) {
        if let Some(last) = self.example_output.last_mut() {
            *last = line;
        } else {
            self.example_output.push(line);
        }
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
    use std::time::Instant;

    use tui_pane::ToastTaskId;

    use super::*;

    fn fresh() -> Inflight { Inflight::new() }

    fn abs(p: &str) -> AbsolutePath { AbsolutePath::from(PathBuf::from(p)) }

    #[test]
    fn new_starts_empty() {
        let inflight = fresh();
        assert!(inflight.clean().is_empty());
        assert!(inflight.clean().toast.is_none());
        assert!(inflight.example_running().is_none());
        assert!(inflight.example_output_is_empty());
    }

    #[test]
    fn running_clean_paths_round_trip() {
        let mut inflight = fresh();
        let p = abs("/tmp/foo");
        inflight.clean_mut().insert(p.clone(), Instant::now());
        assert!(inflight.clean().running.contains_key(&p));
        let removed = inflight.clean_mut().remove(&p);
        assert!(removed.is_some());
        assert!(inflight.clean().is_empty());
    }

    #[test]
    fn clean_toast_round_trip() {
        let mut inflight = fresh();
        inflight.clean_mut().toast = Some(ToastTaskId(7));
        assert_eq!(inflight.clean().toast, Some(ToastTaskId(7)));
        inflight.clean_mut().toast = None;
        assert!(inflight.clean().toast.is_none());
    }

    #[test]
    fn pending_ci_fetch_set_take_clear() {
        use crate::tui::panes::CiFetchKind;

        fn fixture() -> PendingCiFetch {
            PendingCiFetch {
                project_path:      "/tmp/proj".into(),
                ci_run_count:      5,
                oldest_created_at: None,
                kind:              CiFetchKind::Sync,
            }
        }

        let mut inflight = fresh();
        inflight.set_pending_ci_fetch(fixture());
        let taken = inflight.take_pending_ci_fetch();
        assert!(taken.is_some());
        assert!(inflight.take_pending_ci_fetch().is_none());

        inflight.set_pending_ci_fetch(fixture());
        inflight.clear_pending_ci_fetch();
        assert!(inflight.take_pending_ci_fetch().is_none());
    }

    #[test]
    fn example_output_round_trip() {
        let mut inflight = fresh();
        inflight.example_output_mut().push("first line".to_string());
        assert_eq!(inflight.example_output(), &["first line".to_string()]);
        assert!(!inflight.example_output_is_empty());

        inflight.set_example_output(vec!["replaced".to_string()]);
        assert_eq!(inflight.example_output(), &["replaced".to_string()]);
    }

    #[test]
    fn pending_cleans_queue_is_fifo() {
        let mut inflight = fresh();
        inflight.pending_cleans_mut().push_back(PendingClean {
            abs_path: abs("/tmp/a"),
        });
        inflight.pending_cleans_mut().push_back(PendingClean {
            abs_path: abs("/tmp/b"),
        });

        let first = inflight.pending_cleans_mut().pop_front();
        assert_eq!(
            first.unwrap().abs_path.as_path(),
            std::path::Path::new("/tmp/a"),
            "FIFO ordering preserved"
        );
        let second = inflight.pending_cleans_mut().pop_front();
        assert_eq!(
            second.unwrap().abs_path.as_path(),
            std::path::Path::new("/tmp/b")
        );
        assert!(inflight.pending_cleans_mut().pop_front().is_none());
    }

    #[test]
    fn example_child_arc_clone_shares_state() {
        let inflight = fresh();
        let child = inflight.example_child();
        *child.lock().expect("lock pid slot") = Some(42);
        assert_eq!(*inflight.example_child().lock().unwrap(), Some(42));
    }
}
