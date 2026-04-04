use std::collections::HashMap;
use std::collections::HashSet;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;
use std::time::Instant;

use super::snapshots;
use super::types::App;
use super::types::CiState;
use crate::ci::Conclusion;
use crate::config::NavigationKeys;
use crate::config::NonRustInclusion;
use crate::config::ScrollDirection;
use crate::constants::IN_SYNC;
use crate::constants::SYNC_DOWN;
use crate::constants::SYNC_UP;
use crate::project::GitOrigin;
use crate::project::GitPathState;
use crate::project::ProjectLanguage::Rust;
use crate::project::RustProject;
use crate::scan::ProjectNode;
use crate::tui::detail::DetailField;
use crate::tui::detail::ProjectCounts;
use crate::tui::shortcuts::InputContext;
use crate::tui::toasts::ToastTaskId;
use crate::tui::toasts::ToastView;
use crate::tui::types::PaneId;

impl App {
    pub const fn lint_enabled(&self) -> bool { self.current_config.lint.enabled }

    pub const fn invert_scroll(&self) -> ScrollDirection { self.current_config.mouse.invert_scroll }

    pub const fn include_non_rust(&self) -> NonRustInclusion {
        self.current_config.tui.include_non_rust
    }

    pub const fn ci_run_count(&self) -> u32 { self.current_config.tui.ci_run_count }

    pub const fn navigation_keys(&self) -> NavigationKeys {
        self.current_config.tui.navigation_keys
    }

    pub fn editor(&self) -> &str { &self.current_config.tui.editor }

    fn toast_timeout(&self) -> Duration {
        Duration::from_secs_f64(self.current_config.tui.status_flash_secs)
    }

    pub fn active_toasts(&self) -> Vec<ToastView<'_>> { self.toasts.active(Instant::now()) }

    pub fn focused_toast_id(&self) -> Option<u64> {
        let active = self.active_toasts();
        active.get(self.toast_pane.pos()).map(ToastView::id)
    }

    pub fn prune_toasts(&mut self) {
        self.toasts.prune(Instant::now());
        self.toast_pane.set_len(self.active_toasts().len());
        if self.base_focus() == PaneId::Toasts && self.active_toasts().is_empty() {
            self.focus_pane(PaneId::ProjectList);
        }
    }

    pub fn show_timed_toast(&mut self, title: impl Into<String>, body: impl Into<String>) {
        self.toasts.push_timed(title, body, self.toast_timeout());
        self.toast_pane.set_len(self.active_toasts().len());
    }

    pub fn start_task_toast(
        &mut self,
        title: impl Into<String>,
        body: impl Into<String>,
    ) -> ToastTaskId {
        let task_id = self.toasts.push_task(title, body);
        self.toast_pane.set_len(self.active_toasts().len());
        task_id
    }

    pub fn finish_task_toast(&mut self, task_id: ToastTaskId) {
        self.toasts.finish_task(task_id);
        self.prune_toasts();
    }

    pub fn update_task_toast_body(&mut self, task_id: ToastTaskId, body: impl Into<String>) {
        self.toasts.update_task_body(task_id, body);
        self.toast_pane.set_len(self.active_toasts().len());
    }

    pub fn dismiss_focused_toast(&mut self) {
        if let Some(id) = self.focused_toast_id() {
            self.dismiss_toast(id);
        }
    }

    pub fn dismiss_toast(&mut self, id: u64) {
        self.toasts.dismiss(id);
        self.prune_toasts();
    }

    pub fn port_report_is_watchable(&self, project: &RustProject) -> bool {
        if !self.lint_enabled() {
            return false;
        }
        crate::lint::project_is_eligible(
            &self.current_config.lint,
            &project.path,
            &PathBuf::from(&project.abs_path),
            project.is_rust == Rust,
        )
    }

