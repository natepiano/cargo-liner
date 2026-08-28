//! The commits behind an incursion's entered paths.

use std::collections::HashMap;
use std::collections::HashSet;
use std::path::Path;

use super::git_output;
use super::git_output::DriftFingerprintError;
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
use crate::config::BerthConfig;
use crate::config::Enrollment;
use crate::git;
use crate::ids::GitObjectId;
use crate::ids::ReservationScopePath;
use crate::ledger::IncursionPathSet;
use crate::reservation::RetainedReservationSet;

/// The trunk basis used to classify where an incursion commit originated.
enum IncursionCommitOriginBasis {
    ResolvedTrunk(GitObjectId),
    CannotClassifyOrigin,
}

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
    report: &mut DriftReport,
) -> Result<(), DriftFingerprintError> {
    let IncursionAttributionSubjectState::Ready(subjects) =
        attribution_subjects(reservations, changes, report)
    else {
        return Ok(());
    };
    let origin_basis = trunk_object_id(repository_root);
    let batch = attribution_batch(repository_root, &origin_basis, &subjects)?;
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
    origin_basis: &IncursionCommitOriginBasis,
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
    let usable_anchors = subjects
        .anchors
        .iter()
        .filter(|anchor| anchor.state == IncursionAttributionAnchorState::UsableAncestor)
        .map(|anchor| anchor.object_id.clone())
        .collect::<Vec<_>>();
    if usable_anchors.is_empty() {
        return Ok(IncursionAttributionBatch {
            anchors,
            commits: Vec::new(),
            origin_membership: IncursionCommitOriginMembership::CannotClassifyOrigin,
        });
    }
    let union_base = git::incursion_attribution_union_base(repository_root, &usable_anchors)
        .map_err(DriftFingerprintError::from)?;
    let path_log_invocation = git::incursion_path_log(
        repository_root,
        &union_base,
        &subjects.target,
        &subjects.paths,
    );
    let path_log = git_output::completed_git_output(
        path_log_invocation.execution,
        &path_log_invocation.arguments,
    )?;
    let commits = git_output::parse_incursion_path_log(&path_log.stdout)?;
    let candidate_commits = commits
        .iter()
        .map(|commit| commit.commit.clone())
        .collect::<Vec<_>>();
    let subject_anchor_ids = subjects
        .anchors
        .iter()
        .map(|anchor| anchor.object_id.clone())
        .collect::<Vec<_>>();
    let range_commits = git::incursion_range_commits(
        repository_root,
        &subject_anchor_ids,
        &subjects.target,
        &candidate_commits,
    )
    .map_err(DriftFingerprintError::from)?;
    for (anchor, range_commits) in subject_anchor_ids.iter().zip(range_commits) {
        if let Some(attribution) = anchors.get_mut(anchor) {
            attribution.range_commits = range_commits;
        }
    }
    // A `commits_outside_origin_basis` failure may only remove origin
    // classification; it must not discard commits established by the path log
    // and range-membership query.
    let origin_membership = match origin_basis {
        IncursionCommitOriginBasis::ResolvedTrunk(trunk) => {
            git::commits_outside_origin_basis(repository_root, trunk, &subjects.target).map_or(
                IncursionCommitOriginMembership::CannotClassifyOrigin,
                IncursionCommitOriginMembership::Classified,
            )
        },
        IncursionCommitOriginBasis::CannotClassifyOrigin => {
            IncursionCommitOriginMembership::CannotClassifyOrigin
        },
    };
    Ok(IncursionAttributionBatch {
        anchors,
        commits,
        origin_membership,
    })
}

/// The trunk tip used to classify commit origin, or the semantic reason classification cannot run.
fn trunk_object_id(repository_root: &Path) -> IncursionCommitOriginBasis {
    let Ok(Enrollment::Enrolled(configuration)) = BerthConfig::read(repository_root) else {
        return IncursionCommitOriginBasis::CannotClassifyOrigin;
    };
    git::branch_object_id(repository_root, &configuration.trunk).map_or(
        IncursionCommitOriginBasis::CannotClassifyOrigin,
        IncursionCommitOriginBasis::ResolvedTrunk,
    )
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
