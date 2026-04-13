mod async_tasks;
mod ci_state;
mod construct;
mod dismiss;
mod focus;
mod lint;
mod navigation;
mod query;
mod snapshots;
mod types;

use std::collections::HashMap;
use std::collections::HashSet;
use std::collections::VecDeque;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::mpsc;
use std::time::Instant;

use ratatui::widgets::ListState;

use crate::ci::CiRun;
use crate::ci::OwnerRepo;
use crate::config::CargoPortConfig;
use crate::http::HttpClient;
use crate::http::ServiceKind;
use crate::keymap::ResolvedKeymap;
use crate::lint::CacheUsage;
use crate::lint::LintRuns;
use crate::lint::RuntimeHandle;
use crate::project::AbsolutePath;
use crate::project::GitPathState;
use crate::project_list::ProjectList;
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
pub(super) use types::CiState;
pub(super) use types::ConfirmAction;
pub(super) use types::DiscoveryRowKind;
pub(super) use types::ExpandKey;
pub(super) use types::PendingClean;
pub(super) use types::PollBackgroundStats;
#[cfg(test)]
pub(super) use types::SearchHit;
#[cfg(test)]
pub(super) use types::SearchMode;
pub(super) use types::VisibleRow;

pub(super) use super::columns::ResolvedWidths;
use super::detail::PendingCiFetch;
use super::detail::PendingExampleRun;
use super::terminal::CiFetchMsg;
use super::terminal::CleanMsg;
use super::terminal::ExampleMsg;
use super::toasts::ToastManager;
use super::toasts::ToastTaskId;
use super::types::LayoutCache;
use super::types::Pane;
use super::types::PaneId;

pub(super) struct App {
    current_config:           CargoPortConfig,
    scan_root:                PathBuf,
    http_client:              HttpClient,
    repo_fetch_cache:         RepoCache,
    projects:                 ProjectList,
    ci_state:                 HashMap<PathBuf, CiState>,
    ci_display_modes:         HashMap<PathBuf, types::CiRunDisplayMode>,
    lint_cache_usage:         CacheUsage,
    git_path_states:          HashMap<PathBuf, GitPathState>,
    cargo_active_paths:       HashSet<PathBuf>,
    crates_versions:          HashMap<PathBuf, String>,
    crates_downloads:         HashMap<PathBuf, u64>,
    stars:                    HashMap<PathBuf, u64>,
    repo_descriptions:        HashMap<PathBuf, String>,
    discovery_shimmers:       HashMap<PathBuf, types::DiscoveryShimmer>,
    pending_git_first_commit: HashMap<PathBuf, String>,
    bg_tx:                    mpsc::Sender<BackgroundMsg>,
    bg_rx:                    mpsc::Receiver<BackgroundMsg>,
    fully_loaded:             HashSet<PathBuf>,
    priority_fetch_path:      Option<AbsolutePath>,
    expanded:                 HashSet<ExpandKey>,
    list_state:               ListState,
    search_query:             String,
    filtered:                 Vec<types::SearchHit>,
    settings_pane:            Pane,
    settings_edit_buf:        String,
    settings_edit_cursor:     usize,
    focused_pane:             PaneId,
    return_focus:             Option<PaneId>,
    visited_panes:            HashSet<PaneId>,
    package_pane:             Pane,
    git_pane:                 Pane,
    targets_pane:             Pane,
    ci_pane:                  Pane,
    toast_pane:               Pane,
    lint_pane:                Pane,
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
    running_clean_paths:      HashSet<PathBuf>,
    clean_toast:              Option<ToastTaskId>,
    running_lint_paths:       HashMap<PathBuf, Instant>,
    lint_toast:               Option<ToastTaskId>,
    watch_tx:                 mpsc::Sender<WatcherMsg>,
    lint_runtime:             Option<RuntimeHandle>,
    unreachable_services:     HashSet<ServiceKind>,
    service_retry_active:     HashSet<ServiceKind>,
    selection_paths:          types::SelectionPaths,
    finder:                   types::FinderState,
    cached_visible_rows:      Vec<VisibleRow>,
    cached_root_sorted:       Vec<u64>,
    cached_child_sorted:      HashMap<usize, Vec<u64>>,
    cached_fit_widths:        ResolvedWidths,
    builds:                   types::AsyncBuildState,
    data_generation:          u64,
    detail_generation:        u64,
    cached_detail:            Option<types::DetailCache>,
    layout_cache:             LayoutCache,
    status_flash:             Option<(String, std::time::Instant)>,
    toasts:                   ToastManager,
    config_path:              Option<PathBuf>,
    config_last_seen:         Option<types::ConfigFileStamp>,
    current_keymap:           ResolvedKeymap,
    keymap_path:              Option<PathBuf>,
    keymap_last_seen:         Option<types::ConfigFileStamp>,
    keymap_diagnostics_id:    Option<u64>,
    keymap_pane:              Pane,
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

    pub(super) const fn scan_root(&self) -> &PathBuf { &self.scan_root }

    pub(super) const fn projects(&self) -> &ProjectList { &self.projects }

