use std::path::Path;
use std::path::PathBuf;

use super::timestamp;
use crate::ci::Conclusion;
use crate::constants::IN_SYNC;
use crate::constants::NO_CI_RUNS;
use crate::constants::NO_CI_WORKFLOW;
use crate::constants::NO_LINT_RUNS;
use crate::constants::NO_LINT_RUNS_NOT_RUST;
use crate::constants::NO_REMOTE_SYNC;
use crate::constants::SYNC_DOWN;
use crate::constants::SYNC_UP;
use crate::project;
use crate::project::Cargo;
use crate::project::ExampleGroup;
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
use crate::project::WorktreeHealth;
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
                ProjectType::Library => self.libs += 1,
                ProjectType::Binary => self.bins += 1,
                ProjectType::ProcMacro => self.proc_macros += 1,
                ProjectType::BuildScript => {},
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
pub fn build_target_list(info: &DetailInfo) -> Vec<TargetEntry> {
    let mut entries = Vec::new();

    if info.is_binary
        && let Some(name) = &info.binary_name
    {
        entries.push(TargetEntry {
            display_name: name.clone(),
            name:         name.clone(),
            kind:         RunTargetKind::Binary,
        });
    }

    // Collect examples with category prefix for display, sorted with
    // categorized (containing '/') before uncategorized, then alphabetically.
    let mut examples: Vec<(String, String)> = info
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

    let mut bench_names = info.benches.clone();
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
    Worktree,
    WorktreeError,
    CratesIo,
    Version,
}

impl DetailField {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Path => "Path",
            Self::Targets => "Targets",
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
            Self::Worktree => "Worktree",
            Self::WorktreeError => "Error",
            Self::CratesIo => "crates.io",
            Self::Version => "Version",
        }
    }

    pub fn value(self, info: &DetailInfo, app: &App) -> String {
        match self {
            Self::Path => info.path.clone(),
            Self::Targets => info.types.clone(),
            Self::Disk => info.disk.clone(),
            Self::Lint => {
                if !app.is_rust_at_path(info.abs_path.as_path()) {
                    return NO_LINT_RUNS_NOT_RUST.to_string();
                }
                let abs_path = info.abs_path.as_path();
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
                    .git_info_for(info.abs_path.as_path())
                    .is_some_and(|git| git.workflows.is_present());
                if !has_workflows {
                    return NO_CI_WORKFLOW.to_string();
                }
                let icon = info.ci.map_or_else(String::new, |c| c.icon().to_string());
                let ci_runs_label = build_ci_runs_label(app, info.abs_path.as_path());
                if !ci_runs_label.is_empty() {
                    format!("{icon} {ci_runs_label}")
                } else if icon.is_empty() {
                    NO_CI_RUNS.to_string()
                } else {
                    icon
                }
            },
            Self::Branch => {
                let branch = info.git_branch.as_deref().unwrap_or("");
                let is_default = info
                    .local_main_branch
                    .as_deref()
                    .is_some_and(|db| db == branch);
                if is_default {
                    format!("{branch} (HEAD)")
                } else {
                    branch.to_string()
                }
            },
            Self::GitPath => info.git_path.label_with_icon(),
            Self::Sync => info.git_sync.as_deref().unwrap_or("").to_string(),
            Self::VsOrigin => info.git_vs_origin.as_deref().unwrap_or("").to_string(),
            Self::VsLocal => info.git_vs_local.as_deref().unwrap_or("").to_string(),
            Self::Origin => info.git_origin.as_deref().unwrap_or("").to_string(),
            Self::Owner => info.git_owner.as_deref().unwrap_or("").to_string(),
            Self::Repo => info.git_url.as_deref().unwrap_or("").to_string(),
            Self::Stars => info
                .git_stars
                .map_or_else(String::new, |count| format!("⭐ {count}")),
            Self::RepoDesc => info.repo_description.as_deref().unwrap_or("").to_string(),
            Self::Inception => info.git_inception.as_deref().unwrap_or("").to_string(),
            Self::LastCommit => info.git_last_commit.as_deref().unwrap_or("").to_string(),
            Self::Worktree => info.worktree_label.as_deref().unwrap_or("").to_string(),
            Self::WorktreeError => "broken .git — gitdir target missing".to_string(),
            Self::CratesIo => {
                let version = info.crates_version.as_deref().unwrap_or("");
                info.crates_downloads.map_or_else(
                    || version.to_string(),
                    |downloads| format!("{version} ({})", format_downloads(downloads)),
                )
            },
            Self::Version => info.version.clone(),
        }
    }
}

