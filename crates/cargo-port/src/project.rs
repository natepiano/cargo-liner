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
pub(crate) enum GitOrigin {
    /// A local-only repo (no origin remote).
    Local,
    /// A plain git clone (has "origin" remote).
    Clone,
    /// A fork (has an "upstream" remote).
    Fork,
}

impl GitOrigin {
    pub(crate) const fn icon(self) -> &'static str {
        match self {
            Self::Local => GIT_LOCAL,
            Self::Clone => GIT_CLONE,
            Self::Fork => GIT_FORK,
        }
    }

    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Clone => "clone",
            Self::Fork => "fork",
        }
    }
}

/// Git metadata for a project: origin type, owner, repo URL, and current branch.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct GitInfo {
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
    pub(crate) fn detect(project_dir: &Path) -> Option<Self> {
        let repo_root = git_repo_root(project_dir)?;
        let mut info = Self::detect_fast(&repo_root)?;
        info.first_commit = detect_first_commit(&repo_root);
        Some(info)
    }

    /// Detect the subset of git info needed on the startup critical path.
    pub(crate) fn detect_fast(project_dir: &Path) -> Option<Self> {
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

pub(crate) fn detect_first_commit(project_dir: &Path) -> Option<String> {
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
pub(crate) enum GitPathState {
    #[default]
    OutsideRepo,
    Clean,
    Modified,
    Untracked,
    Ignored,
}

impl GitPathState {
    pub(crate) const fn label(self) -> &'static str {
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

pub(crate) fn detect_git_path_state(project_dir: &Path) -> GitPathState {
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

pub(crate) fn git_repo_root(project_dir: &Path) -> Option<PathBuf> {
    project_dir
        .ancestors()
        .find(|dir| {
            let git_path = dir.join(".git");
            git_path.is_dir() || git_path.is_file()
        })
        .map(Path::to_path_buf)
}

/// Resolve the on-disk git directory for a repo root.
///
/// For normal repos, returns `repo_root/.git`.
/// For worktrees, `.git` is a file containing `gitdir: <path>` — this
/// function reads that file and returns the resolved path.
pub(crate) fn resolve_git_dir(repo_root: &Path) -> Option<PathBuf> {
    let git_path = repo_root.join(".git");
    if git_path.is_dir() {
        return Some(git_path);
    }
    if git_path.is_file() {
        let contents = std::fs::read_to_string(&git_path).ok()?;
        let target = contents.strip_prefix("gitdir: ")?.trim();
        let resolved = if Path::new(target).is_absolute() {
            PathBuf::from(target)
        } else {
            repo_root.join(target)
        };
        return Some(resolved.canonicalize().ok().unwrap_or(resolved));
    }
    None
}

pub(crate) fn detect_git_path_states_batch(
    projects: &[ProjectPathEntry],
) -> GitPathStatesByProject {
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

/// Whether a project path lives inside a git repository.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GitRepoPresence {
    InRepo,
    OutsideRepo,
}

impl GitRepoPresence {
    pub(crate) const fn is_in_repo(self) -> bool { matches!(self, Self::InRepo) }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum ProjectType {
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
pub(crate) struct ExampleGroup {
    /// Subdirectory name, or empty for root-level examples.
    pub category: String,
    pub names:    Vec<String>,
}

pub(crate) enum ProjectParseError {
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

/// Result of parsing a `Cargo.toml`: either a workspace or a standalone package.
pub(crate) enum CargoProject {
    Workspace(Project<Workspace>),
    Package(Project<Package>),
}

/// Parse a `Cargo.toml` and return either a workspace or a package project.
pub(crate) fn from_cargo_toml(cargo_toml_path: &Path) -> Result<CargoProject, ProjectParseError> {
    let contents =
        std::fs::read_to_string(cargo_toml_path).map_err(ProjectParseError::ReadError)?;
    let table: Table = contents.parse().map_err(ProjectParseError::ParseError)?;

    let project_dir = cargo_toml_path.parent().unwrap_or(cargo_toml_path);
    let abs_path = project_dir.to_path_buf();

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

    let worktree_name = detect_worktree_name(project_dir);
    let worktree_primary_abs_path = detect_worktree_primary(project_dir).map(PathBuf::from);

    let types = detect_types(&table, project_dir);
    let examples = collect_examples(&table, project_dir);
    let benches = collect_target_names(&table, project_dir, "bench", "benches");
    let test_count = count_targets(&table, project_dir, "test", "tests");

    let cargo = Cargo::new(version, description, types, examples, benches, test_count);

    if table.get("workspace").is_some() {
        Ok(CargoProject::Workspace(Project::<Workspace>::new(
            abs_path,
            name,
            cargo,
            Vec::new(),
            Vec::new(),
            worktree_name,
            worktree_primary_abs_path,
        )))
    } else {
        Ok(CargoProject::Package(Project::<Package>::new(
            abs_path,
            name,
            cargo,
            Vec::new(),
            worktree_name,
            worktree_primary_abs_path,
        )))
    }
}

/// Create a project entry for a non-Rust git repository (no `Cargo.toml`).
pub(crate) fn from_git_dir(project_dir: &Path) -> Project<NonRust> {
    let name = project_dir
        .file_name()
        .map(|n| n.to_string_lossy().to_string());
    let worktree_name = detect_worktree_name(project_dir);
    let worktree_primary_abs_path = detect_worktree_primary(project_dir).map(PathBuf::from);

    Project::<NonRust>::new(
        project_dir.to_path_buf(),
        name,
        worktree_name,
        worktree_primary_abs_path,
    )
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
pub(crate) fn home_relative_path(path: &Path) -> String {
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

// ── New hierarchical data model ────────────────────────────────────────

/// Visibility state for projects and worktree groups.
/// Progression: `Visible -> Deleted -> Dismissed`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Visibility {
    #[default]
    Visible,
    Deleted,
    Dismissed,
}

/// Associated types that drive the structure per project kind.
pub(crate) trait ProjectKind: Clone + 'static {
    type Cargo: Clone;
    type Groups: Clone;
    type Vendored: Clone;
}

#[derive(Clone)]
pub(crate) struct Workspace;

impl ProjectKind for Workspace {
    type Cargo = Cargo;
    type Groups = Vec<MemberGroup>;
    type Vendored = Vec<Project<Package>>;
}

#[derive(Clone)]
pub(crate) struct Package;

impl ProjectKind for Package {
    type Cargo = Cargo;
    type Groups = ();
    type Vendored = Vec<Project<Self>>;
}

#[derive(Clone)]
pub(crate) struct NonRust;

impl ProjectKind for NonRust {
    type Cargo = ();
    type Groups = ();
    type Vendored = ();
}

/// Shared Cargo fields extracted from `Cargo.toml`.
#[derive(Clone, Debug)]
pub(crate) struct Cargo {
    version:     Option<String>,
    description: Option<String>,
    types:       Vec<ProjectType>,
    examples:    Vec<ExampleGroup>,
    benches:     Vec<String>,
    test_count:  usize,
}

impl Cargo {
    pub(crate) const fn new(
        version: Option<String>,
        description: Option<String>,
        types: Vec<ProjectType>,
        examples: Vec<ExampleGroup>,
        benches: Vec<String>,
        test_count: usize,
    ) -> Self {
        Self {
            version,
            description,
            types,
            examples,
            benches,
            test_count,
        }
    }

    pub(crate) fn types(&self) -> &[ProjectType] { &self.types }

    pub(crate) fn examples(&self) -> &[ExampleGroup] { &self.examples }

    pub(crate) fn benches(&self) -> &[String] { &self.benches }

    pub(crate) fn version(&self) -> Option<&str> { self.version.as_deref() }

    pub(crate) fn description(&self) -> Option<&str> { self.description.as_deref() }

    pub(crate) const fn test_count(&self) -> usize { self.test_count }

    pub(crate) fn example_count(&self) -> usize {
        self.examples.iter().map(|g| g.names.len()).sum()
    }

    pub(crate) fn is_binary(&self) -> bool {
        self.types.iter().any(|t| matches!(t, ProjectType::Binary))
    }
}

/// The core project type, parameterized by kind.
/// Private fields with accessors enforce what's available per kind.
pub(crate) struct Project<Kind: ProjectKind> {
    path:                      PathBuf,
    name:                      Option<String>,
    visibility:                Visibility,
    cargo:                     Kind::Cargo,
    groups:                    Kind::Groups,
    vendored:                  Kind::Vendored,
    worktree_name:             Option<String>,
    worktree_primary_abs_path: Option<PathBuf>,
}

impl Clone for Project<Workspace> {
    fn clone(&self) -> Self {
        Self {
            path:                      self.path.clone(),
            name:                      self.name.clone(),
            visibility:                self.visibility,
            cargo:                     self.cargo.clone(),
            groups:                    self.groups.clone(),
            vendored:                  self.vendored.clone(),
            worktree_name:             self.worktree_name.clone(),
            worktree_primary_abs_path: self.worktree_primary_abs_path.clone(),
        }
    }
}

impl Clone for Project<Package> {
    fn clone(&self) -> Self {
        Self {
            path:                      self.path.clone(),
            name:                      self.name.clone(),
            visibility:                self.visibility,
            cargo:                     self.cargo.clone(),
            groups:                    (),
            vendored:                  self.vendored.clone(),
            worktree_name:             self.worktree_name.clone(),
            worktree_primary_abs_path: self.worktree_primary_abs_path.clone(),
        }
    }
}

impl Clone for Project<NonRust> {
    fn clone(&self) -> Self {
        Self {
            path:                      self.path.clone(),
            name:                      self.name.clone(),
            visibility:                self.visibility,
            cargo:                     (),
            groups:                    (),
            vendored:                  (),
            worktree_name:             self.worktree_name.clone(),
            worktree_primary_abs_path: self.worktree_primary_abs_path.clone(),
        }
    }
}

// Shared accessors for all kinds.
impl<Kind: ProjectKind> Project<Kind> {
    pub(crate) fn path(&self) -> &Path { &self.path }

    pub(crate) fn name(&self) -> Option<&str> { self.name.as_deref() }

    pub(crate) const fn visibility(&self) -> Visibility { self.visibility }

    pub(crate) const fn set_visibility(&mut self, v: Visibility) { self.visibility = v; }

    pub(crate) fn worktree_name(&self) -> Option<&str> { self.worktree_name.as_deref() }

    pub(crate) fn worktree_primary_abs_path(&self) -> Option<&Path> {
        self.worktree_primary_abs_path.as_deref()
    }

    /// Display path: `~/`-prefixed for home-relative, otherwise absolute.
    pub(crate) fn display_path(&self) -> String { home_relative_path(&self.path) }

    /// Display name: project name or last path component.
    pub(crate) fn display_name(&self) -> String {
        self.name
            .as_deref()
            .unwrap_or_else(|| {
                self.path
                    .file_name()
                    .map_or("", |n| n.to_str().unwrap_or(""))
            })
            .to_string()
    }
}

// Workspace-specific accessors.
impl Project<Workspace> {
    pub(crate) const fn cargo(&self) -> &Cargo { &self.cargo }

    pub(crate) fn groups(&self) -> &[MemberGroup] { &self.groups }

    pub(crate) const fn groups_mut(&mut self) -> &mut Vec<MemberGroup> { &mut self.groups }

    pub(crate) fn vendored(&self) -> &[Project<Package>] { &self.vendored }

    pub(crate) const fn vendored_mut(&mut self) -> &mut Vec<Project<Package>> { &mut self.vendored }

    pub(crate) fn new(
        path: PathBuf,
        name: Option<String>,
        cargo: Cargo,
        groups: Vec<MemberGroup>,
        vendored: Vec<Project<Package>>,
        worktree_name: Option<String>,
        worktree_primary_abs_path: Option<PathBuf>,
    ) -> Self {
        Self {
            path,
            name,
            visibility: Visibility::default(),
            cargo,
            groups,
            vendored,
            worktree_name,
            worktree_primary_abs_path,
        }
    }

    pub(crate) fn has_members(&self) -> bool { self.groups.iter().any(|g| !g.members().is_empty()) }

    /// Language icon for the project list.
    pub(crate) const fn lang_icon() -> &'static str { "\u{1f980}" }
}

// Package-specific accessors.
impl Project<Package> {
    pub(crate) const fn cargo(&self) -> &Cargo { &self.cargo }

    pub(crate) fn vendored(&self) -> &[Self] { &self.vendored }

    pub(crate) const fn vendored_mut(&mut self) -> &mut Vec<Self> { &mut self.vendored }

    pub(crate) fn new(
        path: PathBuf,
        name: Option<String>,
        cargo: Cargo,
        vendored: Vec<Self>,
        worktree_name: Option<String>,
        worktree_primary_abs_path: Option<PathBuf>,
    ) -> Self {
        Self {
            path,
            name,
            visibility: Visibility::default(),
            cargo,
            groups: (),
            vendored,
            worktree_name,
            worktree_primary_abs_path,
        }
    }

    /// Language icon for the project list.
    pub(crate) const fn lang_icon() -> &'static str { "\u{1f980}" }
}

// NonRust-specific constructor.
impl Project<NonRust> {
    pub(crate) fn new(
        path: PathBuf,
        name: Option<String>,
        worktree_name: Option<String>,
        worktree_primary_abs_path: Option<PathBuf>,
    ) -> Self {
        Self {
            path,
            name,
            visibility: Visibility::default(),
            cargo: (),
            groups: (),
            vendored: (),
            worktree_name,
            worktree_primary_abs_path,
        }
    }

    /// Language icon for the project list.
    pub(crate) const fn lang_icon() -> &'static str { "  " }
}

/// A generic worktree group: primary + linked checkouts.
pub(crate) struct WorktreeGroup<Kind: ProjectKind> {
    primary:    Project<Kind>,
    linked:     Vec<Project<Kind>>,
    visibility: Visibility,
}

impl<Kind: ProjectKind> WorktreeGroup<Kind> {
    pub(crate) fn new(primary: Project<Kind>, linked: Vec<Project<Kind>>) -> Self {
        Self {
            primary,
            linked,
            visibility: Visibility::default(),
        }
    }

    pub(crate) const fn primary(&self) -> &Project<Kind> { &self.primary }

    pub(crate) const fn primary_mut(&mut self) -> &mut Project<Kind> { &mut self.primary }

    pub(crate) fn linked(&self) -> &[Project<Kind>] { &self.linked }

    pub(crate) const fn linked_mut(&mut self) -> &mut Vec<Project<Kind>> { &mut self.linked }

    pub(crate) const fn visibility(&self) -> Visibility { self.visibility }
}

impl Clone for WorktreeGroup<Workspace> {
    fn clone(&self) -> Self {
        Self {
            primary:    self.primary.clone(),
            linked:     self.linked.clone(),
            visibility: self.visibility,
        }
    }
}

impl Clone for WorktreeGroup<Package> {
    fn clone(&self) -> Self {
        Self {
            primary:    self.primary.clone(),
            linked:     self.linked.clone(),
            visibility: self.visibility,
        }
    }
}

/// The top-level enum for the project list.
pub(crate) enum ProjectListItem {
    Workspace(Project<Workspace>),
    Package(Project<Package>),
    NonRust(Project<NonRust>),
    WorkspaceWorktrees(WorktreeGroup<Workspace>),
    PackageWorktrees(WorktreeGroup<Package>),
}

impl Clone for ProjectListItem {
    fn clone(&self) -> Self {
        match self {
            Self::Workspace(p) => Self::Workspace(p.clone()),
            Self::Package(p) => Self::Package(p.clone()),
            Self::NonRust(p) => Self::NonRust(p.clone()),
            Self::WorkspaceWorktrees(g) => Self::WorkspaceWorktrees(g.clone()),
            Self::PackageWorktrees(g) => Self::PackageWorktrees(g.clone()),
        }
    }
}

impl ProjectListItem {
    pub(crate) const fn visibility(&self) -> Visibility {
        match self {
            Self::Workspace(p) => p.visibility(),
            Self::Package(p) => p.visibility(),
            Self::NonRust(p) => p.visibility(),
            Self::WorkspaceWorktrees(g) => g.visibility(),
            Self::PackageWorktrees(g) => g.visibility(),
        }
    }

    /// Absolute path to the primary project root.
    pub(crate) fn path(&self) -> &Path {
        match self {
            Self::Workspace(p) => p.path(),
            Self::Package(p) => p.path(),
            Self::NonRust(p) => p.path(),
            Self::WorkspaceWorktrees(g) => g.primary().path(),
            Self::PackageWorktrees(g) => g.primary().path(),
        }
    }

    pub(crate) fn name(&self) -> Option<&str> {
        match self {
            Self::Workspace(p) => p.name(),
            Self::Package(p) => p.name(),
            Self::NonRust(p) => p.name(),
            Self::WorkspaceWorktrees(g) => g.primary().name(),
            Self::PackageWorktrees(g) => g.primary().name(),
        }
    }

    pub(crate) fn display_path(&self) -> String {
        match self {
            Self::Workspace(p) => p.display_path(),
            Self::Package(p) => p.display_path(),
            Self::NonRust(p) => p.display_path(),
            Self::WorkspaceWorktrees(g) => g.primary().display_path(),
            Self::PackageWorktrees(g) => g.primary().display_path(),
        }
    }

    pub(crate) fn display_name(&self) -> String {
        match self {
            Self::Workspace(p) => p.display_name(),
            Self::Package(p) => p.display_name(),
            Self::NonRust(p) => p.display_name(),
            Self::WorkspaceWorktrees(g) => g.primary().display_name(),
            Self::PackageWorktrees(g) => g.primary().display_name(),
        }
    }

    /// Whether this item has expandable children.
    pub(crate) fn has_children(&self) -> bool {
        match self {
            Self::Workspace(ws) => {
                ws.groups().iter().any(|g| !g.members().is_empty()) || !ws.vendored().is_empty()
            },
            Self::Package(pkg) => !pkg.vendored().is_empty(),
            Self::NonRust(_) => false,
            Self::WorkspaceWorktrees(g) => !g.linked().is_empty(),
            Self::PackageWorktrees(g) => !g.linked().is_empty(),
        }
    }

    /// Language icon for the project list.
    pub(crate) const fn lang_icon(&self) -> &'static str {
        match self {
            Self::Workspace(_) | Self::WorkspaceWorktrees(_) => Project::<Workspace>::lang_icon(),
            Self::Package(_) | Self::PackageWorktrees(_) => Project::<Package>::lang_icon(),
            Self::NonRust(_) => Project::<NonRust>::lang_icon(),
        }
    }

    /// Whether this is a Rust project (has Cargo.toml).
    pub(crate) const fn is_rust(&self) -> bool {
        matches!(
            self,
            Self::Workspace(_)
                | Self::Package(_)
                | Self::WorkspaceWorktrees(_)
                | Self::PackageWorktrees(_)
        )
    }

    /// Check if any project in the hierarchy matches the absolute path and visibility.
    pub fn has_project_with_visibility_by_path(&self, path: &Path, v: Visibility) -> bool {
        match self {
            Self::Workspace(p) => {
                if p.path() == path && p.visibility() == v {
                    return true;
                }
                p.groups().iter().any(|g| {
                    g.members()
                        .iter()
                        .any(|m| m.path() == path && m.visibility() == v)
                }) || p
                    .vendored()
                    .iter()
                    .any(|vp| vp.path() == path && vp.visibility() == v)
            },
            Self::Package(p) => {
                if p.path() == path && p.visibility() == v {
                    return true;
                }
                p.vendored()
                    .iter()
                    .any(|vp| vp.path() == path && vp.visibility() == v)
            },
            Self::NonRust(p) => p.path() == path && p.visibility() == v,
            Self::WorkspaceWorktrees(g) => {
                (g.primary().path() == path && g.primary().visibility() == v)
                    || g.linked()
                        .iter()
                        .any(|l| l.path() == path && l.visibility() == v)
            },
            Self::PackageWorktrees(g) => {
                (g.primary().path() == path && g.primary().visibility() == v)
                    || g.linked()
                        .iter()
                        .any(|l| l.path() == path && l.visibility() == v)
            },
        }
    }

    /// Set visibility on a project anywhere in the hierarchy by display path.
    pub(crate) fn set_visibility_by_path(&mut self, display_path: &str, v: Visibility) -> bool {
        match self {
            Self::Workspace(p) => {
                if p.display_path() == display_path {
                    p.set_visibility(v);
                    return true;
                }
                for g in p.groups_mut() {
                    for m in g.members_mut() {
                        if m.display_path() == display_path {
                            m.set_visibility(v);
                            return true;
                        }
                    }
                }
                for vp in p.vendored_mut() {
                    if vp.display_path() == display_path {
                        vp.set_visibility(v);
                        return true;
                    }
                }
                false
            },
            Self::Package(p) => {
                if p.display_path() == display_path {
                    p.set_visibility(v);
                    return true;
                }
                for vp in p.vendored_mut() {
                    if vp.display_path() == display_path {
                        vp.set_visibility(v);
                        return true;
                    }
                }
                false
            },
            Self::NonRust(p) => {
                if p.display_path() == display_path {
                    p.set_visibility(v);
                    return true;
                }
                false
            },
            Self::WorkspaceWorktrees(g) => {
                if g.primary().display_path() == display_path {
                    g.primary_mut().set_visibility(v);
                    return true;
                }
                for linked in g.linked_mut() {
                    if linked.display_path() == display_path {
                        linked.set_visibility(v);
                        return true;
                    }
                }
                false
            },
            Self::PackageWorktrees(g) => {
                if g.primary().display_path() == display_path {
                    g.primary_mut().set_visibility(v);
                    return true;
                }
                for linked in g.linked_mut() {
                    if linked.display_path() == display_path {
                        linked.set_visibility(v);
                        return true;
                    }
                }
                false
            },
        }
    }
}

/// Members within a workspace organized into groups.
#[derive(Clone)]
pub(crate) enum MemberGroup {
    Named {
        name:    String,
        members: Vec<Project<Package>>,
    },
    Inline {
        members: Vec<Project<Package>>,
    },
}

impl MemberGroup {
    pub(crate) fn members(&self) -> &[Project<Package>] {
        match self {
            Self::Named { members, .. } | Self::Inline { members } => members,
        }
    }

    pub(crate) const fn members_mut(&mut self) -> &mut Vec<Project<Package>> {
        match self {
            Self::Named { members, .. } | Self::Inline { members } => members,
        }
    }

    pub(crate) fn group_name(&self) -> &str {
        match self {
            Self::Named { name, .. } => name,
            Self::Inline { .. } => "",
        }
    }

    pub(crate) const fn is_named(&self) -> bool { matches!(self, Self::Named { .. }) }
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
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
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

    #[test]
    fn resolve_git_dir_returns_dot_git_for_normal_repo() {
        let tmp = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(repo.join(".git")).unwrap_or_else(|_| std::process::abort());

        assert_eq!(
            resolve_git_dir(&repo).as_deref(),
            Some(repo.join(".git").as_path())
        );
    }

    #[test]
    fn resolve_git_dir_follows_worktree_gitdir_file() {
        let tmp = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
        let main_git = tmp
            .path()
            .join("main")
            .join(".git")
            .join("worktrees")
            .join("wt");
        std::fs::create_dir_all(&main_git).unwrap_or_else(|_| std::process::abort());

        let wt = tmp.path().join("wt");
        std::fs::create_dir_all(&wt).unwrap_or_else(|_| std::process::abort());
        std::fs::write(wt.join(".git"), format!("gitdir: {}\n", main_git.display()))
            .unwrap_or_else(|_| std::process::abort());

        let resolved = resolve_git_dir(&wt).expect("should resolve");
        assert_eq!(resolved.canonicalize().ok(), main_git.canonicalize().ok());
    }

    #[test]
    fn resolve_git_dir_returns_none_without_git() {
        let tmp = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
        assert_eq!(resolve_git_dir(tmp.path()), None);
    }
}
