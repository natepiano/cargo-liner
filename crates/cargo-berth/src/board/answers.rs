//! Recorded overlap answers, the durable authorization context each one preserves, and the
//! board's split of them by whether the reservation that answered is still live.

use std::collections::HashSet;
use std::slice;

use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;

use super::error::BoardError;
use super::rows;
use super::rows::DeferralConsequence;
use super::rows::WaitingAction;
use crate::answer::AuthorizedOverlap;
use crate::answer::AuthorizedOverlapSet;
use crate::answer::ConflictAuthorization;
use crate::answer::OverlapAuthorizationReason;
use crate::edge::DeferralOrigin;
use crate::edge::EdgeReadiness;
use crate::edge::EdgeReplayError;
use crate::edge::IntegrationConstraintProjection;
use crate::edge::IntegrationOrderingConstraint;
use crate::edge::OrderingReason;
use crate::ids::EdgeId;
use crate::ids::EventId;
use crate::ids::ReservationId;
use crate::ledger::JournalEvent;
use crate::ledger::JournalOperation;
use crate::ledger::OrderingDirection;
use crate::ledger::ReservationScope;
use crate::ledger::WidenCause;
use crate::reservation::EditBlockingStatus;

/// One durable answer to an overlap with another reservation's scopes.
///
/// A widen that overlaps no foreign reservation, or whose overlaps earlier answers already cover,
/// answers nothing: it is scope growth, kept in the journal and never listed here.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(tag = "answer", rename_all = "snake_case")]
pub(super) enum RecordedOverlapAnswer {
    Enrollment {
        reservation_id:        ReservationId,
        exact_approved_scopes: AuthorizedOverlapSet,
        acquisition:           AnswerAcquisition,
        consequence:           DeferralConsequence,
    },
    Sequence {
        reservation_id:        ReservationId,
        blocker:               ReservationId,
        direction:             OrderingDirection,
        exact_approved_scopes: AuthorizedOverlapSet,
        authorization_reason:  OverlapAuthorizationReason,
        acquisition:           AnswerAcquisition,
        consequence:           OrderingConsequence,
    },
    Defer {
        reservation_id:        ReservationId,
        blocker:               ReservationId,
        exact_approved_scopes: AuthorizedOverlapSet,
        authorization_reason:  OverlapAuthorizationReason,
        acquisition:           AnswerAcquisition,
        consequence:           DeferralConsequence,
    },
    Override {
        reservation_id:        ReservationId,
        blocker:               ReservationId,
        exact_approved_scopes: AuthorizedOverlapSet,
        authorization_reason:  OverlapAuthorizationReason,
        acquisition:           AnswerAcquisition,
        consequence:           OverrideConsequence,
    },
    OrderingCreatedFromDeferral {
        edge_id:               EdgeId,
        deferred:              ReservationId,
        blocker:               ReservationId,
        direction:             OrderingDirection,
        exact_approved_scopes: Vec<AuthorizedOverlap>,
        deferral_reasons:      Vec<OverlapAuthorizationReason>,
        ordering_reason:       OrderingReason,
        consequence:           OrderingConsequence,
    },
}

/// How many recorded overlap answers belong to reservations that are already released.
///
/// The board lists these answers only as a count; the journal keeps each one in full.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(transparent)]
pub(super) struct ReleasedOverlapAnswerCount(usize);

/// The overlap answers the board presents: live reservations' answers in full, and a count of
/// the answers that released reservations recorded.
pub(super) struct BoardOverlapAnswers {
    pub(super) live:     Vec<RecordedOverlapAnswer>,
    pub(super) released: ReleasedOverlapAnswerCount,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(tag = "origin", rename_all = "snake_case")]
