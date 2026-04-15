use std::path::Path;

use super::timestamp;
use crate::ci::CiRun;
use crate::ci::Conclusion;
use crate::constants::IN_SYNC;
use crate::constants::NO_CI_RUNS;
use crate::constants::NO_CI_WORKFLOW;
use crate::constants::NO_LINT_RUNS;
use crate::constants::NO_LINT_RUNS_NOT_RUST;
use crate::constants::NO_REMOTE_SYNC;
use crate::constants::SYNC_DOWN;
use crate::constants::SYNC_UP;
use crate::lint::LintRun;
use crate::project;
use crate::project::AbsolutePath;
use crate::project::Cargo;
use crate::project::ExampleGroup;
use crate::project::GitOrigin;
use crate::project::GitPathState;
use crate::project::NonRustProject;
use crate::project::PackageProject;
use crate::project::ProjectFields;
use crate::project::ProjectType;
use crate::project::RootItem;
use crate::project::RustProject;
use crate::project::SubmoduleInfo;
use crate::project::WorkspaceProject;
use crate::project::WorktreeGroup;
use crate::tui::app::App;

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
    fn add_package(&mut self, project: &PackageProject) { self.add_cargo(project.cargo()); }

    fn add_workspace(&mut self, ws: &WorkspaceProject) {
        self.workspaces += 1;
        self.add_cargo(ws.cargo());
    }

    fn add_cargo(&mut self, cargo: &Cargo) {
        for t in cargo.types() {
            match t {
                ProjectType::Workspace => {},
                ProjectType::Library => self.libs += 1,
                ProjectType::Binary => self.bins += 1,
                ProjectType::ProcMacro => self.proc_macros += 1,
            }
        }
        self.examples += cargo.example_count();
        self.benches += cargo.benches().len();
        self.tests += cargo.test_count();
    }

    /// Returns non-zero stats as (label, count) pairs for column display.
    fn to_rows(&self) -> Vec<(&'static str, usize)> {
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

#[derive(Clone, Copy)]
pub enum RunTargetKind {
    Binary,
    Example,
    Bench,
}

impl RunTargetKind {
    pub const BINARY_COLOR: ratatui::style::Color = crate::tui::constants::SUCCESS_COLOR;
    pub const EXAMPLE_COLOR: ratatui::style::Color = crate::tui::constants::ACCENT_COLOR;
    pub const BENCH_COLOR: ratatui::style::Color = crate::tui::constants::TARGET_BENCH_COLOR;

    pub const fn color(self) -> ratatui::style::Color {
        match self {
            Self::Binary => Self::BINARY_COLOR,
            Self::Example => Self::EXAMPLE_COLOR,
            Self::Bench => Self::BENCH_COLOR,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Binary => "bin",
            Self::Example => "example",
            Self::Bench => "bench",
        }
    }

    /// Longest label width across all variants, plus 1 for trailing pad.
    pub const fn padded_label_width() -> usize {
        let mut max = 0;
        let labels: [&str; 3] = ["bin", "example", "bench"];
        let mut i = 0;
        while i < labels.len() {
            if labels[i].len() > max {
                max = labels[i].len();
            }
            i += 1;
        }
        max + 1
    }
}

pub struct TargetEntry {
    pub name:         String,
    pub display_name: String,
    pub kind:         RunTargetKind,
}

/// Build a flat list of all runnable targets: binaries first, then examples alphabetically,
/// then benches alphabetically.
pub fn build_target_list_from_data(data: &TargetsData) -> Vec<TargetEntry> {
    let mut entries = Vec::new();

    if data.is_binary
        && let Some(name) = &data.binary_name
    {
        entries.push(TargetEntry {
            display_name: name.clone(),
            name:         name.clone(),
            kind:         RunTargetKind::Binary,
        });
    }

    // Collect examples with category prefix for display, sorted with
    // categorized (containing '/') before uncategorized, then alphabetically.
    let mut examples: Vec<(String, String)> = data
        .examples
        .iter()
        .flat_map(|g| {
            g.names.iter().map(|n| {
                let display = if g.category.is_empty() {
                    n.clone()
                } else {
                    format!("{}/{}", g.category, n)
                };
                (n.clone(), display)
            })
        })
        .collect();
    examples.sort_by(|a, b| {
        let a_has_cat = a.1.contains('/');
        let b_has_cat = b.1.contains('/');
        match (a_has_cat, b_has_cat) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.1.cmp(&b.1),
        }
    });
    for (name, display_name) in examples {
        entries.push(TargetEntry {
            name,
            display_name,
            kind: RunTargetKind::Example,
        });
    }

    let mut bench_names = data.benches.clone();
    bench_names.sort();
    for name in bench_names {
        entries.push(TargetEntry {
            display_name: name.clone(),
            name,
            kind: RunTargetKind::Bench,
        });
    }

    entries
}

