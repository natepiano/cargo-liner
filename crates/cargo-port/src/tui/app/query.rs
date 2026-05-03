use std::collections::HashSet;
use std::path::Path;
use std::time::Duration;
use std::time::Instant;

use super::App;
use super::types::DiscoveryRowKind;
use super::types::DiscoveryShimmer;
use crate::ci;
use crate::ci::CiRun;
use crate::ci::Conclusion;
use crate::config::NavigationKeys;
use crate::config::NonRustInclusion;
use crate::config::ScrollDirection;
use crate::constants::IN_SYNC;
use crate::constants::NO_REMOTE_SYNC;
use crate::constants::SYNC_DOWN;
use crate::constants::SYNC_UP;
use crate::project;
use crate::project::AbsolutePath;
use crate::project::CheckoutInfo;
use crate::project::GitStatus;
use crate::project::Package;
use crate::project::ProjectCiData;
use crate::project::ProjectCiInfo;
use crate::project::ProjectFields;
use crate::project::RepoInfo;
use crate::project::RootItem;
use crate::project::RustProject;
use crate::project::Visibility;
use crate::project::Workspace;
use crate::project::WorktreeGroup;
use crate::tui::columns;
use crate::tui::panes;
use crate::tui::panes::DetailField;
use crate::tui::panes::PaneId;
use crate::tui::render;
use crate::tui::shortcuts::InputContext;
use crate::tui::toasts::ToastStyle::Warning;
use crate::tui::toasts::ToastTaskId;
use crate::tui::toasts::ToastView;
use crate::tui::toasts::TrackedItem;

impl App {
    pub const fn lint_enabled(&self) -> bool { self.config.current().lint.enabled }

    pub const fn invert_scroll(&self) -> ScrollDirection {
        self.config.current().mouse.invert_scroll
    }

    pub const fn include_non_rust(&self) -> NonRustInclusion {
        self.config.current().tui.include_non_rust
    }

    pub const fn ci_run_count(&self) -> u32 { self.config.current().tui.ci_run_count }

    pub const fn navigation_keys(&self) -> NavigationKeys {
        self.config.current().tui.navigation_keys
    }

    pub fn editor(&self) -> &str { &self.config.current().tui.editor }

    pub fn terminal_command(&self) -> &str { &self.config.current().tui.terminal_command }

    pub fn terminal_command_configured(&self) -> bool { !self.terminal_command().trim().is_empty() }

    fn toast_timeout(&self) -> Duration {
        Duration::from_secs_f64(self.config.current().tui.status_flash_secs)
    }

    pub fn active_toasts(&self) -> Vec<ToastView<'_>> { self.toasts.active(Instant::now()) }

    pub fn focused_toast_id(&self) -> Option<u64> {
        let active = self.active_toasts();
        active
            .get(self.panes().toasts().viewport().pos())
            .map(ToastView::id)
    }

    #[cfg(test)]
    pub fn toasts_is_alive_for_test(&self, id: u64) -> bool { self.toasts.is_alive(id) }

    pub fn prune_toasts(&mut self) {
        let now = Instant::now();
        let linger = Duration::from_secs_f64(self.config.current().tui.task_linger_secs);
        self.toasts.prune_tracked_items(now, linger);
        self.toasts.prune(now);
        let toast_len = self.active_toasts().len();
        self.panes_mut()
            .toasts_mut()
            .viewport_mut()
            .set_len(toast_len);
        if self.base_focus() == PaneId::Toasts && self.active_toasts().is_empty() {
            self.focus_pane(PaneId::ProjectList);
        }
    }

    pub fn show_timed_toast(&mut self, title: impl Into<String>, body: impl Into<String>) {
        self.toasts.push_timed(title, body, self.toast_timeout(), 1);
        let toast_len = self.active_toasts().len();
        self.panes_mut()
            .toasts_mut()
            .viewport_mut()
            .set_len(toast_len);
    }

