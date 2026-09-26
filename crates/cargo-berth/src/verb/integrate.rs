//! Stateful target integration through the same locked decision as the git hook.

use std::path::Path;
use std::path::PathBuf;

use crate::config::BerthConfig;
use crate::config::Enrollment;
use crate::coordination_identity::RecoveryCommandLine;
use crate::gate;
use crate::gate::GateDecision;
use crate::gate::GateError;
use crate::gate::IntegrationRequest;
use crate::git;
use crate::ids::ReservationId;
use crate::ledger::IntegrationTarget;
use crate::ledger::Ledger;
use crate::ledger::LedgerError;
use crate::ledger::LedgerTransactionError;
use crate::ledger::WorktreeContext;
use crate::output::CommandVerb;
use crate::output::IntegratedGateOutcome;
use crate::output::IntegrationPayload;
use crate::output::OutputEnvelope;
use crate::reconcile::ReconcileError;
use crate::reservation::RetainedReservationSet;

/// One reservation and its inseparable normal-or-forced integration policy.
pub(crate) struct IntegrateRequest {
    /// The reservation whose current protected work should enter its judging branch.
    pub(crate) reservation_id: ReservationId,
    /// Whether ordinary policy applies or one forced permit must be issued.
    pub(crate) integration:    IntegrationRequest,
}

/// Reconcile, decide, and atomically update the judging branch when policy permits it.
pub(crate) fn execute(
    integrate_request: IntegrateRequest,
    recovery_command_line: &RecoveryCommandLine,
) -> OutputEnvelope {
    let (invocation_directory, repository_root) = match integration_directories() {
        Ok(directories) => directories,
        Err(output_envelope) => return *output_envelope,
    };
    let configuration = match read_integration_configuration(&repository_root) {
        Ok(configuration) => configuration,
        Err(output_envelope) => return *output_envelope,
    };
    let target = match integration_target(
        &repository_root,
        &configuration,
        integrate_request.reservation_id,
    ) {
        Ok(target) => target,
        Err(output) => return *output,
    };
    execute_for_target(
        integrate_request,
        recovery_command_line,
        &invocation_directory,
        &repository_root,
        target,
    )
}

fn execute_for_target(
    integrate_request: IntegrateRequest,
    recovery_command_line: &RecoveryCommandLine,
    invocation_directory: &Path,
    repository_root: &Path,
    target: IntegrationTarget,
) -> OutputEnvelope {
    let previous = match git::branch_object_id(repository_root, target.short_name()) {
        Ok(previous) => previous,
        Err(error) => {
            return OutputEnvelope::ledger_unreadable(CommandVerb::Integrate, &error.to_string());
        },
    };
    let proposed = match git::head_object_id(repository_root) {
        Ok(proposed) => proposed,
        Err(error) => {
            return OutputEnvelope::ledger_unreadable(CommandVerb::Integrate, &error.to_string());
        },
    };
    let result = match gate::evaluate_integration(
        invocation_directory,
        integrate_request.reservation_id,
        integrate_request.integration,
        target.clone(),
        previous.clone(),
        proposed.clone(),
        recovery_command_line,
    ) {
        Ok(Enrollment::Enrolled(result)) => result,
        Ok(Enrollment::Unconfigured {
            expected_configuration_path,
        }) => {
            return OutputEnvelope::unconfigured(
                CommandVerb::Integrate,
                &expected_configuration_path,
            );
        },
        Err(error) => return gate_error(integrate_request.reservation_id, error),
    };
    let (generation, gate) = match result.decision {
        GateDecision::Blocked {
            generation,
            violations,
        } => {
            return OutputEnvelope::integration_blocked(
                integrate_request.reservation_id,
                &target,
                generation,
                violations,
            )
            .with_alerts(result.alerts);
        },
        GateDecision::Clear { generation } | GateDecision::Forced { generation, .. } => {
            (generation, IntegratedGateOutcome::Clear)
        },
        GateDecision::Observed {
            generation,
            violations,
        } => (generation, IntegratedGateOutcome::Observed { violations }),
        GateDecision::PermitIssued {
            generation,
            permit_id,
            reservation_id,
            skipped_holds,
            observed_violations,
        } => (
            {
                debug_assert_eq!(reservation_id, integrate_request.reservation_id);
                generation
            },
            IntegratedGateOutcome::Forced {
                permit_id,
                skipped_holds,
                observed_violations,
            },
        ),
    };
    if let Err(error) =
        git::update_local_branch(repository_root, target.short_name(), &proposed, &previous)
    {
        return OutputEnvelope::ledger_unreadable(
            CommandVerb::Integrate,
            &format!(
                "the validated {} update failed: {error}",
                target.short_name()
            ),
        )
        .with_alerts(result.alerts);
    }
    OutputEnvelope::integrated(IntegrationPayload::Integrated {
        reservation_id: integrate_request.reservation_id,
        target,
        previous,
        proposed,
        generation,
        gate,
    })
    .with_alerts(result.alerts)
}

