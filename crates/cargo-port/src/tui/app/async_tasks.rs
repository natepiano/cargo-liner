use std::collections::HashMap;
use std::collections::HashSet;
use std::path::Path;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;
use std::time::Instant;

use ratatui::widgets::ListState;

use super::types::App;
use super::types::ConfigFileStamp;
use super::types::DiskCacheBuildResult;
use super::types::FitWidthsBuildResult;
use super::types::PollBackgroundStats;
use super::types::ScanPhase;
use super::types::StartupPhaseTracker;
use crate::config::CargoPortConfig;
use crate::constants::SERVICE_RETRY_SECS;
use crate::http::ServiceKind;
use crate::http::ServiceSignal;
use crate::keymap::KeymapError;
use crate::keymap::KeymapErrorReason::ParseError;
use crate::lint;
use crate::lint::LintStatus;
use crate::lint::RegisterProjectRequest;
use crate::project::AbsolutePath;
use crate::project::GitInfo;
use crate::project::GitPathState;
use crate::project::RootItem;
use crate::project::Visibility::Deleted;
use crate::project::Visibility::Visible;
use crate::project_list::ProjectList;
use crate::scan;
use crate::scan::BackgroundMsg;
use crate::tui::columns::ResolvedWidths;
use crate::tui::config_reload;
use crate::tui::terminal::CiFetchMsg;
use crate::tui::terminal::CleanMsg;
use crate::tui::terminal::ExampleMsg;
use crate::tui::toasts;
use crate::tui::toasts::ToastStyle::Error;
use crate::tui::toasts::TrackedItem;
use crate::tui::types::PaneId;
use crate::watcher;
use crate::watcher::WatchRequest;

impl App {
    #[cfg(test)]
    pub(super) fn apply_tree_build(&mut self, projects: ProjectList) {
        let selected_path = self
            .selected_display_path()
            .or_else(|| self.selection_paths.last_selected.clone());
        let should_focus_project_list = false;
        self.projects = projects;
        self.dirty.finder.mark_dirty();
        self.dirty.rows.mark_dirty();
        self.dirty.disk_cache.mark_dirty();
        self.dirty.fit_widths.mark_dirty();
        self.recompute_cargo_active_paths();
        self.prune_inactive_project_state();
        self.register_lint_for_root_items();
        self.rebuild_lint_rollups();
        self.data_generation += 1;
        self.detail_generation += 1;

        // Re-run search if active so filtered results match the latest
        // hierarchy state.
        if self.is_searching() && !self.search_query.is_empty() {
            let query = self.search_query.clone();
            self.update_search(&query);
        } else {
            self.filtered.clear();
        }

        // Propagate git info and stars from workspace roots to their members.
        let mut inherited_git_info = Vec::new();
        for item in &self.projects {
            let root_path = item.path();
            let member_paths: Vec<PathBuf> = match item {
                crate::project::RootItem::Workspace(ws) => ws
                    .groups()
                    .iter()
                    .flat_map(|g| g.members().iter().map(|m| m.path().to_path_buf()))
                    .collect(),
                crate::project::RootItem::WorkspaceWorktrees(wtg) => {
                    std::iter::once(wtg.primary())
                        .chain(wtg.linked().iter())
                        .flat_map(|ws| {
                            ws.groups()
                                .iter()
                                .flat_map(|g| g.members().iter().map(|m| m.path().to_path_buf()))
                        })
                        .collect()
                },
                _ => Vec::new(),
            };
            if let Some(info) = self
                .projects
                .at_path(root_path)
                .and_then(|project| project.git_info.clone())
            {
                for member_path in &member_paths {
                    if self
                        .projects
                        .at_path(member_path)
                        .is_none_or(|project| project.git_info.is_none())
                    {
                        inherited_git_info.push((member_path.clone(), info.clone()));
                    }
                }
            }
            if let Some(&stars) = self.stars.get(root_path) {
                for member_path in &member_paths {
                    self.stars.entry(member_path.clone()).or_insert(stars);
                }
            }
        }
        for (member_path, info) in inherited_git_info {
            if let Some(project) = self.projects.at_path_mut(&member_path) {
                project.git_info = Some(info);
            }
        }

        // Try to restore selection
        if let Some(path) = selected_path {
            self.select_project_in_tree(&path);
        } else if !self.projects.is_empty() {
            self.list_state.select(Some(0));
        }
        if should_focus_project_list {
            self.focus_pane(PaneId::ProjectList);
        }
        self.sync_selected_project();
    }

    pub(super) fn config_file_stamp(path: &Path) -> Option<ConfigFileStamp> {
        let metadata = std::fs::metadata(path).ok()?;
        Some(ConfigFileStamp {
            modified: metadata.modified().ok(),
            len:      metadata.len(),
        })
    }

    pub(super) fn sync_config_watch_state(&mut self) {
        self.config_last_seen = self
            .config_path
            .as_deref()
            .and_then(Self::config_file_stamp);
    }

    pub(super) fn record_config_reload_failure(&mut self, err: &str) {
        self.status_flash = Some((
            "Config reload failed; keeping previous settings".to_string(),
            Instant::now(),
        ));
        self.show_timed_toast("Config reload failed", err.to_string());
    }

