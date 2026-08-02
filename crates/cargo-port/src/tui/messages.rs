use super::panes::CiFetchKind;
use crate::project::AbsolutePath;
use crate::scan::CiFetchResult;
use crate::tui::state::OwnedRunId;

/// Correlated immutable result sent by the process observer worker.
pub(super) type ProcessRefreshMsg = crate::process_observation::ProcessRefreshExecution;

pub(super) enum ExampleMsg {
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
