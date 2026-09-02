//! Branch rewrites and the phase-anchor re-anchoring one forces.

use std::convert::Infallible;
use std::path::Path;

use super::error::GateError;
use super::error::GateTransactionRejection;
use super::reference_transaction::ReferenceObject;
use super::reference_transaction::ReferenceUpdate;
use crate::git;
use crate::git::Reachability;
use crate::ids::CoordinationRunId;
use crate::ids::GitObjectId;
use crate::ledger;
use crate::ledger::ClaimHeadSnapshot;
use crate::ledger::FullRefName;
use crate::ledger::JournalOperation;
use crate::ledger::Ledger;
use crate::ledger::LedgerCommittedActionError;
use crate::ledger::LedgerCommittedActionOutcome;
use crate::ledger::ReconciliationValidation;
use crate::ledger::ReservationSnapshot;
use crate::ledger::WorktreeContext;
use crate::reconcile::GateReconciliationError;
use crate::reservation::ReservationLifecycle;
use crate::reservation::RetainedReservationSet;

/// One branch whose new tip no longer contains the tip it replaced.
pub(super) struct BranchRewrite {
    /// The full `refs/heads/...` name the transaction moved.
    reference: FullRefName,
    /// The tip the branch carried before the rewrite.
    previous:  GitObjectId,
    /// The tip the branch carries now.
    proposed:  GitObjectId,
}

/// Select the branch updates that discarded history rather than extending it.
///
/// A rebase, an amend, and a reset all land here; an ordinary commit and a fast-forward
/// merge do not, because their previous tip survives in the proposed history.
pub(super) fn branch_rewrites(
    invocation_directory: &Path,
    updates: &[&ReferenceUpdate],
) -> Result<Vec<BranchRewrite>, GateError> {
    let mut rewrites = Vec::new();
    for update in updates {
        let (ReferenceObject::Object(previous), ReferenceObject::Object(proposed)) =
            (&update.previous, &update.proposed)
        else {
            continue;
        };
        if previous == proposed {
            continue;
        }
        match git::reachability(invocation_directory, previous, proposed).map_err(GateError::Git)? {
            Reachability::NotAncestor => rewrites.push(BranchRewrite {
                reference: update.reference.clone(),
                previous:  previous.clone(),
                proposed:  proposed.clone(),
            }),
            Reachability::Ancestor | Reachability::ObjectUnknown => {},
        }
    }
    Ok(rewrites)
}

/// Move every active reservation's phase start onto the rewritten history of its branch.
///
/// `<phase_start_head>..HEAD` only means "the commits this phase authored" while the
/// proposed history still contains the anchor. A rebase makes that false with no signal
/// of its own, and drift then reads the new base's commits as this phase's work, raising
/// incursions against worktrees that did nothing and widening onto files nobody here
/// opened. Re-anchoring restores the range's meaning at the moment the branch moves.
///
/// The reservations are found by the branch each one recorded at claim time. Git refuses
/// to check one branch out in two worktrees at once, so a branch name identifies the
/// acting worktree even though the hook has already changed directory away from it.
pub(super) fn reanchor_rewritten_phases(
    invocation_directory: &Path,
    worktree_context: &WorktreeContext,
    rewrites: &[BranchRewrite],
) -> Result<(), GateError> {
    let ledger = Ledger::open(invocation_directory)?;
    let journal_mutation_actor = ledger::resolve_identity(worktree_context)?
        .journal_mutation_actor_for(CoordinationRunId::new());
    let repository_root = worktree_context.repository_root();
    let outcome = ledger
        .transact_reconciliation(
            journal_mutation_actor.worktree_id,
            journal_mutation_actor.coordination_run_id,
            |state| {
                let reservations = match RetainedReservationSet::replay(state.events()) {
                    Ok(reservations) => reservations,
                    Err(error) => {
                        return ReconciliationValidation::Reject(
                            GateTransactionRejection::Reconciliation(
                                GateReconciliationError::Reservation(error),
                            ),
                        );
                    },
                };
                ReconciliationValidation::Apply {
                    operations:             resnapshot_operations(
                        repository_root,
                        &reservations,
                        rewrites,
                    ),
                    recoverable_operations: Vec::new(),
                    action:                 (),
                }
            },
            |(), _, _| Ok::<(), Infallible>(()),
        )
        .map_err(|error| match error {
            LedgerCommittedActionError::Transaction(error) => GateError::Transaction(error),
            LedgerCommittedActionError::Action(error) => match error {},
        })?;
    match outcome {
        LedgerCommittedActionOutcome::Appended { output: (), .. } => Ok(()),
        LedgerCommittedActionOutcome::Rejected(rejection) => Err(rejection.into()),
    }
}

/// Compute one replacement anchor per active reservation the rewrites moved.
///
/// A reservation whose anchor git cannot recompute keeps the one it has. The branch has
/// already moved by the time this runs, so refusing the whole transaction would neither
/// undo the rewrite nor leave the ledger in a better state than a stale anchor does.
fn resnapshot_operations(
    repository_root: &Path,
    reservations: &RetainedReservationSet,
    rewrites: &[BranchRewrite],
) -> Vec<JournalOperation> {
    let mut operations = Vec::new();
    for reservation in reservations.iter() {
        if !matches!(reservation.lifecycle(), ReservationLifecycle::Active) {
            continue;
        }
        let ClaimHeadSnapshot::Branch { full_ref, .. } = reservation.head_snapshot() else {
            continue;
        };
        let Some(rewrite) = rewrites
            .iter()
            .find(|rewrite| &rewrite.reference == full_ref)
        else {
            continue;
        };
        let phase_start = reservation.phase_start_head();
        let Ok(claim_snapshot) = git::rewritten_phase_anchor(
            repository_root,
            phase_start.as_ref(),
            &rewrite.previous,
            &rewrite.proposed,
        ) else {
            continue;
        };
        if &claim_snapshot == phase_start.as_ref() {
            continue;
        }
        operations.push(JournalOperation::Resnapshot {
            reservation_id: reservation.id(),
            snapshot:       ReservationSnapshot::Active { claim_snapshot },
        });
    }
    operations
}