    #[cfg(test)]
    pub(super) const fn projects_mut(&mut self) -> &mut ProjectList { &mut self.projects }

    pub(super) const fn repo_fetch_cache(&self) -> &RepoCache { &self.repo_fetch_cache }

    pub(super) const fn ci_state_mut(&mut self) -> &mut HashMap<PathBuf, CiState> {
        &mut self.ci_state
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

    pub(super) const fn crates_versions(&self) -> &HashMap<PathBuf, String> {
        &self.crates_versions
    }

    pub(super) const fn crates_downloads(&self) -> &HashMap<PathBuf, u64> { &self.crates_downloads }

    pub(super) const fn stars(&self) -> &HashMap<PathBuf, u64> { &self.stars }

    pub(super) const fn repo_descriptions(&self) -> &HashMap<PathBuf, String> {
        &self.repo_descriptions
    }

    pub(super) const fn cached_detail(&self) -> Option<&types::DetailCache> {
        self.cached_detail.as_ref()
    }

    pub(super) const fn layout_cache(&self) -> &LayoutCache { &self.layout_cache }

    pub(super) const fn layout_cache_mut(&mut self) -> &mut LayoutCache { &mut self.layout_cache }

    pub(super) const fn cached_fit_widths(&self) -> &ResolvedWidths { &self.cached_fit_widths }

    pub(super) fn cached_root_sorted(&self) -> &[u64] { &self.cached_root_sorted }

    pub(super) const fn cached_child_sorted(&self) -> &HashMap<usize, Vec<u64>> {
        &self.cached_child_sorted
    }

    pub(super) const fn list_state(&self) -> &ListState { &self.list_state }

    pub(super) const fn list_state_mut(&mut self) -> &mut ListState { &mut self.list_state }

    pub(super) const fn focused_pane(&self) -> PaneId { self.focused_pane }

    #[cfg(test)]
    pub(super) const fn set_focused_pane(&mut self, pane: PaneId) { self.focused_pane = pane; }

    pub(super) const fn expanded(&self) -> &HashSet<ExpandKey> { &self.expanded }

    #[cfg(test)]
    pub(super) const fn expanded_mut(&mut self) -> &mut HashSet<ExpandKey> { &mut self.expanded }

    pub(super) const fn dirty(&self) -> &types::DirtyState { &self.dirty }

    pub(super) const fn dirty_mut(&mut self) -> &mut types::DirtyState { &mut self.dirty }

    #[cfg(test)]
    pub(super) const fn ui_modes_mut(&mut self) -> &mut types::UiModes { &mut self.ui_modes }

    pub(super) const fn settings_pane(&self) -> &Pane { &self.settings_pane }

    pub(super) const fn settings_pane_mut(&mut self) -> &mut Pane { &mut self.settings_pane }

    pub(super) const fn package_pane(&self) -> &Pane { &self.package_pane }

    pub(super) const fn package_pane_mut(&mut self) -> &mut Pane { &mut self.package_pane }

    pub(super) const fn git_pane(&self) -> &Pane { &self.git_pane }

    pub(super) const fn git_pane_mut(&mut self) -> &mut Pane { &mut self.git_pane }

    pub(super) const fn targets_pane(&self) -> &Pane { &self.targets_pane }

    pub(super) const fn targets_pane_mut(&mut self) -> &mut Pane { &mut self.targets_pane }

    pub(super) const fn ci_pane(&self) -> &Pane { &self.ci_pane }

    pub(super) const fn ci_pane_mut(&mut self) -> &mut Pane { &mut self.ci_pane }

    pub(super) const fn toast_pane(&self) -> &Pane { &self.toast_pane }

    pub(super) const fn toast_pane_mut(&mut self) -> &mut Pane { &mut self.toast_pane }

    pub(super) const fn lint_pane(&self) -> &Pane { &self.lint_pane }

    pub(super) const fn lint_pane_mut(&mut self) -> &mut Pane { &mut self.lint_pane }

    pub(super) const fn keymap_pane(&self) -> &Pane { &self.keymap_pane }

    pub(super) const fn keymap_pane_mut(&mut self) -> &mut Pane { &mut self.keymap_pane }

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

    pub(super) fn search_query(&self) -> &str { &self.search_query }

    #[cfg(test)]
    pub(super) const fn search_query_mut(&mut self) -> &mut String { &mut self.search_query }

    pub(super) fn filtered(&self) -> &[types::SearchHit] { &self.filtered }

    #[cfg(test)]
    pub(super) const fn filtered_mut(&mut self) -> &mut Vec<types::SearchHit> { &mut self.filtered }

    pub(super) const fn config_path(&self) -> Option<&PathBuf> { self.config_path.as_ref() }

    pub(super) const fn keymap_path(&self) -> Option<&PathBuf> { self.keymap_path.as_ref() }

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

    pub(super) fn owner_paths_for_repo(&self, repo: &OwnerRepo) -> Vec<std::path::PathBuf> {
        self.owner_paths_for_repo_inner(repo)
    }

    pub(super) fn ci_owner_path_for(&self, path: &std::path::Path) -> Option<std::path::PathBuf> {
        self.ci_owner_path_for_inner(path)
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
}
