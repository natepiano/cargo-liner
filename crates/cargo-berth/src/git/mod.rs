//! The small git subprocess surface required by the ledger.

mod command;
mod constants;
mod refs;

use std::collections::HashMap;
use std::collections::HashSet;
use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;
use std::fmt::Write;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::process::Output;
use std::str::FromStr;
use std::string::FromUtf8Error;

use command::GitCommandExecution;
use command::git_output;
use command::git_output_dynamic;
use command::git_output_dynamic_with_environment;
use command::git_output_dynamic_with_environment_and_input;
use command::git_output_dynamic_with_input;
use constants::GIT_ADDED_STATUS;
use constants::GIT_ANCESTOR_RANGE_INFIX;
use constants::GIT_BATCH_CHECK_ARG;
use constants::GIT_CAT_FILE_COMMAND;
use constants::GIT_CHERRY_MARK_ARG;
use constants::GIT_COMMIT_PEEL_SUFFIX;
use constants::GIT_COMMON_DIRECTORY_ARG;
use constants::GIT_COUNT_ARG;
use constants::GIT_DELETED_STATUS;
use constants::GIT_DIFF_COMMAND;
use constants::GIT_EQUIVALENT_COMMIT_MARK;
use constants::GIT_EXCLUDE_REVISION_PREFIX;
use constants::GIT_EXISTS_ARG;
use constants::GIT_FIRST_PARENT_ANCESTOR_INFIX;
use constants::GIT_FIRST_PARENT_ARG;
use constants::GIT_FOR_EACH_REF_COMMAND;
use constants::GIT_FULL_REF_FORMAT_ARG;
use constants::GIT_HEAD_REVISION;
use constants::GIT_HOOKS_PATH;
use constants::GIT_IGNORE_MISSING_ARG;
use constants::GIT_INDEX_FILE_ENV;
use constants::GIT_INDEX_INFO_ARG;
use constants::GIT_INDEX_REMOVAL_RECORD_PREFIX;
use constants::GIT_IS_ANCESTOR_ARG;
use constants::GIT_LEFT_RIGHT_ARG;
use constants::GIT_LITERAL_TOP_PATHSPEC_PREFIX;
use constants::GIT_LOCAL_BRANCH_REF_PREFIX;
use constants::GIT_MAX_COUNT_ARG_PREFIX;
use constants::GIT_MAX_COUNT_ONE_ARG;
use constants::GIT_MERGE_BASE_ARG_PREFIX;
use constants::GIT_MERGE_BASE_COMMAND;
use constants::GIT_MERGE_TREE_CLEAN_EXIT_CODE;
use constants::GIT_MERGE_TREE_COMMAND;
use constants::GIT_MERGE_TREE_CONFLICT_EXIT_CODE;
use constants::GIT_MISSING_OBJECT_SUFFIX;
use constants::GIT_MODIFIED_STATUS;
use constants::GIT_NAME_ONLY_ARG;
use constants::GIT_NO_ABBREV_ARG;
use constants::GIT_NO_MERGE_BASE_EXIT_CODE;
use constants::GIT_NO_MERGES_ARG;
use constants::GIT_NO_RENAMES_ARG;
use constants::GIT_NOT_ANCESTOR_EXIT_CODE;
use constants::GIT_NUL_TERMINATED_ARG;
use constants::GIT_PARENTS_ARG;
use constants::GIT_PATH_ARG;
use constants::GIT_PATH_FORMAT_ABSOLUTE_ARG;
use constants::GIT_PATHSPEC_SEPARATOR;
use constants::GIT_POINTS_AT_ARG_PREFIX;
use constants::GIT_PORCELAIN_ARG;
use constants::GIT_RAW_ARG;
use constants::GIT_READ_TREE_COMMAND;
use constants::GIT_REBASE_APPLY_STATE_PATH;
use constants::GIT_REBASE_MERGE_STATE_PATH;
use constants::GIT_REFLOG_COMMAND;
use constants::GIT_REFLOG_SHOW_ARG;
use constants::GIT_REFLOG_SUBJECT_FORMAT_ARG;
use constants::GIT_REV_LIST_COMMAND;
use constants::GIT_REV_PARSE_COMMAND;
use constants::GIT_SHOW_TOPLEVEL_ARG;
use constants::GIT_STDIN_ARG;
use constants::GIT_SYMMETRIC_RANGE_INFIX;
use constants::GIT_TYPE_CHANGED_STATUS;
use constants::GIT_UPDATE_INDEX_COMMAND;
use constants::GIT_UPDATE_REF_COMMAND;
use constants::GIT_WORKTREE_COMMAND;
use constants::GIT_WORKTREE_LIST_ARG;
use constants::GIT_WRITE_TREE_ARG;
use constants::GIT_WRITE_TREE_COMMAND;
use serde::Deserialize;
use serde::Serialize;
use uuid::Uuid;

use crate::ids::GitObjectId;
use crate::ids::InvalidGitObjectId;
use crate::ids::ReservationId;
use crate::ledger::FullRefName;
use crate::scope::ReservationScopeSet;

/// A worktree's live relationship to the configured trunk.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum AheadBehind {
    /// Both histories share ancestry and have these independent commit counts.
    Counts { ahead: u64, behind: u64 },
    /// Both objects resolve, but their histories have no common ancestor.
    Unrelated,
    /// Git or one required object could not produce a trustworthy comparison.
    Unavailable,
}

/// The parent links needed to compare multiple worktree histories with one revision walk.
struct CommitAncestryGraph {
    parents_by_commit: HashMap<GitObjectId, Vec<GitObjectId>>,
}

impl CommitAncestryGraph {
    fn contains(&self, commit: &GitObjectId) -> bool { self.parents_by_commit.contains_key(commit) }

    fn ancestors_including(&self, tip: &GitObjectId) -> HashSet<GitObjectId> {
        let mut ancestors = HashSet::new();
        let mut pending = vec![tip.clone()];
        while let Some(commit) = pending.pop() {
            if !ancestors.insert(commit.clone()) {
                continue;
            }
            if let Some(parents) = self.parents_by_commit.get(&commit) {
                pending.extend(parents.iter().cloned());
            }
        }
        ancestors
    }
}

impl TryFrom<&str> for CommitAncestryGraph {
    type Error = GitError;

    fn try_from(output: &str) -> Result<Self, Self::Error> {
        let mut parents_by_commit = HashMap::new();
        for line in output.lines() {
            let mut object_ids = line.split_whitespace().map(str::parse::<GitObjectId>);
            let Some(commit) = object_ids
                .next()
                .transpose()
                .map_err(GitError::InvalidObjectId)?
            else {
                continue;
            };
            let parents = object_ids
                .collect::<Result<Vec<_>, _>>()
                .map_err(GitError::InvalidObjectId)?;
            parents_by_commit.insert(commit, parents);
        }
        Ok(Self { parents_by_commit })
    }
}

/// The result of comparing a protected phase's aggregate scoped change with a target history.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ScopedPatchComparison {
    /// The target contains every protected scoped change in one contiguous integration.
    Equivalent,
    /// The target lacks at least one change or cannot prove a contiguous integration.
    Different,
    /// Git could not compare the histories because a required object or result was unavailable.
    Unavailable,
}

enum ScopedPatchComparisonError {
    CommandUnavailable,
    Git(GitError),
}

impl From<GitError> for ScopedPatchComparisonError {
    fn from(error: GitError) -> Self { Self::Git(error) }
}

enum HistoryRelationship {
    Shared,
    Unrelated,
    Unavailable,
}

enum ProtectedScopedChangePaths {
    NoChanges,
    Affected(Vec<String>),
}

enum TargetScopedChangePosition {
    Absent,
    Contiguous,
    Separated,
    Unproven,
}

enum TargetPhaseIntegrationCommits {
    Identified(Vec<GitObjectId>),
    Unresolved,
}

/// The tree built from the protected baseline and only reservation-scoped changes.
enum ScopedProtectedTree {
    /// Git wrote the scoped protected tree with this object id.
    Available(GitObjectId),
    /// Git could not produce a complete scoped protected tree.
    Unavailable,
}

/// Parsed index updates for constructing the scoped protected tree.
enum ScopedProtectedTreeUpdates {
    /// At least one validated in-scope update is ready for `update-index`.
    Available(Vec<u8>),
    /// The scoped raw diff contained no in-scope records.
    Empty,
    /// Git's raw diff did not satisfy the required record format.
    Unavailable,
}

/// Owns the dedicated index path used to construct one scoped protected tree.
struct ScopedProtectedTreeIndex {
    /// The path supplied through `GIT_INDEX_FILE` for every construction command.
    path: PathBuf,
}

impl ScopedProtectedTreeIndex {
    /// Allocate a unique index path and begin owning its cleanup.
    fn new() -> Self {
        Self {
            path: std::env::temp_dir().join(format!(
                "cargo-berth-scoped-protected-tree-{}.index",
                Uuid::now_v7()
            )),
        }
    }

