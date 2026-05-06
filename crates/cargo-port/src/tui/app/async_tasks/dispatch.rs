use crate::http::ServiceSignal;
use crate::project::AbsolutePath;
use crate::project::RootItem;
use crate::scan::BackgroundMsg;
use crate::tui::app::App;
use crate::tui::app::types::ScanPhase;
use crate::tui::project_list::ProjectList;

impl App {
    /// Bump `data_generation` only when a background message can change
    /// what the currently-selected detail set would render.
    ///
    /// Two-stage filter:
    /// 1. **Type-level (compile-time enforced):** `BackgroundMsg::detail_relevance` is exhaustive
    ///    on every variant. Variants whose data flows into the detail set return `Some(path)`;
    ///    variants for service signals, fetch lifecycle, or batched paths return `None`. Adding a
    ///    new variant without classifying it is a build error.
    /// 2. **Runtime (data-dependent):** even a detail-relevant message may target a project that
    ///    isn't selected. `detail_path_is_affected` compares the message's path against the current
    ///    selection.
    ///
    /// Removing this filter (or widening it via `path()`) reintroduces
    /// the regression where every watcher tick invalidates the
    /// detail-pane cache and reduces it to a no-op during scroll.
    pub(super) fn update_generations_for_msg(&mut self, msg: &BackgroundMsg) {
        if let Some(path) = msg.detail_relevance()
            && self.detail_path_is_affected(path)
        {
            self.scan.bump_generation();
        }
    }
    pub(super) fn handle_scan_result(
        &mut self,
        projects: Vec<RootItem>,
        disk_entries: &[(String, AbsolutePath)],
    ) {
        let kind = if self.scan.state.run_count == 1 {
            "initial"
        } else {
            "rescan"
        };

        tracing::info!(
            elapsed_ms = crate::perf_log::ms(self.scan.state.started_at.elapsed().as_millis()),
            kind,
            run = self.scan.state.run_count,
            tree_items = projects.len(),
            disk_entries = disk_entries.len(),
            "scan_result_applied"
        );

        // Apply tree (same as apply_tree_build but inlined to avoid redundant
        // rebuild scheduling).
        let selected_path = self
            .project_list
            .selected_project_path()
            .map(AbsolutePath::from)
            .or_else(|| self.project_list.paths.last_selected.clone());
        self.mutate_tree().replace_all(ProjectList::new(projects));
        self.prune_inactive_project_state();
        let lint_registered = self.register_lint_for_root_items();
        self.startup.lint_count.expected = Some(lint_registered);
        self.startup.lint_count.seen = 0;
        self.startup.lint_count.complete_at = None;
        // When nothing will ever increment `seen` (lint runtime disabled or no
        // eligible projects), nothing else will drive completion — finish the
        // phase here so the startup toast can finish.
        if lint_registered == 0 {
            self.maybe_complete_startup_lint_cache();
        }
        self.refresh_lint_runs_from_disk();
        self.scan.bump_generation();

        // Restore selection.
        if let Some(path) = selected_path {
            self.project_list.select_project_in_tree(
                path.as_path(),
                self.config.include_non_rust().includes_non_rust(),
            );
        } else if !self.project_list.is_empty() {
            self.project_list.set_cursor(0);
        }
        self.sync_selected_project();

        // Register watcher for each item (same as register_item_background_services).
        self.register_background_services_for_tree();
        self.finish_watcher_registration_batch();

        // Mark scan complete and initialize startup tracking.
        self.scan.state.phase = ScanPhase::Complete;
        self.initialize_startup_phase_tracker();
        self.schedule_startup_project_details();
        self.schedule_git_first_commit_refreshes();
    }
    /// Handle a single `BackgroundMsg`. Returns `true` if the tree needs rebuilding.
    pub fn handle_bg_msg(&mut self, msg: BackgroundMsg) -> bool {
        self.update_generations_for_msg(&msg);
        match msg {
            BackgroundMsg::DiskUsage { path, bytes } => {
                self.handle_disk_usage_msg(path.as_path(), bytes);
            },
            BackgroundMsg::DiskUsageBatch { root_path, entries } => {
                self.handle_disk_usage_batch_msg(&root_path, entries);
            },
            BackgroundMsg::CiRuns {
                path,
                runs,
                github_total,
            } => self.insert_ci_runs(path.as_path(), runs, github_total),
            BackgroundMsg::RepoFetchQueued { repo } => self.handle_repo_fetch_queued(repo),
            BackgroundMsg::RepoFetchComplete { repo } => self.handle_repo_fetch_complete(repo),
            BackgroundMsg::CheckoutInfo { path, info } => {
                self.handle_checkout_info(path.as_path(), info);
            },
            BackgroundMsg::RepoInfo { path, info } => {
                self.handle_repo_info(path.as_path(), info);
            },
            BackgroundMsg::GitFirstCommit { path, first_commit } => {
                self.handle_git_first_commit(path.as_path(), first_commit.as_deref());
            },
            BackgroundMsg::Submodules { path, submodules } => {
                if let Some(info) = self.project_list.at_path_mut(path.as_path()) {
                    info.submodules = submodules;
                }
            },
            BackgroundMsg::CratesIoVersion {
                path,
                version,
                downloads,
            } => self
                .project_list
                .handle_crates_io_version_msg(path.as_path(), version, downloads),
            BackgroundMsg::RepoMeta {
                path,
                stars,
                description,
            } => self
                .project_list
                .handle_repo_meta(path.as_path(), stars, description),
            BackgroundMsg::ScanResult {
                projects,
                disk_entries,
            } => {
                self.handle_scan_result(projects, &disk_entries);
            },
            BackgroundMsg::ProjectDiscovered { item } => {
                if self.handle_project_discovered(item) {
                    return true;
                }
            },
            BackgroundMsg::ProjectRefreshed { item } => {
                if self.handle_project_refreshed(item) {
                    return true;
                }
            },
            BackgroundMsg::LintCachePruned {
                runs_evicted,
                bytes_reclaimed,
            } => self.handle_lint_cache_pruned(runs_evicted, bytes_reclaimed),
            BackgroundMsg::LintStatus { path, status } => {
                self.handle_lint_status_msg(path.as_path(), status);
            },
            BackgroundMsg::LintStartupStatus { path, status } => {
                self.handle_lint_startup_status_msg(&path, status);
            },
            BackgroundMsg::ServiceReachable { service } => {
                self.apply_service_signal(ServiceSignal::Reachable(service));
            },
            BackgroundMsg::ServiceRecovered { service } => self.mark_service_recovered(service),
            BackgroundMsg::ServiceUnreachable { service } => {
                self.apply_service_signal(ServiceSignal::Unreachable(service));
            },
            BackgroundMsg::ServiceRateLimited { service } => {
                self.apply_service_signal(ServiceSignal::RateLimited(service));
            },
            BackgroundMsg::LanguageStatsBatch { entries } => {
                self.project_list.handle_language_stats_batch(entries);
            },
            BackgroundMsg::CargoMetadata {
                workspace_root,
                generation,
                fingerprint,
                result,
            } => self.handle_cargo_metadata_msg(workspace_root, generation, &fingerprint, result),
            BackgroundMsg::OutOfTreeTargetSize {
                workspace_root,
                target_dir,
                bytes,
            } => self
                .scan
                .handle_out_of_tree_target_size(&workspace_root, &target_dir, bytes),
        }
        false
    }
}
