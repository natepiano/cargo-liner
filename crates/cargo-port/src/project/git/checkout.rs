use std::path::Component;
use std::path::Path;

use serde::Deserialize;
use serde::Serialize;
use tui_pane::PERF_LOG_TARGET;

use super::branches;
use super::branches::ResolvedBranch;
use super::command;
use super::command::GitAnswer;
use super::command::GitReadError;
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
use super::constants::GIT_SYMBOLIC_REF_COMMAND;
use super::constants::GIT_UNTRACKED_STATUS_CODE;
use super::constants::GIT_VERIFY_ARG;
use super::discovery;
use super::probes;
use super::repo::RemoteInfo;
use super::repo::RepoInfo;
use crate::constants::GIT_STATUS_CLEAN;
use crate::constants::GIT_STATUS_MODIFIED;
use crate::constants::GIT_STATUS_UNREADABLE;
use crate::constants::GIT_STATUS_UNTRACKED;
use crate::constants::GIT_UNREADABLE_LABEL;
use crate::project::RemoteList;

/// The resolved state of `HEAD` for a checkout.
///
/// Replaces the older `branch: Option<String>` field, which conflated distinct cases:
/// - `Unborn`: repo exists but has no commits.
/// - `Unreadable`: git cannot read the HEAD file or the ref it names.
/// - `Branch(name)`: `HEAD` points at the named branch.
/// - `Detached { short_sha }`: `HEAD` is a detached commit; `short_sha` is the 8-char abbreviation
///   (matching `ls_tree_submodule_commits`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum HeadState {
    /// git could not read `HEAD`: the HEAD file or the ref it names is missing or corrupt.
    Unreadable,
    Unborn,
    Branch(String),
    Detached {
        short_sha: String,
    },
}

impl HeadState {
    /// The branch name when `HEAD` points at one. `None` for detached
    /// unborn, and unreadable checkouts. Use for comparisons against
    /// `local_main_branch` and other branch-name lookups.
    pub(crate) const fn branch_name(&self) -> Option<&str> {
        match self {
            Self::Branch(name) => Some(name.as_str()),
            Self::Unreadable | Self::Unborn | Self::Detached { .. } => None,
        }
    }

    /// Short display label, suitable for compact UI like the finder
    /// column. `Branch(name)` → `name`, `Detached { short_sha }` →
    /// `"detached @ <short_sha>"`, `Unborn` / `Unreadable` → `""`.
    pub(crate) fn display_label(&self) -> String {
        match self {
            Self::Branch(name) => name.clone(),
            Self::Detached { short_sha } => format!("detached @ {short_sha}"),
            Self::Unreadable | Self::Unborn => String::new(),
        }
    }
}

/// A commit date keeps an empty history distinct from a failed read.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub(crate) enum CommitDate {
    At(String),
    #[default]
    NoCommits,
    Unreadable,
}

/// The upstream ref confirmed by git for this checkout.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub(crate) enum TrackedRef {
    Ref(String),
    #[default]
    Untracked,
    Unreadable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub(crate) struct AheadBehind {
    pub ahead:  usize,
    pub behind: usize,
}

/// A comparison is absent only when git did not need to compare.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub(crate) enum Comparison {
    Counted(AheadBehind),
    #[default]
    NotCompared,
    Unreadable,
}

/// Per-checkout git metadata: state that can legitimately differ between
/// two worktrees of the same repo. Lives inside `ProjectInfo.local_git_state`.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct CheckoutInfo {
    /// Git path state (clean, modified, untracked, etc.) for this project path.
    pub status:              GitStatus,
    /// The resolved state of `HEAD` (branch, detached, unborn, or unreadable).
    pub head:                HeadState,
    /// ISO 8601 date of the most recent commit on this branch.
    pub last_commit:         CommitDate,
    /// Commits ahead and behind the local `{local_main_branch}`.
    pub ahead_behind_local:  Comparison,
    /// The current branch's `@{upstream}` tracked ref (e.g. `origin/main`).
    /// Stored as a string instead of an index so a primary-side remotes
    /// rewrite cannot silently invalidate a linked checkout's pointer.
    /// `Untracked` when the current branch has no upstream tracking ref.
    pub primary_tracked_ref: TrackedRef,
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
    /// Git could not read or estimate the active bisect.
    Unreadable,
    /// Narrowing the range, carrying git's own remaining-work estimate:
    /// `revisions` left to test and the rough `steps` count.
    Narrowing { revisions: usize, steps: usize },
}

