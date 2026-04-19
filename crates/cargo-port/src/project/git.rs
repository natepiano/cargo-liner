use std::io;
use std::path::Path;
use std::process::Command;

use serde::Deserialize;
use serde::Serialize;

use super::paths::AbsolutePath;
use crate::config;
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

/// How a single git remote relates to the repo: a plain clone or the fork
/// origin when an `upstream` remote also exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum RemoteKind {
    Clone,
    Fork,
}

/// Per-remote metadata. A repo may have any number of these (`origin`,
/// `upstream`, and others).
#[derive(Debug, Clone, Serialize)]
pub(crate) struct RemoteInfo {
    pub name:         String,
    pub url:          Option<String>,
    pub owner:        Option<String>,
    pub repo:         Option<String>,
    pub tracked_ref:  Option<String>,
    pub ahead_behind: Option<(usize, usize)>,
    pub kind:         RemoteKind,
}

/// Git metadata for a project: remotes, current branch, and status info.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct GitInfo {
    /// Git path state (clean, modified, untracked, etc.) for this project path.
    pub status:               GitStatus,
    /// The current branch name.
    pub branch:               Option<String>,
    /// ISO 8601 date of the first commit (inception).
    pub first_commit:         Option<String>,
    /// ISO 8601 date of the most recent commit.
    pub last_commit:          Option<String>,
    /// ISO 8601 timestamp of the last `git fetch` against any remote,
    /// derived from the mtime of `FETCH_HEAD` in the common git dir.
    pub last_fetched:         Option<String>,
    /// The repo's default branch name resolved from `origin/HEAD`.
    pub default_branch:       Option<String>,
    /// The local branch name used for `M` comparisons.
    pub local_main_branch:    Option<String>,
    /// Commits ahead and behind the local `{local_main_branch}`.
    pub ahead_behind_local:   Option<(usize, usize)>,
    /// Whether `.github/workflows/` contains any `.yml` or `.yaml` files.
    pub workflows:            WorkflowPresence,
    /// All remotes declared for this repo.
    pub remotes:              Vec<RemoteInfo>,
    /// Index into `remotes` for the remote matching the current branch's
    /// `@{upstream}` (the "primary" remote). `None` when the current branch
    /// has no upstream tracking ref.
    pub primary_remote_index: Option<usize>,
}

impl GitInfo {
    #[cfg(test)]
    #[expect(dead_code, reason = "Stage 0 scaffolding; used in later stage tests")]
    pub(crate) fn for_tests() -> Self {
        Self {
            status:               GitStatus::Clean,
            branch:               None,
            first_commit:         None,
            last_commit:          None,
            last_fetched:         None,
            default_branch:       None,
            local_main_branch:    None,
            ahead_behind_local:   None,
            workflows:            WorkflowPresence::Missing,
            remotes:              Vec::new(),
            primary_remote_index: None,
        }
    }

    /// The remote matching the current branch's `@{upstream}`, if any.
    pub(crate) fn primary_remote(&self) -> Option<&RemoteInfo> {
        self.primary_remote_index.and_then(|i| self.remotes.get(i))
    }

    /// Convenience: the primary remote's URL.
    pub(crate) fn primary_url(&self) -> Option<&str> {
        self.primary_remote().and_then(|r| r.url.as_deref())
    }

    /// Convenience: the primary remote's ahead/behind vs its tracked ref.
    pub(crate) fn primary_ahead_behind(&self) -> Option<(usize, usize)> {
        self.primary_remote().and_then(|r| r.ahead_behind)
    }

    /// Convenience: the primary remote's tracked ref (e.g. `origin/main`).
    pub(crate) fn primary_tracked_ref(&self) -> Option<&str> {
        self.primary_remote().and_then(|r| r.tracked_ref.as_deref())
    }

    /// Repo-level origin classification derived from `remotes`.
    pub(crate) fn origin_kind(&self) -> GitOrigin {
        if self.remotes.is_empty() {
            GitOrigin::Local
        } else if self.remotes.iter().any(|r| r.name == "upstream") {
            GitOrigin::Fork
        } else {
            GitOrigin::Clone
        }
    }
}

