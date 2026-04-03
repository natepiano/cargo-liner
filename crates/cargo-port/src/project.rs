use std::collections::HashMap;
use std::collections::HashSet;
use std::fmt;
use std::io;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

use serde::Deserialize;
use serde::Serialize;
use toml::Table;
use toml::Value;

use crate::constants::GIT_CLONE;
use crate::constants::GIT_FORK;
use crate::constants::GIT_LOCAL;
use crate::perf_log;

/// Whether a project is a plain clone or a fork (has an "upstream" remote).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum GitOrigin {
    /// A local-only repo (no origin remote).
    Local,
    /// A plain git clone (has "origin" remote).
    Clone,
    /// A fork (has an "upstream" remote).
    Fork,
}

impl GitOrigin {
    pub const fn icon(self) -> &'static str {
        match self {
            Self::Local => GIT_LOCAL,
            Self::Clone => GIT_CLONE,
            Self::Fork => GIT_FORK,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Clone => "clone",
            Self::Fork => "fork",
        }
    }
}

/// Git metadata for a project: origin type, owner, repo URL, and current branch.
#[derive(Debug, Clone, Serialize)]
pub struct GitInfo {
    /// Whether this is a clone or a fork.
    pub origin:              GitOrigin,
    /// The current branch name.
    pub branch:              Option<String>,
    /// The GitHub/GitLab owner (e.g. "natepiano").
    pub owner:               Option<String>,
    /// The HTTPS URL to the repository.
    pub url:                 Option<String>,
    /// ISO 8601 date of the first commit (inception).
    pub first_commit:        Option<String>,
    /// ISO 8601 date of the most recent commit.
    pub last_commit:         Option<String>,
    /// Commits ahead and behind the upstream tracking branch (ahead, behind).
    pub ahead_behind:        Option<(usize, usize)>,
    /// The repo's default branch name resolved from `origin/HEAD`.
    pub default_branch:      Option<String>,
    /// Commits ahead and behind `origin/{default_branch}`.
    pub ahead_behind_origin: Option<(usize, usize)>,
    /// Commits ahead and behind the local `{default_branch}`.
    pub ahead_behind_local:  Option<(usize, usize)>,
}

impl GitInfo {
    /// Detect git info for a project directory.
    pub fn detect(project_dir: &Path) -> Option<Self> {
        let repo_root = git_repo_root(project_dir)?;
        let mut info = Self::detect_fast(&repo_root)?;
        info.first_commit = detect_first_commit(&repo_root);
        Some(info)
    }

    /// Detect the subset of git info needed on the startup critical path.
    pub fn detect_fast(project_dir: &Path) -> Option<Self> {
        let repo_root = git_repo_root(project_dir)?;

        let remote_output = git_output_logged(&repo_root, "remote", ["remote"]).ok()?;
        let remotes = String::from_utf8_lossy(&remote_output.stdout);
        let has_origin = remotes.lines().any(|line| line.trim() == "origin");
        let has_upstream = remotes.lines().any(|line| line.trim() == "upstream");
        let origin = if !has_origin {
            GitOrigin::Local
        } else if has_upstream {
            GitOrigin::Fork
        } else {
            GitOrigin::Clone
        };

        let url_output = git_output_logged(
            &repo_root,
            "remote_get_url_origin",
            ["remote", "get-url", "origin"],
        )
        .ok()?;
        let raw_url = String::from_utf8_lossy(&url_output.stdout)
            .trim()
            .to_string();

        let (owner, url) = parse_remote_url(&raw_url);

        let branch = git_output_logged(
            &repo_root,
            "rev_parse_head",
            ["rev-parse", "--abbrev-ref", "HEAD"],
        )
        .ok()
        .and_then(|o| {
            let b = String::from_utf8_lossy(&o.stdout).trim().to_string();
            if b.is_empty() { None } else { Some(b) }
        });

        let ahead_behind = parse_ahead_behind(&repo_root, "HEAD...@{upstream}", "upstream");

        // Resolve the repo's default branch from origin/HEAD (e.g. "origin/main").
        let default_branch = git_output_logged(
            &repo_root,
            "symbolic_ref_origin_head",
            ["symbolic-ref", "refs/remotes/origin/HEAD", "--short"],
        )
        .ok()
        .and_then(|o| {
            let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
            // Comes back as "origin/main" — strip the "origin/" prefix.
            s.strip_prefix("origin/")
                .filter(|b| !b.is_empty())
                .map(str::to_string)
        });

        // Compare HEAD against the default branch when it differs from the current branch.
        let not_on_default = default_branch
            .as_deref()
            .filter(|db| branch.as_deref() != Some(*db));
        let ahead_behind_origin = not_on_default.and_then(|db| {
            parse_ahead_behind(&repo_root, &format!("HEAD...origin/{db}"), "default_origin")
        });
        let ahead_behind_local = not_on_default.and_then(|db| {
            parse_ahead_behind(&repo_root, &format!("HEAD...{db}"), "default_local")
        });

        let last_commit =
            git_output_logged(&repo_root, "log_last_commit", ["log", "-1", "--format=%aI"])
                .ok()
                .and_then(|o| {
                    let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
                    if s.is_empty() { None } else { Some(s) }
                });

        Some(Self {
            origin,
            branch,
            owner,
            url,
            first_commit: None,
            last_commit,
            ahead_behind,
            default_branch,
            ahead_behind_origin,
            ahead_behind_local,
        })
    }
}

