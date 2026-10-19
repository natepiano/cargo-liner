use std::path::Path;

use serde::Serialize;

use super::checkout::HeadState;
use super::checkout::TrackedRef;
use super::command;
use super::command::GitAnswer;
use super::command::GitReadError;
use super::constants::GIT_ABBREV_REF_ARG;
use super::constants::GIT_FOR_EACH_REF_COMMAND;
use super::constants::GIT_FORMAT_UPSTREAM_SHORT_ARG;
use super::constants::GIT_HEAD;
use super::constants::GIT_LOCAL_BRANCH_REF_PREFIX;
use super::constants::GIT_ORIGIN_HEAD_REF;
use super::constants::GIT_QUIET_ARG;
use super::constants::GIT_QUIET_SHORT_ARG;
use super::constants::GIT_REMOTE_HEAD_REF_SUFFIX;
use super::constants::GIT_REMOTE_ORIGIN_PREFIX;
use super::constants::GIT_REMOTE_REF_PREFIX;
use super::constants::GIT_REV_PARSE_COMMAND;
use super::constants::GIT_SHORT_ARG;
use super::constants::GIT_SHOW_REF_COMMAND;
use super::constants::GIT_SYMBOLIC_REF_COMMAND;
use super::constants::GIT_VERIFY_ARG;
use crate::config::CargoPortConfig;

/// What a successful ref-presence query answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RefPresence {
    Present,
    Absent,
}

/// A branch lookup distinguishes absence from a failed git read.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub(crate) enum ResolvedBranch {
    Named(String),
    #[default]
    NotFound,
    Unreadable,
}

pub(super) fn get_current_branch(repo_root: &Path) -> Option<String> {
    command::git_output_logged(
        repo_root,
        "rev_parse_head",
        &[GIT_REV_PARSE_COMMAND, GIT_ABBREV_REF_ARG, GIT_HEAD],
    )
    .ok()
    .and_then(|output| {
        let name = output.trimmed();
        (!name.is_empty()).then(|| name.to_string())
    })
}

pub(super) fn get_upstream_branch(repo_root: &Path, head: &HeadState) -> TrackedRef {
    match head {
        HeadState::Unreadable => TrackedRef::Unreadable,
        HeadState::Unborn | HeadState::Detached { .. } => TrackedRef::Untracked,
        HeadState::Branch(name) => {
            let reference = format!("{GIT_LOCAL_BRANCH_REF_PREFIX}{name}");
            match command::git_output_logged(
                repo_root,
                "upstream",
                &[
                    GIT_FOR_EACH_REF_COMMAND,
                    GIT_FORMAT_UPSTREAM_SHORT_ARG,
                    &reference,
                ],
            ) {
                Ok(output) if output.trimmed().is_empty() => TrackedRef::Untracked,
                Ok(output) => TrackedRef::Ref(output.trimmed().to_string()),
                Err(_) => TrackedRef::Unreadable,
            }
        },
    }
}

/// Resolve the repo's default branch from origin/HEAD.
pub(super) fn get_default_branch(repo_root: &Path) -> ResolvedBranch {
    match command::git_answer_logged(
        repo_root,
        "symbolic_ref_origin_head",
        &[
            GIT_SYMBOLIC_REF_COMMAND,
            GIT_QUIET_SHORT_ARG,
            GIT_SHORT_ARG,
            GIT_ORIGIN_HEAD_REF,
        ],
    ) {
        Ok(GitAnswer::Yes(output)) => output
            .trimmed()
            .strip_prefix(GIT_REMOTE_ORIGIN_PREFIX)
            .filter(|name| !name.is_empty())
            .map_or(ResolvedBranch::NotFound, |name| {
                ResolvedBranch::Named(name.to_string())
            }),
        Ok(GitAnswer::No) => ResolvedBranch::NotFound,
        Err(_) => ResolvedBranch::Unreadable,
    }
}

/// Try, in order: the checkout upstream on this remote, the remote HEAD,
/// the repository default branch, the current branch, the configured main
/// branch, and the configured alternative primary branches.
/// Stop on unreadable evidence; only a documented negative advances to
/// the next tracked-ref candidate.
pub(super) fn resolve_tracked_ref(
    repo_root: &Path,
    remote_name: &str,
    current_upstream: &TrackedRef,
    default_branch: &ResolvedBranch,
    current_branch: Option<&str>,
    cargo_port_config: &CargoPortConfig,
) -> TrackedRef {
    try_resolve_tracked_ref(
        repo_root,
        remote_name,
        current_upstream,
        default_branch,
        current_branch,
        cargo_port_config,
    )
    .unwrap_or(TrackedRef::Unreadable)
}

