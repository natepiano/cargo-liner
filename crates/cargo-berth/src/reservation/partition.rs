//! How retained reservations partition into the caller's own work and foreign holds.
//!
//! [`AuthorizedEditingIdentity`] is the proven actor identity an overlap decision is taken
//! against, and its methods answer the two questions every partition rests on: whether a
//! holder is foreign, and whether the caller's own reservations already carry an answer for
//! the overlapping scope. [`DriftBlockingCoverage`] and [`WidenScopeBinding`] are the two
//! partitions callers receive.

use super::conflict::ReservationConflict;
use super::lifecycle::EditBlockingStatus;
use super::record::Reservation;
use super::retention::RetainedReservationSet;
use crate::answer::ConflictAuthorization;
use crate::answer::OverlapScopeRevision;
use crate::ids::CoordinationRunId;
use crate::ids::ReservationId;
use crate::ids::WorktreeId;
use crate::scope::PathCase;
use crate::scope::ReservationScope;

/// How current edit-blocking reservations cover one drift path.
pub(crate) enum DriftBlockingCoverage {
    /// Another reservation from the same run and worktree already claims the path.
    SameIdentity,
    /// Reservations from another run or worktree currently block the path.
    Foreign(Vec<ReservationConflict>),
    /// No edit-blocking reservation claims the path.
    Unclaimed,
}

/// The result of re-binding every overlapping scope against existing answers for a proposed
/// widening.
pub(crate) enum WidenScopeBinding {
    /// The complete widened scope set is covered by this durable authorization result.
    Authorized(ConflictAuthorization),
    /// One or more foreign overlaps have no existing answer for their exact scopes.
    Blocked(Vec<ReservationConflict>),
}

/// The actor identity permitted to receive its reservation-specific overlap answers.
///
/// Every identified variant names a worktree, because the worktree is the coordination
/// unit. Two runs in one worktree share one filesystem, one index, and one branch, so
/// they cannot produce the merge collision a reservation exists to prevent.
#[derive(Clone, Copy)]
pub(crate) enum AuthorizedEditingIdentity {
    /// A live session mapping identifies one exact reservation.
    SessionReservation {
        coordination_run_id: CoordinationRunId,
        reservation_id:      ReservationId,
        worktree_id:         WorktreeId,
    },
    /// The environment, a validated marker, or a locked first-touch transaction
    /// identifies this coordination run in this worktree.
    Run {
        coordination_run_id: CoordinationRunId,
        worktree_id:         WorktreeId,
    },
    /// No coordination run can be proven for this edit.
    Unidentified,
}

impl AuthorizedEditingIdentity {
    /// Whether this holder belongs to another worktree, the only foreignness that blocks.
    ///
    /// A holder in the caller's own worktree is never foreign, however many coordination
    /// runs that worktree has issued. A run mismatch alone once blocked here, which let a
    /// worktree block itself with a reservation an earlier session in the same checkout
    /// had left behind.
    pub(super) fn is_foreign(self, holder: &Reservation) -> bool {
        match self {
            Self::SessionReservation { worktree_id, .. } | Self::Run { worktree_id, .. } => {
                holder.actor.worktree != worktree_id
            },
            Self::Unidentified => true,
        }
    }

    pub(super) fn authorizes(
        self,
        reservations: &RetainedReservationSet,
        holder: &Reservation,
        overlap_scope: &ReservationScope,
        path_case: PathCase,
    ) -> bool {
        reservations
            .iter()
            .filter(|requester| {
                self.identifies_requester(requester)
                    && requester.edit_blocking_status() == EditBlockingStatus::Blocking
                    && requester
                        .scopes
                        .as_slice()
                        .iter()
                        .any(|scope| scope.overlaps(overlap_scope, path_case))
            })
            .any(|requester| {
                reservations_authorize_scope(requester, holder, overlap_scope, path_case)
            })
    }

    /// Whether this reservation is one the caller's own worktree holds.
    ///
    /// Overlap answers bind the worktree that recorded them, so a later run in the same
    /// worktree inherits them along with the reservations they were recorded against.
    pub(super) fn identifies_requester(self, requester: &Reservation) -> bool {
        match self {
            Self::SessionReservation { worktree_id, .. } | Self::Run { worktree_id, .. } => {
                requester.actor.worktree == worktree_id
            },
            Self::Unidentified => false,
        }
    }
}

pub(super) fn reservations_authorize_scope(
    requester: &Reservation,
    holder: &Reservation,
    overlap_scope: &ReservationScope,
    path_case: PathCase,
) -> bool {
    let holder_scope_revision = OverlapScopeRevision::from(&holder.scopes);
    let requester_scope_revision = OverlapScopeRevision::from(&requester.scopes);
    requester.authorizations.iter().any(|authorization| {
        authorization.covers(holder.id, &holder_scope_revision, overlap_scope, path_case)
    }) || holder.authorizations.iter().any(|authorization| {
        authorization.covers(
            requester.id,
            &requester_scope_revision,
            overlap_scope,
            path_case,
        )
    })
}