    /// Return the environment entry that directs git to the dedicated index.
    fn environment(&self) -> [(&'static str, &std::ffi::OsStr); 1] {
        [(GIT_INDEX_FILE_ENV, self.path.as_os_str())]
    }
}

impl Drop for ScopedProtectedTreeIndex {
    fn drop(&mut self) { std::mem::drop(fs::remove_file(&self.path)); }
}

/// Resolve the shared administrative directory for a repository worktree.
pub(crate) fn common_directory(repository_root: &Path) -> Result<PathBuf, GitError> {
    let output = git_output(
        repository_root,
        [GIT_REV_PARSE_COMMAND, GIT_COMMON_DIRECTORY_ARG],
    )?;
    if !output.status.success() {
        return Err(GitError::CommandFailed {
            command: GIT_REV_PARSE_COMMAND,
            stderr:  String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }

    let git_directory = String::from_utf8(output.stdout).map_err(GitError::InvalidOutput)?;
    let path = PathBuf::from(git_directory.trim());
    Ok(if path.is_absolute() {
        path
    } else {
        repository_root.join(path)
    })
}

/// Resolve the worktree root for an invocation from anywhere in the repository.
pub(crate) fn repository_root(invocation_directory: &Path) -> Result<PathBuf, GitError> {
    let output = git_output(
        invocation_directory,
        [GIT_REV_PARSE_COMMAND, GIT_SHOW_TOPLEVEL_ARG],
    )?;
    if !output.status.success() {
        return Err(GitError::CommandFailed {
            command: GIT_REV_PARSE_COMMAND,
            stderr:  String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }

    let repository_root = String::from_utf8(output.stdout).map_err(GitError::InvalidOutput)?;
    let path = PathBuf::from(repository_root.trim());
    Ok(if path.is_absolute() {
        path
    } else {
        invocation_directory.join(path)
    })
}

/// Resolve the hook directory Git uses after applying `core.hooksPath`.
pub(crate) fn hooks_directory(repository_root: &Path) -> Result<PathBuf, GitError> {
    let output = git_output(
        repository_root,
        [
            GIT_REV_PARSE_COMMAND,
            GIT_PATH_FORMAT_ABSOLUTE_ARG,
            GIT_PATH_ARG,
            GIT_HOOKS_PATH,
        ],
    )?;
    if !output.status.success() {
        return Err(GitError::CommandFailed {
            command: GIT_REV_PARSE_COMMAND,
            stderr:  String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    let hooks_directory = String::from_utf8(output.stdout).map_err(GitError::InvalidOutput)?;
    Ok(PathBuf::from(hooks_directory.trim()))
}

/// Read the full object id currently named by `HEAD`.
pub(crate) fn head_object_id(repository_root: &Path) -> Result<GitObjectId, GitError> {
    object_id(repository_root, GIT_HEAD_REVISION)
}

/// Read the full object id currently named by a local branch.
pub(crate) fn branch_object_id(
    repository_root: &Path,
    branch: &str,
) -> Result<GitObjectId, GitError> {
    object_id(
        repository_root,
        &format!("{GIT_LOCAL_BRANCH_REF_PREFIX}{branch}"),
    )
}

/// The reflog-proven replacement refs found at one deleted local branch's object tip.
pub(crate) enum LocalBranchReplacementTipMatches {
    /// No local branch at the object records a rename from the deleted branch.
    NoMatches,
    /// Exactly one local branch at the object records the rename.
    ExactlyOne(FullRefName),
    /// More than one local branch at the object records the rename.
    MultipleMatches,
}

/// Whether a local branch's newest reflog entry proves it replaced a deleted branch.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LocalBranchRenameProof {
    /// The newest reflog entry records the candidate's rename from the deleted branch.
    Recorded,
    /// The candidate has no matching newest reflog entry.
    NotRecorded,
}

/// Find whether exactly one local branch at `tip` has proof it replaced the deleted branch.
pub(crate) fn local_branch_replacement_tip_matches(
    repository_root: &Path,
    tip: &GitObjectId,
    deleted_reference: &FullRefName,
) -> Result<LocalBranchReplacementTipMatches, GitError> {
    let arguments = vec![
        GIT_FOR_EACH_REF_COMMAND.to_owned(),
        GIT_FULL_REF_FORMAT_ARG.to_owned(),
        format!("{GIT_POINTS_AT_ARG_PREFIX}{tip}"),
        GIT_LOCAL_BRANCH_REF_PREFIX.to_owned(),
    ];
    let output = git_output_dynamic(repository_root, &arguments)?;
    if !output.status.success() {
        return Err(GitError::CommandFailed {
            command: GIT_FOR_EACH_REF_COMMAND,
            stderr:  String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    let references = String::from_utf8(output.stdout)
        .map_err(GitError::InvalidOutput)?
        .lines()
        .map(|reference| {
            reference
                .parse::<FullRefName>()
                .map_err(|_| GitError::InvalidReferenceName {
                    reference: reference.to_owned(),
                })
        })
        .filter(|reference| {
            reference
                .as_ref()
                .map_or(true, |reference| reference != deleted_reference)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut proven_replacements = LocalBranchReplacementTipMatches::NoMatches;
    for reference in references {
        match local_branch_rename_proof(repository_root, deleted_reference, &reference)? {
            LocalBranchRenameProof::Recorded => match proven_replacements {
                LocalBranchReplacementTipMatches::NoMatches => {
                    proven_replacements = LocalBranchReplacementTipMatches::ExactlyOne(reference);
                },
                LocalBranchReplacementTipMatches::ExactlyOne(_)
                | LocalBranchReplacementTipMatches::MultipleMatches => {
                    return Ok(LocalBranchReplacementTipMatches::MultipleMatches);
                },
            },
            LocalBranchRenameProof::NotRecorded => {},
        }
    }
    Ok(proven_replacements)
}

/// Read whether `candidate_reference` records a rename from `deleted_reference`.
pub(crate) fn local_branch_rename_proof(
    repository_root: &Path,
    deleted_reference: &FullRefName,
    candidate_reference: &FullRefName,
) -> Result<LocalBranchRenameProof, GitError> {
    let candidate_reference = candidate_reference.to_string();
    let output = git_output(
        repository_root,
        [
            GIT_REFLOG_COMMAND,
            GIT_REFLOG_SHOW_ARG,
            GIT_MAX_COUNT_ONE_ARG,
            GIT_REFLOG_SUBJECT_FORMAT_ARG,
            &candidate_reference,
        ],
    )?;
    if !output.status.success() {
        return Err(GitError::CommandFailed {
            command: GIT_REFLOG_COMMAND,
            stderr:  String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    let subject = String::from_utf8(output.stdout).map_err(GitError::InvalidOutput)?;
    let expected_subject = format!("Branch: renamed {deleted_reference} to {candidate_reference}");
    Ok(match subject.lines().next() {
        Some(subject) if subject == expected_subject => LocalBranchRenameProof::Recorded,
        Some(_) | None => LocalBranchRenameProof::NotRecorded,
    })
}

/// Return every commit that would become reachable from `proposed` but not `previous`.
pub(crate) fn newly_reachable_commits(
    repository_root: &Path,
    previous: &GitObjectId,
    proposed: &GitObjectId,
) -> Result<Vec<GitObjectId>, GitError> {
    let arguments = vec![
        GIT_REV_LIST_COMMAND.to_owned(),
        proposed.to_string(),
        format!("{GIT_EXCLUDE_REVISION_PREFIX}{previous}"),
    ];
    let output = git_output_dynamic(repository_root, &arguments)?;
    if !output.status.success() {
        return Err(GitError::CommandFailed {
            command: GIT_REV_LIST_COMMAND,
            stderr:  String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    String::from_utf8(output.stdout)
        .map_err(GitError::InvalidOutput)?
        .lines()
        .map(|line| line.parse().map_err(GitError::InvalidObjectId))
        .collect()
}

/// Report whether git is part-way through replaying commits onto a moved base.
///
/// A rebase moves the worktree onto its new base and replays each commit from there, and
/// git runs `post-commit` for every one of them. Until the branch reference moves at the
/// end, the phase's anchor still describes the history the branch is being lifted off, so
/// a comparison taken now reads the new base's commits as this phase's work. There is no
/// drift answer worth giving about a history that is still being written.
pub(crate) fn rewrite_in_progress(repository_root: &Path) -> Result<bool, GitError> {
    for state_path in [GIT_REBASE_MERGE_STATE_PATH, GIT_REBASE_APPLY_STATE_PATH] {
        if repository_path(repository_root, state_path)?.exists() {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Resolve one of git's administrative paths for the invoking worktree.
fn repository_path(repository_root: &Path, name: &str) -> Result<PathBuf, GitError> {
    let output = git_output(
        repository_root,
        [
            GIT_REV_PARSE_COMMAND,
            GIT_PATH_FORMAT_ABSOLUTE_ARG,
            GIT_PATH_ARG,
            name,
        ],
    )?;
    if !output.status.success() {
        return Err(GitError::CommandFailed {
            command: GIT_REV_PARSE_COMMAND,
            stderr:  String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    Ok(PathBuf::from(
        String::from_utf8(output.stdout)
            .map_err(GitError::InvalidOutput)?
            .trim(),
    ))
}

/// Locate a rewritten branch's replayed phase commits and return the commit beneath them.
///
/// A rebase leaves `phase_start` describing a history the branch no longer has, so
/// `<phase_start>..HEAD` stops meaning "the commits this phase authored" and starts
/// including everything the new base brought in. The phase's own commits survive the
/// rewrite as patch-equivalents sitting at the tip, so the replacement anchor is the
/// commit directly beneath the last of them.
///
/// Counting the phase's commits is not enough, and neither is testing patch identity on
/// its own. A rebase drops a commit whose patch already reached the new base, and the
/// upstream commit carrying that same patch is itself an equivalent, so both a count and
/// a bare equivalence test read a dropped commit exactly like a replayed one. Position
/// separates them: only the replayed commits are contiguous at the tip.
pub(crate) fn rewritten_phase_anchor(
    repository_root: &Path,
    phase_start: &GitObjectId,
    previous_tip: &GitObjectId,
    proposed_tip: &GitObjectId,
) -> Result<GitObjectId, GitError> {
    let phase_commit_count = commit_count(
        repository_root,
        &format!("{phase_start}{GIT_ANCESTOR_RANGE_INFIX}{previous_tip}"),
    )?;
    if phase_commit_count == 0 {
        return Ok(proposed_tip.clone());
    }
    let equivalents =
        phase_equivalent_commits(repository_root, phase_start, previous_tip, proposed_tip)?;
    let replayed = first_parent_commits(repository_root, proposed_tip, phase_commit_count)?
        .iter()
        .take_while(|commit| equivalents.contains(commit))
        .count();
    object_id(
        repository_root,
        &format!("{proposed_tip}{GIT_FIRST_PARENT_ANCESTOR_INFIX}{replayed}"),
    )
}

fn scoped_patch_command_output(
    command_execution: GitCommandExecution,
) -> Result<Output, ScopedPatchComparisonError> {
    match command_execution {
        GitCommandExecution::Completed(output) => Ok(output),
        GitCommandExecution::CouldNotRun => Err(ScopedPatchComparisonError::CommandUnavailable),
    }
}

/// Compare one protected phase's aggregate scoped change with a target history.
///
/// `phase_start_head` excludes earlier branch work from the protected side. The first query
/// submits every scope together and expands tree scopes only to paths changed by the protected
/// phase. The target commits carrying those changes must occupy one contiguous first-parent
/// interval. A three-way replay uses `phase_start_head` as its explicit merge base after removing
/// every protected change outside the reservation scopes. The replay is equivalent only when its
/// complete tree matches the target.
pub(crate) fn scoped_patch_equivalence(
    repository_root: &Path,
    phase_start_head: &GitObjectId,
    scopes: &ReservationScopeSet,
    protected_tip: &GitObjectId,
    target: &GitObjectId,
) -> Result<ScopedPatchComparison, GitError> {
    match compare_scoped_patch(
        repository_root,
        phase_start_head,
        scopes,
        protected_tip,
        target,
    ) {
        Ok(scoped_patch_comparison) => Ok(scoped_patch_comparison),
        Err(ScopedPatchComparisonError::CommandUnavailable) => {
            Ok(ScopedPatchComparison::Unavailable)
        },
        Err(ScopedPatchComparisonError::Git(error)) => Err(error),
    }
}

fn compare_scoped_patch(
    repository_root: &Path,
    phase_start_head: &GitObjectId,
    scopes: &ReservationScopeSet,
    protected_tip: &GitObjectId,
    target: &GitObjectId,
) -> Result<ScopedPatchComparison, ScopedPatchComparisonError> {
    match history_relationship(repository_root, phase_start_head, target)? {
        HistoryRelationship::Shared => {},
        HistoryRelationship::Unrelated => return Ok(ScopedPatchComparison::Different),
        HistoryRelationship::Unavailable => return Ok(ScopedPatchComparison::Unavailable),
    }

    let protected_scoped_change_paths =
        protected_scoped_change_paths(repository_root, phase_start_head, scopes, protected_tip)?;
    let ProtectedScopedChangePaths::Affected(affected_paths) = protected_scoped_change_paths else {
        return Ok(ScopedPatchComparison::Different);
    };

    match target_scoped_change_position(
        repository_root,
        phase_start_head,
        protected_tip,
        target,
        &affected_paths,
    )? {
        TargetScopedChangePosition::Contiguous => {},
        TargetScopedChangePosition::Absent
        | TargetScopedChangePosition::Separated
        | TargetScopedChangePosition::Unproven => {
            return Ok(ScopedPatchComparison::Different);
        },
    }

    target_contains_protected_scoped_change(
        repository_root,
        phase_start_head,
        protected_tip,
        target,
        scopes,
    )
}

fn history_relationship(
    repository_root: &Path,
    left: &GitObjectId,
    right: &GitObjectId,
) -> Result<HistoryRelationship, ScopedPatchComparisonError> {
    let left = left.to_string();
    let right = right.to_string();
    let output = scoped_patch_command_output(
        git_output(repository_root, [GIT_MERGE_BASE_COMMAND, &left, &right]).into(),
    )?;
    if output.status.success() {
        Ok(HistoryRelationship::Shared)
    } else if output.status.code() == Some(GIT_NO_MERGE_BASE_EXIT_CODE) {
        Ok(HistoryRelationship::Unrelated)
    } else {
        Ok(HistoryRelationship::Unavailable)
    }
}

fn target_contains_protected_scoped_change(
    repository_root: &Path,
    phase_start_head: &GitObjectId,
    protected_tip: &GitObjectId,
    target: &GitObjectId,
    scopes: &ReservationScopeSet,
) -> Result<ScopedPatchComparison, ScopedPatchComparisonError> {
    let ScopedProtectedTree::Available(scoped_protected_tree) =
        scoped_protected_tree(repository_root, phase_start_head, protected_tip, scopes)?
    else {
        return Ok(ScopedPatchComparison::Unavailable);
    };
    let replay_arguments = [
        GIT_MERGE_TREE_COMMAND.to_owned(),
        GIT_WRITE_TREE_ARG.to_owned(),
        GIT_NUL_TERMINATED_ARG.to_owned(),
        format!("{GIT_MERGE_BASE_ARG_PREFIX}{phase_start_head}"),
        target.to_string(),
        scoped_protected_tree.to_string(),
    ];
    let replay_output =
        scoped_patch_command_output(git_output_dynamic(repository_root, &replay_arguments).into())?;
    match replay_output.status.code() {
        Some(GIT_MERGE_TREE_CLEAN_EXIT_CODE) => {},
        Some(GIT_MERGE_TREE_CONFLICT_EXIT_CODE) => {
            return Ok(ScopedPatchComparison::Different);
        },
        _ => return Ok(ScopedPatchComparison::Unavailable),
    }
    let Some(replayed_tree) = replay_output.stdout.split(|byte| *byte == b'\0').next() else {
        return Ok(ScopedPatchComparison::Unavailable);
    };
    let Ok(replayed_tree) = str::from_utf8(replayed_tree) else {
        return Ok(ScopedPatchComparison::Unavailable);
    };
    let Ok(replayed_tree) = replayed_tree.parse::<GitObjectId>() else {
        return Ok(ScopedPatchComparison::Unavailable);
    };

    let diff_arguments = [
        GIT_DIFF_COMMAND.to_owned(),
        GIT_NAME_ONLY_ARG.to_owned(),
        replayed_tree.to_string(),
        target.to_string(),
    ];
    let diff_output =
        scoped_patch_command_output(git_output_dynamic(repository_root, &diff_arguments).into())?;
    if !diff_output.status.success() {
        return Err(GitError::CommandFailed {
            command: GIT_DIFF_COMMAND,
            stderr:  String::from_utf8_lossy(&diff_output.stderr)
                .trim()
                .to_owned(),
        }
        .into());
    }
    Ok(if diff_output.stdout.is_empty() {
        ScopedPatchComparison::Equivalent
    } else {
        ScopedPatchComparison::Different
    })
}

/// Build the protected baseline tree with only changes covered by `scopes` applied.
fn scoped_protected_tree(
    repository_root: &Path,
    phase_start_head: &GitObjectId,
    protected_tip: &GitObjectId,
    scopes: &ReservationScopeSet,
) -> Result<ScopedProtectedTree, ScopedPatchComparisonError> {
    let scoped_protected_tree_index = ScopedProtectedTreeIndex::new();
    let environment = scoped_protected_tree_index.environment();

    let read_tree_arguments = [
        GIT_READ_TREE_COMMAND.to_owned(),
        phase_start_head.to_string(),
    ];
    let read_tree_output = scoped_patch_command_output(
        git_output_dynamic_with_environment(repository_root, &read_tree_arguments, &environment)
            .into(),
    )?;
    if !read_tree_output.status.success() {
        return Ok(ScopedProtectedTree::Unavailable);
    }

    let mut diff_arguments = vec![
        GIT_DIFF_COMMAND.to_owned(),
        GIT_RAW_ARG.to_owned(),
        GIT_NUL_TERMINATED_ARG.to_owned(),
        GIT_NO_ABBREV_ARG.to_owned(),
        GIT_NO_RENAMES_ARG.to_owned(),
        phase_start_head.to_string(),
        protected_tip.to_string(),
        GIT_PATHSPEC_SEPARATOR.to_owned(),
    ];
    diff_arguments.extend(
        scopes
            .as_slice()
            .iter()
            .map(|scope| format!("{GIT_LITERAL_TOP_PATHSPEC_PREFIX}{}", scope.path)),
    );
    let diff_output = scoped_patch_command_output(
        git_output_dynamic_with_environment(repository_root, &diff_arguments, &environment).into(),
    )?;
    if !diff_output.status.success() {
        return Ok(ScopedProtectedTree::Unavailable);
    }
    let ScopedProtectedTreeUpdates::Available(index_updates) =
        scoped_protected_tree_updates(&diff_output.stdout, scopes)
    else {
        return Ok(ScopedProtectedTree::Unavailable);
    };

    let update_index_arguments = [
        GIT_UPDATE_INDEX_COMMAND.to_owned(),
        GIT_NUL_TERMINATED_ARG.to_owned(),
        GIT_INDEX_INFO_ARG.to_owned(),
    ];
    let update_index_output = scoped_patch_command_output(
        git_output_dynamic_with_environment_and_input(
            repository_root,
            &update_index_arguments,
            &environment,
            &index_updates,
        )
        .into(),
    )?;
    if !update_index_output.status.success() {
        return Ok(ScopedProtectedTree::Unavailable);
    }

    let write_tree_arguments = [GIT_WRITE_TREE_COMMAND.to_owned()];
    let write_tree_output = scoped_patch_command_output(
        git_output_dynamic_with_environment(repository_root, &write_tree_arguments, &environment)
            .into(),
    )?;
    if !write_tree_output.status.success() {
        return Ok(ScopedProtectedTree::Unavailable);
    }
    let Ok(scoped_protected_tree) = str::from_utf8(&write_tree_output.stdout) else {
        return Ok(ScopedProtectedTree::Unavailable);
    };
    let Ok(scoped_protected_tree) = scoped_protected_tree.trim().parse::<GitObjectId>() else {
        return Ok(ScopedProtectedTree::Unavailable);
    };
    Ok(ScopedProtectedTree::Available(scoped_protected_tree))
}

/// Convert validated in-scope raw diff records into NUL-delimited index updates.
fn scoped_protected_tree_updates(
    raw_diff: &[u8],
    scopes: &ReservationScopeSet,
) -> ScopedProtectedTreeUpdates {
    let mut raw_fields = raw_diff.split(|byte| *byte == b'\0');
    let mut index_updates = Vec::new();
    loop {
        let Some(metadata) = raw_fields.next() else {
            return ScopedProtectedTreeUpdates::Unavailable;
        };
        if metadata.is_empty() {
            if raw_fields.next().is_some() {
                return ScopedProtectedTreeUpdates::Unavailable;
            }
            return if index_updates.is_empty() {
                ScopedProtectedTreeUpdates::Empty
            } else {
                ScopedProtectedTreeUpdates::Available(index_updates)
            };
        }
        let Some(repository_path) = raw_fields.next() else {
            return ScopedProtectedTreeUpdates::Unavailable;
        };
        if repository_path.is_empty() {
            return ScopedProtectedTreeUpdates::Unavailable;
        }
        let Some(metadata) = metadata.strip_prefix(b":") else {
            return ScopedProtectedTreeUpdates::Unavailable;
        };
        let components = metadata.split(|byte| *byte == b' ').collect::<Vec<_>>();
        let [_, destination_mode, _, destination_object_id, status] = components.as_slice() else {
            return ScopedProtectedTreeUpdates::Unavailable;
        };
        if !scopes.covers_path(repository_path) {
            continue;
        }
        match *status {
            GIT_ADDED_STATUS | GIT_MODIFIED_STATUS | GIT_TYPE_CHANGED_STATUS => {
                index_updates.extend_from_slice(destination_mode);
                index_updates.push(b' ');
                index_updates.extend_from_slice(destination_object_id);
                index_updates.push(b'\t');
            },
            GIT_DELETED_STATUS => {
                index_updates.extend_from_slice(GIT_INDEX_REMOVAL_RECORD_PREFIX);
            },
            _ => return ScopedProtectedTreeUpdates::Unavailable,
        }
        index_updates.extend_from_slice(repository_path);
        index_updates.push(b'\0');
    }
}

fn target_scoped_change_position(
    repository_root: &Path,
    phase_start_head: &GitObjectId,
    protected_tip: &GitObjectId,
    target: &GitObjectId,
    affected_paths: &[String],
) -> Result<TargetScopedChangePosition, ScopedPatchComparisonError> {
    let target_commits = first_parent_commits_after(repository_root, phase_start_head, target)?;
    let target_phase_integration_commits = target_phase_integration_commits(
        repository_root,
        phase_start_head,
        protected_tip,
        target,
        &target_commits,
        affected_paths,
    )?;
    let TargetPhaseIntegrationCommits::Identified(scoped_commits) =
        target_phase_integration_commits
    else {
        return Ok(TargetScopedChangePosition::Unproven);
    };
    if scoped_commits.is_empty() {
        return Ok(TargetScopedChangePosition::Absent);
    }
    let positions = scoped_commits
        .iter()
        .map(|scoped_commit| {
            target_commits
                .iter()
                .position(|target_commit| target_commit == scoped_commit)
                .ok_or_else(|| GitError::ScopedCommitMissingFromTargetWalk {
                    commit: scoped_commit.clone(),
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    if positions.windows(2).all(|pair| pair[1] == pair[0] + 1) {
        Ok(TargetScopedChangePosition::Contiguous)
    } else {
        Ok(TargetScopedChangePosition::Separated)
    }
}

fn target_phase_integration_commits(
    repository_root: &Path,
    phase_start_head: &GitObjectId,
    protected_tip: &GitObjectId,
    target: &GitObjectId,
    target_commits: &[GitObjectId],
    affected_paths: &[String],
) -> Result<TargetPhaseIntegrationCommits, ScopedPatchComparisonError> {
    let protected_scoped_commits = first_parent_commits_after_in_paths(
        repository_root,
        phase_start_head,
        protected_tip,
        affected_paths,
    )?;
    let target_scoped_commits = first_parent_commits_after_in_paths(
        repository_root,
        phase_start_head,
        target,
        affected_paths,
    )?;
    if protected_scoped_commits.is_empty() || target_scoped_commits.is_empty() {
        return Ok(TargetPhaseIntegrationCommits::Identified(Vec::new()));
    }

    let scoped_patch_equivalent_commits = scoped_patch_equivalent_commits(
        repository_root,
        phase_start_head,
        protected_tip,
        target,
        affected_paths,
    )?;
    let mut identified_target_commits = target_commits
        .iter()
        .filter(|target_commit| {
            protected_scoped_commits.contains(target_commit)
                || scoped_patch_equivalent_commits.contains(target_commit)
        })
        .cloned()
        .collect::<Vec<_>>();

    if protected_scoped_commits.iter().all(|protected_commit| {
        target_commits.contains(protected_commit)
            || scoped_patch_equivalent_commits.contains(protected_commit)
    }) && !identified_target_commits.is_empty()
    {
        return Ok(TargetPhaseIntegrationCommits::Identified(
            identified_target_commits,
        ));
    }

    let unmatched_target_commits = target_scoped_commits
        .iter()
        .filter(|target_commit| !identified_target_commits.contains(target_commit))
        .cloned()
        .collect::<Vec<_>>();
    let [unmatched_target_commit] = unmatched_target_commits.as_slice() else {
        return Ok(TargetPhaseIntegrationCommits::Unresolved);
    };
    identified_target_commits.push(unmatched_target_commit.clone());
    let identified_target_commits = target_commits
        .iter()
        .filter(|target_commit| identified_target_commits.contains(target_commit))
        .cloned()
        .collect();
    Ok(TargetPhaseIntegrationCommits::Identified(
        identified_target_commits,
    ))
}

fn scoped_patch_equivalent_commits(
    repository_root: &Path,
    phase_start_head: &GitObjectId,
    protected_tip: &GitObjectId,
    target: &GitObjectId,
    affected_paths: &[String],
) -> Result<Vec<GitObjectId>, ScopedPatchComparisonError> {
    let mut arguments = vec![
        GIT_REV_LIST_COMMAND.to_owned(),
        GIT_CHERRY_MARK_ARG.to_owned(),
        GIT_LEFT_RIGHT_ARG.to_owned(),
        GIT_NO_MERGES_ARG.to_owned(),
        GIT_NO_RENAMES_ARG.to_owned(),
        format!("{protected_tip}{GIT_SYMMETRIC_RANGE_INFIX}{target}"),
        format!("{GIT_EXCLUDE_REVISION_PREFIX}{phase_start_head}"),
        GIT_PATHSPEC_SEPARATOR.to_owned(),
    ];
    arguments.extend(
        affected_paths
            .iter()
            .map(|path| format!("{GIT_LITERAL_TOP_PATHSPEC_PREFIX}{path}")),
    );
    Ok(scoped_rev_list(repository_root, &arguments)?
        .lines()
        .filter_map(|line| line.strip_prefix(GIT_EQUIVALENT_COMMIT_MARK))
        .map(|commit| commit.parse().map_err(GitError::InvalidObjectId))
        .collect::<Result<Vec<_>, _>>()?)
}

fn first_parent_commits_after(
    repository_root: &Path,
    excluded_ancestor: &GitObjectId,
    tip: &GitObjectId,
) -> Result<Vec<GitObjectId>, ScopedPatchComparisonError> {
    let arguments = vec![
        GIT_REV_LIST_COMMAND.to_owned(),
        GIT_FIRST_PARENT_ARG.to_owned(),
        tip.to_string(),
        format!("{GIT_EXCLUDE_REVISION_PREFIX}{excluded_ancestor}"),
    ];
    Ok(scoped_rev_list(repository_root, &arguments)?
        .lines()
        .map(|commit| commit.parse().map_err(GitError::InvalidObjectId))
        .collect::<Result<Vec<_>, _>>()?)
}

fn first_parent_commits_after_in_paths(
    repository_root: &Path,
    excluded_ancestor: &GitObjectId,
    tip: &GitObjectId,
    affected_paths: &[String],
) -> Result<Vec<GitObjectId>, ScopedPatchComparisonError> {
    let mut arguments = vec![
        GIT_REV_LIST_COMMAND.to_owned(),
        GIT_FIRST_PARENT_ARG.to_owned(),
        GIT_NO_RENAMES_ARG.to_owned(),
        tip.to_string(),
        format!("{GIT_EXCLUDE_REVISION_PREFIX}{excluded_ancestor}"),
        GIT_PATHSPEC_SEPARATOR.to_owned(),
    ];
    arguments.extend(
        affected_paths
            .iter()
            .map(|path| format!("{GIT_LITERAL_TOP_PATHSPEC_PREFIX}{path}")),
    );
    Ok(scoped_rev_list(repository_root, &arguments)?
        .lines()
        .map(|commit| commit.parse().map_err(GitError::InvalidObjectId))
        .collect::<Result<Vec<_>, _>>()?)
}

fn protected_scoped_change_paths(
    repository_root: &Path,
    phase_start_head: &GitObjectId,
    scopes: &ReservationScopeSet,
    protected_tip: &GitObjectId,
) -> Result<ProtectedScopedChangePaths, ScopedPatchComparisonError> {
    let mut arguments = vec![
        GIT_DIFF_COMMAND.to_owned(),
        GIT_NAME_ONLY_ARG.to_owned(),
        GIT_NUL_TERMINATED_ARG.to_owned(),
        GIT_NO_RENAMES_ARG.to_owned(),
        phase_start_head.to_string(),
        protected_tip.to_string(),
        GIT_PATHSPEC_SEPARATOR.to_owned(),
    ];
    arguments.extend(
        scopes
            .as_slice()
            .iter()
            .map(|scope| format!("{GIT_LITERAL_TOP_PATHSPEC_PREFIX}{}", scope.path)),
    );
    let output =
        scoped_patch_command_output(git_output_dynamic(repository_root, &arguments).into())?;
    if !output.status.success() {
        return Err(GitError::CommandFailed {
            command: GIT_DIFF_COMMAND,
            stderr:  String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        }
        .into());
    }
    let output_text = String::from_utf8(output.stdout).map_err(GitError::InvalidOutput)?;
    let affected_paths = output_text
        .split('\0')
        .filter(|path| !path.is_empty())
        .filter(|path| scopes.covers_path(path.as_bytes()))
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if affected_paths.is_empty() {
        Ok(ProtectedScopedChangePaths::NoChanges)
    } else {
        Ok(ProtectedScopedChangePaths::Affected(affected_paths))
    }
}

/// Count the commits selected by one revision range.
fn commit_count(repository_root: &Path, range: &str) -> Result<usize, GitError> {
    let arguments = vec![
        GIT_REV_LIST_COMMAND.to_owned(),
        GIT_COUNT_ARG.to_owned(),
        range.to_owned(),
    ];
    let output = rev_list(repository_root, &arguments)?;
    output
        .trim()
        .parse()
        .map_err(|_| GitError::UncountableCommitRange {
            range: range.to_owned(),
        })
}

/// Collect the commits on either side of the rewrite that carry a phase commit's patch.
///
/// Excluding `phase_start` keeps the comparison to this phase's own commits, so an
/// earlier phase sharing the branch is never mistaken for part of this one.
fn phase_equivalent_commits(
    repository_root: &Path,
    phase_start: &GitObjectId,
    previous_tip: &GitObjectId,
    proposed_tip: &GitObjectId,
) -> Result<Vec<GitObjectId>, GitError> {
    let arguments = vec![
        GIT_REV_LIST_COMMAND.to_owned(),
        GIT_CHERRY_MARK_ARG.to_owned(),
        GIT_LEFT_RIGHT_ARG.to_owned(),
        GIT_NO_MERGES_ARG.to_owned(),
        format!("{previous_tip}{GIT_SYMMETRIC_RANGE_INFIX}{proposed_tip}"),
        format!("{GIT_EXCLUDE_REVISION_PREFIX}{phase_start}"),
    ];
    rev_list(repository_root, &arguments)?
        .lines()
        .filter_map(|line| line.strip_prefix(GIT_EQUIVALENT_COMMIT_MARK))
        .map(|commit| commit.parse().map_err(GitError::InvalidObjectId))
        .collect()
}

/// Walk one commit's own line of descent from the tip, taking at most `limit` commits.
fn first_parent_commits(
    repository_root: &Path,
    tip: &GitObjectId,
    limit: usize,
) -> Result<Vec<GitObjectId>, GitError> {
    let arguments = vec![
        GIT_REV_LIST_COMMAND.to_owned(),
        GIT_FIRST_PARENT_ARG.to_owned(),
        format!("{GIT_MAX_COUNT_ARG_PREFIX}{limit}"),
        tip.to_string(),
    ];
    rev_list(repository_root, &arguments)?
        .lines()
        .map(|commit| commit.parse().map_err(GitError::InvalidObjectId))
        .collect()
}

/// Run one `rev-list` invocation and return its standard output.
fn scoped_rev_list(
    repository_root: &Path,
    arguments: &[String],
) -> Result<String, ScopedPatchComparisonError> {
    let output =
        scoped_patch_command_output(git_output_dynamic(repository_root, arguments).into())?;
    if !output.status.success() {
        return Err(GitError::CommandFailed {
            command: GIT_REV_LIST_COMMAND,
            stderr:  String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        }
        .into());
    }
    Ok(String::from_utf8(output.stdout).map_err(GitError::InvalidOutput)?)
}

/// Run one `rev-list` invocation and return its standard output.
fn rev_list(repository_root: &Path, arguments: &[String]) -> Result<String, GitError> {
    let output = git_output_dynamic(repository_root, arguments)?;
    if !output.status.success() {
        return Err(GitError::CommandFailed {
            command: GIT_REV_LIST_COMMAND,
            stderr:  String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    String::from_utf8(output.stdout).map_err(GitError::InvalidOutput)
}

/// Return every commit reachable from a proposed initial trunk object.
pub(crate) fn reachable_commits(
    repository_root: &Path,
    proposed: &GitObjectId,
) -> Result<Vec<GitObjectId>, GitError> {
    let arguments = vec![GIT_REV_LIST_COMMAND.to_owned(), proposed.to_string()];
    let output = git_output_dynamic(repository_root, &arguments)?;
    if !output.status.success() {
        return Err(GitError::CommandFailed {
            command: GIT_REV_LIST_COMMAND,
            stderr:  String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    String::from_utf8(output.stdout)
        .map_err(GitError::InvalidOutput)?
        .lines()
        .map(|line| line.parse().map_err(GitError::InvalidObjectId))
        .collect()
}

/// Atomically move one local branch from the expected old object to a proposed object.
pub(crate) fn update_local_branch(
    repository_root: &Path,
    branch: &str,
    proposed: &GitObjectId,
    expected_previous: &GitObjectId,
) -> Result<(), GitError> {
    match reachability(repository_root, expected_previous, proposed)? {
        Reachability::Ancestor => {},
        Reachability::NotAncestor => {
            return Err(GitError::NonFastForwardBranchUpdate {
                previous: expected_previous.clone(),
                proposed: proposed.clone(),
            });
        },
        Reachability::ObjectUnknown => {
            return Err(GitError::BranchUpdateObjectUnavailable {
                previous: expected_previous.clone(),
                proposed: proposed.clone(),
            });
        },
    }
    let reference = format!("{GIT_LOCAL_BRANCH_REF_PREFIX}{branch}");
    let proposed = proposed.to_string();
    let expected_previous = expected_previous.to_string();
    let output = git_output(
        repository_root,
        [
            GIT_UPDATE_REF_COMMAND,
            &reference,
            &proposed,
            &expected_previous,
        ],
    )?;
    if output.status.success() {
        Ok(())
    } else {
        Err(GitError::CommandFailed {
            command: GIT_UPDATE_REF_COMMAND,
            stderr:  String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        })
    }
}

/// Resolve a revision while treating an ordinary unresolved name as a typed absence.
pub(crate) fn reference_lookup(
    repository_root: &Path,
    revision: &str,
) -> Result<ReferenceLookup, GitError> {
    let output = git_output(repository_root, [GIT_REV_PARSE_COMMAND, revision])?;
    if !output.status.success() {
        return Ok(ReferenceLookup::Missing);
    }
    let object_id = String::from_utf8(output.stdout).map_err(GitError::InvalidOutput)?;
    object_id
        .trim()
        .parse()
        .map(ReferenceLookup::Present)
        .map_err(GitError::InvalidObjectId)
}

/// Return whether git can still read one commit object.
pub(crate) fn commit_is_available(
    repository_root: &Path,
    object_id: &GitObjectId,
) -> Result<bool, GitError> {
    let revision = format!("{object_id}{GIT_COMMIT_PEEL_SUFFIX}");
    let output = git_output(
        repository_root,
        [GIT_CAT_FILE_COMMAND, GIT_EXISTS_ARG, &revision],
    )?;
    Ok(output.status.success())
}

/// Read git's NUL-delimited registered-worktree representation.
pub(crate) fn worktree_list_porcelain(repository_root: &Path) -> Result<Vec<u8>, GitError> {
    let output = git_output(
        repository_root,
        [
            GIT_WORKTREE_COMMAND,
            GIT_WORKTREE_LIST_ARG,
            GIT_PORCELAIN_ARG,
            GIT_NUL_TERMINATED_ARG,
        ],
    )?;
    if !output.status.success() {
        return Err(GitError::CommandFailed {
            command: GIT_WORKTREE_COMMAND,
            stderr:  String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    Ok(output.stdout)
}

/// Compute every worktree head's live relationship to trunk with one git invocation.
pub(crate) fn ahead_behind_for_heads(
    repository_root: &Path,
    trunk: &GitObjectId,
    worktree_heads: &[GitObjectId],
) -> Vec<AheadBehind> {
    if worktree_heads.is_empty() {
        return Vec::new();
    }
    let input =
        std::iter::once(trunk)
            .chain(worktree_heads)
            .fold(String::new(), |mut input, object_id| {
                let _ = writeln!(input, "{object_id}");
                input
            });
    let arguments = [
        GIT_REV_LIST_COMMAND.to_owned(),
        GIT_PARENTS_ARG.to_owned(),
        GIT_IGNORE_MISSING_ARG.to_owned(),
        GIT_STDIN_ARG.to_owned(),
    ];
    let Ok(output) = git_output_dynamic_with_input(repository_root, &arguments, input.as_bytes())
    else {
        return vec![AheadBehind::Unavailable; worktree_heads.len()];
    };
    if !output.status.success() {
        return vec![AheadBehind::Unavailable; worktree_heads.len()];
    }
    let Ok(output) = String::from_utf8(output.stdout) else {
        return vec![AheadBehind::Unavailable; worktree_heads.len()];
    };
    let Ok(commit_ancestry_graph) = CommitAncestryGraph::try_from(output.as_str()) else {
        return vec![AheadBehind::Unavailable; worktree_heads.len()];
    };
    if !commit_ancestry_graph.contains(trunk) {
        return vec![AheadBehind::Unavailable; worktree_heads.len()];
    }
    let trunk_ancestors = commit_ancestry_graph.ancestors_including(trunk);
    worktree_heads
        .iter()
        .map(|worktree_head| {
            if !commit_ancestry_graph.contains(worktree_head) {
                return AheadBehind::Unavailable;
            }
            let worktree_ancestors = commit_ancestry_graph.ancestors_including(worktree_head);
            if trunk_ancestors.is_disjoint(&worktree_ancestors) {
                return AheadBehind::Unrelated;
            }
            let Ok(ahead) = u64::try_from(worktree_ancestors.difference(&trunk_ancestors).count())
            else {
                return AheadBehind::Unavailable;
            };
            let Ok(behind) = u64::try_from(trunk_ancestors.difference(&worktree_ancestors).count())
            else {
                return AheadBehind::Unavailable;
            };
            AheadBehind::Counts { ahead, behind }
        })
        .collect()
}

/// Determine whether one commit is an ancestor of another.
pub(crate) fn reachability(
    repository_root: &Path,
    ancestor: &GitObjectId,
    descendant: &GitObjectId,
) -> Result<Reachability, GitError> {
    let ancestor = ancestor.to_string();
    let descendant = descendant.to_string();
    let output = git_output(
        repository_root,
        [
            GIT_MERGE_BASE_COMMAND,
            GIT_IS_ANCESTOR_ARG,
            &ancestor,
            &descendant,
        ],
    )?;
    if output.status.success() {
        Ok(Reachability::Ancestor)
    } else if output.status.code() == Some(GIT_NOT_ANCESTOR_EXIT_CODE) {
        Ok(Reachability::NotAncestor)
    } else {
        Ok(Reachability::ObjectUnknown)
    }
}

/// Classify every candidate ancestor against one target with a fixed number of git invocations.
pub(crate) fn reachability_to_target(
    repository_root: &Path,
    candidate_ancestors: &[GitObjectId],
    target: &GitObjectId,
) -> Result<Vec<Reachability>, GitError> {
    if candidate_ancestors.is_empty() {
        return Ok(Vec::new());
    }
    let mut queried_objects = Vec::with_capacity(candidate_ancestors.len() + 1);
    queried_objects.extend(candidate_ancestors.iter().cloned());
    queried_objects.push(target.clone());
    let object_availability = commit_availability(repository_root, &queried_objects)?;
    let Some((target_availability, candidate_availability)) = object_availability.split_last()
    else {
        return Err(GitError::InvalidBatchObjectCount {
            expected: queried_objects.len(),
            actual:   0,
        });
    };
    if matches!(target_availability, CommitAvailability::ObjectUnknown) {
        return Ok(vec![Reachability::ObjectUnknown; candidate_ancestors.len()]);
    }
    let target_text = target.to_string();
    let arguments = [GIT_REV_LIST_COMMAND.to_owned(), target_text];
    let output = git_output_dynamic(repository_root, &arguments)?;
    if !output.status.success() {
        return Err(GitError::CommandFailed {
            command: GIT_REV_LIST_COMMAND,
            stderr:  String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    let output_text = String::from_utf8(output.stdout).map_err(GitError::InvalidOutput)?;
    let target_history = output_text
        .lines()
        .map(str::parse)
        .collect::<Result<HashSet<GitObjectId>, _>>()
        .map_err(GitError::InvalidObjectId)?;
    Ok(candidate_ancestors
        .iter()
        .zip(candidate_availability)
        .map(|(candidate_ancestor, availability)| match availability {
            CommitAvailability::Available if target_history.contains(candidate_ancestor) => {
                Reachability::Ancestor
            },
            CommitAvailability::Available => Reachability::NotAncestor,
            CommitAvailability::ObjectUnknown => Reachability::ObjectUnknown,
        })
        .collect())
}

/// Classify successor heads against every protected predecessor tip in one revision walk.
pub(crate) fn descendant_commits(
    repository_root: &Path,
    predecessors: &[ProtectedTipSuccessorHeads<'_>],
) -> Result<Vec<DescendantCommitQuery>, GitError> {
    if predecessors.is_empty() {
        return Ok(Vec::new());
    }
    let input = predecessors
        .iter()
        .fold(String::new(), |mut input, predecessor| {
            let _ = writeln!(input, "{}", predecessor.protected_tip);
            for successor_head in predecessor.successor_heads {
                let _ = writeln!(input, "{successor_head}");
            }
            input
        });
    let arguments = [
        GIT_REV_LIST_COMMAND.to_owned(),
        GIT_IGNORE_MISSING_ARG.to_owned(),
        GIT_PARENTS_ARG.to_owned(),
        GIT_STDIN_ARG.to_owned(),
    ];
    let output = git_output_dynamic_with_input(repository_root, &arguments, input.as_bytes())?;
    if !output.status.success() {
        return Err(GitError::CommandFailed {
            command: GIT_REV_LIST_COMMAND,
            stderr:  String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    let output_text = String::from_utf8(output.stdout).map_err(GitError::InvalidOutput)?;
    let commit_ancestry_graph = CommitAncestryGraph::try_from(output_text.as_str())?;
    Ok(predecessors
        .iter()
        .map(|predecessor| {
            if !commit_ancestry_graph.contains(predecessor.protected_tip) {
                return DescendantCommitQuery::AncestorObjectUnknown;
            }
            DescendantCommitQuery::Classified(
                predecessor
                    .successor_heads
                    .iter()
                    .map(|successor_head| {
                        if !commit_ancestry_graph.contains(successor_head) {
                            return CandidateHeadReachability::ObjectUnknown(
                                successor_head.clone(),
                            );
                        }
                        if commit_ancestry_graph
                            .ancestors_including(successor_head)
                            .contains(predecessor.protected_tip)
                        {
                            CandidateHeadReachability::Descendant(successor_head.clone())
                        } else {
                            CandidateHeadReachability::NotDescendant(successor_head.clone())
                        }
                    })
                    .collect(),
            )
        })
        .collect())
}

fn commit_availability(
    repository_root: &Path,
    object_ids: &[GitObjectId],
) -> Result<Vec<CommitAvailability>, GitError> {
    let input = object_ids
        .iter()
        .fold(String::new(), |mut input, object_id| {
            let _ = writeln!(input, "{object_id}{GIT_COMMIT_PEEL_SUFFIX}");
            input
        });
    let arguments = [
        GIT_CAT_FILE_COMMAND.to_owned(),
        GIT_BATCH_CHECK_ARG.to_owned(),
    ];
    let output = git_output_dynamic_with_input(repository_root, &arguments, input.as_bytes())?;
    if !output.status.success() {
        return Err(GitError::CommandFailed {
            command: GIT_CAT_FILE_COMMAND,
            stderr:  String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    let output_text = String::from_utf8(output.stdout).map_err(GitError::InvalidOutput)?;
    let availability = output_text
        .lines()
        .map(|line| {
            if line.ends_with(GIT_MISSING_OBJECT_SUFFIX) {
                CommitAvailability::ObjectUnknown
            } else {
                CommitAvailability::Available
            }
        })
        .collect::<Vec<_>>();
    if availability.len() != object_ids.len() {
        return Err(GitError::InvalidBatchObjectCount {
            expected: object_ids.len(),
            actual:   availability.len(),
        });
    }
    Ok(availability)
}

/// Create or update a reservation's retention ref.
pub(crate) fn write_reservation_retention_ref(
    repository_root: &Path,
    reservation_id: ReservationId,
    protected_tip: &GitObjectId,
) -> Result<(), GitError> {
    refs::write(repository_root, reservation_id, protected_tip)
}

/// Rewrite every readable protected-tip retention ref with at most two git invocations.
pub(crate) fn repair_reservation_retention_refs(
    repository_root: &Path,
    repairs: &[ReservationRetentionRefRepair],
) -> Result<(), GitError> {
    if repairs.is_empty() {
        return Ok(());
    }
    let protected_tips = repairs
        .iter()
        .map(|repair| repair.protected_tip.clone())
        .collect::<Vec<_>>();
    let availability = commit_availability(repository_root, &protected_tips)?;
    let input = repairs.iter().zip(availability).fold(
        String::new(),
        |mut input, (repair, availability)| {
            if matches!(availability, CommitAvailability::Available) {
                let _ = writeln!(
                    input,
                    "update {} {}",
                    refs::name(repair.reservation_id),
                    repair.protected_tip
                );
            }
            input
        },
    );
    if input.is_empty() {
        return Ok(());
    }
    let arguments = [GIT_UPDATE_REF_COMMAND.to_owned(), GIT_STDIN_ARG.to_owned()];
    let output = git_output_dynamic_with_input(repository_root, &arguments, input.as_bytes())?;
    if output.status.success() {
        Ok(())
    } else {
        Err(GitError::CommandFailed {
            command: GIT_UPDATE_REF_COMMAND,
            stderr:  String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        })
    }
}

/// Return the full private ref used to retain one reservation's protected tip.
pub(crate) fn reservation_retention_ref_name(reservation_id: ReservationId) -> String {
    refs::name(reservation_id)
}

/// Delete a reservation's retention ref.
pub(crate) fn delete_reservation_retention_ref(
    repository_root: &Path,
    reservation_id: ReservationId,
) -> Result<(), GitError> {
    refs::delete(repository_root, reservation_id)
}

fn object_id(repository_root: &Path, revision: &str) -> Result<GitObjectId, GitError> {
    let output = git_output(repository_root, [GIT_REV_PARSE_COMMAND, revision])?;
    if !output.status.success() {
        return Err(GitError::CommandFailed {
            command: GIT_REV_PARSE_COMMAND,
            stderr:  String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    let object_id = String::from_utf8(output.stdout).map_err(GitError::InvalidOutput)?;
    GitObjectId::from_str(object_id.trim()).map_err(GitError::InvalidObjectId)
}

/// The three outcomes of `git merge-base --is-ancestor`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Reachability {
    /// The first commit is reachable from the second.
    Ancestor,
    /// Both objects exist, but the first is not reachable from the second.
    NotAncestor,
    /// Git could not read one or both objects.
    ObjectUnknown,
}

/// One candidate head's relation to a protected predecessor tip.
pub(crate) enum CandidateHeadReachability {
    /// The candidate head contains the protected predecessor tip.
    Descendant(GitObjectId),
    /// The candidate head resolves but does not contain the predecessor tip.
    NotDescendant(GitObjectId),
    /// This candidate head does not resolve as a commit.
    ObjectUnknown(GitObjectId),
}

/// The successor heads whose ancestry is evaluated against one protected reservation tip.
pub(crate) struct ProtectedTipSuccessorHeads<'commits> {
    protected_tip:   &'commits GitObjectId,
    successor_heads: &'commits [GitObjectId],
}

impl<'commits> ProtectedTipSuccessorHeads<'commits> {
    pub(crate) const fn new(
        protected_tip: &'commits GitObjectId,
        successor_heads: &'commits [GitObjectId],
    ) -> Self {
        Self {
            protected_tip,
            successor_heads,
        }
    }
}

/// One reservation ref that must retain its protected commit when that commit is readable.
pub(crate) struct ReservationRetentionRefRepair {
    reservation_id: ReservationId,
    protected_tip:  GitObjectId,
}

impl ReservationRetentionRefRepair {
    pub(crate) const fn new(reservation_id: ReservationId, protected_tip: GitObjectId) -> Self {
        Self {
            reservation_id,
            protected_tip,
        }
    }
}

/// The grouped descendant result for one protected predecessor tip.
pub(crate) enum DescendantCommitQuery {
    /// Every candidate head received its own typed reachability result.
    Classified(Vec<CandidateHeadReachability>),
    /// The protected predecessor tip does not resolve as a commit.
    AncestorObjectUnknown,
}

#[derive(Clone, Copy)]
enum CommitAvailability {
    Available,
    ObjectUnknown,
}

/// Whether a full git reference currently resolves to an object.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ReferenceLookup {
    /// The reference resolves to this object id.
    Present(GitObjectId),
    /// Git reports no object under this reference name.
    Missing,
}

/// A failure while resolving git's shared administrative directory.
#[derive(Debug)]
pub(crate) enum GitError {
    /// Git could not be started or read.
    Io(std::io::Error),
    /// Git completed unsuccessfully.
    CommandFailed {
        /// The git subcommand that failed.
        command: &'static str,
        /// The diagnostic git reported.
        stderr:  String,
    },
    /// Git printed non-UTF-8 output where the operation requires UTF-8.
    InvalidOutput(FromUtf8Error),
    /// Git printed text that was not a full object id.
    InvalidObjectId(InvalidGitObjectId),
    /// Git printed text that was not a valid full reference name.
    InvalidReferenceName { reference: String },
    /// `cat-file --batch-check` did not classify every submitted object.
    InvalidBatchObjectCount { expected: usize, actual: usize },
    /// The expected branch object is not an ancestor of the proposed object.
    NonFastForwardBranchUpdate {
        previous: GitObjectId,
        proposed: GitObjectId,
    },
    /// Git could not verify both objects needed for a fast-forward branch update.
    BranchUpdateObjectUnavailable {
        previous: GitObjectId,
        proposed: GitObjectId,
    },
    /// `rev-list --count` printed something that was not a commit total.
    UncountableCommitRange {
        /// The range whose total could not be read.
        range: String,
    },
    /// A path-limited first-parent walk returned a commit absent from the full walk.
    ScopedCommitMissingFromTargetWalk { commit: GitObjectId },
}

impl Display for GitError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "could not run git: {error}"),
            Self::CommandFailed { command, stderr } => {
                write!(formatter, "git {command} failed: {stderr}")
            },
            Self::InvalidOutput(error) => {
                write!(formatter, "git printed non-UTF-8 output: {error}")
            },
            Self::InvalidObjectId(error) => {
                write!(formatter, "git printed an invalid object id: {error}")
            },
            Self::InvalidReferenceName { reference } => {
                write!(formatter, "git printed an invalid ref name: {reference:?}")
            },
            Self::InvalidBatchObjectCount { expected, actual } => write!(
                formatter,
                "git cat-file classified {actual} objects when {expected} were submitted"
            ),
            Self::NonFastForwardBranchUpdate { previous, proposed } => write!(
                formatter,
                "refusing non-fast-forward branch update from {previous} to {proposed}"
            ),
            Self::BranchUpdateObjectUnavailable { previous, proposed } => write!(
                formatter,
                "could not verify a fast-forward branch update from {previous} to {proposed}"
            ),
            Self::UncountableCommitRange { range } => {
                write!(formatter, "git could not count the commits in {range}")
            },
            Self::ScopedCommitMissingFromTargetWalk { commit } => write!(
                formatter,
                "git returned scoped commit {commit} outside the target's first-parent walk"
            ),
        }
    }
}

impl std::error::Error for GitError {}

impl From<std::io::Error> for GitError {
    fn from(error: std::io::Error) -> Self { Self::Io(error) }
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::fs;
    use std::io;
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;
    use std::process::Command;

    use tempfile::TempDir;
    use tempfile::tempdir;

    use super::AheadBehind;
    use super::CandidateHeadReachability;
    use super::DescendantCommitQuery;
    use super::ProtectedTipSuccessorHeads;
    use super::ScopedPatchComparison;
    use super::ScopedPatchComparisonError;
    use super::ahead_behind_for_heads;
    use super::command::GitCommandExecution;
    use super::descendant_commits;
    use super::head_object_id;
    use super::scoped_patch_command_output;
    use super::scoped_patch_equivalence;
    use crate::ids::GitObjectId;
    use crate::ledger::ProtectedPhaseStartHead;
    use crate::ledger::ReservationScope;
    use crate::reservation::IntegrationEvidenceStatus;
    use crate::reservation::IntegrationProof;
    use crate::reservation::PriorIntegrationStatus;
    use crate::reservation::ProtectedReservationTip;
    use crate::reservation::integration_status;
    use crate::scope::ReservationScopeSet;
    use crate::scope::ScopeKind;

    const INITIAL_PRIMARY: &str = "first\nsecond\nthird\n";
    const INITIAL_SECONDARY: &str = "secondary\n";
    const PRIMARY_PATH: &str = "src/primary.rs";
    const SCRIPT_PATH: &str = "scripts/run.sh";
    const SECONDARY_PATH: &str = "src/secondary.rs";
    const UNAVAILABLE_OBJECT_ID: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    type FixtureResult<T = ()> = Result<T, Box<dyn Error>>;

    struct PatchEquivalenceFixture {
        repository:       TempDir,
        phase_start_head: GitObjectId,
    }

    impl PatchEquivalenceFixture {
        fn new() -> FixtureResult<Self> {
            let repository = tempdir()?;
            run_git(
                repository.path(),
                &["init", "--quiet", "--initial-branch", "main"],
            )?;
            run_git(
                repository.path(),
                &["config", "user.email", "test@example.com"],
            )?;
            run_git(repository.path(), &["config", "user.name", "Test User"])?;
            write_file(repository.path(), PRIMARY_PATH, INITIAL_PRIMARY)?;
            write_file(repository.path(), SECONDARY_PATH, INITIAL_SECONDARY)?;
            write_file(repository.path(), SCRIPT_PATH, "#!/bin/sh\nexit 0\n")?;
            run_git(repository.path(), &["add", "."])?;
            run_git(repository.path(), &["commit", "--quiet", "-m", "initial"])?;
            let phase_start_head = head_object_id(repository.path())?;
            Ok(Self {
                repository,
                phase_start_head,
            })
        }

        fn root(&self) -> &Path { self.repository.path() }

        fn write(&self, path: &str, contents: &str) -> io::Result<()> {
            write_file(self.root(), path, contents)
        }

        fn remove(&self, path: &str) -> io::Result<()> { fs::remove_file(self.root().join(path)) }

        fn git(&self, arguments: &[&str]) -> io::Result<()> { run_git(self.root(), arguments) }

        fn commit(&self, message: &str) -> FixtureResult<GitObjectId> {
            self.git(&["add", "--all"])?;
            self.git(&["commit", "--quiet", "-m", message])?;
            Ok(head_object_id(self.root())?)
        }

        fn amend(&self, message: &str) -> FixtureResult<GitObjectId> {
            self.git(&["commit", "--quiet", "--amend", "-m", message])?;
            Ok(head_object_id(self.root())?)
        }

        fn reset_to_phase_start(&self) -> io::Result<()> { self.reset_to(&self.phase_start_head) }

        fn reset_to(&self, target: &GitObjectId) -> io::Result<()> {
            self.git(&["reset", "--hard", &target.to_string()])
        }

        fn set_executable(&self, path: &str) -> io::Result<()> {
            let path = self.root().join(path);
            let mut permissions = fs::metadata(&path)?.permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(path, permissions)
        }

        fn equivalence(
            &self,
            scopes: &ReservationScopeSet,
            protected_tip: &GitObjectId,
            target: &GitObjectId,
        ) -> Result<ScopedPatchComparison, super::GitError> {
            scoped_patch_equivalence(
                self.root(),
                &self.phase_start_head,
                scopes,
                protected_tip,
                target,
            )
        }
    }

    #[test]
    fn scoped_patch_command_spawn_failure_is_unavailable() {
        let command_execution =
            GitCommandExecution::from(Command::new("cargo-berth-missing-git").output());

        assert!(matches!(
            scoped_patch_command_output(command_execution),
            Err(ScopedPatchComparisonError::CommandUnavailable)
        ));
    }

    #[test]
    fn unresolvable_worktree_head_preserves_other_ahead_behind_counts() -> FixtureResult {
        let fixture = PatchEquivalenceFixture::new()?;
        let trunk = fixture.phase_start_head.clone();
        fixture.write(PRIMARY_PATH, "resolvable worktree head\n")?;
        let ahead_head = fixture.commit("worktree ahead of trunk")?;
        let unresolvable_head = UNAVAILABLE_OBJECT_ID.parse::<GitObjectId>()?;

        assert_eq!(
            ahead_behind_for_heads(
                fixture.root(),
                &trunk,
                &[ahead_head, unresolvable_head, trunk.clone()],
            ),
            vec![
                AheadBehind::Counts {
                    ahead:  1,
                    behind: 0,
                },
                AheadBehind::Unavailable,
                AheadBehind::Counts {
                    ahead:  0,
                    behind: 0,
                },
            ]
        );
        Ok(())
    }

    #[test]
    fn batched_descendant_query_confines_unknown_objects_to_their_subjects() -> FixtureResult {
        let fixture = PatchEquivalenceFixture::new()?;
        let ancestor = fixture.phase_start_head.clone();
        fixture.write(PRIMARY_PATH, "descendant worktree head\n")?;
        let descendant = fixture.commit("descendant head")?;
        let unresolvable = UNAVAILABLE_OBJECT_ID.parse::<GitObjectId>()?;
        let mixed_heads = [descendant.clone(), unresolvable.clone()];
        let known_head = [descendant.clone()];
        let unrelated_head = [ancestor.clone()];
        let queries = [
            ProtectedTipSuccessorHeads::new(&ancestor, &mixed_heads),
            ProtectedTipSuccessorHeads::new(&unresolvable, &known_head),
            ProtectedTipSuccessorHeads::new(&descendant, &unrelated_head),
        ];

        let results = descendant_commits(fixture.root(), &queries)?;
        assert!(matches!(
            results.as_slice(),
            [
                DescendantCommitQuery::Classified(mixed),
                DescendantCommitQuery::AncestorObjectUnknown,
                DescendantCommitQuery::Classified(unrelated),
            ] if matches!(
                mixed.as_slice(),
                [
                    CandidateHeadReachability::Descendant(classified_descendant),
                    CandidateHeadReachability::ObjectUnknown(classified_unresolvable),
                ] if classified_descendant == &descendant
                    && classified_unresolvable == &unresolvable
            ) && matches!(
                unrelated.as_slice(),
                [CandidateHeadReachability::NotDescendant(classified_ancestor)]
                    if classified_ancestor == &ancestor
            )
        ));
        Ok(())
    }

    #[test]
    fn amended_commit_retains_scoped_patch_equivalence() -> FixtureResult {
        let fixture = PatchEquivalenceFixture::new()?;
        fixture.write(PRIMARY_PATH, "amended reservation\n")?;
        let protected_tip = fixture.commit("protected identity")?;
        let target = fixture.amend("amended identity")?;

        assert_eq!(
            fixture.equivalence(&file_scopes(&[PRIMARY_PATH])?, &protected_tip, &target)?,
            ScopedPatchComparison::Equivalent
        );
        Ok(())
    }

    #[test]
    fn amended_integration_records_scoped_patch_proof() -> FixtureResult {
        let fixture = PatchEquivalenceFixture::new()?;
        fixture.write(PRIMARY_PATH, "amended reservation\n")?;
        let protected_tip = fixture.commit("protected identity")?;
        let target = fixture.amend("amended identity")?;
        let status = integration_status(
            fixture.root(),
            &ProtectedPhaseStartHead::from(fixture.phase_start_head.clone()),
            &file_scopes(&[PRIMARY_PATH])?,
            &ProtectedReservationTip::from(protected_tip),
            &target,
            PriorIntegrationStatus::Proven,
        )?;

        assert_eq!(
            status,
            IntegrationEvidenceStatus::Integrated {
                trunk_oid: target,
                proof:     IntegrationProof::ScopedPatchEquivalent,
            }
        );
        Ok(())
    }

    #[test]
    fn ancestor_integration_does_not_read_the_patch_baseline() -> FixtureResult {
        let fixture = PatchEquivalenceFixture::new()?;
        let unavailable_phase_start = UNAVAILABLE_OBJECT_ID.parse::<GitObjectId>()?;
        let protected_tip = ProtectedReservationTip::from(fixture.phase_start_head.clone());
        let status = integration_status(
            fixture.root(),
            &ProtectedPhaseStartHead::from(unavailable_phase_start),
            &file_scopes(&[PRIMARY_PATH])?,
            &protected_tip,
            &fixture.phase_start_head,
            PriorIntegrationStatus::Unproven,
        )?;

        assert_eq!(
            status,
            IntegrationEvidenceStatus::Integrated {
                trunk_oid: fixture.phase_start_head,
                proof:     IntegrationProof::ProtectedTipAncestor,
            }
        );
        Ok(())
    }

    #[test]
    fn unavailable_phase_start_cannot_produce_a_scoped_patch_verdict() -> FixtureResult {
        let fixture = PatchEquivalenceFixture::new()?;
        fixture.write(PRIMARY_PATH, "protected content\n")?;
        let protected_tip = fixture.commit("protected identity")?;
        let target = fixture.amend("amended identity")?;
        let unavailable_phase_start = UNAVAILABLE_OBJECT_ID.parse::<GitObjectId>()?;
        let scopes = file_scopes(&[PRIMARY_PATH])?;

        assert_eq!(
            scoped_patch_equivalence(
                fixture.root(),
                &unavailable_phase_start,
                &scopes,
                &protected_tip,
                &target,
            )?,
            ScopedPatchComparison::Unavailable
        );
        assert_eq!(
            integration_status(
                fixture.root(),
                &ProtectedPhaseStartHead::from(unavailable_phase_start),
                &scopes,
                &ProtectedReservationTip::from(protected_tip),
                &target,
                PriorIntegrationStatus::Proven,
            )?,
            IntegrationEvidenceStatus::ObjectUnknown
        );
        Ok(())
    }

    #[test]
    fn rebased_commit_retains_scoped_patch_equivalence() -> FixtureResult {
        let fixture = PatchEquivalenceFixture::new()?;
        fixture.write(PRIMARY_PATH, "rebased reservation\n")?;
        let protected_tip = fixture.commit("protected identity")?;
        fixture.reset_to_phase_start()?;
        fixture.write("docs/upstream.md", "upstream\n")?;
        fixture.commit("new base")?;
        fixture.git(&["cherry-pick", &protected_tip.to_string()])?;
        let target = head_object_id(fixture.root())?;

        assert_eq!(
            fixture.equivalence(&file_scopes(&[PRIMARY_PATH])?, &protected_tip, &target)?,
            ScopedPatchComparison::Equivalent
        );
        Ok(())
    }

    #[test]
    fn branch_ahead_phase_start_can_match_replayed_target_history() -> FixtureResult {
        let mut fixture = PatchEquivalenceFixture::new()?;
        let shared_base = fixture.phase_start_head.clone();
        fixture.write(PRIMARY_PATH, "branch baseline\nsecond\nthird\n")?;
        fixture.phase_start_head = fixture.commit("branch-local baseline")?;
        fixture.write(PRIMARY_PATH, "branch baseline\nprotected\nthird\n")?;
        let protected_tip = fixture.commit("protected edit")?;

        fixture.reset_to(&shared_base)?;
        fixture.write("docs/upstream.md", "upstream\n")?;
        fixture.commit("target base")?;
        fixture.git(&[
            "cherry-pick",
            "--quiet",
            &fixture.phase_start_head.to_string(),
        ])?;
        fixture.git(&["cherry-pick", "--quiet", &protected_tip.to_string()])?;
        let target = head_object_id(fixture.root())?;

        assert_eq!(
            fixture.equivalence(&file_scopes(&[PRIMARY_PATH])?, &protected_tip, &target)?,
            ScopedPatchComparison::Equivalent
        );
        Ok(())
    }

    #[test]
    fn no_ff_merge_retains_rebased_scoped_patch_equivalence() -> FixtureResult {
        let fixture = PatchEquivalenceFixture::new()?;
        fixture.write(PRIMARY_PATH, "merged reservation\n")?;
        let protected_tip = fixture.commit("protected identity")?;
        fixture.reset_to_phase_start()?;
        fixture.write("docs/upstream.md", "upstream\n")?;
        fixture.commit("new base")?;
        fixture.git(&["checkout", "--quiet", "-b", "integrated-work"])?;
        fixture.git(&["cherry-pick", "--quiet", &protected_tip.to_string()])?;
        fixture.git(&["checkout", "--quiet", "main"])?;
        fixture.git(&[
            "merge",
            "--quiet",
            "--no-ff",
            "--no-edit",
            "integrated-work",
        ])?;
        let target = head_object_id(fixture.root())?;

        assert_eq!(
            fixture.equivalence(&file_scopes(&[PRIMARY_PATH])?, &protected_tip, &target)?,
            ScopedPatchComparison::Equivalent
        );
        Ok(())
    }

    #[test]
    fn amended_same_file_addition_preserves_scoped_change() -> FixtureResult {
        let fixture = PatchEquivalenceFixture::new()?;
        fixture.write(PRIMARY_PATH, "reservation\nsecond\nthird\n")?;
        let protected_tip = fixture.commit("protected edit")?;
        fixture.write(
            PRIMARY_PATH,
            "reservation\nsecond\nthird\nunrelated addition\n",
        )?;
        fixture.git(&["add", "--all"])?;
        let target = fixture.amend("amended edit with unrelated addition")?;

        assert_eq!(
            fixture.equivalence(&file_scopes(&[PRIMARY_PATH])?, &protected_tip, &target)?,
            ScopedPatchComparison::Equivalent
        );
        Ok(())
    }

    #[test]
    fn insertion_above_protected_hunk_preserves_scoped_change() -> FixtureResult {
        let mut fixture = PatchEquivalenceFixture::new()?;
        fixture.write(
            PRIMARY_PATH,
            "top one\ntop two\ntop three\nfirst\nsecond\nthird\nbottom\n",
        )?;
        fixture.phase_start_head = fixture.commit("interior hunk baseline")?;
        fixture.write(
            PRIMARY_PATH,
            "top one\ntop two\ntop three\nfirst\nprotected\nthird\nbottom\n",
        )?;
        let protected_tip = fixture.commit("protected edit")?;
        fixture.write(
            PRIMARY_PATH,
            "top one\ntop two\ntop three\nabove one\nabove two\nabove three\nabove four\nabove five\nfirst\nprotected\nthird\nbottom\n",
        )?;
        fixture.git(&["add", "--all"])?;
        let target = fixture.amend("protected edit shifted by insertion")?;

        assert_eq!(
            fixture.equivalence(&file_scopes(&[PRIMARY_PATH])?, &protected_tip, &target)?,
            ScopedPatchComparison::Equivalent
        );
        Ok(())
    }

    #[test]
    fn amended_same_file_replacement_does_not_preserve_scoped_change() -> FixtureResult {
        let fixture = PatchEquivalenceFixture::new()?;
        fixture.write(PRIMARY_PATH, "reservation\nsecond\nthird\n")?;
        let protected_tip = fixture.commit("protected edit")?;
        fixture.write(
            PRIMARY_PATH,
            "replacement\nsecond\nthird\nunrelated addition\n",
        )?;
        fixture.git(&["add", "--all"])?;
        let target = fixture.amend("replaced edit with unrelated addition")?;

        assert_eq!(
            fixture.equivalence(&file_scopes(&[PRIMARY_PATH])?, &protected_tip, &target)?,
            ScopedPatchComparison::Different
        );
        Ok(())
    }

    #[test]
    fn renamed_path_is_compared_as_deletion_and_addition() -> FixtureResult {
        let fixture = PatchEquivalenceFixture::new()?;
        fixture.git(&["mv", PRIMARY_PATH, "src/renamed.rs"])?;
        let protected_tip = fixture.commit("protected rename")?;
        let target = fixture.amend("amended rename")?;

        assert_eq!(
            fixture.equivalence(&tree_scopes(&["src"])?, &protected_tip, &target)?,
            ScopedPatchComparison::Equivalent
        );
        Ok(())
    }

    #[test]
    fn target_side_rename_does_not_hide_missing_protected_modification() -> FixtureResult {
        let fixture = PatchEquivalenceFixture::new()?;
        fixture.write(PRIMARY_PATH, "protected content\n")?;
        let protected_tip = fixture.commit("protected modification")?;
        fixture.reset_to_phase_start()?;
        fixture.git(&["mv", PRIMARY_PATH, "src/renamed.rs"])?;
        let target = fixture.commit("target rename")?;

        assert_eq!(
            fixture.equivalence(&tree_scopes(&["src"])?, &protected_tip, &target)?,
            ScopedPatchComparison::Different
        );
        Ok(())
    }

    #[test]
    fn target_side_rename_outside_scope_does_not_hide_missing_modification() -> FixtureResult {
        let fixture = PatchEquivalenceFixture::new()?;
        fixture.write(PRIMARY_PATH, "protected content\n")?;
        let protected_tip = fixture.commit("protected modification")?;
        fixture.reset_to_phase_start()?;
        fs::create_dir_all(fixture.root().join("other"))?;
        fixture.git(&["mv", PRIMARY_PATH, "other/b.rs"])?;
        let target = fixture.commit("target rename outside scope")?;

        assert_eq!(
            fixture.equivalence(&tree_scopes(&["src"])?, &protected_tip, &target)?,
            ScopedPatchComparison::Different
        );
        Ok(())
    }

    #[test]
    fn reservation_authored_deletion_is_patch_equivalent() -> FixtureResult {
        let fixture = PatchEquivalenceFixture::new()?;
        fixture.remove(PRIMARY_PATH)?;
        let protected_tip = fixture.commit("protected deletion")?;
        let target = fixture.amend("amended deletion")?;

        assert_eq!(
            fixture.equivalence(&file_scopes(&[PRIMARY_PATH])?, &protected_tip, &target)?,
            ScopedPatchComparison::Equivalent
        );
        Ok(())
    }

    #[test]
    fn target_modification_does_not_certify_protected_deletion() -> FixtureResult {
        let fixture = PatchEquivalenceFixture::new()?;
        fixture.remove(PRIMARY_PATH)?;
        let protected_tip = fixture.commit("protected deletion")?;
        fixture.reset_to_phase_start()?;
        fixture.write(PRIMARY_PATH, "target modification\n")?;
        let target = fixture.commit("target modification")?;

        assert_eq!(
            fixture.equivalence(&file_scopes(&[PRIMARY_PATH])?, &protected_tip, &target)?,
            ScopedPatchComparison::Different
        );
        Ok(())
    }

    #[test]
    fn conflict_outside_scopes_does_not_reject_integrated_scoped_content() -> FixtureResult {
        let fixture = PatchEquivalenceFixture::new()?;
        fixture.write(PRIMARY_PATH, "integrated protected content\n")?;
        fixture.write(SECONDARY_PATH, "protected conflicting content\n")?;
        let protected_tip = fixture.commit("protected scoped and unscoped modifications")?;
        fixture.reset_to_phase_start()?;
        fixture.write(PRIMARY_PATH, "integrated protected content\n")?;
        fixture.write(SECONDARY_PATH, "target conflicting content\n")?;
        let target = fixture.commit("target scoped and unscoped modifications")?;

        assert_eq!(
            fixture.equivalence(&file_scopes(&[PRIMARY_PATH])?, &protected_tip, &target)?,
            ScopedPatchComparison::Equivalent
        );
        Ok(())
    }

    #[test]
    fn tree_scope_ignores_later_unrelated_descendant_commit() -> FixtureResult {
        let fixture = PatchEquivalenceFixture::new()?;
        fixture.write(PRIMARY_PATH, "tree reservation\n")?;
        let protected_tip = fixture.commit("protected tree change")?;
        fixture.write("src/later.rs", "later descendant\n")?;
        fixture.git(&["add", "--all"])?;
        let target = fixture.amend("amended tree change with later descendant")?;

        assert_eq!(
            fixture.equivalence(&tree_scopes(&["src"])?, &protected_tip, &target)?,
            ScopedPatchComparison::Equivalent
        );
        Ok(())
    }

    #[test]
    fn target_edits_inside_and_outside_tree_scope_preserve_protected_change() -> FixtureResult {
        let fixture = PatchEquivalenceFixture::new()?;
        fixture.write(PRIMARY_PATH, "tree reservation\n")?;
        let protected_tip = fixture.commit("protected tree change")?;
        fixture.write("src/later.rs", "later descendant\n")?;
        fixture.write("docs/later.md", "later outside scope\n")?;
        fixture.git(&["add", "--all"])?;
        let target = fixture.amend("amended tree change with unrelated edits")?;

        assert_eq!(
            fixture.equivalence(&tree_scopes(&["src"])?, &protected_tip, &target)?,
            ScopedPatchComparison::Equivalent
        );
        Ok(())
    }

    #[test]
    fn later_unrelated_edit_to_same_file_preserves_proof() -> FixtureResult {
        let fixture = PatchEquivalenceFixture::new()?;
        fixture.write(PRIMARY_PATH, "reservation\nsecond\nthird\n")?;
        let protected_tip = fixture.commit("protected edit")?;
        fixture.amend("amended edit")?;
        fixture.write(PRIMARY_PATH, "reservation\nsecond\nlater edit\n")?;
        let target = fixture.commit("later same-file edit")?;

        assert_eq!(
            fixture.equivalence(&file_scopes(&[PRIMARY_PATH])?, &protected_tip, &target)?,
            ScopedPatchComparison::Equivalent
        );
        Ok(())
    }

    #[test]
    fn intervening_unrelated_commit_does_not_separate_the_phase_integration() -> FixtureResult {
        let fixture = PatchEquivalenceFixture::new()?;
        fixture.write(PRIMARY_PATH, "reservation\nsecond\nthird\n")?;
        let protected_tip = fixture.commit("protected edit")?;
        fixture.reset_to_phase_start()?;
        fixture.write(PRIMARY_PATH, "reservation\nsecond\nthird\n")?;
        fixture.commit("rewritten protected edit")?;
        fixture.write("docs/intervening.md", "intervening\n")?;
        fixture.commit("intervening unrelated edit")?;
        fixture.write(PRIMARY_PATH, "reservation\nsecond\nlater edit\n")?;
        let target = fixture.commit("later same-file edit")?;

        assert_eq!(
            fixture.equivalence(&file_scopes(&[PRIMARY_PATH])?, &protected_tip, &target)?,
            ScopedPatchComparison::Equivalent
        );
        Ok(())
    }

    #[test]
    fn duplicate_context_does_not_relocate_the_protected_change() -> FixtureResult {
        let mut fixture = PatchEquivalenceFixture::new()?;
        fixture.write(
            PRIMARY_PATH,
            "prefix\nalpha\ntarget\nomega\nseparator\nalpha\ntarget\nomega\n",
        )?;
        fixture.phase_start_head = fixture.commit("duplicate blocks baseline")?;
        fixture.write(
            PRIMARY_PATH,
            "prefix\nalpha\nchanged\nomega\nseparator\nalpha\ntarget\nomega\n",
        )?;
        let protected_tip = fixture.commit("protected first block")?;
        fixture.reset_to_phase_start()?;
        fixture.write(
            PRIMARY_PATH,
            "prefix\nalpha\ntarget\nomega\nseparator\nalpha\nchanged\nomega\n",
        )?;
        let target = fixture.commit("changed second block")?;

        assert_eq!(
            fixture.equivalence(&file_scopes(&[PRIMARY_PATH])?, &protected_tip, &target)?,
            ScopedPatchComparison::Different
        );
        Ok(())
    }

    #[test]
    fn protected_postimage_elsewhere_does_not_certify_an_overwritten_site() -> FixtureResult {
        let mut fixture = PatchEquivalenceFixture::new()?;
        fixture.write(
            PRIMARY_PATH,
            "prefix\nalpha\ntarget\nomega\nseparator\nalpha\ntarget\nomega\n",
        )?;
        fixture.phase_start_head = fixture.commit("duplicate blocks baseline")?;
        fixture.write(
            PRIMARY_PATH,
            "prefix\nalpha\nprotected\nomega\nseparator\nalpha\ntarget\nomega\n",
        )?;
        let protected_tip = fixture.commit("protected first block")?;
        fixture.reset_to_phase_start()?;
        fixture.write(
            PRIMARY_PATH,
            "prefix\nalpha\noverwritten\nomega\nseparator\nalpha\nprotected\nomega\n",
        )?;
        let target = fixture.commit("overwrote first block and changed second block")?;

        assert_eq!(
            fixture.equivalence(&file_scopes(&[PRIMARY_PATH])?, &protected_tip, &target)?,
            ScopedPatchComparison::Different
        );
        Ok(())
    }

    #[test]
    fn unrelated_root_with_protected_content_is_not_equivalent() -> FixtureResult {
        let fixture = PatchEquivalenceFixture::new()?;
        fixture.write(PRIMARY_PATH, "protected content\n")?;
        let protected_tip = fixture.commit("protected content")?;
        fixture.git(&["checkout", "--quiet", "--orphan", "unrelated-target"])?;
        let target = fixture.commit("unrelated root with protected content")?;

        assert_eq!(
            fixture.equivalence(&file_scopes(&[PRIMARY_PATH])?, &protected_tip, &target)?,
            ScopedPatchComparison::Different
        );
        Ok(())
    }

    #[test]
    fn mode_change_is_patch_equivalent() -> FixtureResult {
        let fixture = PatchEquivalenceFixture::new()?;
        fixture.set_executable(SCRIPT_PATH)?;
        let protected_tip = fixture.commit("protected mode")?;
        let target = fixture.amend("amended mode")?;

        assert_eq!(
            fixture.equivalence(&file_scopes(&[SCRIPT_PATH])?, &protected_tip, &target)?,
            ScopedPatchComparison::Equivalent
        );
        Ok(())
    }

    #[test]
    fn same_path_with_different_content_is_not_equivalent() -> FixtureResult {
        let fixture = PatchEquivalenceFixture::new()?;
        fixture.write(PRIMARY_PATH, "protected content\n")?;
        let protected_tip = fixture.commit("protected content")?;
        fixture.write(PRIMARY_PATH, "different content\n")?;
        fixture.git(&["add", "--all"])?;
        let target = fixture.amend("different target content")?;

        assert_eq!(
            fixture.equivalence(&file_scopes(&[PRIMARY_PATH])?, &protected_tip, &target)?,
            ScopedPatchComparison::Different
        );
        Ok(())
    }

    #[test]
    fn existing_path_without_protected_edit_is_not_equivalent() -> FixtureResult {
        let fixture = PatchEquivalenceFixture::new()?;
        fixture.write(PRIMARY_PATH, "protected content\n")?;
        let protected_tip = fixture.commit("protected content")?;
        fixture.reset_to_phase_start()?;
        let target = head_object_id(fixture.root())?;

        assert_eq!(
            fixture.equivalence(&file_scopes(&[PRIMARY_PATH])?, &protected_tip, &target)?,
            ScopedPatchComparison::Different
        );
        Ok(())
    }

    #[test]
    fn file_scope_does_not_cover_directory_replacing_the_file() -> FixtureResult {
        let fixture = PatchEquivalenceFixture::new()?;
        fixture.write(PRIMARY_PATH, "protected content\n")?;
        let protected_tip = fixture.commit("protected file modification")?;
        fixture.reset_to_phase_start()?;
        fixture.remove(PRIMARY_PATH)?;
        fixture.write("src/primary.rs/child.rs", "target directory child\n")?;
        let target = fixture.commit("target replaces file with directory")?;

        assert_eq!(
            fixture.equivalence(&file_scopes(&[PRIMARY_PATH])?, &protected_tip, &target)?,
            ScopedPatchComparison::Different
        );
        Ok(())
    }

    #[test]
    fn one_equivalent_commit_does_not_certify_partial_integration() -> FixtureResult {
        let fixture = PatchEquivalenceFixture::new()?;
        fixture.write(PRIMARY_PATH, "integrated part\n")?;
        fixture.commit("first protected patch")?;
        fixture.write(SECONDARY_PATH, "missing part\n")?;
        let protected_tip = fixture.commit("second protected patch")?;
        fixture.reset_to_phase_start()?;
        fixture.write(PRIMARY_PATH, "integrated part\n")?;
        let target = fixture.commit("rewritten first patch only")?;

        assert_eq!(
            fixture.equivalence(
                &file_scopes(&[PRIMARY_PATH, SECONDARY_PATH])?,
                &protected_tip,
                &target,
            )?,
            ScopedPatchComparison::Different
        );
        Ok(())
    }

    #[test]
    fn separated_target_equivalents_do_not_prove_one_replayed_phase() -> FixtureResult {
        let fixture = PatchEquivalenceFixture::new()?;
        fixture.write(PRIMARY_PATH, "first protected patch\n")?;
        fixture.commit("first protected patch")?;
        fixture.write(SECONDARY_PATH, "second protected patch\n")?;
        let protected_tip = fixture.commit("second protected patch")?;
        fixture.reset_to_phase_start()?;
        fixture.write(PRIMARY_PATH, "first protected patch\n")?;
        fixture.commit("rewritten first protected patch")?;
        fixture.write("docs/intervening.md", "intervening\n")?;
        fixture.commit("intervening target patch")?;
        fixture.write(SECONDARY_PATH, "second protected patch\n")?;
        let target = fixture.commit("rewritten second protected patch")?;

        assert_eq!(
            fixture.equivalence(
                &file_scopes(&[PRIMARY_PATH, SECONDARY_PATH])?,
                &protected_tip,
                &target,
            )?,
            ScopedPatchComparison::Different
        );
        Ok(())
    }

    fn file_scopes(paths: &[&str]) -> FixtureResult<ReservationScopeSet> {
        scopes(paths, ScopeKind::File)
    }

    fn tree_scopes(paths: &[&str]) -> FixtureResult<ReservationScopeSet> {
        scopes(paths, ScopeKind::Tree)
    }

    fn scopes(paths: &[&str], scope_kind: ScopeKind) -> FixtureResult<ReservationScopeSet> {
        let scopes = paths
            .iter()
            .map(|path| {
                Ok(ReservationScope {
                    path: path.parse()?,
                    kind: scope_kind,
                })
            })
            .collect::<FixtureResult<Vec<_>>>()?;
        Ok(ReservationScopeSet::try_from(scopes)?)
    }

    fn write_file(repository_root: &Path, path: &str, contents: &str) -> io::Result<()> {
        let path = repository_root.join(path);
        let parent = path
            .parent()
            .ok_or_else(|| io::Error::other("fixture path should have a parent"))?;
        fs::create_dir_all(parent)?;
        fs::write(path, contents)
    }

    fn run_git(repository_root: &Path, arguments: &[&str]) -> io::Result<()> {
        let output = Command::new("git")
            .arg("--no-optional-locks")
            .args(arguments)
            .current_dir(repository_root)
            .output()?;
        if output.status.success() {
            Ok(())
        } else {
            Err(io::Error::other(format!(
                "git {} failed: {}",
                arguments.join(" "),
                String::from_utf8_lossy(&output.stderr)
            )))
        }
    }
}
