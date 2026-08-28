//! The coordination-run, reservation, and worktree identity behind one drift invocation.

use super::selection::DriftReservationSelection;
use super::selection::DriftSelectionError;
use crate::ids::CoordinationRunId;
use crate::ids::ReservationId;
use crate::ids::WorktreeId;
use crate::ledger::ResolvedEditAuthorization;
use crate::reservation::AuthorizedEditingIdentity;
use crate::reservation::RetainedReservationSet;

#[derive(Clone, Copy)]
pub(super) enum DriftActingIdentity {
    Session {
        run:         CoordinationRunId,
        reservation: ReservationId,
        worktree:    WorktreeId,
    },
    Run {
        run:      CoordinationRunId,
        worktree: WorktreeId,
    },
    Unidentified {
        worktree: WorktreeId,
    },
}

impl DriftActingIdentity {
    pub(super) fn resolve(
        resolved_edit_authorization: ResolvedEditAuthorization,
        reservations: &RetainedReservationSet,
    ) -> Self {
        match reservations
            .resolve_editing_identity(resolved_edit_authorization.edit_authorization())
        {
            AuthorizedEditingIdentity::SessionReservation {
                coordination_run_id: run,
                reservation_id: reservation,
                worktree_id: worktree,
            } => Self::Session {
                run,
                reservation,
                worktree,
            },
            AuthorizedEditingIdentity::Run {
                coordination_run_id: run,
                worktree_id: worktree,
            } => Self::Run { run, worktree },
            AuthorizedEditingIdentity::Unidentified => Self::Unidentified {
                worktree: resolved_edit_authorization.worktree_id,
            },
        }
    }

    pub(super) const fn worktree(self) -> WorktreeId {
        match self {
            Self::Session { worktree, .. }
            | Self::Run { worktree, .. }
            | Self::Unidentified { worktree } => worktree,
        }
    }

    pub(super) const fn acting_run(self) -> DriftActingRun {
        match self {
            Self::Session { run, .. } | Self::Run { run, .. } => DriftActingRun::Identified(run),
            Self::Unidentified { .. } => DriftActingRun::Unidentified,
        }
    }

    pub(super) const fn session_reservation(self) -> DriftSessionReservation {
        match self {
            Self::Session { reservation, .. } => DriftSessionReservation::Mapped(reservation),
            Self::Run { .. } | Self::Unidentified { .. } => DriftSessionReservation::Unavailable,
        }
    }

    pub(super) fn run_for_mutation(
        self,
        reservation_selection: DriftReservationSelection,
    ) -> Result<DriftMutationActorRun, DriftSelectionError> {
        match self.acting_run() {
            DriftActingRun::Identified(run) => Ok(DriftMutationActorRun::Identified(run)),
            DriftActingRun::Unidentified
                if matches!(
                    reservation_selection,
                    DriftReservationSelection::EveryActiveForPostCommit { .. }
                ) =>
            {
                Ok(DriftMutationActorRun::PostCommitInvocation(
                    CoordinationRunId::new(),
                ))
            },
            DriftActingRun::Unidentified => Err(DriftSelectionError::UnidentifiedActingRun),
        }
    }
}

#[derive(Clone, Copy)]
pub(super) enum DriftActingRun {
    Identified(CoordinationRunId),
    Unidentified,
}

pub(super) enum DriftSessionReservation {
    Mapped(ReservationId),
    Unavailable,
}

/// The run identity recorded on drift mutations from this invocation.
pub(super) enum DriftMutationActorRun {
    /// The process or validated worktree marker identified the invoking run.
    Identified(CoordinationRunId),
    /// An unidentified post-commit invocation received a transaction-only run identity.
    PostCommitInvocation(CoordinationRunId),
}

impl DriftMutationActorRun {
    pub(super) const fn into_coordination_run_id(self) -> CoordinationRunId {
        match self {
            Self::Identified(coordination_run_id)
            | Self::PostCommitInvocation(coordination_run_id) => coordination_run_id,
        }
    }
}
