//! The coordination records of a checkpoint, split by what the ordering graph reads.

use serde::Deserialize;
use serde::Serialize;

use super::CanonicalWorktreeRoot;
use super::ClaimHeadSnapshot;
use super::ClaimSource;
use super::JournalActor;
use super::JournalEvent;
use super::JournalMutationIdentityInputs;
use super::JournalOperation;
use super::ProtectedPhaseStartHead;
use super::ReservationPurpose;
use super::ReservationScopeSet;
use super::ResolvedSessionAtMutation;
use super::TrunkObservationAtClaim;
use super::WorktreeAdministrativeLocator;
use crate::answer::ConflictAuthorization;
use crate::coordination_identity::CoordinationIdentityProvenance;
use crate::edge::OrderingReplayRecord;
use crate::ids::EventId;
use crate::ids::ProjectionGeneration;
use crate::ids::RecordedAt;
use crate::ids::ReservationId;
use crate::ids::SchemaVersion;
use crate::ledger::ClaimTarget;

/// The claim fields the ordering graph consumes, or a complete ordering operation.
#[derive(Deserialize, Serialize)]
pub(in crate::ledger) enum CheckpointOrderingRecord {
    Claim {
        event_id:      EventId,
        authorization: Box<ConflictAuthorization>,
    },
    Event(Box<JournalEvent>),
}

impl CheckpointOrderingRecord {
    pub(in crate::ledger) fn replay_record(
        &self,
        claim_ids: &mut impl Iterator<Item = ReservationId>,
    ) -> Option<OrderingReplayRecord<'_>> {
        Some(match self {
            Self::Claim {
                event_id,
                authorization,
            } => OrderingReplayRecord::Claim {
                event_id: *event_id,
                reservation_id: claim_ids.next()?,
                authorization,
            },
            Self::Event(event) => OrderingReplayRecord::Event(event),
        })
    }
}

/// Coordination records in their original order. References consume ordering records in order.
#[derive(Deserialize, Serialize)]
pub(in crate::ledger) enum CheckpointCoordinationRecord {
    Claim(Box<CheckpointClaim>),
    Event(Box<JournalEvent>),
    Ordering,
}

/// The remaining fields of a claim after the checkpoint moves its ordering fields.
#[derive(Deserialize, Serialize)]
pub(in crate::ledger) struct CheckpointClaim {
    schema_version:                   SchemaVersion,
    actor:                            JournalActor,
    identity_inputs:                  JournalMutationIdentityInputs,
    resolved_session:                 ResolvedSessionAtMutation,
    at:                               RecordedAt,
    projection_generation:            ProjectionGeneration,
    scopes:                           ReservationScopeSet,
    source:                           ClaimSource,
    purpose:                          ReservationPurpose,
    trunk_at_claim:                   TrunkObservationAtClaim,
    target:                           Option<Box<ClaimTarget>>,
    head_snapshot:                    ClaimHeadSnapshot,
    phase_start_head:                 ProtectedPhaseStartHead,
    worktree_root:                    CanonicalWorktreeRoot,
    worktree_administrative_locator:  WorktreeAdministrativeLocator,
    coordination_identity_provenance: CoordinationIdentityProvenance,
}

impl CheckpointCoordinationRecord {
    pub(in crate::ledger) fn split(
        events: &[JournalEvent],
    ) -> (Vec<ReservationId>, Vec<CheckpointOrderingRecord>, Vec<Self>) {
        let mut claim_ids = Vec::new();
        let mut ordering = Vec::new();
        let remainder = events
            .iter()
            .map(|event| match &event.operation {
                JournalOperation::Claim {
                    reservation_id,
                    scopes,
                    source,
                    purpose,
                    trunk_at_claim,
                    target,
                    head_snapshot,
                    phase_start_head,
                    worktree_root,
                    worktree_administrative_locator,
                    authorization,
                    coordination_identity_provenance,
                } => {
                    claim_ids.push(*reservation_id);
                    ordering.push(CheckpointOrderingRecord::Claim {
                        event_id:      event.event_id,
                        authorization: authorization.clone(),
                    });
                    Self::Claim(Box::new(CheckpointClaim {
                        schema_version:                   event.schema_version,
                        actor:                            event.actor.clone(),
                        identity_inputs:                  event.identity_inputs.clone(),
                        resolved_session:                 event.resolved_session.clone(),
                        at:                               event.at.clone(),
                        projection_generation:            event.projection_generation,
                        scopes:                           scopes.clone(),
                        source:                           source.clone(),
                        purpose:                          purpose.clone(),
                        trunk_at_claim:                   trunk_at_claim.clone(),
                        target:                           target.clone(),
                        head_snapshot:                    head_snapshot.clone(),
                        phase_start_head:                 phase_start_head.clone(),
                        worktree_root:                    worktree_root.clone(),
                        worktree_administrative_locator:  worktree_administrative_locator.clone(),
                        coordination_identity_provenance: *coordination_identity_provenance,
                    }))
                },
                JournalOperation::Widen { .. }
                | JournalOperation::Answer {
                    reservation_id: Some(_),
                    ..
                }
                | JournalOperation::Sequence { .. }
                | JournalOperation::ResolveDefer { .. }
                | JournalOperation::RetireCyclicOrderingEdge { .. } => {
                    ordering.push(CheckpointOrderingRecord::Event(Box::new(event.clone())));
                    Self::Ordering
                },
                _ => Self::Event(Box::new(event.clone())),
            })
            .collect();
        (claim_ids, ordering, remainder)
    }

    pub(in crate::ledger) fn restore(
        claim_ids: Vec<ReservationId>,
        ordering: Vec<CheckpointOrderingRecord>,
        remainder: Vec<Self>,
    ) -> Option<Vec<JournalEvent>> {
        let mut claim_ids = claim_ids.into_iter();
        let mut ordering = ordering.into_iter();
        let events = remainder
            .into_iter()
            .map(|record| match record {
                Self::Event(event) => Some(*event),
                Self::Ordering => match ordering.next()? {
                    CheckpointOrderingRecord::Event(event) => Some(*event),
                    CheckpointOrderingRecord::Claim { .. } => None,
                },
                Self::Claim(claim) => match ordering.next()? {
                    CheckpointOrderingRecord::Claim {
                        event_id,
                        authorization,
                    } => Some(claim.restore(event_id, claim_ids.next()?, authorization)),
                    CheckpointOrderingRecord::Event(_) => None,
                },
            })
            .collect::<Option<Vec<_>>>()?;
        (ordering.next().is_none() && claim_ids.next().is_none()).then_some(events)
    }
}

impl CheckpointClaim {
    fn restore(
        self,
        event_id: EventId,
        reservation_id: ReservationId,
        authorization: Box<ConflictAuthorization>,
    ) -> JournalEvent {
        JournalEvent {
            event_id,
            schema_version: self.schema_version,
            actor: self.actor,
            identity_inputs: self.identity_inputs,
            resolved_session: self.resolved_session,
            at: self.at,
            projection_generation: self.projection_generation,
            operation: JournalOperation::Claim {
                reservation_id,
                authorization,
                scopes: self.scopes,
                source: self.source,
                purpose: self.purpose,
                trunk_at_claim: self.trunk_at_claim,
                target: self.target,
                head_snapshot: self.head_snapshot,
                phase_start_head: self.phase_start_head,
                worktree_root: self.worktree_root,
                worktree_administrative_locator: self.worktree_administrative_locator,
                coordination_identity_provenance: self.coordination_identity_provenance,
            },
        }
    }
}