impl GitInfo {
    /// Detect git info for a project directory (excludes `first_commit`, which
    /// is handled by `schedule_git_first_commit_refreshes` batched by repo root).
    pub(crate) fn detect_fast(project_dir: &Path) -> Option<Self> {
        let repo_root = git_repo_root(project_dir)?;
        let cfg = config::active_config();

        let branch = detect_current_branch(&repo_root);
        let current_upstream = detect_upstream_branch(&repo_root);
        let default_branch = detect_default_branch(&repo_root);
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

        let remote_names = list_remote_names(&repo_root);
        let has_upstream = remote_names.iter().any(|n| n == "upstream");
        let remotes: Vec<RemoteInfo> = remote_names
            .iter()
            .map(|name| {
                build_remote_info(
                    &repo_root,
                    name,
                    has_upstream,
                    current_upstream.as_deref(),
                    default_branch.as_deref(),
                    branch.as_deref(),
                    &cfg,
                )
            })
            .collect();

        let primary_remote_index = current_upstream.as_deref().and_then(|us| {
            remotes
                .iter()
                .position(|r| r.tracked_ref.as_deref() == Some(us))
        });

        let last_commit =
            git_output_logged(&repo_root, "log_last_commit", ["log", "-1", "--format=%aI"])
                .ok()
                .and_then(|o| {
                    let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
                    if s.is_empty() { None } else { Some(s) }
                });

        let last_fetched = detect_last_fetched(&repo_root);

        let git_status = detect_git_status_with_root(project_dir, &repo_root);

        Some(Self {
            status: git_status,
            branch,
            first_commit: None,
            last_commit,
            last_fetched,
            default_branch,
            local_main_branch,
            ahead_behind_local,
            workflows: detect_workflow_presence(&repo_root),
            remotes,
            primary_remote_index,
        })
    }
}

fn detect_current_branch(repo_root: &Path) -> Option<String> {
    git_output_logged(
        repo_root,
        "rev_parse_head",
        ["rev-parse", "--abbrev-ref", "HEAD"],
    )
    .ok()
    .and_then(|o| {
        let b = String::from_utf8_lossy(&o.stdout).trim().to_string();
        if b.is_empty() { None } else { Some(b) }
    })
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

/// Resolve the repo's default branch from `origin/HEAD` (e.g. `main`).
fn detect_default_branch(repo_root: &Path) -> Option<String> {
    git_output_logged(
        repo_root,
        "symbolic_ref_origin_head",
        ["symbolic-ref", "refs/remotes/origin/HEAD", "--short"],
    )
    .ok()
    .and_then(|o| {
        let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
        s.strip_prefix("origin/")
            .filter(|b| !b.is_empty())
            .map(str::to_string)
    })
}

fn list_remote_names(repo_root: &Path) -> Vec<String> {
    git_output_logged(repo_root, "remote", ["remote"])
        .ok()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(String::from)
                .collect()
        })
        .unwrap_or_default()
}

fn build_remote_info(
    repo_root: &Path,
    name: &str,
    has_upstream: bool,
    current_upstream: Option<&str>,
    default_branch: Option<&str>,
    current_branch: Option<&str>,
    cfg: &config::CargoPortConfig,
) -> RemoteInfo {
    let (owner, url, repo) = remote_url_info(repo_root, name);
    let tracked_ref = resolve_tracked_ref(
        repo_root,
        name,
        current_upstream,
        default_branch,
        current_branch,
        cfg,
    );
    let ahead_behind = tracked_ref.as_deref().and_then(|r| {
        parse_ahead_behind(
            repo_root,
            &format!("HEAD...{r}"),
            &format!("tracked_{name}"),
        )
    });
    let kind = if name == "origin" && has_upstream {
        RemoteKind::Fork
    } else {
        RemoteKind::Clone
    };
    RemoteInfo {
        name: name.to_string(),
        url,
        owner,
        repo,
        tracked_ref,
        ahead_behind,
        kind,
    }
}

fn remote_url_info(
    repo_root: &Path,
    name: &str,
) -> (Option<String>, Option<String>, Option<String>) {
    git_output_logged(
        repo_root,
        &format!("remote_get_url_{name}"),
        ["remote", "get-url", name],
    )
    .ok()
    .map_or((None, None, None), |out| {
        let raw = String::from_utf8_lossy(&out.stdout).trim().to_string();
        parse_remote_url(&raw)
    })
}

