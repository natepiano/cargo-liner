//! Proposal creation, transport, and escalation material.

use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;
use std::str::FromStr;

use serde::Deserialize;
use serde::Serialize;

use super::scope_binding::AuthorizedOverlap;
use super::scope_binding::AuthorizedOverlapSet;
use crate::ids::CoordinationRunId;
use crate::ids::ReservationId;
use crate::ids::WorktreeId;
use crate::ledger::ClaimSource;
use crate::ledger::OrderingDirection;
use crate::ledger::ReservationPurpose;
use crate::reservation::ReservationConflict;
use crate::scope::ReservationScopeSet;

/// A claim's semantic overlap-answer state after CLI conversion.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum OverlapAuthorizationRequest {
    /// The claim did not attempt to authorize a conflict.
    Absent,
    /// The claim deliberately supplied a permissive answer and its reason.
    Permissive(Box<PermissiveOverlapAuthorizationRequest>),
}

/// A deliberate overlap answer with its inseparable authorization reason.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PermissiveOverlapAuthorizationRequest {
    /// The requested editing and integration behavior.
    pub(crate) answer:              PermissiveOverlapAnswer,
    /// Why the user accepted this overlap.
    pub(crate) reason:              OverlapAuthorizationReason,
    /// Whether the caller requests or spends an exact proposal.
    pub(crate) proposal_submission: OverlapProposalSubmission,
}

/// One of the three overlap answers that permits concurrent editing.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum PermissiveOverlapAnswer {
    /// Declare an integration order between requester and named blocker.
    Sequence {
        /// The blocker named by the caller.
        blocker:   ReservationId,
        /// Which endpoint must integrate first.
        direction: OrderingDirection,
    },
    /// Permit editing while holding both endpoints at integration.
    Defer {
        /// The blocker named by the caller.
        blocker: ReservationId,
    },
    /// Permit editing without adding an integration constraint.
    Override {
        /// The blocker named by the caller.
        blocker: ReservationId,
    },
}

/// Whether a permissive invocation issues a fresh proposal or applies an existing one.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum OverlapProposalSubmission {
    /// Recompute and return the proposal without appending a journal fact.
    Issue,
    /// Apply only when this token equals the recomputed locked proposal.
    Apply(Box<OverlapProposalToken>),
}

/// Why a user approved one specific overlap answer.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub(crate) struct OverlapAuthorizationReason(String);

/// The requester's coordination identity included in an overlap proposal.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "status",
    content = "coordination_run_id",
    rename_all = "snake_case"
)]
pub(crate) enum RequesterCoordinationIdentity {
    /// An argument, environment value, or honored marker identified the caller.
    Presented(CoordinationRunId),
    /// The caller presented no coordination identity.
    NotPresented,
}

/// The requester identity included in a proposal but not repeated in the journal answer.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct OverlapRequester {
    /// The coordination identity the caller actually presented.
    coordination_identity: RequesterCoordinationIdentity,
    /// The worktree in which the candidate claim would be minted.
    worktree_id:           WorktreeId,
    /// The candidate claim's work-plan or explicit provenance.
    source:                ClaimSource,
    /// Why the candidate paths are being protected.
    purpose:               ReservationPurpose,
}

/// The complete locked conflict observation to which an answer is bound.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct OverlapProposal {
    /// The actor whose candidate reservation does not exist yet.
    requester:            OverlapRequester,
    /// Why the user accepts this specific overlap answer.
    authorization_reason: OverlapAuthorizationReason,
    /// The candidate's normalized requested scopes.
    candidate_scopes:     ReservationScopeSet,
    /// The permissive answer the caller proposed.
    answer:               PermissiveOverlapAnswer,
    /// The sole holder, scope revision, and exact overlap observed under the lock.
    overlaps:             AuthorizedOverlapSet,
}

/// A command-line-safe token containing one complete overlap proposal.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct OverlapProposalToken(OverlapProposal);

/// The complete material returned before a permissive answer can be applied.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct OverlapEscalationPayload {
    /// Every holder and provenance record involved in the conflict.
    pub(crate) conflicts:            Vec<ReservationConflict>,
    /// The requested answer and ordering direction, if any.
    pub(crate) answer:               PermissiveOverlapAnswer,
    /// The user-supplied reason for accepting the overlap.
    pub(crate) authorization_reason: OverlapAuthorizationReason,
    /// The editing and integration effect of applying the answer.
    pub(crate) consequence:          OverlapAnswerConsequence,
    /// The exact current proposal material.
    proposal:                        OverlapProposal,
    /// The token required on the applying invocation.
    pub(crate) proposal_token:       OverlapProposalToken,
}

