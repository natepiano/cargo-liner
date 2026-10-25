//! Explicit, append-only retirement of one cycle-closing ordering relationship.

use super::constants::MUTATING_VERB_CONTENTION_TOLERANCE;
use super::error::LedgerError;
use super::error::LedgerTransactionError;
use super::handle;
use super::handle::LedgerPaths;
use super::identity;
use super::journal::Journal;
use super::journal::JournalActor;
use super::journal::JournalEvent;
use super::journal::JournalOperation;
use super::lock::MutationLock;
use super::projection::Projection;
use super::replay_checkpoint;
use crate::edge::CycleClosingEdge;
use crate::edge::EdgeReplayError;
use crate::edge::OrderingCycleRepairPlan;
use crate::edge::OrderingGraph;
use crate::ids::EventId;
use crate::ids::ReservationId;

/// The acquisition to which an ordering declaration belongs.
pub(crate) enum OrderingClaimOrigin {
    /// A claim, widen or answer belongs to this durable reservation acquisition.
    Claim {
        /// The reservation named by the ordering command.
        reservation_id: ReservationId,
        /// The record that first acquired it, even when a later widen added the edge.
        event_id:       EventId,
    },
    /// A standalone sequence or deferral resolution joined two existing reservations.
    Standalone,
}

/// The durable result of an explicit ordering-cycle repair.
pub(crate) enum OrderingCycleRepair {
    /// The journal already describes an acyclic graph; no record was appended.
    AlreadyAcyclic,
    /// One retirement was appended after verifying every other edge can still replay.
    Retired {
        /// The exact relationship removed, including its original declaration record.
        edge:                CycleClosingEdge,
        /// The original journal fact, including the claim when acquisition created the edge.
        declaration:         Box<JournalEvent>,
        /// The reservation acquisition behind a claim-time or widening-time answer.
        claim:               OrderingClaimOrigin,
        /// The newly appended retirement's durable identity.
        retirement_event_id: EventId,
    },
}

/// Read journal truth under the mutation lock, validate the single retirement, then append it.
pub(super) fn repair(
    paths: &LedgerPaths,
    actor: JournalActor,
) -> Result<OrderingCycleRepair, LedgerTransactionError> {
    let lock = MutationLock::acquire(&paths.lock, MUTATING_VERB_CONTENTION_TOLERANCE)
        .map_err(LedgerError::from)?;
    let journal = Journal::open_existing(&paths.journal).map_err(LedgerError::from)?;
    let mut replay = Journal::replay_read_only(&paths.journal).map_err(LedgerError::from)?;
    identity::validate_journal_repository(actor.repository, &replay)?;
    replay
        .reservations
        .as_ref()
        .map_err(Clone::clone)
        .map_err(LedgerError::ReservationReplay)?;
    let OrderingCycleRepairPlan::Retire(edge) =
        OrderingGraph::cycle_repair_plan(&replay.coordination_events).map_err(LedgerError::from)?
    else {
        return Ok(OrderingCycleRepair::AlreadyAcyclic);
    };
    let declaration = replay
        .coordination_events
        .iter()
        .find(|event| event.event_id() == edge.declaration_event_id)
        .cloned()
        .ok_or_else(|| {
            LedgerError::OrderingReplay(EdgeReplayError::MissingDeclaration(
                edge.declaration_event_id,
            ))
        })?;
    let retirement = JournalEvent::for_operation(
        actor,
        handle::next_projection_generation(replay.generation)?,
        JournalOperation::RetireCyclicOrderingEdge { edge: edge.clone() },
    );
    let claim = claim_origin(&declaration, &replay.coordination_events)?;
    let mut repaired_events = replay.coordination_events.clone();
    repaired_events.push(retirement.clone());
    // A second independent cycle or any malformed relationship leaves the journal untouched.
    OrderingGraph::replay(&repaired_events).map_err(LedgerError::from)?;
    // All validation precedes even a partial-tail repair.
    journal
        .advance_repairing_tail(&mut replay)
        .map_err(LedgerError::from)?;
    journal
        .append(&retirement)
        .map_err(handle::journal_append_transaction_error)?;
    journal
        .advance_repairing_tail(&mut replay)
        .map_err(LedgerError::from)?;
    replay_checkpoint::remove(&paths.replay_checkpoint).map_err(LedgerError::from)?;
    Projection::from_replay(retirement.actor.repository, &replay)
        .publish(&paths.directory, &paths.projection)
        .map_err(LedgerError::from)?;
    std::mem::drop(lock);
    Ok(OrderingCycleRepair::Retired {
        edge,
        declaration: Box::new(declaration),
        claim,
        retirement_event_id: retirement.event_id(),
    })
}

