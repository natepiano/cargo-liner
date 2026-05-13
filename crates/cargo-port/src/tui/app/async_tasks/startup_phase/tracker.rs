use std::collections::HashSet;
use std::time::Instant;

use tui_pane::TrackedItem;

use crate::perf_log;
use crate::project;
use crate::tui::app::App;
use crate::tui::app::Startup;
use crate::tui::app::phase_state::PhaseCompletion;
use crate::tui::app::startup;
use crate::tui::constants::STARTUP_PHASE_DISK;
use crate::tui::constants::STARTUP_PHASE_GIT;
use crate::tui::constants::STARTUP_PHASE_GITHUB;
use crate::tui::constants::STARTUP_PHASE_LINT;
use crate::tui::constants::STARTUP_PHASE_METADATA;
use crate::tui::integration::toast_adapters;

impl Startup {
    pub(super) fn log_phase_plan(&self) {
        tracing::info!(
            disk_expected = self.disk.expected_len(),
            git_expected = self.git.expected_len(),
            repo_expected = self.repo.expected_len(),
            lint_expected = self.lint_phase.expected_len(),
            metadata_expected = self.metadata.expected_len(),
            "startup_phase_plan"
        );
    }
    pub(super) fn maybe_complete_lints(&mut self, now: Instant, scan_complete_at: Instant) {
        // Lint is only "complete" once real lint work has been registered —
        // an initialized-empty expected set stays open. This diverges from
        // the generic `PhaseCompletion::is_complete` semantics on purpose,
        // so the check stays inline rather than going through the trait.
        let lint = &self.lint_phase;
        let should_complete = lint.complete_at.is_none()
            && lint
                .expected
                .as_ref()
                .is_some_and(|expected| !expected.is_empty() && lint.seen.len() >= expected.len());
        if !should_complete {
            return;
        }
        self.lint_phase.complete_at = Some(now);
        tracing::info!(
            phase = "lint_terminal_applied",
            since_scan_complete_ms =
                crate::perf_log::ms(now.duration_since(scan_complete_at).as_millis()),
            seen = self.lint_phase.seen.len(),
            expected = self.lint_phase.expected_len(),
            "startup_phase_complete"
        );
    }
}