pub(super) enum AnswerAcquisition {
    Claim,
    /// A `--defer` claim answered while the run held this reservation, without widening it.
    Answer,
    Enrollment,
    Widen {
        added_scopes:         Vec<ReservationScope>,
        cause:                WidenCause,
        edit_blocking_status: EditBlockingStatus,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub(super) enum OrderingConsequence {
    Holding { action: WaitingAction },
    Cancelled,
    Fulfilled,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum OverrideConsequence {
    EditingAuthorizedWithoutIntegrationOrder,
}

/// Recorded overlap answers split as they are read, so only a live answer computes its consequence.
struct AnswerSplit<'live> {
    live_reservations: &'live HashSet<ReservationId>,
    live:              Vec<RecordedOverlapAnswer>,
    released:          usize,
}

impl AnswerSplit<'_> {
    /// Keep one answer in full while its answering reservation is live, or count it once released.
    ///
    /// The answering reservation is the one whose scope acquisition the answer settled: the
    /// requester, or the deferred side of a sequenced deferral.
    fn record(
        &mut self,
        answering_reservation: ReservationId,
        answer: impl FnOnce() -> Result<RecordedOverlapAnswer, BoardError>,
    ) -> Result<(), BoardError> {
        if self.live_reservations.contains(&answering_reservation) {
            self.live.push(answer()?);
        } else {
            self.released += 1;
        }
        Ok(())
    }
}

/// One durable authorization answer's own inputs, apart from where the board appends it.
struct RecordedAuthorizationRow<'authorization> {
    reservation_id: ReservationId,
    authorization:  &'authorization ConflictAuthorization,
    acquisition:    AnswerAcquisition,
}

/// The overlap approvals one deferral accumulated before its ordering answer was recorded.
struct AccumulatedDeferralApprovals {
    exact_approved_scopes: Vec<AuthorizedOverlap>,
    deferral_reasons:      Vec<OverlapAuthorizationReason>,
}

/// The ordering edge `edge_id` a recorded answer or resolution names, or
/// [`BoardError::MissingOrderingEdge`] when replay holds no such edge.
fn recorded_ordering_edge(
    constraints: &IntegrationConstraintProjection,
    edge_id: EdgeId,
) -> Result<&IntegrationOrderingConstraint, BoardError> {
    constraints
        .ordering_constraints
        .iter()
        .find(|edge| edge.edge_id == edge_id)
        .ok_or(BoardError::MissingOrderingEdge(edge_id))
}

fn ordering_consequence(edge: &IntegrationOrderingConstraint) -> OrderingConsequence {
    match edge.readiness {
        EdgeReadiness::Holding { hold } => OrderingConsequence::Holding {
            action: rows::waiting_action(hold, &edge.ordering_target),
        },
        EdgeReadiness::Cancelled => OrderingConsequence::Cancelled,
        EdgeReadiness::Fulfilled => OrderingConsequence::Fulfilled,
    }
}

