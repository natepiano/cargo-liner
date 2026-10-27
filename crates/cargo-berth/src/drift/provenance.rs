//! The commits behind the paths a phase committed into a foreign holder's scope.
//!
//! Read once, before the ledger lock; classification asks the batch when each path was
//! committed, and the report is named from it after the lock without a further git read.

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::collections::HashSet;
use std::path::Path;
use std::thread;

use super::git_output;
use super::git_output::DriftFingerprintError;
use super::git_output::IncursionAttributionActivity;
use super::git_output::IncursionAttributionAnchorState;
use super::git_output::IncursionPathCommit;
use super::identity::DriftActingIdentity;
use super::observation::FullPhaseHistoryObservation;
use super::observation::ObservedDriftChanges;
use super::observation::ReservationPhaseHistory;
use super::ordering;
use super::report::DriftEffect;
use super::report::DriftReport;
use super::report::IncursionCommit;
use super::report::IncursionCommitOrigin;
use super::report::ReservationDriftResult;
use crate::edge::JudgedTargetTip;
use crate::edge::RepositorySnapshot;
use crate::git;
use crate::git::IncursionPathLogInvocation;
use crate::git::LocalTargetUpstream;
use crate::ids::CommitterTime;
use crate::ids::GitObjectId;
use crate::ids::RecordedAt;
use crate::ids::ReservationId;
use crate::ids::ReservationScopePath;
use crate::ledger::ClaimSource;
use crate::ledger::EnteringCommitSet;
use crate::ledger::IncursionPathSet;
use crate::ledger::IntegrationTarget;
use crate::reservation::RetainedReservationSet;

struct IncursionAttributionSubjectAnchor {
    object_id: GitObjectId,
    state:     IncursionAttributionAnchorState,
}

struct IncursionAttributionSubjects {
    target:  GitObjectId,
    anchors: Vec<IncursionAttributionSubjectAnchor>,
    paths:   Vec<ReservationScopePath>,
    covers:  CoveredIntegrationAnchors,
}

struct IncursionAnchorAttribution {
    state:         IncursionAttributionAnchorState,
    range_commits: HashSet<GitObjectId>,
}

/// The commits behind every path a phase committed into a foreign holder's scope.
///
/// Read once, before the ledger lock, over the paths the pre-lock pass found foreign holders
/// for. Classification asks it when each such path was last committed, and the report is
/// named from it afterwards. Path and range history are batched once per invocation;
/// origin membership is read once for each distinct recorded target.
pub(super) struct IncursionAttributionBatch {
    anchors:             HashMap<GitObjectId, IncursionAnchorAttribution>,
    commits:             Vec<IncursionPathCommit>,
    origin_targets:      HashMap<ReservationId, IntegrationTarget>,
    origin_membership:   BTreeMap<IntegrationTarget, IncursionCommitOriginMembership>,
    covered_integration: CoveredIntegrationHistory,
}

/// Whether the pre-lock pass could read committed foreign-path history.
pub(super) enum CommittedForeignPathHistory {
    /// Classification has not yet requested history.
    NotRead,
    /// The observation contains no committed foreign paths to inspect.
    NoCommittedPaths,
    /// Required history could not be read.
    Unavailable,
    Read(IncursionAttributionBatch),
}

/// Authorship of one path inside the reservation's committed range.
pub(super) enum PathCommitAuthorship {
    NoEnteringCommits,
    Entering(EnteringCommitSet),
    HistoryUnavailable,
}

/// Whether committed work is affirmatively carried by this particular cover.
pub(super) enum CoveredPathContribution {
    CoveredIntegration,
    EnteringWork,
    EvidenceUnavailable,
}

/// Cover tips resolved through the operation's existing object reader.
#[derive(Default)]
struct CoveredIntegrationAnchors {
    holders: HashMap<ReservationId, CoveredBranchAnchor>,
}

enum CoveredBranchAnchor {
    Resolved(GitObjectId),
    Unavailable,
}

/// For covers targeting this checkout, commits the covered branch does not contain.
#[derive(Default)]
struct CoveredIntegrationHistory {
    holders: HashMap<ReservationId, CoveredBranchContribution>,
}

