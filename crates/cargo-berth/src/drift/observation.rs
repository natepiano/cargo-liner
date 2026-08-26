//! The git comparison that produces one working-tree observation.

use std::collections::HashMap;
use std::collections::HashSet;
use std::path::Path;

use super::constants::GIT_CACHED_ARGUMENT;
use super::constants::GIT_DIFF_COMMAND;
use super::constants::GIT_EXCLUDE_STANDARD_ARGUMENT;
use super::constants::GIT_HEAD_REVISION;
use super::constants::GIT_LS_FILES_COMMAND;
use super::constants::GIT_NAME_STATUS_ARGUMENT;
use super::constants::GIT_NO_RENAMES_ARGUMENT;
use super::constants::GIT_NUL_TERMINATED_ARGUMENT;
use super::constants::GIT_OTHERS_ARGUMENT;
use super::constants::GIT_PORCELAIN_ARGUMENT;
use super::constants::GIT_STATUS_COMMAND;
use super::fingerprint;
use super::fingerprint::StoredWorkingTreeFingerprint;
use super::fingerprint::WorkingTreeFingerprint;
use super::git_output;
use super::git_output::DriftFingerprintError;
use super::ordering;
use super::report::DriftComparisonMode;
use super::selection::DriftComparisonChoice;
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
changed_path_set!(CommittedPhaseChanges);
changed_path_set!(StagedWorkingTreeChanges);
changed_path_set!(UnstagedWorkingTreeChanges);
changed_path_set!(UntrackedWorkingTreeChanges);

pub(super) struct CheapDeltaChanges {
    tracked:   CheapTrackedChanges,
    untracked: CheapUntrackedChanges,
}

pub(super) struct FullPhaseStartChanges {
    committed: HashMap<ReservationId, CommittedPhaseChanges>,
    staged:    StagedWorkingTreeChanges,
    unstaged:  UnstagedWorkingTreeChanges,
    untracked: UntrackedWorkingTreeChanges,
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
                        .is_some_and(|paths| !paths.as_slice().is_empty())
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
                if let Some(committed) = changes.committed.get(&reservation_id) {
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
    let status = git_output::run_git(
        repository_root,
        &[
            GIT_STATUS_COMMAND,
            GIT_PORCELAIN_ARGUMENT,
            GIT_NUL_TERMINATED_ARGUMENT,
        ],
    )?;
    let untracked = git_output::run_git(
        repository_root,
        &[
            GIT_LS_FILES_COMMAND,
            GIT_NUL_TERMINATED_ARGUMENT,
            GIT_OTHERS_ARGUMENT,
            GIT_EXCLUDE_STANDARD_ARGUMENT,
        ],
    )?;
    let current = WorkingTreeFingerprint {
        tracked_paths:   git_output::parse_status_paths(&status.stdout)?,
        untracked_paths: git_output::parse_path_list(&untracked.stdout)?,
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
    let mut committed = HashMap::new();
    for reservation_id in reservation_ids {
        let reservation = reservations
            .reservation(*reservation_id)
            .map_err(|error| DriftFingerprintError::Reservation(error.to_string()))?;
        let phase_range = format!(
            "{}..{GIT_HEAD_REVISION}",
            reservation.phase_start_head().as_ref()
        );
        let output = git_output::run_git(
            repository_root,
            &[
                GIT_DIFF_COMMAND,
                GIT_NAME_STATUS_ARGUMENT,
                GIT_NUL_TERMINATED_ARGUMENT,
                GIT_NO_RENAMES_ARGUMENT,
                &phase_range,
            ],
        )?;
        committed.insert(
            *reservation_id,
            CommittedPhaseChanges(git_output::parse_name_status_paths(&output.stdout)?),
        );
    }
    let staged = git_output::run_git(
        repository_root,
        &[
            GIT_DIFF_COMMAND,
            GIT_CACHED_ARGUMENT,
            GIT_NAME_STATUS_ARGUMENT,
            GIT_NUL_TERMINATED_ARGUMENT,
            GIT_NO_RENAMES_ARGUMENT,
            GIT_HEAD_REVISION,
        ],
    )?;
    let unstaged = git_output::run_git(
        repository_root,
        &[
            GIT_DIFF_COMMAND,
            GIT_NAME_STATUS_ARGUMENT,
            GIT_NUL_TERMINATED_ARGUMENT,
            GIT_NO_RENAMES_ARGUMENT,
        ],
    )?;
    let untracked = git_output::run_git(
        repository_root,
        &[
            GIT_LS_FILES_COMMAND,
            GIT_NUL_TERMINATED_ARGUMENT,
            GIT_OTHERS_ARGUMENT,
            GIT_EXCLUDE_STANDARD_ARGUMENT,
        ],
    )?;
    let staged_paths = git_output::parse_name_status_paths(&staged.stdout)?;
    let unstaged_paths = git_output::parse_name_status_paths(&unstaged.stdout)?;
    let untracked_paths = git_output::parse_path_list(&untracked.stdout)?;
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
            staged: StagedWorkingTreeChanges(staged_paths),
            unstaged: UnstagedWorkingTreeChanges(unstaged_paths),
            untracked: UntrackedWorkingTreeChanges(untracked_paths),
        }),
        cache_value,
    })
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
