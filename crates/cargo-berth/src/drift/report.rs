//! The serialized drift report, its per-reservation results, and their effects.

use std::error::Error;
use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;

use serde::Deserialize;
use serde::Serialize;

use super::ordering;
use crate::ids::ReservationId;
use crate::ids::ReservationScopePath;
use crate::ledger::CollisionPathSet;
use crate::ledger::ForeignReservationIdSet;
use crate::ledger::IncursionIncidentId;
use crate::ledger::IncursionPathSet;
use crate::ledger::ReservationScopeAdditionSet;
use crate::reservation::ReservationConflict;
use crate::scope::ReservationScopeSet;
use crate::verb::claim::FirstTouchReservationAcquisition;

/// The comparison algorithm that actually produced one report.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum DriftComparisonMode {
    /// A valid cache enabled the two-command delta.
    CheapDelta,
    /// The caller selected the complete phase-start comparison.
    FullPhaseStart,
    /// An absent or unreadable cache required the complete comparison.
    FullPhaseStartFallback,
}

/// One complete drift report, possibly covering several commit-hook subjects.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct DriftReport {
    /// The comparison that actually ran.
    pub(super) comparison:       DriftComparisonMode,
    /// How paths outside the acting run's reservations were attributed.
    #[serde(rename = "widening")]
    pub(crate) path_attribution: DriftPathAttributionOutcome,
    /// One result for every selected reservation.
    pub(crate) results:          Vec<ReservationDriftResult>,
}

impl DriftReport {
    pub(super) fn unchanged(
        comparison: DriftComparisonMode,
        reservation_ids: &[ReservationId],
    ) -> Self {
        Self {
            comparison,
            path_attribution: DriftPathAttributionOutcome::NotNeeded,
            results: reservation_ids
                .iter()
                .map(|reservation_id| ReservationDriftResult::Unchanged {
                    reservation_id: *reservation_id,
                })
                .collect(),
        }
    }

    /// Return whether a blocking effect or unresolved attribution requires a stop.
    pub(crate) fn has_blocking_effect(&self) -> bool {
        matches!(
            self.path_attribution,
            DriftPathAttributionOutcome::Ambiguous { .. }
                | DriftPathAttributionOutcome::CoordinationRunRequired { .. }
                | DriftPathAttributionOutcome::IncursionDetected { .. }
        ) || self.results.iter().any(ReservationDriftResult::blocks)
    }

    /// Return whether this report has a drift effect or unresolved attribution to render.
    pub(crate) fn has_reportable_effect(&self) -> bool {
        matches!(
            self.path_attribution,
            DriftPathAttributionOutcome::FirstTouchReserved { .. }
                | DriftPathAttributionOutcome::IncursionDetected { .. }
                | DriftPathAttributionOutcome::Ambiguous { .. }
                | DriftPathAttributionOutcome::CoordinationRunRequired { .. }
        ) || self
            .results
            .iter()
            .any(|result| matches!(result, ReservationDriftResult::Changed { .. }))
    }

    /// Return every reservation selected by this comparison.
    pub(crate) fn reservation_ids(&self) -> Vec<ReservationId> {
        self.results
            .iter()
            .map(ReservationDriftResult::reservation_id)
            .collect()
    }

    /// Return every foreign reservation that blocked classification.
    pub(crate) fn blocking_reservation_ids(&self) -> Vec<ReservationId> {
        let mut blocking = self
            .results
            .iter()
            .flat_map(ReservationDriftResult::blocking_reservation_ids)
            .collect::<Vec<_>>();
        if let DriftPathAttributionOutcome::IncursionDetected { conflicts, .. } =
            &self.path_attribution
        {
            blocking.extend(conflicts.iter().map(|conflict| conflict.reservation_id));
        }
        ordering::sort_and_deduplicate_reservation_ids(&mut blocking);
        blocking
    }
}

