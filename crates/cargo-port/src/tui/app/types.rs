use std::collections::HashSet;
use std::time::Duration;
use std::time::Instant;

use super::phase_state::KeyedPhase;
use crate::ci::OwnerRepo;
use crate::project::AbsolutePath;
use crate::tui::finder::FINDER_COLUMN_COUNT;
use crate::tui::finder::FinderItem;
use crate::tui::panes::PaneId;
use crate::tui::terminal;
use crate::tui::toasts::ToastTaskId;

/// An action waiting for user confirmation (y/n).
pub enum ConfirmAction {
    /// `cargo clean` on the project at this absolute path.
    Clean(AbsolutePath),
    /// `cargo clean` fanned out across every checkout in a worktree
    /// group (primary + every linked worktree). Triggered by the
    /// Clean shortcut when a `VisibleRow::Root` over a
    /// `WorktreeGroup` is selected.
    CleanGroup {
        primary: AbsolutePath,
        linked:  Vec<AbsolutePath>,
    },
}

#[derive(Clone)]
pub struct PendingClean {
    pub abs_path: AbsolutePath,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HoveredPaneRow {
    pub pane: PaneId,
    pub row:  usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DiscoveryShimmer {
    pub started_at: Instant,
    pub duration:   Duration,
}

impl DiscoveryShimmer {
    pub const fn new(started_at: Instant, duration: Duration) -> Self {
        Self {
            started_at,
            duration,
        }
    }

    pub fn is_active_at(self, now: Instant) -> bool {
        now.duration_since(self.started_at) < self.duration
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiscoveryRowKind {
    Root,
    WorktreeEntry,
    PathOnly,
}

impl DiscoveryRowKind {
    pub(super) const fn allows_parent_kind(self, kind: Self) -> bool {
        matches!(
            (self, kind),
            (Self::Root, Self::Root)
                | (Self::WorktreeEntry, Self::WorktreeEntry)
                | (Self::PathOnly, Self::PathOnly)
        )
    }

    pub(super) const fn discriminant(self) -> u8 {
        match self {
            Self::Root => 0,
            Self::WorktreeEntry => 1,
            Self::PathOnly => 2,
        }
    }
}

#[derive(Debug, Default)]
pub struct StartupPhaseTracker {
    pub scan_complete_at:    Option<Instant>,
    pub startup_toast:       Option<ToastTaskId>,
    pub startup_complete_at: Option<Instant>,

    pub disk:     KeyedPhase<AbsolutePath>,
    pub git:      KeyedPhase<AbsolutePath>,
    pub repo:     KeyedPhase<OwnerRepo>,
    /// Keyed on workspace root; seen when a `BackgroundMsg::CargoMetadata`
    /// arrival is either merged into the store or converted into an error
    /// toast.
    pub metadata: KeyedPhase<AbsolutePath>,
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
pub enum ScanPhase {
    #[default]
    Running,
    Complete,
}

impl ScanPhase {
    pub const fn is_complete(self) -> bool { matches!(self, Self::Complete) }
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
    pub terminal: Dirtiness,
}

impl DirtyState {
    pub const fn initial() -> Self {
        Self {
            terminal: Dirtiness::Clean,
        }
    }
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
    pub last_selected:      Option<AbsolutePath>,
    pub selected_project:   Option<AbsolutePath>,
    pub collapsed_selected: Option<AbsolutePath>,
    pub collapsed_anchor:   Option<AbsolutePath>,
}

impl SelectionPaths {
    pub fn new() -> Self {
        Self {
            last_selected: terminal::load_last_selected(),
            ..Self::default()
        }
    }
}

pub struct FinderState {
    pub query:      String,
    pub results:    Vec<usize>,
    pub total:      usize,
    pub index:      Vec<FinderItem>,
    pub col_widths: [usize; FINDER_COLUMN_COUNT],
}

impl FinderState {
    pub const fn new() -> Self {
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

/// Runtime-only CI fetch tracking. Persistent CI data lives on the project
/// hierarchy; this only records which owner paths currently have a request
/// in flight.
#[derive(Default)]
pub struct CiFetchTracker {
    inner: HashSet<AbsolutePath>,
}

impl CiFetchTracker {
    pub fn start(&mut self, path: AbsolutePath) { self.inner.insert(path); }

    pub fn complete(&mut self, path: &std::path::Path) -> bool { self.inner.remove(path) }

    pub(super) fn is_fetching(&self, path: &std::path::Path) -> bool { self.inner.contains(path) }

    pub(super) fn clear(&mut self) { self.inner.clear(); }

    pub(super) fn retain(&mut self, mut keep: impl FnMut(&AbsolutePath) -> bool) {
        self.inner.retain(|path| keep(path));
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CiRunDisplayMode {
    #[default]
    BranchOnly,
    All,
}
