use std::collections::HashSet;
use std::time::Instant;

use crate::perf_log;
use crate::tui::app::App;
use crate::tui::app::phase_state::PhaseCompletion;
use crate::tui::app::startup;
use crate::tui::constants::STARTUP_PHASE_DISK;
use crate::tui::constants::STARTUP_PHASE_GIT;
use crate::tui::constants::STARTUP_PHASE_GITHUB;
use crate::tui::constants::STARTUP_PHASE_LINT;
use crate::tui::constants::STARTUP_PHASE_METADATA;
use crate::tui::toasts::TrackedItem;

impl App {
    pub fn initialize_startup_phase_tracker(&mut self) {
        self.reset_startup_phase_state();
        self.start_startup_toast();
        self.start_startup_detail_toasts();
        self.log_startup_phase_plan();
        self.maybe_log_startup_phase_completions();
    }
    pub(super) fn reset_startup_phase_state(&mut self) {
        let disk_expected = startup::initial_disk_roots(self.projects());
        let git_expected = self
            .projects()
            .git_directories()
            .into_iter()
            .collect::<HashSet<_>>();
        let git_seen = self
            .projects()
            .iter()
            .filter(|entry| entry.item.git_info().is_some())
            .filter_map(|entry| entry.item.git_directory())
            .collect::<HashSet<_>>();
        let metadata_expected = startup::initial_metadata_roots(self.projects());
        self.scan.scan_state_mut().startup_phases.scan_complete_at = Some(Instant::now());
        self.scan.scan_state_mut().startup_phases.startup_toast = None;
        self.scan
            .scan_state_mut()
            .startup_phases
            .startup_complete_at = None;
        self.scan
            .scan_state_mut()
            .startup_phases
            .disk
            .reset_with_expected(disk_expected);
        self.scan
            .scan_state_mut()
            .startup_phases
            .git
            .reset_with_expected(git_expected);
        self.scan.scan_state_mut().startup_phases.git.seen = git_seen;
        self.scan
            .scan_state_mut()
            .startup_phases
            .repo
            .reset_with_expected(HashSet::new());
        self.lint.phase.reset_with_expected(HashSet::new());
        self.scan
            .scan_state_mut()
            .startup_phases
            .metadata
            .reset_with_expected(metadata_expected);
    }
    pub(super) fn start_startup_toast(&mut self) {
        let now = Instant::now();
        let startup_items = vec![
            TrackedItem {
                label:        STARTUP_PHASE_DISK.to_string(),
                key:          STARTUP_PHASE_DISK.into(),
                started_at:   Some(now),
                completed_at: None,
            },
            TrackedItem {
                label:        STARTUP_PHASE_GIT.to_string(),
                key:          STARTUP_PHASE_GIT.into(),
                started_at:   Some(now),
                completed_at: None,
            },
            TrackedItem {
                label:        STARTUP_PHASE_METADATA.to_string(),
                key:          STARTUP_PHASE_METADATA.into(),
                started_at:   Some(now),
                completed_at: None,
            },
            TrackedItem {
                label:        STARTUP_PHASE_LINT.to_string(),
                key:          STARTUP_PHASE_LINT.into(),
                started_at:   Some(now),
                completed_at: None,
            },
        ];
        let task_id = self.start_task_toast("Startup", "");
        self.set_task_tracked_items(task_id, &startup_items);
        self.scan.scan_state_mut().startup_phases.startup_toast = Some(task_id);
    }
    pub(super) fn start_startup_detail_toasts(&mut self) {
        if let Some(disk_expected) = self
            .scan
            .scan_state_mut()
            .startup_phases
            .disk
            .expected
            .clone()
        {
            let disk_items = Self::tracked_items_for_startup(
                &disk_expected,
                &self.scan.scan_state_mut().startup_phases.disk.seen,
            );
            if !disk_items.is_empty() {
                let body = self.startup_disk_toast_body();
                let task_id = self.start_task_toast("Calculating disk usage", &body);
                self.set_task_tracked_items(task_id, &disk_items);
                self.scan.scan_state_mut().startup_phases.disk.toast = Some(task_id);
            }
        }

        if let Some(git_expected) = self
            .scan
            .scan_state_mut()
            .startup_phases
            .git
            .expected
            .clone()
        {
            let git_items = Self::tracked_items_for_startup(
                &git_expected,
                &self.scan.scan_state_mut().startup_phases.git.seen,
            );
            if !git_items.is_empty() {
                let body = self.startup_git_toast_body();
                let task_id = self.start_task_toast("Scanning local git repos", &body);
                self.set_task_tracked_items(task_id, &git_items);
                self.scan.scan_state_mut().startup_phases.git.toast = Some(task_id);
            }
        }
        if let Some(metadata_expected) = self
            .scan
            .scan_state_mut()
            .startup_phases
            .metadata
            .expected
            .clone()
        {
            let metadata_items = Self::tracked_items_for_startup(
                &metadata_expected,
                &self.scan.scan_state_mut().startup_phases.metadata.seen,
            );
            if !metadata_items.is_empty() {
                let body = self.startup_metadata_toast_body();
                let task_id = self.start_task_toast("Running cargo metadata", &body);
                self.set_task_tracked_items(task_id, &metadata_items);
                self.scan.scan_state_mut().startup_phases.metadata.toast = Some(task_id);
            }
        }
        // The "Retrieving GitHub repo details" toast is driven by
        // `sync_running_repo_fetch_toast` from live `RepoFetchQueued`
        // messages — no separate startup-phase toast here.
    }
    pub(super) fn log_startup_phase_plan(&self) {
        tracing::info!(
            disk_expected = self.scan.scan_state().startup_phases.disk.expected_len(),
            git_expected = self.scan.scan_state().startup_phases.git.expected_len(),
            repo_expected = self.scan.scan_state().startup_phases.repo.expected_len(),
            lint_expected = self.lint.phase.expected_len(),
            metadata_expected = self
                .scan
                .scan_state()
                .startup_phases
                .metadata
                .expected_len(),
            "startup_phase_plan"
        );
    }
    pub fn maybe_log_startup_phase_completions(&mut self) {
        let Some(scan_complete_at) = self.scan.scan_state_mut().startup_phases.scan_complete_at
        else {
            return;
        };
        let now = Instant::now();
        self.maybe_complete_startup_disk(now, scan_complete_at);
        self.maybe_complete_startup_git(now, scan_complete_at);
        self.maybe_complete_startup_repo(now, scan_complete_at);
        self.maybe_complete_startup_metadata(now, scan_complete_at);
        self.maybe_complete_startup_lints(now, scan_complete_at);
        self.maybe_complete_startup_ready(now, scan_complete_at);
    }
    pub fn maybe_complete_startup_disk(&mut self, now: Instant, scan_complete_at: Instant) {
        if !self
            .scan
            .scan_state_mut()
            .startup_phases
            .disk
            .complete_once(now)
        {
            return;
        }
        if let Some(disk_toast) = self.scan.scan_state_mut().startup_phases.disk.take_toast() {
            self.finish_task_toast(disk_toast);
        }
        if let Some(toast) = self.scan.scan_state_mut().startup_phases.startup_toast {
            self.mark_tracked_item_completed(toast, STARTUP_PHASE_DISK);
        }
        tracing::info!(
            phase = "disk_applied",
            since_scan_complete_ms =
                crate::perf_log::ms(now.duration_since(scan_complete_at).as_millis()),
            seen = self.scan.scan_state_mut().startup_phases.disk.seen.len(),
            expected = self
                .scan
                .scan_state_mut()
                .startup_phases
                .disk
                .expected_len(),
            "startup_phase_complete"
        );
    }
    pub fn maybe_complete_startup_git(&mut self, now: Instant, scan_complete_at: Instant) {
        if !self
            .scan
            .scan_state_mut()
            .startup_phases
            .git
            .complete_once(now)
        {
            return;
        }
        if let Some(git_toast) = self.scan.scan_state_mut().startup_phases.git.take_toast() {
            self.finish_task_toast(git_toast);
        }
        if let Some(toast) = self.scan.scan_state_mut().startup_phases.startup_toast {
            self.mark_tracked_item_completed(toast, STARTUP_PHASE_GIT);
        }
        tracing::info!(
            phase = "git_local_applied",
            since_scan_complete_ms =
                crate::perf_log::ms(now.duration_since(scan_complete_at).as_millis()),
            seen = self.scan.scan_state_mut().startup_phases.git.seen.len(),
            expected = self.scan.scan_state_mut().startup_phases.git.expected_len(),
            "startup_phase_complete"
        );
    }
    pub fn maybe_complete_startup_repo(&mut self, now: Instant, scan_complete_at: Instant) {
        // Gate repo-phase completion on git-phase completion. Without
        // this, a scan that completes before any `RepoFetchQueued`
        // arrives would see `repo.seen (0) >= repo.expected (0)` and
        // mark the phase done prematurely; subsequent staggered git
        // arrivals would then strand their repo fetches outside the
        // startup toast.
        if self
            .scan
            .scan_state_mut()
            .startup_phases
            .git
            .complete_at
            .is_none()
        {
            return;
        }
        if !self
            .scan
            .scan_state_mut()
            .startup_phases
            .repo
            .complete_once(now)
        {
            return;
        }
        if let Some(toast) = self.scan.scan_state_mut().startup_phases.startup_toast {
            self.mark_tracked_item_completed(toast, STARTUP_PHASE_GITHUB);
        }
        tracing::info!(
            phase = "repo_fetch_applied",
            since_scan_complete_ms =
                crate::perf_log::ms(now.duration_since(scan_complete_at).as_millis()),
            seen = self.scan.scan_state_mut().startup_phases.repo.seen.len(),
            expected = self
                .scan
                .scan_state_mut()
                .startup_phases
                .repo
                .expected_len(),
            "startup_phase_complete"
        );
    }
    pub fn maybe_complete_startup_metadata(&mut self, now: Instant, scan_complete_at: Instant) {
        if !self
            .scan
            .scan_state_mut()
            .startup_phases
            .metadata
            .complete_once(now)
        {
            return;
        }
        if let Some(metadata_toast) = self
            .scan
            .scan_state_mut()
            .startup_phases
            .metadata
            .take_toast()
        {
            self.finish_task_toast(metadata_toast);
        }
        if let Some(toast) = self.scan.scan_state_mut().startup_phases.startup_toast {
            self.mark_tracked_item_completed(toast, STARTUP_PHASE_METADATA);
        }
        tracing::info!(
            phase = "metadata_applied",
            since_scan_complete_ms =
                crate::perf_log::ms(now.duration_since(scan_complete_at).as_millis()),
            seen = self
                .scan
                .scan_state_mut()
                .startup_phases
                .metadata
                .seen
                .len(),
            expected = self
                .scan
                .scan_state_mut()
                .startup_phases
                .metadata
                .expected_len(),
            "startup_phase_complete"
        );
    }
    pub fn maybe_complete_startup_lints(&mut self, now: Instant, scan_complete_at: Instant) {
        // Lint is only "complete" once real lint work has been registered —
        // an initialized-empty expected set stays open. This diverges from
        // the generic `PhaseCompletion::is_complete` semantics on purpose,
        // so the check stays inline rather than going through the trait.
        let lint = &self.lint.phase;
        let should_complete = lint.complete_at.is_none()
            && lint
                .expected
                .as_ref()
                .is_some_and(|expected| !expected.is_empty() && lint.seen.len() >= expected.len());
        if !should_complete {
            return;
        }
        self.lint.phase.complete_at = Some(now);
        tracing::info!(
            phase = "lint_terminal_applied",
            since_scan_complete_ms =
                crate::perf_log::ms(now.duration_since(scan_complete_at).as_millis()),
            seen = self.lint.phase.seen.len(),
            expected = self.lint.phase.expected_len(),
            "startup_phase_complete"
        );
    }
    pub fn maybe_complete_startup_ready(&mut self, now: Instant, scan_complete_at: Instant) {
        // Pull lint-startup signals first so the &mut borrow of
        // `self.scan` below doesn't conflict with `self.lint`.
        let lint_done = self.lint.startup_phase.complete_at.is_some();
        let lint_seen = self.lint.phase.seen.len();
        let lint_expected = self.lint.phase.expected_len();
        let phases = self.scan.scan_state_mut();
        if phases.startup_phases.startup_complete_at.is_some() {
            return;
        }
        let disk_ready = phases.startup_phases.disk.complete_at.is_some();
        let git_ready = phases.startup_phases.git.complete_at.is_some();
        let repo_ready = phases.startup_phases.repo.complete_at.is_some();
        let metadata_ready = phases.startup_phases.metadata.complete_at.is_some();
        if !(disk_ready && git_ready && repo_ready && metadata_ready) {
            return;
        }
        phases.startup_phases.startup_complete_at = Some(now);
        // Finish the startup toast only when lint startup cache check
        // is also done, so "Lint cache" doesn't spin while the toast
        // exits.
        if lint_done && let Some(toast) = phases.startup_phases.startup_toast.take() {
            self.finish_task_toast(toast);
        }
        let phases = self.scan.scan_state();
        let since_scan_ms = perf_log::ms(now.duration_since(scan_complete_at).as_millis());
        tracing::info!(
            since_scan_complete_ms = since_scan_ms,
            disk_seen = phases.startup_phases.disk.seen.len(),
            disk_expected = phases.startup_phases.disk.expected_len(),
            git_seen = phases.startup_phases.git.seen.len(),
            git_expected = phases.startup_phases.git.expected_len(),
            repo_seen = phases.startup_phases.repo.seen.len(),
            repo_expected = phases.startup_phases.repo.expected_len(),
            lint_seen = lint_seen,
            lint_expected = lint_expected,
            metadata_seen = phases.startup_phases.metadata.seen.len(),
            metadata_expected = phases.startup_phases.metadata.expected_len(),
            "startup_complete"
        );
        tracing::info!(since_scan_complete_ms = since_scan_ms, "steady_state_begin");
    }
}
