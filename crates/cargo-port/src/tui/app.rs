use std::collections::HashMap;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::mpsc;
use std::sync::mpsc::Receiver;

use nucleo_matcher::Matcher;
use nucleo_matcher::Utf32Str;
use nucleo_matcher::pattern::Atom;
use nucleo_matcher::pattern::AtomKind;
use nucleo_matcher::pattern::CaseMatching;
use nucleo_matcher::pattern::Normalization;
use ratatui::widgets::ListState;

use super::detail::DetailField;
use super::detail::DetailInfo;
use super::detail::PendingCiFetch;
use super::detail::PendingExampleRun;
use super::detail::ProjectCounts;
use super::finder::FINDER_COLUMN_COUNT;
use super::finder::FinderItem;
use super::shortcuts::InputContext;
use super::terminal::CiFetchMsg;
use super::terminal::ExampleMsg;
use super::types::FocusTarget;
use super::types::ScrollState;
use crate::ci::CiRun;
use crate::config::Config;
use crate::project::GitInfo;
use crate::project::RustProject;
use crate::scan;
use crate::scan::BackgroundMsg;
use crate::scan::CiFetchResult;
use crate::scan::FlatEntry;
use crate::scan::ProjectNode;

/// An expand key: either a workspace node or a group within a node.
#[derive(Hash, Eq, PartialEq, Clone)]
pub enum ExpandKey {
    Node(usize),
    Group(usize, usize),
}

/// An action waiting for user confirmation (y/n).
pub enum ConfirmAction {
    /// `cargo clean` on the project at this absolute path.
    Clean(String),
}

/// Cached column widths for fit-to-content columns in the project list.
pub struct FitWidths {
    pub name:       usize,
    pub disk:       usize,
    pub sync:       usize,
    pub generation: u64,
}

impl Default for FitWidths {
    fn default() -> Self {
        Self {
            name:       0,
            disk:       "Disk".len(),
            sync:       0,
            generation: u64::MAX,
        }
    }
}

