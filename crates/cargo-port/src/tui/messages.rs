use super::panes::CiFetchKind;
use super::state::OwnedRunId;
use super::state::OwnedRunTerminationOutcome;
use crate::project::AbsolutePath;
use crate::scan::CiFetchResult;

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
