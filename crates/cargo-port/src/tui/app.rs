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

use super::detail::EditingState;
use super::detail::PendingCiFetch;
use super::detail::PendingExampleRun;
use super::scan;
use super::types::BackgroundMsg;
use super::types::CiFetchMsg;
use super::types::ExampleMsg;
use super::types::ExpandKey;
use super::types::FlatEntry;
use super::types::FocusTarget;
use super::types::ProjectCounts;
use super::types::ProjectNode;
use super::types::VisibleRow;
use crate::ci::CiRun;
use crate::config::Config;
use crate::project::GitInfo;
use crate::project::RustProject;

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
    pub(super) cached_detail_info: Option<super::detail::DetailInfo>,
    pub(super) detail_dirty:       bool,
    pub(super) selection_changed:  bool,
}

impl App {
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
        let owned_owners = cfg.tui.owned_owners.clone();
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
            last_selected_path: super::terminal::load_last_selected(),
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
        self.nodes = scan::build_tree(self.all_projects.clone(), &self.inline_dirs);
        self.flat_entries = scan::build_flat_entries(&self.nodes);
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

    pub(super) fn rescan(&mut self) {
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
            .map(|p| super::detail::build_detail_info(self, p));
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

    pub(super) fn start_search(&mut self) {
        self.searching = true;
        self.search_query.clear();
        self.filtered.clear();
        self.rows_dirty = true;
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
            Some(&bytes) => super::render::format_bytes(bytes),
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
