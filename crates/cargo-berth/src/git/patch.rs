//! Patch equivalence between a protected phase and a target history.
//!
//! Two questions rest on that one primitive. Comparison asks whether a target history
//! already carries a protected phase's aggregate scoped change, and carries it in one
//! contiguous integration. Anchoring asks where a rebased history's replacement for a
//! phase's former tip sits, walking the target's first-parent commits for as long as they
//! stay patch equivalents of the phase. Both answers come from git's own notion of an
//! equivalent commit, so both rest on the same range and revision-walk queries.

use std::collections::HashSet;
use std::path::Path;
use std::process::Output;
use std::thread;
use std::thread::ScopedJoinHandle;

use super::command;
use super::command::GitCommandOutputAvailability;
use super::conflict;
use super::conflict::ScopedMergeConflictCoverage;
use super::constants::GIT_ANCESTOR_RANGE_INFIX;
use super::constants::GIT_CHERRY_MARK_ARG;
use super::constants::GIT_COUNT_ARG;
use super::constants::GIT_DIFF_COMMAND;
use super::constants::GIT_EQUIVALENT_COMMIT_MARK;
use super::constants::GIT_EXCLUDE_REVISION_PREFIX;
use super::constants::GIT_FIRST_PARENT_ANCESTOR_INFIX;
use super::constants::GIT_FIRST_PARENT_ARG;
use super::constants::GIT_LEFT_COMMIT_MARK;
use super::constants::GIT_LEFT_RIGHT_ARG;
use super::constants::GIT_LITERAL_TOP_PATHSPEC_PREFIX;
use super::constants::GIT_LOG_COMMAND;
use super::constants::GIT_MAX_COUNT_ARG_PREFIX;
use super::constants::GIT_MERGE_BASE_ARG_PREFIX;
use super::constants::GIT_MERGE_BASE_COMMAND;
use super::constants::GIT_MERGE_TREE_CLEAN_EXIT_CODE;
use super::constants::GIT_MERGE_TREE_COMMAND;
use super::constants::GIT_MERGE_TREE_CONFLICT_EXIT_CODE;
use super::constants::GIT_NAME_ONLY_ARG;
use super::constants::GIT_NAME_STATUS_ARG;
use super::constants::GIT_NO_MERGE_BASE_EXIT_CODE;
use super::constants::GIT_NO_MERGES_ARG;
use super::constants::GIT_NO_RENAMES_ARG;
use super::constants::GIT_NUL_TERMINATED_ARG;
use super::constants::GIT_PATHSPEC_SEPARATOR;
use super::constants::GIT_REV_LIST_COMMAND;
use super::constants::GIT_RIGHT_COMMIT_MARK;
use super::constants::GIT_STRATEGY_OPTION_NO_RENAMES_ARG;
use super::constants::GIT_SYMMETRIC_RANGE_INFIX;
use super::constants::GIT_WRITE_TREE_ARG;
use super::error::GitError;
use super::object;
use super::reachability::PhaseStartTargetFirstParentHistories;
use crate::ids::GitObjectId;
use crate::ledger::ReservationScopeSet;

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

/// A historical location nominated by patch matches, pending complete scoped replay.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum HistoricalIntegrationCandidateDiscovery {
    /// The earliest first-parent target commit containing every trunk-side patch match.
    Nominated(GitObjectId),
    /// No trunk-side commit has a patch equivalent to a protected phase commit.
    NoMatch,
    /// Git could not read the matches or their ancestry; discovery remains retryable.
    Unavailable,
}

