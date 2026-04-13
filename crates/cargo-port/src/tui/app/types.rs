use std::collections::HashMap;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::mpsc::Receiver;
use std::time::Duration;
use std::time::Instant;
use std::time::SystemTime;

use crate::ci::CiRun;
use crate::ci::OwnerRepo;
use crate::project::AbsolutePath;
use crate::tui::columns::ResolvedWidths;
use crate::tui::detail::DetailInfo;
use crate::tui::finder::FINDER_COLUMN_COUNT;
use crate::tui::finder::FinderItem;
use crate::tui::toasts::ToastTaskId;
use crate::tui::types::Pane;

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
pub(in super::super) struct ConfigFileStamp {
    pub modified: Option<SystemTime>,
    pub len:      u64,
}

pub(in super::super) struct FitWidthsBuildResult {
    pub build_id: u64,
    pub widths:   ResolvedWidths,
}

pub(in super::super) struct DiskCacheBuildResult {
    pub build_id:     u64,
    pub root_sorted:  Vec<u64>,
    pub child_sorted: HashMap<usize, Vec<u64>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchHit {
    pub abs_path:     AbsolutePath,
    pub display_path: String,
    pub name:         String,
    pub score:        u16,
    pub is_rust:      bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in super::super) struct DiscoveryShimmer {
    pub started_at: Instant,
    pub duration:   Duration,
}

impl DiscoveryShimmer {
    pub(in super::super) const fn new(started_at: Instant, duration: Duration) -> Self {
        Self {
            started_at,
            duration,
        }
    }

