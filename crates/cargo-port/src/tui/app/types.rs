use std::collections::HashMap;
use std::collections::HashSet;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::mpsc;
use std::sync::mpsc::Receiver;
use std::time::Instant;
use std::time::SystemTime;

use ratatui::widgets::ListState;

use crate::ci::CiRun;
use crate::config::CargoPortConfig;
use crate::http::HttpClient;
use crate::http::ServiceKind;
use crate::keymap::ResolvedKeymap;
use crate::lint::CacheUsage;
use crate::lint::LintRun;
use crate::lint::LintStatus;
use crate::lint::RuntimeHandle;
use crate::project::GitInfo;
use crate::project::GitPathState;
use crate::project::Project;
use crate::scan::BackgroundMsg;
use crate::scan::FlatEntry;
use crate::scan::ProjectNode;
use crate::tui::columns::ResolvedWidths;
use crate::tui::detail::DetailInfo;
use crate::tui::detail::PendingCiFetch;
use crate::tui::detail::PendingExampleRun;
use crate::tui::finder::FINDER_COLUMN_COUNT;
use crate::tui::finder::FinderItem;
use crate::tui::terminal::CiFetchMsg;
use crate::tui::terminal::CleanMsg;
use crate::tui::terminal::ExampleMsg;
use crate::tui::toasts::ToastManager;
use crate::tui::toasts::ToastTaskId;
use crate::tui::types::LayoutCache;
use crate::tui::types::Pane;
use crate::tui::types::PaneId;
use crate::watcher::WatchRequest;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BottomPanel {
    CiRuns,
    Lints,
}

/// An expand key: a node, group, worktree entry, or group within a worktree.
#[derive(Hash, Eq, PartialEq, Clone)]
pub enum ExpandKey {
    Node(usize),
    Group(usize, usize),
    Worktree(usize, usize),
    WorktreeGroup(usize, usize, usize),
}

/// An action waiting for user confirmation (y/n).
pub enum ConfirmAction {
    /// `cargo clean` on the project at this absolute path.
    Clean(String),
}

#[derive(Clone)]
pub struct PendingClean {
    pub abs_path:     String,
    pub project_path: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConfigFileStamp {
    pub modified: Option<SystemTime>,
    pub len:      u64,
}

pub struct TreeBuildResult {
    pub build_id:     u64,
    pub nodes:        Vec<ProjectNode>,
    pub flat_entries: Vec<FlatEntry>,
}

pub struct FitWidthsBuildResult {
    pub build_id: u64,
    pub widths:   ResolvedWidths,
}

pub struct DiskCacheBuildResult {
    pub build_id:     u64,
    pub root_sorted:  Vec<u64>,
    pub child_sorted: HashMap<usize, Vec<u64>>,
}

#[derive(Debug, Default)]
pub struct StartupPhaseTracker {
    pub scan_complete_at:    Option<Instant>,
    pub disk_expected:       Option<usize>,
    pub disk_seen:           HashSet<String>,
    pub disk_complete_at:    Option<Instant>,
    pub git_expected:        HashSet<String>,
    pub git_seen:            HashSet<String>,
    pub git_complete_at:     Option<Instant>,
    pub repo_expected:       HashSet<String>,
    pub repo_seen:           HashSet<String>,
    pub repo_complete_at:    Option<Instant>,
    pub git_toast:           Option<ToastTaskId>,
    pub repo_toast:          Option<ToastTaskId>,
    pub lint_expected:       Option<HashSet<String>>,
    pub lint_seen_terminal:  HashSet<String>,
    pub lint_complete_at:    Option<Instant>,
    pub startup_complete_at: Option<Instant>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Dirtiness {
    #[default]
    Clean,
    Dirty,
}

impl Dirtiness {
    pub const fn is_dirty(self) -> bool { matches!(self, Self::Dirty) }

    pub const fn mark_dirty(&mut self) { *self = Self::Dirty; }

    pub const fn mark_clean(&mut self) { *self = Self::Clean; }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SearchMode {
    #[default]
    Inactive,
    Active,
}

impl SearchMode {
    pub const fn is_active(self) -> bool { matches!(self, Self::Active) }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FinderMode {
    #[default]
    Hidden,
    Visible,
}

impl FinderMode {
    pub const fn is_visible(self) -> bool { matches!(self, Self::Visible) }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SettingsMode {
    #[default]
    Hidden,
    Browsing,
    Editing,
}

impl SettingsMode {
    pub const fn is_visible(self) -> bool { !matches!(self, Self::Hidden) }