/// All fields for the `Package` column.
/// Non-Rust projects show only name, path, disk, and CI.
pub fn package_fields(info: &DetailInfo) -> Vec<DetailField> {
    if info.package_title == "Project" {
        let mut fields = vec![DetailField::Path];
        fields.push(DetailField::Lint);
        fields.push(DetailField::Ci);
        fields.push(DetailField::Disk);
        return fields;
    }
    let mut fields = vec![DetailField::Path, DetailField::Targets];
    fields.push(DetailField::Lint);
    fields.push(DetailField::Ci);
    fields.push(DetailField::Disk);
    if info.crates_version.is_some() {
        fields.push(DetailField::CratesIo);
    }
    if info.has_package {
        fields.push(DetailField::Version);
    }
    fields
}

/// Git fields (right column). Only includes fields that have data.
pub fn git_fields(info: &DetailInfo) -> Vec<DetailField> {
    let mut fields = Vec::new();
    if info.git_url.is_some() {
        fields.push(DetailField::Repo);
    }
    if info.git_owner.is_some() {
        fields.push(DetailField::Owner);
    }
    if info.git_branch.is_some() {
        fields.push(DetailField::Branch);
    }
    if info.git_path != GitPathState::OutsideRepo {
        fields.push(DetailField::GitPath);
    }
    if info.git_vs_origin.is_some() {
        fields.push(DetailField::VsOrigin);
    }
    if info.git_sync.is_some() {
        fields.push(DetailField::Sync);
    }
    if info.git_vs_local.is_some() {
        fields.push(DetailField::VsLocal);
    }
    if info.worktree_label.is_some() {
        fields.push(DetailField::Worktree);
    }
    if matches!(info.worktree_health, WorktreeHealth::Broken) {
        fields.push(DetailField::WorktreeError);
    }
    if info.git_stars.is_some() {
        fields.push(DetailField::Stars);
    }
    if info.repo_description.is_some() {
        fields.push(DetailField::RepoDesc);
    }
    if info.git_inception.is_some() {
        fields.push(DetailField::Inception);
    }
    if info.git_last_commit.is_some() {
        fields.push(DetailField::LastCommit);
    }
    fields
}