enum CoveredBranchContribution {
    Known {
        outside_branch: HashSet<GitObjectId>,
    },
    Unavailable,
}

impl CoveredIntegrationAnchors {
    fn observe(
        repository: &git::RepositorySnapshot,
        reservations: &RetainedReservationSet,
        snapshot: &RepositorySnapshot,
    ) -> Self {
        let Ok(registry) = crate::worktree::WorktreeRegistry::read(repository) else {
            return Self::default();
        };
        let mut tips = BTreeMap::new();
        let mut holders = HashMap::new();
        for holder in reservations.iter() {
            let ClaimSource::Cover { covered_branch } = holder.source() else {
                continue;
            };
            let target = snapshot.recorded_target(holder.id());
            if registry
                .context_for_branch(target.reference())
                .is_none_or(|context| context.repository_root() != repository.repository_root())
            {
                continue;
            }
            let tip = tips.entry(covered_branch).or_insert_with(|| {
                repository
                    .object_reader()
                    .commit(covered_branch.reference().as_str())
            });
            holders.insert(
                holder.id(),
                tip.as_ref()
                    .map_or(CoveredBranchAnchor::Unavailable, |tip| {
                        CoveredBranchAnchor::Resolved(tip.clone())
                    }),
            );
        }
        Self { holders }
    }
}

enum IncursionCommitOriginMembership {
    /// The comparison range was written directly on the recorded integration target.
    TargetCheckout,
    Classified(HashSet<GitObjectId>),
    CannotClassifyOrigin,
}

impl IncursionCommitOriginMembership {
    fn observe(
        repository: &git::RepositorySnapshot,
        origin_basis: &JudgedTargetTip,
        target: &GitObjectId,
        recorded_target: &IntegrationTarget,
        anchors: &HashMap<GitObjectId, IncursionAnchorAttribution>,
    ) -> Self {
        // `IncursionCommitOriginMembership::TargetCheckout` avoids comparing a target checkout
        // against its own new tip, which would call every write inherited. Its phase range is
        // entering work; only per-holder cover ancestry can exempt a covered contribution.
        let Ok(registry) = crate::worktree::WorktreeRegistry::read(repository) else {
            return Self::CannotClassifyOrigin;
        };
        if registry
            .context_for_branch(recorded_target.reference())
            .is_some_and(|context| context.repository_root() == repository.repository_root())
        {
            return Self::TargetCheckout;
        }
        let JudgedTargetTip::Resolved(origin_basis) = origin_basis else {
            return Self::CannotClassifyOrigin;
        };
        let Ok(upstream) = recorded_target.local_upstream(&repository.object_reader()) else {
            return Self::CannotClassifyOrigin;
        };
        if matches!(upstream, LocalTargetUpstream::Unavailable)
            && let Some(anchor) = anchors.get(origin_basis)
            && anchor.state == IncursionAttributionAnchorState::UsableAncestor
        {
            // `anchor.range_commits` already excludes `origin_basis`. With
            // `LocalTargetUpstream::Unavailable`, origin classification asks exactly the same
            // membership question.
            return Self::Classified(anchor.range_commits.clone());
        }
        git::commits_outside_origin_basis(
            repository.repository_root(),
            origin_basis,
            target,
            &upstream,
        )
        .map_or(Self::CannotClassifyOrigin, Self::Classified)
    }
}

