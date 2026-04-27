use std::collections::HashMap;
use std::collections::HashSet;
use std::path::Path;
use std::thread;
use std::time::Duration;
use std::time::Instant;

use super::App;
use super::ExpandKey::Group;
use super::ExpandKey::Node;
use super::ExpandKey::Worktree;
use super::ExpandKey::WorktreeGroup;
use super::phase_state::PhaseCompletion;
use super::service_state::ServiceAvailability;
use super::snapshots;
use super::target_index::MemberKind;
use super::target_index::TargetDirMember;
use super::types::PollBackgroundStats;
use super::types::ScanPhase;
use super::types::StartupPhaseTracker;
use crate::ci;
use crate::ci::OwnerRepo;
use crate::config;
use crate::config::CargoPortConfig;
use crate::constants::SERVICE_RETRY_SECS;
use crate::http::ServiceKind;
use crate::http::ServiceSignal;
use crate::keymap;
use crate::keymap::KeymapError;
use crate::keymap::KeymapErrorReason::Parse;
use crate::lint;
use crate::lint::LintStatus;
use crate::lint::RegisterProjectRequest;
use crate::project;
use crate::project::AbsolutePath;
use crate::project::CheckoutInfo;
use crate::project::GitHubInfo;
use crate::project::GitRepoPresence;
use crate::project::LanguageStats;
use crate::project::LocalGitState;
use crate::project::ManifestFingerprint;
use crate::project::ProjectFields;
use crate::project::RepoInfo;
use crate::project::RootItem;
use crate::project::RustProject;
use crate::project::Visibility::Deleted;
use crate::project::Visibility::Visible;
use crate::project::WorkspaceSnapshot;
use crate::project_list::ProjectList;
use crate::scan;
use crate::scan::BackgroundMsg;
use crate::scan::CachedRepoData;
use crate::scan::CargoMetadataError;
use crate::scan::CiFetchResult;
use crate::scan::DirSizes;
use crate::scan::FetchContext;
use crate::scan::ProjectDetailRequest;
use crate::tui::config_reload;
use crate::tui::constants::STARTUP_PHASE_DISK;
use crate::tui::constants::STARTUP_PHASE_GIT;
use crate::tui::constants::STARTUP_PHASE_GITHUB;
use crate::tui::constants::STARTUP_PHASE_LINT;
use crate::tui::constants::STARTUP_PHASE_METADATA;
use crate::tui::panes::PaneId;
use crate::tui::terminal;
use crate::tui::terminal::CiFetchMsg;
use crate::tui::terminal::CleanMsg;
use crate::tui::terminal::ExampleMsg;
use crate::tui::toasts;
use crate::tui::toasts::ToastStyle::Error;
use crate::tui::toasts::ToastStyle::Warning;
use crate::tui::toasts::ToastTaskId;
use crate::tui::toasts::TrackedItem;
use crate::watcher;
use crate::watcher::WatchRequest;
use crate::watcher::WatcherMsg;

#[derive(Clone)]
struct LegacyRootExpansion {
    root_path:      AbsolutePath,
    old_node_index: usize,
    had_children:   bool,
    named_groups:   Vec<usize>,
}

impl App {
    #[cfg(test)]
    pub fn apply_tree_build(&mut self, projects: ProjectList) {
        let selected_path = self
            .selected_project_path()
            .map(AbsolutePath::from)
            .or_else(|| self.selection.paths_mut().last_selected.clone());
        let should_focus_project_list = false;
        self.mutate_tree().replace_all(projects);
        self.prune_inactive_project_state();
        self.register_lint_for_root_items();
        self.refresh_lint_runs_from_disk();
        self.scan.bump_generation();

        // Try to restore selection
        if let Some(path) = selected_path {
            self.select_project_in_tree(path.as_path());
        } else if !self.projects().is_empty() {
            self.pane_manager_mut()
                .pane_mut(PaneId::ProjectList)
                .set_pos(0);
        }
        if should_focus_project_list {
            self.focus_pane(PaneId::ProjectList);
        }
        self.sync_selected_project();
    }

    pub fn record_config_reload_failure(&mut self, err: &str) {
        self.status_flash = Some((
            "Config reload failed; keeping previous settings".to_string(),
            Instant::now(),
        ));
        self.show_timed_toast("Config reload failed", err.to_string());
    }

    pub fn load_initial_keymap(&mut self) {
        let vim_mode = self.config.current().tui.navigation_keys;
        let result = keymap::load_keymap(vim_mode);
        self.keymap.replace_current(result.keymap);
        self.keymap.sync_stamp();
        if !result.errors.is_empty() {
            self.show_keymap_diagnostics(&result.errors);
        }
        if !result.missing_actions.is_empty() {
            self.show_timed_toast(
                "Keymap updated",
                format!(
                    "Defaults written for missing entries:\n{}",
                    result.missing_actions.join(", ")
                ),
            );
        }
    }

    pub fn maybe_reload_keymap_from_disk(&mut self) {
        let Some(path) = self.keymap.take_stamp_change() else {
            return;
        };
        let path = path.to_path_buf();
        let contents = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => {
                self.show_keymap_diagnostics(&[KeymapError {
                    scope:  String::new(),
                    action: String::new(),
                    key:    String::new(),
                    reason: Parse(format!("read error: {e}")),
                }]);
                return;
            },
        };

        let vim_mode = self.config.current().tui.navigation_keys;
        let result = keymap::load_keymap_from_str(&contents, vim_mode);
        self.keymap.replace_current(result.keymap);

        if result.errors.is_empty() {
            self.dismiss_keymap_diagnostics();
        } else {
            self.show_keymap_diagnostics(&result.errors);
        }

