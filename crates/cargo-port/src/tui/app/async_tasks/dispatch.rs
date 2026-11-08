use std::path::Path;

use tui_pane::Appearance;
use tui_pane::PERF_LOG_TARGET;

use crate::http::ServiceSignal;
use crate::project::AbsolutePath;
use crate::project::DiskMeasurement;
use crate::project::RootItem;
use crate::project::Submodule;
use crate::project::Visibility;
use crate::scan::BackgroundMsg;
use crate::tui::app::App;
use crate::tui::app::ProjectListPresentation;
use crate::tui::app::scan_state::ScanPhase;
use crate::tui::project_list::ProjectPathLookup::Found;
use crate::tui::project_list::ProjectPathLookup::StaleRoute;
use crate::tui::project_list::ProjectPathLookup::UnknownPath;
use crate::tui::project_list::ReplacementExpansion;
use crate::tui::project_list_state::ProjectList;
use crate::tui::sccache;

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

        tracing::trace!(
            target: PERF_LOG_TARGET,
            elapsed_ms = tui_pane::perf_log_ms(self.scan.state.started_at.elapsed().as_millis()),
            kind,
            run = self.scan.state.run_count,
            tree_items = projects.len(),
            disk_entries = disk_entries.len(),
            "scan_result_applied"
        );

        // Install scanned roots before restoring expansion identities.
        let selected_path = self.project_list.paths.last_selected.clone();
        self.mutate_tree().projects.replace_roots_from(
            ProjectList::new(projects),
            ReplacementExpansion::RestoreSavedTargets,
        );
        self.prune_inactive_project_state();
        let lint_registered = self.register_lint_for_root_items();
        self.refresh_lint_runs_from_disk();
        self.scan.bump_generation();

        // Drain `project_list.paths.pending_expanded` into `apply_expanded` before
        // `select_project_in_tree`, so the cursor lands on the correct expanded row. Startup seeds
        // the set from disk and rescans from the live tree.
        let pending = std::mem::take(&mut self.project_list.paths.pending_expanded);
        self.restore_expanded(&pending);
        self.refresh_project_storage();
        self.restore_selection(selected_path.as_deref());
        self.scan.state.phase = ScanPhase::Complete;
        self.sync_selected_project();

        // `register_background_services_for_tree` registers each project watcher, as
        // `register_item_background_services` does per item; `finish_watcher_registration_batch`
        // closes the batch.
        self.register_background_services_for_tree();
        self.finish_watcher_registration_batch();

        self.begin_startup_phase(lint_registered);
    }
    /// Handle a single `BackgroundMsg`. Returns `true` if the tree
    /// needs rebuilding. The match is a 1:1 mapping from variant to
    /// handler — see the style guide entry on never splitting
    /// exhaustive matches with `unreachable!()`. Multi-statement arms
    /// are extracted as small helpers so each arm stays a single
    /// call.
    pub(in crate::tui::app) fn handle_bg_msg(&mut self, msg: BackgroundMsg) -> bool {
        let disk_before = self.disk_message_inputs(&msg);
        let row_generation_before = self.project_list.row_generation;
        let row_before = self.row_message_input(&msg);
        let storage_before = self.project_list.project_storage();
        let is_disk = matches!(
            msg,
            BackgroundMsg::DiskUsage { .. } | BackgroundMsg::DiskUsageBatch { .. }
        );
        let is_storage = matches!(msg, BackgroundMsg::ProjectStorage { .. });
        if !is_disk {
            self.update_generations_for_msg(&msg);
        }
        let rebuild = self.apply_bg_msg(msg);
        let row_changed = match row_before {
            MessageRowInput::Unaffected => false,
            MessageRowInput::Rows(before) => before != self.project_row_presentation(),
        };
        let disk_changed = disk_before
            .iter()
            .any(|(path, before)| disk_row_input(&self.project_list, path) != *before);
        if disk_changed
            || (is_storage && self.project_list.project_storage() != storage_before)
            || row_changed
        {
            if self.project_list.row_generation == row_generation_before {
                self.project_list.row_generation += 1;
            }
            if disk_changed
                && disk_before.iter().any(|(path, before)| {
                    disk_row_input(&self.project_list, path) != *before
                        && self.detail_path_is_affected(path)
                })
            {
                self.scan.bump_generation();
            }
        }
        rebuild
    }

    fn row_message_input(&self, msg: &BackgroundMsg) -> MessageRowInput {
        match msg {
            BackgroundMsg::CiRuns { .. }
            | BackgroundMsg::CheckoutInfo { .. }
            | BackgroundMsg::RepoInfo { .. }
            | BackgroundMsg::NoRepository { .. }
            | BackgroundMsg::Submodules { .. }
            | BackgroundMsg::LintStatus { .. }
            | BackgroundMsg::LintStartupStatus { .. }
            | BackgroundMsg::LanguageStatsBatch { .. }
            | BackgroundMsg::CargoMetadata { .. } => {
                MessageRowInput::Rows(self.project_row_presentation())
            },
            // Structural handlers own index/row generations. Disk and storage
            // compare their measured inputs separately; all other messages
            // update tooling, detail or toast state rather than list rows.
            _ => MessageRowInput::Unaffected,
        }
    }

    fn disk_message_inputs(&self, msg: &BackgroundMsg) -> Vec<(AbsolutePath, DiskRowInput)> {
        match msg {
            BackgroundMsg::DiskUsage { path, .. } => {
                vec![(path.clone(), disk_row_input(&self.project_list, path))]
            },
            BackgroundMsg::DiskUsageBatch { entries, .. } => entries
                .iter()
                .map(|(path, _)| (path.clone(), disk_row_input(&self.project_list, path)))
                .collect(),
            _ => Vec::new(),
        }
    }

    #[allow(
        clippy::too_many_lines,
        reason = "trivial 1:1 exhaustive variant mapping"
    )]
    fn apply_bg_msg(&mut self, msg: BackgroundMsg) -> bool {
        match msg {
            BackgroundMsg::DiskUsage { path, bytes } => {
                self.handle_disk_usage_msg(path.as_path(), bytes);
            },
            BackgroundMsg::DiskUsageBatch { root_path, entries } => {
                self.handle_disk_usage_batch_msg(&root_path, entries);
            },
            BackgroundMsg::ProjectStorage { paths, storage } => {
                self.set_project_storage(&paths, storage);
            },
            BackgroundMsg::CiRuns {
                path,
                runs,
                github_total,
            } => self.insert_ci_runs(path.as_path(), runs, github_total),
            BackgroundMsg::RepoFetchQueued { repo } => self.handle_repo_fetch_queued(repo),
            BackgroundMsg::RepoFetchComplete { repo } => self.handle_repo_fetch_complete(repo),
            BackgroundMsg::PullRequests { repo, data } => {
                self.handle_pull_requests(&repo, &data);
            },
            BackgroundMsg::PullRequestCheckPollStopped { repo, number } => {
                self.handle_pull_request_check_poll_stopped(&repo, number);
            },
            BackgroundMsg::PullRequestDisappeared {
                repo,
                pull_request,
                reason,
            } => {
                self.handle_pull_request_disappeared(&repo, &pull_request, &reason);
            },
            BackgroundMsg::CratesIoFetchQueued { name } => {
                self.handle_crates_io_fetch_queued(name);
            },
            BackgroundMsg::CratesIoFetchComplete { name } => {
                self.handle_crates_io_fetch_complete(&name);
            },
            BackgroundMsg::CheckoutInfo { path, info } => {
                self.handle_checkout_info(path.as_path(), info);
            },
            BackgroundMsg::RepoInfo { path, info } => {
                self.handle_repo_info(path.as_path(), info);
            },
            BackgroundMsg::NoRepository { path } => {
                self.handle_no_repository(path.as_path());
            },
            BackgroundMsg::GitFirstCommit { path, first_commit } => {
                self.handle_git_first_commit(path.as_path(), first_commit);
            },
            BackgroundMsg::ProjectDetailsDeclared { path } => {
                self.mark_startup_project_details_declared(path);
            },
            BackgroundMsg::Submodules { path, submodules } => {
                self.handle_submodules_msg(path.as_path(), submodules);
            },
            BackgroundMsg::CratesIoVersion { path, release } => {
                self.project_list
                    .handle_crates_io_version_msg(path.as_path(), release);
            },
            BackgroundMsg::CratesIoNewRelease {
                display_name,
                previous_version,
                version,
            } => {
                self.handle_crates_io_new_release(&display_name, &previous_version, &version);
            },
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
            } => self.handle_scan_result(projects, &disk_entries),
            BackgroundMsg::ProjectDiscovered { item } => {
                return self.handle_project_discovered(item);
            },
            BackgroundMsg::ProjectRefreshed { item } => return self.handle_project_refreshed(item),
            BackgroundMsg::LintCachePruned {
                runs_evicted,
                bytes_reclaimed,
            } => self.handle_lint_cache_pruned(runs_evicted, bytes_reclaimed),
            BackgroundMsg::LintCacheUsage { usage } => self.lint.set_cache_usage(usage),
            BackgroundMsg::LintHistoryLoaded { entries } => {
                self.apply_lint_history_loaded(entries);
            },
            BackgroundMsg::LintStatus {
                path,
                status,
                origin,
            } => {
                self.handle_lint_status_msg(path.as_path(), status, origin);
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
            BackgroundMsg::ServiceUnreachableConfirmed { service } => {
                self.confirm_service_unreachable(service);
            },
            BackgroundMsg::ServiceRateLimited { service } => {
                self.apply_service_signal(ServiceSignal::RateLimited(service));
            },
            BackgroundMsg::LanguageStatsProgressPlan { units } => {
                self.mark_startup_languages_expected(units);
            },
            BackgroundMsg::LanguageStatsBatch { entries } => {
                self.mark_startup_languages_seen(&entries);
                self.project_list.handle_language_stats_batch(entries);
            },
            BackgroundMsg::TestCountsBatch { entries } => {
                self.mark_startup_tests_seen(&entries);
                self.project_list.handle_test_counts_batch(entries);
                self.scan.bump_generation();
            },
            BackgroundMsg::SccacheStats { request_id, result } => {
                self.overlays.sccache_pane.apply_result(request_id, result);
            },
            BackgroundMsg::SccacheSummary(summary) => {
                sccache::apply_summary(self, summary);
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
            BackgroundMsg::AppearanceChanged(appearance) => self.apply_os_appearance(appearance),
        }
        false
    }
    /// Stash a project's submodule list onto its `ProjectInfo` when
    /// the path resolves to a known leaf. Extracted from the dispatch
    /// match so each arm stays a single call.
    fn handle_submodules_msg(&mut self, path: &Path, submodules: Vec<Submodule>) {
        let submodule_list_changed = self.project_list.replace_submodules(path, submodules);
        if submodule_list_changed {
            self.project_list.rebuild_path_index();
            self.rebuild_visible_rows_now();
        }
    }
    /// Stash the OS appearance and re-resolve the active theme against
    /// the current `[appearance]` config.
    fn apply_os_appearance(&mut self, appearance: Appearance) {
        self.themes.set_os_appearance(Some(appearance));
        self.resolve_and_apply_active_theme();
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::project::Package;
    use crate::project::PinnedCommit;
    use crate::project::ProjectInfo;
    use crate::project::RustProject;
    use crate::tui::project_list::ExpandKey;

    fn submodule(name: &str, path: &str) -> Submodule {
        Submodule {
            name:          name.to_string(),
            path:          AbsolutePath::from(path),
            relative_path: name.to_string(),
            url:           None,
            branch:        None,
            commit:        PinnedCommit::Absent,
            project_info:  ProjectInfo::default(),
            git_repo:      None,
        }
    }

    #[test]
    fn same_length_submodule_replacement_advances_visible_ownership() {
        let project_path = AbsolutePath::from("/tmp/project");
        let project = RootItem::Rust(RustProject::Package(Package {
            path: project_path.clone(),
            ..Package::default()
        }));
        let mut app = crate::tui::test_support::make_app(&[project]);
        app.project_list.expanded.expand(ExpandKey::Node(0), 0);

        let _ = app.handle_bg_msg(BackgroundMsg::Submodules {
            path:       project_path.clone(),
            submodules: vec![submodule("first", "/tmp/project/first")],
        });
        app.project_list.set_cursor(1);
        let visible_rows = app.project_list.visible_rows().to_vec();
        assert_eq!(
            app.project_list.selected_project_path(),
            Some(Path::new("/tmp/project/first"))
        );

        let _ = app.handle_bg_msg(BackgroundMsg::Submodules {
            path:       project_path,
            submodules: vec![submodule("second", "/tmp/project/second")],
        });

        assert_eq!(app.project_list.visible_rows(), visible_rows);
        assert_eq!(
            app.project_list.selected_project_path(),
            Some(Path::new("/tmp/project/second"))
        );
    }
}

/// Disk replies can change deletion state without changing bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DiskRowInput {
    Missing,
    Present {
        disk:       DiskMeasurement,
        visibility: Visibility,
    },
}

fn disk_row_input(projects: &ProjectList, path: &Path) -> DiskRowInput {
    match projects.at_path(path) {
        Found(info) => DiskRowInput::Present {
            disk:       info.disk,
            visibility: info.visibility,
        },
        UnknownPath | StaleRoute => DiskRowInput::Missing,
    }
}

/// Only list-row inputs can change the list's width cache.
enum MessageRowInput {
    Unaffected,
    Rows(ProjectListPresentation),
}
