//! The commits behind an incursion's entered paths.

use std::collections::HashMap;
use std::collections::HashSet;
use std::path::Path;
use std::thread;

use super::git_output;
use super::git_output::DriftFingerprintError;
use super::git_output::IncursionAttributionActivity;
use super::git_output::IncursionAttributionAnchorState;
use super::git_output::IncursionPathCommit;
use super::observation::FullPhaseHistoryObservation;
use super::observation::ObservedDriftChanges;
use super::observation::ReservationPhaseHistory;
use super::ordering;
use super::report::DriftEffect;
use super::report::DriftReport;
use super::report::IncursionCommit;
use super::report::IncursionCommitOrigin;
use super::report::ReservationDriftResult;
use crate::edge::RepositoryTrunk;
use crate::git;
use crate::ids::GitObjectId;
use crate::ids::ReservationScopePath;
use crate::ledger::IncursionPathSet;
use crate::reservation::RetainedReservationSet;

enum IncursionAttributionSubjectState {
    NoCommittedIncursion,
    Ready(IncursionAttributionSubjects),
}

struct IncursionAttributionSubjectAnchor {
    object_id: GitObjectId,
    state:     IncursionAttributionAnchorState,
}

struct IncursionAttributionSubjects {
    target:  GitObjectId,
    anchors: Vec<IncursionAttributionSubjectAnchor>,
    paths:   Vec<ReservationScopePath>,
}

struct IncursionAnchorAttribution {
    state:         IncursionAttributionAnchorState,
    range_commits: HashSet<GitObjectId>,
}

struct IncursionAttributionBatch {
    anchors:           HashMap<GitObjectId, IncursionAnchorAttribution>,
    commits:           Vec<IncursionPathCommit>,
    origin_membership: IncursionCommitOriginMembership,
}

enum IncursionCommitOriginMembership {
    Classified(HashSet<GitObjectId>),
    CannotClassifyOrigin,
}

impl IncursionCommitOriginMembership {
    fn observe(
        repository_root: &Path,
        origin_basis: &RepositoryTrunk,
        target: &GitObjectId,
    ) -> Self {
        let RepositoryTrunk::Resolved(origin_basis) = origin_basis else {
            return Self::CannotClassifyOrigin;
        };
        git::commits_outside_origin_basis(repository_root, origin_basis, target)
            .map_or(Self::CannotClassifyOrigin, Self::Classified)
    }
}

/// Name the commits behind every entered path an incursion took from the phase range.
///
/// The message a reader acts on names paths and reservation ids only, so a path that
/// arrived on a replayed upstream commit and a path this worktree wrote read the same.
/// Only the committed component can carry a path the worktree never opened, so only
/// those paths are looked up, and a working-tree incursion is left as it was.
pub(super) fn name_incursion_commits(
    repository_root: &Path,
    reservations: &RetainedReservationSet,
    changes: &ObservedDriftChanges,
    repository_trunk: &RepositoryTrunk,
    report: &mut DriftReport,
) -> Result<(), DriftFingerprintError> {
    let IncursionAttributionSubjectState::Ready(subjects) =
        attribution_subjects(reservations, changes, report)
    else {
        return Ok(());
    };
    let batch = attribution_batch(repository_root, repository_trunk, &subjects)?;
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
            let DriftEffect::Incursion { paths, commits, .. } = effect else {
                continue;
            };
            let selected_paths = committed_incursion_paths(committed, paths);
            *commits = commits_for_paths(&batch, phase_start, &selected_paths);
        }
    }
    Ok(())
}

