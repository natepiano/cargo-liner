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
use crate::project::GitInfo;
use crate::project::GitOrigin;
use crate::project::GitPathState;
use crate::project::Package;
use crate::project::RootItem;
use crate::project::RustProject;
use crate::project::Visibility;
use crate::tui::detail::DetailField;
use crate::tui::shortcuts::InputContext;
use crate::tui::toasts::ToastTaskId;
use crate::tui::toasts::ToastView;
use crate::tui::toasts::TrackedItem;
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
        let now = Instant::now();
        let linger = Duration::from_secs_f64(self.current_config.tui.task_linger_secs);
        self.toasts.prune_tracked_items(now, linger);
        self.toasts.prune(now);
        self.toast_pane.set_len(self.active_toasts().len());
        if self.base_focus() == PaneId::Toasts && self.active_toasts().is_empty() {
            self.focus_pane(PaneId::ProjectList);
        }
    }

    pub fn show_timed_toast(&mut self, title: impl Into<String>, body: impl Into<String>) {
        self.toasts.push_timed(title, body, self.toast_timeout(), 1);
        self.toast_pane.set_len(self.active_toasts().len());
    }

    pub fn start_task_toast(
        &mut self,
        title: impl Into<String>,
        body: impl Into<String>,
    ) -> ToastTaskId {
        let task_id = self.toasts.push_task(title, body, 1);
        self.toast_pane.set_len(self.active_toasts().len());
        task_id
    }

    pub fn finish_task_toast(&mut self, task_id: ToastTaskId) {
        // If tracked items remain, linger so strikethrough animation plays.
        // If no tracked items, exit immediately — nothing to animate.
        let linger = if self.toasts.tracked_item_count(task_id) > 0 {
            Duration::from_secs_f64(self.current_config.tui.task_linger_secs)
        } else {
            Duration::ZERO
        };
        self.toasts.finish_task(task_id, linger);
        self.prune_toasts();
    }

    pub fn set_task_tracked_items(&mut self, task_id: ToastTaskId, items: &[TrackedItem]) {
        let linger = Duration::from_secs_f64(self.current_config.tui.task_linger_secs);
        self.toasts.set_tracked_items(task_id, items, linger);
        self.toast_pane.set_len(self.active_toasts().len());
    }

    pub fn mark_tracked_item_completed(&mut self, task_id: ToastTaskId, label: &str) {
        self.toasts.mark_item_completed(task_id, label);
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
        let mut is_rust = false;
        self.projects.for_each_leaf_path(|p, rust| {
            if p == path {
                is_rust = rust;
            }
        });
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
                    .git_info_for(path)
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
        self.projects
            .at_path(path)
            .is_some_and(|project| project.visibility == Visibility::Deleted)
    }

    pub fn formatted_disk(&self, path: &Path) -> String {
        let bytes = self
            .projects
            .at_path(path)
            .and_then(|project| project.disk_usage_bytes)
            .unwrap_or(0);
        crate::tui::render::format_bytes(bytes)
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

    pub fn git_info_for(&self, path: &Path) -> Option<&GitInfo> {
        self.projects
            .at_path(path)
            .and_then(|project| project.git_info.as_ref())
    }

    // ── RootItem query methods ─────────────────────────────────────

    /// Count live (visible) worktree entries for a `RootItem`.
    pub fn live_worktree_count_for_item(item: &RootItem) -> usize {
        match item {
            RootItem::WorkspaceWorktrees(wtg) => {
                let live = std::iter::once(wtg.primary().visibility())
                    .chain(
                        wtg.linked()
                            .iter()
                            .map(crate::project::RustProject::visibility),
                    )
                    .filter(|v| !matches!(v, Visibility::Deleted | Visibility::Dismissed))
                    .count();
                if live <= 1 { 0 } else { live }
            },
            RootItem::PackageWorktrees(wtg) => {
                let live = std::iter::once(wtg.primary().visibility())
                    .chain(
                        wtg.linked()
                            .iter()
                            .map(crate::project::RustProject::visibility),
                    )
                    .filter(|v| !matches!(v, Visibility::Deleted | Visibility::Dismissed))
                    .count();
                if live <= 1 { 0 } else { live }
            },
            _ => 0,
        }
    }

    /// All absolute paths for a `RootItem` (root + worktrees).
    fn unique_item_paths(item: &RootItem) -> Vec<PathBuf> {
        let mut paths = Vec::new();
        paths.push(item.path().to_path_buf());
        match item {
            RootItem::WorkspaceWorktrees(wtg) => {
                for linked in wtg.linked() {
                    let p = linked.path().to_path_buf();
                    if !paths.contains(&p) {
                        paths.push(p);
                    }
                }
            },
            RootItem::PackageWorktrees(wtg) => {
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

    /// Aggregate disk usage for a `RootItem`.
    pub fn formatted_disk_for_item(item: &RootItem) -> String {
        item.disk_usage_bytes().map_or_else(
            || crate::tui::render::format_bytes(0),
            crate::tui::render::format_bytes,
        )
    }

    /// Aggregate CI for a `RootItem`.
    pub fn ci_for_item(&self, item: &RootItem) -> Option<Conclusion> {
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
        self.projects.iter().any(|item| match item {
            RootItem::Workspace(ws) => {
                ws.vendored().iter().any(|v| v.display_path() == path)
            },
            RootItem::Package(pkg) => {
                pkg.vendored().iter().any(|v| v.display_path() == path)
            },
            RootItem::WorkspaceWorktrees(wtg) => std::iter::once(wtg.primary())
                .chain(wtg.linked().iter())
                .any(|ws| ws.vendored().iter().any(|v| v.display_path() == path)),
            RootItem::PackageWorktrees(wtg) => std::iter::once(wtg.primary())
                .chain(wtg.linked().iter())
                .any(|pkg| pkg.vendored().iter().any(|v| v.display_path() == path)),
            RootItem::NonRust(_) => false,
        })
    }

    pub fn is_workspace_member_path(&self, path: &str) -> bool {
        self.projects.iter().any(|item| match item {
            RootItem::Workspace(ws) => ws
                .groups()
                .iter()
                .any(|g| g.members().iter().any(|m| m.display_path() == path)),
            RootItem::WorkspaceWorktrees(wtg) => std::iter::once(wtg.primary())
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
        let mut active_paths: HashSet<PathBuf> = HashSet::new();
        self.projects.for_each_leaf(|item| {
            if !self.is_vendored_path(&item.display_path()) {
                active_paths.insert(item.path().to_path_buf());
            }
        });

        // Include vendored projects whose parent is active.
        for item in &self.projects {
            let vendored_paths: Vec<&Path> = match item {
                RootItem::Workspace(ws) => ws
                    .vendored()
                    .iter()
                    .map(RustProject::<Package>::path)
                    .collect(),
                RootItem::Package(pkg) => pkg
                    .vendored()
                    .iter()
                    .map(RustProject::<Package>::path)
                    .collect(),
                RootItem::WorkspaceWorktrees(wtg) => std::iter::once(wtg.primary())
                    .chain(wtg.linked().iter())
                    .flat_map(|ws| ws.vendored().iter().map(RustProject::<Package>::path))
                    .collect(),
                RootItem::PackageWorktrees(wtg) => std::iter::once(wtg.primary())
                    .chain(wtg.linked().iter())
                    .flat_map(|pkg| pkg.vendored().iter().map(RustProject::<Package>::path))
                    .collect(),
                RootItem::NonRust(_) => Vec::new(),
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
        let mut all_paths: HashSet<PathBuf> = HashSet::new();
        self.projects.for_each_leaf_path(|path, _| {
            all_paths.insert(path.to_path_buf());
        });
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
        let Some(info) = self.git_info_for(path) else {
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