    pub fn bottom_panel_available(&self, project: &RustProject) -> bool {
        let has_ci = self.is_ci_owner_path(&project.path)
            && (self
                .ci_state_for(project)
                .is_some_and(|state| !state.runs().is_empty())
                || self
                    .git_info
                    .get(&project.path)
                    .is_some_and(|info| info.url.is_some()));
        let has_port_report = self
            .port_report_runs
            .get(&project.path)
            .is_some_and(|runs| !runs.is_empty())
            || self.port_report_is_watchable(project);
        has_ci || has_port_report
    }

    pub fn sync_selected_project(&mut self) {
        self.ensure_visible_rows_cached();
        let current = self.selected_project().map(|project| project.path.clone());
        if self
            .selection_paths
            .collapsed_anchor
            .as_ref()
            .is_some_and(|anchor| current.as_ref() != Some(anchor))
        {
            self.selection_paths.collapsed_selected = None;
            self.selection_paths.collapsed_anchor = None;
        }
        if self.selection_paths.selected_project == current {
            return;
        }

        self.selection_paths.selected_project.clone_from(&current);
        self.reset_project_panes();

        let panes = self.tabbable_panes();
        if !panes.contains(&self.base_focus()) {
            self.focus_pane(PaneId::ProjectList);
        }

        if self.return_focus.is_some() && !panes.contains(&self.return_focus.unwrap_or_default()) {
            self.return_focus = Some(PaneId::ProjectList);
        }

        if let Some(path) = current
            && self.selection_paths.last_selected.as_ref() != Some(&path)
        {
            self.reload_port_report_history(&path);
            self.data_generation += 1;
            self.detail_generation += 1;
            self.selection_paths.last_selected = Some(path);
            self.mark_selection_changed();
            self.maybe_priority_fetch();
        }
    }

    pub fn workspace_counts(&self, project: &RustProject) -> Option<ProjectCounts> {
        // Check top-level nodes first
        if let Some(node) = self.nodes.iter().find(|n| n.project.path == project.path)
            && node.has_members()
        {
            let mut counts = ProjectCounts::default();
            counts.add_project(&node.project);
            for member in Self::all_group_members(node) {
                counts.add_project(member);
            }
            return Some(counts);
        }
        // Check worktree entries (workspace worktrees have their own groups)
        for node in &self.nodes {
            for wt in &node.worktrees {
                if wt.project.path == project.path && wt.has_members() {
                    let mut counts = ProjectCounts::default();
                    counts.add_project(&wt.project);
                    for group in &wt.groups {
                        for member in &group.members {
                            counts.add_project(member);
                        }
                    }
                    return Some(counts);
                }
            }
        }
        None
    }

    pub fn is_deleted(&self, path: &str) -> bool { self.deleted_projects.contains(path) }

    pub fn live_worktree_count(&self, node: &ProjectNode) -> usize {
        node.worktrees
            .iter()
            .filter(|wt| !self.is_deleted(&wt.project.path))
            .count()
    }

    pub fn formatted_disk(&self, project: &RustProject) -> String {
        match self.disk_usage.get(&project.path) {
            Some(&bytes) => crate::tui::render::format_bytes(bytes),
            None => crate::tui::render::format_bytes(0),
        }
    }

    pub fn selected_ci_project(&self) -> Option<&RustProject> {
        self.selected_project()
            .filter(|project| self.is_ci_owner_path(&project.path))
    }

    pub fn selected_ci_state(&self) -> Option<&CiState> {
        self.selected_ci_project()
            .and_then(|project| self.ci_state.get(&project.path))
    }

    pub fn ci_for(&self, project: &RustProject) -> Option<Conclusion> {
        self.ci_state_for(project)
            .and_then(|_| self.latest_ci_run_for_path(&project.path))
            .map(|run| run.conclusion)
    }

