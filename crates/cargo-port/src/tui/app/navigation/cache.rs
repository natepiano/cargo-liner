use tui_pane::PERF_LOG_TARGET;

use crate::scan::ProjectStorage;
use crate::tui;
use crate::tui::app::App;
use crate::tui::app::VisibleRow;
use crate::tui::columns;
use crate::tui::panes;
use crate::tui::panes::DetailCacheKey;
use crate::tui::render;

impl App {
    pub(in crate::tui) fn ensure_visible_rows_cached(&mut self) {
        let include_non_rust = self.config.include_non_rust().includes_non_rust();
        self.project_list.recompute_visibility(include_non_rust);
    }

    /// Return the cached visible rows. Must call `ensure_visible_rows_cached()` first.
    pub(in crate::tui) fn visible_rows(&self) -> &[VisibleRow] { self.project_list.visible_rows() }

    pub(in crate::tui) fn ensure_fit_widths_cached(&mut self) {
        let root_labels = self
            .project_list
            .resolved_root_labels(self.config.include_non_rust().includes_non_rust());
        let widths = panes::compute_project_list_widths(
            &self.project_list,
            &root_labels,
            self.config.lint_enabled(),
            0,
        );
        let mut widths = widths;
        let total = self.project_list.visible_project_disk_usage();
        widths.observe(
            columns::COL_DISK,
            columns::display_width(&render::format_bytes(total)),
        );
        if let ProjectStorage::Available(bytes) = self.project_list.project_storage() {
            widths.observe(
                columns::COL_DISK,
                columns::display_width(&render::format_bytes(bytes)),
            );
        }
        self.project_list.set_fit_widths(widths);
    }

    pub(in crate::tui) fn ensure_disk_cache(&mut self) {
        let (root_sorted, child_sorted) = panes::compute_disk_cache(&self.project_list);
        self.project_list.set_disk_caches(root_sorted, child_sorted);
    }

    /// Ensure per-pane data on `PaneManager` is up to date for the selected
    /// project. Short-circuits when neither the selected row nor the app's
    /// data generation has changed since the last build — both are the only
    /// inputs to `build_selected_pane_data`, so a matching stamp means the
    /// stored detail is still correct.
    pub(in crate::tui) fn ensure_detail_cached(&mut self) {
        let desired = self.project_list.selected_row().map(|row| DetailCacheKey {
            visible_row: row,
            generation:  self.scan.generation(),
        });
        if self.panes.pane_data.detail_is_current(desired) {
            return;
        }
        let started = std::time::Instant::now();
        let pane_started = std::time::Instant::now();
        let pane = desired.zip(self.build_selected_pane_data());
        let pane_ms = tui_pane::perf_log_ms(pane_started.elapsed().as_millis());
        if let Some((key, data)) = pane {
            let ci_started = std::time::Instant::now();
            let ci = tui::panes::build_ci_data(self);
            let ci_ms = tui_pane::perf_log_ms(ci_started.elapsed().as_millis());
            let lints_started = std::time::Instant::now();
            let lints = tui::panes::build_lints_data(self);
            let lints_ms = tui_pane::perf_log_ms(lints_started.elapsed().as_millis());
            self.ci.set_content(ci);
            self.lint.set_content(lints);
            self.panes
                .set_detail_data(key, data.package, data.git, data.targets);
            tracing::trace!(
                target: PERF_LOG_TARGET,
                total_ms = tui_pane::perf_log_ms(started.elapsed().as_millis()),
                pane_ms,
                ci_ms,
                lints_ms,
                "detail_build_breakdown"
            );
        } else {
            self.ci.clear_content();
            self.lint.clear_content();
            self.panes.clear_detail_data(desired);
        }
    }
}
