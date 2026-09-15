//! Auditing the forced integration permits a committed trunk move consumed.

use std::path::Path;

use super::decision;
use super::decision::GatePurpose;
use super::error::GateError;
use super::error::GateTransactionRejection;
use super::reference_transaction::ProposedMainMove;
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
    issuing_directory: &Path,
) -> Result<(), GateError> {
    let purpose = GatePurpose::Hook {
        phase:             ReferenceTransactionPhase::Committed,
        issuing_directory: issuing_directory.to_path_buf(),
    };
    purpose.identity_validation()?;
    let ledger = Ledger::open(invocation_directory)?;
    let ledger_repository = ledger.repository_identity()?;
    let journal_mutation_actor = ledger::resolve_identity(worktree_context)?
        .journal_mutation_actor_for(CoordinationRunId::new());
    decision::retry_rewrite_reconciliation(|| {
        let rewrite_preflight =
            reconcile::prepare_rewrite_reconciliation(worktree_context, &ledger, berth_config)
                .map_err(GateError::Reconciliation)?;
        let outcome = ledger
            .transact_reconciliation(
                journal_mutation_actor.worktree_id,
                journal_mutation_actor.coordination_run_id,
                |state| {
                    let prepared = match reconcile::prepare_gate_reconciliation(
                        &state,
                        worktree_context,
                        ledger_repository,
                        berth_config,
                        update.proposed.clone(),
                        reconcile::GateReconciliationPurpose::CommittedAudit,
                        rewrite_preflight,
                    ) {
                        Ok(prepared) => prepared,
                        Err(error) => {
                            return ReconciliationValidation::Reject(
                                GateTransactionRejection::Reconciliation(error),
                            );
                        },
                    };
                    let newly_reachable = match decision::newly_reachable_commits(
                        worktree_context.repository_root(),
                        update,
                    ) {
                        Ok(newly_reachable) => newly_reachable,
                        Err(error) => {
                            return ReconciliationValidation::Reject(
                                GateTransactionRejection::Git(error),
                            );
                        },
                    };
                    let entering = decision::entering_reservations(&prepared, &newly_reachable);
                    let (_, operations) = match decision::decide(
                        state.events(),
                        prepared.constraints(),
                        &entering,
                        &purpose,
                        berth_config.gate_mode,
                    ) {
                        Ok(decision) => decision,
                        Err(error) => return ReconciliationValidation::Reject(error),
                    };
                    let (operations, action) = prepared.into_committed_hook_action(operations);
                    ReconciliationValidation::Apply {
                        operations,
                        recoverable_operations: Vec::new(),
                        action,
                    }
                },
                reconcile::CommittedHookReconciliationAction::commit,
            )
            .map_err(|error| match error {
                LedgerCommittedActionError::Transaction(error) => GateError::Transaction(error),
                LedgerCommittedActionError::Action(error) => GateError::Reconciliation(error),
            })?;
        match outcome {
            LedgerCommittedActionOutcome::Appended { output: (), .. } => Ok(()),
            LedgerCommittedActionOutcome::Rejected(rejection) => Err(rejection.into()),
        }
    })
}
