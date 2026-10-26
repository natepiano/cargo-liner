//! Board command over one reconciled locked replay shared by both renderers.

use std::path::Path;
use std::path::PathBuf;

use crate::board;
use crate::board::BoardError;
use crate::board::BoardModel;
use crate::board::BoardReportRendering;
use crate::board::tui;
use crate::board::tui::BoardModelSource;
use crate::board::tui::BoardRefreshFailure;
use crate::board::tui::BoardTerminalViewRunFailure;
use crate::board::tui::TerminalAttachment;
use crate::cli::CliOutputFormat;
use crate::config::Enrollment;
use crate::git::RetentionMaintenance;
use crate::ids::ReservationId;
use crate::ledger::JournalPosition;
use crate::ledger::Ledger;
use crate::output::CommandVerb;
use crate::output::EngineAnswerOccasion;
use crate::output::OutputEnvelope;
use crate::output::ReservationLifecycleQueryRejection;
use crate::reconcile;
use crate::reconcile::ReconciliationReport;
use crate::reconcile::RecoveredBypassReporting;
use crate::reservation::ReservationReplayError;

/// Repository invocation retained for live terminal-board refreshes.
struct ReconciledBoardSource {
    invocation_directory: PathBuf,
    ledger:               Ledger,
    maintenance:          TerminalMaintenance,
}

/// Refreshes remember a deferral silently; restoration consumes its single notice.
struct TerminalMaintenance {
    pending: RetentionMaintenance,
}

impl TerminalMaintenance {
    fn remember(&mut self, maintenance: &RetentionMaintenance) {
        if let RetentionMaintenance::Deferred(_) = maintenance {
            self.pending = maintenance.clone();
        }
    }

    fn report_after_restore(&mut self) {
        std::mem::replace(&mut self.pending, RetentionMaintenance::Completed).report();
    }
}

impl BoardModelSource for ReconciledBoardSource {
    fn current_position(&mut self) -> Result<JournalPosition, BoardRefreshFailure> {
        self.ledger
            .current_journal_position()
            .map_err(BoardRefreshFailure::LedgerRead)
    }

    fn rebuild(&mut self) -> Result<BoardModel, BoardRefreshFailure> {
        let report = match reconcile::reconcile(
            &self.invocation_directory,
            RecoveredBypassReporting::Report,
        )
        .map_err(|error| BoardRefreshFailure::Reconcile(Box::new(error)))?
        {
            Enrollment::Enrolled(report) => report,
            Enrollment::Unconfigured {
                expected_configuration_path,
            } => {
                return Err(BoardRefreshFailure::RepositoryNotConfigured(
                    expected_configuration_path,
                ));
            },
        };
        self.maintenance.remember(&report.retention_maintenance);
        BoardModel::build(&self.invocation_directory, &report).map_err(BoardRefreshFailure::Model)
    }
}

/// Which read-only board result the caller selected.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BoardOutputSelection {
    /// Assemble the complete board and use its selected renderer.
    CompleteBoard,
    /// Read one reservation's lifecycle and extents independently of board placement.
    ReservationLifecycleFor(ReservationId),
}

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
pub(crate) fn execute(
    output_selection: BoardOutputSelection,
    output_format: CliOutputFormat,
) -> BoardDisplayOutcome {
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
    let outcome = match output_selection {
        BoardOutputSelection::CompleteBoard => {
            execute_complete_board(&invocation_directory, &report, output_format)
        },
        BoardOutputSelection::ReservationLifecycleFor(reservation_id) => {
            execute_reservation_lifecycle(&report, reservation_id)
        },
    };
    match outcome {
        BoardDisplayOutcome::HeadlessResponse(output) => BoardDisplayOutcome::HeadlessResponse(
            output.with_retention_maintenance(&report.retention_maintenance),
        ),
        BoardDisplayOutcome::TerminalDidNotOpen(output) => BoardDisplayOutcome::TerminalDidNotOpen(
            output.with_retention_maintenance(&report.retention_maintenance),
        ),
        BoardDisplayOutcome::TerminalFailedAfterOpening(output) => {
            BoardDisplayOutcome::TerminalFailedAfterOpening(
                output.with_retention_maintenance(&report.retention_maintenance),
            )
        },
        BoardDisplayOutcome::TerminalRestored => BoardDisplayOutcome::TerminalRestored,
        BoardDisplayOutcome::FactsUnavailable(output) => BoardDisplayOutcome::FactsUnavailable(
            output.with_retention_maintenance(&report.retention_maintenance),
        ),
    }
}

