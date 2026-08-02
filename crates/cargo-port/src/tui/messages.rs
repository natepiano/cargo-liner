use super::panes::CiFetchKind;
use crate::project::AbsolutePath;
use crate::scan::CiFetchResult;

/// Correlated immutable result sent by the process observer worker.
pub(super) type ProcessRefreshMsg = crate::process_observation::ProcessRefreshExecution;

pub(super) enum ExampleMsg {
    Output(String),
    /// Carriage-return line; replaces the last output line.
    Progress(String),
    Finished,
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
