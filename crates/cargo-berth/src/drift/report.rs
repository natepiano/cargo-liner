//! The serialized drift report, its per-reservation results, and their effects.

use std::error::Error;
use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;

use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;

use super::identity::DriftScopeAcquisition;
use crate::ids::GitObjectId;
use crate::ids::ReservationId;
use crate::ids::ReservationScopePath;
use crate::ids::WireOrderedReservationIds;
use crate::ledger::CollisionPathSet;
use crate::ledger::ForeignReservationIdSet;
use crate::ledger::IncursionIncidentId;
use crate::ledger::IncursionPathSet;
use crate::ledger::ReservationScopeAdditionSet;
use crate::reservation::ReservationConflict;
use crate::scope::ReservationScopeSet;
use crate::verb::claim::FirstTouchReservationAcquisition;

/// The comparison algorithm that actually produced one report.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
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
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
pub(crate) struct DriftReport {
    /// The comparison that actually ran.
    pub(super) comparison:        DriftComparisonMode,
    /// How paths outside the acting run's reservations were attributed.
    #[serde(rename = "widening")]
    pub(crate) path_attribution:  DriftPathAttributionOutcome,
    /// One result for every selected reservation.
    pub(crate) results:           Vec<ReservationDriftResult>,
    /// Whether this invocation may still take or widen scopes in this worktree.
    ///
    /// Observation and classification never depended on this answer, so the report carries it
    /// beside them rather than in place of them: a refused run's report still states every
    /// consequence the commit had, and states the refusal too.
    pub(crate) scope_acquisition: DriftScopeAcquisition,
}

impl DriftReport {
    pub(super) fn unchanged(
        comparison: DriftComparisonMode,
        reservation_ids: &[ReservationId],
    ) -> Self {
        Self {
            comparison,
            path_attribution: DriftPathAttributionOutcome::NotNeeded,
            scope_acquisition: DriftScopeAcquisition::Permitted,
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
            .any(|result| !matches!(result, ReservationDriftResult::Unchanged { .. }))
    }

    /// Return every reservation selected by this comparison.
    pub(crate) fn reservation_ids(&self) -> Vec<ReservationId> {
        self.results
            .iter()
            .map(ReservationDriftResult::reservation_id)
            .collect()
    }

    /// Return every foreign reservation that blocked classification, once each.
    pub(crate) fn blocking_reservation_ids(&self) -> WireOrderedReservationIds {
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
        WireOrderedReservationIds::sorted_and_deduplicated(blocking)
    }
}

/// The outcome of attributing changed paths outside the acting run's reservations.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
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
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
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
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum ReservationDriftResult {
    /// No path requires a consequence because every changed path is already
    /// covered by a reservation with this reservation's run and worktree identity.
    Unchanged {
        /// The reservation compared with the observed paths.
        reservation_id: ReservationId,
    },
    /// Git could not read the phase-start object required for a safe comparison.
    PhaseStartObjectUnknown {
        /// The reservation whose baseline could not be read.
        reservation_id: ReservationId,
        /// The unreadable phase-start object.
        phase_start:    GitObjectId,
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
            Self::Unchanged { reservation_id }
            | Self::PhaseStartObjectUnknown { reservation_id, .. }
            | Self::Changed { reservation_id, .. } => *reservation_id,
        }
    }

