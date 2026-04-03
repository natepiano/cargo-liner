use std::collections::HashMap;
use std::collections::HashSet;
use std::collections::VecDeque;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::mpsc;
use std::sync::mpsc::Receiver;
use std::time::Duration;
use std::time::Instant;
use std::time::SystemTime;

use ratatui::widgets::ListState;

use super::detail::DetailField;
use super::detail::DetailInfo;
use super::detail::PendingCiFetch;
use super::detail::PendingExampleRun;
use super::detail::ProjectCounts;
use super::finder::FINDER_COLUMN_COUNT;
use super::finder::FinderItem;
use super::render::PREFIX_GROUP_COLLAPSED;
use super::render::PREFIX_MEMBER_INLINE;
use super::render::PREFIX_MEMBER_NAMED;
use super::render::PREFIX_VENDORED;
use super::render::PREFIX_WT_COLLAPSED;
use super::render::PREFIX_WT_FLAT;
use super::render::PREFIX_WT_GROUP_COLLAPSED;
use super::render::PREFIX_WT_MEMBER_INLINE;
use super::render::PREFIX_WT_MEMBER_NAMED;
use super::render::PREFIX_WT_VENDORED;
use super::shortcuts::InputContext;
use super::terminal::CiFetchMsg;
use super::terminal::CleanMsg;
use super::terminal::ExampleMsg;
use super::toasts::ToastManager;
use super::toasts::ToastTaskId;
use super::toasts::ToastView;
use super::types::LayoutCache;
use super::types::Pane;
use super::types::PaneId;
use crate::ci::CiRun;
use crate::ci::Conclusion;
use crate::config::Config;
use crate::config::NavigationKeys;
use crate::config::NonRustInclusion;
use crate::config::ScrollDirection;
use crate::constants::IN_SYNC;
use crate::constants::SYNC_DOWN;
use crate::constants::SYNC_UP;
use crate::http::HttpClient;
use crate::http::ServiceKind;
use crate::lint_runtime;
use crate::lint_runtime::RuntimeHandle;
use crate::port_report::LintStatus;
use crate::port_report::PortReportRun;
use crate::project::GitInfo;
use crate::project::GitOrigin;
use crate::project::GitPathState;
use crate::project::ProjectLanguage::Rust;
use crate::project::RustProject;
use crate::scan;
use crate::scan::BackgroundMsg;
use crate::scan::FlatEntry;
use crate::scan::MemberGroup;
use crate::scan::ProjectNode;
use crate::watcher;
use crate::watcher::WatchRequest;

mod background;
mod ci_state;
mod selection;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BottomPanel {
    CiRuns,
    PortReport,
}

/// An expand key: a node, group, worktree entry, or group within a worktree.
#[derive(Hash, Eq, PartialEq, Clone)]
pub(super) enum ExpandKey {
    Node(usize),
    Group(usize, usize),
    Worktree(usize, usize),
    WorktreeGroup(usize, usize, usize),
}

/// An action waiting for user confirmation (y/n).
pub(super) enum ConfirmAction {
    /// `cargo clean` on the project at this absolute path.
    Clean(String),
}

#[derive(Clone)]
pub(super) struct PendingClean {
    pub abs_path: String,
    pub toast:    ToastTaskId,
}

pub(super) use super::columns::ResolvedWidths;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ConfigFileStamp {
    modified: Option<SystemTime>,
    len:      u64,
}

struct TreeBuildResult {
    build_id:     u64,
    nodes:        Vec<ProjectNode>,
    flat_entries: Vec<FlatEntry>,
}

struct FitWidthsBuildResult {
    build_id: u64,
    widths:   ResolvedWidths,
}

struct DiskCacheBuildResult {
    build_id:     u64,
    root_sorted:  Vec<u64>,
    child_sorted: HashMap<usize, Vec<u64>>,
}

#[derive(Debug, Default)]
pub(super) struct StartupPhaseTracker {
    scan_complete_at:    Option<Instant>,
    disk_expected:       Option<usize>,
    disk_seen:           HashSet<String>,
    disk_complete_at:    Option<Instant>,
    git_expected:        HashSet<String>,
    git_seen:            HashSet<String>,
    git_complete_at:     Option<Instant>,
    repo_expected:       HashSet<String>,
    repo_seen:           HashSet<String>,
    repo_complete_at:    Option<Instant>,
    git_toast:           Option<ToastTaskId>,
    repo_toast:          Option<ToastTaskId>,
    lint_expected:       Option<HashSet<String>>,
    lint_seen_terminal:  HashSet<String>,
    lint_complete_at:    Option<Instant>,
    startup_complete_at: Option<Instant>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Dirtiness {
    #[default]
    Clean,
    Dirty,
}

impl Dirtiness {
    pub(super) const fn is_dirty(self) -> bool { matches!(self, Self::Dirty) }

    pub(super) const fn mark_dirty(&mut self) { *self = Self::Dirty; }

    pub(super) const fn mark_clean(&mut self) { *self = Self::Clean; }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum SearchMode {
    #[default]
    Inactive,
    Active,
}

impl SearchMode {
    pub(super) const fn is_active(self) -> bool { matches!(self, Self::Active) }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum FinderMode {
    #[default]
    Hidden,
    Visible,
}

impl FinderMode {
    pub(super) const fn is_visible(self) -> bool { matches!(self, Self::Visible) }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum SettingsMode {
    #[default]
    Hidden,
    Browsing,
    Editing,
}

impl SettingsMode {
    pub(super) const fn is_visible(self) -> bool { !matches!(self, Self::Hidden) }

    pub(super) const fn is_editing(self) -> bool { matches!(self, Self::Editing) }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum ScanPhase {
    #[default]
    Running,
    Complete,
}

impl ScanPhase {
    pub(super) const fn is_complete(self) -> bool { matches!(self, Self::Complete) }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum ExitMode {
    #[default]
    Continue,
    Quit,
    Restart,
}

impl ExitMode {
    pub(super) const fn should_quit(self) -> bool { matches!(self, Self::Quit | Self::Restart) }