/// Resolve the tracked ref for a remote with a fallback chain.
///
/// Tries, in order:
/// 1. The current branch's `@{upstream}` if it belongs to this remote.
/// 2. `symbolic-ref refs/remotes/<remote>/HEAD`.
/// 3. `<remote>/<default_branch>` (from `origin/HEAD`) if the ref exists.
/// 4. `<remote>/<current_branch>` if the ref exists.
/// 5. `<remote>/<cfg.tui.main_branch>` and each `other_primary_branches` entry if the ref exists.
fn resolve_tracked_ref(
    repo_root: &Path,
    remote_name: &str,
    current_upstream: Option<&str>,
    default_branch: Option<&str>,
    current_branch: Option<&str>,
    cfg: &config::CargoPortConfig,
) -> Option<String> {
    let prefix = format!("{remote_name}/");
    if let Some(us) = current_upstream
        && us.starts_with(&prefix)
    {
        return Some(us.to_string());
    }
    if let Ok(out) = git_output_logged(
        repo_root,
        &format!("symbolic_ref_{remote_name}_head"),
        [
            "symbolic-ref",
            &format!("refs/remotes/{remote_name}/HEAD"),
            "--short",
        ],
    ) {
        let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if s.starts_with(&prefix) {
            return Some(s);
        }
    }
    if let Some(db) = default_branch
        && remote_ref_exists(repo_root, remote_name, db)
    {
        return Some(format!("{remote_name}/{db}"));
    }
    if let Some(cb) = current_branch
        && remote_ref_exists(repo_root, remote_name, cb)
    {
        return Some(format!("{remote_name}/{cb}"));
    }
    std::iter::once(cfg.tui.main_branch.as_str())
        .chain(cfg.tui.other_primary_branches.iter().map(String::as_str))
        .find(|b| remote_ref_exists(repo_root, remote_name, b))
        .map(|b| format!("{remote_name}/{b}"))
}

