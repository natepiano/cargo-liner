//! Protected commit roles and git-backed integration evidence.

use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;
use std::path::Path;
use std::str::FromStr;

use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;

use super::lifecycle::IntegrationEvidenceStatus;
use super::lifecycle::IntegrationProof;
use crate::git;
use crate::git::GitError;
use crate::git::Reachability;
use crate::git::ScopedPatchComparison;
use crate::ids::GitObjectId;
use crate::ids::InvalidGitObjectId;
use crate::ids::ReservationId;
use crate::ledger::ProtectedPhaseStartHead;
use crate::scope::ReservationScopeSet;

/// The fixed checkpoint commit used for ordinary integration evidence.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[schemars(rename = "protected_reservation_tip")]
#[schemars(transparent)]
#[serde(transparent)]
pub(crate) struct ProtectedReservationTip(#[schemars(with = "String")] GitObjectId);

impl Display for ProtectedReservationTip {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result { self.0.fmt(formatter) }
}

impl From<GitObjectId> for ProtectedReservationTip {
    fn from(git_object_id: GitObjectId) -> Self { Self(git_object_id) }
}

impl AsRef<GitObjectId> for ProtectedReservationTip {
    fn as_ref(&self) -> &GitObjectId { &self.0 }
}

impl FromStr for ProtectedReservationTip {
    type Err = InvalidGitObjectId;

    fn from_str(value: &str) -> Result<Self, Self::Err> { value.parse::<GitObjectId>().map(Self) }
}

/// Whether this reservation previously had verified integration evidence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PriorIntegrationStatus {
    /// No earlier stateful check proved integration.
    Unproven,
    /// An earlier stateful check proved integration.
    Proven,
}

/// Whether reconciliation observed the integration status without or through a scoped comparison.
pub(crate) enum IntegrationEvidenceObservation {
    /// Reachability alone produced the status.
    Reachability(IntegrationEvidenceStatus),
    /// A scoped patch comparison contributed to the status.
    ScopedPatchComparison(IntegrationEvidenceStatus),
    /// The bounded comparison was not run after reachability rejected the protected-tip proof.
    ScopedPatchComparisonDeferred(DeferredScopedPatchIntegrationStatus),
}

/// The validity of materialized evidence when a scoped patch comparison is deferred.
pub(crate) enum DeferredScopedPatchIntegrationStatus {
    /// The materialized status still applies to the observed trunk.
    StillValid(IntegrationEvidenceStatus),
    /// A refuted or stale affirmative proof was replaced with non-affirmative evidence.
    Degraded(IntegrationEvidenceStatus),
}

impl DeferredScopedPatchIntegrationStatus {
    fn from_materialized(
        materialized: &IntegrationEvidenceStatus,
        observed_trunk_oid: &GitObjectId,
    ) -> Self {
        match materialized {
            IntegrationEvidenceStatus::Integrated {
                trunk_oid,
                proof: IntegrationProof::ScopedPatchEquivalent,
            } if trunk_oid == observed_trunk_oid => Self::StillValid(materialized.clone()),
            IntegrationEvidenceStatus::Integrated {
                proof:
                    IntegrationProof::ProtectedTipAncestor | IntegrationProof::ScopedPatchEquivalent,
                ..
            } => Self::Degraded(IntegrationEvidenceStatus::NotIntegrated),
            IntegrationEvidenceStatus::NotIntegrated
            | IntegrationEvidenceStatus::TrunkRewritten
            | IntegrationEvidenceStatus::ObjectUnknown => Self::StillValid(materialized.clone()),
        }
    }
}

/// Whether the bounded reconciliation slot supplied a scoped patch comparison.
pub(crate) enum ScopedPatchComparisonObservation {
    /// Git produced this comparison during the current reconciliation.
    Observed(ScopedPatchComparison),
    /// Another proof subject received the target's comparison slot.
    Deferred,
}

/// Read the full commit currently named by `HEAD`.
pub(crate) fn current_head(repository_root: &Path) -> Result<GitObjectId, GitError> {
    git::head_object_id(repository_root)
}

/// Read the full commit currently named by the configured trunk branch.
pub(crate) fn current_trunk(
    repository_root: &Path,
    trunk_branch: &str,
) -> Result<GitObjectId, GitError> {
    git::branch_object_id(repository_root, trunk_branch)
}

/// Revalidate a protected tip against current trunk.
pub(crate) fn integration_status(
    repository_root: &Path,
    phase_start_head: &ProtectedPhaseStartHead,
    scopes: &ReservationScopeSet,
    protected_tip: &ProtectedReservationTip,
    trunk_oid: &GitObjectId,
    prior_integration_status: PriorIntegrationStatus,
) -> Result<IntegrationEvidenceStatus, GitError> {
    match git::reachability(repository_root, protected_tip.as_ref(), trunk_oid)? {
        Reachability::Ancestor => Ok(IntegrationEvidenceStatus::Integrated {
            trunk_oid: trunk_oid.clone(),
            proof:     IntegrationProof::ProtectedTipAncestor,
        }),
        Reachability::NotAncestor => match git::scoped_patch_equivalence(
            repository_root,
            phase_start_head.as_ref(),
            scopes,
            protected_tip.as_ref(),
            trunk_oid,
        )? {
            ScopedPatchComparison::Equivalent => Ok(IntegrationEvidenceStatus::Integrated {
                trunk_oid: trunk_oid.clone(),
                proof:     IntegrationProof::ScopedPatchEquivalent,
            }),
            ScopedPatchComparison::Different => match prior_integration_status {
                PriorIntegrationStatus::Unproven => Ok(IntegrationEvidenceStatus::NotIntegrated),
                PriorIntegrationStatus::Proven => Ok(IntegrationEvidenceStatus::TrunkRewritten),
            },
            ScopedPatchComparison::Unavailable => Ok(IntegrationEvidenceStatus::ObjectUnknown),
        },
        Reachability::ObjectUnknown => Ok(IntegrationEvidenceStatus::ObjectUnknown),
    }
}

