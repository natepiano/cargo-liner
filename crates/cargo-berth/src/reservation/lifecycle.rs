//! Reservation progress and independently revalidated integration evidence.
//!
//! Checkpoint and release end a run's editing lifetime. Successful merge-extent emptiness
//! clears branch protection, and once a run has done work it also ends the run: reconciliation
//! checkpoints the run at a head trunk contains and settles it as integrated. An ended run with
//! an empty extent is retained only for audit.

use std::error::Error;
use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;
use std::str::FromStr;

use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;

use super::evidence::ProtectedReservationTip;
use crate::git::Reachability;
use crate::ids::GitObjectId;
use crate::ids::InvalidGitObjectId;

declare_wire_enum! {
    /// The git fact supporting an integrated evidence status.
    #[derive(Clone, Copy, Debug, Default, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
    #[schemars(rename = "integration_proof")]
    #[serde(rename_all = "snake_case")]
    pub(crate) enum IntegrationProof {
        /// Current trunk contains the protected commit itself.
        #[default]
        ProtectedTipAncestor => "protected_tip_ancestor";
        /// Current trunk contains the verified rewritten-integration witness.
        RewrittenWitnessAncestor => "rewritten_witness_ancestor";
        /// Current trunk contains every protected scoped patch under an equivalent commit identity.
        ScopedPatchEquivalent => "scoped_patch_equivalent";
    }
}

/// How far a reservation has progressed through the coordination protocol.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(tag = "stage", rename_all = "snake_case")]
pub(crate) enum ReservationLifecycle {
    /// The holder may still change the reserved paths.
    Active,
    /// The holder checkpointed a fixed commit that has not received a disposition.
    Outstanding {
        /// The fixed commit used for every ordinary integration query.
        protected_tip: ProtectedReservationTip,
    },
    /// A verified or user-confirmed disposition ended the work.
    Released {
        /// The evidence or decision that ended the work.
        disposition: ReleaseDisposition,
    },
}

impl ReservationLifecycle {
    /// Move an active reservation to its first protected checkpoint.
    pub(crate) fn checkpoint(
        &mut self,
        protected_tip: ProtectedReservationTip,
    ) -> Result<(), LifecycleTransitionError> {
        match self {
            Self::Active => {
                *self = Self::Outstanding { protected_tip };
                Ok(())
            },
            Self::Outstanding { .. } | Self::Released { .. } => {
                Err(LifecycleTransitionError::CheckpointRequiresActive)
            },
        }
    }

    /// Replace the protected commit after an outstanding reservation is rebased.
    pub(crate) fn resnapshot(
        &mut self,
        protected_tip: ProtectedReservationTip,
    ) -> Result<(), LifecycleTransitionError> {
        match self {
            Self::Outstanding {
                protected_tip: current_tip,
            } => {
                *current_tip = protected_tip;
                Ok(())
            },
            Self::Active | Self::Released { .. } => {
                Err(LifecycleTransitionError::ResnapshotRequiresOutstanding)
            },
        }
    }

    /// Record a terminal disposition while retaining integration evidence.
    pub(crate) fn release(
        &mut self,
        disposition: ReleaseDisposition,
    ) -> Result<(), LifecycleTransitionError> {
        match self {
            Self::Outstanding { .. } => {
                *self = Self::Released { disposition };
                Ok(())
            },
            Self::Active => Err(LifecycleTransitionError::ReleaseRequiresCheckpoint),
            Self::Released { .. } => Err(LifecycleTransitionError::AlreadyReleased),
        }
    }

    /// Record an abandonment or orphan retirement after explicit user confirmation.
    pub(crate) fn release_after_user_confirmation(
        &mut self,
        disposition: ReleaseDisposition,
    ) -> Result<(), LifecycleTransitionError> {
        if !matches!(
            disposition,
            ReleaseDisposition::Abandoned(_) | ReleaseDisposition::RetiredOrphan(_)
        ) {
            return Err(LifecycleTransitionError::ReleaseRequiresCheckpoint);
        }
        match self {
            Self::Active | Self::Outstanding { .. } => {
                *self = Self::Released { disposition };
                Ok(())
            },
            Self::Released { .. } => Err(LifecycleTransitionError::AlreadyReleased),
        }
    }

