use std::collections::HashSet;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;
use std::time::Instant;

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
use crate::project::Package;
use crate::project::Project;
use crate::project::ProjectListItem;
use crate::project::Visibility;
use crate::tui::detail::DetailField;
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

    pub fn start_clean(&mut self, project_path: &Path) {
        self.running_clean_paths.insert(project_path.to_path_buf());
        self.sync_running_clean_toast();
    }

    pub fn clean_spawn_failed(&mut self, project_path: &Path) {
        self.running_clean_paths.remove(project_path);
        self.sync_running_clean_toast();
    }

    pub fn dismiss_toast(&mut self, id: u64) {
        self.toasts.dismiss(id);
        self.prune_toasts();
    }

    pub fn lint_is_watchable(&self, path: &Path) -> bool {
        if !self.lint_enabled() {
            return false;
        }
        let is_rust = self
            .discovered_projects
            .iter()
            .any(|item| item.path() == path && item.is_rust());
        crate::lint::project_is_eligible(
            &self.current_config.lint,
            &path.to_string_lossy(),
            path,
            is_rust,
        )
    }

    pub fn bottom_panel_available(&self, path: &Path) -> bool {
        let has_ci = self.is_ci_owner_path(path)
            && (self
                .ci_state_for(path)
                .is_some_and(|state| !state.runs().is_empty())
                || self
                    .git_info
                    .get(path)
                    .is_some_and(|info| info.url.is_some()));
        let has_lint_runs = self
            .lint_runs
            .get(path)
            .is_some_and(|runs| !runs.is_empty())
            || self.lint_is_watchable(path);
        has_ci || has_lint_runs
    }

    pub fn sync_selected_project(&mut self) {
        self.ensure_visible_rows_cached();
        let current = self.selected_display_path();
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

        if let Some(display_path) = current
            && self.selection_paths.last_selected.as_ref() != Some(&display_path)
        {
            if let Some(abs_path) = self.selected_project_path().map(Path::to_path_buf) {
                self.reload_lint_history(&abs_path);
            }
            self.data_generation += 1;
            self.detail_generation += 1;
            self.selection_paths.last_selected = Some(display_path);
            self.mark_selection_changed();
            self.maybe_priority_fetch();
        }
    }

    pub fn is_deleted(&self, path: &Path) -> bool {
        use crate::project::Visibility;
        self.project_list_items
            .iter()
            .any(|item| item.has_project_with_visibility_by_path(path, Visibility::Deleted))
    }

    pub fn formatted_disk(&self, path: &Path) -> String {
        match self.disk_usage.get(path) {
            Some(&bytes) => crate::tui::render::format_bytes(bytes),
            None => crate::tui::render::format_bytes(0),
        }
    }

    pub fn selected_ci_path(&self) -> Option<&Path> {
        self.selected_project_path()
            .filter(|path| self.is_ci_owner_path(path))
    }

    pub fn selected_ci_state(&self) -> Option<&CiState> {
        let path = self.selected_ci_path()?;
        self.ci_state_for(path)
    }

    pub fn ci_for(&self, path: &Path) -> Option<Conclusion> {
        self.ci_state_for(path)
            .and_then(|_| self.latest_ci_run_for_path(path))
            .map(|run| run.conclusion)
    }

    pub fn ci_state_for(&self, path: &Path) -> Option<&CiState> {
        self.is_ci_owner_path(path)
            .then(|| self.ci_state.get(path))
            .flatten()
    }

    // ── ProjectListItem query methods ──────────────────────────────

    /// Count live (visible) worktree entries for a `ProjectListItem`.
    pub fn live_worktree_count_for_item(item: &ProjectListItem) -> usize {
        match item {
            ProjectListItem::WorkspaceWorktrees(wtg) => {
                let live = std::iter::once(wtg.primary().visibility())
                    .chain(wtg.linked().iter().map(crate::project::Project::visibility))
                    .filter(|v| !matches!(v, Visibility::Deleted | Visibility::Dismissed))
                    .count();
                if live <= 1 { 0 } else { live }
            },
            ProjectListItem::PackageWorktrees(wtg) => {
                let live = std::iter::once(wtg.primary().visibility())
                    .chain(wtg.linked().iter().map(crate::project::Project::visibility))
                    .filter(|v| !matches!(v, Visibility::Deleted | Visibility::Dismissed))
                    .count();
                if live <= 1 { 0 } else { live }
            },
            _ => 0,
        }
    }

    /// All absolute paths for a `ProjectListItem` (root + worktrees).
    fn unique_item_paths(item: &ProjectListItem) -> Vec<PathBuf> {
        let mut paths = Vec::new();
        paths.push(item.path().to_path_buf());
        match item {
            ProjectListItem::WorkspaceWorktrees(wtg) => {
                for linked in wtg.linked() {
                    let p = linked.path().to_path_buf();
                    if !paths.contains(&p) {
                        paths.push(p);
                    }
                }
            },
            ProjectListItem::PackageWorktrees(wtg) => {
                for linked in wtg.linked() {
                    let p = linked.path().to_path_buf();
                    if !paths.contains(&p) {
                        paths.push(p);
                    }
                }
            },
            _ => {},
        }
        paths
    }

    /// Aggregate disk usage for a `ProjectListItem`.
    pub fn formatted_disk_for_item(&self, item: &ProjectListItem) -> String {
        self.disk_bytes_for_item(item).map_or_else(
            || crate::tui::render::format_bytes(0),
            crate::tui::render::format_bytes,
        )
    }

    /// Get total disk bytes for a `ProjectListItem` (sum of root + worktrees).
    pub fn disk_bytes_for_item(&self, item: &ProjectListItem) -> Option<u64> {
        let paths = Self::unique_item_paths(item);
        if paths.len() == 1 {
            return self.disk_usage.get(&paths[0]).copied();
        }
        let mut total: u64 = 0;
        let mut any_data = false;
        for path in &paths {
            if let Some(&bytes) = self.disk_usage.get(path.as_path()) {
                total += bytes;
                any_data = true;
            }
        }
        if any_data { Some(total) } else { None }
    }

    /// Aggregate CI for a `ProjectListItem`.
    pub fn ci_for_item(&self, item: &ProjectListItem) -> Option<Conclusion> {
        let paths = Self::unique_item_paths(item);
        if paths.len() == 1 {
            return self.ci_for(&paths[0]);
        }
        let mut any_red = false;
        let mut all_green = true;
        let mut any_data = false;
        for path in &paths {
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

    pub fn animation_elapsed(&self) -> Duration { self.animation_started.elapsed() }

    pub fn is_vendored_path(&self, path: &str) -> bool {
        self.project_list_items.iter().any(|item| match item {
            ProjectListItem::Workspace(ws) => {
                ws.vendored().iter().any(|v| v.display_path() == path)
            },
            ProjectListItem::Package(pkg) => {
                pkg.vendored().iter().any(|v| v.display_path() == path)
            },
            ProjectListItem::WorkspaceWorktrees(wtg) => std::iter::once(wtg.primary())
                .chain(wtg.linked().iter())
                .any(|ws| ws.vendored().iter().any(|v| v.display_path() == path)),
            ProjectListItem::PackageWorktrees(wtg) => std::iter::once(wtg.primary())
                .chain(wtg.linked().iter())
                .any(|pkg| pkg.vendored().iter().any(|v| v.display_path() == path)),
            ProjectListItem::NonRust(_) => false,
        })
    }

    pub fn is_workspace_member_path(&self, path: &str) -> bool {
        self.project_list_items.iter().any(|item| match item {
            ProjectListItem::Workspace(ws) => ws
                .groups()
                .iter()
                .any(|g| g.members().iter().any(|m| m.display_path() == path)),
            ProjectListItem::WorkspaceWorktrees(wtg) => std::iter::once(wtg.primary())
                .chain(wtg.linked().iter())
                .any(|ws| {
                    ws.groups()
                        .iter()
                        .any(|g| g.members().iter().any(|m| m.display_path() == path))
                }),
            _ => false,
        })
    }

    pub fn recompute_cargo_active_paths(&mut self) {
        let mut active_paths: HashSet<PathBuf> = self
            .discovered_projects
            .iter()
            .filter(|item| !self.is_vendored_path(&item.display_path()))
            .map(|item| item.path().to_path_buf())
            .collect();

        // Include vendored projects whose parent is active.
        for item in &self.project_list_items {
            let vendored_paths: Vec<&Path> = match item {
                ProjectListItem::Workspace(ws) => {
                    ws.vendored().iter().map(Project::<Package>::path).collect()
                },
                ProjectListItem::Package(pkg) => pkg
                    .vendored()
                    .iter()
                    .map(Project::<Package>::path)
                    .collect(),
                ProjectListItem::WorkspaceWorktrees(wtg) => std::iter::once(wtg.primary())
                    .chain(wtg.linked().iter())
                    .flat_map(|ws| ws.vendored().iter().map(Project::<Package>::path))
                    .collect(),
                ProjectListItem::PackageWorktrees(wtg) => std::iter::once(wtg.primary())
                    .chain(wtg.linked().iter())
                    .flat_map(|pkg| pkg.vendored().iter().map(Project::<Package>::path))
                    .collect(),
                ProjectListItem::NonRust(_) => Vec::new(),
            };
            if active_paths.contains(item.path()) {
                for vp in vendored_paths {
                    active_paths.insert(vp.to_path_buf());
                }
            }
        }

        self.cargo_active_paths = active_paths;
    }

    pub fn is_cargo_active_path(&self, path: &Path) -> bool {
        self.cargo_active_paths.contains(path)
    }

    pub fn git_path_state_for(&self, path: &Path) -> GitPathState {
        self.git_path_states
            .get(path)
            .copied()
            .unwrap_or(GitPathState::OutsideRepo)
    }

    pub fn refresh_git_path_state(&mut self, path: &Path) {
        let state = crate::project::detect_git_path_state(path);
        self.git_path_states.insert(path.to_path_buf(), state);
    }

    pub fn prune_inactive_project_state(&mut self) {
        let all_paths: HashSet<PathBuf> = self
            .discovered_projects
            .iter()
            .map(|item| item.path().to_path_buf())
            .collect();
        self.git_path_states
            .retain(|path, _| all_paths.contains(path));
        for path in &all_paths {
            if self.is_cargo_active_path(path) {
                continue;
            }
            self.ci_state.remove(path);
            self.crates_versions.remove(path);
            self.crates_downloads.remove(path);
            self.lint_runs.remove(path);
            self.lint_status.remove(path);
        }
    }

    /// Formatted ahead/behind sync status for the project list columns.
    pub fn git_sync(&self, path: &Path) -> String {
        if matches!(
            self.git_path_state_for(path),
            GitPathState::Untracked | GitPathState::Ignored
        ) {
            return String::new();
        }
        let Some(info) = self.git_info.get(path) else {
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
            InputContext::ProjectList => Some("open"),
            InputContext::DetailTargets => Some("run"),
            InputContext::DetailFields => {
                let info = &self.cached_detail.as_ref()?.info;
                if self.base_focus() == PaneId::Package {
                    let fields = crate::tui::detail::package_fields(info);
                    let field = *fields.get(self.package_pane.pos())?;
                    if field.is_from_cargo_toml() {
                        Some("open")
                    } else {
                        None
                    }
                } else {
                    // Git column — Repo field opens URL
                    let fields = crate::tui::detail::git_fields(info);
                    match fields.get(self.git_pane.pos()) {
                        Some(DetailField::Repo) if info.git_url.is_some() => Some("open"),
                        _ => None,
                    }
                }
            },
            InputContext::CiRuns => {
                let ci_state = self
                    .selected_project_path()
                    .and_then(|path| self.ci_state_for(path));
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
