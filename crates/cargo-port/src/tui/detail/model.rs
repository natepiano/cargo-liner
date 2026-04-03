use super::App;
use super::format_timestamp;
use crate::ci::Conclusion;
use crate::constants::IN_SYNC;
use crate::constants::SYNC_DOWN;
use crate::constants::SYNC_UP;
use crate::project::ExampleGroup;
use crate::project::GitOrigin;
use crate::project::GitPathState;
use crate::project::ProjectLanguage;
use crate::project::ProjectType;
use crate::project::RustProject;

#[derive(Default)]
pub struct ProjectCounts {
    pub workspaces: usize,
    pub libs: usize,
    pub bins: usize,
    pub proc_macros: usize,
    pub examples: usize,
    pub benches: usize,
    pub tests: usize,
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

#[derive(Clone, Copy)]
pub enum RunTargetKind {
    Binary,
    Example,
    Bench,
}

impl RunTargetKind {
    pub const BINARY_COLOR: ratatui::style::Color = ratatui::style::Color::Green;
    pub const EXAMPLE_COLOR: ratatui::style::Color = ratatui::style::Color::Cyan;
    pub const BENCH_COLOR: ratatui::style::Color = ratatui::style::Color::Magenta;

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
}

pub struct TargetEntry {
    pub name: String,
    pub display_name: String,
    pub kind: RunTargetKind,
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
            name: name.clone(),
            kind: RunTargetKind::Binary,
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
    pub abs_path: String,
    pub target_name: String,
    pub package_name: Option<String>,
    pub kind: RunTargetKind,
    pub release: bool,
}

/// Whether a CI fetch should look for older runs or just refresh for new ones.
#[derive(Clone, Copy)]
pub enum CiFetchKind {
    /// Increment the limit to discover older history.
    FetchOlder,
    /// Re-fetch at the current limit to pick up newly created runs.
    Refresh,
}

/// A pending request to fetch more CI runs for a project.
pub struct PendingCiFetch {
    pub project_path: String,
    pub current_count: u32,
    pub kind: CiFetchKind,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DetailField {
    Name,
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
    CratesIo,
    Version,
    Description,
}

impl DetailField {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Name => "Name",
            Self::Path => "Path",
            Self::Targets => "Targets",
            Self::Disk => "Disk",
            Self::Lint => "Lint",
            Self::Ci => "CI",
            Self::Branch => "Branch",
            Self::GitPath => "Git Path",
            Self::Sync => "Sync",
            Self::VsOrigin => "vs o/dflt",
            Self::VsLocal => "vs dflt",
            Self::Origin => "Origin",
            Self::Owner => "Owner",
            Self::Repo => "Repo",
            Self::Stars => "Stars",
            Self::RepoDesc => "About",
            Self::Inception => "Incept",
            Self::LastCommit => "Latest",
            Self::Worktree => "Worktree",
            Self::CratesIo => "crates.io",
            Self::Version => "Version",
            Self::Description => "Desc",
        }
    }

    pub const fn is_from_cargo_toml(self) -> bool {
        matches!(
            self,
            Self::Name | Self::Targets | Self::Version | Self::Description
        )
    }

    pub fn value(self, info: &DetailInfo) -> String {
        match self {
            Self::Name => info.name.clone(),
            Self::Path => info.path.clone(),
            Self::Targets => info.types.clone(),
            Self::Disk => info.disk.clone(),
            Self::Lint => info.lint_label.clone(),
            Self::Ci => info.ci.map_or_else(String::new, |c| c.icon().to_string()),
            Self::Branch => {
                let branch = info.git_branch.as_deref().unwrap_or("");
                let is_default = info
                    .default_branch
                    .as_deref()
                    .is_some_and(|db| db == branch);
                if is_default {
                    format!("{branch} (HEAD)")
                } else {
                    branch.to_string()
                }
            },
            Self::GitPath => info.git_path.label().to_string(),
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
            Self::CratesIo => {
                let version = info.crates_version.as_deref().unwrap_or("");
                info.crates_downloads.map_or_else(
                    || version.to_string(),
                    |downloads| format!("{version} ({})", format_downloads(downloads)),
                )
            },
            Self::Version => info.version.clone(),
            Self::Description => info.description.as_deref().unwrap_or("—").to_string(),
        }
    }
}