    pub fn load_initial_keymap(&mut self) {
        let vim_mode = self.current_config.tui.navigation_keys;
        let result = crate::keymap::load_keymap(vim_mode);
        self.current_keymap = result.keymap;
        self.sync_keymap_watch_state();
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
        let current_stamp = self
            .keymap_path
            .as_deref()
            .and_then(Self::config_file_stamp);
        if current_stamp == self.keymap_last_seen {
            return;
        }
        self.keymap_last_seen = current_stamp;

        let Some(path) = &self.keymap_path else {
            return;
        };
        let contents = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) => {
                self.show_keymap_diagnostics(&[crate::keymap::KeymapError {
                    scope:  String::new(),
                    action: String::new(),
                    key:    String::new(),
                    reason: ParseError(format!("read error: {e}")),
                }]);
                return;
            },
        };

        let vim_mode = self.current_config.tui.navigation_keys;
        let result = crate::keymap::load_keymap_from_str(&contents, vim_mode);
        self.current_keymap = result.keymap;

        if result.errors.is_empty() {
            self.dismiss_keymap_diagnostics();
        } else {
            self.show_keymap_diagnostics(&result.errors);
        }

        if !result.missing_actions.is_empty() {
            if let Some(path) = &self.keymap_path {
                let content =
                    crate::keymap::ResolvedKeymap::default_toml_from(&self.current_keymap);
                let _ = std::fs::write(path, content);
                self.sync_keymap_watch_state();
            }
            self.show_timed_toast(
                "Keymap updated",
                format!(
                    "Defaults written for missing entries:\n{}",
                    result.missing_actions.join(", ")
                ),
            );
        }
    }

    pub fn sync_keymap_stamp(&mut self) { self.sync_keymap_watch_state(); }

    fn sync_keymap_watch_state(&mut self) {
        self.keymap_last_seen = self
            .keymap_path
            .as_deref()
            .and_then(Self::config_file_stamp);
    }

    fn show_keymap_diagnostics(&mut self, errors: &[KeymapError]) {
        // Dismiss previous diagnostics toast if any.
        self.dismiss_keymap_diagnostics();

        let body = errors
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        let action_path = self.keymap_path.clone();

        let id = self.toasts.push_persistent(
            "Keymap errors (using defaults)",
            body,
            Error,
            action_path,
            1,
        );
        self.keymap_diagnostics_id = Some(id);
        self.toast_pane.set_len(self.active_toasts().len());
    }

    fn dismiss_keymap_diagnostics(&mut self) {
        if let Some(id) = self.keymap_diagnostics_id.take() {
            self.toasts.dismiss(id);
        }
    }

    pub fn maybe_reload_config_from_disk(&mut self) {
        let current_stamp = self
            .config_path
            .as_deref()
            .and_then(Self::config_file_stamp);
        if current_stamp == self.config_last_seen {
            return;
        }

        self.config_last_seen = current_stamp;
        let reload_result = self
            .config_path
            .as_deref()
            .map_or_else(crate::config::try_load, crate::config::try_load_from_path);
        match reload_result {
            Ok(cfg) => {
                self.apply_config(&cfg);
                self.sync_config_watch_state();
                self.show_timed_toast("Settings", "Reloaded from disk");
            },
            Err(err) => self.record_config_reload_failure(&err),
        }
    }

    pub fn save_and_apply_config(&mut self, cfg: &CargoPortConfig) -> Result<(), String> {
        crate::config::save(cfg)?;
        self.apply_config(cfg);
        self.sync_config_watch_state();
        Ok(())
    }

    pub(super) fn apply_config(&mut self, cfg: &CargoPortConfig) {
        if self.current_config == *cfg {
            return;
        }

        let actions = config_reload::collect_reload_actions(
            &self.current_config,
            cfg,
            config_reload::ReloadContext {
                scan_complete:       self.is_scan_complete(),
                has_cached_non_rust: self.has_cached_non_rust_projects(),
            },
        );
        crate::config::set_active_config(cfg);
        self.current_config = cfg.clone();

        if actions.refresh_lint_runtime {
            self.refresh_lint_runtime_from_config(cfg);
        }

        if actions.rescan {
            self.rescan();
        } else {
            if actions.refresh_lint_runtime {
                self.respawn_watcher();
            }
            if actions.rebuild_tree {
                // Regroup workspace members in-place based on updated
                // inline_dirs, then refresh derived state.
                self.projects
                    .regroup_members(&self.current_config.tui.inline_dirs);
                self.refresh_derived_state();
            }
        }
    }

    pub(super) fn refresh_lint_runtime_from_config(&mut self, cfg: &CargoPortConfig) {
        let lint_spawn = lint::spawn(cfg, self.bg_tx.clone());
        self.lint_runtime = lint_spawn.handle;
        self.lint_status.clear();
        self.running_lint_paths.clear();
        self.sync_running_lint_toast();
        self.register_existing_projects();
        self.sync_lint_runtime_projects();
        self.refresh_lint_runs_from_disk();
        self.rebuild_lint_rollups();
        self.cached_fit_widths = ResolvedWidths::new(self.lint_enabled());
        self.dirty.rows.mark_dirty();
        self.dirty.fit_widths.mark_dirty();
        self.data_generation += 1;
        self.detail_generation += 1;
        if let Some(warning) = lint_spawn.warning {
            self.status_flash = Some((warning.clone(), Instant::now()));
            self.show_timed_toast("Lint runtime", warning);
        }
    }

    pub(super) fn respawn_watcher(&mut self) {
        self.watch_tx = watcher::spawn_watcher(
            self.scan_root.clone(),
            self.bg_tx.clone(),
            self.ci_run_count(),
            self.include_non_rust(),
            self.current_config.tui.include_dirs.clone(),
            self.http_client.clone(),
        );
    }

    pub(super) fn register_existing_projects(&self) {
        self.projects.for_each_leaf(|item| {
            self.register_item_background_services(item);
        });
    }

    pub(super) fn refresh_lint_runs_from_disk(&mut self) {
        self.lint_runs.clear();
        let paths: Vec<PathBuf> = {
            let mut v = Vec::new();
            self.projects.for_each_leaf_path(|path, _| {
                v.push(path.to_path_buf());
            });
            v
        };
        for path in &paths {
            if !self.is_cargo_active_path(path) {
                continue;
            }
            let runs = crate::lint::read_history(path);
            if !runs.is_empty() {
                self.lint_runs.insert(path.clone(), runs);
            }
        }
        self.refresh_lint_cache_usage_from_disk();
    }

    pub(super) fn reload_lint_history(&mut self, project_path: &Path) {
        let mut found = false;
        self.projects.for_each_leaf_path(|path, _| {
            if path == project_path {
                found = true;
            }
        });
        if !found {
            self.lint_runs.remove(project_path);
            return;
        }
        if !self.is_cargo_active_path(project_path) {
            self.lint_runs.remove(project_path);
            return;
        }
        let runs = crate::lint::read_history(project_path);
        if runs.is_empty() {
            self.lint_runs.remove(project_path);
        } else {
            self.lint_runs.insert(project_path.to_path_buf(), runs);
        }
        self.refresh_lint_cache_usage_from_disk();
    }

    pub fn refresh_lint_cache_usage_from_disk(&mut self) {
        let cache_size_bytes = self.current_config.lint.cache_size_bytes().unwrap_or(None);
        self.lint_cache_usage = crate::lint::retained_cache_usage(cache_size_bytes);
    }

    /// Register file-system watchers for every item in the tree after a
    /// single-pass scan delivers the complete tree.
    fn register_background_services_for_tree(&self) {
        let started = Instant::now();
        let mut count = 0usize;
        for item in &self.projects {
            self.register_item_background_services(item);
            count += 1;
        }
        tracing::info!(
            elapsed_ms = crate::perf_log::ms(started.elapsed().as_millis()),
            count,
            "register_background_services_for_tree"
        );
    }

    pub(super) fn register_item_background_services(&self, item: &RootItem) {
        let started = Instant::now();
        let abs_path = item.path().to_path_buf();
        let repo_root = crate::project::git_repo_root(&abs_path);
        let has_repo_root = repo_root.is_some();
        let _ = self.watch_tx.send(WatchRequest {
            project_path: abs_path.to_string_lossy().to_string(),
            abs_path: abs_path.clone(),
            repo_root,
        });
        tracing::info!(
            elapsed_ms = crate::perf_log::ms(started.elapsed().as_millis()),
            path = %item.display_path(),
            has_repo_root,
            "app_register_project_background_services"
        );
    }

    pub(super) fn schedule_git_path_state_refreshes(&self) {
        let tx = self.bg_tx.clone();
        let mut projects: Vec<(String, String)> = Vec::new();
        self.projects.for_each_leaf_path(|path, _| {
            let abs = path.to_string_lossy().to_string();
            projects.push((abs.clone(), abs));
        });
        std::thread::spawn(move || {
            let states = crate::project::detect_git_path_states_batch(&projects);
            for (path, state) in states {
                let _ = tx.send(BackgroundMsg::GitPathState {
                    path: PathBuf::from(path).into(),
                    state,
                });
            }
        });
    }

    pub(super) fn schedule_git_first_commit_refreshes(&self) {
        let tx = self.bg_tx.clone();
        let mut projects_by_repo: HashMap<PathBuf, Vec<PathBuf>> = HashMap::new();
        self.projects.for_each_leaf_path(|path, _| {
            let abs_path = path.to_path_buf();
            let Some(repo_root) = crate::project::git_repo_root(&abs_path) else {
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
                let first_commit = crate::project::detect_first_commit(&repo_root);
                tracing::info!(
                    elapsed_ms = crate::perf_log::ms(started.elapsed().as_millis()),
                    repo_root = %repo_root.display(),
                    rows = paths.len(),
                    found = first_commit.is_some(),
                    "git_first_commit_fetch"
                );
                for path in paths {
                    let _ = tx.send(BackgroundMsg::GitFirstCommit {
                        path:         path.into(),
                        first_commit: first_commit.clone(),
                    });
                }
            }
        });
    }

    /// Collect root project paths and metadata for the lint runtime.
    fn lint_runtime_root_entries(&self) -> Vec<(PathBuf, bool)> {
        let mut seen = HashSet::new();
        let mut entries = Vec::new();

        for item in &self.projects {
            let items: Vec<(&Path, bool)> = match item {
                crate::project::RootItem::WorkspaceWorktrees(wtg) => {
                    std::iter::once(wtg.primary())
                        .chain(wtg.linked().iter())
                        .map(|p| (p.path(), true))
                        .collect()
                },
                crate::project::RootItem::PackageWorktrees(wtg) => {
                    std::iter::once(wtg.primary())
                        .chain(wtg.linked().iter())
                        .map(|p| (p.path(), true))
                        .collect()
                },
                _ => vec![(item.path(), item.is_rust())],
            };
            for (path, is_rust) in items {
                let owned = path.to_path_buf();
                if seen.insert(owned.clone()) {
                    entries.push((owned, is_rust));
                }
            }
        }

        entries
    }

    pub(super) fn lint_runtime_projects_snapshot(&self) -> Vec<RegisterProjectRequest> {
        if !self.is_scan_complete() {
            return Vec::new();
        }
        self.lint_runtime_root_entries()
            .into_iter()
            .filter(|(path, _)| !self.is_deleted(path) && self.is_cargo_active_path(path))
            .map(|(abs_path, is_rust)| RegisterProjectRequest {
                project_path: abs_path.display().to_string(),
                abs_path,
                is_rust,
            })
            .collect()
    }

    pub(super) fn sync_lint_runtime_projects(&self) {
        let Some(runtime) = &self.lint_runtime else {
            return;
        };
        runtime.sync_projects(self.lint_runtime_projects_snapshot());
    }

    fn register_lint_for_root_items(&self) {
        let Some(runtime) = &self.lint_runtime else {
            return;
        };
        let mut count = 0;
        for item in &self.projects {
            match item {
                RootItem::Workspace(ws) => {
                    runtime.register_project(crate::lint::RegisterProjectRequest {
                        project_path: ws.display_path(),
                        abs_path:     ws.path().to_path_buf(),
                        is_rust:      true,
                    });
                    count += 1;
                },
                RootItem::Package(pkg) => {
                    runtime.register_project(crate::lint::RegisterProjectRequest {
                        project_path: pkg.display_path(),
                        abs_path:     pkg.path().to_path_buf(),
                        is_rust:      true,
                    });
                    count += 1;
                },
                RootItem::WorkspaceWorktrees(wtg) => {
                    for ws in std::iter::once(wtg.primary()).chain(wtg.linked().iter()) {
                        runtime.register_project(crate::lint::RegisterProjectRequest {
                            project_path: ws.display_path(),
                            abs_path:     ws.path().to_path_buf(),
                            is_rust:      true,
                        });
                        count += 1;
                    }
                },
                RootItem::PackageWorktrees(wtg) => {
                    for pkg in std::iter::once(wtg.primary()).chain(wtg.linked().iter()) {
                        runtime.register_project(crate::lint::RegisterProjectRequest {
                            project_path: pkg.display_path(),
                            abs_path:     pkg.path().to_path_buf(),
                            is_rust:      true,
                        });
                        count += 1;
                    }
                },
                RootItem::NonRust(_) => {},
            }
        }
        tracing::info!(count, "lint_register_root_items");
    }

    fn register_lint_project_if_eligible(&self, item: &RootItem) {
        if !item.is_rust() {
            tracing::info!(reason = "not_rust", path = %item.display_path(), "lint_register_skip");
            return;
        }
        let path = item.path();
        // Skip workspace members — the workspace root's watcher covers them.
        let mut is_member = false;
        self.projects.for_each_leaf(|existing| {
            if matches!(existing, RootItem::Workspace(_))
                && existing.path() != path
                && path.starts_with(existing.path())
            {
                is_member = true;
            }
        });
        if is_member {
            tracing::info!(reason = "workspace_member", path = %item.display_path(), "lint_register_skip");
            return;
        }
        let Some(runtime) = &self.lint_runtime else {
            tracing::info!(reason = "no_runtime", path = %item.display_path(), "lint_register_skip");
            return;
        };
        tracing::info!(path = %item.display_path(), "lint_register");
        runtime.register_project(crate::lint::RegisterProjectRequest {
            project_path: item.display_path(),
            abs_path:     path.to_path_buf(),
            is_rust:      true,
        });
    }

    fn register_lint_for_path(&self, display_path: &str) {
        if let Some(item) = self
            .projects
            .iter()
            .find(|i| i.display_path() == display_path)
        {
            self.register_lint_project_if_eligible(item);
        }
    }

    pub(super) fn initialize_startup_phase_tracker(&mut self) {
        let disk_expected = super::snapshots::initial_disk_batch_count(&self.projects);
        let git_seen = self
            .scan
            .startup_phases
            .git_expected
            .iter()
            .filter(|path| self.git_info_for(path).is_some())
            .cloned()
            .collect::<HashSet<_>>();
        self.scan.startup_phases.disk_complete_at = None;
        self.scan.startup_phases.scan_complete_at = Some(Instant::now());
        self.scan.startup_phases.disk_expected = Some(disk_expected);
        self.scan.startup_phases.git_seen = git_seen;
        self.scan.startup_phases.git_complete_at = None;
        self.scan.startup_phases.repo_complete_at = None;
        self.scan.startup_phases.git_toast = None;
        self.scan.startup_phases.repo_toast = None;
        self.scan.startup_phases.lint_expected = Some(HashSet::new());
        self.scan.startup_phases.lint_seen_terminal.clear();
        self.scan.startup_phases.lint_complete_at = None;
        self.scan.startup_phases.startup_complete_at = None;
        let git_items = Self::tracked_items_for_startup(
            &self.scan.startup_phases.git_expected,
            &self.scan.startup_phases.git_seen,
        );
        if !git_items.is_empty() {
            let body = self.startup_git_toast_body();
            let task_id = self.start_task_toast("Scanning local git repos", &body);
            self.set_task_tracked_items(task_id, &git_items);
            self.scan.startup_phases.git_toast = Some(task_id);
        }
        let repo_items = Self::tracked_items_for_startup(
            &self.scan.startup_phases.repo_expected,
            &self.scan.startup_phases.repo_seen,
        );
        if !repo_items.is_empty() {
            let body = self.startup_repo_toast_body();
            let task_id = self.start_task_toast("Retrieving GitHub repo details", &body);
            self.set_task_tracked_items(task_id, &repo_items);
            self.scan.startup_phases.repo_toast = Some(task_id);
        }
        tracing::info!(
            disk_expected = self.scan.startup_phases.disk_expected.unwrap_or(0),
            git_expected = self.scan.startup_phases.git_expected.len(),
            repo_expected = self.scan.startup_phases.repo_expected.len(),
            lint_expected = self
                .scan
                .startup_phases
                .lint_expected
                .as_ref()
                .map_or(0, HashSet::len),
            "startup_phase_plan"
        );
        self.maybe_log_startup_phase_completions();
    }

    pub(super) fn maybe_log_startup_phase_completions(&mut self) {
        let Some(scan_complete_at) = self.scan.startup_phases.scan_complete_at else {
            return;
        };
        let now = Instant::now();
        self.maybe_complete_startup_disk(now, scan_complete_at);
        self.maybe_complete_startup_git(now, scan_complete_at);
        self.maybe_complete_startup_repo(now, scan_complete_at);
        self.maybe_complete_startup_lints(now, scan_complete_at);
        self.maybe_complete_startup_ready(now, scan_complete_at);
    }

    pub(super) fn maybe_complete_startup_disk(&mut self, now: Instant, scan_complete_at: Instant) {
        if self.scan.startup_phases.disk_complete_at.is_none()
            && self
                .scan
                .startup_phases
                .disk_expected
                .is_some_and(|expected| self.scan.startup_phases.disk_seen.len() >= expected)
        {
            self.scan.startup_phases.disk_complete_at = Some(now);
            tracing::info!(
                phase = "disk_applied",
                since_scan_complete_ms =
                    crate::perf_log::ms(now.duration_since(scan_complete_at).as_millis()),
                seen = self.scan.startup_phases.disk_seen.len(),
                expected = self.scan.startup_phases.disk_expected.unwrap_or(0),
                "startup_phase_complete"
            );
        }
    }

    pub(super) fn maybe_complete_startup_git(&mut self, now: Instant, scan_complete_at: Instant) {
        if self.scan.startup_phases.git_complete_at.is_none()
            && self.scan.startup_phases.git_seen.len()
                >= self.scan.startup_phases.git_expected.len()
        {
            self.scan.startup_phases.git_complete_at = Some(now);
            if let Some(git_toast) = self.scan.startup_phases.git_toast.take() {
                self.finish_task_toast(git_toast);
            }
            tracing::info!(
                phase = "git_local_applied",
                since_scan_complete_ms =
                    crate::perf_log::ms(now.duration_since(scan_complete_at).as_millis()),
                seen = self.scan.startup_phases.git_seen.len(),
                expected = self.scan.startup_phases.git_expected.len(),
                "startup_phase_complete"
            );
        }
    }

    pub(super) fn maybe_complete_startup_repo(&mut self, now: Instant, scan_complete_at: Instant) {
        if self.scan.startup_phases.repo_complete_at.is_none()
            && self.scan.startup_phases.repo_seen.len()
                >= self.scan.startup_phases.repo_expected.len()
        {
            self.scan.startup_phases.repo_complete_at = Some(now);
            if let Some(repo_toast) = self.scan.startup_phases.repo_toast.take() {
                self.finish_task_toast(repo_toast);
            }
            tracing::info!(
                phase = "repo_fetch_applied",
                since_scan_complete_ms =
                    crate::perf_log::ms(now.duration_since(scan_complete_at).as_millis()),
                seen = self.scan.startup_phases.repo_seen.len(),
                expected = self.scan.startup_phases.repo_expected.len(),
                "startup_phase_complete"
            );
        }
    }

    pub(super) fn maybe_complete_startup_lints(&mut self, now: Instant, scan_complete_at: Instant) {
        if self.scan.startup_phases.lint_complete_at.is_none()
            && self
                .scan
                .startup_phases
                .lint_expected
                .as_ref()
                .is_some_and(|expected| {
                    !expected.is_empty()
                        && self.scan.startup_phases.lint_seen_terminal.len() >= expected.len()
                })
        {
            self.scan.startup_phases.lint_complete_at = Some(now);
            tracing::info!(
                phase = "lint_terminal_applied",
                since_scan_complete_ms =
                    crate::perf_log::ms(now.duration_since(scan_complete_at).as_millis()),
                seen = self.scan.startup_phases.lint_seen_terminal.len(),
                expected = self
                    .scan
                    .startup_phases
                    .lint_expected
                    .as_ref()
                    .map_or(0, HashSet::len),
                "startup_phase_complete"
            );
        }
    }

    pub(super) fn maybe_complete_startup_ready(&mut self, now: Instant, scan_complete_at: Instant) {
        if self.scan.startup_phases.startup_complete_at.is_none() {
            let disk_ready = self.scan.startup_phases.disk_complete_at.is_some();
            let git_ready = self.scan.startup_phases.git_complete_at.is_some();
            let repo_ready = self.scan.startup_phases.repo_complete_at.is_some();
            if disk_ready && git_ready && repo_ready {
                self.scan.startup_phases.startup_complete_at = Some(now);
                self.show_timed_toast(
                    "Startup complete",
                    "Disk, local Git, and GitHub startup activity complete.".to_string(),
                );
                tracing::info!(
                    since_scan_complete_ms =
                        crate::perf_log::ms(now.duration_since(scan_complete_at).as_millis()),
                    disk_seen = self.scan.startup_phases.disk_seen.len(),
                    disk_expected = self.scan.startup_phases.disk_expected.unwrap_or(0),
                    git_seen = self.scan.startup_phases.git_seen.len(),
                    git_expected = self.scan.startup_phases.git_expected.len(),
                    repo_seen = self.scan.startup_phases.repo_seen.len(),
                    repo_expected = self.scan.startup_phases.repo_expected.len(),
                    lint_seen = self.scan.startup_phases.lint_seen_terminal.len(),
                    lint_expected = self
                        .scan
                        .startup_phases
                        .lint_expected
                        .as_ref()
                        .map_or(0, HashSet::len),
                    "startup_complete"
                );
                tracing::info!(
                    since_scan_complete_ms =
                        crate::perf_log::ms(now.duration_since(scan_complete_at).as_millis()),
                    "steady_state_begin"
                );
            }
        }
    }

    pub(super) fn startup_git_toast_body(&self) -> String {
        Self::startup_remaining_toast_body(
            &self.scan.startup_phases.git_expected,
            &self.scan.startup_phases.git_seen,
        )
    }

    pub(super) fn startup_repo_toast_body(&self) -> String {
        Self::startup_remaining_toast_body(
            &self.scan.startup_phases.repo_expected,
            &self.scan.startup_phases.repo_seen,
        )
    }

    /// Build tracked items from expected/seen path sets. Already-seen paths
    /// are pre-marked as completed so the renderer shows them with strikethrough.
    pub(super) fn tracked_items_for_startup(
        expected: &HashSet<PathBuf>,
        seen: &HashSet<PathBuf>,
    ) -> Vec<TrackedItem> {
        expected
            .iter()
            .map(|path| {
                let label = crate::project::home_relative_path(path);
                let completed_at = if seen.contains(path) {
                    Some(Instant::now())
                } else {
                    None
                };
                TrackedItem {
                    label,
                    key: AbsolutePath::from(path.as_path()),
                    started_at: None,
                    completed_at,
                }
            })
            .collect()
    }

    pub(super) fn startup_remaining_toast_body(
        expected: &HashSet<PathBuf>,
        seen: &HashSet<PathBuf>,
    ) -> String {
        let items: Vec<String> = expected
            .iter()
            .filter(|path| !seen.contains(*path))
            .map(|p| crate::project::home_relative_path(p))
            .collect();
        let refs: Vec<&str> = items.iter().map(String::as_str).collect();
        if refs.is_empty() {
            return "Complete".to_string();
        }
        toasts::format_toast_items(&refs, toasts::toast_body_width())
    }

    pub(super) fn startup_lint_toast_body_for(
        expected: &HashSet<PathBuf>,
        seen: &HashSet<PathBuf>,
    ) -> String {
        let items: Vec<String> = expected
            .iter()
            .filter(|path| !seen.contains(*path))
            .map(|p| crate::project::home_relative_path(p))
            .collect();
        let refs: Vec<&str> = items.iter().map(String::as_str).collect();
        if refs.is_empty() {
            return "Complete".to_string();
        }
        toasts::format_toast_items(&refs, toasts::toast_body_width())
    }

    pub(super) fn running_lint_toast_body(&self) -> String {
        let paths: HashSet<PathBuf> = self.running_lint_paths.keys().cloned().collect();
        Self::startup_lint_toast_body_for(&paths, &HashSet::new())
    }

    pub(super) fn sync_running_clean_toast(&mut self) {
        if self.running_clean_paths.is_empty() {
            if let Some(task_id) = self.clean_toast.take() {
                self.finish_task_toast(task_id);
            }
            return;
        }

        let items: Vec<TrackedItem> = self
            .running_clean_paths
            .iter()
            .map(|p| TrackedItem {
                label:        crate::project::home_relative_path(p),
                key:          AbsolutePath::from(p.as_path()),
                started_at:   None,
                completed_at: None,
            })
            .collect();
        let body = self.running_clean_toast_body();
        if let Some(task_id) = self.clean_toast {
            self.set_task_tracked_items(task_id, &items);
        } else {
            let task_id = self.start_task_toast("cargo clean", body);
            self.set_task_tracked_items(task_id, &items);
            self.clean_toast = Some(task_id);
        }
    }

    fn running_clean_toast_body(&self) -> String {
        let items: Vec<String> = self
            .running_clean_paths
            .iter()
            .map(|p| crate::project::home_relative_path(p))
            .collect();
        let refs: Vec<&str> = items.iter().map(String::as_str).collect();
        crate::tui::toasts::format_toast_items(&refs, crate::tui::toasts::toast_body_width())
    }

    pub(super) fn sync_running_lint_toast(&mut self) {
        if self.running_lint_paths.is_empty() {
            if let Some(task_id) = self.lint_toast {
                // Mark all remaining tracked items as completed (starts fade).
                let empty: HashSet<String> = HashSet::new();
                self.toasts.complete_missing_items(task_id, &empty);
                // Start countdown only once (finish_task is idempotent check via finished_task
                // flag).
                if !self.toasts.is_task_finished(task_id) {
                    let linger = Duration::from_secs_f64(self.current_config.tui.task_linger_secs);
                    self.toasts.finish_task(task_id, linger);
                }
            }
            return;
        }

        let running_items: Vec<TrackedItem> = self
            .running_lint_paths
            .iter()
            .map(|(p, &started)| TrackedItem {
                label:        crate::project::home_relative_path(p),
                key:          AbsolutePath::from(p.as_path()),
                started_at:   Some(started),
                completed_at: None,
            })
            .collect();
        let running_keys: HashSet<String> = running_items
            .iter()
            .map(|item| item.key.to_string())
            .collect();

        if let Some(task_id) = self.lint_toast
            && self.toasts.reactivate_task(task_id)
        {
            // Mark items no longer running as completed.
            self.toasts.complete_missing_items(task_id, &running_keys);
            // Add new items that aren't already tracked.
            let linger = Duration::from_secs_f64(self.current_config.tui.task_linger_secs);
            self.toasts
                .add_new_tracked_items(task_id, &running_items, linger);
        } else {
            let items = running_items;
            let body = self.running_lint_toast_body();
            let task_id = self.start_task_toast("Lints", body);
            self.set_task_tracked_items(task_id, &items);
            self.lint_toast = Some(task_id);
        }
    }

    pub(super) fn request_fit_widths_build(&mut self) {
        if !self.dirty.fit_widths.is_dirty() {
            return;
        }
        self.builds.fit.latest = self.builds.fit.latest.wrapping_add(1);
        if self.builds.fit.active.is_some() {
            return;
        }
        self.spawn_fit_widths_build(self.builds.fit.latest);
    }

    pub(super) fn spawn_fit_widths_build(&mut self, build_id: u64) {
        let tx = self.builds.fit.tx.clone();
        let items = self.projects.clone();
        let git_path_states = self.git_path_states.clone();
        let lint_enabled = self.lint_enabled();
        self.builds.fit.active = Some(build_id);
        std::thread::spawn(move || {
            let started = Instant::now();
            let state = super::snapshots::FitWidthsState {
                git_path_states: &git_path_states,
            };
            let widths =
                super::snapshots::build_fit_widths_snapshot(&items, &state, lint_enabled, build_id);
            let elapsed = started.elapsed();
            if elapsed.as_millis() >= crate::perf_log::SLOW_WORKER_MS {
                tracing::info!(
                    elapsed_ms = crate::perf_log::ms(elapsed.as_millis()),
                    build_id,
                    items = items.len(),
                    "fit_widths_build"
                );
            }
            let _ = tx.send(FitWidthsBuildResult { build_id, widths });
        });
    }

    pub(super) fn poll_fit_width_builds(&mut self) -> usize {
        let mut applied = 0;
        while let Ok(result) = self.builds.fit.rx.try_recv() {
            if self.builds.fit.active != Some(result.build_id) {
                continue;
            }
            self.builds.fit.active = None;
            self.cached_fit_widths = result.widths;
            applied += 1;
            if result.build_id == self.builds.fit.latest {
                self.dirty.fit_widths.mark_clean();
            } else {
                self.spawn_fit_widths_build(self.builds.fit.latest);
            }
        }
        applied
    }

    pub(super) fn request_disk_cache_build(&mut self) {
        if !self.dirty.disk_cache.is_dirty() {
            return;
        }
        self.builds.disk.latest = self.builds.disk.latest.wrapping_add(1);
        if self.builds.disk.active.is_some() {
            return;
        }
        self.spawn_disk_cache_build(self.builds.disk.latest);
    }

    pub(super) fn spawn_disk_cache_build(&mut self, build_id: u64) {
        let tx = self.builds.disk.tx.clone();
        let items = self.projects.clone();
        self.builds.disk.active = Some(build_id);
        std::thread::spawn(move || {
            let started = Instant::now();
            let (root_sorted, child_sorted) = super::snapshots::build_disk_cache_snapshot(&items);
            let elapsed = started.elapsed();
            if elapsed.as_millis() >= crate::perf_log::SLOW_WORKER_MS {
                tracing::info!(
                    elapsed_ms = crate::perf_log::ms(elapsed.as_millis()),
                    build_id,
                    items = items.len(),
                    root_values = root_sorted.len(),
                    child_sets = child_sorted.len(),
                    "disk_cache_build"
                );
            }
            let _ = tx.send(DiskCacheBuildResult {
                build_id,
                root_sorted,
                child_sorted,
            });
        });
    }

    pub(super) fn poll_disk_cache_builds(&mut self) -> usize {
        let mut applied = 0;
        while let Ok(result) = self.builds.disk.rx.try_recv() {
            if self.builds.disk.active != Some(result.build_id) {
                continue;
            }
            self.builds.disk.active = None;
            self.cached_root_sorted = result.root_sorted;
            self.cached_child_sorted = result.child_sorted;
            applied += 1;
            if result.build_id == self.builds.disk.latest {
                self.dirty.disk_cache.mark_clean();
            } else {
                self.spawn_disk_cache_build(self.builds.disk.latest);
            }
        }
        applied
    }

    /// Lightweight refresh of derived state after in-place hierarchy changes
    /// (discovery, refresh). Marks caches dirty without a full tree rebuild.
    pub(super) fn refresh_derived_state(&mut self) {
        self.recompute_cargo_active_paths();
        self.data_generation += 1;
        self.detail_generation += 1;
        self.dirty.finder.mark_dirty();
        self.dirty.rows.mark_dirty();
        self.dirty.disk_cache.mark_dirty();
        self.dirty.fit_widths.mark_dirty();
        if self.is_searching() && !self.search_query.is_empty() {
            let query = self.search_query.clone();
            self.update_search(&query);
        }
    }

    pub(super) fn refresh_async_caches(&mut self) {
        self.request_disk_cache_build();
        self.request_fit_widths_build();
    }

    pub fn rescan(&mut self) {
        self.projects.clear();
        // disk_usage lives on project items — cleared with projects above
        self.ci_state.clear();
        self.lint_status.clear();
        self.lint_cache_usage = crate::lint::CacheUsage::default();
        self.lint_runs.clear();
        self.git_path_states.clear();
        self.cargo_active_paths.clear();
        self.crates_versions.clear();
        self.crates_downloads.clear();
        self.stars.clear();
        self.repo_descriptions.clear();
        self.scan.phase = ScanPhase::Running;
        self.scan.started_at = Instant::now();
        self.scan.run_count += 1;
        self.scan.startup_phases = StartupPhaseTracker::default();
        tracing::info!(kind = "rescan", run = self.scan.run_count, "scan_start");
        self.fully_loaded.clear();
        self.priority_fetch_path = None;
        self.focus_pane(PaneId::ProjectList);
        self.close_settings();
        self.close_finder();
        self.end_search();
        self.reset_project_panes();
        self.selection_paths.selected_project = None;
        self.pending_ci_fetch = None;
        self.expanded.clear();
        self.list_state = ListState::default();
        self.dirty.rows.mark_dirty();
        self.dirty.disk_cache.mark_dirty();
        self.dirty.fit_widths.mark_dirty();
        self.builds.fit.active = None;
        self.builds.fit.latest = 0;
        self.builds.disk.active = None;
        self.builds.disk.latest = 0;
        self.data_generation += 1;
        self.detail_generation += 1;
        let (tx, rx) = scan::spawn_streaming_scan(
            &self.scan_root,
            self.ci_run_count(),
            &self.current_config.tui.include_dirs,
            &self.current_config.tui.inline_dirs,
            self.include_non_rust(),
            self.http_client.clone(),
        );
        self.bg_tx = tx;
        self.bg_rx = rx;
        self.respawn_watcher();
        let current_config = self.current_config.clone();
        self.refresh_lint_runtime_from_config(&current_config);
    }

    pub fn poll_background(&mut self) -> PollBackgroundStats {
        const MAX_MSGS_PER_FRAME: usize = 50;
        let mut needs_rebuild = false;
        let mut msg_count = 0;
        let started = Instant::now();
        let mut stats = PollBackgroundStats::default();

        while msg_count < MAX_MSGS_PER_FRAME {
            let Ok(msg) = self.bg_rx.try_recv() else {
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
        stats.fit_results = self.poll_fit_width_builds();
        stats.disk_results = self.poll_disk_cache_builds();

        if needs_rebuild {
            self.refresh_derived_state();
            self.maybe_priority_fetch();
        }
        stats.needs_rebuild = needs_rebuild;

        self.refresh_async_caches();
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
                items = self.projects.len(),
                "poll_background"
            );
        }
        stats
    }

    pub(super) const fn record_background_msg_kind(
        stats: &mut PollBackgroundStats,
        msg: &BackgroundMsg,
    ) {
        match msg {
            BackgroundMsg::DiskUsage { .. } | BackgroundMsg::DiskUsageBatch { .. } => {
                stats.disk_usage_msgs += 1;
            },
            BackgroundMsg::GitInfo { .. } | BackgroundMsg::GitFirstCommit { .. } => {
                stats.git_info_msgs += 1;
            },
            BackgroundMsg::GitPathState { .. } => stats.git_path_state_msgs += 1,
            BackgroundMsg::LintStatus { .. } => stats.lint_status_msgs += 1,
            BackgroundMsg::CiRuns { .. }
            | BackgroundMsg::LocalGitQueued { .. }
            | BackgroundMsg::RepoFetchQueued { .. }
            | BackgroundMsg::RepoFetchComplete { .. }
            | BackgroundMsg::CratesIoVersion { .. }
            | BackgroundMsg::RepoMeta { .. }
            | BackgroundMsg::ScanResult { .. }
            | BackgroundMsg::ProjectDiscovered { .. }
            | BackgroundMsg::ProjectRefreshed { .. }
            | BackgroundMsg::LintCachePruned { .. }
            | BackgroundMsg::ServiceReachable { .. }
            | BackgroundMsg::ServiceRecovered { .. }
            | BackgroundMsg::ServiceUnreachable { .. } => {},
        }
    }

    pub(super) fn log_saturated_background_batch(stats: &PollBackgroundStats) {
        const MAX_MSGS_PER_FRAME: usize = 50;
        if stats.bg_msgs != MAX_MSGS_PER_FRAME {
            return;
        }

        tracing::info!(
            bg_msgs = stats.bg_msgs,
            disk_usage_msgs = stats.disk_usage_msgs,
            git_info_msgs = stats.git_info_msgs,
            git_path_state_msgs = stats.git_path_state_msgs,
            lint_status_msgs = stats.lint_status_msgs,
            "poll_background_saturated"
        );
    }

    pub(super) fn poll_ci_fetches(&mut self) -> usize {
        let mut count = 0;
        while let Ok(msg) = self.ci_fetch_rx.try_recv() {
            match msg {
                CiFetchMsg::Complete { path, result, kind } => {
                    self.handle_ci_fetch_complete(&path, result, kind);
                },
            }
            count += 1;
        }
        count
    }

    pub(super) fn poll_example_msgs(&mut self) -> usize {
        let mut count = 0;
        while let Ok(msg) = self.example_rx.try_recv() {
            match msg {
                ExampleMsg::Output(line) => self.example_output.push(line),
                ExampleMsg::Progress(line) => self.apply_example_progress(line),
                ExampleMsg::Finished => self.finish_example_run(),
            }
            count += 1;
        }
        count
    }

    pub(super) fn apply_example_progress(&mut self, line: String) {
        if let Some(last) = self.example_output.last_mut() {
            *last = line;
        } else {
            self.example_output.push(line);
        }
    }

    pub(super) fn finish_example_run(&mut self) {
        self.example_running = None;
        self.example_output.push("── done ──".to_string());
        self.mark_terminal_dirty();
    }

    pub(super) fn poll_clean_msgs(&mut self) {
        while let Ok(msg) = self.clean_rx.try_recv() {
            match msg {
                CleanMsg::Finished(path) => {
                    let abs = PathBuf::from(&path);
                    let already_zero = self
                        .projects
                        .iter()
                        .find(|i| i.path() == abs)
                        .and_then(RootItem::disk_usage_bytes)
                        .is_none_or(|bytes| bytes == 0);
                    if already_zero {
                        self.running_clean_paths.remove(&abs);
                        self.sync_running_clean_toast();
                    }
                },
            }
        }
    }

    pub(super) fn handle_disk_usage(&mut self, path: &Path, bytes: u64) {
        if self.running_clean_paths.remove(path) {
            self.sync_running_clean_toast();
        }
        self.apply_disk_usage(path, bytes, self.is_scan_complete());
    }

    pub(super) fn handle_disk_usage_batch(&mut self, entries: Vec<(AbsolutePath, u64)>) {
        for (path, bytes) in entries {
            self.apply_disk_usage(path.as_path(), bytes, false);
        }
    }

    pub(super) fn apply_disk_usage(
        &mut self,
        path: &Path,
        bytes: u64,
        refresh_git_path_state: bool,
    ) {
        self.fully_loaded.insert(path.to_path_buf());
        if refresh_git_path_state {
            self.refresh_git_path_state(path);
        }
        self.dirty.disk_cache.mark_dirty();
        self.dirty.fit_widths.mark_dirty();

        // Set disk usage on the matching project item and update visibility.
        let mut lint_runtime_changed = false;
        if let Some(project) = self.projects.at_path_mut(path) {
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
            if let Some(runtime) = &self.lint_runtime
                && bytes == 0
            {
                runtime.unregister_project(path.to_path_buf());
            }
            if bytes > 0 {
                let display = crate::project::home_relative_path(path);
                self.register_lint_for_path(&display);
            }
        }
    }

    fn inherited_git_info_paths(&self, path: &Path) -> (Vec<PathBuf>, Vec<PathBuf>) {
        let mut member_paths = Vec::new();
        let mut fallback_worktree_paths = Vec::new();
        for item in &self.projects {
            match item {
                crate::project::RootItem::Workspace(ws) if ws.path() == path => {
                    member_paths.extend(ws.groups().iter().flat_map(|group| {
                        group
                            .members()
                            .iter()
                            .map(|member| member.path().to_path_buf())
                    }));
                },
                crate::project::RootItem::WorkspaceWorktrees(wtg) => {
                    for ws in std::iter::once(wtg.primary()).chain(wtg.linked().iter()) {
                        if ws.path() == path {
                            member_paths.extend(ws.groups().iter().flat_map(|group| {
                                group
                                    .members()
                                    .iter()
                                    .map(|member| member.path().to_path_buf())
                            }));
                        }
                    }
                    if wtg.primary().path() == path {
                        fallback_worktree_paths.extend(
                            wtg.linked()
                                .iter()
                                .filter(|linked| self.git_info_for(linked.path()).is_none())
                                .map(|linked| linked.path().to_path_buf()),
                        );
                    }
                },
                crate::project::RootItem::PackageWorktrees(wtg)
                    if wtg.primary().path() == path =>
                {
                    fallback_worktree_paths.extend(
                        wtg.linked()
                            .iter()
                            .filter(|linked| self.git_info_for(linked.path()).is_none())
                            .map(|linked| linked.path().to_path_buf()),
                    );
                },
                _ => {},
            }
        }
        (member_paths, fallback_worktree_paths)
    }

    pub(super) fn handle_git_info(&mut self, path: &Path, info: GitInfo) {
        self.dirty.fit_widths.mark_dirty();
        let abs = path.to_path_buf();
        let preserved_first_commit = self
            .git_info_for(&abs)
            .and_then(|existing| existing.first_commit.clone());
        let mut info = info;
        if info.first_commit.is_none() {
            info.first_commit = preserved_first_commit;
        }
        let (member_paths, fallback_worktree_paths) = self.inherited_git_info_paths(&abs);
        if let Some(project) = self.projects.at_path_mut(&abs) {
            project.git_info = Some(info.clone());
        }
        for member_path in member_paths {
            if let Some(project) = self.projects.at_path_mut(&member_path) {
                project.git_info = Some(info.clone());
            }
        }
        for linked_path in fallback_worktree_paths {
            if let Some(project) = self.projects.at_path_mut(&linked_path) {
                project.git_info = Some(info.clone());
            }
        }
        if self.is_scan_complete() {
            self.scan.startup_phases.git_seen.insert(abs.clone());
            if let Some(git_toast) = self.scan.startup_phases.git_toast {
                let label = crate::project::home_relative_path(&abs);
                self.mark_tracked_item_completed(git_toast, &label);
            }
            self.maybe_log_startup_phase_completions();
        }
        self.dirty.finder.mark_dirty();
    }

    pub(super) fn handle_git_first_commit(&mut self, path: &Path, first_commit: Option<&str>) {
        let (member_paths, fallback_worktree_paths) = self.inherited_git_info_paths(path);
        let first_commit = first_commit.map(String::from);
        let Some(project) = self.projects.at_path_mut(path) else {
            return;
        };
        let Some(info) = project.git_info.as_mut() else {
            return;
        };
        info.first_commit.clone_from(&first_commit);
        for member_path in member_paths {
            if let Some(project) = self.projects.at_path_mut(&member_path)
                && let Some(info) = project.git_info.as_mut()
            {
                info.first_commit.clone_from(&first_commit);
            }
        }
        for linked_path in fallback_worktree_paths {
            if let Some(project) = self.projects.at_path_mut(&linked_path)
                && let Some(info) = project.git_info.as_mut()
            {
                info.first_commit.clone_from(&first_commit);
            }
        }
    }

    pub(super) fn handle_repo_fetch_complete(&mut self, key: &str) {
        let path = PathBuf::from(key);
        if let Some(repo_toast) = self.scan.startup_phases.repo_toast {
            let label = crate::project::home_relative_path(&path);
            self.mark_tracked_item_completed(repo_toast, &label);
        }
        self.scan.startup_phases.repo_seen.insert(path);
        self.maybe_log_startup_phase_completions();
    }

    pub(super) fn handle_repo_meta(
        &mut self,
        path: &Path,
        stars: u64,
        description: Option<String>,
    ) {
        let abs = path.to_path_buf();
        // Propagate stars to workspace members.
        for item in &self.projects {
            match item {
                crate::project::RootItem::Workspace(ws) if ws.path() == abs => {
                    for group in ws.groups() {
                        for member in group.members() {
                            self.stars
                                .entry(member.path().to_path_buf())
                                .or_insert(stars);
                        }
                    }
                },
                crate::project::RootItem::WorkspaceWorktrees(wtg) => {
                    for ws in std::iter::once(wtg.primary()).chain(wtg.linked().iter()) {
                        if ws.path() == abs {
                            for group in ws.groups() {
                                for member in group.members() {
                                    self.stars
                                        .entry(member.path().to_path_buf())
                                        .or_insert(stars);
                                }
                            }
                        }
                    }
                },
                _ => {},
            }
        }
        self.stars.insert(abs.clone(), stars);
        if let Some(desc) = description {
            self.repo_descriptions.insert(abs, desc);
        }
    }

    pub(super) fn handle_project_discovered(&mut self, item: RootItem) -> bool {
        let display = item.display_path();
        let mut already_exists = false;
        self.projects.for_each_leaf_path(|path, _| {
            if crate::project::home_relative_path(path) == display {
                already_exists = true;
            }
        });
        if already_exists {
            return false;
        }

        self.register_item_background_services(&item);
        // Insert into the hierarchy directly — under a parent workspace if
        // one exists, otherwise as a top-level peer.
        self.projects.insert_into_hierarchy(item);
        // Signal that derived state and caches need refresh.
        // The caller batches multiple discoveries before refreshing once.
        true
    }

    pub(super) fn handle_project_refreshed(&mut self, mut item: RootItem) -> bool {
        let path = item.path().to_path_buf();

        // Replace the leaf in project_list_items, transferring runtime data
        // from the old item to the incoming one.
        let Some(old) = self.projects.replace_leaf_by_path(&path, item.clone()) else {
            return false;
        };
        for (project_path, info) in old.collect_project_info() {
            if let Some(project) = item.at_path_mut(&project_path) {
                *project = info;
            }
        }
        // Re-replace with the runtime-data-enriched version.
        self.projects.replace_leaf_by_path(&path, item);
        self.projects.regroup_top_level_worktrees();
        self.cached_detail = None;
        // Signal that derived state needs refresh (batched by caller).
        true
    }

    pub(super) fn apply_service_signal(&mut self, signal: ServiceSignal) {
        match signal {
            ServiceSignal::Reachable(service) => {
                self.unreachable_services.remove(&service);
            },
            ServiceSignal::Unreachable(service) => {
                self.unreachable_services.insert(service);
                if self.service_retry_active.insert(service) {
                    self.spawn_service_retry(service);
                }
            },
        }
    }

    pub(super) fn spawn_service_retry(&self, service: ServiceKind) {
        #[cfg(test)]
        if !self.retry_spawn_mode.is_enabled() {
            return;
        }

        let tx = self.bg_tx.clone();
        let client = self.http_client.clone();
        thread::spawn(move || {
            loop {
                if client.probe_service(service) {
                    crate::scan::emit_service_recovered(&tx, service);
                    break;
                }
                thread::sleep(Duration::from_secs(SERVICE_RETRY_SECS));
            }
        });
    }

    pub(super) fn mark_service_recovered(&mut self, service: ServiceKind) {
        self.unreachable_services.remove(&service);
        self.service_retry_active.remove(&service);
    }

    pub fn unreachable_service_message(&self) -> Option<String> {
        let mut services = Vec::new();
        for service in [ServiceKind::GitHub, ServiceKind::CratesIo] {
            if self.unreachable_services.contains(&service) {
                services.push(service.label());
            }
        }
        match services.as_slice() {
            [service] => Some(format!(" {service} unreachable ")),
            [first, second] => Some(format!(" {first} and {second} unreachable ")),
            _ => None,
        }
    }

    fn update_generations_for_msg(&mut self, msg: &BackgroundMsg) {
        if msg.path().is_some() {
            self.data_generation += 1;
        }
        if let Some(path) = msg.path()
            && self.detail_path_is_affected(path)
        {
            self.detail_generation += 1;
        }
    }

    fn handle_disk_usage_msg(&mut self, path: &Path, bytes: u64) {
        self.scan
            .startup_phases
            .disk_seen
            .insert(path.to_path_buf());
        self.handle_disk_usage(path, bytes);
        self.maybe_log_startup_phase_completions();
    }

    fn handle_disk_usage_batch_msg(
        &mut self,
        root_path: &AbsolutePath,
        entries: Vec<(AbsolutePath, u64)>,
    ) {
        self.data_generation += 1;
        if entries
            .iter()
            .any(|(path, _)| self.detail_path_is_affected(path.as_path()))
        {
            self.detail_generation += 1;
        }
        self.scan
            .startup_phases
            .disk_seen
            .insert(root_path.to_path_buf());
        self.handle_disk_usage_batch(entries);
        self.maybe_log_startup_phase_completions();
    }

    fn handle_git_path_state_msg(&mut self, path: &AbsolutePath, state: GitPathState) {
        tracing::info!(path = %path, state = %state.label(), "app_git_path_state_applied");
        self.git_path_states.insert(path.to_path_buf(), state);
    }

    fn handle_crates_io_version_msg(&mut self, path: &Path, version: String, downloads: u64) {
        let abs = path.to_path_buf();
        if self.is_cargo_active_path(&abs) {
            self.crates_versions.insert(abs.clone(), version);
            self.crates_downloads.insert(abs, downloads);
        } else {
            self.crates_versions.remove(&abs);
            self.crates_downloads.remove(&abs);
        }
    }

    fn handle_lint_status_msg(&mut self, path: &Path, status: LintStatus) {
        let abs = path.to_path_buf();
        let status_started = matches!(status, LintStatus::Running(_));
        let status_is_terminal = matches!(
            status,
            LintStatus::Passed(_) | LintStatus::Failed(_) | LintStatus::Stale | LintStatus::NoLog
        );
        if !self.is_cargo_active_path(&abs) {
            self.lint_runs.remove(&abs);
            self.lint_status.remove(&abs);
            return;
        }
        let mut is_rust = false;
        self.projects.for_each_leaf_path(|path, rust| {
            if path == abs.as_path() {
                is_rust = rust;
            }
        });
        let eligible = crate::lint::project_is_eligible(
            &self.current_config.lint,
            &abs.to_string_lossy(),
            &abs,
            is_rust,
        );
        if eligible {
            if matches!(status, LintStatus::NoLog) {
                self.lint_status.remove(&abs);
            } else {
                self.lint_status.insert(abs.clone(), status);
            }
        } else {
            self.lint_runs.remove(&abs);
            self.lint_status.remove(&abs);
            self.running_lint_paths.remove(&abs);
        }
        self.update_lint_rollups_for_path(&abs);
        if status_started {
            self.running_lint_paths.insert(abs.clone(), Instant::now());
        }
        if status_is_terminal {
            self.running_lint_paths.remove(&abs);
        }
        self.sync_running_lint_toast();
        if !self.is_scan_complete() {
            return;
        }
        if status_started {
            let expected = self
                .scan
                .startup_phases
                .lint_expected
                .get_or_insert_with(HashSet::new);
            if expected.insert(abs.clone()) {
                self.scan.startup_phases.lint_complete_at = None;
            }
        }
        if status_is_terminal
            && self
                .scan
                .startup_phases
                .lint_expected
                .as_ref()
                .is_some_and(|expected| expected.contains(&abs))
        {
            self.scan.startup_phases.lint_seen_terminal.insert(abs);
        }
        self.maybe_log_startup_phase_completions();
    }

    fn handle_scan_result(
        &mut self,
        projects: Vec<RootItem>,
        disk_entries: &[(String, PathBuf)],
    ) {
        let kind = if self.scan.run_count == 1 {
            "initial"
        } else {
            "rescan"
        };

        tracing::info!(
            elapsed_ms = crate::perf_log::ms(self.scan.started_at.elapsed().as_millis()),
            kind,
            run = self.scan.run_count,
            tree_items = projects.len(),
            disk_entries = disk_entries.len(),
            "scan_result_applied"
        );

        // Apply tree (same as apply_tree_build but inlined to avoid redundant
        // rebuild scheduling).
        let selected_path = self
            .selected_display_path()
            .or_else(|| self.selection_paths.last_selected.clone());
        self.projects = ProjectList::new(projects);
        self.dirty.finder.mark_dirty();
        self.dirty.rows.mark_dirty();
        self.dirty.disk_cache.mark_dirty();
        self.dirty.fit_widths.mark_dirty();
        self.recompute_cargo_active_paths();
        self.prune_inactive_project_state();
        self.register_lint_for_root_items();
        self.rebuild_lint_rollups();
        self.data_generation += 1;
        self.detail_generation += 1;

        // Re-run search if active so filtered results match the latest
        // hierarchy state.
        if self.is_searching() && !self.search_query.is_empty() {
            let query = self.search_query.clone();
            self.update_search(&query);
        } else {
            self.filtered.clear();
        }

        // Propagate git info and stars from workspace roots to members.
        let mut inherited_git_info = Vec::new();
        for item in &self.projects {
            let root_path = item.path();
            let member_paths: Vec<PathBuf> = match item {
                RootItem::Workspace(ws) => ws
                    .groups()
                    .iter()
                    .flat_map(|g| g.members().iter().map(|m| m.path().to_path_buf()))
                    .collect(),
                RootItem::WorkspaceWorktrees(wtg) => std::iter::once(wtg.primary())
                    .chain(wtg.linked().iter())
                    .flat_map(|ws| {
                        ws.groups()
                            .iter()
                            .flat_map(|g| g.members().iter().map(|m| m.path().to_path_buf()))
                    })
                    .collect(),
                _ => Vec::new(),
            };
            if let Some(info) = self
                .projects
                .at_path(root_path)
                .and_then(|project| project.git_info.clone())
            {
                for member_path in &member_paths {
                    if self
                        .projects
                        .at_path(member_path)
                        .is_none_or(|project| project.git_info.is_none())
                    {
                        inherited_git_info.push((member_path.clone(), info.clone()));
                    }
                }
            }
            if let Some(&stars) = self.stars.get(root_path) {
                for member_path in &member_paths {
                    self.stars.entry(member_path.clone()).or_insert(stars);
                }
            }
        }
        for (member_path, info) in inherited_git_info {
            if let Some(project) = self.projects.at_path_mut(&member_path) {
                project.git_info = Some(info);
            }
        }

        // Restore selection.
        if let Some(path) = selected_path {
            self.select_project_in_tree(&path);
        } else if !self.projects.is_empty() {
            self.list_state.select(Some(0));
        }
        self.sync_selected_project();

        // Register watcher for each item (same as register_item_background_services).
        self.register_background_services_for_tree();

        // Mark scan complete and initialize startup tracking.
        self.scan.phase = ScanPhase::Complete;
        self.initialize_startup_phase_tracker();
        self.schedule_git_path_state_refreshes();
        self.schedule_git_first_commit_refreshes();
    }

    /// Handle a single `BackgroundMsg`. Returns `true` if the tree needs rebuilding.
    pub(super) fn handle_bg_msg(&mut self, msg: BackgroundMsg) -> bool {
        self.update_generations_for_msg(&msg);
        match msg {
            BackgroundMsg::DiskUsage { path, bytes } => {
                self.handle_disk_usage_msg(path.as_path(), bytes);
            },
            BackgroundMsg::DiskUsageBatch { root_path, entries } => {
                self.handle_disk_usage_batch_msg(&root_path, entries);
            },
            BackgroundMsg::LocalGitQueued { path } => {
                self.scan
                    .startup_phases
                    .git_expected
                    .insert(path.to_path_buf());
            },
            BackgroundMsg::CiRuns { path, runs } => {
                self.insert_ci_runs(path.as_path(), runs);
            },
            BackgroundMsg::RepoFetchQueued { key } => {
                self.scan
                    .startup_phases
                    .repo_expected
                    .insert(PathBuf::from(key));
            },
            BackgroundMsg::RepoFetchComplete { key } => self.handle_repo_fetch_complete(&key),
            BackgroundMsg::GitInfo { path, info } => {
                self.handle_git_info(path.as_path(), info);
            },
            BackgroundMsg::GitFirstCommit { path, first_commit } => {
                self.handle_git_first_commit(path.as_path(), first_commit.as_deref());
            },
            BackgroundMsg::GitPathState { path, state } => {
                self.handle_git_path_state_msg(&path, state);
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
            } => {
                self.show_timed_toast(
                    "Lint cache",
                    format!(
                        "Evicted {runs_evicted} {}, reclaimed {}",
                        if runs_evicted == 1 { "run" } else { "runs" },
                        crate::tui::render::format_bytes(bytes_reclaimed),
                    ),
                );
                self.refresh_lint_cache_usage_from_disk();
            },
            BackgroundMsg::LintStatus { path, status } => {
                self.handle_lint_status_msg(path.as_path(), status);
            },
            BackgroundMsg::ServiceReachable { service } => {
                self.apply_service_signal(ServiceSignal::Reachable(service));
            },
            BackgroundMsg::ServiceRecovered { service } => {
                self.mark_service_recovered(service);
            },
            BackgroundMsg::ServiceUnreachable { service } => {
                self.apply_service_signal(ServiceSignal::Unreachable(service));
            },
        }
        false
    }

    pub(super) fn detail_path_is_affected(&self, path: &Path) -> bool {
        let Some(selected_path) = self.selected_project_path() else {
            return false;
        };
        let abs = path;
        self.selected_lint_rollup_key().map_or_else(
            || selected_path == abs,
            |key| {
                self.lint_rollup_paths
                    .get(&key)
                    .is_some_and(|paths| paths.iter().any(|candidate| candidate == abs))
            },
        )
    }

    /// Spawn a priority fetch for the selected project if it hasn't been loaded yet.
    pub(super) fn maybe_priority_fetch(&mut self) {
        let Some(abs_path) = self.selected_project_path().map(Path::to_path_buf) else {
            return;
        };
        let display_path = self
            .selected_display_path()
            .unwrap_or_else(|| abs_path.display().to_string());
        let name = self
            .cached_detail
            .as_ref()
            .map(|c| c.info.name.clone())
            .filter(|n| n != "-");
        if !self.fully_loaded.contains(&abs_path)
            && self.priority_fetch_path.as_ref() != Some(&display_path)
        {
            self.priority_fetch_path = Some(display_path.clone());
            let abs_str = abs_path.display().to_string();
            crate::tui::terminal::spawn_priority_fetch(
                self,
                &display_path,
                &abs_str,
                name.as_ref(),
            );
        }
    }
}
