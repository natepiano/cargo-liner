use std::collections::HashMap;
use std::collections::HashSet;
use std::io;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

use serde::Deserialize;
use serde::Serialize;

use crate::config;
use crate::constants::GIT_CLONE;
use crate::constants::GIT_FORK;
use crate::constants::GIT_LOCAL;
use crate::constants::GIT_STATUS_CLEAN;
use crate::constants::GIT_STATUS_MODIFIED;
use crate::constants::GIT_STATUS_UNTRACKED;

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

/// Whether `.github/workflows/` contains any `.yml` or `.yaml` files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub(crate) enum WorkflowPresence {
    /// At least one workflow YAML file exists.
    Present,
    /// No workflow files found (or no `.github/workflows/` directory).
    Missing,
}

impl WorkflowPresence {
    pub(crate) const fn is_present(self) -> bool { matches!(self, Self::Present) }
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
    /// The upstream tracking ref for the current branch (e.g. `origin/main`).
    pub upstream_branch:     Option<String>,
    /// The repo's default branch name resolved from `origin/HEAD`.
    pub default_branch:      Option<String>,
    /// Commits ahead and behind `origin/{default_branch}`.
    pub ahead_behind_origin: Option<(usize, usize)>,
    /// The local branch name used for `M` comparisons.
    pub local_main_branch:   Option<String>,
    /// Commits ahead and behind the local `{local_main_branch}`.
    pub ahead_behind_local:  Option<(usize, usize)>,
    /// Whether `.github/workflows/` contains any `.yml` or `.yaml` files.
    pub workflows:           WorkflowPresence,
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

        let (owner, url) = git_output_logged(
            &repo_root,
            "remote_get_url_origin",
            ["remote", "get-url", "origin"],
        )
        .ok()
        .map_or((None, None), |url_output| {
            let raw_url = String::from_utf8_lossy(&url_output.stdout)
                .trim()
                .to_string();
            parse_remote_url(&raw_url)
        });

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
        let upstream_branch = detect_upstream_branch(&repo_root);

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

        // Compare HEAD against the remote default branch when it differs from the current branch.
        let not_on_default = default_branch
            .as_deref()
            .filter(|db| branch.as_deref() != Some(*db));
        let ahead_behind_origin = not_on_default.and_then(|db| {
            parse_ahead_behind(&repo_root, &format!("HEAD...origin/{db}"), "default_origin")
        });
        let local_main_branch = resolve_local_main_branch(&repo_root);
        let ahead_behind_local = local_main_branch
            .as_deref()
            .filter(|branch_name| branch.as_deref() != Some(*branch_name))
            .and_then(|branch_name| {
                parse_ahead_behind(
                    &repo_root,
                    &format!("HEAD...{branch_name}"),
                    "configured_local_main",
                )
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
            upstream_branch,
            default_branch,
            ahead_behind_origin,
            local_main_branch,
            ahead_behind_local,
            workflows: detect_workflow_presence(&repo_root),
        })
    }
}

fn detect_upstream_branch(project_dir: &Path) -> Option<String> {
    git_output_logged(
        project_dir,
        "rev_parse_upstream_name",
        [
            "rev-parse",
            "--abbrev-ref",
            "--symbolic-full-name",
            "@{upstream}",
        ],
    )
    .ok()
    .and_then(|o| {
        let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
        if s.is_empty() { None } else { Some(s) }
    })
}

fn resolve_local_main_branch(project_dir: &Path) -> Option<String> {
    let cfg = config::active_config();
    std::iter::once(cfg.tui.main_branch.as_str())
        .chain(cfg.tui.other_primary_branches.iter().map(String::as_str))
        .find(|branch| local_branch_exists(project_dir, branch))
        .map(str::to_string)
}

fn local_branch_exists(project_dir: &Path, branch: &str) -> bool {
    git_output_logged(
        project_dir,
        "show_ref_local_main",
        [
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/heads/{branch}"),
        ],
    )
    .is_ok()
}