/// What a visible row represents.
#[derive(Clone, Copy)]
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
    /// A worktree entry shown directly under the parent node.
    WorktreeEntry {
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

/// Generation-stamped detail cache. Automatically stale when `data_generation`
/// on `App` has advanced past the generation stored here.
pub(super) struct DetailCache {
    generation: u64,
    path:       String,
    pub info:   DetailInfo,
}

#[allow(clippy::struct_excessive_bools)]
pub struct App {
    pub scan_root:           PathBuf,
    pub inline_dirs:         Vec<String>,
    pub exclude_dirs:        Vec<String>,
    pub ci_run_count:        u32,
    pub include_non_rust:    bool,
    pub editor:              String,
    pub all_projects:        Vec<RustProject>,
    pub nodes:               Vec<ProjectNode>,
    pub flat_entries:        Vec<FlatEntry>,
    pub disk_usage:          HashMap<String, u64>,
    pub ci_state:            HashMap<String, CiState>,
    pub git_info:            HashMap<String, GitInfo>,
    pub crates_versions:     HashMap<String, String>,
    pub stars:               HashMap<String, u64>,
    pub bg_tx:               mpsc::Sender<BackgroundMsg>,
    pub bg_rx:               Receiver<BackgroundMsg>,
    pub fully_loaded:        HashSet<String>,
    pub priority_fetch_path: Option<String>,
    pub invert_scroll:       bool,
    pub expanded:            HashSet<ExpandKey>,
    pub list_state:          ListState,
    pub searching:           bool,
    pub search_query:        String,
    pub filtered:            Vec<usize>,
    pub show_settings:       bool,
    pub settings_cursor:     ScrollState,
    pub settings_editing:    bool,
    pub settings_edit_buf:   String,
    pub scan_complete:       bool,
    pub scan_log:            Vec<String>,
    pub scan_log_state:      ListState,
    pub focus:               FocusTarget,
    pub detail_column:       ScrollState,
    pub detail_cursor:       ScrollState,
    pub ci_runs_cursor:      ScrollState,
    pub examples_scroll:     ScrollState,
    pub pending_example_run: Option<PendingExampleRun>,
    pub pending_ci_fetch:    Option<PendingCiFetch>,
    pub pending_clean:       Option<String>,
    pub confirm:             Option<ConfirmAction>,
    pub spinner_tick:        usize,
    pub ci_fetch_tx:         mpsc::Sender<CiFetchMsg>,
    pub ci_fetch_rx:         mpsc::Receiver<CiFetchMsg>,
    pub example_running:     Option<String>,
    pub example_child:       Arc<Mutex<Option<u32>>>,
    pub example_output:      Vec<String>,
    pub example_tx:          mpsc::Sender<ExampleMsg>,
    pub example_rx:          mpsc::Receiver<ExampleMsg>,
    pub last_selected_path:  Option<String>,
    pub terminal_dirty:      bool,
    pub should_quit:         bool,
    pub should_restart:      bool,

    // Network state
    pub network_offline: bool,

    // Universal finder
    pub show_finder:       bool,
    pub finder_query:      String,
    pub finder_results:    Vec<usize>,
    pub finder_total:      usize,
    pub finder_cursor:     ScrollState,
    pub finder_index:      Vec<FinderItem>,
    pub finder_col_widths: [usize; FINDER_COLUMN_COUNT],
    pub finder_dirty:      bool,

    // Caches for per-frame hot paths
    pub cached_visible_rows:      Vec<VisibleRow>,
    pub rows_dirty:               bool,
    pub cached_root_sorted:       Vec<u64>,
    pub cached_child_sorted:      HashMap<usize, Vec<u64>>,
    pub disk_cache_dirty:         bool,
    pub cached_fit_widths:        FitWidths,
    pub(super) data_generation:   u64,
    pub(super) cached_detail:     Option<DetailCache>,
    pub(super) selection_changed: bool,
}

impl App {
    /// Derive the current input context from app state.
    pub fn input_context(&self) -> InputContext {
        if self.show_finder {
            InputContext::Finder
        } else if self.show_settings {
            InputContext::Settings
        } else if self.searching {
            InputContext::Searching
        } else {
            match self.focus {
                FocusTarget::DetailFields => {
                    let (_, targets_col) = super::detail::detail_layout_pub(self);
                    if Some(self.detail_column.pos()) == targets_col {
                        InputContext::DetailTargets
                    } else {
                        InputContext::DetailFields
                    }
                },
                FocusTarget::CiRuns => InputContext::CiRuns,
                FocusTarget::ScanLog => InputContext::ScanLog,
                FocusTarget::ProjectList => InputContext::ProjectList,
            }
        }
    }

    pub(super) fn new(
        scan_root: PathBuf,
        projects: Vec<RustProject>,
        bg_tx: mpsc::Sender<BackgroundMsg>,
        bg_rx: Receiver<BackgroundMsg>,
        cfg: &Config,
    ) -> Self {
        let (example_tx, example_rx) = mpsc::channel();
        let (ci_fetch_tx, ci_fetch_rx) = mpsc::channel();
        let inline_dirs = cfg.tui.inline_dirs.clone();
        let exclude_dirs = cfg.tui.exclude_dirs.clone();
        let ci_run_count = cfg.tui.ci_run_count;
        let include_non_rust = cfg.tui.include_non_rust;
        let editor = cfg.tui.editor.clone();
        let nodes = scan::build_tree(projects.clone(), &inline_dirs);
        let flat_entries = scan::build_flat_entries(&nodes);
        let mut list_state = ListState::default();
        if !nodes.is_empty() {
            list_state.select(Some(0));
        }
        Self {
            scan_root,
            inline_dirs,
            exclude_dirs,
            ci_run_count,
            include_non_rust,
            editor,
            all_projects: projects,
            nodes,
            flat_entries,
            disk_usage: HashMap::new(),
            ci_state: HashMap::new(),
            git_info: HashMap::new(),
            crates_versions: HashMap::new(),
            stars: HashMap::new(),
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
            settings_cursor: ScrollState::default(),
            settings_editing: false,
            settings_edit_buf: String::new(),
            scan_complete: false,
            scan_log: Vec::new(),
            scan_log_state: ListState::default(),
            focus: FocusTarget::ProjectList,
            detail_column: ScrollState::default(),
            detail_cursor: ScrollState::default(),
            ci_runs_cursor: ScrollState::default(),
            examples_scroll: ScrollState::default(),
            pending_example_run: None,
            pending_ci_fetch: None,
            pending_clean: None,
            confirm: None,
            spinner_tick: 0,
            ci_fetch_tx,
            ci_fetch_rx,
            example_running: None,
            example_child: Arc::new(Mutex::new(None)),
            example_output: Vec::new(),
            example_tx,
            example_rx,
            last_selected_path: super::terminal::load_last_selected(),
            terminal_dirty: false,
            should_quit: false,
            should_restart: false,

            network_offline: false,

            show_finder: false,
            finder_query: String::new(),
            finder_results: Vec::new(),
            finder_total: 0,
            finder_cursor: ScrollState::default(),
            finder_index: Vec::new(),
            finder_col_widths: [0; super::finder::FINDER_COLUMN_COUNT],
            finder_dirty: true,

            cached_visible_rows: Vec::new(),
            rows_dirty: true,
            cached_root_sorted: Vec::new(),
            cached_child_sorted: HashMap::new(),
            disk_cache_dirty: true,
            cached_fit_widths: FitWidths::default(),
            data_generation: 0,
            cached_detail: None,
            selection_changed: false,
        }
    }

    pub fn rebuild_tree(&mut self) {
        let selected_path = self
            .selected_project()
            .map(|p| p.path.clone())
            .or_else(|| self.last_selected_path.clone());
        self.nodes = scan::build_tree(self.all_projects.clone(), &self.inline_dirs);
        self.flat_entries = scan::build_flat_entries(&self.nodes);
        self.finder_dirty = true;
        self.rows_dirty = true;
        self.disk_cache_dirty = true;
        self.data_generation += 1;

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
                for group in &node.groups {
                    for member in &group.members {
                        self.ci_state.entry(member.path.clone()).or_insert_with(|| {
                            CiState::Loaded {
                                runs:      runs.clone(),
                                exhausted: false,
                            }
                        });
                    }
                }
            }
            if let Some(info) = self.git_info.get(&node.project.path).cloned() {
                for group in &node.groups {
                    for member in &group.members {
                        self.git_info
                            .entry(member.path.clone())
                            .or_insert_with(|| info.clone());
                    }
                }
            }
            if let Some(&stars) = self.stars.get(&node.project.path) {
                for group in &node.groups {
                    for member in &group.members {
                        self.stars.entry(member.path.clone()).or_insert(stars);
                    }
                }
            }
        }

        // Try to restore selection
        if let Some(path) = selected_path {
            self.select_project_in_tree(&path);
        } else if !self.nodes.is_empty() {
            self.list_state.select(Some(0));
        }
    }

    pub(super) fn rescan(&mut self) {
        self.all_projects.clear();
        self.nodes.clear();
        self.flat_entries.clear();
        self.disk_usage.clear();
        self.ci_state.clear();
        self.git_info.clear();
        self.crates_versions.clear();
        self.stars.clear();
        self.scan_log.clear();
        self.scan_log_state = ListState::default();
        self.scan_complete = false;
        self.fully_loaded.clear();
        self.priority_fetch_path = None;
        self.focus = FocusTarget::ProjectList;
        self.detail_column.jump_home();
        self.detail_cursor.jump_home();
        self.ci_runs_cursor.jump_home();
        self.examples_scroll.jump_home();
        self.pending_ci_fetch = None;
        self.expanded.clear();
        self.list_state = ListState::default();
        self.rows_dirty = true;
        self.disk_cache_dirty = true;
        self.data_generation += 1;
        let (tx, rx) = scan::spawn_streaming_scan(
            &self.scan_root,
            self.ci_run_count,
            &self.exclude_dirs,
            self.include_non_rust,
        );
        self.bg_tx = tx;
        self.bg_rx = rx;
    }

    pub(super) fn poll_background(&mut self) {
        const MAX_MSGS_PER_FRAME: usize = 50;
        let mut needs_rebuild = false;
        let mut msg_count = 0;

        while msg_count < MAX_MSGS_PER_FRAME {
            let Ok(msg) = self.bg_rx.try_recv() else {
                break;
            };
            msg_count += 1;
            needs_rebuild |= self.handle_bg_msg(msg);
        }

        // Poll CI fetch results
        while let Ok(msg) = self.ci_fetch_rx.try_recv() {
            match msg {
                CiFetchMsg::Complete { path, result } => {
                    self.handle_ci_fetch_complete(path, result);
                },
            }
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
        }

        if needs_rebuild {
            self.rebuild_tree();
            self.maybe_priority_fetch();
        }
    }

    /// Handle a single `BackgroundMsg`. Returns `true` if the tree needs rebuilding.
    fn handle_bg_msg(&mut self, msg: BackgroundMsg) -> bool {
        // Bump generation for any message that carries project data, so the
        // detail cache auto-invalidates without a separate dirty flag.
        if msg.path().is_some() {
            self.data_generation += 1;
        }
        match msg {
            BackgroundMsg::DiskUsage { path, bytes } => {
                self.fully_loaded.insert(path.clone());
                self.disk_usage.insert(path, bytes);
                self.disk_cache_dirty = true;
            },
            BackgroundMsg::CiRuns { path, runs } => {
                self.insert_ci_runs(path, runs);
            },
            BackgroundMsg::GitInfo { path, info } => {
                // Propagate to workspace members and worktrees.
                // Search both top-level nodes and worktree sub-nodes.
                let matching_node =
                    self.nodes
                        .iter()
                        .find(|n| n.project.path == path)
                        .or_else(|| {
                            self.nodes
                                .iter()
                                .flat_map(|n| n.worktrees.iter())
                                .find(|wt| wt.project.path == path)
                        });
                if let Some(node) = matching_node {
                    for group in &node.groups {
                        for member in &group.members {
                            // Always overwrite — the correct branch comes from
                            // the workspace root, not from a stale propagation.
                            self.git_info.insert(member.path.clone(), info.clone());
                        }
                    }
                    for wt in &node.worktrees {
                        self.git_info
                            .entry(wt.project.path.clone())
                            .or_insert_with(|| info.clone());
                    }
                }
                self.git_info.insert(path, info);
                self.finder_dirty = true;
            },
            BackgroundMsg::CratesIoVersion { path, version } => {
                self.crates_versions.insert(path, version);
                self.network_offline = false;
            },
            BackgroundMsg::Stars { path, count } => {
                self.network_offline = false;
                // Propagate to workspace members
                if let Some(node) = self.nodes.iter().find(|n| n.project.path == path) {
                    for group in &node.groups {
                        for member in &group.members {
                            self.stars.entry(member.path.clone()).or_insert(count);
                        }
                    }
                }
                self.stars.insert(path, count);
            },
            BackgroundMsg::ProjectDiscovered { project } => {
                if !self.all_projects.iter().any(|p| p.path == project.path) {
                    self.all_projects.push(project);
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
            BackgroundMsg::ScanComplete => {
                self.scan_complete = true;
                if self.focus == FocusTarget::ScanLog {
                    self.focus = FocusTarget::ProjectList;
                }
            },
            BackgroundMsg::NetworkOffline => {
                self.network_offline = true;
            },
        }
        false
    }

    /// Insert CI runs from the initial scan, propagating to workspace members.
    fn insert_ci_runs(&mut self, path: String, runs: Vec<CiRun>) {
        let exhausted = self
            .git_info
            .get(&path)
            .and_then(|g| g.url.as_ref())
            .and_then(|url| crate::ci::parse_owner_repo(url))
            .is_some_and(|(owner, repo)| scan::is_exhausted(&owner, &repo));
        if let Some(node) = self.nodes.iter().find(|n| n.project.path == path) {
            for group in &node.groups {
                for member in &group.members {
                    self.ci_state
                        .entry(member.path.clone())
                        .or_insert_with(|| CiState::Loaded {
                            runs: runs.clone(),
                            exhausted,
                        });
                }
            }
        }
        self.ci_state
            .insert(path, CiState::Loaded { runs, exhausted });
    }

    /// Process a completed CI fetch: merge runs, detect exhaustion, propagate to members.
    fn handle_ci_fetch_complete(&mut self, path: String, result: CiFetchResult) {
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

        let exhausted = if merged.len() <= prev_count {
            if let Some(git) = self.git_info.get(&path)
                && let Some(ref url) = git.url
                && let Some((owner, repo)) = crate::ci::parse_owner_repo(url)
            {
                scan::mark_exhausted(&owner, &repo);
            }
            true
        } else {
            false
        };

        let state = CiState::Loaded {
            runs: merged.clone(),
            exhausted,
        };

        if let Some(node) = self.nodes.iter().find(|n| n.project.path == path) {
            for group in &node.groups {
                for member in &group.members {
                    self.ci_state
                        .entry(member.path.clone())
                        .or_insert_with(|| CiState::Loaded {
                            runs: merged.clone(),
                            exhausted,
                        });
                }
            }
        }
        self.ci_runs_cursor.set(merged.len());
        self.ci_state.insert(path, state);
        self.data_generation += 1;
    }

    /// Spawn a priority fetch for the selected project if it hasn't been loaded yet.
    pub(super) fn maybe_priority_fetch(&mut self) {
        if self.scan_complete {
            return;
        }
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
        self.cached_visible_rows.clear();
        for (ni, node) in self.nodes.iter().enumerate() {
            self.cached_visible_rows
                .push(VisibleRow::Root { node_index: ni });
            if self.expanded.contains(&ExpandKey::Node(ni)) {
                for (gi, group) in node.groups.iter().enumerate() {
                    if group.name.is_empty() {
                        for (mi, _) in group.members.iter().enumerate() {
                            self.cached_visible_rows.push(VisibleRow::Member {
                                node_index:   ni,
                                group_index:  gi,
                                member_index: mi,
                            });
                        }
                    } else {
                        self.cached_visible_rows.push(VisibleRow::GroupHeader {
                            node_index:  ni,
                            group_index: gi,
                        });
                        if self.expanded.contains(&ExpandKey::Group(ni, gi)) {
                            for (mi, _) in group.members.iter().enumerate() {
                                self.cached_visible_rows.push(VisibleRow::Member {
                                    node_index:   ni,
                                    group_index:  gi,
                                    member_index: mi,
                                });
                            }
                        }
                    }
                }

                // Worktree entries shown directly under the node
                for (wi, _wt) in node.worktrees.iter().enumerate() {
                    self.cached_visible_rows.push(VisibleRow::WorktreeEntry {
                        node_index:     ni,
                        worktree_index: wi,
                    });
                }
            }
        }
    }

    /// Return the cached visible rows. Must call `ensure_visible_rows_cached()` first.
    pub fn visible_rows(&self) -> &[VisibleRow] { &self.cached_visible_rows }

    /// Recompute fit-to-content column widths across all projects.
    /// Called alongside other cache refreshes in the render loop.
    pub fn ensure_fit_widths_cached(&mut self) {
        if self.cached_fit_widths.generation == self.data_generation {
            return;
        }
        let mut name_width = 0usize;
        let mut disk_width = "Disk".len();
        let mut sync_width = 0usize;

        for node in &self.nodes {
            name_width = name_width.max(self.fit_name_for_node(node));
            disk_width = disk_width.max(self.formatted_disk_for_node(node).len());
            sync_width = sync_width.max(self.git_sync(&node.project).len());

            for group in &node.groups {
                for member in &group.members {
                    let prefix = if group.name.is_empty() { 4 } else { 8 };
                    name_width = name_width.max(prefix + member.display_name().len());
                    disk_width = disk_width.max(self.formatted_disk(member).len());
                    sync_width = sync_width.max(self.git_sync(member).len());
                }
                if !group.name.is_empty() {
                    name_width = name_width.max(6 + group.name.len() + 4);
                }
            }
            for wt in &node.worktrees {
                let wt_name = wt
                    .project
                    .worktree_name
                    .as_deref()
                    .unwrap_or(&wt.project.path);
                name_width = name_width.max(8 + wt_name.len());
                disk_width = disk_width.max(self.formatted_disk(&wt.project).len());
                sync_width = sync_width.max(self.git_sync(&wt.project).len());
            }
        }

        self.cached_fit_widths = FitWidths {
            name:       name_width + 1,
            disk:       disk_width,
            sync:       sync_width,
            generation: self.data_generation,
        };
    }

    fn fit_name_for_node(&self, node: &ProjectNode) -> usize {
        let mut name = node.project.display_name();
        if !node.worktrees.is_empty() {
            name = format!("{name} wt:{}", node.worktrees.len());
        }
        2 + name.len()
    }

    /// Ensure the cached disk sort data is up to date, recomputing only when dirty.
    pub fn ensure_disk_cache(&mut self) {
        if !self.disk_cache_dirty {
            return;
        }
        self.disk_cache_dirty = false;

        // Root-level sorted disk values
        self.cached_root_sorted.clear();
        for node in &self.nodes {
            if let Some(bytes) = self.disk_bytes_for_node(node) {
                self.cached_root_sorted.push(bytes);
            }
        }
        self.cached_root_sorted.sort_unstable();

        // Per-node child sorted disk values
        self.cached_child_sorted.clear();
        for (ni, node) in self.nodes.iter().enumerate() {
            let mut values: Vec<u64> = Vec::new();
            for group in &node.groups {
                for member in &group.members {
                    if let Some(&bytes) = self.disk_usage.get(&member.path) {
                        values.push(bytes);
                    }
                }
            }
            for wt in &node.worktrees {
                if let Some(&bytes) = self.disk_usage.get(&wt.project.path) {
                    values.push(bytes);
                }
            }
            if !values.is_empty() {
                values.sort_unstable();
                self.cached_child_sorted.insert(ni, values);
            }
        }
    }

    /// Ensure the cached `DetailInfo` is up to date for the selected project.
    /// The cache is valid only when the generation AND path both match.
    pub fn ensure_detail_cached(&mut self) {
        let current_path = self
            .selected_project()
            .map(|p| p.path.clone())
            .unwrap_or_default();

        if let Some(ref cache) = self.cached_detail
            && cache.generation == self.data_generation
            && cache.path == current_path
        {
            return;
        }

        self.cached_detail = self.selected_project().map(|p| DetailCache {
            generation: self.data_generation,
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
                } => {
                    let node = self.nodes.get(*node_index)?;
                    let wt = node.worktrees.get(*worktree_index)?;
                    Some(&wt.project)
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
            Some(VisibleRow::GroupHeader { .. }) => true,
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
            _ => {},
        }
        self.rows_dirty = true;
    }

    pub(super) fn collapse(&mut self) {
        let Some(selected) = self.list_state.selected() else {
            return;
        };
        let Some(row) = self.visible_rows().get(selected).copied() else {
            return;
        };

        match row {
            VisibleRow::Root { node_index } => {
                self.expanded.remove(&ExpandKey::Node(node_index));
                self.rows_dirty = true;
            },
            VisibleRow::GroupHeader {
                node_index,
                group_index,
            } => {
                if self
                    .expanded
                    .remove(&ExpandKey::Group(node_index, group_index))
                {
                    // Group was expanded, now collapsed — done
                    self.rows_dirty = true;
                } else {
                    // Already collapsed group — collapse parent node
                    self.expanded.remove(&ExpandKey::Node(node_index));
                    self.rows_dirty = true;
                    // Recompute rows and move cursor to the node root
                    self.ensure_visible_rows_cached();
                    if let Some(pos) = self.visible_rows().iter().position(
                        |r| matches!(r, VisibleRow::Root { node_index: ni } if *ni == node_index),
                    ) {
                        self.list_state.select(Some(pos));
                    }
                }
            },
            VisibleRow::Member {
                node_index,
                group_index,
                ..
            } => {
                let group_name = &self.nodes[node_index].groups[group_index].name;
                if group_name.is_empty() {
                    self.expanded.remove(&ExpandKey::Node(node_index));
                    self.rows_dirty = true;
                    self.ensure_visible_rows_cached();
                    if let Some(pos) = self.visible_rows().iter().position(
                        |r| matches!(r, VisibleRow::Root { node_index: ni } if *ni == node_index),
                    ) {
                        self.list_state.select(Some(pos));
                    }
                } else {
                    self.expanded
                        .remove(&ExpandKey::Group(node_index, group_index));
                    self.rows_dirty = true;
                    self.ensure_visible_rows_cached();
                    if let Some(pos) = self.visible_rows().iter().position(|r| {
                        matches!(r, VisibleRow::GroupHeader { node_index: ni, group_index: gi }
                            if *ni == node_index && *gi == group_index)
                    }) {
                        self.list_state.select(Some(pos));
                    }
                }
            },
            VisibleRow::WorktreeEntry { node_index, .. } => {
                self.expanded.remove(&ExpandKey::Node(node_index));
                self.rows_dirty = true;
                self.ensure_visible_rows_cached();
                if let Some(pos) = self.visible_rows().iter().position(
                    |r| matches!(r, VisibleRow::Root { node_index: ni } if *ni == node_index),
                ) {
                    self.list_state.select(Some(pos));
                }
            },
        }
    }

    fn row_count(&self) -> usize {
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

    pub(super) fn scan_log_to_top(&mut self) {
        if !self.scan_log.is_empty() {
            self.scan_log_state.select(Some(0));
        }
    }

    pub(super) fn scan_log_to_bottom(&mut self) {
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
            for wt in &node.worktrees {
                if wt.project.path == target_path {
                    self.expanded.insert(ExpandKey::Node(ni));
                }
                for group in &wt.groups {
                    for member in &group.members {
                        if member.path == target_path {
                            self.expanded.insert(ExpandKey::Node(ni));
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
                VisibleRow::GroupHeader { .. } => {},
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
        let node = self.nodes.iter().find(|n| n.project.path == project.path)?;
        if !node.has_members() {
            return None;
        }
        let mut counts = ProjectCounts::default();
        counts.add_project(&node.project);
        for group in &node.groups {
            for member in &group.members {
                counts.add_project(member);
            }
        }
        Some(counts)
    }

    pub fn formatted_disk(&self, project: &RustProject) -> String {
        match self.disk_usage.get(&project.path) {
            Some(&bytes) => super::render::format_bytes(bytes),
            None => "—".to_string(),
        }
    }

    pub fn ci_for(&self, project: &RustProject) -> String {
        self.ci_state
            .get(&project.path)
            .and_then(|s| s.runs().first())
            .map_or_else(|| "—".to_string(), |run| run.conclusion.clone())
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

    /// Aggregate CI for a node: ✓ if all green, ✗ if any red, — otherwise.
    pub fn ci_for_node(&self, node: &ProjectNode) -> String {
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
                if run.conclusion.contains('✗') {
                    any_red = true;
                    all_green = false;
                } else if !run.conclusion.contains('✓') {
                    all_green = false;
                }
            }
        }
        if !any_data {
            "—".to_string()
        } else if any_red {
            "✗".to_string()
        } else if all_green {
            "✓".to_string()
        } else {
            "—".to_string()
        }
    }

    pub fn ci_state_for(&self, project: &RustProject) -> Option<&CiState> {
        self.ci_state.get(&project.path)
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
        let Some((ahead, behind)) = info.ahead_behind else {
            return String::new();
        };
        match (ahead, behind) {
            (0, 0) => "✓".to_string(),
            (a, 0) => format!("↑{a}"),
            (0, b) => format!("↓{b}"),
            (a, b) => format!("↑{a}↓{b}"),
        }
    }

    /// Returns the Enter-key action label for the current cursor position,
    /// or `None` if Enter does nothing here. Used by the shortcut bar to
    /// only show Enter when it's actionable.
    pub fn enter_action(&self) -> Option<&'static str> {
        use super::shortcuts::InputContext;
        match self.input_context() {
            InputContext::ProjectList | InputContext::ScanLog => Some("open"),
            InputContext::DetailTargets => Some("run"),
            InputContext::DetailFields => {
                if self.detail_column.pos() == 0 {
                    let info = self
                        .selected_project()
                        .map(|p| super::detail::build_detail_info(self, p))?;
                    let fields = super::detail::package_fields(&info);
                    let field = *fields.get(self.detail_cursor.pos())?;
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
                    match fields.get(self.detail_cursor.pos()) {
                        Some(DetailField::Repo) if info.git_url.is_some() => Some("open"),
                        _ => None,
                    }
                }
            },
            InputContext::CiRuns => {
                let ci_state = self.selected_project().and_then(|p| self.ci_state_for(p));
                let run_count = ci_state.map_or(0, |s| s.runs().len());
                if self.ci_runs_cursor.pos() == run_count
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