    pub(in super::super) fn is_active_at(self, now: Instant) -> bool {
        now.duration_since(self.started_at) < self.duration
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiscoveryRowKind {
    Root,
    WorktreeEntry,
    PathOnly,
    Search,
}

#[derive(Debug, Default)]
pub(in super::super) struct StartupPhaseTracker {
    pub scan_complete_at:    Option<Instant>,
    pub disk_expected:       Option<usize>,
    pub disk_seen:           HashSet<PathBuf>,
    pub disk_complete_at:    Option<Instant>,
    pub git_expected:        HashSet<PathBuf>,
    pub git_seen:            HashSet<PathBuf>,
    pub git_complete_at:     Option<Instant>,
    pub repo_expected:       HashSet<OwnerRepo>,
    pub repo_seen:           HashSet<OwnerRepo>,
    pub repo_complete_at:    Option<Instant>,
    pub git_toast:           Option<ToastTaskId>,
    pub repo_toast:          Option<ToastTaskId>,
    pub lint_expected:       Option<HashSet<PathBuf>>,
    pub lint_seen_terminal:  HashSet<PathBuf>,
    pub lint_complete_at:    Option<Instant>,
    pub startup_complete_at: Option<Instant>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in super::super) enum Dirtiness {
    #[default]
    Clean,
    Dirty,
}

impl Dirtiness {
    pub(in super::super) const fn is_dirty(self) -> bool { matches!(self, Self::Dirty) }

    pub(in super::super) const fn mark_dirty(&mut self) { *self = Self::Dirty; }

    pub(in super::super) const fn mark_clean(&mut self) { *self = Self::Clean; }
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
pub(in super::super) enum FinderMode {
    #[default]
    Hidden,
    Visible,
}

impl FinderMode {
    pub(in super::super) const fn is_visible(self) -> bool { matches!(self, Self::Visible) }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in super::super) enum SettingsMode {
    #[default]
    Hidden,
    Browsing,
    Editing,
}

impl SettingsMode {
    pub(in super::super) const fn is_visible(self) -> bool { !matches!(self, Self::Hidden) }

    pub(in super::super) const fn is_editing(self) -> bool { matches!(self, Self::Editing) }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in super::super) enum KeymapMode {
    #[default]
    Hidden,
    Browsing,
    AwaitingKey,
}

impl KeymapMode {
    pub(in super::super) const fn is_visible(self) -> bool { !matches!(self, Self::Hidden) }

    pub(in super::super) const fn is_awaiting_key(self) -> bool {
        matches!(self, Self::AwaitingKey)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in super::super) enum ScanPhase {
    #[default]
    Running,
    Complete,
}

impl ScanPhase {
    pub(in super::super) const fn is_complete(self) -> bool { matches!(self, Self::Complete) }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in super::super) enum ExitMode {
    #[default]
    Continue,
    Quit,
    Restart,
}

impl ExitMode {
    pub(in super::super) const fn should_quit(self) -> bool {
        matches!(self, Self::Quit | Self::Restart)
    }

    pub(in super::super) const fn should_restart(self) -> bool { matches!(self, Self::Restart) }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in super::super) enum SelectionSync {
    #[default]
    Stable,
    Changed,
}

impl SelectionSync {
    pub(in super::super) const fn is_changed(self) -> bool { matches!(self, Self::Changed) }
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum RetrySpawnMode {
    #[default]
    Enabled,
    Disabled,
}

#[cfg(test)]
impl RetrySpawnMode {
    pub(super) const fn is_enabled(self) -> bool { matches!(self, Self::Enabled) }
}

#[derive(Debug)]
pub(in super::super) struct DirtyState {
    pub rows:       Dirtiness,
    pub disk_cache: Dirtiness,
    pub fit_widths: Dirtiness,
    pub finder:     Dirtiness,
    pub terminal:   Dirtiness,
}

impl DirtyState {
    pub(in super::super) const fn initial() -> Self {
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
pub(in super::super) struct UiModes {
    pub search:   SearchMode,
    pub finder:   FinderMode,
    pub settings: SettingsMode,
    pub keymap:   KeymapMode,
    pub exit:     ExitMode,
}

#[derive(Debug)]
pub(in super::super) struct ScanState {
    pub phase:          ScanPhase,
    pub started_at:     Instant,
    pub run_count:      u64,
    pub startup_phases: StartupPhaseTracker,
}

impl ScanState {
    pub(in super::super) fn new(started_at: Instant) -> Self {
        Self {
            phase: ScanPhase::Running,
            started_at,
            run_count: 1,
            startup_phases: StartupPhaseTracker::default(),
        }
    }
}

#[derive(Debug, Default)]
pub(in super::super) struct SelectionPaths {
    pub last_selected:      Option<AbsolutePath>,
    pub selected_project:   Option<AbsolutePath>,
    pub collapsed_selected: Option<AbsolutePath>,
    pub collapsed_anchor:   Option<AbsolutePath>,
}

impl SelectionPaths {
    pub(in super::super) fn new() -> Self {
        Self {
            last_selected: crate::tui::terminal::load_last_selected(),
            ..Self::default()
        }
    }
}

pub(in super::super) struct FinderState {
    pub query:      String,
    pub results:    Vec<usize>,
    pub total:      usize,
    pub pane:       Pane,
    pub index:      Vec<FinderItem>,
    pub col_widths: [usize; FINDER_COLUMN_COUNT],
}

impl FinderState {
    pub(in super::super) const fn new() -> Self {
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

pub(in super::super) struct BuildQueue<T> {
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

pub(in super::super) struct AsyncBuildState {
    pub fit:  BuildQueue<FitWidthsBuildResult>,
    pub disk: BuildQueue<DiskCacheBuildResult>,
}

impl AsyncBuildState {
    pub(in super::super) fn new(channels: BuildChannels) -> Self {
        Self {
            fit:  BuildQueue::new(channels.fit_tx, channels.fit_rx),
            disk: BuildQueue::new(channels.disk_tx, channels.disk_rx),
        }
    }
}

pub(in super::super) struct BuildChannels {
    pub fit_tx:  mpsc::Sender<FitWidthsBuildResult>,
    pub fit_rx:  Receiver<FitWidthsBuildResult>,
    pub disk_tx: mpsc::Sender<DiskCacheBuildResult>,
    pub disk_rx: Receiver<DiskCacheBuildResult>,
}

impl BuildChannels {
    pub(in super::super) fn new() -> Self {
        let (fit_tx, fit_rx) = mpsc::channel();
        let (disk_tx, disk_rx) = mpsc::channel();
        Self {
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
    /// A git submodule nested under the root project.
    Submodule {
        node_index:      usize,
        submodule_index: usize,
    },
}

/// Per-project CI state. Replaces the scattered `(ci_runs, ci_fetching,
/// ci_no_more_runs, ci_fetch_count)` fields with a single enum so invalid
/// combinations are unrepresentable.
pub enum CiState {
    /// A fetch-more request is in progress. Keeps existing runs visible
    /// so the UI never flashes empty during pagination.
    Fetching { runs: Vec<CiRun> },
    /// Runs are available (possibly empty when the repo genuinely has no CI).
    Loaded {
        runs:         Vec<CiRun>,
        exhausted:    bool,
        /// Total completed workflow runs reported by the GitHub API.
        github_total: u32,
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

    /// Total completed runs reported by GitHub, or 0 if not yet fetched.
    pub const fn github_total(&self) -> u32 {
        match self {
            Self::Loaded { github_total, .. } => *github_total,
            Self::Fetching { .. } => 0,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in super::super) enum CiRunDisplayMode {
    #[default]
    BranchOnly,
    All,
}

/// Generation-stamped detail cache. Automatically stale when `detail_generation`
/// on `App` has advanced past the generation stored here.
pub(in super::super) struct DetailCache {
    pub generation: u64,
    pub selection:  String,
    pub info:       DetailInfo,
}