pub fn detect_first_commit(project_dir: &Path) -> Option<String> {
    let repo_root = git_repo_root(project_dir)?;
    git_output_logged(
        &repo_root,
        "log_first_commit",
        [
            "log",
            "--max-parents=0",
            "--reverse",
            "--format=%aI",
            "HEAD",
        ],
    )
    .ok()
    .and_then(|o| {
        String::from_utf8_lossy(&o.stdout)
            .lines()
            .next()
            .filter(|s| !s.is_empty())
            .map(std::string::ToString::to_string)
    })
}

fn git_output_logged<const N: usize>(
    repo_root: &Path,
    op: &str,
    args: [&str; N],
) -> io::Result<std::process::Output> {
    let started = std::time::Instant::now();
    let output = Command::new("git")
        .args(args)
        .current_dir(repo_root)
        .output();
    let status = output
        .as_ref()
        .ok()
        .and_then(|out| out.status.code())
        .map_or_else(|| "signal".to_string(), |code| code.to_string());
    perf_log::log_duration(
        "git_info_detect_call",
        started.elapsed(),
        &format!(
            "repo_root={} op={} status={status}",
            repo_root.display(),
            op,
        ),
        0,
    );
    output
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum GitPathState {
    #[default]
    OutsideRepo,
    Clean,
    Modified,
    Untracked,
    Ignored,
}

impl GitPathState {
    pub const fn label(self) -> &'static str {
        match self {
            Self::OutsideRepo => "outside repo",
            Self::Clean => "clean",
            Self::Modified => "modified",
            Self::Untracked => "untracked",
            Self::Ignored => "ignored",
        }
    }
}

type ProjectPathEntry = (String, String);
type GitPathStatesByProject = HashMap<String, GitPathState>;
type ProjectsByRepoRoot = HashMap<PathBuf, Vec<ProjectPathEntry>>;

pub fn detect_git_path_state(project_dir: &Path) -> GitPathState {
    let started = std::time::Instant::now();
    let Some(repo_root) = git_repo_root(project_dir) else {
        return GitPathState::OutsideRepo;
    };
    let relative_path = relative_git_path(&repo_root, project_dir);
    if relative_path != "." {
        let ignored = Command::new("git")
            .args(["check-ignore", "-q", "--", &relative_path])
            .current_dir(&repo_root)
            .status()
            .ok()
            .is_some_and(|status| status.success());
        if ignored {
            let state = GitPathState::Ignored;
            perf_log::log_duration(
                "git_path_state_single",
                started.elapsed(),
                &format!(
                    "repo_root={} project_dir={} state={}",
                    repo_root.display(),
                    project_dir.display(),
                    state.label()
                ),
                0,
            );
            return state;
        }
    }
    let status_output = Command::new("git")
        .args([
            "status",
            "--porcelain=v1",
            "--ignored=matching",
            "--untracked-files=all",
            "--",
            &relative_path,
        ])
        .current_dir(&repo_root)
        .output();
    let Ok(status_output) = status_output else {
        return GitPathState::Clean;
    };
    let stdout = String::from_utf8_lossy(&status_output.stdout);
    let mut has_modified = false;
    let mut has_untracked = false;

    for line in stdout.lines().filter(|line| line.len() >= 3) {
        let status_code = &line[..2];
        match status_code {
            "!!" => {},
            "??" => has_untracked = true,
            _ => has_modified = true,
        }
    }

    let state = if has_modified {
        GitPathState::Modified
    } else if has_untracked {
        GitPathState::Untracked
    } else {
        GitPathState::Clean
    };
    perf_log::log_duration(
        "git_path_state_single",
        started.elapsed(),
        &format!(
            "repo_root={} project_dir={} state={}",
            repo_root.display(),
            project_dir.display(),
            state.label()
        ),
        0,
    );
    state
}