fn try_resolve_tracked_ref(
    repo_root: &Path,
    remote_name: &str,
    current_upstream: &TrackedRef,
    default_branch: &ResolvedBranch,
    current_branch: Option<&str>,
    cargo_port_config: &CargoPortConfig,
) -> Result<TrackedRef, GitReadError> {
    let prefix = format!("{remote_name}/");
    match current_upstream {
        TrackedRef::Unreadable => return Ok(TrackedRef::Unreadable),
        TrackedRef::Ref(name) if name.starts_with(&prefix) => {
            return Ok(TrackedRef::Ref(name.clone()));
        },
        TrackedRef::Ref(_) | TrackedRef::Untracked => {},
    }
    let remote_head = format!("{GIT_REMOTE_REF_PREFIX}{remote_name}{GIT_REMOTE_HEAD_REF_SUFFIX}");
    match command::git_answer_logged(
        repo_root,
        &format!("symbolic_ref_{remote_name}_head"),
        &[
            GIT_SYMBOLIC_REF_COMMAND,
            GIT_QUIET_SHORT_ARG,
            GIT_SHORT_ARG,
            &remote_head,
        ],
    )? {
        GitAnswer::Yes(output) if output.trimmed().starts_with(&prefix) => {
            return Ok(TrackedRef::Ref(output.trimmed().to_string()));
        },
        GitAnswer::Yes(_) | GitAnswer::No => {},
    }
    match default_branch {
        ResolvedBranch::Unreadable => return Ok(TrackedRef::Unreadable),
        ResolvedBranch::Named(name) => {
            if remote_ref_exists(repo_root, remote_name, name)? == RefPresence::Present {
                return Ok(TrackedRef::Ref(format!("{prefix}{name}")));
            }
        },
        ResolvedBranch::NotFound => {},
    }
    for name in current_branch
        .into_iter()
        .chain(std::iter::once(cargo_port_config.tui.main_branch.as_str()))
        .chain(
            cargo_port_config
                .tui
                .other_primary_branches
                .iter()
                .map(String::as_str),
        )
    {
        if remote_ref_exists(repo_root, remote_name, name)? == RefPresence::Present {
            return Ok(TrackedRef::Ref(format!("{prefix}{name}")));
        }
    }
    Ok(TrackedRef::Untracked)
}

fn remote_ref_exists(
    repo_root: &Path,
    remote_name: &str,
    branch: &str,
) -> Result<RefPresence, GitReadError> {
    ref_presence(
        repo_root,
        &format!("{GIT_REMOTE_REF_PREFIX}{remote_name}/{branch}"),
    )
}

/// The caller reads the configuration once for all its probes.
pub(super) fn resolve_local_main_branch(
    repo_root: &Path,
    config: &CargoPortConfig,
) -> ResolvedBranch {
    for name in std::iter::once(config.tui.main_branch.as_str())
        .chain(config.tui.other_primary_branches.iter().map(String::as_str))
    {
        match local_branch_exists(repo_root, name) {
            Ok(RefPresence::Present) => return ResolvedBranch::Named(name.to_string()),
            Ok(RefPresence::Absent) => {},
            Err(_) => return ResolvedBranch::Unreadable,
        }
    }
    ResolvedBranch::NotFound
}

fn local_branch_exists(repo_root: &Path, branch: &str) -> Result<RefPresence, GitReadError> {
    ref_presence(repo_root, &format!("{GIT_LOCAL_BRANCH_REF_PREFIX}{branch}"))
}

fn ref_presence(repo_root: &Path, reference: &str) -> Result<RefPresence, GitReadError> {
    command::git_answer_logged(
        repo_root,
        "show_ref",
        &[
            GIT_SHOW_REF_COMMAND,
            GIT_VERIFY_ARG,
            GIT_QUIET_ARG,
            reference,
        ],
    )
    .map(|answer| match answer {
        GitAnswer::Yes(_) => RefPresence::Present,
        GitAnswer::No => RefPresence::Absent,
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
    fn existence_checks_tell_absent_from_unreadable() {
        let tmp = tempfile::tempdir().expect("create refs fixture");
        crate::test_support::init_git_repo(tmp.path());
        assert_eq!(
            local_branch_exists(tmp.path(), "nope").expect("read missing local ref"),
            RefPresence::Absent
        );
        assert_eq!(
            remote_ref_exists(tmp.path(), "origin", "main").expect("read missing remote ref"),
            RefPresence::Absent
        );
        let config = CargoPortConfig::default();
        assert_eq!(
            resolve_tracked_ref(
                tmp.path(),
                "origin",
                &TrackedRef::Untracked,
                &ResolvedBranch::NotFound,
                None,
                &config
            ),
            TrackedRef::Untracked
        );
        assert_eq!(get_default_branch(tmp.path()), ResolvedBranch::NotFound);
        std::fs::write(tmp.path().join(".git/HEAD"), "garbage\n").expect("corrupt HEAD");
        assert!(local_branch_exists(tmp.path(), "nope").is_err());
        assert!(remote_ref_exists(tmp.path(), "origin", "main").is_err());
        assert_eq!(
            resolve_tracked_ref(
                tmp.path(),
                "origin",
                &TrackedRef::Untracked,
                &ResolvedBranch::NotFound,
                None,
                &config
            ),
            TrackedRef::Unreadable
        );
        assert_eq!(get_default_branch(tmp.path()), ResolvedBranch::Unreadable);
    }
}
