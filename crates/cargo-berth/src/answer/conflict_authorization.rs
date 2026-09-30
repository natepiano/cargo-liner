//! Durable authorization recorded with a claim or widen operation.

use serde::Deserialize;
use serde::Serialize;

use super::request::OverlapAuthorizationReason;
use super::request::PermissiveOverlapAnswer;
use super::scope_binding::AuthorizedOverlapSet;
use super::scope_binding::OverlapScopeRevision;
use crate::ids::EdgeId;
use crate::ids::ReservationId;
use crate::ledger::OrderingDirection;
use crate::ledger::ReservationScope;
use crate::scope::PathCase;

/// The complete overlap decision recorded within a claim or widen transaction.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum ConflictAuthorization {
    /// No foreign overlap existed when the transaction acquired these scopes.
    NoConflict,
    /// Enrollment preserves editing while shared work awaits an integration order.
    Enrollment {
        /// The counterpart reservations and shared scopes observed at enrollment.
        overlaps: AuthorizedOverlapSet,
    },
    /// An ordering edge authorizes the shared scopes of this observed overlap set.
    Sequence {
        /// The exact holder bindings observed under the ledger lock.
        overlaps:  AuthorizedOverlapSet,
        /// The holder named as the other endpoint of the ordering edge.
        blocker:   ReservationId,
        /// The requested ordering direction.
        direction: OrderingDirection,
        /// The edge born with this acquisition.
        edge_id:   EdgeId,
        /// The caller's reason for selecting an order.
        reason:    OverlapAuthorizationReason,
    },
    /// Editing can proceed while integration remains held pending an order.
    Defer {
        /// The exact holder bindings observed under the ledger lock.
        overlaps: AuthorizedOverlapSet,
        /// The holder whose overlap the caller answered.
        blocker:  ReservationId,
        /// The caller's reason for delaying the order.
        reason:   OverlapAuthorizationReason,
    },
    /// Editing can proceed without declaring an ordering relationship.
    Override {
        /// The exact holder bindings observed under the ledger lock.
        overlaps: AuthorizedOverlapSet,
        /// The holder whose overlap the caller answered.
        blocker:  ReservationId,
        /// The caller's reason for accepting the conflict.
        reason:   OverlapAuthorizationReason,
    },
    /// Existing answers cover every current foreign overlap after a widen.
    ExistingAnswersCoverEveryOverlap {
        /// The exact current holder bindings covered by those earlier answers.
        overlaps: AuthorizedOverlapSet,
    },
}

impl ConflictAuthorization {
    /// Record an answer whose named holder is the only conflict observed under the lock.
    pub(crate) fn answered(
        answer: PermissiveOverlapAnswer,
        overlaps: AuthorizedOverlapSet,
        reason: OverlapAuthorizationReason,
    ) -> Self {
        match answer {
            PermissiveOverlapAnswer::Sequence { blocker, direction } => Self::Sequence {
                overlaps,
                blocker,
                direction,
                edge_id: EdgeId::new(),
                reason,
            },
            PermissiveOverlapAnswer::Defer { blocker } => Self::Defer {
                overlaps,
                blocker,
                reason,
            },
            PermissiveOverlapAnswer::Override { blocker } => Self::Override {
                overlaps,
                blocker,
                reason,
            },
        }
    }

    /// Return whether this answer covers one exact counterpart and scope.
    ///
    /// An enrollment or a recorded answer covers the shared scopes it names whatever else
    /// the counterpart later protects, so a holder that widens elsewhere leaves the answer
    /// standing. A scope newly shared with that holder is outside the answered set and still
    /// needs an answer of its own. A widen covered by existing answers binds only the exact
    /// counterpart revision it observed.
    pub(crate) fn covers(
        &self,
        counterpart_id: ReservationId,
        counterpart_scope_revision: &OverlapScopeRevision,
        overlap_scope: &ReservationScope,
        path_case: PathCase,
    ) -> bool {
        match self {
            Self::NoConflict => false,
            Self::Enrollment { overlaps }
            | Self::Sequence { overlaps, .. }
            | Self::Defer { overlaps, .. }
            | Self::Override { overlaps, .. } => {
                overlaps.as_slice().iter().any(|authorized_overlap| {
                    authorized_overlap.covers_shared_scope(counterpart_id, overlap_scope, path_case)
                })
            },
            Self::ExistingAnswersCoverEveryOverlap { overlaps } => {
                overlaps.as_slice().iter().any(|authorized_overlap| {
                    authorized_overlap.covers(
                        counterpart_id,
                        counterpart_scope_revision,
                        overlap_scope,
                        path_case,
                    )
                })
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use super::ConflictAuthorization;
    use crate::answer::AuthorizedOverlap;
    use crate::answer::AuthorizedOverlapSet;
    use crate::answer::OverlapScopeRevision;
    use crate::ids::EdgeId;
    use crate::ids::ReservationId;
    use crate::ledger::OrderingDirection;
    use crate::ledger::ReservationScope;
    use crate::ledger::ReservationScopeSet;
    use crate::ledger::ScopeKind;
    use crate::scope::PathCase;

    #[test]
    fn answers_cover_shared_scopes_after_unrelated_widen() -> Result<(), Box<dyn Error>> {
        let counterpart = ReservationId::new();
        let shared = ReservationScope {
            path: "shared.rs".parse()?,
            kind: ScopeKind::File,
        };
        let added = ReservationScope {
            path: "other.rs".parse()?,
            kind: ScopeKind::File,
        };
        let original_scopes = ReservationScopeSet::try_from(vec![shared.clone()])?;
        let original_revision = OverlapScopeRevision::from(&original_scopes);
        let widened_revision = OverlapScopeRevision::from(&ReservationScopeSet::try_from(vec![
            shared.clone(),
            added.clone(),
        ])?);
        let overlaps = AuthorizedOverlapSet::from(AuthorizedOverlap {
            reservation_id: counterpart,
            scope_revision: original_revision.clone(),
            scopes:         original_scopes.into(),
        });
        let answers = [
            ConflictAuthorization::Enrollment {
                overlaps: overlaps.clone(),
            },
            ConflictAuthorization::Sequence {
                overlaps:  overlaps.clone(),
                blocker:   counterpart,
                direction: OrderingDirection::HolderBeforeRequester,
                edge_id:   EdgeId::new(),
                reason:    "integrate after the holder".parse()?,
            },
            ConflictAuthorization::Defer {
                overlaps: overlaps.clone(),
                blocker:  counterpart,
                reason:   "decide the order later".parse()?,
            },
            ConflictAuthorization::Override {
                overlaps: overlaps.clone(),
                blocker:  counterpart,
                reason:   "accept the conflict".parse()?,
            },
        ];
        for answer in &answers {
            for revision in [&original_revision, &widened_revision] {
                assert!(answer.covers(counterpart, revision, &shared, PathCase::Sensitive));
                assert!(!answer.covers(counterpart, revision, &added, PathCase::Sensitive));
                assert!(!answer.covers(
                    ReservationId::new(),
                    revision,
                    &shared,
                    PathCase::Sensitive,
                ));
            }
        }
        let existing = ConflictAuthorization::ExistingAnswersCoverEveryOverlap { overlaps };
        assert!(existing.covers(
            counterpart,
            &original_revision,
            &shared,
            PathCase::Sensitive,
        ));
        assert!(!existing.covers(counterpart, &widened_revision, &shared, PathCase::Sensitive));
        Ok(())
    }
}