    pub fn show_timed_warning_toast(&mut self, title: impl Into<String>, body: impl Into<String>) {
        self.toasts
            .push_timed_styled(title, body, self.toast_timeout(), 1, Warning);
        let toast_len = self.active_toasts().len();
        self.panes_mut()
            .toasts_mut()
            .viewport_mut()
            .set_len(toast_len);
    }

    pub fn start_task_toast(
        &mut self,
        title: impl Into<String>,
        body: impl Into<String>,
    ) -> ToastTaskId {
        let task_id = self.toasts.push_task(title, body, 1);
        let toast_len = self.active_toasts().len();
        self.panes_mut()
            .toasts_mut()
            .viewport_mut()
            .set_len(toast_len);
        task_id
    }

    pub fn finish_task_toast(&mut self, task_id: ToastTaskId) {
        // If tracked items remain, linger so strikethrough animation plays.
        // If no tracked items, exit immediately — nothing to animate.
        let linger = if self.toasts.tracked_item_count(task_id) > 0 {
            Duration::from_secs_f64(self.config.current().tui.task_linger_secs)
        } else {
            Duration::ZERO
        };
        self.toasts.finish_task(task_id, linger);
        self.prune_toasts();
    }

    pub fn set_task_tracked_items(&mut self, task_id: ToastTaskId, items: &[TrackedItem]) {
        let linger = Duration::from_secs_f64(self.config.current().tui.task_linger_secs);
        self.toasts.set_tracked_items(task_id, items, linger);
        let toast_len = self.active_toasts().len();
        self.panes_mut()
            .toasts_mut()
            .viewport_mut()
            .set_len(toast_len);
    }

    pub fn mark_tracked_item_completed(&mut self, task_id: ToastTaskId, key: &str) {
        self.toasts.mark_item_completed(task_id, key);
        let toast_len = self.active_toasts().len();
        self.panes_mut()
            .toasts_mut()
            .viewport_mut()
            .set_len(toast_len);
    }

    /// Begin a clean for `project_path`. Returns `true` if a cargo clean
    /// should be spawned; `false` when the project is already clean,
    /// in which case a timed "Already clean" toast is shown and no
    /// spinner is started.
    ///
    /// Honors the workspace's resolved `target_directory` from the
    /// metadata store (design plan → **Call-site migrations → step 2**):
    /// a project redirected via `CARGO_TARGET_DIR` or
    /// `.cargo/config.toml`'s `build.target-dir` is cleaned at the real
    /// location. Falls back to `<project>/target` when no snapshot
    /// covers `project_path` yet.
    pub fn start_clean(&mut self, project_path: &AbsolutePath) -> bool {
        let target_dir = self
            .resolve_target_dir(project_path)
            .unwrap_or_else(|| AbsolutePath::from(project_path.as_path().join("target")));
        if !target_dir.as_path().exists() {
            let name = project::home_relative_path(project_path.as_path());
            self.show_timed_toast("Already clean", name);
            return false;
        }
        self.inflight
            .clean_mut()
            .insert(project_path.clone(), Instant::now());
        self.sync_running_clean_toast();
        true
    }

    pub fn clean_spawn_failed(&mut self, project_path: &AbsolutePath) {
        self.inflight.clean_mut().remove(project_path.as_path());
        self.sync_running_clean_toast();
    }

    pub fn dismiss_toast(&mut self, id: u64) {
        self.toasts.dismiss(id);
        self.prune_toasts();
    }

    pub fn sync_selected_project(&mut self) {
        self.ensure_visible_rows_cached();
        let current = self.selected_project_path().map(AbsolutePath::from);
        if self
            .selection
            .paths()
            .collapsed_anchor
            .as_ref()
            .is_some_and(|anchor| current.as_ref() != Some(anchor))
        {
            self.selection.paths_mut().collapsed_selected = None;
            self.selection.paths_mut().collapsed_anchor = None;
        }
        if self.selection.paths_mut().selected_project == current {
            return;
        }

        self.selection
            .paths_mut()
            .selected_project
            .clone_from(&current);
        self.reset_project_panes();

        let panes = self.tabbable_panes();
        if !panes.contains(&self.base_focus()) {
            self.focus_pane(PaneId::ProjectList);
        }

        if self.return_focus.is_some() && !panes.contains(&self.return_focus.unwrap_or_default()) {
            self.return_focus = Some(PaneId::ProjectList);
        }

        if let Some(abs_path) = current
            && self.selection.paths_mut().last_selected.as_ref() != Some(&abs_path)
        {
            self.scan.bump_generation();
            self.selection.paths_mut().last_selected = Some(abs_path);
            self.mark_selection_changed();
            self.maybe_priority_fetch();
        }
    }

