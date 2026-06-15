use std::path::Component;
use std::path::Path;

use serde::Deserialize;
use serde::Serialize;
use tui_pane::PERF_LOG_TARGET;

use super::branches;
use super::command;
use super::constants::GIT_BISECT_BAD_REF;
use super::constants::GIT_BISECT_GOOD_REF_PREFIX;
use super::constants::GIT_BISECT_REFS_PREFIX;
use super::constants::GIT_BISECT_START_FILE;
use super::constants::GIT_BISECT_VARS_ARG;
use super::constants::GIT_CHECK_IGNORE_COMMAND;
use super::constants::GIT_COUNT_ARG;
use super::constants::GIT_DOUBLE_DASH_ARG;
use super::constants::GIT_FOR_EACH_REF_COMMAND;
use super::constants::GIT_FORMAT_ISO8601_ARG;
use super::constants::GIT_FORMAT_REFNAME_ARG;
use super::constants::GIT_HEAD;
use super::constants::GIT_HEAD_REVSPEC_PREFIX;
use super::constants::GIT_IGNORED_STATUS_CODE;
use super::constants::GIT_LEFT_RIGHT_ARG;
use super::constants::GIT_LOG_COMMAND;
use super::constants::GIT_LOG_LAST_COMMIT_ARG;
use super::constants::GIT_NOT_ARG;
use super::constants::GIT_QUIET_SHORT_ARG;
use super::constants::GIT_REV_LIST_COMMAND;
use super::constants::GIT_REV_PARSE_COMMAND;
use super::constants::GIT_SHORT_HEAD_ARG;
use super::constants::GIT_STATUS_COMMAND;
use super::constants::GIT_STATUS_IGNORED_MATCHING_ARG;
use super::constants::GIT_STATUS_PORCELAIN_V1_ARG;
use super::constants::GIT_STATUS_UNTRACKED_ALL_ARG;
use super::constants::GIT_UNTRACKED_STATUS_CODE;
use super::discovery;
use super::repo::RemoteInfo;
use super::repo::RepoInfo;
use crate::constants::GIT_STATUS_CLEAN;
use crate::constants::GIT_STATUS_MODIFIED;
use crate::constants::GIT_STATUS_UNTRACKED;

/// The resolved state of `HEAD` for a checkout.
///
/// Replaces the older `branch: Option<String>` field, which conflated three
/// distinct cases:
/// - `Unborn`: repo exists but has no commits (`git rev-parse HEAD` fails).
/// - `Branch(name)`: `HEAD` points at the named branch.
/// - `Detached { short_sha }`: `HEAD` is a detached commit; `short_sha` is the 8-char abbreviation
///   (matching `ls_tree_submodule_commits`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum HeadState {
    Unborn,
    Branch(String),
    Detached { short_sha: String },
}

impl HeadState {
    /// The branch name when `HEAD` points at one. `None` for detached
    /// and unborn checkouts. Use for comparisons against
    /// `local_main_branch` and other branch-name lookups.
    pub const fn branch_name(&self) -> Option<&str> {
        match self {
            Self::Branch(name) => Some(name.as_str()),
            Self::Unborn | Self::Detached { .. } => None,
        }
    }

    /// Short display label, suitable for compact UI like the finder
    /// column. `Branch(name)` → `name`, `Detached { short_sha }` →
    /// `"detached @ <short_sha>"`, `Unborn` → `""`.
    pub fn display_label(&self) -> String {
        match self {
            Self::Branch(name) => name.clone(),
            Self::Detached { short_sha } => format!("detached @ {short_sha}"),
            Self::Unborn => String::new(),
        }
    }
}

/// Per-checkout git metadata: state that can legitimately differ between
/// two worktrees of the same repo. Lives inside `ProjectInfo.local_git_state`.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct CheckoutInfo {
    /// Git path state (clean, modified, untracked, etc.) for this project path.
    pub status:              GitStatus,
    /// The resolved state of `HEAD` (branch, detached, or unborn).
    pub head:                HeadState,
    /// ISO 8601 date of the most recent commit on this branch.
    pub last_commit:         Option<String>,
    /// Commits ahead and behind the local `{local_main_branch}`.
    pub ahead_behind_local:  Option<(usize, usize)>,
    /// The current branch's `@{upstream}` tracked ref (e.g. `origin/main`).
    /// Stored as a string instead of an index so a primary-side remotes
    /// rewrite cannot silently invalidate a linked checkout's pointer.
    /// `None` when the current branch has no upstream tracking ref.
    pub primary_tracked_ref: Option<String>,
    /// Progress of an in-flight `git bisect`, when one is running for this
    /// checkout. `None` when no bisect is active.
    pub bisect:              Option<BisectProgress>,
}