fn detect_workflow_presence(repo_root: &Path) -> WorkflowPresence {
    let workflows_dir = repo_root.join(".github").join("workflows");
    let has_yaml = std::fs::read_dir(workflows_dir).is_ok_and(|entries| {
        entries.filter_map(Result::ok).any(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            name.ends_with(".yml") || name.ends_with(".yaml")
        })
    });
    if has_yaml {
        WorkflowPresence::Present
    } else {
        WorkflowPresence::Missing
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
    tracing::info!(
        elapsed_ms = crate::perf_log::ms(started.elapsed().as_millis()),
        repo_root = %repo_root.display(),
        op,
        status,
        "git_info_detect_call"
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

    pub(crate) const fn icon(self) -> &'static str {
        match self {
            Self::Clean => GIT_STATUS_CLEAN,
            Self::Modified => GIT_STATUS_MODIFIED,
            Self::Untracked => GIT_STATUS_UNTRACKED,
            Self::OutsideRepo | Self::Ignored => "",
        }
    }

    pub(crate) fn label_with_icon(self) -> String {
        let icon = self.icon();
        if icon.is_empty() {
            self.label().to_string()
        } else {
            format!("{icon} {}", self.label())
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
            tracing::info!(
                elapsed_ms = crate::perf_log::ms(started.elapsed().as_millis()),
                repo_root = %repo_root.display(),
                project_dir = %project_dir.display(),
                state = %state.label(),
                "git_path_state_single"
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
    tracing::info!(
        elapsed_ms = crate::perf_log::ms(started.elapsed().as_millis()),
        repo_root = %repo_root.display(),
        project_dir = %project_dir.display(),
        state = %state.label(),
        "git_path_state_single"
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

/// Resolve the common git directory for a repo root.
///
/// For normal repos this is the same path as [`resolve_git_dir`]. For linked
/// worktrees, the resolved git dir may contain a `commondir` file pointing back
/// to the shared `<primary>/.git` directory where branch refs are updated.
pub(crate) fn resolve_common_git_dir(repo_root: &Path) -> Option<PathBuf> {
    let git_dir = resolve_git_dir(repo_root)?;
    let commondir_path = git_dir.join("commondir");
    if !commondir_path.is_file() {
        return Some(git_dir);
    }

    let contents = std::fs::read_to_string(&commondir_path).ok()?;
    let target = contents.trim();
    let resolved = if Path::new(target).is_absolute() {
        PathBuf::from(target)
    } else {
        git_dir.join(target)
    };
    Some(resolved.canonicalize().ok().unwrap_or(resolved))
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

    tracing::info!(
        elapsed_ms = crate::perf_log::ms(started.elapsed().as_millis()),
        repos = repo_count,
        rows = projects.len(),
        "git_path_states_batch"
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

    tracing::info!(
        repo_root = %repo_root.display(),
        rows = prefixes.len(),
        status_ms = status_elapsed_ms,
        ignored_ms = ignored_elapsed_ms,
        "git_path_states_repo"
    );

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

/// Check if a project directory is a broken worktree — `.git` is a file whose
/// gitdir target does not exist on disk.
pub(crate) fn detect_worktree_health(project_dir: &Path) -> WorktreeHealth {
    let git_path = project_dir.join(".git");
    if !git_path.is_file() {
        return WorktreeHealth::Normal;
    }
    let Ok(contents) = std::fs::read_to_string(&git_path) else {
        return WorktreeHealth::Broken;
    };
    let Some(gitdir_str) = contents.strip_prefix("gitdir: ") else {
        return WorktreeHealth::Broken;
    };
    let gitdir = if Path::new(gitdir_str.trim()).is_absolute() {
        PathBuf::from(gitdir_str.trim())
    } else {
        project_dir.join(gitdir_str.trim())
    };
    if gitdir.exists() {
        WorktreeHealth::Normal
    } else {
        WorktreeHealth::Broken
    }
}

pub(super) fn detect_worktree_name(project_dir: &Path) -> Option<String> {
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
pub(super) fn detect_worktree_primary(project_dir: &Path) -> Option<String> {
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

// Re-export `WorktreeHealth` here since `detect_worktree_health` returns it
// and it's conceptually a git type.
use super::types::WorktreeHealth;

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