impl CheckoutInfo {
    /// The remote matching the current branch's `@{upstream}` within
    /// `repo`, if any. Lookup is by name match against
    /// `repo.remotes[i].tracked_ref` — this is rendered data, not hot.
    pub(crate) fn primary_remote<'r>(&self, repo: &'r RepoInfo) -> Option<&'r RemoteInfo> {
        match &self.primary_tracked_ref {
            TrackedRef::Ref(want) => repo.remotes.listed().iter().find(
                |remote| matches!(&remote.tracked_ref, TrackedRef::Ref(name) if name == want),
            ),
            TrackedRef::Untracked | TrackedRef::Unreadable => None,
        }
    }

    /// The primary remote's URL, looked up against `repo`.
    pub(crate) fn primary_url<'r>(&self, repo: &'r RepoInfo) -> Option<&'r str> {
        self.primary_remote(repo)
            .and_then(|remote| remote.url.as_deref())
    }

    pub(crate) fn primary_ahead_behind(&self, repo: &RepoInfo) -> Comparison {
        match &self.primary_tracked_ref {
            TrackedRef::Unreadable => Comparison::Unreadable,
            TrackedRef::Ref(_) => match &repo.remotes {
                RemoteList::Unreadable => Comparison::Unreadable,
                RemoteList::Listed(_) => self
                    .primary_remote(repo)
                    .map_or(Comparison::NotCompared, |remote| remote.ahead_behind),
            },
            TrackedRef::Untracked => Comparison::NotCompared,
        }
    }

    /// Probe per-checkout git metadata for `probe_path`. Cheap (no
    /// per-remote loop). `local_main_branch` is supplied by the caller —
    /// usually pulled from the entry's `RepoInfo.local_main_branch`,
    /// which is identical across siblings so probing it once at the
    /// `RepoInfo::get` call avoids redundant work.
    pub(crate) fn get(probe_path: &Path, local_main_branch: &ResolvedBranch) -> Option<Self> {
        let repo_root = discovery::git_repo_root(probe_path)?;

        let head = resolve_head_state(&repo_root);
        let (current_upstream, last_commit, status, bisect) = std::thread::scope(|scope| {
            let upstream = scope.spawn(|| branches::get_upstream_branch(&repo_root, &head));
            let commit = scope.spawn(|| last_commit_date(&repo_root, &head));
            let status = scope.spawn(|| get_git_status(probe_path, &repo_root));
            let bisect = scope.spawn(|| bisect_progress(&repo_root));
            (
                probes::finish_probe(upstream),
                probes::finish_probe(commit),
                probes::finish_probe(status),
                probes::finish_probe(bisect),
            )
        });
        let ahead_behind_local = match (&head, local_main_branch) {
            (
                HeadState::Unreadable,
                ResolvedBranch::Named(_) | ResolvedBranch::NotFound | ResolvedBranch::Unreadable,
            )
            | (
                HeadState::Branch(_) | HeadState::Unborn | HeadState::Detached { .. },
                ResolvedBranch::Unreadable,
            ) => Comparison::Unreadable,
            (HeadState::Unborn, ResolvedBranch::Named(_) | ResolvedBranch::NotFound)
            | (HeadState::Branch(_) | HeadState::Detached { .. }, ResolvedBranch::NotFound) => {
                Comparison::NotCompared
            },
            (HeadState::Branch(name), ResolvedBranch::Named(main)) if name == main => {
                Comparison::NotCompared
            },
            (HeadState::Branch(_) | HeadState::Detached { .. }, ResolvedBranch::Named(main)) => {
                parse_ahead_behind(
                    &repo_root,
                    &format!("{GIT_HEAD_REVSPEC_PREFIX}{main}"),
                    "configured_local_main",
                )
                .map_or(Comparison::Unreadable, Comparison::Counted)
            },
        };
        Some(Self {
            status,
            head,
            last_commit,
            ahead_behind_local,
            primary_tracked_ref: current_upstream,
            bisect,
        })
    }
}