fn integration_target(
    repository_root: &Path,
    configuration: &BerthConfig,
    reservation_id: ReservationId,
) -> Result<IntegrationTarget, Box<OutputEnvelope>> {
    let trunk = configuration.repository_trunk().map_err(|error| {
        Box::new(OutputEnvelope::invalid_input(
            CommandVerb::Integrate,
            &error,
        ))
    })?;
    let ledger = Ledger::open(repository_root)
        .map_err(|error| Box::new(OutputEnvelope::ledger_error(CommandVerb::Integrate, &error)))?;
    let events = ledger
        .read_validated_events()
        .map_err(|error| Box::new(OutputEnvelope::ledger_error(CommandVerb::Integrate, &error)))?;
    let reservations = RetainedReservationSet::replay(&events).map_err(|error| {
        Box::new(OutputEnvelope::replay_failure(
            CommandVerb::Integrate,
            &error,
        ))
    })?;
    let recorded = reservations
        .target_of(reservation_id, &trunk)
        .map_err(|error| {
            Box::new(OutputEnvelope::replay_failure(
                CommandVerb::Integrate,
                &error,
            ))
        })?;
    let judging = recorded
        .judging_branch(repository_root, &trunk)
        .map_err(|error| {
            Box::new(OutputEnvelope::ledger_unreadable(
                CommandVerb::Integrate,
                &error.to_string(),
            ))
        })?;
    Ok(judging.target().clone())
}

/// Resolve the issuing directory and its repository without losing either identity.
fn integration_directories() -> Result<(PathBuf, PathBuf), Box<OutputEnvelope>> {
    let invocation_directory = std::env::current_dir().map_err(|error| {
        Box::new(OutputEnvelope::ledger_unreadable(
            CommandVerb::Integrate,
            &error.to_string(),
        ))
    })?;
    let repository_root = git::repository_root(&invocation_directory).map_err(|error| {
        Box::new(OutputEnvelope::ledger_unreadable(
            CommandVerb::Integrate,
            &error.to_string(),
        ))
    })?;
    Ok((invocation_directory, repository_root))
}

/// Read enrolled integration policy or build the exact fact-free failure response.
fn read_integration_configuration(
    repository_root: &Path,
) -> Result<BerthConfig, Box<OutputEnvelope>> {
    let worktree_context = WorktreeContext::discover(repository_root)
        .map_err(|error| Box::new(OutputEnvelope::ledger_error(CommandVerb::Integrate, &error)))?;
    match BerthConfig::read(&worktree_context.configuration_lookup()) {
        Ok(Enrollment::Enrolled(berth_config)) => Ok(berth_config),
        Ok(Enrollment::Unconfigured {
            expected_configuration_path,
        }) => Err(Box::new(OutputEnvelope::unconfigured(
            CommandVerb::Integrate,
            &expected_configuration_path,
        ))),
        Err(error) => Err(Box::new(OutputEnvelope::ledger_error(
            CommandVerb::Integrate,
            &LedgerError::Config(error),
        ))),
    }
}

