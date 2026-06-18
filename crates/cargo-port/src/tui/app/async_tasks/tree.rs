use std::time::Instant;

use tui_pane::PERF_LOG_TARGET;

use crate::lint::CacheUsage;
#[cfg(test)]
use crate::project::AbsolutePath;
use crate::scan;
use crate::tui::app::App;
use crate::tui::app::scan_state::ScanPhase;
use crate::tui::panes::PaneId;
#[cfg(test)]
use crate::tui::project_list::ProjectList;
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
        let should_focus_project_list = false;
        self.mutate_tree().replace_all(projects);
        self.prune_inactive_project_state();
        self.register_lint_for_root_items();
        self.refresh_lint_runs_from_disk();
        self.scan.bump_generation();

        // Try to restore selection
        if let Some(path) = selected_path {
            let include_non_rust = self.config.include_non_rust().includes_non_rust();
            self.project_list
                .select_project_in_tree(path.as_path(), include_non_rust);
        } else if !self.project_list.is_empty() {
            self.project_list.set_cursor(0);
        }
        if should_focus_project_list {
            self.set_focus_to_pane(PaneId::ProjectList);
        }
        self.sync_selected_project();
    }

    pub(super) fn rebuild_visible_rows_now(&mut self) {
        let include_non_rust = self.config.include_non_rust().includes_non_rust();
        self.project_list.recompute_visibility(include_non_rust);
    }

    pub fn rescan(&mut self) {
        // Preserve the open/closed layout across the rebuild: snapshot the live
        // expansions as restart-stable targets before the tree is cleared, then
        // hand them back through `pending_expanded` so `handle_scan_result`
        // re-applies them once the new tree is built.
        let preserved_expanded = self.project_list.export_expanded();
        self.project_list.clear();
        // disk_usage lives on project items — cleared with projects above
        self.ci.fetch_tracker.clear();
        self.ci.clear_display_modes();
        self.clear_all_lint_state();
        self.lint.set_cache_usage(CacheUsage::default());
        self.net.clear_for_tree_change();
        // The rescan re-opens the consolidated startup panel, which takes back
        // ownership of the GitHub / crates.io rows: finish any live standalone
        // network toasts and return the stage to startup-owned so a fetch
        // dispatched during this scan cannot leak a standalone toast.
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
        self.project_list.paths.selected_project = None;
        self.inflight.clear_pending_ci_fetch();
        self.project_list.expanded.clear();
        self.project_list.paths.pending_expanded = preserved_expanded;
        self.project_list.set_cursor(0);
        self.panes.project_list.viewport.set_scroll_offset(0);
        self.scan.bump_generation();
        let scan_dirs = scan::resolve_include_dirs(&self.config.current().tui.include_dirs);
        let scan_start = self
            .startup_services
            .spawn_streaming_scan(StreamingScanStartup {
                scan_dirs,
                inline_dirs: &self.config.current().tui.inline_dirs,
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