/// Progress of an in-flight `git bisect`.
///
/// `git bisect` checks out each candidate in detached `HEAD`, so this is
/// the only context where the Git pane reports position-in-history rather
/// than just the detached SHA.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum BisectProgress {
    /// Bisect started, but a known-good and known-bad commit are not both
    /// marked yet, so git cannot estimate the remaining work.
    Awaiting,
    /// Narrowing the range, carrying git's own remaining-work estimate:
    /// `revisions` left to test and the rough `steps` count.
    Narrowing { revisions: usize, steps: usize },
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

        let head = resolve_head_state(&repo_root);
        let current_upstream = branches::get_upstream_branch(&repo_root);
        let ahead_behind_local = local_main_branch
            .filter(|branch_name| head.branch_name() != Some(*branch_name))
            .and_then(|branch_name| {
                parse_ahead_behind(
                    &repo_root,
                    &format!("{GIT_HEAD_REVSPEC_PREFIX}{branch_name}"),
                    "configured_local_main",
                )
            });
        let last_commit = command::git_output_logged(
            &repo_root,
            "log_last_commit",
            [
                GIT_LOG_COMMAND,
                GIT_LOG_LAST_COMMIT_ARG,
                GIT_FORMAT_ISO8601_ARG,
            ],
        )
        .ok()
        .and_then(|o| {
            let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
            if s.is_empty() { None } else { Some(s) }
        });

        Some(Self {
            status: get_git_status(probe_path, &repo_root),
            head,
            last_commit,
            ahead_behind_local,
            primary_tracked_ref: current_upstream,
            bisect: bisect_progress(&repo_root),
        })
    }
}

/// Detect an in-flight `git bisect` for `repo_root` and report its
/// progress. Returns `None` in the common case where no bisect is running.
///
/// The `BISECT_START` existence check is a cheap gate that keeps the
/// non-bisecting path free of any subprocess spawn — git writes the file
/// on `git bisect start` and removes it on `git bisect reset`.
fn bisect_progress(repo_root: &Path) -> Option<BisectProgress> {
    let git_dir = discovery::resolve_git_dir(repo_root)?;
    if !git_dir.as_path().join(GIT_BISECT_START_FILE).exists() {
        return None;
    }

    let refs = command::git_output_logged(
        repo_root,
        "bisect_refs",
        [
            GIT_FOR_EACH_REF_COMMAND,
            GIT_FORMAT_REFNAME_ARG,
            GIT_BISECT_REFS_PREFIX,
        ],
    )
    .ok()?;
    let refs = String::from_utf8_lossy(&refs.stdout);

    let mut bad = false;
    let mut good = Vec::new();
    for refname in refs.lines().map(str::trim).filter(|line| !line.is_empty()) {
        if refname == GIT_BISECT_BAD_REF {
            bad = true;
        } else if refname.starts_with(GIT_BISECT_GOOD_REF_PREFIX) {
            good.push(refname.to_string());
        }
    }

    // git can only estimate remaining work once both bounds are marked.
    if !bad || good.is_empty() {
        return Some(BisectProgress::Awaiting);
    }
    Some(bisect_vars(repo_root, &good).unwrap_or(BisectProgress::Awaiting))
}

