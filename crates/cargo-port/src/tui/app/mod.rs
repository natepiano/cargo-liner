mod async_tasks;
mod ci;
mod construct;
mod dismiss;
mod focus;
mod lint;
mod navigation;
mod query;
mod service_state;
mod snapshots;
mod types;

use std::collections::HashMap;
use std::collections::HashSet;
use std::collections::VecDeque;
use std::path::Path;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::mpsc;
use std::time::Instant;

use ratatui::layout::Position;

use self::service_state::CratesIoState;
use self::service_state::GitHubState;
use super::cpu::CpuPoller;
use super::pane::PaneManager;
use super::panes::LayoutCache;
use super::panes::PaneDataStore;
use super::panes::PaneId;
use crate::ci::CiRun;
use crate::ci::OwnerRepo;
use crate::config::CargoPortConfig;
use crate::http::GitHubRateLimit;
use crate::http::HttpClient;
use crate::keymap::ResolvedKeymap;
use crate::lint::CacheUsage;
use crate::lint::LintRuns;
use crate::lint::RuntimeHandle;
use crate::project::AbsolutePath;
use crate::project::ProjectCiData;
use crate::project_list::ProjectList;
use crate::scan;
use crate::scan::BackgroundMsg;
use crate::scan::RepoCache;
use crate::watcher::WatcherMsg;

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
#[allow(
    clippy::unwrap_used,
    reason = "tests should panic on unexpected values"
)]
#[allow(clippy::panic, reason = "tests should panic on unexpected values")]
mod tests;

pub(super) use dismiss::DismissTarget;
pub(super) use types::CiFetchTracker;
pub(super) use types::ConfirmAction;
pub(super) use types::DiscoveryRowKind;
pub(super) use types::ExpandKey;
pub(super) use types::HoveredPaneRow;
pub(super) use types::PendingClean;
pub(super) use types::PollBackgroundStats;
pub(super) use types::VisibleRow;

pub(super) use super::columns::ResolvedWidths;
use super::panes::PendingCiFetch;
use super::panes::PendingExampleRun;
use super::terminal::CiFetchMsg;
use super::terminal::CleanMsg;
use super::terminal::ExampleMsg;
use super::toasts::ToastManager;
use super::toasts::ToastTaskId;
pub(super) struct App {
    current_config:           CargoPortConfig,
    http_client:              HttpClient,
    github:                   GitHubState,
    crates_io:                CratesIoState,
    projects:                 ProjectList,
    ci_fetch_tracker:         CiFetchTracker,
    ci_display_modes:         HashMap<AbsolutePath, types::CiRunDisplayMode>,
    lint_cache_usage:         CacheUsage,
    discovery_shimmers:       HashMap<AbsolutePath, types::DiscoveryShimmer>,
    pending_git_first_commit: HashMap<AbsolutePath, String>,
    cpu_poller:               CpuPoller,
    bg_tx:                    mpsc::Sender<BackgroundMsg>,
    bg_rx:                    mpsc::Receiver<BackgroundMsg>,
    priority_fetch_path:      Option<AbsolutePath>,
    expanded:                 HashSet<ExpandKey>,
    pane_manager:             PaneManager<PaneId>,
    pane_data:                PaneDataStore,
    settings_edit_buf:        String,
    settings_edit_cursor:     usize,
    focused_pane:             PaneId,
    return_focus:             Option<PaneId>,
    visited_panes:            HashSet<PaneId>,
    pending_example_run:      Option<PendingExampleRun>,
    pending_ci_fetch:         Option<PendingCiFetch>,
    pending_cleans:           VecDeque<PendingClean>,
    confirm:                  Option<ConfirmAction>,
    animation_started:        Instant,
    ci_fetch_tx:              mpsc::Sender<CiFetchMsg>,
    ci_fetch_rx:              mpsc::Receiver<CiFetchMsg>,
    clean_tx:                 mpsc::Sender<CleanMsg>,
    clean_rx:                 mpsc::Receiver<CleanMsg>,
    example_running:          Option<String>,
    example_child:            Arc<Mutex<Option<u32>>>,
    example_output:           Vec<String>,
    example_tx:               mpsc::Sender<ExampleMsg>,
    example_rx:               mpsc::Receiver<ExampleMsg>,
    running_clean_paths:      HashSet<AbsolutePath>,
    clean_toast:              Option<ToastTaskId>,
    running_lint_paths:       HashMap<AbsolutePath, Instant>,
    lint_toast:               Option<ToastTaskId>,
    ci_fetch_toast:           Option<ToastTaskId>,
    watch_tx:                 mpsc::Sender<WatcherMsg>,
    lint_runtime:             Option<RuntimeHandle>,
    selection_paths:          types::SelectionPaths,
    finder:                   types::FinderState,
    cached_visible_rows:      Vec<VisibleRow>,
    cached_root_sorted:       Vec<u64>,
    cached_child_sorted:      HashMap<usize, Vec<u64>>,
    cached_fit_widths:        ResolvedWidths,
    data_generation:          u64,
    mouse_pos:                Option<Position>,
    hovered_pane_row:         Option<types::HoveredPaneRow>,
    layout_cache:             LayoutCache,
    status_flash:             Option<(String, std::time::Instant)>,
    toasts:                   ToastManager,
    config_path:              Option<AbsolutePath>,
    config_last_seen:         Option<types::ConfigFileStamp>,
    current_keymap:           ResolvedKeymap,
    keymap_path:              Option<AbsolutePath>,
    keymap_last_seen:         Option<types::ConfigFileStamp>,
    keymap_diagnostics_id:    Option<u64>,
    inline_error:             Option<String>,
    ui_modes:                 types::UiModes,
    dirty:                    types::DirtyState,
    scan:                     types::ScanState,
    selection:                types::SelectionSync,
    #[cfg(test)]
    retry_spawn_mode:         types::RetrySpawnMode,
}