/// Split every recorded overlap answer by whether its answering reservation is still live.
pub(super) fn board_overlap_answers(
    events: &[JournalEvent],
    constraints: &IntegrationConstraintProjection,
    live_reservations: &HashSet<ReservationId>,
) -> Result<BoardOverlapAnswers, BoardError> {
    let resolved_pairs = resolved_defer_pairs(events);
    let mut answers = AnswerSplit {
        live_reservations,
        live: Vec::new(),
        released: 0,
    };
    for event in events {
        match &event.operation {
            JournalOperation::Claim {
                reservation_id,
                authorization,
                ..
            } => append_authorization_answer(
                &mut answers,
                RecordedAuthorizationRow {
                    reservation_id: *reservation_id,
                    authorization,
                    acquisition: claim_acquisition(authorization),
                },
                &resolved_pairs,
                constraints,
            )?,
            JournalOperation::Widen {
                reservation_id,
                added_scopes,
                cause,
                authorization,
                edit_blocking_status,
            } => append_authorization_answer(
                &mut answers,
                RecordedAuthorizationRow {
                    reservation_id: *reservation_id,
                    authorization,
                    acquisition: AnswerAcquisition::Widen {
                        added_scopes:         added_scopes.as_slice().to_vec(),
                        cause:                cause.clone(),
                        edit_blocking_status: *edit_blocking_status,
                    },
                },
                &resolved_pairs,
                constraints,
            )?,
            // An answer recorded without a reservation has no answering row to list.
            JournalOperation::Answer {
                reservation_id: Some(reservation_id),
                authorizations,
                ..
            } => authorizations.iter().try_for_each(|authorization| {
                append_authorization_answer(
                    &mut answers,
                    RecordedAuthorizationRow {
                        reservation_id: *reservation_id,
                        authorization,
                        acquisition: AnswerAcquisition::Answer,
                    },
                    &resolved_pairs,
                    constraints,
                )
            })?,
            JournalOperation::ResolveDefer {
                deferred_reservation_id,
                blocker_reservation_id,
                edge_id,
                direction,
                reason,
            } => {
                let AccumulatedDeferralApprovals {
                    exact_approved_scopes,
                    deferral_reasons,
                } = accumulated_deferral_approvals(
                    events,
                    constraints,
                    event.event_id(),
                    *deferred_reservation_id,
                    *blocker_reservation_id,
                );
                let edge = recorded_ordering_edge(constraints, *edge_id)?;
                answers.record(*deferred_reservation_id, || {
                    Ok(RecordedOverlapAnswer::OrderingCreatedFromDeferral {
                        edge_id: *edge_id,
                        deferred: *deferred_reservation_id,
                        blocker: *blocker_reservation_id,
                        direction: *direction,
                        exact_approved_scopes,
                        deferral_reasons,
                        ordering_reason: reason.clone(),
                        consequence: ordering_consequence(edge),
                    })
                })?;
            },
            _ => {},
        }
    }
    Ok(BoardOverlapAnswers {
        live:     answers.live,
        released: ReleasedOverlapAnswerCount(answers.released),
    })
}

const fn claim_acquisition(authorization: &ConflictAuthorization) -> AnswerAcquisition {
    match authorization {
        ConflictAuthorization::Enrollment { .. } => AnswerAcquisition::Enrollment,
        _ => AnswerAcquisition::Claim,
    }
}

fn resolved_defer_pairs(events: &[JournalEvent]) -> HashSet<(ReservationId, ReservationId)> {
    events
        .iter()
        .filter_map(|event| match &event.operation {
            JournalOperation::ResolveDefer {
                deferred_reservation_id,
                blocker_reservation_id,
                ..
            } => Some((*deferred_reservation_id, *blocker_reservation_id)),
            _ => None,
        })
        .collect()
}

fn accumulated_deferral_approvals(
    events: &[JournalEvent],
    constraints: &IntegrationConstraintProjection,
    resolution_event_id: EventId,
    deferred_reservation_id: ReservationId,
    blocker_reservation_id: ReservationId,
) -> AccumulatedDeferralApprovals {
    let mut exact_approved_scopes = Vec::new();
    let mut deferral_reasons = Vec::new();
    let is_pair = |deferred, blocker| {
        (deferred == deferred_reservation_id && blocker == blocker_reservation_id)
            || (deferred == blocker_reservation_id && blocker == deferred_reservation_id)
    };
    let mut enrollment_reasons = constraints
        .deferrals
        .iter()
        .filter(|deferral| {
            deferral.origin == DeferralOrigin::Enrollment
                && is_pair(deferral.deferred, deferral.blocker)
        })
        .map(|deferral| &deferral.reason);
    let prior_answers = events
        .iter()
        .take_while(|prior| prior.event_id() != resolution_event_id)
        .filter_map(|prior| reservation_answers(&prior.operation))
        .flat_map(|(requester, authorizations)| {
            authorizations
                .iter()
                .map(move |authorization| (requester, authorization))
        });
    for (requester, authorization) in prior_answers {
        if requester == deferred_reservation_id
            && let ConflictAuthorization::Defer {
                overlaps,
                blocker,
                reason,
            } = authorization
            && *blocker == blocker_reservation_id
        {
            exact_approved_scopes.extend(overlaps.as_slice().iter().cloned());
            deferral_reasons.push(reason.clone());
        }
        if let ConflictAuthorization::Enrollment { overlaps } = authorization {
            let mut approved = overlaps
                .as_slice()
                .iter()
                .filter(|overlap| is_pair(requester, overlap.reservation_id))
                .peekable();
            if approved.peek().is_some() {
                exact_approved_scopes.extend(approved.cloned());
                // Replay retains one deferral per enrollment pair in journal order.
                deferral_reasons.extend(enrollment_reasons.next().cloned());
            }
        }
    }
    AccumulatedDeferralApprovals {
        exact_approved_scopes,
        deferral_reasons,
    }
}