pub struct PendingExampleRun {
    pub abs_path:     String,
    pub target_name:  String,
    pub package_name: Option<String>,
    pub kind:         RunTargetKind,
    pub release:      bool,
}

/// Whether a CI fetch should sync recent runs or discover older history.
#[derive(Clone, Copy)]
pub enum CiFetchKind {
    /// Fetch runs older than the oldest cached run.
    FetchOlder,
    /// Re-sync the most recent N runs, refreshing stale failures.
    Sync,
}

/// A pending request to fetch more CI runs for a project.
pub struct PendingCiFetch {
    pub project_path:      String,
    pub ci_run_count:      u32,
    pub oldest_created_at: Option<String>,
    pub kind:              CiFetchKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DetailField {
    Path,
    Targets,
    Disk,
    Lint,
    Ci,
    Branch,
    GitPath,
    Sync,
    VsOrigin,
    VsLocal,
    Origin,
    Owner,
    Repo,
    Stars,
    RepoDesc,
    Inception,
    LastCommit,
    WorktreeError,
    CratesIo,
    Downloads,
    Version,
}

impl DetailField {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Path => "Path",
            Self::Targets => "Type",
            Self::Disk => "Disk",
            Self::Lint => "Lint",
            Self::Ci => "CI",
            Self::Branch => "Branch",
            Self::GitPath => "Git Path",
            Self::Sync => "Remote status",
            Self::VsOrigin => "Remote branch",
            Self::VsLocal => "vs local main",
            Self::Origin | Self::Repo => "Origin",
            Self::Owner => "Owner",
            Self::Stars => "Stars",
            Self::RepoDesc => "About",
            Self::Inception => "Incept",
            Self::LastCommit => "Latest",
            Self::WorktreeError => "Error",
            Self::CratesIo => "crates.io",
            Self::Downloads => "Downloads",
            Self::Version => "Version",
        }
    }

    /// Get the display value for a package field from `PackageData`.
    pub fn package_value(self, data: &PackageData, app: &App) -> String {
        match self {
            Self::Path => data.path.clone(),
            Self::Disk => data.disk.clone(),
            Self::Targets => data.types.clone(),
            Self::Lint => {
                if !app.is_rust_at_path(data.abs_path.as_path()) {
                    return NO_LINT_RUNS_NOT_RUST.to_string();
                }
                let abs_path = data.abs_path.as_path();
                let is_worktree_group = app
                    .projects()
                    .iter()
                    .any(|item| item.path() == abs_path && matches!(item, RootItem::Worktrees(_)));
                let lint_icon = if is_worktree_group {
                    app.selected_lint_icon(abs_path)
                        .unwrap_or_else(|| app.lint_icon(abs_path))
                } else {
                    app.lint_icon(abs_path)
                };
                match lint_run_count_for(app, abs_path, is_worktree_group) {
                    Some(0) | None => NO_LINT_RUNS.to_string(),
                    Some(n) => format!("{lint_icon} {n}"),
                }
            },
            Self::Ci => {
                let has_workflows = app
                    .git_info_for(data.abs_path.as_path())
                    .is_some_and(|git| git.workflows.is_present());
                if !has_workflows {
                    return NO_CI_WORKFLOW.to_string();
                }
                let icon = data.ci.map_or_else(String::new, |c| c.icon().to_string());
                let ci_runs_label = build_ci_runs_label(app, data.abs_path.as_path());
                if !ci_runs_label.is_empty() {
                    format!("{icon} {ci_runs_label}")
                } else if icon.is_empty() {
                    NO_CI_RUNS.to_string()
                } else {
                    icon
                }
            },
            Self::CratesIo => data.crates_version.as_deref().unwrap_or("").to_string(),
            Self::Downloads => data
                .crates_downloads
                .map_or_else(String::new, format_downloads),
            Self::Version => data.version.clone(),
            Self::WorktreeError => "broken .git — gitdir target missing".to_string(),
            // Git fields — should not be called with package_value.
            Self::Branch
            | Self::GitPath
            | Self::Sync
            | Self::VsOrigin
            | Self::VsLocal
            | Self::Origin
            | Self::Owner
            | Self::Repo
            | Self::Stars
            | Self::RepoDesc
            | Self::Inception
            | Self::LastCommit => String::new(),
        }
    }

    /// Get the display value for a git field from `GitData`.
    pub fn git_value(self, data: &GitData) -> String {
        match self {
            Self::Branch => {
                let branch = data.branch.as_deref().unwrap_or("");
                let is_default = data
                    .local_main_branch
                    .as_deref()
                    .is_some_and(|db| db == branch);
                if is_default {
                    format!("{branch} (HEAD)")
                } else {
                    branch.to_string()
                }
            },
            Self::GitPath => data.path_state.label_with_icon(),
            Self::Sync => data.sync.as_deref().unwrap_or("").to_string(),
            Self::VsOrigin => data.vs_origin.as_deref().unwrap_or("").to_string(),
            Self::VsLocal => data.vs_local.as_deref().unwrap_or("").to_string(),
            Self::Origin => data.origin.as_deref().unwrap_or("").to_string(),
            Self::Owner => data.owner.as_deref().unwrap_or("").to_string(),
            Self::Repo => data.url.as_deref().unwrap_or("").to_string(),
            Self::Stars => data
                .stars
                .map_or_else(String::new, |count| format!("⭐ {count}")),
            Self::RepoDesc => data.description.as_deref().unwrap_or("").to_string(),
            Self::Inception => data.inception.as_deref().unwrap_or("").to_string(),
            Self::LastCommit => data.last_commit.as_deref().unwrap_or("").to_string(),
            // Package fields — should not be called with git_value.
            Self::Path
            | Self::Disk
            | Self::Targets
            | Self::Lint
            | Self::Ci
            | Self::CratesIo
            | Self::Downloads
            | Self::Version
            | Self::WorktreeError => String::new(),
        }
    }
}

