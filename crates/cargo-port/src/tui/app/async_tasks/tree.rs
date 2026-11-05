use std::path::Path;
use std::time::Instant;

use tui_pane::PERF_LOG_TARGET;

use crate::lint::CacheUsage;
#[cfg(test)]
use crate::project::AbsolutePath;
use crate::scan;
use crate::tui::app::App;
use crate::tui::app::scan_state::ScanPhase;
use crate::tui::panes::PaneId;
use crate::tui::project_list::ExpandTarget;
#[cfg(test)]
use crate::tui::project_list::ReplacementExpansion;
#[cfg(test)]
use crate::tui::project_list_state::ProjectList;
use crate::tui::startup_services::StartupEffect;
use crate::tui::startup_services::StreamingScanStartup;

impl App {
    #[cfg(test)]
    pub fn apply_tree_build(&mut self, projects: ProjectList) {
        let selected_path = self
            .project_list
            .selected_project_path()
            .map(AbsolutePath::from)
            .or_else(|| self.project_list.paths.last_selected.clone());
        let expanded = self.project_list.export_expanded();
        let inline_dirs = self.config.current().tui.inline_dirs.clone();
        self.mutate_tree().projects.replace_roots_from(
            projects,
            &inline_dirs,
            ReplacementExpansion::RestoreSavedTargets,
        );
        self.restore_expanded(&expanded);
        self.prune_inactive_project_state();
        self.register_lint_for_root_items();
        self.refresh_lint_runs_from_disk();
        self.scan.bump_generation();

        self.restore_selection(selected_path.as_deref());
        self.sync_selected_project();
    }

    pub(super) fn restore_expanded(&mut self, expanded: &[ExpandTarget]) {
        self.project_list.expanded.reset();
        self.project_list.apply_expanded(expanded);
        self.rebuild_visible_rows_now();
    }

    pub(super) fn restore_selection(&mut self, path: Option<&Path>) {
        self.project_list.set_cursor(0);
        if let Some(path) = path {
            self.project_list
                .select_project_in_tree(path, self.config.include_non_rust().includes_non_rust());
        }
    }

    pub(super) fn rebuild_visible_rows_now(&mut self) {
        let include_non_rust = self.config.include_non_rust().includes_non_rust();
        self.project_list.recompute_visibility(include_non_rust);
    }

    pub(in crate::tui::app) fn prepare_rescan(&mut self) {
        // Keep selection and expansion identities across root reordering.
        let preserved_expanded = self.project_list.export_expanded();
        let selected_path = self.project_list.paths.last_selected.clone();
        // `scan.manifest_metadata` survives row replacement. Rediscovered manifests
        // retain their last diagnostic until an accepted metadata reply replaces it.
        self.mutate_tree().projects.retain_for_rescan();
        self.ci.fetch_tracker.clear();
        self.ci.clear_display_modes();
        self.clear_all_lint_state();
        self.lint.set_cache_usage(CacheUsage::default());
        self.net.clear_for_tree_change();
        // `enter_startup_owned_network_stage` finishes live standalone GitHub and crates.io task
        // toasts and restores startup ownership before `startup.reset()` reopens the consolidated
        // panel, so fetches during rescan cannot create standalone network toasts.
        self.enter_startup_owned_network_stage();
        self.scan.discovery_shimmers_mut().clear();
        self.scan.state.phase = ScanPhase::Running;
        self.scan.state.started_at = Instant::now();
        self.scan.state.run_count += 1;
        self.startup.reset();
        tracing::trace!(
            target: PERF_LOG_TARGET,
            kind = "rescan",
            run = self.scan.state.run_count,
            "scan_start"
        );
        self.scan.set_priority_fetch_path(None);
        self.set_focus_to_pane(PaneId::ProjectList);
        let _ = self.overlays.take_finder_return();
        self.overlays.close_settings();
        self.overlays.close_finder();
        self.reset_project_panes();
        self.project_list.clear_selected_project();
        self.inflight.clear_pending_ci_fetch();
        self.restore_expanded(&preserved_expanded);
        self.project_list.paths.pending_expanded = preserved_expanded;
        self.restore_selection(selected_path.as_deref());
        self.panes.project_list.viewport.set_scroll_offset(0);
        self.scan.bump_generation();
    }

    pub(in crate::tui) fn rescan(&mut self) {
        self.prepare_rescan();
        let scan_dirs = scan::resolve_include_dirs(&self.config.current().tui.include_dirs);
        let scan_start = self
            .startup_services
            .spawn_streaming_scan(StreamingScanStartup {
                scan_dirs,
                inline_dirs: &self.config.current().tui.inline_dirs,
                exclude_dirs: self.config.exclude_dirs(),
                non_rust: self.config.include_non_rust(),
                client: self.net.http_client(),
                metadata_store: self.scan.metadata_store_handle(),
            });
        let scan_effect = scan_start.effect;
        self.background
            .swap_background_channel(scan_start.sender, scan_start.receiver);
        self.respawn_watcher();
        let current_config = self.config.current().clone();
        self.refresh_lint_runtime_from_config(&current_config);
        if scan_effect == StartupEffect::Suppressed {
            self.handle_scan_result(Vec::new(), &[]);
        }
    }
}