fn last_commit_date(repo_root: &Path, head: &HeadState) -> CommitDate {
    match head {
        HeadState::Unborn => CommitDate::NoCommits,
        HeadState::Unreadable => CommitDate::Unreadable,
        HeadState::Branch(_) | HeadState::Detached { .. } => match command::git_output_logged(
            repo_root,
            "log_last_commit",
            &[
                GIT_LOG_COMMAND,
                GIT_LOG_LAST_COMMIT_ARG,
                GIT_FORMAT_ISO8601_ARG,
            ],
        ) {
            Ok(output) if !output.trimmed().is_empty() => {
                CommitDate::At(output.trimmed().to_string())
            },
            Ok(_) | Err(_) => CommitDate::Unreadable,
        },
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
        &[
            GIT_FOR_EACH_REF_COMMAND,
            GIT_FORMAT_REFNAME_ARG,
            GIT_BISECT_REFS_PREFIX,
        ],
    );
    let Ok(refs) = refs else {
        return Some(BisectProgress::Unreadable);
    };

    let mut bad = false;
    let mut good = Vec::new();
    for refname in refs.lines().map(str::trim).filter(|line| !line.is_empty()) {
        if refname == GIT_BISECT_BAD_REF {
            bad = true;
        } else if refname.starts_with(GIT_BISECT_GOOD_REF_PREFIX) {
            good.push(refname.to_string());
        }
    }

    // `BisectProgress::Awaiting` is returned until `bad` is true and `good` is
    // nonempty, because Git needs both bisect bounds to estimate remaining work.
    if !bad || good.is_empty() {
        return Some(BisectProgress::Awaiting);
    }
    Some(bisect_vars(repo_root, &good).unwrap_or(BisectProgress::Unreadable))
}

