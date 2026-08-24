//! Mutation-free tier-one edit overlap checks with one blocked-path reconciliation retry.

use std::path::Path;

use crate::ledger::EditAuthorization;
use crate::ledger::Ledger;
use crate::output::CommandVerb;
use crate::output::OutputEnvelope;
use crate::reconcile;
use crate::reservation::ReservationConflict;
use crate::reservation::RetainedReservationSet;
use crate::scope::DeclaredReservationScopeSet;
use crate::scope::PathCase;
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
        Ok(check_decision) => check_decision,
        Err(error) => return OutputEnvelope::ledger_unreadable(CommandVerb::Check, &error),
    };
    if first_decision.conflicts.is_empty() {
        return OutputEnvelope::clear_check(first_decision.scopes);
    }
    let Ok(reconciliation_report) = reconcile::reconcile(&invocation_directory) else {
        return OutputEnvelope::blocked_check(first_decision.scopes, first_decision.conflicts);
    };
    match decide(&invocation_directory, check_request.declared_scopes) {
        Ok(check_decision) if check_decision.conflicts.is_empty() => {
            OutputEnvelope::clear_check(check_decision.scopes)
                .with_alerts(reconciliation_report.alerts)
        },
        Ok(check_decision) => {
            OutputEnvelope::blocked_check(check_decision.scopes, check_decision.conflicts)
                .with_alerts(reconciliation_report.alerts)
        },
        Err(_) => OutputEnvelope::blocked_check(first_decision.scopes, first_decision.conflicts)
            .with_alerts(reconciliation_report.alerts),
    }
}

fn decide(
    invocation_directory: &Path,
    declared_scopes: DeclaredReservationScopeSet,
) -> Result<CheckDecision, String> {
    let snapshot =
        Ledger::read_for_edit_check(invocation_directory).map_err(|error| error.to_string())?;
    let path_case = PathCase::read(snapshot.worktree_context().common_git_directory())
        .map_err(|error| error.to_string())?;
    let scopes = declared_scopes.into_minimal_antichain(path_case);
    let reservations =
        RetainedReservationSet::replay(snapshot.events()).map_err(|error| error.to_string())?;
    let edit_authorization =
        EditAuthorization::resolve(snapshot.worktree_context().administrative_directory());
    let conflicts = reservations.conflicts_for_edit(&scopes, edit_authorization, path_case);
    Ok(CheckDecision { scopes, conflicts })
}
