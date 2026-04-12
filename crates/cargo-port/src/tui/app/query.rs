use std::collections::HashSet;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;
use std::time::Instant;

use super::App;
use super::types::CiState;
use super::types::DiscoveryRowKind;
use super::types::DiscoveryShimmer;
use crate::ci::CiRun;
use crate::ci::Conclusion;
use crate::config::NavigationKeys;
use crate::config::NonRustInclusion;
use crate::config::ScrollDirection;
use crate::constants::IN_SYNC;
use crate::constants::NO_REMOTE_SYNC;
use crate::constants::SYNC_DOWN;
use crate::constants::SYNC_UP;
use crate::project::AbsolutePath;
use crate::project::GitInfo;
use crate::project::GitPathState;
use crate::project::PackageProject;
use crate::project::ProjectFields;
use crate::project::RootItem;
use crate::project::RustProject;
use crate::project::WorkspaceProject;
use crate::project::WorktreeGroup;
use crate::tui::columns;
use crate::tui::detail::DetailField;
use crate::tui::shortcuts::InputContext;
use crate::tui::toasts::ToastTaskId;
use crate::tui::toasts::ToastView;
use crate::tui::toasts::TrackedItem;
use crate::tui::types::PaneId;

impl App {
    pub(in super::super) const fn lint_enabled(&self) -> bool { self.current_config.lint.enabled }

    pub(in super::super) const fn invert_scroll(&self) -> ScrollDirection {
        self.current_config.mouse.invert_scroll
    }

    pub(in super::super) const fn include_non_rust(&self) -> NonRustInclusion {
        self.current_config.tui.include_non_rust
    }

    pub(in super::super) const fn ci_run_count(&self) -> u32 {
        self.current_config.tui.ci_run_count
    }

    pub(in super::super) const fn navigation_keys(&self) -> NavigationKeys {
        self.current_config.tui.navigation_keys
    }

    pub(in super::super) fn editor(&self) -> &str { &self.current_config.tui.editor }

    pub(in super::super) fn terminal_command(&self) -> &str {
        &self.current_config.tui.terminal_command
    }

    pub(in super::super) fn terminal_command_configured(&self) -> bool {
        !self.terminal_command().trim().is_empty()
    }

    fn toast_timeout(&self) -> Duration {
        Duration::from_secs_f64(self.current_config.tui.status_flash_secs)
    }

