use std::path::Path;
use std::time::Instant;

use crate::lint;
use crate::lint::CachedLintStatus;
use crate::lint::LintRunOrigin;
use crate::lint::LintStatus;
use crate::lint::LintStatusKind;
use crate::project::AbsolutePath;
use crate::tui::app::App;
use crate::tui::app::phase_state::PhaseCompletion;

impl App {
    pub(super) fn handle_lint_startup_status_msg(
        &mut self,
        path: &AbsolutePath,
        status: CachedLintStatus,
    ) {
        if let Some(lr) = self.project_list.lint_at_path_mut(path)
            && !matches!(lr.status(), LintStatus::Running(_))
        {
            lr.set_status(status.into_lint_status());
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
        // `lint_count` is internal cardinality (the cached-status load), not
        // a panel row; the lint *row* tracks `lint_phase`. The panel is
        // closed by `maybe_complete_startup_ready`, not here.
        self.refresh_lint_cache_usage_from_disk();
        if let Some(scan_complete_at) = self.startup.scan_complete_at {
            tracing::trace!(
                target: tui_pane::PERF_LOG_TARGET,
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
    pub(super) fn handle_lint_status_msg(
        &mut self,
        path: &Path,
        status: LintStatus,
        origin: LintRunOrigin,
    ) {
        let status_kind = status.kind();
        let Some(owner_path) = self.project_list.lint_owner_path(path) else {
            tracing::warn!(
                path = %path.display(),
                status = ?status_kind,
                origin = ?origin,
                "lint_status_dropped_no_owner"
            );
            if !matches!(status_kind, LintStatusKind::Running) {
                self.lint.clear_running_path(path);
            }
            self.sync_running_lint_toast();
            return;
        };
        let owner_abs = owner_path;
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
            self.lint
                .apply_lint_status(owner_abs.clone(), status_kind, origin);
        } else {
            if !matches!(status_kind, LintStatusKind::Running) {
                self.lint.clear_running_path(owner_abs.as_path());
            }
            tracing::warn!(
                path = %path.display(),
                owner = %owner_abs,
                status = ?status_kind,
                origin = ?origin,
                eligible,
                "lint_status_owner_missing_model_slot"
            );
        }
        self.sync_running_lint_toast();
        tracing::trace!(
            target: tui_pane::PERF_LOG_TARGET,
            path = %path.display(),
            owner = %owner_abs,
            status = ?status_kind,
            origin = ?origin,
            eligible,
            applied_to_model,
            running_lints = self.lint.running_toast_path_count(),
            generation = self.scan.generation(),
            "lint_status_applied"
        );
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