/// All fields for the `Package` column.
/// Non-Rust projects show only name, path, disk, and CI.
pub fn package_fields_from_data(data: &PackageData) -> Vec<DetailField> {
    if data.package_title == "Project" {
        return vec![
            DetailField::Path,
            DetailField::Disk,
            DetailField::Lint,
            DetailField::Ci,
        ];
    }
    let mut fields = vec![
        DetailField::Path,
        DetailField::Disk,
        DetailField::Targets,
        DetailField::Lint,
        DetailField::Ci,
    ];
    if data.has_package {
        fields.push(DetailField::Version);
    }
    if data.crates_version.is_some() {
        fields.push(DetailField::CratesIo);
    }
    if data.crates_downloads.is_some() {
        fields.push(DetailField::Downloads);
    }
    fields
}

pub fn git_fields_from_data(data: &GitData) -> Vec<DetailField> {
    let mut fields = Vec::new();
    if data.url.is_some() {
        fields.push(DetailField::Repo);
    }
    if data.owner.is_some() {
        fields.push(DetailField::Owner);
    }
    if data.branch.is_some() {
        fields.push(DetailField::Branch);
    }
    if data.path_state != GitPathState::OutsideRepo {
        fields.push(DetailField::GitPath);
    }
    if data.vs_origin.is_some() {
        fields.push(DetailField::VsOrigin);
    }
    if data.sync.is_some() {
        fields.push(DetailField::Sync);
    }
    if data.vs_local.is_some() {
        fields.push(DetailField::VsLocal);
    }
    if data.stars.is_some() {
        fields.push(DetailField::Stars);
    }
    if data.description.is_some() {
        fields.push(DetailField::RepoDesc);
    }
    if data.inception.is_some() {
        fields.push(DetailField::Inception);
    }
    if data.last_commit.is_some() {
        fields.push(DetailField::LastCommit);
    }
    if !data.worktree_names.is_empty() {
        // Worktree count is appended by the render function, not as a field.
    }
    fields
}