fn remote_ref_exists(repo_root: &Path, remote_name: &str, branch: &str) -> bool {
    git_output_logged(
        repo_root,
        &format!("show_ref_{remote_name}"),
        [
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/remotes/{remote_name}/{branch}"),
        ],
    )
    .is_ok()
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

/// Resolve the current branch for a worktree at `worktree_dir`.
/// Returns `None` for detached HEAD or read failures.
pub(crate) fn detect_worktree_branch(worktree_dir: &Path) -> Option<String> {
    git_output_logged(
        worktree_dir,
        "worktree_branch",
        ["rev-parse", "--abbrev-ref", "HEAD"],
    )
    .ok()
    .and_then(|o| {
        let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
        if s.is_empty() || s == "HEAD" {
            None
        } else {
            Some(s)
        }
    })
}

/// Ahead/behind of `worktree_dir`'s HEAD vs `primary_dir`'s HEAD. The
/// primary HEAD is resolved to a commit SHA so refs resolve cleanly across
/// the worktree's ref namespace.
pub(crate) fn worktree_ahead_behind_primary(
    worktree_dir: &Path,
    primary_dir: &Path,
) -> Option<(usize, usize)> {
    let primary_sha = git_output_logged(primary_dir, "worktree_primary_sha", ["rev-parse", "HEAD"])
        .ok()
        .and_then(|o| {
            let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
            if s.is_empty() { None } else { Some(s) }
        })?;
    parse_ahead_behind(
        worktree_dir,
        &format!("HEAD...{primary_sha}"),
        "worktree_vs_primary",
    )
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum GitStatus {
    Clean,
    Modified,
    Untracked,
    Ignored,
}

impl GitStatus {
    pub(crate) const fn label(self) -> &'static str {
        match self {
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
            Self::Ignored => "",
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

/// Wrapper for `GitInfo` that distinguishes "not yet detected" from
/// "detected with full metadata."
#[derive(Clone, Debug, Default)]
pub(crate) enum LocalGitState {
    /// Not yet detected (during startup/scan).
    #[default]
    Pending,
    /// Full git metadata detected for this project.
    Detected(Box<GitInfo>),
}

impl LocalGitState {
    pub(crate) fn info(&self) -> Option<&GitInfo> {
        match self {
            Self::Detected(info) => Some(info),
            Self::Pending => None,
        }
    }

    pub(crate) fn info_mut(&mut self) -> Option<&mut GitInfo> {
        match self {
            Self::Detected(info) => Some(info),
            Self::Pending => None,
        }
    }
}

/// Detect git path state when the repo root is already known, avoiding a
/// redundant `git_repo_root()` call.
fn detect_git_status_with_root(project_dir: &Path, repo_root: &Path) -> GitStatus {
    let started = std::time::Instant::now();
    let relative_path = relative_git_path(repo_root, project_dir);
    if relative_path != "." {
        let ignored = Command::new("git")
            .args(["check-ignore", "-q", "--", &relative_path])
            .current_dir(repo_root)
            .status()
            .ok()
            .is_some_and(|status| status.success());
        if ignored {
            let state = GitStatus::Ignored;
            tracing::info!(
                elapsed_ms = crate::perf_log::ms(started.elapsed().as_millis()),
                repo_root = %repo_root.display(),
                project_dir = %project_dir.display(),
                state = %state.label(),
                "git_status_single"
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
        .current_dir(repo_root)
        .output();
    let Ok(status_output) = status_output else {
        return GitStatus::Clean;
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
        GitStatus::Modified
    } else if has_untracked {
        GitStatus::Untracked
    } else {
        GitStatus::Clean
    };
    tracing::info!(
        elapsed_ms = crate::perf_log::ms(started.elapsed().as_millis()),
        repo_root = %repo_root.display(),
        project_dir = %project_dir.display(),
        state = %state.label(),
        "git_status_single"
    );
    state
}

pub(crate) fn git_repo_root(project_dir: &Path) -> Option<AbsolutePath> {
    project_dir
        .ancestors()
        .find(|dir| {
            let git_path = dir.join(".git");
            git_path.is_dir() || git_path.is_file()
        })
        .map(AbsolutePath::from)
}

/// Resolve the on-disk git directory for a repo root.
///
/// For normal repos, returns `repo_root/.git`.
/// For worktrees, `.git` is a file containing `gitdir: <path>` — this
/// function reads that file and returns the resolved path.
pub(crate) fn resolve_git_dir(repo_root: &Path) -> Option<AbsolutePath> {
    let git_path = repo_root.join(".git");
    if git_path.is_dir() {
        return Some(git_path.into());
    }
    if git_path.is_file() {
        let contents = std::fs::read_to_string(&git_path).ok()?;
        let target = contents.strip_prefix("gitdir: ")?.trim();
        return Some(AbsolutePath::resolve(target, repo_root));
    }
    None
}

/// Resolve the common git directory for a repo root.
///
/// For normal repos this is the same path as [`resolve_git_dir`]. For linked
/// worktrees, the resolved git dir may contain a `commondir` file pointing back
/// to the shared `<primary>/.git` directory where branch refs are updated.
pub(crate) fn resolve_common_git_dir(repo_root: &Path) -> Option<AbsolutePath> {
    let git_dir = resolve_git_dir(repo_root)?;
    let commondir_path = git_dir.join("commondir");
    if !commondir_path.is_file() {
        return Some(git_dir);
    }

    let contents = std::fs::read_to_string(&commondir_path).ok()?;
    let target = contents.trim();
    Some(AbsolutePath::resolve(target, &git_dir))
}

/// Read `FETCH_HEAD` mtime from the common git dir and render it as UTC ISO
/// 8601. `FETCH_HEAD` is rewritten on every `git fetch` regardless of whether
/// refs changed, so its mtime is the most reliable "last fetched" signal.
fn detect_last_fetched(repo_root: &Path) -> Option<String> {
    let common_dir = resolve_common_git_dir(repo_root)?;
    let fetch_head = common_dir.join("FETCH_HEAD");
    let modified = std::fs::metadata(&fetch_head).ok()?.modified().ok()?;
    system_time_to_iso8601_utc(modified)
}

fn system_time_to_iso8601_utc(t: std::time::SystemTime) -> Option<String> {
    let secs = i64::try_from(
        t.duration_since(std::time::SystemTime::UNIX_EPOCH)
            .ok()?
            .as_secs(),
    )
    .ok()?;
    let days = secs.div_euclid(86_400);
    let time_of_day = secs.rem_euclid(86_400);
    let hour = time_of_day / 3_600;
    let min = (time_of_day % 3_600) / 60;
    let sec = time_of_day % 60;
    let (year, month, day) = civil_from_days(days);
    Some(format!(
        "{year:04}-{month:02}-{day:02}T{hour:02}:{min:02}:{sec:02}Z"
    ))
}

/// Inverse of `days_from_civil`: days since Unix epoch → (year, month, day).
/// Howard Hinnant's algorithm.
#[allow(
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::cast_possible_truncation,
    reason = "Hinnant's algorithm bounces between signed/unsigned; month/day always 1..=12 / 1..=31"
)]
const fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };
    (year, m as u32, d as u32)
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

/// Extract `(owner, url, repo)` from a git remote URL.
///
/// Handles:
/// - `https://github.com/owner/repo.git`
/// - `git@github.com:owner/repo.git`
///
/// SSH forms are canonicalized to HTTPS so downstream prefix-matching against
/// `default_remote_host_url` works uniformly.
fn parse_remote_url(raw: &str) -> (Option<String>, Option<String>, Option<String>) {
    if let Some(after_at) = raw.strip_prefix("git@")
        && let Some((host, path)) = after_at.split_once(':')
    {
        let path = path.strip_suffix(".git").unwrap_or(path);
        let mut parts = path.splitn(2, '/');
        let owner = parts.next().map(String::from);
        let repo = parts.next().map(String::from);
        let url = format!("https://{host}/{path}");
        return (owner, Some(url), repo);
    }

    if raw.starts_with("https://") || raw.starts_with("http://") {
        let clean = raw.strip_suffix(".git").unwrap_or(raw);
        let mut segments = clean.split('/').skip(3);
        let owner = segments.next().map(String::from);
        let repo = segments.next().map(String::from);
        return (owner, Some(clean.to_string()), repo);
    }

    (None, None, None)
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
    let gitdir = AbsolutePath::resolve_no_canonicalize(gitdir_str.trim(), project_dir);
    if gitdir.exists() {
        WorktreeHealth::Normal
    } else {
        WorktreeHealth::Broken
    }
}

/// The git worktree status of a project directory.
///
/// Captures the mutually exclusive ways a project can relate to git:
/// not in a repo at all, inside a primary (unlinked) repo, or inside a
/// linked worktree. `Primary.root` and `Linked.primary` are both the
/// canonical path of the repo where `.git/` (a directory) lives —
/// distinguishing the two ensures we always know whether this project
/// sits on the main checkout or on a linked one.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum WorktreeStatus {
    #[default]
    NotGit,
    Primary {
        root: AbsolutePath,
    },
    Linked {
        primary: AbsolutePath,
    },
}

impl WorktreeStatus {
    pub(crate) const fn is_linked_worktree(&self) -> bool { matches!(self, Self::Linked { .. }) }

    /// Canonical path of the primary repo root (where `.git/` is a
    /// directory). For `NotGit` returns `None`; for both `Primary` and
    /// `Linked` returns the primary repo's root.
    pub(crate) const fn primary_root(&self) -> Option<&AbsolutePath> {
        match self {
            Self::NotGit => None,
            Self::Primary { root } => Some(root),
            Self::Linked { primary } => Some(primary),
        }
    }
}

/// Detect the git worktree status for a project directory by walking up
/// until a `.git` entry is found: file → `Linked`, directory → `Primary`,
/// nothing found → `NotGit`.
pub(super) fn detect_worktree_status(project_dir: &Path) -> WorktreeStatus {
    let mut dir = project_dir;
    loop {
        let git_path = dir.join(".git");
        if git_path.is_file() {
            return linked_status_from_gitfile(&git_path, dir);
        }
        if git_path.is_dir() {
            return dir
                .canonicalize()
                .map_or(WorktreeStatus::NotGit, |canonical| {
                    WorktreeStatus::Primary {
                        root: AbsolutePath::from(canonical),
                    }
                });
        }
        let Some(parent) = dir.parent() else {
            return WorktreeStatus::NotGit;
        };
        dir = parent;
    }
}

fn linked_status_from_gitfile(git_path: &Path, dir: &Path) -> WorktreeStatus {
    let Ok(contents) = std::fs::read_to_string(git_path) else {
        return WorktreeStatus::NotGit;
    };
    let Some(gitdir_str) = contents.strip_prefix("gitdir: ") else {
        return WorktreeStatus::NotGit;
    };
    let gitdir = AbsolutePath::resolve(gitdir_str.trim(), dir);
    // gitdir is `<primary>/.git/worktrees/<name>` — go up 3 levels
    let Some(primary_root) = gitdir
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
    else {
        return WorktreeStatus::NotGit;
    };
    WorktreeStatus::Linked {
        primary: AbsolutePath::from(primary_root.to_path_buf()),
    }
}

use super::info::WorktreeHealth;

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
