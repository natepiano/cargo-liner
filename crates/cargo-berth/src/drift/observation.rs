//! The git comparison that produces one working-tree observation.

use std::collections::HashMap;
use std::collections::HashSet;
use std::path::Path;
use std::thread;

use super::constants::GIT_NO_RENAMES_ARGUMENT;
use super::constants::GIT_NUL_TERMINATED_ARGUMENT;
use super::constants::GIT_PORCELAIN_ARGUMENT;
use super::constants::GIT_STATUS_COMMAND;
use super::constants::GIT_UNTRACKED_FILES_ALL_ARGUMENT;
use super::fingerprint;
use super::fingerprint::StoredWorkingTreeFingerprint;
use super::fingerprint::WorkingTreeFingerprint;
use super::git_output;
use super::git_output::DriftFingerprintError;
use super::git_output::FullDriftObservationActivity;
use super::git_output::IncursionAttributionAnchorState;
use super::git_output::WorkingTreeChangePartition;
use super::ordering;
use super::report::DriftComparisonMode;
use super::selection::DriftComparisonChoice;
use crate::git;
use crate::ids::GitObjectId;
use crate::ids::ReservationId;
use crate::ids::ReservationScopePath;
use crate::reservation::RetainedReservationSet;

macro_rules! changed_path_set {
    ($name:ident) => {
        struct $name(Vec<ReservationScopePath>);

        impl $name {
            fn as_slice(&self) -> &[ReservationScopePath] { &self.0 }
        }
    };
}

changed_path_set!(CheapTrackedChanges);
changed_path_set!(CheapUntrackedChanges);
changed_path_set!(StagedWorkingTreeChanges);
changed_path_set!(UnstagedWorkingTreeChanges);
changed_path_set!(UntrackedWorkingTreeChanges);

struct CommittedPhaseChanges(Vec<ReservationScopePath>);

impl CommittedPhaseChanges {
    fn as_slice(&self) -> &[ReservationScopePath] { &self.0 }
}

enum FullReservationPhaseHistory {
    Compared(CommittedPhaseChanges),
    PhaseStartObjectUnknown(GitObjectId),
}

