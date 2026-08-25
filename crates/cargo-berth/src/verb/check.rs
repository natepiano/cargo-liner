//! Mutation-free tier-one edit overlap checks with one blocked-path reconciliation retry.

use std::path::Path;

use crate::config::Enrollment;
use crate::ledger::EditAuthorization;
use crate::ledger::Ledger;
use crate::ledger::LedgerError;
use crate::output::CommandVerb;
use crate::output::OutputEnvelope;
use crate::reconcile;
use crate::reconcile::RecoveredBypassReporting;
use crate::reservation::ReservationConflict;
use crate::reservation::ReservationReplayError;
use crate::reservation::RetainedReservationSet;
use crate::scope::DeclaredReservationScopeSet;
use crate::scope::PathCase;
use crate::scope::PathCaseError;
use crate::scope::ReservationScopeSet;

/// A parsed edit check with lexically valid requested paths.
pub(crate) struct CheckRequest {
    /// The exact paths the edit operation proposes to modify.
    pub(crate) declared_scopes: DeclaredReservationScopeSet,
}

struct CheckDecision {
    scopes:    ReservationScopeSet,
    conflicts: Vec<ReservationConflict>,
}

/// A prerequisite that failed before an overlap decision could be reached.
enum CheckDecisionError {
    /// The ledger snapshot could not be read.
    Ledger(LedgerError),
    /// The repository's path-case rule could not be determined.
    PathCase(PathCaseError),
    /// The retained reservation set could not be replayed.
    ReservationReplay(ReservationReplayError),
}

impl CheckDecisionError {
    fn into_output(self) -> OutputEnvelope {
        match self {
            Self::Ledger(error) => OutputEnvelope::ledger_error(CommandVerb::Check, &error),
            Self::PathCase(error) => {
                OutputEnvelope::ledger_unreadable(CommandVerb::Check, &error.to_string())
            },
            Self::ReservationReplay(error) => {
                OutputEnvelope::ledger_unreadable(CommandVerb::Check, &error.to_string())
            },
        }
    }
}

/// Evaluate tier-one overlap and reconcile only after the read-only snapshot blocks.
pub(crate) fn execute(check_request: CheckRequest) -> OutputEnvelope {
    let invocation_directory = match std::env::current_dir() {
        Ok(invocation_directory) => invocation_directory,
        Err(error) => {
            return OutputEnvelope::ledger_unreadable(CommandVerb::Check, &error.to_string());
        },
    };
    let first_decision = match decide(&invocation_directory, check_request.declared_scopes.clone())
    {
        Ok(Enrollment::Enrolled(check_decision)) => check_decision,
        Ok(Enrollment::Unconfigured {
            expected_configuration_path,
        }) => {
            return OutputEnvelope::unconfigured(CommandVerb::Check, &expected_configuration_path);
        },
        Err(error) => return error.into_output(),
    };
    if first_decision.conflicts.is_empty() {
        return OutputEnvelope::clear_check(first_decision.scopes);
    }
    let reconciliation_report =
        match reconcile::reconcile(&invocation_directory, RecoveredBypassReporting::Defer) {
            Ok(Enrollment::Enrolled(reconciliation_report)) => reconciliation_report,
            Ok(Enrollment::Unconfigured {
                expected_configuration_path,
            }) => {
                return OutputEnvelope::unconfigured(
                    CommandVerb::Check,
                    &expected_configuration_path,
                );
            },
            Err(_) => {
                return OutputEnvelope::blocked_check(
                    first_decision.scopes,
                    first_decision.conflicts,
                );
            },
        };
    match decide(&invocation_directory, check_request.declared_scopes) {
        Ok(Enrollment::Enrolled(check_decision)) if check_decision.conflicts.is_empty() => {
            OutputEnvelope::clear_check(check_decision.scopes)
                .with_alerts(reconciliation_report.alerts)
        },
        Ok(Enrollment::Enrolled(check_decision)) => {
            OutputEnvelope::blocked_check(check_decision.scopes, check_decision.conflicts)
                .with_alerts(reconciliation_report.alerts)
        },
        Ok(Enrollment::Unconfigured {
            expected_configuration_path,
        }) => OutputEnvelope::unconfigured(CommandVerb::Check, &expected_configuration_path),
        Err(_) => OutputEnvelope::blocked_check(first_decision.scopes, first_decision.conflicts)
            .with_alerts(reconciliation_report.alerts),
    }
}

fn decide(
    invocation_directory: &Path,
    declared_scopes: DeclaredReservationScopeSet,
) -> Result<Enrollment<CheckDecision>, CheckDecisionError> {
    let snapshot = match Ledger::read_for_edit_check(invocation_directory)
        .map_err(CheckDecisionError::Ledger)?
    {
        Enrollment::Enrolled(snapshot) => snapshot,
        Enrollment::Unconfigured {
            expected_configuration_path,
        } => {
            return Ok(Enrollment::Unconfigured {
                expected_configuration_path,
            });
        },
    };
    let path_case = PathCase::read(snapshot.worktree_context().common_git_directory())
        .map_err(CheckDecisionError::PathCase)?;
    let scopes = declared_scopes.into_minimal_antichain(path_case);
    let reservations = RetainedReservationSet::replay(snapshot.events())
        .map_err(CheckDecisionError::ReservationReplay)?;
    let edit_authorization = EditAuthorization::resolve(
        snapshot.worktree_context().administrative_directory(),
        &snapshot.worktree_context().ledger_directory(),
    );
    let conflicts = reservations.conflicts_for_edit(&scopes, edit_authorization, path_case);
    Ok(Enrollment::Enrolled(CheckDecision { scopes, conflicts }))
}
