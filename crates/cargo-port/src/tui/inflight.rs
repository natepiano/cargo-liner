//! The `Inflight` subsystem.
//!
//! Phase 4 of the App-API carve (see `docs/app-api.md`). Absorbs
//! the in-flight bookkeeping App tracked across thirteen raw
//! fields:
//! - `running_clean_paths`, `running_lint_paths`
//! - `clean_toast`, `lint_toast`, `ci_fetch_toast`
//! - `ci_fetch_tracker`
//! - `pending_cleans`, `pending_ci_fetch`, `pending_example_run`
//! - `example_running`, `example_child`, `example_output`
//! - `lint_runtime` (relocated here from Background — `start_lint` is the only consumer, so
//!   co-locating runtime with start avoids cross-subsystem reach)
//!
//! Phase 4 absorbs the field cluster and exposes raw accessors;
//! the documented `start_*` / `finish_*` / `StartContext` cluster
//! lands incrementally as call sites migrate. Today every
//! existing call site touches multiple sub-fields directly via
//! the accessors below, matching the doc's "Phase 4 absorbs the
//! field cluster only" stance.

use std::collections::HashMap;
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Instant;

use super::app::CiFetchTracker;
use super::app::PendingClean;
use super::panes::PendingCiFetch;
use super::panes::PendingExampleRun;
use super::toasts::ToastTaskId;
use crate::lint::RuntimeHandle;
use crate::project::AbsolutePath;

/// Owns all in-flight bookkeeping App previously held as 13
/// separate fields. App holds a single `inflight: Inflight` after
/// Phase 4.
pub(super) struct Inflight {
    running_clean_paths: HashMap<AbsolutePath, Instant>,
    running_lint_paths:  HashMap<AbsolutePath, Instant>,
    clean_toast:         Option<ToastTaskId>,
    lint_toast:          Option<ToastTaskId>,
    ci_fetch_toast:      Option<ToastTaskId>,
    ci_fetch_tracker:    CiFetchTracker,
    pending_cleans:      VecDeque<PendingClean>,
    pending_ci_fetch:    Option<PendingCiFetch>,
    pending_example_run: Option<PendingExampleRun>,
    example_running:     Option<String>,
    example_child:       Arc<Mutex<Option<u32>>>,
    example_output:      Vec<String>,
    lint_runtime:        Option<RuntimeHandle>,
}

impl Inflight {
    pub(super) fn new(lint_runtime: Option<RuntimeHandle>) -> Self {
        Self {
            running_clean_paths: HashMap::new(),
            running_lint_paths: HashMap::new(),
            clean_toast: None,
            lint_toast: None,
            ci_fetch_toast: None,
            ci_fetch_tracker: CiFetchTracker::default(),
            pending_cleans: VecDeque::new(),
            pending_ci_fetch: None,
            pending_example_run: None,
            example_running: None,
            example_child: Arc::new(Mutex::new(None)),
            example_output: Vec::new(),
            lint_runtime,
        }
    }

    // ── running paths ───────────────────────────────────────────────

    pub(super) const fn running_clean_paths(&self) -> &HashMap<AbsolutePath, Instant> {
        &self.running_clean_paths
    }

    pub(super) const fn running_clean_paths_mut(&mut self) -> &mut HashMap<AbsolutePath, Instant> {
        &mut self.running_clean_paths
    }

    pub(super) const fn running_lint_paths(&self) -> &HashMap<AbsolutePath, Instant> {
        &self.running_lint_paths
    }

    pub(super) const fn running_lint_paths_mut(&mut self) -> &mut HashMap<AbsolutePath, Instant> {
        &mut self.running_lint_paths
    }

    // ── toast slots ─────────────────────────────────────────────────

    pub(super) const fn clean_toast(&self) -> Option<ToastTaskId> { self.clean_toast }

    pub(super) const fn set_clean_toast(&mut self, task_id: Option<ToastTaskId>) {
        self.clean_toast = task_id;
    }

    pub(super) const fn lint_toast(&self) -> Option<ToastTaskId> { self.lint_toast }

    pub(super) const fn set_lint_toast(&mut self, task_id: Option<ToastTaskId>) {
        self.lint_toast = task_id;
    }

    /// Test-only — production paths atomically consume the slot
    /// via [`Self::take_ci_fetch_toast`].
    #[cfg(test)]
    pub(super) const fn ci_fetch_toast(&self) -> Option<ToastTaskId> { self.ci_fetch_toast }

    pub(super) const fn set_ci_fetch_toast(&mut self, task_id: Option<ToastTaskId>) {
        self.ci_fetch_toast = task_id;
    }

    pub(super) const fn take_ci_fetch_toast(&mut self) -> Option<ToastTaskId> {
        self.ci_fetch_toast.take()
    }

    // ── ci fetch tracker ────────────────────────────────────────────