pub fn git_repo_root(project_dir: &Path) -> Option<PathBuf> {
    project_dir
        .ancestors()
        .find(|dir| {
            let git_path = dir.join(".git");
            git_path.is_dir() || git_path.is_file()
        })
        .map(Path::to_path_buf)
}

pub fn detect_git_path_states_batch(projects: &[ProjectPathEntry]) -> GitPathStatesByProject {
    let started = std::time::Instant::now();
    let (mut states, repos) = partition_projects_by_repo(projects);

    let repo_count = repos.len();
    for (repo_root, entries) in repos {
        states.extend(detect_repo_git_path_states(&repo_root, &entries));
    }

    perf_log::log_duration(
        "git_path_states_batch",
        started.elapsed(),
        &format!("repos={} rows={}", repo_count, projects.len()),
        0,
    );
    states
}

fn partition_projects_by_repo(
    projects: &[ProjectPathEntry],
) -> (GitPathStatesByProject, ProjectsByRepoRoot) {
    let mut states = HashMap::new();
    let mut repos: ProjectsByRepoRoot = HashMap::new();

    for (path, abs_path) in projects {
        let abs_path = PathBuf::from(abs_path);
        if let Some(repo_root) = git_repo_root(&abs_path) {
            repos
                .entry(repo_root)
                .or_default()
                .push((path.clone(), abs_path.to_string_lossy().to_string()));
        } else {
            states.insert(path.clone(), GitPathState::OutsideRepo);
        }
    }

    (states, repos)
}

fn detect_repo_git_path_states(
    repo_root: &Path,
    entries: &[ProjectPathEntry],
) -> GitPathStatesByProject {
    let prefixes: Vec<ProjectPathEntry> = entries
        .iter()
        .map(|(path, abs_path)| {
            (
                path.clone(),
                normalize_git_relative_path(&relative_git_path(repo_root, Path::new(abs_path))),
            )
        })
        .collect();
    let mut repo_states: GitPathStatesByProject = prefixes
        .iter()
        .map(|(path, _)| (path.clone(), GitPathState::Clean))
        .collect();

    let status_started = std::time::Instant::now();
    let status_output = Command::new("git")
        .args(["status", "--porcelain=v1", "--untracked-files=normal"])
        .current_dir(repo_root)
        .output();
    let status_elapsed_ms = status_started.elapsed().as_millis();
    apply_repo_status_output(&mut repo_states, &prefixes, status_output);

    let ignored_elapsed_ms = update_ignored_repo_states(repo_root, &prefixes, &mut repo_states);

    perf_log::log_event(&format!(
        "git_path_states_repo repo_root={} rows={} status_ms={} ignored_ms={ignored_elapsed_ms}",
        repo_root.display(),
        prefixes.len(),
        status_elapsed_ms,
    ));

    repo_states
}

