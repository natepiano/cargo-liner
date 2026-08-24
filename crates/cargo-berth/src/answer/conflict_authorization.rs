//! Durable authorization recorded with a claim or widen operation.

use serde::Deserialize;
use serde::Serialize;

use super::proposal::OverlapAuthorizationReason;
use super::proposal::OverlapProposal;
use super::proposal::PermissiveOverlapAnswer;
use super::scope_binding::AuthorizedOverlap;
use super::scope_binding::AuthorizedOverlapSet;
use super::scope_binding::OverlapScopeRevision;
use crate::ids::EdgeId;
use crate::ids::ReservationId;
use crate::ledger::OrderingDirection;
use crate::scope::PathCase;
use crate::scope::ReservationScope;

/// The complete overlap decision recorded within a claim or widen transaction.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum ConflictAuthorization {
    /// No foreign overlap existed when the transaction acquired these scopes.
    NoConflict,
    /// An ordering edge authorizes this exact observed overlap set.
    Sequence {
        /// The exact holder bindings shown to the user.
        overlaps:  AuthorizedOverlapSet,
        /// The holder named as the other endpoint of the ordering edge.
        blocker:   ReservationId,
        /// The requested ordering direction.
        direction: OrderingDirection,
        /// The edge born with this acquisition.
        edge_id:   EdgeId,
        /// The approved reason for selecting an order.
        reason:    OverlapAuthorizationReason,
    },
    /// Editing can proceed while integration remains held pending an order.
    Defer {
        /// The exact holder bindings shown to the user.
        overlaps: AuthorizedOverlapSet,
        /// The holder whose overlap the caller answered.
        blocker:  ReservationId,
        /// The approved reason for delaying the order.
        reason:   OverlapAuthorizationReason,
    },
    /// Editing can proceed without declaring an ordering relationship.
    Override {
        /// The exact holder bindings shown to the user.
        overlaps: AuthorizedOverlapSet,
        /// The holder whose overlap the caller answered.
        blocker:  ReservationId,
        /// The approved reason for accepting the conflict.
        reason:   OverlapAuthorizationReason,
    },
    /// Existing answers still cover every current foreign overlap after a widen.
    Revalidated {
        /// The exact current holder bindings covered by those earlier answers.
        overlaps: AuthorizedOverlapSet,
    },
}

impl ConflictAuthorization {
    /// Build the durable authorization from a proposal that matched under the lock.
    pub(crate) fn from_approved_proposal(proposal: OverlapProposal) -> Self {
        let (answer, overlaps, authorization_reason) = proposal.into_authorization_parts();
        match answer {
            PermissiveOverlapAnswer::Sequence { blocker, direction } => Self::Sequence {
                overlaps,
                blocker,
                direction,
                edge_id: EdgeId::new(),
                reason: authorization_reason,
            },
            PermissiveOverlapAnswer::Defer { blocker } => Self::Defer {
                overlaps,
                blocker,
                reason: authorization_reason,
            },
            PermissiveOverlapAnswer::Override { blocker } => Self::Override {
                overlaps,
                blocker,
                reason: authorization_reason,
            },
        }
    }

    /// Borrow the exact holder bindings covered by this authorization.
    pub(crate) fn authorized_overlaps(&self) -> &[AuthorizedOverlap] {
        match self {
            Self::NoConflict => &[],
            Self::Sequence { overlaps, .. }
            | Self::Defer { overlaps, .. }
            | Self::Override { overlaps, .. }
            | Self::Revalidated { overlaps } => overlaps.as_slice(),
        }
    }

    /// Return whether this answer covers one exact counterpart and scope.
    pub(crate) fn covers(
        &self,
        counterpart_id: ReservationId,
        counterpart_scope_revision: &OverlapScopeRevision,
        overlap_scope: &ReservationScope,
        path_case: PathCase,
    ) -> bool {
        self.authorized_overlaps().iter().any(|authorized_overlap| {
            authorized_overlap.covers(
                counterpart_id,
                counterpart_scope_revision,
                overlap_scope,
                path_case,
            )
        })
    }
}