    pub(super) const fn ci_fetch_tracker(&self) -> &CiFetchTracker { &self.ci_fetch_tracker }

    pub(super) const fn ci_fetch_tracker_mut(&mut self) -> &mut CiFetchTracker {
        &mut self.ci_fetch_tracker
    }

    // ── pending queues ──────────────────────────────────────────────

    pub(super) const fn pending_cleans_mut(&mut self) -> &mut VecDeque<PendingClean> {
        &mut self.pending_cleans
    }

    pub(super) fn set_pending_ci_fetch(&mut self, fetch: PendingCiFetch) {
        self.pending_ci_fetch = Some(fetch);
    }

    /// Test-only inspection accessor — production paths consume
    /// the slot via [`Self::take_pending_ci_fetch`].
    #[cfg(test)]
    pub(super) const fn pending_ci_fetch_ref(&self) -> Option<&PendingCiFetch> {
        self.pending_ci_fetch.as_ref()
    }

    pub(super) const fn take_pending_ci_fetch(&mut self) -> Option<PendingCiFetch> {
        self.pending_ci_fetch.take()
    }

    pub(super) fn clear_pending_ci_fetch(&mut self) { self.pending_ci_fetch = None; }

    pub(super) fn set_pending_example_run(&mut self, run: PendingExampleRun) {
        self.pending_example_run = Some(run);
    }

    pub(super) const fn take_pending_example_run(&mut self) -> Option<PendingExampleRun> {
        self.pending_example_run.take()
    }

    // ── example runner ──────────────────────────────────────────────

    pub(super) fn example_running(&self) -> Option<&str> { self.example_running.as_deref() }

    pub(super) fn set_example_running(&mut self, running: Option<String>) {
        self.example_running = running;
    }

    pub(super) fn example_child(&self) -> Arc<Mutex<Option<u32>>> {
        Arc::clone(&self.example_child)
    }

    pub(super) fn example_output(&self) -> &[String] { &self.example_output }

    pub(super) const fn example_output_mut(&mut self) -> &mut Vec<String> {
        &mut self.example_output
    }

    pub(super) fn set_example_output(&mut self, output: Vec<String>) {
        self.example_output = output;
    }

    pub(super) const fn example_output_is_empty(&self) -> bool { self.example_output.is_empty() }

    // ── lint runtime ────────────────────────────────────────────────

    pub(super) const fn lint_runtime(&self) -> Option<&RuntimeHandle> { self.lint_runtime.as_ref() }

    pub(super) fn lint_runtime_clone(&self) -> Option<RuntimeHandle> { self.lint_runtime.clone() }

    pub(super) fn set_lint_runtime(&mut self, handle: Option<RuntimeHandle>) {
        self.lint_runtime = handle;
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
    use crate::tui::toasts::ToastTaskId;

    fn fresh() -> Inflight { Inflight::new(None) }

    fn abs(p: &str) -> AbsolutePath { AbsolutePath::from(PathBuf::from(p)) }

    #[test]
    fn new_starts_empty() {
        let inflight = fresh();
        assert!(inflight.running_clean_paths().is_empty());
        assert!(inflight.running_lint_paths().is_empty());
        assert!(inflight.clean_toast().is_none());
        assert!(inflight.lint_toast().is_none());
        assert!(inflight.ci_fetch_toast().is_none());
        assert!(inflight.example_running().is_none());
        assert!(inflight.example_output_is_empty());
        assert!(inflight.lint_runtime().is_none());
    }

    #[test]
    fn running_clean_paths_round_trip() {
        let mut inflight = fresh();
        let p = abs("/tmp/foo");
        inflight
            .running_clean_paths_mut()
            .insert(p.clone(), Instant::now());
        assert!(inflight.running_clean_paths().contains_key(&p));
        let removed = inflight.running_clean_paths_mut().remove(&p);
        assert!(removed.is_some());
        assert!(inflight.running_clean_paths().is_empty());
    }

    #[test]
    fn toast_slots_set_and_take() {
        let mut inflight = fresh();
        inflight.set_clean_toast(Some(ToastTaskId(7)));
        inflight.set_lint_toast(Some(ToastTaskId(8)));
        inflight.set_ci_fetch_toast(Some(ToastTaskId(9)));
        assert_eq!(inflight.clean_toast(), Some(ToastTaskId(7)));
        assert_eq!(inflight.lint_toast(), Some(ToastTaskId(8)));
        assert_eq!(inflight.ci_fetch_toast(), Some(ToastTaskId(9)));

        let taken = inflight.take_ci_fetch_toast();
        assert_eq!(taken, Some(ToastTaskId(9)));
        assert!(inflight.ci_fetch_toast().is_none());
        assert_eq!(
            inflight.clean_toast(),
            Some(ToastTaskId(7)),
            "take on ci_fetch slot must not affect siblings"
        );
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