impl App {
    pub(super) const fn current_config(&self) -> &CargoPortConfig { &self.current_config }

    pub(super) const fn current_keymap(&self) -> &ResolvedKeymap { &self.current_keymap }

    pub(super) const fn current_keymap_mut(&mut self) -> &mut ResolvedKeymap {
        &mut self.current_keymap
    }

    pub(super) fn resolved_dirs(&self) -> Vec<AbsolutePath> {
        scan::resolve_include_dirs(&self.current_config.tui.include_dirs)
    }

    pub(super) const fn projects(&self) -> &ProjectList { &self.projects }

    #[cfg(test)]
    pub(super) const fn projects_mut(&mut self) -> &mut ProjectList { &mut self.projects }

    pub(super) const fn repo_fetch_cache(&self) -> &RepoCache { &self.github.fetch_cache }

    /// True when the GitHub service is currently marked unreachable
    /// (network failure or rate-limit). Delegates to the per-service
    /// availability struct; used by the Git pane to apply the
    /// "(github unreachable)" decoration on rate-limit rows.
    pub(super) const fn is_github_unreachable(&self) -> bool {
        self.github.availability.is_unreachable()
    }

    /// Snapshot of GitHub's REST + GraphQL rate-limit buckets. Rebuilt
    /// from the shared `HttpClient` state every frame — not persisted.
    pub(super) fn rate_limit(&self) -> GitHubRateLimit { self.http_client.rate_limit_snapshot() }

    pub(in super::super) fn complete_ci_fetch_for(&mut self, path: &Path) -> bool {
        self.ci_fetch_tracker.complete(path)
    }

    pub(in super::super) fn replace_ci_data_for_path(
        &mut self,
        path: &Path,
        ci_data: ProjectCiData,
    ) {
        if let Some(repo) = self
            .projects
            .entry_containing_mut(path)
            .and_then(|entry| entry.git_repo.as_mut())
        {
            repo.ci_data = ci_data;
        }
    }

    pub(in super::super) fn start_ci_fetch_for(&mut self, path: AbsolutePath) {
        self.ci_fetch_tracker.start(path);
    }

    pub(super) const fn lint_cache_usage(&self) -> &CacheUsage { &self.lint_cache_usage }

    pub(super) fn lint_at_path(&self, path: &Path) -> Option<&LintRuns> {
        self.projects.lint_at_path(path)
    }

    pub(super) fn lint_at_path_mut(&mut self, path: &Path) -> Option<&mut LintRuns> {
        self.projects.lint_at_path_mut(path)
    }

