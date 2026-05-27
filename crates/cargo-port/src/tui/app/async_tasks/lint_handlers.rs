use std::path::Path;
use std::time::Instant;

use crate::lint;
use crate::lint::LintStatus;
use crate::project::AbsolutePath;
use crate::tui::app::App;
use crate::tui::app::phase_state::PhaseCompletion;
use crate::tui::constants::STARTUP_PHASE_LINT;

impl App {
    pub(super) fn handle_lint_startup_status_msg(
        &mut self,
        path: &AbsolutePath,
        status: LintStatus,
    ) {
        // Apply the cached status to the project (same as a live status).
        if let Some(lr) = self.project_list.lint_at_path_mut(path) {
            lr.set_status(status);
        }
        self.startup.lint_count.seen += 1;
        self.maybe_complete_startup_lint_cache();
    }
    pub(super) fn maybe_complete_startup_lint_cache(&mut self) {
        let now = Instant::now();
        if !self.startup.lint_count.complete_once(now) {
            return;
        }
        // All startup lint statuses collected — compute cache size once.
        self.refresh_lint_cache_usage_from_disk();
        if let Some(toast) = self.startup.toast {
            self.framework
                .toasts
                .mark_tracked_item_completed(toast, STARTUP_PHASE_LINT);
        }
        // Clear the embedding's slot if core startup already finished.
        // The overall startup toast auto-finishes when its last item
        // (the Lint item, just marked completed above) is marked
        // completed — matching the prior explicit-finish gate.
        if self.startup.complete_at.is_some() {
            let _ = self.startup.toast.take();
        }
        if let Some(scan_complete_at) = self.startup.scan_complete_at {
            tracing::info!(
                phase = "lint_startup_applied",
                since_scan_complete_ms =
                    tui_pane::perf_log_ms(now.duration_since(scan_complete_at).as_millis()),
                seen = self.startup.lint_count.seen,
                expected = self.startup.lint_count.expected.unwrap_or(0),
                "startup_phase_complete"
            );
        }
        self.maybe_log_startup_phase_completions();
    }
    pub(super) fn handle_lint_status_msg(&mut self, path: &Path, status: LintStatus) {
        let Some(owner_path) = self.project_list.lint_owner_path(path) else {
            tracing::warn!(
                path = %path.display(),
                status = ?status.kind(),
                "lint_status_dropped_no_owner"
            );
            self.sync_running_lint_toast();
            return;
        };
        let owner_abs = owner_path;
        let status_kind = status.kind();
        let status_started = matches!(status, LintStatus::Running(_));
        let status_is_terminal = matches!(
            status,
            LintStatus::Passed(_) | LintStatus::Failed(_) | LintStatus::Stale | LintStatus::NoLog
        );
        let eligible = lint::project_is_eligible(
            &self.config.current().lint,
            &owner_abs.as_path().to_string_lossy(),
            owner_abs.as_path(),
            true,
        );
        let applied_to_model = self
            .project_list
            .lint_at_path_mut(owner_abs.as_path())
            .is_some_and(|lr| {
                lr.set_status(status);
                true
            });
        if status_is_terminal {
            self.reload_lint_history(owner_abs.as_path());
        }
        if applied_to_model {
            self.scan.bump_generation();
        } else {
            tracing::warn!(
                path = %path.display(),
                owner = %owner_abs,
                status = ?status_kind,
                eligible,
                "lint_status_owner_missing_model_slot"
            );
        }
        self.sync_running_lint_toast();
        tracing::info!(
            path = %path.display(),
            owner = %owner_abs,
            status = ?status_kind,
            eligible,
            applied_to_model,
            running_lints = self.lint.running_toast_path_count(),
            generation = self.scan.generation(),
            "lint_status_applied"
        );
        if !self.scan.is_complete() {
            return;
        }
        if status_started {
            let expected = self.startup.lint_phase.ensure_expected();
            if expected.insert(owner_abs.clone()) {
                self.startup.lint_phase.complete_at = None;
            }
        }
        if status_is_terminal
            && self
                .startup
                .lint_phase
                .expected
                .as_ref()
                .is_some_and(|expected| expected.contains(owner_abs.as_path()))
        {
            self.startup.lint_phase.seen.insert(owner_abs);
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
