use super::panes::CiFetchKind;
use crate::project::AbsolutePath;
use crate::scan::CiFetchResult;
use crate::tui::state::OwnedRunId;
use crate::tui::state::OwnedRunTerminationOutcome;

/// Correlated immutable result sent by the process observer worker.
pub(super) type ProcessRefreshMsg = crate::process_observation::ProcessRefreshExecution<
    crate::build_monitor::BuildMonitoringRefreshCycleExecution,
>;

/// A frozen external termination request transported to the dedicated worker.
pub(super) type ProcessTerminationPlanMsg = crate::process_termination::TerminationExecutionPlan;

/// A correlated external termination result returned by the dedicated worker.
pub(super) type ProcessTerminationOutcomeMsg =
    crate::process_termination::TerminationOutcomeSummary;

/// Ordered events for one Cargo Port-owned run.
///
/// The process actor sends both termination outcomes and child completion on
/// this channel, so completion cannot overtake an accepted request's outcome.
pub(super) enum OwnedRunEvent {
    Started {
        owned_run_id: OwnedRunId,
    },
    Output {
        owned_run_id: OwnedRunId,
        line:         String,
    },
    /// Carriage-return line; replaces the last output line.
    Progress {
        owned_run_id: OwnedRunId,
        line:         String,
    },
    TerminationOutcome(OwnedRunTerminationOutcome),
    Finished {
        owned_run_id: OwnedRunId,
    },
}

/// Message sent when a background CI fetch completes.
pub(super) enum CiFetchMsg {
    /// The fetch completed with updated runs for the given project path.
    Complete {
        path:   String,
        result: CiFetchResult,
        kind:   CiFetchKind,
    },
}

pub(super) enum CleanMsg {
    Finished(AbsolutePath),
}