/// All fields for the `Package` column.
/// Non-Rust projects show only name, path, disk, and CI.
pub fn package_fields(info: &DetailInfo) -> Vec<DetailField> {
    if info.is_rust == ProjectLanguage::NonRust {
        let mut fields = vec![DetailField::Name, DetailField::Path];
        if info.cargo_active && !info.lint_label.is_empty() {
            fields.push(DetailField::Lint);
        }
        if info.cargo_active {
            fields.push(DetailField::Ci);
        }
        fields.push(DetailField::Disk);
        return fields;
    }
    let mut fields = vec![DetailField::Name, DetailField::Path, DetailField::Targets];
    if info.cargo_active && !info.lint_label.is_empty() {
        fields.push(DetailField::Lint);
    }
    if info.cargo_active {
        fields.push(DetailField::Ci);
    }
    fields.push(DetailField::Disk);
    if info.cargo_active && info.crates_version.is_some() {
        fields.push(DetailField::CratesIo);
    }
    if info.has_package {
        fields.push(DetailField::Version);
        fields.push(DetailField::Description);
    }
    fields
}

/// Git fields (right column). Only includes fields that have data.
pub fn git_fields(info: &DetailInfo) -> Vec<DetailField> {
    let mut fields = Vec::new();
    if info.git_branch.is_some() {
        fields.push(DetailField::Branch);
    }
    if info.git_path != GitPathState::OutsideRepo {
        fields.push(DetailField::GitPath);
    }
    if info.git_sync.is_some() {
        fields.push(DetailField::Sync);
    }
    if info.git_vs_origin.is_some() {
        fields.push(DetailField::VsOrigin);
    }
    if info.git_vs_local.is_some() {
        fields.push(DetailField::VsLocal);
    }
    if info.worktree_label.is_some() {
        fields.push(DetailField::Worktree);
    }
    if info.git_origin.is_some() {
        fields.push(DetailField::Origin);
    }
    if info.git_url.is_some() {
        fields.push(DetailField::Repo);
    }
    if info.git_owner.is_some() {
        fields.push(DetailField::Owner);
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
    pub package_title: String,
    pub name: String,
    pub path: String,
    pub version: String,
    pub description: Option<String>,
    pub crates_version: Option<String>,
    pub crates_downloads: Option<u64>,
    pub types: String,
    pub disk: String,
    pub lint_label: String,
    pub ci: Option<Conclusion>,
    pub stats_rows: Vec<(&'static str, usize)>,
    pub git_branch: Option<String>,
    pub git_path: GitPathState,
    pub git_sync: Option<String>,
    /// Ahead/behind vs `origin/{default_branch}`.
    pub git_vs_origin: Option<String>,
    /// Ahead/behind vs local `{default_branch}`.
    pub git_vs_local: Option<String>,
    /// The repo's default branch name (e.g. "main", "master").
    pub default_branch: Option<String>,
    pub git_origin: Option<String>,
    pub git_owner: Option<String>,
    pub git_url: Option<String>,
    pub git_stars: Option<u64>,
    pub repo_description: Option<String>,
    pub git_inception: Option<String>,
    pub git_last_commit: Option<String>,
    pub worktree_label: Option<String>,
    pub worktree_names: Vec<String>,
    pub is_binary: bool,
    pub binary_name: Option<String>,
    pub examples: Vec<ExampleGroup>,
    pub benches: Vec<String>,
    /// Whether this is a Rust project (has `Cargo.toml`).
    pub is_rust: ProjectLanguage,
    /// Whether this project declares `[package]` (has version/description fields).
    pub has_package: bool,
    pub cargo_active: bool,
}

/// Resolve the title shown in the `Package` column header.
fn resolve_package_title(app: &App, project: &RustProject) -> String {
    if project.is_rust == ProjectLanguage::NonRust {
        return "Project".to_string();
    }
    if app.is_vendored_path(&project.path) {
        return "Vendored Crate".to_string();
    }
    if project.is_workspace() {
        return "Workspace".to_string();
    }
    if app.is_workspace_member_path(&project.path) {
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
    branch: Option<String>,
    path: GitPathState,
    sync: Option<String>,
    vs_origin: Option<String>,
    vs_local: Option<String>,
    default_branch: Option<String>,
    origin: Option<String>,
    owner: Option<String>,
    url: Option<String>,
    stars: Option<u64>,
    description: Option<String>,
    inception: Option<String>,
    last_commit: Option<String>,
}

fn build_git_detail_fields(app: &App, project: &RustProject) -> GitDetailFields {
    let git = app.git_info.get(&project.path);
    let branch = git.and_then(|info| info.branch.clone());
    let sync = git
        .map(|info| match info.ahead_behind {
            Some((0, 0)) => IN_SYNC.to_string(),
            Some((ahead, 0)) => format!("{SYNC_UP}{ahead} ahead"),
            Some((0, behind)) => format!("{SYNC_DOWN}{behind} behind"),
            Some((ahead, behind)) => format!("{SYNC_UP}{ahead} {SYNC_DOWN}{behind}"),
            None if info.origin != GitOrigin::Local => "not published".to_string(),
            None => String::new(),
        })
        .filter(|value| !value.is_empty());
    let vs_origin = git
        .and_then(|info| info.ahead_behind_origin)
        .map(format_ahead_behind);
    let vs_local = git
        .and_then(|info| info.ahead_behind_local)
        .map(format_ahead_behind);
    let default_branch = git.and_then(|info| info.default_branch.clone());
    let origin = git.map(|info| format!("{} {}", info.origin.icon(), info.origin.label()));
    let owner = git.and_then(|info| info.owner.clone());
    let url = git.and_then(|info| info.url.clone());
    let stars = app.stars.get(&project.path).copied();
    let description = app.repo_descriptions.get(&project.path).cloned();
    let inception = git
        .and_then(|info| info.first_commit.as_deref())
        .map(format_timestamp);
    let last_commit = git
        .and_then(|info| info.last_commit.as_deref())
        .map(format_timestamp);
    GitDetailFields {
        branch,
        path: app.git_path_state_for(&project.path),
        sync,
        vs_origin,
        vs_local,
        default_branch,
        origin,
        owner,
        url,
        stars,
        description,
        inception,
        last_commit,
    }
}

pub fn build_detail_info(app: &App, project: &RustProject) -> DetailInfo {
    let mut counts = app.workspace_counts(project).unwrap_or_else(|| {
        let mut counts = ProjectCounts::default();
        counts.add_project(project);
        counts
    });
    if !project.is_workspace() {
        counts.examples = project.example_count();
        counts.benches = project.benches.len();
        counts.tests = project.test_count;
    }
    let stats_rows = counts.to_rows();

    let git_detail = build_git_detail_fields(app, project);
    let crates_version = app.crates_versions.get(&project.path).cloned();
    let crates_downloads = app.crates_downloads.get(&project.path).copied();
    let worktree_label = project.worktree_name.clone();
    let cargo_active = app.is_cargo_active_path(&project.path);

    let worktree_node = app
        .selected_node()
        .filter(|node| node.project.path == project.path && !node.worktrees.is_empty());

    let (disk, ci) = worktree_node.map_or_else(
        || {
            (
                app.formatted_disk(project),
                if cargo_active {
                    app.ci_for(project)
                } else {
                    None
                },
            )
        },
        |node| (app.formatted_disk_for_node(node), app.ci_for_node(node)),
    );

    let package_title = resolve_package_title(app, project);

    let worktree_names: Vec<String> = worktree_node.map_or_else(Vec::new, |node| {
        node.worktrees
            .iter()
            .map(|worktree| {
                worktree
                    .project
                    .worktree_name
                    .as_deref()
                    .unwrap_or(&worktree.project.path)
                    .to_string()
            })
            .collect()
    });

    let is_binary = project
        .types
        .iter()
        .any(|project_type| matches!(project_type, ProjectType::Binary));
    let binary_name = if is_binary {
        project.name.clone()
    } else {
        None
    };

    DetailInfo {
        package_title,
        name: project.name.clone().unwrap_or_else(|| "-".to_string()),
        path: project.path.clone(),
        version: project.version.clone().unwrap_or_else(|| "-".to_string()),
        description: project.description.clone(),
        crates_version,
        crates_downloads,
        types: project
            .types
            .iter()
            .map(std::string::ToString::to_string)
            .collect::<Vec<_>>()
            .join(", "),
        disk,
        lint_label: app
            .selected_lint_icon(project)
            .map_or_else(String::new, std::string::ToString::to_string),
        ci,
        stats_rows,
        git_branch: git_detail.branch,
        git_path: git_detail.path,
        git_sync: git_detail.sync,
        git_vs_origin: git_detail.vs_origin,
        git_vs_local: git_detail.vs_local,
        default_branch: git_detail.default_branch,
        git_origin: git_detail.origin,
        git_owner: git_detail.owner,
        git_url: git_detail.url,
        git_stars: git_detail.stars,
        repo_description: git_detail.description,
        git_inception: git_detail.inception,
        git_last_commit: git_detail.last_commit,
        worktree_label,
        worktree_names,
        is_binary,
        binary_name,
        examples: project.examples.clone(),
        benches: project.benches.clone(),
        is_rust: project.is_rust,
        has_package: project.name.is_some(),
        cargo_active,
    }
}