/// Judge a cover independently of every other holder on the path. Missing branch
/// evidence never proves integration, even when Git returned an empty missing range.
pub(super) fn covered_path_contribution(
    history: &CommittedForeignPathHistory,
    phase_start: &GitObjectId,
    path: &ReservationScopePath,
    holder: ReservationId,
) -> CoveredPathContribution {
    let CommittedForeignPathHistory::Read(batch) = history else {
        return CoveredPathContribution::EvidenceUnavailable;
    };
    let Some(contribution) = batch.covered_integration.holders.get(&holder) else {
        return CoveredPathContribution::EnteringWork;
    };
    let CoveredBranchContribution::Known { outside_branch } = contribution else {
        return CoveredPathContribution::EvidenceUnavailable;
    };
    let Some(anchor) = batch.anchors.get(phase_start) else {
        return CoveredPathContribution::EvidenceUnavailable;
    };
    if anchor.state != IncursionAttributionAnchorState::UsableAncestor {
        return CoveredPathContribution::EvidenceUnavailable;
    }
    let mut commits = batch
        .commits
        .iter()
        .filter(|commit| {
            anchor.range_commits.contains(&commit.commit) && commit.paths.contains(path)
        })
        .peekable();
    if commits.peek().is_none() {
        return CoveredPathContribution::EvidenceUnavailable;
    }
    if commits.all(|commit| !outside_branch.contains(&commit.commit)) {
        CoveredPathContribution::CoveredIntegration
    } else {
        CoveredPathContribution::EnteringWork
    }
}

/// Read the commits behind the paths a phase committed into a foreign holder's scope.
///
/// `committed_foreign_paths` names each such path with the reservation whose phase range
/// carried it. Nothing is read when there are none, or when the observation is not a full
/// comparison with an anchored target — a cheap comparison reads no committed history.
pub(super) fn read_committed_foreign_paths(
    repository: &git::RepositorySnapshot,
    reservations: &RetainedReservationSet,
    changes: &ObservedDriftChanges,
    acting_identity: DriftActingIdentity,
    repository_snapshot: &RepositorySnapshot,
    committed_foreign_paths: &[(ReservationId, ReservationScopePath)],
) -> Result<CommittedForeignPathHistory, DriftFingerprintError> {
    let repository_root = repository.repository_root();
    let mut anchors = Vec::new();
    let mut paths = Vec::new();
    let mut origin_targets = HashMap::new();
    let mut distinct_targets = BTreeMap::new();
    for (reservation_id, path) in committed_foreign_paths {
        let Ok(reservation) = reservations.reservation(*reservation_id) else {
            continue;
        };
        let phase_start = reservation.phase_start_head().as_ref();
        if !anchors.contains(phase_start) {
            anchors.push(phase_start.clone());
        }
        paths.push(path.clone());
        let origin_reservation = origin_reservation_for_path(acting_identity, *reservation_id);
        let target = repository_snapshot
            .recorded_target(origin_reservation)
            .clone();
        distinct_targets
            .entry(target.clone())
            .or_insert_with(|| local_authorship_basis(repository, &target, repository_snapshot));
        origin_targets.insert(*reservation_id, target);
    }
    ordering::normalize_paths(&mut paths);
    if paths.is_empty() {
        return Ok(CommittedForeignPathHistory::NoCommittedPaths);
    }
    let ObservedDriftChanges::Full(full_changes) = changes else {
        return Ok(CommittedForeignPathHistory::Unavailable);
    };
    let FullPhaseHistoryObservation::Anchored {
        target: observed_tip,
        anchor_states,
        ..
    } = full_changes.phase_history()
    else {
        return Ok(CommittedForeignPathHistory::Unavailable);
    };
    let anchors = anchors
        .into_iter()
        .map(|object_id| IncursionAttributionSubjectAnchor {
            state: anchor_states
                .get(&object_id)
                .copied()
                .unwrap_or(IncursionAttributionAnchorState::ObjectUnknown),
            object_id,
        })
        .collect();
    let mut batch = attribution_batch(
        repository_root,
        &IncursionAttributionSubjects {
            target: observed_tip.clone(),
            anchors,
            paths,
            covers: CoveredIntegrationAnchors::observe(
                repository,
                reservations,
                repository_snapshot,
            ),
        },
    )?;
    if batch
        .anchors
        .values()
        .any(|anchor| anchor.state == IncursionAttributionAnchorState::UsableAncestor)
    {
        batch.origin_membership = distinct_targets
            .into_iter()
            .map(|(target, observation)| {
                let membership = IncursionCommitOriginMembership::observe(
                    repository,
                    &observation,
                    observed_tip,
                    &target,
                    &batch.anchors,
                );
                (target, membership)
            })
            .collect();
    }
    batch.origin_targets = origin_targets;
    Ok(CommittedForeignPathHistory::Read(batch))
}