fn apply_repo_status_output(
    repo_states: &mut GitPathStatesByProject,
    prefixes: &[ProjectPathEntry],
    status_output: io::Result<std::process::Output>,
) {
    let Ok(output) = status_output else {
        return;
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines().filter(|line| line.len() >= 3) {
        let state = if &line[..2] == "??" {
            GitPathState::Untracked
        } else {
            GitPathState::Modified
        };
        let changed_path = normalize_git_relative_path(parse_status_path(line));
        if changed_path.is_empty() {
            continue;
        }
        for (path, prefix) in prefixes {
            if path_contains_git_entry(prefix, &changed_path) {
                apply_git_path_state(
                    repo_states
                        .entry(path.clone())
                        .or_insert(GitPathState::Clean),
                    state,
                );
            }
        }
    }
}

fn update_ignored_repo_states(
    repo_root: &Path,
    prefixes: &[ProjectPathEntry],
    repo_states: &mut GitPathStatesByProject,
) -> u128 {
    let remaining_clean: HashSet<String> = repo_states
        .iter()
        .filter(|(_, state)| matches!(state, GitPathState::Clean))
        .map(|(path, _)| path.clone())
        .collect();
    if remaining_clean.is_empty() {
        return 0;
    }

    let ignored_started = std::time::Instant::now();
    let ignored_output = Command::new("git")
        .args([
            "ls-files",
            "--others",
            "-i",
            "--exclude-standard",
            "--directory",
        ])
        .current_dir(repo_root)
        .output();
    let ignored_elapsed_ms = ignored_started.elapsed().as_millis();
    let Ok(output) = ignored_output else {
        return ignored_elapsed_ms;
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    for ignored in stdout.lines() {
        let ignored = normalize_git_relative_path(ignored);
        if ignored.is_empty() {
            continue;
        }
        for (path, prefix) in prefixes {
            if !remaining_clean.contains(path) {
                continue;
            }
            if git_entry_contains_path(&ignored, prefix) {
                repo_states.insert(path.clone(), GitPathState::Ignored);
            }
        }
    }

    ignored_elapsed_ms
}

fn relative_git_path(repo_root: &Path, project_dir: &Path) -> String {
    project_dir.strip_prefix(repo_root).ok().map_or_else(
        || ".".to_string(),
        |path| {
            let normalized = path
                .components()
                .filter_map(|component| match component {
                    std::path::Component::Normal(segment) => {
                        Some(segment.to_string_lossy().to_string())
                    },
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("/");
            if normalized.is_empty() {
                ".".to_string()
            } else {
                normalized
            }
        },
    )
}

fn parse_status_path(line: &str) -> &str {
    let path = &line[3..];
    path.rsplit_once(" -> ").map_or(path, |(_, after)| after)
}

fn normalize_git_relative_path(path: &str) -> String {
    let normalized = path.trim().trim_matches('"').trim_end_matches('/');
    if normalized == "." {
        String::new()
    } else {
        normalized.to_string()
    }
}

fn path_contains_git_entry(prefix: &str, entry: &str) -> bool {
    prefix.is_empty() || entry == prefix || entry.starts_with(&format!("{prefix}/"))
}

fn git_entry_contains_path(entry: &str, path: &str) -> bool {
    entry == path || path.starts_with(&format!("{entry}/"))
}

const fn apply_git_path_state(current: &mut GitPathState, candidate: GitPathState) {
    *current = match (*current, candidate) {
        (_, GitPathState::Modified) => GitPathState::Modified,
        (GitPathState::Clean | GitPathState::Ignored, GitPathState::Untracked) => {
            GitPathState::Untracked
        },
        (GitPathState::Clean, GitPathState::Ignored) => GitPathState::Ignored,
        (state, _) => state,
    };
}

/// Parse `git rev-list --left-right --count` output into `(ahead, behind)`.
fn parse_ahead_behind(
    project_dir: &Path,
    revspec: &str,
    op_suffix: &str,
) -> Option<(usize, usize)> {
    git_output_logged(
        project_dir,
        &format!("rev_list_{op_suffix}"),
        ["rev-list", "--left-right", "--count", revspec],
    )
    .ok()
    .and_then(|o| {
        let s = String::from_utf8_lossy(&o.stdout);
        let mut parts = s.trim().split('\t');
        let ahead = parts.next()?.parse::<usize>().ok()?;
        let behind = parts.next()?.parse::<usize>().ok()?;
        Some((ahead, behind))
    })
}

/// Extract the owner and HTTPS URL from a git remote URL.
///
/// Handles:
/// - `https://github.com/owner/repo.git`
/// - `git@github.com:owner/repo.git`
fn parse_remote_url(raw: &str) -> (Option<String>, Option<String>) {
    // SSH: git@github.com:owner/repo.git
    if let Some(after_at) = raw.strip_prefix("git@")
        && let Some((host, path)) = after_at.split_once(':')
    {
        let path = path.strip_suffix(".git").unwrap_or(path);
        let owner = path.split('/').next().map(|s| (*s).to_string());
        let url = format!("https://{host}/{path}");
        return (owner, Some(url));
    }

    // HTTPS: https://github.com/owner/repo.git
    if raw.starts_with("https://") || raw.starts_with("http://") {
        let clean = raw.strip_suffix(".git").unwrap_or(raw);
        // Extract owner from path: https://host/owner/repo
        let owner = clean.split('/').nth(3).map(|s| (*s).to_string());
        return (owner, Some((*clean).to_string()));
    }

    (None, None)
}

/// Whether a project has a `[workspace]` section in `Cargo.toml`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "bool", into = "bool")]
pub enum WorkspaceStatus {
    Workspace,
    #[default]
    Standalone,
}

impl From<bool> for WorkspaceStatus {
    fn from(b: bool) -> Self { if b { Self::Workspace } else { Self::Standalone } }
}

impl From<WorkspaceStatus> for bool {
    fn from(val: WorkspaceStatus) -> Self { matches!(val, WorkspaceStatus::Workspace) }
}

/// Whether a project is a Rust project (has `Cargo.toml`) or a non-Rust git repo.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "bool", into = "bool")]
pub enum ProjectLanguage {
    #[default]
    Rust,
    NonRust,
}

impl From<bool> for ProjectLanguage {
    fn from(b: bool) -> Self { if b { Self::Rust } else { Self::NonRust } }
}

impl From<ProjectLanguage> for bool {
    fn from(val: ProjectLanguage) -> Self { matches!(val, ProjectLanguage::Rust) }
}

/// Whether a project path lives inside a git repository.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitRepoPresence {
    InRepo,
    OutsideRepo,
}

impl GitRepoPresence {
    pub const fn is_in_repo(self) -> bool { matches!(self, Self::InRepo) }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProjectType {
    Binary,
    Library,
    ProcMacro,
    BuildScript,
}

impl fmt::Display for ProjectType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Binary => write!(f, "binary"),
            Self::Library => write!(f, "library"),
            Self::ProcMacro => write!(f, "proc-macro"),
            Self::BuildScript => write!(f, "build-script"),
        }
    }
}

/// A group of examples in a subdirectory, or root-level examples (empty category).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExampleGroup {
    /// Subdirectory name, or empty for root-level examples.
    pub category: String,
    pub names:    Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RustProject {
    /// Display path (e.g. `~/rust/bevy`).
    pub path:                      String,
    /// Absolute filesystem path for operations that need to access the project on disk.
    #[serde(skip)]
    pub abs_path:                  String,
    pub name:                      Option<String>,
    pub version:                   Option<String>,
    pub description:               Option<String>,
    pub worktree_name:             Option<String>,
    /// Absolute path of the primary git repo root. Shared by primaries and their
    /// worktrees, used as the identity key for grouping worktrees together.
    #[serde(skip)]
    pub worktree_primary_abs_path: Option<String>,
    /// Whether this project has a `[workspace]` section.
    #[serde(default)]
    pub is_workspace:              WorkspaceStatus,
    pub types:                     Vec<ProjectType>,
    pub examples:                  Vec<ExampleGroup>,
    pub benches:                   Vec<String>,
    pub test_count:                usize,
    /// Whether this project is a Rust project (has `Cargo.toml`).
    #[serde(default)]
    pub is_rust:                   ProjectLanguage,
    #[serde(skip)]
    pub local_dependency_paths:    Vec<String>,
}

impl RustProject {
    /// Total number of examples across all groups.
    pub fn example_count(&self) -> usize { self.examples.iter().map(|g| g.names.len()).sum() }

    /// Language icon for the project list.
    pub const fn lang_icon(&self) -> &'static str {
        match self.is_rust {
            ProjectLanguage::Rust => "🦀",
            ProjectLanguage::NonRust => "  ",
        }
    }

    /// Display name for the project list.
    /// Falls back to the last path component for workspace-only projects.
    pub fn display_name(&self) -> String {
        self.name
            .as_deref()
            .unwrap_or_else(|| self.path.rsplit('/').next().unwrap_or(&self.path))
            .to_string()
    }
}