    fn blocks(&self) -> bool {
        match self {
            Self::Unchanged { .. } => false,
            Self::PhaseStartObjectUnknown { .. } => true,
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
            Self::Unchanged { .. } | Self::PhaseStartObjectUnknown { .. } => Vec::new(),
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
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
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
        /// The commits that introduced the entered paths this phase committed.
        ///
        /// Empty for an incursion carrying only working-tree changes, where the write
        /// that caused it is the one the reader just made.
        commits:                 Vec<IncursionCommit>,
    },
    /// A path that was initially unheld gained a blocker before the widening lock.
    Collision {
        /// The reservations that prevented the locked widening.
        foreign_reservation_ids: ForeignReservationIdSet,
        /// The paths that could not be widened.
        paths:                   CollisionPathSet,
    },
}

/// One commit in a reservation's phase range that introduced entered paths.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
pub(crate) struct IncursionCommit {
    /// The commit that introduced the paths below.
    pub(crate) commit:  GitObjectId,
    /// The commit's subject line.
    pub(crate) subject: String,
    /// Whether this phase wrote the commit or received it.
    pub(crate) origin:  IncursionCommitOrigin,
    /// The entered paths this commit introduced.
    pub(crate) paths:   Vec<ReservationScopePath>,
}

declare_wire_enum! {
    /// Where a commit behind an entered path came from.
    #[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
    #[serde(rename_all = "snake_case")]
    pub(crate) enum IncursionCommitOrigin {
        /// Trunk does not carry the commit, so this phase authored it.
        PhaseAuthored => "phase_authored";
        /// Trunk already carried the commit, so the phase received it rather than wrote it.
        AlreadyOnTrunk => "already_on_trunk";
        /// Trunk could not be resolved, so the commit's origin was not decided.
        Unknown => "unknown";
    }
}

/// A non-empty set of consequences for one reservation.
#[derive(Clone, Debug, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(transparent)]
pub(crate) struct DriftEffectSet(#[schemars(length(min = 1))] Vec<DriftEffect>);

impl DriftEffectSet {
    /// Borrow the effects without weakening the non-empty construction boundary.
    pub(crate) fn as_slice(&self) -> &[DriftEffect] { &self.0 }

    /// Borrow the effects for in-place enrichment, which cannot empty the set.
    pub(super) fn as_mut_slice(&mut self) -> &mut [DriftEffect] { &mut self.0 }
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
        #[derive(Clone, Debug, Eq, JsonSchema, PartialEq, Serialize)]
        #[serde(transparent)]
        pub(crate) struct $name(#[schemars(length(min = 1))] Vec<$item>);

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

#[cfg(test)]
#[expect(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use serde_json::Value;
    use serde_json::json;

    use super::DriftReport;
    use crate::output::OutputEnvelope;

    const INCUMBENT_RESERVATION_ID: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1f";
    const INCUMBENT_RUN_ID: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1e";
    const ISSUING_RUN_ID: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a20";
    const PHASE_START: &str = "1111111111111111111111111111111111111111";
    const SUBJECT_RESERVATION_ID: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a21";
    const WORKTREE_ID: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1d";

    /// A refusal is the status even when a lower-ranked condition is also present.
    ///
    /// Both of those conditions name a remedy this same rule would refuse --- an ambiguous
    /// attribution says to rerun with `--reservation <id>`, an unreadable phase start says to
    /// rerun with `--full` --- so ranking either above the refusal hands a refused caller a
    /// command that cannot succeed. Neither combination arises from the claim path today,
    /// because the occupancy rule prevents the two presented runs from both holding `Active`
    /// reservations in one worktree, so the report is assembled directly: the ranking is the
    /// unit under test, not the ledger state that would have to produce it.
    ///
    /// `a_completed_but_refused_run_carries_its_own_status` in `tests/drift.rs` covers the
    /// other half: it drives a real refused run end to end, but passes under either ranking
    /// because its fixture attributes nothing. That one pins the status, this one the order.
    #[test]
    fn a_refusal_outranks_the_conditions_whose_remedies_it_would_refuse() {
        for (condition, report) in [
            (
                "ambiguous attribution",
                refused_report(&ambiguous_attribution(), &json!([])),
            ),
            (
                "unreadable phase start",
                refused_report(&json!({"status": "not_needed"}), &unreadable_phase_start()),
            ),
            (
                "both at once",
                refused_report(&ambiguous_attribution(), &unreadable_phase_start()),
            ),
        ] {
            let envelope = serde_json::to_value(OutputEnvelope::drift(report))
                .expect("a drift envelope should serialize");
            assert_eq!(
                envelope["status"], "scope_acquisition_refused",
                "the refusal must outrank {condition}: {envelope}"
            );
        }
    }

    /// The same lower-ranked conditions still decide the status when nothing was refused.
    ///
    /// Without this the test above would pass against a status that ignored the report
    /// entirely, so it pins that the ranking reorders live answers rather than suppressing
    /// them.
    #[test]
    fn the_outranked_conditions_still_decide_a_permitted_run() {
        let attribution_only = permitted_report(&ambiguous_attribution(), &json!([]));
        let envelope = serde_json::to_value(OutputEnvelope::drift(attribution_only))
            .expect("a drift envelope should serialize");
        assert_eq!(
            envelope["status"], "drift_attribution_required",
            "{envelope}"
        );

        let phase_start_only =
            permitted_report(&json!({"status": "not_needed"}), &unreadable_phase_start());
        let envelope = serde_json::to_value(OutputEnvelope::drift(phase_start_only))
            .expect("a drift envelope should serialize");
        assert_eq!(envelope["status"], "object_unknown", "{envelope}");
    }

    fn ambiguous_attribution() -> Value {
        json!({
            "status": "ambiguous",
            "candidates": [SUBJECT_RESERVATION_ID, INCUMBENT_RESERVATION_ID],
            "paths": ["src/lib.rs"],
        })
    }

    fn unreadable_phase_start() -> Value {
        json!([{
            "status": "phase_start_object_unknown",
            "reservation_id": SUBJECT_RESERVATION_ID,
            "phase_start": PHASE_START,
        }])
    }

    fn refused_report(path_attribution: &Value, results: &Value) -> DriftReport {
        report(
            path_attribution,
            results,
            &json!({
                "status": "refused_to_second_run",
                "rejection": {
                    "kind": "worktree_held_by_another_run",
                    "incumbent_coordination_run_id": INCUMBENT_RUN_ID,
                    "incumbent_reservation_id": INCUMBENT_RESERVATION_ID,
                    "issuing_coordination_run_id": ISSUING_RUN_ID,
                    "issuing_worktree_id": WORKTREE_ID,
                    "issuing_root": "/repo",
                    "recovery_actions": [{
                        "kind": "release_incumbent_reservation",
                        "argv": ["cargo-berth", "release", INCUMBENT_RESERVATION_ID, "--json"],
                        "cwd": "/repo",
                    }],
                },
            }),
        )
    }

    fn permitted_report(path_attribution: &Value, results: &Value) -> DriftReport {
        report(path_attribution, results, &json!({"status": "permitted"}))
    }

    fn report(path_attribution: &Value, results: &Value, scope_acquisition: &Value) -> DriftReport {
        serde_json::from_value(json!({
            "comparison": "cheap_delta",
            "widening": path_attribution,
            "results": results,
            "scope_acquisition": scope_acquisition,
        }))
        .expect("the drift report fixture should deserialize")
    }
}