/// Authorship excludes commits received from the local target as well as its upstream;
/// integration proof alone excludes only the pushed target history.
fn local_authorship_basis(
    repository: &git::RepositorySnapshot,
    target: &IntegrationTarget,
    snapshot: &RepositorySnapshot,
) -> JudgedTargetTip {
    target
        .judging_branch(
            repository.repository_root(),
            snapshot.repository_trunk_target(),
        )
        .and_then(|branch| {
            git::branch_object_id(&repository.object_reader(), branch.target().short_name())
        })
        .map_or(JudgedTargetTip::ObjectUnknown, JudgedTargetTip::Resolved)
}

const fn origin_reservation_for_path(
    acting_identity: DriftActingIdentity,
    reporting_reservation: ReservationId,
) -> ReservationId {
    match acting_identity {
        DriftActingIdentity::Session { reservation, .. } => reservation,
        DriftActingIdentity::Run { .. } | DriftActingIdentity::Unidentified { .. } => {
            reporting_reservation
        },
    }
}

/// Exclude target-reachable commits before deciding whether the path entered a holder.
pub(super) fn path_commit_authorship(
    history: &CommittedForeignPathHistory,
    reservation_id: ReservationId,
    phase_start: &GitObjectId,
    path: &ReservationScopePath,
    reservations: &RetainedReservationSet,
    holders: &[ReservationId],
) -> PathCommitAuthorship {
    let batch = match history {
        CommittedForeignPathHistory::NoCommittedPaths => {
            return PathCommitAuthorship::NoEnteringCommits;
        },
        CommittedForeignPathHistory::NotRead | CommittedForeignPathHistory::Unavailable => {
            return PathCommitAuthorship::HistoryUnavailable;
        },
        CommittedForeignPathHistory::Read(batch) => batch,
    };
    let Some(anchor) = batch.anchors.get(phase_start) else {
        return PathCommitAuthorship::HistoryUnavailable;
    };
    if anchor.state != IncursionAttributionAnchorState::UsableAncestor {
        return PathCommitAuthorship::HistoryUnavailable;
    }
    let commits = batch
        .commits
        .iter()
        .filter(|commit| anchor.range_commits.contains(&commit.commit))
        .filter(|commit| commit.paths.contains(path))
        .collect::<Vec<_>>();
    if commits.is_empty() {
        return PathCommitAuthorship::HistoryUnavailable;
    }
    let entering = commits
        .iter()
        .filter(|commit| {
            origin_for_commit(batch, reservation_id, &commit.commit)
                != IncursionCommitOrigin::AlreadyOnTrunk
        })
        .filter(|commit| {
            holders.iter().any(|holder| {
                reservations
                    .reservation(*holder)
                    .is_ok_and(|holder| !holder.claimed_at().follows(commit.committed_at))
            })
        })
        .map(|commit| commit.commit.clone())
        .collect::<Vec<_>>();
    EnteringCommitSet::try_from(entering).map_or(
        PathCommitAuthorship::NoEnteringCommits,
        PathCommitAuthorship::Entering,
    )
}

fn origin_for_commit(
    batch: &IncursionAttributionBatch,
    reservation_id: ReservationId,
    commit: &GitObjectId,
) -> IncursionCommitOrigin {
    batch
        .origin_targets
        .get(&reservation_id)
        .and_then(|target| batch.origin_membership.get(target))
        .map_or(IncursionCommitOrigin::Unknown, |membership| {
            commit_origin(membership, commit)
        })
}

/// When one path was last committed to inside the phase range starting at `phase_start`.
///
/// `None` when the batch did not read that range — its phase start is unknown to git or is
/// not an ancestor of the target — or when no commit in it touched the path.
pub(super) fn latest_commit(
    batch: &IncursionAttributionBatch,
    reservation_id: ReservationId,
    phase_start: &GitObjectId,
    path: &ReservationScopePath,
) -> Option<CommitterTime> {
    let anchor = batch.anchors.get(phase_start)?;
    if anchor.state != IncursionAttributionAnchorState::UsableAncestor {
        return None;
    }
    batch
        .commits
        .iter()
        .filter(|commit| anchor.range_commits.contains(&commit.commit))
        .filter(|commit| commit.paths.contains(path))
        .filter(|commit| {
            origin_for_commit(batch, reservation_id, &commit.commit)
                != IncursionCommitOrigin::AlreadyOnTrunk
        })
        .map(|commit| commit.committed_at)
        .max()
}

