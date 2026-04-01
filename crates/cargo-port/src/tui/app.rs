use std::collections::HashMap;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::mpsc;
use std::sync::mpsc::Receiver;
use std::time::Duration;
use std::time::Instant;

use nucleo_matcher::Matcher;
use nucleo_matcher::Utf32Str;
use nucleo_matcher::pattern::Atom;
use nucleo_matcher::pattern::AtomKind;
use nucleo_matcher::pattern::CaseMatching;
use nucleo_matcher::pattern::Normalization;
use ratatui::widgets::ListState;

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
use super::render::PREFIX_WT_COLLAPSED;
use super::render::PREFIX_WT_FLAT;
use super::render::PREFIX_WT_GROUP_COLLAPSED;
use super::render::PREFIX_WT_MEMBER_INLINE;
use super::render::PREFIX_WT_MEMBER_NAMED;
use super::shortcuts::InputContext;
use super::terminal::CiFetchMsg;
use super::terminal::ExampleMsg;
use super::types::LayoutCache;
use super::types::Pane;
use super::types::PaneId;
use crate::ci;
use crate::ci::CiRun;
use crate::ci::Conclusion;
use crate::config::Config;
use crate::config::NonRustInclusion;
use crate::config::ScrollDirection;
use crate::constants::IN_SYNC;
use crate::constants::SYNC_DOWN;
use crate::constants::SYNC_UP;
use crate::constants::WORKTREE;
use crate::http::HttpClient;
use crate::port_report::LintStatus;
use crate::project::GitInfo;
use crate::project::GitOrigin;
use crate::project::GitTracking;
use crate::project::RustProject;
use crate::scan;
use crate::scan::BackgroundMsg;
use crate::scan::CiFetchResult;
use crate::scan::FlatEntry;
use crate::scan::ProjectNode;
use crate::watcher;
use crate::watcher::WatchRequest;

/// Whether the application has network connectivity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum NetworkStatus {
    Online,
    Offline,
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

pub(super) use super::columns::ResolvedWidths;

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

#[derive(Default)]
pub(super) struct PollBackgroundStats {
    pub bg_msgs:       usize,
    pub ci_msgs:       usize,
    pub example_msgs:  usize,
    pub tree_results:  usize,
    pub fit_results:   usize,
    pub disk_results:  usize,
    pub needs_rebuild: bool,
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
    path:       String,
    pub info:   DetailInfo,
}

#[allow(
    clippy::struct_excessive_bools,
    reason = "independent UI state toggles"
)]
pub(super) struct App {
    pub scan_root:             PathBuf,
    pub inline_dirs:           Vec<String>,
    pub include_dirs:          Vec<String>,
    pub http_client:           HttpClient,
    pub ci_run_count:          u32,
    pub include_non_rust:      NonRustInclusion,
    pub editor:                String,
    pub status_flash_millis:   u64,
    pub all_projects:          Vec<RustProject>,
    pub nodes:                 Vec<ProjectNode>,
    pub flat_entries:          Vec<FlatEntry>,
    pub disk_usage:            HashMap<String, u64>,
    pub ci_state:              HashMap<String, CiState>,
    pub lint_status:           HashMap<String, LintStatus>,
    pub lint_enabled:          bool,
    pub git_info:              HashMap<String, GitInfo>,
    pub crates_versions:       HashMap<String, String>,
    pub crates_downloads:      HashMap<String, u64>,
    pub stars:                 HashMap<String, u64>,
    pub repo_descriptions:     HashMap<String, String>,
    pub bg_tx:                 mpsc::Sender<BackgroundMsg>,
    pub bg_rx:                 Receiver<BackgroundMsg>,
    pub fully_loaded:          HashSet<String>,
    pub priority_fetch_path:   Option<String>,
    pub invert_scroll:         ScrollDirection,
    pub expanded:              HashSet<ExpandKey>,
    pub list_state:            ListState,
    pub searching:             bool,
    pub search_query:          String,
    pub filtered:              Vec<usize>,
    pub show_settings:         bool,
    pub settings_pane:         Pane,
    pub settings_editing:      bool,
    pub settings_edit_buf:     String,
    pub scan_complete:         bool,
    pub scan_log:              Vec<String>,
    pub scan_log_state:        ListState,
    pub focused_pane:          PaneId,
    pub return_focus:          Option<PaneId>,
    pub visited_panes:         HashSet<PaneId>,
    pub package_pane:          Pane,
    pub git_pane:              Pane,
    pub targets_pane:          Pane,
    pub ci_pane:               Pane,
    pub pending_example_run:   Option<PendingExampleRun>,
    pub pending_ci_fetch:      Option<PendingCiFetch>,
    pub pending_clean:         Option<String>,
    pub confirm:               Option<ConfirmAction>,
    pub animation_started:     Instant,
    pub ci_fetch_tx:           mpsc::Sender<CiFetchMsg>,
    pub ci_fetch_rx:           mpsc::Receiver<CiFetchMsg>,
    pub example_running:       Option<String>,
    pub example_child:         Arc<Mutex<Option<u32>>>,
    pub example_output:        Vec<String>,
    pub example_tx:            mpsc::Sender<ExampleMsg>,
    pub example_rx:            mpsc::Receiver<ExampleMsg>,
    pub last_selected_path:    Option<String>,
    pub selected_project_path: Option<String>,
    pub terminal_dirty:        bool,
    pub should_quit:           bool,
    pub should_restart:        bool,

