//! Ancestry and reachability queries over the commit graph.
//!
//! Every question here takes one target commit and a batch of candidate
//! commits, and returns a typed answer for each candidate. The batching matters — a
//! per-candidate query would spend one git invocation per reservation — so the
//! types in this module carry both the classification and the object availability
//! the same batch proved.

use std::collections::HashMap;
use std::collections::HashSet;
use std::fmt::Write;
use std::path::Path;
use std::thread;

use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;

use super::command;
use super::constants::GIT_ANCESTOR_RANGE_INFIX;
use super::constants::GIT_DIFF_COMMAND;
use super::constants::GIT_EXCLUDE_REVISION_PREFIX;
use super::constants::GIT_HEAD_REVISION;
use super::constants::GIT_IGNORE_MISSING_ARG;
use super::constants::GIT_IS_ANCESTOR_ARG;
use super::constants::GIT_LOCAL_BRANCH_REF_PREFIX;
use super::constants::GIT_MERGE_BASE_COMMAND;
use super::constants::GIT_MERGE_TREE_CLEAN_EXIT_CODE;
use super::constants::GIT_MERGE_TREE_COMMAND;
use super::constants::GIT_MERGE_TREE_CONFLICT_EXIT_CODE;
use super::constants::GIT_NAME_ONLY_ARG;
use super::constants::GIT_NO_MESSAGES_ARG;
use super::constants::GIT_NO_RENAMES_ARG;
use super::constants::GIT_NOT_ANCESTOR_EXIT_CODE;
use super::constants::GIT_NUL_TERMINATED_ARG;
use super::constants::GIT_PARENTS_ARG;
use super::constants::GIT_PATHSPEC_SEPARATOR;
use super::constants::GIT_REV_LIST_COMMAND;
use super::constants::GIT_STDIN_ARG;
use super::constants::GIT_WRITE_TREE_ARG;
use super::error;
use super::error::GitError;
use super::object;
use super::object::CommitAvailability;
use super::object::CommitObjectResolution;
use super::patch::HistoricalIntegrationCandidateDiscovery;
use super::patch::ScopedPatchTargetHistory;
use crate::ids::GitObjectId;
use crate::ids::ReservationScopePath;

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
#[derive(Clone, Default)]
pub(crate) struct ResolvedBatchCommitCandidates(HashSet<GitObjectId>);

impl ResolvedBatchCommitCandidates {
    /// Report whether the object batch resolved this candidate as a commit.
    pub(super) fn contains(&self, candidate: &GitObjectId) -> bool { self.0.contains(candidate) }

    /// Combine commit availability established by independent target batches.
    pub(crate) fn extend(&mut self, other: Self) { self.0.extend(other.0); }
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

/// Target intervals and ancestry shared by scoped replay and historical nomination.
#[derive(Default)]
pub(crate) struct PhaseStartTargetFirstParentHistories {
    intervals:         HashMap<GitObjectId, Vec<GitObjectId>>,
    candidate_history: HistoricalCandidateHistory,
}

/// Whether the batch retained the complete graph needed to locate historical matches.
#[derive(Default)]
enum HistoricalCandidateHistory {
    /// The target and every reachable parent link came from the shared batch.
    Available(ResolvedTargetCommitHistory),
    /// No matching batch graph is available; admitted discovery may read one.
    #[default]
    NeedsGitQuery,
}

impl PhaseStartTargetFirstParentHistories {
    /// Borrow the target interval after one phase start when the graph proved it.
    pub(crate) fn after_phase_start(
        &self,
        phase_start: &GitObjectId,
    ) -> ScopedPatchTargetHistory<'_> {
        self.intervals
            .get(phase_start)
            .map_or(ScopedPatchTargetHistory::NeedsGitQueries, |commits| {
                ScopedPatchTargetHistory::ProvenFirstParentInterval { commits }
            })
    }

