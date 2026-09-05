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
    /// Every holder of the path has no foreign standing against the subject.
    ///
    /// The subject's own reservation in this worktree qualifies, and so does a same-worktree
    /// holder of another run that no longer occupies it — one that has left `Active`, or one
    /// claimed under an identity the engine created for itself rather than one a caller
    /// presented. The probe is the exact inverse of the foreignness the conflict pass applies,
    /// so both read [`Reservation::is_foreign_to_coordination_run_in_worktree`] and cannot
    /// disagree.
    NoForeignStanding,
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
/// Every identified variant names a worktree and the coordination run acting in it. The
/// worktree is the coordination unit and one run occupies it at a time, so both terms are
/// needed: recorded overlap answers bind the worktree, while active work belongs to the
/// run that acquired it.
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
    /// Whether this holder is foreign to the caller: another worktree, or another run
    /// still occupying this one.
    ///
    /// A holder in the caller's own worktree is foreign only while it is `Active` for a
    /// different coordination run, because one run occupies a worktree at a time. The
    /// `Active` term is what keeps a worktree from blocking itself: once a holder reaches
    /// `Outstanding` it has released and is only awaiting integration, and a later session
    /// in the same checkout must be free to edit the paths its predecessor left behind.
    /// Deciding foreignness on the run alone, with no lifecycle term, once did exactly
    /// that.
    ///
    /// The same-worktree case narrows once more on the holder's own identity provenance:
    /// occupancy is a rule between two coordination identities a caller presented, so a
    /// holder claimed under an identity the engine created for itself is never foreign
    /// inside its own worktree. The pre-edit hook therefore lets a run edit over the
    /// reservation post-commit drift first-touched in that same checkout, while a holder in
    /// any other worktree stays foreign exactly as before.
    pub(super) fn is_foreign(self, holder: &Reservation) -> bool {
        match self {
            Self::SessionReservation {
                coordination_run_id,
                worktree_id,
                ..
            }
            | Self::Run {
                coordination_run_id,
                worktree_id,
            } => {
                holder.is_foreign_to_coordination_run_in_worktree(coordination_run_id, worktree_id)
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