/// The phase-history state available while classifying one reservation.
pub(super) enum ReservationPhaseHistory<'history> {
    /// Cheap comparison did not inspect committed phase history.
    NotObserved,
    /// Git compared the phase start with the observation target.
    Compared(&'history [ReservationScopePath]),
    /// Git could not read the reservation's phase-start object.
    PhaseStartObjectUnknown(&'history GitObjectId),
}

pub(super) struct CheapDeltaChanges {
    tracked:   CheapTrackedChanges,
    untracked: CheapUntrackedChanges,
    /// The paths still modified when the comparison was taken.
    ///
    /// The symmetric difference above answers "which paths moved since the last
    /// observation", so it names a path restored to its committed content alongside
    /// one that was edited. Only this set separates the two.
    modified:  HashSet<ReservationScopePath>,
}

pub(super) struct FullPhaseStartChanges {
    committed: HashMap<ReservationId, FullReservationPhaseHistory>,
    history:   FullPhaseHistoryObservation,
    staged:    StagedWorkingTreeChanges,
    unstaged:  UnstagedWorkingTreeChanges,
    untracked: UntrackedWorkingTreeChanges,
}

/// The shared target and phase-start states established by one full comparison.
pub(super) enum FullPhaseHistoryObservation {
    /// No reservation supplied a phase start, so no phase history was queried.
    NoReservationAnchor,
    /// Every requested phase start was classified against this target.
    Anchored {
        target:        GitObjectId,
        anchor_states: HashMap<GitObjectId, IncursionAttributionAnchorState>,
    },
}

impl FullPhaseStartChanges {
    pub(super) const fn phase_history(&self) -> &FullPhaseHistoryObservation { &self.history }
}

pub(super) enum ObservedDriftChanges {
    Cheap(CheapDeltaChanges),
    Full(FullPhaseStartChanges),
}

impl ObservedDriftChanges {
    pub(super) fn has_changes_for(&self, reservation_ids: &[ReservationId]) -> bool {
        match self {
            Self::Cheap(changes) => {
                !changes.tracked.as_slice().is_empty() || !changes.untracked.as_slice().is_empty()
            },
            Self::Full(changes) => {
                reservation_ids.iter().any(|reservation_id| {
                    changes
                        .committed
                        .get(reservation_id)
                        .is_some_and(|history| match history {
                            FullReservationPhaseHistory::Compared(paths) => {
                                !paths.as_slice().is_empty()
                            },
                            FullReservationPhaseHistory::PhaseStartObjectUnknown(_) => true,
                        })
                }) || !changes.staged.as_slice().is_empty()
                    || !changes.unstaged.as_slice().is_empty()
                    || !changes.untracked.as_slice().is_empty()
            },
        }
    }

    fn observed_paths(&self) -> Vec<ReservationScopePath> {
        let mut paths = match self {
            Self::Cheap(changes) => changes
                .tracked
                .0
                .iter()
                .chain(&changes.untracked.0)
                .cloned()
                .collect::<Vec<_>>(),
            Self::Full(changes) => changes
                .staged
                .0
                .iter()
                .chain(&changes.unstaged.0)
                .chain(&changes.untracked.0)
                .cloned()
                .collect::<Vec<_>>(),
        };
        ordering::normalize_paths(&mut paths);
        paths
    }

    /// Whether an observed path carries work the acting worktree could acquire.
    ///
    /// Widening is an acquisition, so it needs a path that carries work. Incursion and
    /// collision classification do not: there "what moved since the last observation" is
    /// the right question, and a path another worktree holds is worth reporting however
    /// it moved.
    ///
    /// Every component of a full comparison is a positive statement about the present —
    /// what this phase committed, what differs from `HEAD` now, what is untracked now.
    /// None of them is a symmetric difference, so none can name a restored path.
    pub(super) fn carries_work(&self, path: &ReservationScopePath) -> bool {
        match self {
            Self::Cheap(changes) => changes.modified.contains(path),
            Self::Full(_) => true,
        }
    }

    /// Return the committed-history state observed for one reservation.
    pub(super) fn reservation_phase_history(
        &self,
        reservation_id: ReservationId,
    ) -> ReservationPhaseHistory<'_> {
        match self {
            Self::Cheap(_) => ReservationPhaseHistory::NotObserved,
            Self::Full(changes) => match changes.committed.get(&reservation_id) {
                Some(FullReservationPhaseHistory::Compared(paths)) => {
                    ReservationPhaseHistory::Compared(paths.as_slice())
                },
                Some(FullReservationPhaseHistory::PhaseStartObjectUnknown(phase_start)) => {
                    ReservationPhaseHistory::PhaseStartObjectUnknown(phase_start)
                },
                None => ReservationPhaseHistory::NotObserved,
            },
        }
    }

    pub(super) fn visit_paths(
        &self,
        reservation_id: ReservationId,
        mut visit: impl FnMut(&ReservationScopePath),
    ) {
        match self {
            Self::Cheap(changes) => {
                for path in changes.tracked.as_slice() {
                    visit(path);
                }
                for path in changes.untracked.as_slice() {
                    visit(path);
                }
            },
            Self::Full(changes) => {
                if let Some(FullReservationPhaseHistory::Compared(committed)) =
                    changes.committed.get(&reservation_id)
                {
                    for path in committed.as_slice() {
                        visit(path);
                    }
                }
                for path in changes.staged.as_slice() {
                    visit(path);
                }
                for path in changes.unstaged.as_slice() {
                    visit(path);
                }
                for path in changes.untracked.as_slice() {
                    visit(path);
                }
            },
        }
    }
}

pub(super) struct FingerprintObservation {
    pub(super) comparison:  DriftComparisonMode,
    pub(super) changes:     ObservedDriftChanges,
    pub(super) cache_value: WorkingTreeFingerprint,
}

impl FingerprintObservation {
    /// The observed paths a post-write first-touch claim may acquire.
    ///
    /// A cheap comparison answers "which paths moved since the last observation",
    /// so it reports a path restored to its committed content alongside one that
    /// was edited. A restored path carries no work for a new reservation to
    /// protect, and the fingerprint about to be cached lists only the paths
    /// modified at the moment of the call, so it decides what may be claimed.
    pub(super) fn post_write_claim_subject(&self) -> PostWriteClaimSubject {
        let modified = self.cache_value.modified_paths();
        PostWriteClaimSubject::from_modified(
            self.changes
                .observed_paths()
                .into_iter()
                .filter(|path| modified.contains(path))
                .collect(),
        )
    }
}

/// Which observed paths a post-write first-touch claim may acquire.
pub(super) enum PostWriteClaimSubject {
    /// Every observed path is back to its committed content, so none carries work.
    NoModifiedPath,
    /// These observed paths are still modified in the working tree.
    ModifiedPaths(Vec<ReservationScopePath>),
}

impl PostWriteClaimSubject {
    fn from_modified(paths: Vec<ReservationScopePath>) -> Self {
        if paths.is_empty() {
            Self::NoModifiedPath
        } else {
            Self::ModifiedPaths(paths)
        }
    }
}

