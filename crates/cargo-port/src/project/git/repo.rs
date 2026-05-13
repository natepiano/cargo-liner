use std::path::Path;
use std::time::SystemTime;

use serde::Serialize;

use super::checkout::parse_ahead_behind;
use super::command::git_output_logged;
use super::discovery::git_repo_root;
use super::discovery::resolve_common_git_dir;
use crate::config;
use crate::config::CargoPortConfig;

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

/// Whether `.github/workflows/` contains any `.yml` or `.yaml` files.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub enum WorkflowPresence {
    /// At least one workflow YAML file exists.
    Present,
    /// No workflow files found (or no `.github/workflows/` directory).
    #[default]
    Missing,
}

impl WorkflowPresence {
    pub const fn is_present(self) -> bool { matches!(self, Self::Present) }
}

/// How a single git remote relates to the repo: a plain clone or the fork
/// origin when an `upstream` remote also exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum RemoteKind {
    Clone,
    Fork,
}

/// Per-remote metadata. A repo may have any number of these (`origin`,
/// `upstream`, and others).
#[derive(Debug, Clone, Serialize)]
pub struct RemoteInfo {
    pub name:         String,
    pub url:          Option<String>,
    pub owner:        Option<String>,
    pub repo:         Option<String>,
    pub tracked_ref:  Option<String>,
    pub ahead_behind: Option<(usize, usize)>,
    pub kind:         RemoteKind,
}

/// Repo-level metadata: state that is the same across every checkout of
/// the same git repo. Lives on `GitRepo::repo_info` so siblings cannot
/// drift.
#[derive(Debug, Clone, Default, Serialize)]
pub struct RepoInfo {
    /// All remotes declared for this repo.
    pub remotes:           Vec<RemoteInfo>,
    /// Whether `.github/workflows/` contains any `.yml` or `.yaml` files.
    pub workflows:         WorkflowPresence,
    /// ISO 8601 date of the first commit (inception).
    pub first_commit:      Option<String>,
    /// ISO 8601 timestamp of the last `git fetch` against any remote,
    /// derived from the mtime of `FETCH_HEAD` in the common git dir.
    pub last_fetched:      Option<String>,
    /// The repo's default branch name resolved from `origin/HEAD`.
    pub default_branch:    Option<String>,
    /// The local branch name used for `M` comparisons.
    pub local_main_branch: Option<String>,
}

impl RepoInfo {
    /// Repo-level origin classification derived from `remotes`.
    pub fn origin_kind(&self) -> GitOrigin {
        if self.remotes.is_empty() {
            GitOrigin::Local
        } else if self.remotes.iter().any(|r| r.name == "upstream") {
            GitOrigin::Fork
        } else {
            GitOrigin::Clone
        }
    }

    /// Probe per-repo git metadata. Run once per repo (typically on the
    /// primary checkout's path) and shared across every linked
    /// worktree. Excludes `first_commit`, which is handled by
    /// `schedule_git_first_commit_refreshes` batched by repo root.
    pub fn get(probe_path: &Path) -> Option<Self> {
        let repo_root = git_repo_root(probe_path)?;
        let cfg = config::active_config();

        // Branch / upstream / default-branch context is probed here
        // because `build_remote_info` uses it to resolve each remote's
        // `tracked_ref` and compute `ahead_behind`. Siblings reuse this
        // work; the canonical source is the primary checkout's view.
        let branch = get_current_branch(&repo_root);
        let current_upstream = get_upstream_branch(&repo_root);
        let default_branch = get_default_branch(&repo_root);
        let local_main_branch = resolve_local_main_branch(&repo_root);

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

        Some(Self {
            remotes,
            workflows: get_workflow_presence(&repo_root),
            first_commit: None,
            last_fetched: get_last_fetched(&repo_root),
            default_branch,
            local_main_branch,
        })
    }
}

pub(super) fn get_current_branch(repo_root: &Path) -> Option<String> {
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

pub(super) fn get_upstream_branch(project_dir: &Path) -> Option<String> {
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
fn get_default_branch(repo_root: &Path) -> Option<String> {
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
    cfg: &CargoPortConfig,
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
    cfg: &CargoPortConfig,
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

fn get_workflow_presence(repo_root: &Path) -> WorkflowPresence {
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

pub fn get_first_commit(project_dir: &Path) -> Option<String> {
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

/// Read `FETCH_HEAD` mtime from the common git dir and render it as UTC ISO
/// 8601. `FETCH_HEAD` is rewritten on every `git fetch` regardless of whether
/// refs changed, so its mtime is the most reliable "last fetched" signal.
fn get_last_fetched(repo_root: &Path) -> Option<String> {
    let common_dir = resolve_common_git_dir(repo_root)?;
    let fetch_head = common_dir.join("FETCH_HEAD");
    let modified = std::fs::metadata(&fetch_head).ok()?.modified().ok()?;
    system_time_to_iso8601_utc(modified)
}

fn system_time_to_iso8601_utc(t: SystemTime) -> Option<String> {
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