    /// Aggregate disk usage for a node: sums the root and all worktrees.
    pub fn formatted_disk_for_node(&self, node: &ProjectNode) -> String {
        if node.worktrees.is_empty() {
            return self.formatted_disk(&node.project);
        }
        let mut total: u64 = 0;
        let mut any_data = false;
        for path in snapshots::unique_node_paths(node) {
            if let Some(&bytes) = self.disk_usage.get(path) {
                total += bytes;
                any_data = true;
            }
        }
        if any_data {
            crate::tui::render::format_bytes(total)
        } else {
            crate::tui::render::format_bytes(0)
        }
    }

    /// Get total disk bytes for a node (sum of root + worktrees).
    pub fn disk_bytes_for_node(&self, node: &ProjectNode) -> Option<u64> {
        if node.worktrees.is_empty() {
            return self.disk_usage.get(&node.project.path).copied();
        }
        let mut total: u64 = 0;
        let mut any_data = false;
        for path in snapshots::unique_node_paths(node) {
            if let Some(&bytes) = self.disk_usage.get(path) {
                total += bytes;
                any_data = true;
            }
        }
        if any_data { Some(total) } else { None }
    }

    /// Aggregate CI for a node: `Success` if all green, `Failure` if any red, `None` if no data.
    pub fn ci_for_node(&self, node: &ProjectNode) -> Option<Conclusion> {
        if node.worktrees.is_empty() {
            return self.ci_for(&node.project);
        }
        let mut any_red = false;
        let mut all_green = true;
        let mut any_data = false;
        for path in std::iter::once(&node.project.path)
            .chain(node.worktrees.iter().map(|wt| &wt.project.path))
        {
            if let Some(run) = self.latest_ci_run_for_path(path) {
                any_data = true;
                if run.conclusion.is_failure() {
                    any_red = true;
                    all_green = false;
                } else if !run.conclusion.is_success() {
                    all_green = false;
                }
            }
        }
        if !any_data {
            None
        } else if any_red {
            Some(Conclusion::Failure)
        } else if all_green {
            Some(Conclusion::Success)
        } else {
            None
        }
    }

    pub fn ci_state_for(&self, project: &RustProject) -> Option<&CiState> {
        self.is_ci_owner_path(&project.path)
            .then(|| self.ci_state.get(&project.path))
            .flatten()
    }

    pub fn animation_elapsed(&self) -> Duration { self.animation_started.elapsed() }

    pub fn is_vendored_path(&self, path: &str) -> bool {
        self.nodes.iter().any(|node| {
            node.vendored.iter().any(|project| project.path == path)
                || node
                    .worktrees
                    .iter()
                    .any(|worktree| worktree.vendored.iter().any(|project| project.path == path))
        })
    }

    pub fn is_workspace_member_path(&self, path: &str) -> bool {
        self.nodes.iter().any(|node| {
            node.project.is_workspace()
                && node
                    .groups
                    .iter()
                    .any(|group| group.members.iter().any(|member| member.path == path))
                || node.worktrees.iter().any(|worktree| {
                    worktree.project.is_workspace()
                        && worktree
                            .groups
                            .iter()
                            .any(|group| group.members.iter().any(|member| member.path == path))
                })
        })
    }

    pub fn project_by_path(&self, path: &str) -> Option<&RustProject> {
        self.all_projects
            .iter()
            .find(|project| project.path == path)
    }

    pub fn recompute_cargo_active_paths(&mut self) {
        let project_index: HashMap<String, Vec<String>> = self
            .all_projects
            .iter()
            .map(|project| (project.path.clone(), project.local_dependency_paths.clone()))
            .collect();
        let mut active_paths: HashSet<String> = self
            .all_projects
            .iter()
            .filter(|project| !self.is_vendored_path(&project.path))
            .map(|project| project.path.clone())
            .collect();
        let mut frontier: Vec<String> = active_paths.iter().cloned().collect();

        while let Some(path) = frontier.pop() {
            let Some(dependencies) = project_index.get(&path) else {
                continue;
            };
            for dependency_path in dependencies {
                if project_index.contains_key(dependency_path)
                    && active_paths.insert(dependency_path.clone())
                {
                    frontier.push(dependency_path.clone());
                }
            }
        }

        self.cargo_active_paths = active_paths;
    }

