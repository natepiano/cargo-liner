//! Best-effort audit records for overlap refusals, after the refusing decision is final.

use std::convert::Infallible;

use crate::ledger::JournalOperation;
use crate::ledger::Ledger;
use crate::ledger::RefusedVerb;
use crate::ledger::RefusingHolderSet;
use crate::ledger::ReservationScopeSet;
use crate::ledger::ResolvedJournalMutationActor;
use crate::ledger::TransactionValidation;
use crate::ledger::WorktreeContext;
use crate::reservation::ReservationConflict;

/// Append a [`JournalOperation::OverlapRefused`] record; an empty holder set, a ledger
/// that will not open, or a failed append leaves the refusal unchanged and writes nothing.
pub(super) fn record(
    worktree_context: &WorktreeContext,
    requester: ResolvedJournalMutationActor,
    verb: RefusedVerb,
    scopes: &ReservationScopeSet,
    conflicts: &[ReservationConflict],
) {
    let Ok(holders) = RefusingHolderSet::try_from(conflicts) else {
        return;
    };
    let Ok(ledger) = Ledger::open_from_discovered_worktree(worktree_context) else {
        return;
    };
    let _ = ledger.transact(requester.worktree_id, requester.coordination_run_id, |_| {
        TransactionValidation::<Infallible>::Append(Box::new(JournalOperation::OverlapRefused {
            verb,
            scopes: scopes.clone(),
            holders: holders.clone(),
        }))
    });
}