    pub const fn is_editing(self) -> bool { matches!(self, Self::Editing) }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum KeymapMode {
    #[default]
    Hidden,
    Browsing,
    AwaitingKey,
}

impl KeymapMode {
    pub const fn is_visible(self) -> bool { !matches!(self, Self::Hidden) }

    pub const fn is_awaiting_key(self) -> bool { matches!(self, Self::AwaitingKey) }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ScanPhase {
    #[default]
    Running,
    Complete,
}

impl ScanPhase {
    pub const fn is_complete(self) -> bool { matches!(self, Self::Complete) }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ExitMode {
    #[default]
    Continue,
    Quit,
    Restart,
}

impl ExitMode {
    pub const fn should_quit(self) -> bool { matches!(self, Self::Quit | Self::Restart) }

    pub const fn should_restart(self) -> bool { matches!(self, Self::Restart) }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SelectionSync {
    #[default]
    Stable,
    Changed,
}

impl SelectionSync {
    pub const fn is_changed(self) -> bool { matches!(self, Self::Changed) }
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RetrySpawnMode {
    #[default]
    Enabled,
    Disabled,
}

#[cfg(test)]
impl RetrySpawnMode {
    pub const fn is_enabled(self) -> bool { matches!(self, Self::Enabled) }
}

#[derive(Debug)]
pub struct DirtyState {
    pub rows:       Dirtiness,
    pub disk_cache: Dirtiness,
    pub fit_widths: Dirtiness,
    pub finder:     Dirtiness,
    pub terminal:   Dirtiness,
}

impl DirtyState {
    pub const fn initial() -> Self {
        Self {
            rows:       Dirtiness::Dirty,
            disk_cache: Dirtiness::Dirty,
            fit_widths: Dirtiness::Dirty,
            finder:     Dirtiness::Dirty,
            terminal:   Dirtiness::Clean,
        }
    }
}

#[derive(Debug, Default)]
pub struct UiModes {
    pub search:   SearchMode,
    pub finder:   FinderMode,
    pub settings: SettingsMode,
    pub keymap:   KeymapMode,
    pub exit:     ExitMode,
}

#[derive(Debug)]
pub struct ScanState {
    pub phase:          ScanPhase,
    pub started_at:     Instant,
    pub run_count:      u64,
    pub startup_phases: StartupPhaseTracker,
}

impl ScanState {
    pub fn new(started_at: Instant) -> Self {
        Self {
            phase: ScanPhase::Running,
            started_at,
            run_count: 1,
            startup_phases: StartupPhaseTracker::default(),
        }
    }
}

#[derive(Debug, Default)]
pub struct SelectionPaths {
    pub last_selected:      Option<String>,
    pub selected_project:   Option<String>,
    pub collapsed_selected: Option<String>,
    pub collapsed_anchor:   Option<String>,
}

impl SelectionPaths {
    pub fn new() -> Self {
        Self {
            last_selected: crate::tui::terminal::load_last_selected(),
            ..Self::default()
        }
    }
}

pub struct FinderState {
    pub query:      String,
    pub results:    Vec<usize>,
    pub total:      usize,
    pub pane:       Pane,
    pub index:      Vec<FinderItem>,
    pub col_widths: [usize; FINDER_COLUMN_COUNT],
}

impl FinderState {
    pub const fn new() -> Self {
        Self {
            query:      String::new(),
            results:    Vec::new(),
            total:      0,
            pane:       Pane::new(),
            index:      Vec::new(),
            col_widths: [0; FINDER_COLUMN_COUNT],
        }
    }
}

pub struct BuildQueue<T> {
    pub tx:     mpsc::Sender<T>,
    pub rx:     Receiver<T>,
    pub active: Option<u64>,
    pub latest: u64,
}

impl<T> BuildQueue<T> {
    const fn new(tx: mpsc::Sender<T>, rx: Receiver<T>) -> Self {
        Self {
            tx,
            rx,
            active: None,
            latest: 0,
        }
    }
}

pub struct AsyncBuildState {
    pub tree: BuildQueue<TreeBuildResult>,
    pub fit:  BuildQueue<FitWidthsBuildResult>,
    pub disk: BuildQueue<DiskCacheBuildResult>,
}

impl AsyncBuildState {
    pub fn new(channels: BuildChannels) -> Self {
        Self {
            tree: BuildQueue::new(channels.tree_tx, channels.tree_rx),
            fit:  BuildQueue::new(channels.fit_tx, channels.fit_rx),
            disk: BuildQueue::new(channels.disk_tx, channels.disk_rx),
        }
    }
}

pub struct BuildChannels {
    pub tree_tx: mpsc::Sender<TreeBuildResult>,
    pub tree_rx: Receiver<TreeBuildResult>,
    pub fit_tx:  mpsc::Sender<FitWidthsBuildResult>,
    pub fit_rx:  Receiver<FitWidthsBuildResult>,
    pub disk_tx: mpsc::Sender<DiskCacheBuildResult>,
    pub disk_rx: Receiver<DiskCacheBuildResult>,
}

impl BuildChannels {
    pub fn new() -> Self {
        let (tree_tx, tree_rx) = mpsc::channel();
        let (fit_tx, fit_rx) = mpsc::channel();
        let (disk_tx, disk_rx) = mpsc::channel();
        Self {
            tree_tx,
            tree_rx,
            fit_tx,
            fit_rx,
            disk_tx,
            disk_rx,
        }
    }
}

#[derive(Default)]
pub struct PollBackgroundStats {
    pub bg_msgs:             usize,
    pub disk_usage_msgs:     usize,
    pub git_path_state_msgs: usize,
    pub git_info_msgs:       usize,
    pub lint_status_msgs:    usize,
    pub ci_msgs:             usize,
    pub example_msgs:        usize,
    pub tree_results:        usize,
    pub fit_results:         usize,
    pub disk_results:        usize,
    pub needs_rebuild:       bool,
}

/// What a visible row represents.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VisibleRow {
    /// A top-level project/workspace root.
    Root { node_index: usize },
    /// A group header (e.g., "examples").
    GroupHeader {
        node_index:  usize,
        group_index: usize,
    },
    /// An actual project member.
    Member {
        node_index:   usize,
        group_index:  usize,
        member_index: usize,
    },
    /// A vendored crate nested directly under the root project.
    Vendored {
        node_index:     usize,
        vendored_index: usize,
    },
    /// A worktree entry shown directly under the parent node.
    WorktreeEntry {
        node_index:     usize,
        worktree_index: usize,
    },
    /// A group header inside an expanded worktree entry.
    WorktreeGroupHeader {
        node_index:     usize,
        worktree_index: usize,
        group_index:    usize,
    },
    /// A member inside an expanded worktree entry.
    WorktreeMember {
        node_index:     usize,
        worktree_index: usize,
        group_index:    usize,
        member_index:   usize,
    },
    /// A vendored crate nested under a worktree entry.
    WorktreeVendored {
        node_index:     usize,
        worktree_index: usize,
        vendored_index: usize,
    },
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub enum LintRollupKey {
    Root {
        node_index: usize,
    },
    Worktree {
        node_index:     usize,
        worktree_index: usize,
    },
}

/// Per-project CI state. Replaces the scattered `(ci_runs, ci_fetching,
/// ci_no_more_runs, ci_fetch_count)` fields with a single enum so invalid
/// combinations are unrepresentable.
pub enum CiState {
    /// A fetch-more request is in progress. Keeps existing runs visible
    /// so the UI never flashes empty during pagination.
    Fetching { runs: Vec<CiRun>, count: u32 },
    /// Runs are available (possibly empty when the repo genuinely has no CI).
    Loaded {
        runs:      Vec<CiRun>,
        exhausted: bool,
    },
}

impl CiState {
    /// Access the runs regardless of which variant we are in.
    pub fn runs(&self) -> &[CiRun] {
        match self {
            Self::Fetching { runs, .. } | Self::Loaded { runs, .. } => runs,
        }
    }