    pub fn is_deleted(&self, path: &Path) -> bool {
        use crate::project::Visibility;
        self.projects()
            .at_path(path)
            .is_some_and(|project| project.visibility == Visibility::Deleted)
    }

    pub fn selected_project_is_deleted(&self) -> bool {
        self.selected_project_path()
            .is_some_and(|path| self.is_deleted(path))
    }

    pub fn formatted_disk(&self, path: &Path) -> String {
        let bytes = self
            .projects()
            .at_path(path)
            .and_then(|project| project.disk_usage_bytes)
            .unwrap_or(0);
        render::format_bytes(bytes)
    }

    pub fn selected_ci_path(&self) -> Option<AbsolutePath> {
        let path = self.selected_project_path()?;
        let entry = self.projects().entry_containing(path)?;
        Some(entry.item.path().clone())
    }

    pub fn selected_ci_runs(&self) -> Vec<CiRun> {
        self.selected_project_path()
            .map_or_else(Vec::new, |path| self.ci_runs_for_display(path))
    }

    pub fn unpublished_ci_branch_name(&self, path: &Path) -> Option<String> {
        let git = self.git_info_for(path)?;
        let default_branch = self
            .repo_info_for(path)
            .and_then(|repo| repo.default_branch.as_deref());
        (git.primary_tracked_ref().is_none() && git.branch.as_deref() != default_branch)
            .then(|| git.branch.clone())
            .flatten()
    }

    pub fn ci_for(&self, path: &Path) -> Option<Conclusion> {
        // A branch with no upstream tracking can't have CI runs — don't
        // show the parent repo's result for an unpushed worktree branch.
        if self.unpublished_ci_branch_name(path).is_some() {
            return None;
        }
        self.ci_info_for(path)
            .and_then(|_| self.latest_ci_run_for_path(path))
            .map(|run| run.conclusion)
    }

    pub fn ci_data_for(&self, path: &Path) -> Option<&ProjectCiData> {
        self.projects()
            .entry_containing(path)
            .and_then(|entry| entry.git_repo.as_ref())
            .map(|repo| &repo.ci_data)
    }

    pub fn ci_info_for(&self, path: &Path) -> Option<&ProjectCiInfo> {
        self.ci_data_for(path).and_then(ProjectCiData::info)
    }

    pub fn ci_is_fetching(&self, path: &Path) -> bool {
        self.projects().entry_containing(path).is_some_and(|entry| {
            self.inflight
                .ci_fetch_tracker()
                .is_fetching(entry.item.path().as_path())
        })
    }

    pub fn ci_is_exhausted(&self, path: &Path) -> bool {
        self.ci_data_for(path)
            .is_some_and(ProjectCiData::is_exhausted)
    }

    pub fn git_info_for(&self, path: &Path) -> Option<&CheckoutInfo> {
        self.projects()
            .at_path(path)
            .and_then(|project| project.local_git_state.info())
    }

    /// Per-repo info (remotes, workflows, default branch, ...) for the
    /// entry containing `path`. `None` means either the path isn't in a
    /// known entry, the entry isn't in a git repo, or the background
    /// `LocalGitInfo::get` call hasn't completed yet.
    pub fn repo_info_for(&self, path: &Path) -> Option<&RepoInfo> {
        self.projects()
            .entry_containing(path)
            .and_then(|entry| entry.git_repo.as_ref()?.repo_info.as_ref())
    }