/// Per-pane data for the Package detail panel.
#[derive(Clone)]
pub struct PackageData {
    pub package_title:    String,
    pub title_name:       String,
    pub abs_path:         AbsolutePath,
    pub path:             String,
    pub version:          String,
    pub description:      Option<String>,
    pub crates_version:   Option<String>,
    pub crates_downloads: Option<u64>,
    pub types:            String,
    pub disk:             String,
    pub ci:               Option<Conclusion>,
    pub stats_rows:       Vec<(&'static str, usize)>,
    pub has_package:      bool,
}

/// Per-pane data for the Git detail panel.
#[derive(Clone)]
pub struct GitData {
    pub branch:            Option<String>,
    pub path_state:        GitPathState,
    pub sync:              Option<String>,
    pub vs_origin:         Option<String>,
    pub vs_local:          Option<String>,
    pub local_main_branch: Option<String>,
    pub main_branch_label: String,
    pub origin:            Option<String>,
    pub owner:             Option<String>,
    pub url:               Option<String>,
    pub stars:             Option<u64>,
    pub description:       Option<String>,
    pub inception:         Option<String>,
    pub last_commit:       Option<String>,
    pub worktree_names:    Vec<String>,
}

/// Per-pane data for the Targets panel.
#[derive(Clone)]
pub struct TargetsData {
    pub is_binary:   bool,
    pub binary_name: Option<String>,
    pub examples:    Vec<ExampleGroup>,
    pub benches:     Vec<String>,
}

impl TargetsData {
    pub const fn has_targets(&self) -> bool {
        self.is_binary || !self.examples.is_empty() || !self.benches.is_empty()
    }
}

#[derive(Clone)]
pub enum CiEmptyState {
    BranchScopedOnly,
    Loading,
    NoRuns,
    NoWorkflowConfigured,
    NotGitRepo,
    RequiresGithubRemote,
}

impl CiEmptyState {
    pub const fn title(&self) -> &'static str {
        match self {
            Self::BranchScopedOnly => " CI Runs — shown on branch/worktree rows ",
            Self::Loading => " CI Runs — loading… ",
            Self::NoRuns => " No CI Runs ",
            Self::NoWorkflowConfigured => " No CI workflow configured ",
            Self::NotGitRepo => " CI Runs — not a git repository ",
            Self::RequiresGithubRemote => " CI Runs — requires a GitHub origin remote ",
        }
    }
}

#[derive(Clone)]
pub struct CiData {
    pub runs:        Vec<CiRun>,
    pub mode_label:  Option<String>,
    pub empty_state: CiEmptyState,
}

impl CiData {
    pub const fn has_runs(&self) -> bool { !self.runs.is_empty() }
}

#[derive(Clone)]
pub struct LintsData {
    pub runs:            Vec<LintRun>,
    pub is_cargo_active: bool,
}

impl LintsData {
    pub const fn has_runs(&self) -> bool { !self.runs.is_empty() }
}

pub struct DetailPaneData {
    pub package: PackageData,
    pub git:     GitData,
    pub targets: TargetsData,
}

/// Resolve the title shown in the `Package` column header.
fn resolve_package_title(app: &App, item: &RootItem) -> String {
    if !item.is_rust() {
        return "Project".to_string();
    }
    if app.is_vendored_path(item.path()) {
        return "Vendored Crate".to_string();
    }
    if matches!(item, RootItem::Worktrees(_)) {
        return "Worktree Group".to_string();
    }
    if matches!(item, RootItem::Rust(RustProject::Workspace(_))) {
        return "Workspace".to_string();
    }
    if app.is_workspace_member_path(item.path()) {
        "Workspace Member".to_string()
    } else {
        "Package".to_string()
    }
}

/// Resolve the package title for a non-root package (member or vendored).
fn resolve_package_title_for_package(app: &App, pkg: &PackageProject) -> String {
    if app.is_vendored_path(pkg.path()) {
        "Vendored Crate".to_string()
    } else if app.is_workspace_member_path(pkg.path()) {
        "Workspace Member".to_string()
    } else {
        "Package".to_string()
    }
}