pub enum ProjectParseError {
    ReadError(io::Error),
    ParseError(toml::de::Error),
}

impl fmt::Display for ProjectParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ReadError(e) => write!(f, "read error: {e}"),
            Self::ParseError(e) => write!(f, "parse error: {e}"),
        }
    }
}

impl RustProject {
    pub fn from_cargo_toml(cargo_toml_path: &Path) -> Result<Self, ProjectParseError> {
        let contents =
            std::fs::read_to_string(cargo_toml_path).map_err(ProjectParseError::ReadError)?;
        let table: Table = contents.parse().map_err(ProjectParseError::ParseError)?;

        let project_dir = cargo_toml_path.parent().unwrap_or(cargo_toml_path);

        let path_str = home_relative_path(project_dir);

        let name = table
            .get("package")
            .and_then(|p| p.get("name"))
            .and_then(|n| n.as_str())
            .map(|s| (*s).to_string());

        let version = table
            .get("package")
            .and_then(|p| p.get("version"))
            .map(|v| {
                v.as_str().map_or_else(
                    || {
                        if v.get("workspace").and_then(Value::as_bool) == Some(true) {
                            "(workspace)".to_string()
                        } else {
                            "-".to_string()
                        }
                    },
                    |s| (*s).to_string(),
                )
            });

        let description = table
            .get("package")
            .and_then(|p| p.get("description"))
            .and_then(|n| n.as_str())
            .map(|s| (*s).to_string());

        // A `.git` file (not directory) indicates a git worktree
        let worktree_name = detect_worktree_name(project_dir);
        let worktree_primary_abs_path = detect_worktree_primary(project_dir);

        let is_workspace = if table.get("workspace").is_some() {
            WorkspaceStatus::Workspace
        } else {
            WorkspaceStatus::Standalone
        };
        let types = detect_types(&table, project_dir);
        let examples = collect_examples(&table, project_dir);
        let benches = collect_target_names(&table, project_dir, "bench", "benches");
        let test_count = count_targets(&table, project_dir, "test", "tests");
        let local_dependency_paths = collect_local_dependency_paths(&table, project_dir);

        let abs_path = project_dir.display().to_string();

        Ok(Self {
            path: path_str,
            abs_path,
            name,
            version,
            description,
            worktree_name,
            worktree_primary_abs_path,
            is_workspace,
            types,
            examples,
            benches,
            test_count,
            is_rust: ProjectLanguage::Rust,
            local_dependency_paths,
        })
    }