    /// Replace invalidated git-backed release evidence without erasing its history.
    pub(crate) fn replace_release_disposition(
        &mut self,
        superseded: &ReleaseDisposition,
        replacement: ReleaseDisposition,
    ) -> Result<(), LifecycleTransitionError> {
        match self {
            Self::Released { disposition } if disposition == superseded => {
                *disposition = replacement;
                Ok(())
            },
            Self::Released { .. } => Err(LifecycleTransitionError::SupersededDispositionMismatch),
            Self::Active | Self::Outstanding { .. } => {
                Err(LifecycleTransitionError::ReplacementRequiresRelease)
            },
        }
    }
}

/// Which trunk commit witnesses the protected work within an evaluated trunk.
#[derive(Clone, Debug, Default, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[schemars(rename = "integration_witness")]
#[serde(tag = "kind", content = "commit", rename_all = "snake_case")]
pub(crate) enum IntegrationWitness {
    /// The evaluated trunk itself witnesses integration.
    #[default]
    EvaluatedTrunk,
    /// An earlier verified trunk commit witnesses rewritten integration.
    Historical(
        #[schemars(with = "String")]
        #[schemars(length(min = 1))]
        RewrittenIntegrationTrunkCommit,
    ),
}

impl IntegrationWitness {
    /// Resolve the witness identity against the trunk used for this evaluation.
    pub(crate) fn resolve(&self, evaluated_trunk: &GitObjectId) -> RewrittenIntegrationTrunkCommit {
        match self {
            Self::EvaluatedTrunk => RewrittenIntegrationTrunkCommit::from(evaluated_trunk.clone()),
            Self::Historical(commit) => commit.clone(),
        }
    }
}

/// What the current trunk proves about retained reservation evidence.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[schemars(rename = "integration_evidence_status")]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum IntegrationEvidenceStatus {
    /// The protected commit is not reachable from current trunk.
    NotIntegrated,
    /// Current trunk contains the protected integration evidence.
    Integrated {
        /// The current trunk commit that was checked.
        #[schemars(with = "String")]
        #[schemars(length(min = 1))]
        trunk_oid: GitObjectId,
        /// The git fact that established integration.
        #[serde(default)]
        proof:     IntegrationProof,
        /// The commit witnessing integration within the evaluated trunk.
        #[serde(default)]
        witness:   IntegrationWitness,
    },
    /// Trunk no longer contains evidence that was previously verified.
    TrunkRewritten,
    /// Git could not resolve the object needed for the reachability query.
    ObjectUnknown,
}

#[cfg(test)]
impl IntegrationEvidenceStatus {
    /// Reproduce the historical checkpoint-only decision for existing journal fixtures.
    pub(crate) const fn edit_blocking_status(&self) -> EditBlockingStatus {
        match self {
            Self::Integrated { .. } => EditBlockingStatus::Clear,
            Self::NotIntegrated | Self::TrunkRewritten | Self::ObjectUnknown => {
                EditBlockingStatus::Blocking
            },
        }
    }
}

declare_wire_enum! {
    /// The effective edit decision derived from reservation lifecycle and integration evidence.
    #[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
    #[serde(rename_all = "snake_case")]
    pub(crate) enum EditBlockingStatus {
        /// The reservation still blocks foreign edits.
        Blocking => "blocking";
        /// The reservation is released or its outstanding evidence permits foreign edits.
        Clear => "clear";
    }
}

/// A verified or user-confirmed terminal reservation outcome.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[schemars(rename = "release_disposition")]
#[serde(tag = "kind", content = "evidence", rename_all = "snake_case")]
pub(crate) enum ReleaseDisposition {
    /// Git proved the protected work reached trunk.
    Integrated,
    /// An alternate trunk commit witnesses verified rewritten integration.
    RewrittenIntegration(
        #[schemars(with = "String")]
        #[schemars(length(min = 1))]
        RewrittenIntegrationTrunkCommit,
    ),
    /// The user deliberately discarded the reservation's work.
    Abandoned(
        #[schemars(with = "String")]
        #[schemars(length(min = 1))]
        AbandonmentReason,
    ),
    /// The user confirmed an orphaned reservation can retire.
    RetiredOrphan(
        #[schemars(with = "String")]
        #[schemars(length(min = 1))]
        OrphanRetirementReason,
    ),
}