/// Name the commits behind every entered path an incursion took from the phase range.
///
/// Target-reachable commits have already been excluded from classification and are also
/// omitted here. Paths in both the committed and working-tree components retain their
/// entering commits; a working-tree-only incursion has none.
///
/// Nothing is read here. `batch` is what [`read_committed_foreign_paths`] took before the
/// lock, and the commits it yields are cut to those made while a named holder stood.
pub(super) fn name_incursion_commits(
    reservations: &RetainedReservationSet,
    changes: &ObservedDriftChanges,
    history: &CommittedForeignPathHistory,
    report: &mut DriftReport,
) {
    let CommittedForeignPathHistory::Read(batch) = history else {
        return;
    };
    for result in &mut report.results {
        let ReservationDriftResult::Changed {
            reservation_id,
            effects,
        } = result
        else {
            continue;
        };
        let Ok(reservation) = reservations.reservation(*reservation_id) else {
            continue;
        };
        let phase_start = reservation.phase_start_head().as_ref();
        let ReservationPhaseHistory::Compared(committed) =
            changes.reservation_phase_history(*reservation_id)
        else {
            continue;
        };
        for effect in effects.as_mut_slice() {
            let DriftEffect::Incursion {
                paths,
                commits,
                foreign_reservation_ids,
                ..
            } = effect
            else {
                continue;
            };
            let selected_paths = committed_incursion_paths(committed, paths);
            let earliest_claim = foreign_reservation_ids
                .as_slice()
                .iter()
                .filter_map(|holder_id| reservations.reservation(*holder_id).ok())
                .map(|holder| holder.claimed_at().clone())
                .min();
            *commits = commits_for_paths(
                batch,
                *reservation_id,
                phase_start,
                &selected_paths,
                earliest_claim.as_ref(),
            );
        }
    }
}

fn committed_incursion_paths(
    committed: &[ReservationScopePath],
    entered: &IncursionPathSet,
) -> Vec<ReservationScopePath> {
    entered
        .as_slice()
        .iter()
        .filter(|path| committed.contains(path))
        .cloned()
        .collect()
}

