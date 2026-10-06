//! A claim's overlap answer and the reason recorded with it.

use std::error::Error;
use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;
use std::str::FromStr;

use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;

use super::constants::DEFAULT_ANSWER_FIRST_READY_REASON;
use super::constants::ENROLLMENT_AUTHORIZATION_REASON;
use crate::config::DefaultAnswer;
use crate::ids::ReservationId;
use crate::ledger::OrderingDirection;

/// A claim's semantic overlap-answer state after CLI conversion.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum OverlapAuthorizationRequest {
    /// The claim did not attempt to authorize a conflict.
    Absent,
    /// The claim deliberately supplied a permissive answer and its reason.
    Permissive(Box<PermissiveOverlapAuthorizationRequest>),
    /// The claim defers the integration order with every named holder and reserves nothing.
    Defer(Box<DeferAnswerRequest>),
}

/// A deferral of the integration order with each named holder, recorded without a reservation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DeferAnswerRequest {
    /// The holders named by each `--defer`; the CLI builds this only from one or more flags.
    pub(crate) blockers: Vec<ReservationId>,
    /// Why the caller deferred the order.
    pub(crate) reason:   OverlapAuthorizationReason,
}

/// A deliberate overlap answer with its inseparable authorization reason.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PermissiveOverlapAuthorizationRequest {
    /// The requested editing and integration behavior.
    pub(crate) answer: PermissiveOverlapAnswer,
    /// Why the caller chose this answer.
    pub(crate) reason: OverlapAuthorizationReason,
}

/// One holder a `--before` or `--after` flag named, and the order that flag chose.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SequencedBlocker {
    /// The holder named as the other endpoint of the ordering edge.
    pub(crate) blocker:   ReservationId,
    /// Which endpoint of the ordering edge must integrate first.
    pub(crate) direction: OrderingDirection,
}

/// Whether a sequence answer may retire live ordering edges that point the other way.
///
/// The edges in question join a named holder to a nonterminal reservation of the claimant's own
/// coordination run and worktree, so a later claim that orders the same work in the opposite
/// direction re-rules the earlier answer instead of recording a second, contradicting one.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum OrderingReplacement {
    /// Refuse the claim when its order contradicts a live edge; the caller did not pass
    /// `--replace`.
    Keep,
    /// Retire every contradicted live edge in the same journal record that adds the new ones.
    Replace,
}

/// One of the overlap answers that permits concurrent editing and reserves the claimed paths.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum PermissiveOverlapAnswer {
    /// Declare an integration order between the requester and every named blocker.
    Sequence {
        /// One entry per distinct holder named by `--before` or `--after`, each in the direction
        /// its own flag chose; the CLI builds this only from one or more flags and refuses a
        /// holder named by both.
        blockers:    Vec<SequencedBlocker>,
        /// Whether `--replace` lets this answer retire the live edges it contradicts.
        replacement: OrderingReplacement,
    },
    /// Permit editing without adding an integration constraint.
    Override {
        /// The blocker named by the caller.
        blocker: ReservationId,
    },
}

/// Why the caller chose one specific overlap answer.
#[derive(Clone, Debug, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(transparent)]
pub(crate) struct OverlapAuthorizationReason(#[schemars(length(min = 1))] String);

impl OverlapAuthorizationReason {
    /// The engine's explanation for a deferral recorded by enrollment.
    pub(crate) fn enrollment() -> Self { Self(ENROLLMENT_AUTHORIZATION_REASON.to_owned()) }
}

impl From<DefaultAnswer> for OverlapAuthorizationReason {
    /// The engine's explanation for an override the repository's default answer recorded.
    fn from(default_answer: DefaultAnswer) -> Self {
        match default_answer {
            DefaultAnswer::FirstReady => Self(DEFAULT_ANSWER_FIRST_READY_REASON.to_owned()),
        }
    }
}

impl Display for OverlapAuthorizationReason {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result { formatter.write_str(&self.0) }
}

impl FromStr for OverlapAuthorizationReason {
    type Err = EmptyOverlapAuthorizationReason;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let reason = value.trim();
        if reason.is_empty() {
            Err(EmptyOverlapAuthorizationReason)
        } else {
            Ok(Self(reason.to_owned()))
        }
    }
}

impl<'de> Deserialize<'de> for OverlapAuthorizationReason {
    fn deserialize<DeserializerType>(
        deserializer: DeserializerType,
    ) -> Result<Self, DeserializerType::Error>
    where
        DeserializerType: serde::Deserializer<'de>,
    {
        let reason = String::deserialize(deserializer)?;
        reason.parse().map_err(serde::de::Error::custom)
    }
}

impl From<bool> for OrderingReplacement {
    fn from(replace: bool) -> Self { if replace { Self::Replace } else { Self::Keep } }
}

impl PermissiveOverlapAnswer {
    /// Return every blocker identifier the answer flags named.
    pub(crate) fn blockers(&self) -> Vec<ReservationId> {
        match self {
            Self::Sequence { blockers, .. } => blockers
                .iter()
                .map(|sequenced_blocker| sequenced_blocker.blocker)
                .collect(),
            Self::Override { blocker } => vec![*blocker],
        }
    }
}

/// An error returned when an overlap authorization reason contains no text.
#[derive(Debug)]
pub(crate) struct EmptyOverlapAuthorizationReason;

impl Display for EmptyOverlapAuthorizationReason {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str("an overlap authorization reason cannot be empty")
    }
}

impl Error for EmptyOverlapAuthorizationReason {}

#[cfg(test)]
mod tests {
    use super::OverlapAuthorizationReason;

    #[test]
    fn overlap_authorization_reasons_reject_empty_deserialized_values() {
        assert!(serde_json::from_str::<OverlapAuthorizationReason>(r#"""#).is_err());
        assert!(serde_json::from_str::<OverlapAuthorizationReason>(r#""   ""#).is_err());
    }
}