impl ReleaseDisposition {
    /// Return whether future trunk checks may invalidate this disposition.
    pub(crate) const fn revalidation_subject(&self) -> ReleaseRevalidationSubject<'_> {
        match self {
            Self::Integrated => ReleaseRevalidationSubject::ProtectedTip,
            Self::RewrittenIntegration(trunk_commit) => {
                ReleaseRevalidationSubject::RewrittenIntegration(trunk_commit)
            },
            Self::Abandoned(_) | Self::RetiredOrphan(_) => ReleaseRevalidationSubject::None,
        }
    }
}

/// The commit role, if any, that a released reservation must revalidate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ReleaseRevalidationSubject<'reservation> {
    /// Ordinary integration continues to use the retained protected tip.
    ProtectedTip,
    /// Rewritten integration requires ancestry of its verified trunk witness.
    RewrittenIntegration(&'reservation RewrittenIntegrationTrunkCommit),
    /// A deliberate retirement has no future git evidence to revalidate.
    None,
}

macro_rules! nonempty_release_reason {
    ($name:ident, $error:ident, $documentation:literal, $message:literal) => {
        #[doc = $documentation]
        #[derive(Clone, Debug, Eq, PartialEq)]
        pub(crate) struct $name(String);

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }

        impl FromStr for $name {
            type Err = $error;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                let value = value.trim();
                if value.is_empty() {
                    Err($error)
                } else {
                    Ok(Self(value.to_owned()))
                }
            }
        }

        impl Serialize for $name {
            fn serialize<SerializerType>(
                &self,
                serializer: SerializerType,
            ) -> Result<SerializerType::Ok, SerializerType::Error>
            where
                SerializerType: serde::Serializer,
            {
                serializer.serialize_str(&self.0)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<DeserializerType>(
                deserializer: DeserializerType,
            ) -> Result<Self, DeserializerType::Error>
            where
                DeserializerType: serde::Deserializer<'de>,
            {
                let value = String::deserialize(deserializer)?;
                value.parse().map_err(serde::de::Error::custom)
            }
        }

        #[doc = concat!("An error returned when `", stringify!($name), "` is empty.")]
        #[derive(Debug)]
        pub(crate) struct $error;

        impl fmt::Display for $error {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str($message)
            }
        }

        impl std::error::Error for $error {}
    };
}

nonempty_release_reason!(
    AbandonmentReason,
    EmptyAbandonmentReason,
    "The required explanation for a user-confirmed abandonment.",
    "an abandonment reason cannot be empty"
);
nonempty_release_reason!(
    OrphanRetirementReason,
    EmptyOrphanRetirementReason,
    "The required explanation for a user-confirmed orphan retirement.",
    "an orphan-retirement reason cannot be empty"
);

/// The verified trunk commit witnessing rewritten integration.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub(crate) struct RewrittenIntegrationTrunkCommit(GitObjectId);

impl RewrittenIntegrationTrunkCommit {
    /// Revalidate this witness by ancestry alone, without replaying the original phase.
    pub(crate) fn revalidate_ancestry(
        &self,
        trunk: &GitObjectId,
        reachability: impl FnOnce(&GitObjectId) -> Reachability,
    ) -> IntegrationEvidenceStatus {
        match reachability(self.as_ref()) {
            Reachability::Ancestor => IntegrationEvidenceStatus::Integrated {
                trunk_oid: trunk.clone(),
                proof:     IntegrationProof::RewrittenWitnessAncestor,
                witness:   IntegrationWitness::Historical(self.clone()),
            },
            Reachability::NotAncestor => IntegrationEvidenceStatus::TrunkRewritten,
            Reachability::ObjectUnknown => IntegrationEvidenceStatus::ObjectUnknown,
        }
    }
}

