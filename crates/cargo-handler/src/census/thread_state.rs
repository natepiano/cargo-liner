//! The most recent activity of a live Codex thread.

use serde::Deserialize;
use serde::Serialize;

const IDLE_TEXT: &str = "idle";

/// The most recent turn event in a live Codex thread's rollout.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ThreadState {
    /// A turn has started and has not completed or aborted.
    Busy,
    /// The last turn finished, aborted, or no turn has started yet.
    #[default]
    Idle,
}

impl ThreadState {
    /// The same activity words used for Claude sessions.
    pub(crate) const fn text(self) -> &'static str {
        match self {
            Self::Busy => crate::constants::BUSY_STATUS,
            Self::Idle => IDLE_TEXT,
        }
    }

    /// Old probes and non-thread children need no extra idle field.
    #[expect(
        clippy::trivially_copy_pass_by_ref,
        reason = "serde skip_serializing_if predicates receive a reference"
    )]
    pub(super) const fn is_idle(&self) -> bool { matches!(self, Self::Idle) }
}
