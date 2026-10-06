//! Durable authorization recorded with a claim or widen operation.

use schemars::JsonSchema;
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

/// One holder a [`ConflictAuthorization::SequencePerHolder`] answer named, the order its own
/// flag chose, and the ordering edge born against it.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct SequencedHolder {
    /// The holder named as the other endpoint of this ordering edge.
    pub(crate) blocker:   ReservationId,
    /// Which endpoint of this ordering edge must integrate first.
    pub(crate) direction: OrderingDirection,
    /// The edge born with this acquisition against `blocker`.
    pub(crate) edge_id:   EdgeId,
}

/// One live ordering edge between a holder a sequence answer named and a nonterminal reservation
/// of the claimant's own coordination run and worktree, pointing the opposite way to that answer.
///
/// A claim without `--replace` is refused with these in its payload; a claim with it records them
/// as `replaced`, and replay retires each `edge_id` before adding the claim's own edges.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[schemars(rename = "contradicted_ordering")]
pub(crate) struct ContradictedOrdering {
    /// The claimant's earlier reservation at the other endpoint of the edge.
    pub(crate) reservation_id: ReservationId,
    /// The live edge the new answer contradicts.
    pub(crate) edge_id:        EdgeId,
    /// The holder the new answer named, at one endpoint of the edge.
    pub(crate) holder:         ReservationId,
}

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
    ///
    /// A sequence answer to one holder records this form; an answer naming several holders
    /// records [`Self::SequencePerHolder`].
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
        /// The live edges `--replace` retired before `edge_id` was added; empty without it.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        replaced:  Vec<ContradictedOrdering>,
    },
    /// One ordering edge per holder, each in its own direction, authorizes the shared scopes of
    /// every observed holder.
    ///
    /// Recorded only when the answer named several holders, one [`SequencedHolder`] each; an
    /// answer to one holder keeps the [`Self::Sequence`] form.
    SequencePerHolder {
        /// The exact holder bindings observed under the ledger lock, one per holder.
        overlaps: AuthorizedOverlapSet,
        /// Each named holder, its direction, and the edge born against it.
        holders:  Vec<SequencedHolder>,
        /// The caller's reason for selecting an order.
        reason:   OverlapAuthorizationReason,
        /// The live edges `--replace` retired before the holders' edges were added, each naming
        /// its holder; empty without it.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        replaced: Vec<ContradictedOrdering>,
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
    /// Record an answer whose named holders are exactly the conflicts observed under the lock.
    ///
    /// `overlaps` carries one binding per holder, and `answer` names exactly those holders, once
    /// each: the claim's `validate_authorization` refuses any other answer. A sequence answer
    /// gives each holder an ordering edge of its own, in the direction its flag chose: one holder
    /// records [`Self::Sequence`], several record [`Self::SequencePerHolder`]; either carries
    /// `replaced`, the live edges `--replace` retires. An override names one holder, so
    /// `overlaps` binds only it, and contradicts no order, so `replaced` is empty for it.
    pub(crate) fn answered(
        answer: &PermissiveOverlapAnswer,
        overlaps: AuthorizedOverlapSet,
        reason: OverlapAuthorizationReason,
        replaced: Vec<ContradictedOrdering>,
    ) -> Self {
        match answer {
            PermissiveOverlapAnswer::Sequence { blockers, .. } => {
                let holders = blockers
                    .iter()
                    .map(|sequenced_blocker| SequencedHolder {
                        blocker:   sequenced_blocker.blocker,
                        direction: sequenced_blocker.direction,
                        edge_id:   EdgeId::new(),
                    })
                    .collect::<Vec<_>>();
                match holders.as_slice() {
                    [holder] => Self::Sequence {
                        overlaps,
                        blocker: holder.blocker,
                        direction: holder.direction,
                        edge_id: holder.edge_id,
                        reason,
                        replaced,
                    },
                    _ => Self::SequencePerHolder {
                        overlaps,
                        holders,
                        reason,
                        replaced,
                    },
                }
            },
            PermissiveOverlapAnswer::Override { blocker } => Self::Override {
                overlaps,
                blocker: *blocker,
                reason,
            },
        }
    }

    /// The live edges this answer's `--replace` retired; empty for every other answer.
    pub(crate) fn replaced(&self) -> &[ContradictedOrdering] {
        match self {
            Self::Sequence { replaced, .. } | Self::SequencePerHolder { replaced, .. } => replaced,
            Self::NoConflict
            | Self::Enrollment { .. }
            | Self::Defer { .. }
            | Self::Override { .. }
            | Self::ExistingAnswersCoverEveryOverlap { .. } => &[],
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
            | Self::SequencePerHolder { overlaps, .. }
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
    use std::collections::HashSet;
    use std::error::Error;

    use super::ConflictAuthorization;
    use super::ContradictedOrdering;
    use super::SequencedHolder;
    use crate::answer::AuthorizedOverlap;
    use crate::answer::AuthorizedOverlapSet;
    use crate::answer::OrderingReplacement;
    use crate::answer::OverlapScopeRevision;
    use crate::answer::PermissiveOverlapAnswer;
    use crate::answer::SequencedBlocker;
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
                replaced:  Vec::new(),
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

    /// A `SequencePerHolder` answer covers each holder's own bound scope, and neither a scope
    /// bound only for the other holder nor any scope of a reservation it never named.
    #[test]
    fn a_sequence_per_holder_covers_each_holders_own_scope() -> Result<(), Box<dyn Error>> {
        let first_holder = ReservationId::new();
        let second_holder = ReservationId::new();
        let shared = ReservationScope {
            path: "shared.rs".parse()?,
            kind: ScopeKind::File,
        };
        let other = ReservationScope {
            path: "other.rs".parse()?,
            kind: ScopeKind::File,
        };
        let first_overlap = authorized_overlap(first_holder, &shared)?;
        let second_overlap = authorized_overlap(second_holder, &other)?;
        let first_revision = first_overlap.scope_revision.clone();
        let second_revision = second_overlap.scope_revision.clone();
        let answer = ConflictAuthorization::SequencePerHolder {
            overlaps: AuthorizedOverlapSet::try_from(vec![first_overlap, second_overlap])?,
            holders:  vec![
                SequencedHolder {
                    blocker:   first_holder,
                    direction: OrderingDirection::HolderBeforeRequester,
                    edge_id:   EdgeId::new(),
                },
                SequencedHolder {
                    blocker:   second_holder,
                    direction: OrderingDirection::RequesterBeforeHolder,
                    edge_id:   EdgeId::new(),
                },
            ],
            reason:   "integrate after the first holder and before the second".parse()?,
            replaced: Vec::new(),
        };

        assert!(answer.covers(first_holder, &first_revision, &shared, PathCase::Sensitive));
        assert!(answer.covers(second_holder, &second_revision, &other, PathCase::Sensitive));
        assert!(!answer.covers(first_holder, &first_revision, &other, PathCase::Sensitive));
        assert!(!answer.covers(
            second_holder,
            &second_revision,
            &shared,
            PathCase::Sensitive
        ));
        assert!(!answer.covers(
            ReservationId::new(),
            &first_revision,
            &shared,
            PathCase::Sensitive,
        ));
        Ok(())
    }

    /// [`ConflictAuthorization::answered`] keeps the `Sequence` form for one holder binding and
    /// records `SequencePerHolder`, one edge per holder in that holder's own direction, for two.
    #[test]
    fn a_sequence_answer_records_one_edge_per_holder() -> Result<(), Box<dyn Error>> {
        let first_holder = ReservationId::new();
        let second_holder = ReservationId::new();
        let shared = ReservationScope {
            path: "shared.rs".parse()?,
            kind: ScopeKind::File,
        };
        let first_overlap = authorized_overlap(first_holder, &shared)?;
        let second_overlap = authorized_overlap(second_holder, &shared)?;
        let after_first = SequencedBlocker {
            blocker:   first_holder,
            direction: OrderingDirection::HolderBeforeRequester,
        };
        let before_second = SequencedBlocker {
            blocker:   second_holder,
            direction: OrderingDirection::RequesterBeforeHolder,
        };

        let one_holder = ConflictAuthorization::answered(
            &PermissiveOverlapAnswer::Sequence {
                blockers:    vec![after_first],
                replacement: OrderingReplacement::Keep,
            },
            AuthorizedOverlapSet::from(first_overlap.clone()),
            "integrate after the holder".parse()?,
            Vec::new(),
        );
        assert!(matches!(
            one_holder,
            ConflictAuthorization::Sequence { blocker, direction, .. }
                if blocker == first_holder && direction == after_first.direction
        ));

        let per_holder = ConflictAuthorization::answered(
            &PermissiveOverlapAnswer::Sequence {
                blockers:    vec![after_first, before_second],
                replacement: OrderingReplacement::Keep,
            },
            AuthorizedOverlapSet::try_from(vec![first_overlap, second_overlap])?,
            "integrate after the first holder and before the second".parse()?,
            Vec::new(),
        );
        let ConflictAuthorization::SequencePerHolder { holders, .. } = per_holder else {
            return Err("two holder bindings should record SequencePerHolder".into());
        };
        assert_eq!(
            holders
                .iter()
                .map(|holder| SequencedBlocker {
                    blocker:   holder.blocker,
                    direction: holder.direction,
                })
                .collect::<Vec<_>>(),
            vec![after_first, before_second]
        );
        let edge_ids = holders
            .iter()
            .map(|holder| holder.edge_id)
            .collect::<HashSet<_>>();
        assert_eq!(edge_ids.len(), holders.len());
        Ok(())
    }

    /// A record without `--replace` keeps its earlier wire form, so journals written before
    /// `replaced` existed replay unchanged, and a record with it round-trips the retired edges.
    #[test]
    fn replaced_edges_are_additive_on_the_wire() -> Result<(), Box<dyn Error>> {
        let holder = ReservationId::new();
        let shared = ReservationScope {
            path: "shared.rs".parse()?,
            kind: ScopeKind::File,
        };
        let overlaps = AuthorizedOverlapSet::from(authorized_overlap(holder, &shared)?);
        let answer = |replaced| -> Result<ConflictAuthorization, Box<dyn Error>> {
            Ok(ConflictAuthorization::answered(
                &PermissiveOverlapAnswer::Sequence {
                    blockers:    vec![SequencedBlocker {
                        blocker:   holder,
                        direction: OrderingDirection::HolderBeforeRequester,
                    }],
                    replacement: OrderingReplacement::Replace,
                },
                overlaps.clone(),
                "integrate after the holder".parse()?,
                replaced,
            ))
        };

        let kept = answer(Vec::new())?;
        let kept_wire = serde_json::to_value(&kept)?;
        assert!(kept_wire.get("replaced").is_none(), "{kept_wire}");
        assert_eq!(
            serde_json::from_value::<ConflictAuthorization>(kept_wire)?,
            kept
        );

        let replaced = answer(vec![ContradictedOrdering {
            reservation_id: ReservationId::new(),
            edge_id: EdgeId::new(),
            holder,
        }])?;
        let replaced_wire = serde_json::to_value(&replaced)?;
        assert!(replaced_wire.get("replaced").is_some(), "{replaced_wire}");
        assert_eq!(
            serde_json::from_value::<ConflictAuthorization>(replaced_wire)?,
            replaced
        );
        Ok(())
    }

    fn authorized_overlap(
        reservation_id: ReservationId,
        scope: &ReservationScope,
    ) -> Result<AuthorizedOverlap, Box<dyn Error>> {
        let scopes = ReservationScopeSet::try_from(vec![scope.clone()])?;
        Ok(AuthorizedOverlap {
            reservation_id,
            scope_revision: OverlapScopeRevision::from(&scopes),
            scopes: scopes.into(),
        })
    }
}