fn format_ahead_behind((ahead, behind): (usize, usize)) -> String {
    match (ahead, behind) {
        (0, 0) => IN_SYNC.to_string(),
        (ahead, 0) => format!("{SYNC_UP}{ahead} ahead"),
        (0, behind) => format!("{SYNC_DOWN}{behind} behind"),
        (ahead, behind) => format!("{SYNC_UP}{ahead} {SYNC_DOWN}{behind}"),
    }
}

pub(super) fn format_remote_status(ahead_behind: Option<(usize, usize)>) -> String {
    match ahead_behind {
        Some((0, 0)) => IN_SYNC.to_string(),
        Some((ahead, 0)) => format!("{SYNC_UP}{ahead} ahead"),
        Some((0, behind)) => format!("{SYNC_DOWN}{behind} behind"),
        Some((ahead, behind)) => format!("{SYNC_UP}{ahead} {SYNC_DOWN}{behind}"),
        None => NO_REMOTE_SYNC.to_string(),
    }
}

/// Format a download count with comma-separated thousands (e.g. `1,234,567`).
fn format_downloads(count: u64) -> String {
    let digits = count.to_string();
    let mut result = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, ch) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            result.push(',');
        }
        result.push(ch);
    }
    result
}

struct GitDetailFields {
    branch:            Option<String>,
    path:              GitPathState,
    sync:              Option<String>,
    vs_origin:         Option<String>,
    vs_local:          Option<String>,
    local_main_branch: Option<String>,
    main_branch_label: String,
    origin:            Option<String>,
    owner:             Option<String>,
    url:               Option<String>,
    stars:             Option<u64>,
    description:       Option<String>,
    inception:         Option<String>,
    last_commit:       Option<String>,
}

fn build_git_detail_fields(app: &App, abs_path: &Path) -> GitDetailFields {
    let owner_path = app
        .ci_owner_path_for(abs_path)
        .unwrap_or_else(|| AbsolutePath::from(abs_path));
    let git = app.git_info_for(owner_path.as_path());
    let branch = git.and_then(|info| info.branch.clone());
    let sync = git.map(|info| format_remote_status(info.ahead_behind));
    let vs_origin = git.map(|info| {
        info.upstream_branch.as_deref().map_or_else(
            || "none".to_string(),
            |branch| format!("{branch} (local cached ref)"),
        )
    });
    let vs_local = git
        .and_then(|info| info.ahead_behind_local)
        .map(format_ahead_behind);
    let local_main_branch = git.and_then(|info| info.local_main_branch.clone());
    let main_branch_label = app.current_config().tui.main_branch.clone();
    let origin = git.map(|info| format!("{} {}", info.origin.icon(), info.origin.label()));
    let owner = git.and_then(|info| info.owner.clone());
    let url = git.and_then(|info| info.url.clone());
    let github = app
        .projects()
        .at_path(owner_path.as_path())
        .and_then(|p| p.github_info.as_ref());
    let stars = github.map(|g| g.stars);
    let description = github.and_then(|g| g.description.clone());
    let inception = git
        .and_then(|info| info.first_commit.as_deref())
        .map(timestamp::format_timestamp);
    let last_commit = git
        .and_then(|info| info.last_commit.as_deref())
        .map(timestamp::format_timestamp);
    GitDetailFields {
        branch,
        path: app.git_path_state_for(abs_path),
        sync,
        vs_origin,
        vs_local,
        local_main_branch,
        main_branch_label,
        origin,
        owner,
        url,
        stars,
        description,
        inception,
        last_commit,
    }
}

/// Check whether a `RootItem` is a worktree group.
const fn is_worktree_group(item: &RootItem) -> bool { matches!(item, RootItem::Worktrees(_)) }