    pub(super) fn clear_all_lint_state(&mut self) {
        let mut paths = Vec::new();
        self.projects.for_each_leaf_path(|path, is_rust| {
            if is_rust {
                paths.push(path.to_path_buf());
            }
        });
        for path in &paths {
            if let Some(lr) = self.projects.lint_at_path_mut(path) {
                lr.clear_runs();
            }
        }
    }

    pub(super) const fn layout_cache(&self) -> &LayoutCache { &self.layout_cache }

    pub(super) const fn layout_cache_mut(&mut self) -> &mut LayoutCache { &mut self.layout_cache }

    pub(super) const fn pane_data(&self) -> &PaneDataStore { &self.pane_data }

    pub(super) const fn pane_data_mut(&mut self) -> &mut PaneDataStore { &mut self.pane_data }

    pub(super) const fn mouse_pos(&self) -> Option<Position> { self.mouse_pos }

    pub(super) const fn set_mouse_pos(&mut self, pos: Option<Position>) { self.mouse_pos = pos; }

    pub(super) const fn set_hovered_pane_row(
        &mut self,
        hovered_pane_row: Option<types::HoveredPaneRow>,
    ) {
        self.hovered_pane_row = hovered_pane_row;
    }

    pub(super) fn apply_hovered_pane_row(&mut self) {
        self.pane_manager.clear_hover();
        let Some(hovered) = self.hovered_pane_row else {
            return;
        };
        self.pane_manager
            .pane_mut(hovered.pane)
            .set_hovered(Some(hovered.row));
    }

    pub(super) const fn cached_fit_widths(&self) -> &ResolvedWidths { &self.cached_fit_widths }

    pub(super) fn cached_root_sorted(&self) -> &[u64] { &self.cached_root_sorted }

    pub(super) const fn cached_child_sorted(&self) -> &HashMap<usize, Vec<u64>> {
        &self.cached_child_sorted
    }

    pub(super) const fn focused_pane(&self) -> PaneId { self.focused_pane }

    pub(super) const fn expanded(&self) -> &HashSet<ExpandKey> { &self.expanded }

    #[cfg(test)]
    pub(super) const fn expanded_mut(&mut self) -> &mut HashSet<ExpandKey> { &mut self.expanded }

    pub(super) const fn pane_manager(&self) -> &PaneManager<PaneId> { &self.pane_manager }

    pub(super) const fn pane_manager_mut(&mut self) -> &mut PaneManager<PaneId> {
        &mut self.pane_manager
    }

    pub(super) const fn finder(&self) -> &types::FinderState { &self.finder }

    pub(super) const fn finder_mut(&mut self) -> &mut types::FinderState { &mut self.finder }

    pub(super) const fn last_selected_path(&self) -> Option<&AbsolutePath> {
        self.selection_paths.last_selected.as_ref()
    }

    pub(super) fn set_pending_example_run(&mut self, run: PendingExampleRun) {
        self.pending_example_run = Some(run);
    }

    pub(super) const fn take_pending_example_run(&mut self) -> Option<PendingExampleRun> {
        self.pending_example_run.take()
    }

    pub(super) fn set_pending_ci_fetch(&mut self, fetch: PendingCiFetch) {
        self.pending_ci_fetch = Some(fetch);
    }

    pub(super) const fn set_ci_fetch_toast(&mut self, task_id: ToastTaskId) {
        self.ci_fetch_toast = Some(task_id);
    }

    pub(super) const fn take_pending_ci_fetch(&mut self) -> Option<PendingCiFetch> {
        self.pending_ci_fetch.take()
    }

    pub(super) const fn pending_cleans_mut(&mut self) -> &mut VecDeque<PendingClean> {
        &mut self.pending_cleans
    }

    pub(super) fn set_confirm(&mut self, action: ConfirmAction) { self.confirm = Some(action); }

    pub(super) const fn confirm(&self) -> Option<&ConfirmAction> { self.confirm.as_ref() }

    pub(super) fn settings_edit_buf(&self) -> &str { &self.settings_edit_buf }

    pub(super) const fn settings_edit_cursor(&self) -> usize { self.settings_edit_cursor }

    pub(super) const fn settings_edit_parts_mut(&mut self) -> (&mut String, &mut usize) {
        (&mut self.settings_edit_buf, &mut self.settings_edit_cursor)
    }

