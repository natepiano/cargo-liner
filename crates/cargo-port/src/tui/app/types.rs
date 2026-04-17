use std::collections::HashSet;
use std::time::Duration;
use std::time::Instant;
use std::time::SystemTime;

use crate::ci::OwnerRepo;
use crate::project::AbsolutePath;
use crate::tui::finder::FINDER_COLUMN_COUNT;
use crate::tui::finder::FinderItem;
use crate::tui::panes::PaneId;
use crate::tui::toasts::ToastTaskId;

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
    Clean(AbsolutePath),
}

#[derive(Clone)]
pub struct PendingClean {
    pub abs_path: AbsolutePath,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in super::super) struct HoveredPaneRow {
    pub pane: PaneId,
    pub row:  usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in super::super) struct ConfigFileStamp {
    pub modified: Option<SystemTime>,
    pub len:      u64,
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
}

#[derive(Debug, Default)]
pub(in super::super) struct StartupPhaseTracker {
    pub scan_complete_at:         Option<Instant>,
    pub disk_expected:            Option<usize>,
    pub disk_seen:                HashSet<AbsolutePath>,
    pub disk_complete_at:         Option<Instant>,
    pub git_expected:             HashSet<AbsolutePath>,
    pub git_seen:                 HashSet<AbsolutePath>,
    pub git_complete_at:          Option<Instant>,
    pub repo_expected:            HashSet<OwnerRepo>,
    pub repo_seen:                HashSet<OwnerRepo>,
    pub repo_complete_at:         Option<Instant>,
    pub git_toast:                Option<ToastTaskId>,
    pub repo_toast:               Option<ToastTaskId>,
    pub startup_toast:            Option<ToastTaskId>,
    pub lint_expected:            Option<HashSet<AbsolutePath>>,
    pub lint_seen_terminal:       HashSet<AbsolutePath>,
    pub lint_complete_at:         Option<Instant>,
    pub lint_startup_expected:    Option<usize>,
    pub lint_startup_seen:        usize,
    pub lint_startup_complete_at: Option<Instant>,
    pub startup_complete_at:      Option<Instant>,
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
    pub finder:   Dirtiness,
    pub terminal: Dirtiness,
}

impl DirtyState {
    pub(in super::super) const fn initial() -> Self {
        Self {
            finder:   Dirtiness::Dirty,
            terminal: Dirtiness::Clean,
        }
    }
}

#[derive(Debug, Default)]
pub(in super::super) struct UiModes {
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
    pub index:      Vec<FinderItem>,
    pub col_widths: [usize; FINDER_COLUMN_COUNT],
}

impl FinderState {
    pub(in super::super) const fn new() -> Self {
        Self {
            query:      String::new(),
            results:    Vec::new(),
            total:      0,
            index:      Vec::new(),
            col_widths: [0; FINDER_COLUMN_COUNT],
        }
    }
}

#[derive(Default)]
pub struct PollBackgroundStats {
    pub bg_msgs:          usize,
    pub disk_usage_msgs:  usize,
    pub git_info_msgs:    usize,
    pub lint_status_msgs: usize,
    pub ci_msgs:          usize,
    pub example_msgs:     usize,
    pub tree_results:     usize,
    pub fit_results:      usize,
    pub disk_results:     usize,
    pub needs_rebuild:    bool,
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

/// Runtime-only CI fetch tracking. Persistent CI data lives on the project
/// hierarchy; this only records which owner paths currently have a request
/// in flight.
#[derive(Default)]
pub struct CiFetchTracker {
    inner: HashSet<AbsolutePath>,
}

impl CiFetchTracker {
    pub(super) fn start(&mut self, path: AbsolutePath) { self.inner.insert(path); }

    pub(super) fn complete(&mut self, path: &std::path::Path) -> bool { self.inner.remove(path) }

    pub(super) fn is_fetching(&self, path: &std::path::Path) -> bool { self.inner.contains(path) }

    pub(super) fn clear(&mut self) { self.inner.clear(); }

    pub(super) fn retain(&mut self, mut keep: impl FnMut(&AbsolutePath) -> bool) {
        self.inner.retain(|path| keep(path));
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
/// Cache key for per-pane detail data. When generation and selection
/// match, the pane data on `PaneManager` is still valid.
pub(in super::super) struct DetailCacheKey {
    pub generation: u64,
    pub selection:  String,
}