    // Disk watcher
    pub watch_tx: mpsc::Sender<WatchRequest>,

    // Network state
    pub network_status: NetworkStatus,

    // Projects whose directories have been deleted from disk.
    pub deleted_projects: HashSet<String>,

    // Universal finder
    pub show_finder:       bool,
    pub finder_query:      String,
    pub finder_results:    Vec<usize>,
    pub finder_total:      usize,
    pub finder_pane:       Pane,
    pub finder_index:      Vec<FinderItem>,
    pub finder_col_widths: [usize; FINDER_COLUMN_COUNT],
    pub finder_dirty:      bool,

    // Caches for per-frame hot paths
    pub cached_visible_rows:      Vec<VisibleRow>,
    pub rows_dirty:               bool,
    pub cached_root_sorted:       Vec<u64>,
    pub cached_child_sorted:      HashMap<usize, Vec<u64>>,
    pub disk_cache_dirty:         bool,
    pub cached_fit_widths:        ResolvedWidths,
    fit_widths_dirty:             bool,
    tree_build_tx:                mpsc::Sender<TreeBuildResult>,
    tree_build_rx:                Receiver<TreeBuildResult>,
    tree_build_active:            Option<u64>,
    tree_build_latest:            u64,
    fit_build_tx:                 mpsc::Sender<FitWidthsBuildResult>,
    fit_build_rx:                 Receiver<FitWidthsBuildResult>,
    fit_build_active:             Option<u64>,
    fit_build_latest:             u64,
    disk_build_tx:                mpsc::Sender<DiskCacheBuildResult>,
    disk_build_rx:                Receiver<DiskCacheBuildResult>,
    disk_build_active:            Option<u64>,
    disk_build_latest:            u64,
    pub(super) data_generation:   u64,
    pub(super) detail_generation: u64,
    pub(super) cached_detail:     Option<DetailCache>,
    pub(super) selection_changed: bool,
    pub(super) layout_cache:      LayoutCache,

    /// Transient message shown in the status bar, auto-cleared after a timeout.
    pub(super) status_flash: Option<(String, std::time::Instant)>,
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

