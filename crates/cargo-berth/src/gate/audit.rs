//! Auditing the forced integration permits a committed trunk move consumed.

use std::convert::Infallible;
use std::path::Path;

use super::decision::GatePurpose;
use super::decision::decide;
use super::decision::entering_reservations;
use super::decision::newly_reachable_commits;
use super::error::GateError;
use super::error::GateTransactionRejection;
use super::reference_transaction::ProposedMainMove;
use super::reference_transaction::ReferenceTransactionIssuingDirectory;
use super::reference_transaction::ReferenceTransactionPhase;
use crate::config::BerthConfig;
use crate::ids::CoordinationRunId;
use crate::ledger;
use crate::ledger::Ledger;
use crate::ledger::LedgerCommittedActionError;
use crate::ledger::LedgerCommittedActionOutcome;
use crate::ledger::ReconciliationValidation;
use crate::ledger::WorktreeContext;
use crate::reconcile;

pub(super) fn commit_forced_permit_audits(
    invocation_directory: &Path,
    worktree_context: &WorktreeContext,
    berth_config: &BerthConfig,
    update: &ProposedMainMove,
    issuing_directory: &ReferenceTransactionIssuingDirectory,
) -> Result<(), GateError> {
    let purpose = GatePurpose::Hook {
        phase:             ReferenceTransactionPhase::Committed,
        issuing_directory: issuing_directory.clone(),
    };
    purpose.identity_validation()?;
    let ledger = Ledger::open(invocation_directory)?;
    let ledger_repository = ledger.repository_identity()?;
    let journal_mutation_actor = ledger::resolve_identity(worktree_context)?
        .journal_mutation_actor_for(CoordinationRunId::new());
    let outcome = ledger
        .transact_reconciliation(
            journal_mutation_actor.worktree_id,
            journal_mutation_actor.coordination_run_id,
            |state| {
                let prepared = match reconcile::prepare_gate_reconciliation(
                    state.events(),
                    state.generation(),
                    worktree_context,
                    ledger_repository,
                    berth_config,
                    update.proposed.clone(),
                ) {
                    Ok(prepared) => prepared,
                    Err(error) => {
                        return ReconciliationValidation::Reject(
                            GateTransactionRejection::Reconciliation(error),
                        );
                    },
                };
                let newly_reachable =
                    match newly_reachable_commits(worktree_context.repository_root(), update) {
                        Ok(newly_reachable) => newly_reachable,
                        Err(error) => {
                            return ReconciliationValidation::Reject(
                                GateTransactionRejection::Git(error),
                            );
                        },
                    };
                let entering = entering_reservations(prepared.constraints(), &newly_reachable);
                let (_, operations) = match decide(
                    state.events(),
                    prepared.constraints(),
                    &entering,
                    &purpose,
                    berth_config.gate_mode,
                ) {
                    Ok(decision) => decision,
                    Err(error) => return ReconciliationValidation::Reject(error),
                };
                let operations = prepared.into_committed_hook_operations(operations);
                ReconciliationValidation::Apply {
                    operations,
                    recoverable_operations: Vec::new(),
                    action: (),
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