    /// Create a project entry for a non-Rust git repository (no `Cargo.toml`).
    pub fn from_git_dir(project_dir: &Path) -> Self {
        let name = project_dir
            .file_name()
            .map(|n| n.to_string_lossy().to_string());
        let path = home_relative_path(project_dir);
        let abs_path = project_dir.display().to_string();
        let worktree_name = detect_worktree_name(project_dir);
        let worktree_primary_abs_path = detect_worktree_primary(project_dir);

        Self {
            path,
            abs_path,
            name,
            version: None,
            description: None,
            worktree_name,
            worktree_primary_abs_path,
            is_workspace: WorkspaceStatus::Standalone,
            types: Vec::new(),
            examples: Vec::new(),
            benches: Vec::new(),
            test_count: 0,
            is_rust: ProjectLanguage::NonRust,
            local_dependency_paths: Vec::new(),
        }
    }

    pub const fn is_workspace(&self) -> bool {
        matches!(self.is_workspace, WorkspaceStatus::Workspace)
    }
}

fn collect_local_dependency_paths(table: &Table, project_dir: &Path) -> Vec<String> {
    let mut paths = Vec::new();
    collect_dependency_paths_from_table(table.get("dependencies"), project_dir, &mut paths);
    collect_dependency_paths_from_table(table.get("dev-dependencies"), project_dir, &mut paths);
    collect_dependency_paths_from_table(table.get("build-dependencies"), project_dir, &mut paths);
    collect_target_dependency_paths(table, project_dir, &mut paths);
    collect_workspace_dependency_paths(table, project_dir, &mut paths);
    collect_patch_dependency_paths(table, project_dir, &mut paths);
    paths.sort();
    paths.dedup();
    paths
}

fn collect_dependency_paths_from_table(
    value: Option<&Value>,
    project_dir: &Path,
    paths: &mut Vec<String>,
) {
    let Some(table) = value.and_then(Value::as_table) else {
        return;
    };
    for dependency in table.values() {
        if let Some(path) = dependency
            .as_table()
            .and_then(|dep_table| dep_table.get("path"))
            .and_then(Value::as_str)
            && let Some(normalized) = normalize_local_dependency_path(project_dir, path)
        {
            paths.push(normalized);
        }
    }
}

fn collect_target_dependency_paths(table: &Table, project_dir: &Path, paths: &mut Vec<String>) {
    let Some(targets) = table.get("target").and_then(Value::as_table) else {
        return;
    };
    for target in targets.values().filter_map(Value::as_table) {
        collect_dependency_paths_from_table(target.get("dependencies"), project_dir, paths);
        collect_dependency_paths_from_table(target.get("dev-dependencies"), project_dir, paths);
        collect_dependency_paths_from_table(target.get("build-dependencies"), project_dir, paths);
    }
}

fn collect_workspace_dependency_paths(table: &Table, project_dir: &Path, paths: &mut Vec<String>) {
    let Some(workspace) = table.get("workspace").and_then(Value::as_table) else {
        return;
    };
    collect_dependency_paths_from_table(workspace.get("dependencies"), project_dir, paths);
}

fn collect_patch_dependency_paths(table: &Table, project_dir: &Path, paths: &mut Vec<String>) {
    let Some(patch) = table.get("patch").and_then(Value::as_table) else {
        return;
    };
    for registry in patch.values().filter_map(Value::as_table) {
        collect_dependency_paths_from_table(
            Some(&Value::Table(registry.clone())),
            project_dir,
            paths,
        );
    }
}

