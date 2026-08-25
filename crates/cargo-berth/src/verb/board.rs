//! Headless board command over one reconciled locked replay.

use crate::board::BoardModel;
use crate::output::CommandVerb;
use crate::output::OutputEnvelope;
use crate::reconcile;
use crate::reconcile::RecoveredBypassReporting;

/// Reconcile current repository facts and return the coherent board projection.
pub(crate) fn execute() -> OutputEnvelope {
    let invocation_directory = match std::env::current_dir() {
        Ok(invocation_directory) => invocation_directory,
        Err(error) => {
            return OutputEnvelope::ledger_unreadable(CommandVerb::Board, &error.to_string());
        },
    };
    let report = match reconcile::reconcile(&invocation_directory, RecoveredBypassReporting::Report)
    {
        Ok(report) => report,
        Err(error) => return error.into_output(CommandVerb::Board),
    };
    match BoardModel::build(&invocation_directory, &report) {
        Ok(board) => OutputEnvelope::board(board),
        Err(error) => OutputEnvelope::ledger_unreadable(CommandVerb::Board, &error.to_string()),
    }
}