/// The answers one operation recorded, with the reservation that answered them.
fn reservation_answers(
    operation: &JournalOperation,
) -> Option<(ReservationId, &[ConflictAuthorization])> {
    match operation {
        JournalOperation::Claim {
            reservation_id,
            authorization,
            ..
        }
        | JournalOperation::Widen {
            reservation_id,
            authorization,
            ..
        } => Some((*reservation_id, slice::from_ref(authorization))),
        JournalOperation::Answer {
            reservation_id,
            authorizations,
            ..
        } => reservation_id.map(|reservation_id| (reservation_id, authorizations.as_slice())),
        _ => None,
    }
}

fn append_authorization_answer(
    answers: &mut AnswerSplit<'_>,
    row: RecordedAuthorizationRow<'_>,
    resolved_pairs: &HashSet<(ReservationId, ReservationId)>,
    constraints: &IntegrationConstraintProjection,
) -> Result<(), BoardError> {
    let RecordedAuthorizationRow {
        reservation_id,
        authorization,
        acquisition,
    } = row;
    match authorization {
        ConflictAuthorization::Enrollment { overlaps } => append_enrollment_answer(
            answers,
            reservation_id,
            overlaps,
            acquisition,
            resolved_pairs,
            constraints,
        )?,
        ConflictAuthorization::Sequence {
            overlaps,
            blocker,
            direction,
            edge_id,
            reason,
        } => {
            let edge = recorded_ordering_edge(constraints, *edge_id)?;
            answers.record(reservation_id, || {
                Ok(RecordedOverlapAnswer::Sequence {
                    reservation_id,
                    blocker: *blocker,
                    direction: *direction,
                    exact_approved_scopes: overlaps.clone(),
                    authorization_reason: reason.clone(),
                    acquisition,
                    consequence: ordering_consequence(edge),
                })
            })?;
        },
        // One row per holder, each with that holder's own binding, direction and edge, so the
        // board lists a several-holder answer exactly as it lists one `Sequence` answer per holder.
        ConflictAuthorization::SequencePerHolder {
            overlaps,
            holders,
            reason,
        } => {
            for holder in holders {
                let edge = recorded_ordering_edge(constraints, holder.edge_id)?;
                let exact_approved_scopes = overlaps
                    .of_holder(holder.blocker)
                    .map_err(|_| EdgeReplayError::MissingAuthorizedScopes(holder.blocker))?;
                answers.record(reservation_id, || {
                    Ok(RecordedOverlapAnswer::Sequence {
                        reservation_id,
                        blocker: holder.blocker,
                        direction: holder.direction,
                        exact_approved_scopes,
                        authorization_reason: reason.clone(),
                        acquisition: acquisition.clone(),
                        consequence: ordering_consequence(edge),
                    })
                })?;
            }
        },
        ConflictAuthorization::Defer {
            overlaps,
            blocker,
            reason,
        } if !resolved_pairs.contains(&(reservation_id, *blocker)) => {
            answers.record(reservation_id, || {
                let blocker_lifecycle = &constraints.reservation(*blocker)?.lifecycle;
                Ok(RecordedOverlapAnswer::Defer {
                    reservation_id,
                    blocker: *blocker,
                    exact_approved_scopes: overlaps.clone(),
                    authorization_reason: reason.clone(),
                    acquisition,
                    consequence: DeferralConsequence::with_live_deferred([blocker_lifecycle]),
                })
            })?;
        },
        ConflictAuthorization::Override {
            overlaps,
            blocker,
            reason,
        } => answers.record(reservation_id, || {
            Ok(RecordedOverlapAnswer::Override {
                reservation_id,
                blocker: *blocker,
                exact_approved_scopes: overlaps.clone(),
                authorization_reason: reason.clone(),
                acquisition,
                consequence: OverrideConsequence::EditingAuthorizedWithoutIntegrationOrder,
            })
        })?,
        // No foreign overlap, overlaps earlier answers already cover, and a deferral a later
        // sequence resolved each record no answer of their own.
        ConflictAuthorization::NoConflict
        | ConflictAuthorization::ExistingAnswersCoverEveryOverlap { .. }
        | ConflictAuthorization::Defer { .. } => {},
    }
    Ok(())
}