fn normalize_local_dependency_path(project_dir: &Path, dependency_path: &str) -> Option<String> {
    let joined = project_dir.join(dependency_path);
    let dependency_dir = if joined.file_name().is_some_and(|name| name == "Cargo.toml") {
        joined.parent().map(Path::to_path_buf)?
    } else {
        joined
    };
    Some(home_relative_path(&normalize_path(&dependency_dir)))
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {},
            std::path::Component::ParentDir => {
                normalized.pop();
            },
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

fn detect_types(table: &Table, project_dir: &Path) -> Vec<ProjectType> {
    let mut types = Vec::new();

    let is_proc_macro = table
        .get("lib")
        .and_then(|lib| lib.get("proc-macro"))
        .and_then(Value::as_bool)
        == Some(true);

    if is_proc_macro {
        types.push(ProjectType::ProcMacro);
    } else {
        let has_lib_section = table.get("lib").is_some();
        let has_lib_rs = project_dir.join("src/lib.rs").exists();
        if has_lib_section || has_lib_rs {
            types.push(ProjectType::Library);
        }
    }

    let has_bin_section = table.get("bin").is_some();
    let has_main_rs = project_dir.join("src/main.rs").exists();
    if has_bin_section || has_main_rs {
        types.push(ProjectType::Binary);
    }

    if project_dir.join("build.rs").exists() {
        types.push(ProjectType::BuildScript);
    }

    types
}

/// Collect examples grouped by category. Prefers `[[example]]` declarations, falls back to
/// file discovery.
fn collect_examples(table: &Table, project_dir: &Path) -> Vec<ExampleGroup> {
    // Collect from `[[example]]` entries in `Cargo.toml`
    if let Some(arr) = table.get("example").and_then(|v| v.as_array())
        && !arr.is_empty()
    {
        let mut groups: HashMap<String, Vec<String>> = HashMap::new();
        for entry in arr {
            let name = entry
                .get("name")
                .and_then(|n| n.as_str())
                .unwrap_or_default()
                .to_string();
            if name.is_empty() {
                continue;
            }
            // Derive category from path: "examples/2d/foo.rs" -> "2d"
            let category = entry
                .get("path")
                .and_then(|p| p.as_str())
                .and_then(|p| {
                    let parts: Vec<&str> = p.split('/').collect();
                    // "examples/category/file.rs" -> category
                    if parts.len() >= 3 {
                        Some(parts[1].to_string())
                    } else {
                        None
                    }
                })
                .unwrap_or_default();
            groups.entry(category).or_default().push(name);
        }
        return build_sorted_groups(groups);
    }

    // Auto-discover from examples/ directory
    let examples_dir = project_dir.join("examples");
    if !examples_dir.is_dir() {
        return Vec::new();
    }

    discover_examples_grouped(&examples_dir)
}

fn build_sorted_groups(
    mut groups: std::collections::HashMap<String, Vec<String>>,
) -> Vec<ExampleGroup> {
    let mut result: Vec<ExampleGroup> = groups
        .drain()
        .map(|(category, mut names)| {
            names.sort();
            ExampleGroup { category, names }
        })
        .collect();
    // Root-level first, then alphabetically by category
    result.sort_by(|a, b| {
        let a_root = a.category.is_empty();
        let b_root = b.category.is_empty();
        match (a_root, b_root) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.category.cmp(&b.category),
        }
    });
    result
}

/// Auto-discover examples from a directory, grouping by subdirectory.
fn discover_examples_grouped(examples_dir: &Path) -> Vec<ExampleGroup> {
    let Ok(entries) = std::fs::read_dir(examples_dir) else {
        return Vec::new();
    };

    let mut groups: HashMap<String, Vec<String>> = HashMap::new();

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() && path.extension().is_some_and(|e| e == "rs") {
            if let Some(stem) = path.file_stem() {
                groups
                    .entry(String::new())
                    .or_default()
                    .push(stem.to_string_lossy().to_string());
            }
        } else if path.is_dir() {
            let dir_name = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            // Collect `.rs` files and `main.rs` subdirs within this category
            if let Ok(sub_entries) = std::fs::read_dir(&path) {
                for sub in sub_entries.flatten() {
                    let sub_path = sub.path();
                    if sub_path.is_file() && sub_path.extension().is_some_and(|e| e == "rs") {
                        if let Some(stem) = sub_path.file_stem() {
                            groups
                                .entry(dir_name.clone())
                                .or_default()
                                .push(stem.to_string_lossy().to_string());
                        }
                    } else if sub_path.is_dir()
                        && sub_path.join("main.rs").exists()
                        && let Some(name) = sub_path.file_name()
                    {
                        groups
                            .entry(dir_name.clone())
                            .or_default()
                            .push(name.to_string_lossy().to_string());
                    }
                }
            }
        }
    }

    build_sorted_groups(groups)
}

/// Collect target names (e.g. benches). Prefers `[[toml_key]]` declarations, falls back to
/// file discovery in `dir_name/`.
fn collect_target_names(
    table: &Table,
    project_dir: &Path,
    toml_key: &str,
    dir_name: &str,
) -> Vec<String> {
    if let Some(arr) = table.get(toml_key).and_then(|v| v.as_array())
        && !arr.is_empty()
    {
        let mut names: Vec<String> = arr
            .iter()
            .filter_map(|entry| {
                entry
                    .get("name")
                    .and_then(|n| n.as_str())
                    .map(std::string::ToString::to_string)
            })
            .collect();
        names.sort();
        return names;
    }

    let dir = project_dir.join(dir_name);
    if !dir.is_dir() {
        return Vec::new();
    }

    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };

    let mut names = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() && path.extension().is_some_and(|e| e == "rs") {
            if let Some(stem) = path.file_stem() {
                names.push(stem.to_string_lossy().to_string());
            }
        } else if path.is_dir()
            && path.join("main.rs").exists()
            && let Some(name) = path.file_name()
        {
            names.push(name.to_string_lossy().to_string());
        }
    }
    names.sort();
    names
}