    pub(super) fn set_settings_edit_state(&mut self, value: String, cursor: usize) {
        self.settings_edit_buf = value;
        self.settings_edit_cursor = cursor;
    }

    pub(super) const fn inline_error(&self) -> Option<&String> { self.inline_error.as_ref() }

    pub(super) fn set_inline_error(&mut self, error: impl Into<String>) {
        self.inline_error = Some(error.into());
    }

    pub(super) fn clear_inline_error(&mut self) { self.inline_error = None; }

    pub(super) fn bg_tx(&self) -> mpsc::Sender<BackgroundMsg> { self.bg_tx.clone() }

    pub(super) fn http_client(&self) -> HttpClient { self.http_client.clone() }

    pub(super) fn ci_fetch_tx(&self) -> mpsc::Sender<CiFetchMsg> { self.ci_fetch_tx.clone() }

    pub(super) fn clean_tx(&self) -> mpsc::Sender<CleanMsg> { self.clean_tx.clone() }

    pub(super) fn example_tx(&self) -> mpsc::Sender<ExampleMsg> { self.example_tx.clone() }

    pub(super) fn example_child(&self) -> Arc<Mutex<Option<u32>>> {
        Arc::clone(&self.example_child)
    }

    pub(super) fn example_output(&self) -> &[String] { &self.example_output }

    pub(super) fn set_example_output(&mut self, output: Vec<String>) {
        let was_empty = self.example_output.is_empty();
        self.example_output = output;
        if was_empty && !self.example_output.is_empty() {
            self.focus_pane(PaneId::Output);
        }
    }

    pub(super) const fn example_output_mut(&mut self) -> &mut Vec<String> {
        &mut self.example_output
    }

    pub(super) fn example_running(&self) -> Option<&str> { self.example_running.as_deref() }

    pub(super) fn set_example_running(&mut self, running: Option<String>) {
        self.example_running = running;
    }

    pub(super) const fn increment_data_generation(&mut self) { self.data_generation += 1; }

    pub(super) const fn config_path(&self) -> Option<&AbsolutePath> { self.config_path.as_ref() }

    pub(super) const fn keymap_path(&self) -> Option<&AbsolutePath> { self.keymap_path.as_ref() }

    pub(super) const fn ui_modes(&self) -> &types::UiModes { &self.ui_modes }

    pub(super) const fn take_confirm(&mut self) -> Option<ConfirmAction> { self.confirm.take() }

    #[cfg(test)]
    pub(super) fn set_projects(&mut self, projects: ProjectList) { self.projects = projects; }

    #[cfg(test)]
    pub(super) const fn toasts_mut(&mut self) -> &mut ToastManager { &mut self.toasts }

    pub(super) fn dismiss_target_for_row(&self, row: VisibleRow) -> Option<DismissTarget> {
        self.dismiss_target_for_row_inner(row)
    }

    pub(super) fn owner_repo_for_path(&self, path: &std::path::Path) -> Option<OwnerRepo> {
        self.owner_repo_for_path_inner(path)
    }

    pub(super) fn ci_display_mode_label_for(&self, path: &std::path::Path) -> &'static str {
        self.ci_display_mode_label_for_inner(path)
    }

    pub(super) fn ci_toggle_available_for(&self, path: &std::path::Path) -> bool {
        self.ci_toggle_available_for_inner(path)
    }

    pub(super) fn toggle_ci_display_mode_for(&mut self, path: &std::path::Path) {
        self.toggle_ci_display_mode_for_inner(path);
    }

    pub(super) fn ci_runs_for_display(&self, path: &std::path::Path) -> Vec<CiRun> {
        self.ci_runs_for_display_inner(path)
    }

    pub(super) fn poll_cpu_if_due(&mut self, now: Instant) {
        if let Some(snapshot) = self.cpu_poller.poll_if_due(now) {
            self.pane_data_mut().cpu = Some(snapshot);
        }
    }

    pub(super) fn reset_cpu_placeholder(&mut self) {
        self.cpu_poller = CpuPoller::new(&self.current_config.cpu);
        self.pane_data_mut().cpu = Some(self.cpu_poller.placeholder_snapshot());
    }
}