            for (wi, wt) in node.worktrees.iter().enumerate() {
                rows.push(VisibleRow::WorktreeEntry {
                    node_index:     ni,
                    worktree_index: wi,
                });
                if wt.has_members() && expanded.contains(&ExpandKey::Worktree(ni, wi)) {
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

fn disk_bytes_for_node_snapshot(
    node: &ProjectNode,
    disk_usage: &HashMap<String, u64>,
) -> Option<u64> {
    if node.worktrees.is_empty() {
        return disk_usage.get(&node.project.path).copied();
    }
    let mut total = 0;
    let mut any_data = false;
    for path in
        std::iter::once(&node.project.path).chain(node.worktrees.iter().map(|wt| &wt.project.path))
    {
        if let Some(&bytes) = disk_usage.get(path) {
            total += bytes;
            any_data = true;
        }
    }
    if any_data { Some(total) } else { None }
}

fn formatted_disk_snapshot(disk_usage: &HashMap<String, u64>, path: &str) -> String {
    disk_usage
        .get(path)
        .copied()
        .map_or_else(|| "—".to_string(), super::render::format_bytes)
}

fn formatted_disk_for_node_snapshot(
    node: &ProjectNode,
    disk_usage: &HashMap<String, u64>,
) -> String {
    disk_bytes_for_node_snapshot(node, disk_usage)
        .map_or_else(|| "—".to_string(), super::render::format_bytes)
}

fn git_sync_snapshot(git_info: &HashMap<String, GitInfo>, path: &str) -> String {
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
    deleted_projects: &HashSet<String>,
    generation: u64,
) -> ResolvedWidths {
    use super::columns::COL_DISK;
    use super::columns::COL_SYNC;

    let dw = super::columns::display_width;
    let mut widths = ResolvedWidths::default();

    for node in nodes {
        App::observe_name_width(
            &mut widths,
            App::fit_name_for_node(node, live_worktree_count_for_node(node, deleted_projects)),
        );
        widths.observe(
            COL_DISK,
            dw(&formatted_disk_for_node_snapshot(node, disk_usage)),
        );
        widths.observe(
            COL_SYNC,
            dw(&git_sync_snapshot(git_info, &node.project.path)),
        );

        for group in &node.groups {
            for member in &group.members {
                let prefix = if group.name.is_empty() {
                    PREFIX_MEMBER_INLINE
                } else {
                    PREFIX_MEMBER_NAMED
                };
                App::observe_name_width(&mut widths, dw(prefix) + dw(&member.display_name()));
                widths.observe(
                    COL_DISK,
                    dw(&formatted_disk_snapshot(disk_usage, &member.path)),
                );
                widths.observe(COL_SYNC, dw(&git_sync_snapshot(git_info, &member.path)));
            }
            if !group.name.is_empty() {
                let label = format!("{} ({})", group.name, group.members.len());
                App::observe_name_width(&mut widths, dw(PREFIX_GROUP_COLLAPSED) + dw(&label));
            }
        }
        for wt in &node.worktrees {
            let wt_name = wt
                .project
                .worktree_name
                .as_deref()
                .unwrap_or(&wt.project.path);
            let wt_prefix = if wt.has_members() {
                PREFIX_WT_COLLAPSED
            } else {
                PREFIX_WT_FLAT
            };
            App::observe_name_width(&mut widths, dw(wt_prefix) + dw(wt_name));
            widths.observe(
                COL_DISK,
                dw(&formatted_disk_snapshot(disk_usage, &wt.project.path)),
            );
            widths.observe(COL_SYNC, dw(&git_sync_snapshot(git_info, &wt.project.path)));
            for group in &wt.groups {
                for member in &group.members {
                    let prefix = if group.name.is_empty() {
                        PREFIX_WT_MEMBER_INLINE
                    } else {
                        PREFIX_WT_MEMBER_NAMED
                    };
                    App::observe_name_width(&mut widths, dw(prefix) + dw(&member.display_name()));
                    widths.observe(
                        COL_DISK,
                        dw(&formatted_disk_snapshot(disk_usage, &member.path)),
                    );
                    widths.observe(COL_SYNC, dw(&git_sync_snapshot(git_info, &member.path)));
                }
                if !group.name.is_empty() {
                    let label = format!("{} ({})", group.name, group.members.len());
                    App::observe_name_width(
                        &mut widths,
                        dw(PREFIX_WT_GROUP_COLLAPSED) + dw(&label),
                    );
                }
            }
        }
    }

    widths.generation = generation;
    widths
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
    const TAB_ORDER: [PaneId; 6] = [
        PaneId::ProjectList,
        PaneId::Package,
        PaneId::Git,
        PaneId::Targets,
        PaneId::CiRuns,
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
                PaneId::CiRuns => InputContext::CiRuns,
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
                PaneId::CiRuns => self.selected_project().is_some_and(|project| {
                    self.ci_state
                        .get(&project.path)
                        .is_some_and(|state| !state.runs().is_empty())
                        || self
                            .git_info
                            .get(&project.path)
                            .is_some_and(|info| info.url.is_some())
                }),
                PaneId::ScanLog => !self.scan_complete,
                PaneId::Search | PaneId::Settings | PaneId::Finder => false,
            })
            .collect()
    }

    pub fn focus_next_pane(&mut self) {
        let panes = self.tabbable_panes();
        if panes.is_empty() {
            return;
        }
        let current = self.base_focus();
        let index = panes.iter().position(|pane| *pane == current).unwrap_or(0);
        let next = panes[(index + 1) % panes.len()];
        self.focus_pane(next);
    }

    pub fn focus_previous_pane(&mut self) {
        let panes = self.tabbable_panes();
        if panes.is_empty() {
            return;
        }
        let current = self.base_focus();
        let index = panes.iter().position(|pane| *pane == current).unwrap_or(0);
        let prev = panes[(index + panes.len() - 1) % panes.len()];
        self.focus_pane(prev);
    }

    pub fn reset_project_panes(&mut self) {
        self.package_pane.home();
        self.git_pane.home();
        self.targets_pane.home();
        self.ci_pane.home();
        self.visited_panes.remove(&PaneId::Package);
        self.visited_panes.remove(&PaneId::Git);
        self.visited_panes.remove(&PaneId::Targets);
        self.visited_panes.remove(&PaneId::CiRuns);
    }

    pub fn remembers_selection(&self, pane: PaneId) -> bool { self.visited_panes.contains(&pane) }

    pub fn sync_selected_project(&mut self) {
        self.ensure_visible_rows_cached();
        let current = self.selected_project().map(|project| project.path.clone());
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
    ) -> Self {
        let (example_tx, example_rx) = mpsc::channel();
        let (ci_fetch_tx, ci_fetch_rx) = mpsc::channel();
        let inline_dirs = cfg.tui.inline_dirs.clone();
        let include_dirs = cfg.tui.include_dirs.clone();
        let ci_run_count = cfg.tui.ci_run_count;
        let include_non_rust = cfg.tui.include_non_rust;
        let watch_tx = watcher::spawn_watcher(
            scan_root.clone(),
            bg_tx.clone(),
            ci_run_count,
            include_non_rust,
            include_dirs.clone(),
            http_client.clone(),
        );
        let editor = cfg.tui.editor.clone();
        let nodes = scan::build_tree(&projects, &inline_dirs);
        let flat_entries = scan::build_flat_entries(&nodes);
        let list_state = initial_list_state(&nodes);
        let (tree_build_tx, tree_build_rx) = mpsc::channel();
        let (fit_build_tx, fit_build_rx) = mpsc::channel();
        let (disk_build_tx, disk_build_rx) = mpsc::channel();
        Self {
            scan_root,
            inline_dirs,
            include_dirs,
            http_client,
            ci_run_count,
            include_non_rust,
            editor,
            #[allow(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "config value is always positive; sub-millisecond truncation is intentional"
            )]
            status_flash_millis: (cfg.tui.status_flash_secs * 1000.0) as u64,
            all_projects: projects,
            nodes,
            flat_entries,
            disk_usage: HashMap::new(),
            ci_state: HashMap::new(),
            lint_status: HashMap::new(),
            lint_enabled: cfg.lint.enabled,
            git_info: HashMap::new(),
            crates_versions: HashMap::new(),
            crates_downloads: HashMap::new(),
            stars: HashMap::new(),
            repo_descriptions: HashMap::new(),
            bg_tx,
            bg_rx,
            fully_loaded: HashSet::new(),
            priority_fetch_path: None,
            invert_scroll: cfg.mouse.invert_scroll,
            expanded: HashSet::new(),
            list_state,
            searching: false,
            search_query: String::new(),
            filtered: Vec::new(),
            show_settings: false,
            settings_pane: Pane::new(),
            settings_editing: false,
            settings_edit_buf: String::new(),
            scan_complete: false,
            scan_log: Vec::new(),
            scan_log_state: ListState::default(),
            focused_pane: PaneId::ProjectList,
            return_focus: None,
            visited_panes: std::iter::once(PaneId::ProjectList).collect(),
            package_pane: Pane::new(),
            git_pane: Pane::new(),
            targets_pane: Pane::new(),
            ci_pane: Pane::new(),
            pending_example_run: None,
            pending_ci_fetch: None,
            pending_clean: None,
            confirm: None,
            animation_started: Instant::now(),
            ci_fetch_tx,
            ci_fetch_rx,
            example_running: None,
            example_child: Arc::new(Mutex::new(None)),
            example_output: Vec::new(),
            example_tx,
            example_rx,
            last_selected_path: super::terminal::load_last_selected(),
            selected_project_path: None,
            terminal_dirty: false,
            should_quit: false,
            should_restart: false,

            watch_tx,

            network_status: NetworkStatus::Online,

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
            cached_fit_widths: ResolvedWidths::default(),
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
            status_flash: None,
        }
    }

    fn apply_tree_build(&mut self, nodes: Vec<ProjectNode>, flat_entries: Vec<FlatEntry>) {
        let selected_path = self
            .selected_project()
            .map(|p| p.path.clone())
            .or_else(|| self.last_selected_path.clone());
        self.nodes = nodes;
        self.flat_entries = flat_entries;
        self.finder_dirty = true;
        self.rows_dirty = true;
        self.disk_cache_dirty = true;
        self.fit_widths_dirty = true;
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
                            runs:      runs.clone(),
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
        self.sync_selected_project();
    }

    pub fn rebuild_tree(&mut self) { self.request_tree_rebuild(); }

    fn request_tree_rebuild(&mut self) {
        self.tree_build_latest = self.tree_build_latest.wrapping_add(1);
        if self.tree_build_active.is_some() {
            return;
        }
        self.spawn_tree_build(self.tree_build_latest);
    }

    fn spawn_tree_build(&mut self, build_id: u64) {
        let tx = self.tree_build_tx.clone();
        let projects = self.all_projects.clone();
        let inline_dirs = self.inline_dirs.clone();
        self.tree_build_active = Some(build_id);
        std::thread::spawn(move || {
            let started = Instant::now();
            let nodes = scan::build_tree(&projects, &inline_dirs);
            let flat_entries = scan::build_flat_entries(&nodes);
            super::perf::log_duration(
                "tree_build",
                started.elapsed(),
                &format!(
                    "build_id={} projects={} nodes={} flat_entries={}",
                    build_id,
                    projects.len(),
                    nodes.len(),
                    flat_entries.len()
                ),
                super::perf::slow_worker_threshold_ms(),
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
        let deleted_projects = self.deleted_projects.clone();
        self.fit_build_active = Some(build_id);
        std::thread::spawn(move || {
            let started = Instant::now();
            let widths = build_fit_widths_snapshot(
                &nodes,
                &disk_usage,
                &git_info,
                &deleted_projects,
                build_id,
            );
            super::perf::log_duration(
                "fit_widths_build",
                started.elapsed(),
                &format!("build_id={} nodes={}", build_id, nodes.len()),
                super::perf::slow_worker_threshold_ms(),
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
            super::perf::log_duration(
                "disk_cache_build",
                started.elapsed(),
                &format!(
                    "build_id={} nodes={} root_values={} child_sets={}",
                    build_id,
                    nodes.len(),
                    root_sorted.len(),
                    child_sorted.len()
                ),
                super::perf::slow_worker_threshold_ms(),
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
        self.git_info.clear();
        self.crates_versions.clear();
        self.crates_downloads.clear();
        self.stars.clear();
        self.repo_descriptions.clear();
        self.scan_log.clear();
        self.scan_log_state = ListState::default();
        self.scan_complete = false;
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
        self.data_generation += 1;
        self.detail_generation += 1;
        let (tx, rx) = scan::spawn_streaming_scan(
            &self.scan_root,
            self.ci_run_count,
            &self.include_dirs,
            self.include_non_rust,
            self.http_client.clone(),
        );
        self.bg_tx = tx;
        self.bg_rx = rx;
        self.watch_tx = watcher::spawn_watcher(
            self.scan_root.clone(),
            self.bg_tx.clone(),
            self.ci_run_count,
            self.include_non_rust,
            self.include_dirs.clone(),
            self.http_client.clone(),
        );
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
            msg_count += 1;
            needs_rebuild |= self.handle_bg_msg(msg);
        }
        stats.bg_msgs = msg_count;

        // Poll CI fetch results
        while let Ok(msg) = self.ci_fetch_rx.try_recv() {
            match msg {
                CiFetchMsg::Complete { path, result, kind } => {
                    self.handle_ci_fetch_complete(path, result, kind);
                },
            }
            stats.ci_msgs += 1;
        }

        // Poll example process output
        while let Ok(msg) = self.example_rx.try_recv() {
            match msg {
                ExampleMsg::Output(line) => {
                    self.example_output.push(line);
                },
                ExampleMsg::Progress(line) => {
                    // Replace the last line (cargo progress bar uses \r)
                    if let Some(last) = self.example_output.last_mut() {
                        *last = line;
                    } else {
                        self.example_output.push(line);
                    }
                },
                ExampleMsg::Finished => {
                    self.example_running = None;
                    self.example_output.push("── done ──".to_string());
                    self.terminal_dirty = true;
                },
            }
            stats.example_msgs += 1;
        }

        stats.tree_results = self.poll_tree_builds();
        stats.fit_results = self.poll_fit_width_builds();
        stats.disk_results = self.poll_disk_cache_builds();

        if needs_rebuild {
            self.request_tree_rebuild();
            self.maybe_priority_fetch();
        }
        stats.needs_rebuild = needs_rebuild;

        self.refresh_async_caches();
        super::perf::log_duration(
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
            super::perf::slow_bg_batch_threshold_ms(),
        );
        stats
    }

    fn handle_disk_usage(&mut self, path: String, bytes: u64) {
        self.fully_loaded.insert(path.clone());
        self.disk_usage.insert(path.clone(), bytes);
        self.disk_cache_dirty = true;
        self.fit_widths_dirty = true;
        if bytes == 0 {
            let abs = self
                .nodes
                .iter()
                .find(|n| n.project.path == path)
                .map(|n| &n.project.abs_path)
                .or_else(|| {
                    self.nodes
                        .iter()
                        .flat_map(|n| n.worktrees.iter())
                        .find(|wt| wt.project.path == path)
                        .map(|wt| &wt.project.abs_path)
                });
            if let Some(abs) = abs
                && !std::path::Path::new(abs).exists()
            {
                self.deleted_projects.insert(path);
            }
        } else {
            self.deleted_projects.remove(&path);
        }
    }

    fn handle_git_info(&mut self, path: String, info: GitInfo) {
        self.fit_widths_dirty = true;
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
        self.finder_dirty = true;
    }

    fn handle_repo_meta(&mut self, path: String, stars: u64, description: Option<String>) {
        self.network_status = NetworkStatus::Online;
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

        let abs_path = PathBuf::from(&project.abs_path);
        let git_tracking = if abs_path.join(".git").exists() {
            GitTracking::Tracked
        } else {
            GitTracking::Untracked
        };
        let _ = self.watch_tx.send(WatchRequest {
            project_path: project.path.clone(),
            abs_path,
            git_tracking,
        });
        self.all_projects.push(project);
        true
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
                self.handle_disk_usage(path, bytes);
            },
            BackgroundMsg::CiRuns { path, runs } => {
                self.insert_ci_runs(path, runs);
            },
            BackgroundMsg::GitInfo { path, info } => {
                self.handle_git_info(path, info);
            },
            BackgroundMsg::CratesIoVersion {
                path,
                version,
                downloads,
            } => {
                self.crates_versions.insert(path.clone(), version);
                self.crates_downloads.insert(path, downloads);
                self.network_status = NetworkStatus::Online;
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
            BackgroundMsg::ScanActivity { path } => {
                self.scan_log.push(path);
                let len = self.scan_log.len();
                if self
                    .scan_log_state
                    .selected()
                    .is_none_or(|s| s >= len.saturating_sub(2))
                {
                    self.scan_log_state.select(Some(len.saturating_sub(1)));
                }
            },
            BackgroundMsg::LintStatus { path, status } => {
                self.lint_status.insert(path, status);
            },
            BackgroundMsg::ScanComplete => {
                self.scan_complete = true;
                if self.focused_pane == PaneId::ScanLog {
                    self.focus_pane(PaneId::ProjectList);
                }
            },
            BackgroundMsg::NetworkOffline => {
                self.network_status = NetworkStatus::Offline;
            },
        }
        false
    }

    fn detail_path_is_affected(&self, path: &str) -> bool {
        let Some(project) = self.selected_project() else {
            return false;
        };
        if project.path == path {
            return true;
        }
        let Some(node) = self.selected_node() else {
            return false;
        };
        if node.project.path != project.path || node.worktrees.is_empty() {
            return false;
        }
        node.worktrees.iter().any(|wt| wt.project.path == path)
    }

    /// Insert CI runs from the initial scan, propagating to workspace members.
    fn insert_ci_runs(&mut self, path: String, runs: Vec<CiRun>) {
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
    pub fn visible_rows(&self) -> &[VisibleRow] { &self.cached_visible_rows }

    /// Keep fit-to-content widths rebuilding in the background, never inline on the UI thread.
    pub fn ensure_fit_widths_cached(&mut self) { self.request_fit_widths_build(); }

    /// Iterate all group members in a node, including those nested under worktree entries.
    fn all_group_members(node: &ProjectNode) -> impl Iterator<Item = &RustProject> {
        let direct = node.groups.iter().flat_map(|g| g.members.iter());
        let wt = node
            .worktrees
            .iter()
            .flat_map(|wt| wt.groups.iter().flat_map(|g| g.members.iter()));
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
    pub fn ensure_disk_cache(&mut self) { self.request_disk_cache_build(); }

    /// Ensure the cached `DetailInfo` is up to date for the selected project.
    /// The cache is valid only when the generation AND path both match.
    pub fn ensure_detail_cached(&mut self) {
        let current_path = self
            .selected_project()
            .map(|p| p.path.clone())
            .unwrap_or_default();

        if let Some(ref cache) = self.cached_detail
            && cache.generation == self.detail_generation
            && cache.path == current_path
        {
            return;
        }

        self.cached_detail = self.selected_project().map(|p| DetailCache {
            generation: self.detail_generation,
            path:       current_path,
            info:       super::detail::build_detail_info(self, p),
        });
    }

    /// Returns the `ProjectNode` when a root row is selected (not a member or worktree).
    pub fn selected_node(&self) -> Option<&ProjectNode> {
        if self.searching && !self.search_query.is_empty() {
            return None;
        }
        let rows = self.visible_rows();
        let selected = self.list_state.selected()?;
        match rows.get(selected)? {
            VisibleRow::Root { node_index } => self.nodes.get(*node_index),
            _ => None,
        }
    }

    pub fn selected_project(&self) -> Option<&RustProject> {
        if self.searching && !self.search_query.is_empty() {
            let selected = self.list_state.selected()?;
            let flat_idx = *self.filtered.get(selected)?;
            let entry = self.flat_entries.get(flat_idx)?;
            let node = self.nodes.get(entry.node_index)?;
            Some(
                node.groups
                    .get(entry.group_index)
                    .and_then(|g| g.members.get(entry.member_index))
                    .unwrap_or(&node.project),
            )
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
            }) => self.nodes[*node_index].worktrees[*worktree_index].has_members(),
            _ => false,
        }
    }

    pub(super) fn expand(&mut self) {
        if !self.selected_is_expandable() {
            return;
        }
        let Some(selected) = self.list_state.selected() else {
            return;
        };
        let Some(row) = self.visible_rows().get(selected).copied() else {
            return;
        };
        match row {
            VisibleRow::Root { node_index } => {
                self.expanded.insert(ExpandKey::Node(node_index));
            },
            VisibleRow::GroupHeader {
                node_index,
                group_index,
            } => {
                self.expanded
                    .insert(ExpandKey::Group(node_index, group_index));
            },
            VisibleRow::WorktreeEntry {
                node_index,
                worktree_index,
            } => {
                self.expanded
                    .insert(ExpandKey::Worktree(node_index, worktree_index));
            },
            VisibleRow::WorktreeGroupHeader {
                node_index,
                worktree_index,
                group_index,
            } => {
                self.expanded.insert(ExpandKey::WorktreeGroup(
                    node_index,
                    worktree_index,
                    group_index,
                ));
            },
            _ => {},
        }
        self.rows_dirty = true;
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

    pub(super) fn collapse(&mut self) {
        let Some(selected) = self.list_state.selected() else {
            return;
        };
        let Some(row) = self.visible_rows().get(selected).copied() else {
            return;
        };
        self.collapse_row(row);
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
                            node_index:  ni,
                            group_index: gi,
                        },
                    );
                }
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
                            node_index:     ni,
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
                            node_index:     ni,
                            worktree_index: wi,
                        },
                    );
                } else {
                    self.collapse_to(
                        &ExpandKey::WorktreeGroup(ni, wi, gi),
                        VisibleRow::WorktreeGroupHeader {
                            node_index:     ni,
                            worktree_index: wi,
                            group_index:    gi,
                        },
                    );
                }
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

    pub(super) fn select_project_in_tree(&mut self, target_path: &str) {
        // Expand the containing node, group, or worktree parent
        for (ni, node) in self.nodes.iter().enumerate() {
            // Direct members
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
            // Worktree entries
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
            }
        }

        self.rows_dirty = true;
        self.ensure_visible_rows_cached();
        let rows = self.visible_rows();
        for (i, row) in rows.iter().enumerate() {
            match row {
                VisibleRow::Root { node_index } => {
                    if self.nodes[*node_index].project.path == target_path {
                        self.list_state.select(Some(i));
                        return;
                    }
                },
                VisibleRow::Member {
                    node_index,
                    group_index,
                    member_index,
                } => {
                    let project =
                        &self.nodes[*node_index].groups[*group_index].members[*member_index];
                    if project.path == target_path {
                        self.list_state.select(Some(i));
                        return;
                    }
                },
                VisibleRow::WorktreeEntry {
                    node_index,
                    worktree_index,
                } => {
                    let wt = &self.nodes[*node_index].worktrees[*worktree_index];
                    if wt.project.path == target_path {
                        self.list_state.select(Some(i));
                        return;
                    }
                },
                VisibleRow::WorktreeMember {
                    node_index,
                    worktree_index,
                    group_index,
                    member_index,
                } => {
                    let wt = &self.nodes[*node_index].worktrees[*worktree_index];
                    let member = &wt.groups[*group_index].members[*member_index];
                    if member.path == target_path {
                        self.list_state.select(Some(i));
                        return;
                    }
                },
                VisibleRow::GroupHeader { .. } | VisibleRow::WorktreeGroupHeader { .. } => {},
            }
        }
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

    pub fn is_deleted(&self, path: &str) -> bool { self.deleted_projects.contains(path) }

    pub fn live_worktree_count(&self, node: &ProjectNode) -> usize {
        node.worktrees
            .iter()
            .filter(|wt| !self.is_deleted(&wt.project.path))
            .count()
    }

    pub fn live_node_count(&self) -> usize {
        self.nodes
            .iter()
            .filter(|n| !self.is_deleted(&n.project.path))
            .count()
    }

    pub fn formatted_disk(&self, project: &RustProject) -> String {
        match self.disk_usage.get(&project.path) {
            Some(&bytes) => super::render::format_bytes(bytes),
            None => "—".to_string(),
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
        for path in std::iter::once(&node.project.path)
            .chain(node.worktrees.iter().map(|wt| &wt.project.path))
        {
            if let Some(&bytes) = self.disk_usage.get(path) {
                total += bytes;
                any_data = true;
            }
        }
        if any_data {
            super::render::format_bytes(total)
        } else {
            "—".to_string()
        }
    }

    /// Get total disk bytes for a node (sum of root + worktrees).
    pub fn disk_bytes_for_node(&self, node: &ProjectNode) -> Option<u64> {
        if node.worktrees.is_empty() {
            return self.disk_usage.get(&node.project.path).copied();
        }
        let mut total: u64 = 0;
        let mut any_data = false;
        for path in std::iter::once(&node.project.path)
            .chain(node.worktrees.iter().map(|wt| &wt.project.path))
        {
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

    pub fn animation_elapsed(&self) -> Duration { self.animation_started.elapsed() }

    /// Lint icon frame for the current animation state, or a blank space if lint is
    /// disabled or no log exists.
    pub fn lint_icon(&self, project: &RustProject) -> &'static str {
        use crate::constants::LINT_NO_LOG;

        if !self.lint_enabled {
            return LINT_NO_LOG;
        }
        let Some(status) = self.lint_status.get(&project.path) else {
            return LINT_NO_LOG;
        };
        status.icon().frame_at(self.animation_elapsed())
    }

    pub fn git_icon(&self, project: &RustProject) -> &'static str {
        self.git_info
            .get(&project.path)
            .map_or(" ", |info| info.origin.icon())
    }

    /// Formatted ahead/behind sync status for the project list columns.
    pub fn git_sync(&self, project: &RustProject) -> String {
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

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::sync::OnceLock;
    use std::sync::mpsc;

    use super::*;
    use crate::config::Config;
    use crate::http::HttpClient;
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
            path:                      path.to_string(),
            abs_path:                  path.to_string(),
            name:                      name.map(String::from),
            version:                   None,
            description:               None,
            worktree_name:             None,
            worktree_primary_abs_path: None,
            is_workspace:              WorkspaceStatus::Standalone,
            types:                     Vec::new(),
            examples:                  Vec::new(),
            benches:                   Vec::new(),
            test_count:                0,
            is_rust:                   ProjectLanguage::Rust,
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
        let (bg_tx, bg_rx) = mpsc::channel();
        let scan_root =
            std::env::temp_dir().join(format!("cargo-port-polish-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&scan_root);
        let mut app = App::new(
            scan_root,
            projects,
            bg_tx,
            bg_rx,
            &Config::default(),
            test_http_client(),
        );
        app.sync_selected_project();
        app
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
            name:    String::new(),
            members: vec![member_a.clone(), member_b.clone()],
        }];

        // Actual worktree with a named group
        let mut wt1 = make_node(make_project(None, "~/ws_feat"));
        wt1.project.worktree_name = Some("ws_feat".to_string());
        wt1.groups = vec![MemberGroup {
            name:    "crates".to_string(),
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
                node_index:     0,
                worktree_index: 0,
            }
        ));
        assert!(matches!(
            rows[2],
            VisibleRow::WorktreeMember {
                node_index:     0,
                worktree_index: 0,
                group_index:    0,
                member_index:   0,
            }
        ));
        assert!(matches!(
            rows[4],
            VisibleRow::WorktreeEntry {
                node_index:     0,
                worktree_index: 1,
            }
        ));
        assert!(matches!(
            rows[5],
            VisibleRow::WorktreeGroupHeader {
                node_index:     0,
                worktree_index: 1,
                group_index:    0,
            }
        ));
        assert!(matches!(
            rows[7],
            VisibleRow::WorktreeMember {
                node_index:     0,
                worktree_index: 1,
                group_index:    0,
                member_index:   1,
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
            name:    String::new(),
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
    fn name_width_with_gutter_reserves_space_before_lint() {
        assert_eq!(App::name_width_with_gutter(0), 1);
        assert_eq!(App::name_width_with_gutter(42), 43);
    }

    #[test]
    fn tabbable_panes_follow_canonical_order() {
        let mut project = make_project(Some("demo"), "~/demo");
        project.examples = vec![ExampleGroup {
            category: String::new(),
            names:    vec!["example".to_string()],
        }];

        let mut app = make_app(vec![project.clone()]);
        app.scan_complete = true;
        app.git_info.insert(
            project.path,
            GitInfo {
                origin:              GitOrigin::Clone,
                branch:              None,
                owner:               None,
                url:                 Some("https://github.com/acme/demo".to_string()),
                first_commit:        None,
                last_commit:         None,
                ahead_behind:        None,
                default_branch:      None,
                ahead_behind_origin: None,
                ahead_behind_local:  None,
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

        app.focus_next_pane();
        assert_eq!(app.focused_pane, PaneId::Package);
        app.focus_next_pane();
        assert_eq!(app.focused_pane, PaneId::Git);
        app.focus_next_pane();
        assert_eq!(app.focused_pane, PaneId::Targets);
        app.focus_previous_pane();
        assert_eq!(app.focused_pane, PaneId::Git);
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
}