/// The outcome of attributing changed paths outside the acting run's reservations.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum DriftPathAttributionOutcome {
    /// This observation found no unclaimed path requiring attribution.
    NotNeeded,
    /// One reservation was selected for the widening attempt.
    Attributed {
        /// The only reservation permitted to receive unclaimed paths.
        reservation_id: ReservationId,
    },
    /// No reservation existed, so the post-write observation acquired one.
    #[serde(rename = "first_touch_claimed")]
    FirstTouchReserved {
        /// The complete reservation identity, baseline, and publication results.
        acquisition: FirstTouchReservationAcquisition,
        /// The exact file scopes protected after the write.
        scopes:      ReservationScopeSet,
    },
    /// The write already happened in paths a foreign reservation held.
    #[serde(rename = "post_write_incursion")]
    IncursionDetected {
        /// The exact changed paths that could not be claimed after the write.
        paths:      UnattributedDriftPathSet,
        /// Every holder intersecting those changed paths.
        conflicts:  Vec<ReservationConflict>,
        /// Whether other paths from the same write received first-touch protection.
        protection: PostWriteFreePathProtection,
    },
    /// Several local reservations were candidates, so no widening was attempted.
    Ambiguous {
        /// Every active local reservation the caller may name explicitly.
        candidates: DriftAttributionCandidateSet,
        /// The exact paths left unassigned by this observation.
        paths:      UnattributedDriftPathSet,
    },
    /// No coordination run was identified, so no reservation can receive the paths.
    CoordinationRunRequired {
        /// The exact paths left unassigned by this observation.
        paths: UnattributedDriftPathSet,
    },
}

/// The protection result for free paths observed alongside a post-write incursion.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum PostWriteFreePathProtection {
    /// Every observed path had a foreign holder, so no reservation changed.
    NotAcquired,
    /// The free subset is now protected by this first-touch reservation.
    Acquired {
        /// The complete reservation identity, baseline, and publication results.
        acquisition: FirstTouchReservationAcquisition,
        /// The exact file scopes protected after the write.
        scopes:      ReservationScopeSet,
    },
}

/// The drift result for one selected reservation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum ReservationDriftResult {
    /// No path requires a consequence because every changed path is already
    /// covered by a reservation with this reservation's run and worktree identity.
    Unchanged {
        /// The reservation compared with the observed paths.
        reservation_id: ReservationId,
    },
    /// At least one durable or blocking consequence was found.
    Changed {
        /// The reservation receiving these consequences.
        reservation_id: ReservationId,
        /// Every distinct consequence for this reservation.
        effects:        DriftEffectSet,
    },
}

impl ReservationDriftResult {
    const fn reservation_id(&self) -> ReservationId {
        match self {
            Self::Unchanged { reservation_id } | Self::Changed { reservation_id, .. } => {
                *reservation_id
            },
        }
    }

    fn blocks(&self) -> bool {
        match self {
            Self::Unchanged { .. } => false,
            Self::Changed { effects, .. } => effects.as_slice().iter().any(|effect| {
                matches!(
                    effect,
                    DriftEffect::Incursion { .. } | DriftEffect::Collision { .. }
                )
            }),
        }
    }

    fn blocking_reservation_ids(&self) -> Vec<ReservationId> {
        match self {
            Self::Unchanged { .. } => Vec::new(),
            Self::Changed { effects, .. } => effects
                .as_slice()
                .iter()
                .flat_map(|effect| match effect {
                    DriftEffect::Incursion {
                        foreign_reservation_ids,
                        ..
                    }
                    | DriftEffect::Collision {
                        foreign_reservation_ids,
                        ..
                    } => foreign_reservation_ids.as_slice().to_vec(),
                    DriftEffect::Widened { .. } => Vec::new(),
                })
                .collect(),
        }
    }
}