fn claim_origin(
    declaration: &JournalEvent,
    events: &[JournalEvent],
) -> Result<OrderingClaimOrigin, LedgerError> {
    let reservation = match &declaration.operation {
        JournalOperation::Claim { reservation_id, .. }
        | JournalOperation::Widen { reservation_id, .. }
        | JournalOperation::Answer {
            reservation_id: Some(reservation_id),
            ..
        } => *reservation_id,
        _ => return Ok(OrderingClaimOrigin::Standalone),
    };
    events
        .iter()
        .find_map(|event| match &event.operation {
            JournalOperation::Claim { reservation_id, .. } if *reservation_id == reservation => {
                Some(OrderingClaimOrigin::Claim {
                    reservation_id: reservation,
                    event_id:       event.event_id(),
                })
            },
            _ => None,
        })
        .ok_or_else(|| EdgeReplayError::MissingDeclaration(declaration.event_id()).into())
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::fs;

    use serde_json::Value;
    use tempfile::TempDir;

    use super::OrderingCycleRepair;
    use crate::edge::EdgeReplayError;
    use crate::edge::OrderingGraph;
    use crate::ids::CoordinationRunId;
    use crate::ids::EdgeId;
    use crate::ids::ReservationId;
    use crate::ids::WorktreeId;
    use crate::ledger::JournalActor;
    use crate::ledger::JournalOperation;
    use crate::ledger::Ledger;
    use crate::ledger::journal;
    use crate::ledger::test_support;
    use crate::reservation::RetainedReservationSet;

    #[derive(Clone, Copy)]
    enum CycleDeclaration {
        Claim,
        Widen,
    }

    struct CyclicLedger {
        repository: TempDir,
        ids:        [ReservationId; 3],
        edges:      [EdgeId; 3],
    }

    #[test]
    fn ordering_cycle_repair_retires_only_the_closing_edge_and_keeps_every_claim()
    -> Result<(), Box<dyn Error>> {
        assert_cycle_repair(CycleDeclaration::Claim)
    }

    #[test]
    fn ordering_cycle_repair_names_the_original_claim_when_a_widen_added_the_edge()
    -> Result<(), Box<dyn Error>> {
        assert_cycle_repair(CycleDeclaration::Widen)
    }

    fn cyclic_ledger(declaration: CycleDeclaration) -> Result<CyclicLedger, Box<dyn Error>> {
        let repository = test_support::scratch_repository();
        Ledger::initialize(repository.path())?;
        let ledger = Ledger::open(repository.path())?;
        let ids = [
            ReservationId::new(),
            ReservationId::new(),
            ReservationId::new(),
        ];
        let edges = [EdgeId::new(), EdgeId::new(), EdgeId::new()];
        for (index, id) in ids.iter().enumerate() {
            let authorization = match index {
                0 => serde_json::json!({"kind": "no_conflict"}),
                1 => sequence_authorization(&[(ids[0], "holder_before_requester", edges[0])]),
                _ if matches!(declaration, CycleDeclaration::Claim) => sequence_authorization(&[
                    (ids[1], "holder_before_requester", edges[1]),
                    (ids[0], "requester_before_holder", edges[2]),
                ]),
                _ => serde_json::json!({"kind": "no_conflict"}),
            };
            let claim: JournalOperation = serde_json::from_value(serde_json::json!({
                "op": "claim", "reservation_id": id,
                "scopes": [{"path": "shared.rs", "kind": "file"}],
                "source": {"kind": "explicit"}, "purpose": {"kind": "not_provided_by_caller"},
                "trunk_at_claim": "1111111111111111111111111111111111111111",
                "head_snapshot": {"kind": "branch", "full_ref": "refs/heads/lane",
                    "head": "1111111111111111111111111111111111111111"},
                "phase_start_head": "1111111111111111111111111111111111111111",
                "worktree_root": repository.path(), "worktree_administrative_locator": "main",
                "authorization": authorization, "coordination_identity_provenance": "not_presented",
            }))?;
            ledger.append_legacy_journal_operation(
                JournalActor {
                    repository: ledger.repository_identity()?,
                    worktree:   WorktreeId::new(),
                    run:        CoordinationRunId::new(),
                },
                claim,
            )?;
        }
        if matches!(declaration, CycleDeclaration::Widen) {
            let operation: JournalOperation = serde_json::from_value(serde_json::json!({
                "op": "widen", "reservation_id": ids[2],
                "added_scopes": [{"path": "additional.rs", "kind": "file"}],
                "cause": {"kind": "explicit", "reason": "add ordered work"},
                "edit_blocking_status": "blocking",
                "authorization": sequence_authorization(&[
                    (ids[1], "holder_before_requester", edges[1]),
                    (ids[0], "requester_before_holder", edges[2]),
                ]),
            }))?;
            ledger.append_legacy_journal_operation(
                JournalActor {
                    repository: ledger.repository_identity()?,
                    worktree:   WorktreeId::new(),
                    run:        CoordinationRunId::new(),
                },
                operation,
            )?;
        }
        Ok(CyclicLedger {
            repository,
            ids,
            edges,
        })
    }

    fn assert_cycle_repair(declaration_kind: CycleDeclaration) -> Result<(), Box<dyn Error>> {
        let CyclicLedger {
            repository,
            ids,
            edges,
        } = cyclic_ledger(declaration_kind)?;
        let ledger = Ledger::open(repository.path())?;
        let before = fs::read(&ledger.paths.journal)?;
        let events = journal::complete_record_events(&before)?;
        assert!(matches!(
            OrderingGraph::replay(&events),
            Err(EdgeReplayError::Cycle)
        ));
        let reservations = RetainedReservationSet::replay(&events)?;
        let OrderingCycleRepair::Retired {
            edge,
            declaration,
            retirement_event_id,
            claim,
        } = Ledger::repair_ordering_cycle(repository.path())?
        else {
            return Err("cyclic ledger must append a retirement".into());
        };
        assert_eq!(edge.edge_id, edges[2]);
        assert_eq!((edge.predecessor, edge.successor), (ids[2], ids[0]));
        assert_eq!(&*declaration, events.last().ok_or("cycle declaration")?);
        assert_eq!(edge.declaration_event_id, declaration.event_id());
        assert!(
            matches!(claim, super::OrderingClaimOrigin::Claim { reservation_id, event_id }
            if reservation_id == ids[2] && event_id == events[2].event_id())
        );
        let after = fs::read(&ledger.paths.journal)?;
        assert_eq!(after[..before.len()], before);
        let repaired = journal::complete_record_events(&after)?;
        assert_eq!(repaired.len(), events.len() + 1);
        assert_eq!(
            repaired.last().ok_or("retirement record")?.event_id(),
            retirement_event_id
        );
        assert_eq!(RetainedReservationSet::replay(&repaired)?, reservations);
        let graph = OrderingGraph::replay(&repaired)?;
        assert_eq!(graph.edge_count(), edges.len() - 1);
        let mut endpoints = graph
            .predecessors()
            .flat_map(|before| {
                before
                    .successors
                    .iter()
                    .map(move |after| (before.reservation_id, *after))
            })
            .collect::<Vec<_>>();
        endpoints.sort();
        let mut expected = vec![(ids[0], ids[1]), (ids[1], ids[2])];
        expected.sort();
        assert_eq!(endpoints, expected);
        assert!(matches!(
            Ledger::repair_ordering_cycle(repository.path())?,
            OrderingCycleRepair::AlreadyAcyclic
        ));
        assert_eq!(fs::read(&ledger.paths.journal)?, after);
        Ok(())
    }

    fn sequence_authorization(holders: &[(ReservationId, &str, EdgeId)]) -> Value {
        serde_json::json!({
            "kind": "sequence_per_holder",
            "overlaps": holders.iter().map(|(holder, _, _)| serde_json::json!({
                "reservation_id": holder,
                "scope_revision": [{"path": "shared.rs", "kind": "file"}],
                "scopes": [{"path": "shared.rs", "kind": "file"}],
            })).collect::<Vec<_>>(),
            "holders": holders.iter().map(|(holder, direction, edge)| serde_json::json!({
                "blocker": holder, "direction": direction, "edge_id": edge,
            })).collect::<Vec<_>>(),
            "reason": "test ordering",
        })
    }
}
