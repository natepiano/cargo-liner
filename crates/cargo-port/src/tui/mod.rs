mod detail;
mod render;
mod scan;
mod settings;

use std::collections::HashMap;
use std::collections::HashSet;
use std::io;
use std::io::Stdout;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::mpsc;
use std::sync::mpsc::Receiver;
use std::thread;
use std::time::Duration;

use crossterm::event::DisableMouseCapture;
use crossterm::event::EnableMouseCapture;
use crossterm::event::Event;
use crossterm::event::KeyCode;
use crossterm::event::MouseEventKind;
use crossterm::execute;
use crossterm::terminal::EnterAlternateScreen;
use crossterm::terminal::LeaveAlternateScreen;
use crossterm::terminal::disable_raw_mode;
use crossterm::terminal::enable_raw_mode;
use detail::EditingState;
use detail::PendingCiFetch;
use detail::PendingExampleRun;
use detail::RunTargetKind;
use detail::handle_ci_runs_key;
use detail::handle_detail_key;
use detail::handle_field_edit_key;
use nucleo_matcher::Matcher;
use nucleo_matcher::Utf32Str;
use nucleo_matcher::pattern::Atom;
use nucleo_matcher::pattern::AtomKind;
use nucleo_matcher::pattern::CaseMatching;
use nucleo_matcher::pattern::Normalization;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::widgets::ListState;
use render::format_bytes;
use render::ui;
use scan::build_flat_entries;
use scan::build_tree;
use scan::spawn_streaming_scan;
use settings::handle_settings_key;

use crate::ci::CiRun;
use crate::config;
use crate::config::Config;
use crate::project::GitInfo;
use crate::project::ProjectType;
use crate::project::RustProject;

#[derive(Default, PartialEq, Eq, Clone, Copy)]
pub enum FocusTarget {
    #[default]
    ProjectList,
    DetailFields,
    CiRuns,
    ScanLog,
}

/// An expand key: either a workspace node or a group within a node.
#[derive(Hash, Eq, PartialEq, Clone)]
pub enum ExpandKey {
    Node(usize),
    Group(usize, usize),
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

/// Members within a workspace are organized into groups by their first subdirectory.
/// The "inline" group (empty name) contains members directly under the workspace root
/// or under the primary `crates/` directory — these are shown without a folder header.
pub struct MemberGroup {
    pub name:    String,
    pub members: Vec<RustProject>,
}

pub struct ProjectNode {
    pub project:   RustProject,
    pub groups:    Vec<MemberGroup>,
    pub worktrees: Vec<Self>,
    pub vendored:  Vec<RustProject>,
}

impl ProjectNode {
    pub fn has_members(&self) -> bool { self.groups.iter().any(|g| !g.members.is_empty()) }

    pub fn has_children(&self) -> bool {
        self.has_members() || !self.worktrees.is_empty() || !self.vendored.is_empty()
    }
}

/// A flattened entry for fuzzy search.
pub struct FlatEntry {
    pub node_index:   usize,
    pub group_index:  usize,
    pub member_index: usize,
    pub name:         String,
}

pub enum ExampleMsg {
    Output(String),
    Finished,
}

pub enum BackgroundMsg {
    DiskUsage { path: String, bytes: u64 },
    CiRuns { path: String, runs: Vec<CiRun> },
    GitInfo { path: String, info: GitInfo },
    CratesIoVersion { path: String, version: String },
    Stars { path: String, count: u64 },
    ProjectDiscovered { project: RustProject },
    ScanActivity { path: String },
    ScanComplete,
}

impl BackgroundMsg {
    /// Returns the project path this message relates to, if any.
    fn path(&self) -> Option<&str> {
        match self {
            Self::DiskUsage { path, .. }
            | Self::CiRuns { path, .. }
            | Self::GitInfo { path, .. }
            | Self::CratesIoVersion { path, .. }
            | Self::Stars { path, .. } => Some(path),
            Self::ProjectDiscovered { project } => Some(&project.path),
            Self::ScanActivity { .. } | Self::ScanComplete => None,
        }
    }
}

/// Message sent when a background CI fetch completes.
pub enum CiFetchMsg {
    /// The fetch completed with updated runs for the given project path.
    Complete { path: String, runs: Vec<CiRun> },
}

#[derive(Default)]
pub struct ProjectCounts {
    pub workspaces:  usize,
    pub libs:        usize,
    pub bins:        usize,
    pub proc_macros: usize,
    pub examples:    usize,
    pub benches:     usize,
    pub tests:       usize,
}

impl ProjectCounts {
    pub fn add_project(&mut self, project: &RustProject) {
        if project.is_workspace() {
            self.workspaces += 1;
        }
        for t in &project.types {
            match t {
                ProjectType::Library => self.libs += 1,
                ProjectType::Binary => self.bins += 1,
                ProjectType::ProcMacro => self.proc_macros += 1,
                ProjectType::BuildScript => {},
            }
        }
        self.examples += project.example_count();
        self.benches += project.benches.len();
        self.tests += project.test_count;
    }