fn count_targets(table: &Table, project_dir: &Path, toml_key: &str, dir_name: &str) -> usize {
    let declared = table
        .get(toml_key)
        .and_then(|v| v.as_array())
        .map_or(0, Vec::len);

    if declared > 0 {
        return declared;
    }

    let dir = project_dir.join(dir_name);
    if !dir.is_dir() {
        return 0;
    }

    count_rs_files_recursive(&dir)
}

/// Detect if a project directory is inside a git worktree.
/// Walks up the directory tree looking for a `.git` file (not directory).
/// Returns the worktree root directory name as the label.
/// Returns a `~/`-prefixed path if under the home directory, otherwise the absolute path.
/// Returns a `~/`-prefixed path if under the home directory, otherwise the absolute path.
pub fn home_relative_path(path: &Path) -> String {
    if let Some(home) = dirs::home_dir()
        && let Ok(rel) = path.strip_prefix(&home)
    {
        return format!("~/{}", rel.display());
    }
    path.display().to_string()
}

fn detect_worktree_name(project_dir: &Path) -> Option<String> {
    let mut dir = project_dir;
    loop {
        let git_path = dir.join(".git");
        if git_path.is_file() {
            return dir.file_name().map(|n| n.to_string_lossy().to_string());
        }
        if git_path.is_dir() {
            // Found a real .git directory — not a worktree
            return None;
        }
        dir = dir.parent()?;
    }
}

/// Resolve the primary git repo root for a project directory.
///
/// For worktrees (`.git` is a file containing `gitdir: ...`), parse the gitdir
/// path and strip the `.git/worktrees/<name>` suffix to find the primary root.
/// For primary repos (`.git` is a directory), return the canonicalized directory.
fn detect_worktree_primary(project_dir: &Path) -> Option<String> {
    let mut dir = project_dir;
    loop {
        let git_path = dir.join(".git");
        if git_path.is_file() {
            let contents = std::fs::read_to_string(&git_path).ok()?;
            let gitdir_str = contents.strip_prefix("gitdir: ")?.trim();
            let gitdir = if Path::new(gitdir_str).is_absolute() {
                PathBuf::from(gitdir_str)
            } else {
                dir.join(gitdir_str)
            };
            // gitdir is `<primary>/.git/worktrees/<name>` — go up 3 levels
            let canonical = gitdir.canonicalize().ok()?;
            let primary_root = canonical.parent()?.parent()?.parent()?;
            return Some(primary_root.to_string_lossy().to_string());
        }
        if git_path.is_dir() {
            // This IS the primary — canonicalize for consistent comparison
            return dir
                .canonicalize()
                .ok()
                .map(|p| p.to_string_lossy().to_string());
        }
        dir = dir.parent()?;
    }
}

fn count_rs_files_recursive(dir: &Path) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };

    let mut count = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() && path.extension().is_some_and(|e| e == "rs") {
            count += 1;
        } else if path.is_dir() {
            // Subdirectories can contain examples too (e.g., `examples/foo/main.rs`)
            // Count the directory as one example if it has a `main.rs`
            if path.join("main.rs").exists() {
                count += 1;
            }
        }
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_repo_root_finds_ancestor_git_directory() {
        let tmp = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
        let repo_root = tmp.path().join("repo");
        let nested = repo_root.join("crates").join("demo");
        std::fs::create_dir_all(repo_root.join(".git")).unwrap_or_else(|_| std::process::abort());
        std::fs::create_dir_all(&nested).unwrap_or_else(|_| std::process::abort());

        assert_eq!(git_repo_root(&nested).as_deref(), Some(repo_root.as_path()));
    }

    #[test]
    fn git_repo_root_finds_worktree_git_file() {
        let tmp = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
        let repo_root = tmp.path().join("repo");
        let nested = repo_root.join("crates").join("demo");
        std::fs::create_dir_all(&nested).unwrap_or_else(|_| std::process::abort());
        std::fs::write(repo_root.join(".git"), "gitdir: /tmp/fake\n")
            .unwrap_or_else(|_| std::process::abort());

        assert_eq!(git_repo_root(&nested).as_deref(), Some(repo_root.as_path()));
    }
}
