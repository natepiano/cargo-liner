use std::collections::HashMap;
use std::collections::HashSet;
use std::collections::VecDeque;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::mpsc;
use std::sync::mpsc::Receiver;
use std::thread;
use std::time::Duration;
use std::time::Instant;
use std::time::SystemTime;

use nucleo_matcher::Matcher;
use nucleo_matcher::Utf32Str;
use nucleo_matcher::pattern::Atom;
use nucleo_matcher::pattern::AtomKind;
use nucleo_matcher::pattern::CaseMatching;
use nucleo_matcher::pattern::Normalization;
use ratatui::widgets::ListState;

use super::config_reload;
use super::detail::CiFetchKind;
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
use super::render::PREFIX_ROOT_COLLAPSED;
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
use crate::ci;
use crate::ci::CiRun;
use crate::ci::Conclusion;
use crate::config::Config;
use crate::config::NavigationKeys;
use crate::config::NonRustInclusion;
use crate::config::ScrollDirection;
use crate::constants::IN_SYNC;
use crate::constants::SERVICE_RETRY_SECS;
use crate::constants::SYNC_DOWN;
use crate::constants::SYNC_UP;
use crate::constants::WORKTREE;
use crate::http::HttpClient;
use crate::http::ServiceKind;
use crate::http::ServiceSignal;
use crate::lint_runtime;
use crate::lint_runtime::RegisterProjectRequest;
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
use crate::scan::CiFetchResult;
use crate::scan::FlatEntry;
use crate::scan::MemberGroup;
use crate::scan::ProjectNode;
use crate::watcher;
use crate::watcher::WatchRequest;

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
    pub toast: ToastTaskId,
}

pub(super) use super::columns::ResolvedWidths;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ConfigFileStamp {
    modified: Option<SystemTime>,
    len: u64,
}

struct TreeBuildResult {
    build_id: u64,
    nodes: Vec<ProjectNode>,
    flat_entries: Vec<FlatEntry>,
}

struct FitWidthsBuildResult {
    build_id: u64,
    widths: ResolvedWidths,
}

struct DiskCacheBuildResult {
    build_id: u64,
    root_sorted: Vec<u64>,
    child_sorted: HashMap<usize, Vec<u64>>,
}

#[derive(Default)]
struct StartupPhaseTracker {
    scan_complete_at: Option<Instant>,
    disk_expected: Option<usize>,
    disk_seen: HashSet<String>,
    disk_complete_at: Option<Instant>,
    git_expected: HashSet<String>,
    git_seen: HashSet<String>,
    git_complete_at: Option<Instant>,
    repo_expected: HashSet<String>,
    repo_seen: HashSet<String>,
    repo_complete_at: Option<Instant>,
    git_toast: Option<ToastTaskId>,
    repo_toast: Option<ToastTaskId>,
    lint_expected: Option<HashSet<String>>,
    lint_seen_terminal: HashSet<String>,
    lint_complete_at: Option<Instant>,
    startup_complete_at: Option<Instant>,
}

#[derive(Default)]
pub(super) struct PollBackgroundStats {
    pub bg_msgs: usize,
    pub disk_usage_msgs: usize,
    pub git_path_state_msgs: usize,
    pub git_info_msgs: usize,
    pub lint_status_msgs: usize,
    pub ci_msgs: usize,
    pub example_msgs: usize,
    pub tree_results: usize,
    pub fit_results: usize,
    pub disk_results: usize,
    pub needs_rebuild: bool,
}

/// What a visible row represents.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum VisibleRow {
    /// A top-level project/workspace root.
    Root { node_index: usize },
    /// A group header (e.g., "examples").
    GroupHeader {
        node_index: usize,
        group_index: usize,
    },
    /// An actual project member.
    Member {
        node_index: usize,
        group_index: usize,
        member_index: usize,
    },
    /// A vendored crate nested directly under the root project.
    Vendored {
        node_index: usize,
        vendored_index: usize,
    },
    /// A worktree entry shown directly under the parent node.
    WorktreeEntry {
        node_index: usize,
        worktree_index: usize,
    },
    /// A group header inside an expanded worktree entry.
    WorktreeGroupHeader {
        node_index: usize,
        worktree_index: usize,
        group_index: usize,
    },
    /// A member inside an expanded worktree entry.
    WorktreeMember {
        node_index: usize,
        worktree_index: usize,
        group_index: usize,
        member_index: usize,
    },
    /// A vendored crate nested under a worktree entry.
    WorktreeVendored {
        node_index: usize,
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
        node_index: usize,
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
    Loaded { runs: Vec<CiRun>, exhausted: bool },
}

impl CiState {
    /// Access the runs regardless of which variant we are in.
    pub fn runs(&self) -> &[CiRun] {
        match self {
            Self::Fetching { runs, .. } | Self::Loaded { runs, .. } => runs,
        }
    }

    pub const fn is_fetching(&self) -> bool {
        matches!(self, Self::Fetching { .. })
    }

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
    selection: String,
    pub info: DetailInfo,
}

#[allow(
    clippy::struct_excessive_bools,
    reason = "independent UI state toggles"
)]
pub(super) struct App {
    pub(super) current_config: Config,
    pub scan_root: PathBuf,
    pub http_client: HttpClient,
    pub all_projects: Vec<RustProject>,
    pub nodes: Vec<ProjectNode>,
    pub flat_entries: Vec<FlatEntry>,
    pub disk_usage: HashMap<String, u64>,
    pub ci_state: HashMap<String, CiState>,
    pub lint_status: HashMap<String, LintStatus>,
    pub port_report_runs: HashMap<String, Vec<PortReportRun>>,
    lint_rollup_status: HashMap<LintRollupKey, LintStatus>,
    lint_rollup_paths: HashMap<LintRollupKey, Vec<String>>,
    lint_rollup_keys_by_path: HashMap<String, Vec<LintRollupKey>>,
    pub git_info: HashMap<String, GitInfo>,
    pub git_path_states: HashMap<String, GitPathState>,
    cargo_active_paths: HashSet<String>,
    pub crates_versions: HashMap<String, String>,
    pub crates_downloads: HashMap<String, u64>,
    pub stars: HashMap<String, u64>,
    pub repo_descriptions: HashMap<String, String>,
    pub bg_tx: mpsc::Sender<BackgroundMsg>,
    pub bg_rx: Receiver<BackgroundMsg>,
    pub fully_loaded: HashSet<String>,
    pub priority_fetch_path: Option<String>,
    pub expanded: HashSet<ExpandKey>,
    pub list_state: ListState,
    pub searching: bool,
    pub search_query: String,
    pub filtered: Vec<usize>,
    pub show_settings: bool,
    pub settings_pane: Pane,
    pub settings_editing: bool,
    pub settings_edit_buf: String,
    pub settings_edit_cursor: usize,
    pub scan_complete: bool,
    scan_started_at: Instant,
    scan_run_count: u64,
    startup_phases: StartupPhaseTracker,
    pub scan_log: Vec<String>,
    pub scan_log_state: ListState,
    pub focused_pane: PaneId,
    pub return_focus: Option<PaneId>,
    pub visited_panes: HashSet<PaneId>,
    pub package_pane: Pane,
    pub git_pane: Pane,
    pub targets_pane: Pane,
    pub ci_pane: Pane,
    pub toast_pane: Pane,
    pub port_report_pane: Pane,
    pub bottom_panel: BottomPanel,
    pub pending_example_run: Option<PendingExampleRun>,
    pub pending_ci_fetch: Option<PendingCiFetch>,
    pub pending_cleans: VecDeque<PendingClean>,
    pub confirm: Option<ConfirmAction>,
    pub animation_started: Instant,
    pub ci_fetch_tx: mpsc::Sender<CiFetchMsg>,
    pub ci_fetch_rx: mpsc::Receiver<CiFetchMsg>,
    pub clean_tx: mpsc::Sender<CleanMsg>,
    pub clean_rx: mpsc::Receiver<CleanMsg>,
    pub example_running: Option<String>,
    pub example_child: Arc<Mutex<Option<u32>>>,
    pub example_output: Vec<String>,
    pub example_tx: mpsc::Sender<ExampleMsg>,
    pub example_rx: mpsc::Receiver<ExampleMsg>,
    pub last_selected_path: Option<String>,
    pub selected_project_path: Option<String>,
    collapsed_selection_path: Option<String>,
    collapsed_anchor_path: Option<String>,
    running_lint_paths: HashSet<String>,
    lint_toast: Option<ToastTaskId>,
    pub terminal_dirty: bool,
    pub should_quit: bool,
    pub should_restart: bool,

    // Disk watcher
    pub watch_tx: mpsc::Sender<WatchRequest>,
    pub lint_runtime: Option<RuntimeHandle>,
    pub unreachable_services: HashSet<ServiceKind>,
    service_retry_active: HashSet<ServiceKind>,
    #[cfg(test)]
    pub service_retry_spawns_enabled: bool,

    // Projects whose directories have been deleted from disk.
    pub deleted_projects: HashSet<String>,

    // Universal finder
    pub show_finder: bool,
    pub finder_query: String,
    pub finder_results: Vec<usize>,
    pub finder_total: usize,
    pub finder_pane: Pane,
    pub finder_index: Vec<FinderItem>,
    pub finder_col_widths: [usize; FINDER_COLUMN_COUNT],
    pub finder_dirty: bool,

    // Caches for per-frame hot paths
    pub cached_visible_rows: Vec<VisibleRow>,
    pub rows_dirty: bool,
    pub cached_root_sorted: Vec<u64>,
    pub cached_child_sorted: HashMap<usize, Vec<u64>>,
    pub disk_cache_dirty: bool,
    pub cached_fit_widths: ResolvedWidths,
    fit_widths_dirty: bool,
    tree_build_tx: mpsc::Sender<TreeBuildResult>,
    tree_build_rx: Receiver<TreeBuildResult>,
    tree_build_active: Option<u64>,
    tree_build_latest: u64,
    fit_build_tx: mpsc::Sender<FitWidthsBuildResult>,
    fit_build_rx: Receiver<FitWidthsBuildResult>,
    fit_build_active: Option<u64>,
    fit_build_latest: u64,
    disk_build_tx: mpsc::Sender<DiskCacheBuildResult>,
    disk_build_rx: Receiver<DiskCacheBuildResult>,
    disk_build_active: Option<u64>,
    disk_build_latest: u64,
    pub(super) data_generation: u64,
    pub(super) detail_generation: u64,
    pub(super) cached_detail: Option<DetailCache>,
    pub(super) selection_changed: bool,
    pub(super) layout_cache: LayoutCache,