    pub fn summary(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if self.workspaces > 0 {
            parts.push(format!("{} ws", self.workspaces));
        }
        if self.libs > 0 {
            parts.push(format!("{} lib", self.libs));
        }
        if self.bins > 0 {
            parts.push(format!("{} bin", self.bins));
        }
        if self.proc_macros > 0 {
            parts.push(format!("{} proc", self.proc_macros));
        }
        if self.examples > 0 {
            parts.push(format!("{} ex", self.examples));
        }
        if self.benches > 0 {
            parts.push(format!("{} bench", self.benches));
        }
        if self.tests > 0 {
            parts.push(format!("{} test", self.tests));
        }
        parts.join("  ")
    }

    /// Returns non-zero stats as (label, count) pairs for column display.
    pub fn to_rows(&self) -> Vec<(&'static str, usize)> {
        let mut rows = Vec::new();
        if self.workspaces > 0 {
            rows.push(("ws", self.workspaces));
        }
        if self.libs > 0 {
            rows.push(("lib", self.libs));
        }
        if self.bins > 0 {
            rows.push(("bin", self.bins));
        }
        if self.proc_macros > 0 {
            rows.push(("proc-macro", self.proc_macros));
        }
        if self.examples > 0 {
            rows.push(("example", self.examples));
        }
        if self.benches > 0 {
            rows.push(("bench", self.benches));
        }
        if self.tests > 0 {
            rows.push(("test", self.tests));
        }
        rows
    }
}

#[allow(clippy::struct_excessive_bools)]
pub struct App {
    pub scan_root:           PathBuf,
    pub inline_dirs:         Vec<String>,
    pub exclude_dirs:        Vec<String>,
    pub ci_run_count:        u32,
    pub include_non_rust:    bool,
    pub owned_owners:        Vec<String>,
    pub all_projects:        Vec<RustProject>,
    pub nodes:               Vec<ProjectNode>,
    pub flat_entries:        Vec<FlatEntry>,
    pub disk_usage:          HashMap<String, u64>,
    pub ci_runs:             HashMap<String, Vec<CiRun>>,
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
    pub settings_cursor:     usize,
    pub settings_editing:    bool,
    pub settings_edit_buf:   String,
    pub scan_complete:       bool,
    pub scan_log:            Vec<String>,
    pub scan_log_state:      ListState,
    pub focus:               FocusTarget,
    pub detail_column:       usize,
    pub detail_cursor:       usize,
    pub ci_runs_cursor:      usize,
    pub examples_scroll:     usize,
    pub editing:             Option<EditingState>,
    pub pending_example_run: Option<PendingExampleRun>,
    pub pending_ci_fetch:    Option<PendingCiFetch>,
    pub ci_fetching:         bool,
    pub ci_fetch_count:      u32,
    pub ci_fetch_prev_count: usize,
    pub ci_no_more_runs:     HashSet<String>,
    pub spinner_tick:        usize,
    pub ci_fetch_tx:         mpsc::Sender<CiFetchMsg>,
    pub ci_fetch_rx:         mpsc::Receiver<CiFetchMsg>,
    pub example_running:     Option<String>,
    pub example_child:       Arc<Mutex<Option<u32>>>,
    pub example_output:      Vec<String>,
    pub example_tx:          mpsc::Sender<ExampleMsg>,
    pub example_rx:          mpsc::Receiver<ExampleMsg>,
    pub last_selected_path:  Option<String>,
    pub should_quit:         bool,

    // Caches for per-frame hot paths
    pub cached_visible_rows:       Vec<VisibleRow>,
    pub rows_dirty:                bool,
    pub cached_root_sorted:        Vec<u64>,
    pub cached_child_sorted:       HashMap<usize, Vec<u64>>,
    pub disk_cache_dirty:          bool,
    pub(super) cached_detail_path: String,
    pub(super) cached_detail_info: Option<detail::DetailInfo>,
    pub(super) detail_dirty:       bool,
    pub(super) selection_changed:  bool,
}

impl App {
    fn new(
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
        let owned_owners = cfg.tui.owned_owners.clone();
        let nodes = build_tree(projects.clone(), &inline_dirs);
        let flat_entries = build_flat_entries(&nodes);
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
            owned_owners,
            all_projects: projects,
            nodes,
            flat_entries,
            disk_usage: HashMap::new(),
            ci_runs: HashMap::new(),
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
            settings_cursor: 0,
            settings_editing: false,
            settings_edit_buf: String::new(),
            scan_complete: false,
            scan_log: Vec::new(),
            scan_log_state: ListState::default(),
            focus: FocusTarget::ProjectList,
            detail_column: 0,
            detail_cursor: 0,
            ci_runs_cursor: 0,
            examples_scroll: 0,
            editing: None,
            pending_example_run: None,
            pending_ci_fetch: None,
            ci_fetching: false,
            ci_fetch_count: 0,
            ci_fetch_prev_count: 0,
            ci_no_more_runs: HashSet::new(),
            spinner_tick: 0,
            ci_fetch_tx,
            ci_fetch_rx,
            example_running: None,
            example_child: Arc::new(Mutex::new(None)),
            example_output: Vec::new(),
            example_tx,
            example_rx,
            last_selected_path: load_last_selected(),
            should_quit: false,

            cached_visible_rows: Vec::new(),
            rows_dirty: true,
            cached_root_sorted: Vec::new(),
            cached_child_sorted: HashMap::new(),
            disk_cache_dirty: true,
            cached_detail_path: String::new(),
            cached_detail_info: None,
            detail_dirty: true,
            selection_changed: false,
        }
    }