impl From<GitObjectId> for RewrittenIntegrationTrunkCommit {
    fn from(git_object_id: GitObjectId) -> Self { Self(git_object_id) }
}

impl AsRef<GitObjectId> for RewrittenIntegrationTrunkCommit {
    fn as_ref(&self) -> &GitObjectId { &self.0 }
}

impl FromStr for RewrittenIntegrationTrunkCommit {
    type Err = InvalidGitObjectId;

    fn from_str(value: &str) -> Result<Self, Self::Err> { value.parse::<GitObjectId>().map(Self) }
}

/// A journal sequence requested a lifecycle transition from the wrong state.
#[derive(Debug)]
pub(crate) enum LifecycleTransitionError {
    /// A checkpoint operation named a reservation that was not active.
    CheckpointRequiresActive,
    /// A resnapshot operation named a reservation that was not outstanding.
    ResnapshotRequiresOutstanding,
    /// A release operation named a reservation without a protected checkpoint.
    ReleaseRequiresCheckpoint,
    /// A second terminal disposition named an already released reservation.
    AlreadyReleased,
    /// A replacement record did not name the disposition currently retained by replay.
    SupersededDispositionMismatch,
    /// A replacement disposition named a reservation without an earlier release.
    ReplacementRequiresRelease,
}

impl Display for LifecycleTransitionError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::CheckpointRequiresActive => {
                formatter.write_str("checkpoint requires an active reservation")
            },
            Self::ResnapshotRequiresOutstanding => {
                formatter.write_str("resnapshot requires an outstanding reservation")
            },
            Self::ReleaseRequiresCheckpoint => {
                formatter.write_str("release requires a checkpointed reservation")
            },
            Self::AlreadyReleased => formatter.write_str("reservation is already released"),
            Self::SupersededDispositionMismatch => {
                formatter.write_str("superseded disposition does not match the current release")
            },
            Self::ReplacementRequiresRelease => {
                formatter.write_str("replacement disposition requires a released reservation")
            },
        }
    }
}

impl Error for LifecycleTransitionError {}

#[cfg(test)]
mod tests {
    use super::IntegrationEvidenceStatus;
    use super::IntegrationProof;
    use super::IntegrationWitness;
    use super::ReleaseDisposition;
    use super::ReleaseRevalidationSubject;
    use super::RewrittenIntegrationTrunkCommit;
    use crate::git::Reachability;
    use crate::ids::GitObjectId;

    #[test]
    fn revalidation_subject_selection() -> Result<(), Box<dyn std::error::Error>> {
        let witness = "1111111111111111111111111111111111111111".parse::<GitObjectId>()?;
        let trunk = "2222222222222222222222222222222222222222".parse::<GitObjectId>()?;
        let disposition = ReleaseDisposition::RewrittenIntegration(
            RewrittenIntegrationTrunkCommit::from(witness.clone()),
        );
        assert_eq!(
            ReleaseDisposition::Integrated.revalidation_subject(),
            ReleaseRevalidationSubject::ProtectedTip
        );
        let ReleaseRevalidationSubject::RewrittenIntegration(subject) =
            disposition.revalidation_subject()
        else {
            return Err("rewritten integration must select the trunk witness".into());
        };
        for (reachability, expected) in [
            (
                Reachability::Ancestor,
                IntegrationEvidenceStatus::Integrated {
                    trunk_oid: trunk.clone(),
                    proof:     IntegrationProof::RewrittenWitnessAncestor,
                    witness:   IntegrationWitness::Historical(subject.clone()),
                },
            ),
            (
                Reachability::NotAncestor,
                IntegrationEvidenceStatus::TrunkRewritten,
            ),
            (
                Reachability::ObjectUnknown,
                IntegrationEvidenceStatus::ObjectUnknown,
            ),
        ] {
            let mut ancestry_queries = 0;
            let status = subject.revalidate_ancestry(&trunk, |commit| {
                assert_eq!(commit, &witness);
                ancestry_queries += 1;
                reachability
            });
            assert_eq!(status, expected);
            assert_eq!(ancestry_queries, 1);
        }
        Ok(())
    }
}
