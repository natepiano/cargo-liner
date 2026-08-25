//! Board command over one reconciled locked replay shared by both renderers.

use crate::board::BoardModel;
use crate::board::tui;
use crate::cli::CliOutputFormat;
use crate::config::Enrollment;
use crate::output::CommandVerb;
use crate::output::OutputEnvelope;
use crate::reconcile;
use crate::reconcile::RecoveredBypassReporting;

/// How the resolved board output mode completed.
pub(crate) enum BoardDisplayOutcome {
    /// The headless JSON response is ready for the ordinary emitter.
    HeadlessResponse(OutputEnvelope),
    /// The terminal view did not open, so the headless board response is ready.
    TerminalDidNotOpen(OutputEnvelope),
    /// The terminal view exited and restored the caller's terminal.
    TerminalRestored,
    /// The terminal view failed after it opened and displayed the board.
    TerminalFailedAfterOpening(OutputEnvelope),
    /// Repository or ledger facts could not be read.
    FactsUnavailable(OutputEnvelope),
}

/// Reconcile current repository facts and dispatch the resolved output mode.
pub(crate) fn execute(output_format: CliOutputFormat) -> BoardDisplayOutcome {
    let invocation_directory = match std::env::current_dir() {
        Ok(invocation_directory) => invocation_directory,
        Err(error) => {
            return BoardDisplayOutcome::FactsUnavailable(OutputEnvelope::ledger_unreadable(
                CommandVerb::Board,
                &error.to_string(),
            ));
        },
    };
    let report = match reconcile::reconcile(&invocation_directory, RecoveredBypassReporting::Report)
    {
        Ok(Enrollment::Enrolled(report)) => report,
        Ok(Enrollment::Unconfigured {
            expected_configuration_path,
        }) => {
            return BoardDisplayOutcome::FactsUnavailable(OutputEnvelope::unconfigured(
                CommandVerb::Board,
                &expected_configuration_path,
            ));
        },
        Err(error) => {
            return BoardDisplayOutcome::FactsUnavailable(error.into_output(CommandVerb::Board));
        },
    };
    let board = match BoardModel::build(&invocation_directory, &report) {
        Ok(board) => board,
        Err(error) => {
            return BoardDisplayOutcome::FactsUnavailable(OutputEnvelope::ledger_unreadable(
                CommandVerb::Board,
                &error.to_string(),
            ));
        },
    };

    match output_format {
        CliOutputFormat::Json => {
            BoardDisplayOutcome::HeadlessResponse(OutputEnvelope::board(board))
        },
        CliOutputFormat::Text => match tui::terminal_attachment() {
            tui::TerminalAttachment::Detached => {
                BoardDisplayOutcome::TerminalDidNotOpen(OutputEnvelope::board(board))
            },
            tui::TerminalAttachment::Attached => match tui::run(&board) {
                Ok(()) => BoardDisplayOutcome::TerminalRestored,
                Err(tui::BoardTerminalViewRunFailure::BeforeOpening(failure)) => {
                    BoardDisplayOutcome::TerminalDidNotOpen(
                        OutputEnvelope::board_with_terminal_view_opening_failure(
                            board,
                            &failure.to_string(),
                        ),
                    )
                },
                Err(tui::BoardTerminalViewRunFailure::AfterOpening(failure)) => {
                    BoardDisplayOutcome::TerminalFailedAfterOpening(
                        OutputEnvelope::terminal_view_failed_after_board_opened(
                            &failure.to_string(),
                        ),
                    )
                },
            },
        },
    }
}
