//! Mutation-free tier-one edit overlap checks.

use crate::ledger::EditAuthorization;
use crate::ledger::Ledger;
use crate::output::CommandVerb;
use crate::output::OutputEnvelope;
use crate::reservation::RetainedReservationSet;
use crate::scope::DeclaredReservationScopeSet;
use crate::scope::PathCase;

/// A parsed edit check with lexically valid requested paths.
pub(crate) struct CheckRequest {
    /// The exact paths the edit operation proposes to modify.
    pub(crate) declared_scopes: DeclaredReservationScopeSet,
}

/// Evaluate only tier-one overlap without git or any ledger mutation.
pub(crate) fn execute(check_request: CheckRequest) -> OutputEnvelope {
    let invocation_directory = match std::env::current_dir() {
        Ok(invocation_directory) => invocation_directory,
        Err(error) => {
            return OutputEnvelope::ledger_unreadable(CommandVerb::Check, &error.to_string());
        },
    };
    let snapshot = match Ledger::read_for_edit_check(&invocation_directory) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            return OutputEnvelope::ledger_unreadable(CommandVerb::Check, &error.to_string());
        },
    };
    let path_case = match PathCase::read(snapshot.worktree_context().common_git_directory()) {
        Ok(path_case) => path_case,
        Err(error) => {
            return OutputEnvelope::ledger_unreadable(CommandVerb::Check, &error.to_string());
        },
    };
    let scopes = check_request
        .declared_scopes
        .into_minimal_antichain(path_case);
    let reservations = match RetainedReservationSet::replay(snapshot.events()) {
        Ok(reservations) => reservations,
        Err(error) => {
            return OutputEnvelope::ledger_unreadable(CommandVerb::Check, &error.to_string());
        },
    };
    let edit_authorization =
        EditAuthorization::resolve(snapshot.worktree_context().administrative_directory());
    let conflicts = reservations.conflicts_for_edit(&scopes, edit_authorization, path_case);
    if conflicts.is_empty() {
        OutputEnvelope::clear_check(scopes)
    } else {
        OutputEnvelope::blocked_check(scopes, conflicts)
    }
}