fn gate_error(reservation_id: ReservationId, error: GateError) -> OutputEnvelope {
    match error {
        GateError::Transaction(LedgerTransactionError::LockContention) => {
            OutputEnvelope::contention(
                CommandVerb::Integrate,
                &format!(
                    "The 10-second cargo-berth lock deadline was exhausted; no integration decision was made. Run cargo-berth integrate {reservation_id} again."
                ),
            )
        },
        GateError::Transaction(LedgerTransactionError::CorrectableInput(error)) => {
            OutputEnvelope::invalid_input(CommandVerb::Integrate, &error.to_string())
        },
        GateError::Transaction(error @ LedgerTransactionError::DerivedRecordTooLarge { .. }) => {
            OutputEnvelope::ledger_unreadable(CommandVerb::Integrate, &error.to_string())
        },
        GateError::CoordinationIdentity(rejection) => {
            OutputEnvelope::integration_rejected(reservation_id, rejection)
        },
        GateError::ReservationNotEntering(_)
        | GateError::NoHoldToForce(_)
        | GateError::MissingSkippedHold
        | GateError::HookReportedNoIssuingDirectory => {
            OutputEnvelope::invalid_input(CommandVerb::Integrate, &error.to_string())
        },
        GateError::Config(error) => {
            OutputEnvelope::ledger_error(CommandVerb::Integrate, &LedgerError::Config(error))
        },
        GateError::Ledger(error)
        | GateError::Transaction(LedgerTransactionError::LedgerUnreadable(error)) => {
            OutputEnvelope::ledger_error(CommandVerb::Integrate, &error)
        },
        GateError::Reconciliation(ReconcileError::Replay(error)) => {
            OutputEnvelope::replay_failure(CommandVerb::Integrate, &error)
        },
        GateError::PermitReplay(error) => {
            OutputEnvelope::forced_integration_permit_replay_failure(&error)
        },
        GateError::Reconciliation(_)
        | GateError::Planning(_)
        | GateError::MissingConstraintFact(_)
        | GateError::UnsupportedSymbolicTargetUpdate(_)
        | GateError::MultipleGatedReferences(_)
        | GateError::Git(_) => {
            OutputEnvelope::ledger_unreadable(CommandVerb::Integrate, &error.to_string())
        },
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use super::GateError;
    use super::ReconcileError;
    use super::ReservationId;
    use super::gate_error;
    use crate::gate::permit::ForcedIntegrationPermitReplayError;
    use crate::ids::ForcedIntegrationPermitId;
    use crate::reservation::ReservationReplayError;

    #[test]
    fn reconciliation_replay_error_preserves_reason_and_subject() {
        let reservation_id = ReservationId::new();
        let output_envelope = gate_error(
            reservation_id,
            GateError::Reconciliation(ReconcileError::Replay(
                ReservationReplayError::UnknownReservation(reservation_id),
            )),
        );
        let value =
            serde_json::to_value(output_envelope).expect("integration response should serialize");
        assert_eq!(value["payload"]["kind"], "replay_failure");
        assert_eq!(value["payload"]["data"]["reason"], "unknown_reservation");
        assert_eq!(value["payload"]["data"]["subject"]["kind"], "reservation");
        assert_eq!(
            value["payload"]["data"]["subject"]["id"],
            reservation_id.to_string()
        );
    }

    #[test]
    fn permit_replay_error_preserves_reason_and_subject() {
        let reservation_id = ReservationId::new();
        let permit_id = ForcedIntegrationPermitId::new();
        let output_envelope = gate_error(
            reservation_id,
            GateError::PermitReplay(ForcedIntegrationPermitReplayError::UnknownPermit(permit_id)),
        );
        let value =
            serde_json::to_value(output_envelope).expect("integration response should serialize");
        assert_eq!(value["payload"]["kind"], "replay_failure");
        assert_eq!(value["payload"]["data"]["reason"], "unknown_permit");
        assert_eq!(
            value["payload"]["data"]["subject"]["kind"],
            "forced_integration_permit"
        );
        assert_eq!(
            value["payload"]["data"]["subject"]["id"],
            permit_id.to_string()
        );
    }
}
