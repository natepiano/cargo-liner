//! The parsed drift request and the reservations one invocation acts on.

use std::error::Error;
use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;

use super::identity::DriftActingIdentity;
use super::identity::DriftActingRun;
use super::identity::DriftSessionReservation;
use super::ordering;
use super::report::DriftAttributionCandidateSet;
use super::report::DriftComparisonMode;
use crate::ids::CoordinationRunId;
use crate::ids::ReservationId;
use crate::ids::WorktreeId;
use crate::reservation::Reservation;
use crate::reservation::ReservationLifecycle;
use crate::reservation::RetainedReservationSet;

/// Which working-tree comparison the caller requested.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DriftComparisonChoice {
    /// Compare the cheap working-tree observation with the last cache entry.
    CheapDelta,
    /// Compare all active-phase changes with the claim's protected starting commit.
    FullPhaseStart,
}

impl DriftComparisonChoice {
    pub(super) const fn report_mode(self) -> DriftComparisonMode {
        match self {
            Self::CheapDelta => DriftComparisonMode::CheapDelta,
            Self::FullPhaseStart => DriftComparisonMode::FullPhaseStart,
        }
    }
}

/// How a hand-run or hook-run drift command chooses its reservation subjects.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DriftReservationSelection {
    /// Act on the caller-supplied reservation.
    Explicit(ReservationId),
    /// Prefer the session-mapped reservation, otherwise require one active match.
    SessionMappingOrSingleActive,
    /// Report across every local reservation while attributing widening separately.
    EveryActiveForPostCommit {
        /// How an explicit flag or implicit identity selects the widening target.
        widening: PostCommitWideningSelection,
    },
}

impl DriftReservationSelection {
    pub(super) fn resolve(
        self,
        reservations: &RetainedReservationSet,
        acting_identity: DriftActingIdentity,
    ) -> Result<ResolvedDriftSubjects, DriftSelectionError> {
        let worktree = acting_identity.worktree();
        if let Self::EveryActiveForPostCommit { widening } = self {
            return widening.resolve_post_commit(reservations, acting_identity);
        }
        let run = match acting_identity.acting_run() {
            DriftActingRun::Identified(run) => run,
            DriftActingRun::Unidentified => return Err(DriftSelectionError::UnidentifiedActingRun),
        };
        let mut candidates = reservations
            .iter()
            .filter(|reservation| {
                matches!(reservation.lifecycle(), ReservationLifecycle::Active)
                    && reservation.actor().run == run
                    && reservation.actor().worktree == worktree
            })
            .map(Reservation::id)
            .collect::<Vec<_>>();
        ordering::sort_reservation_ids(&mut candidates);
        match self {
            Self::Explicit(reservation_id) if candidates.contains(&reservation_id) => {
                Ok(ResolvedDriftSubjects {
                    reporting:              vec![reservation_id],
                    widening:               DriftWideningSelection::Selected(reservation_id),
                    post_write_first_touch: PostWriteFirstTouchRequirement::NotRequired,
                })
            },
            Self::Explicit(reservation_id) => Err(DriftSelectionError::ExplicitNotActive {
                reservation_id,
                run,
                worktree,
            }),
            Self::SessionMappingOrSingleActive => {
                let selected = match acting_identity.session_reservation() {
                    DriftSessionReservation::Mapped(reservation_id)
                        if candidates.contains(&reservation_id) =>
                    {
                        reservation_id
                    },
                    DriftSessionReservation::Mapped(_) | DriftSessionReservation::Unavailable => {
                        match candidates.as_slice() {
                            [reservation_id] => *reservation_id,
                            [] => {
                                return Err(DriftSelectionError::NoActiveReservation {
                                    run,
                                    worktree,
                                });
                            },
                            _ => {
                                return Err(DriftSelectionError::AmbiguousActiveReservations(
                                    candidates,
                                ));
                            },
                        }
                    },
                };
                Ok(ResolvedDriftSubjects {
                    reporting:              vec![selected],
                    widening:               DriftWideningSelection::Selected(selected),
                    post_write_first_touch: PostWriteFirstTouchRequirement::NotRequired,
                })
            },
            Self::EveryActiveForPostCommit { .. } => {
                Err(DriftSelectionError::NoPostCommitCandidate)
            },
        }
    }
}

/// How a post-commit invocation chooses its one possible widening target.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PostCommitWideningSelection {
    /// Widen only the caller-supplied active reservation.
    Explicit(ReservationId),
    /// Prefer the session mapping, then the only active local candidate.
    SessionMappingOrSingleCandidate,
}