fn attribution_batch(
    repository_root: &Path,
    subjects: &IncursionAttributionSubjects,
) -> Result<IncursionAttributionBatch, DriftFingerprintError> {
    let mut anchors = subjects
        .anchors
        .iter()
        .map(|anchor| {
            (
                anchor.object_id.clone(),
                IncursionAnchorAttribution {
                    state:         anchor.state,
                    range_commits: HashSet::new(),
                },
            )
        })
        .collect::<HashMap<_, _>>();
    let has_usable_anchor = subjects
        .anchors
        .iter()
        .any(|anchor| anchor.state == IncursionAttributionAnchorState::UsableAncestor);
    if !has_usable_anchor {
        return Ok(IncursionAttributionBatch {
            anchors,
            commits: Vec::new(),
            origin_targets: HashMap::new(),
            origin_membership: BTreeMap::new(),
            covered_integration: CoveredIntegrationHistory::default(),
        });
    }
    let mut subject_anchor_ids = subjects
        .anchors
        .iter()
        .map(|anchor| anchor.object_id.clone())
        .collect::<Vec<_>>();
    for anchor in subjects.covers.holders.values() {
        if let CoveredBranchAnchor::Resolved(tip) = anchor
            && !subject_anchor_ids.contains(tip)
        {
            subject_anchor_ids.push(tip.clone());
        }
    }
    let (commits, range_commits_by_anchor) = thread::scope(|scope| {
        let path_log_worker = scope.spawn(|| {
            let path_log_invocation: IncursionPathLogInvocation =
                git::incursion_path_log(repository_root, &subjects.target, &subjects.paths);
            let path_log = git_output::completed_git_output(
                path_log_invocation.output_availability,
                &path_log_invocation.arguments,
            )?;
            git_output::parse_incursion_path_log(&path_log.stdout)
        });
        let commit_graph_worker = scope.spawn(|| {
            git::incursion_range_commits(repository_root, &subject_anchor_ids, &subjects.target)
        });
        let commits = path_log_worker.join().map_err(|_| {
            DriftFingerprintError::IncursionAttributionWorkerPanicked {
                activity: IncursionAttributionActivity::PathLog,
            }
        })??;
        let range_commits_by_anchor = commit_graph_worker.join().map_err(|_| {
            DriftFingerprintError::IncursionAttributionWorkerPanicked {
                activity: IncursionAttributionActivity::CommitGraph,
            }
        })??;
        Ok::<_, DriftFingerprintError>((commits, range_commits_by_anchor))
    })?;
    let ranges = subject_anchor_ids
        .into_iter()
        .zip(range_commits_by_anchor)
        .collect::<HashMap<_, _>>();
    let covered_integration = CoveredIntegrationHistory {
        holders: subjects
            .covers
            .holders
            .iter()
            .map(|(holder, anchor)| {
                let contribution = match anchor {
                    CoveredBranchAnchor::Resolved(tip) => ranges.get(tip).map_or(
                        CoveredBranchContribution::Unavailable,
                        |outside_branch| CoveredBranchContribution::Known {
                            outside_branch: outside_branch.clone(),
                        },
                    ),
                    CoveredBranchAnchor::Unavailable => CoveredBranchContribution::Unavailable,
                };
                (*holder, contribution)
            })
            .collect(),
    };
    for (anchor, range_commits) in ranges {
        if let Some(attribution) = anchors.get_mut(&anchor) {
            attribution.range_commits = range_commits;
        }
    }
    Ok(IncursionAttributionBatch {
        anchors,
        commits,
        origin_targets: HashMap::new(),
        origin_membership: BTreeMap::new(),
        covered_integration,
    })
}

/// The commits behind the selected paths that were made while a named holder stood.
///
/// `earliest_claim` is the oldest claim among the incursion's holders. A commit older than
/// it entered nobody the incident names — that holder is kept for a later commit on the
/// same path — so the message does not accuse it.
fn commits_for_paths(
    batch: &IncursionAttributionBatch,
    reservation_id: ReservationId,
    phase_start: &GitObjectId,
    selected_paths: &[ReservationScopePath],
    earliest_claim: Option<&RecordedAt>,
) -> Vec<IncursionCommit> {
    let Some(anchor) = batch.anchors.get(phase_start) else {
        return Vec::new();
    };
    if anchor.state != IncursionAttributionAnchorState::UsableAncestor {
        return Vec::new();
    }
    batch
        .commits
        .iter()
        .filter(|commit| anchor.range_commits.contains(&commit.commit))
        .filter(|commit| {
            origin_for_commit(batch, reservation_id, &commit.commit)
                != IncursionCommitOrigin::AlreadyOnTrunk
        })
        .filter(|commit| !earliest_claim.is_some_and(|claim| claim.follows(commit.committed_at)))
        .filter_map(|commit| {
            let mut paths = commit
                .paths
                .iter()
                .filter(|path| selected_paths.contains(path))
                .cloned()
                .collect::<Vec<_>>();
            if paths.is_empty() {
                return None;
            }
            ordering::normalize_paths(&mut paths);
            Some(IncursionCommit {
                origin: batch
                    .origin_targets
                    .get(&reservation_id)
                    .and_then(|target| batch.origin_membership.get(target))
                    .map_or(IncursionCommitOrigin::Unknown, |membership| {
                        commit_origin(membership, &commit.commit)
                    }),
                commit: commit.commit.clone(),
                subject: commit.subject.clone(),
                paths,
            })
        })
        .collect()
}