    pub(super) status_flash: Option<(String, std::time::Instant)>,
    pub(super) toasts: ToastManager,
    config_path: Option<PathBuf>,
    config_last_seen: Option<ConfigFileStamp>,
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
                            node_index: ni,
                            group_index: gi,
                            member_index: mi,
                        });
                    }
                } else {
                    rows.push(VisibleRow::GroupHeader {
                        node_index: ni,
                        group_index: gi,
                    });
                    if expanded.contains(&ExpandKey::Group(ni, gi)) {
                        for (mi, _) in group.members.iter().enumerate() {
                            rows.push(VisibleRow::Member {
                                node_index: ni,
                                group_index: gi,
                                member_index: mi,
                            });
                        }
                    }
                }
            }

            for (vi, _) in node.vendored.iter().enumerate() {
                rows.push(VisibleRow::Vendored {
                    node_index: ni,
                    vendored_index: vi,
                });
            }

            for (wi, wt) in node.worktrees.iter().enumerate() {
                rows.push(VisibleRow::WorktreeEntry {
                    node_index: ni,
                    worktree_index: wi,
                });
                if wt.has_children() && expanded.contains(&ExpandKey::Worktree(ni, wi)) {
                    for (gi, group) in wt.groups.iter().enumerate() {
                        if group.name.is_empty() {
                            for (mi, _) in group.members.iter().enumerate() {
                                rows.push(VisibleRow::WorktreeMember {
                                    node_index: ni,
                                    worktree_index: wi,
                                    group_index: gi,
                                    member_index: mi,
                                });
                            }
                        } else {
                            rows.push(VisibleRow::WorktreeGroupHeader {
                                node_index: ni,
                                worktree_index: wi,
                                group_index: gi,
                            });
                            if expanded.contains(&ExpandKey::WorktreeGroup(ni, wi, gi)) {
                                for (mi, _) in group.members.iter().enumerate() {
                                    rows.push(VisibleRow::WorktreeMember {
                                        node_index: ni,
                                        worktree_index: wi,
                                        group_index: gi,
                                        member_index: mi,
                                    });
                                }
                            }
                        }
                    }

                    for (vi, _) in wt.vendored.iter().enumerate() {
                        rows.push(VisibleRow::WorktreeVendored {
                            node_index: ni,
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

    /// Derive the current input context from app state.
    pub const fn input_context(&self) -> InputContext {
        if self.show_finder {
            InputContext::Finder
        } else if self.show_settings {
            InputContext::Settings
        } else if self.searching {
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

    pub fn is_focused(&self, pane: PaneId) -> bool {
        self.focused_pane == pane
    }

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

    pub fn remembers_selection(&self, pane: PaneId) -> bool {
        self.visited_panes.contains(&pane)
    }

    pub const fn toggle_bottom_panel(&mut self) {
        self.bottom_panel = match self.bottom_panel {
            BottomPanel::CiRuns => BottomPanel::PortReport,
            BottomPanel::PortReport => BottomPanel::CiRuns,
        };
    }

    pub const fn showing_port_report(&self) -> bool {
        matches!(self.bottom_panel, BottomPanel::PortReport)
    }

    pub const fn lint_enabled(&self) -> bool {
        self.current_config.lint.enabled
    }

    pub const fn invert_scroll(&self) -> ScrollDirection {
        self.current_config.mouse.invert_scroll
    }

    pub const fn include_non_rust(&self) -> NonRustInclusion {
        self.current_config.tui.include_non_rust
    }

    pub const fn ci_run_count(&self) -> u32 {
        self.current_config.tui.ci_run_count
    }

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

    pub(super) fn editor(&self) -> &str {
        &self.current_config.tui.editor
    }

    fn status_flash_millis(&self) -> u64 {
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "config value is always positive; sub-millisecond truncation is intentional"
        )]
        {
            (self.current_config.tui.status_flash_secs * 1000.0) as u64
        }
    }

    fn toast_timeout(&self) -> Duration {
        Duration::from_millis(self.status_flash_millis())
    }

    pub(super) fn active_toasts(&self) -> Vec<ToastView<'_>> {
        self.toasts.active(Instant::now())
    }

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

    pub fn bottom_panel_available(&self, project: &RustProject) -> bool {
        let has_ci = self
            .ci_state
            .get(&project.path)
            .is_some_and(|state| !state.runs().is_empty())
            || self
                .git_info
                .get(&project.path)
                .is_some_and(|info| info.url.is_some());
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
            .collapsed_anchor_path
            .as_ref()
            .is_some_and(|anchor| current.as_ref() != Some(anchor))
        {
            self.collapsed_selection_path = None;
            self.collapsed_anchor_path = None;
        }
        if self.selected_project_path == current {
            return;
        }

        self.selected_project_path.clone_from(&current);
        self.reset_project_panes();

        let panes = self.tabbable_panes();
        if !panes.contains(&self.base_focus()) {
            self.focus_pane(PaneId::ProjectList);
        }

        if self.return_focus.is_some() && !panes.contains(&self.return_focus.unwrap_or_default()) {
            self.return_focus = Some(PaneId::ProjectList);
        }

        if let Some(path) = current
            && self.last_selected_path.as_ref() != Some(&path)
        {
            self.reload_port_report_history(&path);
            self.data_generation += 1;
            self.detail_generation += 1;
            self.last_selected_path = Some(path);
            self.selection_changed = true;
            self.maybe_priority_fetch();
        }
    }

    #[allow(
        clippy::too_many_lines,
        reason = "struct constructor — all fields must be initialized"
    )]
    pub(super) fn new(
        scan_root: PathBuf,
        projects: Vec<RustProject>,
        bg_tx: mpsc::Sender<BackgroundMsg>,
        bg_rx: Receiver<BackgroundMsg>,
        cfg: &Config,
        http_client: HttpClient,
        scan_started_at: Instant,
    ) -> Self {
        let (example_tx, example_rx) = mpsc::channel();
        let (ci_fetch_tx, ci_fetch_rx) = mpsc::channel();
        let (clean_tx, clean_rx) = mpsc::channel();
        crate::config::set_active_config(cfg);
        let config_path = crate::config::config_path();
        let config_last_seen = config_path.as_deref().and_then(Self::config_file_stamp);
        let lint_spawn = lint_runtime::spawn(cfg, bg_tx.clone());
        let lint_warning = lint_spawn.warning.clone();
        let watch_tx = watcher::spawn_watcher(
            scan_root.clone(),
            bg_tx.clone(),
            cfg.tui.ci_run_count,
            cfg.tui.include_non_rust,
            cfg.lint.enabled,
            cfg.tui.include_dirs.clone(),
            http_client.clone(),
        );
        let tree_projects = Self::filter_tree_projects(&projects, cfg.tui.include_non_rust);
        let nodes = scan::build_tree(&tree_projects, &cfg.tui.inline_dirs);
        let flat_entries = scan::build_flat_entries(&nodes);
        let list_state = initial_list_state(&nodes);
        let (tree_build_tx, tree_build_rx) = mpsc::channel();
        let (fit_build_tx, fit_build_rx) = mpsc::channel();
        let (disk_build_tx, disk_build_rx) = mpsc::channel();
        let mut app = Self {
            current_config: cfg.clone(),
            scan_root,
            http_client,
            all_projects: projects,
            nodes,
            flat_entries,
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
            list_state,
            searching: false,
            search_query: String::new(),
            filtered: Vec::new(),
            show_settings: false,
            settings_pane: Pane::new(),
            settings_editing: false,
            settings_edit_buf: String::new(),
            settings_edit_cursor: 0,
            scan_complete: false,
            scan_started_at,
            scan_run_count: 1,
            startup_phases: StartupPhaseTracker::default(),
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
            ci_fetch_tx,
            ci_fetch_rx,
            clean_tx,
            clean_rx,
            example_running: None,
            example_child: Arc::new(Mutex::new(None)),
            example_output: Vec::new(),
            example_tx,
            example_rx,
            last_selected_path: super::terminal::load_last_selected(),
            selected_project_path: None,
            collapsed_selection_path: None,
            collapsed_anchor_path: None,
            running_lint_paths: HashSet::new(),
            lint_toast: None,
            terminal_dirty: false,
            should_quit: false,
            should_restart: false,

            watch_tx,
            lint_runtime: lint_spawn.handle,
            unreachable_services: HashSet::new(),
            service_retry_active: HashSet::new(),
            #[cfg(test)]
            service_retry_spawns_enabled: true,

            deleted_projects: HashSet::new(),

            show_finder: false,
            finder_query: String::new(),
            finder_results: Vec::new(),
            finder_total: 0,
            finder_pane: Pane::new(),
            finder_index: Vec::new(),
            finder_col_widths: [0; super::finder::FINDER_COLUMN_COUNT],
            finder_dirty: true,

            cached_visible_rows: Vec::new(),
            rows_dirty: true,
            cached_root_sorted: Vec::new(),
            cached_child_sorted: HashMap::new(),
            disk_cache_dirty: true,
            cached_fit_widths: ResolvedWidths::new(cfg.lint.enabled),
            fit_widths_dirty: true,
            tree_build_tx,
            tree_build_rx,
            tree_build_active: None,
            tree_build_latest: 0,
            fit_build_tx,
            fit_build_rx,
            fit_build_active: None,
            fit_build_latest: 0,
            disk_build_tx,
            disk_build_rx,
            disk_build_active: None,
            disk_build_latest: 0,
            data_generation: 0,
            detail_generation: 0,
            cached_detail: None,
            selection_changed: false,
            layout_cache: LayoutCache::default(),
            status_flash: lint_warning
                .clone()
                .map(|warning| (warning, Instant::now())),
            toasts: ToastManager::default(),
            config_path,
            config_last_seen,
        };
        if let Some(warning) = lint_warning {
            app.show_timed_toast("Lint runtime", warning.clone());
            app.scan_log.push(warning);
            app.scan_log_state.select(Some(0));
        }
        if app.current_config.tui.include_dirs.is_empty() {
            app.show_timed_toast(
                "Scan root",
                format!(
                    "Using {}. Set include_dirs in Settings to limit scan scope.",
                    crate::project::home_relative_path(&app.scan_root)
                ),
            );
        }
        app.recompute_cargo_active_paths();
        app.prune_inactive_project_state();
        app.register_existing_projects();
        app.sync_lint_runtime_projects();
        app.refresh_port_report_histories_from_disk();
        app.rebuild_lint_rollups();
        app
    }

    fn apply_tree_build(&mut self, nodes: Vec<ProjectNode>, flat_entries: Vec<FlatEntry>) {
        let selected_path = self
            .selected_project()
            .map(|p| p.path.clone())
            .or_else(|| self.last_selected_path.clone());
        let should_focus_project_list = self.focused_pane == PaneId::ScanLog && !nodes.is_empty();
        self.nodes = nodes;
        self.flat_entries = flat_entries;
        self.finder_dirty = true;
        self.rows_dirty = true;
        self.disk_cache_dirty = true;
        self.fit_widths_dirty = true;
        self.recompute_cargo_active_paths();
        self.prune_inactive_project_state();
        self.sync_lint_runtime_projects();
        self.rebuild_lint_rollups();
        self.data_generation += 1;
        self.detail_generation += 1;

        // Re-run search if active so filtered indices match new flat_entries
        if self.searching && !self.search_query.is_empty() {
            let query = self.search_query.clone();
            self.update_search(&query);
        } else {
            self.filtered.clear();
        }

        // Propagate CI state, git info, and stars from workspace roots to their members
        for node in &self.nodes {
            if let Some(runs) = self
                .ci_state
                .get(&node.project.path)
                .map(|s| s.runs().to_vec())
            {
                for member in Self::all_group_members(node) {
                    self.ci_state
                        .entry(member.path.clone())
                        .or_insert_with(|| CiState::Loaded {
                            runs: runs.clone(),
                            exhausted: false,
                        });
                }
            }
            if let Some(info) = self.git_info.get(&node.project.path).cloned() {
                for member in Self::all_group_members(node) {
                    self.git_info
                        .entry(member.path.clone())
                        .or_insert_with(|| info.clone());
                }
            }
            if let Some(&stars) = self.stars.get(&node.project.path) {
                for member in Self::all_group_members(node) {
                    self.stars.entry(member.path.clone()).or_insert(stars);
                }
            }
        }

        // Try to restore selection
        if let Some(path) = selected_path {
            self.select_project_in_tree(&path);
        } else if !self.nodes.is_empty() {
            self.list_state.select(Some(0));
        }
        if should_focus_project_list {
            self.focus_pane(PaneId::ProjectList);
        }
        self.sync_selected_project();
    }

    pub fn rebuild_tree(&mut self) {
        self.request_tree_rebuild();
    }

    fn config_file_stamp(path: &Path) -> Option<ConfigFileStamp> {
        let metadata = std::fs::metadata(path).ok()?;
        Some(ConfigFileStamp {
            modified: metadata.modified().ok(),
            len: metadata.len(),
        })
    }

    fn sync_config_watch_state(&mut self) {
        self.config_last_seen = self
            .config_path
            .as_deref()
            .and_then(Self::config_file_stamp);
    }

    fn record_config_reload_failure(&mut self, err: &str) {
        self.status_flash = Some((
            "Config reload failed; keeping previous settings".to_string(),
            Instant::now(),
        ));
        self.show_timed_toast(
            "Config reload failed",
            "Keeping previous settings".to_string(),
        );
        self.scan_log.push(format!("config reload failed: {err}"));
        self.scan_log_state
            .select(Some(self.scan_log.len().saturating_sub(1)));
    }

    pub(super) fn maybe_reload_config_from_disk(&mut self) {
        let current_stamp = self
            .config_path
            .as_deref()
            .and_then(Self::config_file_stamp);
        if current_stamp == self.config_last_seen {
            return;
        }

        self.config_last_seen = current_stamp;
        let reload_result = self
            .config_path
            .as_deref()
            .map_or_else(crate::config::try_load, crate::config::try_load_from_path);
        match reload_result {
            Ok(cfg) => {
                self.apply_config(&cfg);
                self.sync_config_watch_state();
            },
            Err(err) => self.record_config_reload_failure(&err),
        }
    }

    pub(super) fn save_and_apply_config(&mut self, cfg: &Config) -> Result<(), String> {
        crate::config::save(cfg)?;
        self.apply_config(cfg);
        self.sync_config_watch_state();
        Ok(())
    }

    pub(super) fn apply_config(&mut self, cfg: &Config) {
        if self.current_config == *cfg {
            return;
        }

        let actions = config_reload::collect_reload_actions(
            &self.current_config,
            cfg,
            config_reload::ReloadContext {
                scan_complete: self.scan_complete,
                has_cached_non_rust: self.has_cached_non_rust_projects(),
            },
        );
        crate::config::set_active_config(cfg);
        self.current_config = cfg.clone();

        if actions.refresh_lint_runtime {
            self.refresh_lint_runtime_from_config(cfg);
        }

        if actions.rescan {
            self.rescan();
        } else {
            if actions.refresh_lint_runtime {
                self.respawn_watcher();
            }
            if actions.rebuild_tree {
                self.rebuild_tree();
            }
        }
    }

    fn refresh_lint_runtime_from_config(&mut self, cfg: &Config) {
        let lint_spawn = lint_runtime::spawn(cfg, self.bg_tx.clone());
        self.lint_runtime = lint_spawn.handle;
        self.register_existing_projects();
        self.sync_lint_runtime_projects_immediately();
        self.refresh_lint_statuses_from_disk();
        self.refresh_port_report_histories_from_disk();
        self.rebuild_lint_rollups();
        self.cached_fit_widths = ResolvedWidths::new(self.lint_enabled());
        self.rows_dirty = true;
        self.fit_widths_dirty = true;
        self.data_generation += 1;
        self.detail_generation += 1;
        if let Some(warning) = lint_spawn.warning {
            self.status_flash = Some((warning.clone(), Instant::now()));
            self.show_timed_toast("Lint runtime", warning.clone());
            self.scan_log.push(warning);
            self.scan_log_state
                .select(Some(self.scan_log.len().saturating_sub(1)));
        }
    }

    fn respawn_watcher(&mut self) {
        self.watch_tx = watcher::spawn_watcher(
            self.scan_root.clone(),
            self.bg_tx.clone(),
            self.ci_run_count(),
            self.include_non_rust(),
            self.lint_enabled(),
            self.current_config.tui.include_dirs.clone(),
            self.http_client.clone(),
        );
    }

    fn register_existing_projects(&self) {
        for project in &self.all_projects {
            self.register_project_background_services(project);
        }
    }

    fn refresh_lint_statuses_from_disk(&mut self) {
        self.lint_status.clear();
        if !self.lint_enabled() {
            return;
        }
        for project in &self.all_projects {
            if !self.is_cargo_active_path(&project.path) {
                continue;
            }
            if !crate::lint_runtime::project_is_eligible(
                &self.current_config.lint,
                &project.path,
                &PathBuf::from(&project.abs_path),
                project.is_rust == Rust,
            ) {
                continue;
            }
            let status = crate::port_report::read_status(&PathBuf::from(&project.abs_path));
            if !matches!(status, LintStatus::NoLog) {
                self.lint_status.insert(project.path.clone(), status);
            }
        }
    }

    fn refresh_port_report_histories_from_disk(&mut self) {
        self.port_report_runs.clear();
        for project in &self.all_projects {
            if !self.is_cargo_active_path(&project.path) {
                continue;
            }
            let runs = crate::port_report::read_history(&PathBuf::from(&project.abs_path));
            if !runs.is_empty() {
                self.port_report_runs.insert(project.path.clone(), runs);
            }
        }
    }

    fn reload_port_report_history(&mut self, project_path: &str) {
        let Some(project) = self
            .all_projects
            .iter()
            .find(|project| project.path == project_path)
        else {
            self.port_report_runs.remove(project_path);
            return;
        };
        if !self.is_cargo_active_path(project_path) {
            self.port_report_runs.remove(project_path);
            return;
        }
        let runs = crate::port_report::read_history(&PathBuf::from(&project.abs_path));
        if runs.is_empty() {
            self.port_report_runs.remove(project_path);
        } else {
            self.port_report_runs.insert(project_path.to_string(), runs);
        }
    }

    fn register_project_background_services(&self, project: &RustProject) {
        let started = Instant::now();
        let abs_path = PathBuf::from(&project.abs_path);
        let repo_root = crate::project::git_repo_root(&abs_path);
        let has_repo_root = repo_root.is_some();
        let _ = self.watch_tx.send(WatchRequest {
            project_path: project.path.clone(),
            abs_path,
            repo_root,
        });
        crate::perf_log::log_duration(
            "app_register_project_background_services",
            started.elapsed(),
            &format!("path={} has_repo_root={has_repo_root}", project.path),
            0,
        );
    }

    fn schedule_git_path_state_refreshes(&self) {
        let tx = self.bg_tx.clone();
        let projects: Vec<(String, String)> = self
            .all_projects
            .iter()
            .map(|project| (project.path.clone(), project.abs_path.clone()))
            .collect();
        std::thread::spawn(move || {
            let states = crate::project::detect_git_path_states_batch(&projects);
            for (path, state) in states {
                let _ = tx.send(BackgroundMsg::GitPathState { path, state });
            }
        });
    }

    fn schedule_git_first_commit_refreshes(&self) {
        let tx = self.bg_tx.clone();
        let mut projects_by_repo: HashMap<PathBuf, Vec<String>> = HashMap::new();
        for project in &self.all_projects {
            let abs_path = PathBuf::from(&project.abs_path);
            let Some(repo_root) = crate::project::git_repo_root(&abs_path) else {
                continue;
            };
            projects_by_repo
                .entry(repo_root)
                .or_default()
                .push(project.path.clone());
        }
        std::thread::spawn(move || {
            for (repo_root, paths) in projects_by_repo {
                let started = Instant::now();
                let first_commit = crate::project::detect_first_commit(&repo_root);
                crate::perf_log::log_duration(
                    "git_first_commit_fetch",
                    started.elapsed(),
                    &format!(
                        "repo_root={} rows={} found={}",
                        repo_root.display(),
                        paths.len(),
                        first_commit.is_some()
                    ),
                    0,
                );
                for path in paths {
                    let _ = tx.send(BackgroundMsg::GitFirstCommit {
                        path,
                        first_commit: first_commit.clone(),
                    });
                }
            }
        });
    }

    fn sync_lint_runtime_projects(&self) {
        self.sync_lint_runtime_projects_with(false);
    }

    fn sync_lint_runtime_projects_immediately(&self) {
        self.sync_lint_runtime_projects_with(true);
    }

    fn lint_runtime_root_projects(&self) -> Vec<&RustProject> {
        let mut projects = Vec::new();
        let mut seen = HashSet::new();

        for node in &self.nodes {
            if seen.insert(node.project.path.clone()) {
                projects.push(&node.project);
            }
            for worktree in &node.worktrees {
                if seen.insert(worktree.project.path.clone()) {
                    projects.push(&worktree.project);
                }
            }
        }

        if !projects.is_empty() {
            return projects;
        }

        self.all_projects
            .iter()
            .filter(|project| seen.insert(project.path.clone()))
            .collect()
    }

    fn lint_runtime_projects_snapshot(&self) -> Vec<RegisterProjectRequest> {
        if !self.scan_complete {
            return Vec::new();
        }
        self.lint_runtime_root_projects()
            .into_iter()
            .filter(|project| !self.deleted_projects.contains(&project.path))
            .filter(|project| self.is_cargo_active_path(&project.path))
            .map(|project| RegisterProjectRequest {
                project_path: project.path.clone(),
                abs_path: PathBuf::from(&project.abs_path),
                is_rust: project.is_rust == Rust,
            })
            .collect()
    }

    fn sync_lint_runtime_projects_with(&self, force_immediate_run: bool) {
        let Some(runtime) = &self.lint_runtime else {
            return;
        };
        let projects = self.lint_runtime_projects_snapshot();
        if force_immediate_run {
            runtime.sync_projects_immediately(projects);
        } else {
            runtime.sync_projects(projects);
        }
    }

    fn initialize_startup_phase_tracker(&mut self) {
        let disk_expected = initial_disk_batch_count(&self.all_projects);
        let git_seen = self
            .startup_phases
            .git_expected
            .iter()
            .filter(|path| self.git_info.contains_key(*path))
            .cloned()
            .collect::<HashSet<_>>();
        self.startup_phases.disk_complete_at = None;
        self.startup_phases.scan_complete_at = Some(Instant::now());
        self.startup_phases.disk_expected = Some(disk_expected);
        self.startup_phases.git_seen = git_seen;
        self.startup_phases.git_complete_at = None;
        self.startup_phases.repo_complete_at = None;
        self.startup_phases.git_toast = None;
        self.startup_phases.repo_toast = None;
        self.startup_phases.lint_expected = Some(HashSet::new());
        self.startup_phases.lint_seen_terminal.clear();
        self.startup_phases.lint_complete_at = None;
        self.startup_phases.startup_complete_at = None;
        let git_remaining = self
            .startup_phases
            .git_expected
            .len()
            .saturating_sub(self.startup_phases.git_seen.len());
        if git_remaining > 0 {
            self.startup_phases.git_toast =
                Some(self.start_task_toast(
                    "Scanning local git repos",
                    self.startup_git_toast_body(),
                ));
        }
        let repo_remaining = self
            .startup_phases
            .repo_expected
            .len()
            .saturating_sub(self.startup_phases.repo_seen.len());
        if repo_remaining > 0 {
            self.startup_phases.repo_toast =
                Some(self.start_task_toast(
                    "Retrieving GitHub repo details",
                    self.startup_repo_toast_body(),
                ));
        }
        crate::perf_log::log_event(&format!(
            "startup_phase_plan disk_expected={} git_expected={} repo_expected={} lint_expected={}",
            self.startup_phases.disk_expected.unwrap_or(0),
            self.startup_phases.git_expected.len(),
            self.startup_phases.repo_expected.len(),
            self.startup_phases
                .lint_expected
                .as_ref()
                .map_or(0, HashSet::len)
        ));
        self.maybe_log_startup_phase_completions();
    }

    fn maybe_log_startup_phase_completions(&mut self) {
        let Some(scan_complete_at) = self.startup_phases.scan_complete_at else {
            return;
        };
        let now = Instant::now();
        self.maybe_complete_startup_disk(now, scan_complete_at);
        self.maybe_complete_startup_git(now, scan_complete_at);
        self.maybe_complete_startup_repo(now, scan_complete_at);
        self.maybe_complete_startup_lints(now, scan_complete_at);
        self.maybe_complete_startup_ready(now, scan_complete_at);
    }

    fn maybe_complete_startup_disk(&mut self, now: Instant, scan_complete_at: Instant) {
        if self.startup_phases.disk_complete_at.is_none()
            && self
                .startup_phases
                .disk_expected
                .is_some_and(|expected| self.startup_phases.disk_seen.len() >= expected)
        {
            self.startup_phases.disk_complete_at = Some(now);
            crate::perf_log::log_event(&format!(
                "startup_phase_complete phase=disk_applied since_scan_complete_ms={} seen={} expected={}",
                now.duration_since(scan_complete_at).as_millis(),
                self.startup_phases.disk_seen.len(),
                self.startup_phases.disk_expected.unwrap_or(0)
            ));
        }
    }

    fn maybe_complete_startup_git(&mut self, now: Instant, scan_complete_at: Instant) {
        if self.startup_phases.git_complete_at.is_none()
            && self.startup_phases.git_seen.len() >= self.startup_phases.git_expected.len()
        {
            self.startup_phases.git_complete_at = Some(now);
            if let Some(git_toast) = self.startup_phases.git_toast.take() {
                self.finish_task_toast(git_toast);
            }
            crate::perf_log::log_event(&format!(
                "startup_phase_complete phase=git_local_applied since_scan_complete_ms={} seen={} expected={}",
                now.duration_since(scan_complete_at).as_millis(),
                self.startup_phases.git_seen.len(),
                self.startup_phases.git_expected.len()
            ));
        } else if let Some(git_toast) = self.startup_phases.git_toast {
            self.update_task_toast_body(git_toast, self.startup_git_toast_body());
        }
    }

    fn maybe_complete_startup_repo(&mut self, now: Instant, scan_complete_at: Instant) {
        if self.startup_phases.repo_complete_at.is_none()
            && self.startup_phases.repo_seen.len() >= self.startup_phases.repo_expected.len()
        {
            self.startup_phases.repo_complete_at = Some(now);
            if let Some(repo_toast) = self.startup_phases.repo_toast.take() {
                self.finish_task_toast(repo_toast);
            }
            crate::perf_log::log_event(&format!(
                "startup_phase_complete phase=repo_fetch_applied since_scan_complete_ms={} seen={} expected={}",
                now.duration_since(scan_complete_at).as_millis(),
                self.startup_phases.repo_seen.len(),
                self.startup_phases.repo_expected.len()
            ));
        } else if let Some(repo_toast) = self.startup_phases.repo_toast {
            self.update_task_toast_body(repo_toast, self.startup_repo_toast_body());
        }
    }

    fn maybe_complete_startup_lints(&mut self, now: Instant, scan_complete_at: Instant) {
        if self.startup_phases.lint_complete_at.is_none()
            && self
                .startup_phases
                .lint_expected
                .as_ref()
                .is_some_and(|expected| {
                    !expected.is_empty()
                        && self.startup_phases.lint_seen_terminal.len() >= expected.len()
                })
        {
            self.startup_phases.lint_complete_at = Some(now);
            crate::perf_log::log_event(&format!(
                "startup_phase_complete phase=lint_terminal_applied since_scan_complete_ms={} seen={} expected={}",
                now.duration_since(scan_complete_at).as_millis(),
                self.startup_phases.lint_seen_terminal.len(),
                self.startup_phases
                    .lint_expected
                    .as_ref()
                    .map_or(0, HashSet::len)
            ));
        }
    }

    fn maybe_complete_startup_ready(&mut self, now: Instant, scan_complete_at: Instant) {
        if self.startup_phases.startup_complete_at.is_none() {
            let disk_ready = self.startup_phases.disk_complete_at.is_some();
            let git_ready = self.startup_phases.git_complete_at.is_some();
            let repo_ready = self.startup_phases.repo_complete_at.is_some();
            if disk_ready && git_ready && repo_ready {
                self.startup_phases.startup_complete_at = Some(now);
                self.show_timed_toast(
                    "Startup complete",
                    "Disk, local Git, and GitHub startup activity complete.".to_string(),
                );
                crate::perf_log::log_event(&format!(
                    "startup_complete since_scan_complete_ms={} disk_seen={} disk_expected={} git_seen={} git_expected={} repo_seen={} repo_expected={} lint_seen={} lint_expected={}",
                    now.duration_since(scan_complete_at).as_millis(),
                    self.startup_phases.disk_seen.len(),
                    self.startup_phases.disk_expected.unwrap_or(0),
                    self.startup_phases.git_seen.len(),
                    self.startup_phases.git_expected.len(),
                    self.startup_phases.repo_seen.len(),
                    self.startup_phases.repo_expected.len(),
                    self.startup_phases.lint_seen_terminal.len(),
                    self.startup_phases
                        .lint_expected
                        .as_ref()
                        .map_or(0, HashSet::len)
                ));
                crate::perf_log::log_event(&format!(
                    "steady_state_begin since_scan_complete_ms={}",
                    now.duration_since(scan_complete_at).as_millis()
                ));
            }
        }
    }

    fn startup_git_toast_body(&self) -> String {
        Self::startup_remaining_toast_body(
            &self.startup_phases.git_expected,
            &self.startup_phases.git_seen,
        )
    }

    fn startup_repo_toast_body(&self) -> String {
        Self::startup_remaining_toast_body(
            &self.startup_phases.repo_expected,
            &self.startup_phases.repo_seen,
        )
    }

    fn startup_remaining_toast_body(expected: &HashSet<String>, seen: &HashSet<String>) -> String {
        let Some(current) = expected.iter().find(|path| !seen.contains(*path)) else {
            return "Complete".to_string();
        };
        let remaining = expected.len().saturating_sub(seen.len());
        if remaining <= 1 {
            current.clone()
        } else {
            format!("{current}\n+ {} others", remaining - 1)
        }
    }

    fn startup_lint_toast_body_for(expected: &HashSet<String>, seen: &HashSet<String>) -> String {
        let mut remaining = expected.iter().filter(|path| !seen.contains(*path));
        let Some(first) = remaining.next() else {
            return "Complete".to_string();
        };
        let Some(second) = remaining.next() else {
            return first.clone();
        };
        let other_count = remaining.count();
        if other_count == 0 {
            format!("{first}\n{second}")
        } else {
            format!("{first}\n{second} (+ {other_count} others)")
        }
    }

    fn running_lint_toast_body(&self) -> String {
        Self::startup_lint_toast_body_for(&self.running_lint_paths, &HashSet::new())
    }

    fn sync_running_lint_toast(&mut self) {
        if self.running_lint_paths.is_empty() {
            if let Some(task_id) = self.lint_toast.take() {
                self.finish_task_toast(task_id);
            }
            return;
        }

        let body = self.running_lint_toast_body();
        if let Some(task_id) = self.lint_toast {
            self.update_task_toast_body(task_id, body);
        } else {
            self.lint_toast = Some(self.start_task_toast("Lints", body));
        }
    }

    fn request_tree_rebuild(&mut self) {
        self.tree_build_latest = self.tree_build_latest.wrapping_add(1);
        if self.tree_build_active.is_some() {
            return;
        }
        self.spawn_tree_build(self.tree_build_latest);
    }

    fn spawn_tree_build(&mut self, build_id: u64) {
        let tx = self.tree_build_tx.clone();
        let projects = self.tree_projects_snapshot();
        let inline_dirs = self.current_config.tui.inline_dirs.clone();
        self.tree_build_active = Some(build_id);
        std::thread::spawn(move || {
            let started = Instant::now();
            let nodes = scan::build_tree(&projects, &inline_dirs);
            let flat_entries = scan::build_flat_entries(&nodes);
            crate::perf_log::log_duration(
                "tree_build",
                started.elapsed(),
                &format!(
                    "build_id={} projects={} nodes={} flat_entries={}",
                    build_id,
                    projects.len(),
                    nodes.len(),
                    flat_entries.len()
                ),
                crate::perf_log::slow_worker_threshold_ms(),
            );
            let _ = tx.send(TreeBuildResult {
                build_id,
                nodes,
                flat_entries,
            });
        });
    }

    fn poll_tree_builds(&mut self) -> usize {
        let mut applied = 0;
        while let Ok(result) = self.tree_build_rx.try_recv() {
            if self.tree_build_active != Some(result.build_id) {
                continue;
            }
            self.tree_build_active = None;
            self.apply_tree_build(result.nodes, result.flat_entries);
            applied += 1;
            if result.build_id != self.tree_build_latest {
                self.spawn_tree_build(self.tree_build_latest);
            }
        }
        applied
    }

    fn request_fit_widths_build(&mut self) {
        if !self.fit_widths_dirty {
            return;
        }
        self.fit_build_latest = self.fit_build_latest.wrapping_add(1);
        if self.fit_build_active.is_some() {
            return;
        }
        self.spawn_fit_widths_build(self.fit_build_latest);
    }

    fn spawn_fit_widths_build(&mut self, build_id: u64) {
        let tx = self.fit_build_tx.clone();
        let nodes = self.nodes.clone();
        let disk_usage = self.disk_usage.clone();
        let git_info = self.git_info.clone();
        let git_path_states = self.git_path_states.clone();
        let deleted_projects = self.deleted_projects.clone();
        let lint_enabled = self.lint_enabled();
        self.fit_build_active = Some(build_id);
        std::thread::spawn(move || {
            let started = Instant::now();
            let widths = build_fit_widths_snapshot(
                &nodes,
                &disk_usage,
                &git_info,
                &git_path_states,
                &deleted_projects,
                lint_enabled,
                build_id,
            );
            crate::perf_log::log_duration(
                "fit_widths_build",
                started.elapsed(),
                &format!("build_id={} nodes={}", build_id, nodes.len()),
                crate::perf_log::slow_worker_threshold_ms(),
            );
            let _ = tx.send(FitWidthsBuildResult { build_id, widths });
        });
    }

    fn poll_fit_width_builds(&mut self) -> usize {
        let mut applied = 0;
        while let Ok(result) = self.fit_build_rx.try_recv() {
            if self.fit_build_active != Some(result.build_id) {
                continue;
            }
            self.fit_build_active = None;
            self.cached_fit_widths = result.widths;
            applied += 1;
            if result.build_id == self.fit_build_latest {
                self.fit_widths_dirty = false;
            } else {
                self.spawn_fit_widths_build(self.fit_build_latest);
            }
        }
        applied
    }

    fn request_disk_cache_build(&mut self) {
        if !self.disk_cache_dirty {
            return;
        }
        self.disk_build_latest = self.disk_build_latest.wrapping_add(1);
        if self.disk_build_active.is_some() {
            return;
        }
        self.spawn_disk_cache_build(self.disk_build_latest);
    }

    fn spawn_disk_cache_build(&mut self, build_id: u64) {
        let tx = self.disk_build_tx.clone();
        let nodes = self.nodes.clone();
        let disk_usage = self.disk_usage.clone();
        self.disk_build_active = Some(build_id);
        std::thread::spawn(move || {
            let started = Instant::now();
            let (root_sorted, child_sorted) = build_disk_cache_snapshot(&nodes, &disk_usage);
            crate::perf_log::log_duration(
                "disk_cache_build",
                started.elapsed(),
                &format!(
                    "build_id={} nodes={} root_values={} child_sets={}",
                    build_id,
                    nodes.len(),
                    root_sorted.len(),
                    child_sorted.len()
                ),
                crate::perf_log::slow_worker_threshold_ms(),
            );
            let _ = tx.send(DiskCacheBuildResult {
                build_id,
                root_sorted,
                child_sorted,
            });
        });
    }

    fn poll_disk_cache_builds(&mut self) -> usize {
        let mut applied = 0;
        while let Ok(result) = self.disk_build_rx.try_recv() {
            if self.disk_build_active != Some(result.build_id) {
                continue;
            }
            self.disk_build_active = None;
            self.cached_root_sorted = result.root_sorted;
            self.cached_child_sorted = result.child_sorted;
            applied += 1;
            if result.build_id == self.disk_build_latest {
                self.disk_cache_dirty = false;
            } else {
                self.spawn_disk_cache_build(self.disk_build_latest);
            }
        }
        applied
    }

    pub fn refresh_async_caches(&mut self) {
        self.request_disk_cache_build();
        self.request_fit_widths_build();
    }

    pub(super) fn rescan(&mut self) {
        self.all_projects.clear();
        self.nodes.clear();
        self.flat_entries.clear();
        self.disk_usage.clear();
        self.ci_state.clear();
        self.lint_status.clear();
        self.port_report_runs.clear();
        self.git_info.clear();
        self.git_path_states.clear();
        self.cargo_active_paths.clear();
        self.crates_versions.clear();
        self.crates_downloads.clear();
        self.stars.clear();
        self.repo_descriptions.clear();
        self.scan_log.clear();
        self.scan_log_state = ListState::default();
        self.scan_complete = false;
        self.scan_started_at = Instant::now();
        self.scan_run_count += 1;
        self.startup_phases = StartupPhaseTracker::default();
        crate::perf_log::log_event(&format!(
            "scan_start kind=rescan run={}",
            self.scan_run_count
        ));
        self.fully_loaded.clear();
        self.priority_fetch_path = None;
        self.focus_pane(PaneId::ProjectList);
        self.show_settings = false;
        self.show_finder = false;
        self.searching = false;
        self.reset_project_panes();
        self.selected_project_path = None;
        self.pending_ci_fetch = None;
        self.expanded.clear();
        self.list_state = ListState::default();
        self.rows_dirty = true;
        self.disk_cache_dirty = true;
        self.fit_widths_dirty = true;
        self.tree_build_active = None;
        self.tree_build_latest = 0;
        self.fit_build_active = None;
        self.fit_build_latest = 0;
        self.disk_build_active = None;
        self.disk_build_latest = 0;
        self.sync_lint_runtime_projects();
        self.data_generation += 1;
        self.detail_generation += 1;
        let (tx, rx) = scan::spawn_streaming_scan(
            &self.scan_root,
            self.ci_run_count(),
            &self.current_config.tui.include_dirs,
            self.include_non_rust(),
            self.lint_enabled(),
            self.http_client.clone(),
        );
        self.bg_tx = tx;
        self.bg_rx = rx;
        self.respawn_watcher();
    }

    pub(super) fn poll_background(&mut self) -> PollBackgroundStats {
        const MAX_MSGS_PER_FRAME: usize = 50;
        let mut needs_rebuild = false;
        let mut msg_count = 0;
        let started = Instant::now();
        let mut stats = PollBackgroundStats::default();

        while msg_count < MAX_MSGS_PER_FRAME {
            let Ok(msg) = self.bg_rx.try_recv() else {
                break;
            };
            Self::record_background_msg_kind(&mut stats, &msg);
            msg_count += 1;
            needs_rebuild |= self.handle_bg_msg(msg);
        }
        stats.bg_msgs = msg_count;
        Self::log_saturated_background_batch(&stats);
        stats.ci_msgs = self.poll_ci_fetches();
        stats.example_msgs = self.poll_example_msgs();
        self.poll_clean_msgs();

        stats.tree_results = self.poll_tree_builds();
        stats.fit_results = self.poll_fit_width_builds();
        stats.disk_results = self.poll_disk_cache_builds();

        if needs_rebuild {
            self.request_tree_rebuild();
            self.maybe_priority_fetch();
        }
        stats.needs_rebuild = needs_rebuild;

        self.refresh_async_caches();
        crate::perf_log::log_duration(
            "poll_background",
            started.elapsed(),
            &format!(
                "bg_msgs={} ci_msgs={} example_msgs={} tree_results={} fit_results={} disk_results={} needs_rebuild={} projects={} nodes={}",
                stats.bg_msgs,
                stats.ci_msgs,
                stats.example_msgs,
                stats.tree_results,
                stats.fit_results,
                stats.disk_results,
                stats.needs_rebuild,
                self.all_projects.len(),
                self.nodes.len()
            ),
            crate::perf_log::slow_bg_batch_threshold_ms(),
        );
        stats
    }

    const fn record_background_msg_kind(stats: &mut PollBackgroundStats, msg: &BackgroundMsg) {
        match msg {
            BackgroundMsg::DiskUsage { .. } | BackgroundMsg::DiskUsageBatch { .. } => {
                stats.disk_usage_msgs += 1;
            },
            BackgroundMsg::GitInfo { .. } | BackgroundMsg::GitFirstCommit { .. } => {
                stats.git_info_msgs += 1;
            },
            BackgroundMsg::GitPathState { .. } => stats.git_path_state_msgs += 1,
            BackgroundMsg::LintStatus { .. } => stats.lint_status_msgs += 1,
            BackgroundMsg::CiRuns { .. }
            | BackgroundMsg::LocalGitQueued { .. }
            | BackgroundMsg::RepoFetchQueued { .. }
            | BackgroundMsg::RepoFetchComplete { .. }
            | BackgroundMsg::CratesIoVersion { .. }
            | BackgroundMsg::RepoMeta { .. }
            | BackgroundMsg::ProjectDiscovered { .. }
            | BackgroundMsg::ProjectRefreshed { .. }
            | BackgroundMsg::ScanComplete
            | BackgroundMsg::ServiceReachable { .. }
            | BackgroundMsg::ServiceRecovered { .. }
            | BackgroundMsg::ServiceUnreachable { .. } => {},
        }
    }

    fn log_saturated_background_batch(stats: &PollBackgroundStats) {
        const MAX_MSGS_PER_FRAME: usize = 50;
        if stats.bg_msgs != MAX_MSGS_PER_FRAME {
            return;
        }

        crate::perf_log::log_event(&format!(
            "poll_background_saturated bg_msgs={} disk_usage_msgs={} git_info_msgs={} git_path_state_msgs={} lint_status_msgs={}",
            stats.bg_msgs,
            stats.disk_usage_msgs,
            stats.git_info_msgs,
            stats.git_path_state_msgs,
            stats.lint_status_msgs
        ));
    }

    fn poll_ci_fetches(&mut self) -> usize {
        let mut count = 0;
        while let Ok(msg) = self.ci_fetch_rx.try_recv() {
            match msg {
                CiFetchMsg::Complete { path, result, kind } => {
                    self.handle_ci_fetch_complete(path, result, kind);
                },
            }
            count += 1;
        }
        count
    }

    fn poll_example_msgs(&mut self) -> usize {
        let mut count = 0;
        while let Ok(msg) = self.example_rx.try_recv() {
            match msg {
                ExampleMsg::Output(line) => self.example_output.push(line),
                ExampleMsg::Progress(line) => self.apply_example_progress(line),
                ExampleMsg::Finished => self.finish_example_run(),
            }
            count += 1;
        }
        count
    }

    fn apply_example_progress(&mut self, line: String) {
        if let Some(last) = self.example_output.last_mut() {
            *last = line;
        } else {
            self.example_output.push(line);
        }
    }

    fn finish_example_run(&mut self) {
        self.example_running = None;
        self.example_output.push("── done ──".to_string());
        self.terminal_dirty = true;
    }

    fn poll_clean_msgs(&mut self) {
        while let Ok(msg) = self.clean_rx.try_recv() {
            match msg {
                CleanMsg::Finished(task_id) => self.finish_task_toast(task_id),
            }
        }
    }

    fn handle_disk_usage(&mut self, path: String, bytes: u64) {
        self.apply_disk_usage(path, bytes, self.scan_complete);
    }

    fn handle_disk_usage_batch(&mut self, entries: Vec<(String, u64)>) {
        for (path, bytes) in entries {
            self.apply_disk_usage(path, bytes, false);
        }
    }

    fn apply_disk_usage(&mut self, path: String, bytes: u64, refresh_git_path_state: bool) {
        self.fully_loaded.insert(path.clone());
        self.disk_usage.insert(path.clone(), bytes);
        if refresh_git_path_state {
            self.refresh_git_path_state(&path);
        }
        self.disk_cache_dirty = true;
        self.fit_widths_dirty = true;
        let mut lint_runtime_changed = false;
        if bytes == 0 {
            let abs = self
                .all_projects
                .iter()
                .find(|project| project.path == path)
                .map(|project| project.abs_path.as_str());
            if let Some(abs) = abs
                && !std::path::Path::new(abs).exists()
            {
                lint_runtime_changed |= self.deleted_projects.insert(path);
            }
        } else {
            lint_runtime_changed |= self.deleted_projects.remove(&path);
        }
        if lint_runtime_changed {
            self.sync_lint_runtime_projects();
        }
    }

    fn handle_git_info(&mut self, path: String, info: GitInfo) {
        self.fit_widths_dirty = true;
        let seen_path = path.clone();
        let preserved_first_commit = self
            .git_info
            .get(&path)
            .and_then(|existing| existing.first_commit.clone());
        let mut info = info;
        if info.first_commit.is_none() {
            info.first_commit = preserved_first_commit;
        }
        let matching_node = self
            .nodes
            .iter()
            .find(|node| node.project.path == path)
            .or_else(|| {
                self.nodes
                    .iter()
                    .flat_map(|node| node.worktrees.iter())
                    .find(|worktree| worktree.project.path == path)
            });
        if let Some(node) = matching_node {
            for member in Self::all_group_members(node) {
                // Always overwrite: the correct branch comes from the
                // workspace root, not from a stale propagation.
                self.git_info.insert(member.path.clone(), info.clone());
            }
            for worktree in &node.worktrees {
                self.git_info
                    .entry(worktree.project.path.clone())
                    .or_insert_with(|| info.clone());
            }
        }
        self.git_info.insert(path, info);
        if self.scan_complete {
            self.startup_phases.git_seen.insert(seen_path);
            self.maybe_log_startup_phase_completions();
        }
        self.finder_dirty = true;
    }

    fn handle_git_first_commit(&mut self, path: &str, first_commit: Option<String>) {
        let Some(info) = self.git_info.get_mut(path) else {
            return;
        };
        info.first_commit = first_commit;
    }

    fn handle_repo_fetch_complete(&mut self, key: String) {
        self.startup_phases.repo_seen.insert(key);
        self.maybe_log_startup_phase_completions();
    }

    fn handle_repo_meta(&mut self, path: String, stars: u64, description: Option<String>) {
        if let Some(node) = self.nodes.iter().find(|node| node.project.path == path) {
            for member in Self::all_group_members(node) {
                self.stars.entry(member.path.clone()).or_insert(stars);
            }
        }
        self.stars.insert(path.clone(), stars);
        if let Some(desc) = description {
            self.repo_descriptions.insert(path, desc);
        }
    }

    fn handle_project_discovered(&mut self, project: RustProject) -> bool {
        if self
            .all_projects
            .iter()
            .any(|existing| existing.path == project.path)
        {
            return false;
        }

        self.register_project_background_services(&project);
        self.all_projects.push(project);
        if self.scan_complete {
            self.sync_lint_runtime_projects();
        }
        true
    }

    fn handle_project_refreshed(&mut self, project: &RustProject) -> bool {
        let project_path = project.path.clone();
        let updated_in_all_projects = self
            .all_projects
            .iter_mut()
            .find(|existing| existing.path == project_path)
            .is_some_and(|existing| {
                *existing = project.clone();
                true
            });

        let updated_in_nodes = self.replace_project_in_nodes(&project_path, project);
        let updated = updated_in_all_projects || updated_in_nodes;

        if !updated {
            return false;
        }

        self.recompute_cargo_active_paths();
        self.prune_inactive_project_state();
        self.sync_lint_runtime_projects();
        self.cached_detail = None;
        self.finder_dirty = true;
        self.rows_dirty = true;
        self.fit_widths_dirty = true;
        true
    }

    fn replace_project_in_nodes(&mut self, project_path: &str, project: &RustProject) -> bool {
        let mut updated = false;

        for node in &mut self.nodes {
            updated |= replace_project_in_node(node, project_path, project);
        }

        updated
    }

    fn apply_service_signal(&mut self, signal: ServiceSignal) {
        match signal {
            ServiceSignal::Reachable(service) => {
                self.unreachable_services.remove(&service);
            },
            ServiceSignal::Unreachable(service) => {
                self.unreachable_services.insert(service);
                if self.service_retry_active.insert(service) {
                    self.spawn_service_retry(service);
                }
            },
        }
    }

    fn spawn_service_retry(&self, service: ServiceKind) {
        #[cfg(test)]
        if !self.service_retry_spawns_enabled {
            return;
        }

        let tx = self.bg_tx.clone();
        let client = self.http_client.clone();
        thread::spawn(move || {
            loop {
                if client.probe_service(service) {
                    crate::scan::emit_service_recovered(&tx, service);
                    break;
                }
                thread::sleep(Duration::from_secs(SERVICE_RETRY_SECS));
            }
        });
    }

    fn mark_service_recovered(&mut self, service: ServiceKind) {
        self.unreachable_services.remove(&service);
        self.service_retry_active.remove(&service);
    }

    pub(super) fn unreachable_service_message(&self) -> Option<String> {
        let mut services = Vec::new();
        for service in [ServiceKind::GitHub, ServiceKind::CratesIo] {
            if self.unreachable_services.contains(&service) {
                services.push(service.label());
            }
        }
        match services.as_slice() {
            [service] => Some(format!(" {service} unreachable ")),
            [first, second] => Some(format!(" {first} and {second} unreachable ")),
            _ => None,
        }
    }

    /// Handle a single `BackgroundMsg`. Returns `true` if the tree needs rebuilding.
    #[allow(clippy::too_many_lines, reason = "match arms are simple dispatches")]
    fn handle_bg_msg(&mut self, msg: BackgroundMsg) -> bool {
        // Bump generation for any message that carries project data, so the
        // visible caches auto-invalidate without separate dirty flags.
        if msg.path().is_some() {
            self.data_generation += 1;
        }
        if let Some(path) = msg.path()
            && self.detail_path_is_affected(path)
        {
            self.detail_generation += 1;
        }
        match msg {
            BackgroundMsg::DiskUsage { path, bytes } => {
                self.startup_phases.disk_seen.insert(path.clone());
                self.handle_disk_usage(path, bytes);
                self.maybe_log_startup_phase_completions();
            },
            BackgroundMsg::DiskUsageBatch { root_path, entries } => {
                self.data_generation += 1;
                if entries
                    .iter()
                    .any(|(path, _)| self.detail_path_is_affected(path))
                {
                    self.detail_generation += 1;
                }
                self.startup_phases.disk_seen.insert(root_path);
                self.handle_disk_usage_batch(entries);
                self.maybe_log_startup_phase_completions();
            },
            BackgroundMsg::LocalGitQueued { path } => {
                self.startup_phases.git_expected.insert(path);
            },
            BackgroundMsg::CiRuns { path, runs } => {
                self.insert_ci_runs(path, runs);
            },
            BackgroundMsg::RepoFetchQueued { key } => {
                self.startup_phases.repo_expected.insert(key);
            },
            BackgroundMsg::RepoFetchComplete { key } => {
                self.handle_repo_fetch_complete(key);
            },
            BackgroundMsg::GitInfo { path, info } => {
                self.handle_git_info(path, info);
            },
            BackgroundMsg::GitFirstCommit { path, first_commit } => {
                self.handle_git_first_commit(&path, first_commit);
            },
            BackgroundMsg::GitPathState { path, state } => {
                crate::perf_log::log_event(&format!(
                    "app_git_path_state_applied path={} state={}",
                    path,
                    state.label()
                ));
                self.git_path_states.insert(path, state);
            },
            BackgroundMsg::CratesIoVersion {
                path,
                version,
                downloads,
            } => {
                if self.is_cargo_active_path(&path) {
                    self.crates_versions.insert(path.clone(), version);
                    self.crates_downloads.insert(path, downloads);
                } else {
                    self.crates_versions.remove(&path);
                    self.crates_downloads.remove(&path);
                }
            },
            BackgroundMsg::RepoMeta {
                path,
                stars,
                description,
            } => {
                self.handle_repo_meta(path, stars, description);
            },
            BackgroundMsg::ProjectDiscovered { project } => {
                if self.handle_project_discovered(project) {
                    return true;
                }
            },
            BackgroundMsg::ProjectRefreshed { project } => {
                if self.handle_project_refreshed(&project) {
                    return true;
                }
            },
            BackgroundMsg::LintStatus { path, status } => {
                let status_started = matches!(status, LintStatus::Running(_));
                let status_is_terminal = matches!(
                    status,
                    LintStatus::Passed(_)
                        | LintStatus::Failed(_)
                        | LintStatus::Stale
                        | LintStatus::NoLog
                );
                if !self.is_cargo_active_path(&path) {
                    self.port_report_runs.remove(&path);
                    self.lint_status.remove(&path);
                    return false;
                }
                let eligible = self
                    .all_projects
                    .iter()
                    .find(|project| project.path == path)
                    .is_some_and(|project| {
                        crate::lint_runtime::project_is_eligible(
                            &self.current_config.lint,
                            &project.path,
                            &PathBuf::from(&project.abs_path),
                            project.is_rust == Rust,
                        )
                    });
                if eligible {
                    self.reload_port_report_history(&path);
                    if matches!(status, LintStatus::NoLog) {
                        self.lint_status.remove(&path);
                    } else {
                        self.lint_status.insert(path.clone(), status);
                    }
                } else {
                    self.port_report_runs.remove(&path);
                    self.lint_status.remove(&path);
                    self.running_lint_paths.remove(&path);
                }
                self.update_lint_rollups_for_path(&path);
                if status_started {
                    self.running_lint_paths.insert(path.clone());
                }
                if status_is_terminal {
                    self.running_lint_paths.remove(&path);
                }
                self.sync_running_lint_toast();
                if self.scan_complete {
                    if status_started {
                        let expected = self
                            .startup_phases
                            .lint_expected
                            .get_or_insert_with(HashSet::new);
                        let inserted = expected.insert(path.clone());
                        if inserted {
                            self.startup_phases.lint_complete_at = None;
                        }
                    }
                    if status_is_terminal
                        && self
                            .startup_phases
                            .lint_expected
                            .as_ref()
                            .is_some_and(|expected| expected.contains(&path))
                    {
                        self.startup_phases.lint_seen_terminal.insert(path);
                    }
                    self.maybe_log_startup_phase_completions();
                }
            },
            BackgroundMsg::ScanComplete => {
                let kind = if self.scan_run_count == 1 {
                    "initial"
                } else {
                    "rescan"
                };
                crate::perf_log::log_duration(
                    "scan_complete",
                    self.scan_started_at.elapsed(),
                    &format!(
                        "kind={} run={} projects={}",
                        kind,
                        self.scan_run_count,
                        self.all_projects.len()
                    ),
                    0,
                );
                self.scan_complete = true;
                self.initialize_startup_phase_tracker();
                self.sync_lint_runtime_projects_immediately();
                self.schedule_git_path_state_refreshes();
                self.schedule_git_first_commit_refreshes();
                if self.focused_pane == PaneId::ScanLog {
                    self.focus_pane(PaneId::ProjectList);
                }
            },
            BackgroundMsg::ServiceReachable { service } => {
                self.apply_service_signal(ServiceSignal::Reachable(service));
            },
            BackgroundMsg::ServiceRecovered { service } => {
                self.mark_service_recovered(service);
            },
            BackgroundMsg::ServiceUnreachable { service } => {
                self.apply_service_signal(ServiceSignal::Unreachable(service));
            },
        }
        false
    }

    fn detail_path_is_affected(&self, path: &str) -> bool {
        let Some(project) = self.selected_project() else {
            return false;
        };
        self.selected_lint_rollup_key().map_or_else(
            || project.path == path,
            |key| {
                self.lint_rollup_paths
                    .get(&key)
                    .is_some_and(|paths| paths.iter().any(|candidate| candidate == path))
            },
        )
    }

    /// Insert CI runs from the initial scan, propagating to workspace members.
    fn insert_ci_runs(&mut self, path: String, runs: Vec<CiRun>) {
        if !self.is_cargo_active_path(&path) {
            self.ci_state.remove(&path);
            return;
        }
        let exhausted = self
            .git_info
            .get(&path)
            .and_then(|g| g.url.as_ref())
            .and_then(|url| ci::parse_owner_repo(url))
            .is_some_and(|(owner, repo)| scan::is_exhausted(&owner, &repo));
        if let Some(node) = self.nodes.iter().find(|n| n.project.path == path) {
            for member in Self::all_group_members(node) {
                self.ci_state
                    .entry(member.path.clone())
                    .or_insert_with(|| CiState::Loaded {
                        runs: runs.clone(),
                        exhausted,
                    });
            }
        }
        self.ci_state
            .insert(path, CiState::Loaded { runs, exhausted });
    }

    /// Process a completed CI fetch: merge runs, detect exhaustion, propagate to members.
    fn handle_ci_fetch_complete(&mut self, path: String, result: CiFetchResult, kind: CiFetchKind) {
        let new_runs = match result {
            CiFetchResult::Loaded(runs) | CiFetchResult::CacheOnly(runs) => runs,
        };

        // Previous count comes from what was visible in the Fetching state
        let prev_count = self.ci_state.get(&path).map_or(0, |s| s.runs().len());

        let existing = self
            .ci_state
            .remove(&path)
            .map(|s| match s {
                CiState::Fetching { runs, .. } | CiState::Loaded { runs, .. } => runs,
            })
            .unwrap_or_default();

        let mut seen = HashSet::new();
        let mut merged: Vec<CiRun> = Vec::new();
        for run in new_runs {
            if seen.insert(run.run_id) {
                merged.push(run);
            }
        }
        for run in existing {
            if seen.insert(run.run_id) {
                merged.push(run);
            }
        }
        merged.sort_by(|a, b| b.run_id.cmp(&a.run_id));

        let found_new = merged.len() > prev_count;
        let exhausted = if found_new {
            // New runs discovered — clear the exhaustion marker on disk.
            if let Some(git) = self.git_info.get(&path)
                && let Some(ref url) = git.url
                && let Some((owner, repo)) = ci::parse_owner_repo(url)
            {
                scan::clear_exhausted(&owner, &repo);
            }
            false
        } else {
            // No new runs — mark exhausted for both fetch-older and refresh.
            if let Some(git) = self.git_info.get(&path)
                && let Some(ref url) = git.url
                && let Some((owner, repo)) = ci::parse_owner_repo(url)
            {
                scan::mark_exhausted(&owner, &repo);
            }
            if matches!(kind, CiFetchKind::Refresh) {
                self.status_flash =
                    Some(("no new runs found".to_string(), std::time::Instant::now()));
                self.show_timed_toast("CI", "No new runs found".to_string());
            }
            true
        };

        let state = CiState::Loaded {
            runs: merged.clone(),
            exhausted,
        };

        if let Some(node) = self.nodes.iter().find(|n| n.project.path == path) {
            for member in Self::all_group_members(node) {
                self.ci_state
                    .entry(member.path.clone())
                    .or_insert_with(|| CiState::Loaded {
                        runs: merged.clone(),
                        exhausted,
                    });
            }
        }
        self.ci_pane.set_pos(merged.len());
        self.ci_state.insert(path, state);
        self.data_generation += 1;
    }

    /// Spawn a priority fetch for the selected project if it hasn't been loaded yet.
    pub(super) fn maybe_priority_fetch(&mut self) {
        let Some(project) = self.selected_project() else {
            return;
        };
        let path = project.path.clone();
        let abs_path = project.abs_path.clone();
        let name = project.name.clone();
        if !self.fully_loaded.contains(&path) && self.priority_fetch_path.as_ref() != Some(&path) {
            self.priority_fetch_path = Some(path.clone());
            super::terminal::spawn_priority_fetch(self, &path, &abs_path, name.as_ref());
        }
    }

    /// Ensure the cached visible rows are up to date, recomputing only when dirty.
    pub fn ensure_visible_rows_cached(&mut self) {
        if !self.rows_dirty {
            return;
        }
        self.rows_dirty = false;
        self.cached_visible_rows = build_visible_rows(&self.nodes, &self.expanded);
    }

    /// Return the cached visible rows. Must call `ensure_visible_rows_cached()` first.
    pub fn visible_rows(&self) -> &[VisibleRow] {
        &self.cached_visible_rows
    }

    /// Keep fit-to-content widths rebuilding in the background, never inline on the UI thread.
    pub fn ensure_fit_widths_cached(&mut self) {
        self.request_fit_widths_build();
    }

    /// Iterate all group members in a node, including those nested under worktree entries.
    fn all_group_members(node: &ProjectNode) -> impl Iterator<Item = &RustProject> {
        let direct = node.groups.iter().flat_map(|g| g.members.iter());
        let wt = node
            .worktrees
            .iter()
            .flat_map(|wt| wt.groups.iter().flat_map(|g| g.members.iter()));
        direct.chain(wt)
    }

    fn all_vendored_projects(node: &ProjectNode) -> impl Iterator<Item = &RustProject> {
        let direct = node.vendored.iter();
        let wt = node
            .worktrees
            .iter()
            .flat_map(|worktree| worktree.vendored.iter());
        direct.chain(wt)
    }

    fn observe_name_width(widths: &mut ResolvedWidths, content_width: usize) {
        use super::columns::COL_NAME;

        widths.observe(COL_NAME, Self::name_width_with_gutter(content_width));
    }

    const fn name_width_with_gutter(content_width: usize) -> usize {
        content_width.saturating_add(1)
    }

    fn fit_name_for_node(node: &ProjectNode, live_worktrees: usize) -> usize {
        let dw = super::columns::display_width;
        let mut name = node.project.display_name();
        if live_worktrees > 0 {
            name = format!("{name} {WORKTREE}:{live_worktrees}");
        }
        dw(PREFIX_ROOT_COLLAPSED) + dw(&name)
    }

    /// Keep disk sort caches rebuilding in the background, never inline on the UI thread.
    pub fn ensure_disk_cache(&mut self) {
        self.request_disk_cache_build();
    }

    /// Ensure the cached `DetailInfo` is up to date for the selected project.
    /// The cache is valid only when the generation AND path both match.
    pub fn ensure_detail_cached(&mut self) {
        let current_selection = self.current_detail_selection_key();

        if let Some(ref cache) = self.cached_detail
            && cache.generation == self.detail_generation
            && cache.selection == current_selection
        {
            return;
        }

        self.cached_detail = self.selected_project().map(|p| DetailCache {
            generation: self.detail_generation,
            selection: current_selection,
            info: super::detail::build_detail_info(self, p),
        });
    }

    fn selected_row(&self) -> Option<VisibleRow> {
        if self.searching && !self.search_query.is_empty() {
            return None;
        }
        let rows = self.visible_rows();
        let selected = self.list_state.selected()?;
        rows.get(selected).copied()
    }

    fn current_detail_selection_key(&self) -> String {
        if self.searching && !self.search_query.is_empty() {
            return self
                .selected_project()
                .map(|project| format!("search:{}", project.path))
                .unwrap_or_default();
        }
        match self.selected_row() {
            Some(VisibleRow::Root { node_index }) => format!("root:{node_index}"),
            Some(VisibleRow::GroupHeader {
                node_index,
                group_index,
            }) => format!("group:{node_index}:{group_index}"),
            Some(VisibleRow::Member {
                node_index,
                group_index,
                member_index,
            }) => format!("member:{node_index}:{group_index}:{member_index}"),
            Some(VisibleRow::Vendored {
                node_index,
                vendored_index,
            }) => format!("vendored:{node_index}:{vendored_index}"),
            Some(VisibleRow::WorktreeEntry {
                node_index,
                worktree_index,
            }) => format!("worktree:{node_index}:{worktree_index}"),
            Some(VisibleRow::WorktreeGroupHeader {
                node_index,
                worktree_index,
                group_index,
            }) => format!("worktree-group:{node_index}:{worktree_index}:{group_index}"),
            Some(VisibleRow::WorktreeMember {
                node_index,
                worktree_index,
                group_index,
                member_index,
            }) => format!(
                "worktree-member:{node_index}:{worktree_index}:{group_index}:{member_index}"
            ),
            Some(VisibleRow::WorktreeVendored {
                node_index,
                worktree_index,
                vendored_index,
            }) => format!("worktree-vendored:{node_index}:{worktree_index}:{vendored_index}"),
            None => String::new(),
        }
    }

    /// Returns the `ProjectNode` when a root row is selected (not a member or worktree).
    pub fn selected_node(&self) -> Option<&ProjectNode> {
        match self.selected_row()? {
            VisibleRow::Root { node_index } => self.nodes.get(node_index),
            _ => None,
        }
    }

    pub fn selected_project(&self) -> Option<&RustProject> {
        if self.searching && !self.search_query.is_empty() {
            let selected = self.list_state.selected()?;
            let flat_idx = *self.filtered.get(selected)?;
            let entry = self.flat_entries.get(flat_idx)?;
            self.project_by_path(&entry.path)
        } else {
            let rows = self.visible_rows();
            let selected = self.list_state.selected()?;
            match rows.get(selected)? {
                VisibleRow::Root { node_index } | VisibleRow::GroupHeader { node_index, .. } => {
                    Some(&self.nodes.get(*node_index)?.project)
                },
                VisibleRow::Member {
                    node_index,
                    group_index,
                    member_index,
                } => {
                    let node = self.nodes.get(*node_index)?;
                    let group = node.groups.get(*group_index)?;
                    group.members.get(*member_index)
                },
                VisibleRow::Vendored {
                    node_index,
                    vendored_index,
                } => self.nodes.get(*node_index)?.vendored.get(*vendored_index),
                VisibleRow::WorktreeEntry {
                    node_index,
                    worktree_index,
                }
                | VisibleRow::WorktreeGroupHeader {
                    node_index,
                    worktree_index,
                    ..
                } => {
                    let node = self.nodes.get(*node_index)?;
                    let wt = node.worktrees.get(*worktree_index)?;
                    Some(&wt.project)
                },
                VisibleRow::WorktreeMember {
                    node_index,
                    worktree_index,
                    group_index,
                    member_index,
                } => {
                    let wt = self
                        .nodes
                        .get(*node_index)?
                        .worktrees
                        .get(*worktree_index)?;
                    let group = wt.groups.get(*group_index)?;
                    group.members.get(*member_index)
                },
                VisibleRow::WorktreeVendored {
                    node_index,
                    worktree_index,
                    vendored_index,
                } => self
                    .nodes
                    .get(*node_index)?
                    .worktrees
                    .get(*worktree_index)?
                    .vendored
                    .get(*vendored_index),
            }
        }
    }

    fn selected_is_expandable(&self) -> bool {
        if self.searching && !self.search_query.is_empty() {
            return false;
        }
        let rows = self.visible_rows();
        let Some(selected) = self.list_state.selected() else {
            return false;
        };
        match rows.get(selected) {
            Some(VisibleRow::Root { node_index }) => self.nodes[*node_index].has_children(),
            Some(VisibleRow::GroupHeader { .. } | VisibleRow::WorktreeGroupHeader { .. }) => true,
            Some(VisibleRow::WorktreeEntry {
                node_index,
                worktree_index,
            }) => self.nodes[*node_index].worktrees[*worktree_index].has_children(),
            _ => false,
        }
    }

    fn expand_key_for_row(&self, row: VisibleRow) -> Option<ExpandKey> {
        match row {
            VisibleRow::Root { node_index } => self.nodes[node_index]
                .has_children()
                .then_some(ExpandKey::Node(node_index)),
            VisibleRow::GroupHeader {
                node_index,
                group_index,
            } => Some(ExpandKey::Group(node_index, group_index)),
            VisibleRow::WorktreeEntry {
                node_index,
                worktree_index,
            } => self.nodes[node_index].worktrees[worktree_index]
                .has_children()
                .then_some(ExpandKey::Worktree(node_index, worktree_index)),
            VisibleRow::WorktreeGroupHeader {
                node_index,
                worktree_index,
                group_index,
            } => Some(ExpandKey::WorktreeGroup(
                node_index,
                worktree_index,
                group_index,
            )),
            VisibleRow::Member { .. }
            | VisibleRow::Vendored { .. }
            | VisibleRow::WorktreeMember { .. }
            | VisibleRow::WorktreeVendored { .. } => None,
        }
    }

    pub(super) fn expand(&mut self) -> bool {
        if !self.selected_is_expandable() {
            return false;
        }
        let Some(selected) = self.list_state.selected() else {
            return false;
        };
        let Some(row) = self.visible_rows().get(selected).copied() else {
            return false;
        };
        let Some(key) = self.expand_key_for_row(row) else {
            return false;
        };
        if self.expanded.insert(key) {
            self.rows_dirty = true;
            true
        } else {
            false
        }
    }

    /// Remove `key` from expanded, recompute rows, and move cursor to `target`.
    fn collapse_to(&mut self, key: &ExpandKey, target: VisibleRow) {
        self.expanded.remove(key);
        self.rows_dirty = true;
        self.ensure_visible_rows_cached();
        if let Some(pos) = self.visible_rows().iter().position(|r| *r == target) {
            self.list_state.select(Some(pos));
        }
    }

    /// Try to remove `key` from expanded. If present, mark dirty and return `true`.
    /// Otherwise return `false` (caller should cascade to parent).
    fn try_collapse(&mut self, key: &ExpandKey) -> bool {
        if self.expanded.remove(key) {
            self.rows_dirty = true;
            true
        } else {
            false
        }
    }

    pub(super) fn collapse(&mut self) -> bool {
        let Some(selected) = self.list_state.selected() else {
            return false;
        };
        let Some(row) = self.visible_rows().get(selected).copied() else {
            return false;
        };
        let expanded_before = self.expanded.len();
        let selected_before = self.list_state.selected();
        self.collapse_row(row);
        self.expanded.len() != expanded_before
            || self.list_state.selected() != selected_before
            || self.rows_dirty
    }

    fn collapse_row(&mut self, row: VisibleRow) {
        match row {
            VisibleRow::Root { node_index: ni } => {
                self.try_collapse(&ExpandKey::Node(ni));
            },
            VisibleRow::GroupHeader {
                node_index: ni,
                group_index: gi,
            } => {
                if !self.try_collapse(&ExpandKey::Group(ni, gi)) {
                    self.collapse_to(&ExpandKey::Node(ni), VisibleRow::Root { node_index: ni });
                }
            },
            VisibleRow::Member {
                node_index: ni,
                group_index: gi,
                ..
            } => {
                if self.nodes[ni].groups[gi].name.is_empty() {
                    self.collapse_to(&ExpandKey::Node(ni), VisibleRow::Root { node_index: ni });
                } else {
                    self.collapse_to(
                        &ExpandKey::Group(ni, gi),
                        VisibleRow::GroupHeader {
                            node_index: ni,
                            group_index: gi,
                        },
                    );
                }
            },
            VisibleRow::Vendored { node_index: ni, .. } => {
                self.collapse_to(&ExpandKey::Node(ni), VisibleRow::Root { node_index: ni });
            },
            VisibleRow::WorktreeEntry {
                node_index: ni,
                worktree_index: wi,
            } => {
                if !self.try_collapse(&ExpandKey::Worktree(ni, wi)) {
                    self.collapse_to(&ExpandKey::Node(ni), VisibleRow::Root { node_index: ni });
                }
            },
            VisibleRow::WorktreeGroupHeader {
                node_index: ni,
                worktree_index: wi,
                group_index: gi,
            } => {
                if !self.try_collapse(&ExpandKey::WorktreeGroup(ni, wi, gi)) {
                    self.collapse_to(
                        &ExpandKey::Worktree(ni, wi),
                        VisibleRow::WorktreeEntry {
                            node_index: ni,
                            worktree_index: wi,
                        },
                    );
                }
            },
            VisibleRow::WorktreeMember {
                node_index: ni,
                worktree_index: wi,
                group_index: gi,
                ..
            } => {
                if self.nodes[ni].worktrees[wi].groups[gi].name.is_empty() {
                    self.collapse_to(
                        &ExpandKey::Worktree(ni, wi),
                        VisibleRow::WorktreeEntry {
                            node_index: ni,
                            worktree_index: wi,
                        },
                    );
                } else {
                    self.collapse_to(
                        &ExpandKey::WorktreeGroup(ni, wi, gi),
                        VisibleRow::WorktreeGroupHeader {
                            node_index: ni,
                            worktree_index: wi,
                            group_index: gi,
                        },
                    );
                }
            },
            VisibleRow::WorktreeVendored {
                node_index: ni,
                worktree_index: wi,
                ..
            } => {
                self.collapse_to(
                    &ExpandKey::Worktree(ni, wi),
                    VisibleRow::WorktreeEntry {
                        node_index: ni,
                        worktree_index: wi,
                    },
                );
            },
        }
    }

    pub(super) fn row_count(&self) -> usize {
        if self.searching && !self.search_query.is_empty() {
            self.filtered.len()
        } else {
            self.visible_rows().len()
        }
    }

    pub(super) fn move_up(&mut self) {
        let count = self.row_count();
        if count == 0 {
            return;
        }
        let current = self.list_state.selected().unwrap_or(0);
        if current > 0 {
            self.list_state.select(Some(current - 1));
        }
    }

    pub(super) fn move_down(&mut self) {
        let count = self.row_count();
        if count == 0 {
            return;
        }
        let current = self.list_state.selected().unwrap_or(0);
        if current < count - 1 {
            self.list_state.select(Some(current + 1));
        }
    }

    pub(super) fn move_to_top(&mut self) {
        if self.row_count() > 0 {
            self.list_state.select(Some(0));
        }
    }

    pub(super) fn move_to_bottom(&mut self) {
        let count = self.row_count();
        if count > 0 {
            self.list_state.select(Some(count - 1));
        }
    }

    const fn collapse_anchor_row(row: VisibleRow) -> VisibleRow {
        match row {
            VisibleRow::GroupHeader { node_index, .. }
            | VisibleRow::Member { node_index, .. }
            | VisibleRow::Vendored { node_index, .. } => VisibleRow::Root { node_index },
            VisibleRow::Root { .. } | VisibleRow::WorktreeEntry { .. } => row,
            VisibleRow::WorktreeGroupHeader {
                node_index,
                worktree_index,
                ..
            }
            | VisibleRow::WorktreeMember {
                node_index,
                worktree_index,
                ..
            }
            | VisibleRow::WorktreeVendored {
                node_index,
                worktree_index,
                ..
            } => VisibleRow::WorktreeEntry {
                node_index,
                worktree_index,
            },
        }
    }

    pub(super) fn expand_all(&mut self) {
        let selected_path = self
            .collapsed_selection_path
            .take()
            .or_else(|| self.selected_project().map(|project| project.path.clone()));
        self.collapsed_anchor_path = None;
        for (node_index, node) in self.nodes.iter().enumerate() {
            if node.has_children() {
                self.expanded.insert(ExpandKey::Node(node_index));
            }
            for (group_index, group) in node.groups.iter().enumerate() {
                if !group.name.is_empty() {
                    self.expanded
                        .insert(ExpandKey::Group(node_index, group_index));
                }
            }
            for (worktree_index, worktree) in node.worktrees.iter().enumerate() {
                if worktree.has_children() {
                    self.expanded
                        .insert(ExpandKey::Worktree(node_index, worktree_index));
                }
                for (group_index, group) in worktree.groups.iter().enumerate() {
                    if !group.name.is_empty() {
                        self.expanded.insert(ExpandKey::WorktreeGroup(
                            node_index,
                            worktree_index,
                            group_index,
                        ));
                    }
                }
            }
        }
        self.rows_dirty = true;
        if let Some(path) = selected_path {
            self.select_project_in_tree(&path);
        }
    }

    pub(super) fn collapse_all(&mut self) {
        let selected_path = self.selected_project().map(|project| project.path.clone());
        let anchor = self.selected_row().map(Self::collapse_anchor_row);
        self.expanded.clear();
        self.rows_dirty = true;
        self.ensure_visible_rows_cached();
        if let Some(anchor) = anchor
            && let Some(pos) = self.visible_rows().iter().position(|row| *row == anchor)
        {
            self.list_state.select(Some(pos));
        }
        let anchor_path = self.selected_project().map(|project| project.path.clone());
        if selected_path == anchor_path {
            self.collapsed_selection_path = None;
            self.collapsed_anchor_path = None;
        } else {
            self.collapsed_selection_path = selected_path;
            self.collapsed_anchor_path = anchor_path;
        }
    }

    pub(super) fn scan_log_scroll_up(&mut self) {
        if self.scan_log.is_empty() {
            return;
        }
        let current = self.scan_log_state.selected().unwrap_or(0);
        if current > 0 {
            self.scan_log_state.select(Some(current - 1));
        }
    }

    pub(super) fn scan_log_scroll_down(&mut self) {
        if self.scan_log.is_empty() {
            return;
        }
        let current = self.scan_log_state.selected().unwrap_or(0);
        if current < self.scan_log.len() - 1 {
            self.scan_log_state.select(Some(current + 1));
        }
    }

    pub(super) const fn scan_log_to_top(&mut self) {
        if !self.scan_log.is_empty() {
            self.scan_log_state.select(Some(0));
        }
    }

    pub(super) const fn scan_log_to_bottom(&mut self) {
        if !self.scan_log.is_empty() {
            self.scan_log_state
                .select(Some(self.scan_log.len().saturating_sub(1)));
        }
    }

    pub(super) fn cancel_search(&mut self) {
        self.searching = false;
        self.search_query.clear();
        self.filtered.clear();
        self.rows_dirty = true;
        self.close_overlay();
        if !self.nodes.is_empty() {
            self.list_state.select(Some(0));
        }
    }

    pub(super) fn confirm_search(&mut self) {
        let project_path = self.selected_project().map(|p| p.path.clone());
        self.searching = false;
        self.search_query.clear();
        self.filtered.clear();
        self.rows_dirty = true;
        self.close_overlay();

        if let Some(target_path) = project_path {
            self.select_project_in_tree(&target_path);
        }
    }

    fn expand_path_in_tree(&mut self, target_path: &str) {
        for (ni, node) in self.nodes.iter().enumerate() {
            for (gi, group) in node.groups.iter().enumerate() {
                for member in &group.members {
                    if member.path == target_path {
                        self.expanded.insert(ExpandKey::Node(ni));
                        if !group.name.is_empty() {
                            self.expanded.insert(ExpandKey::Group(ni, gi));
                        }
                    }
                }
            }
            for vendored in &node.vendored {
                if vendored.path == target_path {
                    self.expanded.insert(ExpandKey::Node(ni));
                }
            }
            for (wi, wt) in node.worktrees.iter().enumerate() {
                if wt.project.path == target_path {
                    self.expanded.insert(ExpandKey::Node(ni));
                }
                for (gi, group) in wt.groups.iter().enumerate() {
                    for member in &group.members {
                        if member.path == target_path {
                            self.expanded.insert(ExpandKey::Node(ni));
                            self.expanded.insert(ExpandKey::Worktree(ni, wi));
                            if !group.name.is_empty() {
                                self.expanded.insert(ExpandKey::WorktreeGroup(ni, wi, gi));
                            }
                        }
                    }
                }
                for vendored in &wt.vendored {
                    if vendored.path == target_path {
                        self.expanded.insert(ExpandKey::Node(ni));
                        self.expanded.insert(ExpandKey::Worktree(ni, wi));
                    }
                }
            }
        }
    }

    fn row_matches_project_path(&self, row: VisibleRow, target_path: &str) -> bool {
        match row {
            VisibleRow::Root { node_index } => self.nodes[node_index].project.path == target_path,
            VisibleRow::Member {
                node_index,
                group_index,
                member_index,
            } => {
                self.nodes[node_index].groups[group_index].members[member_index].path == target_path
            },
            VisibleRow::WorktreeEntry {
                node_index,
                worktree_index,
            } => {
                self.nodes[node_index].worktrees[worktree_index]
                    .project
                    .path
                    == target_path
            },
            VisibleRow::WorktreeMember {
                node_index,
                worktree_index,
                group_index,
                member_index,
            } => {
                self.nodes[node_index].worktrees[worktree_index].groups[group_index].members
                    [member_index]
                    .path
                    == target_path
            },
            VisibleRow::Vendored {
                node_index,
                vendored_index,
            } => self.nodes[node_index].vendored[vendored_index].path == target_path,
            VisibleRow::WorktreeVendored {
                node_index,
                worktree_index,
                vendored_index,
            } => {
                self.nodes[node_index].worktrees[worktree_index].vendored[vendored_index].path
                    == target_path
            },
            VisibleRow::GroupHeader { .. } | VisibleRow::WorktreeGroupHeader { .. } => false,
        }
    }

    fn select_matching_visible_row(&mut self, target_path: &str) {
        self.ensure_visible_rows_cached();
        let selected_index = self
            .visible_rows()
            .iter()
            .position(|row| self.row_matches_project_path(*row, target_path));
        if let Some(selected_index) = selected_index {
            self.list_state.select(Some(selected_index));
        }
    }

    pub(super) fn select_project_in_tree(&mut self, target_path: &str) {
        self.expand_path_in_tree(target_path);
        self.rows_dirty = true;
        self.select_matching_visible_row(target_path);
    }

    pub(super) fn update_search(&mut self, query: &str) {
        self.search_query = query.to_string();

        if query.is_empty() {
            self.filtered.clear();
            self.list_state.select(Some(0));
            return;
        }

        let mut matcher = Matcher::default();
        let atom = Atom::new(
            query,
            CaseMatching::Smart,
            Normalization::Smart,
            AtomKind::Fuzzy,
            false,
        );

        let mut scored: Vec<(usize, u16)> = self
            .flat_entries
            .iter()
            .enumerate()
            .filter_map(|(i, entry)| {
                let mut buf = Vec::new();
                let haystack = Utf32Str::new(&entry.name, &mut buf);
                atom.score(haystack, &mut matcher).map(|score| (i, score))
            })
            .collect();

        scored.sort_by(|a, b| b.1.cmp(&a.1));
        self.filtered = scored.into_iter().map(|(i, _)| i).collect();

        if self.filtered.is_empty() {
            self.list_state.select(None);
        } else {
            self.list_state.select(Some(0));
        }
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

    pub fn is_deleted(&self, path: &str) -> bool {
        self.deleted_projects.contains(path)
    }

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
            .chain(node.worktrees.iter().map(|worktree| worktree.project.path.clone()))
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

    pub fn ci_for(&self, project: &RustProject) -> Option<Conclusion> {
        self.ci_state
            .get(&project.path)
            .and_then(|s| s.runs().first())
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
            if let Some(state) = self.ci_state.get(path)
                && let Some(run) = state.runs().first()
            {
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
        self.ci_state.get(&project.path)
    }

    pub fn animation_elapsed(&self) -> Duration {
        self.animation_started.elapsed()
    }

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
mod tests {
    use std::collections::HashSet;
    use std::sync::OnceLock;
    use std::sync::mpsc;

    use chrono::DateTime;

    use super::*;
    use crate::config::Config;
    use crate::http::HttpClient;
    use crate::http::ServiceKind;
    use crate::port_report::LintStatus;
    use crate::project::ExampleGroup;
    use crate::project::ProjectLanguage;
    use crate::project::RustProject;
    use crate::project::WorkspaceStatus;
    use crate::scan::MemberGroup;
    use crate::scan::ProjectNode;

    fn test_http_client() -> HttpClient {
        static TEST_RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
        let rt = TEST_RT.get_or_init(|| {
            tokio::runtime::Runtime::new().unwrap_or_else(|_| std::process::abort())
        });
        HttpClient::new(rt.handle().clone()).unwrap_or_else(|| std::process::abort())
    }

    fn make_project(name: Option<&str>, path: &str) -> RustProject {
        RustProject {
            path: path.to_string(),
            abs_path: path.to_string(),
            name: name.map(String::from),
            version: None,
            description: None,
            worktree_name: None,
            worktree_primary_abs_path: None,
            is_workspace: WorkspaceStatus::Standalone,
            types: Vec::new(),
            examples: Vec::new(),
            benches: Vec::new(),
            test_count: 0,
            is_rust: ProjectLanguage::Rust,
            local_dependency_paths: Vec::new(),
        }
    }

    fn make_node(project: RustProject) -> ProjectNode {
        ProjectNode {
            project,
            groups: Vec::new(),
            worktrees: Vec::new(),
            vendored: Vec::new(),
        }
    }

    fn make_app(projects: Vec<RustProject>) -> App {
        make_app_with_config(projects, &Config::default())
    }

    fn make_app_with_config(projects: Vec<RustProject>, cfg: &Config) -> App {
        let (bg_tx, bg_rx) = mpsc::channel();
        let scan_root =
            std::env::temp_dir().join(format!("cargo-port-polish-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&scan_root);
        let mut app = App::new(
            scan_root,
            projects,
            bg_tx,
            bg_rx,
            cfg,
            test_http_client(),
            Instant::now(),
        );
        app.service_retry_spawns_enabled = false;
        app.sync_selected_project();
        app
    }

    fn make_non_rust_project(name: Option<&str>, path: &str) -> RustProject {
        let mut project = make_project(name, path);
        project.is_rust = ProjectLanguage::NonRust;
        project
    }

    fn wait_for_tree_build(app: &mut App) {
        for _ in 0..100 {
            let _ = app.poll_tree_builds();
            if app.tree_build_active.is_none() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        app.ensure_visible_rows_cached();
    }

    #[test]
    fn external_config_reload_applies_valid_changes() {
        let mut app = make_app(Vec::new());
        let dir = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
        let path = dir.path().join("config.toml");

        let mut cfg = Config::default();
        cfg.tui.editor = "helix".to_string();
        cfg.tui.ci_run_count = 9;
        cfg.mouse.invert_scroll = ScrollDirection::Normal;
        std::fs::write(
            &path,
            toml::to_string_pretty(&cfg).unwrap_or_else(|_| std::process::abort()),
        )
        .unwrap_or_else(|_| std::process::abort());

        app.config_path = Some(path);
        app.config_last_seen = None;
        app.maybe_reload_config_from_disk();

        assert_eq!(app.editor(), "helix");
        assert_eq!(app.ci_run_count(), 9);
        assert_eq!(app.invert_scroll(), ScrollDirection::Normal);
        assert_eq!(app.current_config.tui.editor, "helix");
        assert_eq!(app.current_config.tui.ci_run_count, 9);
    }

    #[test]
    fn external_config_reload_keeps_last_good_config_on_parse_error() {
        let mut app = make_app(Vec::new());
        let dir = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
        let path = dir.path().join("config.toml");

        let mut cfg = Config::default();
        cfg.tui.editor = "zed".to_string();
        std::fs::write(
            &path,
            toml::to_string_pretty(&cfg).unwrap_or_else(|_| std::process::abort()),
        )
        .unwrap_or_else(|_| std::process::abort());

        app.config_path = Some(path.clone());
        app.config_last_seen = None;
        app.maybe_reload_config_from_disk();

        std::fs::write(&path, "[tui\neditor = \"vim\"\n").unwrap_or_else(|_| std::process::abort());
        app.config_last_seen = None;
        app.maybe_reload_config_from_disk();

        assert_eq!(app.editor(), "zed");
        assert_eq!(app.current_config.tui.editor, "zed");
        assert!(
            app.status_flash
                .as_ref()
                .is_some_and(|(msg, _)| msg.contains("Config reload failed"))
        );
    }

    #[test]
    fn completed_scan_hides_and_restores_cached_non_rust_projects_without_rescan() {
        let rust_project = make_project(Some("rust"), "~/rust");
        let non_rust_project = make_non_rust_project(Some("js"), "~/js");
        let mut cfg = Config::default();
        cfg.tui.include_non_rust = NonRustInclusion::Include;
        let mut app =
            make_app_with_config(vec![rust_project.clone(), non_rust_project.clone()], &cfg);
        app.scan_complete = true;

        assert_eq!(app.all_projects.len(), 2);
        assert_eq!(app.nodes.len(), 2);

        let mut hide_cfg = cfg.clone();
        hide_cfg.tui.include_non_rust = NonRustInclusion::Exclude;
        app.apply_config(&hide_cfg);
        wait_for_tree_build(&mut app);

        assert_eq!(app.all_projects.len(), 2);
        assert!(app.scan_complete);
        assert_eq!(app.nodes.len(), 1);
        assert_eq!(app.nodes[0].project.path, rust_project.path);

        app.apply_config(&cfg);
        wait_for_tree_build(&mut app);

        assert_eq!(app.all_projects.len(), 2);
        assert!(app.scan_complete);
        assert_eq!(app.nodes.len(), 2);
        assert!(
            app.nodes
                .iter()
                .any(|node| node.project.path == non_rust_project.path)
        );
    }

    #[test]
    fn completed_scan_rescans_when_enabling_non_rust_without_cached_projects() {
        let rust_project = make_project(Some("rust"), "~/rust");
        let mut app = make_app(vec![rust_project]);
        app.scan_complete = true;

        let mut cfg = app.current_config.clone();
        cfg.tui.include_non_rust = NonRustInclusion::Include;
        app.apply_config(&cfg);

        assert!(app.all_projects.is_empty());
        assert!(!app.scan_complete);
    }

    fn apply_nodes(app: &mut App, nodes: Vec<ProjectNode>) {
        let flat_entries = crate::scan::build_flat_entries(&nodes);
        app.apply_tree_build(nodes, flat_entries);
        app.ensure_visible_rows_cached();
    }

    fn parse_ts(ts: &str) -> DateTime<chrono::FixedOffset> {
        DateTime::parse_from_rfc3339(ts).unwrap_or_else(|_| std::process::abort())
    }

    #[test]
    fn service_reachability_tracks_background_messages() {
        let mut app = make_app(Vec::new());

        assert!(app.unreachable_services.is_empty());
        assert!(app.unreachable_service_message().is_none());

        assert!(!app.handle_bg_msg(BackgroundMsg::ServiceUnreachable {
            service: ServiceKind::GitHub,
        }));
        assert!(app.unreachable_services.contains(&ServiceKind::GitHub));
        assert_eq!(
            app.unreachable_service_message().as_deref(),
            Some(" GitHub unreachable ")
        );

        assert!(!app.handle_bg_msg(BackgroundMsg::ServiceUnreachable {
            service: ServiceKind::CratesIo,
        }));
        assert!(app.unreachable_services.contains(&ServiceKind::CratesIo));
        assert_eq!(
            app.unreachable_service_message().as_deref(),
            Some(" GitHub and crates.io unreachable ")
        );

        assert!(!app.handle_bg_msg(BackgroundMsg::ServiceReachable {
            service: ServiceKind::GitHub,
        }));
        assert!(!app.unreachable_services.contains(&ServiceKind::GitHub));
        assert_eq!(
            app.unreachable_service_message().as_deref(),
            Some(" crates.io unreachable ")
        );

        assert!(!app.handle_bg_msg(BackgroundMsg::ServiceReachable {
            service: ServiceKind::CratesIo,
        }));
        assert!(app.unreachable_services.is_empty());
        assert!(app.unreachable_service_message().is_none());
    }

    #[test]
    fn visible_rows_workspace_with_worktrees() {
        // A workspace whose groups have been moved to worktree entries
        let mut root = make_node(make_project(None, "~/ws"));
        let member_a = make_project(Some("a"), "~/ws/a");
        let member_b = make_project(Some("b"), "~/ws/b");

        // Primary-as-worktree with inline members
        let mut wt0 = make_node(make_project(None, "~/ws"));
        wt0.project.worktree_name = Some("ws".to_string());
        wt0.groups = vec![MemberGroup {
            name: String::new(),
            members: vec![member_a.clone(), member_b.clone()],
        }];

        // Actual worktree with a named group
        let mut wt1 = make_node(make_project(None, "~/ws_feat"));
        wt1.project.worktree_name = Some("ws_feat".to_string());
        wt1.groups = vec![MemberGroup {
            name: "crates".to_string(),
            members: vec![member_a, member_b],
        }];

        root.worktrees = vec![wt0, wt1];

        // Expand everything: node, both worktrees, and the named group
        let expanded: HashSet<ExpandKey> = [
            ExpandKey::Node(0),
            ExpandKey::Worktree(0, 0),
            ExpandKey::Worktree(0, 1),
            ExpandKey::WorktreeGroup(0, 1, 0),
        ]
        .into();

        let rows = build_visible_rows(&[root], &expanded);

        // Expected:
        // 0: Root(0)
        // 1: WorktreeEntry(0, 0)
        // 2: WorktreeMember(0, 0, 0, 0)  — inline member a
        // 3: WorktreeMember(0, 0, 0, 1)  — inline member b
        // 4: WorktreeEntry(0, 1)
        // 5: WorktreeGroupHeader(0, 1, 0) — "crates"
        // 6: WorktreeMember(0, 1, 0, 0)
        // 7: WorktreeMember(0, 1, 0, 1)
        assert_eq!(rows.len(), 8, "expected 8 rows, got: {rows:?}");
        assert!(matches!(rows[0], VisibleRow::Root { node_index: 0 }));
        assert!(matches!(
            rows[1],
            VisibleRow::WorktreeEntry {
                node_index: 0,
                worktree_index: 0,
            }
        ));
        assert!(matches!(
            rows[2],
            VisibleRow::WorktreeMember {
                node_index: 0,
                worktree_index: 0,
                group_index: 0,
                member_index: 0,
            }
        ));
        assert!(matches!(
            rows[4],
            VisibleRow::WorktreeEntry {
                node_index: 0,
                worktree_index: 1,
            }
        ));
        assert!(matches!(
            rows[5],
            VisibleRow::WorktreeGroupHeader {
                node_index: 0,
                worktree_index: 1,
                group_index: 0,
            }
        ));
        assert!(matches!(
            rows[7],
            VisibleRow::WorktreeMember {
                node_index: 0,
                worktree_index: 1,
                group_index: 0,
                member_index: 1,
            }
        ));
    }

    #[test]
    fn visible_rows_non_workspace_worktrees() {
        let build_root = || {
            let mut root = make_node(make_project(Some("app"), "~/app"));
            let mut wt0 = make_node(make_project(Some("app"), "~/app"));
            wt0.project.worktree_name = Some("app".to_string());
            let mut wt1 = make_node(make_project(Some("app"), "~/app_feat"));
            wt1.project.worktree_name = Some("app_feat".to_string());
            root.worktrees = vec![wt0, wt1];
            root
        };

        // Standalone project with worktrees — flat, not expandable
        let expanded: HashSet<ExpandKey> = [ExpandKey::Node(0)].into();
        let rows = build_visible_rows(&[build_root()], &expanded);

        // Root + 2 flat worktree entries
        assert_eq!(rows.len(), 3, "got: {rows:?}");
        assert!(matches!(rows[0], VisibleRow::Root { .. }));
        assert!(matches!(rows[1], VisibleRow::WorktreeEntry { .. }));
        assert!(matches!(rows[2], VisibleRow::WorktreeEntry { .. }));

        // Expanding a worktree with no groups produces no additional rows
        let expanded2: HashSet<ExpandKey> = [ExpandKey::Node(0), ExpandKey::Worktree(0, 0)].into();
        let rows2 = build_visible_rows(&[build_root()], &expanded2);
        assert_eq!(rows2.len(), 3, "no extra rows for non-workspace worktree");
    }

    #[test]
    fn visible_rows_workspace_no_worktrees() {
        // Workspace with groups, no worktrees — regression test
        let member_a = make_project(Some("a"), "~/ws/a");
        let member_b = make_project(Some("b"), "~/ws/b");
        let mut root = make_node(make_project(None, "~/ws"));
        root.groups = vec![MemberGroup {
            name: String::new(),
            members: vec![member_a, member_b],
        }];

        let expanded: HashSet<ExpandKey> = [ExpandKey::Node(0)].into();
        let rows = build_visible_rows(&[root], &expanded);

        // Root + 2 inline members
        assert_eq!(rows.len(), 3, "got: {rows:?}");
        assert!(matches!(rows[0], VisibleRow::Root { .. }));
        assert!(matches!(
            rows[1],
            VisibleRow::Member {
                member_index: 0,
                ..
            }
        ));
        assert!(matches!(
            rows[2],
            VisibleRow::Member {
                member_index: 1,
                ..
            }
        ));
    }

    #[test]
    fn visible_rows_include_vendored_children() {
        let member = make_project(Some("member"), "~/ws/member");
        let vendored = make_project(Some("vendored"), "~/ws/vendor/helper");
        let mut root = make_node(make_project(None, "~/ws"));
        root.groups = vec![MemberGroup {
            name: String::new(),
            members: vec![member],
        }];
        root.vendored = vec![vendored];

        let expanded: HashSet<ExpandKey> = [ExpandKey::Node(0)].into();
        let rows = build_visible_rows(&[root], &expanded);

        assert_eq!(rows.len(), 3, "got: {rows:?}");
        assert!(matches!(rows[0], VisibleRow::Root { .. }));
        assert!(matches!(rows[1], VisibleRow::Member { .. }));
        assert!(matches!(
            rows[2],
            VisibleRow::Vendored {
                node_index: 0,
                vendored_index: 0,
            }
        ));
    }

    #[test]
    fn lint_runtime_waits_for_scan_completion() {
        let project = make_project(Some("demo"), "~/demo");
        let path = project.path.clone();
        let mut app = make_app(vec![project]);

        assert!(app.lint_runtime_projects_snapshot().is_empty());

        app.scan_complete = true;
        let projects = app.lint_runtime_projects_snapshot();
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0].project_path, path);
    }

    #[test]
    fn startup_lint_expectation_tracks_running_startup_lints() {
        let project_a = make_project(Some("a"), "~/a");
        let project_b = make_project(Some("b"), "~/b");
        let mut app = make_app(vec![project_a.clone(), project_b]);
        app.scan_complete = true;

        app.initialize_startup_phase_tracker();

        let expected = app
            .startup_phases
            .lint_expected
            .as_ref()
            .expect("lint expected");
        assert!(expected.is_empty());
        assert!(app.lint_toast.is_none());

        app.handle_bg_msg(BackgroundMsg::LintStatus {
            path: project_a.path.clone(),
            status: LintStatus::Running(parse_ts("2026-03-30T14:22:18-05:00")),
        });

        let expected = app
            .startup_phases
            .lint_expected
            .as_ref()
            .expect("lint expected");
        assert_eq!(expected.len(), 1);
        assert!(expected.contains(&project_a.path));
        assert!(
            !app.startup_phases
                .lint_seen_terminal
                .contains(&project_a.path)
        );
        assert!(app.running_lint_paths.contains(&project_a.path));
        assert!(app.lint_toast.is_some());

        app.handle_bg_msg(BackgroundMsg::LintStatus {
            path: project_a.path,
            status: LintStatus::Passed(parse_ts("2026-03-30T14:23:18-05:00")),
        });

        assert!(app.startup_phases.lint_complete_at.is_some());
        assert!(app.running_lint_paths.is_empty());
        assert!(app.lint_toast.is_none());
    }

    #[test]
    fn startup_lint_toast_body_shows_two_paths_then_others() {
        let expected = HashSet::from([
            "~/a".to_string(),
            "~/b".to_string(),
            "~/c".to_string(),
            "~/d".to_string(),
        ]);
        let seen = HashSet::from(["~/d".to_string()]);

        let body = App::startup_lint_toast_body_for(&expected, &seen);
        let lines = body.lines().collect::<Vec<_>>();

        assert_eq!(lines.len(), 2);
        assert!(lines[0].starts_with("~/"));
        assert!(lines[1].starts_with("~/"));
        assert!(lines[1].contains("(+ 1 others)"));
    }

    #[test]
    fn lint_toast_reappears_for_new_running_lints() {
        let project = make_project(Some("a"), "~/a");
        let mut app = make_app(vec![project.clone()]);
        app.scan_complete = true;

        app.handle_bg_msg(BackgroundMsg::LintStatus {
            path: project.path.clone(),
            status: LintStatus::Running(parse_ts("2026-03-30T14:22:18-05:00")),
        });
        let first_toast = app.lint_toast;
        assert!(first_toast.is_some());

        app.handle_bg_msg(BackgroundMsg::LintStatus {
            path: project.path.clone(),
            status: LintStatus::Passed(parse_ts("2026-03-30T14:23:18-05:00")),
        });
        assert!(app.lint_toast.is_none());

        app.handle_bg_msg(BackgroundMsg::LintStatus {
            path: project.path,
            status: LintStatus::Running(parse_ts("2026-03-30T14:24:18-05:00")),
        });
        assert!(app.lint_toast.is_some());
        assert_ne!(app.lint_toast, first_toast);
    }

    #[test]
    fn collapse_all_anchors_member_selection_to_root() {
        let workspace = make_project(Some("hana"), "~/rust/hana");
        let member = make_project(Some("hana_core"), "~/rust/hana/crates/hana_core");

        let mut root = make_node(workspace.clone());
        root.groups = vec![MemberGroup {
            name: String::new(),
            members: vec![member.clone()],
        }];

        let mut app = make_app(vec![workspace, member.clone()]);
        apply_nodes(&mut app, vec![root]);
        app.expanded.insert(ExpandKey::Node(0));
        app.rows_dirty = true;
        app.select_project_in_tree(&member.path);

        app.collapse_all();

        assert_eq!(app.selected_row(), Some(VisibleRow::Root { node_index: 0 }));
    }

    #[test]
    fn expand_all_preserves_selected_project_path() {
        let workspace = make_project(Some("hana"), "~/rust/hana");
        let member = make_project(Some("hana_core"), "~/rust/hana/crates/hana_core");

        let mut root = make_node(workspace.clone());
        root.groups = vec![MemberGroup {
            name: String::new(),
            members: vec![member.clone()],
        }];

        let mut app = make_app(vec![workspace, member.clone()]);
        apply_nodes(&mut app, vec![root]);
        app.select_project_in_tree(&member.path);
        app.collapse_all();

        app.expand_all();

        assert_eq!(
            app.selected_project().map(|project| project.path.as_str()),
            Some(member.path.as_str())
        );
    }

    #[test]
    fn lint_runtime_snapshot_uses_workspace_root_not_members() {
        let mut workspace = make_project(Some("hana"), "~/rust/hana");
        workspace.is_workspace = WorkspaceStatus::Workspace;
        let member_a = make_project(Some("hana_core"), "~/rust/hana/crates/hana_core");
        let member_b = make_project(Some("hana_ui"), "~/rust/hana/crates/hana_ui");

        let mut root = make_node(workspace.clone());
        root.groups = vec![MemberGroup {
            name: String::new(),
            members: vec![member_a.clone(), member_b.clone()],
        }];

        let mut app = make_app(vec![workspace.clone(), member_a, member_b]);
        apply_nodes(&mut app, vec![root]);
        app.scan_complete = true;

        let projects = app.lint_runtime_projects_snapshot();
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0].project_path, workspace.path);
    }

    #[test]
    fn lint_runtime_snapshot_deduplicates_primary_worktree_path() {
        let root_project = make_project(Some("ws"), "~/ws");
        let mut root = make_node(root_project.clone());

        let mut primary = make_node(root_project.clone());
        primary.project.worktree_name = Some("ws".to_string());

        let mut feature = make_node(make_project(Some("ws_feat"), "~/ws_feat"));
        feature.project.worktree_name = Some("ws_feat".to_string());

        root.worktrees = vec![primary, feature.clone()];

        let mut app = make_app(vec![root_project.clone(), feature.project.clone()]);
        apply_nodes(&mut app, vec![root]);
        app.scan_complete = true;

        let projects = app.lint_runtime_projects_snapshot();
        assert_eq!(projects.len(), 2);
        assert_eq!(projects[0].project_path, root_project.path);
        assert_eq!(projects[1].project_path, feature.project.path);
    }

    #[test]
    fn vendored_path_dependency_becomes_cargo_active() {
        let mut root_project = make_project(Some("app"), "~/app");
        let vendored = make_project(Some("helper"), "~/app/vendor/helper");
        root_project.local_dependency_paths = vec![vendored.path.clone()];

        let mut root = make_node(root_project.clone());
        root.vendored = vec![vendored.clone()];

        let mut app = make_app(vec![root_project, vendored.clone()]);
        apply_nodes(&mut app, vec![root]);

        assert!(app.is_vendored_path(&vendored.path));
        assert!(app.is_cargo_active_path(&vendored.path));
    }

    #[test]
    fn git_path_state_suppresses_sync_for_untracked_and_ignored() {
        let project = make_project(Some("demo"), "~/demo");
        let path = project.path.clone();
        let mut app = make_app(vec![project.clone()]);

        app.git_info.insert(
            path.clone(),
            GitInfo {
                origin: GitOrigin::Clone,
                branch: Some("feat/demo".to_string()),
                owner: None,
                url: Some("https://github.com/acme/demo".to_string()),
                first_commit: None,
                last_commit: None,
                ahead_behind: Some((2, 0)),
                default_branch: Some("main".to_string()),
                ahead_behind_origin: None,
                ahead_behind_local: None,
            },
        );

        app.git_path_states
            .insert(path.clone(), GitPathState::Untracked);
        assert!(app.git_sync(&project).is_empty());

        app.git_path_states.insert(path, GitPathState::Ignored);
        assert!(app.git_sync(&project).is_empty());
    }

    #[test]
    fn name_width_with_gutter_reserves_space_before_lint() {
        assert_eq!(App::name_width_with_gutter(0), 1);
        assert_eq!(App::name_width_with_gutter(42), 43);
    }

    #[test]
    fn tabbable_panes_follow_canonical_order() {
        let mut project = make_project(Some("demo"), "~/demo");
        project.examples = vec![ExampleGroup {
            category: String::new(),
            names: vec!["example".to_string()],
        }];

        let mut app = make_app(vec![project.clone()]);
        app.toasts = ToastManager::default();
        app.toast_pane.set_len(0);
        app.scan_complete = true;
        app.git_info.insert(
            project.path,
            GitInfo {
                origin: GitOrigin::Clone,
                branch: None,
                owner: None,
                url: Some("https://github.com/acme/demo".to_string()),
                first_commit: None,
                last_commit: None,
                ahead_behind: None,
                default_branch: None,
                ahead_behind_origin: None,
                ahead_behind_local: None,
            },
        );

        assert_eq!(
            app.tabbable_panes(),
            vec![
                PaneId::ProjectList,
                PaneId::Package,
                PaneId::Git,
                PaneId::Targets,
                PaneId::CiRuns,
            ]
        );

        app.show_timed_toast("Settings", "Updated");
        assert_eq!(
            app.tabbable_panes(),
            vec![
                PaneId::ProjectList,
                PaneId::Package,
                PaneId::Git,
                PaneId::Targets,
                PaneId::CiRuns,
                PaneId::Toasts,
            ]
        );

        app.focus_next_pane();
        assert_eq!(app.focused_pane, PaneId::Package);
        app.focus_next_pane();
        assert_eq!(app.focused_pane, PaneId::Git);
        app.focus_next_pane();
        assert_eq!(app.focused_pane, PaneId::Targets);
        app.focus_next_pane();
        assert_eq!(app.focused_pane, PaneId::CiRuns);
        app.focus_next_pane();
        assert_eq!(app.focused_pane, PaneId::Toasts);
        app.focus_previous_pane();
        assert_eq!(app.focused_pane, PaneId::CiRuns);
    }

    #[test]
    fn new_toasts_do_not_steal_focus() {
        let project = make_project(Some("demo"), "~/demo");
        let mut app = make_app(vec![project]);
        app.focus_pane(PaneId::Git);

        app.show_timed_toast("Settings", "Updated");
        assert_eq!(app.focused_pane, PaneId::Git);

        let _task = app.start_task_toast("Startup lints", "Running startup lint jobs...");
        assert_eq!(app.focused_pane, PaneId::Git);
    }

    #[test]
    fn project_refresh_updates_selected_tree_project_targets() {
        let project = make_project(Some("demo"), "~/demo");
        let mut app = make_app(vec![project.clone()]);
        app.scan_complete = true;
        app.list_state.select(Some(0));
        app.sync_selected_project();

        assert_eq!(
            app.selected_project().map(RustProject::example_count),
            Some(0)
        );
        assert!(!app.tabbable_panes().contains(&PaneId::Targets));

        let mut refreshed = project;
        refreshed.examples = vec![ExampleGroup {
            category: String::new(),
            names: vec!["tracked_row_paths".to_string()],
        }];

        assert!(app.handle_project_refreshed(&refreshed));
        app.sync_selected_project();

        assert_eq!(
            app.selected_project().map(RustProject::example_count),
            Some(1)
        );
        assert!(app.tabbable_panes().contains(&PaneId::Targets));
    }

    #[test]
    fn first_non_empty_tree_build_focuses_project_list() {
        let project = make_project(Some("demo"), "~/demo");
        let mut app = make_app(vec![project.clone()]);
        app.focus_pane(PaneId::ScanLog);

        apply_nodes(&mut app, vec![make_node(project)]);

        assert_eq!(app.focused_pane, PaneId::ProjectList);
        assert_eq!(app.list_state.selected(), Some(0));
    }

    #[test]
    fn initial_disk_batch_count_groups_nested_projects_under_one_root() {
        let projects = vec![
            make_project(Some("bevy"), "~/rust/bevy"),
            make_project(Some("ecs"), "~/rust/bevy/crates/bevy_ecs"),
            make_project(Some("render"), "~/rust/bevy/crates/bevy_render"),
            make_project(Some("hana"), "~/rust/hana"),
            make_project(Some("hana_core"), "~/rust/hana/crates/hana"),
        ];

        assert_eq!(initial_disk_batch_count(&projects), 2);
    }

    #[test]
    fn overlays_restore_prior_focus() {
        let app_project = make_project(Some("demo"), "~/demo");
        let mut app = make_app(vec![app_project]);
        app.focus_pane(PaneId::Git);

        app.open_overlay(PaneId::Settings);
        app.show_settings = true;
        assert_eq!(app.focused_pane, PaneId::Settings);
        assert_eq!(app.return_focus, Some(PaneId::Git));

        app.show_settings = false;
        app.close_overlay();
        assert_eq!(app.focused_pane, PaneId::Git);
        assert!(app.return_focus.is_none());
    }

    #[test]
    fn detail_panes_do_not_remember_selection_until_focused() {
        let project = make_project(Some("demo"), "~/demo");
        let mut app = make_app(vec![project]);

        assert!(app.remembers_selection(PaneId::ProjectList));
        assert!(!app.remembers_selection(PaneId::Package));
        assert!(!app.remembers_selection(PaneId::Git));
        assert!(!app.remembers_selection(PaneId::Targets));
        assert!(!app.remembers_selection(PaneId::CiRuns));

        app.focus_pane(PaneId::Package);
        assert!(app.remembers_selection(PaneId::Package));
    }

    #[test]
    fn project_change_resets_project_dependent_panes() {
        let project_a = make_project(Some("a"), "~/a");
        let project_b = make_project(Some("b"), "~/b");
        let mut app = make_app(vec![project_a, project_b]);

        app.focus_pane(PaneId::Package);
        app.focus_pane(PaneId::Git);
        app.focus_pane(PaneId::Targets);
        app.focus_pane(PaneId::CiRuns);
        app.package_pane.set_pos(3);
        app.git_pane.set_pos(4);
        app.targets_pane.set_pos(5);
        app.ci_pane.set_pos(6);

        app.list_state.select(Some(1));
        app.sync_selected_project();

        assert_eq!(app.package_pane.pos(), 0);
        assert_eq!(app.git_pane.pos(), 0);
        assert_eq!(app.targets_pane.pos(), 0);
        assert_eq!(app.ci_pane.pos(), 0);
        assert!(!app.remembers_selection(PaneId::Package));
        assert!(!app.remembers_selection(PaneId::Git));
        assert!(!app.remembers_selection(PaneId::Targets));
        assert!(!app.remembers_selection(PaneId::CiRuns));
        assert_eq!(app.selected_project_path.as_deref(), Some("~/b"));
    }

    #[test]
    fn apply_config_resets_column_layout_flag() {
        let mut app = make_app(vec![make_project(Some("demo"), "~/demo")]);
        let mut cfg = Config::default();

        assert!(!app.cached_fit_widths.lint_enabled());

        cfg.lint.enabled = true;
        app.apply_config(&cfg);
        assert!(app.cached_fit_widths.lint_enabled());

        cfg.lint.enabled = false;
        app.apply_config(&cfg);
        assert!(!app.cached_fit_widths.lint_enabled());
    }

    #[test]
    fn zero_byte_update_marks_deleted_child_member() {
        let tmp = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
        let workspace_dir = tmp.path().join("hana");
        let member_dir = workspace_dir.join("crates").join("clay-layout");
        std::fs::create_dir_all(&member_dir).unwrap_or_else(|_| std::process::abort());

        let mut workspace = make_project(Some("hana"), "~/rust/hana");
        workspace.abs_path = workspace_dir.to_string_lossy().to_string();
        workspace.is_workspace = WorkspaceStatus::Workspace;

        let mut member = make_project(Some("clay-layout"), "~/rust/hana/crates/clay-layout");
        member.abs_path = member_dir.to_string_lossy().to_string();

        let mut root = make_node(workspace.clone());
        root.groups = vec![MemberGroup {
            name: String::new(),
            members: vec![member.clone()],
        }];

        let mut app = make_app(vec![workspace, member.clone()]);
        apply_nodes(&mut app, vec![root]);

        std::fs::remove_dir_all(&member_dir).unwrap_or_else(|_| std::process::abort());
        app.handle_disk_usage(member.path.clone(), 0);

        assert!(app.deleted_projects.contains(&member.path));
    }

    #[test]
    fn disk_updates_skip_git_path_refresh_during_scan() {
        let tmp = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
        let abs_path = tmp.path().join("demo");
        std::fs::create_dir_all(&abs_path).unwrap_or_else(|_| std::process::abort());

        let mut project = make_project(Some("demo"), "~/demo");
        project.abs_path = abs_path.to_string_lossy().to_string();
        let path = project.path.clone();
        let mut app = make_app(vec![project]);

        app.handle_disk_usage(path.clone(), 123);
        assert!(!app.git_path_states.contains_key(&path));

        app.scan_complete = true;
        app.handle_disk_usage(path.clone(), 123);
        assert_eq!(
            app.git_path_states.get(&path),
            Some(&GitPathState::OutsideRepo)
        );
    }

    #[test]
    fn bottom_panel_changes_input_context_for_lower_pane() {
        let mut app = make_app(vec![make_project(Some("demo"), "~/demo")]);
        app.focus_pane(PaneId::CiRuns);
        assert_eq!(app.input_context(), InputContext::CiRuns);

        app.toggle_bottom_panel();
        assert_eq!(app.input_context(), InputContext::PortReport);
    }

    #[test]
    fn lint_rollups_distinguish_root_from_primary_worktree() {
        let mut root = make_node(make_project(None, "~/ws"));
        let mut primary = make_node(make_project(None, "~/ws"));
        primary.project.worktree_name = Some("ws".to_string());

        let mut feature = make_node(make_project(None, "~/ws_feat"));
        feature.project.worktree_name = Some("ws_feat".to_string());

        root.worktrees = vec![primary, feature];

        let mut app = make_app(vec![root.project.clone()]);
        app.current_config.lint.enabled = true;
        apply_nodes(&mut app, vec![root]);
        app.lint_status.insert(
            "~/ws".to_string(),
            LintStatus::Passed(parse_ts("2026-03-30T14:22:18-05:00")),
        );
        app.lint_status.insert(
            "~/ws_feat".to_string(),
            LintStatus::Failed(parse_ts("2026-03-30T15:22:18-05:00")),
        );
        app.rebuild_lint_rollups();

        assert!(matches!(
            app.lint_status_for_rollup_key(LintRollupKey::Root { node_index: 0 }),
            Some(LintStatus::Failed(_))
        ));
        assert!(matches!(
            app.lint_status_for_rollup_key(LintRollupKey::Worktree {
                node_index: 0,
                worktree_index: 0,
            }),
            Some(LintStatus::Passed(_))
        ));
        assert!(matches!(
            app.lint_status_for_rollup_key(LintRollupKey::Worktree {
                node_index: 0,
                worktree_index: 1,
            }),
            Some(LintStatus::Failed(_))
        ));
    }

    #[test]
    fn lint_rollup_prefers_running_root_over_member_history() {
        let mut root = make_node(make_project(None, "~/ws"));
        let member = make_project(Some("a"), "~/ws/a");

        root.groups = vec![MemberGroup {
            name: String::new(),
            members: vec![member],
        }];

        let mut app = make_app(vec![root.project.clone()]);
        app.current_config.lint.enabled = true;
        apply_nodes(&mut app, vec![root]);
        app.lint_status.insert(
            "~/ws".to_string(),
            LintStatus::Running(parse_ts("2026-03-30T16:22:18-05:00")),
        );
        app.lint_status.insert(
            "~/ws/a".to_string(),
            LintStatus::Failed(parse_ts("2026-03-30T15:22:18-05:00")),
        );
        app.rebuild_lint_rollups();

        assert!(matches!(
            app.lint_status_for_rollup_key(LintRollupKey::Root { node_index: 0 }),
            Some(LintStatus::Running(_))
        ));
    }

    #[test]
    fn lint_rollup_prefers_running_worktree_over_failed_root_history() {
        let mut root = make_node(make_project(None, "~/ws"));
        let mut primary = make_node(make_project(None, "~/ws"));
        primary.project.worktree_name = Some("ws".to_string());

        let mut feature = make_node(make_project(None, "~/ws_feat"));
        feature.project.worktree_name = Some("ws_feat".to_string());

        root.worktrees = vec![primary, feature];

        let mut app = make_app(vec![root.project.clone()]);
        app.current_config.lint.enabled = true;
        apply_nodes(&mut app, vec![root]);
        app.lint_status.insert(
            "~/ws".to_string(),
            LintStatus::Failed(parse_ts("2026-03-30T15:22:18-05:00")),
        );
        app.lint_status.insert(
            "~/ws_feat".to_string(),
            LintStatus::Running(parse_ts("2026-03-30T16:22:18-05:00")),
        );
        app.rebuild_lint_rollups();

        assert!(matches!(
            app.lint_status_for_rollup_key(LintRollupKey::Root { node_index: 0 }),
            Some(LintStatus::Running(_))
        ));
        assert!(matches!(
            app.lint_status_for_rollup_key(LintRollupKey::Worktree {
                node_index: 0,
                worktree_index: 1,
            }),
            Some(LintStatus::Running(_))
        ));
    }

    #[test]
    fn detail_cache_separates_root_and_worktree_rows_with_same_path() {
        let mut root = make_node(make_project(None, "~/ws"));
        let member_a = make_project(Some("a"), "~/ws/a");
        let member_b = make_project(Some("b"), "~/ws_feat/b");

        let mut primary = make_node(make_project(None, "~/ws"));
        primary.project.worktree_name = Some("ws".to_string());
        primary.groups = vec![MemberGroup {
            name: String::new(),
            members: vec![member_a],
        }];

        let mut feature = make_node(make_project(None, "~/ws_feat"));
        feature.project.worktree_name = Some("ws_feat".to_string());
        feature.groups = vec![MemberGroup {
            name: String::new(),
            members: vec![member_b],
        }];

        root.worktrees = vec![primary, feature];

        let mut app = make_app(vec![root.project.clone()]);
        app.current_config.lint.enabled = true;
        apply_nodes(&mut app, vec![root]);
        app.expanded.insert(ExpandKey::Node(0));
        app.rows_dirty = true;
        app.ensure_visible_rows_cached();

        app.lint_status.insert(
            "~/ws".to_string(),
            LintStatus::Passed(parse_ts("2026-03-30T14:22:18-05:00")),
        );
        app.lint_status.insert(
            "~/ws_feat".to_string(),
            LintStatus::Failed(parse_ts("2026-03-30T15:22:18-05:00")),
        );
        app.rebuild_lint_rollups();

        app.list_state.select(Some(0));
        app.sync_selected_project();
        app.ensure_detail_cached();
        assert_eq!(
            app.cached_detail
                .as_ref()
                .map(|cache| cache.info.lint_label.as_str()),
            Some("🔴")
        );

        app.list_state.select(Some(1));
        app.sync_selected_project();
        app.ensure_detail_cached();
        assert_eq!(
            app.cached_detail
                .as_ref()
                .map(|cache| cache.info.lint_label.as_str()),
            Some("🟢")
        );
    }

    #[test]
    fn disk_rollup_deduplicates_primary_worktree_path() {
        let mut root = make_node(make_project(None, "~/ws"));
        let mut primary = make_node(make_project(None, "~/ws"));
        primary.project.worktree_name = Some("ws".to_string());
        let mut feature = make_node(make_project(None, "~/ws_feat"));
        feature.project.worktree_name = Some("ws_feat".to_string());
        root.worktrees = vec![primary, feature];

        let mut app = make_app(vec![root.project.clone()]);
        apply_nodes(&mut app, vec![root.clone()]);
        app.disk_usage.insert("~/ws".to_string(), 15);
        app.disk_usage.insert("~/ws_feat".to_string(), 21);

        assert_eq!(app.disk_bytes_for_node(&root), Some(36));
        assert_eq!(
            disk_bytes_for_node_snapshot(&root, &app.disk_usage),
            Some(36)
        );
        assert_eq!(
            app.formatted_disk_for_node(&root),
            crate::tui::render::format_bytes(36)
        );
    }
}