/// Revalidate an outstanding tip and distinguish trunk replacement from ordinary non-integration.
pub(crate) fn outstanding_integration_status(
    repository_root: &Path,
    phase_start_head: &ProtectedPhaseStartHead,
    scopes: &ReservationScopeSet,
    protected_tip: &ProtectedReservationTip,
    previous_trunk_oid: &GitObjectId,
    current_trunk_oid: &GitObjectId,
) -> Result<IntegrationEvidenceStatus, GitError> {
    let status = integration_status(
        repository_root,
        phase_start_head,
        scopes,
        protected_tip,
        current_trunk_oid,
        PriorIntegrationStatus::Unproven,
    )?;
    if !matches!(status, IntegrationEvidenceStatus::NotIntegrated) {
        return Ok(status);
    }
    match git::reachability(repository_root, previous_trunk_oid, current_trunk_oid)? {
        Reachability::Ancestor => Ok(IntegrationEvidenceStatus::NotIntegrated),
        Reachability::NotAncestor => Ok(IntegrationEvidenceStatus::TrunkRewritten),
        Reachability::ObjectUnknown => Ok(IntegrationEvidenceStatus::ObjectUnknown),
    }
}

/// Observe integration while allowing reconciliation to defer only the scoped comparison.
pub(crate) fn observe_integration_status(
    protected_tip_reachability: Reachability,
    trunk_oid: &GitObjectId,
    prior_integration_status: PriorIntegrationStatus,
    materialized: &IntegrationEvidenceStatus,
    observe_scoped_patch_comparison: impl FnOnce() -> ScopedPatchComparisonObservation,
) -> IntegrationEvidenceObservation {
    match protected_tip_reachability {
        Reachability::Ancestor => {
            IntegrationEvidenceObservation::Reachability(IntegrationEvidenceStatus::Integrated {
                trunk_oid: trunk_oid.clone(),
                proof:     IntegrationProof::ProtectedTipAncestor,
            })
        },
        Reachability::NotAncestor => match observe_scoped_patch_comparison() {
            ScopedPatchComparisonObservation::Observed(scoped_patch_comparison) => {
                IntegrationEvidenceObservation::ScopedPatchComparison(
                    status_from_scoped_patch_comparison(
                        scoped_patch_comparison,
                        trunk_oid,
                        prior_integration_status,
                    ),
                )
            },
            ScopedPatchComparisonObservation::Deferred => {
                IntegrationEvidenceObservation::ScopedPatchComparisonDeferred(
                    DeferredScopedPatchIntegrationStatus::from_materialized(
                        materialized,
                        trunk_oid,
                    ),
                )
            },
        },
        Reachability::ObjectUnknown => {
            IntegrationEvidenceObservation::Reachability(IntegrationEvidenceStatus::ObjectUnknown)
        },
    }
}

/// Observe outstanding integration while deferring only its scoped comparison when bounded.
pub(crate) fn observe_outstanding_integration_status(
    protected_tip_reachability: Reachability,
    previous_trunk_reachability: Reachability,
    current_trunk_oid: &GitObjectId,
    materialized: &IntegrationEvidenceStatus,
    observe_scoped_patch_comparison: impl FnOnce() -> ScopedPatchComparisonObservation,
) -> IntegrationEvidenceObservation {
    let observation = observe_integration_status(
        protected_tip_reachability,
        current_trunk_oid,
        PriorIntegrationStatus::Unproven,
        materialized,
        observe_scoped_patch_comparison,
    );
    let IntegrationEvidenceObservation::ScopedPatchComparison(
        IntegrationEvidenceStatus::NotIntegrated,
    ) = observation
    else {
        return observation;
    };
    let status = match previous_trunk_reachability {
        Reachability::Ancestor => IntegrationEvidenceStatus::NotIntegrated,
        Reachability::NotAncestor => IntegrationEvidenceStatus::TrunkRewritten,
        Reachability::ObjectUnknown => IntegrationEvidenceStatus::ObjectUnknown,
    };
    IntegrationEvidenceObservation::ScopedPatchComparison(status)
}

fn status_from_scoped_patch_comparison(
    scoped_patch_comparison: ScopedPatchComparison,
    trunk_oid: &GitObjectId,
    prior_integration_status: PriorIntegrationStatus,
) -> IntegrationEvidenceStatus {
    match scoped_patch_comparison {
        ScopedPatchComparison::Equivalent => IntegrationEvidenceStatus::Integrated {
            trunk_oid: trunk_oid.clone(),
            proof:     IntegrationProof::ScopedPatchEquivalent,
        },
        ScopedPatchComparison::Different => match prior_integration_status {
            PriorIntegrationStatus::Unproven => IntegrationEvidenceStatus::NotIntegrated,
            PriorIntegrationStatus::Proven => IntegrationEvidenceStatus::TrunkRewritten,
        },
        ScopedPatchComparison::Unavailable => IntegrationEvidenceStatus::ObjectUnknown,
    }
}

/// Create or update the ref that keeps a protected tip reachable.
pub(crate) fn retain_protected_tip(
    repository_root: &Path,
    reservation_id: ReservationId,
    protected_tip: &ProtectedReservationTip,
) -> Result<(), GitError> {
    git::write_reservation_retention_ref(repository_root, reservation_id, protected_tip.as_ref())
}
