use crate::perf_log;
use crate::tui;
use crate::tui::app::App;
use crate::tui::app::VisibleRow;
use crate::tui::panes;
use crate::tui::panes::DetailCacheKey;

impl App {
    pub fn ensure_visible_rows_cached(&mut self) {
        let include_non_rust = self.config.include_non_rust().includes_non_rust();
        self.project_list.recompute_visibility(include_non_rust);
    }

    /// Return the cached visible rows. Must call `ensure_visible_rows_cached()` first.
    pub fn visible_rows(&self) -> &[VisibleRow] { self.project_list.visible_rows() }

    pub fn ensure_fit_widths_cached(&mut self) {
        let root_labels = self
            .projects()
            .resolved_root_labels(self.config.include_non_rust().includes_non_rust());
        let widths = panes::compute_project_list_widths(
            self.projects(),
            &root_labels,
            self.config.lint_enabled(),
            0,
        );
        self.project_list.set_fit_widths(widths);
    }

    pub fn ensure_disk_cache(&mut self) {
        let (root_sorted, child_sorted) = panes::compute_disk_cache(self.projects());
        self.project_list.set_disk_caches(root_sorted, child_sorted);
    }

    /// Ensure per-pane data on `PaneManager` is up to date for the selected
    /// project. Short-circuits when neither the selected row nor the app's
    /// data generation has changed since the last build — both are the only
    /// inputs to `build_selected_pane_data`, so a matching stamp means the
    /// stored detail is still correct.
    pub fn ensure_detail_cached(&mut self) {
        let desired = self.selected_row().map(|row| DetailCacheKey {
            row,
            generation: self.scan.generation(),
        });
        if self.panes.pane_data().detail_is_current(desired) {
            return;
        }
        let started = std::time::Instant::now();
        let pane_started = std::time::Instant::now();
        let pane = desired.and_then(|key| self.build_selected_pane_data().map(|data| (key, data)));
        let pane_ms = perf_log::ms(pane_started.elapsed().as_millis());
        if let Some((key, data)) = pane {
            let ci_started = std::time::Instant::now();
            let ci = tui::panes::build_ci_data(self);
            let ci_ms = perf_log::ms(ci_started.elapsed().as_millis());
            let lints_started = std::time::Instant::now();
            let lints = tui::panes::build_lints_data(self);
            let lints_ms = perf_log::ms(lints_started.elapsed().as_millis());
            self.ci.set_content(ci);
            self.lint.set_content(lints);
            self.panes
                .set_detail_data(key, data.package, data.git, data.targets);
            tracing::info!(
                total_ms = perf_log::ms(started.elapsed().as_millis()),
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
