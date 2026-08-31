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
use std::path::Path;
use std::path::PathBuf;
use std::process::Output;
use std::str::FromStr;
use std::string::FromUtf8Error;
use std::thread;
use std::thread::ScopedJoinHandle;

pub(crate) use command::GitCommandOutputAvailability;
pub(crate) use command::git_execution as execute_read_only_git;
use command::git_output;
use command::git_output_dynamic;
use command::git_output_dynamic_with_input;
use constants::GIT_AMBIGUOUS_OBJECT_SUFFIX;
use constants::GIT_ANCESTOR_RANGE_INFIX;
use constants::GIT_BATCH_CHECK_ARG;
use constants::GIT_BATCH_CHECK_OBJECT_FORMAT_ARG;
use constants::GIT_CAT_FILE_COMMAND;
use constants::GIT_CHERRY_MARK_ARG;
use constants::GIT_COMMIT_OBJECT_TYPE;
use constants::GIT_COMMIT_PEEL_SUFFIX;
use constants::GIT_COMMON_DIRECTORY_ARG;
use constants::GIT_COUNT_ARG;
use constants::GIT_DENSE_COMBINED_ARG;
use constants::GIT_DIFF_COMMAND;
use constants::GIT_DIFF_TREE_COMMAND;
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
use constants::GIT_IS_ANCESTOR_ARG;
use constants::GIT_LEFT_COMMIT_MARK;
use constants::GIT_LEFT_RIGHT_ARG;
use constants::GIT_LITERAL_TOP_PATHSPEC_PREFIX;
use constants::GIT_LOCAL_BRANCH_REF_PREFIX;
use constants::GIT_LOG_COMMAND;
use constants::GIT_MAX_COUNT_ARG_PREFIX;
use constants::GIT_MAX_COUNT_ONE_ARG;
use constants::GIT_MERGE_BASE_ARG_PREFIX;
use constants::GIT_MERGE_BASE_COMMAND;
use constants::GIT_MERGE_TREE_CLEAN_EXIT_CODE;
use constants::GIT_MERGE_TREE_COMMAND;
use constants::GIT_MERGE_TREE_CONFLICT_EXIT_CODE;
use constants::GIT_MISSING_OBJECT_SUFFIX;
use constants::GIT_NAME_ONLY_ARG;
use constants::GIT_NAME_STATUS_ARG;
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
use constants::GIT_REBASE_APPLY_STATE_PATH;
use constants::GIT_REBASE_MERGE_STATE_PATH;
use constants::GIT_RECURSIVE_ARG;
use constants::GIT_REFLOG_COMMAND;
use constants::GIT_REFLOG_SHOW_ARG;
use constants::GIT_REFLOG_SUBJECT_FORMAT_ARG;
use constants::GIT_REV_LIST_COMMAND;
use constants::GIT_REV_PARSE_COMMAND;
use constants::GIT_RIGHT_COMMIT_MARK;
use constants::GIT_SHOW_TOPLEVEL_ARG;
use constants::GIT_STDIN_ARG;
use constants::GIT_STRATEGY_OPTION_NO_RENAMES_ARG;
use constants::GIT_SYMBOLIC_REF_COMMAND;
use constants::GIT_SYMMETRIC_RANGE_INFIX;
use constants::GIT_UPDATE_REF_COMMAND;
use constants::GIT_WORKTREE_COMMAND;
use constants::GIT_WORKTREE_LIST_ARG;
use constants::GIT_WRITE_TREE_ARG;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;

use crate::ids::GitObjectId;
use crate::ids::InvalidGitObjectId;
use crate::ids::ReservationId;
use crate::ids::ReservationScopePath;
use crate::ledger::FullRefName;
use crate::scope::ReservationScopeSet;
use crate::scope::ScopeKind;

const GIT_DETACHED_HEAD_EXIT_CODE: i32 = 1;
const GIT_MISSING_REFERENCE_EXIT_CODE: i32 = 2;
const GIT_QUIET_ARG: &str = "--quiet";
const GIT_SHOW_REF_COMMAND: &str = "show-ref";
const GIT_SHOW_REF_EXISTS_ARG: &str = "--exists";

/// A worktree's live relationship to the configured trunk.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum AheadBehind {
    /// Both histories share ancestry and have these independent commit counts.
    Counts { ahead: u64, behind: u64 },
    /// Both objects resolve, but their histories have no common ancestor.
    Unrelated,
    /// Git or one required object could not produce a trustworthy comparison.
    Unavailable,
}

/// One candidate commit's typed relation to a resolved target commit.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum CommitCandidateReachability {
    /// The candidate is an ancestor of the target.
    Ancestor,
    /// The candidate resolves as a commit but is not an ancestor of the target.
    NotAncestor,
    /// No object resolves from the submitted expression.
    Missing,
    /// More than one object matches the submitted expression.
    Ambiguous,
    /// The expression resolves, but not to a commit object.
    WrongType { object_type: String },
}

/// One target commit and every candidate classified by the same object-resolution batch.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum CommitTargetReachability {
    /// The target resolves as a commit and every candidate has a typed result.
    Resolved {
        target:     GitObjectId,
        candidates: Vec<CommitCandidateReachability>,
    },
    /// No object resolves from the target expression.
    Missing,
    /// More than one object matches the target expression.
    Ambiguous,
    /// The target expression resolves, but not to a commit object.
    WrongType { object_type: String },
}

/// Candidate commits resolved by one object batch before its graph classification.
#[derive(Default)]
pub(crate) struct ResolvedBatchCommitCandidates(HashSet<GitObjectId>);

impl ResolvedBatchCommitCandidates {
    /// Report whether the object batch resolved this candidate as a commit.
    fn contains(&self, candidate: &GitObjectId) -> bool { self.0.contains(candidate) }
}

/// One target-reachability answer and the candidate availability proved by its object batch.
pub(crate) struct CommitTargetReachabilityObservation {
    /// The target and every candidate's graph relation when the target resolved.
    pub(crate) reachability:        CommitTargetReachability,
    /// Candidate commits safe to reuse during the same admitted operation.
    pub(crate) resolved_candidates: ResolvedBatchCommitCandidates,
    /// First-parent target intervals for candidate ancestors proved by the same graph walk.
    pub(crate) target_histories:    PhaseStartTargetFirstParentHistories,
}

/// Target first-parent intervals keyed by a phase start proved to be its ancestor.
#[derive(Default)]
pub(crate) struct PhaseStartTargetFirstParentHistories(HashMap<GitObjectId, Vec<GitObjectId>>);

impl PhaseStartTargetFirstParentHistories {
    /// Borrow the target interval after one phase start when the graph proved it.
    pub(crate) fn after_phase_start(
        &self,
        phase_start: &GitObjectId,
    ) -> ScopedPatchTargetHistory<'_> {
        self.0
            .get(phase_start)
            .map_or(ScopedPatchTargetHistory::NeedsGitQueries, |commits| {
                ScopedPatchTargetHistory::ProvenFirstParentInterval { commits }
            })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum CommitObjectResolution {
    Resolved(GitObjectId),
    Missing,
    Ambiguous,
    WrongType { object_type: String },
}

/// The parent links needed to compare multiple worktree histories with one revision walk.
struct CommitAncestryGraph {
    parents_by_commit: HashMap<GitObjectId, Vec<GitObjectId>>,
}

struct ResolvedTargetCommitHistory {
    target: GitObjectId,
    graph:  CommitAncestryGraph,
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