    pub fn is_cargo_active_path(&self, path: &str) -> bool {
        self.cargo_active_paths.contains(path)
    }

    pub fn git_path_state_for(&self, path: &str) -> GitPathState {
        self.git_path_states
            .get(path)
            .copied()
            .unwrap_or(GitPathState::OutsideRepo)
    }

    pub fn refresh_git_path_state(&mut self, path: &str) {
        let Some(project) = self.project_by_path(path) else {
            self.git_path_states.remove(path);
            return;
        };
        let state = crate::project::detect_git_path_state(Path::new(&project.abs_path));
        self.git_path_states.insert(path.to_string(), state);
    }

    pub fn prune_inactive_project_state(&mut self) {
        let all_paths: HashSet<String> = self
            .all_projects
            .iter()
            .map(|project| project.path.clone())
            .collect();
        self.git_path_states
            .retain(|path, _| all_paths.contains(path));
        for path in all_paths {
            if self.is_cargo_active_path(&path) {
                continue;
            }
            self.ci_state.remove(&path);
            self.crates_versions.remove(&path);
            self.crates_downloads.remove(&path);
            self.port_report_runs.remove(&path);
            self.lint_status.remove(&path);
        }
    }

    /// Formatted ahead/behind sync status for the project list columns.
    pub fn git_sync(&self, project: &RustProject) -> String {
        if matches!(
            self.git_path_state_for(&project.path),
            GitPathState::Untracked | GitPathState::Ignored
        ) {
            return String::new();
        }
        let Some(info) = self.git_info.get(&project.path) else {
            return String::new();
        };
        match info.ahead_behind {
            Some((0, 0)) => IN_SYNC.to_string(),
            Some((a, 0)) => format!("{SYNC_UP}{a}"),
            Some((0, b)) => format!("{SYNC_DOWN}{b}"),
            Some((a, b)) => format!("{SYNC_UP}{a}{SYNC_DOWN}{b}"),
            // No upstream but has a remote — branch not published.
            None if info.origin != GitOrigin::Local => "-".to_string(),
            None => String::new(),
        }
    }

    /// Returns the Enter-key action label for the current cursor position,
    /// or `None` if Enter does nothing here. Used by the shortcut bar to
    /// only show Enter when it's actionable.
    pub fn enter_action(&self) -> Option<&'static str> {
        match self.input_context() {
            InputContext::ProjectList | InputContext::ScanLog => Some("open"),
            InputContext::DetailTargets => Some("run"),
            InputContext::DetailFields => {
                if self.base_focus() == PaneId::Package {
                    let info = self
                        .selected_project()
                        .map(|p| crate::tui::detail::build_detail_info(self, p))?;
                    let fields = crate::tui::detail::package_fields(&info);
                    let field = *fields.get(self.package_pane.pos())?;
                    if field.is_from_cargo_toml() {
                        Some("open")
                    } else {
                        None
                    }
                } else {
                    // Git column — Repo field opens URL
                    let info = self
                        .selected_project()
                        .map(|p| crate::tui::detail::build_detail_info(self, p))?;
                    let fields = crate::tui::detail::git_fields(&info);
                    match fields.get(self.git_pane.pos()) {
                        Some(DetailField::Repo) if info.git_url.is_some() => Some("open"),
                        _ => None,
                    }
                }
            },
            InputContext::CiRuns => {
                let ci_state = self.selected_project().and_then(|p| self.ci_state_for(p));
                let run_count = ci_state.map_or(0, |s| s.runs().len());
                if self.ci_pane.pos() == run_count
                    && !ci_state.is_some_and(CiState::is_fetching)
                    && !ci_state.is_some_and(CiState::is_exhausted)
                {
                    Some("fetch")
                } else {
                    None
                }
            },
            _ => None,
        }
    }
}