/// Whether the reservation's target already carried a commit before this phase received it.
fn commit_origin(
    origin_membership: &IncursionCommitOriginMembership,
    commit: &GitObjectId,
) -> IncursionCommitOrigin {
    match origin_membership {
        IncursionCommitOriginMembership::TargetCheckout => IncursionCommitOrigin::PhaseAuthored,
        IncursionCommitOriginMembership::Classified(commits_outside_origin_basis)
            if commits_outside_origin_basis.contains(commit) =>
        {
            IncursionCommitOrigin::PhaseAuthored
        },
        IncursionCommitOriginMembership::Classified(_) => IncursionCommitOrigin::AlreadyOnTrunk,
        IncursionCommitOriginMembership::CannotClassifyOrigin => IncursionCommitOrigin::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::collections::BTreeSet;
    use std::collections::HashMap;

    use super::origin_reservation_for_path;
    use crate::drift::identity::DriftActingIdentity;
    use crate::edge::RepositorySnapshot;
    use crate::edge::TargetObservation;
    use crate::ids::CoordinationRunId;
    use crate::ids::ReservationId;
    use crate::ids::WorktreeId;
    use crate::ledger::IntegrationTarget;

    #[test]
    fn absent_committed_paths_are_not_reported_as_unavailable_history()
    -> Result<(), Box<dyn std::error::Error>> {
        let reservations = crate::reservation::RetainedReservationSet::replay(&[])?;
        let reservation = ReservationId::new();
        let phase_start = "1111111111111111111111111111111111111111".parse()?;
        let path = "src/lib.rs".parse()?;
        let authorship = super::path_commit_authorship(
            &super::CommittedForeignPathHistory::NoCommittedPaths,
            reservation,
            &phase_start,
            &path,
            &reservations,
            &[],
        );
        assert!(matches!(
            authorship,
            super::PathCommitAuthorship::NoEnteringCommits
        ));
        for history in [
            super::CommittedForeignPathHistory::NotRead,
            super::CommittedForeignPathHistory::Unavailable,
        ] {
            assert!(matches!(
                super::path_commit_authorship(
                    &history,
                    reservation,
                    &phase_start,
                    &path,
                    &reservations,
                    &[]
                ),
                super::PathCommitAuthorship::HistoryUnavailable
            ));
        }
        Ok(())
    }

    #[test]
    fn origin_targets_batch_by_session_or_reporting_reservations()
    -> Result<(), Box<dyn std::error::Error>> {
        let trunk = IntegrationTarget::from_branch_argument("main")?;
        let integration = IntegrationTarget::from_branch_argument("integration")?;
        let session = ReservationId::new();
        let reporting = ReservationId::new();
        let other = ReservationId::new();
        let snapshot = RepositorySnapshot::new(
            trunk.clone(),
            BTreeMap::from([
                (trunk.clone(), TargetObservation::ObjectUnknown),
                (integration.clone(), TargetObservation::ObjectUnknown),
            ]),
            HashMap::from([
                (session, integration.clone()),
                (reporting, trunk.clone()),
                (other, integration.clone()),
            ]),
            Vec::new(),
            Vec::new(),
            crate::edge::CrossTargetPredecessorEvidence::default(),
        );
        let session_identity = DriftActingIdentity::Session {
            run:         CoordinationRunId::new(),
            reservation: session,
            worktree:    WorktreeId::new(),
        };
        let run_identity = DriftActingIdentity::Run {
            run:      CoordinationRunId::new(),
            worktree: WorktreeId::new(),
        };
        let session_origins = [reporting, other]
            .into_iter()
            .map(|id| {
                snapshot
                    .recorded_target(origin_reservation_for_path(session_identity, id))
                    .clone()
            })
            .collect::<BTreeSet<_>>();
        assert_eq!(
            session_origins,
            std::collections::BTreeSet::from([integration.clone()])
        );
        let run_origins = [reporting, other]
            .into_iter()
            .map(|id| {
                snapshot
                    .recorded_target(origin_reservation_for_path(run_identity, id))
                    .clone()
            })
            .collect::<BTreeSet<_>>();
        assert_eq!(
            run_origins,
            std::collections::BTreeSet::from([trunk, integration])
        );
        Ok(())
    }
}