    fn first_parent_commits_after(
        &self,
        tip: &GitObjectId,
        excluded_ancestor: &GitObjectId,
    ) -> Vec<GitObjectId> {
        let excluded_commits = self.ancestors_including(excluded_ancestor);
        let mut commits = Vec::new();
        let mut current = tip;
        while !excluded_commits.contains(current) {
            commits.push(current.clone());
            let Some(first_parent) = self
                .parents_by_commit
                .get(current)
                .and_then(|parents| parents.first())
            else {
                break;
            };
            current = first_parent;
        }
        commits
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
    Git(GitError),
}

impl From<GitError> for ScopedPatchComparisonError {
    fn from(error: GitError) -> Self { Self::Git(error) }
}

fn join_scoped_patch_worker<T>(
    worker: ScopedJoinHandle<'_, Result<T, ScopedPatchComparisonError>>,
    activity: &'static str,
) -> Result<T, ScopedPatchComparisonError> {
    worker
        .join()
        .unwrap_or_else(|_| Err(GitError::ScopedPatchWorkerPanicked { activity }.into()))
}

fn concurrent_scoped_patch_reads<T, U, F, G>(
    first_read: F,
    first_activity: &'static str,
    second_read: G,
    second_activity: &'static str,
) -> (
    Result<T, ScopedPatchComparisonError>,
    Result<U, ScopedPatchComparisonError>,
)
where
    T: Send,
    U: Send,
    F: FnOnce() -> Result<T, ScopedPatchComparisonError> + Send,
    G: FnOnce() -> Result<U, ScopedPatchComparisonError> + Send,
{
    thread::scope(|scope| {
        let first_worker = scope.spawn(first_read);
        let second_worker = scope.spawn(second_read);
        (
            join_scoped_patch_worker(first_worker, first_activity),
            join_scoped_patch_worker(second_worker, second_activity),
        )
    })
}

enum HistoryRelationship {
    Shared,
    Unrelated,
    Unavailable,
}

/// Existing evidence about whether a scoped comparison's histories share ancestry.
#[derive(Clone, Copy)]
pub(crate) enum ScopedPatchTargetHistory<'history> {
    /// The admitted graph proved shared history and supplied the exact target interval.
    ProvenFirstParentInterval { commits: &'history [GitObjectId] },
    /// Existing evidence did not prove shared history, so Git must read both facts.
    NeedsGitQueries,
}

enum ProtectedScopedChanges {
    NoChanges,
    Affected {
        paths:                          Vec<String>,
        scoped_replay_rename_detection: ScopedReplayRenameDetection,
    },
    Unreadable,
}

/// Whether a scoped replay must follow a rename inside the protected scope.
#[derive(Clone, Copy)]
enum ScopedReplayRenameDetection {
    /// No protected-scope rename was detected, so rename following stays disabled.
    DisabledWithoutProtectedRename,
    /// A protected-scope rename was detected, so rename following is required.
    RequiredForProtectedRename,
}

enum ProtectedScopedReplayState {
    RequiredAfterRenameClassification,
    EvaluatedAssumingNoRename(Result<ScopedPatchComparison, ScopedPatchComparisonError>),
}

struct InitialScopedPatchEvidence {
    history_relationship:     Result<HistoryRelationship, ScopedPatchComparisonError>,
    protected_scoped_changes: Result<ProtectedScopedChanges, ScopedPatchComparisonError>,
    protected_scoped_replay:  ProtectedScopedReplayState,
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

struct TargetFirstParentHistory {
    commits:        Vec<GitObjectId>,
    scoped_commits: Vec<GitObjectId>,
}

/// Whether the protected tip carries a scoped commit with no equivalent on the target.
enum ProtectedUnmatchedCommit {
    Present,
    Absent,
}

struct ScopedSymmetricDifference {
    protected_unmatched_commit: ProtectedUnmatchedCommit,
    target_unmatched_commits:   HashSet<GitObjectId>,
}

/// Whether a conflicted replay is still usable for one reservation's proof.
enum ScopedMergeConflictCoverage {
    /// Every reported conflict path lies outside the reservation scopes.
    OutsideReservationScopes,
    /// At least one reported conflict path is covered by the reservation.
    CoveredByReservation,
    /// Git moved a reserved file aside because the target replaced it with a directory.
    DisplacedReservedFile,
    /// Git's conflict records did not satisfy the documented `-z` record layout.
    Unreadable,
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

/// Ask Git for the branch reference named by `HEAD`.
pub(crate) fn symbolic_head_reference(repository_root: &Path) -> Result<FullRefName, GitError> {
    let output = completed_git_command(
        git_output(
            repository_root,
            [GIT_SYMBOLIC_REF_COMMAND, GIT_HEAD_REVISION],
        )
        .into(),
    )?;
    if !output.status.success() {
        return Err(GitError::CommandFailed {
            command: GIT_SYMBOLIC_REF_COMMAND,
            stderr:  String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    let reference = String::from_utf8(output.stdout).map_err(GitError::InvalidOutput)?;
    reference
        .trim()
        .parse()
        .map_err(|_| GitError::InvalidReferenceName { reference })
}

/// Whether Git currently reports `HEAD` as attached to a branch or detached.
pub(crate) enum HeadAttachment {
    /// `HEAD` names this full branch reference.
    Branch { full_ref: FullRefName },
    /// `HEAD` names a commit directly.
    Detached,
}

/// Ask Git whether `HEAD` is attached to a branch or detached.
pub(crate) fn head_attachment(repository_root: &Path) -> Result<HeadAttachment, GitError> {
    let output = git_output(
        repository_root,
        [GIT_SYMBOLIC_REF_COMMAND, GIT_QUIET_ARG, GIT_HEAD_REVISION],
    )?;
    if output.status.success() {
        let reference = String::from_utf8(output.stdout).map_err(GitError::InvalidOutput)?;
        return reference
            .trim()
            .parse()
            .map(|full_ref| HeadAttachment::Branch { full_ref })
            .map_err(|_| GitError::InvalidReferenceName { reference });
    }
    if output.status.code() == Some(GIT_DETACHED_HEAD_EXIT_CODE) {
        return Ok(HeadAttachment::Detached);
    }
    Err(GitError::CommandFailed {
        command: GIT_SYMBOLIC_REF_COMMAND,
        stderr:  String::from_utf8_lossy(&output.stderr).trim().to_owned(),
    })
}

/// The two commits recorded by one active-reservation checkpoint.
pub(crate) struct ReservationCheckpointCommits {
    /// The invoking worktree commit protected by the checkpoint.
    pub(crate) protected_tip: GitObjectId,
    /// The configured trunk commit observed by the same object batch.
    pub(crate) trunk:         GitObjectId,
}

/// Resolve the checkpoint's known commit expressions through one object batch.
pub(crate) fn reservation_checkpoint_commits(
    repository_root: &Path,
    trunk_branch: &str,
) -> Result<ReservationCheckpointCommits, GitError> {
    let trunk_expression = format!("{GIT_LOCAL_BRANCH_REF_PREFIX}{trunk_branch}");
    let expressions = [GIT_HEAD_REVISION.to_owned(), trunk_expression.clone()];
    let [protected_tip, trunk] = commit_object_resolutions(repository_root, &expressions)?
        .try_into()
        .map_err(|resolutions: Vec<_>| GitError::InvalidBatchObjectCount {
            expected: expressions.len(),
            actual:   resolutions.len(),
        })?;
    Ok(ReservationCheckpointCommits {
        protected_tip: required_commit_expression(GIT_HEAD_REVISION, protected_tip)?,
        trunk:         required_commit_expression(&trunk_expression, trunk)?,
    })
}

fn required_commit_expression(
    expression: &str,
    resolution: CommitObjectResolution,
) -> Result<GitObjectId, GitError> {
    match resolution {
        CommitObjectResolution::Resolved(object_id) => Ok(object_id),
        CommitObjectResolution::Missing => Err(GitError::MissingCommitExpression {
            expression: expression.to_owned(),
        }),
        CommitObjectResolution::Ambiguous => Err(GitError::AmbiguousCommitExpression {
            expression: expression.to_owned(),
        }),
        CommitObjectResolution::WrongType { object_type } => {
            Err(GitError::WrongCommitExpressionType {
                expression: expression.to_owned(),
                object_type,
            })
        },
    }
}

/// Resolve `HEAD` and classify every candidate ancestor through one object batch.
pub(crate) fn head_commit_reachability(
    repository_root: &Path,
    candidate_ancestors: &[GitObjectId],
) -> Result<CommitTargetReachability, GitError> {
    commit_target_reachability(repository_root, GIT_HEAD_REVISION, candidate_ancestors)
        .map(|observation| observation.reachability)
}

/// Resolve one local branch and classify every candidate ancestor through one object batch.
pub(crate) fn branch_commit_reachability(
    repository_root: &Path,
    branch: &str,
    candidate_ancestors: &[GitObjectId],
) -> Result<CommitTargetReachabilityObservation, GitError> {
    commit_target_reachability(
        repository_root,
        &format!("{GIT_LOCAL_BRANCH_REF_PREFIX}{branch}"),
        candidate_ancestors,
    )
}

/// Whether one rename target was proven for a deleted local branch's object tip.
pub(crate) enum LocalBranchRenameTargetResolution {
    /// No local branch at the object proves a rename from the deleted branch.
    NotProven,
    /// Exactly one local branch at the object proves the rename.
    Unique(FullRefName),
    /// Several local branches prove the rename, so no single target can be chosen.
    Ambiguous,
}

/// Whether a local branch's newest reflog entry proves it replaced a deleted branch.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LocalBranchRenameProof {
    /// The newest reflog entry records the candidate's rename from the deleted branch.
    Recorded,
    /// The candidate has no matching newest reflog entry.
    NotRecorded,
}

/// Find whether exactly one local branch at `tip` has proof it replaced the deleted branch.
pub(crate) fn local_branch_rename_target_resolution(
    repository_root: &Path,
    tip: &GitObjectId,
    deleted_reference: &FullRefName,
) -> Result<LocalBranchRenameTargetResolution, GitError> {
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
    let mut proven_replacements = LocalBranchRenameTargetResolution::NotProven;
    for reference in references {
        match local_branch_rename_proof(repository_root, deleted_reference, &reference)? {
            LocalBranchRenameProof::Recorded => match proven_replacements {
                LocalBranchRenameTargetResolution::NotProven => {
                    proven_replacements = LocalBranchRenameTargetResolution::Unique(reference);
                },
                LocalBranchRenameTargetResolution::Unique(_)
                | LocalBranchRenameTargetResolution::Ambiguous => {
                    return Ok(LocalBranchRenameTargetResolution::Ambiguous);
                },
            },
            LocalBranchRenameProof::NotRecorded => {},
        }
    }
    Ok(proven_replacements)
}

/// Read whether `candidate_reference` records a rename from `deleted_reference`.
fn local_branch_rename_proof(
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
pub(crate) fn rewrite_in_progress(worktree_administrative_directory: &Path) -> bool {
    [GIT_REBASE_MERGE_STATE_PATH, GIT_REBASE_APPLY_STATE_PATH]
        .iter()
        .any(|state_path| worktree_administrative_directory.join(state_path).exists())
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
    output_availability: GitCommandOutputAvailability,
) -> Result<Output, ScopedPatchComparisonError> {
    match output_availability {
        GitCommandOutputAvailability::Available(output) => Ok(output),
        GitCommandOutputAvailability::Unavailable(error) => Err(GitError::Io(error).into()),
    }
}

/// Compare one protected phase's aggregate scoped change with a target history.
///
/// `phase_start_head` excludes earlier branch work from the protected side. The first query
/// submits every scope together and expands tree scopes only to paths changed by the protected
/// phase. The target commits carrying those changes must occupy one contiguous first-parent
/// interval. A three-way replay merges the complete `protected_tip` into `target` with
/// `phase_start_head` as the explicit merge base. Conflicts outside the reservation scopes are
/// permitted; a conflict covered by a scope rejects equivalence. The final diff is limited to the
/// reservation scopes, and the replay is equivalent when no covered path differs from `target`.
pub(crate) fn scoped_patch_equivalence(
    repository_root: &Path,
    phase_start_head: &GitObjectId,
    scopes: &ReservationScopeSet,
    protected_tip: &GitObjectId,
    target: &GitObjectId,
) -> Result<ScopedPatchComparison, GitError> {
    scoped_patch_equivalence_with_target_history(
        repository_root,
        phase_start_head,
        scopes,
        protected_tip,
        target,
        ScopedPatchTargetHistory::NeedsGitQueries,
    )
}

/// Compare scoped changes while reusing an admitted phase-start ancestry result.
pub(crate) fn scoped_patch_equivalence_with_target_history(
    repository_root: &Path,
    phase_start_head: &GitObjectId,
    scopes: &ReservationScopeSet,
    protected_tip: &GitObjectId,
    target: &GitObjectId,
    target_history: ScopedPatchTargetHistory<'_>,
) -> Result<ScopedPatchComparison, GitError> {
    match compare_scoped_patch(
        repository_root,
        phase_start_head,
        scopes,
        protected_tip,
        target,
        target_history,
    ) {
        Ok(scoped_patch_comparison) => Ok(scoped_patch_comparison),
        Err(ScopedPatchComparisonError::Git(GitError::Io(_))) => {
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
    target_history: ScopedPatchTargetHistory<'_>,
) -> Result<ScopedPatchComparison, ScopedPatchComparisonError> {
    let InitialScopedPatchEvidence {
        history_relationship,
        protected_scoped_changes,
        protected_scoped_replay,
    } = initial_scoped_patch_evidence(
        repository_root,
        phase_start_head,
        scopes,
        protected_tip,
        target,
        target_history,
    );
    match history_relationship? {
        HistoryRelationship::Shared => {},
        HistoryRelationship::Unrelated => return Ok(ScopedPatchComparison::Different),
        HistoryRelationship::Unavailable => return Ok(ScopedPatchComparison::Unavailable),
    }

    let (affected_paths, scoped_replay_rename_detection) = match protected_scoped_changes? {
        ProtectedScopedChanges::NoChanges => return Ok(ScopedPatchComparison::Different),
        ProtectedScopedChanges::Affected {
            paths,
            scoped_replay_rename_detection,
        } => (paths, scoped_replay_rename_detection),
        ProtectedScopedChanges::Unreadable => {
            return Ok(ScopedPatchComparison::Unavailable);
        },
    };

    let locate_target_scoped_commits = || {
        target_scoped_change_position(
            repository_root,
            phase_start_head,
            protected_tip,
            target,
            &affected_paths,
        )
    };
    let (target_scoped_change_position, protected_scoped_change) =
        match (scoped_replay_rename_detection, protected_scoped_replay) {
            (
                ScopedReplayRenameDetection::DisabledWithoutProtectedRename,
                ProtectedScopedReplayState::EvaluatedAssumingNoRename(protected_scoped_change),
            ) => (locate_target_scoped_commits(), protected_scoped_change),
            (
                ScopedReplayRenameDetection::RequiredForProtectedRename,
                ProtectedScopedReplayState::EvaluatedAssumingNoRename(_),
            ) => concurrent_scoped_patch_reads(
                locate_target_scoped_commits,
                "locate target scoped commits",
                || {
                    target_contains_protected_scoped_change(
                        repository_root,
                        phase_start_head,
                        protected_tip,
                        target,
                        scopes,
                        ScopedReplayRenameDetection::RequiredForProtectedRename,
                    )
                },
                "replay the protected scoped change with renames",
            ),
            (
                scoped_replay_rename_detection,
                ProtectedScopedReplayState::RequiredAfterRenameClassification,
            ) => concurrent_scoped_patch_reads(
                locate_target_scoped_commits,
                "locate target scoped commits",
                || {
                    target_contains_protected_scoped_change(
                        repository_root,
                        phase_start_head,
                        protected_tip,
                        target,
                        scopes,
                        scoped_replay_rename_detection,
                    )
                },
                "replay the protected scoped change",
            ),
        };
    match target_scoped_change_position? {
        TargetScopedChangePosition::Contiguous => {},
        TargetScopedChangePosition::Absent
        | TargetScopedChangePosition::Separated
        | TargetScopedChangePosition::Unproven => {
            return Ok(ScopedPatchComparison::Different);
        },
    }

    protected_scoped_change
}

fn initial_scoped_patch_evidence(
    repository_root: &Path,
    phase_start_head: &GitObjectId,
    scopes: &ReservationScopeSet,
    protected_tip: &GitObjectId,
    target: &GitObjectId,
    target_history: ScopedPatchTargetHistory<'_>,
) -> InitialScopedPatchEvidence {
    match target_history {
        ScopedPatchTargetHistory::ProvenFirstParentInterval { commits } => {
            debug_assert!(commits.iter().all(|commit| commit != phase_start_head));
            let (protected_scoped_changes, protected_scoped_replay) = concurrent_scoped_patch_reads(
                || {
                    protected_scoped_changes(
                        repository_root,
                        phase_start_head,
                        scopes,
                        protected_tip,
                    )
                },
                "identify protected scoped paths",
                || {
                    target_contains_protected_scoped_change(
                        repository_root,
                        phase_start_head,
                        protected_tip,
                        target,
                        scopes,
                        ScopedReplayRenameDetection::DisabledWithoutProtectedRename,
                    )
                },
                "replay the protected scoped change without renames",
            );
            InitialScopedPatchEvidence {
                history_relationship: Ok(HistoryRelationship::Shared),
                protected_scoped_changes,
                protected_scoped_replay: ProtectedScopedReplayState::EvaluatedAssumingNoRename(
                    protected_scoped_replay,
                ),
            }
        },
        ScopedPatchTargetHistory::NeedsGitQueries => {
            let (history_relationship, protected_scoped_changes) = concurrent_scoped_patch_reads(
                || history_relationship(repository_root, phase_start_head, target),
                "compare scoped history",
                || {
                    protected_scoped_changes(
                        repository_root,
                        phase_start_head,
                        scopes,
                        protected_tip,
                    )
                },
                "identify protected scoped paths",
            );
            InitialScopedPatchEvidence {
                history_relationship,
                protected_scoped_changes,
                protected_scoped_replay:
                    ProtectedScopedReplayState::RequiredAfterRenameClassification,
            }
        },
    }
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
    scoped_replay_rename_detection: ScopedReplayRenameDetection,
) -> Result<ScopedPatchComparison, ScopedPatchComparisonError> {
    let mut replay_arguments = vec![
        GIT_MERGE_TREE_COMMAND.to_owned(),
        GIT_WRITE_TREE_ARG.to_owned(),
        GIT_NUL_TERMINATED_ARG.to_owned(),
    ];
    match scoped_replay_rename_detection {
        ScopedReplayRenameDetection::DisabledWithoutProtectedRename => {
            replay_arguments.push(GIT_STRATEGY_OPTION_NO_RENAMES_ARG.to_owned());
        },
        ScopedReplayRenameDetection::RequiredForProtectedRename => {},
    }
    replay_arguments.extend([
        format!("{GIT_MERGE_BASE_ARG_PREFIX}{phase_start_head}"),
        target.to_string(),
        protected_tip.to_string(),
    ]);
    let replay_output =
        scoped_patch_command_output(git_output_dynamic(repository_root, &replay_arguments).into())?;
    match replay_output.status.code() {
        Some(GIT_MERGE_TREE_CLEAN_EXIT_CODE) => {},
        Some(GIT_MERGE_TREE_CONFLICT_EXIT_CODE) => {
            match scoped_merge_conflict_coverage(&replay_output.stdout, scopes, protected_tip) {
                ScopedMergeConflictCoverage::OutsideReservationScopes => {},
                ScopedMergeConflictCoverage::CoveredByReservation
                | ScopedMergeConflictCoverage::DisplacedReservedFile => {
                    return Ok(ScopedPatchComparison::Different);
                },
                ScopedMergeConflictCoverage::Unreadable => {
                    return Ok(ScopedPatchComparison::Unavailable);
                },
            }
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

    let mut diff_arguments = vec![
        GIT_DIFF_COMMAND.to_owned(),
        GIT_NAME_ONLY_ARG.to_owned(),
        GIT_NO_RENAMES_ARG.to_owned(),
        replayed_tree.to_string(),
        target.to_string(),
        GIT_PATHSPEC_SEPARATOR.to_owned(),
    ];
    diff_arguments.extend(
        scopes
            .as_slice()
            .iter()
            .map(|scope| format!("{GIT_LITERAL_TOP_PATHSPEC_PREFIX}{}", scope.path)),
    );
    let diff_output =
        scoped_patch_command_output(git_output_dynamic(repository_root, &diff_arguments).into())?;
    if !diff_output.status.success() {
        return Ok(ScopedPatchComparison::Unavailable);
    }
    Ok(if diff_output.stdout.is_empty() {
        ScopedPatchComparison::Equivalent
    } else {
        ScopedPatchComparison::Different
    })
}

fn scoped_merge_conflict_coverage(
    merge_tree_output: &[u8],
    scopes: &ReservationScopeSet,
    protected_tip: &GitObjectId,
) -> ScopedMergeConflictCoverage {
    let mut records = merge_tree_output.split(|byte| *byte == b'\0');
    let Some(tree_object_id) = records.next() else {
        return ScopedMergeConflictCoverage::Unreadable;
    };
    if tree_object_id.is_empty() {
        return ScopedMergeConflictCoverage::Unreadable;
    }

    let mut conflict_paths = Vec::new();
    let mut conflict_record_count = 0;
    loop {
        let Some(record) = records.next() else {
            return ScopedMergeConflictCoverage::Unreadable;
        };
        if record.is_empty() {
            if conflict_record_count == 0 {
                return ScopedMergeConflictCoverage::Unreadable;
            }
            break;
        }
        conflict_record_count += 1;
        let Some(path_separator) = record.iter().position(|byte| *byte == b'\t') else {
            return ScopedMergeConflictCoverage::Unreadable;
        };
        let metadata = &record[..path_separator];
        let path = &record[path_separator + 1..];
        if metadata.split(|byte| *byte == b' ').count() != 3 || path.is_empty() {
            return ScopedMergeConflictCoverage::Unreadable;
        }
        if scoped_merge_conflict_path_is_covered(path, scopes) {
            return ScopedMergeConflictCoverage::CoveredByReservation;
        }
        conflict_paths.push(path);
    }

    loop {
        let Some(path_count) = records.next() else {
            return ScopedMergeConflictCoverage::Unreadable;
        };
        if path_count.is_empty() {
            return ScopedMergeConflictCoverage::OutsideReservationScopes;
        }
        let Ok(path_count) = str::from_utf8(path_count) else {
            return ScopedMergeConflictCoverage::Unreadable;
        };
        let Ok(path_count) = path_count.parse::<usize>() else {
            return ScopedMergeConflictCoverage::Unreadable;
        };
        let mut message_paths = Vec::with_capacity(path_count);
        for _ in 0..path_count {
            let Some(path) = records.next() else {
                return ScopedMergeConflictCoverage::Unreadable;
            };
            if path.is_empty() {
                return ScopedMergeConflictCoverage::Unreadable;
            }
            message_paths.push(path);
        }
        let Some(conflict_type) = records.next() else {
            return ScopedMergeConflictCoverage::Unreadable;
        };
        let Some(message) = records.next() else {
            return ScopedMergeConflictCoverage::Unreadable;
        };
        if conflict_type.is_empty() || message.is_empty() {
            return ScopedMergeConflictCoverage::Unreadable;
        }
        if conflict_type == b"CONFLICT (file/directory)"
            && scoped_merge_displaced_reserved_file(
                &conflict_paths,
                &message_paths,
                scopes,
                protected_tip,
            )
        {
            return ScopedMergeConflictCoverage::DisplacedReservedFile;
        }
    }
}

fn scoped_merge_conflict_path_is_covered(
    conflict_path: &[u8],
    scopes: &ReservationScopeSet,
) -> bool {
    scopes.covers_path(conflict_path)
}

fn scoped_merge_displaced_reserved_file(
    conflict_paths: &[&[u8]],
    message_paths: &[&[u8]],
    scopes: &ReservationScopeSet,
    protected_tip: &GitObjectId,
) -> bool {
    scopes.as_slice().iter().any(|scope| {
        if scope.kind != ScopeKind::File {
            return false;
        }
        let reserved_path = scope.path.to_string();
        let displaced_path = format!("{reserved_path}~{protected_tip}");
        conflict_paths.contains(&displaced_path.as_bytes())
            && message_paths.contains(&reserved_path.as_bytes())
            && message_paths.contains(&displaced_path.as_bytes())
    })
}

fn target_scoped_change_position(
    repository_root: &Path,
    phase_start_head: &GitObjectId,
    protected_tip: &GitObjectId,
    target: &GitObjectId,
    affected_paths: &[String],
) -> Result<TargetScopedChangePosition, ScopedPatchComparisonError> {
    let (target_history, symmetric_difference) = concurrent_scoped_patch_reads(
        || target_first_parent_history(repository_root, phase_start_head, target, affected_paths),
        "walk target first-parent history",
        || {
            scoped_symmetric_difference(
                repository_root,
                phase_start_head,
                protected_tip,
                target,
                affected_paths,
            )
        },
        "compare protected and target scoped commits",
    );
    let target_history = target_history?;
    let symmetric_difference = symmetric_difference?;
    let target_phase_integration_commits =
        classify_target_phase_integration_commits(&target_history, &symmetric_difference);
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
            target_history
                .commits
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

fn classify_target_phase_integration_commits(
    target_history: &TargetFirstParentHistory,
    symmetric_difference: &ScopedSymmetricDifference,
) -> TargetPhaseIntegrationCommits {
    if target_history.scoped_commits.is_empty() {
        return TargetPhaseIntegrationCommits::Identified(Vec::new());
    }

    let mut identified_target_commits = target_history
        .scoped_commits
        .iter()
        .filter(|target_commit| {
            !symmetric_difference
                .target_unmatched_commits
                .contains(target_commit)
        })
        .cloned()
        .collect::<Vec<_>>();

    if matches!(
        symmetric_difference.protected_unmatched_commit,
        ProtectedUnmatchedCommit::Absent
    ) && !identified_target_commits.is_empty()
    {
        return TargetPhaseIntegrationCommits::Identified(identified_target_commits);
    }

    let unmatched_target_commits = target_history
        .scoped_commits
        .iter()
        .filter(|target_commit| !identified_target_commits.contains(target_commit))
        .cloned()
        .collect::<Vec<_>>();
    let [unmatched_target_commit] = unmatched_target_commits.as_slice() else {
        return TargetPhaseIntegrationCommits::Unresolved;
    };
    identified_target_commits.push(unmatched_target_commit.clone());
    let identified_target_commits = target_history
        .commits
        .iter()
        .filter(|target_commit| identified_target_commits.contains(target_commit))
        .cloned()
        .collect();
    TargetPhaseIntegrationCommits::Identified(identified_target_commits)
}

fn scoped_symmetric_difference(
    repository_root: &Path,
    phase_start_head: &GitObjectId,
    protected_tip: &GitObjectId,
    target: &GitObjectId,
    affected_paths: &[String],
) -> Result<ScopedSymmetricDifference, ScopedPatchComparisonError> {
    let mut arguments = vec![
        GIT_REV_LIST_COMMAND.to_owned(),
        GIT_CHERRY_MARK_ARG.to_owned(),
        GIT_LEFT_RIGHT_ARG.to_owned(),
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
    let output = scoped_rev_list(repository_root, &arguments)?;
    let mut protected_unmatched_commit = ProtectedUnmatchedCommit::Absent;
    let mut target_unmatched_commits = HashSet::new();
    for line in output.lines() {
        let mut characters = line.chars();
        let Some(commit_mark) = characters.next() else {
            continue;
        };
        let commit = characters
            .as_str()
            .parse()
            .map_err(GitError::InvalidObjectId)?;
        match commit_mark {
            GIT_LEFT_COMMIT_MARK => protected_unmatched_commit = ProtectedUnmatchedCommit::Present,
            GIT_RIGHT_COMMIT_MARK => {
                target_unmatched_commits.insert(commit);
            },
            GIT_EQUIVALENT_COMMIT_MARK => {},
            _ => {
                return Err(GitError::InvalidScopedHistoryLine {
                    line: line.to_owned(),
                }
                .into());
            },
        }
    }
    Ok(ScopedSymmetricDifference {
        protected_unmatched_commit,
        target_unmatched_commits,
    })
}

fn target_first_parent_history(
    repository_root: &Path,
    excluded_ancestor: &GitObjectId,
    tip: &GitObjectId,
    affected_paths: &[String],
) -> Result<TargetFirstParentHistory, ScopedPatchComparisonError> {
    let record_format = format!("--format=%x00{TARGET_FIRST_PARENT_RECORD_MARKER}%x00%H");
    let arguments = [
        GIT_LOG_COMMAND.to_owned(),
        GIT_NUL_TERMINATED_ARG.to_owned(),
        GIT_NAME_ONLY_ARG.to_owned(),
        GIT_NO_RENAMES_ARG.to_owned(),
        GIT_FIRST_PARENT_ARG.to_owned(),
        record_format,
        tip.to_string(),
        format!("{GIT_EXCLUDE_REVISION_PREFIX}{excluded_ancestor}"),
        GIT_PATHSPEC_SEPARATOR.to_owned(),
    ];
    let output =
        scoped_patch_command_output(git_output_dynamic(repository_root, &arguments).into())?;
    if !output.status.success() {
        return Err(GitError::CommandFailed {
            command: GIT_LOG_COMMAND,
            stderr:  String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        }
        .into());
    }
    parse_target_first_parent_history(&output.stdout, affected_paths)
}

fn parse_target_first_parent_history(
    output: &[u8],
    affected_paths: &[String],
) -> Result<TargetFirstParentHistory, ScopedPatchComparisonError> {
    let fields = output.split(|byte| *byte == b'\0').collect::<Vec<_>>();
    let mut commits = Vec::new();
    let mut scoped_commits = Vec::new();
    let mut index = 0;
    while index < fields.len() {
        while fields.get(index).is_some_and(|field| field.is_empty()) {
            index += 1;
        }
        if index == fields.len() {
            break;
        }
        if fields[index] != TARGET_FIRST_PARENT_RECORD_MARKER.as_bytes() {
            return Err(GitError::InvalidScopedHistoryLine {
                line: String::from_utf8_lossy(fields[index]).into_owned(),
            }
            .into());
        }
        index += 1;
        let Some(commit_field) = fields.get(index) else {
            return Err(GitError::InvalidScopedHistoryLine {
                line: "target history ended before its commit".to_owned(),
            }
            .into());
        };
        index += 1;
        let commit = str::from_utf8(commit_field)
            .map_err(|_| GitError::InvalidScopedHistoryLine {
                line: String::from_utf8_lossy(commit_field).into_owned(),
            })?
            .parse::<GitObjectId>()
            .map_err(GitError::InvalidObjectId)?;
        let mut affects_scope = false;
        while index < fields.len() {
            if fields[index].is_empty() {
                index += 1;
                if fields
                    .get(index)
                    .is_none_or(|field| *field == TARGET_FIRST_PARENT_RECORD_MARKER.as_bytes())
                {
                    break;
                }
                continue;
            }
            let path = fields[index].strip_prefix(b"\n").unwrap_or(fields[index]);
            affects_scope |= affected_paths
                .iter()
                .any(|affected_path| path == affected_path.as_bytes());
            index += 1;
        }
        if affects_scope {
            scoped_commits.push(commit.clone());
        }
        commits.push(commit);
    }
    Ok(TargetFirstParentHistory {
        commits,
        scoped_commits,
    })
}

const TARGET_FIRST_PARENT_RECORD_MARKER: &str = "cargo-berth-target-first-parent";

fn protected_scoped_changes(
    repository_root: &Path,
    phase_start_head: &GitObjectId,
    scopes: &ReservationScopeSet,
    protected_tip: &GitObjectId,
) -> Result<ProtectedScopedChanges, ScopedPatchComparisonError> {
    let mut arguments = vec![
        GIT_DIFF_COMMAND.to_owned(),
        GIT_NAME_STATUS_ARG.to_owned(),
        GIT_NUL_TERMINATED_ARG.to_owned(),
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
    let mut fields = output_text.split('\0');
    let mut affected_paths = Vec::new();
    let mut scoped_replay_rename_detection =
        ScopedReplayRenameDetection::DisabledWithoutProtectedRename;
    loop {
        let Some(status) = fields.next() else {
            return Ok(ProtectedScopedChanges::Unreadable);
        };
        if status.is_empty() {
            if fields.next().is_some() {
                return Ok(ProtectedScopedChanges::Unreadable);
            }
            break;
        }
        let Some(first_path) = fields.next() else {
            return Ok(ProtectedScopedChanges::Unreadable);
        };
        if first_path.is_empty() {
            return Ok(ProtectedScopedChanges::Unreadable);
        }
        if status.starts_with('R') {
            let Some(second_path) = fields.next() else {
                return Ok(ProtectedScopedChanges::Unreadable);
            };
            if second_path.is_empty() {
                return Ok(ProtectedScopedChanges::Unreadable);
            }
            let source_is_covered = scopes.covers_path(first_path.as_bytes());
            let destination_is_covered = scopes.covers_path(second_path.as_bytes());
            if source_is_covered && destination_is_covered {
                scoped_replay_rename_detection =
                    ScopedReplayRenameDetection::RequiredForProtectedRename;
            }
            if source_is_covered {
                affected_paths.push(first_path.to_owned());
            }
            if destination_is_covered {
                affected_paths.push(second_path.to_owned());
            }
        } else if status.starts_with('C') {
            let Some(copied_path) = fields.next() else {
                return Ok(ProtectedScopedChanges::Unreadable);
            };
            if copied_path.is_empty() {
                return Ok(ProtectedScopedChanges::Unreadable);
            }
            if scopes.covers_path(copied_path.as_bytes()) {
                affected_paths.push(copied_path.to_owned());
            }
        } else if scopes.covers_path(first_path.as_bytes()) {
            affected_paths.push(first_path.to_owned());
        }
    }
    if affected_paths.is_empty() {
        Ok(ProtectedScopedChanges::NoChanges)
    } else {
        Ok(ProtectedScopedChanges::Affected {
            paths: affected_paths,
            scoped_replay_rename_detection,
        })
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

/// Resolve a full reference while preserving Git failures separately from a missing reference.
pub(crate) fn reference_lookup(
    repository_root: &Path,
    reference: &str,
) -> Result<ReferenceLookup, GitError> {
    reference
        .parse::<FullRefName>()
        .map_err(|_| GitError::InvalidReferenceName {
            reference: reference.to_owned(),
        })?;
    let existence_output = git_output(
        repository_root,
        [GIT_SHOW_REF_COMMAND, GIT_SHOW_REF_EXISTS_ARG, reference],
    )?;
    if !existence_output.status.success() {
        if existence_output.status.code() == Some(GIT_MISSING_REFERENCE_EXIT_CODE) {
            return Ok(ReferenceLookup::Missing);
        }
        return Err(GitError::CommandFailed {
            command: GIT_SHOW_REF_COMMAND,
            stderr:  String::from_utf8_lossy(&existence_output.stderr)
                .trim()
                .to_owned(),
        });
    }

    let output = git_output(repository_root, [GIT_REV_PARSE_COMMAND, reference])?;
    if !output.status.success() {
        return Err(GitError::CommandFailed {
            command: GIT_REV_PARSE_COMMAND,
            stderr:  String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
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

#[allow(
    clippy::too_many_lines,
    reason = "the resolved and three unavailable target states share one candidate batch"
)]
fn commit_target_reachability(
    repository_root: &Path,
    target_expression: &str,
    candidate_ancestors: &[GitObjectId],
) -> Result<CommitTargetReachabilityObservation, GitError> {
    let candidate_expressions = candidate_ancestors
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    if candidate_expressions.is_empty() {
        let [target_resolution] =
            commit_object_resolutions(repository_root, &[target_expression.to_owned()])?
                .try_into()
                .map_err(|resolutions: Vec<_>| GitError::InvalidBatchObjectCount {
                    expected: 1,
                    actual:   resolutions.len(),
                })?;
        let reachability = match target_resolution {
            CommitObjectResolution::Resolved(target) => CommitTargetReachability::Resolved {
                target,
                candidates: Vec::new(),
            },
            CommitObjectResolution::Missing => CommitTargetReachability::Missing,
            CommitObjectResolution::Ambiguous => CommitTargetReachability::Ambiguous,
            CommitObjectResolution::WrongType { object_type } => {
                CommitTargetReachability::WrongType { object_type }
            },
        };
        return Ok(CommitTargetReachabilityObservation {
            reachability,
            resolved_candidates: ResolvedBatchCommitCandidates::default(),
            target_histories: PhaseStartTargetFirstParentHistories::default(),
        });
    }
    let (target_history, candidate_resolutions) = thread::scope(|scope| {
        let target_history_worker =
            scope.spawn(|| target_commit_history(repository_root, target_expression));
        let candidate_resolution_worker =
            scope.spawn(|| commit_object_resolutions(repository_root, &candidate_expressions));
        let target_history =
            target_history_worker
                .join()
                .map_err(|_| GitError::ConcurrentReadWorkerPanicked {
                    activity: "read target commit history",
                })?;
        let candidate_resolutions = candidate_resolution_worker.join().map_err(|_| {
            GitError::ConcurrentReadWorkerPanicked {
                activity: "resolve candidate commit objects",
            }
        })??;
        Ok::<_, GitError>((target_history, candidate_resolutions))
    })?;
    let resolved_candidates = ResolvedBatchCommitCandidates(
        candidate_resolutions
            .iter()
            .filter_map(|resolution| match resolution {
                CommitObjectResolution::Resolved(candidate) => Some(candidate.clone()),
                CommitObjectResolution::Missing
                | CommitObjectResolution::Ambiguous
                | CommitObjectResolution::WrongType { .. } => None,
            })
            .collect(),
    );
    let ResolvedTargetCommitHistory {
        target,
        graph: target_history,
    } = match target_history {
        Ok(target_history) => target_history,
        Err(history_error) => {
            let [target_resolution] =
                commit_object_resolutions(repository_root, &[target_expression.to_owned()])?
                    .try_into()
                    .map_err(|resolutions: Vec<_>| GitError::InvalidBatchObjectCount {
                        expected: 1,
                        actual:   resolutions.len(),
                    })?;
            match target_resolution {
                CommitObjectResolution::Resolved(_) => return Err(history_error),
                CommitObjectResolution::Missing => {
                    return Ok(CommitTargetReachabilityObservation {
                        reachability: CommitTargetReachability::Missing,
                        resolved_candidates,
                        target_histories: PhaseStartTargetFirstParentHistories::default(),
                    });
                },
                CommitObjectResolution::Ambiguous => {
                    return Ok(CommitTargetReachabilityObservation {
                        reachability: CommitTargetReachability::Ambiguous,
                        resolved_candidates,
                        target_histories: PhaseStartTargetFirstParentHistories::default(),
                    });
                },
                CommitObjectResolution::WrongType { object_type } => {
                    return Ok(CommitTargetReachabilityObservation {
                        reachability: CommitTargetReachability::WrongType { object_type },
                        resolved_candidates,
                        target_histories: PhaseStartTargetFirstParentHistories::default(),
                    });
                },
            }
        },
    };
    if candidate_resolutions.is_empty() {
        return Ok(CommitTargetReachabilityObservation {
            reachability: CommitTargetReachability::Resolved {
                target,
                candidates: Vec::new(),
            },
            resolved_candidates,
            target_histories: PhaseStartTargetFirstParentHistories::default(),
        });
    }
    let target_histories = PhaseStartTargetFirstParentHistories(
        candidate_resolutions
            .iter()
            .filter_map(|resolution| match resolution {
                CommitObjectResolution::Resolved(candidate)
                    if target_history.contains(candidate) =>
                {
                    Some((
                        candidate.clone(),
                        target_history.first_parent_commits_after(&target, candidate),
                    ))
                },
                CommitObjectResolution::Resolved(_)
                | CommitObjectResolution::Missing
                | CommitObjectResolution::Ambiguous
                | CommitObjectResolution::WrongType { .. } => None,
            })
            .collect(),
    );
    let candidates = candidate_resolutions
        .into_iter()
        .map(|resolution| match resolution {
            CommitObjectResolution::Resolved(candidate) if target_history.contains(&candidate) => {
                CommitCandidateReachability::Ancestor
            },
            CommitObjectResolution::Resolved(_) => CommitCandidateReachability::NotAncestor,
            CommitObjectResolution::Missing => CommitCandidateReachability::Missing,
            CommitObjectResolution::Ambiguous => CommitCandidateReachability::Ambiguous,
            CommitObjectResolution::WrongType { object_type } => {
                CommitCandidateReachability::WrongType { object_type }
            },
        })
        .collect();
    Ok(CommitTargetReachabilityObservation {
        reachability: CommitTargetReachability::Resolved { target, candidates },
        resolved_candidates,
        target_histories,
    })
}

fn target_commit_history(
    repository_root: &Path,
    target_expression: &str,
) -> Result<ResolvedTargetCommitHistory, GitError> {
    let arguments = [
        GIT_REV_LIST_COMMAND.to_owned(),
        GIT_PARENTS_ARG.to_owned(),
        target_expression.to_owned(),
    ];
    let output = completed_git_command(git_output_dynamic(repository_root, &arguments).into())?;
    if !output.status.success() {
        return Err(GitError::CommandFailed {
            command: GIT_REV_LIST_COMMAND,
            stderr:  String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    let output_text = String::from_utf8(output.stdout).map_err(GitError::InvalidOutput)?;
    let target = output_text
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().next())
        .ok_or(GitError::MissingTargetCommitHistory)?
        .parse()
        .map_err(GitError::InvalidObjectId)?;
    let graph = CommitAncestryGraph::try_from(output_text.as_str())?;
    Ok(ResolvedTargetCommitHistory { target, graph })
}

fn commit_object_resolutions(
    repository_root: &Path,
    expressions: &[String],
) -> Result<Vec<CommitObjectResolution>, GitError> {
    let input = expressions
        .iter()
        .fold(String::new(), |mut input, expression| {
            let _ = writeln!(input, "{expression}");
            input
        });
    let arguments = [
        GIT_CAT_FILE_COMMAND.to_owned(),
        GIT_BATCH_CHECK_OBJECT_FORMAT_ARG.to_owned(),
    ];
    let output = completed_git_command(
        git_output_dynamic_with_input(repository_root, &arguments, input.as_bytes()).into(),
    )?;
    if !output.status.success() {
        return Err(GitError::CommandFailed {
            command: GIT_CAT_FILE_COMMAND,
            stderr:  String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    let output_text = String::from_utf8(output.stdout).map_err(GitError::InvalidOutput)?;
    let resolutions = output_text
        .lines()
        .map(commit_object_resolution)
        .collect::<Result<Vec<_>, _>>()?;
    if resolutions.len() != expressions.len() {
        return Err(GitError::InvalidBatchObjectCount {
            expected: expressions.len(),
            actual:   resolutions.len(),
        });
    }
    Ok(resolutions)
}

fn commit_object_resolution(line: &str) -> Result<CommitObjectResolution, GitError> {
    if line.ends_with(GIT_MISSING_OBJECT_SUFFIX) {
        return Ok(CommitObjectResolution::Missing);
    }
    if line.ends_with(GIT_AMBIGUOUS_OBJECT_SUFFIX) {
        return Ok(CommitObjectResolution::Ambiguous);
    }
    let Some((object_id, object_type)) = line.split_once(' ') else {
        return Err(GitError::InvalidBatchObjectLine {
            line: line.to_owned(),
        });
    };
    if object_type != GIT_COMMIT_OBJECT_TYPE {
        return Ok(CommitObjectResolution::WrongType {
            object_type: object_type.to_owned(),
        });
    }
    object_id
        .parse()
        .map(CommitObjectResolution::Resolved)
        .map_err(GitError::InvalidObjectId)
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
    let target_history = target_commit_history(repository_root, &target.to_string())?.graph;
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

/// Compare every readable phase start with one target in a single git invocation.
///
/// `diff-tree --stdin` prefixes every non-empty result with the first supplied
/// object, so each input line starts with its distinct phase-start anchor. Empty
/// comparisons emit no record and remain distinguishable because callers
/// initialize every requested anchor before parsing the output.
pub(crate) fn phase_committed_path_diffs(
    repository_root: &Path,
    anchors: &[GitObjectId],
    target: &GitObjectId,
) -> GitCommandOutputAvailability {
    let input = anchors.iter().fold(String::new(), |mut input, anchor| {
        let _ = writeln!(input, "{anchor} {target}");
        input
    });
    let arguments = [
        GIT_DIFF_TREE_COMMAND.to_owned(),
        GIT_STDIN_ARG.to_owned(),
        GIT_RECURSIVE_ARG.to_owned(),
        GIT_NAME_STATUS_ARG.to_owned(),
        GIT_NUL_TERMINATED_ARG.to_owned(),
        GIT_NO_RENAMES_ARG.to_owned(),
    ];
    git_output_dynamic_with_input(repository_root, &arguments, input.as_bytes()).into()
}

/// Read every selected path's commits for later per-anchor membership filtering.
pub(crate) fn incursion_path_log(
    repository_root: &Path,
    target: &GitObjectId,
    paths: &[ReservationScopePath],
) -> IncursionPathLogInvocation {
    let record_format = format!("--format=%x00{INCURSION_ATTRIBUTION_RECORD_MARKER}%x00%H%x00%s");
    let mut arguments = Vec::with_capacity(paths.len() + 8);
    arguments.extend([
        GIT_LOG_COMMAND.to_owned(),
        GIT_NUL_TERMINATED_ARG.to_owned(),
        GIT_NAME_ONLY_ARG.to_owned(),
        GIT_NO_RENAMES_ARG.to_owned(),
        GIT_DENSE_COMBINED_ARG.to_owned(),
        record_format,
        target.to_string(),
        GIT_PATHSPEC_SEPARATOR.to_owned(),
    ]);
    arguments.extend(
        paths
            .iter()
            .map(|path| format!("{GIT_LITERAL_TOP_PATHSPEC_PREFIX}{path}")),
    );
    let output_availability = git_output_dynamic(repository_root, &arguments).into();
    IncursionPathLogInvocation {
        arguments,
        output_availability,
    }
}

/// The record boundary emitted by the batched incursion-attribution log.
pub(crate) const INCURSION_ATTRIBUTION_RECORD_MARKER: &str = "cargo-berth-incursion-commit";

/// Read each phase start's complete `anchor..target` membership through one graph.
pub(crate) fn incursion_range_commits(
    repository_root: &Path,
    anchors: &[GitObjectId],
    target: &GitObjectId,
) -> Result<Vec<HashSet<GitObjectId>>, GitError> {
    let requested_objects = std::iter::once(target)
        .chain(anchors)
        .cloned()
        .collect::<Vec<_>>();
    let input = requested_objects
        .iter()
        .fold(String::new(), |mut input, object_id| {
            let _ = writeln!(input, "{object_id}");
            input
        });
    let arguments = [
        GIT_REV_LIST_COMMAND.to_owned(),
        GIT_IGNORE_MISSING_ARG.to_owned(),
        GIT_PARENTS_ARG.to_owned(),
        GIT_STDIN_ARG.to_owned(),
    ];
    let output = completed_git_command(
        git_output_dynamic_with_input(repository_root, &arguments, input.as_bytes()).into(),
    )?;
    if !output.status.success() {
        return Err(GitError::CommandFailed {
            command: GIT_REV_LIST_COMMAND,
            stderr:  String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    let output_text = String::from_utf8(output.stdout).map_err(GitError::InvalidOutput)?;
    let commit_ancestry_graph = CommitAncestryGraph::try_from(output_text.as_str())?;
    let target_history = commit_ancestry_graph.ancestors_including(target);
    Ok(anchors
        .iter()
        .map(|anchor| {
            if !commit_ancestry_graph.contains(anchor) {
                return HashSet::new();
            }
            let anchor_history = commit_ancestry_graph.ancestors_including(anchor);
            target_history
                .difference(&anchor_history)
                .cloned()
                .collect()
        })
        .collect())
}

/// List target commits that are not reachable from the origin-classification basis.
pub(crate) fn commits_outside_origin_basis(
    repository_root: &Path,
    origin_basis: &GitObjectId,
    target: &GitObjectId,
) -> Result<HashSet<GitObjectId>, GitError> {
    let range = format!("{origin_basis}{GIT_ANCESTOR_RANGE_INFIX}{target}");
    let arguments = [GIT_REV_LIST_COMMAND.to_owned(), range];
    let output = completed_git_command(git_output_dynamic(repository_root, &arguments).into())?;
    if !output.status.success() {
        return Err(GitError::CommandFailed {
            command: GIT_REV_LIST_COMMAND,
            stderr:  String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    let output_text = String::from_utf8(output.stdout).map_err(GitError::InvalidOutput)?;
    output_text
        .lines()
        .map(str::parse)
        .collect::<Result<HashSet<_>, _>>()
        .map_err(GitError::InvalidObjectId)
}

fn completed_git_command(
    output_availability: GitCommandOutputAvailability,
) -> Result<Output, GitError> {
    match output_availability {
        GitCommandOutputAvailability::Available(output) => Ok(output),
        GitCommandOutputAvailability::Unavailable(error) => Err(GitError::Io(error)),
    }
}

/// Classify successor heads against every protected predecessor tip in one revision walk.
pub(crate) fn descendant_commits(
    repository_root: &Path,
    predecessors: &[ProtectedTipSuccessorHeads<'_>],
) -> Result<Vec<ProtectedTipSuccessorHeadClassification>, GitError> {
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
    let output = completed_git_command(
        git_output_dynamic_with_input(repository_root, &arguments, input.as_bytes()).into(),
    )?;
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
                return ProtectedTipSuccessorHeadClassification::AncestorObjectUnknown;
            }
            ProtectedTipSuccessorHeadClassification::Classified(
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
                            CandidateHeadReachability::Descendant {
                                head:                                successor_head.clone(),
                                first_parent_commits_after_ancestor: commit_ancestry_graph
                                    .first_parent_commits_after(
                                        successor_head,
                                        predecessor.protected_tip,
                                    ),
                            }
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
    let output = completed_git_command(
        git_output_dynamic_with_input(repository_root, &arguments, input.as_bytes()).into(),
    )?;
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

/// Apply all retention-ref repairs and deletions in one ref-mutating transaction.
pub(crate) fn update_reservation_retention_refs(
    repository_root: &Path,
    repairs: &[ReservationRetentionRefRepair],
    deletions: &[ReservationId],
) -> Result<(), GitError> {
    if repairs.is_empty() && deletions.is_empty() {
        return Ok(());
    }
    let protected_tips = repairs
        .iter()
        .map(|repair| repair.protected_tip.clone())
        .collect::<Vec<_>>();
    let availability = if protected_tips.is_empty() {
        Vec::new()
    } else {
        commit_availability(repository_root, &protected_tips)?
    };
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
    let input = deletions.iter().fold(input, |mut input, reservation_id| {
        let _ = writeln!(input, "delete {}", refs::name(*reservation_id));
        input
    });
    if input.is_empty() {
        return Ok(());
    }
    refs::apply_transaction(repository_root, &input)
}

/// Apply retention changes using candidate commits resolved earlier in the same locked pass.
pub(crate) fn update_reservation_retention_refs_from_resolved_batch(
    repository_root: &Path,
    repairs: &[ReservationRetentionRefRepair],
    deletions: &[ReservationId],
    resolved_candidates: &ResolvedBatchCommitCandidates,
) -> Result<(), GitError> {
    let input = repairs
        .iter()
        .filter(|repair| resolved_candidates.contains(&repair.protected_tip))
        .fold(String::new(), |mut input, repair| {
            let _ = writeln!(
                input,
                "update {} {}",
                refs::name(repair.reservation_id),
                repair.protected_tip
            );
            input
        });
    let input = deletions.iter().fold(input, |mut input, reservation_id| {
        let _ = writeln!(input, "delete {}", refs::name(*reservation_id));
        input
    });
    if input.is_empty() {
        return Ok(());
    }
    refs::apply_transaction(repository_root, &input)
}

/// Return the full private ref used to retain one reservation's protected tip.
pub(crate) fn reservation_retention_ref_name(reservation_id: ReservationId) -> String {
    refs::name(reservation_id)
}

fn object_id(repository_root: &Path, revision: &str) -> Result<GitObjectId, GitError> {
    let output = completed_git_command(
        git_output(repository_root, [GIT_REV_PARSE_COMMAND, revision]).into(),
    )?;
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

/// One incursion path-log invocation and the exact arguments supplied to git.
pub(crate) struct IncursionPathLogInvocation {
    /// The arguments supplied after the git binary.
    pub(crate) arguments:           Vec<String>,
    /// Whether that invocation left a process output available.
    pub(crate) output_availability: GitCommandOutputAvailability,
}

/// One candidate head's relation to a protected predecessor tip.
pub(crate) enum CandidateHeadReachability {
    /// The candidate head contains the protected predecessor tip.
    Descendant {
        /// The classified candidate head.
        head:                                GitObjectId,
        /// Its first-parent interval after the queried ancestor.
        first_parent_commits_after_ancestor: Vec<GitObjectId>,
    },
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
pub(crate) enum ProtectedTipSuccessorHeadClassification {
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
    /// A supplied or returned full reference name is invalid.
    InvalidReferenceName { reference: String },
    /// `cat-file --batch-check` did not classify every submitted object.
    InvalidBatchObjectCount { expected: usize, actual: usize },
    /// `cat-file --batch-check` printed a record without an object status or type.
    InvalidBatchObjectLine { line: String },
    /// A batched scoped-history query printed a record outside its typed grammar.
    InvalidScopedHistoryLine { line: String },
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
    /// A target-history read completed without identifying its requested tip.
    MissingTargetCommitHistory,
    /// No object resolves from a required commit expression.
    MissingCommitExpression { expression: String },
    /// More than one object matches a required commit expression.
    AmbiguousCommitExpression { expression: String },
    /// A required commit expression resolves to another object type.
    WrongCommitExpressionType {
        expression:  String,
        object_type: String,
    },
    /// One independent Git read ended before returning its typed observation.
    ConcurrentReadWorkerPanicked { activity: &'static str },
    /// One parallel scoped-proof worker ended before returning its typed observation.
    ScopedPatchWorkerPanicked { activity: &'static str },
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
                write!(formatter, "invalid full git reference name: {reference:?}")
            },
            Self::InvalidBatchObjectCount { expected, actual } => write!(
                formatter,
                "git cat-file classified {actual} objects when {expected} were submitted"
            ),
            Self::InvalidBatchObjectLine { line } => {
                write!(
                    formatter,
                    "git cat-file printed an invalid object record: {line:?}"
                )
            },
            Self::InvalidScopedHistoryLine { line } => {
                write!(
                    formatter,
                    "git printed an invalid scoped-history record: {line:?}"
                )
            },
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
            Self::MissingTargetCommitHistory => {
                formatter.write_str("git returned an empty target commit history")
            },
            Self::MissingCommitExpression { expression } => {
                write!(
                    formatter,
                    "git commit expression {expression:?} does not resolve"
                )
            },
            Self::AmbiguousCommitExpression { expression } => {
                write!(
                    formatter,
                    "git commit expression {expression:?} is ambiguous"
                )
            },
            Self::WrongCommitExpressionType {
                expression,
                object_type,
            } => write!(
                formatter,
                "git commit expression {expression:?} resolves to a {object_type} object"
            ),
            Self::ConcurrentReadWorkerPanicked { activity } => {
                write!(
                    formatter,
                    "git read worker panicked while attempting to {activity}"
                )
            },
            Self::ScopedPatchWorkerPanicked { activity } => {
                write!(
                    formatter,
                    "scoped patch worker panicked while attempting to {activity}"
                )
            },
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
    use super::CommitObjectResolution;
    use super::GitError;
    use super::ProtectedTipSuccessorHeadClassification;
    use super::ProtectedTipSuccessorHeads;
    use super::ScopedPatchComparison;
    use super::ScopedPatchComparisonError;
    use super::ScopedPatchTargetHistory;
    use super::ahead_behind_for_heads;
    use super::command::GitCommandOutputAvailability;
    use super::commit_object_resolution;
    use super::concurrent_scoped_patch_reads;
    use super::descendant_commits;
    use super::head_object_id;
    use super::scoped_patch_command_output;
    use super::scoped_patch_equivalence;
    use super::scoped_patch_equivalence_with_target_history;
    use crate::ids::GitObjectId;
    use crate::ledger::ProtectedPhaseStartHead;
    use crate::ledger::ReservationScope;
    use crate::reservation;
    use crate::reservation::IntegrationEvidenceStatus;
    use crate::reservation::IntegrationProof;
    use crate::reservation::PriorIntegrationStatus;
    use crate::reservation::ProtectedReservationTip;
    use crate::scope::ReservationScopeSet;
    use crate::scope::ScopeKind;

    const INITIAL_PRIMARY: &str = "first\nsecond\nthird\n";
    const INITIAL_SECONDARY: &str = "secondary\n";
    const PRIMARY_BACKUP_PATH: &str = "src/primary.rs~backup";
    const PRIMARY_PATH: &str = "src/primary.rs";
    const SCRIPT_PATH: &str = "scripts/run.sh";
    const SECONDARY_PATH: &str = "src/secondary.rs";
    const UNAVAILABLE_OBJECT_ID: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    type FixtureResult<T = ()> = Result<T, Box<dyn Error>>;

    #[test]
    fn batch_object_records_preserve_every_resolution_failure() -> FixtureResult {
        let object_id = UNAVAILABLE_OBJECT_ID.parse::<GitObjectId>()?;
        assert_eq!(
            commit_object_resolution(&format!("{object_id} commit"))?,
            CommitObjectResolution::Resolved(object_id.clone())
        );
        assert_eq!(
            commit_object_resolution("missing-expression missing")?,
            CommitObjectResolution::Missing
        );
        assert_eq!(
            commit_object_resolution("ambiguous-expression ambiguous")?,
            CommitObjectResolution::Ambiguous
        );
        assert_eq!(
            commit_object_resolution(&format!("{object_id} tree"))?,
            CommitObjectResolution::WrongType {
                object_type: "tree".to_owned(),
            }
        );
        Ok(())
    }

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
    fn concurrent_scoped_patch_read_maps_a_worker_panic_to_git_error() {
        let (panicked_read, completed_read) = concurrent_scoped_patch_reads(
            || -> Result<(), ScopedPatchComparisonError> {
                std::panic::resume_unwind(Box::new("scoped patch worker fixture"))
            },
            "read panicking fixture",
            || Ok(()),
            "read completed fixture",
        );

        assert!(matches!(
            panicked_read,
            Err(ScopedPatchComparisonError::Git(
                GitError::ScopedPatchWorkerPanicked {
                    activity: "read panicking fixture",
                }
            ))
        ));
        assert!(completed_read.is_ok());
    }

    #[test]
    fn proven_target_history_keeps_a_shared_scoped_commit() -> FixtureResult {
        let fixture = PatchEquivalenceFixture::new()?;
        fixture.write(PRIMARY_PATH, "shared scoped change\n")?;
        let shared_scoped_commit = fixture.commit("shared scoped change")?;
        fixture.write(SECONDARY_PATH, "protected unscoped change\n")?;
        let protected_tip = fixture.commit("protected unscoped change")?;

        fixture.reset_to(&shared_scoped_commit)?;
        fixture.write(SCRIPT_PATH, "#!/bin/sh\necho target\n")?;
        let target = fixture.commit("target unscoped change")?;
        let target_history = [target.clone(), shared_scoped_commit];
        let scopes = file_scopes(&[PRIMARY_PATH])?;

        let proven_comparison = scoped_patch_equivalence_with_target_history(
            fixture.root(),
            &fixture.phase_start_head,
            &scopes,
            &protected_tip,
            &target,
            ScopedPatchTargetHistory::ProvenFirstParentInterval {
                commits: &target_history,
            },
        )?;
        let queried_comparison = fixture.equivalence(&scopes, &protected_tip, &target)?;

        assert_eq!(proven_comparison, ScopedPatchComparison::Equivalent);
        assert_eq!(proven_comparison, queried_comparison);
        Ok(())
    }

    #[test]
    fn scoped_patch_command_spawn_failure_is_unavailable() -> Result<(), Box<dyn Error>> {
        let spawn_failure = Command::new("cargo-berth-missing-git")
            .output()
            .err()
            .ok_or_else(|| io::Error::other("the missing git fixture should fail to spawn"))?;
        let expected_kind = spawn_failure.kind();
        let expected_message = spawn_failure.to_string();
        let output_availability = GitCommandOutputAvailability::from(Err(spawn_failure));
        let comparison = scoped_patch_command_output(output_availability);

        assert!(matches!(
            &comparison,
            Err(ScopedPatchComparisonError::Git(GitError::Io(_)))
        ));
        assert!(matches!(
            comparison,
            Err(ScopedPatchComparisonError::Git(GitError::Io(error)))
                if error.kind() == expected_kind && error.to_string() == expected_message
        ));
        Ok(())
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
                ProtectedTipSuccessorHeadClassification::Classified(mixed),
                ProtectedTipSuccessorHeadClassification::AncestorObjectUnknown,
                ProtectedTipSuccessorHeadClassification::Classified(unrelated),
            ] if matches!(
                mixed.as_slice(),
                [
                    CandidateHeadReachability::Descendant {
                        head: classified_descendant,
                        ..
                    },
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
        let status = reservation::integration_status(
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
        let status = reservation::integration_status(
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
            reservation::integration_status(
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
    fn integrated_rename_with_later_target_edit_preserves_proof() -> FixtureResult {
        let fixture = PatchEquivalenceFixture::new()?;
        fixture.git(&["mv", PRIMARY_PATH, "src/renamed.rs"])?;
        let protected_tip = fixture.commit("protected rename")?;
        fixture.amend("amended rename")?;
        fixture.write(
            "src/renamed.rs",
            "first\nsecond\nthird\nlater target edit\n",
        )?;
        let target = fixture.commit("later edit after integrated rename")?;

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
    fn file_scope_ignores_conflict_at_tracked_tilde_sibling() -> FixtureResult {
        let mut fixture = PatchEquivalenceFixture::new()?;
        fixture.write(PRIMARY_BACKUP_PATH, "tracked sibling baseline\n")?;
        fixture.phase_start_head = fixture.commit("track tilde sibling")?;
        fixture.write(PRIMARY_PATH, "integrated protected content\n")?;
        fixture.write(PRIMARY_BACKUP_PATH, "protected sibling content\n")?;
        let protected_tip = fixture.commit("protected scoped and sibling modifications")?;
        fixture.reset_to_phase_start()?;
        fixture.write(PRIMARY_PATH, "integrated protected content\n")?;
        fixture.write(PRIMARY_BACKUP_PATH, "target sibling content\n")?;
        let target = fixture.commit("target scoped and sibling modifications")?;

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