/// One non-empty consequence of classifying observed paths.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum DriftEffect {
    /// Unreserved file paths were added to the reservation.
    Widened {
        /// The exact complete scopes appended to the journal.
        added_scopes: ReservationScopeAdditionSet,
    },
    /// Writes entered paths held by foreign edit-blocking reservations.
    Incursion {
        /// The durable incident identity carried by the journal record.
        incident_id:             IncursionIncidentId,
        /// The foreign holders named by the incursion record.
        foreign_reservation_ids: ForeignReservationIdSet,
        /// The exact paths entered.
        paths:                   IncursionPathSet,
    },
    /// A path that was initially unheld gained a blocker before the widening lock.
    Collision {
        /// The reservations that prevented the locked widening.
        foreign_reservation_ids: ForeignReservationIdSet,
        /// The paths that could not be widened.
        paths:                   CollisionPathSet,
    },
}

/// A non-empty set of consequences for one reservation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub(crate) struct DriftEffectSet(Vec<DriftEffect>);

impl DriftEffectSet {
    /// Borrow the effects without weakening the non-empty construction boundary.
    pub(crate) fn as_slice(&self) -> &[DriftEffect] { &self.0 }
}

impl TryFrom<Vec<DriftEffect>> for DriftEffectSet {
    type Error = EmptyDriftEffectSet;

    fn try_from(effects: Vec<DriftEffect>) -> Result<Self, Self::Error> {
        if effects.is_empty() {
            Err(EmptyDriftEffectSet)
        } else {
            Ok(Self(effects))
        }
    }
}

impl<'de> Deserialize<'de> for DriftEffectSet {
    fn deserialize<DeserializerType>(
        deserializer: DeserializerType,
    ) -> Result<Self, DeserializerType::Error>
    where
        DeserializerType: serde::Deserializer<'de>,
    {
        let effects = Vec::<DriftEffect>::deserialize(deserializer)?;
        Self::try_from(effects).map_err(serde::de::Error::custom)
    }
}

/// An error returned when a changed result contains no consequence.
#[derive(Debug)]
pub(crate) struct EmptyDriftEffectSet;

impl Display for EmptyDriftEffectSet {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str("a changed drift result must contain at least one effect")
    }
}

impl Error for EmptyDriftEffectSet {}

macro_rules! nonempty_drift_set {
    ($name:ident, $item:ty, $error:ident, $documentation:literal, $message:literal) => {
        #[doc = $documentation]
        #[derive(Clone, Debug, Eq, PartialEq, Serialize)]
        #[serde(transparent)]
        pub(crate) struct $name(Vec<$item>);

        impl $name {
            #[doc = concat!("Borrow the values in this `", stringify!($name), "`.")]
            pub(crate) fn as_slice(&self) -> &[$item] { &self.0 }
        }

        impl TryFrom<Vec<$item>> for $name {
            type Error = $error;

            fn try_from(values: Vec<$item>) -> Result<Self, Self::Error> {
                if values.is_empty() {
                    Err($error)
                } else {
                    Ok(Self(values))
                }
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<DeserializerType>(
                deserializer: DeserializerType,
            ) -> Result<Self, DeserializerType::Error>
            where
                DeserializerType: serde::Deserializer<'de>,
            {
                let values = Vec::<$item>::deserialize(deserializer)?;
                Self::try_from(values).map_err(serde::de::Error::custom)
            }
        }

        #[doc = concat!("An error returned when constructing an empty `", stringify!($name), "`.")]
        #[derive(Debug)]
        pub(crate) struct $error;

        impl Display for $error {
            fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
                formatter.write_str($message)
            }
        }

        impl std::error::Error for $error {}
    };
}

nonempty_drift_set!(
    DriftAttributionCandidateSet,
    ReservationId,
    EmptyDriftAttributionCandidateSet,
    "The non-empty reservation candidates for one ambiguous widening attribution.",
    "an ambiguous widening attribution must name at least one reservation"
);
nonempty_drift_set!(
    UnattributedDriftPathSet,
    ReservationScopePath,
    EmptyUnattributedDriftPathSet,
    "The non-empty path set left unassigned by an ambiguous widening attribution.",
    "an ambiguous widening attribution must name at least one path"
);
