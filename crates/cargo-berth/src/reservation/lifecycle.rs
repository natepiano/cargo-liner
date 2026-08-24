//! Reservation progress and independently revalidated integration evidence.

use std::error::Error;
use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;
use std::str::FromStr;

use serde::Deserialize;
use serde::Serialize;

use super::evidence::ProtectedReservationTip;
use crate::ids::GitObjectId;
use crate::ids::InvalidGitObjectId;

/// How far a reservation has progressed through the coordination protocol.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
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
            Self::Released { disposition } => match disposition.revalidation_subject() {
                ReleaseRevalidationSubject::ProtectedTip
                | ReleaseRevalidationSubject::RewrittenIntegration(_) => {
                    *self = Self::Outstanding { protected_tip };
                    Ok(())
                },
                ReleaseRevalidationSubject::None => {
                    Err(LifecycleTransitionError::ResnapshotRequiresGitEvidence)
                },
            },
            Self::Active => Err(LifecycleTransitionError::ResnapshotRequiresGitEvidence),
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

/// What the current trunk proves about retained reservation evidence.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum IntegrationEvidenceStatus {
    /// The protected commit is not reachable from current trunk.
    NotIntegrated,
    /// Current trunk contains the protected integration evidence.
    Integrated {
        /// The current trunk commit that was checked.
        trunk_oid: GitObjectId,
    },
    /// Trunk no longer contains evidence that was previously verified.
    TrunkRewritten,
    /// Git could not resolve the object needed for the reachability query.
    ObjectUnknown,
}

impl IntegrationEvidenceStatus {
    /// Convert this point-in-time result into the state consumed by edit checks.
    pub(crate) const fn edit_blocking_status(&self) -> EditBlockingStatus {
        match self {
            Self::Integrated { .. } => EditBlockingStatus::Clear,
            Self::NotIntegrated | Self::TrunkRewritten | Self::ObjectUnknown => {
                EditBlockingStatus::Blocking
            },
        }
    }
}

/// The journaled edit decision consumed without executing git.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum EditBlockingStatus {
    /// The reservation still blocks foreign edits.
    Blocking,
    /// Current materialized evidence permits foreign edits.
    Clear,
}

/// A user-confirmed terminal reservation outcome.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", content = "evidence", rename_all = "snake_case")]
pub(crate) enum ReleaseDisposition {
    /// Git proved the protected work reached trunk.
    Integrated,
    /// The user supplied a verified alternate trunk commit.
    RewrittenIntegration(RewrittenIntegrationTrunkCommit),
    /// The user deliberately discarded the reservation's work.
    Abandoned(AbandonmentReason),
    /// The user confirmed an orphaned reservation can retire.
    RetiredOrphan(OrphanRetirementReason),
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
    /// Rewritten integration uses the user-verified trunk commit.
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

/// The verified trunk commit supplied for rewritten integration.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub(crate) struct RewrittenIntegrationTrunkCommit(GitObjectId);

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
    ResnapshotRequiresGitEvidence,
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
            Self::ResnapshotRequiresGitEvidence => {
                formatter.write_str("resnapshot requires retained git integration evidence")
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