    pub(super) const fn should_restart(self) -> bool { matches!(self, Self::Restart) }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum SelectionSync {
    #[default]
    Stable,
    Changed,
}

impl SelectionSync {
    pub(super) const fn is_changed(self) -> bool { matches!(self, Self::Changed) }
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
pub(super) struct DirtyState {
    pub(super) rows:       Dirtiness,
    pub(super) disk_cache: Dirtiness,
    pub(super) fit_widths: Dirtiness,
    pub(super) finder:     Dirtiness,
    pub(super) terminal:   Dirtiness,
}

impl DirtyState {
    const fn initial() -> Self {
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
pub(super) struct UiModes {
    pub(super) search:   SearchMode,
    pub(super) finder:   FinderMode,
    pub(super) settings: SettingsMode,
    pub(super) exit:     ExitMode,
}

#[derive(Debug)]
pub(super) struct ScanState {
    pub(super) phase:          ScanPhase,
    pub(super) started_at:     Instant,
    pub(super) run_count:      u64,
    pub(super) startup_phases: StartupPhaseTracker,
}

impl ScanState {
    fn new(started_at: Instant) -> Self {
        Self {
            phase: ScanPhase::Running,
            started_at,
            run_count: 1,
            startup_phases: StartupPhaseTracker::default(),
        }
    }
}

#[derive(Debug, Default)]
pub(super) struct SelectionPaths {
    pub(super) last_selected:      Option<String>,
    pub(super) selected_project:   Option<String>,
    pub(super) collapsed_selected: Option<String>,
    pub(super) collapsed_anchor:   Option<String>,
}

impl SelectionPaths {
    fn new() -> Self {
        Self {
            last_selected: super::terminal::load_last_selected(),
            ..Self::default()
        }
    }
}

pub(super) struct FinderState {
    pub(super) query:      String,
    pub(super) results:    Vec<usize>,
    pub(super) total:      usize,
    pub(super) pane:       Pane,
    pub(super) index:      Vec<FinderItem>,
    pub(super) col_widths: [usize; FINDER_COLUMN_COUNT],
}

impl FinderState {
    const fn new() -> Self {
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

struct BuildQueue<T> {
    tx:     mpsc::Sender<T>,
    rx:     Receiver<T>,
    active: Option<u64>,
    latest: u64,
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

struct AsyncBuildState {
    tree: BuildQueue<TreeBuildResult>,
    fit:  BuildQueue<FitWidthsBuildResult>,
    disk: BuildQueue<DiskCacheBuildResult>,
}

impl AsyncBuildState {
    fn new(channels: BuildChannels) -> Self {
        Self {
            tree: BuildQueue::new(channels.tree_tx, channels.tree_rx),
            fit:  BuildQueue::new(channels.fit_tx, channels.fit_rx),
            disk: BuildQueue::new(channels.disk_tx, channels.disk_rx),
        }
    }
}

#[derive(Default)]
pub(super) struct PollBackgroundStats {
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
pub(super) enum VisibleRow {
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
enum LintRollupKey {
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
pub(super) enum CiState {
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
pub(super) struct DetailCache {
    generation: u64,
    selection:  String,
    pub info:   DetailInfo,
}

pub(super) struct App {
    pub(super) current_config: Config,
    pub scan_root:             PathBuf,
    pub http_client:           HttpClient,
    pub all_projects:          Vec<RustProject>,
    pub nodes:                 Vec<ProjectNode>,
    pub flat_entries:          Vec<FlatEntry>,
    pub disk_usage:            HashMap<String, u64>,
    pub ci_state:              HashMap<String, CiState>,
    pub lint_status:           HashMap<String, LintStatus>,
    pub port_report_runs:      HashMap<String, Vec<PortReportRun>>,
    lint_rollup_status:        HashMap<LintRollupKey, LintStatus>,
    lint_rollup_paths:         HashMap<LintRollupKey, Vec<String>>,
    lint_rollup_keys_by_path:  HashMap<String, Vec<LintRollupKey>>,
    pub git_info:              HashMap<String, GitInfo>,
    pub git_path_states:       HashMap<String, GitPathState>,
    cargo_active_paths:        HashSet<String>,
    pub crates_versions:       HashMap<String, String>,
    pub crates_downloads:      HashMap<String, u64>,
    pub stars:                 HashMap<String, u64>,
    pub repo_descriptions:     HashMap<String, String>,
    pub bg_tx:                 mpsc::Sender<BackgroundMsg>,
    pub bg_rx:                 Receiver<BackgroundMsg>,
    pub fully_loaded:          HashSet<String>,
    pub priority_fetch_path:   Option<String>,
    pub expanded:              HashSet<ExpandKey>,
    pub list_state:            ListState,
    pub search_query:          String,
    pub filtered:              Vec<usize>,
    pub settings_pane:         Pane,
    pub settings_edit_buf:     String,
    pub settings_edit_cursor:  usize,
    pub scan_log:              Vec<String>,
    pub scan_log_state:        ListState,
    pub focused_pane:          PaneId,
    pub return_focus:          Option<PaneId>,
    pub visited_panes:         HashSet<PaneId>,
    pub package_pane:          Pane,
    pub git_pane:              Pane,
    pub targets_pane:          Pane,
    pub ci_pane:               Pane,
    pub toast_pane:            Pane,
    pub port_report_pane:      Pane,
    pub bottom_panel:          BottomPanel,
    pub pending_example_run:   Option<PendingExampleRun>,
    pub pending_ci_fetch:      Option<PendingCiFetch>,
    pub pending_cleans:        VecDeque<PendingClean>,
    pub confirm:               Option<ConfirmAction>,
    pub animation_started:     Instant,
    pub ci_fetch_tx:           mpsc::Sender<CiFetchMsg>,
    pub ci_fetch_rx:           mpsc::Receiver<CiFetchMsg>,
    pub clean_tx:              mpsc::Sender<CleanMsg>,
    pub clean_rx:              mpsc::Receiver<CleanMsg>,
    pub example_running:       Option<String>,
    pub example_child:         Arc<Mutex<Option<u32>>>,
    pub example_output:        Vec<String>,
    pub example_tx:            mpsc::Sender<ExampleMsg>,
    pub example_rx:            mpsc::Receiver<ExampleMsg>,
    running_lint_paths:        HashSet<String>,
    lint_toast:                Option<ToastTaskId>,

    // Disk watcher
    pub watch_tx:             mpsc::Sender<WatchRequest>,
    pub lint_runtime:         Option<RuntimeHandle>,
    pub unreachable_services: HashSet<ServiceKind>,
    service_retry_active:     HashSet<ServiceKind>,

    // Projects whose directories have been deleted from disk.
    pub deleted_projects: HashSet<String>,

    // Universal finder
    pub(super) selection_paths: SelectionPaths,
    pub(super) finder:          FinderState,

    // Caches for per-frame hot paths
    pub cached_visible_rows:      Vec<VisibleRow>,
    pub cached_root_sorted:       Vec<u64>,
    pub cached_child_sorted:      HashMap<usize, Vec<u64>>,
    pub cached_fit_widths:        ResolvedWidths,
    builds:                       AsyncBuildState,
    pub(super) data_generation:   u64,
    pub(super) detail_generation: u64,
    pub(super) cached_detail:     Option<DetailCache>,
    pub(super) layout_cache:      LayoutCache,

    pub(super) status_flash:     Option<(String, std::time::Instant)>,
    pub(super) toasts:           ToastManager,
    config_path:                 Option<PathBuf>,
    config_last_seen:            Option<ConfigFileStamp>,
    pub(super) ui_modes:         UiModes,
    pub(super) dirty:            DirtyState,
    pub(super) scan:             ScanState,
    pub(super) selection:        SelectionSync,
    #[cfg(test)]
    pub(super) retry_spawn_mode: RetrySpawnMode,
}

/// Build the flat list of visible rows from the node tree and expansion state.
fn build_visible_rows(nodes: &[ProjectNode], expanded: &HashSet<ExpandKey>) -> Vec<VisibleRow> {
    let mut rows = Vec::new();
    for (ni, node) in nodes.iter().enumerate() {
        rows.push(VisibleRow::Root { node_index: ni });
        if expanded.contains(&ExpandKey::Node(ni)) {
            for (gi, group) in node.groups.iter().enumerate() {
                if group.name.is_empty() {
                    for (mi, _) in group.members.iter().enumerate() {
                        rows.push(VisibleRow::Member {
                            node_index:   ni,
                            group_index:  gi,
                            member_index: mi,
                        });
                    }
                } else {
                    rows.push(VisibleRow::GroupHeader {
                        node_index:  ni,
                        group_index: gi,
                    });
                    if expanded.contains(&ExpandKey::Group(ni, gi)) {
                        for (mi, _) in group.members.iter().enumerate() {
                            rows.push(VisibleRow::Member {
                                node_index:   ni,
                                group_index:  gi,
                                member_index: mi,
                            });
                        }
                    }
                }
            }

            for (vi, _) in node.vendored.iter().enumerate() {
                rows.push(VisibleRow::Vendored {
                    node_index:     ni,
                    vendored_index: vi,
                });
            }

            for (wi, wt) in node.worktrees.iter().enumerate() {
                rows.push(VisibleRow::WorktreeEntry {
                    node_index:     ni,
                    worktree_index: wi,
                });
                if wt.has_children() && expanded.contains(&ExpandKey::Worktree(ni, wi)) {
                    for (gi, group) in wt.groups.iter().enumerate() {
                        if group.name.is_empty() {
                            for (mi, _) in group.members.iter().enumerate() {
                                rows.push(VisibleRow::WorktreeMember {
                                    node_index:     ni,
                                    worktree_index: wi,
                                    group_index:    gi,
                                    member_index:   mi,
                                });
                            }
                        } else {
                            rows.push(VisibleRow::WorktreeGroupHeader {
                                node_index:     ni,
                                worktree_index: wi,
                                group_index:    gi,
                            });
                            if expanded.contains(&ExpandKey::WorktreeGroup(ni, wi, gi)) {
                                for (mi, _) in group.members.iter().enumerate() {
                                    rows.push(VisibleRow::WorktreeMember {
                                        node_index:     ni,
                                        worktree_index: wi,
                                        group_index:    gi,
                                        member_index:   mi,
                                    });
                                }
                            }
                        }
                    }

                    for (vi, _) in wt.vendored.iter().enumerate() {
                        rows.push(VisibleRow::WorktreeVendored {
                            node_index:     ni,
                            worktree_index: wi,
                            vendored_index: vi,
                        });
                    }
                }
            }
        }
    }
    rows
}

fn live_worktree_count_for_node(node: &ProjectNode, deleted_projects: &HashSet<String>) -> usize {
    node.worktrees
        .iter()
        .filter(|wt| !deleted_projects.contains(&wt.project.path))
        .count()
}

fn unique_node_paths(node: &ProjectNode) -> Vec<&str> {
    let mut seen = HashSet::new();
    let mut paths = Vec::new();

    for path in std::iter::once(node.project.path.as_str())
        .chain(node.worktrees.iter().map(|wt| wt.project.path.as_str()))
    {
        if seen.insert(path) {
            paths.push(path);
        }
    }

    paths
}

fn disk_bytes_for_node_snapshot(
    node: &ProjectNode,
    disk_usage: &HashMap<String, u64>,
) -> Option<u64> {
    if node.worktrees.is_empty() {
        return disk_usage.get(&node.project.path).copied();
    }
    let mut total = 0;
    let mut any_data = false;
    for path in unique_node_paths(node) {
        if let Some(&bytes) = disk_usage.get(path) {
            total += bytes;
            any_data = true;
        }
    }
    if any_data { Some(total) } else { None }
}

fn formatted_disk_snapshot(disk_usage: &HashMap<String, u64>, path: &str) -> String {
    disk_usage.get(path).copied().map_or_else(
        || super::render::format_bytes(0),
        super::render::format_bytes,
    )
}

fn formatted_disk_for_node_snapshot(
    node: &ProjectNode,
    disk_usage: &HashMap<String, u64>,
) -> String {
    disk_bytes_for_node_snapshot(node, disk_usage).map_or_else(
        || super::render::format_bytes(0),
        super::render::format_bytes,
    )
}

fn git_sync_snapshot(
    git_info: &HashMap<String, GitInfo>,
    git_path_states: &HashMap<String, GitPathState>,
    path: &str,
) -> String {
    if matches!(
        git_path_states
            .get(path)
            .copied()
            .unwrap_or(GitPathState::OutsideRepo),
        GitPathState::Untracked | GitPathState::Ignored
    ) {
        return String::new();
    }
    let Some(info) = git_info.get(path) else {
        return String::new();
    };
    match info.ahead_behind {
        Some((0, 0)) => IN_SYNC.to_string(),
        Some((a, 0)) => format!("{SYNC_UP}{a}"),
        Some((0, b)) => format!("{SYNC_DOWN}{b}"),
        Some((a, b)) => format!("{SYNC_UP}{a}{SYNC_DOWN}{b}"),
        None if info.origin != GitOrigin::Local => "-".to_string(),
        None => String::new(),
    }
}

fn build_fit_widths_snapshot(
    nodes: &[ProjectNode],
    disk_usage: &HashMap<String, u64>,
    git_info: &HashMap<String, GitInfo>,
    git_path_states: &HashMap<String, GitPathState>,
    deleted_projects: &HashSet<String>,
    lint_enabled: bool,
    generation: u64,
) -> ResolvedWidths {
    let mut widths = ResolvedWidths::new(lint_enabled);

    for node in nodes {
        observe_node_fit_widths(
            &mut widths,
            node,
            disk_usage,
            git_info,
            git_path_states,
            deleted_projects,
        );
    }

    widths.generation = generation;
    widths
}

fn observe_node_fit_widths(
    widths: &mut ResolvedWidths,
    node: &ProjectNode,
    disk_usage: &HashMap<String, u64>,
    git_info: &HashMap<String, GitInfo>,
    git_path_states: &HashMap<String, GitPathState>,
    deleted_projects: &HashSet<String>,
) {
    use super::columns::COL_DISK;
    use super::columns::COL_SYNC;

    let dw = super::columns::display_width;
    App::observe_name_width(
        widths,
        App::fit_name_for_node(node, live_worktree_count_for_node(node, deleted_projects)),
    );
    widths.observe(
        COL_DISK,
        dw(&formatted_disk_for_node_snapshot(node, disk_usage)),
    );
    widths.observe(
        COL_SYNC,
        dw(&git_sync_snapshot(
            git_info,
            git_path_states,
            &node.project.path,
        )),
    );

    observe_member_group_fit_widths(widths, &node.groups, disk_usage, git_info, git_path_states);
    observe_vendored_fit_widths(widths, &node.vendored, disk_usage, PREFIX_VENDORED);
    for worktree in &node.worktrees {
        observe_worktree_fit_widths(widths, worktree, disk_usage, git_info, git_path_states);
    }
}

fn observe_member_group_fit_widths(
    widths: &mut ResolvedWidths,
    groups: &[MemberGroup],
    disk_usage: &HashMap<String, u64>,
    git_info: &HashMap<String, GitInfo>,
    git_path_states: &HashMap<String, GitPathState>,
) {
    use super::columns::COL_DISK;
    use super::columns::COL_SYNC;

    let dw = super::columns::display_width;
    for group in groups {
        for member in &group.members {
            let prefix = if group.name.is_empty() {
                PREFIX_MEMBER_INLINE
            } else {
                PREFIX_MEMBER_NAMED
            };
            App::observe_name_width(widths, dw(prefix) + dw(&member.display_name()));
            widths.observe(
                COL_DISK,
                dw(&formatted_disk_snapshot(disk_usage, &member.path)),
            );
            widths.observe(
                COL_SYNC,
                dw(&git_sync_snapshot(git_info, git_path_states, &member.path)),
            );
        }
        if !group.name.is_empty() {
            let label = format!("{} ({})", group.name, group.members.len());
            App::observe_name_width(widths, dw(PREFIX_GROUP_COLLAPSED) + dw(&label));
        }
    }
}

fn observe_vendored_fit_widths(
    widths: &mut ResolvedWidths,
    vendored: &[RustProject],
    disk_usage: &HashMap<String, u64>,
    prefix: &str,
) {
    use super::columns::COL_DISK;

    let dw = super::columns::display_width;
    for project in vendored {
        let label = format!("{} (vendored)", project.display_name());
        App::observe_name_width(widths, dw(prefix) + dw(&label));
        widths.observe(
            COL_DISK,
            dw(&formatted_disk_snapshot(disk_usage, &project.path)),
        );
    }
}

fn observe_worktree_fit_widths(
    widths: &mut ResolvedWidths,
    worktree: &ProjectNode,
    disk_usage: &HashMap<String, u64>,
    git_info: &HashMap<String, GitInfo>,
    git_path_states: &HashMap<String, GitPathState>,
) {
    use super::columns::COL_DISK;
    use super::columns::COL_SYNC;

    let dw = super::columns::display_width;
    let worktree_name = worktree
        .project
        .worktree_name
        .as_deref()
        .unwrap_or(&worktree.project.path);
    let worktree_prefix = if worktree.has_children() {
        PREFIX_WT_COLLAPSED
    } else {
        PREFIX_WT_FLAT
    };
    App::observe_name_width(widths, dw(worktree_prefix) + dw(worktree_name));
    widths.observe(
        COL_DISK,
        dw(&formatted_disk_snapshot(disk_usage, &worktree.project.path)),
    );
    widths.observe(
        COL_SYNC,
        dw(&git_sync_snapshot(
            git_info,
            git_path_states,
            &worktree.project.path,
        )),
    );
    observe_worktree_group_fit_widths(
        widths,
        &worktree.groups,
        disk_usage,
        git_info,
        git_path_states,
    );
    observe_vendored_fit_widths(widths, &worktree.vendored, disk_usage, PREFIX_WT_VENDORED);
}

fn observe_worktree_group_fit_widths(
    widths: &mut ResolvedWidths,
    groups: &[MemberGroup],
    disk_usage: &HashMap<String, u64>,
    git_info: &HashMap<String, GitInfo>,
    git_path_states: &HashMap<String, GitPathState>,
) {
    use super::columns::COL_DISK;
    use super::columns::COL_SYNC;

    let dw = super::columns::display_width;
    for group in groups {
        for member in &group.members {
            let prefix = if group.name.is_empty() {
                PREFIX_WT_MEMBER_INLINE
            } else {
                PREFIX_WT_MEMBER_NAMED
            };
            App::observe_name_width(widths, dw(prefix) + dw(&member.display_name()));
            widths.observe(
                COL_DISK,
                dw(&formatted_disk_snapshot(disk_usage, &member.path)),
            );
            widths.observe(
                COL_SYNC,
                dw(&git_sync_snapshot(git_info, git_path_states, &member.path)),
            );
        }
        if !group.name.is_empty() {
            let label = format!("{} ({})", group.name, group.members.len());
            App::observe_name_width(widths, dw(PREFIX_WT_GROUP_COLLAPSED) + dw(&label));
        }
    }
}

fn build_disk_cache_snapshot(
    nodes: &[ProjectNode],
    disk_usage: &HashMap<String, u64>,
) -> (Vec<u64>, HashMap<usize, Vec<u64>>) {
    let mut root_sorted = Vec::new();
    for node in nodes {
        if let Some(bytes) = disk_bytes_for_node_snapshot(node, disk_usage) {
            root_sorted.push(bytes);
        }
    }
    root_sorted.sort_unstable();

    let mut child_sorted = HashMap::new();
    for (ni, node) in nodes.iter().enumerate() {
        let mut values = Vec::new();
        for member in App::all_group_members(node) {
            if let Some(&bytes) = disk_usage.get(&member.path) {
                values.push(bytes);
            }
        }
        for vendored in App::all_vendored_projects(node) {
            if let Some(&bytes) = disk_usage.get(&vendored.path) {
                values.push(bytes);
            }
        }
        for wt in &node.worktrees {
            if let Some(&bytes) = disk_usage.get(&wt.project.path) {
                values.push(bytes);
            }
        }
        if !values.is_empty() {
            values.sort_unstable();
            child_sorted.insert(ni, values);
        }
    }

    (root_sorted, child_sorted)
}

fn initial_list_state(nodes: &[ProjectNode]) -> ListState {
    let mut state = ListState::default();
    if !nodes.is_empty() {
        state.select(Some(0));
    }
    state
}

struct AppChannels {
    example_tx:  mpsc::Sender<ExampleMsg>,
    example_rx:  mpsc::Receiver<ExampleMsg>,
    ci_fetch_tx: mpsc::Sender<CiFetchMsg>,
    ci_fetch_rx: mpsc::Receiver<CiFetchMsg>,
    clean_tx:    mpsc::Sender<CleanMsg>,
    clean_rx:    mpsc::Receiver<CleanMsg>,
}

impl AppChannels {
    fn new() -> Self {
        let (example_tx, example_rx) = mpsc::channel();
        let (ci_fetch_tx, ci_fetch_rx) = mpsc::channel();
        let (clean_tx, clean_rx) = mpsc::channel();
        Self {
            example_tx,
            example_rx,
            ci_fetch_tx,
            ci_fetch_rx,
            clean_tx,
            clean_rx,
        }
    }
}

struct BuildChannels {
    tree_tx: mpsc::Sender<TreeBuildResult>,
    tree_rx: Receiver<TreeBuildResult>,
    fit_tx:  mpsc::Sender<FitWidthsBuildResult>,
    fit_rx:  Receiver<FitWidthsBuildResult>,
    disk_tx: mpsc::Sender<DiskCacheBuildResult>,
    disk_rx: Receiver<DiskCacheBuildResult>,
}

impl BuildChannels {
    fn new() -> Self {
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

struct AppInit {
    config_path:      Option<PathBuf>,
    config_last_seen: Option<ConfigFileStamp>,
    lint_warning:     Option<String>,
    lint_runtime:     Option<RuntimeHandle>,
    watch_tx:         mpsc::Sender<WatchRequest>,
    nodes:            Vec<ProjectNode>,
    flat_entries:     Vec<FlatEntry>,
    list_state:       ListState,
}

impl AppInit {
    fn new(
        scan_root: &Path,
        projects: &[RustProject],
        bg_tx: &mpsc::Sender<BackgroundMsg>,
        cfg: &Config,
        http_client: &HttpClient,
    ) -> Self {
        crate::config::set_active_config(cfg);
        let config_path = crate::config::config_path();
        let config_last_seen = config_path.as_deref().and_then(App::config_file_stamp);
        let lint_spawn = lint_runtime::spawn(cfg, bg_tx.clone());
        let watch_tx = watcher::spawn_watcher(
            scan_root.to_path_buf(),
            bg_tx.clone(),
            cfg.tui.ci_run_count,
            cfg.tui.include_non_rust,
            cfg.lint.enabled,
            cfg.tui.include_dirs.clone(),
            http_client.clone(),
        );
        let tree_projects = App::filter_tree_projects(projects, cfg.tui.include_non_rust);
        let nodes = scan::build_tree(&tree_projects, &cfg.tui.inline_dirs);
        let flat_entries = scan::build_flat_entries(&nodes);
        let list_state = initial_list_state(&nodes);

        Self {
            config_path,
            config_last_seen,
            lint_warning: lint_spawn.warning,
            lint_runtime: lint_spawn.handle,
            watch_tx,
            nodes,
            flat_entries,
            list_state,
        }
    }
}

struct AppBuildInputs {
    scan_root:       PathBuf,
    projects:        Vec<RustProject>,
    bg_tx:           mpsc::Sender<BackgroundMsg>,
    bg_rx:           Receiver<BackgroundMsg>,
    cfg:             Config,
    http_client:     HttpClient,
    scan_started_at: Instant,
    channels:        AppChannels,
    builds:          AsyncBuildState,
    init:            AppInit,
}

impl App {
    const TAB_ORDER: [PaneId; 7] = [
        PaneId::ProjectList,
        PaneId::Package,
        PaneId::Git,
        PaneId::Targets,
        PaneId::CiRuns,
        PaneId::Toasts,
        PaneId::ScanLog,
    ];

    pub(super) const fn is_searching(&self) -> bool { self.ui_modes.search.is_active() }

    pub(super) const fn is_finder_open(&self) -> bool { self.ui_modes.finder.is_visible() }

    pub(super) const fn is_settings_open(&self) -> bool { self.ui_modes.settings.is_visible() }

    pub(super) const fn is_settings_editing(&self) -> bool { self.ui_modes.settings.is_editing() }

    pub(super) const fn is_scan_complete(&self) -> bool { self.scan.phase.is_complete() }

    pub(super) const fn should_quit(&self) -> bool { self.ui_modes.exit.should_quit() }

    pub(super) const fn should_restart(&self) -> bool { self.ui_modes.exit.should_restart() }

    pub(super) const fn selection_changed(&self) -> bool { self.selection.is_changed() }

    pub(super) const fn mark_selection_changed(&mut self) {
        self.selection = SelectionSync::Changed;
    }

    pub(super) const fn clear_selection_changed(&mut self) {
        self.selection = SelectionSync::Stable;
    }

    pub(super) const fn request_quit(&mut self) { self.ui_modes.exit = ExitMode::Quit; }

    pub(super) const fn request_restart(&mut self) { self.ui_modes.exit = ExitMode::Restart; }

    pub(super) const fn open_finder(&mut self) { self.ui_modes.finder = FinderMode::Visible; }

    pub(super) const fn close_finder(&mut self) { self.ui_modes.finder = FinderMode::Hidden; }

    pub(super) const fn end_search(&mut self) { self.ui_modes.search = SearchMode::Inactive; }

    pub(super) const fn open_settings(&mut self) {
        self.ui_modes.settings = SettingsMode::Browsing;
    }

    pub(super) const fn close_settings(&mut self) { self.ui_modes.settings = SettingsMode::Hidden; }

    pub(super) const fn begin_settings_editing(&mut self) {
        self.ui_modes.settings = SettingsMode::Editing;
    }

    pub(super) const fn end_settings_editing(&mut self) {
        self.ui_modes.settings = SettingsMode::Browsing;
    }

    pub(super) const fn mark_terminal_dirty(&mut self) { self.dirty.terminal.mark_dirty(); }

    pub(super) const fn clear_terminal_dirty(&mut self) { self.dirty.terminal.mark_clean(); }

    pub(super) const fn terminal_is_dirty(&self) -> bool { self.dirty.terminal.is_dirty() }

    /// Derive the current input context from app state.
    pub const fn input_context(&self) -> InputContext {
        if self.ui_modes.finder.is_visible() {
            InputContext::Finder
        } else if self.ui_modes.settings.is_visible() {
            InputContext::Settings
        } else if self.ui_modes.search.is_active() {
            InputContext::Searching
        } else {
            match self.focused_pane {
                PaneId::Package | PaneId::Git => InputContext::DetailFields,
                PaneId::Targets => InputContext::DetailTargets,
                PaneId::CiRuns => {
                    if matches!(self.bottom_panel, BottomPanel::PortReport) {
                        InputContext::PortReport
                    } else {
                        InputContext::CiRuns
                    }
                },
                PaneId::Toasts => InputContext::Toasts,
                PaneId::ScanLog => InputContext::ScanLog,
                PaneId::Search => InputContext::Searching,
                PaneId::Settings => InputContext::Settings,
                PaneId::Finder => InputContext::Finder,
                PaneId::ProjectList => InputContext::ProjectList,
            }
        }
    }

    pub fn is_focused(&self, pane: PaneId) -> bool { self.focused_pane == pane }

    pub fn base_focus(&self) -> PaneId {
        if self.focused_pane.is_overlay() {
            self.return_focus.unwrap_or(PaneId::ProjectList)
        } else {
            self.focused_pane
        }
    }

    pub fn focus_pane(&mut self, pane: PaneId) {
        self.focused_pane = pane;
        if !pane.is_overlay() {
            self.visited_panes.insert(pane);
            self.return_focus = None;
        }
    }

    pub fn open_overlay(&mut self, pane: PaneId) {
        if !pane.is_overlay() {
            self.focus_pane(pane);
            return;
        }
        self.return_focus = Some(self.base_focus());
        self.focused_pane = pane;
    }

    pub fn close_overlay(&mut self) {
        self.focused_pane = self.return_focus.unwrap_or(PaneId::ProjectList);
        self.return_focus = None;
    }

    pub fn tabbable_panes(&self) -> Vec<PaneId> {
        Self::TAB_ORDER
            .into_iter()
            .filter(|pane| match pane {
                PaneId::ProjectList => true,
                PaneId::Package => self.selected_project().is_some(),
                PaneId::Git => self.selected_project().is_some_and(|project| {
                    self.git_info
                        .get(&project.path)
                        .is_some_and(|info| info.url.is_some())
                }),
                PaneId::Targets => self.selected_project().is_some_and(|project| {
                    let info = super::detail::build_detail_info(self, project);
                    info.is_binary || !info.examples.is_empty() || !info.benches.is_empty()
                }),
                PaneId::CiRuns => self
                    .selected_project()
                    .is_some_and(|project| self.bottom_panel_available(project)),
                PaneId::Toasts => !self.active_toasts().is_empty(),
                PaneId::ScanLog | PaneId::Search | PaneId::Settings | PaneId::Finder => false,
            })
            .collect()
    }

    pub fn focus_next_pane(&mut self) {
        self.prune_toasts();
        let panes = self.tabbable_panes();
        if panes.is_empty() {
            return;
        }
        let current = self.base_focus();
        if current == PaneId::Toasts && self.toast_pane.pos() + 1 < self.active_toasts().len() {
            self.toast_pane.down();
            self.focus_pane(PaneId::Toasts);
            return;
        }
        let index = panes.iter().position(|pane| *pane == current).unwrap_or(0);
        let next = panes[(index + 1) % panes.len()];
        self.focus_pane(next);
        if next == PaneId::Toasts {
            self.toast_pane.home();
        }
    }

    pub fn focus_previous_pane(&mut self) {
        self.prune_toasts();
        let panes = self.tabbable_panes();
        if panes.is_empty() {
            return;
        }
        let current = self.base_focus();
        if current == PaneId::Toasts && self.toast_pane.pos() > 0 {
            self.toast_pane.up();
            self.focus_pane(PaneId::Toasts);
            return;
        }
        let index = panes.iter().position(|pane| *pane == current).unwrap_or(0);
        let prev = panes[(index + panes.len() - 1) % panes.len()];
        self.focus_pane(prev);
        if prev == PaneId::Toasts {
            self.toast_pane
                .set_pos(self.active_toasts().len().saturating_sub(1));
        }
    }

    pub fn reset_project_panes(&mut self) {
        self.package_pane.home();
        self.git_pane.home();
        self.targets_pane.home();
        self.ci_pane.home();
        self.port_report_pane.home();
        self.toast_pane.home();
        self.visited_panes.remove(&PaneId::Package);
        self.visited_panes.remove(&PaneId::Git);
        self.visited_panes.remove(&PaneId::Targets);
        self.visited_panes.remove(&PaneId::CiRuns);
    }

    pub fn remembers_selection(&self, pane: PaneId) -> bool { self.visited_panes.contains(&pane) }

    pub const fn toggle_bottom_panel(&mut self) {
        self.bottom_panel = match self.bottom_panel {
            BottomPanel::CiRuns => BottomPanel::PortReport,
            BottomPanel::PortReport => BottomPanel::CiRuns,
        };
    }

    pub const fn showing_port_report(&self) -> bool {
        matches!(self.bottom_panel, BottomPanel::PortReport)
    }

    pub const fn lint_enabled(&self) -> bool { self.current_config.lint.enabled }

    pub const fn invert_scroll(&self) -> ScrollDirection { self.current_config.mouse.invert_scroll }

    pub const fn include_non_rust(&self) -> NonRustInclusion {
        self.current_config.tui.include_non_rust
    }

    pub const fn ci_run_count(&self) -> u32 { self.current_config.tui.ci_run_count }

    pub const fn navigation_keys(&self) -> NavigationKeys {
        self.current_config.tui.navigation_keys
    }

    fn has_cached_non_rust_projects(&self) -> bool {
        self.all_projects
            .iter()
            .any(|project| project.is_rust != Rust)
    }

    fn filter_tree_projects(
        projects: &[RustProject],
        include_non_rust: NonRustInclusion,
    ) -> Vec<RustProject> {
        if include_non_rust.includes_non_rust() {
            projects.to_vec()
        } else {
            projects
                .iter()
                .filter(|project| project.is_rust == Rust)
                .cloned()
                .collect()
        }
    }

    fn tree_projects_snapshot(&self) -> Vec<RustProject> {
        Self::filter_tree_projects(&self.all_projects, self.include_non_rust())
    }

    pub(super) fn editor(&self) -> &str { &self.current_config.tui.editor }

    fn initial_status_flash(init: &AppInit) -> Option<(String, Instant)> {
        init.lint_warning
            .clone()
            .map(|warning| (warning, Instant::now()))
    }

    fn toast_timeout(&self) -> Duration {
        Duration::from_secs_f64(self.current_config.tui.status_flash_secs)
    }

    pub(super) fn active_toasts(&self) -> Vec<ToastView<'_>> { self.toasts.active(Instant::now()) }

    pub(super) fn focused_toast_id(&self) -> Option<u64> {
        let active = self.active_toasts();
        active.get(self.toast_pane.pos()).map(ToastView::id)
    }

    pub(super) fn prune_toasts(&mut self) {
        self.toasts.prune(Instant::now());
        self.toast_pane.set_len(self.active_toasts().len());
        if self.base_focus() == PaneId::Toasts && self.active_toasts().is_empty() {
            self.focus_pane(PaneId::ProjectList);
        }
    }

    pub(super) fn show_timed_toast(&mut self, title: impl Into<String>, body: impl Into<String>) {
        self.toasts.push_timed(title, body, self.toast_timeout());
        self.toast_pane.set_len(self.active_toasts().len());
    }

    pub(super) fn start_task_toast(
        &mut self,
        title: impl Into<String>,
        body: impl Into<String>,
    ) -> ToastTaskId {
        let task_id = self.toasts.push_task(title, body);
        self.toast_pane.set_len(self.active_toasts().len());
        task_id
    }

    pub(super) fn finish_task_toast(&mut self, task_id: ToastTaskId) {
        self.toasts.finish_task(task_id);
        self.prune_toasts();
    }

    fn update_task_toast_body(&mut self, task_id: ToastTaskId, body: impl Into<String>) {
        self.toasts.update_task_body(task_id, body);
        self.toast_pane.set_len(self.active_toasts().len());
    }

    pub(super) fn dismiss_focused_toast(&mut self) {
        if let Some(id) = self.focused_toast_id() {
            self.dismiss_toast(id);
        }
    }

    pub(super) fn dismiss_toast(&mut self, id: u64) {
        self.toasts.dismiss(id);
        self.prune_toasts();
    }

    pub fn port_report_is_watchable(&self, project: &RustProject) -> bool {
        if !self.lint_enabled() {
            return false;
        }
        crate::lint_runtime::project_is_eligible(
            &self.current_config.lint,
            &project.path,
            &PathBuf::from(&project.abs_path),
            project.is_rust == Rust,
        )
    }

    pub fn maybe_reload_config_from_disk(&mut self) { self.maybe_reload_config_from_disk_impl(); }

    pub fn save_and_apply_config(&mut self, cfg: &Config) -> Result<(), String> {
        self.save_and_apply_config_impl(cfg)
    }

    pub fn rescan(&mut self) { self.rescan_impl(); }

    pub fn poll_background(&mut self) -> PollBackgroundStats { self.poll_background_impl() }

    pub fn unreachable_service_message(&self) -> Option<String> {
        self.unreachable_service_message_impl()
    }

    pub fn ensure_visible_rows_cached(&mut self) { self.ensure_visible_rows_cached_impl(); }

    pub fn visible_rows(&self) -> &[VisibleRow] { self.visible_rows_impl() }

    pub fn ensure_fit_widths_cached(&mut self) { self.ensure_fit_widths_cached_impl(); }

    pub fn ensure_disk_cache(&mut self) { self.ensure_disk_cache_impl(); }

    pub fn ensure_detail_cached(&mut self) { self.ensure_detail_cached_impl(); }

    pub fn selected_node(&self) -> Option<&ProjectNode> { self.selected_node_impl() }

    pub fn selected_project(&self) -> Option<&RustProject> { self.selected_project_impl() }

    pub fn expand(&mut self) -> bool { self.expand_impl() }

    pub fn collapse(&mut self) -> bool { self.collapse_impl() }

    pub fn row_count(&self) -> usize { self.row_count_impl() }

    pub fn move_up(&mut self) { self.move_up_impl(); }

    pub fn move_down(&mut self) { self.move_down_impl(); }

    pub fn move_to_top(&mut self) { self.move_to_top_impl(); }

    pub fn move_to_bottom(&mut self) { self.move_to_bottom_impl(); }

    pub fn expand_all(&mut self) { self.expand_all_impl(); }

    pub fn collapse_all(&mut self) { self.collapse_all_impl(); }

    pub fn scan_log_scroll_up(&mut self) { self.scan_log_scroll_up_impl(); }

    pub fn scan_log_scroll_down(&mut self) { self.scan_log_scroll_down_impl(); }

    pub const fn scan_log_to_top(&mut self) { self.scan_log_to_top_impl(); }

    pub const fn scan_log_to_bottom(&mut self) { self.scan_log_to_bottom_impl(); }

    pub fn cancel_search(&mut self) { self.cancel_search_impl(); }

    pub fn confirm_search(&mut self) { self.confirm_search_impl(); }

    pub fn select_project_in_tree(&mut self, target_path: &str) {
        self.select_project_in_tree_impl(target_path);
    }

    pub fn update_search(&mut self, query: &str) { self.update_search_impl(query); }

    pub fn bottom_panel_available(&self, project: &RustProject) -> bool {
        let has_ci = self.is_ci_owner_path(&project.path)
            && (self
                .ci_state_for(project)
                .is_some_and(|state| !state.runs().is_empty())
                || self
                    .git_info
                    .get(&project.path)
                    .is_some_and(|info| info.url.is_some()));
        let has_port_report = self
            .port_report_runs
            .get(&project.path)
            .is_some_and(|runs| !runs.is_empty())
            || self.port_report_is_watchable(project);
        has_ci || has_port_report
    }

    pub fn sync_selected_project(&mut self) {
        self.ensure_visible_rows_cached();
        let current = self.selected_project().map(|project| project.path.clone());
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

        if let Some(path) = current
            && self.selection_paths.last_selected.as_ref() != Some(&path)
        {
            self.reload_port_report_history(&path);
            self.data_generation += 1;
            self.detail_generation += 1;
            self.selection_paths.last_selected = Some(path);
            self.mark_selection_changed();
            self.maybe_priority_fetch();
        }
    }

    pub(super) fn new(
        scan_root: PathBuf,
        projects: Vec<RustProject>,
        bg_tx: mpsc::Sender<BackgroundMsg>,
        bg_rx: Receiver<BackgroundMsg>,
        cfg: &Config,
        http_client: HttpClient,
        scan_started_at: Instant,
    ) -> Self {
        let channels = AppChannels::new();
        let builds = BuildChannels::new();
        let init = AppInit::new(&scan_root, &projects, &bg_tx, cfg, &http_client);
        let mut app = Self::build_app(AppBuildInputs {
            scan_root,
            projects,
            bg_tx,
            bg_rx,
            cfg: cfg.clone(),
            http_client,
            scan_started_at,
            channels,
            builds: AsyncBuildState::new(builds),
            init,
        });
        app.finish_new();
        app
    }

    fn build_app(inputs: AppBuildInputs) -> Self {
        let AppBuildInputs {
            scan_root,
            projects,
            bg_tx,
            bg_rx,
            cfg,
            http_client,
            scan_started_at,
            channels,
            builds,
            init,
        } = inputs;
        let status_flash = Self::initial_status_flash(&init);
        Self {
            current_config: cfg.clone(),
            scan_root,
            http_client,
            all_projects: projects,
            nodes: init.nodes,
            flat_entries: init.flat_entries,
            disk_usage: HashMap::new(),
            ci_state: HashMap::new(),
            lint_status: HashMap::new(),
            port_report_runs: HashMap::new(),
            lint_rollup_status: HashMap::new(),
            lint_rollup_paths: HashMap::new(),
            lint_rollup_keys_by_path: HashMap::new(),
            git_info: HashMap::new(),
            git_path_states: HashMap::new(),
            cargo_active_paths: HashSet::new(),
            crates_versions: HashMap::new(),
            crates_downloads: HashMap::new(),
            stars: HashMap::new(),
            repo_descriptions: HashMap::new(),
            bg_tx,
            bg_rx,
            fully_loaded: HashSet::new(),
            priority_fetch_path: None,
            expanded: HashSet::new(),
            list_state: init.list_state,
            search_query: String::new(),
            filtered: Vec::new(),
            settings_pane: Pane::new(),
            settings_edit_buf: String::new(),
            settings_edit_cursor: 0,
            scan_log: Vec::new(),
            scan_log_state: ListState::default(),
            focused_pane: PaneId::ProjectList,
            return_focus: None,
            visited_panes: std::iter::once(PaneId::ProjectList).collect(),
            package_pane: Pane::new(),
            git_pane: Pane::new(),
            targets_pane: Pane::new(),
            ci_pane: Pane::new(),
            toast_pane: Pane::new(),
            port_report_pane: Pane::new(),
            bottom_panel: BottomPanel::CiRuns,
            pending_example_run: None,
            pending_ci_fetch: None,
            pending_cleans: VecDeque::new(),
            confirm: None,
            animation_started: Instant::now(),
            ci_fetch_tx: channels.ci_fetch_tx,
            ci_fetch_rx: channels.ci_fetch_rx,
            clean_tx: channels.clean_tx,
            clean_rx: channels.clean_rx,
            example_running: None,
            example_child: Arc::new(Mutex::new(None)),
            example_output: Vec::new(),
            example_tx: channels.example_tx,
            example_rx: channels.example_rx,
            running_lint_paths: HashSet::new(),
            lint_toast: None,
            watch_tx: init.watch_tx,
            lint_runtime: init.lint_runtime,
            unreachable_services: HashSet::new(),
            service_retry_active: HashSet::new(),
            #[cfg(test)]
            retry_spawn_mode: RetrySpawnMode::Enabled,
            deleted_projects: HashSet::new(),
            selection_paths: SelectionPaths::new(),
            finder: FinderState::new(),
            cached_visible_rows: Vec::new(),
            cached_root_sorted: Vec::new(),
            cached_child_sorted: HashMap::new(),
            cached_fit_widths: ResolvedWidths::new(cfg.lint.enabled),
            builds,
            data_generation: 0,
            detail_generation: 0,
            cached_detail: None,
            layout_cache: LayoutCache::default(),
            status_flash,
            toasts: ToastManager::default(),
            config_path: init.config_path,
            config_last_seen: init.config_last_seen,
            ui_modes: UiModes::default(),
            dirty: DirtyState::initial(),
            scan: ScanState::new(scan_started_at),
            selection: SelectionSync::Stable,
        }
    }

    fn finish_new(&mut self) {
        if let Some(warning) = self
            .status_flash
            .as_ref()
            .map(|(warning, _)| warning.clone())
        {
            self.show_timed_toast("Lint runtime", warning.clone());
            self.scan_log.push(warning);
            self.scan_log_state.select(Some(0));
        }
        if self.current_config.tui.include_dirs.is_empty() {
            self.show_timed_toast(
                "Scan root",
                format!(
                    "Using {}. Set include_dirs in Settings to limit scan scope.",
                    crate::project::home_relative_path(&self.scan_root)
                ),
            );
        }
        self.recompute_cargo_active_paths();
        self.prune_inactive_project_state();
        self.register_existing_projects();
        self.sync_lint_runtime_projects();
        self.refresh_port_report_histories_from_disk();
        self.rebuild_lint_rollups();
    }

    pub fn workspace_counts(&self, project: &RustProject) -> Option<ProjectCounts> {
        // Check top-level nodes first
        if let Some(node) = self.nodes.iter().find(|n| n.project.path == project.path)
            && node.has_members()
        {
            let mut counts = ProjectCounts::default();
            counts.add_project(&node.project);
            for member in Self::all_group_members(node) {
                counts.add_project(member);
            }
            return Some(counts);
        }
        // Check worktree entries (workspace worktrees have their own groups)
        for node in &self.nodes {
            for wt in &node.worktrees {
                if wt.project.path == project.path && wt.has_members() {
                    let mut counts = ProjectCounts::default();
                    counts.add_project(&wt.project);
                    for group in &wt.groups {
                        for member in &group.members {
                            counts.add_project(member);
                        }
                    }
                    return Some(counts);
                }
            }
        }
        None
    }

    pub fn is_deleted(&self, path: &str) -> bool { self.deleted_projects.contains(path) }

    pub fn live_worktree_count(&self, node: &ProjectNode) -> usize {
        node.worktrees
            .iter()
            .filter(|wt| !self.is_deleted(&wt.project.path))
            .count()
    }

    fn rebuild_lint_rollups(&mut self) {
        self.lint_rollup_status.clear();
        self.lint_rollup_paths.clear();
        self.lint_rollup_keys_by_path.clear();

        let mut registrations: Vec<(LintRollupKey, Vec<String>)> = Vec::new();
        for (node_index, node) in self.nodes.iter().enumerate() {
            registrations.push((
                LintRollupKey::Root { node_index },
                Self::lint_root_paths_for_node(node),
            ));
            for (worktree_index, worktree) in node.worktrees.iter().enumerate() {
                registrations.push((
                    LintRollupKey::Worktree {
                        node_index,
                        worktree_index,
                    },
                    Self::lint_root_paths_for_worktree(worktree),
                ));
            }
        }

        for (key, paths) in registrations {
            self.register_lint_rollup(key, paths);
        }

        let keys: Vec<LintRollupKey> = self.lint_rollup_paths.keys().copied().collect();
        for key in keys {
            self.recompute_lint_rollup(key);
        }
    }

    fn register_lint_rollup(&mut self, key: LintRollupKey, mut paths: Vec<String>) {
        let mut seen = HashSet::new();
        paths.retain(|path| seen.insert(path.clone()));
        for path in &paths {
            self.lint_rollup_keys_by_path
                .entry(path.clone())
                .or_default()
                .push(key);
        }
        self.lint_rollup_paths.insert(key, paths);
    }

    fn update_lint_rollups_for_path(&mut self, path: &str) {
        let Some(keys) = self.lint_rollup_keys_by_path.get(path).cloned() else {
            return;
        };
        for key in keys {
            self.recompute_lint_rollup(key);
        }
    }

    fn recompute_lint_rollup(&mut self, key: LintRollupKey) {
        let Some(paths) = self.lint_rollup_paths.get(&key) else {
            self.lint_rollup_status.remove(&key);
            return;
        };
        let statuses: Vec<LintStatus> = paths
            .iter()
            .filter_map(|path| self.lint_status.get(path).cloned())
            .collect();
        let status = Self::aggregate_lint_rollup_statuses(&statuses);
        if matches!(status, LintStatus::NoLog) {
            self.lint_rollup_status.remove(&key);
        } else {
            self.lint_rollup_status.insert(key, status);
        }
    }

    fn aggregate_lint_rollup_statuses(statuses: &[LintStatus]) -> LintStatus {
        let running_statuses: Vec<LintStatus> = statuses
            .iter()
            .filter(|status| matches!(status, LintStatus::Running(_)))
            .cloned()
            .collect();
        if !running_statuses.is_empty() {
            return LintStatus::aggregate(running_statuses);
        }
        LintStatus::aggregate(statuses.iter().cloned())
    }

    fn lint_root_paths_for_node(node: &ProjectNode) -> Vec<String> {
        std::iter::once(node.project.path.clone())
            .chain(
                node.worktrees
                    .iter()
                    .map(|worktree| worktree.project.path.clone()),
            )
            .collect()
    }

    fn lint_root_paths_for_worktree(node: &ProjectNode) -> Vec<String> {
        vec![node.project.path.clone()]
    }

    fn selected_lint_rollup_key(&self) -> Option<LintRollupKey> {
        match self.selected_row()? {
            VisibleRow::Root { node_index } | VisibleRow::GroupHeader { node_index, .. } => {
                Some(LintRollupKey::Root { node_index })
            },
            VisibleRow::WorktreeEntry {
                node_index,
                worktree_index,
            }
            | VisibleRow::WorktreeGroupHeader {
                node_index,
                worktree_index,
                ..
            } => Some(LintRollupKey::Worktree {
                node_index,
                worktree_index,
            }),
            VisibleRow::Member { .. }
            | VisibleRow::Vendored { .. }
            | VisibleRow::WorktreeMember { .. }
            | VisibleRow::WorktreeVendored { .. } => None,
        }
    }

    fn lint_status_for_rollup_key(&self, key: LintRollupKey) -> Option<&LintStatus> {
        self.lint_rollup_status.get(&key)
    }

    pub fn formatted_disk(&self, project: &RustProject) -> String {
        match self.disk_usage.get(&project.path) {
            Some(&bytes) => super::render::format_bytes(bytes),
            None => super::render::format_bytes(0),
        }
    }

    pub(super) fn selected_ci_project(&self) -> Option<&RustProject> {
        self.selected_project()
            .filter(|project| self.is_ci_owner_path(&project.path))
    }

    pub(super) fn selected_ci_state(&self) -> Option<&CiState> {
        self.selected_ci_project()
            .and_then(|project| self.ci_state.get(&project.path))
    }

    pub fn ci_for(&self, project: &RustProject) -> Option<Conclusion> {
        self.ci_state_for(project)
            .and_then(|_| self.latest_ci_run_for_path(&project.path))
            .map(|run| run.conclusion)
    }

    /// Aggregate disk usage for a node: sums the root and all worktrees.
    pub fn formatted_disk_for_node(&self, node: &ProjectNode) -> String {
        if node.worktrees.is_empty() {
            return self.formatted_disk(&node.project);
        }
        let mut total: u64 = 0;
        let mut any_data = false;
        for path in unique_node_paths(node) {
            if let Some(&bytes) = self.disk_usage.get(path) {
                total += bytes;
                any_data = true;
            }
        }
        if any_data {
            super::render::format_bytes(total)
        } else {
            super::render::format_bytes(0)
        }
    }

    /// Get total disk bytes for a node (sum of root + worktrees).
    pub fn disk_bytes_for_node(&self, node: &ProjectNode) -> Option<u64> {
        if node.worktrees.is_empty() {
            return self.disk_usage.get(&node.project.path).copied();
        }
        let mut total: u64 = 0;
        let mut any_data = false;
        for path in unique_node_paths(node) {
            if let Some(&bytes) = self.disk_usage.get(path) {
                total += bytes;
                any_data = true;
            }
        }
        if any_data { Some(total) } else { None }
    }

    /// Aggregate CI for a node: `Success` if all green, `Failure` if any red, `None` if no data.
    pub fn ci_for_node(&self, node: &ProjectNode) -> Option<Conclusion> {
        if node.worktrees.is_empty() {
            return self.ci_for(&node.project);
        }
        let mut any_red = false;
        let mut all_green = true;
        let mut any_data = false;
        for path in std::iter::once(&node.project.path)
            .chain(node.worktrees.iter().map(|wt| &wt.project.path))
        {
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

    pub fn ci_state_for(&self, project: &RustProject) -> Option<&CiState> {
        self.is_ci_owner_path(&project.path)
            .then(|| self.ci_state.get(&project.path))
            .flatten()
    }

    pub fn animation_elapsed(&self) -> Duration { self.animation_started.elapsed() }

    /// Lint icon frame for the current animation state, or a blank space if lint is
    /// disabled or no log exists.
    pub fn lint_icon(&self, project: &RustProject) -> &'static str {
        use crate::constants::LINT_NO_LOG;

        if !self.lint_enabled() {
            return LINT_NO_LOG;
        }
        let Some(status) = self.lint_status.get(&project.path) else {
            return LINT_NO_LOG;
        };
        status.icon().frame_at(self.animation_elapsed())
    }

    pub fn lint_icon_for_root(&self, node_index: usize) -> &'static str {
        use crate::constants::LINT_NO_LOG;

        if !self.lint_enabled() {
            return LINT_NO_LOG;
        }
        let Some(status) = self.lint_status_for_rollup_key(LintRollupKey::Root { node_index })
        else {
            return LINT_NO_LOG;
        };
        status.icon().frame_at(self.animation_elapsed())
    }

    pub fn lint_icon_for_worktree(&self, node_index: usize, worktree_index: usize) -> &'static str {
        use crate::constants::LINT_NO_LOG;

        if !self.lint_enabled() {
            return LINT_NO_LOG;
        }
        let Some(status) = self.lint_status_for_rollup_key(LintRollupKey::Worktree {
            node_index,
            worktree_index,
        }) else {
            return LINT_NO_LOG;
        };
        status.icon().frame_at(self.animation_elapsed())
    }

    pub(super) fn selected_lint_icon(&self, project: &RustProject) -> Option<&'static str> {
        if !self.lint_enabled() {
            return None;
        }
        match self.selected_row() {
            Some(VisibleRow::Root { node_index } | VisibleRow::GroupHeader { node_index, .. }) => {
                self.lint_status_for_rollup_key(LintRollupKey::Root { node_index })
                    .map(|status| status.icon().frame_at(self.animation_elapsed()))
            },
            Some(
                VisibleRow::WorktreeEntry {
                    node_index,
                    worktree_index,
                }
                | VisibleRow::WorktreeGroupHeader {
                    node_index,
                    worktree_index,
                    ..
                },
            ) => self
                .lint_status_for_rollup_key(LintRollupKey::Worktree {
                    node_index,
                    worktree_index,
                })
                .map(|status| status.icon().frame_at(self.animation_elapsed())),
            Some(
                VisibleRow::Member { .. }
                | VisibleRow::Vendored { .. }
                | VisibleRow::WorktreeMember { .. }
                | VisibleRow::WorktreeVendored { .. },
            )
            | None => self
                .lint_status
                .get(&project.path)
                .map(|status| status.icon().frame_at(self.animation_elapsed())),
        }
    }

    pub(super) fn is_vendored_path(&self, path: &str) -> bool {
        self.nodes.iter().any(|node| {
            node.vendored.iter().any(|project| project.path == path)
                || node
                    .worktrees
                    .iter()
                    .any(|worktree| worktree.vendored.iter().any(|project| project.path == path))
        })
    }

    pub(super) fn is_workspace_member_path(&self, path: &str) -> bool {
        self.nodes.iter().any(|node| {
            node.project.is_workspace()
                && node
                    .groups
                    .iter()
                    .any(|group| group.members.iter().any(|member| member.path == path))
                || node.worktrees.iter().any(|worktree| {
                    worktree.project.is_workspace()
                        && worktree
                            .groups
                            .iter()
                            .any(|group| group.members.iter().any(|member| member.path == path))
                })
        })
    }

    fn project_by_path(&self, path: &str) -> Option<&RustProject> {
        self.all_projects
            .iter()
            .find(|project| project.path == path)
    }

    fn recompute_cargo_active_paths(&mut self) {
        let project_index: HashMap<String, Vec<String>> = self
            .all_projects
            .iter()
            .map(|project| (project.path.clone(), project.local_dependency_paths.clone()))
            .collect();
        let mut active_paths: HashSet<String> = self
            .all_projects
            .iter()
            .filter(|project| !self.is_vendored_path(&project.path))
            .map(|project| project.path.clone())
            .collect();
        let mut frontier: Vec<String> = active_paths.iter().cloned().collect();

        while let Some(path) = frontier.pop() {
            let Some(dependencies) = project_index.get(&path) else {
                continue;
            };
            for dependency_path in dependencies {
                if project_index.contains_key(dependency_path)
                    && active_paths.insert(dependency_path.clone())
                {
                    frontier.push(dependency_path.clone());
                }
            }
        }

        self.cargo_active_paths = active_paths;
    }

    pub(super) fn is_cargo_active_path(&self, path: &str) -> bool {
        self.cargo_active_paths.contains(path)
    }

    pub(super) fn git_path_state_for(&self, path: &str) -> GitPathState {
        self.git_path_states
            .get(path)
            .copied()
            .unwrap_or(GitPathState::OutsideRepo)
    }

    fn refresh_git_path_state(&mut self, path: &str) {
        let Some(project) = self.project_by_path(path) else {
            self.git_path_states.remove(path);
            return;
        };
        let state = crate::project::detect_git_path_state(Path::new(&project.abs_path));
        self.git_path_states.insert(path.to_string(), state);
    }

    fn prune_inactive_project_state(&mut self) {
        let all_paths: HashSet<String> = self
            .all_projects
            .iter()
            .map(|project| project.path.clone())
            .collect();
        self.git_path_states
            .retain(|path, _| all_paths.contains(path));
        for path in all_paths {
            if self.is_cargo_active_path(&path) {
                continue;
            }
            self.ci_state.remove(&path);
            self.crates_versions.remove(&path);
            self.crates_downloads.remove(&path);
            self.port_report_runs.remove(&path);
            self.lint_status.remove(&path);
        }
    }

    /// Formatted ahead/behind sync status for the project list columns.
    pub fn git_sync(&self, project: &RustProject) -> String {
        if matches!(
            self.git_path_state_for(&project.path),
            GitPathState::Untracked | GitPathState::Ignored
        ) {
            return String::new();
        }
        let Some(info) = self.git_info.get(&project.path) else {
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
            InputContext::ProjectList | InputContext::ScanLog => Some("open"),
            InputContext::DetailTargets => Some("run"),
            InputContext::DetailFields => {
                if self.base_focus() == PaneId::Package {
                    let info = self
                        .selected_project()
                        .map(|p| super::detail::build_detail_info(self, p))?;
                    let fields = super::detail::package_fields(&info);
                    let field = *fields.get(self.package_pane.pos())?;
                    if field.is_from_cargo_toml() {
                        Some("open")
                    } else {
                        None
                    }
                } else {
                    // Git column — Repo field opens URL
                    let info = self
                        .selected_project()
                        .map(|p| super::detail::build_detail_info(self, p))?;
                    let fields = super::detail::git_fields(&info);
                    match fields.get(self.git_pane.pos()) {
                        Some(DetailField::Repo) if info.git_url.is_some() => Some("open"),
                        _ => None,
                    }
                }
            },
            InputContext::CiRuns => {
                let ci_state = self.selected_project().and_then(|p| self.ci_state_for(p));
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

fn replace_project_in_node(
    node: &mut ProjectNode,
    project_path: &str,
    project: &RustProject,
) -> bool {
    let updated = if node.project.path == project_path {
        node.project = project.clone();
        true
    } else {
        false
    };

    let mut updated = updated;

    for group in &mut node.groups {
        for member in &mut group.members {
            if member.path == project_path {
                *member = project.clone();
                updated = true;
            }
        }
    }

    for vendored in &mut node.vendored {
        if vendored.path == project_path {
            *vendored = project.clone();
            updated = true;
        }
    }

    for worktree in &mut node.worktrees {
        if worktree.project.path == project_path {
            worktree.project = project.clone();
            updated = true;
        }

        for group in &mut worktree.groups {
            for member in &mut group.members {
                if member.path == project_path {
                    *member = project.clone();
                    updated = true;
                }
            }
        }

        for vendored in &mut worktree.vendored {
            if vendored.path == project_path {
                *vendored = project.clone();
                updated = true;
            }
        }
    }

    updated
}

fn initial_disk_batch_count(projects: &[RustProject]) -> usize {
    let mut abs_paths: Vec<&str> = projects
        .iter()
        .map(|project| project.abs_path.as_str())
        .collect();
    abs_paths.sort_by(|left, right| {
        Path::new(left)
            .components()
            .count()
            .cmp(&Path::new(right).components().count())
            .then_with(|| left.cmp(right))
    });

    let mut roots: Vec<&str> = Vec::new();
    for abs_path in abs_paths {
        if roots
            .iter()
            .any(|root| Path::new(abs_path).starts_with(Path::new(root)))
        {
            continue;
        }
        roots.push(abs_path);
    }

    roots.len()
}