/// Run `git rev-list --bisect-vars` over the marked range and parse the
/// `bisect_nr` (revisions left) and `bisect_steps` (rough step count)
/// fields git emits. `good` must be non-empty.
fn bisect_vars(repo_root: &Path, good: &[String]) -> Result<BisectProgress, GitReadError> {
    let mut args: Vec<&str> = vec![
        GIT_REV_LIST_COMMAND,
        GIT_BISECT_VARS_ARG,
        GIT_BISECT_BAD_REF,
        GIT_NOT_ARG,
    ];
    args.extend(good.iter().map(String::as_str));
    let output = command::git_output_logged(repo_root, "bisect_vars", &args)?;
    parse_bisect_vars(output.trimmed()).ok_or_else(|| GitReadError::unparsable("bisect_vars"))
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

/// Resolve `HEAD` using the exit status of each git probe.
///
/// | Abbreviated HEAD | Follow-up | State |
/// | --- | --- | --- |
/// | Branch name, exit 0 | None | Branch |
/// | HEAD, exit 0 | Verified short SHA, exit 0 and non-empty | Detached |
/// | HEAD, exit 0 | Verification fails or is empty | Unreadable |
/// | No successful abbreviated HEAD | symbolic-ref exits 0 | Unborn |
/// | No successful abbreviated HEAD | symbolic-ref fails | Unreadable |
pub(super) fn resolve_head_state(repo_root: &Path) -> HeadState {
    let abbrev = branches::get_current_branch(repo_root);
    match abbrev.as_deref() {
        None => unborn_or_unreadable(repo_root),
        Some(GIT_HEAD) => command::git_output_logged(
            repo_root,
            "rev_parse_short_head",
            &[
                GIT_REV_PARSE_COMMAND,
                GIT_VERIFY_ARG,
                GIT_QUIET_SHORT_ARG,
                GIT_SHORT_HEAD_ARG,
                GIT_HEAD,
            ],
        )
        .ok()
        .and_then(|output| {
            let sha = output.trimmed().to_string();
            if sha.is_empty() { None } else { Some(sha) }
        })
        .map_or(HeadState::Unreadable, |short_sha| HeadState::Detached {
            short_sha,
        }),
        Some(name) => HeadState::Branch(name.to_string()),
    }
}

/// A symbolic HEAD with no readable commit is unborn; a failed symbolic
/// reference probe means git cannot read HEAD.
fn unborn_or_unreadable(repo_root: &Path) -> HeadState {
    match command::git_answer_logged(
        repo_root,
        "symbolic_ref_head",
        &[GIT_SYMBOLIC_REF_COMMAND, GIT_QUIET_SHORT_ARG, GIT_HEAD],
    ) {
        Ok(GitAnswer::Yes(_)) => HeadState::Unborn,
        Ok(GitAnswer::No) | Err(_) => HeadState::Unreadable,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum GitStatus {
    Unreadable,
    Clean,
    Modified,
    Untracked,
    Ignored,
}

impl GitStatus {
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Unreadable => GIT_UNREADABLE_LABEL,
            Self::Clean => "clean",
            Self::Modified => "modified",
            Self::Untracked => "untracked",
            Self::Ignored => "ignored",
        }
    }

    pub(crate) const fn icon(self) -> &'static str {
        match self {
            Self::Unreadable => GIT_STATUS_UNREADABLE,
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
    pub(crate) fn info(&self) -> Option<&CheckoutInfo> {
        match self {
            Self::Detected(info) => Some(info),
            Self::Pending => None,
        }
    }
}

/// Ahead/behind of `worktree_dir`'s HEAD vs `primary_dir`'s HEAD. The
/// primary HEAD is resolved to a commit SHA so refs resolve cleanly across
/// the worktree's ref namespace.
pub(crate) fn worktree_ahead_behind_primary(worktree_dir: &Path, primary_dir: &Path) -> Comparison {
    match command::git_answer_logged(
        primary_dir,
        "worktree_primary_sha",
        &[
            GIT_REV_PARSE_COMMAND,
            GIT_QUIET_SHORT_ARG,
            GIT_VERIFY_ARG,
            GIT_HEAD,
        ],
    ) {
        Ok(GitAnswer::Yes(output)) if !output.trimmed().is_empty() => parse_ahead_behind(
            worktree_dir,
            &format!("{GIT_HEAD_REVSPEC_PREFIX}{}", output.trimmed()),
            "worktree_vs_primary",
        )
        .map_or(Comparison::Unreadable, Comparison::Counted),
        Ok(GitAnswer::No) => Comparison::NotCompared,
        Ok(GitAnswer::Yes(_)) | Err(_) => Comparison::Unreadable,
    }
}

/// Get git path state when the repo root is already known, avoiding a
/// redundant `git_repo_root()` call.
fn get_git_status(project_dir: &Path, repo_root: &Path) -> GitStatus {
    let started = std::time::Instant::now();
    let state = read_git_status(project_dir, repo_root);
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

fn read_git_status(project_dir: &Path, repo_root: &Path) -> GitStatus {
    let relative_path = relative_git_path(repo_root, project_dir);
    if relative_path != "." {
        match command::git_answer_logged(
            repo_root,
            "check_ignore",
            &[
                GIT_CHECK_IGNORE_COMMAND,
                GIT_QUIET_SHORT_ARG,
                GIT_DOUBLE_DASH_ARG,
                &relative_path,
            ],
        ) {
            Ok(GitAnswer::Yes(_)) => return GitStatus::Ignored,
            Ok(GitAnswer::No) => {},
            Err(_) => return GitStatus::Unreadable,
        }
    }
    let Ok(stdout) = command::git_output_logged(
        repo_root,
        "status",
        &[
            GIT_STATUS_COMMAND,
            GIT_STATUS_PORCELAIN_V1_ARG,
            GIT_STATUS_IGNORED_MATCHING_ARG,
            GIT_STATUS_UNTRACKED_ALL_ARG,
            GIT_DOUBLE_DASH_ARG,
            &relative_path,
        ],
    ) else {
        return GitStatus::Unreadable;
    };
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

    if has_modified {
        GitStatus::Modified
    } else if has_untracked {
        GitStatus::Untracked
    } else {
        GitStatus::Clean
    }
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
) -> Result<AheadBehind, GitReadError> {
    let op = format!("rev_list_{op_suffix}");
    let output = command::git_output_logged(
        project_dir,
        &op,
        &[
            GIT_REV_LIST_COMMAND,
            GIT_LEFT_RIGHT_ARG,
            GIT_COUNT_ARG,
            revspec,
        ],
    )?;
    let mut parts = output.trimmed().split_whitespace();
    let counts = parts
        .next()
        .and_then(|part| part.parse().ok())
        .zip(parts.next().and_then(|part| part.parse().ok()));
    match counts {
        Some((ahead, behind)) if parts.next().is_none() => Ok(AheadBehind { ahead, behind }),
        Some(_) | None => Err(GitReadError::unparsable(&op)),
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::sync::Arc;
    use std::sync::Mutex;

    use tracing::Event;
    use tracing::Metadata;
    use tracing::Subscriber;
    use tracing::field::Field;
    use tracing::field::Visit;
    use tracing::span::Attributes;
    use tracing::span::Id;
    use tracing::span::Record;

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
            HeadState::Unreadable,
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
    fn resolve_head_state_reports_unreadable_head_not_unborn() {
        let corrupt_head = tempfile::tempdir().expect("create corrupt HEAD fixture");
        crate::test_support::init_git_repo(corrupt_head.path());
        std::fs::write(corrupt_head.path().join(".git/HEAD"), "garbage\n").expect("corrupt HEAD");
        assert_eq!(
            resolve_head_state(corrupt_head.path()),
            HeadState::Unreadable
        );
        assert_eq!(
            CheckoutInfo::get(
                corrupt_head.path(),
                &crate::project::ResolvedBranch::NotFound
            )
            .expect("probe corrupt checkout")
            .head,
            HeadState::Unreadable,
        );

        let corrupt_ref = tempfile::tempdir().expect("create corrupt ref fixture");
        crate::test_support::init_git_repo(corrupt_ref.path());
        let head = std::fs::read_to_string(corrupt_ref.path().join(".git/HEAD"))
            .expect("read symbolic HEAD");
        let branch_ref = head.trim().strip_prefix("ref: ").expect("HEAD names a ref");
        std::fs::write(corrupt_ref.path().join(".git").join(branch_ref), "zzzz\n")
            .expect("corrupt branch ref");
        assert_eq!(
            resolve_head_state(corrupt_ref.path()),
            HeadState::Unreadable
        );

        let unborn = tempfile::tempdir().expect("create unborn fixture");
        let output = std::process::Command::new(crate::test_support::git_binary())
            .arg("init")
            .current_dir(unborn.path())
            .output()
            .expect("initialize unborn repository");
        assert!(output.status.success());
        assert_eq!(resolve_head_state(unborn.path()), HeadState::Unborn);

        let intact = tempfile::tempdir().expect("create intact fixture");
        crate::test_support::init_git_repo(intact.path());
        assert!(matches!(
            resolve_head_state(intact.path()),
            HeadState::Branch(_)
        ));
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

    #[derive(Clone, Default)]
    struct GitStatusTrace {
        events: Arc<Mutex<Vec<String>>>,
    }

    impl Subscriber for GitStatusTrace {
        fn enabled(&self, _: &Metadata<'_>) -> bool { true }
        fn new_span(&self, _: &Attributes<'_>) -> Id { tracing::span::Id::from_u64(1) }
        fn record(&self, _: &Id, _: &Record<'_>) {}
        fn record_follows_from(&self, _: &Id, _: &Id) {}
        fn enter(&self, _: &Id) {}
        fn exit(&self, _: &Id) {}
        fn event(&self, event: &Event<'_>) {
            if event.metadata().target() == PERF_LOG_TARGET {
                let mut fields = TraceFields::default();
                event.record(&mut fields);
                self.events.lock().expect("trace lock").push(fields.message);
            }
        }
    }

    #[derive(Default)]
    struct TraceFields {
        message: String,
    }

    impl Visit for TraceFields {
        fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
            if field.name() == "message" {
                self.message = format!("{value:?}");
            }
        }
    }

    #[test]
    fn ignored_git_path_emits_one_status_performance_trace() {
        let repo = tempfile::tempdir().expect("fixture");
        crate::test_support::init_git_repo(repo.path());
        let ignored = repo.path().join("ignored");
        std::fs::create_dir(&ignored).expect("ignored directory");
        std::fs::write(repo.path().join(".gitignore"), "ignored/\n").expect("ignore rule");
        let trace = GitStatusTrace::default();
        tracing::subscriber::with_default(trace.clone(), || {
            assert_eq!(get_git_status(&ignored, repo.path()), GitStatus::Ignored);
        });
        let events = trace.events.lock().expect("trace lock");
        assert_eq!(
            events
                .iter()
                .filter(|message| message.as_str() == "git_status_single")
                .count(),
            1
        );
    }

    #[test]
    fn git_status_of_an_unreadable_checkout_is_unreadable_not_clean() {
        let repo = tempfile::tempdir().expect("fixture");
        crate::test_support::init_git_repo(repo.path());
        std::fs::create_dir(repo.path().join("member")).expect("member");
        std::fs::write(repo.path().join(".git/HEAD"), "garbage\n").expect("corrupt HEAD");
        assert_eq!(
            get_git_status(repo.path(), repo.path()),
            GitStatus::Unreadable
        );
        assert_eq!(
            get_git_status(&repo.path().join("member"), repo.path()),
            GitStatus::Unreadable
        );
    }

    #[test]
    fn unborn_checkout_has_no_local_comparison_even_with_a_named_main() {
        let repo = tempfile::tempdir().expect("fixture");
        assert!(
            std::process::Command::new(crate::test_support::git_binary())
                .arg("init")
                .current_dir(repo.path())
                .output()
                .expect("init")
                .status
                .success()
        );
        for branch in [
            ResolvedBranch::Named("main".into()),
            ResolvedBranch::NotFound,
        ] {
            let info = CheckoutInfo::get(repo.path(), &branch).expect("checkout");
            assert_eq!(info.head, HeadState::Unborn);
            assert_eq!(info.ahead_behind_local, Comparison::NotCompared);
        }
    }

    #[test]
    fn value_reads_of_an_unreadable_checkout_are_unreadable() {
        let corrupt = tempfile::tempdir().expect("fixture");
        crate::test_support::init_git_repo(corrupt.path());
        std::fs::write(corrupt.path().join(".git/HEAD"), "garbage\n").expect("corrupt HEAD");
        let info = CheckoutInfo::get(corrupt.path(), &ResolvedBranch::Named("main".into()))
            .expect("checkout");
        assert_eq!(info.status, GitStatus::Unreadable);
        assert_eq!(info.last_commit, CommitDate::Unreadable);
        assert_eq!(info.primary_tracked_ref, TrackedRef::Unreadable);
        assert_eq!(info.ahead_behind_local, Comparison::Unreadable);
        assert_eq!(
            super::super::repo::get_first_commit(corrupt.path()),
            CommitDate::Unreadable
        );
        let repo = RepoInfo::get(corrupt.path()).expect("repository");
        assert!(matches!(
            repo.remotes,
            crate::project::RemoteList::Unreadable
        ));
        assert_eq!(repo.origin_kind(), crate::project::GitOrigin::Unreadable);
        assert_eq!(repo.local_main_branch, ResolvedBranch::Unreadable);

        let unborn = tempfile::tempdir().expect("fixture");
        assert!(
            std::process::Command::new(crate::test_support::git_binary())
                .arg("init")
                .current_dir(unborn.path())
                .output()
                .expect("init")
                .status
                .success()
        );
        let info = CheckoutInfo::get(unborn.path(), &ResolvedBranch::NotFound).expect("checkout");
        assert_eq!(info.last_commit, CommitDate::NoCommits);
        assert_eq!(info.primary_tracked_ref, TrackedRef::Untracked);
        assert_eq!(
            super::super::repo::get_first_commit(unborn.path()),
            CommitDate::NoCommits
        );
    }

    #[test]
    fn primary_head_failure_is_unreadable_but_unborn_is_not_compared() {
        let checkout = tempfile::tempdir().expect("fixture");
        crate::test_support::init_git_repo(checkout.path());
        let primary = tempfile::tempdir().expect("fixture");
        assert!(
            std::process::Command::new(crate::test_support::git_binary())
                .arg("init")
                .current_dir(primary.path())
                .output()
                .expect("init")
                .status
                .success()
        );
        assert_eq!(
            worktree_ahead_behind_primary(checkout.path(), primary.path()),
            Comparison::NotCompared
        );
        std::fs::write(primary.path().join(".git/HEAD"), "garbage\n").expect("corrupt HEAD");
        assert_eq!(
            worktree_ahead_behind_primary(checkout.path(), primary.path()),
            Comparison::Unreadable
        );
    }

    #[test]
    fn missing_comparison_ref_is_an_error() {
        let repo = tempfile::tempdir().expect("fixture");
        crate::test_support::init_git_repo(repo.path());
        assert!(parse_ahead_behind(repo.path(), "HEAD...nope", "missing_ref_test").is_err());
    }

    #[test]
    fn corrupt_bisect_refs_report_unreadable_progress() {
        let repo = tempfile::tempdir().expect("fixture");
        crate::test_support::init_git_repo(repo.path());
        std::fs::write(repo.path().join(".git/HEAD"), "garbage\n").expect("corrupt HEAD");
        std::fs::write(repo.path().join(".git/BISECT_START"), "main\n").expect("bisect marker");
        assert_eq!(
            bisect_progress(repo.path()),
            Some(BisectProgress::Unreadable)
        );
    }
}