    pub const fn is_fetching(&self) -> bool { matches!(self, Self::Fetching { .. }) }

    pub const fn is_exhausted(&self) -> bool {
        matches!(
            self,
            Self::Loaded {
                exhausted: true,
                ..
            }
        )
    }

    pub const fn fetch_count(&self) -> u32 {
        match self {
            Self::Fetching { count, .. } => *count,
            Self::Loaded { .. } => 0,
        }
    }
}

/// Generation-stamped detail cache. Automatically stale when `detail_generation`
/// on `App` has advanced past the generation stored here.
pub struct DetailCache {
    pub generation: u64,
    pub selection:  String,
    pub info:       DetailInfo,
}

pub struct App {
    pub current_config:           CargoPortConfig,
    pub scan_root:                PathBuf,
    pub http_client:              HttpClient,
    pub all_projects:             Vec<Project>,
    pub nodes:                    Vec<ProjectNode>,
    pub flat_entries:             Vec<FlatEntry>,
    pub disk_usage:               HashMap<String, u64>,
    pub ci_state:                 HashMap<String, CiState>,
    pub lint_status:              HashMap<String, LintStatus>,
    pub lint_cache_usage:         CacheUsage,
    pub lint_runs:                HashMap<String, Vec<LintRun>>,
    pub lint_rollup_status:       HashMap<LintRollupKey, LintStatus>,
    pub lint_rollup_paths:        HashMap<LintRollupKey, Vec<String>>,
    pub lint_rollup_keys_by_path: HashMap<String, Vec<LintRollupKey>>,
    pub git_info:                 HashMap<String, GitInfo>,
    pub git_path_states:          HashMap<String, GitPathState>,
    pub cargo_active_paths:       HashSet<String>,
    pub crates_versions:          HashMap<String, String>,
    pub crates_downloads:         HashMap<String, u64>,
    pub stars:                    HashMap<String, u64>,
    pub repo_descriptions:        HashMap<String, String>,
    pub bg_tx:                    mpsc::Sender<BackgroundMsg>,
    pub bg_rx:                    Receiver<BackgroundMsg>,
    pub fully_loaded:             HashSet<String>,
    pub priority_fetch_path:      Option<String>,
    pub expanded:                 HashSet<ExpandKey>,
    pub list_state:               ListState,
    pub search_query:             String,
    pub filtered:                 Vec<usize>,
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
    pub bottom_panel:             BottomPanel,
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
    pub running_clean_paths:      HashSet<String>,
    pub clean_toast:              Option<ToastTaskId>,
    pub running_lint_paths:       HashSet<String>,
    pub lint_toast:               Option<ToastTaskId>,

