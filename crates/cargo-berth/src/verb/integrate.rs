//! Stateful trunk integration through the same locked decision as the git hook.

use std::path::Path;

use crate::config::BerthConfig;
use crate::config::Enrollment;
use crate::gate;
use crate::gate::GateDecision;
use crate::gate::GateError;
use crate::gate::IntegrationRequest;
use crate::git;
use crate::ids::ReservationId;
use crate::ledger::LedgerError;
use crate::ledger::LedgerTransactionError;
use crate::output::CommandVerb;
use crate::output::IntegratedGateOutcome;
use crate::output::IntegrationPayload;
use crate::output::OutputEnvelope;

/// One reservation and its inseparable normal-or-forced integration policy.
pub(crate) struct IntegrateRequest {
    /// The reservation whose current protected work should enter trunk.
    pub(crate) reservation_id: ReservationId,
    /// Whether ordinary policy applies or one forced permit must be minted.
    pub(crate) integration:    IntegrationRequest,
}

/// Reconcile, decide, and atomically update configured trunk when policy permits it.
pub(crate) fn execute(integrate_request: IntegrateRequest) -> OutputEnvelope {
    let invocation_directory = match std::env::current_dir() {
        Ok(invocation_directory) => invocation_directory,
        Err(error) => {
            return OutputEnvelope::ledger_unreadable(CommandVerb::Integrate, &error.to_string());
        },
    };
    let repository_root = match git::repository_root(&invocation_directory) {
        Ok(repository_root) => repository_root,
        Err(error) => {
            return OutputEnvelope::ledger_unreadable(CommandVerb::Integrate, &error.to_string());
        },
    };
    let configuration = match read_integration_configuration(&repository_root) {
        Ok(configuration) => configuration,
        Err(output_envelope) => return *output_envelope,
    };
    let previous = match git::branch_object_id(&repository_root, &configuration.trunk) {
        Ok(previous) => previous,
        Err(error) => {
            return OutputEnvelope::ledger_unreadable(CommandVerb::Integrate, &error.to_string());
        },
    };
    let proposed = match git::head_object_id(&repository_root) {
        Ok(proposed) => proposed,
        Err(error) => {
            return OutputEnvelope::ledger_unreadable(CommandVerb::Integrate, &error.to_string());
        },
    };
    let result = match gate::evaluate_integration(
        &invocation_directory,
        integrate_request.reservation_id,
        integrate_request.integration,
        previous.clone(),
        proposed.clone(),
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
        git::update_local_branch(&repository_root, &configuration.trunk, &proposed, &previous)
    {
        return OutputEnvelope::ledger_unreadable(
            CommandVerb::Integrate,
            &format!("the validated main update failed: {error}"),
        )
        .with_alerts(result.alerts);
    }
    OutputEnvelope::integrated(IntegrationPayload::Integrated {
        reservation_id: integrate_request.reservation_id,
        previous,
        proposed,
        generation,
        gate,
    })
    .with_alerts(result.alerts)
}

/// Read enrolled integration policy or build the exact fact-free failure response.
fn read_integration_configuration(
    repository_root: &Path,
) -> Result<BerthConfig, Box<OutputEnvelope>> {
    match BerthConfig::read(repository_root) {
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
        GateError::InactiveSessionMapping(_)
        | GateError::InactiveMarkerRun(_)
        | GateError::ReservationNotEntering(_)
        | GateError::NoHoldToForce(_)
        | GateError::MissingSkippedHold => {
            OutputEnvelope::invalid_input(CommandVerb::Integrate, &error.to_string())
        },
        GateError::Config(error) => {
            OutputEnvelope::ledger_error(CommandVerb::Integrate, &LedgerError::Config(error))
        },
        GateError::Ledger(error)
        | GateError::Transaction(LedgerTransactionError::LedgerUnreadable(error)) => {
            OutputEnvelope::ledger_error(CommandVerb::Integrate, &error)
        },
        GateError::Reconciliation(_)
        | GateError::Planning(_)
        | GateError::MissingConstraintFact(_)
        | GateError::UnsupportedSymbolicTrunkUpdate
        | GateError::Git(_)
        | GateError::PermitReplay(_) => {
            OutputEnvelope::ledger_unreadable(CommandVerb::Integrate, &error.to_string())
        },
    }
}
