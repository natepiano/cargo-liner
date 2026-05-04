use std::path::Path;
use std::time::Instant;

use crate::lint;
use crate::lint::LintStatus;
use crate::project::AbsolutePath;
use crate::tui::app::App;
use crate::tui::app::phase_state::PhaseCompletion;
use crate::tui::constants::STARTUP_PHASE_LINT;

impl App {
    pub(super) fn handle_crates_io_version_msg(
        &mut self,
        path: &Path,
        version: String,
        downloads: u64,
    ) {
        if let Some(rust_info) = self.projects_mut().rust_info_at_path_mut(path) {
            rust_info.set_crates_io(version, downloads);
        } else if let Some(vendored) = self.projects_mut().vendored_at_path_mut(path) {
            vendored.set_crates_io(version, downloads);
        }
    }
    pub(super) fn handle_lint_startup_status_msg(
        &mut self,
        path: &AbsolutePath,
        status: LintStatus,
    ) {
        // Apply the cached status to the project (same as a live status).
        if let Some(lr) = self.scan.projects_mut().lint_at_path_mut(path) {
            lr.set_status(status);
        }
        self.lint.startup_phase.seen += 1;
        self.maybe_complete_startup_lint_cache();
    }
    pub(super) fn maybe_complete_startup_lint_cache(&mut self) {
        let now = Instant::now();
        if !self.lint.startup_phase.complete_once(now) {
            return;
        }
        // All startup lint statuses collected — compute cache size once.
        self.refresh_lint_cache_usage_from_disk();
        if let Some(toast) = self.scan.scan_state_mut().startup_phases.startup_toast {
            self.mark_tracked_item_completed(toast, STARTUP_PHASE_LINT);
        }
        // If core startup already finished, now finish the startup toast.
        if self
            .scan
            .scan_state()
            .startup_phases
            .startup_complete_at
            .is_some()
            && let Some(toast) = self
                .scan
                .scan_state_mut()
                .startup_phases
                .startup_toast
                .take()
        {
            self.finish_task_toast(toast);
        }
        if let Some(scan_complete_at) = self.scan.scan_state().startup_phases.scan_complete_at {
            tracing::info!(
                phase = "lint_startup_applied",
                since_scan_complete_ms =
                    crate::perf_log::ms(now.duration_since(scan_complete_at).as_millis()),
                seen = self.lint.startup_phase.seen,
                expected = self.lint.startup_phase.expected.unwrap_or(0),
                "startup_phase_complete"
            );
        }
        self.maybe_log_startup_phase_completions();
    }
    pub(super) fn handle_lint_status_msg(&mut self, path: &Path, status: LintStatus) {
        let abs = AbsolutePath::from(path);
        let status_started = matches!(status, LintStatus::Running(_));
        let status_is_terminal = matches!(
            status,
            LintStatus::Passed(_) | LintStatus::Failed(_) | LintStatus::Stale | LintStatus::NoLog
        );
        if !self.projects().is_rust_at_path(path) {
            if let Some(lr) = self.scan.projects_mut().lint_at_path_mut(path) {
                lr.clear_runs();
            }
            return;
        }
        let mut is_rust = false;
        self.projects().for_each_leaf_path(|p, rust| {
            if p == path {
                is_rust = rust;
            }
        });
        let eligible = lint::project_is_eligible(
            &self.config.current().lint,
            &path.to_string_lossy(),
            path,
            is_rust,
        );
        if eligible {
            if let Some(lr) = self.scan.projects_mut().lint_at_path_mut(path) {
                lr.set_status(status);
            }
            if status_is_terminal {
                self.reload_lint_history(path);
            }
        } else {
            if let Some(lr) = self.scan.projects_mut().lint_at_path_mut(path) {
                lr.clear_runs();
            }
            self.lint.running_mut().remove(path);
        }
        if status_started {
            self.lint.running_mut().insert(abs, Instant::now());
        }
        if status_is_terminal {
            self.lint.running_mut().remove(path);
        }
        self.sync_running_lint_toast();
        if !self.scan.is_complete() {
            return;
        }
        if status_started {
            let abs_path = AbsolutePath::from(path);
            let expected = self.lint.phase.ensure_expected();
            if expected.insert(abs_path) {
                self.lint.phase.complete_at = None;
            }
        }
        if status_is_terminal {
            let abs_path = AbsolutePath::from(path);
            if self
                .lint
                .phase
                .expected
                .as_ref()
                .is_some_and(|expected| expected.contains(path))
            {
                self.lint.phase.seen.insert(abs_path);
            }
        }
        self.maybe_log_startup_phase_completions();
    }
    pub(super) fn handle_lint_cache_pruned(&mut self, runs_evicted: usize, bytes_reclaimed: u64) {
        let noun = if runs_evicted == 1 { "run" } else { "runs" };
        self.show_timed_toast(
            "Lint cache",
            format!(
                "Evicted {runs_evicted} {noun}, reclaimed {}",
                crate::tui::render::format_bytes(bytes_reclaimed),
            ),
        );
        self.refresh_lint_cache_usage_from_disk();
    }
}