/// Nominate a historical integration site without certifying the protected phase's content.
pub(crate) fn discover_historical_integration_candidate(
    repository_root: &Path,
    phase_start: &GitObjectId,
    protected_tip: &GitObjectId,
    target: &GitObjectId,
    target_histories: &PhaseStartTargetFirstParentHistories,
) -> HistoricalIntegrationCandidateDiscovery {
    let Ok(matches) = phase_equivalent_commits(repository_root, phase_start, protected_tip, target)
    else {
        return HistoricalIntegrationCandidateDiscovery::Unavailable;
    };
    if matches.is_empty() {
        return HistoricalIntegrationCandidateDiscovery::NoMatch;
    }
    target_histories.earliest_containing_matches(repository_root, phase_start, target, &matches)
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
    /// Git's rewrite map located these phase destinations; ancestry and replay still need proof.
    MappedDestinations { commits: &'history [GitObjectId] },
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
    object::object_id(
        repository_root,
        &format!("{proposed_tip}{GIT_FIRST_PARENT_ANCESTOR_INFIX}{replayed}"),
    )
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

/// Collect target-side commits that carry a phase commit's patch.
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
        "--right-only".to_owned(),
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
fn rev_list(repository_root: &Path, arguments: &[String]) -> Result<String, GitError> {
    let output = command::git_output_dynamic(repository_root, arguments)?;
    if !output.status.success() {
        return Err(GitError::CommandFailed {
            command: GIT_REV_LIST_COMMAND,
            stderr:  String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    String::from_utf8(output.stdout).map_err(GitError::InvalidOutput)
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
        ProtectedScopedChanges::NoChanges => {
            return scoped_content_comparison(repository_root, protected_tip, scopes, target);
        },
        ProtectedScopedChanges::Affected {
            paths,
            scoped_replay_rename_detection,
        } => (paths, scoped_replay_rename_detection),
        ProtectedScopedChanges::Unreadable => {
            return Ok(ScopedPatchComparison::Unavailable);
        },
    };

    let locate_target_scoped_commits = || {
        if let ScopedPatchTargetHistory::MappedDestinations { commits } = target_history {
            return Ok(if commits.is_empty() {
                TargetScopedChangePosition::Unproven
            } else {
                TargetScopedChangePosition::Contiguous
            });
        }
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
        ScopedPatchTargetHistory::NeedsGitQueries
        | ScopedPatchTargetHistory::MappedDestinations { .. } => {
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
        command::git_output(repository_root, [GIT_MERGE_BASE_COMMAND, &left, &right]).into(),
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
    let replay_output = scoped_patch_command_output(
        command::git_output_dynamic(repository_root, &replay_arguments).into(),
    )?;
    match replay_output.status.code() {
        Some(GIT_MERGE_TREE_CLEAN_EXIT_CODE) => {},
        Some(GIT_MERGE_TREE_CONFLICT_EXIT_CODE) => {
            match conflict::scoped_merge_conflict_coverage(
                &replay_output.stdout,
                scopes,
                protected_tip,
            ) {
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
    let diff_output = scoped_patch_command_output(
        command::git_output_dynamic(repository_root, &diff_arguments).into(),
    )?;
    if !diff_output.status.success() {
        return Ok(ScopedPatchComparison::Unavailable);
    }
    Ok(if diff_output.stdout.is_empty() {
        ScopedPatchComparison::Equivalent
    } else {
        ScopedPatchComparison::Different
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
                .scoped_commits
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
    let output = scoped_patch_command_output(
        command::git_output_dynamic(repository_root, &arguments).into(),
    )?;
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

/// Answer a protected phase that changed nothing inside its own scopes.
///
/// There is no scoped patch for the target to replay, so the question reduces to content: the
/// target contains this phase's scoped work exactly when its scoped content already matches the
/// protected tip's. A phase claiming paths it only read reaches here -- first-touch claiming and
/// drift widening both produce such reservations -- and a trunk that merely rebased the tip still
/// holds every byte it protected.
fn scoped_content_comparison(
    repository_root: &Path,
    protected_tip: &GitObjectId,
    scopes: &ReservationScopeSet,
    target: &GitObjectId,
) -> Result<ScopedPatchComparison, ScopedPatchComparisonError> {
    Ok(
        match self::protected_scoped_changes(repository_root, protected_tip, scopes, target)? {
            ProtectedScopedChanges::NoChanges => ScopedPatchComparison::Equivalent,
            ProtectedScopedChanges::Affected { .. } => ScopedPatchComparison::Different,
            ProtectedScopedChanges::Unreadable => ScopedPatchComparison::Unavailable,
        },
    )
}

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
    let output = scoped_patch_command_output(
        command::git_output_dynamic(repository_root, &arguments).into(),
    )?;
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

/// Run one `rev-list` invocation and return its standard output.
fn scoped_rev_list(
    repository_root: &Path,
    arguments: &[String],
) -> Result<String, ScopedPatchComparisonError> {
    let output = scoped_patch_command_output(
        command::git_output_dynamic(repository_root, arguments).into(),
    )?;
    if !output.status.success() {
        return Err(GitError::CommandFailed {
            command: GIT_REV_LIST_COMMAND,
            stderr:  String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        }
        .into());
    }
    Ok(String::from_utf8(output.stdout).map_err(GitError::InvalidOutput)?)
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::fs;
    use std::io;
    use std::path::Path;
    use std::process::Command;

    use super::HistoricalIntegrationCandidateDiscovery;
    use super::ScopedPatchComparison;
    use super::ScopedPatchComparisonError;
    use super::ScopedPatchTargetHistory;
    use super::concurrent_scoped_patch_reads;
    use super::discover_historical_integration_candidate;
    use super::scoped_patch_command_output;
    use super::scoped_patch_equivalence;
    use super::scoped_patch_equivalence_with_target_history;
    use crate::gate;
    use crate::gate::RewriteBase;
    use crate::gate::RewriteCreatedCommits;
    use crate::gate::rewrite_map::MappedPhaseInterval;
    use crate::gate::rewrite_map::PhaseRewriteMapping;
    use crate::gate::rewrite_map::RewriteMapPair;
    use crate::git::command::GitCommandOutputAvailability;
    use crate::git::error::GitError;
    use crate::git::fixture;
    use crate::git::fixture::FixtureResult;
    use crate::git::fixture::PRIMARY_BACKUP_PATH;
    use crate::git::fixture::PRIMARY_PATH;
    use crate::git::fixture::PatchEquivalenceFixture;
    use crate::git::fixture::SCRIPT_PATH;
    use crate::git::fixture::SECONDARY_PATH;
    use crate::git::fixture::UNAVAILABLE_OBJECT_ID;
    use crate::git::refs;
    use crate::ids::GitObjectId;
    use crate::ledger::ProtectedPhaseStartHead;
    use crate::reservation;
    use crate::reservation::IntegrationEvidenceStatus;
    use crate::reservation::IntegrationProof;
    use crate::reservation::IntegrationWitness;
    use crate::reservation::PriorIntegrationStatus;
    use crate::reservation::ProtectedReservationTip;

    /// Exercise both the retained graph and the bounded fallback with the same repository.
    fn historical_candidate(
        fixture: &PatchEquivalenceFixture,
        protected_tip: &GitObjectId,
        target: &GitObjectId,
    ) -> FixtureResult<HistoricalIntegrationCandidateDiscovery> {
        let fallback = discover_historical_integration_candidate(
            fixture.root(),
            &fixture.phase_start_head,
            protected_tip,
            target,
            &crate::git::PhaseStartTargetFirstParentHistories::default(),
        );
        let observation = super::super::reachability::branch_commit_reachability(
            fixture.root(),
            "main",
            std::slice::from_ref(&fixture.phase_start_head),
        )?;
        assert_eq!(
            fallback,
            discover_historical_integration_candidate(
                fixture.root(),
                &fixture.phase_start_head,
                protected_tip,
                target,
                &observation.target_histories,
            )
        );
        Ok(fallback)
    }

    /// Spaced edits let replay distinguish protected changes from resolution additions.
    const MAPPED_BASE: &str =
        "one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nten\neleven\ntwelve\n";

    fn mapped_fixture() -> FixtureResult<PatchEquivalenceFixture> {
        let mut fixture = PatchEquivalenceFixture::new()?;
        fixture.write(PRIMARY_PATH, MAPPED_BASE)?;
        fixture.phase_start_head = fixture.commit("spaced baseline")?;
        Ok(fixture)
    }

    fn mapped_equivalence(
        fixture: &PatchEquivalenceFixture,
        phase_start: &GitObjectId,
        protected_tip: &GitObjectId,
        target: &GitObjectId,
        destinations: &[GitObjectId],
    ) -> Result<ScopedPatchComparison, Box<dyn Error>> {
        Ok(scoped_patch_equivalence_with_target_history(
            fixture.root(),
            phase_start,
            &fixture::file_scopes(&[PRIMARY_PATH])?,
            protected_tip,
            target,
            ScopedPatchTargetHistory::MappedDestinations {
                commits: destinations,
            },
        )?)
    }

    #[test]
    fn mapped_destinations_do_not_certify_unrelated_history() -> FixtureResult {
        let fixture = mapped_fixture()?;
        fixture.write(PRIMARY_PATH, &MAPPED_BASE.replace("one\n", "protected\n"))?;
        let protected_tip = fixture.commit("protected edit")?;
        fixture.git(&["checkout", "--orphan", "unrelated"])?;
        let unrelated = fixture.commit("same contents without shared ancestry")?;
        assert_eq!(
            mapped_equivalence(
                &fixture,
                &fixture.phase_start_head,
                &protected_tip,
                &unrelated,
                std::slice::from_ref(&unrelated)
            )?,
            ScopedPatchComparison::Different
        );
        Ok(())
    }

    #[test]
    fn mapped_destinations_preserve_scoped_replay_conflict_refusal() -> FixtureResult {
        let fixture = mapped_fixture()?;
        fixture.write(
            PRIMARY_PATH,
            &MAPPED_BASE.replace("twelve\n", "protected\n"),
        )?;
        let protected_tip = fixture.commit("protected final hunk")?;
        fixture.reset_to_phase_start()?;
        fixture.write(
            PRIMARY_PATH,
            &MAPPED_BASE.replace("twelve\n", "protected\nadjacent resolution edit\n"),
        )?;
        let target = fixture.commit("overlapping resolution edit")?;
        // Git's explicit-base replay conflicts on overlapping replacements even when
        // the target retains the old line. A map supplies no authority to ignore that.
        assert_eq!(
            mapped_equivalence(
                &fixture,
                &fixture.phase_start_head,
                &protected_tip,
                &target,
                std::slice::from_ref(&target)
            )?,
            ScopedPatchComparison::Different
        );
        Ok(())
    }

    #[test]
    fn mapped_acceptance_keeps_the_protected_hunk_after_a_conflict() -> FixtureResult {
        let fixture = mapped_fixture()?;
        let protected = MAPPED_BASE.replace("one\n", "protected\n");
        fixture.write(PRIMARY_PATH, &protected)?;
        let old_tip = fixture.commit("protected edit")?;
        fixture.reset_to_phase_start()?;
        fixture.write(PRIMARY_PATH, &MAPPED_BASE.replace("one\n", "upstream\n"))?;
        fixture.commit("conflicting upstream edit")?;
        assert!(fixture.git(&["cherry-pick", &old_tip.to_string()]).is_err());
        fixture.write(
            PRIMARY_PATH,
            &protected.replace("twelve\n", "resolution addition\n"),
        )?;
        fixture.git(&["add", "--all"])?;
        fixture.git(&["cherry-pick", "--continue"])?;
        let target = refs::head_object_id(fixture.root())?;
        assert_eq!(
            mapped_equivalence(
                &fixture,
                &fixture.phase_start_head,
                &old_tip,
                &target,
                std::slice::from_ref(&target)
            )?,
            ScopedPatchComparison::Equivalent
        );
        Ok(())
    }

    /// Capture the branch history beyond the base recorded by its rebase operation.
    fn capture_created_commits(
        repository_root: &Path,
        administrative_directory: &Path,
        proposed: &GitObjectId,
        previous: &GitObjectId,
        pairs: &[RewriteMapPair],
        onto: &GitObjectId,
    ) -> FixtureResult<RewriteCreatedCommits> {
        let commits = gate::capture_rewrite_created_commits(
            repository_root,
            administrative_directory,
            proposed,
            previous,
            pairs,
            &RewriteBase::RebaseOnto(onto.clone()),
        )?;
        Ok(gate::RewriteCreatedCommits::from_commits(commits))
    }

    #[test]
    fn rewrite_mapping_stops_at_onto_independently_of_other_refs() -> FixtureResult {
        for reference_kind in ["branch", "tag", "remote", "detached_worktree", "no_ref"] {
            let fixture = mapped_fixture()?;
            fixture.write(PRIMARY_PATH, &MAPPED_BASE.replace("one\n", "protected\n"))?;
            let old_tip = fixture.commit("old protected edit")?;
            fixture.reset_to_phase_start()?;
            fixture.write(SECONDARY_PATH, "other reservation's work\n")?;
            let other_base = fixture.commit("other reservation")?;
            match reference_kind {
                "branch" => fixture.git(&["branch", "other", &other_base.to_string()])?,
                "tag" => fixture.git(&["tag", "-a", "other", "-m", "other base"])?,
                "remote" => fixture.git(&[
                    "update-ref",
                    "refs/remotes/origin/other",
                    &other_base.to_string(),
                ])?,
                "detached_worktree" => fixture.git(&[
                    "worktree",
                    "add",
                    "--detach",
                    "other-checkout",
                    &other_base.to_string(),
                ])?,
                _ => {},
            }
            // Stage only the protected path when an untracked linked checkout exists.
            fixture.write(PRIMARY_PATH, &MAPPED_BASE.replace("one\n", "protected\n"))?;
            fixture.git(&["add", PRIMARY_PATH])?;
            fixture.git(&["commit", "--quiet", "-m", "rewritten protected edit"])?;
            let target = refs::head_object_id(fixture.root())?;
            let pairs = [RewriteMapPair {
                old: old_tip.clone(),
                new: target.clone(),
            }];
            let history = gate::rewritten_first_parent_history(fixture.root(), &target)?;
            let created = capture_created_commits(
                fixture.root(),
                &fixture.root().join(".git"),
                &target,
                &old_tip,
                &pairs,
                &other_base,
            )?;
            assert_eq!(
                gate::map_phase_interval(&pairs, &[old_tip], &history, &created),
                PhaseRewriteMapping::Mapped(MappedPhaseInterval {
                    phase_start_head: other_base,
                    protected_tip:    target.clone(),
                    destinations:     vec![target],
                }),
                "onto must establish the phase base with {reference_kind}",
            );
        }
        Ok(())
    }

    #[test]
    fn split_capture_survives_retention_and_sibling_refs_at_the_new_tip() -> FixtureResult {
        for retention_updated_before_capture in [false, true] {
            let fixture = mapped_fixture()?;
            let first = MAPPED_BASE.replace("one\n", "first protected\n");
            let complete = first.replace("twelve\n", "second protected\n");
            fixture.write(PRIMARY_PATH, &complete)?;
            let old_tip = fixture.commit("combined protected edit")?;
            let retention_ref = "refs/cargo-berth/reservations/split";
            fixture.git(&["update-ref", retention_ref, &old_tip.to_string()])?;
            fixture.reset_to_phase_start()?;
            fixture.write(PRIMARY_PATH, &first)?;
            let split_first = fixture.commit("first split edit")?;
            fixture.write(PRIMARY_PATH, &complete)?;
            let target = fixture.commit("second split edit")?;
            fixture.git(&["branch", "sibling", &target.to_string()])?;
            if retention_updated_before_capture {
                fixture.git(&["update-ref", retention_ref, &target.to_string()])?;
            }
            let pairs = [RewriteMapPair {
                old: old_tip.clone(),
                new: target.clone(),
            }];
            let created = capture_created_commits(
                fixture.root(),
                &fixture.root().join(".git"),
                &target,
                &old_tip,
                &pairs,
                &fixture.phase_start_head,
            )?;
            // Another reservation may re-anchor before this event's interval is consumed.
            fixture.git(&["update-ref", retention_ref, &target.to_string()])?;
            let history = gate::rewritten_first_parent_history(fixture.root(), &target)?;
            assert_eq!(
                gate::map_phase_interval(&pairs, &[old_tip], &history, &created),
                PhaseRewriteMapping::Mapped(MappedPhaseInterval {
                    phase_start_head: fixture.phase_start_head,
                    protected_tip:    target.clone(),
                    destinations:     vec![split_first, target],
                }),
                "retention updated before capture: {retention_updated_before_capture}",
            );
        }
        Ok(())
    }

    #[test]
    fn stored_rewrite_commits_survive_the_issuing_checkout_switching() -> FixtureResult {
        let fixture = mapped_fixture()?;
        let first = MAPPED_BASE.replace("one\n", "first protected\n");
        let complete = first.replace("twelve\n", "second protected\n");
        fixture.write(PRIMARY_PATH, &complete)?;
        let old_tip = fixture.commit("combined protected edit")?;
        fixture.reset_to_phase_start()?;
        fixture.write(PRIMARY_PATH, &first)?;
        let split_first = fixture.commit("first split edit")?;
        fixture.write(PRIMARY_PATH, &complete)?;
        let target = fixture.commit("second split edit")?;
        let pairs = [RewriteMapPair {
            old: old_tip.clone(),
            new: target.clone(),
        }];
        let history = gate::rewritten_first_parent_history(fixture.root(), &target)?;
        let captured = gate::capture_rewrite_created_commits(
            fixture.root(),
            &fixture.root().join(".git"),
            &target,
            &old_tip,
            &pairs,
            &RewriteBase::RebaseOnto(fixture.phase_start_head.clone()),
        )?;
        let stored = serde_json::to_vec(&captured)?;
        fixture.git(&["switch", "--detach", &fixture.phase_start_head.to_string()])?;
        let restored = serde_json::from_slice::<Vec<GitObjectId>>(&stored)?;
        assert_eq!(restored, captured);
        let created = gate::RewriteCreatedCommits::from_commits(restored);
        assert_eq!(
            gate::map_phase_interval(&pairs, &[old_tip], &history, &created),
            PhaseRewriteMapping::Mapped(MappedPhaseInterval {
                phase_start_head: fixture.phase_start_head,
                protected_tip:    target.clone(),
                destinations:     vec![split_first, target],
            }),
        );
        Ok(())
    }

    #[test]
    fn linked_rewrite_uses_onto_with_a_detached_main_worktree() -> FixtureResult {
        let fixture = mapped_fixture()?;
        let first = MAPPED_BASE.replace("one\n", "first protected\n");
        let complete = first.replace("twelve\n", "second protected\n");
        fixture.write(PRIMARY_PATH, &complete)?;
        let old_tip = fixture.commit("combined protected edit")?;
        fixture.reset_to_phase_start()?;
        fixture.git(&["switch", "--detach"])?;
        fixture.write(SECONDARY_PATH, "other reservation's work\n")?;
        let other_base = fixture.commit("detached main base")?;
        fixture.git(&["worktree", "add", "-b", "topic", "issuing"])?;
        let issuing = fixture.root().join("issuing");
        fs::write(issuing.join(PRIMARY_PATH), first)?;
        fixture.git(&["-C", "issuing", "commit", "--quiet", "-am", "first split"])?;
        let split_first = refs::head_object_id(&issuing)?;
        fs::write(issuing.join(PRIMARY_PATH), complete)?;
        fixture.git(&["-C", "issuing", "commit", "--quiet", "-am", "second split"])?;
        let target = refs::head_object_id(&issuing)?;
        let pairs = [RewriteMapPair {
            old: old_tip.clone(),
            new: target.clone(),
        }];
        let history = gate::rewritten_first_parent_history(fixture.root(), &target)?;
        let created = capture_created_commits(
            fixture.root(),
            &fixture.root().join(".git/worktrees/issuing"),
            &target,
            &old_tip,
            &pairs,
            &other_base,
        )?;
        assert_eq!(
            gate::map_phase_interval(&pairs, &[old_tip], &history, &created),
            PhaseRewriteMapping::Mapped(MappedPhaseInterval {
                phase_start_head: other_base,
                protected_tip:    target.clone(),
                destinations:     vec![split_first, target],
            }),
        );
        Ok(())
    }

    #[test]
    fn rewrite_mapping_ignores_a_missing_old_exclusion() -> FixtureResult {
        let fixture = mapped_fixture()?;
        fixture.write(PRIMARY_PATH, &MAPPED_BASE.replace("one\n", "protected\n"))?;
        let target = fixture.commit("rewritten protected edit")?;
        let missing_old = UNAVAILABLE_OBJECT_ID.parse::<GitObjectId>()?;
        let pairs = [RewriteMapPair {
            old: missing_old.clone(),
            new: target.clone(),
        }];
        let history = gate::rewritten_first_parent_history(fixture.root(), &target)?;
        let created = capture_created_commits(
            fixture.root(),
            &fixture.root().join(".git"),
            &target,
            &fixture.phase_start_head,
            &pairs,
            &fixture.phase_start_head,
        )?;
        assert_eq!(
            gate::map_phase_interval(&pairs, &[missing_old], &history, &created),
            PhaseRewriteMapping::Mapped(MappedPhaseInterval {
                phase_start_head: fixture.phase_start_head,
                protected_tip:    target.clone(),
                destinations:     vec![target],
            }),
        );
        Ok(())
    }

    #[test]
    fn mapped_acceptance_contains_a_split_phase() -> FixtureResult {
        let fixture = mapped_fixture()?;
        let first = MAPPED_BASE.replace("one\n", "first protected\n");
        let complete = first.replace("twelve\n", "second protected\n");
        fixture.write(PRIMARY_PATH, &complete)?;
        let old_tip = fixture.commit("combined protected edit")?;
        fixture.reset_to_phase_start()?;
        fixture.write(PRIMARY_PATH, &first)?;
        let split_first = fixture.commit("first split edit")?;
        fixture.write(PRIMARY_PATH, &complete)?;
        let target = fixture.commit("second split edit")?;
        let pairs = [RewriteMapPair {
            old: old_tip.clone(),
            new: target.clone(),
        }];
        let history = gate::rewritten_first_parent_history(fixture.root(), &target)?;
        let created = capture_created_commits(
            fixture.root(),
            &fixture.root().join(".git"),
            &target,
            &old_tip,
            &pairs,
            &fixture.phase_start_head,
        )?;
        let mapped = MappedPhaseInterval {
            phase_start_head: fixture.phase_start_head.clone(),
            protected_tip:    target.clone(),
            destinations:     vec![split_first.clone(), target.clone()],
        };
        assert_eq!(
            gate::map_phase_interval(&pairs, std::slice::from_ref(&old_tip), &history, &created),
            PhaseRewriteMapping::Mapped(mapped.clone())
        );
        assert_eq!(
            mapped_equivalence(
                &fixture,
                &fixture.phase_start_head,
                &old_tip,
                &target,
                &mapped.destinations
            )?,
            ScopedPatchComparison::Equivalent
        );

        // A rebase onto the first split records that commit as the phase base.
        let created = capture_created_commits(
            fixture.root(),
            &fixture.root().join(".git"),
            &target,
            &old_tip,
            &pairs,
            &split_first,
        )?;
        assert_eq!(
            gate::map_phase_interval(&pairs, std::slice::from_ref(&old_tip), &history, &created),
            PhaseRewriteMapping::Mapped(MappedPhaseInterval {
                phase_start_head: split_first,
                protected_tip:    target.clone(),
                destinations:     vec![target],
            })
        );

        // A later rewrite that drops the first split must still fail the retained interval.
        fixture.reset_to_phase_start()?;
        fixture.write(
            PRIMARY_PATH,
            &MAPPED_BASE.replace("twelve\n", "second protected\n"),
        )?;
        let dropped_first = fixture.commit("drop first split edit")?;
        assert_eq!(
            mapped_equivalence(
                &fixture,
                &mapped.phase_start_head,
                &mapped.protected_tip,
                &dropped_first,
                std::slice::from_ref(&dropped_first),
            )?,
            ScopedPatchComparison::Different
        );
        Ok(())
    }

    #[test]
    fn mapped_acceptance_preserves_two_checkpoints_with_resolution_edits() -> FixtureResult {
        for resolve_first in [false, true] {
            let fixture = mapped_fixture()?;
            let first = MAPPED_BASE.replace("one\n", "first protected\n");
            let second = first.replace("twelve\n", "second protected\n");
            fixture.write(PRIMARY_PATH, &first)?;
            let old_first = fixture.commit("first checkpoint")?;
            fixture.write(PRIMARY_PATH, &second)?;
            let old_second = fixture.commit("second checkpoint")?;
            fixture.reset_to_phase_start()?;
            fixture.write(PRIMARY_PATH, &MAPPED_BASE.replace("six\n", "upstream\n"))?;
            fixture.commit("upstream same-path change")?;
            let resolved_first = first.replace(
                "six\n",
                if resolve_first {
                    "first resolution\n"
                } else {
                    "upstream\n"
                },
            );
            fixture.write(PRIMARY_PATH, &resolved_first)?;
            let new_first = fixture.commit("rewritten first checkpoint")?;
            let resolved_second = resolved_first
                .replace("twelve\n", "second protected\n")
                .replace("nine\n", "second resolution\n");
            fixture.write(PRIMARY_PATH, &resolved_second)?;
            let new_second = fixture.commit("rewritten second checkpoint with resolution")?;
            assert_eq!(
                mapped_equivalence(
                    &fixture,
                    &fixture.phase_start_head,
                    &old_first,
                    &new_first,
                    std::slice::from_ref(&new_first)
                )?,
                ScopedPatchComparison::Equivalent
            );
            assert_eq!(
                mapped_equivalence(
                    &fixture,
                    &old_first,
                    &old_second,
                    &new_second,
                    std::slice::from_ref(&new_second)
                )?,
                ScopedPatchComparison::Equivalent
            );
        }
        Ok(())
    }

    #[test]
    fn mapped_acceptance_keeps_a_commit_skipped_into_the_base() -> FixtureResult {
        let fixture = mapped_fixture()?;
        let first = MAPPED_BASE.replace("one\n", "already upstream\n");
        let complete = first.replace("twelve\n", "remaining protected\n");
        fixture.write(PRIMARY_PATH, &first)?;
        fixture.commit("phase edit later skipped")?;
        fixture.write(PRIMARY_PATH, &complete)?;
        let old_tip = fixture.commit("remaining phase edit")?;
        fixture.reset_to_phase_start()?;
        fixture.write(PRIMARY_PATH, &first)?;
        fixture.commit("base already carries first edit")?;
        fixture.write(PRIMARY_PATH, &complete)?;
        let target = fixture.commit("replayed remaining phase edit")?;
        assert_eq!(
            mapped_equivalence(
                &fixture,
                &fixture.phase_start_head,
                &old_tip,
                &target,
                std::slice::from_ref(&target)
            )?,
            ScopedPatchComparison::Equivalent
        );
        Ok(())
    }

    #[test]
    fn mapped_acceptance_refuses_discarded_hunks_and_deliberate_drops() -> FixtureResult {
        for discard_hunk in [true, false] {
            let fixture = mapped_fixture()?;
            let first = MAPPED_BASE.replace("one\n", "first protected\n");
            fixture.write(PRIMARY_PATH, &first)?;
            fixture.commit("first protected edit")?;
            fixture.write(
                PRIMARY_PATH,
                &first.replace("twelve\n", "second protected\n"),
            )?;
            let old_tip = fixture.commit("second protected edit")?;
            fixture.reset_to_phase_start()?;
            let target_content = if discard_hunk {
                MAPPED_BASE
                    .replace("one\n", "resolution discards protected hunk\n")
                    .replace("twelve\n", "second protected\n")
            } else {
                first
            };
            fixture.write(PRIMARY_PATH, &target_content)?;
            let target = fixture.commit("incomplete rewrite")?;
            assert_eq!(
                mapped_equivalence(
                    &fixture,
                    &fixture.phase_start_head,
                    &old_tip,
                    &target,
                    std::slice::from_ref(&target)
                )?,
                ScopedPatchComparison::Different
            );
        }
        Ok(())
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
        let scopes = fixture::file_scopes(&[PRIMARY_PATH])?;

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
    fn amended_commit_retains_scoped_patch_equivalence() -> FixtureResult {
        let fixture = PatchEquivalenceFixture::new()?;
        fixture.write(PRIMARY_PATH, "amended reservation\n")?;
        let protected_tip = fixture.commit("protected identity")?;
        let target = fixture.amend("amended identity")?;

        assert_eq!(
            fixture.equivalence(
                &fixture::file_scopes(&[PRIMARY_PATH])?,
                &protected_tip,
                &target
            )?,
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
            &fixture::file_scopes(&[PRIMARY_PATH])?,
            &ProtectedReservationTip::from(protected_tip),
            &target,
            PriorIntegrationStatus::Proven,
        )?;

        assert_eq!(
            status,
            IntegrationEvidenceStatus::Integrated {
                trunk_oid: target,
                proof:     IntegrationProof::ScopedPatchEquivalent,
                witness:   IntegrationWitness::EvaluatedTrunk,
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
            &fixture::file_scopes(&[PRIMARY_PATH])?,
            &protected_tip,
            &fixture.phase_start_head,
            PriorIntegrationStatus::Unproven,
        )?;

        assert_eq!(
            status,
            IntegrationEvidenceStatus::Integrated {
                trunk_oid: fixture.phase_start_head,
                proof:     IntegrationProof::ProtectedTipAncestor,
                witness:   IntegrationWitness::EvaluatedTrunk,
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
        let scopes = fixture::file_scopes(&[PRIMARY_PATH])?;

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
        let target = refs::head_object_id(fixture.root())?;

        assert_eq!(
            fixture.equivalence(
                &fixture::file_scopes(&[PRIMARY_PATH])?,
                &protected_tip,
                &target
            )?,
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
        let target = refs::head_object_id(fixture.root())?;

        assert_eq!(
            fixture.equivalence(
                &fixture::file_scopes(&[PRIMARY_PATH])?,
                &protected_tip,
                &target
            )?,
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
        let target = refs::head_object_id(fixture.root())?;

        assert_eq!(
            fixture.equivalence(
                &fixture::file_scopes(&[PRIMARY_PATH])?,
                &protected_tip,
                &target
            )?,
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
            fixture.equivalence(
                &fixture::file_scopes(&[PRIMARY_PATH])?,
                &protected_tip,
                &target
            )?,
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
            fixture.equivalence(
                &fixture::file_scopes(&[PRIMARY_PATH])?,
                &protected_tip,
                &target
            )?,
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
            fixture.equivalence(
                &fixture::file_scopes(&[PRIMARY_PATH])?,
                &protected_tip,
                &target
            )?,
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
            fixture.equivalence(&fixture::tree_scopes(&["src"])?, &protected_tip, &target)?,
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
            fixture.equivalence(&fixture::tree_scopes(&["src"])?, &protected_tip, &target)?,
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
            fixture.equivalence(&fixture::tree_scopes(&["src"])?, &protected_tip, &target)?,
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
            fixture.equivalence(&fixture::tree_scopes(&["src"])?, &protected_tip, &target)?,
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
            fixture.equivalence(
                &fixture::file_scopes(&[PRIMARY_PATH])?,
                &protected_tip,
                &target
            )?,
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
            fixture.equivalence(
                &fixture::file_scopes(&[PRIMARY_PATH])?,
                &protected_tip,
                &target
            )?,
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
            fixture.equivalence(
                &fixture::file_scopes(&[PRIMARY_PATH])?,
                &protected_tip,
                &target
            )?,
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
            fixture.equivalence(
                &fixture::file_scopes(&[PRIMARY_PATH])?,
                &protected_tip,
                &target
            )?,
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
            fixture.equivalence(&fixture::tree_scopes(&["src"])?, &protected_tip, &target)?,
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
            fixture.equivalence(&fixture::tree_scopes(&["src"])?, &protected_tip, &target)?,
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
            fixture.equivalence(
                &fixture::file_scopes(&[PRIMARY_PATH])?,
                &protected_tip,
                &target
            )?,
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
            fixture.equivalence(
                &fixture::file_scopes(&[PRIMARY_PATH])?,
                &protected_tip,
                &target
            )?,
            ScopedPatchComparison::Equivalent
        );
        Ok(())
    }

    #[test]
    fn duplicate_context_does_not_relocate_the_protected_change() -> FixtureResult {
        let mut fixture = PatchEquivalenceFixture::new()?;
        fixture.write(
            PRIMARY_PATH,
            "prefix\nalpha one\nalpha two\nalpha three\ntarget\nomega one\nomega two\nomega three\nseparator\nalpha one\nalpha two\nalpha three\ntarget\nomega one\nomega two\nomega three\n",
        )?;
        fixture.phase_start_head = fixture.commit("duplicate blocks baseline")?;
        fixture.write(
            PRIMARY_PATH,
            "prefix\nalpha one\nalpha two\nalpha three\nchanged\nomega one\nomega two\nomega three\nseparator\nalpha one\nalpha two\nalpha three\ntarget\nomega one\nomega two\nomega three\n",
        )?;
        let protected_tip = fixture.commit("protected first block")?;
        fixture.reset_to_phase_start()?;
        fixture.write(
            PRIMARY_PATH,
            "prefix\nalpha one\nalpha two\nalpha three\ntarget\nomega one\nomega two\nomega three\nseparator\nalpha one\nalpha two\nalpha three\nchanged\nomega one\nomega two\nomega three\n",
        )?;
        let target = fixture.commit("changed second block")?;

        assert_eq!(
            historical_candidate(&fixture, &protected_tip, &target)?,
            HistoricalIntegrationCandidateDiscovery::Nominated(target.clone()),
        );
        assert_eq!(
            fixture.equivalence(
                &fixture::file_scopes(&[PRIMARY_PATH])?,
                &protected_tip,
                &target
            )?,
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
            fixture.equivalence(
                &fixture::file_scopes(&[PRIMARY_PATH])?,
                &protected_tip,
                &target
            )?,
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
            fixture.equivalence(
                &fixture::file_scopes(&[PRIMARY_PATH])?,
                &protected_tip,
                &target
            )?,
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
            fixture.equivalence(
                &fixture::file_scopes(&[SCRIPT_PATH])?,
                &protected_tip,
                &target
            )?,
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
            fixture.equivalence(
                &fixture::file_scopes(&[PRIMARY_PATH])?,
                &protected_tip,
                &target
            )?,
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
        let target = refs::head_object_id(fixture.root())?;

        assert_eq!(
            fixture.equivalence(
                &fixture::file_scopes(&[PRIMARY_PATH])?,
                &protected_tip,
                &target
            )?,
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
            fixture.equivalence(
                &fixture::file_scopes(&[PRIMARY_PATH])?,
                &protected_tip,
                &target
            )?,
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
            historical_candidate(&fixture, &protected_tip, &target)?,
            HistoricalIntegrationCandidateDiscovery::Nominated(target.clone()),
        );
        assert_eq!(
            fixture.equivalence(
                &fixture::file_scopes(&[PRIMARY_PATH, SECONDARY_PATH])?,
                &protected_tip,
                &target,
            )?,
            ScopedPatchComparison::Different
        );
        fixture.reset_to_phase_start()?;
        fixture.write(SECONDARY_PATH, "missing part\n")?;
        let final_only = fixture.commit("rewritten final patch only")?;
        assert_eq!(
            historical_candidate(&fixture, &protected_tip, &final_only)?,
            HistoricalIntegrationCandidateDiscovery::Nominated(final_only.clone()),
        );
        let scopes = fixture::file_scopes(&[PRIMARY_PATH, SECONDARY_PATH])?;
        assert_eq!(
            fixture.equivalence(&scopes, &protected_tip, &final_only)?,
            ScopedPatchComparison::Different
        );
        fixture.write(PRIMARY_PATH, "integrated part\n")?;
        let reordered = fixture.commit("replay first patch after final patch")?;
        assert_eq!(
            historical_candidate(&fixture, &protected_tip, &reordered)?,
            HistoricalIntegrationCandidateDiscovery::Nominated(reordered.clone()),
        );
        assert_eq!(
            fixture.equivalence(&scopes, &protected_tip, &reordered)?,
            ScopedPatchComparison::Equivalent
        );
        Ok(())
    }

    #[test]
    fn historical_candidate_pins_complete_integration_before_later_hunk_rewrite() -> FixtureResult {
        let fixture = PatchEquivalenceFixture::new()?;
        fixture.write(PRIMARY_PATH, "phase first patch\n")?;
        fixture.commit("phase prefix")?;
        fixture.write(SECONDARY_PATH, "phase second patch\n")?;
        let protected_tip = fixture.commit("phase complete")?;
        fixture.reset_to_phase_start()?;
        fixture.write(PRIMARY_PATH, "phase first patch\n")?;
        let sibling_prefix = fixture.commit("sibling carries only phase prefix")?;
        fixture.git(&["branch", "prefix-sibling"])?;
        let scopes = fixture::file_scopes(&[PRIMARY_PATH, SECONDARY_PATH])?;
        assert_eq!(
            historical_candidate(&fixture, &protected_tip, &sibling_prefix)?,
            HistoricalIntegrationCandidateDiscovery::Nominated(sibling_prefix.clone())
        );
        assert_eq!(
            fixture.equivalence(&scopes, &protected_tip, &sibling_prefix)?,
            ScopedPatchComparison::Different
        );
        fixture.reset_to_phase_start()?;
        fixture.write(PRIMARY_PATH, "phase first patch\n")?;
        fixture.commit("trunk replays phase prefix")?;
        fixture.write(SECONDARY_PATH, "phase second patch\n")?;
        let candidate = fixture.commit("trunk completes phase")?;
        fixture.write(PRIMARY_PATH, "later trunk replacement\n")?;
        let target = fixture.commit("rewrite integrated hunk")?;
        assert_eq!(
            historical_candidate(&fixture, &protected_tip, &target)?,
            HistoricalIntegrationCandidateDiscovery::Nominated(candidate.clone())
        );
        assert_eq!(
            fixture.equivalence(&scopes, &protected_tip, &candidate)?,
            ScopedPatchComparison::Equivalent
        );
        assert_eq!(
            fixture.equivalence(&scopes, &protected_tip, &target)?,
            ScopedPatchComparison::Different
        );
        Ok(())
    }

    #[test]
    fn historical_candidate_distinguishes_no_match_from_unavailable_history() -> FixtureResult {
        let fixture = PatchEquivalenceFixture::new()?;
        fixture.write(PRIMARY_PATH, "protected change\n")?;
        let protected_tip = fixture.commit("protected change")?;
        fixture.reset_to_phase_start()?;
        fixture.write(SECONDARY_PATH, "unrelated target change\n")?;
        let target = fixture.commit("unrelated target change")?;
        assert_eq!(
            historical_candidate(&fixture, &protected_tip, &target)?,
            HistoricalIntegrationCandidateDiscovery::NoMatch
        );
        assert_eq!(
            discover_historical_integration_candidate(
                fixture.root(),
                &fixture.phase_start_head,
                &protected_tip,
                &UNAVAILABLE_OBJECT_ID.parse()?,
                &crate::git::PhaseStartTargetFirstParentHistories::default(),
            ),
            HistoricalIntegrationCandidateDiscovery::Unavailable
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
                &fixture::file_scopes(&[PRIMARY_PATH, SECONDARY_PATH])?,
                &protected_tip,
                &target,
            )?,
            ScopedPatchComparison::Equivalent
        );
        fixture.reset_to_phase_start()?;
        fixture.write(PRIMARY_PATH, "first protected patch\n")?;
        fixture.commit("rewritten first patch with protected gap")?;
        fixture.write(
            PRIMARY_PATH,
            "first protected patch\nintervening scoped edit\n",
        )?;
        fixture.commit("intervening protected-path edit")?;
        fixture.write(SECONDARY_PATH, "second protected patch\n")?;
        let separated = fixture.commit("rewritten second patch after protected gap")?;
        assert_eq!(
            historical_candidate(&fixture, &protected_tip, &separated)?,
            HistoricalIntegrationCandidateDiscovery::Nominated(separated.clone()),
        );
        assert_eq!(
            fixture.equivalence(
                &fixture::file_scopes(&[PRIMARY_PATH, SECONDARY_PATH])?,
                &protected_tip,
                &separated,
            )?,
            ScopedPatchComparison::Different
        );
        Ok(())
    }
}