    pub(in super::super) fn active_toasts(&self) -> Vec<ToastView<'_>> {
        self.toasts.active(Instant::now())
    }

    pub(in super::super) fn focused_toast_id(&self) -> Option<u64> {
        let active = self.active_toasts();
        active.get(self.toast_pane.pos()).map(ToastView::id)
    }

    pub(in super::super) fn prune_toasts(&mut self) {
        let now = Instant::now();
        let linger = Duration::from_secs_f64(self.current_config.tui.task_linger_secs);
        self.toasts.prune_tracked_items(now, linger);
        self.toasts.prune(now);
        self.toast_pane.set_len(self.active_toasts().len());
        if self.base_focus() == PaneId::Toasts && self.active_toasts().is_empty() {
            self.focus_pane(PaneId::ProjectList);
        }
    }

    pub(in super::super) fn show_timed_toast(
        &mut self,
        title: impl Into<String>,
        body: impl Into<String>,
    ) {
        self.toasts.push_timed(title, body, self.toast_timeout(), 1);
        self.toast_pane.set_len(self.active_toasts().len());
    }

    pub(in super::super) fn start_task_toast(
        &mut self,
        title: impl Into<String>,
        body: impl Into<String>,
    ) -> ToastTaskId {
        let task_id = self.toasts.push_task(title, body, 1);
        self.toast_pane.set_len(self.active_toasts().len());
        task_id
    }

    pub(in super::super) fn finish_task_toast(&mut self, task_id: ToastTaskId) {
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

    pub(in super::super) fn set_task_tracked_items(
        &mut self,
        task_id: ToastTaskId,
        items: &[TrackedItem],
    ) {
        let linger = Duration::from_secs_f64(self.current_config.tui.task_linger_secs);
        self.toasts.set_tracked_items(task_id, items, linger);
        self.toast_pane.set_len(self.active_toasts().len());
    }

    pub(in super::super) fn mark_tracked_item_completed(
        &mut self,
        task_id: ToastTaskId,
        label: &str,
    ) {
        self.toasts.mark_item_completed(task_id, label);
        self.toast_pane.set_len(self.active_toasts().len());
    }

    pub(in super::super) fn start_clean(&mut self, project_path: &Path) {
        self.running_clean_paths.insert(project_path.to_path_buf());
        self.sync_running_clean_toast();
    }

    pub(in super::super) fn clean_spawn_failed(&mut self, project_path: &Path) {
        self.running_clean_paths.remove(project_path);
        self.sync_running_clean_toast();
    }

    pub(in super::super) fn dismiss_toast(&mut self, id: u64) {
        self.toasts.dismiss(id);
        self.prune_toasts();
    }

    pub(in super::super) fn sync_selected_project(&mut self) {
        self.ensure_visible_rows_cached();
        let current = self.selected_project_path().map(AbsolutePath::from);
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

        if let Some(abs_path) = current
            && self.selection_paths.last_selected.as_ref() != Some(&abs_path)
        {
            if let Some(path) = self.selected_project_path().map(Path::to_path_buf) {
                self.reload_lint_history(&path);
            }
            self.data_generation += 1;
            self.detail_generation += 1;
            self.selection_paths.last_selected = Some(abs_path);
            self.mark_selection_changed();
            self.maybe_priority_fetch();
        }
    }

    pub(in super::super) fn is_deleted(&self, path: &Path) -> bool {
        use crate::project::Visibility;
        self.projects
            .at_path(path)
            .is_some_and(|project| project.visibility == Visibility::Deleted)
    }

    pub(in super::super) fn formatted_disk(&self, path: &Path) -> String {
        let bytes = self
            .projects
            .at_path(path)
            .and_then(|project| project.disk_usage_bytes)
            .unwrap_or(0);
        crate::tui::render::format_bytes(bytes)
    }

    pub(in super::super) fn selected_ci_path(&self) -> Option<PathBuf> {
        self.selected_project_path()
            .and_then(|path| self.ci_owner_path_for(path))
    }

    pub(in super::super) fn selected_ci_state(&self) -> Option<&CiState> {
        let path = self.selected_ci_path()?;
        self.ci_state_for(path.as_path())
    }

    pub(in super::super) fn selected_ci_runs(&self) -> Vec<CiRun> {
        self.selected_project_path()
            .map_or_else(Vec::new, |path| self.ci_runs_for_display(path))
    }

    pub(in super::super) fn ci_for(&self, path: &Path) -> Option<Conclusion> {
        // A branch with no upstream tracking can't have CI runs — don't
        // show the parent repo's result for an unpushed worktree branch.
        if let Some(git) = self.git_info_for(path)
            && git.upstream_branch.is_none()
            && git.branch.as_deref() != git.default_branch.as_deref()
        {
            return None;
        }
        self.ci_state_for(path)
            .and_then(|_| self.latest_ci_run_for_path(path))
            .map(|run| run.conclusion)
    }

    pub(in super::super) fn ci_state_for(&self, path: &Path) -> Option<&CiState> {
        let owner_path = self.ci_owner_path_for(path)?;
        self.ci_state.get(owner_path.as_path())
    }

    pub(in super::super) fn git_info_for(&self, path: &Path) -> Option<&GitInfo> {
        self.projects
            .at_path(path)
            .and_then(|project| project.git_info.as_ref())
    }

    // ── RootItem query methods ─────────────────────────────────────

    /// All absolute paths for a `RootItem` (root + worktrees).
    fn unique_item_paths(item: &RootItem) -> Vec<PathBuf> {
        let mut paths = Vec::new();
        paths.push(item.path().to_path_buf());
        match item {
            RootItem::Worktrees(WorktreeGroup::Workspaces { linked, .. }) => {
                for l in linked {
                    let p = l.path().to_path_buf();
                    if !paths.contains(&p) {
                        paths.push(p);
                    }
                }
            },
            RootItem::Worktrees(WorktreeGroup::Packages { linked, .. }) => {
                for l in linked {
                    let p = l.path().to_path_buf();
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
    pub(in super::super) fn formatted_disk_for_item(item: &RootItem) -> String {
        item.disk_usage_bytes().map_or_else(
            || crate::tui::render::format_bytes(0),
            crate::tui::render::format_bytes,
        )
    }

    /// Aggregate CI for a `RootItem`.
    pub(in super::super) fn ci_for_item(&self, item: &RootItem) -> Option<Conclusion> {
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

    pub(in super::super) fn animation_elapsed(&self) -> Duration {
        self.animation_started.elapsed()
    }

    pub(in super::super) fn discovery_shimmer_enabled(&self) -> bool {
        self.current_config.tui.discovery_shimmer_secs > 0.0
    }

    pub(in super::super) fn discovery_shimmer_duration(&self) -> Duration {
        Duration::from_secs_f64(self.current_config.tui.discovery_shimmer_secs)
    }

    pub(in super::super) fn register_discovery_shimmer(&mut self, path: &Path) {
        if !self.is_scan_complete() || !self.discovery_shimmer_enabled() {
            return;
        }
        self.discovery_shimmers.insert(
            path.to_path_buf(),
            DiscoveryShimmer::new(Instant::now(), self.discovery_shimmer_duration()),
        );
    }

    pub(in super::super) fn prune_discovery_shimmers(&mut self, now: Instant) {
        self.discovery_shimmers
            .retain(|_, shimmer| shimmer.is_active_at(now));
    }

    pub(in super::super) fn discovery_name_segments_for_path(
        &self,
        row_path: &Path,
        name: &str,
        git_path_state: GitPathState,
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

        let base_style = columns::project_name_style(git_path_state);
        let accent_style = columns::project_name_shimmer_style(git_path_state);
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
    ) -> Option<(PathBuf, DiscoveryShimmer)> {
        self.discovery_shimmers
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
        self.projects
            .iter()
            .any(|item| root_item_scope_contains(item, session_path, row_path))
    }

    fn discovery_parent_row(&self, session_path: &Path) -> Option<DiscoveryParentRow> {
        self.projects
            .iter()
            .find_map(|item| root_item_parent_row(item, session_path))
    }

    pub(in super::super) fn is_vendored_path(&self, path: &Path) -> bool {
        self.projects.iter().any(|item| match item {
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

    pub(in super::super) fn is_workspace_member_path(&self, path: &Path) -> bool {
        self.projects.iter().any(|item| match item {
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

    pub(in super::super) fn recompute_cargo_active_paths(&mut self) {
        let mut active_paths: HashSet<PathBuf> = HashSet::new();
        self.projects.for_each_leaf(|item| {
            if !self.is_vendored_path(item.path()) {
                active_paths.insert(item.path().to_path_buf());
            }
        });

        // Include vendored projects whose parent is active.
        for item in &self.projects {
            let vendored_paths: Vec<&Path> = match item {
                RootItem::Rust(RustProject::Workspace(ws)) => {
                    ws.vendored().iter().map(PackageProject::path).collect()
                },
                RootItem::Rust(RustProject::Package(pkg)) => {
                    pkg.vendored().iter().map(PackageProject::path).collect()
                },
                RootItem::Worktrees(WorktreeGroup::Workspaces {
                    primary, linked, ..
                }) => std::iter::once(primary)
                    .chain(linked.iter())
                    .flat_map(|ws| ws.vendored().iter().map(PackageProject::path))
                    .collect(),
                RootItem::Worktrees(WorktreeGroup::Packages {
                    primary, linked, ..
                }) => std::iter::once(primary)
                    .chain(linked.iter())
                    .flat_map(|pkg| pkg.vendored().iter().map(PackageProject::path))
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

    pub(in super::super) fn is_cargo_active_path(&self, path: &Path) -> bool {
        self.cargo_active_paths.contains(path)
    }

    pub(in super::super) fn git_path_state_for(&self, path: &Path) -> GitPathState {
        self.git_path_states
            .get(path)
            .copied()
            .unwrap_or(GitPathState::OutsideRepo)
    }

    /// Roll up the worst git path state across all children of a `RootItem`.
    /// For worktree groups, checks primary + all linked entries.
    /// For everything else, returns the state for the single path.
    pub(in super::super) fn git_path_state_for_item(&self, item: &RootItem) -> GitPathState {
        match item {
            RootItem::Worktrees(g) => {
                let states: Box<dyn Iterator<Item = GitPathState>> = match g {
                    WorktreeGroup::Workspaces {
                        primary, linked, ..
                    } => Box::new(
                        std::iter::once(self.git_path_state_for(primary.path()))
                            .chain(linked.iter().map(|l| self.git_path_state_for(l.path()))),
                    ),
                    WorktreeGroup::Packages {
                        primary, linked, ..
                    } => Box::new(
                        std::iter::once(self.git_path_state_for(primary.path()))
                            .chain(linked.iter().map(|l| self.git_path_state_for(l.path()))),
                    ),
                };
                worst_git_path_state(states)
            },
            _ => self.git_path_state_for(item.path()),
        }
    }

    pub(in super::super) fn refresh_git_path_state(&mut self, path: &Path) {
        let state = crate::project::detect_git_path_state(path);
        self.git_path_states.insert(path.to_path_buf(), state);
    }

    pub(in super::super) fn prune_inactive_project_state(&mut self) {
        let mut all_paths: HashSet<PathBuf> = HashSet::new();
        self.projects.for_each_leaf_path(|path, _| {
            all_paths.insert(path.to_path_buf());
        });
        self.git_path_states
            .retain(|path, _| all_paths.contains(path));
        self.pending_git_first_commit
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
    pub(in super::super) fn git_sync(&self, path: &Path) -> String {
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
            // No upstream tracking branch: render a flat placeholder in the O column.
            None => NO_REMOTE_SYNC.to_string(),
        }
    }

    pub(in super::super) fn git_main(&self, path: &Path) -> String {
        if matches!(
            self.git_path_state_for(path),
            GitPathState::Untracked | GitPathState::Ignored
        ) {
            return String::new();
        }
        let Some(info) = self.git_info_for(path) else {
            return String::new();
        };
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
    pub(in super::super) fn enter_action(&self) -> Option<&'static str> {
        match self.input_context() {
            InputContext::DetailTargets => Some("run"),
            InputContext::DetailFields => {
                let info = &self.cached_detail.as_ref()?.info;
                if self.base_focus() == PaneId::Package {
                    let fields = crate::tui::detail::package_fields(info);
                    let field = *fields.get(self.package_pane.pos())?;
                    if field == DetailField::CratesIo && info.crates_version.is_some() {
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
    path: PathBuf,
    kind: DiscoveryRowKind,
}

impl DiscoveryRowKind {
    const fn allows_parent_kind(self, kind: Self) -> bool {
        matches!(
            (self, kind),
            (Self::Root, Self::Root)
                | (Self::WorktreeEntry, Self::WorktreeEntry)
                | (Self::PathOnly, Self::PathOnly)
                | (Self::Search, _)
        )
    }

    const fn discriminant(self) -> u8 {
        match self {
            Self::Root => 0,
            Self::WorktreeEntry => 1,
            Self::PathOnly => 2,
            Self::Search => 3,
        }
    }
}

fn package_contains_path(pkg: &PackageProject, row_path: &Path) -> bool {
    pkg.path() == row_path
        || pkg
            .vendored()
            .iter()
            .any(|vendored| vendored.path() == row_path)
}

fn workspace_contains_path(ws: &WorkspaceProject, row_path: &Path) -> bool {
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

fn workspace_scope_contains(ws: &WorkspaceProject, session_path: &Path, row_path: &Path) -> bool {
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

fn package_scope_contains(pkg: &PackageProject, session_path: &Path, row_path: &Path) -> bool {
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
                    path: primary.path().to_path_buf(),
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
                    path: primary.path().to_path_buf(),
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
    ws: &WorkspaceProject,
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
            path: ws.path().to_path_buf(),
            kind: parent_kind,
        });
    }
    for group in ws.groups() {
        for member in group.members() {
            if member.path() == session_path {
                return Some(DiscoveryParentRow {
                    path: ws.path().to_path_buf(),
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
    pkg: &PackageProject,
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
            path: pkg.path().to_path_buf(),
            kind: parent_kind,
        })
}

/// Return the most severe git path state from an iterator.
/// Severity: `Modified` > `Untracked` > `Clean` > `Ignored` > `OutsideRepo`.
fn worst_git_path_state(states: impl Iterator<Item = GitPathState>) -> GitPathState {
    const fn severity(state: GitPathState) -> u8 {
        match state {
            GitPathState::Modified => 4,
            GitPathState::Untracked => 3,
            GitPathState::Clean => 2,
            GitPathState::Ignored => 1,
            GitPathState::OutsideRepo => 0,
        }
    }
    states
        .max_by_key(|s| severity(*s))
        .unwrap_or(GitPathState::OutsideRepo)
}