impl App {
    pub fn initialize_startup_phase_tracker(&mut self) {
        self.reset_startup_phase_state();
        self.start_startup_toast();
        self.start_startup_detail_toasts();
        self.startup.log_phase_plan();
        self.maybe_log_startup_phase_completions();
    }
    pub(super) fn reset_startup_phase_state(&mut self) {
        let disk_expected = startup::initial_disk_roots(&self.project_list);
        let git_expected = self
            .project_list
            .git_directories()
            .into_iter()
            .collect::<HashSet<_>>();
        let git_seen = self
            .project_list
            .iter()
            .filter(|entry| entry.item.git_info().is_some())
            .filter_map(|entry| entry.item.git_directory())
            .collect::<HashSet<_>>();
        let metadata_expected = startup::initial_metadata_roots(&self.project_list);
        self.startup.scan_complete_at = Some(Instant::now());
        self.startup.toast = None;
        self.startup.complete_at = None;
        self.startup.disk.reset_with_expected(disk_expected);
        self.startup.git.reset_with_expected(git_expected);
        self.startup.git.seen = git_seen;
        self.startup.repo.reset_with_expected(HashSet::new());
        self.startup.lint_phase.reset_with_expected(HashSet::new());
        self.startup.metadata.reset_with_expected(metadata_expected);
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
        let task_id = self.framework.toasts.start_task("Startup", "");
        self.set_task_tracked_items(task_id, &startup_items);
        self.startup.toast = Some(task_id);
    }
    pub(super) fn start_startup_detail_toasts(&mut self) {
        if self.startup.disk.expected.is_some() {
            let disk_items = self.startup.disk.tracked_items(
                |p| project::home_relative_path(p.as_path()),
                toast_adapters::path_key,
            );
            if !disk_items.is_empty() {
                let body = self
                    .startup
                    .disk_toast_body(self.framework.toast_settings());
                let task_id = self
                    .framework
                    .toasts
                    .start_task("Calculating disk usage", &body);
                self.set_task_tracked_items(task_id, &disk_items);
                self.startup.disk.toast = Some(task_id);
            }
        }

        if self.startup.git.expected.is_some() {
            let git_items = self.startup.git.tracked_items(
                |p| project::home_relative_path(p.as_path()),
                toast_adapters::path_key,
            );
            if !git_items.is_empty() {
                let body = self.startup.git_toast_body(self.framework.toast_settings());
                let task_id = self
                    .framework
                    .toasts
                    .start_task("Scanning local git repos", &body);
                self.set_task_tracked_items(task_id, &git_items);
                self.startup.git.toast = Some(task_id);
            }
        }
        if self.startup.metadata.expected.is_some() {
            let metadata_items = self.startup.metadata.tracked_items(
                |p| project::home_relative_path(p.as_path()),
                toast_adapters::path_key,
            );
            if !metadata_items.is_empty() {
                let body = self
                    .startup
                    .metadata_toast_body(self.framework.toast_settings());
                let task_id = self
                    .framework
                    .toasts
                    .start_task("Running cargo metadata", &body);
                self.set_task_tracked_items(task_id, &metadata_items);
                self.startup.metadata.toast = Some(task_id);
            }
        }
        // The "Retrieving GitHub repo details" toast is driven by
        // `sync_running_repo_fetch_toast` from live `RepoFetchQueued`
        // messages — no separate startup-phase toast here.
    }
    pub fn maybe_log_startup_phase_completions(&mut self) {
        let Some(scan_complete_at) = self.startup.scan_complete_at else {
            return;
        };
        let now = Instant::now();
        self.maybe_complete_startup_disk(now, scan_complete_at);
        self.maybe_complete_startup_git(now, scan_complete_at);
        self.maybe_complete_startup_repo(now, scan_complete_at);
        self.maybe_complete_startup_metadata(now, scan_complete_at);
        self.startup.maybe_complete_lints(now, scan_complete_at);
        self.maybe_complete_startup_ready(now, scan_complete_at);
    }
    pub fn maybe_complete_startup_disk(&mut self, now: Instant, scan_complete_at: Instant) {
        if !self.startup.disk.complete_once(now) {
            return;
        }
        if let Some(disk_toast) = self.startup.disk.take_toast() {
            self.finish_task_toast(disk_toast);
        }
        if let Some(toast) = self.startup.toast {
            self.framework
                .toasts
                .mark_tracked_item_completed(toast, STARTUP_PHASE_DISK);
        }
        tracing::info!(
            phase = "disk_applied",
            since_scan_complete_ms =
                crate::perf_log::ms(now.duration_since(scan_complete_at).as_millis()),
            seen = self.startup.disk.seen.len(),
            expected = self.startup.disk.expected_len(),
            "startup_phase_complete"
        );
    }
    pub fn maybe_complete_startup_git(&mut self, now: Instant, scan_complete_at: Instant) {
        if !self.startup.git.complete_once(now) {
            return;
        }
        if let Some(git_toast) = self.startup.git.take_toast() {
            self.finish_task_toast(git_toast);
        }
        if let Some(toast) = self.startup.toast {
            self.framework
                .toasts
                .mark_tracked_item_completed(toast, STARTUP_PHASE_GIT);
        }
        tracing::info!(
            phase = "git_local_applied",
            since_scan_complete_ms =
                crate::perf_log::ms(now.duration_since(scan_complete_at).as_millis()),
            seen = self.startup.git.seen.len(),
            expected = self.startup.git.expected_len(),
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
        if self.startup.git.complete_at.is_none() {
            return;
        }
        if !self.startup.repo.complete_once(now) {
            return;
        }
        if let Some(toast) = self.startup.toast {
            self.framework
                .toasts
                .mark_tracked_item_completed(toast, STARTUP_PHASE_GITHUB);
        }
        tracing::info!(
            phase = "repo_fetch_applied",
            since_scan_complete_ms =
                crate::perf_log::ms(now.duration_since(scan_complete_at).as_millis()),
            seen = self.startup.repo.seen.len(),
            expected = self.startup.repo.expected_len(),
            "startup_phase_complete"
        );
    }
    pub(super) fn maybe_complete_startup_metadata(
        &mut self,
        now: Instant,
        scan_complete_at: Instant,
    ) {
        if !self.startup.metadata.complete_once(now) {
            return;
        }
        if let Some(metadata_toast) = self.startup.metadata.take_toast() {
            self.finish_task_toast(metadata_toast);
        }
        if let Some(toast) = self.startup.toast {
            self.framework
                .toasts
                .mark_tracked_item_completed(toast, STARTUP_PHASE_METADATA);
        }
        tracing::info!(
            phase = "metadata_applied",
            since_scan_complete_ms =
                crate::perf_log::ms(now.duration_since(scan_complete_at).as_millis()),
            seen = self.startup.metadata.seen.len(),
            expected = self.startup.metadata.expected_len(),
            "startup_phase_complete"
        );
    }
    pub fn maybe_complete_startup_ready(&mut self, now: Instant, scan_complete_at: Instant) {
        let lint_done = self.startup.lint_count.complete_at.is_some();
        let lint_seen = self.startup.lint_phase.seen.len();
        let lint_expected = self.startup.lint_phase.expected_len();
        if self.startup.complete_at.is_some() {
            return;
        }
        let disk_ready = self.startup.disk.complete_at.is_some();
        let git_ready = self.startup.git.complete_at.is_some();
        let repo_ready = self.startup.repo.complete_at.is_some();
        let metadata_ready = self.startup.metadata.complete_at.is_some();
        if !(disk_ready && git_ready && repo_ready && metadata_ready) {
            return;
        }
        self.startup.complete_at = Some(now);
        // Finish the startup toast only when lint startup cache check
        // is also done, so "Lint cache" doesn't spin while the toast
        // exits.
        if lint_done && let Some(toast) = self.startup.toast.take() {
            self.finish_task_toast(toast);
        }
        let since_scan_ms = perf_log::ms(now.duration_since(scan_complete_at).as_millis());
        tracing::info!(
            since_scan_complete_ms = since_scan_ms,
            disk_seen = self.startup.disk.seen.len(),
            disk_expected = self.startup.disk.expected_len(),
            git_seen = self.startup.git.seen.len(),
            git_expected = self.startup.git.expected_len(),
            repo_seen = self.startup.repo.seen.len(),
            repo_expected = self.startup.repo.expected_len(),
            lint_seen = lint_seen,
            lint_expected = lint_expected,
            metadata_seen = self.startup.metadata.seen.len(),
            metadata_expected = self.startup.metadata.expected_len(),
            "startup_complete"
        );
        tracing::info!(since_scan_complete_ms = since_scan_ms, "steady_state_begin");
    }
}