    /// Locate every match on the first-parent chain, reusing the batch or reading one graph.
    pub(super) fn earliest_containing_matches(
        &self,
        repository_root: &Path,
        phase_start: &GitObjectId,
        target: &GitObjectId,
        matches: &[GitObjectId],
    ) -> HistoricalIntegrationCandidateDiscovery {
        if let HistoricalCandidateHistory::Available(history) = &self.candidate_history
            && &history.target == target
        {
            return history
                .graph
                .earliest_containing_matches(target, phase_start, matches);
        }
        match target_commit_history(repository_root, &target.to_string()) {
            Ok(history) => history
                .graph
                .earliest_containing_matches(target, phase_start, matches),
            Err(_) => HistoricalIntegrationCandidateDiscovery::Unavailable,
        }
    }
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
    /// Walk each ancestor once while advancing from the oldest first-parent commit.
    fn earliest_containing_matches(
        &self,
        target: &GitObjectId,
        phase_start: &GitObjectId,
        matches: &[GitObjectId],
    ) -> HistoricalIntegrationCandidateDiscovery {
        let mut remaining = matches.iter().collect::<HashSet<_>>();
        if remaining.is_empty() {
            return HistoricalIntegrationCandidateDiscovery::NoMatch;
        }
        let mut visited = HashSet::new();
        for commit in self
            .first_parent_commits_after(target, phase_start)
            .into_iter()
            .rev()
        {
            let mut pending = vec![&commit];
            while let Some(ancestor) = pending.pop() {
                if !visited.insert(ancestor.clone()) {
                    continue;
                }
                remaining.remove(ancestor);
                let Some(parents) = self.parents_by_commit.get(ancestor) else {
                    return HistoricalIntegrationCandidateDiscovery::Unavailable;
                };
                pending.extend(parents);
            }
            if remaining.is_empty() {
                return HistoricalIntegrationCandidateDiscovery::Nominated(commit);
            }
        }
        // Cherry-mark supplied target ancestors; failure to locate one means incomplete history.
        HistoricalIntegrationCandidateDiscovery::Unavailable
    }

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
    let [protected_tip, trunk] = object::commit_object_resolutions(repository_root, &expressions)?
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

/// Return the paths merging this branch into trunk would change, plus every conflicted path.
///
/// `git merge-tree --write-tree` merges `head` into `trunk` as `git merge` would, including
/// the virtual merge base git builds when criss-cross merges leave several merge bases. A
/// name-only diff from trunk to that result tree excludes trunk-only changes, paths the branch
/// changed and then restored, and changes trunk already carries, so an integrated head returns
/// an empty set even when trunk has moved ahead. A conflicted path can keep trunk's content in
/// the result tree, as a modify/delete conflict does, so the conflicted paths `merge-tree` lists
/// after the tree id join the diff. NUL delimiters preserve whitespace in names, and disabling
/// rename detection in the diff retains both the removed and added paths. Missing objects,
/// unrelated histories, and unreadable output remain failures rather than evidence of an empty
/// merge extent.
pub(crate) fn unmerged_branch_paths(
    repository_root: &Path,
    trunk: &GitObjectId,
    head: &GitObjectId,
) -> Result<Vec<ReservationScopePath>, GitError> {
    let merge_arguments = [
        GIT_MERGE_TREE_COMMAND.to_owned(),
        GIT_WRITE_TREE_ARG.to_owned(),
        GIT_NAME_ONLY_ARG.to_owned(),
        GIT_NO_MESSAGES_ARG.to_owned(),
        GIT_NUL_TERMINATED_ARG.to_owned(),
        trunk.to_string(),
        head.to_string(),
    ];
    let merge_output = command::git_output_dynamic(repository_root, &merge_arguments)?;
    let merge_failure = GitError::CommandFailed {
        command: GIT_MERGE_TREE_COMMAND,
        stderr:  String::from_utf8_lossy(&merge_output.stderr)
            .trim()
            .to_owned(),
    };
    if !matches!(
        merge_output.status.code(),
        Some(GIT_MERGE_TREE_CLEAN_EXIT_CODE | GIT_MERGE_TREE_CONFLICT_EXIT_CODE)
    ) {
        return Err(merge_failure);
    }
    let merge_output = String::from_utf8(merge_output.stdout).map_err(GitError::InvalidOutput)?;
    let mut merge_fields = merge_output.split_terminator('\0');
    // A missing object also exits 1, the conflict status, but writes no result tree.
    let Some(result_tree) = merge_fields.next() else {
        return Err(merge_failure);
    };
    let result_tree = result_tree
        .parse::<GitObjectId>()
        .map_err(GitError::InvalidObjectId)?;
    let mut paths = merge_fields
        .map(|path| path.parse().map_err(GitError::InvalidReservationPath))
        .collect::<Result<Vec<ReservationScopePath>, _>>()?;

    let diff_arguments = [
        GIT_DIFF_COMMAND.to_owned(),
        GIT_NAME_ONLY_ARG.to_owned(),
        GIT_NUL_TERMINATED_ARG.to_owned(),
        GIT_NO_RENAMES_ARG.to_owned(),
        trunk.to_string(),
        result_tree.to_string(),
        GIT_PATHSPEC_SEPARATOR.to_owned(),
    ];
    let diff_output = command::git_output_dynamic(repository_root, &diff_arguments)?;
    if !diff_output.status.success() {
        return Err(GitError::CommandFailed {
            command: GIT_DIFF_COMMAND,
            stderr:  String::from_utf8_lossy(&diff_output.stderr)
                .trim()
                .to_owned(),
        });
    }
    for path in String::from_utf8(diff_output.stdout)
        .map_err(GitError::InvalidOutput)?
        .split_terminator('\0')
    {
        paths.push(path.parse().map_err(GitError::InvalidReservationPath)?);
    }
    paths.sort_by_cached_key(ToString::to_string);
    paths.dedup();
    Ok(paths)
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
    let output = command::git_output_dynamic(repository_root, &arguments)?;
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

/// Return every commit reachable from a proposed initial trunk object.
pub(crate) fn reachable_commits(
    repository_root: &Path,
    proposed: &GitObjectId,
) -> Result<Vec<GitObjectId>, GitError> {
    let arguments = vec![GIT_REV_LIST_COMMAND.to_owned(), proposed.to_string()];
    let output = command::git_output_dynamic(repository_root, &arguments)?;
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
    let Ok(output) =
        command::git_output_dynamic_with_input(repository_root, &arguments, input.as_bytes())
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
    let output = command::git_output(
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

/// Why one target expression named no commit for candidates to be classified against.
enum UnusableCommitTarget {
    /// No object resolves from the target expression.
    Missing,
    /// More than one object matches the target expression.
    Ambiguous,
    /// The target expression resolves, but not to a commit object.
    WrongType { object_type: String },
}

impl UnusableCommitTarget {
    /// State the unusable target as the answer every caller reads.
    fn into_reachability(self) -> CommitTargetReachability {
        match self {
            Self::Missing => CommitTargetReachability::Missing,
            Self::Ambiguous => CommitTargetReachability::Ambiguous,
            Self::WrongType { object_type } => CommitTargetReachability::WrongType { object_type },
        }
    }
}

/// One target expression resolved on its own, before any candidate is classified.
enum SoleCommitTarget {
    /// The expression names one commit.
    Resolved(GitObjectId),
    /// The expression names nothing this module can classify candidates against.
    Unusable(UnusableCommitTarget),
}

impl From<CommitObjectResolution> for SoleCommitTarget {
    fn from(resolution: CommitObjectResolution) -> Self {
        match resolution {
            CommitObjectResolution::Resolved(target) => Self::Resolved(target),
            CommitObjectResolution::Missing => Self::Unusable(UnusableCommitTarget::Missing),
            CommitObjectResolution::Ambiguous => Self::Unusable(UnusableCommitTarget::Ambiguous),
            CommitObjectResolution::WrongType { object_type } => {
                Self::Unusable(UnusableCommitTarget::WrongType { object_type })
            },
        }
    }
}

/// A completed target-history read, before its failure case has been diagnosed.
enum TargetHistoryRead {
    /// The revision walk produced the target commit and its parent links.
    Walked(ResolvedTargetCommitHistory),
    /// The walk failed; only a second look at the target says whether that is an error.
    Failed(GitError),
}

/// Resolve one target expression through an object batch of its own.
fn sole_commit_target(
    repository_root: &Path,
    target_expression: &str,
) -> Result<SoleCommitTarget, GitError> {
    let [resolution] =
        object::commit_object_resolutions(repository_root, &[target_expression.to_owned()])?
            .try_into()
            .map_err(|resolutions: Vec<_>| GitError::InvalidBatchObjectCount {
                expected: 1,
                actual:   resolutions.len(),
            })?;
    Ok(SoleCommitTarget::from(resolution))
}

/// Answer for a target with no candidates, which needs no history walk at all.
fn uncontested_commit_target_reachability(
    repository_root: &Path,
    target_expression: &str,
) -> Result<CommitTargetReachabilityObservation, GitError> {
    let reachability = match sole_commit_target(repository_root, target_expression)? {
        SoleCommitTarget::Resolved(target) => CommitTargetReachability::Resolved {
            target,
            candidates: Vec::new(),
        },
        SoleCommitTarget::Unusable(unusable) => unusable.into_reachability(),
    };
    Ok(CommitTargetReachabilityObservation {
        reachability,
        resolved_candidates: ResolvedBatchCommitCandidates::default(),
        target_histories: PhaseStartTargetFirstParentHistories::default(),
    })
}

/// Read the target's ancestry graph and the candidate object batch at the same time.
fn read_target_history_and_candidates(
    repository_root: &Path,
    target_expression: &str,
    candidate_expressions: &[String],
) -> Result<(TargetHistoryRead, Vec<CommitObjectResolution>), GitError> {
    thread::scope(|scope| {
        let target_history_worker =
            scope.spawn(|| target_commit_history(repository_root, target_expression));
        let candidate_resolution_worker = scope
            .spawn(|| object::commit_object_resolutions(repository_root, candidate_expressions));
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
        let target_history = match target_history {
            Ok(target_history) => TargetHistoryRead::Walked(target_history),
            Err(history_error) => TargetHistoryRead::Failed(history_error),
        };
        Ok::<_, GitError>((target_history, candidate_resolutions))
    })
}

/// Decide whether a failed history walk is an error or an unusable target expression.
fn diagnose_failed_target_history(
    repository_root: &Path,
    target_expression: &str,
    history_error: GitError,
    resolved_candidates: ResolvedBatchCommitCandidates,
) -> Result<CommitTargetReachabilityObservation, GitError> {
    match sole_commit_target(repository_root, target_expression)? {
        SoleCommitTarget::Resolved(_) => Err(history_error),
        SoleCommitTarget::Unusable(unusable) => Ok(CommitTargetReachabilityObservation {
            reachability: unusable.into_reachability(),
            resolved_candidates,
            target_histories: PhaseStartTargetFirstParentHistories::default(),
        }),
    }
}

/// Classify every candidate against the target's own line of descent.
fn classify_candidates_against_target(
    target: &GitObjectId,
    target_history: CommitAncestryGraph,
    candidate_resolutions: Vec<CommitObjectResolution>,
) -> (
    PhaseStartTargetFirstParentHistories,
    Vec<CommitCandidateReachability>,
) {
    let intervals = candidate_resolutions
        .iter()
        .filter_map(|resolution| match resolution {
            CommitObjectResolution::Resolved(candidate) if target_history.contains(candidate) => {
                Some((
                    candidate.clone(),
                    target_history.first_parent_commits_after(target, candidate),
                ))
            },
            CommitObjectResolution::Resolved(_)
            | CommitObjectResolution::Missing
            | CommitObjectResolution::Ambiguous
            | CommitObjectResolution::WrongType { .. } => None,
        })
        .collect();
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
    let target_histories = PhaseStartTargetFirstParentHistories {
        intervals,
        candidate_history: HistoricalCandidateHistory::Available(ResolvedTargetCommitHistory {
            target: target.clone(),
            graph:  target_history,
        }),
    };
    (target_histories, candidates)
}

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
        return uncontested_commit_target_reachability(repository_root, target_expression);
    }
    let (target_history, candidate_resolutions) = read_target_history_and_candidates(
        repository_root,
        target_expression,
        &candidate_expressions,
    )?;
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
        TargetHistoryRead::Walked(target_history) => target_history,
        TargetHistoryRead::Failed(history_error) => {
            return diagnose_failed_target_history(
                repository_root,
                target_expression,
                history_error,
                resolved_candidates,
            );
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
    let (target_histories, candidates) =
        classify_candidates_against_target(&target, target_history, candidate_resolutions);
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
    let output = error::completed_git_command(
        command::git_output_dynamic(repository_root, &arguments).into(),
    )?;
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
    let object_availability = object::commit_availability(repository_root, &queried_objects)?;
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
    let output = error::completed_git_command(
        command::git_output_dynamic_with_input(repository_root, &arguments, input.as_bytes())
            .into(),
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
    let output = error::completed_git_command(
        command::git_output_dynamic(repository_root, &arguments).into(),
    )?;
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
    let output = error::completed_git_command(
        command::git_output_dynamic_with_input(repository_root, &arguments, input.as_bytes())
            .into(),
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

/// The grouped descendant result for one protected predecessor tip.
pub(crate) enum ProtectedTipSuccessorHeadClassification {
    /// Every candidate head received its own typed reachability result.
    Classified(Vec<CandidateHeadReachability>),
    /// The protected predecessor tip does not resolve as a commit.
    AncestorObjectUnknown,
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use super::AheadBehind;
    use super::CandidateHeadReachability;
    use super::ProtectedTipSuccessorHeadClassification;
    use super::ProtectedTipSuccessorHeads;
    use super::ahead_behind_for_heads;
    use super::descendant_commits;
    use super::unmerged_branch_paths;
    use crate::git::GitError;
    use crate::git::fixture::FixtureResult;
    use crate::git::fixture::PRIMARY_PATH;
    use crate::git::fixture::PatchEquivalenceFixture;
    use crate::git::fixture::SECONDARY_PATH;
    use crate::git::fixture::UNAVAILABLE_OBJECT_ID;
    use crate::ids::GitObjectId;

    /// The lane branch whose history criss-crosses with `main`.
    const CRISS_CROSS_LANE: &str = "lane";

    #[test]
    fn historical_candidate_history_survives_a_phase_start_outside_trunk_ancestry() -> FixtureResult
    {
        let mut fixture = PatchEquivalenceFixture::new()?;
        let original_base = fixture.phase_start_head.clone();
        fixture.write(SECONDARY_PATH, "earlier phase\n")?;
        fixture.phase_start_head = fixture.commit("old phase start")?;
        fixture.write(PRIMARY_PATH, "protected later checkpoint\n")?;
        let protected_tip = fixture.commit("protected later checkpoint")?;
        fixture.reset_to(&original_base)?;
        fixture.write("docs/new-base.md", "new trunk base\n")?;
        fixture.commit("advance trunk before rewrite")?;
        fixture.write(SECONDARY_PATH, "earlier phase\n")?;
        fixture.commit("rebased phase start")?;
        fixture.write(PRIMARY_PATH, "protected later checkpoint\n")?;
        let candidate = fixture.commit("rebased later checkpoint")?;
        fixture.write(PRIMARY_PATH, "later trunk hunk rewrite\n")?;
        let trunk = fixture.commit("rewrite protected hunk")?;
        let observation = super::branch_commit_reachability(
            fixture.root(),
            "main",
            std::slice::from_ref(&fixture.phase_start_head),
        )?;
        assert!(matches!(
            observation
                .target_histories
                .after_phase_start(&fixture.phase_start_head),
            super::ScopedPatchTargetHistory::NeedsGitQueries
        ));
        // An invalid query directory proves this uses the retained batch, including off-trunk
        // starts.
        assert_eq!(
            observation.target_histories.earliest_containing_matches(
                &fixture.root().join("absent-directory"),
                &fixture.phase_start_head,
                &trunk,
                std::slice::from_ref(&candidate),
            ),
            super::HistoricalIntegrationCandidateDiscovery::Nominated(candidate.clone())
        );
        assert_eq!(
            crate::git::discover_historical_integration_candidate(
                fixture.root(),
                &fixture.phase_start_head,
                &crate::git::fixture::file_scopes(&[PRIMARY_PATH])?,
                &protected_tip,
                &trunk,
                &observation.target_histories,
            ),
            super::HistoricalIntegrationCandidateDiscovery::Nominated(candidate.clone())
        );
        assert_eq!(
            crate::git::discover_historical_integration_candidate(
                fixture.root(),
                &fixture.phase_start_head,
                &crate::git::fixture::file_scopes(&[PRIMARY_PATH])?,
                &protected_tip,
                &trunk,
                &super::PhaseStartTargetFirstParentHistories::default(),
            ),
            super::HistoricalIntegrationCandidateDiscovery::Nominated(candidate.clone())
        );
        assert_eq!(
            fixture.equivalence(
                &crate::git::fixture::file_scopes(&[PRIMARY_PATH])?,
                &protected_tip,
                &candidate,
            )?,
            crate::git::ScopedPatchComparison::Equivalent
        );
        Ok(())
    }

    #[test]
    fn historical_candidate_locates_other_parent_matches_at_the_first_parent_merge() -> FixtureResult
    {
        let fixture = PatchEquivalenceFixture::new()?;
        fixture.write(PRIMARY_PATH, "first phase patch\n")?;
        fixture.commit("original first phase patch")?;
        fixture.write(SECONDARY_PATH, "second phase patch\n")?;
        let protected_tip = fixture.commit("original second phase patch")?;
        fixture.reset_to_phase_start()?;
        fixture.write(PRIMARY_PATH, "first phase patch\n")?;
        let first_match = fixture.commit("trunk first phase patch")?;
        fixture.git(&[
            "checkout",
            "--quiet",
            "-b",
            "side",
            &fixture.phase_start_head.to_string(),
        ])?;
        fixture.write(SECONDARY_PATH, "second phase patch\n")?;
        let other_parent_match = fixture.commit("side second phase patch")?;
        fixture.git(&["checkout", "--quiet", "main"])?;
        fixture.git(&[
            "merge",
            "--quiet",
            "--no-ff",
            "side",
            "-m",
            "merge second phase patch",
        ])?;
        let candidate = crate::git::refs::head_object_id(fixture.root())?;
        fixture.write(PRIMARY_PATH, "later trunk hunk rewrite\n")?;
        let trunk = fixture.commit("rewrite phase after merge")?;
        let observation = super::branch_commit_reachability(
            fixture.root(),
            "main",
            std::slice::from_ref(&fixture.phase_start_head),
        )?;
        let super::ScopedPatchTargetHistory::ProvenFirstParentInterval { commits } = observation
            .target_histories
            .after_phase_start(&fixture.phase_start_head)
        else {
            return Err("expected retained first-parent interval".into());
        };
        assert!(commits.contains(&candidate));
        assert!(!commits.contains(&other_parent_match));
        let matches = [first_match, other_parent_match];
        assert_eq!(
            observation.target_histories.earliest_containing_matches(
                &fixture.root().join("absent-directory"),
                &fixture.phase_start_head,
                &trunk,
                &matches,
            ),
            super::HistoricalIntegrationCandidateDiscovery::Nominated(candidate.clone())
        );
        assert_eq!(
            super::PhaseStartTargetFirstParentHistories::default().earliest_containing_matches(
                fixture.root(),
                &fixture.phase_start_head,
                &trunk,
                &matches,
            ),
            super::HistoricalIntegrationCandidateDiscovery::Nominated(candidate.clone())
        );
        assert_eq!(
            crate::git::discover_historical_integration_candidate(
                fixture.root(),
                &fixture.phase_start_head,
                &crate::git::fixture::file_scopes(&[PRIMARY_PATH, SECONDARY_PATH])?,
                &protected_tip,
                &trunk,
                &observation.target_histories,
            ),
            super::HistoricalIntegrationCandidateDiscovery::Nominated(candidate)
        );
        assert_eq!(
            super::PhaseStartTargetFirstParentHistories::default().earliest_containing_matches(
                &fixture.root().join("absent-directory"),
                &fixture.phase_start_head,
                &trunk,
                &matches,
            ),
            super::HistoricalIntegrationCandidateDiscovery::Unavailable
        );
        Ok(())
    }

    #[test]
    fn integrated_head_has_no_merge_paths_when_trunk_moves_ahead() -> FixtureResult {
        let fixture = PatchEquivalenceFixture::new()?;
        let head = fixture.phase_start_head.clone();
        assert!(unmerged_branch_paths(fixture.root(), &head, &head)?.is_empty());

        fixture.write(SECONDARY_PATH, "trunk-only change\n")?;
        let trunk = fixture.commit("advance trunk")?;
        assert!(unmerged_branch_paths(fixture.root(), &trunk, &head)?.is_empty());
        Ok(())
    }

    #[test]
    fn branch_merge_paths_exclude_trunk_changes_and_reverted_paths() -> FixtureResult {
        let fixture = PatchEquivalenceFixture::new()?;
        fixture.write(PRIMARY_PATH, "branch change\n")?;
        fixture.write("reverted.rs", "temporary branch change\n")?;
        fixture.commit("branch changes")?;
        fixture.remove("reverted.rs")?;
        let head = fixture.commit("revert temporary path")?;

        fixture.reset_to_phase_start()?;
        fixture.write(SECONDARY_PATH, "trunk-only change\n")?;
        let trunk = fixture.commit("diverged trunk")?;
        assert_eq!(
            unmerged_branch_paths(fixture.root(), &trunk, &head)?,
            vec![PRIMARY_PATH.parse()?],
        );
        Ok(())
    }

    #[test]
    fn branch_merge_paths_keep_both_rename_sides_and_verbatim_names() -> FixtureResult {
        let fixture = PatchEquivalenceFixture::new()?;
        let trunk = fixture.phase_start_head.clone();
        let renamed_path = "src/name with\ttab and\nnewline.rs";
        fixture.git(&["mv", PRIMARY_PATH, renamed_path])?;
        let head = fixture.commit("rename branch path")?;
        let paths = unmerged_branch_paths(fixture.root(), &trunk, &head)?;
        assert_eq!(paths.len(), 2);
        assert!(paths.contains(&PRIMARY_PATH.parse()?));
        assert!(paths.contains(&renamed_path.parse()?));
        Ok(())
    }

    #[test]
    fn unanswerable_branch_merge_paths_fail_instead_of_appearing_empty() -> FixtureResult {
        let fixture = PatchEquivalenceFixture::new()?;
        let trunk = fixture.phase_start_head.clone();
        let unavailable = UNAVAILABLE_OBJECT_ID.parse::<GitObjectId>()?;
        assert!(matches!(
            unmerged_branch_paths(fixture.root(), &trunk, &unavailable),
            Err(GitError::CommandFailed { .. }),
        ));

        fixture.git(&["checkout", "--quiet", "--orphan", "unrelated"])?;
        let unrelated = fixture.commit("unrelated history")?;
        assert!(matches!(
            unmerged_branch_paths(fixture.root(), &trunk, &unrelated),
            Err(GitError::CommandFailed { .. }),
        ));
        Ok(())
    }

    #[test]
    fn criss_cross_branch_merge_paths_are_only_the_branch_work() -> FixtureResult {
        let fixture = PatchEquivalenceFixture::new()?;
        criss_cross_lane(&fixture)?;
        fixture.write(SECONDARY_PATH, "trunk-only change\n")?;
        let trunk = fixture.commit("trunk after the criss-cross")?;
        fixture.git(&["checkout", "--quiet", CRISS_CROSS_LANE])?;
        fixture.write(PRIMARY_PATH, "lane work after the criss-cross\n")?;
        let head = fixture.commit("lane after the criss-cross")?;
        assert_eq!(merge_base_count(&fixture, &trunk, &head)?, 2);

        assert_eq!(
            unmerged_branch_paths(fixture.root(), &trunk, &head)?,
            vec![PRIMARY_PATH.parse()?],
        );
        Ok(())
    }

    #[test]
    fn a_modify_delete_conflict_path_is_a_branch_merge_path() -> FixtureResult {
        let fixture = PatchEquivalenceFixture::new()?;
        criss_cross_lane(&fixture)?;
        fixture.write(SECONDARY_PATH, "trunk keeps and modifies this path\n")?;
        let trunk = fixture.commit("trunk modifies the path the lane deletes")?;
        fixture.git(&["checkout", "--quiet", CRISS_CROSS_LANE])?;
        fixture.remove(SECONDARY_PATH)?;
        fixture.write(PRIMARY_PATH, "lane work beside the deletion\n")?;
        let head = fixture.commit("lane deletes the path trunk modifies")?;

        // The conflicted merge keeps trunk's modified file, so only the conflict record names it.
        assert_eq!(
            unmerged_branch_paths(fixture.root(), &trunk, &head)?,
            vec![PRIMARY_PATH.parse()?, SECONDARY_PATH.parse()?],
        );
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

    /// Merge the lane's first commit into trunk and trunk's first commit into the lane, so any
    /// later trunk and lane tips have two merge bases. Leaves `main` checked out.
    fn criss_cross_lane(fixture: &PatchEquivalenceFixture) -> FixtureResult {
        fixture.git(&["checkout", "--quiet", "-b", CRISS_CROSS_LANE])?;
        fixture.write("lane_first.rs", "lane work trunk merges\n")?;
        let lane_first = fixture.commit("first lane commit")?;
        fixture.git(&["checkout", "--quiet", "main"])?;
        fixture.write("trunk_first.rs", "trunk work the lane merges\n")?;
        let trunk_first = fixture.commit("first trunk commit")?;
        fixture.git(&[
            "merge",
            "--quiet",
            "--no-ff",
            "--no-edit",
            &lane_first.to_string(),
        ])?;
        fixture.git(&["checkout", "--quiet", CRISS_CROSS_LANE])?;
        fixture.git(&[
            "merge",
            "--quiet",
            "--no-ff",
            "--no-edit",
            &trunk_first.to_string(),
        ])?;
        fixture.git(&["checkout", "--quiet", "main"])?;
        Ok(())
    }

    fn merge_base_count(
        fixture: &PatchEquivalenceFixture,
        trunk: &GitObjectId,
        head: &GitObjectId,
    ) -> FixtureResult<usize> {
        let output = Command::new("git")
            .args(["merge-base", "--all", &trunk.to_string(), &head.to_string()])
            .current_dir(fixture.root())
            .output()?;
        assert!(
            output.status.success(),
            "git merge-base --all should succeed"
        );
        Ok(String::from_utf8(output.stdout)?.lines().count())
    }
}
