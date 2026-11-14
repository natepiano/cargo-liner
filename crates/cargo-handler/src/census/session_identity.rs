//! Proven identities for staff placement and probe transport.

use serde::Deserialize;
use serde::Serialize;
use serde::ser::SerializeMap;

use super::hierarchy::AstraMembership;

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
    /// A proven Codex thread has checked Astra membership.
    Astra(AstraSession),
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
    pub(crate) fn ids(&self) -> impl Iterator<Item = &str> {
        match self {
            Self::Unrecorded => None,
            Self::Claude(id) => Some(id.as_str()),
            Self::Codex(proof) => Some(proof.id()),
            Self::Astra(session) => Some(session.proof.id()),
        }
        .into_iter()
    }
}

impl Serialize for SessionIdentity {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut fields = serializer.serialize_map(None)?;
        let claude = match self {
            Self::Claude(id) => Some(id),
            Self::Unrecorded | Self::Codex(_) | Self::Astra(_) => None,
        };
        fields.serialize_entry("session_id", &claude)?;
        match self {
            Self::Codex(proof) => fields.serialize_entry("codex_session", proof)?,
            Self::Astra(session) => {
                fields.serialize_entry("codex_session", &session.proof)?;
                fields.serialize_entry("astra_member", &session.membership)?;
            },
            Self::Claude(_) | Self::Unrecorded => {},
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
            Self::Unrecorded | Self::Codex(_) | Self::Astra(_) => None,
        }
    }
}

impl ProvenCodexThread {
    pub(super) fn id(&self) -> &str {
        match self {
            Self::ResumeArgument(id) | Self::OpenStaffRollout(id) => id,
        }
    }
}

impl SessionIdentity {
    pub(crate) const fn astra_membership(&self) -> AstraSessionMembership<'_> {
        match self {
            Self::Astra(session) => AstraSessionMembership::Member(&session.membership),
            _ => AstraSessionMembership::Ordinary,
        }
    }

    pub(super) fn with_astra(self, member: AstraMembership) -> Self {
        match self {
            Self::Codex(proof) if proof.id() == member.session() => Self::Astra(AstraSession {
                proof,
                membership: member,
            }),
            Self::Astra(mut session) if session.proof.id() == member.session() => {
                session.membership = member;
                Self::Astra(session)
            },
            other => other,
        }
    }
}

/// The private fields keep the proven thread equal to the membership session.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct AstraSession {
    proof:      ProvenCodexThread,
    membership: AstraMembership,
}

/// An ordinary session has no Astra responsibility.
#[derive(Clone, Copy, Debug)]
pub(crate) enum AstraSessionMembership<'a> {
    Ordinary,
    Member(&'a AstraMembership),
}

#[cfg(test)]
impl AstraSessionMembership<'_> {
    pub(crate) const fn is_member(self) -> bool { matches!(self, Self::Member(_)) }
}