/// Collect worktree names from a worktree group item.
fn worktree_names_from_item(item: &RootItem) -> Vec<String> {
    match item {
        RootItem::Worktrees(WorktreeGroup::Workspaces {
            primary, linked, ..
        }) => std::iter::once(primary)
            .chain(linked.iter())
            .map(|ws| {
                ws.worktree_name()
                    .unwrap_or_else(|| ws.path().to_str().unwrap_or(""))
                    .to_string()
            })
            .collect(),
        RootItem::Worktrees(WorktreeGroup::Packages {
            primary, linked, ..
        }) => std::iter::once(primary)
            .chain(linked.iter())
            .map(|pkg| {
                pkg.worktree_name()
                    .unwrap_or_else(|| pkg.path().to_str().unwrap_or(""))
                    .to_string()
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// Build pane data for a root `RootItem`.
pub fn build_pane_data(app: &App, item: &RootItem) -> DetailPaneData {
    let display_path = item.display_path().into_string();
    let is_wt_group = is_worktree_group(item);

    match item {
        RootItem::Rust(RustProject::Workspace(ws)) => {
            build_pane_data_for_workspace(app, ws, &display_path, is_wt_group, Some(item))
        },
        RootItem::Rust(RustProject::Package(pkg)) => {
            build_pane_data_for_package(app, pkg, &display_path, is_wt_group, Some(item))
        },
        RootItem::NonRust(nr) => {
            build_pane_data_non_rust(app, nr, &display_path, is_wt_group, Some(item))
        },
        RootItem::Worktrees(WorktreeGroup::Workspaces { primary, .. }) => {
            build_pane_data_for_workspace(app, primary, &display_path, true, Some(item))
        },
        RootItem::Worktrees(WorktreeGroup::Packages { primary, .. }) => {
            build_pane_data_for_package(app, primary, &display_path, true, Some(item))
        },
    }
}

/// Build pane data for a `Project<Package>` (member or vendored row).
pub fn build_pane_data_for_member(app: &App, pkg: &PackageProject) -> DetailPaneData {
    let display_path = pkg.display_path().into_string();
    build_pane_data_for_package(app, pkg, &display_path, false, None)
}

/// Build pane data for a linked `Project<Workspace>` worktree entry.
pub fn build_pane_data_for_workspace_ref(
    app: &App,
    ws: &WorkspaceProject,
    display_path: &str,
) -> DetailPaneData {
    build_pane_data_for_workspace(app, ws, display_path, false, None)
}

/// Build pane data for a git submodule nested under a project.
pub fn build_pane_data_for_submodule(app: &App, submodule: &SubmoduleInfo) -> DetailPaneData {
    let abs_path = &submodule.path;
    let display_path = project::home_relative_path(abs_path);
    let git_detail = build_git_detail_fields(app, abs_path);

    let version = submodule.commit.as_deref().unwrap_or("-").to_string();
    let disk = submodule
        .info
        .disk_usage_bytes
        .map_or_else(String::new, crate::tui::render::format_bytes);

    DetailPaneData {
        package: PackageData {
            package_title: "Submodule".to_string(),
            title_name: submodule.name.clone(),
            abs_path: abs_path.clone(),
            path: display_path,
            version,
            description: submodule.url.clone(),
            crates_version: None,
            crates_downloads: None,
            types: String::new(),
            disk,
            ci: None,
            stats_rows: Vec::new(),
            has_package: false,
        },
        git:     GitData {
            branch:            git_detail.branch,
            path_state:        git_detail.path,
            sync:              git_detail.sync,
            vs_origin:         git_detail.vs_origin,
            vs_local:          git_detail.vs_local,
            local_main_branch: git_detail.local_main_branch,
            main_branch_label: git_detail.main_branch_label,
            origin:            git_detail.origin,
            owner:             git_detail.owner,
            url:               git_detail.url,
            stars:             git_detail.stars,
            description:       git_detail.description,
            inception:         git_detail.inception,
            last_commit:       git_detail.last_commit,
            worktree_names:    Vec::new(),
        },
        targets: TargetsData {
            is_binary:   false,
            binary_name: None,
            examples:    Vec::new(),
            benches:     Vec::new(),
        },
    }
}

fn build_pane_data_for_workspace(
    app: &App,
    ws: &WorkspaceProject,
    display_path: &str,
    is_wt_group: bool,
    wt_item: Option<&RootItem>,
) -> DetailPaneData {
    let abs_path = ws.path();
    let cargo = ws.cargo();

    let mut counts = ProjectCounts::default();
    counts.add_workspace(ws);
    if ws.has_members() {
        for group in ws.groups() {
            for member in group.members() {
                counts.add_package(member);
            }
        }
    }
    let stats_rows = counts.to_rows();

    let wt_item_ref = wt_item.filter(|_| is_wt_group);
    build_pane_data_common(
        app,
        PaneDataSource {
            abs_path,
            display_path,
            title_name: ws.package_name().into_string(),
            has_cargo: ws.name().is_some(),
            cargo: Some(cargo),
            wt_item: wt_item_ref,
            stats_rows,
            package_title: "Workspace".to_string(),
        },
    )
}

fn build_pane_data_for_package(
    app: &App,
    pkg: &PackageProject,
    display_path: &str,
    is_wt_group: bool,
    wt_item: Option<&RootItem>,
) -> DetailPaneData {
    let abs_path = pkg.path();
    let cargo = pkg.cargo();

    let mut counts = ProjectCounts::default();
    counts.add_package(pkg);
    let stats_rows = counts.to_rows();

    let wt_item_ref = wt_item.filter(|_| is_wt_group);
    let package_title = wt_item.map_or_else(
        || resolve_package_title_for_package(app, pkg),
        |item| resolve_package_title(app, item),
    );

    build_pane_data_common(
        app,
        PaneDataSource {
            abs_path,
            display_path,
            title_name: pkg.package_name().into_string(),
            has_cargo: true,
            cargo: Some(cargo),
            wt_item: wt_item_ref,
            stats_rows,
            package_title,
        },
    )
}

fn build_pane_data_non_rust(
    app: &App,
    nr: &NonRustProject,
    display_path: &str,
    is_wt_group: bool,
    wt_item: Option<&RootItem>,
) -> DetailPaneData {
    let abs_path = nr.path();
    let wt_item_ref = wt_item.filter(|_| is_wt_group);

    build_pane_data_common(
        app,
        PaneDataSource {
            abs_path,
            display_path,
            title_name: nr.root_directory_name().into_string(),
            has_cargo: false,
            cargo: None,
            wt_item: wt_item_ref,
            stats_rows: Vec::new(),
            package_title: "Project".to_string(),
        },
    )
}

struct PaneDataSource<'a> {
    abs_path:      &'a Path,
    display_path:  &'a str,
    title_name:    String,
    has_cargo:     bool,
    cargo:         Option<&'a Cargo>,
    wt_item:       Option<&'a RootItem>,
    stats_rows:    Vec<(&'static str, usize)>,
    package_title: String,
}

fn build_pane_data_common(app: &App, src: PaneDataSource<'_>) -> DetailPaneData {
    let abs_path = src.abs_path;
    let cargo = src.cargo;
    let wt_item = src.wt_item;
    let git_detail = build_git_detail_fields(app, abs_path);
    let rust_info = app.projects().rust_info_at_path(abs_path);
    let crates_version = rust_info.and_then(|r| r.crates_version().map(str::to_string));
    let crates_downloads = rust_info.and_then(crate::project::RustInfo::crates_downloads);

    let (disk, ci) = wt_item.map_or_else(
        || {
            let ci = if app.is_rust_at_path(abs_path) {
                app.ci_for(abs_path)
            } else {
                None
            };
            (app.formatted_disk(abs_path), ci)
        },
        |item| (App::formatted_disk_for_item(item), app.ci_for_item(item)),
    );

    let worktree_names = wt_item.map_or_else(Vec::new, worktree_names_from_item);

    let types_str = cargo.map_or_else(String::new, |c| {
        c.types()
            .iter()
            .map(std::string::ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    });

    let is_binary = cargo.is_some_and(Cargo::is_binary);

    let title_name = src.title_name;
    DetailPaneData {
        package: PackageData {
            package_title: src.package_title,
            title_name: title_name.clone(),
            abs_path: AbsolutePath::from(abs_path),
            path: src.display_path.to_string(),
            version: cargo.and_then(Cargo::version).unwrap_or("-").to_string(),
            description: cargo.and_then(Cargo::description).map(str::to_string),
            crates_version,
            crates_downloads,
            types: types_str,
            disk,
            ci,
            stats_rows: src.stats_rows,
            has_package: src.has_cargo,
        },
        git:     GitData {
            branch: git_detail.branch,
            path_state: git_detail.path,
            sync: git_detail.sync,
            vs_origin: git_detail.vs_origin,
            vs_local: git_detail.vs_local,
            local_main_branch: git_detail.local_main_branch,
            main_branch_label: git_detail.main_branch_label,
            origin: git_detail.origin,
            owner: git_detail.owner,
            url: git_detail.url,
            stars: git_detail.stars,
            description: git_detail.description,
            inception: git_detail.inception,
            last_commit: git_detail.last_commit,
            worktree_names,
        },
        targets: TargetsData {
            is_binary,
            binary_name: if is_binary { Some(title_name) } else { None },
            examples: cargo.map_or_else(Vec::new, |c| c.examples().to_vec()),
            benches: cargo.map_or_else(Vec::new, |c| c.benches().to_vec()),
        },
    }
}

pub fn build_ci_data(app: &App) -> CiData {
    let selected_path = app.selected_project_path();
    let has_ci_owner = app.selected_ci_path().is_some();
    let git_info = selected_path.and_then(|path| app.git_info_for(path));
    let empty_state = if selected_path.is_some() && !has_ci_owner {
        CiEmptyState::BranchScopedOnly
    } else if git_info.is_none() {
        CiEmptyState::NotGitRepo
    } else if has_ci_owner
        && git_info.is_some_and(|g| g.origin == GitOrigin::Local || g.url.is_none())
    {
        CiEmptyState::RequiresGithubRemote
    } else if git_info.is_some_and(|g| !g.workflows.is_present()) {
        CiEmptyState::NoWorkflowConfigured
    } else if !app.is_scan_complete() {
        CiEmptyState::Loading
    } else {
        CiEmptyState::NoRuns
    };

    CiData {
        runs: app.selected_ci_runs(),
        mode_label: selected_path.and_then(|path| {
            app.ci_toggle_available_for(path)
                .then(|| app.ci_display_mode_label_for(path).to_string())
        }),
        empty_state,
    }
}

pub fn build_lints_data(app: &App) -> LintsData {
    let selected_path = app.selected_project_path();
    LintsData {
        runs:            selected_path
            .and_then(|path| app.lint_at_path(path))
            .map(|lr| lr.runs().to_vec())
            .unwrap_or_default(),
        is_cargo_active: selected_path.is_some_and(|path| app.is_cargo_active_path(path)),
    }
}

/// Lint run count: for worktree groups, sum across all entries; for single
/// projects, return the run count at the given path.
fn lint_run_count_for(app: &App, abs_path: &Path, is_worktree_group: bool) -> Option<usize> {
    if is_worktree_group {
        let Some(RootItem::Worktrees(g)) = app
            .projects()
            .iter()
            .find(|item| item.path() == abs_path && matches!(item, RootItem::Worktrees(_)))
        else {
            return app.lint_at_path(abs_path).map(|lr| lr.runs().len());
        };
        let mut total = 0usize;
        let paths: Vec<AbsolutePath> = match g {
            WorktreeGroup::Workspaces {
                primary, linked, ..
            } => std::iter::once(primary.path())
                .chain(linked.iter().map(ProjectFields::path))
                .map(AbsolutePath::clone)
                .collect(),
            WorktreeGroup::Packages {
                primary, linked, ..
            } => std::iter::once(primary.path())
                .chain(linked.iter().map(ProjectFields::path))
                .map(AbsolutePath::clone)
                .collect(),
        };
        let mut any = false;
        for path in &paths {
            if let Some(lr) = app.lint_at_path(path) {
                total += lr.runs().len();
                any = true;
            }
        }
        return any.then_some(total);
    }
    app.lint_at_path(abs_path).map(|lr| lr.runs().len())
}

fn build_ci_runs_label(app: &App, abs_path: &Path) -> String {
    let ci_state = app.ci_state_for(abs_path);
    let Some(state) = ci_state else {
        return String::new();
    };
    let local = state.runs().len();
    let github_total = state.github_total();
    if github_total > 0 {
        format!("local {local} / github {github_total}")
    } else if local > 0 {
        format!("{local}")
    } else {
        String::new()
    }
}