fn execute_complete_board(
    invocation_directory: &Path,
    report: &ReconciliationReport,
    output_format: CliOutputFormat,
) -> BoardDisplayOutcome {
    let board = match BoardModel::build(invocation_directory, report) {
        Ok(board) => board,
        Err(error) => {
            return BoardDisplayOutcome::FactsUnavailable(OutputEnvelope::ledger_unreadable(
                CommandVerb::Board,
                &error.to_string(),
            ));
        },
    };
    let occasion = EngineAnswerOccasion::current();
    let alert_routing = report.alert_routing(occasion);
    let rendering = BoardReportRendering::for_occasion(occasion);

    match output_format {
        CliOutputFormat::Json => BoardDisplayOutcome::HeadlessResponse(OutputEnvelope::board(
            board,
            &alert_routing,
            rendering,
        )),
        CliOutputFormat::Text => match tui::terminal_attachment() {
            TerminalAttachment::Detached => BoardDisplayOutcome::TerminalDidNotOpen(
                OutputEnvelope::board(board, &alert_routing, rendering),
            ),
            TerminalAttachment::Attached => {
                let ledger = match Ledger::open(invocation_directory) {
                    Ok(ledger) => ledger,
                    Err(error) => {
                        return BoardDisplayOutcome::FactsUnavailable(
                            OutputEnvelope::ledger_error(CommandVerb::Board, &error),
                        );
                    },
                };
                let mut source = ReconciledBoardSource {
                    invocation_directory: invocation_directory.to_path_buf(),
                    ledger,
                    maintenance: TerminalMaintenance {
                        pending: report.retention_maintenance.clone(),
                    },
                };
                match tui::run(&mut source, &board) {
                    Ok(()) => {
                        source.maintenance.report_after_restore();
                        BoardDisplayOutcome::TerminalRestored
                    },
                    Err(BoardTerminalViewRunFailure::BeforeOpening(failure)) => {
                        BoardDisplayOutcome::TerminalDidNotOpen(
                            OutputEnvelope::board_with_terminal_view_opening_failure(
                                board,
                                &alert_routing,
                                rendering,
                                &failure.to_string(),
                            ),
                        )
                    },
                    Err(BoardTerminalViewRunFailure::AfterOpening(failure)) => {
                        BoardDisplayOutcome::TerminalFailedAfterOpening(
                            OutputEnvelope::terminal_view_failed_after_board_opened(
                                &failure.to_string(),
                            )
                            .with_retention_maintenance(&source.maintenance.pending),
                        )
                    },
                }
            },
        },
    }
}

fn execute_reservation_lifecycle(
    report: &ReconciliationReport,
    reservation_id: ReservationId,
) -> BoardDisplayOutcome {
    match board::reservation_lifecycle_snapshot(report, reservation_id) {
        Ok(reservation_report) => BoardDisplayOutcome::HeadlessResponse(
            OutputEnvelope::reservation_lifecycle(reservation_report),
        ),
        Err(BoardError::Reservation(ReservationReplayError::UnknownReservation(
            reservation_id,
        ))) => BoardDisplayOutcome::HeadlessResponse(
            OutputEnvelope::reservation_lifecycle_query_rejected(
                ReservationLifecycleQueryRejection::UnknownReservation { reservation_id },
            ),
        ),
        Err(error) => BoardDisplayOutcome::FactsUnavailable(OutputEnvelope::ledger_unreadable(
            CommandVerb::Board,
            &error.to_string(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use cargo_berth_test_support::CommandDeadlineExt as _;

    use super::TerminalMaintenance;
    use crate::git::GitError;
    use crate::git::RetentionMaintenance;
    use crate::ids::ReservationId;

    #[test]
    fn terminal_refresh_deferrals_are_silent_until_one_notice_after_restore()
    -> Result<(), Box<dyn std::error::Error>> {
        const CHILD: &str = "CARGO_BERTH_TEST_TERMINAL_MAINTENANCE";
        const TEST: &str = "verb::board::tests::terminal_refresh_deferrals_are_silent_until_one_notice_after_restore";
        if std::env::var_os(CHILD).is_none() {
            let output = std::process::Command::new(std::env::current_exe()?)
                .args(["--exact", TEST, "--nocapture"])
                .env(CHILD, "1")
                .bounded_output()?;
            assert!(output.status.success(), "{output:?}");
            let diagnostic = String::from_utf8_lossy(&output.stderr);
            assert!(
                diagnostic.starts_with("terminal restored\n"),
                "no notice while drawing: {diagnostic}"
            );
            assert_eq!(
                diagnostic
                    .matches("Retention-ref maintenance deferred:")
                    .count(),
                1,
                "{diagnostic}"
            );
            return Ok(());
        }
        let repository = tempfile::tempdir()?;
        for args in [
            vec!["init", "--quiet"],
            vec![
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.com",
                "commit",
                "--quiet",
                "--allow-empty",
                "-m",
                "initial",
            ],
        ] {
            let output = cargo_berth_test_support::fixture_git_command()
                .current_dir(repository.path())
                .args(args)
                .bounded_output()?;
            assert!(output.status.success(), "{output:?}");
        }
        let tip = cargo_berth_test_support::fixture_git_command()
            .current_dir(repository.path())
            .args(["rev-parse", "HEAD"])
            .bounded_output()?;
        let tip = String::from_utf8(tip.stdout)?.trim().parse()?;
        let id = ReservationId::new();
        let lock = repository.path().join(format!(
            ".git/{}.lock",
            crate::git::reservation_retention_ref_name(id)
        ));
        std::fs::create_dir_all(lock.parent().ok_or("lock directory")?)?;
        std::fs::write(&lock, b"")?;
        let output = cargo_berth_test_support::fixture_git_command()
            .current_dir(repository.path())
            .args(["config", "core.filesRefLockTimeout", "1"])
            .bounded_output()?;
        assert!(output.status.success(), "{output:?}");
        let Err(GitError::LockRefused(lock)) =
            crate::git::write_reservation_retention_ref(repository.path(), id, &tip)
        else {
            return Err("expected refusal".into());
        };
        let deferred = RetentionMaintenance::Deferred(lock);
        let mut terminal = TerminalMaintenance {
            pending: RetentionMaintenance::Completed,
        };
        terminal.remember(&deferred);
        terminal.remember(&deferred);
        terminal.remember(&RetentionMaintenance::Completed);
        eprintln!("terminal restored");
        terminal.report_after_restore();
        terminal.report_after_restore();
        Ok(())
    }
}
