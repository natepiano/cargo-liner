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
use super::types::TreeBuildResult;
use crate::config::Config;
use crate::constants::SERVICE_RETRY_SECS;
use crate::http::ServiceKind;
use crate::http::ServiceSignal;
use crate::lint;
use crate::lint::LintStatus;
use crate::lint::RegisterProjectRequest;
use crate::project::GitInfo;
use crate::project::GitPathState;
use crate::project::ProjectLanguage::Rust;
use crate::project::RustProject;
use crate::scan;
use crate::scan::BackgroundMsg;
use crate::scan::FlatEntry;
use crate::scan::ProjectNode;
use crate::tui::columns::ResolvedWidths;
use crate::tui::config_reload;
use crate::tui::terminal::CiFetchMsg;
use crate::tui::terminal::CleanMsg;
use crate::tui::terminal::ExampleMsg;
use crate::tui::types::PaneId;
use crate::watcher;
use crate::watcher::WatchRequest;

impl App {
    pub(super) fn apply_tree_build(
        &mut self,
        nodes: Vec<ProjectNode>,
        flat_entries: Vec<FlatEntry>,
    ) {
        let selected_path = self
            .selected_project()
            .map(|p| p.path.clone())
            .or_else(|| self.selection_paths.last_selected.clone());
        let should_focus_project_list = self.focused_pane == PaneId::ScanLog && !nodes.is_empty();
        self.nodes = nodes;
        self.flat_entries = flat_entries;
        self.dirty.finder.mark_dirty();
        self.dirty.rows.mark_dirty();
        self.dirty.disk_cache.mark_dirty();
        self.dirty.fit_widths.mark_dirty();
        self.recompute_cargo_active_paths();
        self.prune_inactive_project_state();
        self.sync_lint_runtime_projects();
        self.rebuild_lint_rollups();
        self.data_generation += 1;
        self.detail_generation += 1;

        // Re-run search if active so filtered indices match new flat_entries
        if self.is_searching() && !self.search_query.is_empty() {
            let query = self.search_query.clone();
            self.update_search(&query);
        } else {
            self.filtered.clear();
        }

        // Propagate git info and stars from workspace roots to their members.
        for node in &self.nodes {
            if let Some(info) = self.git_info.get(&node.project.path).cloned() {
                for member in Self::all_group_members(node) {
                    self.git_info
                        .entry(member.path.clone())
                        .or_insert_with(|| info.clone());
                }
            }
            if let Some(&stars) = self.stars.get(&node.project.path) {
                for member in Self::all_group_members(node) {
                    self.stars.entry(member.path.clone()).or_insert(stars);
                }
            }
        }

        // Try to restore selection
        if let Some(path) = selected_path {
            self.select_project_in_tree(&path);
        } else if !self.nodes.is_empty() {
            self.list_state.select(Some(0));
        }
        if should_focus_project_list {
            self.focus_pane(PaneId::ProjectList);
        }
        self.sync_selected_project();
    }

