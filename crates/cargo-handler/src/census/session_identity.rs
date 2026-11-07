//! Proven identities for staff placement and probe transport.

use serde::Deserialize;
use serde::Serialize;
use serde::ser::SerializeMap;

/// `SessionIdentity` carries the evidence that identifies a live session.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub(crate) enum SessionIdentity {
    /// No session evidence identifies this row.
    #[default]
    Unrecorded,
    /// The `sessionId` in a Claude Code session record.
    Claude(String),
    /// The interactive Codex process proves its thread.
    Codex(ProvenCodexThread),
}

/// `ProvenCodexThread` records how an interactive process proves its thread.
#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ProvenCodexThread {
    /// The process requests this thread on its command line.
    ResumeArgument(String),
    /// The process holds this staff session's rollout open.
    OpenStaffRollout(String),
}

impl SessionIdentity {
    /// `SessionIdentity` supplies zero or one stable ids for staff matching.
    pub(super) fn ids(&self) -> impl Iterator<Item = &str> {
        match self {
            Self::Unrecorded => None,
            Self::Claude(id)
            | Self::Codex(
                ProvenCodexThread::ResumeArgument(id) | ProvenCodexThread::OpenStaffRollout(id),
            ) => Some(id.as_str()),
        }
        .into_iter()
    }
}

impl Serialize for SessionIdentity {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut fields = serializer.serialize_map(None)?;
        let claude = match self {
            Self::Claude(id) => Some(id),
            Self::Unrecorded | Self::Codex(_) => None,
        };
        fields.serialize_entry("session_id", &claude)?;
        if let Self::Codex(proof) = self {
            fields.serialize_entry("codex_session", proof)?;
        }
        fields.end()
    }
}

impl From<String> for SessionIdentity {
    fn from(id: String) -> Self {
        if id.is_empty() {
            Self::Unrecorded
        } else {
            Self::Claude(id)
        }
    }
}

impl SessionIdentity {
    /// A stable Claude id joins a director record to its live census row.
    pub(super) fn claude_id(&self) -> Option<&str> {
        match self {
            Self::Claude(id) => Some(id),
            Self::Unrecorded | Self::Codex(_) => None,
        }
    }
}