    /// Convenience: the primary remote's URL for the checkout at `path`,
    /// looked up against its containing entry's `RepoInfo`.
    pub fn primary_url_for(&self, path: &Path) -> Option<&str> {
        let checkout = self.git_info_for(path)?;
        let repo = self.repo_info_for(path)?;
        checkout.primary_url(repo)
    }

    /// Convenience: the primary remote's ahead/behind for the checkout
    /// at `path`.
    pub fn primary_ahead_behind_for(&self, path: &Path) -> Option<(usize, usize)> {
        let checkout = self.git_info_for(path)?;
        let repo = self.repo_info_for(path)?;
        checkout.primary_ahead_behind(repo)
    }

    /// Pick a remote URL to drive the GitHub fetch for the entry
    /// containing `path`. Independent of the current checkout's
    /// upstream tracking: a worktree on a branch without an upstream
    /// still belongs to the repo and should fetch repo-level metadata.
    /// Preference order: `upstream`, then `origin`, then the first
    /// remote with a parseable owner/repo URL.
    pub fn fetch_url_for(&self, path: &Path) -> Option<String> {
        let repo = self.repo_info_for(path)?;
        let parseable = |name: &str| {
            repo.remotes
                .iter()
                .find(|r| r.name == name)
                .and_then(|r| r.url.as_deref())
                .filter(|url| ci::parse_owner_repo(url).is_some())
        };
        parseable("upstream")
            .or_else(|| parseable("origin"))
            .or_else(|| {
                repo.remotes.iter().find_map(|r| {
                    let url = r.url.as_deref()?;
                    ci::parse_owner_repo(url).map(|_| url)
                })
            })
            .map(String::from)
    }

    pub fn is_rust_at_path(&self, path: &Path) -> bool {
        self.projects().iter().any(|item| {
            if item
                .submodules()
                .iter()
                .any(|submodule| submodule.path.as_path() == path)
            {
                return false;
            }
            (item.path() == path || item.at_path(path).is_some()) && item.is_rust()
        })
    }

    // ── RootItem query methods ─────────────────────────────────────