pub(super) fn observe(
    choice: DriftComparisonChoice,
    repository_root: &Path,
    reservations: &RetainedReservationSet,
    reservation_ids: &[ReservationId],
    cache_path: &Path,
) -> Result<FingerprintObservation, DriftFingerprintError> {
    match choice {
        DriftComparisonChoice::CheapDelta => match fingerprint::read_fingerprint(cache_path) {
            StoredWorkingTreeFingerprint::Available(previous) => {
                observe_cheap(repository_root, &previous)
            },
            StoredWorkingTreeFingerprint::Unavailable => observe_full(
                repository_root,
                reservations,
                reservation_ids,
                DriftComparisonMode::FullPhaseStartFallback,
            ),
        },
        DriftComparisonChoice::FullPhaseStart => observe_full(
            repository_root,
            reservations,
            reservation_ids,
            DriftComparisonMode::FullPhaseStart,
        ),
    }
}

fn observe_cheap(
    repository_root: &Path,
    previous: &WorkingTreeFingerprint,
) -> Result<FingerprintObservation, DriftFingerprintError> {
    let working_tree_status = observe_working_tree_status(repository_root)?;
    let current = WorkingTreeFingerprint {
        tracked_paths:   working_tree_status.tracked(),
        untracked_paths: working_tree_status.untracked,
    }
    .normalized();
    let changes = CheapDeltaChanges {
        tracked:   CheapTrackedChanges(symmetric_difference(
            &previous.tracked_paths,
            &current.tracked_paths,
        )),
        untracked: CheapUntrackedChanges(symmetric_difference(
            &previous.untracked_paths,
            &current.untracked_paths,
        )),
        modified:  current.modified_paths().into_iter().cloned().collect(),
    };
    Ok(FingerprintObservation {
        comparison:  DriftComparisonMode::CheapDelta,
        changes:     ObservedDriftChanges::Cheap(changes),
        cache_value: current,
    })
}

fn observe_full(
    repository_root: &Path,
    reservations: &RetainedReservationSet,
    reservation_ids: &[ReservationId],
    comparison: DriftComparisonMode,
) -> Result<FingerprintObservation, DriftFingerprintError> {
    let mut reservation_anchors = HashMap::new();
    let mut anchors = Vec::new();
    for reservation_id in reservation_ids {
        let reservation = reservations
            .reservation(*reservation_id)
            .map_err(|error| DriftFingerprintError::Reservation(error.to_string()))?;
        let phase_start = reservation.phase_start_head().as_ref().clone();
        if !anchors.contains(&phase_start) {
            anchors.push(phase_start.clone());
        }
        reservation_anchors.insert(*reservation_id, phase_start);
    }
    let (phase_history, working_tree_status) = thread::scope(|scope| {
        let phase_history_worker = scope.spawn(|| observe_phase_history(repository_root, &anchors));
        let working_tree_status_worker =
            scope.spawn(|| observe_working_tree_status(repository_root));
        (
            join_full_observation_worker(
                phase_history_worker,
                FullDriftObservationActivity::PhaseHistory,
            ),
            join_full_observation_worker(
                working_tree_status_worker,
                FullDriftObservationActivity::WorkingTreeStatus,
            ),
        )
    });
    let (history, committed_by_anchor) = phase_history?;
    let working_tree_status = working_tree_status?;
    let anchor_states = match &history {
        FullPhaseHistoryObservation::NoReservationAnchor => HashMap::new(),
        FullPhaseHistoryObservation::Anchored { anchor_states, .. } => anchor_states.clone(),
    };
    let committed = reservation_anchors
        .into_iter()
        .map(
            |(reservation_id, anchor)| -> Result<_, DriftFingerprintError> {
                let phase_history = match anchor_states.get(&anchor) {
                    Some(IncursionAttributionAnchorState::ObjectUnknown) => {
                        FullReservationPhaseHistory::PhaseStartObjectUnknown(anchor)
                    },
                    Some(
                        IncursionAttributionAnchorState::UsableAncestor
                        | IncursionAttributionAnchorState::NotAncestorOfHead,
                    ) => FullReservationPhaseHistory::Compared(CommittedPhaseChanges(
                        committed_by_anchor.get(&anchor).cloned().ok_or_else(|| {
                            DriftFingerprintError::MalformedGitOutput(format!(
                                "phase comparison omitted requested anchor {anchor}"
                            ))
                        })?,
                    )),
                    None => {
                        return Err(DriftFingerprintError::MalformedGitOutput(format!(
                            "phase reachability omitted requested anchor {anchor}"
                        )));
                    },
                };
                Ok((reservation_id, phase_history))
            },
        )
        .collect::<Result<HashMap<_, _>, _>>()?;
    let WorkingTreeChangePartition {
        staged: staged_paths,
        unstaged: unstaged_paths,
        untracked: untracked_paths,
    } = working_tree_status;
    let mut tracked_cache_paths = staged_paths.clone();
    tracked_cache_paths.extend(unstaged_paths.iter().cloned());
    ordering::normalize_paths(&mut tracked_cache_paths);
    let cache_value = WorkingTreeFingerprint {
        tracked_paths:   tracked_cache_paths,
        untracked_paths: untracked_paths.clone(),
    }
    .normalized();
    Ok(FingerprintObservation {
        comparison,
        changes: ObservedDriftChanges::Full(FullPhaseStartChanges {
            committed,
            history,
            staged: StagedWorkingTreeChanges(staged_paths),
            unstaged: UnstagedWorkingTreeChanges(unstaged_paths),
            untracked: UntrackedWorkingTreeChanges(untracked_paths),
        }),
        cache_value,
    })
}