/// Record the pairs of one `ConflictAuthorization::Enrollment` that no `ResolveDefer` in
/// `resolved_pairs` has sequenced; an enrollment whose every pair is sequenced records nothing.
fn append_enrollment_answer(
    answers: &mut AnswerSplit<'_>,
    reservation_id: ReservationId,
    overlaps: &AuthorizedOverlapSet,
    acquisition: AnswerAcquisition,
    resolved_pairs: &HashSet<(ReservationId, ReservationId)>,
    constraints: &IntegrationConstraintProjection,
) -> Result<(), BoardError> {
    let unresolved = overlaps
        .as_slice()
        .iter()
        .filter(|overlap| {
            !resolved_pairs.contains(&(reservation_id, overlap.reservation_id))
                && !resolved_pairs.contains(&(overlap.reservation_id, reservation_id))
        })
        .cloned()
        .collect::<Vec<_>>();
    let Ok(exact_approved_scopes) = AuthorizedOverlapSet::try_from(unresolved) else {
        return Ok(());
    };
    answers.record(reservation_id, || {
        let counterparts = exact_approved_scopes
            .as_slice()
            .iter()
            .map(|overlap| {
                constraints
                    .reservation(overlap.reservation_id)
                    .map(|facts| &facts.lifecycle)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let consequence = DeferralConsequence::with_live_deferred(counterparts);
        Ok(RecordedOverlapAnswer::Enrollment {
            reservation_id,
            exact_approved_scopes,
            acquisition,
            consequence,
        })
    })
}

#[cfg(test)]
mod tests {
    use std::io;

    use super::AnswerAcquisition;
    use super::OrderingConsequence;
    use super::OverrideConsequence;
    use super::RecordedOverlapAnswer;
    use crate::answer::AuthorizedOverlap;
    use crate::answer::AuthorizedOverlapSet;
    use crate::answer::ConflictAuthorization;
    use crate::answer::OverlapScopeRevision;
    use crate::board::rows::BoardModel;
    use crate::board::rows::DeferralConsequence;
    use crate::board::rows::WaitingAction;
    use crate::board::test_support;
    use crate::board::test_support::BoardFixture;
    use crate::board::test_support::FixtureResult;
    use crate::board::test_support::OverlapAnswerFixture;
    use crate::ids::EdgeId;
    use crate::ids::ReservationId;
    use crate::ledger::JournalOperation;
    use crate::ledger::OrderingDirection;
    use crate::ledger::ReservationScope;
    use crate::ledger::ReservationScopeAdditionSet;
    use crate::ledger::ReservationScopeSet;
    use crate::ledger::ScopeKind;
    use crate::ledger::WidenCause;
    use crate::reservation::AbandonmentReason;
    use crate::reservation::EditBlockingStatus;
    use crate::reservation::ReleaseDisposition;

    /// Automatic drift widens onto files no other reservation holds, as a long-lived lane
    /// accumulates them.
    const WIDENS_WITHOUT_FOREIGN_OVERLAP: usize = 25;

    #[test]
    fn enrollment_answers_preserve_each_pending_pair_until_sequence() -> FixtureResult<()> {
        for direction in [
            OrderingDirection::RequesterBeforeHolder,
            OrderingDirection::HolderBeforeRequester,
        ] {
            let fixture = BoardFixture::new()?;
            let actor = fixture.main_actor();
            let first = fixture.claim(&actor, "shared.rs", ConflictAuthorization::NoConflict)?;
            let second = fixture.claim(&actor, "shared.rs", ConflictAuthorization::NoConflict)?;
            let overlaps = enrollment_overlaps([first.reservation_id, second.reservation_id])?;
            let enrolled = fixture.claim(
                &actor,
                "shared.rs",
                ConflictAuthorization::Enrollment {
                    overlaps: overlaps.clone(),
                },
            )?;
            let model = fixture.model()?;
            let answer = recorded_answer(&model, enrolled.reservation_id)?;
            assert_eq!(
                answer,
                &RecordedOverlapAnswer::Enrollment {
                    reservation_id:        enrolled.reservation_id,
                    exact_approved_scopes: overlaps,
                    acquisition:           AnswerAcquisition::Enrollment,
                    consequence:           DeferralConsequence::Both,
                }
            );
            let wire = serde_json::to_value(answer)?;
            assert_eq!(wire["answer"], "enrollment");
            assert_eq!(wire["acquisition"]["origin"], "enrollment");
            assert!(wire.get("authorization_reason").is_none());

            for blocker in [first.reservation_id, second.reservation_id] {
                fixture.append_as(
                    &actor,
                    JournalOperation::ResolveDefer {
                        deferred_reservation_id: enrolled.reservation_id,
                        blocker_reservation_id: blocker,
                        edge_id: EdgeId::new(),
                        direction,
                        reason: "sequence enrolled work".parse()?,
                    },
                )?;
                let model = fixture.model()?;
                let answers = &model.live_overlap_answers.entries;
                assert_resolved_enrollment_answer(&model, blocker)?;
                if blocker == first.reservation_id {
                    let RecordedOverlapAnswer::Enrollment {
                        exact_approved_scopes,
                        ..
                    } = recorded_answer(&model, enrolled.reservation_id)?
                    else {
                        return Err(io::Error::other(
                            "remaining pair should retain enrollment label",
                        )
                        .into());
                    };
                    assert_authorized_overlap(exact_approved_scopes, second.reservation_id);
                } else {
                    assert!(
                        !answers.iter().any(|answer| matches!(
                            answer,
                            RecordedOverlapAnswer::Enrollment { .. }
                        ))
                    );
                }
            }
        }
        Ok(())
    }

    fn enrollment_overlaps(
        reservation_ids: [ReservationId; 2],
    ) -> FixtureResult<AuthorizedOverlapSet> {
        let scopes = ReservationScopeSet::try_from(vec![ReservationScope {
            path: "shared.rs".parse()?,
            kind: ScopeKind::File,
        }])?;
        Ok(AuthorizedOverlapSet::try_from(
            reservation_ids
                .map(|reservation_id| AuthorizedOverlap {
                    reservation_id,
                    scope_revision: OverlapScopeRevision::from(&scopes),
                    scopes: scopes.clone().into(),
                })
                .to_vec(),
        )?)
    }

    fn assert_resolved_enrollment_answer(
        model: &BoardModel,
        blocker: ReservationId,
    ) -> FixtureResult<()> {
        let resolved = model
            .live_overlap_answers
            .entries
            .iter()
            .find(|answer| {
                matches!(answer,
                    RecordedOverlapAnswer::OrderingCreatedFromDeferral { blocker: candidate, .. }
                        if *candidate == blocker
                )
            })
            .ok_or_else(|| io::Error::other("enrollment resolution should have an audit row"))?;
        let RecordedOverlapAnswer::OrderingCreatedFromDeferral {
            exact_approved_scopes,
            deferral_reasons,
            ..
        } = resolved
        else {
            return Err(io::Error::other("expected resolved enrollment audit row").into());
        };
        assert_eq!(exact_approved_scopes.len(), 1);
        assert_eq!(exact_approved_scopes[0].reservation_id, blocker);
        assert_eq!(deferral_reasons.len(), 1);
        assert!(deferral_reasons[0].to_string().contains("Enrollment"));
        Ok(())
    }

    #[test]
    fn overlap_answers_preserve_typed_authorization_variants() -> FixtureResult<()> {
        let sequence = test_support::answered_board(OverlapAnswerFixture::Sequence)?;
        let sequence_answer = recorded_answer(&sequence.model, sequence.requester_id)?;
        let RecordedOverlapAnswer::Sequence {
            reservation_id,
            blocker,
            direction,
            exact_approved_scopes,
            authorization_reason,
            acquisition,
            consequence,
        } = sequence_answer
        else {
            return Err(
                io::Error::other("sequence fixture should produce a sequence audit row").into(),
            );
        };
        assert_eq!(*reservation_id, sequence.requester_id);
        assert_eq!(*blocker, sequence.blocker_id);
        assert_eq!(*direction, OrderingDirection::HolderBeforeRequester);
        assert_authorized_overlap(exact_approved_scopes, sequence.blocker_id);
        assert_eq!(
            authorization_reason.to_string(),
            "holder must integrate first"
        );
        assert!(matches!(acquisition, AnswerAcquisition::Claim));
        assert!(matches!(
            consequence,
            OrderingConsequence::Holding {
                action: WaitingAction::PredecessorCheckpoint { .. },
            }
        ));

        let defer = test_support::answered_board(OverlapAnswerFixture::Defer)?;
        let defer_answer = recorded_answer(&defer.model, defer.requester_id)?;
        let RecordedOverlapAnswer::Defer {
            reservation_id,
            blocker,
            exact_approved_scopes,
            authorization_reason,
            acquisition,
            consequence,
        } = defer_answer
        else {
            return Err(io::Error::other("defer fixture should produce a defer audit row").into());
        };
        assert_eq!(*reservation_id, defer.requester_id);
        assert_eq!(*blocker, defer.blocker_id);
        assert_authorized_overlap(exact_approved_scopes, defer.blocker_id);
        assert_eq!(
            authorization_reason.to_string(),
            "integration order is deferred"
        );
        assert!(matches!(acquisition, AnswerAcquisition::Claim));
        assert_eq!(*consequence, DeferralConsequence::Both);

        let override_fixture = test_support::answered_board(OverlapAnswerFixture::Override)?;
        let override_answer =
            recorded_answer(&override_fixture.model, override_fixture.requester_id)?;
        let RecordedOverlapAnswer::Override {
            reservation_id,
            blocker,
            exact_approved_scopes,
            authorization_reason,
            acquisition,
            consequence,
        } = override_answer
        else {
            return Err(
                io::Error::other("override fixture should produce an override audit row").into(),
            );
        };
        assert_eq!(*reservation_id, override_fixture.requester_id);
        assert_eq!(*blocker, override_fixture.blocker_id);
        assert_authorized_overlap(exact_approved_scopes, override_fixture.blocker_id);
        assert_eq!(
            authorization_reason.to_string(),
            "overlapping edits are accepted"
        );
        assert!(matches!(acquisition, AnswerAcquisition::Claim));
        assert_eq!(
            *consequence,
            OverrideConsequence::EditingAuthorizedWithoutIntegrationOrder
        );
        Ok(())
    }

    #[test]
    fn scope_growth_without_a_new_answer_leaves_only_the_defer_listed() -> FixtureResult<()> {
        let fixture = BoardFixture::new()?;
        let actor = fixture.main_actor();
        let blocker = fixture.claim(&actor, "shared.rs", ConflictAuthorization::NoConflict)?;
        let defer = test_support::conflict_authorization(OverlapAnswerFixture::Defer, &blocker)?;
        let ConflictAuthorization::Defer { overlaps, .. } = &defer else {
            return Err(
                io::Error::other("defer fixture should build a defer authorization").into(),
            );
        };
        let covered = ConflictAuthorization::ExistingAnswersCoverEveryOverlap {
            overlaps: overlaps.clone(),
        };
        let requester = fixture.claim(&actor, "shared.rs", defer)?;
        for index in 0..WIDENS_WITHOUT_FOREIGN_OVERLAP {
            fixture.append_as(
                &actor,
                drift_widen(
                    requester.reservation_id,
                    &format!("grown_{index}.rs"),
                    ConflictAuthorization::NoConflict,
                )?,
            )?;
        }
        fixture.append_as(
            &actor,
            drift_widen(requester.reservation_id, "covered.rs", covered)?,
        )?;

        let model = fixture.model()?;
        assert!(matches!(
            model.live_overlap_answers.entries.as_slice(),
            [RecordedOverlapAnswer::Defer { reservation_id, .. }]
                if *reservation_id == requester.reservation_id
        ));
        let wire = serde_json::to_value(&model)?;
        let listed = wire["live_overlap_answers"]["entries"]
            .as_array()
            .ok_or_else(|| io::Error::other("live overlap answers should be an array"))?;
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0]["answer"], "defer");
        assert_eq!(wire["released_overlap_answer_count"], 0);
        Ok(())
    }

    #[test]
    fn a_released_reservations_answers_leave_the_live_listing() -> FixtureResult<()> {
        let fixture = BoardFixture::new()?;
        let actor = fixture.main_actor();
        let blocker = fixture.claim(&actor, "shared.rs", ConflictAuthorization::NoConflict)?;
        let released = fixture.claim(
            &actor,
            "shared.rs",
            test_support::conflict_authorization(OverlapAnswerFixture::Override, &blocker)?,
        )?;
        let live = fixture.claim(
            &actor,
            "shared.rs",
            test_support::conflict_authorization(OverlapAnswerFixture::Override, &blocker)?,
        )?;
        fixture.release(
            &actor,
            released.reservation_id,
            ReleaseDisposition::Abandoned(
                "the overlapping work was dropped".parse::<AbandonmentReason>()?,
            ),
        )?;

        let model = fixture.model()?;
        let listed = model
            .live_overlap_answers
            .entries
            .iter()
            .map(|answer| match answer {
                RecordedOverlapAnswer::Override { reservation_id, .. } => Some(*reservation_id),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(listed, [Some(live.reservation_id)]);
        let wire = serde_json::to_value(&model)?;
        assert_eq!(wire["released_overlap_answer_count"], 1);
        Ok(())
    }

    fn drift_widen(
        reservation_id: ReservationId,
        path: &str,
        authorization: ConflictAuthorization,
    ) -> FixtureResult<JournalOperation> {
        Ok(JournalOperation::Widen {
            reservation_id,
            added_scopes: ReservationScopeAdditionSet::try_from(vec![ReservationScope {
                path: path.parse()?,
                kind: ScopeKind::File,
            }])?,
            cause: WidenCause::Drift,
            authorization,
            edit_blocking_status: EditBlockingStatus::Blocking,
        })
    }

    fn recorded_answer(
        model: &BoardModel,
        reservation_id: ReservationId,
    ) -> FixtureResult<&RecordedOverlapAnswer> {
        model
            .live_overlap_answers
            .entries
            .iter()
            .find(|answer| match answer {
                RecordedOverlapAnswer::Enrollment {
                    reservation_id: candidate,
                    ..
                }
                | RecordedOverlapAnswer::Sequence {
                    reservation_id: candidate,
                    ..
                }
                | RecordedOverlapAnswer::Defer {
                    reservation_id: candidate,
                    ..
                }
                | RecordedOverlapAnswer::Override {
                    reservation_id: candidate,
                    ..
                } => *candidate == reservation_id,
                RecordedOverlapAnswer::OrderingCreatedFromDeferral { .. } => false,
            })
            .ok_or_else(|| io::Error::other("recorded answer should exist").into())
    }

    fn assert_authorized_overlap(overlaps: &AuthorizedOverlapSet, blocker_id: ReservationId) {
        assert_eq!(overlaps.as_slice().len(), 1);
        let overlap = &overlaps.as_slice()[0];
        assert_eq!(overlap.reservation_id, blocker_id);
        assert_eq!(overlap.scopes.as_slice().len(), 1);
        assert_eq!(overlap.scopes.as_slice()[0].path.to_string(), "shared.rs");
        assert_eq!(overlap.scopes.as_slice()[0].kind, ScopeKind::File);
    }
}