impl PostCommitWideningSelection {
    fn resolve_post_commit(
        self,
        reservations: &RetainedReservationSet,
        acting_identity: DriftActingIdentity,
    ) -> Result<ResolvedDriftSubjects, DriftSelectionError> {
        let worktree = acting_identity.worktree();
        let mut reporting = reservations
            .iter()
            .filter(|reservation| {
                matches!(reservation.lifecycle(), ReservationLifecycle::Active)
                    && reservation.actor().worktree == worktree
            })
            .map(Reservation::id)
            .collect::<Vec<_>>();
        ordering::sort_reservation_ids(&mut reporting);
        let acting_run = acting_identity.acting_run();
        let mut candidates = match acting_run {
            DriftActingRun::Identified(run) => reservations
                .iter()
                .filter(|reservation| {
                    matches!(reservation.lifecycle(), ReservationLifecycle::Active)
                        && reservation.actor().run == run
                        && reservation.actor().worktree == worktree
                })
                .map(Reservation::id)
                .collect::<Vec<_>>(),
            DriftActingRun::Unidentified => Vec::new(),
        };
        ordering::sort_reservation_ids(&mut candidates);
        let post_write_first_touch = match candidates.as_slice() {
            [] => PostWriteFirstTouchRequirement::Required,
            [_, ..] => PostWriteFirstTouchRequirement::NotRequired,
        };
        let widening = match self {
            Self::Explicit(reservation_id) => {
                let DriftActingRun::Identified(run) = acting_run else {
                    return Err(DriftSelectionError::UnidentifiedActingRun);
                };
                if !candidates.contains(&reservation_id) {
                    return Err(DriftSelectionError::ExplicitNotActive {
                        reservation_id,
                        run,
                        worktree,
                    });
                }
                DriftWideningSelection::Selected(reservation_id)
            },
            Self::SessionMappingOrSingleCandidate
                if matches!(acting_run, DriftActingRun::Unidentified) =>
            {
                DriftWideningSelection::NotNeeded
            },
            Self::SessionMappingOrSingleCandidate => match acting_identity.session_reservation() {
                DriftSessionReservation::Mapped(reservation_id)
                    if candidates.contains(&reservation_id) =>
                {
                    DriftWideningSelection::Selected(reservation_id)
                },
                DriftSessionReservation::Mapped(_) | DriftSessionReservation::Unavailable => {
                    match candidates.as_slice() {
                        [] => DriftWideningSelection::NotNeeded,
                        [reservation_id] => DriftWideningSelection::Selected(*reservation_id),
                        _ => DriftWideningSelection::Ambiguous(
                            DriftAttributionCandidateSet::try_from(candidates)
                                .map_err(|_| DriftSelectionError::NoPostCommitCandidate)?,
                        ),
                    }
                },
            },
        };
        Ok(ResolvedDriftSubjects {
            reporting,
            widening,
            post_write_first_touch,
        })
    }
}

/// A drift request after clap primitives have been converted into domain choices.
#[derive(Clone, Copy)]
pub(crate) struct DriftRequest {
    /// The comparison algorithm selected at the command boundary.
    pub(crate) comparison:  DriftComparisonChoice,
    /// The semantic reservation-selection rule.
    pub(crate) reservation: DriftReservationSelection,
}

pub(super) struct ResolvedDriftSubjects {
    pub(super) reporting:              Vec<ReservationId>,
    pub(super) widening:               DriftWideningSelection,
    pub(super) post_write_first_touch: PostWriteFirstTouchRequirement,
}

#[derive(Clone, Copy)]
pub(super) enum PostWriteFirstTouchRequirement {
    NotRequired,
    Required,
}

pub(super) enum DriftWideningSelection {
    NotNeeded,
    Selected(ReservationId),
    Ambiguous(DriftAttributionCandidateSet),
}

/// A caller identity could not choose a safe drift subject.
#[derive(Debug)]
pub(super) enum DriftSelectionError {
    UnidentifiedActingRun,
    NoPostCommitCandidate,
    NoActiveReservation {
        run:      CoordinationRunId,
        worktree: WorktreeId,
    },
    AmbiguousActiveReservations(Vec<ReservationId>),
    ExplicitNotActive {
        reservation_id: ReservationId,
        run:            CoordinationRunId,
        worktree:       WorktreeId,
    },
}

impl Display for DriftSelectionError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnidentifiedActingRun => formatter
                .write_str("drift requires a live session mapping, active coordination-run marker, or CARGO_BERTH_RUN"),
            Self::NoPostCommitCandidate => formatter
                .write_str("post-commit drift found no active reservation candidate"),
            Self::NoActiveReservation { run, worktree } => write!(
                formatter,
                "coordination run {run} has no active reservation in worktree {worktree}"
            ),
            Self::AmbiguousActiveReservations(candidates) => write!(
                formatter,
                "drift is ambiguous; choose one active reservation with --reservation: {}",
                candidates
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Self::ExplicitNotActive {
                reservation_id,
                run,
                worktree,
            } => write!(
                formatter,
                "reservation {reservation_id} is not active for coordination run {run} in worktree {worktree}"
            ),
        }
    }
}

impl Error for DriftSelectionError {}
