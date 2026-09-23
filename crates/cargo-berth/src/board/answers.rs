//! Recorded overlap answers and the durable authorization context each one preserves.

use std::collections::HashSet;

use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;

use super::error::BoardError;
use super::rows;
use super::rows::SymmetricDeferralConsequence;
use super::rows::WaitingAction;
use crate::answer::AuthorizedOverlap;
use crate::answer::AuthorizedOverlapSet;
use crate::answer::ConflictAuthorization;
use crate::answer::OverlapAuthorizationReason;
use crate::edge::DeferralOrigin;
use crate::edge::EdgeReadiness;
use crate::edge::IntegrationConstraintProjection;
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

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(tag = "answer", rename_all = "snake_case")]
pub(super) enum RecordedAnswer {
    Enrollment {
        reservation_id:        ReservationId,
        exact_approved_scopes: AuthorizedOverlapSet,
        acquisition:           AnswerAcquisition,
        consequence:           SymmetricDeferralConsequence,
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
        consequence:           SymmetricDeferralConsequence,
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
    ExistingAnswersCoverEveryOverlap {
        reservation_id:          ReservationId,
        exact_existing_bindings: AuthorizedOverlapSet,
        added_scopes:            Vec<ReservationScope>,
        cause:                   WidenCause,
        edit_blocking_status:    EditBlockingStatus,
        consequence:             RevalidationConsequence,
    },
    WidenWithoutForeignOverlap {
        reservation_id:       ReservationId,
        added_scopes:         Vec<ReservationScope>,
        cause:                WidenCause,
        edit_blocking_status: EditBlockingStatus,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(tag = "origin", rename_all = "snake_case")]
pub(super) enum AnswerAcquisition {
    Claim,
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

#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum RevalidationConsequence {
    ExistingAnswersStillCoverWidenedScopesNoNewEdge,
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

fn ordering_consequence(readiness: EdgeReadiness) -> OrderingConsequence {
    match readiness {
        EdgeReadiness::Holding { hold } => OrderingConsequence::Holding {
            action: rows::waiting_action(hold),
        },
        EdgeReadiness::Cancelled => OrderingConsequence::Cancelled,
        EdgeReadiness::Fulfilled => OrderingConsequence::Fulfilled,
    }
}

pub(super) fn recorded_answers(
    events: &[JournalEvent],
    constraints: &IntegrationConstraintProjection,
) -> Result<Vec<RecordedAnswer>, BoardError> {
    let resolved_pairs = resolved_defer_pairs(events);
    let mut answers = Vec::new();
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
            } => {
                let acquisition = AnswerAcquisition::Widen {
                    added_scopes:         added_scopes.as_slice().to_vec(),
                    cause:                cause.clone(),
                    edit_blocking_status: *edit_blocking_status,
                };
                match authorization {
                    ConflictAuthorization::ExistingAnswersCoverEveryOverlap { overlaps } => {
                        answers.push(RecordedAnswer::ExistingAnswersCoverEveryOverlap {
                            reservation_id: *reservation_id,
                            exact_existing_bindings: overlaps.clone(),
                            added_scopes: added_scopes.as_slice().to_vec(),
                            cause: cause.clone(),
                            edit_blocking_status: *edit_blocking_status,
                            consequence: RevalidationConsequence::ExistingAnswersStillCoverWidenedScopesNoNewEdge,
                        });
                    },
                    ConflictAuthorization::NoConflict => {
                        answers.push(RecordedAnswer::WidenWithoutForeignOverlap {
                            reservation_id:       *reservation_id,
                            added_scopes:         added_scopes.as_slice().to_vec(),
                            cause:                cause.clone(),
                            edit_blocking_status: *edit_blocking_status,
                        });
                    },
                    _ => append_authorization_answer(
                        &mut answers,
                        RecordedAuthorizationRow {
                            reservation_id: *reservation_id,
                            authorization,
                            acquisition,
                        },
                        &resolved_pairs,
                        constraints,
                    )?,
                }
            },
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
                let edge = constraints
                    .ordering_constraints
                    .iter()
                    .find(|edge| edge.edge_id == *edge_id)
                    .ok_or(BoardError::MissingOrderingEdge(*edge_id))?;
                answers.push(RecordedAnswer::OrderingCreatedFromDeferral {
                    edge_id: *edge_id,
                    deferred: *deferred_reservation_id,
                    blocker: *blocker_reservation_id,
                    direction: *direction,
                    exact_approved_scopes,
                    deferral_reasons,
                    ordering_reason: reason.clone(),
                    consequence: ordering_consequence(edge.readiness),
                });
            },
            _ => {},
        }
    }
    Ok(answers)
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
    for prior in events
        .iter()
        .take_while(|prior| prior.event_id() != resolution_event_id)
    {
        let (requester, authorization) = match &prior.operation {
            JournalOperation::Claim {
                reservation_id,
                authorization,
                ..
            }
            | JournalOperation::Widen {
                reservation_id,
                authorization,
                ..
            } => (*reservation_id, authorization),
            _ => continue,
        };
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

fn append_authorization_answer(
    answers: &mut Vec<RecordedAnswer>,
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
        ConflictAuthorization::Enrollment { overlaps } => {
            let unresolved = overlaps
                .as_slice()
                .iter()
                .filter(|overlap| {
                    !resolved_pairs.contains(&(reservation_id, overlap.reservation_id))
                        && !resolved_pairs.contains(&(overlap.reservation_id, reservation_id))
                })
                .cloned()
                .collect::<Vec<_>>();
            if let Ok(exact_approved_scopes) = AuthorizedOverlapSet::try_from(unresolved) {
                answers.push(RecordedAnswer::Enrollment {
                    reservation_id,
                    exact_approved_scopes,
                    acquisition,
                    consequence: SymmetricDeferralConsequence::BothIntegrationsHeldUntilSequence,
                });
            }
        },
        ConflictAuthorization::Sequence {
            overlaps,
            blocker,
            direction,
            edge_id,
            reason,
        } => {
            let edge = constraints
                .ordering_constraints
                .iter()
                .find(|edge| edge.edge_id == *edge_id)
                .ok_or(BoardError::MissingOrderingEdge(*edge_id))?;
            answers.push(RecordedAnswer::Sequence {
                reservation_id,
                blocker: *blocker,
                direction: *direction,
                exact_approved_scopes: overlaps.clone(),
                authorization_reason: reason.clone(),
                acquisition,
                consequence: ordering_consequence(edge.readiness),
            });
        },
        ConflictAuthorization::Defer {
            overlaps,
            blocker,
            reason,
        } if !resolved_pairs.contains(&(reservation_id, *blocker)) => {
            answers.push(RecordedAnswer::Defer {
                reservation_id,
                blocker: *blocker,
                exact_approved_scopes: overlaps.clone(),
                authorization_reason: reason.clone(),
                acquisition,
                consequence: SymmetricDeferralConsequence::BothIntegrationsHeldUntilSequence,
            });
        },
        ConflictAuthorization::Override {
            overlaps,
            blocker,
            reason,
        } => answers.push(RecordedAnswer::Override {
            reservation_id,
            blocker: *blocker,
            exact_approved_scopes: overlaps.clone(),
            authorization_reason: reason.clone(),
            acquisition,
            consequence: OverrideConsequence::EditingAuthorizedWithoutIntegrationOrder,
        }),
        ConflictAuthorization::NoConflict
        | ConflictAuthorization::ExistingAnswersCoverEveryOverlap { .. }
        | ConflictAuthorization::Defer { .. } => {},
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io;

    use super::AnswerAcquisition;
    use super::OrderingConsequence;
    use super::OverrideConsequence;
    use super::RecordedAnswer;
    use crate::answer::AuthorizedOverlap;
    use crate::answer::AuthorizedOverlapSet;
    use crate::answer::ConflictAuthorization;
    use crate::answer::OverlapScopeRevision;
    use crate::board::rows::BoardModel;
    use crate::board::rows::SymmetricDeferralConsequence;
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
    use crate::ledger::ReservationScopeSet;
    use crate::ledger::ScopeKind;

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
                &RecordedAnswer::Enrollment {
                    reservation_id:        enrolled.reservation_id,
                    exact_approved_scopes: overlaps,
                    acquisition:           AnswerAcquisition::Enrollment,
                    consequence:
                        SymmetricDeferralConsequence::BothIntegrationsHeldUntilSequence,
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
                let answers = &model.recorded_overlap_answers.entries;
                assert_resolved_enrollment_answer(&model, blocker)?;
                if blocker == first.reservation_id {
                    let RecordedAnswer::Enrollment {
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
                        !answers
                            .iter()
                            .any(|answer| matches!(answer, RecordedAnswer::Enrollment { .. }))
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
            .recorded_overlap_answers
            .entries
            .iter()
            .find(|answer| {
                matches!(answer,
                    RecordedAnswer::OrderingCreatedFromDeferral { blocker: candidate, .. }
                        if *candidate == blocker
                )
            })
            .ok_or_else(|| io::Error::other("enrollment resolution should have an audit row"))?;
        let RecordedAnswer::OrderingCreatedFromDeferral {
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
        let RecordedAnswer::Sequence {
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
        let RecordedAnswer::Defer {
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
        assert_eq!(
            *consequence,
            SymmetricDeferralConsequence::BothIntegrationsHeldUntilSequence
        );

        let override_fixture = test_support::answered_board(OverlapAnswerFixture::Override)?;
        let override_answer =
            recorded_answer(&override_fixture.model, override_fixture.requester_id)?;
        let RecordedAnswer::Override {
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

    fn recorded_answer(
        model: &BoardModel,
        reservation_id: ReservationId,
    ) -> FixtureResult<&RecordedAnswer> {
        model
            .recorded_overlap_answers
            .entries
            .iter()
            .find(|answer| match answer {
                RecordedAnswer::Enrollment {
                    reservation_id: candidate,
                    ..
                }
                | RecordedAnswer::Sequence {
                    reservation_id: candidate,
                    ..
                }
                | RecordedAnswer::Defer {
                    reservation_id: candidate,
                    ..
                }
                | RecordedAnswer::Override {
                    reservation_id: candidate,
                    ..
                }
                | RecordedAnswer::ExistingAnswersCoverEveryOverlap {
                    reservation_id: candidate,
                    ..
                }
                | RecordedAnswer::WidenWithoutForeignOverlap {
                    reservation_id: candidate,
                    ..
                } => *candidate == reservation_id,
                RecordedAnswer::OrderingCreatedFromDeferral { .. } => false,
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