fn observe_working_tree_status(
    repository_root: &Path,
) -> Result<WorkingTreeChangePartition, DriftFingerprintError> {
    let status = git_output::run_git(
        repository_root,
        &[
            GIT_STATUS_COMMAND,
            GIT_PORCELAIN_ARGUMENT,
            GIT_NUL_TERMINATED_ARGUMENT,
            GIT_NO_RENAMES_ARGUMENT,
            GIT_UNTRACKED_FILES_ALL_ARGUMENT,
        ],
    )?;
    git_output::parse_working_tree_status(&status.stdout)
}

fn join_full_observation_worker<T>(
    worker: thread::ScopedJoinHandle<'_, Result<T, DriftFingerprintError>>,
    activity: FullDriftObservationActivity,
) -> Result<T, DriftFingerprintError> {
    worker
        .join()
        .map_err(|_| DriftFingerprintError::WorkerPanicked { activity })?
}

fn observe_phase_history(
    repository_root: &Path,
    anchors: &[GitObjectId],
) -> Result<
    (
        FullPhaseHistoryObservation,
        HashMap<GitObjectId, Vec<ReservationScopePath>>,
    ),
    DriftFingerprintError,
> {
    if anchors.is_empty() {
        return Ok((
            FullPhaseHistoryObservation::NoReservationAnchor,
            HashMap::new(),
        ));
    }
    let (target, reachability) = match git::head_commit_reachability(repository_root, anchors)
        .map_err(DriftFingerprintError::from)?
    {
        git::CommitTargetReachability::Resolved { target, candidates } => (target, candidates),
        git::CommitTargetReachability::Missing => {
            return Err(DriftFingerprintError::MalformedGitOutput(
                "HEAD did not resolve to an object".to_owned(),
            ));
        },
        git::CommitTargetReachability::Ambiguous => {
            return Err(DriftFingerprintError::MalformedGitOutput(
                "HEAD resolved ambiguously".to_owned(),
            ));
        },
        git::CommitTargetReachability::WrongType { object_type } => {
            return Err(DriftFingerprintError::MalformedGitOutput(format!(
                "HEAD resolved to {object_type}, not a commit"
            )));
        },
    };
    let anchor_states = anchors
        .iter()
        .cloned()
        .zip(reachability)
        .map(|(anchor, reachability)| {
            let reachability = match reachability {
                git::CommitCandidateReachability::Ancestor => git::Reachability::Ancestor,
                git::CommitCandidateReachability::NotAncestor => git::Reachability::NotAncestor,
                git::CommitCandidateReachability::Missing
                | git::CommitCandidateReachability::Ambiguous
                | git::CommitCandidateReachability::WrongType { .. } => {
                    git::Reachability::ObjectUnknown
                },
            };
            (anchor, reachability.into())
        })
        .collect::<HashMap<_, _>>();
    let comparable_anchors = anchors
        .iter()
        .filter(|anchor| {
            anchor_states.get(*anchor) != Some(&IncursionAttributionAnchorState::ObjectUnknown)
        })
        .cloned()
        .collect::<Vec<_>>();
    let committed_by_anchor = if comparable_anchors.is_empty() {
        HashMap::new()
    } else {
        let execution =
            git::phase_committed_path_diffs(repository_root, &comparable_anchors, &target);
        let output =
            git_output::completed_git_output(execution, &["diff-tree", "batched phase starts"])?;
        git_output::parse_phase_committed_paths(&output.stdout, &comparable_anchors)?
    };
    Ok((
        FullPhaseHistoryObservation::Anchored {
            target,
            anchor_states,
        },
        committed_by_anchor,
    ))
}

fn symmetric_difference(
    previous: &[ReservationScopePath],
    current: &[ReservationScopePath],
) -> Vec<ReservationScopePath> {
    let previous_names = previous
        .iter()
        .map(ToString::to_string)
        .collect::<HashSet<_>>();
    let current_names = current
        .iter()
        .map(ToString::to_string)
        .collect::<HashSet<_>>();
    let mut paths = previous
        .iter()
        .filter(|path| !current_names.contains(&path.to_string()))
        .chain(
            current
                .iter()
                .filter(|path| !previous_names.contains(&path.to_string())),
        )
        .cloned()
        .collect::<Vec<_>>();
    ordering::normalize_paths(&mut paths);
    paths
}