#[derive(Clone)]
pub struct DetailInfo {
    pub package_title:     String,
    pub name:              String,
    /// Primary name for the detail title bar. For Rust projects this is the
    /// Cargo package name; for non-Rust projects this is the directory leaf.
    pub title_name:        String,
    pub abs_path:          PathBuf,
    pub path:              String,
    pub version:           String,
    pub description:       Option<String>,
    pub crates_version:    Option<String>,
    pub crates_downloads:  Option<u64>,
    pub types:             String,
    pub disk:              String,
    pub ci:                Option<Conclusion>,
    pub stats_rows:        Vec<(&'static str, usize)>,
    pub git_branch:        Option<String>,
    pub git_path:          GitPathState,
    pub git_sync:          Option<String>,
    pub git_vs_origin:     Option<String>,
    /// Ahead/behind vs local `{local_main_branch}`.
    pub git_vs_local:      Option<String>,
    /// The actual local branch used for `M` comparisons.
    pub local_main_branch: Option<String>,
    /// The configured user-facing label for the local main branch.
    pub main_branch_label: String,
    pub git_origin:        Option<String>,
    pub git_owner:         Option<String>,
    pub git_url:           Option<String>,
    pub git_stars:         Option<u64>,
    pub repo_description:  Option<String>,
    pub git_inception:     Option<String>,
    pub git_last_commit:   Option<String>,
    pub worktree_label:    Option<String>,
    pub worktree_health:   WorktreeHealth,
    pub worktree_names:    Vec<String>,
    pub is_binary:         bool,
    pub binary_name:       Option<String>,
    pub examples:          Vec<ExampleGroup>,
    pub benches:           Vec<String>,
    /// Whether this project declares `[package]` (has version/description fields).
    pub has_package:       bool,
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
        .unwrap_or_else(|| abs_path.to_path_buf());
    let git = app
        .git_info_for(abs_path)
        .or_else(|| app.git_info_for(owner_path.as_path()));
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
    let stars = app
        .stars()
        .get(abs_path)
        .copied()
        .or_else(|| app.stars().get(owner_path.as_path()).copied());
    let description = app
        .repo_descriptions()
        .get(abs_path)
        .cloned()
        .or_else(|| app.repo_descriptions().get(owner_path.as_path()).cloned());
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

/// Build `DetailInfo` for a root `RootItem`.
pub fn build_detail_info(app: &App, item: &RootItem) -> DetailInfo {
    let display_path = item.display_path().into_string();
    let is_wt_group = is_worktree_group(item);

    match item {
        RootItem::Rust(RustProject::Workspace(ws)) => {
            build_detail_info_for_workspace(app, ws, &display_path, is_wt_group, Some(item))
        },
        RootItem::Rust(RustProject::Package(pkg)) => {
            build_detail_info_for_package(app, pkg, &display_path, is_wt_group, Some(item))
        },
        RootItem::NonRust(nr) => {
            build_detail_info_non_rust(app, nr, &display_path, is_wt_group, Some(item))
        },
        RootItem::Worktrees(WorktreeGroup::Workspaces { primary, .. }) => {
            build_detail_info_for_workspace(app, primary, &display_path, true, Some(item))
        },
        RootItem::Worktrees(WorktreeGroup::Packages { primary, .. }) => {
            build_detail_info_for_package(app, primary, &display_path, true, Some(item))
        },
    }
}

/// Build `DetailInfo` for a `Project<Package>` (member or vendored row).
pub fn build_detail_info_for_member(app: &App, pkg: &PackageProject) -> DetailInfo {
    let display_path = pkg.display_path().into_string();
    build_detail_info_for_package(app, pkg, &display_path, false, None)
}

/// Build `DetailInfo` for a linked `Project<Workspace>` worktree entry.
pub fn build_detail_info_for_workspace_ref(
    app: &App,
    ws: &WorkspaceProject,
    display_path: &str,
) -> DetailInfo {
    build_detail_info_for_workspace(app, ws, display_path, false, None)
}

/// Build `DetailInfo` for a git submodule nested under a project.
pub fn build_detail_info_for_submodule(app: &App, submodule: &SubmoduleInfo) -> DetailInfo {
    let abs_path = &submodule.path;
    let display_path = project::home_relative_path(abs_path);
    let git_detail = build_git_detail_fields(app, abs_path);

    let version = submodule.commit.as_deref().unwrap_or("-").to_string();
    let disk = submodule
        .info
        .disk_usage_bytes
        .map_or_else(String::new, crate::tui::render::format_bytes);

    DetailInfo {
        package_title: "Submodule".to_string(),
        name: submodule.name.clone(),
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
        git_branch: git_detail.branch,
        git_path: git_detail.path,
        git_sync: git_detail.sync,
        git_vs_origin: git_detail.vs_origin,
        git_vs_local: git_detail.vs_local,
        local_main_branch: git_detail.local_main_branch,
        main_branch_label: git_detail.main_branch_label,
        git_origin: git_detail.origin,
        git_owner: git_detail.owner,
        git_url: git_detail.url,
        git_stars: git_detail.stars,
        repo_description: git_detail.description,
        git_inception: git_detail.inception,
        git_last_commit: git_detail.last_commit,
        worktree_label: None,
        worktree_health: WorktreeHealth::Normal,
        worktree_names: Vec::new(),
        is_binary: false,
        binary_name: None,
        examples: Vec::new(),
        benches: Vec::new(),
        has_package: false,
    }
}

fn build_detail_info_for_workspace(
    app: &App,
    ws: &WorkspaceProject,
    display_path: &str,
    is_wt_group: bool,
    wt_item: Option<&RootItem>,
) -> DetailInfo {
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
    build_detail_info_common(
        app,
        DetailSource {
            abs_path,
            display_path,
            title_name: ws.package_name().into_string(),
            has_cargo: ws.name().is_some(),
            cargo: Some(cargo),
            worktree_name: ws.worktree_name(),
            worktree_health: ws.worktree_health(),
            wt_item: wt_item_ref,
            stats_rows,
            package_title: "Workspace".to_string(),
        },
    )
}

fn build_detail_info_for_package(
    app: &App,
    pkg: &PackageProject,
    display_path: &str,
    is_wt_group: bool,
    wt_item: Option<&RootItem>,
) -> DetailInfo {
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

    build_detail_info_common(
        app,
        DetailSource {
            abs_path,
            display_path,
            title_name: pkg.package_name().into_string(),
            has_cargo: true,
            cargo: Some(cargo),
            worktree_name: pkg.worktree_name(),
            worktree_health: pkg.worktree_health(),
            wt_item: wt_item_ref,
            stats_rows,
            package_title,
        },
    )
}

fn build_detail_info_non_rust(
    app: &App,
    nr: &NonRustProject,
    display_path: &str,
    is_wt_group: bool,
    wt_item: Option<&RootItem>,
) -> DetailInfo {
    let abs_path = nr.path();
    let wt_item_ref = wt_item.filter(|_| is_wt_group);

    build_detail_info_common(
        app,
        DetailSource {
            abs_path,
            display_path,
            title_name: nr.root_directory_name().into_string(),
            has_cargo: false,
            cargo: None,
            worktree_name: None,
            worktree_health: nr.worktree_health(),
            wt_item: wt_item_ref,
            stats_rows: Vec::new(),
            package_title: "Project".to_string(),
        },
    )
}

struct DetailSource<'a> {
    abs_path:        &'a Path,
    display_path:    &'a str,
    title_name:      String,
    has_cargo:       bool,
    cargo:           Option<&'a Cargo>,
    worktree_name:   Option<&'a str>,
    worktree_health: WorktreeHealth,
    wt_item:         Option<&'a RootItem>,
    stats_rows:      Vec<(&'static str, usize)>,
    package_title:   String,
}

fn build_detail_info_common(app: &App, src: DetailSource<'_>) -> DetailInfo {
    let abs_path = src.abs_path;
    let cargo = src.cargo;
    let wt_item = src.wt_item;
    let git_detail = build_git_detail_fields(app, abs_path);
    let crates_version = app.crates_versions().get(abs_path).cloned();
    let crates_downloads = app.crates_downloads().get(abs_path).copied();

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
    let binary_name = if is_binary {
        Some(src.title_name.clone())
    } else {
        None
    };

    DetailInfo {
        package_title: src.package_title,
        name: src.title_name.clone(),
        title_name: src.title_name,
        abs_path: abs_path.to_path_buf(),
        path: src.display_path.to_string(),
        version: cargo.and_then(Cargo::version).unwrap_or("-").to_string(),
        description: cargo.and_then(Cargo::description).map(str::to_string),
        crates_version,
        crates_downloads,
        types: types_str,
        disk,
        ci,
        stats_rows: src.stats_rows,
        git_branch: git_detail.branch,
        git_path: git_detail.path,
        git_sync: git_detail.sync,
        git_vs_origin: git_detail.vs_origin,
        git_vs_local: git_detail.vs_local,
        local_main_branch: git_detail.local_main_branch,
        main_branch_label: git_detail.main_branch_label,
        git_origin: git_detail.origin,
        git_owner: git_detail.owner,
        git_url: git_detail.url,
        git_stars: git_detail.stars,
        repo_description: git_detail.description,
        git_inception: git_detail.inception,
        git_last_commit: git_detail.last_commit,
        worktree_label: src.worktree_name.map(str::to_string),
        worktree_health: src.worktree_health,
        worktree_names,
        is_binary,
        binary_name,
        examples: cargo.map_or_else(Vec::new, |c| c.examples().to_vec()),
        benches: cargo.map_or_else(Vec::new, |c| c.benches().to_vec()),
        has_package: src.has_cargo,
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
        let paths: Vec<std::path::PathBuf> = match g {
            WorktreeGroup::Workspaces {
                primary, linked, ..
            } => std::iter::once(primary.path())
                .chain(linked.iter().map(ProjectFields::path))
                .map(std::path::Path::to_path_buf)
                .collect(),
            WorktreeGroup::Packages {
                primary, linked, ..
            } => std::iter::once(primary.path())
                .chain(linked.iter().map(ProjectFields::path))
                .map(std::path::Path::to_path_buf)
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
