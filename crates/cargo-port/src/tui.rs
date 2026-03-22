use std::collections::HashMap;
use std::collections::HashSet;
use std::io;
use std::io::Stdout;
use std::path::Path;
use std::path::PathBuf;
use std::process::ExitCode;
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
use nucleo_matcher::Matcher;
use nucleo_matcher::Utf32Str;
use nucleo_matcher::pattern::Atom;
use nucleo_matcher::pattern::AtomKind;
use nucleo_matcher::pattern::CaseMatching;
use nucleo_matcher::pattern::Normalization;
use ratatui::Frame;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::Constraint;
use ratatui::layout::Direction;
use ratatui::layout::Layout;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Block;
use ratatui::widgets::Borders;
use ratatui::widgets::Clear;
use ratatui::widgets::List;
use ratatui::widgets::ListItem;
use ratatui::widgets::ListState;
use ratatui::widgets::Paragraph;
use toml_edit::DocumentMut;
use walkdir::WalkDir;

use crate::config;
use crate::config::Config;
use crate::project::RustProject;

const BYTES_PER_MIB: u64 = 1024 * 1024;
const BYTES_PER_GIB: u64 = 1024 * 1024 * 1024;

#[derive(Default, PartialEq, Eq, Clone, Copy)]
enum FocusTarget {
    #[default]
    ProjectList,
    DetailFields,
    CiRuns,
    ScanLog,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DetailField {
    Name,
    Path,
    Types,
    Disk,
    Ci,
    Stats,
    Origin,
    Owner,
    Repo,
    Version,
    Description,
}

impl DetailField {
    fn label(self) -> &'static str {
        match self {
            Self::Name => "Name",
            Self::Path => "Path",
            Self::Types => "Types",
            Self::Disk => "Disk",
            Self::Ci => "CI",
            Self::Stats => "Stats",
            Self::Origin => "Origin",
            Self::Owner => "Owner",
            Self::Repo => "Repo",
            Self::Version => "Version",
            Self::Description => "Desc",
        }
    }

    fn value(self, info: &DetailInfo) -> String {
        match self {
            Self::Name => (*info.name).to_string(),
            Self::Path => (*info.path).to_string(),
            Self::Types => (*info.types).to_string(),
            Self::Disk => (*info.disk).to_string(),
            Self::Ci => (*info.ci).to_string(),
            Self::Stats => (*info.stats).to_string(),
            Self::Origin => info.git_origin.as_deref().unwrap_or("").to_string(),
            Self::Owner => info.git_owner.as_deref().unwrap_or("").to_string(),
            Self::Repo => info.git_url.as_deref().unwrap_or("").to_string(),
            Self::Version => match &info.crates_version {
                Some(cv) => format!("{} (crates.io: {cv})", info.version),
                None => (*info.version).to_string(),
            },
            Self::Description => info.description.as_deref().unwrap_or("—").to_string(),
        }
    }
}

/// Non-editable fields for the left column.
fn info_fields(info: &DetailInfo) -> Vec<DetailField> {
    let mut fields = vec![
        DetailField::Name,
        DetailField::Path,
        DetailField::Types,
        DetailField::Disk,
        DetailField::Ci,
        DetailField::Stats,
    ];
    if info.git_origin.is_some() {
        fields.push(DetailField::Origin);
    }
    if info.git_owner.is_some() {
        fields.push(DetailField::Owner);
    }
    if info.git_url.is_some() {
        fields.push(DetailField::Repo);
    }
    fields
}

/// Editable fields for the right column.
fn editable_fields() -> Vec<DetailField> { vec![DetailField::Version, DetailField::Description] }

/// Members within a workspace are organized into groups by their first subdirectory.
/// The "inline" group (empty name) contains members directly under the workspace root
/// or under the primary `crates/` directory — these are shown without a folder header.
struct MemberGroup {
    name:    String,
    members: Vec<RustProject>,
}

struct ProjectNode {
    project: RustProject,
    groups:  Vec<MemberGroup>,
}

impl ProjectNode {
    fn has_members(&self) -> bool { self.groups.iter().any(|g| !g.members.is_empty()) }
}

/// A flattened entry for fuzzy search.
struct FlatEntry {
    node_index:   usize,
    group_index:  usize,
    member_index: usize,
    name:         String,
}

use crate::ci::CiRun;
use crate::project::GitInfo;
use crate::project::ProjectType;

#[derive(Default)]
struct ProjectCounts {
    workspaces:  usize,
    libs:        usize,
    bins:        usize,
    proc_macros: usize,
    examples:    usize,
    benches:     usize,
    tests:       usize,
}

impl ProjectCounts {
    fn add_project(&mut self, project: &RustProject) {
        for t in &project.types {
            match t {
                ProjectType::Workspace => self.workspaces += 1,
                ProjectType::Library => self.libs += 1,
                ProjectType::Binary => self.bins += 1,
                ProjectType::ProcMacro => self.proc_macros += 1,
                ProjectType::BuildScript => {},
            }
        }
        self.examples += project.example_count;
        self.benches += project.bench_count;
        self.tests += project.test_count;
    }

    fn summary(&self) -> String {
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
}

enum BackgroundMsg {
    DiskUsage { path: String, bytes: u64 },
    CiRuns { path: String, runs: Vec<CiRun> },
    GitInfo { path: String, info: GitInfo },
    CratesIoVersion { path: String, version: String },
    ProjectDiscovered { project: RustProject },
    ScanActivity { path: String },
    ScanComplete,
}

/// An expand key: either a workspace node or a group within a node.
#[derive(Hash, Eq, PartialEq, Clone)]
enum ExpandKey {
    Node(usize),
    Group(usize, usize),
}

/// What a visible row represents.
enum VisibleRow {
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
}

struct App {
    scan_root:            PathBuf,
    inline_dirs:          Vec<String>,
    exclude_dirs:         Vec<String>,
    ci_run_count:         u32,
    all_projects:         Vec<RustProject>,
    nodes:                Vec<ProjectNode>,
    flat_entries:         Vec<FlatEntry>,
    disk_usage:           HashMap<String, u64>,
    ci_runs:              HashMap<String, Vec<CiRun>>,
    git_info:             HashMap<String, GitInfo>,
    crates_versions:      HashMap<String, String>,
    bg_rx:                Receiver<BackgroundMsg>,
    invert_scroll:        bool,
    expanded:             HashSet<ExpandKey>,
    list_state:           ListState,
    searching:            bool,
    search_query:         String,
    filtered:             Vec<usize>,
    show_settings:        bool,
    settings_cursor:      usize,
    settings_editing:     bool,
    settings_edit_buf:    String,
    scan_complete:        bool,
    scan_log:             Vec<String>,
    scan_log_state:       ListState,
    focus:                FocusTarget,
    detail_column:        usize,
    detail_cursor:        usize,
    ci_runs_cursor:       usize,
    editing_version:      bool,
    version_edit_buf:     String,
    editing_description:  bool,
    description_edit_buf: String,
    should_quit:          bool,
}

impl App {
    fn new(
        scan_root: PathBuf,
        projects: Vec<RustProject>,
        bg_rx: Receiver<BackgroundMsg>,
        cfg: &Config,
    ) -> Self {
        let inline_dirs = cfg.tui.inline_dirs.clone();
        let exclude_dirs = cfg.tui.exclude_dirs.clone();
        let ci_run_count = cfg.tui.ci_run_count;
        let nodes = build_tree(&scan_root, projects.clone(), &inline_dirs);
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
            all_projects: projects,
            nodes,
            flat_entries,
            disk_usage: HashMap::new(),
            ci_runs: HashMap::new(),
            git_info: HashMap::new(),
            crates_versions: HashMap::new(),
            bg_rx,
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
            detail_column: 1,
            detail_cursor: 0,
            ci_runs_cursor: 0,
            editing_version: false,
            version_edit_buf: String::new(),
            editing_description: false,
            description_edit_buf: String::new(),
            should_quit: false,
        }
    }