/// Run `git rev-list --bisect-vars` over the marked range and parse the
/// `bisect_nr` (revisions left) and `bisect_steps` (rough step count)
/// fields git emits. `good` must be non-empty.
fn bisect_vars(repo_root: &Path, good: &[String]) -> Option<BisectProgress> {
    let mut args: Vec<&str> = vec![
        GIT_REV_LIST_COMMAND,
        GIT_BISECT_VARS_ARG,
        GIT_BISECT_BAD_REF,
        GIT_NOT_ARG,
    ];
    args.extend(good.iter().map(String::as_str));
    let output = command::git_command(repo_root).args(&args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    parse_bisect_vars(&String::from_utf8_lossy(&output.stdout))
}

/// Parse the `bisect_nr` (revisions left) and `bisect_steps` (rough step
/// count) fields from `git rev-list --bisect-vars` output. `None` if
/// either field is absent or unparseable.
fn parse_bisect_vars(stdout: &str) -> Option<BisectProgress> {
    let mut revisions = None;
    let mut steps = None;
    for line in stdout.lines() {
        if let Some(value) = line.strip_prefix("bisect_nr=") {
            revisions = value.trim().parse::<usize>().ok();
        } else if let Some(value) = line.strip_prefix("bisect_steps=") {
            steps = value.trim().parse::<usize>().ok();
        }
    }
    Some(BisectProgress::Narrowing {
        revisions: revisions?,
        steps:     steps?,
    })
}

/// Resolve `HEAD` to a `HeadState`:
/// - `rev-parse --abbrev-ref HEAD` returns the branch name, or the literal `"HEAD"` when detached,
///   or empty when unborn.
/// - For detached, run `rev-parse --short=8 HEAD` to fetch the SHA.
/// - For an unborn or otherwise-unresolvable HEAD, return `Unborn`.
fn resolve_head_state(repo_root: &Path) -> HeadState {
    let abbrev = branches::get_current_branch(repo_root);
    match abbrev.as_deref() {
        None => HeadState::Unborn,
        Some(GIT_HEAD) => command::git_output_logged(
            repo_root,
            "rev_parse_short_head",
            [GIT_REV_PARSE_COMMAND, GIT_SHORT_HEAD_ARG, GIT_HEAD],
        )
        .ok()
        .and_then(|o| {
            let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
            if s.is_empty() { None } else { Some(s) }
        })
        .map_or(HeadState::Unborn, |short_sha| HeadState::Detached {
            short_sha,
        }),
        Some(name) => HeadState::Branch(name.to_string()),
    }
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
pub(crate) enum LocalGitState {
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
pub(crate) fn worktree_ahead_behind_primary(
    worktree_dir: &Path,
    primary_dir: &Path,
) -> Option<(usize, usize)> {
    let primary_sha = command::git_output_logged(
        primary_dir,
        "worktree_primary_sha",
        [GIT_REV_PARSE_COMMAND, GIT_HEAD],
    )
    .ok()
    .and_then(|o| {
        let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
        if s.is_empty() { None } else { Some(s) }
    })?;
    parse_ahead_behind(
        worktree_dir,
        &format!("{GIT_HEAD_REVSPEC_PREFIX}{primary_sha}"),
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
            .args([
                GIT_CHECK_IGNORE_COMMAND,
                GIT_QUIET_SHORT_ARG,
                GIT_DOUBLE_DASH_ARG,
                &relative_path,
            ])
            .status()
            .ok()
            .is_some_and(|status| status.success());
        if ignored {
            let state = GitStatus::Ignored;
            tracing::trace!(
                target: PERF_LOG_TARGET,
                elapsed_ms = tui_pane::perf_log_ms(started.elapsed().as_millis()),
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
            GIT_STATUS_COMMAND,
            GIT_STATUS_PORCELAIN_V1_ARG,
            GIT_STATUS_IGNORED_MATCHING_ARG,
            GIT_STATUS_UNTRACKED_ALL_ARG,
            GIT_DOUBLE_DASH_ARG,
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
            GIT_IGNORED_STATUS_CODE => {},
            GIT_UNTRACKED_STATUS_CODE => has_untracked = true,
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
    tracing::trace!(
        target: PERF_LOG_TARGET,
        elapsed_ms = tui_pane::perf_log_ms(started.elapsed().as_millis()),
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
        [
            GIT_REV_LIST_COMMAND,
            GIT_LEFT_RIGHT_ARG,
            GIT_COUNT_ARG,
            revspec,
        ],
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

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use super::*;

    #[test]
    fn head_state_branch_name_for_branch() {
        let h = HeadState::Branch("main".to_string());
        assert_eq!(h.branch_name(), Some("main"));
    }

    #[test]
    fn head_state_branch_name_for_detached() {
        let h = HeadState::Detached {
            short_sha: "abc12345".to_string(),
        };
        assert_eq!(h.branch_name(), None);
    }

    #[test]
    fn head_state_branch_name_for_unborn() {
        assert_eq!(HeadState::Unborn.branch_name(), None);
    }

    #[test]
    fn head_state_display_label() {
        assert_eq!(HeadState::Branch("dev".to_string()).display_label(), "dev");
        assert_eq!(
            HeadState::Detached {
                short_sha: "deadbeef".to_string(),
            }
            .display_label(),
            "detached @ deadbeef"
        );
        assert_eq!(HeadState::Unborn.display_label(), "");
    }

    #[test]
    fn head_state_serde_round_trip() {
        for state in [
            HeadState::Unborn,
            HeadState::Branch("feat/x".to_string()),
            HeadState::Detached {
                short_sha: "01234567".to_string(),
            },
        ] {
            let json = serde_json::to_string(&state).expect("serialize");
            let back: HeadState = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(state, back);
        }
    }

    #[test]
    fn parse_bisect_vars_reads_nr_and_steps() {
        let stdout = "\
bisect_rev=abc123
bisect_nr=6
bisect_steps=3
bisect_good=def456
bisect_bad=789abc
bisect_all=13
";
        assert_eq!(
            parse_bisect_vars(stdout),
            Some(BisectProgress::Narrowing {
                revisions: 6,
                steps:     3,
            })
        );
    }

    #[test]
    fn parse_bisect_vars_missing_field_is_none() {
        assert_eq!(parse_bisect_vars("bisect_nr=6\n"), None);
        assert_eq!(parse_bisect_vars(""), None);
    }
}
