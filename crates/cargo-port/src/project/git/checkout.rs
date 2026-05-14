use std::path::Component;
use std::path::Path;

use serde::Deserialize;
use serde::Serialize;

use super::command;
use super::discovery;
use super::repo;
use super::repo::RemoteInfo;
use super::repo::RepoInfo;
use crate::constants::GIT_STATUS_CLEAN;
use crate::constants::GIT_STATUS_MODIFIED;
use crate::constants::GIT_STATUS_UNTRACKED;

/// Per-checkout git metadata: state that can legitimately differ between
/// two worktrees of the same repo. Lives inside `ProjectInfo.local_git_state`.
#[derive(Debug, Clone, Serialize)]
pub struct CheckoutInfo {
    /// Git path state (clean, modified, untracked, etc.) for this project path.
    pub status:              GitStatus,
    /// The current branch name.
    pub branch:              Option<String>,
    /// ISO 8601 date of the most recent commit on this branch.
    pub last_commit:         Option<String>,
    /// Commits ahead and behind the local `{local_main_branch}`.
    pub ahead_behind_local:  Option<(usize, usize)>,
    /// The current branch's `@{upstream}` tracked ref (e.g. `origin/main`).
    /// Stored as a string instead of an index so a primary-side remotes
    /// rewrite cannot silently invalidate a linked checkout's pointer.
    /// `None` when the current branch has no upstream tracking ref.
    pub primary_tracked_ref: Option<String>,
}

impl CheckoutInfo {
    /// The remote matching the current branch's `@{upstream}` within
    /// `repo`, if any. Lookup is by name match against
    /// `repo.remotes[i].tracked_ref` — this is rendered data, not hot.
    pub fn primary_remote<'r>(&self, repo: &'r RepoInfo) -> Option<&'r RemoteInfo> {
        let want = self.primary_tracked_ref.as_deref()?;
        repo.remotes
            .iter()
            .find(|r| r.tracked_ref.as_deref() == Some(want))
    }

    /// The primary remote's URL, looked up against `repo`.
    pub fn primary_url<'r>(&self, repo: &'r RepoInfo) -> Option<&'r str> {
        self.primary_remote(repo).and_then(|r| r.url.as_deref())
    }

    /// The primary remote's ahead/behind vs its tracked ref.
    pub fn primary_ahead_behind(&self, repo: &RepoInfo) -> Option<(usize, usize)> {
        self.primary_remote(repo).and_then(|r| r.ahead_behind)
    }

    /// The primary remote's tracked ref (e.g. `origin/main`).
    pub fn primary_tracked_ref(&self) -> Option<&str> { self.primary_tracked_ref.as_deref() }

    /// Probe per-checkout git metadata for `probe_path`. Cheap (no
    /// per-remote loop). `local_main_branch` is supplied by the caller —
    /// usually pulled from the entry's `RepoInfo.local_main_branch`,
    /// which is identical across siblings so probing it once at the
    /// `RepoInfo::get` call avoids redundant work.
    pub fn get(probe_path: &Path, local_main_branch: Option<&str>) -> Option<Self> {
        let repo_root = discovery::git_repo_root(probe_path)?;

        let branch = repo::get_current_branch(&repo_root);
        let current_upstream = repo::get_upstream_branch(&repo_root);
        let ahead_behind_local = local_main_branch
            .filter(|branch_name| branch.as_deref() != Some(*branch_name))
            .and_then(|branch_name| {
                parse_ahead_behind(
                    &repo_root,
                    &format!("HEAD...{branch_name}"),
                    "configured_local_main",
                )
            });
        let last_commit = command::git_output_logged(
            &repo_root,
            "log_last_commit",
            ["log", "-1", "--format=%aI"],
        )
        .ok()
        .and_then(|o| {
            let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
            if s.is_empty() { None } else { Some(s) }
        });

        Some(Self {
            status: get_git_status(probe_path, &repo_root),
            branch,
            last_commit,
            ahead_behind_local,
            primary_tracked_ref: current_upstream,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum GitStatus {
    Clean,
    Modified,
    Untracked,
    Ignored,
}

impl GitStatus {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Clean => "clean",
            Self::Modified => "modified",
            Self::Untracked => "untracked",
            Self::Ignored => "ignored",
        }
    }

    pub const fn icon(self) -> &'static str {
        match self {
            Self::Clean => GIT_STATUS_CLEAN,
            Self::Modified => GIT_STATUS_MODIFIED,
            Self::Untracked => GIT_STATUS_UNTRACKED,
            Self::Ignored => "",
        }
    }

    pub fn label_with_icon(self) -> String {
        let icon = self.icon();
        if icon.is_empty() {
            self.label().to_string()
        } else {
            format!("{icon} {}", self.label())
        }
    }
}

/// Wrapper for `CheckoutInfo` that distinguishes "not yet detected"
/// from "detected with full metadata."
#[derive(Clone, Debug, Default)]
pub enum LocalGitState {
    /// Not yet detected (during startup/scan).
    #[default]
    Pending,
    /// Per-checkout git metadata detected for this project path.
    Detected(Box<CheckoutInfo>),
}

impl LocalGitState {
    pub fn info(&self) -> Option<&CheckoutInfo> {
        match self {
            Self::Detected(info) => Some(info),
            Self::Pending => None,
        }
    }
}

/// Ahead/behind of `worktree_dir`'s HEAD vs `primary_dir`'s HEAD. The
/// primary HEAD is resolved to a commit SHA so refs resolve cleanly across
/// the worktree's ref namespace.
pub fn worktree_ahead_behind_primary(
    worktree_dir: &Path,
    primary_dir: &Path,
) -> Option<(usize, usize)> {
    let primary_sha =
        command::git_output_logged(primary_dir, "worktree_primary_sha", ["rev-parse", "HEAD"])
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

/// Get git path state when the repo root is already known, avoiding a
/// redundant `git_repo_root()` call.
fn get_git_status(project_dir: &Path, repo_root: &Path) -> GitStatus {
    let started = std::time::Instant::now();
    let relative_path = relative_git_path(repo_root, project_dir);
    if relative_path != "." {
        let ignored = command::git_command(repo_root)
            .args(["check-ignore", "-q", "--", &relative_path])
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
    let status_output = command::git_command(repo_root)
        .args([
            "status",
            "--porcelain=v1",
            "--ignored=matching",
            "--untracked-files=all",
            "--",
            &relative_path,
        ])
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

fn relative_git_path(repo_root: &Path, project_dir: &Path) -> String {
    project_dir.strip_prefix(repo_root).ok().map_or_else(
        || ".".to_string(),
        |path| {
            let normalized = path
                .components()
                .filter_map(|component| match component {
                    Component::Normal(segment) => Some(segment.to_string_lossy().to_string()),
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
pub(super) fn parse_ahead_behind(
    project_dir: &Path,
    revspec: &str,
    op_suffix: &str,
) -> Option<(usize, usize)> {
    command::git_output_logged(
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