    fn rebuild_tree(&mut self) {
        let selected_path = self.selected_project().map(|p| (*p.path).to_string());
        self.nodes = build_tree(
            &self.scan_root,
            self.all_projects.clone(),
            &self.inline_dirs,
        );
        self.flat_entries = build_flat_entries(&self.nodes);

        // Propagate CI runs from workspace roots to their members
        for node in &self.nodes {
            if let Some(runs) = self.ci_runs.get(&node.project.path).cloned() {
                for group in &node.groups {
                    for member in &group.members {
                        self.ci_runs
                            .entry((*member.path).to_string())
                            .or_insert_with(|| runs.clone());
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
        self.scan_log.clear();
        self.scan_log_state = ListState::default();
        self.scan_complete = false;
        self.focus = FocusTarget::ProjectList;
        self.detail_column = 1;
        self.detail_cursor = 0;
        self.ci_runs_cursor = 0;
        self.editing_version = false;
        self.version_edit_buf.clear();
        self.editing_description = false;
        self.description_edit_buf.clear();
        self.expanded.clear();
        self.list_state = ListState::default();
        self.bg_rx = spawn_streaming_scan(&self.scan_root, self.ci_run_count, &self.exclude_dirs);
    }

    fn poll_background(&mut self) {
        let mut needs_rebuild = false;
        while let Ok(msg) = self.bg_rx.try_recv() {
            match msg {
                BackgroundMsg::DiskUsage { path, bytes } => {
                    self.disk_usage.insert(path, bytes);
                },
                BackgroundMsg::CiRuns { path, runs } => {
                    // Propagate to workspace members
                    if let Some(node) = self.nodes.iter().find(|n| n.project.path == path) {
                        for group in &node.groups {
                            for member in &group.members {
                                self.ci_runs
                                    .entry((*member.path).to_string())
                                    .or_insert_with(|| runs.clone());
                            }
                        }
                    }
                    self.ci_runs.insert(path, runs);
                },
                BackgroundMsg::GitInfo { path, info } => {
                    self.git_info.insert(path, info);
                },
                BackgroundMsg::CratesIoVersion { path, version } => {
                    self.crates_versions.insert(path, version);
                },
                BackgroundMsg::ProjectDiscovered { project } => {
                    if !self.all_projects.iter().any(|p| p.path == project.path) {
                        self.all_projects.push(project);
                        needs_rebuild = true;
                    }
                },
                BackgroundMsg::ScanActivity { path } => {
                    self.scan_log.push(path);
                    // Auto-scroll to bottom unless user has scrolled up
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
        }
        if needs_rebuild {
            self.rebuild_tree();
        }
    }

    fn visible_rows(&self) -> Vec<VisibleRow> {
        let mut rows = Vec::new();
        for (ni, node) in self.nodes.iter().enumerate() {
            rows.push(VisibleRow::Root { node_index: ni });
            if self.expanded.contains(&ExpandKey::Node(ni)) {
                for (gi, group) in node.groups.iter().enumerate() {
                    if group.name.is_empty() {
                        // Inline group: show members directly
                        for (mi, _) in group.members.iter().enumerate() {
                            rows.push(VisibleRow::Member {
                                node_index:   ni,
                                group_index:  gi,
                                member_index: mi,
                            });
                        }
                    } else {
                        // Named group: show header, then members if expanded
                        rows.push(VisibleRow::GroupHeader {
                            node_index:  ni,
                            group_index: gi,
                        });
                        if self.expanded.contains(&ExpandKey::Group(ni, gi)) {
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
            }
        }
        rows
    }

    fn selected_project(&self) -> Option<&RustProject> {
        if self.searching && !self.search_query.is_empty() {
            let selected = self.list_state.selected()?;
            let flat_idx = self.filtered.get(selected)?;
            let entry = &self.flat_entries[*flat_idx];
            Some(
                &self.nodes[entry.node_index].groups[entry.group_index].members[entry.member_index],
            )
        } else {
            let rows = self.visible_rows();
            let selected = self.list_state.selected()?;
            match rows.get(selected)? {
                VisibleRow::Root { node_index } => Some(&self.nodes[*node_index].project),
                VisibleRow::GroupHeader { node_index, .. } => {
                    Some(&self.nodes[*node_index].project)
                },
                VisibleRow::Member {
                    node_index,
                    group_index,
                    member_index,
                } => Some(&self.nodes[*node_index].groups[*group_index].members[*member_index]),
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
            Some(VisibleRow::Root { node_index }) => self.nodes[*node_index].has_members(),
            Some(VisibleRow::GroupHeader { .. }) => true,
            _ => false,
        }
    }

    fn expand(&mut self) {
        if !self.selected_is_expandable() {
            return;
        }
        let rows = self.visible_rows();
        let Some(selected) = self.list_state.selected() else {
            return;
        };
        match rows.get(selected) {
            Some(VisibleRow::Root { node_index }) => {
                self.expanded.insert(ExpandKey::Node(*node_index));
            },
            Some(VisibleRow::GroupHeader {
                node_index,
                group_index,
            }) => {
                self.expanded
                    .insert(ExpandKey::Group(*node_index, *group_index));
            },
            _ => {},
        }
    }

    fn collapse(&mut self) {
        let rows = self.visible_rows();
        let Some(selected) = self.list_state.selected() else {
            return;
        };
        let Some(row) = rows.get(selected) else {
            return;
        };

        match row {
            VisibleRow::Root { node_index } => {
                let key = ExpandKey::Node(*node_index);
                if self.expanded.contains(&key) {
                    self.expanded.remove(&key);
                }
            },
            VisibleRow::GroupHeader {
                node_index,
                group_index,
            } => {
                let group_key = ExpandKey::Group(*node_index, *group_index);
                if self.expanded.contains(&group_key) {
                    self.expanded.remove(&group_key);
                } else {
                    // Already collapsed group — collapse parent node
                    let ni = *node_index;
                    self.expanded.remove(&ExpandKey::Node(ni));
                    // Move cursor to the node root
                    let new_rows = self.visible_rows();
                    if let Some(pos) = new_rows.iter().position(
                        |r| matches!(r, VisibleRow::Root { node_index } if *node_index == ni),
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
                let ni = *node_index;
                let gi = *group_index;
                let group_name = &self.nodes[ni].groups[gi].name;
                if group_name.is_empty() {
                    // Inline group — collapse the node
                    self.expanded.remove(&ExpandKey::Node(ni));
                    let new_rows = self.visible_rows();
                    if let Some(pos) = new_rows.iter().position(
                        |r| matches!(r, VisibleRow::Root { node_index } if *node_index == ni),
                    ) {
                        self.list_state.select(Some(pos));
                    }
                } else {
                    // Named group — collapse the group
                    self.expanded.remove(&ExpandKey::Group(ni, gi));
                    let new_rows = self.visible_rows();
                    if let Some(pos) = new_rows.iter().position(|r| {
                        matches!(r, VisibleRow::GroupHeader { node_index, group_index }
                            if *node_index == ni && *group_index == gi)
                    }) {
                        self.list_state.select(Some(pos));
                    }
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

    fn scroll_up(&mut self) {
        let count = self.row_count();
        if count == 0 {
            return;
        }
        let current = self.list_state.selected().unwrap_or(0);
        if current > 0 {
            self.list_state.select(Some(current - 1));
        }
    }

    fn scroll_down(&mut self) {
        let count = self.row_count();
        if count == 0 {
            return;
        }
        let current = self.list_state.selected().unwrap_or(0);
        if current < count - 1 {
            self.list_state.select(Some(current + 1));
        }
    }

    fn start_search(&mut self) {
        self.searching = true;
        self.search_query.clear();
        self.filtered.clear();
    }

    fn cancel_search(&mut self) {
        self.searching = false;
        self.search_query.clear();
        self.filtered.clear();
        if !self.nodes.is_empty() {
            self.list_state.select(Some(0));
        }
    }

    fn confirm_search(&mut self) {
        let project_path = self.selected_project().map(|p| (*p.path).to_string());
        self.searching = false;
        self.search_query.clear();
        self.filtered.clear();

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
            if let VisibleRow::Root { node_index } = row {
                if self.nodes[*node_index].project.path == target_path {
                    self.list_state.select(Some(i));
                    return;
                }
            }
        }
    }

    fn update_search(&mut self, query: &str) {
        self.search_query = (*query).to_string();

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

    fn max_name_width(&self) -> usize {
        let mut max_width = 0usize;
        for node in &self.nodes {
            let name = node.project.name.as_deref().unwrap_or(&node.project.path);
            // Root items: "▶ " or "  " prefix = 2 display chars
            max_width = max_width.max(2 + name.len());
            for group in &node.groups {
                if group.name.is_empty() {
                    // Inline members: "    " prefix = 4 chars
                    for member in &group.members {
                        let name = member.name.as_deref().unwrap_or(&member.path);
                        max_width = max_width.max(4 + name.len());
                    }
                } else {
                    // Group header: "    ▶ " prefix = 6 display chars + name + count
                    let label_len = 6 + group.name.len() + 4; // " (NN)"
                    max_width = max_width.max(label_len);
                    // Group members: "        " prefix = 8 chars
                    for member in &group.members {
                        let name = member.name.as_deref().unwrap_or(&member.path);
                        max_width = max_width.max(8 + name.len());
                    }
                }
            }
        }
        max_width
    }

    fn project_counts(&self) -> ProjectCounts {
        let mut counts = ProjectCounts::default();
        for node in &self.nodes {
            counts.add_project(&node.project);
            for group in &node.groups {
                for member in &group.members {
                    counts.add_project(member);
                }
            }
        }
        counts
    }

    fn workspace_counts(&self, project: &RustProject) -> Option<ProjectCounts> {
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

    fn formatted_disk(&self, project: &RustProject) -> String {
        match self.disk_usage.get(&project.path) {
            Some(&bytes) => format_bytes(bytes),
            None => "—".to_string(),
        }
    }

    fn ci_for(&self, project: &RustProject) -> String {
        self.ci_runs
            .get(&project.path)
            .and_then(|runs| runs.first())
            .map(|run| (*run.conclusion).to_string())
            .unwrap_or_else(|| "—".to_string())
    }

    fn ci_runs_for(&self, project: &RustProject) -> Option<&Vec<CiRun>> {
        self.ci_runs.get(&project.path)
    }

    fn git_icon(&self, project: &RustProject) -> &'static str {
        match self.git_info.get(&project.path) {
            Some(info) => info.origin.icon(),
            None => " ",
        }
    }
}

fn format_bytes(bytes: u64) -> String {
    if bytes >= BYTES_PER_GIB {
        format!("{:.1} GiB", bytes as f64 / BYTES_PER_GIB as f64)
    } else {
        format!("{:.1} MiB", bytes as f64 / BYTES_PER_MIB as f64)
    }
}

const CACHE_DIR: &str = "cargo-port/ci-cache";

fn cache_dir() -> Option<PathBuf> {
    std::env::var("TMPDIR")
        .ok()
        .map(PathBuf::from)
        .or_else(|| Some(PathBuf::from("/tmp")))
        .map(|d| d.join(CACHE_DIR))
}

fn save_cached_run(ci_run: &CiRun) {
    let Some(dir) = cache_dir() else {
        return;
    };
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join(format!("{}.json", ci_run.run_id));
    if let Ok(json) = serde_json::to_string(ci_run) {
        let _ = std::fs::write(&path, json);
    }
}

fn fetch_ci_runs_cached(repo_dir: &Path, count: u32) -> Vec<CiRun> {
    use crate::ci::fetch_ci_runs;
    let runs = fetch_ci_runs(repo_dir, count);
    for ci_run in &runs {
        save_cached_run(ci_run);
    }
    runs
}

fn fetch_crates_io_version(crate_name: &str) -> Option<String> {
    let url = format!("https://crates.io/api/v1/crates/{crate_name}");
    let output = std::process::Command::new("curl")
        .args([
            "-sf",
            "--max-time",
            "5",
            "-H",
            "User-Agent: cargo-port",
            &url,
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).ok()?;
    json.get("crate")?
        .get("max_stable_version")?
        .as_str()
        .map(|s| (*s).to_string())
}

fn dir_size(path: &Path, excludes: &HashSet<String>) -> u64 {
    WalkDir::new(path)
        .into_iter()
        .filter_entry(|entry| {
            if entry.file_type().is_dir() {
                let name = entry.file_name().to_string_lossy();
                !name.starts_with('.') && name != "target" && !excludes.contains(name.as_ref())
            } else {
                true
            }
        })
        .flatten()
        .filter(|e| e.file_type().is_file())
        .filter_map(|e| e.metadata().ok())
        .map(|m| m.len())
        .sum()
}

fn build_tree(
    _scan_root: &Path,
    projects: Vec<RustProject>,
    inline_dirs: &[String],
) -> Vec<ProjectNode> {
    let workspace_paths: Vec<String> = projects
        .iter()
        .filter(|p| p.is_workspace())
        .map(|p| (*p.path).to_string())
        .collect();

    let mut nodes: Vec<ProjectNode> = Vec::new();
    let mut consumed: HashSet<usize> = HashSet::new();

    let top_level_workspaces: HashSet<usize> = projects
        .iter()
        .enumerate()
        .filter(|(_, p)| {
            p.is_workspace()
                && !workspace_paths
                    .iter()
                    .any(|ws| *ws != p.path && p.path.starts_with(&format!("{ws}/")))
        })
        .map(|(i, _)| i)
        .collect();

    for (i, project) in projects.iter().enumerate() {
        if top_level_workspaces.contains(&i) {
            let mut all_members: Vec<RustProject> = projects
                .iter()
                .enumerate()
                .filter(|(j, p)| {
                    *j != i
                        && !top_level_workspaces.contains(j)
                        && p.path.starts_with(&format!("{}/", project.path))
                })
                .map(|(j, p)| {
                    consumed.insert(j);
                    p.clone()
                })
                .collect();

            all_members.sort_by(|a, b| {
                let name_a = a.name.as_deref().unwrap_or(&a.path);
                let name_b = b.name.as_deref().unwrap_or(&b.path);
                name_a.cmp(name_b)
            });

            let groups = group_members(&project.path, all_members, inline_dirs);

            consumed.insert(i);
            nodes.push(ProjectNode {
                project: project.clone(),
                groups,
            });
        }
    }

    for (i, project) in projects.iter().enumerate() {
        if consumed.contains(&i) {
            continue;
        }
        let under_workspace = workspace_paths
            .iter()
            .any(|ws| project.path.starts_with(&format!("{ws}/")));
        if !under_workspace {
            nodes.push(ProjectNode {
                project: project.clone(),
                groups:  Vec::new(),
            });
        }
    }

    nodes.sort_by(|a, b| a.project.path.cmp(&b.project.path));
    nodes
}

fn group_members(
    workspace_path: &str,
    members: Vec<RustProject>,
    inline_dirs: &[String],
) -> Vec<MemberGroup> {
    let prefix = format!("{workspace_path}/");

    let mut group_map: HashMap<String, Vec<RustProject>> = HashMap::new();

    for member in members {
        let relative = member.path.strip_prefix(&prefix).unwrap_or(&member.path);
        let subdir = relative.split('/').next().unwrap_or("").to_string();

        // Members in configured inline dirs or directly in the workspace root are shown inline.
        // Everything else gets grouped by first subdirectory.
        let group_name = if inline_dirs.iter().any(|d| *d == subdir) || !relative.contains('/') {
            String::new()
        } else {
            subdir
        };

        group_map.entry(group_name).or_default().push(member);
    }

    let mut groups: Vec<MemberGroup> = group_map
        .into_iter()
        .map(|(name, members)| MemberGroup { name, members })
        .collect();

    // Sort: inline group first, then alphabetically by name
    groups.sort_by(|a, b| {
        let a_inline = a.name.is_empty();
        let b_inline = b.name.is_empty();
        match (a_inline, b_inline) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.name.cmp(&b.name),
        }
    });

    groups
}

fn build_flat_entries(nodes: &[ProjectNode]) -> Vec<FlatEntry> {
    let mut entries = Vec::new();
    for (ni, node) in nodes.iter().enumerate() {
        // Add workspace root itself
        let name = node.project.name.as_deref().unwrap_or(&node.project.path);
        entries.push(FlatEntry {
            node_index:   ni,
            group_index:  0,
            member_index: 0,
            name:         (*name).to_string(),
        });
        // Add all members
        for (gi, group) in node.groups.iter().enumerate() {
            for (mi, member) in group.members.iter().enumerate() {
                let name = member.name.as_deref().unwrap_or(&member.path);
                entries.push(FlatEntry {
                    node_index:   ni,
                    group_index:  gi,
                    member_index: mi,
                    name:         (*name).to_string(),
                });
            }
        }
    }
    entries
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

fn display_width(s: &str) -> usize {
    use unicode_width::UnicodeWidthStr;
    UnicodeWidthStr::width(s)
}

fn project_row_spans(
    prefix: &str,
    name: &str,
    version: &str,
    disk: &str,
    ci: &str,
    git_icon: &str,
    name_width: usize,
) -> Line<'static> {
    let prefix_width = display_width(prefix);
    let available = name_width.saturating_sub(prefix_width);
    let padded_name = format!("{prefix}{name:<width$}", width = available);
    let version_style = Style::default().fg(Color::DarkGray);
    let disk_style = Style::default().fg(Color::DarkGray);
    let ci_style = if ci.contains('✓') {
        Style::default().fg(Color::Green)
    } else if ci.contains('✗') {
        Style::default().fg(Color::Red)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let git_style = match git_icon {
        "⑂" => Style::default().fg(Color::Cyan),
        "⊙" => Style::default().fg(Color::DarkGray),
        _ => Style::default(),
    };

    Line::from(vec![
        Span::raw(padded_name),
        Span::styled(format!(" {version:>12}"), version_style),
        Span::styled(format!(" {disk:>9}"), disk_style),
        Span::styled(format!("  {ci}"), ci_style),
        Span::styled(format!(" {git_icon}"), git_style),
    ])
}

fn group_header_spans(prefix: &str, name: &str, name_width: usize) -> Line<'static> {
    let prefix_width = display_width(prefix);
    let available = name_width.saturating_sub(prefix_width);
    let padded = format!("{prefix}{name:<width$}", width = available);
    Line::from(vec![Span::styled(
        padded,
        Style::default().fg(Color::Yellow),
    )])
}

fn ui(frame: &mut Frame, app: &mut App) {
    let outer_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(frame.area());

    // Left panel width: name column + version(13) + disk(10) + ci(4) + git(2) + borders(2) +
    // padding(1)
    let version_col_width = 13;
    let disk_col_width = 10;
    let ci_col_width = 4;
    let git_col_width = 2;
    let left_width = (app.max_name_width()
        + version_col_width
        + disk_col_width
        + ci_col_width
        + git_col_width
        + 3) as u16;

    let main_layout = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(left_width), Constraint::Min(20)])
        .split(outer_layout[0]);

    // Left panel: split into search bar + project list + optional scan log
    let left_constraints = if app.scan_complete {
        vec![Constraint::Length(3), Constraint::Min(1)]
    } else {
        // Project list height = rows + 2 (borders), minimum 3 so the box is visible
        let project_rows = app.visible_rows().len() as u16;
        let project_height = (project_rows + 2).max(3);
        vec![
            Constraint::Length(3),
            Constraint::Length(project_height),
            Constraint::Min(3),
        ]
    };
    let left_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints(left_constraints)
        .split(main_layout[0]);

    // Search bar
    let search_style = if app.searching {
        Style::default().fg(Color::White)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let search_text = if app.searching {
        if app.search_query.is_empty() {
            "…".to_string()
        } else {
            app.search_query.clone()
        }
    } else {
        "/ to search".to_string()
    };

    let search_bar = Paragraph::new(Line::from(vec![
        Span::styled(" 🔍 ", Style::default().fg(Color::Yellow)),
        Span::styled(search_text, search_style),
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(if app.searching {
                Style::default().fg(Color::Cyan)
            } else {
                Style::default().fg(Color::DarkGray)
            }),
    );

    frame.render_widget(search_bar, left_layout[0]);

    // Column widths (version/disk/ci defined above in layout calc)
    let inner_width = left_layout[1].width.saturating_sub(2) as usize;
    let name_col_width = inner_width
        .saturating_sub(version_col_width + disk_col_width + ci_col_width + git_col_width);

    // Collect detail info directly without selected_project()
    let selected_idx = app.list_state.selected();
    let selected_project_ref: Option<&RustProject> = if app.searching
        && !app.search_query.is_empty()
    {
        selected_idx.and_then(|sel| {
            let flat_idx = app.filtered.get(sel)?;
            let entry = &app.flat_entries[*flat_idx];
            Some(&app.nodes[entry.node_index].groups[entry.group_index].members[entry.member_index])
        })
    } else {
        let rows = app.visible_rows();
        selected_idx.and_then(|sel| {
            let row = rows.get(sel)?;
            match row {
                VisibleRow::Root { node_index } => Some(&app.nodes[*node_index].project),
                VisibleRow::GroupHeader { node_index, .. } => Some(&app.nodes[*node_index].project),
                VisibleRow::Member {
                    node_index,
                    group_index,
                    member_index,
                } => Some(&app.nodes[*node_index].groups[*group_index].members[*member_index]),
            }
        })
    };

    let detail_info = selected_project_ref.map(|p| build_detail_info(app, p));
    let detail_ci_runs: Vec<CiRun> = selected_project_ref
        .and_then(|p| app.ci_runs_for(p))
        .cloned()
        .unwrap_or_default();

    let items: Vec<ListItem> = if app.searching && !app.search_query.is_empty() {
        render_filtered_items(app, name_col_width)
    } else {
        render_tree_items(app, name_col_width)
    };

    let header_style = Style::default()
        .fg(Color::DarkGray)
        .add_modifier(Modifier::BOLD);

    let header_line = Line::from(vec![
        Span::styled(
            format!("{:<width$}", "Project", width = name_col_width),
            header_style,
        ),
        Span::styled(
            format!(" {:>width$}", "Version", width = version_col_width - 1),
            header_style,
        ),
        Span::styled(format!(" {:>9}", "Disk"), header_style),
        Span::styled(format!("  {}", "CI"), header_style),
    ]);

    let project_list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(header_line)
                .title_style(
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ),
        )
        .highlight_style(if app.focus == FocusTarget::ProjectList {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Cyan)
        });

    frame.render_stateful_widget(project_list, left_layout[1], &mut app.list_state);

    // Scan log panel (only during scanning)
    if !app.scan_complete {
        let log_items: Vec<ListItem> = app
            .scan_log
            .iter()
            .map(|p| {
                ListItem::new(Span::styled(
                    format!("  {p}"),
                    Style::default().fg(Color::DarkGray),
                ))
            })
            .collect();

        let scan_focused = app.focus == FocusTarget::ScanLog;
        let scan_title = if scan_focused {
            " Scanning (focused) "
        } else {
            " Scanning "
        };
        let scan_log = List::new(log_items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(scan_title)
                    .title_style(if scan_focused {
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD)
                    })
                    .border_style(if scan_focused {
                        Style::default().fg(Color::Cyan)
                    } else {
                        Style::default().fg(Color::DarkGray)
                    }),
            )
            .highlight_style(
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            );

        frame.render_stateful_widget(scan_log, left_layout[2], &mut app.scan_log_state);
    }

    // Right panel: split into detail fields (top) and CI runs (bottom)
    let detail_focused = app.focus == FocusTarget::DetailFields;
    let ci_focused = app.focus == FocusTarget::CiRuns;

    let has_ci_runs = !detail_ci_runs.is_empty();
    let right_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints(if has_ci_runs {
            vec![Constraint::Length(14), Constraint::Min(3)]
        } else {
            vec![Constraint::Min(1), Constraint::Length(0)]
        })
        .split(main_layout[1]);

    // --- Detail fields: two columns ---
    let detail_block = Block::default()
        .borders(Borders::ALL)
        .title(" Details ")
        .title_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
        .border_style(if detail_focused {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default()
        });

    match &detail_info {
        Some(info) => {
            let detail_inner = detail_block.inner(right_layout[0]);
            frame.render_widget(detail_block, right_layout[0]);

            let columns = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
                .split(detail_inner);

            let highlight_style = Style::default().fg(Color::Black).bg(Color::Cyan);
            let editable_label_style = Style::default().fg(Color::Cyan);
            let readonly_label_style = Style::default().fg(Color::DarkGray);

            // Left column: non-editable info fields
            let left_fields = info_fields(info);
            let mut left_lines: Vec<Line<'static>> = Vec::new();
            for (i, field) in left_fields.iter().enumerate() {
                let label = field.label();
                let value = field.value(info);
                let is_focused = detail_focused && app.detail_column == 0 && i == app.detail_cursor;
                let style = if is_focused {
                    highlight_style
                } else if *field == DetailField::Ci {
                    conclusion_style(&info.ci)
                } else {
                    Style::default()
                };
                let label_style = if is_focused {
                    highlight_style
                } else {
                    readonly_label_style
                };
                left_lines.push(Line::from(vec![
                    Span::styled(format!("  {label:<8} "), label_style),
                    Span::styled(value, style),
                ]));
            }
            frame.render_widget(Paragraph::new(left_lines), columns[0]);

            // Right column: editable fields
            let right_fields = editable_fields();
            let mut right_lines: Vec<Line<'static>> = Vec::new();
            for (i, field) in right_fields.iter().enumerate() {
                let label = field.label();
                let is_focused = detail_focused && app.detail_column == 1 && i == app.detail_cursor;

                let is_editing = is_focused
                    && ((*field == DetailField::Version && app.editing_version)
                        || (*field == DetailField::Description && app.editing_description));

                if is_editing {
                    let buf = match *field {
                        DetailField::Version => &app.version_edit_buf,
                        DetailField::Description => &app.description_edit_buf,
                        _ => &app.version_edit_buf,
                    };
                    let text = format!("{buf}_");
                    right_lines.push(Line::from(vec![
                        Span::styled(format!("  {label:<8} "), Style::default().fg(Color::Yellow)),
                        Span::styled(text, Style::default().fg(Color::Yellow)),
                    ]));
                } else {
                    let value = field.value(info);
                    let label_style = if is_focused {
                        highlight_style
                    } else {
                        editable_label_style
                    };
                    let value_style = if is_focused {
                        highlight_style
                    } else {
                        Style::default()
                    };
                    right_lines.push(Line::from(vec![
                        Span::styled(format!("  {label:<8} "), label_style),
                        Span::styled(value, value_style),
                    ]));
                }
            }
            frame.render_widget(Paragraph::new(right_lines), columns[1]);
        },
        None => {
            let content = vec![Line::from("  No project selected")];
            let detail = Paragraph::new(content).block(detail_block);
            frame.render_widget(detail, right_layout[0]);
        },
    };

    // --- CI Runs panel ---
    if has_ci_runs {
        let ci_block = Block::default()
            .borders(Borders::ALL)
            .title(" CI Runs ")
            .title_style(
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )
            .border_style(if ci_focused {
                Style::default().fg(Color::Cyan)
            } else {
                Style::default()
            });

        let mut ci_lines: Vec<Line<'static>> = Vec::new();

        let has_bench = detail_ci_runs
            .iter()
            .any(|r| r.jobs.iter().any(|j| matches_column(&j.name, "bench")));

        let mut cols: Vec<&str> = vec!["fmt", "taplo", "clippy", "mend", "build", "test"];
        if has_bench {
            cols.push("bench");
        }

        const COL_W: usize = 10;

        // Header
        let mut hdr = format!("  | {:<12} | {:<10} |", "Branch", "Date");
        for col in &cols {
            hdr.push_str(&format!(" {:<COL_W$} |", col));
        }
        hdr.push_str(&format!(" {:<COL_W$} |", "Total"));
        ci_lines.push(Line::from(hdr));

        // Separator
        let col_sep = format!("{}-|", "-".repeat(COL_W));
        let mut sep = "  |--------------|------------|".to_string();
        for _ in &cols {
            sep.push_str(&col_sep);
        }
        sep.push_str(&col_sep);
        ci_lines.push(Line::from(sep));

        // Data rows
        for (ri, ci_run) in detail_ci_runs.iter().enumerate() {
            let date = ci_run
                .created_at
                .split_once('T')
                .map(|(d, _)| (*d).to_string())
                .unwrap_or_else(|| (*ci_run.created_at).to_string());

            let branch = truncate_str(&ci_run.branch, 12);

            let total_dur = ci_run
                .wall_clock_secs
                .map(crate::ci::format_secs)
                .unwrap_or_else(|| "—".to_string());

            let mut spans: Vec<Span> = Vec::new();
            let row_prefix = if ci_focused && ri == app.ci_runs_cursor {
                format!(" ▶| {branch:<12} | {date:<10} |")
            } else {
                format!("  | {branch:<12} | {date:<10} |")
            };
            spans.push(Span::raw(row_prefix));

            for col in &cols {
                let job = ci_run.jobs.iter().find(|j| matches_column(&j.name, col));

                match job {
                    Some(j) => {
                        let cell = format!("{} {}", j.conclusion, j.duration);
                        let padded = pad_to_width(&cell, COL_W);
                        let style = conclusion_style(&j.conclusion);
                        spans.push(Span::styled(format!(" {padded}"), style));
                        spans.push(Span::raw("|"));
                    },
                    None => {
                        spans.push(Span::styled(
                            format!(" {}", pad_to_width("—", COL_W)),
                            Style::default().fg(Color::DarkGray),
                        ));
                        spans.push(Span::raw("|"));
                    },
                }
            }

            // Total column
            let total_cell = format!("{} {total_dur}", ci_run.conclusion);
            let padded_total = pad_to_width(&total_cell, COL_W);
            let total_style = conclusion_style(&ci_run.conclusion);
            spans.push(Span::styled(format!(" {padded_total}"), total_style));
            spans.push(Span::raw("|"));

            ci_lines.push(Line::from(spans));
        }

        let ci_paragraph = Paragraph::new(ci_lines).block(ci_block);
        frame.render_widget(ci_paragraph, right_layout[1]);
    }

    // Bottom bar
    let key_style = Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD);

    let global_counts = app.project_counts();
    let count_str = global_counts.summary();
    let count_style = Style::default().fg(Color::Yellow);

    let scan_indicator = if app.scan_complete {
        Span::raw("")
    } else {
        Span::styled(
            " ⟳ scanning… ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
    };

    let status_spans = if app.editing_version || app.editing_description {
        vec![
            scan_indicator,
            Span::styled(" Enter", key_style),
            Span::raw(" confirm  "),
            Span::styled("Esc", key_style),
            Span::raw(" cancel"),
        ]
    } else if app.searching {
        vec![
            scan_indicator,
            Span::styled(" ↑/↓", key_style),
            Span::raw(" navigate  "),
            Span::styled("enter", key_style),
            Span::raw(" select  "),
            Span::styled("esc", key_style),
            Span::raw(" cancel"),
        ]
    } else if app.focus == FocusTarget::DetailFields {
        vec![
            scan_indicator,
            Span::styled(" ↑/↓", key_style),
            Span::raw(" nav  "),
            Span::styled("←/→", key_style),
            Span::raw(" column  "),
            Span::styled("Enter", key_style),
            Span::raw(" edit  "),
            Span::styled("Tab", key_style),
            Span::raw(" next  "),
            Span::styled("Esc", key_style),
            Span::raw(" back"),
        ]
    } else if app.focus == FocusTarget::CiRuns {
        vec![
            scan_indicator,
            Span::styled(" ↑/↓", key_style),
            Span::raw(" nav  "),
            Span::styled("Tab", key_style),
            Span::raw(" next  "),
            Span::styled("Esc", key_style),
            Span::raw(" back"),
        ]
    } else {
        vec![
            scan_indicator,
            Span::styled(format!(" {count_str}"), count_style),
            Span::raw("  "),
            Span::styled("↑/↓", key_style),
            Span::raw(" nav  "),
            Span::styled("←/→", key_style),
            Span::raw(" expand  "),
            Span::styled("Tab", key_style),
            Span::raw(" details  "),
            Span::styled("Home/End", key_style),
            Span::raw(" top/btm  "),
            Span::styled("/", key_style),
            Span::raw(" search  "),
            Span::styled("r", key_style),
            Span::raw(" rescan  "),
            Span::styled("s", key_style),
            Span::raw(" settings  "),
            Span::styled("q", key_style),
            Span::raw(" quit"),
        ]
    };

    let status_bar = Paragraph::new(Line::from(status_spans))
        .style(Style::default().bg(Color::DarkGray).fg(Color::White));

    frame.render_widget(status_bar, outer_layout[1]);

    // Settings popup
    if app.show_settings {
        render_settings_popup(frame, app);
    }
}

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(height) / 2;
    Rect::new(x, y, width.min(area.width), height.min(area.height))
}

const SETTINGS_COUNT: usize = 4;
const SETTING_INVERT_SCROLL: usize = 0;
const SETTING_CI_RUN_COUNT: usize = 1;
const SETTING_INLINE_DIRS: usize = 2;
const SETTING_EXCLUDE_DIRS: usize = 3;

fn render_settings_popup(frame: &mut Frame, app: &App) {
    let area = centered_rect(60, SETTINGS_COUNT as u16 + 6, frame.area());

    frame.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Settings ")
        .title_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
        .border_style(Style::default().fg(Color::Cyan));

    let key_style = Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD);
    let label_style = Style::default().fg(Color::DarkGray);
    let highlight_style = Style::default().fg(Color::Black).bg(Color::Cyan);

    let cfg = config::load();

    let settings: Vec<(&str, String)> = vec![
        (
            "Invert scroll",
            if app.invert_scroll { "ON" } else { "OFF" }.to_string(),
        ),
        ("CI run count", format!("{}", cfg.tui.ci_run_count)),
        ("Inline dirs", cfg.tui.inline_dirs.join(", ")),
        ("Exclude dirs", cfg.tui.exclude_dirs.join(", ")),
    ];

    let mut lines: Vec<Line<'static>> = vec![Line::from("")];

    for (i, (name, value)) in settings.into_iter().enumerate() {
        let cursor = if app.settings_cursor == i {
            "▶ "
        } else {
            "  "
        };
        let is_selected = app.settings_cursor == i;

        if app.settings_editing && is_selected {
            let label = format!("  {cursor}{name}:  ");
            lines.push(Line::from(vec![
                Span::styled(label, Style::default().fg(Color::Yellow)),
                Span::styled(
                    format!("{}_", app.settings_edit_buf),
                    Style::default().fg(Color::Yellow),
                ),
            ]));
        } else if i == SETTING_INVERT_SCROLL {
            let toggle_style = if app.invert_scroll {
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
            };
            let row_style = if is_selected {
                highlight_style
            } else {
                label_style
            };
            lines.push(Line::from(vec![
                Span::styled(format!("  {cursor}{name}:  "), row_style),
                Span::styled("< ", Style::default().fg(Color::DarkGray)),
                Span::styled(value, toggle_style),
                Span::styled(" >", Style::default().fg(Color::DarkGray)),
            ]));
        } else if i == SETTING_CI_RUN_COUNT && is_selected && !app.settings_editing {
            lines.push(Line::from(vec![
                Span::styled(format!("  {cursor}{name}:  "), highlight_style),
                Span::styled("< ", Style::default().fg(Color::DarkGray)),
                Span::styled(value, Style::default().add_modifier(Modifier::BOLD)),
                Span::styled(" >", Style::default().fg(Color::DarkGray)),
            ]));
        } else {
            let style = if is_selected {
                highlight_style
            } else {
                label_style
            };
            lines.push(Line::from(Span::styled(
                format!("  {cursor}{name}:  {value}"),
                style,
            )));
        }
    }

    lines.push(Line::from(""));
    if app.settings_editing {
        lines.push(Line::from(vec![
            Span::styled("  Enter", key_style),
            Span::raw(" confirm  "),
            Span::styled("Esc", key_style),
            Span::raw(" cancel"),
        ]));
    } else {
        lines.push(Line::from(vec![
            Span::styled("  ↑/↓", key_style),
            Span::raw(" nav  "),
            Span::styled("Enter", key_style),
            Span::raw(" edit  "),
            Span::styled("←/→", key_style),
            Span::raw(" toggle  "),
            Span::styled("Esc", key_style),
            Span::raw(" close"),
        ]));
    }

    let paragraph = Paragraph::new(lines).block(block);
    frame.render_widget(paragraph, area);
}

fn conclusion_style(conclusion: &str) -> Style {
    if conclusion.contains('✓') {
        Style::default().fg(Color::Green)
    } else if conclusion.contains('✗') {
        Style::default().fg(Color::Red)
    } else {
        Style::default().fg(Color::DarkGray)
    }
}

fn matches_column(job_name: &str, column: &str) -> bool {
    let lower = job_name.to_lowercase();
    match column {
        "fmt" => lower.contains("format") || lower.contains("fmt"),
        "taplo" => lower.contains("taplo"),
        "clippy" => lower.contains("clippy"),
        "mend" => lower.contains("mend"),
        "build" => lower.contains("build"),
        "test" => lower.contains("test"),
        "bench" => lower.contains("bench"),
        _ => false,
    }
}

fn pad_to_width(s: &str, width: usize) -> String {
    let dw = display_width(s);
    if dw >= width {
        (*s).to_string()
    } else {
        format!("{s}{}", " ".repeat(width - dw))
    }
}

fn truncate_str(s: &str, max_len: usize) -> String {
    if s.len() > max_len {
        format!("{}…", &s[..max_len - 1])
    } else {
        (*s).to_string()
    }
}

struct DetailInfo {
    name:           String,
    path:           String,
    version:        String,
    description:    Option<String>,
    crates_version: Option<String>,
    types:          String,
    disk:           String,
    ci:             String,
    stats:          String,
    git_origin:     Option<String>,
    git_owner:      Option<String>,
    git_url:        Option<String>,
}

fn build_detail_info(app: &App, project: &RustProject) -> DetailInfo {
    let ws_counts = app.workspace_counts(project);
    let stats = ws_counts
        .as_ref()
        .map(ProjectCounts::summary)
        .unwrap_or_else(|| {
            let mut parts: Vec<String> = Vec::new();
            if project.example_count > 0 {
                parts.push(format!("{} examples", project.example_count));
            }
            if project.bench_count > 0 {
                parts.push(format!("{} benches", project.bench_count));
            }
            if project.test_count > 0 {
                parts.push(format!("{} tests", project.test_count));
            }
            if parts.is_empty() {
                "—".to_string()
            } else {
                parts.join("  ")
            }
        });

    let git = app.git_info.get(&project.path);
    let git_origin = git.map(|g| format!("{} {}", g.origin.icon(), g.origin.label()));
    let git_owner = git.and_then(|g| g.owner.clone());
    let git_url = git.and_then(|g| g.url.clone());
    let crates_version = app.crates_versions.get(&project.path).cloned();

    DetailInfo {
        name: project.name.clone().unwrap_or_else(|| "-".to_string()),
        path: (*project.path).to_string(),
        version: project.version.clone().unwrap_or_else(|| "-".to_string()),
        description: project.description.clone(),
        types: project
            .types
            .iter()
            .map(|t| t.to_string())
            .collect::<Vec<_>>()
            .join(", "),
        disk: app.formatted_disk(project),
        ci: app.ci_for(project),
        stats,
        crates_version,
        git_origin,
        git_owner,
        git_url,
    }
}

fn version_str(project: &RustProject) -> &str { project.version.as_deref().unwrap_or("-") }

fn render_tree_items(app: &App, name_width: usize) -> Vec<ListItem<'static>> {
    let rows = app.visible_rows();
    rows.iter()
        .map(|row| match row {
            VisibleRow::Root { node_index } => {
                let node = &app.nodes[*node_index];
                let project = &node.project;
                let name = project.name.as_deref().unwrap_or(&project.path);
                let version = version_str(project);
                let disk = app.formatted_disk(project);
                let ci = app.ci_for(project);
                let git = app.git_icon(project);
                if node.has_members() {
                    let arrow = if app.expanded.contains(&ExpandKey::Node(*node_index)) {
                        "▼ "
                    } else {
                        "▶ "
                    };
                    ListItem::new(project_row_spans(
                        arrow, name, version, &disk, &ci, git, name_width,
                    ))
                } else {
                    ListItem::new(project_row_spans(
                        "  ", name, version, &disk, &ci, git, name_width,
                    ))
                }
            },
            VisibleRow::GroupHeader {
                node_index,
                group_index,
            } => {
                let group = &app.nodes[*node_index].groups[*group_index];
                let arrow = if app
                    .expanded
                    .contains(&ExpandKey::Group(*node_index, *group_index))
                {
                    "▼ "
                } else {
                    "▶ "
                };
                let prefix = format!("    {arrow}");
                let label = format!("{} ({})", group.name, group.members.len());
                ListItem::new(group_header_spans(&prefix, &label, name_width))
            },
            VisibleRow::Member {
                node_index,
                group_index,
                member_index,
            } => {
                let group = &app.nodes[*node_index].groups[*group_index];
                let member = &group.members[*member_index];
                let name = member.name.as_deref().unwrap_or(&member.path);
                let version = version_str(member);
                let disk = app.formatted_disk(member);
                let ci = app.ci_for(member);
                let git = app.git_icon(member);
                let indent = if group.name.is_empty() {
                    "    "
                } else {
                    "        "
                };
                ListItem::new(project_row_spans(
                    indent, name, version, &disk, &ci, git, name_width,
                ))
            },
        })
        .collect()
}

fn render_filtered_items(app: &App, name_width: usize) -> Vec<ListItem<'static>> {
    app.filtered
        .iter()
        .map(|&flat_idx| {
            let entry = &app.flat_entries[flat_idx];
            let project =
                &app.nodes[entry.node_index].groups[entry.group_index].members[entry.member_index];
            let version = version_str(project);
            let disk = app.formatted_disk(project);
            let ci = app.ci_for(project);
            let git = app.git_icon(project);
            ListItem::new(project_row_spans(
                "  ",
                &entry.name,
                version,
                &disk,
                &ci,
                git,
                name_width,
            ))
        })
        .collect()
}

/// Spawn a streaming scan: walk the directory tree, and for each project discovered
/// do disk + CI together on rayon so progress fills in visibly.
fn spawn_streaming_scan(
    scan_root: &Path,
    ci_run_count: u32,
    exclude_dirs: &[String],
) -> Receiver<BackgroundMsg> {
    let (tx, rx) = mpsc::channel();
    let root = scan_root.to_path_buf();
    let excludes: HashSet<String> = exclude_dirs.iter().cloned().collect();

    thread::spawn(move || {
        let entries = WalkDir::new(&root).into_iter().filter_entry(|entry| {
            if entry.file_type().is_dir() {
                let name = entry.file_name().to_string_lossy();
                !name.starts_with('.') && name != "target" && !excludes.contains(name.as_ref())
            } else {
                true
            }
        });

        rayon::scope(|s| {
            for entry in entries.flatten() {
                if entry.file_type().is_dir() {
                    let rel = entry
                        .path()
                        .strip_prefix(&root)
                        .unwrap_or(entry.path())
                        .display()
                        .to_string();
                    let _ = tx.send(BackgroundMsg::ScanActivity {
                        path: if rel.is_empty() { ".".to_string() } else { rel },
                    });
                }
                if entry.file_type().is_file() && entry.file_name() == "Cargo.toml" {
                    if let Ok(project) = RustProject::from_cargo_toml(entry.path(), &root) {
                        let abs_path = root.join(&project.path);
                        let has_git = abs_path.join(".git").exists();

                        let _ = tx.send(BackgroundMsg::ProjectDiscovered {
                            project: project.clone(),
                        });

                        // Spawn one rayon task per project that does disk + CI together
                        let task_tx = tx.clone();
                        let task_path = (*project.path).to_string();
                        let task_name = project.name.clone();
                        let task_abs = abs_path;
                        let task_excludes = excludes.clone();
                        s.spawn(move |_| {
                            // Disk
                            let bytes = dir_size(&task_abs, &task_excludes);
                            let _ = task_tx.send(BackgroundMsg::DiskUsage {
                                path: (*task_path).to_string(),
                                bytes,
                            });

                            // Git info (fork vs clone, owner, URL)
                            if has_git {
                                if let Some(info) = GitInfo::detect(&task_abs) {
                                    let _ = task_tx.send(BackgroundMsg::GitInfo {
                                        path: (*task_path).to_string(),
                                        info,
                                    });
                                }
                            }

                            // Crates.io version
                            if let Some(name) = &task_name {
                                if let Some(version) = fetch_crates_io_version(name) {
                                    let _ = task_tx.send(BackgroundMsg::CratesIoVersion {
                                        path: (*task_path).to_string(),
                                        version,
                                    });
                                }
                            }

                            // CI
                            if has_git {
                                let _ = task_tx.send(BackgroundMsg::ScanActivity {
                                    path: format!("CI: {task_path}"),
                                });
                                let runs = fetch_ci_runs_cached(&task_abs, ci_run_count);
                                let _ = task_tx.send(BackgroundMsg::CiRuns {
                                    path: task_path,
                                    runs,
                                });
                            }
                        });
                    }
                }
            }
        });

        let _ = tx.send(BackgroundMsg::ScanComplete);
    });

    rx
}

pub fn run(path: PathBuf) -> ExitCode {
    let scan_root = match path.canonicalize() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Error: cannot resolve path '{}': {e}", path.display());
            return ExitCode::FAILURE;
        },
    };

    let cfg = config::load();
    let bg_rx = spawn_streaming_scan(&scan_root, cfg.tui.ci_run_count, &cfg.tui.exclude_dirs);
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

    let mut app = App::new(scan_root, projects, bg_rx, &cfg);

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
            if app.show_settings {
                handle_settings_key(app, key.code);
            } else if app.editing_version || app.editing_description {
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
                    app.scroll_down();
                } else {
                    app.scroll_up();
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
                    app.scroll_up();
                } else {
                    app.scroll_down();
                }
            },
            _ => {},
        },
        _ => {},
    }
}

fn event_loop(terminal: &mut Terminal<CrosstermBackend<Stdout>>, app: &mut App) -> io::Result<()> {
    loop {
        app.poll_background();
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
        }

        if app.should_quit {
            break;
        }
    }
    Ok(())
}

fn handle_normal_key(app: &mut App, key: KeyCode) {
    match key {
        KeyCode::Char('q') | KeyCode::Esc => app.should_quit = true,
        KeyCode::Tab => {
            advance_focus(app);
        },
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

fn handle_settings_key(app: &mut App, key: KeyCode) {
    if app.settings_editing {
        handle_settings_edit_key(app, key);
        return;
    }

    match key {
        KeyCode::Esc | KeyCode::Char('s') => {
            app.show_settings = false;
            app.settings_cursor = 0;
        },
        KeyCode::Up => {
            if app.settings_cursor > 0 {
                app.settings_cursor -= 1;
            }
        },
        KeyCode::Down => {
            if app.settings_cursor < SETTINGS_COUNT - 1 {
                app.settings_cursor += 1;
            }
        },
        KeyCode::Left | KeyCode::Right => match app.settings_cursor {
            SETTING_INVERT_SCROLL => {
                app.invert_scroll = !app.invert_scroll;
                save_settings(app);
            },
            SETTING_CI_RUN_COUNT => {
                let mut cfg = config::load();
                if key == KeyCode::Right {
                    cfg.tui.ci_run_count = cfg.tui.ci_run_count.saturating_add(1);
                } else {
                    cfg.tui.ci_run_count = cfg.tui.ci_run_count.saturating_sub(1).max(1);
                }
                app.ci_run_count = cfg.tui.ci_run_count;
                let _ = config::save(&cfg);
            },
            _ => {},
        },
        KeyCode::Enter | KeyCode::Char(' ') => match app.settings_cursor {
            SETTING_INVERT_SCROLL => {
                app.invert_scroll = !app.invert_scroll;
                save_settings(app);
            },
            SETTING_CI_RUN_COUNT => {
                let cfg = config::load();
                app.settings_edit_buf = format!("{}", cfg.tui.ci_run_count);
                app.settings_editing = true;
            },
            SETTING_INLINE_DIRS => {
                app.settings_edit_buf = app.inline_dirs.join(", ");
                app.settings_editing = true;
            },
            SETTING_EXCLUDE_DIRS => {
                app.settings_edit_buf = app.exclude_dirs.join(", ");
                app.settings_editing = true;
            },
            _ => {},
        },
        _ => {},
    }
}

fn handle_settings_edit_key(app: &mut App, key: KeyCode) {
    match key {
        KeyCode::Enter => {
            let value = app.settings_edit_buf.clone();
            match app.settings_cursor {
                SETTING_CI_RUN_COUNT => {
                    if let Ok(n) = value.parse::<u32>() {
                        let count = n.max(1);
                        app.ci_run_count = count;
                        let mut cfg = config::load();
                        cfg.tui.ci_run_count = count;
                        let _ = config::save(&cfg);
                    }
                },
                SETTING_INLINE_DIRS => {
                    let dirs: Vec<String> = value
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                    app.inline_dirs = dirs.clone();
                    let mut cfg = config::load();
                    cfg.tui.inline_dirs = dirs;
                    let _ = config::save(&cfg);
                    app.rebuild_tree();
                },
                SETTING_EXCLUDE_DIRS => {
                    let dirs: Vec<String> = value
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                    app.exclude_dirs = dirs.clone();
                    let mut cfg = config::load();
                    cfg.tui.exclude_dirs = dirs;
                    let _ = config::save(&cfg);
                },
                _ => {},
            }
            app.settings_editing = false;
            app.settings_edit_buf.clear();
        },
        KeyCode::Esc => {
            app.settings_editing = false;
            app.settings_edit_buf.clear();
        },
        KeyCode::Backspace => {
            app.settings_edit_buf.pop();
        },
        KeyCode::Char(c) => {
            app.settings_edit_buf.push(c);
        },
        _ => {},
    }
}

fn save_settings(app: &App) {
    let mut cfg = config::load();
    cfg.mouse.invert_scroll = app.invert_scroll;
    let _ = config::save(&cfg);
}

fn handle_search_key(app: &mut App, key: KeyCode) {
    match key {
        KeyCode::Esc => app.cancel_search(),
        KeyCode::Enter => app.confirm_search(),
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

fn advance_focus(app: &mut App) {
    let has_ci = app
        .selected_project()
        .and_then(|p| app.ci_runs_for(p))
        .is_some_and(|r| !r.is_empty());

    app.focus = match app.focus {
        FocusTarget::ProjectList => {
            app.detail_column = 1;
            app.detail_cursor = 0;
            FocusTarget::DetailFields
        },
        FocusTarget::DetailFields => {
            if has_ci {
                app.ci_runs_cursor = 0;
                FocusTarget::CiRuns
            } else if !app.scan_complete {
                FocusTarget::ScanLog
            } else {
                FocusTarget::ProjectList
            }
        },
        FocusTarget::CiRuns => {
            if !app.scan_complete {
                FocusTarget::ScanLog
            } else {
                FocusTarget::ProjectList
            }
        },
        FocusTarget::ScanLog => FocusTarget::ProjectList,
    };

    if app.focus == FocusTarget::ScanLog && !app.scan_log.is_empty() {
        if app.scan_log_state.selected().is_none() {
            app.scan_log_state
                .select(Some(app.scan_log.len().saturating_sub(1)));
        }
    }
}

fn handle_detail_key(app: &mut App, key: KeyCode) {
    let field_count = if app.detail_column == 0 {
        app.selected_project()
            .map(|p| {
                let info = build_detail_info(app, p);
                info_fields(&info).len()
            })
            .unwrap_or(0)
    } else {
        editable_fields().len()
    };

    match key {
        KeyCode::Up => {
            if app.detail_cursor > 0 {
                app.detail_cursor -= 1;
            }
        },
        KeyCode::Down => {
            if field_count > 0 && app.detail_cursor < field_count - 1 {
                app.detail_cursor += 1;
            }
        },
        KeyCode::Left => {
            if app.detail_column > 0 {
                app.detail_column = 0;
                // Clamp cursor to left column size
                let left_count = app
                    .selected_project()
                    .map(|p| {
                        let info = build_detail_info(app, p);
                        info_fields(&info).len()
                    })
                    .unwrap_or(0);
                if app.detail_cursor >= left_count {
                    app.detail_cursor = left_count.saturating_sub(1);
                }
            }
        },
        KeyCode::Right => {
            if app.detail_column < 1 {
                app.detail_column = 1;
                let right_count = editable_fields().len();
                if app.detail_cursor >= right_count {
                    app.detail_cursor = right_count.saturating_sub(1);
                }
            }
        },
        KeyCode::Enter => {
            if app.detail_column == 1 {
                let fields = editable_fields();
                if let Some(field) = fields.get(app.detail_cursor) {
                    if let Some(project) = app.selected_project() {
                        match *field {
                            DetailField::Version => {
                                let version = project.version.clone().unwrap_or_default();
                                if version != "(workspace)" {
                                    app.version_edit_buf = version;
                                    app.editing_version = true;
                                }
                            },
                            DetailField::Description => {
                                app.description_edit_buf =
                                    project.description.clone().unwrap_or_default();
                                app.editing_description = true;
                            },
                            _ => {},
                        }
                    }
                }
            }
        },
        KeyCode::Tab => advance_focus(app),
        KeyCode::Esc => {
            app.focus = FocusTarget::ProjectList;
        },
        KeyCode::Char('q') => app.should_quit = true,
        _ => {},
    }
}

fn handle_ci_runs_key(app: &mut App, key: KeyCode) {
    let run_count = app
        .selected_project()
        .and_then(|p| app.ci_runs_for(p))
        .map(Vec::len)
        .unwrap_or(0);

    match key {
        KeyCode::Up => {
            if app.ci_runs_cursor > 0 {
                app.ci_runs_cursor -= 1;
            }
        },
        KeyCode::Down => {
            if run_count > 0 && app.ci_runs_cursor < run_count - 1 {
                app.ci_runs_cursor += 1;
            }
        },
        KeyCode::Tab => advance_focus(app),
        KeyCode::Esc => {
            app.focus = FocusTarget::ProjectList;
        },
        KeyCode::Char('q') => app.should_quit = true,
        _ => {},
    }
}

fn handle_field_edit_key(app: &mut App, key: KeyCode) {
    let buf = if app.editing_version {
        &mut app.version_edit_buf
    } else {
        &mut app.description_edit_buf
    };

    match key {
        KeyCode::Enter => {
            let new_value = buf.clone();
            if app.editing_version {
                if let Some(result) = write_toml_field(app, "version", &new_value) {
                    if result.is_ok() {
                        update_project_field(app, "version", &new_value);
                    }
                }
                app.editing_version = false;
                app.version_edit_buf.clear();
            } else {
                if let Some(result) = write_toml_field(app, "description", &new_value) {
                    if result.is_ok() {
                        update_project_field(app, "description", &new_value);
                    }
                }
                app.editing_description = false;
                app.description_edit_buf.clear();
            }
        },
        KeyCode::Esc => {
            if app.editing_version {
                app.editing_version = false;
                app.version_edit_buf.clear();
            } else {
                app.editing_description = false;
                app.description_edit_buf.clear();
            }
        },
        KeyCode::Backspace => {
            buf.pop();
        },
        KeyCode::Char(c) => {
            buf.push(c);
        },
        _ => {},
    }
}

fn write_toml_field(app: &App, field: &str, value: &str) -> Option<Result<(), String>> {
    let project = app.selected_project()?;
    let abs_path = app.scan_root.join(&project.path).join("Cargo.toml");
    let contents = match std::fs::read_to_string(&abs_path) {
        Ok(c) => c,
        Err(e) => return Some(Err(format!("Failed to read {}: {e}", abs_path.display()))),
    };
    let mut doc: DocumentMut = match contents.parse() {
        Ok(d) => d,
        Err(e) => return Some(Err(format!("Failed to parse TOML: {e}"))),
    };
    doc["package"][field] = toml_edit::value(value);
    if let Err(e) = std::fs::write(&abs_path, doc.to_string()) {
        return Some(Err(format!("Failed to write {}: {e}", abs_path.display())));
    }

    // Run taplo fmt on the edited file
    let _ = std::process::Command::new("taplo")
        .args(["fmt", &abs_path.to_string_lossy()])
        .output();

    Some(Ok(()))
}

fn update_project_field(app: &mut App, field: &str, new_value: &str) {
    let project_path = match app.selected_project() {
        Some(p) => (*p.path).to_string(),
        None => return,
    };

    for p in &mut app.all_projects {
        if p.path == project_path {
            match field {
                "version" => p.version = Some((*new_value).to_string()),
                "description" => p.description = Some((*new_value).to_string()),
                _ => {},
            }
        }
    }

    for node in &mut app.nodes {
        if node.project.path == project_path {
            match field {
                "version" => node.project.version = Some((*new_value).to_string()),
                "description" => node.project.description = Some((*new_value).to_string()),
                _ => {},
            }
        }
        for group in &mut node.groups {
            for member in &mut group.members {
                if member.path == project_path {
                    match field {
                        "version" => member.version = Some((*new_value).to_string()),
                        "description" => member.description = Some((*new_value).to_string()),
                        _ => {},
                    }
                }
            }
        }
    }
}
