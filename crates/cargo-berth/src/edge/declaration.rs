//! Whole-operation ordering admission before a declaration can reach the journal.

use super::EdgeReplayError;
use super::OrderingCycle;
use crate::answer::ConflictAuthorization;
use crate::ids::EventId;
use crate::ledger::JournalOperation;
use crate::ledger::ReplayedLedgerState;

/// A journal operation whose entire edge set preserves an acyclic ordering graph.
pub(crate) struct CycleFreeOrderingOperation(JournalOperation);

impl CycleFreeOrderingOperation {
    /// Admit one operation against the transaction's current replay.
    pub(crate) fn check(
        state: &ReplayedLedgerState<'_>,
        operation: JournalOperation,
    ) -> Result<Self, OrderingDeclarationError> {
        Ok(Self::check_sequence(state, vec![operation])?.remove(0))
    }

    /// Replay once and extend one graph through every operation in publication order.
    pub(crate) fn check_sequence(
        state: &ReplayedLedgerState<'_>,
        operations: Vec<JournalOperation>,
    ) -> Result<Vec<Self>, OrderingDeclarationError> {
        if !operations.iter().any(introduces_ordering) {
            return Ok(operations.into_iter().map(Self).collect());
        }
        let mut graph = state
            .ordering_graph()
            .map_err(OrderingDeclarationError::Replay)?
            .clone();
        let mut admitted = Vec::with_capacity(operations.len());
        for operation in operations {
            graph
                .apply_operation(&operation, EventId::new())
                .map_err(OrderingDeclarationError::Replay)?;
            if introduces_ordering(&operation) {
                graph.check_acyclic()?;
            }
            admitted.push(Self(operation));
        }
        Ok(admitted)
    }

    /// Borrow the admitted facts when preparing their session publication.
    pub(crate) const fn operation(&self) -> &JournalOperation { &self.0 }

    /// Cross the journal representation boundary after whole-operation validation.
    pub(crate) fn into_operation(self) -> JournalOperation { self.0 }
}

/// An ordering proposal either closes a named cycle or depends on invalid recorded facts.
#[derive(Debug)]
pub(crate) enum OrderingDeclarationError {
    /// The complete proposed edge set closes this cycle.
    Cycle(OrderingCycle),
    /// Existing facts or bindings do not replay.
    Replay(EdgeReplayError),
}

/// Only sequence authorizations add edges; other operations need no ordering replay.
fn introduces_ordering(operation: &JournalOperation) -> bool {
    match operation {
        JournalOperation::Claim { authorization, .. } => authorization_adds_edges(authorization),
        JournalOperation::Widen { authorization, .. } => authorization_adds_edges(authorization),
        JournalOperation::Answer {
            reservation_id: Some(_),
            authorizations,
            ..
        } => authorizations.iter().any(authorization_adds_edges),
        JournalOperation::Sequence { .. } | JournalOperation::ResolveDefer { .. } => true,
        JournalOperation::Answer {
            reservation_id: None,
            ..
        }
        | JournalOperation::MergeExtentObserved { .. }
        | JournalOperation::HolderMergeExtentObserved { .. }
        | JournalOperation::Checkpoint { .. }
        | JournalOperation::Resnapshot { .. }
        | JournalOperation::Retarget { .. }
        | JournalOperation::UnrecordedTargetsPinned { .. }
        | JournalOperation::Renew { .. }
        | JournalOperation::Release { .. }
        | JournalOperation::ReplaceReleaseDisposition { .. }
        | JournalOperation::EvidenceRevalidated { .. }
        | JournalOperation::ScopedPatchEquivalenceChecked { .. }
        | JournalOperation::ScopedPatchComparisonAttempted { .. }
        | JournalOperation::SuccessorScopedPatchEquivalenceChecked { .. }
        | JournalOperation::SuccessorScopedPatchComparisonAttempted { .. }
        | JournalOperation::OverlapRefused { .. }
        | JournalOperation::RetireCyclicOrderingEdge { .. }
        | JournalOperation::Incursion { .. }
        | JournalOperation::ResolveIncursion { .. }
        | JournalOperation::ForcedIntegrationPermit { .. }
        | JournalOperation::ConsumeForcedIntegrationPermit { .. }
        | JournalOperation::Bypass { .. }
        | JournalOperation::RebindWorktree { .. }
        | JournalOperation::RelocateWorktree { .. } => false,
    }
}

const fn authorization_adds_edges(authorization: &ConflictAuthorization) -> bool {
    matches!(
        authorization,
        ConflictAuthorization::Sequence { .. }
            | ConflictAuthorization::SequencePerHolder { .. }
            | ConflictAuthorization::SequenceWithExistingEdges { .. }
    )
}