fn attribution_subjects(
    reservations: &RetainedReservationSet,
    changes: &ObservedDriftChanges,
    report: &DriftReport,
) -> IncursionAttributionSubjectState {
    let mut anchors = Vec::new();
    let mut paths = Vec::new();
    for result in &report.results {
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
        let ReservationPhaseHistory::Compared(committed) =
            changes.reservation_phase_history(*reservation_id)
        else {
            continue;
        };
        for effect in effects.as_slice() {
            let DriftEffect::Incursion {
                paths: entered_paths,
                ..
            } = effect
            else {
                continue;
            };
            let selected_paths = committed_incursion_paths(committed, entered_paths);
            if selected_paths.is_empty() {
                continue;
            }
            let phase_start = reservation.phase_start_head().as_ref();
            if !anchors.contains(phase_start) {
                anchors.push(phase_start.clone());
            }
            paths.extend(selected_paths);
        }
    }
    ordering::normalize_paths(&mut paths);
    if paths.is_empty() {
        return IncursionAttributionSubjectState::NoCommittedIncursion;
    }
    let ObservedDriftChanges::Full(full_changes) = changes else {
        return IncursionAttributionSubjectState::NoCommittedIncursion;
    };
    let FullPhaseHistoryObservation::Anchored {
        target,
        anchor_states,
    } = full_changes.phase_history()
    else {
        return IncursionAttributionSubjectState::NoCommittedIncursion;
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
    IncursionAttributionSubjectState::Ready(IncursionAttributionSubjects {
        target: target.clone(),
        anchors,
        paths,
    })
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
    origin_basis: &RepositoryTrunk,
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
            origin_membership: IncursionCommitOriginMembership::CannotClassifyOrigin,
        });
    }
    let subject_anchor_ids = subjects
        .anchors
        .iter()
        .map(|anchor| anchor.object_id.clone())
        .collect::<Vec<_>>();
    let (commits, range_commits_by_anchor, origin_membership) = thread::scope(|scope| {
        let path_log_worker = scope.spawn(|| {
            let path_log_invocation =
                git::incursion_path_log(repository_root, &subjects.target, &subjects.paths);
            let path_log = git_output::completed_git_output(
                path_log_invocation.execution,
                &path_log_invocation.arguments,
            )?;
            git_output::parse_incursion_path_log(&path_log.stdout)
        });
        let commit_graph_worker = scope.spawn(|| {
            git::incursion_range_commits(repository_root, &subject_anchor_ids, &subjects.target)
        });
        let origin_membership_worker = scope.spawn(|| {
            IncursionCommitOriginMembership::observe(
                repository_root,
                origin_basis,
                &subjects.target,
            )
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
        let origin_membership = origin_membership_worker.join().map_err(|_| {
            DriftFingerprintError::IncursionAttributionWorkerPanicked {
                activity: IncursionAttributionActivity::OriginMembership,
            }
        })?;
        Ok::<_, DriftFingerprintError>((commits, range_commits_by_anchor, origin_membership))
    })?;
    for (anchor, range_commits) in subject_anchor_ids.iter().zip(range_commits_by_anchor) {
        if let Some(attribution) = anchors.get_mut(anchor) {
            attribution.range_commits = range_commits;
        }
    }
    Ok(IncursionAttributionBatch {
        anchors,
        commits,
        origin_membership,
    })
}

fn commits_for_paths(
    batch: &IncursionAttributionBatch,
    phase_start: &GitObjectId,
    selected_paths: &[ReservationScopePath],
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
                origin: commit_origin(&batch.origin_membership, &commit.commit),
                commit: commit.commit.clone(),
                subject: commit.subject.clone(),
                paths,
            })
        })
        .collect()
}

/// Whether trunk already carried a commit, so this phase received it rather than wrote it.
fn commit_origin(
    origin_membership: &IncursionCommitOriginMembership,
    commit: &GitObjectId,
) -> IncursionCommitOrigin {
    match origin_membership {
        IncursionCommitOriginMembership::Classified(commits_outside_origin_basis)
            if commits_outside_origin_basis.contains(commit) =>
        {
            IncursionCommitOrigin::PhaseAuthored
        },
        IncursionCommitOriginMembership::Classified(_) => IncursionCommitOrigin::AlreadyOnTrunk,
        IncursionCommitOriginMembership::CannotClassifyOrigin => IncursionCommitOrigin::Unknown,
    }
}