    pub(super) fn rebuild_tree(&mut self) { self.request_tree_rebuild(); }

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
        self.show_timed_toast(
            "Config reload failed",
            "Keeping previous settings".to_string(),
        );
        self.scan_log.push(format!("config reload failed: {err}"));
        self.scan_log_state
            .select(Some(self.scan_log.len().saturating_sub(1)));
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
            },
            Err(err) => self.record_config_reload_failure(&err),
        }
    }

    pub fn save_and_apply_config(&mut self, cfg: &Config) -> Result<(), String> {
        crate::config::save(cfg)?;
        self.apply_config(cfg);
        self.sync_config_watch_state();
        Ok(())
    }

    pub(super) fn apply_config(&mut self, cfg: &Config) {
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
                self.rebuild_tree();
            }
        }
    }

    pub(super) fn refresh_lint_runtime_from_config(&mut self, cfg: &Config) {
        let lint_spawn = lint::spawn(cfg, self.bg_tx.clone());
        self.lint_runtime = lint_spawn.handle;
        self.register_existing_projects();
        self.sync_lint_runtime_projects_immediately();
        self.refresh_lint_statuses_from_disk();
        self.refresh_port_report_histories_from_disk();
        self.rebuild_lint_rollups();
        self.cached_fit_widths = ResolvedWidths::new(self.lint_enabled());
        self.dirty.rows.mark_dirty();
        self.dirty.fit_widths.mark_dirty();
        self.data_generation += 1;
        self.detail_generation += 1;
        if let Some(warning) = lint_spawn.warning {
            self.status_flash = Some((warning.clone(), Instant::now()));
            self.show_timed_toast("Lint runtime", warning.clone());
            self.scan_log.push(warning);
            self.scan_log_state
                .select(Some(self.scan_log.len().saturating_sub(1)));
        }
    }

    pub(super) fn respawn_watcher(&mut self) {
        self.watch_tx = watcher::spawn_watcher(
            self.scan_root.clone(),
            self.bg_tx.clone(),
            self.ci_run_count(),
            self.include_non_rust(),
            self.lint_enabled(),
            self.current_config.tui.include_dirs.clone(),
            self.http_client.clone(),
        );
    }

    pub(super) fn register_existing_projects(&self) {
        for project in &self.all_projects {
            self.register_project_background_services(project);
        }
    }

    pub(super) fn refresh_lint_statuses_from_disk(&mut self) {
        self.lint_status.clear();
        if !self.lint_enabled() {
            return;
        }
        for project in &self.all_projects {
            if !self.is_cargo_active_path(&project.path) {
                continue;
            }
            if !crate::lint::project_is_eligible(
                &self.current_config.lint,
                &project.path,
                &PathBuf::from(&project.abs_path),
                project.is_rust == Rust,
            ) {
                continue;
            }
            let status = crate::lint::read_status(&PathBuf::from(&project.abs_path));
            if !matches!(status, LintStatus::NoLog) {
                self.lint_status.insert(project.path.clone(), status);
            }
        }
    }

    pub(super) fn refresh_port_report_histories_from_disk(&mut self) {
        self.port_report_runs.clear();
        for project in &self.all_projects {
            if !self.is_cargo_active_path(&project.path) {
                continue;
            }
            let runs = crate::lint::read_history(&PathBuf::from(&project.abs_path));
            if !runs.is_empty() {
                self.port_report_runs.insert(project.path.clone(), runs);
            }
        }
        self.refresh_lint_history_usage_from_disk();
    }

    pub(super) fn reload_port_report_history(&mut self, project_path: &str) {
        let Some(project) = self
            .all_projects
            .iter()
            .find(|project| project.path == project_path)
        else {
            self.port_report_runs.remove(project_path);
            return;
        };
        if !self.is_cargo_active_path(project_path) {
            self.port_report_runs.remove(project_path);
            return;
        }
        let runs = crate::lint::read_history(&PathBuf::from(&project.abs_path));
        if runs.is_empty() {
            self.port_report_runs.remove(project_path);
        } else {
            self.port_report_runs.insert(project_path.to_string(), runs);
        }
        self.refresh_lint_history_usage_from_disk();
    }

    pub(super) fn refresh_lint_history_usage_from_disk(&mut self) {
        let history_budget_bytes = self
            .current_config
            .port_report
            .history_budget_bytes()
            .unwrap_or(None);
        self.lint_history_usage = crate::lint::retained_history_usage(history_budget_bytes);
    }

    pub(super) fn register_project_background_services(&self, project: &RustProject) {
        let started = Instant::now();
        let abs_path = PathBuf::from(&project.abs_path);
        let repo_root = crate::project::git_repo_root(&abs_path);
        let has_repo_root = repo_root.is_some();
        let _ = self.watch_tx.send(WatchRequest {
            project_path: project.path.clone(),
            abs_path,
            repo_root,
        });
        crate::perf_log::log_duration(
            "app_register_project_background_services",
            started.elapsed(),
            &format!("path={} has_repo_root={has_repo_root}", project.path),
            0,
        );
    }

    pub(super) fn schedule_git_path_state_refreshes(&self) {
        let tx = self.bg_tx.clone();
        let projects: Vec<(String, String)> = self
            .all_projects
            .iter()
            .map(|project| (project.path.clone(), project.abs_path.clone()))
            .collect();
        std::thread::spawn(move || {
            let states = crate::project::detect_git_path_states_batch(&projects);
            for (path, state) in states {
                let _ = tx.send(BackgroundMsg::GitPathState { path, state });
            }
        });
    }

    pub(super) fn schedule_git_first_commit_refreshes(&self) {
        let tx = self.bg_tx.clone();
        let mut projects_by_repo: HashMap<PathBuf, Vec<String>> = HashMap::new();
        for project in &self.all_projects {
            let abs_path = PathBuf::from(&project.abs_path);
            let Some(repo_root) = crate::project::git_repo_root(&abs_path) else {
                continue;
            };
            projects_by_repo
                .entry(repo_root)
                .or_default()
                .push(project.path.clone());
        }
        std::thread::spawn(move || {
            for (repo_root, paths) in projects_by_repo {
                let started = Instant::now();
                let first_commit = crate::project::detect_first_commit(&repo_root);
                crate::perf_log::log_duration(
                    "git_first_commit_fetch",
                    started.elapsed(),
                    &format!(
                        "repo_root={} rows={} found={}",
                        repo_root.display(),
                        paths.len(),
                        first_commit.is_some()
                    ),
                    0,
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

    pub(super) fn sync_lint_runtime_projects(&self) { self.sync_lint_runtime_projects_with(false); }

    pub(super) fn sync_lint_runtime_projects_immediately(&self) {
        self.sync_lint_runtime_projects_with(true);
    }

    pub(super) fn lint_runtime_root_projects(&self) -> Vec<&RustProject> {
        let mut projects = Vec::new();
        let mut seen = HashSet::new();

        for node in &self.nodes {
            if seen.insert(node.project.path.clone()) {
                projects.push(&node.project);
            }
            for worktree in &node.worktrees {
                if seen.insert(worktree.project.path.clone()) {
                    projects.push(&worktree.project);
                }
            }
        }

        if !projects.is_empty() {
            return projects;
        }

        self.all_projects
            .iter()
            .filter(|project| seen.insert(project.path.clone()))
            .collect()
    }

    pub(super) fn lint_runtime_projects_snapshot(&self) -> Vec<RegisterProjectRequest> {
        if !self.is_scan_complete() {
            return Vec::new();
        }
        self.lint_runtime_root_projects()
            .into_iter()
            .filter(|project| !self.deleted_projects.contains(&project.path))
            .filter(|project| self.is_cargo_active_path(&project.path))
            .map(|project| RegisterProjectRequest {
                project_path: project.path.clone(),
                abs_path:     PathBuf::from(&project.abs_path),
                is_rust:      project.is_rust == Rust,
            })
            .collect()
    }

    pub(super) fn sync_lint_runtime_projects_with(&self, force_immediate_run: bool) {
        let Some(runtime) = &self.lint_runtime else {
            return;
        };
        let projects = self.lint_runtime_projects_snapshot();
        if force_immediate_run {
            runtime.sync_projects_immediately(projects);
        } else {
            runtime.sync_projects(projects);
        }
    }

    pub(super) fn initialize_startup_phase_tracker(&mut self) {
        let disk_expected = super::snapshots::initial_disk_batch_count(&self.all_projects);
        let git_seen = self
            .scan
            .startup_phases
            .git_expected
            .iter()
            .filter(|path| self.git_info.contains_key(*path))
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
        let git_remaining = self
            .scan
            .startup_phases
            .git_expected
            .len()
            .saturating_sub(self.scan.startup_phases.git_seen.len());
        if git_remaining > 0 {
            self.scan.startup_phases.git_toast = Some(
                self.start_task_toast("Scanning local git repos", self.startup_git_toast_body()),
            );
        }
        let repo_remaining = self
            .scan
            .startup_phases
            .repo_expected
            .len()
            .saturating_sub(self.scan.startup_phases.repo_seen.len());
        if repo_remaining > 0 {
            self.scan.startup_phases.repo_toast = Some(self.start_task_toast(
                "Retrieving GitHub repo details",
                self.startup_repo_toast_body(),
            ));
        }
        crate::perf_log::log_event(&format!(
            "startup_phase_plan disk_expected={} git_expected={} repo_expected={} lint_expected={}",
            self.scan.startup_phases.disk_expected.unwrap_or(0),
            self.scan.startup_phases.git_expected.len(),
            self.scan.startup_phases.repo_expected.len(),
            self.scan
                .startup_phases
                .lint_expected
                .as_ref()
                .map_or(0, HashSet::len)
        ));
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
            crate::perf_log::log_event(&format!(
                "startup_phase_complete phase=disk_applied since_scan_complete_ms={} seen={} expected={}",
                now.duration_since(scan_complete_at).as_millis(),
                self.scan.startup_phases.disk_seen.len(),
                self.scan.startup_phases.disk_expected.unwrap_or(0)
            ));
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
            crate::perf_log::log_event(&format!(
                "startup_phase_complete phase=git_local_applied since_scan_complete_ms={} seen={} expected={}",
                now.duration_since(scan_complete_at).as_millis(),
                self.scan.startup_phases.git_seen.len(),
                self.scan.startup_phases.git_expected.len()
            ));
        } else if let Some(git_toast) = self.scan.startup_phases.git_toast {
            self.update_task_toast_body(git_toast, self.startup_git_toast_body());
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
            crate::perf_log::log_event(&format!(
                "startup_phase_complete phase=repo_fetch_applied since_scan_complete_ms={} seen={} expected={}",
                now.duration_since(scan_complete_at).as_millis(),
                self.scan.startup_phases.repo_seen.len(),
                self.scan.startup_phases.repo_expected.len()
            ));
        } else if let Some(repo_toast) = self.scan.startup_phases.repo_toast {
            self.update_task_toast_body(repo_toast, self.startup_repo_toast_body());
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
            crate::perf_log::log_event(&format!(
                "startup_phase_complete phase=lint_terminal_applied since_scan_complete_ms={} seen={} expected={}",
                now.duration_since(scan_complete_at).as_millis(),
                self.scan.startup_phases.lint_seen_terminal.len(),
                self.scan
                    .startup_phases
                    .lint_expected
                    .as_ref()
                    .map_or(0, HashSet::len)
            ));
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
                crate::perf_log::log_event(&format!(
                    "startup_complete since_scan_complete_ms={} disk_seen={} disk_expected={} git_seen={} git_expected={} repo_seen={} repo_expected={} lint_seen={} lint_expected={}",
                    now.duration_since(scan_complete_at).as_millis(),
                    self.scan.startup_phases.disk_seen.len(),
                    self.scan.startup_phases.disk_expected.unwrap_or(0),
                    self.scan.startup_phases.git_seen.len(),
                    self.scan.startup_phases.git_expected.len(),
                    self.scan.startup_phases.repo_seen.len(),
                    self.scan.startup_phases.repo_expected.len(),
                    self.scan.startup_phases.lint_seen_terminal.len(),
                    self.scan
                        .startup_phases
                        .lint_expected
                        .as_ref()
                        .map_or(0, HashSet::len)
                ));
                crate::perf_log::log_event(&format!(
                    "steady_state_begin since_scan_complete_ms={}",
                    now.duration_since(scan_complete_at).as_millis()
                ));
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

    pub(super) fn startup_remaining_toast_body(
        expected: &HashSet<String>,
        seen: &HashSet<String>,
    ) -> String {
        let Some(current) = expected.iter().find(|path| !seen.contains(*path)) else {
            return "Complete".to_string();
        };
        let remaining = expected.len().saturating_sub(seen.len());
        if remaining <= 1 {
            current.clone()
        } else {
            format!("{current}\n+ {} others", remaining - 1)
        }
    }

    pub(super) fn startup_lint_toast_body_for(
        expected: &HashSet<String>,
        seen: &HashSet<String>,
    ) -> String {
        let mut remaining = expected.iter().filter(|path| !seen.contains(*path));
        let Some(first) = remaining.next() else {
            return "Complete".to_string();
        };
        let Some(second) = remaining.next() else {
            return first.clone();
        };
        let other_count = remaining.count();
        if other_count == 0 {
            format!("{first}\n{second}")
        } else {
            format!("{first}\n{second} (+ {other_count} others)")
        }
    }

    pub(super) fn running_lint_toast_body(&self) -> String {
        Self::startup_lint_toast_body_for(&self.running_lint_paths, &HashSet::new())
    }

    pub(super) fn sync_running_lint_toast(&mut self) {
        if self.running_lint_paths.is_empty() {
            if let Some(task_id) = self.lint_toast.take() {
                self.finish_task_toast(task_id);
            }
            return;
        }

        let body = self.running_lint_toast_body();
        if let Some(task_id) = self.lint_toast {
            self.update_task_toast_body(task_id, body);
        } else {
            self.lint_toast = Some(self.start_task_toast("Lints", body));
        }
    }

    pub(super) fn request_tree_rebuild(&mut self) {
        self.builds.tree.latest = self.builds.tree.latest.wrapping_add(1);
        if self.builds.tree.active.is_some() {
            return;
        }
        self.spawn_tree_build(self.builds.tree.latest);
    }

    pub(super) fn spawn_tree_build(&mut self, build_id: u64) {
        let tx = self.builds.tree.tx.clone();
        let projects = self.tree_projects_snapshot();
        let inline_dirs = self.current_config.tui.inline_dirs.clone();
        self.builds.tree.active = Some(build_id);
        std::thread::spawn(move || {
            let started = Instant::now();
            let nodes = scan::build_tree(&projects, &inline_dirs);
            let flat_entries = scan::build_flat_entries(&nodes);
            crate::perf_log::log_duration(
                "tree_build",
                started.elapsed(),
                &format!(
                    "build_id={} projects={} nodes={} flat_entries={}",
                    build_id,
                    projects.len(),
                    nodes.len(),
                    flat_entries.len()
                ),
                crate::perf_log::slow_worker_threshold_ms(),
            );
            let _ = tx.send(TreeBuildResult {
                build_id,
                nodes,
                flat_entries,
            });
        });
    }

    pub(super) fn poll_tree_builds(&mut self) -> usize {
        let mut applied = 0;
        while let Ok(result) = self.builds.tree.rx.try_recv() {
            if self.builds.tree.active != Some(result.build_id) {
                continue;
            }
            self.builds.tree.active = None;
            self.apply_tree_build(result.nodes, result.flat_entries);
            applied += 1;
            if result.build_id != self.builds.tree.latest {
                self.spawn_tree_build(self.builds.tree.latest);
            }
        }
        applied
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
        let nodes = self.nodes.clone();
        let disk_usage = self.disk_usage.clone();
        let git_info = self.git_info.clone();
        let git_path_states = self.git_path_states.clone();
        let deleted_projects = self.deleted_projects.clone();
        let lint_enabled = self.lint_enabled();
        self.builds.fit.active = Some(build_id);
        std::thread::spawn(move || {
            let started = Instant::now();
            let widths = super::snapshots::build_fit_widths_snapshot(
                &nodes,
                &disk_usage,
                &git_info,
                &git_path_states,
                &deleted_projects,
                lint_enabled,
                build_id,
            );
            crate::perf_log::log_duration(
                "fit_widths_build",
                started.elapsed(),
                &format!("build_id={} nodes={}", build_id, nodes.len()),
                crate::perf_log::slow_worker_threshold_ms(),
            );
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
        let nodes = self.nodes.clone();
        let disk_usage = self.disk_usage.clone();
        self.builds.disk.active = Some(build_id);
        std::thread::spawn(move || {
            let started = Instant::now();
            let (root_sorted, child_sorted) =
                super::snapshots::build_disk_cache_snapshot(&nodes, &disk_usage);
            crate::perf_log::log_duration(
                "disk_cache_build",
                started.elapsed(),
                &format!(
                    "build_id={} nodes={} root_values={} child_sets={}",
                    build_id,
                    nodes.len(),
                    root_sorted.len(),
                    child_sorted.len()
                ),
                crate::perf_log::slow_worker_threshold_ms(),
            );
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

    pub(super) fn refresh_async_caches(&mut self) {
        self.request_disk_cache_build();
        self.request_fit_widths_build();
    }

    pub fn rescan(&mut self) {
        self.all_projects.clear();
        self.nodes.clear();
        self.flat_entries.clear();
        self.disk_usage.clear();
        self.ci_state.clear();
        self.lint_status.clear();
        self.lint_history_usage = crate::lint::HistoryUsage::default();
        self.port_report_runs.clear();
        self.git_info.clear();
        self.git_path_states.clear();
        self.cargo_active_paths.clear();
        self.crates_versions.clear();
        self.crates_downloads.clear();
        self.stars.clear();
        self.repo_descriptions.clear();
        self.scan_log.clear();
        self.scan_log_state = ListState::default();
        self.scan.phase = ScanPhase::Running;
        self.scan.started_at = Instant::now();
        self.scan.run_count += 1;
        self.scan.startup_phases = StartupPhaseTracker::default();
        crate::perf_log::log_event(&format!(
            "scan_start kind=rescan run={}",
            self.scan.run_count
        ));
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
        self.builds.tree.active = None;
        self.builds.tree.latest = 0;
        self.builds.fit.active = None;
        self.builds.fit.latest = 0;
        self.builds.disk.active = None;
        self.builds.disk.latest = 0;
        self.sync_lint_runtime_projects();
        self.data_generation += 1;
        self.detail_generation += 1;
        let (tx, rx) = scan::spawn_streaming_scan(
            &self.scan_root,
            self.ci_run_count(),
            &self.current_config.tui.include_dirs,
            self.include_non_rust(),
            self.lint_enabled(),
            self.http_client.clone(),
        );
        self.bg_tx = tx;
        self.bg_rx = rx;
        self.respawn_watcher();
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

        stats.tree_results = self.poll_tree_builds();
        stats.fit_results = self.poll_fit_width_builds();
        stats.disk_results = self.poll_disk_cache_builds();

        if needs_rebuild {
            self.request_tree_rebuild();
            self.maybe_priority_fetch();
        }
        stats.needs_rebuild = needs_rebuild;

        self.refresh_async_caches();
        crate::perf_log::log_duration(
            "poll_background",
            started.elapsed(),
            &format!(
                "bg_msgs={} ci_msgs={} example_msgs={} tree_results={} fit_results={} disk_results={} needs_rebuild={} projects={} nodes={}",
                stats.bg_msgs,
                stats.ci_msgs,
                stats.example_msgs,
                stats.tree_results,
                stats.fit_results,
                stats.disk_results,
                stats.needs_rebuild,
                self.all_projects.len(),
                self.nodes.len()
            ),
            crate::perf_log::slow_bg_batch_threshold_ms(),
        );
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
            | BackgroundMsg::ProjectDiscovered { .. }
            | BackgroundMsg::ProjectRefreshed { .. }
            | BackgroundMsg::ScanComplete
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

        crate::perf_log::log_event(&format!(
            "poll_background_saturated bg_msgs={} disk_usage_msgs={} git_info_msgs={} git_path_state_msgs={} lint_status_msgs={}",
            stats.bg_msgs,
            stats.disk_usage_msgs,
            stats.git_info_msgs,
            stats.git_path_state_msgs,
            stats.lint_status_msgs
        ));
    }

    pub(super) fn poll_ci_fetches(&mut self) -> usize {
        let mut count = 0;
        while let Ok(msg) = self.ci_fetch_rx.try_recv() {
            match msg {
                CiFetchMsg::Complete { path, result, kind } => {
                    self.handle_ci_fetch_complete(path, result, kind);
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
                CleanMsg::Finished(task_id) => self.finish_task_toast(task_id),
            }
        }
    }

    pub(super) fn handle_disk_usage(&mut self, path: String, bytes: u64) {
        self.apply_disk_usage(path, bytes, self.is_scan_complete());
    }

    pub(super) fn handle_disk_usage_batch(&mut self, entries: Vec<(String, u64)>) {
        for (path, bytes) in entries {
            self.apply_disk_usage(path, bytes, false);
        }
    }

    pub(super) fn apply_disk_usage(
        &mut self,
        path: String,
        bytes: u64,
        refresh_git_path_state: bool,
    ) {
        self.fully_loaded.insert(path.clone());
        self.disk_usage.insert(path.clone(), bytes);
        if refresh_git_path_state {
            self.refresh_git_path_state(&path);
        }
        self.dirty.disk_cache.mark_dirty();
        self.dirty.fit_widths.mark_dirty();
        let mut lint_runtime_changed = false;
        if bytes == 0 {
            let abs = self
                .all_projects
                .iter()
                .find(|project| project.path == path)
                .map(|project| project.abs_path.as_str());
            if let Some(abs) = abs
                && !std::path::Path::new(abs).exists()
            {
                lint_runtime_changed |= self.deleted_projects.insert(path);
            }
        } else {
            lint_runtime_changed |= self.deleted_projects.remove(&path);
        }
        if lint_runtime_changed {
            self.sync_lint_runtime_projects();
        }
    }

    pub(super) fn handle_git_info(&mut self, path: String, info: GitInfo) {
        self.dirty.fit_widths.mark_dirty();
        let seen_path = path.clone();
        let preserved_first_commit = self
            .git_info
            .get(&path)
            .and_then(|existing| existing.first_commit.clone());
        let mut info = info;
        if info.first_commit.is_none() {
            info.first_commit = preserved_first_commit;
        }
        let matching_node = self
            .nodes
            .iter()
            .find(|node| node.project.path == path)
            .or_else(|| {
                self.nodes
                    .iter()
                    .flat_map(|node| node.worktrees.iter())
                    .find(|worktree| worktree.project.path == path)
            });
        if let Some(node) = matching_node {
            for member in Self::all_group_members(node) {
                // Always overwrite: the correct branch comes from the
                // workspace root, not from a stale propagation.
                self.git_info.insert(member.path.clone(), info.clone());
            }
            for worktree in &node.worktrees {
                self.git_info
                    .entry(worktree.project.path.clone())
                    .or_insert_with(|| info.clone());
            }
        }
        self.git_info.insert(path, info);
        if self.is_scan_complete() {
            self.scan.startup_phases.git_seen.insert(seen_path);
            self.maybe_log_startup_phase_completions();
        }
        self.dirty.finder.mark_dirty();
    }

    pub(super) fn handle_git_first_commit(&mut self, path: &str, first_commit: Option<String>) {
        let Some(info) = self.git_info.get_mut(path) else {
            return;
        };
        info.first_commit = first_commit;
    }

    pub(super) fn handle_repo_fetch_complete(&mut self, key: String) {
        self.scan.startup_phases.repo_seen.insert(key);
        self.maybe_log_startup_phase_completions();
    }

    pub(super) fn handle_repo_meta(
        &mut self,
        path: String,
        stars: u64,
        description: Option<String>,
    ) {
        if let Some(node) = self.nodes.iter().find(|node| node.project.path == path) {
            for member in Self::all_group_members(node) {
                self.stars.entry(member.path.clone()).or_insert(stars);
            }
        }
        self.stars.insert(path.clone(), stars);
        if let Some(desc) = description {
            self.repo_descriptions.insert(path, desc);
        }
    }

    pub(super) fn handle_project_discovered(&mut self, project: RustProject) -> bool {
        if self
            .all_projects
            .iter()
            .any(|existing| existing.path == project.path)
        {
            return false;
        }

        self.register_project_background_services(&project);
        self.all_projects.push(project);
        if self.is_scan_complete() {
            self.sync_lint_runtime_projects();
        }
        true
    }

    pub(super) fn handle_project_refreshed(&mut self, project: &RustProject) -> bool {
        let project_path = project.path.clone();
        let updated_in_all_projects = self
            .all_projects
            .iter_mut()
            .find(|existing| existing.path == project_path)
            .is_some_and(|existing| {
                *existing = project.clone();
                true
            });

        let updated_in_nodes = self.replace_project_in_nodes(&project_path, project);
        let updated = updated_in_all_projects || updated_in_nodes;

        if !updated {
            return false;
        }

        self.recompute_cargo_active_paths();
        self.prune_inactive_project_state();
        self.sync_lint_runtime_projects();
        self.cached_detail = None;
        self.dirty.finder.mark_dirty();
        self.dirty.rows.mark_dirty();
        self.dirty.fit_widths.mark_dirty();
        true
    }

    pub(super) fn replace_project_in_nodes(
        &mut self,
        project_path: &str,
        project: &RustProject,
    ) -> bool {
        let mut updated = false;

        for node in &mut self.nodes {
            updated |= super::snapshots::replace_project_in_node(node, project_path, project);
        }

        updated
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

    fn handle_disk_usage_msg(&mut self, path: String, bytes: u64) {
        self.scan.startup_phases.disk_seen.insert(path.clone());
        self.handle_disk_usage(path, bytes);
        self.maybe_log_startup_phase_completions();
    }

    fn handle_disk_usage_batch_msg(&mut self, root_path: String, entries: Vec<(String, u64)>) {
        self.data_generation += 1;
        if entries
            .iter()
            .any(|(path, _)| self.detail_path_is_affected(path))
        {
            self.detail_generation += 1;
        }
        self.scan.startup_phases.disk_seen.insert(root_path);
        self.handle_disk_usage_batch(entries);
        self.maybe_log_startup_phase_completions();
    }

    fn handle_git_path_state_msg(&mut self, path: String, state: GitPathState) {
        crate::perf_log::log_event(&format!(
            "app_git_path_state_applied path={} state={}",
            path,
            state.label()
        ));
        self.git_path_states.insert(path, state);
    }

    fn handle_crates_io_version_msg(&mut self, path: String, version: String, downloads: u64) {
        if self.is_cargo_active_path(&path) {
            self.crates_versions.insert(path.clone(), version);
            self.crates_downloads.insert(path, downloads);
        } else {
            self.crates_versions.remove(&path);
            self.crates_downloads.remove(&path);
        }
    }

    fn handle_lint_status_msg(&mut self, path: String, status: LintStatus) {
        let status_started = matches!(status, LintStatus::Running(_));
        let status_is_terminal = matches!(
            status,
            LintStatus::Passed(_) | LintStatus::Failed(_) | LintStatus::Stale | LintStatus::NoLog
        );
        if !self.is_cargo_active_path(&path) {
            self.port_report_runs.remove(&path);
            self.lint_status.remove(&path);
            return;
        }
        let eligible = self
            .all_projects
            .iter()
            .find(|project| project.path == path)
            .is_some_and(|project| {
                crate::lint::project_is_eligible(
                    &self.current_config.lint,
                    &project.path,
                    &PathBuf::from(&project.abs_path),
                    project.is_rust == Rust,
                )
            });
        if eligible {
            self.reload_port_report_history(&path);
            if matches!(status, LintStatus::NoLog) {
                self.lint_status.remove(&path);
            } else {
                self.lint_status.insert(path.clone(), status);
            }
        } else {
            self.port_report_runs.remove(&path);
            self.lint_status.remove(&path);
            self.running_lint_paths.remove(&path);
        }
        self.update_lint_rollups_for_path(&path);
        if status_started {
            self.running_lint_paths.insert(path.clone());
        }
        if status_is_terminal {
            self.running_lint_paths.remove(&path);
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
            if expected.insert(path.clone()) {
                self.scan.startup_phases.lint_complete_at = None;
            }
        }
        if status_is_terminal
            && self
                .scan
                .startup_phases
                .lint_expected
                .as_ref()
                .is_some_and(|expected| expected.contains(&path))
        {
            self.scan.startup_phases.lint_seen_terminal.insert(path);
        }
        self.maybe_log_startup_phase_completions();
    }

    fn handle_scan_complete_msg(&mut self) {
        let kind = if self.scan.run_count == 1 {
            "initial"
        } else {
            "rescan"
        };
        crate::perf_log::log_duration(
            "scan_complete",
            self.scan.started_at.elapsed(),
            &format!(
                "kind={} run={} projects={}",
                kind,
                self.scan.run_count,
                self.all_projects.len()
            ),
            0,
        );
        self.scan.phase = ScanPhase::Complete;
        self.initialize_startup_phase_tracker();
        self.sync_lint_runtime_projects_immediately();
        self.schedule_git_path_state_refreshes();
        self.schedule_git_first_commit_refreshes();
        if self.focused_pane == PaneId::ScanLog {
            self.focus_pane(PaneId::ProjectList);
        }
    }

    /// Handle a single `BackgroundMsg`. Returns `true` if the tree needs rebuilding.
    pub(super) fn handle_bg_msg(&mut self, msg: BackgroundMsg) -> bool {
        self.update_generations_for_msg(&msg);
        match msg {
            BackgroundMsg::DiskUsage { path, bytes } => self.handle_disk_usage_msg(path, bytes),
            BackgroundMsg::DiskUsageBatch { root_path, entries } => {
                self.handle_disk_usage_batch_msg(root_path, entries);
            },
            BackgroundMsg::LocalGitQueued { path } => {
                self.scan.startup_phases.git_expected.insert(path);
            },
            BackgroundMsg::CiRuns { path, runs } => self.insert_ci_runs(path, runs),
            BackgroundMsg::RepoFetchQueued { key } => {
                self.scan.startup_phases.repo_expected.insert(key);
            },
            BackgroundMsg::RepoFetchComplete { key } => self.handle_repo_fetch_complete(key),
            BackgroundMsg::GitInfo { path, info } => self.handle_git_info(path, info),
            BackgroundMsg::GitFirstCommit { path, first_commit } => {
                self.handle_git_first_commit(&path, first_commit);
            },
            BackgroundMsg::GitPathState { path, state } => {
                self.handle_git_path_state_msg(path, state);
            },
            BackgroundMsg::CratesIoVersion {
                path,
                version,
                downloads,
            } => self.handle_crates_io_version_msg(path, version, downloads),
            BackgroundMsg::RepoMeta {
                path,
                stars,
                description,
            } => self.handle_repo_meta(path, stars, description),
            BackgroundMsg::ProjectDiscovered { project } => {
                if self.handle_project_discovered(project) {
                    return true;
                }
            },
            BackgroundMsg::ProjectRefreshed { project } => {
                if self.handle_project_refreshed(&project) {
                    return true;
                }
            },
            BackgroundMsg::LintStatus { path, status } => self.handle_lint_status_msg(path, status),
            BackgroundMsg::ScanComplete => self.handle_scan_complete_msg(),
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

    pub(super) fn detail_path_is_affected(&self, path: &str) -> bool {
        let Some(project) = self.selected_project() else {
            return false;
        };
        self.selected_lint_rollup_key().map_or_else(
            || project.path == path,
            |key| {
                self.lint_rollup_paths
                    .get(&key)
                    .is_some_and(|paths| paths.iter().any(|candidate| candidate == path))
            },
        )
    }

    /// Spawn a priority fetch for the selected project if it hasn't been loaded yet.
    pub(super) fn maybe_priority_fetch(&mut self) {
        let Some(project) = self.selected_project() else {
            return;
        };
        let path = project.path.clone();
        let abs_path = project.abs_path.clone();
        let name = project.name.clone();
        if !self.fully_loaded.contains(&path) && self.priority_fetch_path.as_ref() != Some(&path) {
            self.priority_fetch_path = Some(path.clone());
            crate::tui::terminal::spawn_priority_fetch(self, &path, &abs_path, name.as_ref());
        }
    }
}
