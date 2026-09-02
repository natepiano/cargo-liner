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
use super::constants::GIT_EXCLUDE_REVISION_PREFIX;
use super::constants::GIT_HEAD_REVISION;
use super::constants::GIT_IGNORE_MISSING_ARG;
use super::constants::GIT_IS_ANCESTOR_ARG;
use super::constants::GIT_LOCAL_BRANCH_REF_PREFIX;
use super::constants::GIT_MERGE_BASE_COMMAND;
use super::constants::GIT_NOT_ANCESTOR_EXIT_CODE;
use super::constants::GIT_PARENTS_ARG;
use super::constants::GIT_REV_LIST_COMMAND;
use super::constants::GIT_STDIN_ARG;
use super::error;
use super::error::GitError;
use super::object;
use super::object::CommitAvailability;
use super::object::CommitObjectResolution;
use super::patch::ScopedPatchTargetHistory;
use crate::ids::GitObjectId;

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
    pub(super) fn contains(&self, candidate: &GitObjectId) -> bool { self.0.contains(candidate) }
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
    target_history: &CommitAncestryGraph,
    candidate_resolutions: Vec<CommitObjectResolution>,
) -> (
    PhaseStartTargetFirstParentHistories,
    Vec<CommitCandidateReachability>,
) {
    let target_histories = PhaseStartTargetFirstParentHistories(
        candidate_resolutions
            .iter()
            .filter_map(|resolution| match resolution {
                CommitObjectResolution::Resolved(candidate)
                    if target_history.contains(candidate) =>
                {
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
        classify_candidates_against_target(&target, &target_history, candidate_resolutions);
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
    use super::AheadBehind;
    use super::CandidateHeadReachability;
    use super::ProtectedTipSuccessorHeadClassification;
    use super::ProtectedTipSuccessorHeads;
    use super::ahead_behind_for_heads;
    use super::descendant_commits;
    use crate::git::fixture::FixtureResult;
    use crate::git::fixture::PRIMARY_PATH;
    use crate::git::fixture::PatchEquivalenceFixture;
    use crate::git::fixture::UNAVAILABLE_OBJECT_ID;
    use crate::ids::GitObjectId;

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
}