    /// All absolute paths for a `RootItem` (root + worktrees).
    fn unique_item_paths(item: &RootItem) -> Vec<AbsolutePath> {
        let mut paths = Vec::new();
        paths.push(item.path().clone());
        match item {
            RootItem::Worktrees(WorktreeGroup::Workspaces { linked, .. }) => {
                for l in linked {
                    let p = l.path().clone();
                    if !paths.contains(&p) {
                        paths.push(p);
                    }
                }
            },
            RootItem::Worktrees(WorktreeGroup::Packages { linked, .. }) => {
                for l in linked {
                    let p = l.path().clone();
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
        item.disk_usage_bytes()
            .map_or_else(|| render::format_bytes(0), render::format_bytes)
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

    pub fn discovery_shimmer_enabled(&self) -> bool {
        self.config.current().tui.discovery_shimmer_secs > 0.0
    }

    pub fn discovery_shimmer_duration(&self) -> Duration {
        Duration::from_secs_f64(self.config.current().tui.discovery_shimmer_secs)
    }

    pub fn register_discovery_shimmer(&mut self, path: &Path) {
        if !self.is_scan_complete() || !self.discovery_shimmer_enabled() {
            return;
        }
        let shimmer = DiscoveryShimmer::new(Instant::now(), self.discovery_shimmer_duration());
        self.scan
            .discovery_shimmers_mut()
            .insert(AbsolutePath::from(path), shimmer);
    }

    pub fn prune_discovery_shimmers(&mut self, now: Instant) {
        self.scan
            .discovery_shimmers_mut()
            .retain(|_, shimmer| shimmer.is_active_at(now));
    }

    pub fn discovery_name_segments_for_path(
        &self,
        row_path: &Path,
        name: &str,
        git_status: Option<GitStatus>,
        row_kind: DiscoveryRowKind,
    ) -> Option<Vec<columns::StyledSegment>> {
        if !self.discovery_shimmer_enabled() {
            return None;
        }
        let now = Instant::now();
        let (session_path, shimmer) =
            self.discovery_shimmer_session_for_path(row_path, now, row_kind)?;
        let char_count = name.chars().count();
        if char_count == 0 {
            return None;
        }

        let base_style = columns::project_name_style(git_status);
        let accent_style = columns::project_name_shimmer_style(git_status);
        let window = discovery_shimmer_window_len(char_count);
        let elapsed_ms = usize::try_from(now.duration_since(shimmer.started_at).as_millis())
            .unwrap_or(usize::MAX);
        let step = elapsed_ms / discovery_shimmer_step_millis();
        let head = (step
            + discovery_shimmer_phase_offset(
                session_path.as_path(),
                row_path,
                row_kind,
                char_count,
            ))
            % char_count;

        Some(columns::build_shimmer_segments(
            name,
            base_style,
            accent_style,
            head,
            window,
        ))
    }

    fn discovery_shimmer_session_for_path(
        &self,
        row_path: &Path,
        now: Instant,
        row_kind: DiscoveryRowKind,
    ) -> Option<(AbsolutePath, DiscoveryShimmer)> {
        self.scan
            .discovery_shimmers()
            .iter()
            .filter(|(session_path, shimmer)| {
                shimmer.is_active_at(now)
                    && self.discovery_shimmer_session_matches(
                        session_path.as_path(),
                        row_path,
                        row_kind,
                    )
            })
            .max_by_key(|(_, shimmer)| shimmer.started_at)
            .map(|(session_path, shimmer)| (session_path.clone(), *shimmer))
    }

    fn discovery_shimmer_session_matches(
        &self,
        session_path: &Path,
        row_path: &Path,
        row_kind: DiscoveryRowKind,
    ) -> bool {
        self.discovery_scope_contains(session_path, row_path)
            || self
                .discovery_parent_row(session_path)
                .is_some_and(|parent| {
                    parent.path.as_path() == row_path && row_kind.allows_parent_kind(parent.kind)
                })
    }

    fn discovery_scope_contains(&self, session_path: &Path, row_path: &Path) -> bool {
        self.projects()
            .iter()
            .any(|item| root_item_scope_contains(item, session_path, row_path))
    }

    fn discovery_parent_row(&self, session_path: &Path) -> Option<DiscoveryParentRow> {
        self.projects()
            .iter()
            .find_map(|item| root_item_parent_row(item, session_path))
    }

    pub fn is_vendored_path(&self, path: &Path) -> bool {
        self.projects().iter().any(|item| match &item.item {
            RootItem::Rust(RustProject::Workspace(ws)) => {
                ws.vendored().iter().any(|v| v.path() == path)
            },
            RootItem::Rust(RustProject::Package(pkg)) => {
                pkg.vendored().iter().any(|v| v.path() == path)
            },
            RootItem::Worktrees(WorktreeGroup::Workspaces {
                primary, linked, ..
            }) => std::iter::once(primary)
                .chain(linked.iter())
                .any(|ws| ws.vendored().iter().any(|v| v.path() == path)),
            RootItem::Worktrees(WorktreeGroup::Packages {
                primary, linked, ..
            }) => std::iter::once(primary)
                .chain(linked.iter())
                .any(|pkg| pkg.vendored().iter().any(|v| v.path() == path)),
            RootItem::NonRust(_) => false,
        })
    }

    pub fn is_workspace_member_path(&self, path: &Path) -> bool {
        self.projects().iter().any(|item| match &item.item {
            RootItem::Rust(RustProject::Workspace(ws)) => ws
                .groups()
                .iter()
                .any(|g| g.members().iter().any(|m| m.path() == path)),
            RootItem::Worktrees(WorktreeGroup::Workspaces {
                primary, linked, ..
            }) => std::iter::once(primary).chain(linked.iter()).any(|ws| {
                ws.groups()
                    .iter()
                    .any(|g| g.members().iter().any(|m| m.path() == path))
            }),
            _ => false,
        })
    }

    pub fn git_status_for(&self, path: &Path) -> Option<GitStatus> {
        self.git_info_for(path).map(|info| info.status)
    }

    /// Roll up the worst git path state across all **visible** children of a
    /// `RootItem`.  For worktree groups, checks primary + non-dismissed linked
    /// entries.  For everything else, returns the state for the single path.
    pub fn git_status_for_item(&self, item: &RootItem) -> Option<GitStatus> {
        match item {
            RootItem::Worktrees(g) => {
                let states: Box<dyn Iterator<Item = Option<GitStatus>>> = match g {
                    WorktreeGroup::Workspaces {
                        primary, linked, ..
                    } => Box::new(
                        std::iter::once(self.git_status_for(primary.path())).chain(
                            linked
                                .iter()
                                .filter(|l| l.visibility() == Visibility::Visible)
                                .map(|l| self.git_status_for(l.path())),
                        ),
                    ),
                    WorktreeGroup::Packages {
                        primary, linked, ..
                    } => Box::new(
                        std::iter::once(self.git_status_for(primary.path())).chain(
                            linked
                                .iter()
                                .filter(|l| l.visibility() == Visibility::Visible)
                                .map(|l| self.git_status_for(l.path())),
                        ),
                    ),
                };
                worst_git_status(states)
            },
            _ => self.git_status_for(item.path()),
        }
    }

    pub fn prune_inactive_project_state(&mut self) {
        let mut all_paths: HashSet<AbsolutePath> = HashSet::new();
        self.projects().for_each_leaf_path(|path, _| {
            all_paths.insert(AbsolutePath::from(path));
        });
        self.scan
            .pending_git_first_commit_mut()
            .retain(|path, _| all_paths.contains(path));
        self.inflight
            .ci_fetch_tracker_mut()
            .retain(|path| all_paths.contains(path));
    }

    /// Formatted ahead/behind sync status for the project list columns.
    pub fn git_sync(&self, path: &Path) -> String {
        let Some(info) = self.git_info_for(path) else {
            return String::new();
        };
        if matches!(info.status, GitStatus::Untracked | GitStatus::Ignored) {
            return String::new();
        }
        match self.primary_ahead_behind_for(path) {
            Some((0, 0)) => IN_SYNC.to_string(),
            Some((a, 0)) => format!("{SYNC_UP}{a}"),
            Some((0, b)) => format!("{SYNC_DOWN}{b}"),
            Some((a, b)) => format!("{SYNC_UP}{a}{SYNC_DOWN}{b}"),
            // No upstream tracking branch: render a flat placeholder in the O column.
            None => NO_REMOTE_SYNC.to_string(),
        }
    }

    pub fn git_main(&self, path: &Path) -> String {
        let Some(info) = self.git_info_for(path) else {
            return String::new();
        };
        if matches!(info.status, GitStatus::Untracked | GitStatus::Ignored) {
            return String::new();
        }
        match info.ahead_behind_local {
            Some((0, 0)) => IN_SYNC.to_string(),
            Some((a, 0)) => format!("{SYNC_UP}{a}"),
            Some((0, b)) => format!("{SYNC_DOWN}{b}"),
            Some((a, b)) => format!("{SYNC_UP}{a}{SYNC_DOWN}{b}"),
            None => String::new(),
        }
    }

    /// Returns the Enter-key action label for the current cursor position,
    /// or `None` if Enter does nothing here. Used by the shortcut bar to
    /// only show Enter when it's actionable.
    pub fn enter_action(&self) -> Option<&'static str> {
        match self.input_context() {
            InputContext::DetailTargets => Some("run"),
            InputContext::DetailFields => {
                if self.base_focus() == PaneId::Package {
                    let pkg = self.panes().package().content()?;
                    let fields = panes::package_fields_from_data(pkg);
                    let field = *fields.get(self.panes().package().viewport().pos())?;
                    if field == DetailField::CratesIo && pkg.crates_version.is_some() {
                        Some("open")
                    } else {
                        None
                    }
                } else {
                    let git = self.panes().git().content()?;
                    let pos = self.panes().git().viewport().pos();
                    match panes::git_row_at(git, pos) {
                        Some(panes::GitRow::Remote(remote)) if remote.full_url.is_some() => {
                            Some("open")
                        },
                        _ => None,
                    }
                }
            },
            InputContext::CiRuns => {
                let ci_info = self
                    .selected_project_path()
                    .and_then(|path| self.ci_info_for(path));
                let run_count = ci_info.map_or(0, |info| info.runs.len());
                let selected_path = self.selected_project_path();
                if self.panes().ci().viewport().pos() == run_count
                    && !selected_path.is_some_and(|path| self.ci_is_fetching(path))
                    && !selected_path.is_some_and(|path| self.ci_is_exhausted(path))
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

const fn discovery_shimmer_window_len(char_count: usize) -> usize {
    match char_count {
        0 => 0,
        1..=2 => 1,
        3..=5 => 2,
        6..=8 => 3,
        _ => 4,
    }
}

const fn discovery_shimmer_step_millis() -> usize { 85 }

fn discovery_shimmer_phase_offset(
    session_path: &Path,
    row_path: &Path,
    row_kind: DiscoveryRowKind,
    char_count: usize,
) -> usize {
    if char_count == 0 {
        return 0;
    }
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    let key = format!(
        "{}|{}|{}",
        session_path.to_string_lossy(),
        row_path.to_string_lossy(),
        row_kind.discriminant()
    );
    for byte in key.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    usize::try_from(hash % u64::try_from(char_count).unwrap_or(1)).unwrap_or(0)
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct DiscoveryParentRow {
    path: AbsolutePath,
    kind: DiscoveryRowKind,
}

impl DiscoveryRowKind {
    const fn allows_parent_kind(self, kind: Self) -> bool {
        matches!(
            (self, kind),
            (Self::Root, Self::Root)
                | (Self::WorktreeEntry, Self::WorktreeEntry)
                | (Self::PathOnly, Self::PathOnly)
        )
    }

    const fn discriminant(self) -> u8 {
        match self {
            Self::Root => 0,
            Self::WorktreeEntry => 1,
            Self::PathOnly => 2,
        }
    }
}

fn package_contains_path(pkg: &Package, row_path: &Path) -> bool {
    pkg.path() == row_path
        || pkg
            .vendored()
            .iter()
            .any(|vendored| vendored.path() == row_path)
}

fn workspace_contains_path(ws: &Workspace, row_path: &Path) -> bool {
    ws.path() == row_path
        || ws.groups().iter().any(|group| {
            group
                .members()
                .iter()
                .any(|member| package_contains_path(member, row_path))
        })
        || ws
            .vendored()
            .iter()
            .any(|vendored| vendored.path() == row_path)
}

fn root_item_scope_contains(item: &RootItem, session_path: &Path, row_path: &Path) -> bool {
    match item {
        RootItem::Rust(RustProject::Workspace(ws)) => {
            workspace_scope_contains(ws, session_path, row_path)
        },
        RootItem::Rust(RustProject::Package(pkg)) => {
            package_scope_contains(pkg, session_path, row_path)
        },
        RootItem::NonRust(project) => project.path() == session_path && project.path() == row_path,
        RootItem::Worktrees(WorktreeGroup::Workspaces {
            primary, linked, ..
        }) => {
            workspace_scope_contains(primary, session_path, row_path)
                || linked
                    .iter()
                    .any(|l| workspace_scope_contains(l, session_path, row_path))
        },
        RootItem::Worktrees(WorktreeGroup::Packages {
            primary, linked, ..
        }) => {
            package_scope_contains(primary, session_path, row_path)
                || linked
                    .iter()
                    .any(|l| package_scope_contains(l, session_path, row_path))
        },
    }
}

fn workspace_scope_contains(ws: &Workspace, session_path: &Path, row_path: &Path) -> bool {
    if ws.path() == session_path {
        return workspace_contains_path(ws, row_path);
    }
    if ws
        .vendored()
        .iter()
        .any(|vendored| vendored.path() == session_path && vendored.path() == row_path)
    {
        return true;
    }
    ws.groups().iter().any(|group| {
        group
            .members()
            .iter()
            .any(|member| package_scope_contains(member, session_path, row_path))
    })
}

fn package_scope_contains(pkg: &Package, session_path: &Path, row_path: &Path) -> bool {
    if pkg.path() == session_path {
        return package_contains_path(pkg, row_path);
    }
    pkg.vendored()
        .iter()
        .any(|vendored| vendored.path() == session_path && vendored.path() == row_path)
}

fn root_item_parent_row(item: &RootItem, session_path: &Path) -> Option<DiscoveryParentRow> {
    match item {
        RootItem::Rust(RustProject::Workspace(ws)) => {
            workspace_parent_row(ws, session_path, DiscoveryRowKind::Root)
        },
        RootItem::Rust(RustProject::Package(pkg)) => {
            package_parent_row(pkg, session_path, DiscoveryRowKind::Root)
        },
        RootItem::NonRust(_) => None,
        RootItem::Worktrees(WorktreeGroup::Workspaces {
            primary, linked, ..
        }) => {
            if primary.path() == session_path {
                return None;
            }
            if linked.iter().any(|l| l.path() == session_path) {
                return Some(DiscoveryParentRow {
                    path: primary.path().clone(),
                    kind: DiscoveryRowKind::Root,
                });
            }
            workspace_parent_row(primary, session_path, DiscoveryRowKind::WorktreeEntry).or_else(
                || {
                    linked.iter().find_map(|l| {
                        workspace_parent_row(l, session_path, DiscoveryRowKind::WorktreeEntry)
                    })
                },
            )
        },
        RootItem::Worktrees(WorktreeGroup::Packages {
            primary, linked, ..
        }) => {
            if primary.path() == session_path {
                return None;
            }
            if linked.iter().any(|l| l.path() == session_path) {
                return Some(DiscoveryParentRow {
                    path: primary.path().clone(),
                    kind: DiscoveryRowKind::Root,
                });
            }
            package_parent_row(primary, session_path, DiscoveryRowKind::WorktreeEntry).or_else(
                || {
                    linked.iter().find_map(|l| {
                        package_parent_row(l, session_path, DiscoveryRowKind::WorktreeEntry)
                    })
                },
            )
        },
    }
}

fn workspace_parent_row(
    ws: &Workspace,
    session_path: &Path,
    parent_kind: DiscoveryRowKind,
) -> Option<DiscoveryParentRow> {
    if ws.path() == session_path {
        return None;
    }
    if ws
        .vendored()
        .iter()
        .any(|vendored| vendored.path() == session_path)
    {
        return Some(DiscoveryParentRow {
            path: ws.path().clone(),
            kind: parent_kind,
        });
    }
    for group in ws.groups() {
        for member in group.members() {
            if member.path() == session_path {
                return Some(DiscoveryParentRow {
                    path: ws.path().clone(),
                    kind: parent_kind,
                });
            }
            if let Some(parent) =
                package_parent_row(member, session_path, DiscoveryRowKind::PathOnly)
            {
                return Some(parent);
            }
        }
    }
    None
}

fn package_parent_row(
    pkg: &Package,
    session_path: &Path,
    parent_kind: DiscoveryRowKind,
) -> Option<DiscoveryParentRow> {
    if pkg.path() == session_path {
        return None;
    }
    pkg.vendored()
        .iter()
        .any(|vendored| vendored.path() == session_path)
        .then(|| DiscoveryParentRow {
            path: pkg.path().clone(),
            kind: parent_kind,
        })
}

/// Return the most severe git path state from an iterator.
/// Severity: `Modified` > `Untracked` > `Clean` > `Ignored`.
fn worst_git_status(states: impl Iterator<Item = Option<GitStatus>>) -> Option<GitStatus> {
    const fn severity(state: GitStatus) -> u8 {
        match state {
            GitStatus::Modified => 4,
            GitStatus::Untracked => 3,
            GitStatus::Clean => 2,
            GitStatus::Ignored => 1,
        }
    }
    states.flatten().max_by_key(|s| severity(*s))
}
