//! Replace a live reservation's durable integration branch.

use super::claim;
use crate::config::Enrollment;
use crate::ids::ReservationId;
use crate::ledger;
use crate::ledger::ClaimHeadSnapshot;
use crate::ledger::IntegrationTarget;
use crate::ledger::JournalOperation;
use crate::ledger::Ledger;
use crate::ledger::LedgerTransactionError;
use crate::ledger::LedgerTransactionOutcome;
use crate::ledger::TargetRefusal;
use crate::ledger::TargetSource;
use crate::ledger::TransactionValidation;
use crate::ledger::WorktreeContext;
use crate::output::CommandVerb;
use crate::output::OutputEnvelope;
use crate::output::TargetView;
use crate::reconcile;
use crate::reconcile::RecoveredBypassReporting;
use crate::reservation::ReservationLifecycle;
use crate::reservation::ReservationReplayError;
use crate::reservation::RetainedReservationSet;

enum RetargetRejection {
    InvalidInput(String),
    InvalidTarget(TargetRefusal),
    Replay(ReservationReplayError),
}

/// Reconcile and append a replacement target for one unreleased reservation.
pub(crate) fn execute(reservation_id: ReservationId, target_argument: &str) -> OutputEnvelope {
    let invocation_directory = match std::env::current_dir() {
        Ok(directory) => directory,
        Err(error) => {
            return OutputEnvelope::ledger_unreadable(CommandVerb::Retarget, &error.to_string());
        },
    };
    let report = match reconcile::reconcile(&invocation_directory, RecoveredBypassReporting::Defer)
    {
        Ok(Enrollment::Enrolled(report)) => report,
        Ok(Enrollment::Unconfigured {
            expected_configuration_path,
        }) => {
            return OutputEnvelope::unconfigured(
                CommandVerb::Retarget,
                &expected_configuration_path,
            );
        },
        Err(error) => return error.into_output(CommandVerb::Retarget),
    };
    let response = execute_after_reconciliation(reservation_id, target_argument);
    response.with_alerts(report.alerts)
}

fn execute_after_reconciliation(
    reservation_id: ReservationId,
    target_argument: &str,
) -> OutputEnvelope {
    let Ok(target) = IntegrationTarget::from_branch_argument(target_argument) else {
        return OutputEnvelope::invalid_input(
            CommandVerb::Retarget,
            &TargetRefusal::unresolved(target_argument).message(),
        );
    };
    let invocation_directory = match std::env::current_dir() {
        Ok(directory) => directory,
        Err(error) => {
            return OutputEnvelope::ledger_unreadable(CommandVerb::Retarget, &error.to_string());
        },
    };
    let context = match WorktreeContext::discover(&invocation_directory) {
        Ok(context) => context,
        Err(error) => {
            return OutputEnvelope::ledger_unreadable(CommandVerb::Retarget, &error.to_string());
        },
    };
    let Ok(target_commit) = claim::read_reference(&context, target.reference().as_str()) else {
        return OutputEnvelope::invalid_input(
            CommandVerb::Retarget,
            &TargetRefusal::unresolved(target_argument).message(),
        );
    };
    let actor = match ledger::resolve_identity(&context) {
        Ok(actor) => actor,
        Err(error) => {
            return OutputEnvelope::ledger_unreadable(CommandVerb::Retarget, &error.to_string());
        },
    };
    let ledger = match Ledger::open_from_discovered_worktree(&context) {
        Ok(ledger) => ledger,
        Err(error) => return OutputEnvelope::ledger_error(CommandVerb::Retarget, &error),
    };
    let outcome = ledger.transact(actor.worktree_id, actor.coordination_run_id, |state| {
        let reservations = match RetainedReservationSet::replay(state.events()) {
            Ok(reservations) => reservations,
            Err(error) => return TransactionValidation::Reject(RetargetRejection::Replay(error)),
        };
        let Ok(reservation) = reservations.reservation(reservation_id) else {
            return TransactionValidation::Reject(RetargetRejection::InvalidInput(format!(
                "reservation {reservation_id} does not exist"
            )));
        };
        if matches!(
            reservation.lifecycle(),
            ReservationLifecycle::Released { .. }
        ) {
            return TransactionValidation::Reject(RetargetRejection::InvalidInput(format!(
                "reservation {reservation_id} is released"
            )));
        }
        if let ClaimHeadSnapshot::Branch { full_ref, .. } = reservation.head_snapshot()
            && full_ref == target.reference()
        {
            return TransactionValidation::Reject(RetargetRejection::InvalidTarget(
                TargetRefusal::own_branch(target_argument),
            ));
        }
        TransactionValidation::Append(Box::new(JournalOperation::Retarget {
            reservation_id,
            target: target.clone(),
            source: TargetSource::ClaimArgument,
            target_commit: target_commit.clone(),
        }))
    });
    match outcome {
        Ok(LedgerTransactionOutcome::Appended { .. }) => OutputEnvelope::retargeted(
            reservation_id,
            TargetView::retargeted(&target, &target_commit),
        ),
        Ok(LedgerTransactionOutcome::Rejected(RetargetRejection::InvalidInput(error))) => {
            OutputEnvelope::invalid_input(CommandVerb::Retarget, &error)
        },
        Ok(LedgerTransactionOutcome::Rejected(RetargetRejection::InvalidTarget(refusal))) => {
            OutputEnvelope::invalid_input(CommandVerb::Retarget, &refusal.message())
        },
        Ok(LedgerTransactionOutcome::Rejected(RetargetRejection::Replay(error))) => {
            OutputEnvelope::replay_failure(CommandVerb::Retarget, &error)
        },
        Err(LedgerTransactionError::LockContention) => OutputEnvelope::contention(
            CommandVerb::Retarget,
            &LedgerTransactionError::LockContention.to_string(),
        ),
        Err(LedgerTransactionError::CorrectableInput(error)) => {
            OutputEnvelope::invalid_input(CommandVerb::Retarget, &error.to_string())
        },
        Err(LedgerTransactionError::LedgerUnreadable(error)) => {
            OutputEnvelope::ledger_error(CommandVerb::Retarget, &error)
        },
        Err(error @ LedgerTransactionError::DerivedRecordTooLarge { .. }) => {
            OutputEnvelope::ledger_unreadable(CommandVerb::Retarget, &error.to_string())
        },
    }
}
