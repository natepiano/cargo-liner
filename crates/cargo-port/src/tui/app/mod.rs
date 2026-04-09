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
use crate::lint::LintRun;
use crate::lint::LintStatus;
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
    pub current_config:           CargoPortConfig,
    pub scan_root:                PathBuf,
    pub http_client:              HttpClient,
    pub repo_fetch_cache:         RepoCache,
    pub projects:                 ProjectList,
    pub ci_state:                 HashMap<PathBuf, CiState>,
    pub ci_display_modes:         HashMap<PathBuf, types::CiRunDisplayMode>,
    pub lint_status:              HashMap<PathBuf, LintStatus>,
    pub lint_cache_usage:         CacheUsage,
    pub lint_runs:                HashMap<PathBuf, Vec<LintRun>>,
    pub lint_rollup_status:       HashMap<types::LintRollupKey, LintStatus>,
    pub lint_rollup_paths:        HashMap<types::LintRollupKey, Vec<PathBuf>>,
    pub lint_rollup_keys_by_path: HashMap<PathBuf, Vec<types::LintRollupKey>>,
    pub git_path_states:          HashMap<PathBuf, GitPathState>,
    pub cargo_active_paths:       HashSet<PathBuf>,
    pub crates_versions:          HashMap<PathBuf, String>,
    pub crates_downloads:         HashMap<PathBuf, u64>,
    pub stars:                    HashMap<PathBuf, u64>,
    pub repo_descriptions:        HashMap<PathBuf, String>,
    pub discovery_shimmers:       HashMap<PathBuf, types::DiscoveryShimmer>,
    pub pending_git_first_commit: HashMap<PathBuf, String>,
    pub bg_tx:                    mpsc::Sender<BackgroundMsg>,
    pub bg_rx:                    mpsc::Receiver<BackgroundMsg>,
    pub fully_loaded:             HashSet<PathBuf>,
    pub priority_fetch_path:      Option<AbsolutePath>,
    pub expanded:                 HashSet<ExpandKey>,
    pub list_state:               ListState,
    pub search_query:             String,
    pub filtered:                 Vec<types::SearchHit>,
    pub settings_pane:            Pane,
    pub settings_edit_buf:        String,
    pub settings_edit_cursor:     usize,
    pub focused_pane:             PaneId,
    pub return_focus:             Option<PaneId>,
    pub visited_panes:            HashSet<PaneId>,
    pub package_pane:             Pane,
    pub git_pane:                 Pane,
    pub targets_pane:             Pane,
    pub ci_pane:                  Pane,
    pub toast_pane:               Pane,
    pub lint_pane:                Pane,
    pub pending_example_run:      Option<PendingExampleRun>,
    pub pending_ci_fetch:         Option<PendingCiFetch>,
    pub pending_cleans:           VecDeque<PendingClean>,
    pub confirm:                  Option<ConfirmAction>,
    pub animation_started:        Instant,
    pub ci_fetch_tx:              mpsc::Sender<CiFetchMsg>,
    pub ci_fetch_rx:              mpsc::Receiver<CiFetchMsg>,
    pub clean_tx:                 mpsc::Sender<CleanMsg>,
    pub clean_rx:                 mpsc::Receiver<CleanMsg>,
    pub example_running:          Option<String>,
    pub example_child:            Arc<Mutex<Option<u32>>>,
    pub example_output:           Vec<String>,
    pub example_tx:               mpsc::Sender<ExampleMsg>,
    pub example_rx:               mpsc::Receiver<ExampleMsg>,
    pub running_clean_paths:      HashSet<PathBuf>,
    pub clean_toast:              Option<ToastTaskId>,
    pub running_lint_paths:       HashMap<PathBuf, Instant>,
    pub lint_toast:               Option<ToastTaskId>,
    pub watch_tx:                 mpsc::Sender<WatcherMsg>,
    pub lint_runtime:             Option<RuntimeHandle>,
    pub unreachable_services:     HashSet<ServiceKind>,
    pub service_retry_active:     HashSet<ServiceKind>,
    pub selection_paths:          types::SelectionPaths,
    pub finder:                   types::FinderState,
    pub cached_visible_rows:      Vec<VisibleRow>,
    pub cached_root_sorted:       Vec<u64>,
    pub cached_child_sorted:      HashMap<usize, Vec<u64>>,
    pub cached_fit_widths:        ResolvedWidths,
    pub builds:                   types::AsyncBuildState,
    pub data_generation:          u64,
    pub detail_generation:        u64,
    pub cached_detail:            Option<types::DetailCache>,
    pub layout_cache:             LayoutCache,
    pub status_flash:             Option<(String, std::time::Instant)>,
    pub toasts:                   ToastManager,
    pub config_path:              Option<PathBuf>,
    pub config_last_seen:         Option<types::ConfigFileStamp>,
    pub current_keymap:           ResolvedKeymap,
    pub keymap_path:              Option<PathBuf>,
    pub keymap_last_seen:         Option<types::ConfigFileStamp>,
    pub keymap_diagnostics_id:    Option<u64>,
    pub keymap_pane:              Pane,
    pub inline_error:             Option<String>,
    pub ui_modes:                 types::UiModes,
    pub dirty:                    types::DirtyState,
    pub scan:                     types::ScanState,
    pub selection:                types::SelectionSync,
    #[cfg(test)]
    pub retry_spawn_mode:         types::RetrySpawnMode,
}

impl App {
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