        if !result.missing_actions.is_empty() {
            let content = crate::keymap::ResolvedKeymap::default_toml_from(self.keymap.current());
            let _ = std::fs::write(&path, content);
            self.keymap.sync_stamp();
            self.show_timed_toast(
                "Keymap updated",
                format!(
                    "Defaults written for missing entries:\n{}",
                    result.missing_actions.join(", ")
                ),
            );
        }
    }

    pub fn sync_keymap_stamp(&mut self) { self.keymap.sync_stamp(); }

    fn show_keymap_diagnostics(&mut self, errors: &[KeymapError]) {
        // Dismiss previous diagnostics toast if any.
        self.dismiss_keymap_diagnostics();

        let body = errors
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        let action_path = self
            .keymap
            .path()
            .map(|p| AbsolutePath::from(p.to_path_buf()));

        let id = self.toasts.push_persistent(
            "Keymap errors (using defaults)",
            body,
            Error,
            action_path,
            1,
        );
        self.keymap.set_diagnostics_id(Some(id));
        let toast_len = self.active_toasts().len();
        self.pane_manager_mut()
            .pane_mut(PaneId::Toasts)
            .set_len(toast_len);
    }

    fn dismiss_keymap_diagnostics(&mut self) {
        if let Some(id) = self.keymap.take_diagnostics_id() {
            self.toasts.dismiss(id);
        }
    }

    pub fn maybe_reload_config_from_disk(&mut self) {
        let Some(path) = self.config.take_stamp_change() else {
            return;
        };
        let path_buf = path.to_path_buf();
        let reload_result = config::try_load_from_path(&path_buf);
        match reload_result {
            Ok(cfg) => {
                self.apply_config(&cfg);
                self.config.sync_stamp();
                self.show_timed_toast("Settings", "Reloaded from disk");
            },
            Err(err) => self.record_config_reload_failure(&err),
        }
    }

    pub fn save_and_apply_config(&mut self, cfg: &CargoPortConfig) -> Result<(), String> {
        config::save(cfg)?;
        self.apply_config(cfg);
        self.config.sync_stamp();
        Ok(())
    }

    pub fn apply_config(&mut self, cfg: &CargoPortConfig) {
        if self.config.current() == cfg {
            return;
        }

        let prev_force = self.config.current().debug.force_github_rate_limit;
        let next_force = cfg.debug.force_github_rate_limit;

        let actions = config_reload::collect_reload_actions(
            self.config.current(),
            cfg,
            config_reload::ReloadContext {
                scan_complete:       self.is_scan_complete(),
                has_cached_non_rust: self.has_cached_non_rust_projects(),
            },
        );
        config::set_active_config(cfg);
        *self.config.current_mut() = cfg.clone();
        if !self.discovery_shimmer_enabled() {
            self.scan.discovery_shimmers_mut().clear();
        }

        if prev_force != next_force {
            self.http_client.set_force_github_rate_limit(next_force);
            // Synthesize a signal so the UI reflects the flag flip
            // immediately instead of waiting for the next natural
            // GitHub request — otherwise toggling the flag would look
            // broken until the next refresh. The force flag simulates a
            // rate-limit (not a network outage), so emit `RateLimited`.
            if next_force {
                self.apply_service_signal(ServiceSignal::RateLimited(ServiceKind::GitHub));
            } else {
                self.mark_service_recovered(ServiceKind::GitHub);
            }
        }
        if actions.refresh_cpu.should_apply() {
            self.reset_cpu_placeholder();
        }

        if actions.refresh_lint_runtime.should_apply() {
            self.refresh_lint_runtime_from_config(cfg);
        }

        if actions.rescan.should_apply() {
            self.rescan();
            self.force_settings_if_unconfigured();
        } else {
            if actions.refresh_lint_runtime.should_apply() {
                self.respawn_watcher_and_register_existing_projects();
            }
            if actions.rebuild_tree.should_apply() {
                // Regroup workspace members in-place based on updated
                // inline_dirs, then refresh derived state.
                self.scan
                    .projects_mut()
                    .regroup_members(&self.config.current().tui.inline_dirs);
                self.refresh_derived_state();
            }
        }
    }

    /// Apply a lint configuration change. Cross-subsystem
    /// orchestration — not a method on any single subsystem because
    /// lint config changes fan out across three areas:
    ///
    /// - **Inflight**: respawns the lint runtime, clears in-flight lint paths, refreshes the lint
    ///   toast, syncs the running project list against the new runtime.
    /// - **Scan**-shaped state owned today by App: clears in-memory lint state on `projects`,
    ///   refreshes lint runs from disk, bumps `data_generation` so detail panes redraw. (Phase 6
    ///   moves this cluster into a real `Scan` subsystem; the call shape stays.)
    /// - **Selection**: recomputes `cached_fit_widths` because the project pane's column schema
    ///   depends on whether lints are enabled.
    ///
    /// Called from the per-tick config-reload handler (today via the
    /// `refresh_lint_runtime_from_config` shim below). New side-
    /// effects of a lint-config change MUST be added here (or in
    /// the relevant subsystem method this function calls), not in
    /// random callers — that's the point of having a single
    /// orchestrator. See "Recurring patterns" in
    /// `src/tui/app/mod.rs` for the cross-subsystem orchestrator
    /// pattern.
    pub fn apply_lint_config_change(&mut self, cfg: &CargoPortConfig) {
        // Inflight: respawn the lint runtime + clear in-flight tracking.
        let lint_spawn = lint::spawn(cfg, self.background.bg_sender());
        self.inflight.set_lint_runtime(lint_spawn.handle);
        self.inflight.running_lint_paths_mut().clear();
        self.sync_running_lint_toast();
        self.sync_lint_runtime_projects();

        // Scan-shaped state on App: clear lint state, refresh from
        // disk, bump generation.
        self.clear_all_lint_state();
        self.refresh_lint_runs_from_disk();
        self.scan.bump_generation();

        // Selection: recompute fit widths (column schema differs
        // with lint enabled / disabled).
        self.selection.reset_fit_widths(self.lint_enabled());

        if let Some(warning) = lint_spawn.warning {
            self.status_flash = Some((warning.clone(), Instant::now()));
            self.show_timed_toast("Lint runtime", warning);
        }
    }

    /// Backwards-compatible shim. Existing callers (rescan, config
    /// reload) still call `refresh_lint_runtime_from_config`; the
    /// real orchestration lives in [`Self::apply_lint_config_change`].
    pub fn refresh_lint_runtime_from_config(&mut self, cfg: &CargoPortConfig) {
        self.apply_lint_config_change(cfg);
    }

    pub fn respawn_watcher(&mut self) {
        let watch_roots = scan::resolve_include_dirs(&self.config.current().tui.include_dirs);
        let new_watcher = watcher::spawn_watcher(
            &watch_roots,
            self.background.bg_sender(),
            self.ci_run_count(),
            self.include_non_rust(),
            self.http_client.clone(),
            self.inflight.lint_runtime_clone(),
            self.metadata_store_handle(),
        );
        self.background.replace_watcher_sender(new_watcher);
    }

    pub fn register_existing_projects(&self) {
        self.projects().for_each_leaf(|item| {
            self.register_item_background_services(item);
        });
    }

    pub fn finish_watcher_registration_batch(&self) {
        let _ = self
            .background
            .send_watcher(WatcherMsg::InitialRegistrationComplete);
    }

    fn respawn_watcher_and_register_existing_projects(&mut self) {
        self.respawn_watcher();
        self.register_existing_projects();
        self.finish_watcher_registration_batch();
    }

    pub fn refresh_lint_runs_from_disk(&mut self) {
        let mut paths = Vec::new();
        self.projects().for_each_leaf_path(|path, is_rust| {
            if is_rust {
                paths.push(path.to_path_buf());
            }
        });
        for path in &paths {
            let runs = lint::read_history(path);
            if let Some(lr) = self.scan.projects_mut().lint_at_path_mut(path) {
                lr.set_runs(runs, path);
            }
        }
        self.refresh_lint_cache_usage_from_disk();
    }

    pub fn reload_lint_history(&mut self, project_path: &Path) {
        if !self.is_rust_at_path(project_path) {
            return;
        }
        let runs = lint::read_history(project_path);
        if let Some(lr) = self.scan.projects_mut().lint_at_path_mut(project_path) {
            lr.set_runs(runs, project_path);
        }
    }

    pub fn refresh_lint_cache_usage_from_disk(&mut self) {
        let cache_size_bytes = self
            .config
            .current()
            .lint
            .cache_size_bytes()
            .unwrap_or(None);
        self.scan
            .set_lint_cache_usage(lint::retained_cache_usage(cache_size_bytes));
    }

    /// Register file-system watchers for every item in the tree after a
    /// single-pass scan delivers the complete tree.
    fn register_background_services_for_tree(&self) {
        let started = Instant::now();
        let mut count = 0usize;
        self.projects().for_each_leaf(|item| {
            self.register_item_background_services(item);
            count += 1;
        });
        tracing::info!(
            elapsed_ms = crate::perf_log::ms(started.elapsed().as_millis()),
            count,
            "register_background_services_for_tree"
        );
    }

    pub fn register_item_background_services(&self, item: &RootItem) {
        let started = Instant::now();
        let abs_path = AbsolutePath::from(item.path().to_path_buf());
        let repo_root = project::git_repo_root(&abs_path);
        let has_repo_root = repo_root.is_some();
        let _ = self
            .background
            .send_watcher(WatcherMsg::Register(WatchRequest {
                project_label: abs_path.to_string_lossy().to_string(),
                abs_path: abs_path.clone(),
                repo_root,
            }));
        tracing::info!(
            elapsed_ms = crate::perf_log::ms(started.elapsed().as_millis()),
            path = %item.display_path(),
            has_repo_root,
            "app_register_project_background_services"
        );
    }

    fn schedule_startup_project_details(&self) {
        let tx = self.background.bg_sender();
        let fetch_context = std::sync::Arc::new(FetchContext {
            client: self.http_client.clone(),
        });
        self.projects().for_each_leaf(|item| {
            let abs_path = item.path().to_path_buf();
            let display_path = item.display_path().into_string();
            let project_name = item
                .is_rust()
                .then(|| item.name().map(str::to_string))
                .flatten()
                .filter(|_| {
                    self.projects()
                        .rust_info_at_path(item.path())
                        .is_some_and(|r| r.cargo().publishable())
                });
            let repo_presence = if project::git_repo_root(&abs_path).is_some() {
                GitRepoPresence::InRepo
            } else {
                GitRepoPresence::OutsideRepo
            };
            let tx = tx.clone();
            let fetch_context = std::sync::Arc::clone(&fetch_context);
            rayon::spawn(move || {
                let request = ProjectDetailRequest {
                    tx: &tx,
                    fetch_context: fetch_context.as_ref(),
                    _project_path: display_path.as_str(),
                    abs_path: &abs_path,
                    project_name: project_name.as_deref(),
                    repo_presence,
                };
                scan::fetch_project_details(&request);
            });
        });
        self.schedule_member_crates_io_fetches();
    }

    /// Fire crates.io fetches for publishable workspace members and vendored
    /// crates.
    ///
    /// `schedule_startup_project_details` only iterates leaf-level projects
    /// (workspace roots), not individual workspace members or vendored
    /// crates. This method supplements it by iterating both and fetching
    /// crates.io data for each publishable one.
    fn schedule_member_crates_io_fetches(&self) {
        let tx = self.background.bg_sender();
        let client = self.http_client.clone();
        let mut targets: Vec<(AbsolutePath, String)> = Vec::new();
        for entry in self.projects() {
            collect_publishable_children(&entry.item, &mut targets);
        }
        if targets.is_empty() {
            return;
        }
        rayon::spawn(move || {
            for (path, name) in targets {
                let (info, signal) = client.fetch_crates_io_info(&name);
                scan::emit_service_signal(&tx, signal);
                if let Some(info) = info {
                    let _ = tx.send(BackgroundMsg::CratesIoVersion {
                        path,
                        version: info.version,
                        downloads: info.downloads,
                    });
                }
            }
        });
    }

    pub fn schedule_git_first_commit_refreshes(&self) {
        let tx = self.background.bg_sender();
        let mut projects_by_repo: HashMap<AbsolutePath, Vec<AbsolutePath>> = HashMap::new();
        self.projects().for_each_leaf_path(|path, _| {
            let abs_path = AbsolutePath::from(path);
            let Some(repo_root) = project::git_repo_root(&abs_path) else {
                return;
            };
            projects_by_repo
                .entry(repo_root)
                .or_default()
                .push(abs_path);
        });
        std::thread::spawn(move || {
            for (repo_root, paths) in projects_by_repo {
                let started = Instant::now();
                let first_commit = project::get_first_commit(&repo_root);
                tracing::info!(
                    elapsed_ms = crate::perf_log::ms(started.elapsed().as_millis()),
                    repo_root = %repo_root.display(),
                    rows = paths.len(),
                    found = first_commit.is_some(),
                    "git_first_commit_fetch"
                );
                for path in paths {
                    let _ = tx.send(BackgroundMsg::GitFirstCommit {
                        path,
                        first_commit: first_commit.clone(),
                    });
                }
            }
        });
    }

    /// Collect root project paths and metadata for the lint runtime.
    fn lint_runtime_root_entries(&self) -> Vec<(AbsolutePath, bool)> {
        let mut seen = HashSet::new();
        let mut entries = Vec::new();

        for entry in self.projects() {
            let items: Vec<(&AbsolutePath, bool)> = match &entry.item {
                RootItem::Worktrees(crate::project::WorktreeGroup::Workspaces {
                    primary,
                    linked,
                    ..
                }) => std::iter::once(primary)
                    .chain(linked.iter())
                    .map(|p| (p.path(), true))
                    .collect(),
                RootItem::Worktrees(crate::project::WorktreeGroup::Packages {
                    primary,
                    linked,
                    ..
                }) => std::iter::once(primary)
                    .chain(linked.iter())
                    .map(|p| (p.path(), true))
                    .collect(),
                _ => vec![(entry.item.path(), entry.item.is_rust())],
            };
            for (path, is_rust) in items {
                let owned = path.clone();
                if seen.insert(owned.clone()) {
                    entries.push((owned, is_rust));
                }
            }
        }

        entries
    }

    pub fn lint_runtime_projects_snapshot(&self) -> Vec<RegisterProjectRequest> {
        if !self.is_scan_complete() {
            return Vec::new();
        }
        self.lint_runtime_root_entries()
            .into_iter()
            .filter(|(path, _)| !self.is_deleted(path))
            .map(|(abs_path, is_rust)| RegisterProjectRequest {
                project_label: project::home_relative_path(&abs_path),
                abs_path,
                is_rust,
            })
            .collect()
    }

    pub fn sync_lint_runtime_projects(&self) {
        let Some(runtime) = self.inflight.lint_runtime() else {
            return;
        };
        runtime.sync_projects(self.lint_runtime_projects_snapshot());
    }

    fn register_lint_for_root_items(&self) -> usize {
        let Some(runtime) = self.inflight.lint_runtime() else {
            return 0;
        };
        let mut count = 0;
        for entry in self.projects() {
            match &entry.item {
                RootItem::Rust(RustProject::Workspace(ws)) => {
                    runtime.register_project(RegisterProjectRequest {
                        project_label: ws.display_path().into_string(),
                        abs_path:      ws.path().clone(),
                        is_rust:       true,
                    });
                    count += 1;
                },
                RootItem::Rust(RustProject::Package(pkg)) => {
                    runtime.register_project(RegisterProjectRequest {
                        project_label: pkg.display_path().into_string(),
                        abs_path:      pkg.path().clone(),
                        is_rust:       true,
                    });
                    count += 1;
                },
                RootItem::Worktrees(crate::project::WorktreeGroup::Workspaces {
                    primary,
                    linked,
                    ..
                }) => {
                    for ws in std::iter::once(primary).chain(linked.iter()) {
                        runtime.register_project(RegisterProjectRequest {
                            project_label: ws.display_path().into_string(),
                            abs_path:      ws.path().clone(),
                            is_rust:       true,
                        });
                        count += 1;
                    }
                },
                RootItem::Worktrees(crate::project::WorktreeGroup::Packages {
                    primary,
                    linked,
                    ..
                }) => {
                    for pkg in std::iter::once(primary).chain(linked.iter()) {
                        runtime.register_project(RegisterProjectRequest {
                            project_label: pkg.display_path().into_string(),
                            abs_path:      pkg.path().clone(),
                            is_rust:       true,
                        });
                        count += 1;
                    }
                },
                RootItem::NonRust(_) => {},
            }
        }
        tracing::info!(count, "lint_register_root_items");
        count
    }

    fn register_lint_project_if_eligible(&self, item: &RootItem) {
        if !item.is_rust() {
            tracing::info!(reason = "not_rust", path = %item.display_path(), "lint_register_skip");
            return;
        }
        let path = item.path();
        // Skip workspace members — the workspace root's watcher covers them.
        let mut is_member = false;
        self.projects().for_each_leaf(|existing| {
            if matches!(
                &existing.item,
                RootItem::Rust(crate::project::RustProject::Workspace(_))
            ) && existing.item.path() != path
                && path.starts_with(existing.item.path())
            {
                is_member = true;
            }
        });
        if is_member {
            tracing::info!(reason = "workspace_member", path = %item.display_path(), "lint_register_skip");
            return;
        }
        let Some(runtime) = self.inflight.lint_runtime() else {
            tracing::info!(reason = "no_runtime", path = %item.display_path(), "lint_register_skip");
            return;
        };
        tracing::info!(path = %item.display_path(), "lint_register");
        runtime.register_project(RegisterProjectRequest {
            project_label: item.display_path().into_string(),
            abs_path:      path.clone(),
            is_rust:       true,
        });
    }

    fn register_lint_for_path(&self, path: &Path) {
        if let Some(item) = self.projects().iter().find(|i| i.path() == path) {
            self.register_lint_project_if_eligible(item);
        }
    }

    pub fn initialize_startup_phase_tracker(&mut self) {
        self.reset_startup_phase_state();
        self.start_startup_toast();
        self.start_startup_detail_toasts();
        self.log_startup_phase_plan();
        self.maybe_log_startup_phase_completions();
    }

    fn reset_startup_phase_state(&mut self) {
        let disk_expected = snapshots::initial_disk_roots(self.projects());
        let git_expected = self
            .projects()
            .git_directories()
            .into_iter()
            .collect::<HashSet<_>>();
        let git_seen = self
            .projects()
            .iter()
            .filter(|entry| entry.item.git_info().is_some())
            .filter_map(|entry| entry.item.git_directory())
            .collect::<HashSet<_>>();
        let metadata_expected = snapshots::initial_metadata_roots(self.projects());
        self.scan.scan_state_mut().startup_phases.scan_complete_at = Some(Instant::now());
        self.scan.scan_state_mut().startup_phases.startup_toast = None;
        self.scan
            .scan_state_mut()
            .startup_phases
            .startup_complete_at = None;
        self.scan
            .scan_state_mut()
            .startup_phases
            .disk
            .reset_with_expected(disk_expected);
        self.scan
            .scan_state_mut()
            .startup_phases
            .git
            .reset_with_expected(git_expected);
        self.scan.scan_state_mut().startup_phases.git.seen = git_seen;
        self.scan
            .scan_state_mut()
            .startup_phases
            .repo
            .reset_with_expected(HashSet::new());
        self.scan
            .scan_state_mut()
            .startup_phases
            .lint
            .reset_with_expected(HashSet::new());
        self.scan
            .scan_state_mut()
            .startup_phases
            .metadata
            .reset_with_expected(metadata_expected);
    }

    // Created first so it appears above the detail toasts.
    fn start_startup_toast(&mut self) {
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
        let task_id = self.start_task_toast("Startup", "");
        self.set_task_tracked_items(task_id, &startup_items);
        self.scan.scan_state_mut().startup_phases.startup_toast = Some(task_id);
    }

    fn start_startup_detail_toasts(&mut self) {
        if let Some(disk_expected) = self
            .scan
            .scan_state_mut()
            .startup_phases
            .disk
            .expected
            .clone()
        {
            let disk_items = Self::tracked_items_for_startup(
                &disk_expected,
                &self.scan.scan_state_mut().startup_phases.disk.seen,
            );
            if !disk_items.is_empty() {
                let body = self.startup_disk_toast_body();
                let task_id = self.start_task_toast("Calculating disk usage", &body);
                self.set_task_tracked_items(task_id, &disk_items);
                self.scan.scan_state_mut().startup_phases.disk.toast = Some(task_id);
            }
        }

        if let Some(git_expected) = self
            .scan
            .scan_state_mut()
            .startup_phases
            .git
            .expected
            .clone()
        {
            let git_items = Self::tracked_items_for_startup(
                &git_expected,
                &self.scan.scan_state_mut().startup_phases.git.seen,
            );
            if !git_items.is_empty() {
                let body = self.startup_git_toast_body();
                let task_id = self.start_task_toast("Scanning local git repos", &body);
                self.set_task_tracked_items(task_id, &git_items);
                self.scan.scan_state_mut().startup_phases.git.toast = Some(task_id);
            }
        }
        if let Some(metadata_expected) = self
            .scan
            .scan_state_mut()
            .startup_phases
            .metadata
            .expected
            .clone()
        {
            let metadata_items = Self::tracked_items_for_startup(
                &metadata_expected,
                &self.scan.scan_state_mut().startup_phases.metadata.seen,
            );
            if !metadata_items.is_empty() {
                let body = self.startup_metadata_toast_body();
                let task_id = self.start_task_toast("Running cargo metadata", &body);
                self.set_task_tracked_items(task_id, &metadata_items);
                self.scan.scan_state_mut().startup_phases.metadata.toast = Some(task_id);
            }
        }
        // The "Retrieving GitHub repo details" toast is driven by
        // `sync_running_repo_fetch_toast` from live `RepoFetchQueued`
        // messages — no separate startup-phase toast here.
    }

    fn log_startup_phase_plan(&self) {
        tracing::info!(
            disk_expected = self.scan.scan_state().startup_phases.disk.expected_len(),
            git_expected = self.scan.scan_state().startup_phases.git.expected_len(),
            repo_expected = self.scan.scan_state().startup_phases.repo.expected_len(),
            lint_expected = self.scan.scan_state().startup_phases.lint.expected_len(),
            metadata_expected = self
                .scan
                .scan_state()
                .startup_phases
                .metadata
                .expected_len(),
            "startup_phase_plan"
        );
    }

    pub fn maybe_log_startup_phase_completions(&mut self) {
        let Some(scan_complete_at) = self.scan.scan_state_mut().startup_phases.scan_complete_at
        else {
            return;
        };
        let now = Instant::now();
        self.maybe_complete_startup_disk(now, scan_complete_at);
        self.maybe_complete_startup_git(now, scan_complete_at);
        self.maybe_complete_startup_repo(now, scan_complete_at);
        self.maybe_complete_startup_metadata(now, scan_complete_at);
        self.maybe_complete_startup_lints(now, scan_complete_at);
        self.maybe_complete_startup_ready(now, scan_complete_at);
    }

    pub fn maybe_complete_startup_disk(&mut self, now: Instant, scan_complete_at: Instant) {
        if !self
            .scan
            .scan_state_mut()
            .startup_phases
            .disk
            .complete_once(now)
        {
            return;
        }
        if let Some(disk_toast) = self.scan.scan_state_mut().startup_phases.disk.take_toast() {
            self.finish_task_toast(disk_toast);
        }
        if let Some(toast) = self.scan.scan_state_mut().startup_phases.startup_toast {
            self.mark_tracked_item_completed(toast, STARTUP_PHASE_DISK);
        }
        tracing::info!(
            phase = "disk_applied",
            since_scan_complete_ms =
                crate::perf_log::ms(now.duration_since(scan_complete_at).as_millis()),
            seen = self.scan.scan_state_mut().startup_phases.disk.seen.len(),
            expected = self
                .scan
                .scan_state_mut()
                .startup_phases
                .disk
                .expected_len(),
            "startup_phase_complete"
        );
    }

    pub fn maybe_complete_startup_git(&mut self, now: Instant, scan_complete_at: Instant) {
        if !self
            .scan
            .scan_state_mut()
            .startup_phases
            .git
            .complete_once(now)
        {
            return;
        }
        if let Some(git_toast) = self.scan.scan_state_mut().startup_phases.git.take_toast() {
            self.finish_task_toast(git_toast);
        }
        if let Some(toast) = self.scan.scan_state_mut().startup_phases.startup_toast {
            self.mark_tracked_item_completed(toast, STARTUP_PHASE_GIT);
        }
        tracing::info!(
            phase = "git_local_applied",
            since_scan_complete_ms =
                crate::perf_log::ms(now.duration_since(scan_complete_at).as_millis()),
            seen = self.scan.scan_state_mut().startup_phases.git.seen.len(),
            expected = self.scan.scan_state_mut().startup_phases.git.expected_len(),
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
        if self
            .scan
            .scan_state_mut()
            .startup_phases
            .git
            .complete_at
            .is_none()
        {
            return;
        }
        if !self
            .scan
            .scan_state_mut()
            .startup_phases
            .repo
            .complete_once(now)
        {
            return;
        }
        if let Some(toast) = self.scan.scan_state_mut().startup_phases.startup_toast {
            self.mark_tracked_item_completed(toast, STARTUP_PHASE_GITHUB);
        }
        tracing::info!(
            phase = "repo_fetch_applied",
            since_scan_complete_ms =
                crate::perf_log::ms(now.duration_since(scan_complete_at).as_millis()),
            seen = self.scan.scan_state_mut().startup_phases.repo.seen.len(),
            expected = self
                .scan
                .scan_state_mut()
                .startup_phases
                .repo
                .expected_len(),
            "startup_phase_complete"
        );
    }

    pub fn maybe_complete_startup_metadata(&mut self, now: Instant, scan_complete_at: Instant) {
        if !self
            .scan
            .scan_state_mut()
            .startup_phases
            .metadata
            .complete_once(now)
        {
            return;
        }
        if let Some(metadata_toast) = self
            .scan
            .scan_state_mut()
            .startup_phases
            .metadata
            .take_toast()
        {
            self.finish_task_toast(metadata_toast);
        }
        if let Some(toast) = self.scan.scan_state_mut().startup_phases.startup_toast {
            self.mark_tracked_item_completed(toast, STARTUP_PHASE_METADATA);
        }
        tracing::info!(
            phase = "metadata_applied",
            since_scan_complete_ms =
                crate::perf_log::ms(now.duration_since(scan_complete_at).as_millis()),
            seen = self
                .scan
                .scan_state_mut()
                .startup_phases
                .metadata
                .seen
                .len(),
            expected = self
                .scan
                .scan_state_mut()
                .startup_phases
                .metadata
                .expected_len(),
            "startup_phase_complete"
        );
    }

    pub fn maybe_complete_startup_lints(&mut self, now: Instant, scan_complete_at: Instant) {
        // Lint is only "complete" once real lint work has been registered —
        // an initialized-empty expected set stays open. This diverges from
        // the generic `PhaseCompletion::is_complete` semantics on purpose,
        // so the check stays inline rather than going through the trait.
        let lint = &self.scan.scan_state_mut().startup_phases.lint;
        let should_complete = lint.complete_at.is_none()
            && lint
                .expected
                .as_ref()
                .is_some_and(|expected| !expected.is_empty() && lint.seen.len() >= expected.len());
        if !should_complete {
            return;
        }
        self.scan.scan_state_mut().startup_phases.lint.complete_at = Some(now);
        tracing::info!(
            phase = "lint_terminal_applied",
            since_scan_complete_ms =
                crate::perf_log::ms(now.duration_since(scan_complete_at).as_millis()),
            seen = self.scan.scan_state_mut().startup_phases.lint.seen.len(),
            expected = self
                .scan
                .scan_state_mut()
                .startup_phases
                .lint
                .expected_len(),
            "startup_phase_complete"
        );
    }

    pub fn maybe_complete_startup_ready(&mut self, now: Instant, scan_complete_at: Instant) {
        let phases = self.scan.scan_state_mut();
        if phases.startup_phases.startup_complete_at.is_some() {
            return;
        }
        let disk_ready = phases.startup_phases.disk.complete_at.is_some();
        let git_ready = phases.startup_phases.git.complete_at.is_some();
        let repo_ready = phases.startup_phases.repo.complete_at.is_some();
        let metadata_ready = phases.startup_phases.metadata.complete_at.is_some();
        if !(disk_ready && git_ready && repo_ready && metadata_ready) {
            return;
        }
        phases.startup_phases.startup_complete_at = Some(now);
        // Finish the startup toast only when lint startup cache check
        // is also done, so "Lint cache" doesn't spin while the toast
        // exits.
        let lint_done = phases.startup_phases.lint_startup.complete_at.is_some();
        if lint_done && let Some(toast) = phases.startup_phases.startup_toast.take() {
            self.finish_task_toast(toast);
        }
        let phases = self.scan.scan_state();
        let since_scan_ms = crate::perf_log::ms(now.duration_since(scan_complete_at).as_millis());
        tracing::info!(
            since_scan_complete_ms = since_scan_ms,
            disk_seen = phases.startup_phases.disk.seen.len(),
            disk_expected = phases.startup_phases.disk.expected_len(),
            git_seen = phases.startup_phases.git.seen.len(),
            git_expected = phases.startup_phases.git.expected_len(),
            repo_seen = phases.startup_phases.repo.seen.len(),
            repo_expected = phases.startup_phases.repo.expected_len(),
            lint_seen = phases.startup_phases.lint.seen.len(),
            lint_expected = phases.startup_phases.lint.expected_len(),
            metadata_seen = phases.startup_phases.metadata.seen.len(),
            metadata_expected = phases.startup_phases.metadata.expected_len(),
            "startup_complete"
        );
        tracing::info!(since_scan_complete_ms = since_scan_ms, "steady_state_begin");
    }

    pub fn startup_disk_toast_body(&self) -> String {
        let empty = HashSet::new();
        let expected = self
            .scan
            .scan_state()
            .startup_phases
            .disk
            .expected
            .as_ref()
            .unwrap_or(&empty);
        Self::startup_remaining_toast_body(
            expected,
            &self.scan.scan_state().startup_phases.disk.seen,
        )
    }

    pub fn startup_git_toast_body(&self) -> String {
        let empty = HashSet::new();
        let expected = self
            .scan
            .scan_state()
            .startup_phases
            .git
            .expected
            .as_ref()
            .unwrap_or(&empty);
        Self::startup_remaining_toast_body(
            expected,
            &self.scan.scan_state().startup_phases.git.seen,
        )
    }

    pub fn startup_metadata_toast_body(&self) -> String {
        let empty = HashSet::new();
        let expected = self
            .scan
            .scan_state()
            .startup_phases
            .metadata
            .expected
            .as_ref()
            .unwrap_or(&empty);
        Self::startup_remaining_toast_body(
            expected,
            &self.scan.scan_state().startup_phases.metadata.seen,
        )
    }

    /// Build tracked items from expected/seen path sets. Already-seen paths
    /// are pre-marked as completed so the renderer shows them with strikethrough.
    /// Pending items get `started_at = now` so they render with a live
    /// spinner + ticking duration that freezes when the item completes —
    /// matching the GitHub repo-fetch toast.
    pub fn tracked_items_for_startup(
        expected: &HashSet<AbsolutePath>,
        seen: &HashSet<AbsolutePath>,
    ) -> Vec<TrackedItem> {
        let now = Instant::now();
        expected
            .iter()
            .map(|path| {
                let label = project::home_relative_path(path);
                let is_seen = seen.contains(path);
                TrackedItem {
                    label,
                    key: path.into(),
                    started_at: if is_seen { None } else { Some(now) },
                    completed_at: if is_seen { Some(now) } else { None },
                }
            })
            .collect()
    }

    pub fn startup_remaining_toast_body(
        expected: &HashSet<AbsolutePath>,
        seen: &HashSet<AbsolutePath>,
    ) -> String {
        let items: Vec<String> = expected
            .iter()
            .filter(|path| !seen.contains(*path))
            .map(|p| project::home_relative_path(p))
            .collect();
        let refs: Vec<&str> = items.iter().map(String::as_str).collect();
        if refs.is_empty() {
            return "Complete".to_string();
        }
        toasts::format_toast_items(&refs, toasts::toast_body_width())
    }

    fn startup_git_directory_for_path(&self, path: &Path) -> Option<AbsolutePath> {
        self.projects()
            .iter()
            .find(|entry| entry.item.at_path(path).is_some())
            .and_then(|entry| entry.item.git_directory())
    }

    #[cfg(test)]
    pub fn startup_lint_toast_body_for(
        expected: &HashSet<AbsolutePath>,
        seen: &HashSet<AbsolutePath>,
    ) -> String {
        let items: Vec<String> = expected
            .iter()
            .filter(|path| !seen.contains(*path))
            .map(|p| project::home_relative_path(p))
            .collect();
        let refs: Vec<&str> = items.iter().map(String::as_str).collect();
        if refs.is_empty() {
            return "Complete".to_string();
        }
        toasts::format_toast_items(&refs, toasts::toast_body_width())
    }

    pub fn sync_running_clean_toast(&mut self) {
        let running = self.inflight.running_clean_paths().clone();
        let next =
            self.sync_tracked_path_toast(self.inflight.clean_toast(), "cargo clean", &running);
        self.inflight.set_clean_toast(next);
    }

    /// Shared per-path task toast sync: grows as new paths start,
    /// marks items completed (freezing elapsed + starting strikethrough)
    /// as paths finish, and begins the toast-level linger countdown once
    /// all paths are done. Used by both lint and clean flows.
    fn sync_tracked_path_toast(
        &mut self,
        toast_slot: Option<ToastTaskId>,
        title: &str,
        running_paths: &HashMap<AbsolutePath, Instant>,
    ) -> Option<ToastTaskId> {
        if running_paths.is_empty() {
            if let Some(task_id) = toast_slot {
                let empty: HashSet<String> = HashSet::new();
                self.toasts.complete_missing_items(task_id, &empty);
                if !self.toasts.is_task_finished(task_id) {
                    let linger =
                        Duration::from_secs_f64(self.config.current().tui.task_linger_secs);
                    self.toasts.finish_task(task_id, linger);
                }
            }
            return toast_slot;
        }

        let running_items: Vec<TrackedItem> = running_paths
            .iter()
            .map(|(p, &started)| TrackedItem {
                label:        project::home_relative_path(p.as_path()),
                key:          p.clone().into(),
                started_at:   Some(started),
                completed_at: None,
            })
            .collect();
        let running_keys: HashSet<String> = running_items
            .iter()
            .map(|item| item.key.to_string())
            .collect();

        if let Some(task_id) = toast_slot
            && self.toasts.reactivate_task(task_id)
        {
            self.toasts.complete_missing_items(task_id, &running_keys);
            let linger = Duration::from_secs_f64(self.config.current().tui.task_linger_secs);
            self.toasts
                .add_new_tracked_items(task_id, &running_items, linger);
            for item in &running_items {
                if let Some(started) = item.started_at {
                    self.toasts
                        .restart_tracked_item(task_id, &item.key, started);
                }
            }
            Some(task_id)
        } else {
            let labels: Vec<&str> = running_items.iter().map(|i| i.label.as_str()).collect();
            let body = toasts::format_toast_items(&labels, toasts::toast_body_width());
            let task_id = self.start_task_toast(title, body);
            self.set_task_tracked_items(task_id, &running_items);
            Some(task_id)
        }
    }

    /// Keep a single "Retrieving GitHub repo details" toast in sync
    /// with the live in-flight repo fetches — mirrors
    /// `sync_running_lint_toast`. Grows as new fetches queue, strikes
    /// through as they complete, and reactivates the same toast when a
    /// new runtime fetch arrives while the prior one is still
    /// lingering.
    fn sync_running_repo_fetch_toast(&mut self) {
        if self.github.running_fetches.is_empty() {
            if let Some(task_id) = self.github.running_fetch_toast {
                let empty: HashSet<String> = HashSet::new();
                self.toasts.complete_missing_items(task_id, &empty);
                if !self.toasts.is_task_finished(task_id) {
                    let linger =
                        Duration::from_secs_f64(self.config.current().tui.task_linger_secs);
                    self.toasts.finish_task(task_id, linger);
                }
            }
            return;
        }

        let running_items: Vec<TrackedItem> = self
            .github
            .running_fetches
            .iter()
            .map(|(repo, &started)| TrackedItem {
                label:        repo.to_string(),
                key:          repo.into(),
                started_at:   Some(started),
                completed_at: None,
            })
            .collect();
        let running_keys: HashSet<String> = running_items
            .iter()
            .map(|item| item.key.to_string())
            .collect();

        if let Some(task_id) = self.github.running_fetch_toast
            && self.toasts.reactivate_task(task_id)
        {
            self.toasts.complete_missing_items(task_id, &running_keys);
            let linger = Duration::from_secs_f64(self.config.current().tui.task_linger_secs);
            self.toasts
                .add_new_tracked_items(task_id, &running_items, linger);
            for item in &running_items {
                if let Some(started) = item.started_at {
                    self.toasts
                        .restart_tracked_item(task_id, &item.key, started);
                }
            }
        } else {
            let task_id = self.start_task_toast("Retrieving GitHub repo details", "");
            self.set_task_tracked_items(task_id, &running_items);
            self.github.running_fetch_toast = Some(task_id);
        }
    }

    pub fn sync_running_lint_toast(&mut self) {
        let running = self.inflight.running_lint_paths().clone();
        let next = self.sync_tracked_path_toast(self.inflight.lint_toast(), "Lints", &running);
        self.inflight.set_lint_toast(next);
    }

    /// Lightweight refresh of derived state after in-place hierarchy changes
    /// (discovery, refresh). Marks caches dirty without a full tree rebuild.
    pub const fn refresh_derived_state(&mut self) { self.scan.bump_generation(); }

    fn capture_legacy_root_expansions(&self) -> Vec<LegacyRootExpansion> {
        self.projects()
            .iter()
            .enumerate()
            .filter_map(|(ni, entry)| {
                if !self.selection.expanded().contains(&Node(ni)) {
                    return None;
                }

                match &entry.item {
                    RootItem::Rust(RustProject::Workspace(ws)) => Some(LegacyRootExpansion {
                        root_path:      ws.path().clone(),
                        old_node_index: ni,
                        had_children:   ws.has_members() || !ws.vendored().is_empty(),
                        named_groups:   ws
                            .groups()
                            .iter()
                            .enumerate()
                            .filter_map(|(gi, group)| {
                                group
                                    .is_named()
                                    .then(|| self.selection.expanded().contains(&Group(ni, gi)))
                                    .filter(|expanded| *expanded)
                                    .map(|_| gi)
                            })
                            .collect(),
                    }),
                    RootItem::Rust(RustProject::Package(pkg)) => Some(LegacyRootExpansion {
                        root_path:      pkg.path().clone(),
                        old_node_index: ni,
                        had_children:   !pkg.vendored().is_empty(),
                        named_groups:   Vec::new(),
                    }),
                    _ => None,
                }
            })
            .collect()
    }

    fn migrate_legacy_root_expansions(&mut self, legacy: &[LegacyRootExpansion]) {
        let Self {
            scan, selection, ..
        } = self;
        for legacy_root in legacy {
            let Some((current_index, current_entry)) = scan
                .projects()
                .iter()
                .enumerate()
                .find(|(_, entry)| entry.item.path() == legacy_root.root_path.as_path())
            else {
                continue;
            };

            match &current_entry.item {
                RootItem::Worktrees(
                    group @ crate::project::WorktreeGroup::Workspaces { primary, .. },
                ) if group.renders_as_group() => {
                    selection.expanded_mut().insert(Node(current_index));
                    if legacy_root.had_children {
                        selection.expanded_mut().insert(Worktree(current_index, 0));
                    }
                    for &group_index in &legacy_root.named_groups {
                        if primary.groups().get(group_index).is_some() {
                            selection.expanded_mut().insert(WorktreeGroup(
                                current_index,
                                0,
                                group_index,
                            ));
                        }
                        selection
                            .expanded_mut()
                            .remove(&Group(legacy_root.old_node_index, group_index));
                    }
                },
                RootItem::Worktrees(group @ crate::project::WorktreeGroup::Packages { .. })
                    if group.renders_as_group() =>
                {
                    selection.expanded_mut().insert(Node(current_index));
                    if legacy_root.had_children {
                        selection.expanded_mut().insert(Worktree(current_index, 0));
                    }
                },
                _ => {},
            }
        }
    }

    fn rebuild_visible_rows_now(&mut self) {
        let include_non_rust = self.include_non_rust().includes_non_rust();
        let Self {
            scan, selection, ..
        } = self;
        selection.recompute_visibility(scan.projects(), include_non_rust);
    }

    pub fn rescan(&mut self) {
        self.scan.projects_mut().clear();
        // disk_usage lives on project items — cleared with projects above
        self.inflight.ci_fetch_tracker_mut().clear();
        self.panes.clear_ci_display_modes();
        self.clear_all_lint_state();
        self.scan
            .set_lint_cache_usage(crate::lint::CacheUsage::default());
        self.github.fetch_cache = scan::new_repo_cache();
        self.github.repo_fetch_in_flight.clear();
        self.github.running_fetches.clear();
        self.github.running_fetch_toast = None;
        self.scan.discovery_shimmers_mut().clear();
        self.scan.scan_state_mut().phase = ScanPhase::Running;
        self.scan.scan_state_mut().started_at = Instant::now();
        self.scan.scan_state_mut().run_count += 1;
        self.scan.scan_state_mut().startup_phases = StartupPhaseTracker::default();
        tracing::info!(
            kind = "rescan",
            run = self.scan.scan_state().run_count,
            "scan_start"
        );
        self.scan.set_priority_fetch_path(None);
        self.focus_pane(PaneId::ProjectList);
        self.close_settings();
        self.close_finder();
        self.reset_project_panes();
        self.selection.paths_mut().selected_project = None;
        self.inflight.clear_pending_ci_fetch();
        self.selection.expanded_mut().clear();
        self.pane_manager_mut().pane_mut(PaneId::ProjectList).home();
        self.pane_manager_mut()
            .pane_mut(PaneId::ProjectList)
            .set_scroll_offset(0);
        self.scan.bump_generation();
        let scan_dirs = scan::resolve_include_dirs(&self.config.current().tui.include_dirs);
        let (tx, rx) = scan::spawn_streaming_scan(
            scan_dirs,
            &self.config.current().tui.inline_dirs,
            self.include_non_rust(),
            self.http_client.clone(),
            self.metadata_store_handle(),
        );
        self.background.swap_bg_channel(tx, rx);
        self.respawn_watcher();
        let current_config = self.config.current().clone();
        self.refresh_lint_runtime_from_config(&current_config);
    }

    pub fn poll_background(&mut self) -> PollBackgroundStats {
        const MAX_MSGS_PER_FRAME: usize = 50;
        let mut needs_rebuild = false;
        let mut msg_count = 0;
        let started = Instant::now();
        let mut stats = PollBackgroundStats::default();

        while msg_count < MAX_MSGS_PER_FRAME {
            let Ok(msg) = self.background.bg_rx().try_recv() else {
                break;
            };
            Self::record_background_msg_kind(&mut stats, &msg);
            msg_count += 1;
            needs_rebuild |= self.handle_bg_msg(msg);
        }
        stats.bg_msgs = msg_count;
        Self::log_saturated_background_batch(&stats);
        stats.ci_msgs = self.poll_ci_fetches();
        stats.example_msgs = self.poll_example_msgs();
        self.poll_clean_msgs();

        stats.tree_results = 0;
        stats.fit_results = 0;
        stats.disk_results = 0;

        if needs_rebuild {
            self.refresh_derived_state();
            self.maybe_priority_fetch();
        }
        stats.needs_rebuild = needs_rebuild;

        let elapsed = started.elapsed();
        if elapsed.as_millis() >= crate::perf_log::SLOW_BG_BATCH_MS {
            tracing::info!(
                elapsed_ms = crate::perf_log::ms(elapsed.as_millis()),
                bg_msgs = stats.bg_msgs,
                ci_msgs = stats.ci_msgs,
                example_msgs = stats.example_msgs,
                tree_results = stats.tree_results,
                fit_results = stats.fit_results,
                disk_results = stats.disk_results,
                needs_rebuild = stats.needs_rebuild,
                items = self.projects().len(),
                "poll_background"
            );
        }
        stats
    }

    pub const fn record_background_msg_kind(stats: &mut PollBackgroundStats, msg: &BackgroundMsg) {
        match msg {
            BackgroundMsg::DiskUsage { .. } | BackgroundMsg::DiskUsageBatch { .. } => {
                stats.disk_usage_msgs += 1;
            },
            BackgroundMsg::CheckoutInfo { .. }
            | BackgroundMsg::RepoInfo { .. }
            | BackgroundMsg::GitFirstCommit { .. } => {
                stats.git_info_msgs += 1;
            },
            BackgroundMsg::LintStatus { .. } | BackgroundMsg::LintStartupStatus { .. } => {
                stats.lint_status_msgs += 1;
            },
            BackgroundMsg::CiRuns { .. }
            | BackgroundMsg::RepoFetchQueued { .. }
            | BackgroundMsg::RepoFetchComplete { .. }
            | BackgroundMsg::CratesIoVersion { .. }
            | BackgroundMsg::RepoMeta { .. }
            | BackgroundMsg::Submodules { .. }
            | BackgroundMsg::ScanResult { .. }
            | BackgroundMsg::ProjectDiscovered { .. }
            | BackgroundMsg::ProjectRefreshed { .. }
            | BackgroundMsg::LintCachePruned { .. }
            | BackgroundMsg::ServiceReachable { .. }
            | BackgroundMsg::ServiceRecovered { .. }
            | BackgroundMsg::ServiceUnreachable { .. }
            | BackgroundMsg::ServiceRateLimited { .. }
            | BackgroundMsg::LanguageStatsBatch { .. }
            | BackgroundMsg::CargoMetadata { .. }
            | BackgroundMsg::OutOfTreeTargetSize { .. } => {},
        }
    }

    pub fn log_saturated_background_batch(stats: &PollBackgroundStats) {
        const MAX_MSGS_PER_FRAME: usize = 50;
        if stats.bg_msgs != MAX_MSGS_PER_FRAME {
            return;
        }

        tracing::info!(
            bg_msgs = stats.bg_msgs,
            disk_usage_msgs = stats.disk_usage_msgs,
            git_info_msgs = stats.git_info_msgs,
            lint_status_msgs = stats.lint_status_msgs,
            "poll_background_saturated"
        );
    }

    pub fn poll_ci_fetches(&mut self) -> usize {
        let mut count = 0;
        while let Ok(msg) = self.background.ci_fetch_rx().try_recv() {
            match msg {
                CiFetchMsg::Complete { path, result, kind } => {
                    let before = self
                        .ci_info_for(Path::new(&path))
                        .map_or(0, |info| info.runs.len());
                    self.handle_ci_fetch_complete(&path, result, kind);
                    let after = self
                        .ci_info_for(Path::new(&path))
                        .map_or(0, |info| info.runs.len());
                    let new_runs = after.saturating_sub(before);
                    if let Some(task_id) = self.inflight.take_ci_fetch_toast() {
                        let empty: std::collections::HashSet<String> =
                            std::collections::HashSet::new();
                        self.toasts.complete_missing_items(task_id, &empty);
                        let label = if new_runs > 0 {
                            format!("{new_runs} new runs fetched")
                        } else {
                            "no new runs".to_string()
                        };
                        let result_item = TrackedItem {
                            label,
                            key: AbsolutePath::from(format!("{path}:result")).into(),
                            started_at: None,
                            completed_at: None,
                        };
                        let linger = std::time::Duration::from_secs_f64(
                            self.config.current().tui.task_linger_secs,
                        );
                        self.toasts
                            .add_new_tracked_items(task_id, &[result_item], linger);
                        self.finish_task_toast(task_id);
                    }
                },
            }
            count += 1;
        }
        count
    }

    pub fn poll_example_msgs(&mut self) -> usize {
        let mut count = 0;
        while let Ok(msg) = self.background.example_rx().try_recv() {
            match msg {
                ExampleMsg::Output(line) => self.inflight.example_output_mut().push(line),
                ExampleMsg::Progress(line) => self.apply_example_progress(line),
                ExampleMsg::Finished => self.finish_example_run(),
            }
            count += 1;
        }
        count
    }

    pub fn apply_example_progress(&mut self, line: String) {
        if let Some(last) = self.inflight.example_output_mut().last_mut() {
            *last = line;
        } else {
            self.inflight.example_output_mut().push(line);
        }
    }

    pub fn finish_example_run(&mut self) {
        self.inflight.set_example_running(None);
        self.inflight
            .example_output_mut()
            .push("── done ──".to_string());
        self.mark_terminal_dirty();
    }

    pub fn poll_clean_msgs(&mut self) {
        while let Ok(msg) = self.background.clean_rx().try_recv() {
            match msg {
                CleanMsg::Finished(abs_path) => {
                    // Normally `handle_disk_usage` removes the path
                    // first (filesystem watcher sees target/ shrink).
                    // This is the safety-net terminator if no disk
                    // update arrives.
                    if self
                        .inflight
                        .running_clean_paths_mut()
                        .remove(abs_path.as_path())
                        .is_some()
                    {
                        self.sync_running_clean_toast();
                    }
                },
            }
        }
    }

    pub fn handle_disk_usage(&mut self, path: &Path, bytes: u64) {
        if self
            .inflight
            .running_clean_paths_mut()
            .remove(path)
            .is_some()
        {
            self.sync_running_clean_toast();
        }
        self.apply_disk_usage(path, bytes);
    }

    pub fn handle_disk_usage_batch(&mut self, entries: Vec<(AbsolutePath, DirSizes)>) {
        for (path, sizes) in entries {
            self.apply_disk_usage_breakdown(path.as_path(), sizes);
        }
    }

    /// Apply a [`DirSizes`] breakdown to the matching project. Shares
    /// the post-set logic with `apply_disk_usage` (visibility /
    /// lint-runtime registration) by reusing that helper for the
    /// total — the new breakdown fields just ride alongside.
    fn apply_disk_usage_breakdown(&mut self, path: &Path, sizes: DirSizes) {
        if let Some(project) = self.projects_mut().at_path_mut(path) {
            project.in_project_target = Some(sizes.in_project_target);
            project.in_project_non_target = Some(sizes.in_project_non_target);
        }
        self.apply_disk_usage(path, sizes.total);
    }

    pub fn apply_disk_usage(&mut self, path: &Path, bytes: u64) {
        // Set disk usage on the matching project item and update visibility.
        let mut lint_runtime_changed = false;
        if let Some(project) = self.projects_mut().at_path_mut(path) {
            project.disk_usage_bytes = Some(bytes);
            if bytes == 0 && !path.exists() && project.visibility != Deleted {
                project.visibility = Deleted;
                lint_runtime_changed = true;
            } else if bytes > 0 && project.visibility != Visible {
                project.visibility = Visible;
                lint_runtime_changed = true;
            }
        }
        if lint_runtime_changed {
            if let Some(runtime) = self.inflight.lint_runtime()
                && bytes == 0
            {
                runtime.unregister_project(AbsolutePath::from(path));
            }
            if bytes > 0 {
                self.register_lint_for_path(path);
            }
        }
    }

    fn spawn_repo_fetch_for_git_info(&mut self, path: &Path, repo_url: &str) {
        let Some(owner_repo) = ci::parse_owner_repo(repo_url) else {
            return;
        };
        // Dedup by `OwnerRepo`: a fetch for this repo is either already
        // running or queued. The `RepoFetchComplete` background message
        // removes the entry, so a later spawn after completion is not
        // blocked.
        if !self.github.repo_fetch_in_flight.insert(owner_repo.clone()) {
            return;
        }

        let tx = self.background.bg_sender();
        let client = self.http_client.clone();
        let repo_cache = self.github.fetch_cache.clone();
        let path: AbsolutePath = AbsolutePath::from(path);
        let repo_url = repo_url.to_string();
        let ci_run_count = self.ci_run_count();
        thread::spawn(move || {
            let data = scan::load_cached_repo_data(&repo_cache, &owner_repo).unwrap_or_else(|| {
                let _ = tx.send(BackgroundMsg::RepoFetchQueued {
                    repo: owner_repo.clone(),
                });
                let (result, meta, signal) = scan::fetch_ci_runs_cached(
                    &client,
                    &repo_url,
                    owner_repo.owner(),
                    owner_repo.repo(),
                    ci_run_count,
                );
                scan::emit_service_signal(&tx, signal);
                let (runs, github_total) = match result {
                    CiFetchResult::Loaded { runs, github_total } => (runs, github_total),
                    CiFetchResult::CacheOnly(runs) => (runs, 0),
                };
                let data = CachedRepoData {
                    runs,
                    meta,
                    github_total,
                };
                scan::store_cached_repo_data(&repo_cache, &owner_repo, data.clone());
                data
            });

            let _ = tx.send(BackgroundMsg::CiRuns {
                path:         path.clone(),
                runs:         data.runs,
                github_total: data.github_total,
            });
            if let Some(meta) = data.meta {
                let _ = tx.send(BackgroundMsg::RepoMeta {
                    path,
                    stars: meta.stars,
                    description: meta.description,
                });
            }
            // Fire `RepoFetchComplete` from the always-runs tail so the
            // dedup set clears on cache hits too. The startup toast
            // handler is a no-op for repos that were never queued.
            let _ = tx.send(BackgroundMsg::RepoFetchComplete { repo: owner_repo });
        });
    }

    /// Handle a per-checkout git state update. Writes to the
    /// `ProjectInfo.local_git_state` for `path`, runs startup tracking
    /// hooks, and triggers a repo-level fetch if applicable. The repo
    /// fetch trigger is here because either a `RepoInfo` or
    /// `CheckoutInfo` arrival can signal "this repo's state changed";
    /// the dedup set absorbs N attempts for the same `OwnerRepo`.
    pub fn handle_checkout_info(&mut self, path: &Path, info: CheckoutInfo) {
        tracing::info!(
            path = %path.display(),
            git_status = %info.status.label(),
            "checkout_info_applied"
        );

        if let Some(project) = self.projects_mut().at_path_mut(path) {
            project.local_git_state = LocalGitState::Detected(Box::new(info));
        }
        // Detected git state implies the entry is in a git repo. Ensure
        // the entry has a `git_repo` slot so per-repo writes (CI,
        // GitHub meta, RepoInfo) can land on it.
        if let Some(entry) = self.scan.projects_mut().entry_containing_mut(path) {
            entry.git_repo.get_or_insert_with(Default::default);
        }

        if self.is_scan_complete() {
            let git_dir = self
                .startup_git_directory_for_path(path)
                .unwrap_or_else(|| AbsolutePath::from(path));
            self.scan
                .scan_state_mut()
                .startup_phases
                .git
                .seen
                .insert(git_dir.clone());
            if let Some(git_toast) = self.scan.scan_state_mut().startup_phases.git.toast {
                self.mark_tracked_item_completed(git_toast, &git_dir.to_string());
            }
            self.maybe_log_startup_phase_completions();
        }

        self.maybe_trigger_repo_fetch(path);
    }

    /// Handle a per-repo git state update. Only the primary checkout
    /// writes `RepoInfo` (linked worktrees share the primary's
    /// `.git/config` by design; admitting last-writer-wins from any
    /// checkout would produce silent arbitration if they ever
    /// diverged). The `path` is the primary's path — the emitter is
    /// responsible for that contract.
    pub fn handle_repo_info(&mut self, path: &Path, mut info: RepoInfo) {
        // Preserve a previously-fetched `first_commit` across refresh.
        // `RepoInfo::get` always returns `None` for it; the value is
        // filled in either by a prior `handle_git_first_commit` write
        // or via the `pending_git_first_commit` map below.
        let preserved_first_commit = self
            .repo_info_for(path)
            .and_then(|existing| existing.first_commit.clone());
        if info.first_commit.is_none() {
            info.first_commit = preserved_first_commit
                .or_else(|| self.scan.pending_git_first_commit_mut().remove(path));
        }

        // Gate GitHub cache invalidation on `FETCH_HEAD` mtime actually
        // advancing. Without this, every watcher tick / commit / branch
        // switch would invalidate the cache and trigger a refetch,
        // burning REST quota. ISO 8601 strings compare lexically in
        // chronological order, so `!=` captures advance reliably.
        let previous_last_fetched = self
            .repo_info_for(path)
            .and_then(|existing| existing.last_fetched.clone());
        let fetch_head_advanced =
            info.last_fetched.is_some() && info.last_fetched != previous_last_fetched;

        if let Some(entry) = self.scan.projects_mut().entry_containing_mut(path) {
            if entry.item.path().as_path() != path {
                // Non-primary write — discard per the policy above.
                return;
            }
            let git_repo = entry.git_repo.get_or_insert_with(Default::default);
            git_repo.repo_info = Some(info);
        }

        if fetch_head_advanced
            && self.is_scan_complete()
            && let Some(url) = self.fetch_url_for(path)
            && let Some(owner_repo) = ci::parse_owner_repo(&url)
            && !self.github.repo_fetch_in_flight.contains(&owner_repo)
        {
            scan::invalidate_cached_repo_data(&self.github.fetch_cache, &owner_repo);
        }

        self.maybe_trigger_repo_fetch(path);
    }

    /// Shared between `handle_repo_info` and `handle_checkout_info`:
    /// kick a GitHub fetch for this path's repo if we have a parseable
    /// remote URL. The dedup set absorbs concurrent attempts for the
    /// same `OwnerRepo`; cache invalidation is gated inside
    /// `handle_repo_info` by `last_fetched` advance. Submodule paths are
    /// excluded — submodule CI/metadata is shown on the parent project.
    fn maybe_trigger_repo_fetch(&mut self, path: &Path) {
        if self.projects().is_submodule_path(path) {
            return;
        }
        let Some(url) = self.fetch_url_for(path) else {
            return;
        };
        self.spawn_repo_fetch_for_git_info(path, &url);
    }

    pub fn handle_git_first_commit(&mut self, path: &Path, first_commit: Option<&str>) {
        let first_commit = first_commit.map(String::from);
        // first_commit is per-repo, so it lands on the entry's
        // `RepoInfo`. If the entry's `repo_info` slot doesn't exist yet
        // (`RepoInfo::get` hasn't completed), stash the value in
        // `pending_git_first_commit` and `handle_repo_info` will fold
        // it in when repo info arrives.
        let applied = self
            .scan
            .projects_mut()
            .entry_containing_mut(path)
            .and_then(|entry| entry.git_repo.as_mut()?.repo_info.as_mut())
            .map(|repo| repo.first_commit.clone_from(&first_commit))
            .is_some();
        if applied {
            self.scan.pending_git_first_commit_mut().remove(path);
        } else if let Some(first_commit) = first_commit {
            self.scan
                .pending_git_first_commit_mut()
                .insert(AbsolutePath::from(path), first_commit);
        } else {
            self.scan.pending_git_first_commit_mut().remove(path);
        }
    }

    fn handle_repo_fetch_queued(&mut self, repo: OwnerRepo) {
        let first_repo = self
            .scan
            .scan_state_mut()
            .startup_phases
            .repo
            .expected
            .as_ref()
            .is_none_or(HashSet::is_empty);
        self.scan
            .scan_state_mut()
            .startup_phases
            .repo
            .ensure_expected()
            .insert(repo.clone());
        self.github.running_fetches.insert(repo, Instant::now());
        if first_repo {
            // First repo queued — add the "GitHub repos" tracked item
            // to the startup toast and reset completion so the phase
            // is re-evaluated now that there's actual work to track.
            self.scan.scan_state_mut().startup_phases.repo.complete_at = None;
            self.scan
                .scan_state_mut()
                .startup_phases
                .startup_complete_at = None;
            if let Some(toast) = self.scan.scan_state_mut().startup_phases.startup_toast {
                let linger = Duration::from_secs_f64(self.config.current().tui.task_linger_secs);
                self.toasts.add_new_tracked_items(
                    toast,
                    &[TrackedItem {
                        label:        STARTUP_PHASE_GITHUB.to_string(),
                        key:          STARTUP_PHASE_GITHUB.into(),
                        started_at:   Some(Instant::now()),
                        completed_at: None,
                    }],
                    linger,
                );
                let toast_len = self.active_toasts().len();
                self.pane_manager_mut()
                    .pane_mut(PaneId::Toasts)
                    .set_len(toast_len);
            }
        }
        self.sync_running_repo_fetch_toast();
    }

    pub fn handle_repo_fetch_complete(&mut self, repo: OwnerRepo) {
        self.github.repo_fetch_in_flight.remove(&repo);
        self.github.running_fetches.remove(&repo);
        self.scan
            .scan_state_mut()
            .startup_phases
            .repo
            .seen
            .insert(repo);
        self.maybe_log_startup_phase_completions();
        self.sync_running_repo_fetch_toast();
    }

    pub fn handle_repo_meta(&mut self, path: &Path, stars: u64, description: Option<String>) {
        if let Some(entry) = self.scan.projects_mut().entry_containing_mut(path) {
            let repo = entry.git_repo.get_or_insert_with(Default::default);
            repo.github_info = Some(GitHubInfo { stars, description });
        }
    }

    pub fn handle_project_discovered(&mut self, item: RootItem) -> bool {
        let legacy_expansions = self.capture_legacy_root_expansions();
        let discovered_path = item.path().to_path_buf();
        let mut already_exists = false;
        self.projects().for_each_leaf_path(|path, _| {
            if path == discovered_path {
                already_exists = true;
            }
        });
        if already_exists {
            return false;
        }

        self.register_item_background_services(&item);
        // Insert into the hierarchy directly — under a parent workspace if
        // one exists, otherwise as a top-level peer.
        let discovered_path = item.path().to_path_buf();
        let inline_dirs = self.config.current().tui.inline_dirs.clone();
        {
            let mut tree = self.mutate_tree();
            tree.insert_into_hierarchy(item);
            tree.regroup_members(&inline_dirs);
        }
        self.register_discovery_shimmer(discovered_path.as_path());
        self.migrate_legacy_root_expansions(&legacy_expansions);
        self.rebuild_visible_rows_now();
        // Signal that derived state and caches need refresh.
        // The caller batches multiple discoveries before refreshing once.
        true
    }

    pub fn handle_project_refreshed(&mut self, item: RootItem) -> bool {
        let legacy_expansions = self.capture_legacy_root_expansions();
        let path = item.path().to_path_buf();

        // Replace the leaf in project_list_items, transferring runtime data
        // from the old item to the incoming one. `worktree_health` is
        // filesystem-detected at refresh time and must survive the info copy.
        // `worktree_status` is no longer on `ProjectInfo` — it lives directly
        // on `Workspace` / `Package` / `NonRustProject` — so this copy cannot
        // clobber it.
        let inline_dirs = self.config.current().tui.inline_dirs.clone();
        {
            let mut tree = self.mutate_tree();
            let Some(old) = tree.replace_leaf_by_path(&path, item.clone()) else {
                return false;
            };
            let mut item = item;
            for (project_path, info) in old.collect_project_info() {
                if let Some(project) = item.at_path_mut(&project_path) {
                    let fresh_worktree_health = project.worktree_health;
                    *project = info;
                    project.worktree_health = fresh_worktree_health;
                }
            }
            // Re-replace with the runtime-data-enriched version.
            tree.replace_leaf_by_path(&path, item);
            tree.regroup_members(&inline_dirs);
            tree.regroup_top_level_worktrees();
        }
        self.reload_lint_history(&path);
        self.migrate_legacy_root_expansions(&legacy_expansions);
        self.rebuild_visible_rows_now();
        self.pane_data_mut().clear_detail_data(None);
        // Signal that derived state needs refresh (batched by caller).
        true
    }

    pub fn apply_service_signal(&mut self, signal: ServiceSignal) {
        match signal {
            ServiceSignal::Reachable(service) => {
                self.handle_service_reachable(service);
            },
            ServiceSignal::Unreachable(service) => {
                self.apply_unavailability(service, AvailabilityKind::Unreachable);
            },
            ServiceSignal::RateLimited(service) => {
                self.apply_unavailability(service, AvailabilityKind::RateLimited);
            },
        }
    }

    /// A successful request is authoritative evidence the service
    /// works; treat it as recovery. Previously `Reachable` was a
    /// no-op to avoid flicker, but that left the persistent
    /// unavailability toast stuck whenever the retry probe couldn't
    /// complete (tight 1s timeout, graphql quota quirks, etc.). The
    /// recovery work fires only on the actual state transition, so
    /// steady-state success signals stay silent.
    fn handle_service_reachable(&mut self, service: ServiceKind) {
        let Some(toast_id) = self.availability_for(service).mark_reachable() else {
            return;
        };
        self.toasts.dismiss(toast_id);
        let (title, body) = service_recovered_message(service);
        self.show_timed_toast(title, body);
    }

    fn apply_unavailability(&mut self, service: ServiceKind, kind: AvailabilityKind) {
        let (spawn_retry, prior_toast) = {
            let avail = self.availability_for(service);
            let spawn_retry = match kind {
                AvailabilityKind::Unreachable => avail.mark_unreachable(),
                AvailabilityKind::RateLimited => avail.mark_rate_limited(),
            };
            (spawn_retry, avail.toast_id())
        };
        if spawn_retry {
            self.spawn_service_retry(service);
        }
        // The tracked toast id can go stale if the user dismissed the
        // toast while the service was still unavailable — the toast
        // manager evicts it after its exit animation, but the
        // `ServiceAvailability` still holds the id. Recheck aliveness
        // so the next unavailability signal re-pushes a fresh toast
        // instead of silently assuming one is visible.
        let alive = prior_toast.is_some_and(|id| self.toasts.is_alive(id));
        if !alive {
            let toast_id = self.push_service_unavailable_toast(service, kind);
            self.availability_for(service).set_toast(toast_id);
        }
    }

    const fn availability_for(&mut self, service: ServiceKind) -> &mut ServiceAvailability {
        match service {
            ServiceKind::GitHub => &mut self.github.availability,
            ServiceKind::CratesIo => &mut self.crates_io.availability,
        }
    }

    fn push_service_unavailable_toast(
        &mut self,
        service: ServiceKind,
        kind: AvailabilityKind,
    ) -> u64 {
        let (title, body) = service_unavailable_message(service, kind);
        let id = self.toasts.push_persistent(title, body, Warning, None, 1);
        let toast_len = self.active_toasts().len();
        self.pane_manager_mut()
            .pane_mut(PaneId::Toasts)
            .set_len(toast_len);
        id
    }

    pub fn spawn_service_retry(&self, service: ServiceKind) {
        #[cfg(test)]
        if !self.scan.retry_spawn_mode().is_enabled() {
            return;
        }

        let tx = self.background.bg_sender();
        let client = self.http_client.clone();
        thread::spawn(move || {
            loop {
                if client.probe_service(service) {
                    scan::emit_service_recovered(&tx, service);
                    break;
                }
                thread::sleep(Duration::from_secs(SERVICE_RETRY_SECS));
            }
        });
    }

    /// One-shot: hit GitHub's `/rate_limit` endpoint so the shared
    /// snapshot is populated before any real request. The endpoint is
    /// quota-exempt, so this is safe to run even when GitHub is
    /// refusing other calls. Logged via `rate_limit_prime_ok` /
    /// `rate_limit_prime_failed`.
    pub fn spawn_rate_limit_prime(&self) {
        let client = self.http_client.clone();
        thread::spawn(move || {
            let (snapshot, _signal) = client.fetch_rate_limit();
            if snapshot.is_some() {
                tracing::info!("rate_limit_prime_ok");
            } else {
                tracing::info!("rate_limit_prime_failed");
            }
        });
    }

    pub fn mark_service_recovered(&mut self, service: ServiceKind) {
        let Some(toast_id) = self.availability_for(service).mark_recovered() else {
            return;
        };
        self.toasts.dismiss(toast_id);
        let (title, body) = service_recovered_message(service);
        self.show_timed_toast(title, body);
    }

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
    fn update_generations_for_msg(&mut self, msg: &BackgroundMsg) {
        if let Some(path) = msg.detail_relevance()
            && self.detail_path_is_affected(path)
        {
            self.scan.bump_generation();
        }
    }

    fn handle_disk_usage_msg(&mut self, path: &Path, bytes: u64) {
        let abs = AbsolutePath::from(path);
        self.scan
            .scan_state_mut()
            .startup_phases
            .disk
            .seen
            .insert(abs.clone());
        if let Some(disk_toast) = self.scan.scan_state_mut().startup_phases.disk.toast {
            self.mark_tracked_item_completed(disk_toast, &abs.to_string());
        }
        self.handle_disk_usage(path, bytes);
        self.maybe_log_startup_phase_completions();
    }

    fn handle_disk_usage_batch_msg(
        &mut self,
        root_path: &AbsolutePath,
        entries: Vec<(AbsolutePath, DirSizes)>,
    ) {
        self.scan.bump_generation();
        self.scan
            .scan_state_mut()
            .startup_phases
            .disk
            .seen
            .insert(root_path.clone());
        if let Some(disk_toast) = self.scan.scan_state_mut().startup_phases.disk.toast {
            self.mark_tracked_item_completed(disk_toast, &root_path.to_string());
        }
        self.handle_disk_usage_batch(entries);
        self.maybe_log_startup_phase_completions();
    }

    fn handle_crates_io_version_msg(&mut self, path: &Path, version: String, downloads: u64) {
        if let Some(rust_info) = self.projects_mut().rust_info_at_path_mut(path) {
            rust_info.set_crates_io(version, downloads);
        } else if let Some(vendored) = self.projects_mut().vendored_at_path_mut(path) {
            vendored.set_crates_io(version, downloads);
        }
    }

    fn handle_lint_startup_status_msg(&mut self, path: &AbsolutePath, status: LintStatus) {
        // Apply the cached status to the project (same as a live status).
        if let Some(lr) = self.scan.projects_mut().lint_at_path_mut(path) {
            lr.set_status(status);
        }
        self.scan.scan_state_mut().startup_phases.lint_startup.seen += 1;
        self.maybe_complete_startup_lint_cache();
    }

    fn maybe_complete_startup_lint_cache(&mut self) {
        let now = Instant::now();
        if !self
            .scan
            .scan_state_mut()
            .startup_phases
            .lint_startup
            .complete_once(now)
        {
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
                seen = self.scan.scan_state().startup_phases.lint_startup.seen,
                expected = self
                    .scan
                    .scan_state()
                    .startup_phases
                    .lint_startup
                    .expected
                    .unwrap_or(0),
                "startup_phase_complete"
            );
        }
        self.maybe_log_startup_phase_completions();
    }

    fn handle_lint_status_msg(&mut self, path: &Path, status: LintStatus) {
        let abs = AbsolutePath::from(path);
        let status_started = matches!(status, LintStatus::Running(_));
        let status_is_terminal = matches!(
            status,
            LintStatus::Passed(_) | LintStatus::Failed(_) | LintStatus::Stale | LintStatus::NoLog
        );
        if !self.is_rust_at_path(path) {
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
            self.inflight.running_lint_paths_mut().remove(path);
        }
        if status_started {
            self.inflight
                .running_lint_paths_mut()
                .insert(abs, Instant::now());
        }
        if status_is_terminal {
            self.inflight.running_lint_paths_mut().remove(path);
        }
        self.sync_running_lint_toast();
        if !self.is_scan_complete() {
            return;
        }
        if status_started {
            let abs_path = AbsolutePath::from(path);
            let expected = self
                .scan
                .scan_state_mut()
                .startup_phases
                .lint
                .ensure_expected();
            if expected.insert(abs_path) {
                self.scan.scan_state_mut().startup_phases.lint.complete_at = None;
            }
        }
        if status_is_terminal {
            let abs_path = AbsolutePath::from(path);
            if self
                .scan
                .scan_state_mut()
                .startup_phases
                .lint
                .expected
                .as_ref()
                .is_some_and(|expected| expected.contains(path))
            {
                self.scan
                    .scan_state_mut()
                    .startup_phases
                    .lint
                    .seen
                    .insert(abs_path);
            }
        }
        self.maybe_log_startup_phase_completions();
    }

    fn handle_scan_result(
        &mut self,
        projects: Vec<RootItem>,
        disk_entries: &[(String, AbsolutePath)],
    ) {
        let kind = if self.scan.scan_state_mut().run_count == 1 {
            "initial"
        } else {
            "rescan"
        };

        tracing::info!(
            elapsed_ms =
                crate::perf_log::ms(self.scan.scan_state().started_at.elapsed().as_millis()),
            kind,
            run = self.scan.scan_state_mut().run_count,
            tree_items = projects.len(),
            disk_entries = disk_entries.len(),
            "scan_result_applied"
        );

        // Apply tree (same as apply_tree_build but inlined to avoid redundant
        // rebuild scheduling).
        let selected_path = self
            .selected_project_path()
            .map(AbsolutePath::from)
            .or_else(|| self.selection.paths_mut().last_selected.clone());
        self.mutate_tree().replace_all(ProjectList::new(projects));
        self.prune_inactive_project_state();
        let lint_registered = self.register_lint_for_root_items();
        self.scan
            .scan_state_mut()
            .startup_phases
            .lint_startup
            .expected = Some(lint_registered);
        self.scan.scan_state_mut().startup_phases.lint_startup.seen = 0;
        self.scan
            .scan_state_mut()
            .startup_phases
            .lint_startup
            .complete_at = None;
        self.refresh_lint_runs_from_disk();
        self.scan.bump_generation();

        // Restore selection.
        if let Some(path) = selected_path {
            self.select_project_in_tree(path.as_path());
        } else if !self.projects().is_empty() {
            self.pane_manager_mut()
                .pane_mut(PaneId::ProjectList)
                .set_pos(0);
        }
        self.sync_selected_project();

        // Register watcher for each item (same as register_item_background_services).
        self.register_background_services_for_tree();
        self.finish_watcher_registration_batch();

        // Mark scan complete and initialize startup tracking.
        self.scan.scan_state_mut().phase = ScanPhase::Complete;
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
            } => {
                self.insert_ci_runs(path.as_path(), runs, github_total);
            },
            BackgroundMsg::RepoFetchQueued { repo } => {
                self.handle_repo_fetch_queued(repo);
            },
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
                if let Some(info) = self.projects_mut().at_path_mut(path.as_path()) {
                    info.submodules = submodules;
                }
            },
            BackgroundMsg::CratesIoVersion {
                path,
                version,
                downloads,
            } => self.handle_crates_io_version_msg(path.as_path(), version, downloads),
            BackgroundMsg::RepoMeta {
                path,
                stars,
                description,
            } => self.handle_repo_meta(path.as_path(), stars, description),
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
                self.handle_language_stats_batch(entries);
            },
            BackgroundMsg::CargoMetadata {
                workspace_root,
                generation,
                fingerprint,
                result,
            } => {
                self.handle_cargo_metadata_msg(workspace_root, generation, &fingerprint, result);
            },
            BackgroundMsg::OutOfTreeTargetSize {
                workspace_root,
                target_dir,
                bytes,
            } => {
                self.handle_out_of_tree_target_size(&workspace_root, &target_dir, bytes);
            },
        }
        false
    }

    /// Merge an out-of-tree target walk result into the snapshot cache.
    /// Declines when the snapshot's `target_directory` has since been
    /// redirected — a fresh walk is already in flight under the new dir.
    fn handle_out_of_tree_target_size(
        &self,
        workspace_root: &AbsolutePath,
        target_dir: &AbsolutePath,
        bytes: u64,
    ) {
        let Ok(mut store) = self.scan.metadata_store().lock() else {
            return;
        };
        if !store.set_out_of_tree_target_bytes(workspace_root, target_dir, bytes) {
            tracing::debug!(
                workspace_root = %workspace_root.as_path().display(),
                target_dir = %target_dir.as_path().display(),
                "out_of_tree_target_size_discarded_stale"
            );
        }
    }

    fn handle_language_stats_batch(&mut self, entries: Vec<(AbsolutePath, LanguageStats)>) {
        for (path, stats) in entries {
            if let Some(project) = self.projects_mut().at_path_mut(path.as_path()) {
                project.language_stats = Some(stats);
            }
        }
    }

    fn handle_lint_cache_pruned(&mut self, runs_evicted: usize, bytes_reclaimed: u64) {
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

    /// Merge a `cargo metadata` arrival back into the process-wide store and
    /// advance the startup metadata phase. The startup path drives UI
    /// feedback via the grouped "Running cargo metadata" tracked toast
    /// created in `start_startup_detail_toasts`; post-startup per-workspace
    /// spinners land with Step 1b (watcher-triggered refresh) — until then
    /// only the startup path can arrive here.
    fn handle_cargo_metadata_msg(
        &mut self,
        workspace_root: AbsolutePath,
        generation: u64,
        fingerprint: &ManifestFingerprint,
        result: Result<WorkspaceSnapshot, CargoMetadataError>,
    ) {
        let Some(is_current) = self
            .scan
            .metadata_store()
            .lock()
            .ok()
            .map(|store| store.is_current_generation(&workspace_root, generation))
        else {
            tracing::warn!(
                workspace_root = %workspace_root.as_path().display(),
                generation,
                "cargo_metadata_store_lock_poisoned"
            );
            return;
        };
        if !is_current {
            tracing::debug!(
                workspace_root = %workspace_root.as_path().display(),
                generation,
                "cargo_metadata_msg_stale_generation"
            );
            return;
        }

        match result {
            Ok(snapshot) => {
                if !self.accept_cargo_metadata_snapshot(
                    &workspace_root,
                    generation,
                    fingerprint,
                    snapshot,
                ) {
                    return;
                }
            },
            Err(err) => match err.user_facing_message() {
                Some(message) => {
                    let label = project::home_relative_path(workspace_root.as_path());
                    self.show_timed_toast(
                        format!("cargo metadata failed ({label})"),
                        message.to_string(),
                    );
                    tracing::warn!(
                        workspace_root = %workspace_root.as_path().display(),
                        generation,
                        error = %message,
                        "cargo_metadata_failed"
                    );
                },
                None => {
                    // `WorkspaceMissing`: the workspace root vanished
                    // between dispatch and run (typically the user just
                    // deleted a worktree). Stale-refresh race, not a real
                    // failure — suppress the toast.
                    tracing::debug!(
                        workspace_root = %workspace_root.as_path().display(),
                        generation,
                        "cargo_metadata_workspace_missing"
                    );
                },
            },
        }

        if let Some(task_id) = self.scan.scan_state_mut().startup_phases.metadata.toast {
            let key = workspace_root.to_string();
            self.toasts.mark_item_completed(task_id, &key);
        }
        // Step 6e: if the user had a confirm popup waiting on this
        // workspace's re-fingerprint, clear the Verifying flag so
        // the next render shows Ready and 'y' starts working again.
        self.clear_confirm_verifying_for(&workspace_root);
        self.scan
            .scan_state_mut()
            .startup_phases
            .metadata
            .seen
            .insert(workspace_root);
        self.maybe_log_startup_phase_completions();
    }

    /// Merge a successful `cargo metadata` arrival. Returns `false` when the
    /// arrival was dropped because the captured fingerprint no longer
    /// matches what's on disk — caller should skip startup-phase bookkeeping
    /// so a later dispatch can still tick it off.
    fn accept_cargo_metadata_snapshot(
        &mut self,
        workspace_root: &AbsolutePath,
        generation: u64,
        fingerprint: &ManifestFingerprint,
        snapshot: WorkspaceSnapshot,
    ) -> bool {
        let current_fp =
            crate::project::ManifestFingerprint::capture(workspace_root.as_path()).ok();
        let fingerprint_drift = current_fp
            .as_ref()
            .is_some_and(|current| current != fingerprint);
        if fingerprint_drift {
            tracing::debug!(
                workspace_root = %workspace_root.as_path().display(),
                generation,
                "cargo_metadata_msg_fingerprint_drift"
            );
            return false;
        }
        let target_directory = snapshot.target_directory.clone();
        let member_roots = snapshot_member_roots(&snapshot);
        let needs_out_of_tree_walk = !target_directory
            .as_path()
            .starts_with(workspace_root.as_path());
        // Step 3b: stamp Cargo fields (types / examples / benches /
        // test_count / publishable) from each PackageRecord onto the
        // matching Package / Workspace / VendoredPackage in the
        // project list. Retires the hand-parsed defaults left in
        // place by `from_cargo_toml`; the authoritative view is the
        // snapshot.
        self.apply_cargo_fields_from_snapshot(&snapshot);
        if let Ok(mut store) = self.scan.metadata_store().lock() {
            store.upsert(snapshot);
        }
        if needs_out_of_tree_walk {
            scan::spawn_out_of_tree_target_walk(
                &self.http_client.handle,
                self.background.bg_sender(),
                workspace_root.clone(),
                target_directory.clone(),
            );
        }
        // Refresh the target-dir index so build_clean_plan / siblings
        // lookups see the fresh membership. Every package under this
        // workspace shares `target_directory`; upsert each so a
        // subsequent clean on any member resolves to the correct dir.
        // (Members that were in a *previous* snapshot but not this one
        // will linger until a full scan restart — minor staleness,
        // acceptable for Step 6c.)
        for project_root in member_roots {
            self.scan.target_dir_index_mut().upsert(
                TargetDirMember {
                    project_root,
                    kind: MemberKind::Project,
                },
                target_directory.clone(),
            );
        }
        tracing::info!(
            workspace_root = %workspace_root.as_path().display(),
            generation,
            "cargo_metadata_applied"
        );
        true
    }

    /// Step 3b: derive [`Cargo`] fields from every [`PackageRecord`] in
    /// `snapshot` and stamp them onto the matching live project entry
    /// (standalone package, workspace member, or vendored package).
    /// Workspaces themselves keep the empty-default `Cargo` the parser
    /// produces — they have no single `PackageRecord`; members fan out
    /// into individual packages underneath.
    fn apply_cargo_fields_from_snapshot(&mut self, snapshot: &WorkspaceSnapshot) {
        use crate::project::Cargo;
        for record in snapshot.packages.values() {
            let Some(manifest_dir) = record.manifest_path.as_path().parent() else {
                continue;
            };
            let cargo = Cargo::from_package_record(record);
            if let Some(rust_info) = self.projects_mut().rust_info_at_path_mut(manifest_dir) {
                rust_info.cargo = cargo.clone();
            }
            if let Some(vendored) = self.projects_mut().vendored_at_path_mut(manifest_dir) {
                vendored.cargo = cargo;
            }
        }
    }

    pub fn detail_path_is_affected(&self, path: &Path) -> bool {
        let Some(selected_path) = self.selected_project_path() else {
            return false;
        };
        if selected_path == path {
            return true;
        }
        // Check if both paths resolve to the same lint-owning node (e.g.,
        // a worktree group where one entry's status change affects the
        // root rollup displayed in the detail pane).
        self.projects()
            .lint_at_path(selected_path)
            .zip(self.projects().lint_at_path(path))
            .is_some_and(|(a, b)| std::ptr::eq(a, b))
    }

    /// Spawn a priority fetch for the selected project if it hasn't been loaded yet.
    pub fn maybe_priority_fetch(&mut self) {
        let Some(abs_path) = self.selected_project_path().map(Path::to_path_buf) else {
            return;
        };
        let abs_key: AbsolutePath = abs_path.clone().into();
        let display_path = self
            .selected_display_path()
            .unwrap_or_else(|| abs_key.display_path());
        let name = self
            .pane_data()
            .package()
            .map(|d| d.title_name.clone())
            .filter(|n| n != "-");
        if self
            .projects()
            .at_path(abs_key.as_path())
            .is_none_or(|p| p.disk_usage_bytes.is_none())
            && self.scan.priority_fetch_path() != Some(&abs_key)
        {
            self.scan.set_priority_fetch_path(Some(abs_key));
            let abs_str = abs_path.display().to_string();
            terminal::spawn_priority_fetch(self, display_path.as_str(), &abs_str, name.as_ref());
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AvailabilityKind {
    Unreachable,
    RateLimited,
}

const fn service_unavailable_message(
    service: ServiceKind,
    kind: AvailabilityKind,
) -> (&'static str, &'static str) {
    match (service, kind) {
        (ServiceKind::GitHub, AvailabilityKind::Unreachable) => (
            "GitHub unreachable",
            "Rate limits and CI data are unavailable until GitHub recovers.",
        ),
        (ServiceKind::GitHub, AvailabilityKind::RateLimited) => (
            "GitHub rate-limited",
            "CI data is paused until the rate-limit bucket refills.",
        ),
        (ServiceKind::CratesIo, AvailabilityKind::Unreachable) => (
            "crates.io unreachable",
            "Crate metadata is unavailable until crates.io recovers.",
        ),
        (ServiceKind::CratesIo, AvailabilityKind::RateLimited) => (
            "crates.io rate-limited",
            "Crate metadata is paused until the rate-limit bucket refills.",
        ),
    }
}

const fn service_recovered_message(service: ServiceKind) -> (&'static str, &'static str) {
    match service {
        ServiceKind::GitHub => ("GitHub available", "Back online."),
        ServiceKind::CratesIo => ("crates.io available", "Back online."),
    }
}

/// Collect publishable workspace members and vendored crates into a flat
/// `(path, crates.io name)` list for crates.io scheduling.
fn collect_publishable_children(item: &RootItem, out: &mut Vec<(AbsolutePath, String)>) {
    use crate::project::Package;
    use crate::project::RustProject;
    use crate::project::Workspace;
    use crate::project::WorktreeGroup;

    fn push_workspace(ws: &Workspace, out: &mut Vec<(AbsolutePath, String)>) {
        for group in ws.groups() {
            for member in group.members() {
                if let Some(name) = member.crates_io_name() {
                    out.push((member.path().clone(), name.to_string()));
                }
            }
        }
        for vendored in ws.vendored() {
            if let Some(name) = vendored.crates_io_name() {
                out.push((vendored.path().clone(), name.to_string()));
            }
        }
    }
    fn push_package_vendored(pkg: &Package, out: &mut Vec<(AbsolutePath, String)>) {
        for vendored in pkg.vendored() {
            if let Some(name) = vendored.crates_io_name() {
                out.push((vendored.path().clone(), name.to_string()));
            }
        }
    }

    match item {
        RootItem::Rust(RustProject::Workspace(ws)) => push_workspace(ws, out),
        RootItem::Rust(RustProject::Package(pkg)) => push_package_vendored(pkg, out),
        RootItem::Worktrees(WorktreeGroup::Workspaces {
            primary, linked, ..
        }) => {
            push_workspace(primary, out);
            for ws in linked {
                push_workspace(ws, out);
            }
        },
        RootItem::Worktrees(WorktreeGroup::Packages {
            primary, linked, ..
        }) => {
            push_package_vendored(primary, out);
            for pkg in linked {
                push_package_vendored(pkg, out);
            }
        },
        RootItem::NonRust(_) => {},
    }
}

/// Project root for each package covered by a [] —
/// derived from each package's `manifest_path.parent()`. Feeds the
/// `TargetDirIndex` membership update after a successful
/// `BackgroundMsg::CargoMetadata` arrival; every package under a given
/// workspace shares the snapshot's `target_directory`.
fn snapshot_member_roots(snapshot: &WorkspaceSnapshot) -> Vec<AbsolutePath> {
    snapshot
        .packages
        .values()
        .filter_map(|pkg| {
            pkg.manifest_path
                .as_path()
                .parent()
                .map(|parent| AbsolutePath::from(parent.to_path_buf()))
        })
        .collect()
}