/// The integration effect attached to an overlap escalation.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum OverlapAnswerConsequence {
    /// Editing proceeds and the declared requester/holder order constrains integration.
    SequencedIntegration,
    /// Editing proceeds while both reservations remain held until an order exists.
    BothIntegrationsHeld,
    /// Editing and integration proceed without an ordering constraint.
    IntegrationUnconstrained,
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
            Self::Sequence { blocker, .. }
            | Self::Defer { blocker }
            | Self::Override { blocker } => *blocker,
        }
    }

    /// Return the integration consequence the escalation must state.
    pub(crate) const fn consequence(&self) -> OverlapAnswerConsequence {
        match self {
            Self::Sequence { .. } => OverlapAnswerConsequence::SequencedIntegration,
            Self::Defer { .. } => OverlapAnswerConsequence::BothIntegrationsHeld,
            Self::Override { .. } => OverlapAnswerConsequence::IntegrationUnconstrained,
        }
    }
}

impl OverlapRequester {
    /// Identify the caller and worktree without minting a reservation id.
    pub(crate) const fn new(
        coordination_identity: RequesterCoordinationIdentity,
        worktree_id: WorktreeId,
        source: ClaimSource,
        purpose: ReservationPurpose,
    ) -> Self {
        Self {
            coordination_identity,
            worktree_id,
            source,
            purpose,
        }
    }
}

impl OverlapProposal {
    /// Recompute proposal material from one locked replay's sole conflict.
    pub(crate) fn recompute(
        requester: OverlapRequester,
        authorization_reason: OverlapAuthorizationReason,
        candidate_scopes: &ReservationScopeSet,
        answer: PermissiveOverlapAnswer,
        conflict: &ReservationConflict,
    ) -> Self {
        let overlaps = AuthorizedOverlap::from(conflict).into();
        Self {
            requester,
            authorization_reason,
            candidate_scopes: candidate_scopes.clone(),
            answer,
            overlaps,
        }
    }

    /// Build the escalation material and its derived token.
    pub(crate) fn escalation(
        self,
        conflicts: Vec<ReservationConflict>,
    ) -> OverlapEscalationPayload {
        let answer = self.answer.clone();
        let authorization_reason = self.authorization_reason.clone();
        let consequence = answer.consequence();
        let proposal_token = self.clone().into();
        OverlapEscalationPayload {
            conflicts,
            answer,
            authorization_reason,
            consequence,
            proposal: self,
            proposal_token,
        }
    }

    pub(super) fn into_authorization_parts(
        self,
    ) -> (
        PermissiveOverlapAnswer,
        AuthorizedOverlapSet,
        OverlapAuthorizationReason,
    ) {
        (self.answer, self.overlaps, self.authorization_reason)
    }
}

impl From<OverlapProposal> for OverlapProposalToken {
    fn from(proposal: OverlapProposal) -> Self { Self(proposal) }
}

impl OverlapProposalToken {
    /// Return whether this token contains the current locked proposal.
    pub(crate) fn matches(&self, current: &OverlapProposal) -> bool { self.0 == *current }
}

impl Display for OverlapProposalToken {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        serde_json::to_string(&self.0)
            .map_err(|_| fmt::Error)
            .and_then(|proposal| formatter.write_str(&proposal))
    }
}

impl FromStr for OverlapProposalToken {
    type Err = InvalidOverlapProposalToken;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        serde_json::from_str(value)
            .map(Self)
            .map_err(InvalidOverlapProposalToken)
    }
}

impl Serialize for OverlapProposalToken {
    fn serialize<SerializerType>(
        &self,
        serializer: SerializerType,
    ) -> Result<SerializerType::Ok, SerializerType::Error>
    where
        SerializerType: serde::Serializer,
    {
        serde_json::to_string(&self.0)
            .map_err(serde::ser::Error::custom)?
            .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for OverlapProposalToken {
    fn deserialize<DeserializerType>(
        deserializer: DeserializerType,
    ) -> Result<Self, DeserializerType::Error>
    where
        DeserializerType: serde::Deserializer<'de>,
    {
        let token = String::deserialize(deserializer)?;
        token.parse().map_err(serde::de::Error::custom)
    }
}

impl Display for OverlapAnswerConsequence {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::SequencedIntegration => formatter.write_str(
                "editing proceeds on the shown scopes and integration enforces the selected order",
            ),
            Self::BothIntegrationsHeld => formatter.write_str(
                "editing proceeds on the shown scopes and both reservations remain held at integration until an order is declared",
            ),
            Self::IntegrationUnconstrained => formatter.write_str(
                "editing proceeds on the shown scopes without an integration-order constraint",
            ),
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

impl std::error::Error for EmptyOverlapAuthorizationReason {}

/// An error returned when a proposal token is not a serialized proposal.
#[derive(Debug)]
pub(crate) struct InvalidOverlapProposalToken(serde_json::Error);

impl Display for InvalidOverlapProposalToken {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid overlap proposal token: {}", self.0)
    }
}

impl std::error::Error for InvalidOverlapProposalToken {}

#[cfg(test)]
mod tests {
    use super::OverlapAuthorizationReason;

    #[test]
    fn overlap_authorization_reasons_reject_empty_deserialized_values() {
        assert!(serde_json::from_str::<OverlapAuthorizationReason>(r#"""#).is_err());
        assert!(serde_json::from_str::<OverlapAuthorizationReason>(r#""   ""#).is_err());
    }
}