    pub fn rebuild_tree(&mut self) {
        let selected_path = self
            .selected_project()
            .map(|p| p.path.clone())
            .or_else(|| self.last_selected_path.clone());
        self.nodes = build_tree(self.all_projects.clone(), &self.inline_dirs);
        self.flat_entries = build_flat_entries(&self.nodes);
        self.rows_dirty = true;
        self.disk_cache_dirty = true;
        self.detail_dirty = true;

        // Re-run search if active so filtered indices match new flat_entries
        if self.searching && !self.search_query.is_empty() {
            let query = self.search_query.clone();
            self.update_search(&query);
        } else {
            self.filtered.clear();
        }

        // Propagate CI runs, git info, and stars from workspace roots to their members
        for node in &self.nodes {
            if let Some(runs) = self.ci_runs.get(&node.project.path).cloned() {
                for group in &node.groups {
                    for member in &group.members {
                        self.ci_runs
                            .entry(member.path.clone())
                            .or_insert_with(|| runs.clone());
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

    fn rescan(&mut self) {
        self.all_projects.clear();
        self.nodes.clear();
        self.flat_entries.clear();
        self.disk_usage.clear();
        self.ci_runs.clear();
        self.git_info.clear();
        self.crates_versions.clear();
        self.stars.clear();
        self.scan_log.clear();
        self.scan_log_state = ListState::default();
        self.scan_complete = false;
        self.fully_loaded.clear();
        self.priority_fetch_path = None;
        self.focus = FocusTarget::ProjectList;
        self.detail_column = 0;
        self.detail_cursor = 0;
        self.ci_runs_cursor = 0;
        self.examples_scroll = 0;
        self.editing = None;
        self.pending_ci_fetch = None;
        self.expanded.clear();
        self.list_state = ListState::default();
        self.rows_dirty = true;
        self.disk_cache_dirty = true;
        self.detail_dirty = true;
        let (tx, rx) = spawn_streaming_scan(
            &self.scan_root,
            self.ci_run_count,
            &self.exclude_dirs,
            self.include_non_rust,
        );
        self.bg_tx = tx;
        self.bg_rx = rx;
    }

    fn poll_background(&mut self) {
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
                CiFetchMsg::Complete { path, runs } => {
                    self.handle_ci_fetch_complete(path, runs);
                },
            }
        }

        // Poll example process output
        while let Ok(msg) = self.example_rx.try_recv() {
            match msg {
                ExampleMsg::Output(line) => {
                    self.example_output.push(line);
                },
                ExampleMsg::Finished => {
                    self.example_running = None;
                    self.example_output.push("── done ──".to_string());
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
        // Only mark detail dirty if this message is for the selected project
        let selected_path = self.cached_detail_path.clone();
        if let Some(msg_path) = msg.path()
            && msg_path == selected_path
        {
            self.detail_dirty = true;
        }
        match msg {
            BackgroundMsg::DiskUsage { path, bytes } => {
                self.fully_loaded.insert(path.clone());
                self.disk_usage.insert(path, bytes);
                self.disk_cache_dirty = true;
            },
            BackgroundMsg::CiRuns { path, runs } => {
                if let Some(git) = self.git_info.get(&path)
                    && let Some(ref url) = git.url
                    && let Some((owner, repo)) = crate::ci::parse_owner_repo(url)
                    && scan::is_exhausted(&owner, &repo)
                {
                    self.ci_no_more_runs.insert(path.clone());
                }
                if let Some(node) = self.nodes.iter().find(|n| n.project.path == path) {
                    for group in &node.groups {
                        for member in &group.members {
                            self.ci_runs
                                .entry(member.path.clone())
                                .or_insert_with(|| runs.clone());
                        }
                    }
                }
                self.ci_runs.insert(path, runs);
            },
            BackgroundMsg::GitInfo { path, info } => {
                // Propagate to workspace members
                if let Some(node) = self.nodes.iter().find(|n| n.project.path == path) {
                    for group in &node.groups {
                        for member in &group.members {
                            self.git_info
                                .entry(member.path.clone())
                                .or_insert_with(|| info.clone());
                        }
                    }
                    for wt in &node.worktrees {
                        self.git_info
                            .entry(wt.project.path.clone())
                            .or_insert_with(|| info.clone());
                    }
                }
                self.git_info.insert(path, info);
            },
            BackgroundMsg::CratesIoVersion { path, version } => {
                self.crates_versions.insert(path, version);
            },
            BackgroundMsg::Stars { path, count } => {
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
        }
        false
    }

    /// Process a completed CI fetch: merge runs, detect exhaustion, propagate to members.
    fn handle_ci_fetch_complete(&mut self, path: String, runs: Vec<CiRun>) {
        self.ci_fetching = false;

        let existing = self.ci_runs.remove(&path).unwrap_or_default();
        let mut seen = HashSet::new();
        let mut merged: Vec<CiRun> = Vec::new();
        for run in runs {
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

        if merged.len() <= self.ci_fetch_prev_count {
            self.ci_no_more_runs.insert(path.clone());
            if let Some(git) = self.git_info.get(&path)
                && let Some(ref url) = git.url
                && let Some((owner, repo)) = crate::ci::parse_owner_repo(url)
            {
                scan::mark_exhausted(&owner, &repo);
            }
        } else {
            self.ci_no_more_runs.remove(&path);
        }

        if let Some(node) = self.nodes.iter().find(|n| n.project.path == path) {
            for group in &node.groups {
                for member in &group.members {
                    self.ci_runs
                        .entry(member.path.clone())
                        .or_insert_with(|| merged.clone());
                }
            }
        }
        self.ci_runs_cursor = merged.len();
        self.ci_runs.insert(path, merged);
    }

    /// Spawn a priority fetch for the selected project if it hasn't been loaded yet.
    fn maybe_priority_fetch(&mut self) {
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
            spawn_priority_fetch(self, &path, &abs_path, name.as_ref());
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
    pub fn ensure_detail_cached(&mut self) {
        let current_path = self
            .selected_project()
            .map(|p| p.path.clone())
            .unwrap_or_default();

        if !self.detail_dirty && self.cached_detail_path == current_path {
            return;
        }

        self.cached_detail_path = current_path;
        self.cached_detail_info = self
            .selected_project()
            .map(|p| detail::build_detail_info(self, p));
        self.detail_dirty = false;
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

    fn expand(&mut self) {
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

    fn collapse(&mut self) {
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

    fn move_up(&mut self) {
        let count = self.row_count();
        if count == 0 {
            return;
        }
        let current = self.list_state.selected().unwrap_or(0);
        if current > 0 {
            self.list_state.select(Some(current - 1));
        }
    }

    fn move_down(&mut self) {
        let count = self.row_count();
        if count == 0 {
            return;
        }
        let current = self.list_state.selected().unwrap_or(0);
        if current < count - 1 {
            self.list_state.select(Some(current + 1));
        }
    }

    fn move_to_top(&mut self) {
        if self.row_count() > 0 {
            self.list_state.select(Some(0));
        }
    }

    fn move_to_bottom(&mut self) {
        let count = self.row_count();
        if count > 0 {
            self.list_state.select(Some(count - 1));
        }
    }

    fn scan_log_scroll_up(&mut self) {
        if self.scan_log.is_empty() {
            return;
        }
        let current = self.scan_log_state.selected().unwrap_or(0);
        if current > 0 {
            self.scan_log_state.select(Some(current - 1));
        }
    }

    fn scan_log_scroll_down(&mut self) {
        if self.scan_log.is_empty() {
            return;
        }
        let current = self.scan_log_state.selected().unwrap_or(0);
        if current < self.scan_log.len() - 1 {
            self.scan_log_state.select(Some(current + 1));
        }
    }

    fn scan_log_to_top(&mut self) {
        if !self.scan_log.is_empty() {
            self.scan_log_state.select(Some(0));
        }
    }

    fn scan_log_to_bottom(&mut self) {
        if !self.scan_log.is_empty() {
            self.scan_log_state
                .select(Some(self.scan_log.len().saturating_sub(1)));
        }
    }

    fn start_search(&mut self) {
        self.searching = true;
        self.search_query.clear();
        self.filtered.clear();
        self.rows_dirty = true;
    }

    fn cancel_search(&mut self) {
        self.searching = false;
        self.search_query.clear();
        self.filtered.clear();
        self.rows_dirty = true;
        if !self.nodes.is_empty() {
            self.list_state.select(Some(0));
        }
    }

    fn confirm_search(&mut self) {
        let project_path = self.selected_project().map(|p| p.path.clone());
        self.searching = false;
        self.search_query.clear();
        self.filtered.clear();
        self.rows_dirty = true;

        if let Some(target_path) = project_path {
            self.select_project_in_tree(&target_path);
        }
    }

    fn select_project_in_tree(&mut self, target_path: &str) {
        // Expand the containing node and group
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
        }

        self.rows_dirty = true;
        self.ensure_visible_rows_cached();
        let rows = self.visible_rows();
        for (i, row) in rows.iter().enumerate() {
            if let VisibleRow::Member {
                node_index,
                group_index,
                member_index,
            } = row
            {
                let project = &self.nodes[*node_index].groups[*group_index].members[*member_index];
                if project.path == target_path {
                    self.list_state.select(Some(i));
                    return;
                }
            }
            if let VisibleRow::Root { node_index } = row
                && self.nodes[*node_index].project.path == target_path
            {
                self.list_state.select(Some(i));
                return;
            }
        }
    }

    fn update_search(&mut self, query: &str) {
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

    pub fn max_name_width(&self) -> usize {
        let mut max_width = 0usize;
        for node in &self.nodes {
            let name = node.project.display_name();
            // Root items: "▶ " or "  " prefix = 2 display chars
            max_width = max_width.max(2 + name.len());
            for group in &node.groups {
                if group.name.is_empty() {
                    // Inline members: "    " prefix = 4 chars
                    for member in &group.members {
                        let name = member.display_name();
                        max_width = max_width.max(4 + name.len());
                    }
                } else {
                    // Group header: "    ▶ " prefix = 6 display chars + name + count
                    let label_len = 6 + group.name.len() + 4; // " (NN)"
                    max_width = max_width.max(label_len);
                    // Group members: "        " prefix = 8 chars
                    for member in &group.members {
                        let name = member.display_name();
                        max_width = max_width.max(8 + name.len());
                    }
                }
            }
            // Worktree entries: "        " prefix = 8 chars
            for wt in &node.worktrees {
                let wt_name = wt
                    .project
                    .worktree_name
                    .as_deref()
                    .unwrap_or(&wt.project.path);
                max_width = max_width.max(8 + wt_name.len());
            }
        }
        max_width
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
            Some(&bytes) => format_bytes(bytes),
            None => "—".to_string(),
        }
    }

    pub fn ci_for(&self, project: &RustProject) -> String {
        self.ci_runs
            .get(&project.path)
            .and_then(|runs| runs.first())
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
            format_bytes(total)
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
            if let Some(runs) = self.ci_runs.get(path)
                && let Some(run) = runs.first()
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

    pub fn ci_runs_for(&self, project: &RustProject) -> Option<&Vec<CiRun>> {
        self.ci_runs.get(&project.path)
    }

    pub fn git_icon(&self, project: &RustProject) -> &'static str {
        self.git_info
            .get(&project.path)
            .map_or(" ", |info| info.origin.icon())
    }
}

fn setup_terminal() -> io::Result<Terminal<CrosstermBackend<Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    Terminal::new(backend)
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> io::Result<()> {
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    Ok(())
}

pub fn run(path: PathBuf) -> ExitCode {
    let Ok(scan_root) = path.canonicalize() else {
        eprintln!("Error: cannot resolve path '{}'", path.display());
        return ExitCode::FAILURE;
    };

    let cfg = config::load();
    let (bg_tx, bg_rx) = spawn_streaming_scan(
        &scan_root,
        cfg.tui.ci_run_count,
        &cfg.tui.exclude_dirs,
        cfg.tui.include_non_rust,
    );
    let projects: Vec<RustProject> = Vec::new();

    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture);
        original_hook(panic_info);
    }));

    let mut terminal = match setup_terminal() {
        Ok(t) => t,
        Err(e) => {
            eprintln!("Error: failed to initialize terminal: {e}");
            return ExitCode::FAILURE;
        },
    };

    let mut app = App::new(scan_root, projects, bg_tx, bg_rx, &cfg);

    let result = event_loop(&mut terminal, &mut app);

    let _ = restore_terminal(&mut terminal);

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("Error: {e}");
            ExitCode::FAILURE
        },
    }
}

fn handle_event(app: &mut App, event: Event) {
    match event {
        Event::Key(key) => {
            // Esc: if running, kill process (keep output). If not running, clear output.
            if key.code == KeyCode::Esc && app.example_running.is_some() {
                // First Esc: kill the process, keep output visible
                let pid = *app
                    .example_child
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if let Some(pid) = pid {
                    let _ = std::process::Command::new("kill")
                        .arg(pid.to_string())
                        .output();
                }
                app.example_running = None;
                app.example_output.push("── killed ──".to_string());
                return;
            }
            if key.code == KeyCode::Esc && !app.example_output.is_empty() {
                // Second Esc: clear the output panel
                app.example_output.clear();
                return;
            }
            if app.show_settings {
                handle_settings_key(app, key.code);
            } else if app.editing.is_some() {
                handle_field_edit_key(app, key.code);
            } else if app.searching {
                handle_search_key(app, key.code);
            } else if app.focus == FocusTarget::DetailFields {
                handle_detail_key(app, key.code);
            } else if app.focus == FocusTarget::CiRuns {
                handle_ci_runs_key(app, key.code);
            } else {
                handle_normal_key(app, key.code);
            }
        },
        Event::Mouse(mouse) => match mouse.kind {
            MouseEventKind::ScrollUp => {
                if app.focus == FocusTarget::ScanLog {
                    if app.invert_scroll {
                        app.scan_log_scroll_down();
                    } else {
                        app.scan_log_scroll_up();
                    }
                } else if app.invert_scroll {
                    app.move_down();
                } else {
                    app.move_up();
                }
            },
            MouseEventKind::ScrollDown => {
                if app.focus == FocusTarget::ScanLog {
                    if app.invert_scroll {
                        app.scan_log_scroll_up();
                    } else {
                        app.scan_log_scroll_down();
                    }
                } else if app.invert_scroll {
                    app.move_up();
                } else {
                    app.move_down();
                }
            },
            _ => {},
        },
        _ => {},
    }

    // Track project selection changes for session persistence
    if app.focus == FocusTarget::ProjectList {
        track_selection(app);
    }
}

fn event_loop(terminal: &mut Terminal<CrosstermBackend<Stdout>>, app: &mut App) -> io::Result<()> {
    loop {
        app.poll_background();
        app.spinner_tick = app.spinner_tick.wrapping_add(1);
        app.ensure_visible_rows_cached();
        app.ensure_disk_cache();
        app.ensure_detail_cached();
        terminal.draw(|frame| ui(frame, app))?;

        // Wait for at least one event (up to 16ms for ~60fps)
        if crossterm::event::poll(Duration::from_millis(16))? {
            handle_event(app, crossterm::event::read()?);

            // Drain any additional queued events without waiting
            while crossterm::event::poll(Duration::ZERO)? {
                handle_event(app, crossterm::event::read()?);
                if app.should_quit {
                    return Ok(());
                }
            }
        } else if app.selection_changed {
            // No events this frame — flush deferred selection save to disk
            if let Some(path) = &app.last_selected_path {
                save_last_selected(path);
            }
            app.selection_changed = false;
        }

        if app.should_quit {
            // Flush any pending selection save
            if app.selection_changed
                && let Some(path) = &app.last_selected_path
            {
                save_last_selected(path);
            }
            break;
        }

        // Spawn a pending example as a background process
        if let Some(run) = app.pending_example_run.take() {
            spawn_example_process(app, &run);
        }

        // Spawn a pending CI fetch as a background process
        if let Some(fetch) = app.pending_ci_fetch.take() {
            app.ci_fetching = true;
            app.ci_fetch_count = 5;
            app.ci_fetch_prev_count = app.ci_runs.get(&fetch.project_path).map_or(0, Vec::len);
            spawn_ci_fetch(app, &fetch);
        }
    }
    Ok(())
}

fn spawn_example_process(app: &mut App, run: &PendingExampleRun) {
    use std::io::BufRead;
    use std::io::BufReader;
    use std::process::Stdio;

    let mut cmd = std::process::Command::new("cargo");
    match run.kind {
        RunTargetKind::Binary => {
            cmd.arg("run");
        },
        RunTargetKind::Example => {
            cmd.arg("run").arg("--example").arg(&run.target_name);
        },
        RunTargetKind::Bench => {
            cmd.arg("bench").arg("--bench").arg(&run.target_name);
        },
    }
    if run.release {
        cmd.arg("--release");
    }
    if let Some(pkg) = &run.package_name {
        cmd.arg("-p").arg(pkg);
    }
    cmd.current_dir(&run.abs_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            app.example_output = vec![format!("Failed to start: {e}")];
            app.example_running = Some(run.target_name.clone());
            return;
        },
    };

    // Store PID so we can kill from the main thread
    let pid = child.id();
    *app.example_child
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(pid);

    let name = run.target_name.clone();
    let mode = if run.release { " (release)" } else { "" };
    app.example_output = vec![format!("Building {name}{mode}...")];
    app.example_running = Some(format!("{name}{mode}"));

    // Take ownership of pipes before moving child to thread
    let stderr = child.stderr.take();
    let stdout = child.stdout.take();

    let pid_holder = app.example_child.clone();
    let tx = app.example_tx.clone();
    thread::spawn(move || {
        // Read stderr (cargo output goes here)
        if let Some(stderr) = stderr {
            let reader = BufReader::new(stderr);
            for line in reader.lines().map_while(Result::ok) {
                let _ = tx.send(ExampleMsg::Output(line));
            }
        }
        // Read stdout
        if let Some(stdout) = stdout {
            let reader = BufReader::new(stdout);
            for line in reader.lines().map_while(Result::ok) {
                let _ = tx.send(ExampleMsg::Output(line));
            }
        }

        // Wait for the child to finish and clear the PID
        let _ = child.wait();
        *pid_holder
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
        let _ = tx.send(ExampleMsg::Finished);
    });
}

fn spawn_ci_fetch(app: &App, fetch: &PendingCiFetch) {
    use scan::fetch_older_runs;

    let tx = app.ci_fetch_tx.clone();
    let abs_path = fetch.abs_path.clone();
    let project_path = fetch.project_path.clone();
    let current_count = fetch.current_count;

    thread::spawn(move || {
        let repo_dir = PathBuf::from(&abs_path);
        let runs = fetch_older_runs(&repo_dir, current_count);
        let _ = tx.send(CiFetchMsg::Complete {
            path: project_path,
            runs,
        });
    });
}

fn last_selected_path_file() -> Option<PathBuf> {
    scan::cache_dir().map(|d| d.join("last_selected.txt"))
}

fn load_last_selected() -> Option<String> {
    let path = last_selected_path_file()?;
    std::fs::read_to_string(path).ok().filter(|s| !s.is_empty())
}

fn save_last_selected(project_path: &str) {
    if let Some(path) = last_selected_path_file() {
        let _ = std::fs::write(path, project_path);
    }
}

/// Update the last selected path when the user navigates.
/// If the scan is still running and the selected project doesn't have details yet,
/// spawn a priority fetch to load its data immediately.
fn track_selection(app: &mut App) {
    if let Some(project) = app.selected_project() {
        let path = project.path.clone();
        if app.last_selected_path.as_ref() != Some(&path) {
            app.detail_dirty = true;
            app.last_selected_path = Some(path);
            // Disk write deferred to save_selection_on_idle / quit
            app.selection_changed = true;
            app.maybe_priority_fetch();
        }
    }
}

/// Spawn a background thread to fetch details for a single project ahead of the main scan.
fn spawn_priority_fetch(app: &App, path: &str, abs_path: &str, name: Option<&String>) {
    use crate::project::GitInfo;

    let tx = app.bg_tx.clone();
    let project_path = path.to_string();
    let abs = PathBuf::from(abs_path);
    let has_git = abs.join(".git").exists();
    let ci_run_count = app.ci_run_count;
    let project_name = name.cloned();

    // Git info is local and instant — fetch on a separate thread immediately
    if has_git {
        let tx_git = tx.clone();
        let path_git = project_path.clone();
        let abs_git = abs.clone();
        thread::spawn(move || {
            if let Some(info) = GitInfo::detect(&abs_git) {
                let _ = tx_git.send(BackgroundMsg::GitInfo {
                    path: path_git,
                    info,
                });
            }
        });
    }

    // CI runs from cache are also fast — separate thread
    if has_git {
        let tx_ci = tx.clone();
        let path_ci = project_path.clone();
        let abs_ci = abs.clone();
        thread::spawn(move || {
            let runs = scan::fetch_ci_runs_cached(&abs_ci, ci_run_count);
            let _ = tx_ci.send(BackgroundMsg::CiRuns {
                path: path_ci,
                runs,
            });
        });
    }

    // Disk + crates.io on another thread (slower)
    thread::spawn(move || {
        let bytes = scan::dir_size(&abs);
        let _ = tx.send(BackgroundMsg::DiskUsage {
            path: project_path.clone(),
            bytes,
        });

        if let Some(name) = project_name.as_ref()
            && let Some(version) = scan::fetch_crates_io_version(name)
        {
            let _ = tx.send(BackgroundMsg::CratesIoVersion {
                path: project_path,
                version,
            });
        }
    });
}

fn handle_normal_key(app: &mut App, key: KeyCode) {
    match key {
        KeyCode::Char('q') => app.should_quit = true,
        KeyCode::Tab => advance_focus(app),
        KeyCode::BackTab => reverse_focus(app),
        KeyCode::Up => {
            if app.focus == FocusTarget::ScanLog {
                app.scan_log_scroll_up();
            } else {
                app.move_up();
            }
        },
        KeyCode::Down => {
            if app.focus == FocusTarget::ScanLog {
                app.scan_log_scroll_down();
            } else {
                app.move_down();
            }
        },
        KeyCode::Home => {
            if app.focus == FocusTarget::ScanLog {
                app.scan_log_to_top();
            } else {
                app.move_to_top();
            }
        },
        KeyCode::End => {
            if app.focus == FocusTarget::ScanLog {
                app.scan_log_to_bottom();
            } else {
                app.move_to_bottom();
            }
        },
        KeyCode::Enter | KeyCode::Right => app.expand(),
        KeyCode::Left => app.collapse(),
        KeyCode::Char('/') => app.start_search(),
        KeyCode::Char('s') => app.show_settings = true,
        KeyCode::Char('r') => app.rescan(),
        _ => {},
    }
}

fn handle_search_key(app: &mut App, key: KeyCode) {
    match key {
        KeyCode::Esc => app.cancel_search(),
        KeyCode::Enter => app.confirm_search(),
        KeyCode::Tab => advance_focus(app),
        KeyCode::BackTab => reverse_focus(app),
        KeyCode::Up => app.move_up(),
        KeyCode::Down => app.move_down(),
        KeyCode::Backspace => {
            let mut query = app.search_query.clone();
            query.pop();
            app.update_search(&query);
        },
        KeyCode::Char(c) => {
            let query = format!("{}{c}", app.search_query);
            app.update_search(&query);
        },
        _ => {},
    }
}

pub fn advance_focus(app: &mut App) {
    use detail::detail_layout_pub;
    use detail::detail_max_column;

    let has_ci = app.selected_project().is_some_and(|p| {
        app.ci_runs.get(&p.path).is_some_and(|r| !r.is_empty())
            || app.git_info.get(&p.path).is_some_and(|g| g.url.is_some())
    });

    let max_detail_col = detail_max_column(app);

    app.focus = match app.focus {
        FocusTarget::ProjectList => {
            app.detail_column = 0;
            app.detail_cursor = 0;
            FocusTarget::DetailFields
        },
        FocusTarget::DetailFields => {
            // Advance through detail columns first
            if app.detail_column < max_detail_col {
                app.detail_column += 1;
                app.detail_cursor = 0;
                let (_, targets_col) = detail_layout_pub(app);
                if Some(app.detail_column) == targets_col {
                    app.examples_scroll = 0;
                }
                FocusTarget::DetailFields
            } else if has_ci {
                app.ci_runs_cursor = 0;
                FocusTarget::CiRuns
            } else if app.scan_complete {
                FocusTarget::ProjectList
            } else {
                FocusTarget::ScanLog
            }
        },
        FocusTarget::CiRuns => {
            if app.scan_complete {
                FocusTarget::ProjectList
            } else {
                FocusTarget::ScanLog
            }
        },
        FocusTarget::ScanLog => FocusTarget::ProjectList,
    };

    if app.focus == FocusTarget::ScanLog
        && !app.scan_log.is_empty()
        && app.scan_log_state.selected().is_none()
    {
        app.scan_log_state
            .select(Some(app.scan_log.len().saturating_sub(1)));
    }
}

pub fn reverse_focus(app: &mut App) {
    use detail::detail_layout_pub;
    use detail::detail_max_column;

    let has_ci = app.selected_project().is_some_and(|p| {
        app.ci_runs.get(&p.path).is_some_and(|r| !r.is_empty())
            || app.git_info.get(&p.path).is_some_and(|g| g.url.is_some())
    });

    let max_detail_col = detail_max_column(app);
    let (_, targets_col) = detail_layout_pub(app);

    app.focus = match app.focus {
        FocusTarget::ProjectList => {
            if !app.scan_complete {
                FocusTarget::ScanLog
            } else if has_ci {
                app.ci_runs_cursor = 0;
                FocusTarget::CiRuns
            } else {
                app.detail_column = max_detail_col;
                app.detail_cursor = 0;
                if Some(max_detail_col) == targets_col {
                    app.examples_scroll = 0;
                }
                FocusTarget::DetailFields
            }
        },
        FocusTarget::DetailFields => {
            // Reverse through detail columns first
            if app.detail_column > 0 {
                app.detail_column -= 1;
                app.detail_cursor = 0;
                FocusTarget::DetailFields
            } else {
                FocusTarget::ProjectList
            }
        },
        FocusTarget::CiRuns => {
            app.detail_column = max_detail_col;
            app.detail_cursor = 0;
            if Some(max_detail_col) == targets_col {
                app.examples_scroll = 0;
            }
            FocusTarget::DetailFields
        },
        FocusTarget::ScanLog => {
            if has_ci {
                app.ci_runs_cursor = 0;
                FocusTarget::CiRuns
            } else {
                app.detail_column = max_detail_col;
                app.detail_cursor = 0;
                if Some(max_detail_col) == targets_col {
                    app.examples_scroll = 0;
                }
                FocusTarget::DetailFields
            }
        },
    };

    if app.focus == FocusTarget::ScanLog
        && !app.scan_log.is_empty()
        && app.scan_log_state.selected().is_none()
    {
        app.scan_log_state
            .select(Some(app.scan_log.len().saturating_sub(1)));
    }
}