    // Disk watcher
    pub watch_tx:             mpsc::Sender<WatchRequest>,
    pub lint_runtime:         Option<RuntimeHandle>,
    pub unreachable_services: HashSet<ServiceKind>,
    pub service_retry_active: HashSet<ServiceKind>,

    // Projects whose directories have been deleted from disk.
    pub deleted_projects: HashSet<String>,

    // Projects the user has explicitly dismissed via [x].
    pub dismissed_projects: HashSet<String>,

    // Universal finder
    pub selection_paths: SelectionPaths,
    pub finder:          FinderState,

    // Caches for per-frame hot paths
    pub cached_visible_rows: Vec<VisibleRow>,
    pub cached_root_sorted:  Vec<u64>,
    pub cached_child_sorted: HashMap<usize, Vec<u64>>,
    pub cached_fit_widths:   ResolvedWidths,
    pub builds:              AsyncBuildState,
    pub data_generation:     u64,
    pub detail_generation:   u64,
    pub cached_detail:       Option<DetailCache>,
    pub layout_cache:        LayoutCache,

    pub status_flash:          Option<(String, std::time::Instant)>,
    pub toasts:                ToastManager,
    pub config_path:           Option<PathBuf>,
    pub config_last_seen:      Option<ConfigFileStamp>,
    pub current_keymap:        ResolvedKeymap,
    pub keymap_path:           Option<PathBuf>,
    pub keymap_last_seen:      Option<ConfigFileStamp>,
    pub keymap_diagnostics_id: Option<u64>,
    pub keymap_pane:           Pane,
    pub keymap_conflict:       Option<String>,
    pub ui_modes:              UiModes,
    pub dirty:                 DirtyState,
    pub scan:                  ScanState,
    pub selection:             SelectionSync,
    #[cfg(test)]
    pub retry_spawn_mode:      RetrySpawnMode,
}
