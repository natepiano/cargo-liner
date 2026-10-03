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

/// One of the overlap answers that permits concurrent editing and reserves the claimed paths.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PermissiveOverlapAnswer {
    /// Declare an integration order between requester and named blocker.
    Sequence {
        /// The blocker named by the caller.
        blocker:   ReservationId,
        /// Which endpoint must integrate first.
        direction: OrderingDirection,
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

impl PermissiveOverlapAnswer {
    /// Return the blocker identifier named by the answer flag.
    pub(crate) const fn blocker(&self) -> ReservationId {
        match self {
            Self::Sequence { blocker, .. } | Self::Override { blocker } => *blocker,
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
