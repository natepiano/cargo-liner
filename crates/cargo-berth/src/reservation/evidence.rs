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
use super::lifecycle::IntegrationWitness;
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
pub(crate) struct ProtectedReservationTip(
    #[schemars(with = "String")]
    #[schemars(length(min = 1))]
    GitObjectId,
);

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
    ScopedPatchComparison {
        /// Status evaluated against the observed trunk.
        status:     IntegrationEvidenceStatus,
        /// Certified witness and whether a negative remains retryable.
        evaluation: ScopedPatchIntegrationEvaluation,
    },
    /// The bounded comparison did not run, so nothing this pass judged the materialized status.
    ///
    /// Reaching here means reachability answered `NotAncestor` for the protected tip, which is not
    /// a verdict on the reservation's content -- only the comparison that was skipped could reach
    /// one. The materialized status therefore stands untouched, and the reservation waits for a
    /// pass whose budget admits its comparison.
    ScopedPatchComparisonDeferred,
}

/// The semantic result of one admitted scoped integration evaluation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ScopedPatchIntegrationEvaluation {
    /// Replay certifies the entire phase at this integration witness.
    Equivalent(IntegrationWitness),
    /// Discovery and replay definitively reject integration.
    Different,
    /// Current-trunk replay differs, but unavailable historical evidence needs a retry.
    HistoricalEvidenceUnavailable,
    /// Scoped replay could not be completed.
    Unavailable,
}

impl From<ScopedPatchComparison> for ScopedPatchIntegrationEvaluation {
    fn from(comparison: ScopedPatchComparison) -> Self {
        match comparison {
            ScopedPatchComparison::Equivalent => {
                Self::Equivalent(IntegrationWitness::EvaluatedTrunk)
            },
            ScopedPatchComparison::Different => Self::Different,
            ScopedPatchComparison::Unavailable => Self::Unavailable,
        }
    }
}

/// Whether the bounded reconciliation slot supplied a scoped patch comparison.
pub(crate) enum ScopedPatchComparisonObservation {
    /// Git produced this comparison during the current reconciliation.
    Observed(ScopedPatchIntegrationEvaluation),
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
            witness:   IntegrationWitness::EvaluatedTrunk,
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
                witness:   IntegrationWitness::EvaluatedTrunk,
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
fn observe_integration_status(
    protected_tip_reachability: Reachability,
    trunk_oid: &GitObjectId,
    prior_integration_status: PriorIntegrationStatus,
    observe_scoped_patch_comparison: impl FnOnce() -> ScopedPatchComparisonObservation,
) -> IntegrationEvidenceObservation {
    match protected_tip_reachability {
        Reachability::Ancestor => {
            IntegrationEvidenceObservation::Reachability(IntegrationEvidenceStatus::Integrated {
                trunk_oid: trunk_oid.clone(),
                proof:     IntegrationProof::ProtectedTipAncestor,
                witness:   IntegrationWitness::EvaluatedTrunk,
            })
        },
        Reachability::NotAncestor => match observe_scoped_patch_comparison() {
            ScopedPatchComparisonObservation::Observed(scoped_patch_comparison) => {
                IntegrationEvidenceObservation::ScopedPatchComparison {
                    status:     status_from_scoped_patch_evaluation(
                        &scoped_patch_comparison,
                        trunk_oid,
                        prior_integration_status,
                    ),
                    evaluation: scoped_patch_comparison,
                }
            },
            ScopedPatchComparisonObservation::Deferred => {
                IntegrationEvidenceObservation::ScopedPatchComparisonDeferred
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
    observe_scoped_patch_comparison: impl FnOnce() -> ScopedPatchComparisonObservation,
) -> IntegrationEvidenceObservation {
    let observation = observe_integration_status(
        protected_tip_reachability,
        current_trunk_oid,
        PriorIntegrationStatus::Unproven,
        observe_scoped_patch_comparison,
    );
    let IntegrationEvidenceObservation::ScopedPatchComparison {
        status: IntegrationEvidenceStatus::NotIntegrated,
        evaluation,
    } = observation
    else {
        return observation;
    };
    let status = match previous_trunk_reachability {
        Reachability::Ancestor => IntegrationEvidenceStatus::NotIntegrated,
        Reachability::NotAncestor => IntegrationEvidenceStatus::TrunkRewritten,
        Reachability::ObjectUnknown => IntegrationEvidenceStatus::ObjectUnknown,
    };
    IntegrationEvidenceObservation::ScopedPatchComparison { status, evaluation }
}

fn status_from_scoped_patch_evaluation(
    evaluation: &ScopedPatchIntegrationEvaluation,
    trunk_oid: &GitObjectId,
    prior_integration_status: PriorIntegrationStatus,
) -> IntegrationEvidenceStatus {
    match evaluation {
        ScopedPatchIntegrationEvaluation::Equivalent(witness) => {
            IntegrationEvidenceStatus::Integrated {
                trunk_oid: trunk_oid.clone(),
                proof:     IntegrationProof::ScopedPatchEquivalent,
                witness:   witness.clone(),
            }
        },
        ScopedPatchIntegrationEvaluation::Different
        | ScopedPatchIntegrationEvaluation::HistoricalEvidenceUnavailable => {
            match prior_integration_status {
                PriorIntegrationStatus::Unproven => IntegrationEvidenceStatus::NotIntegrated,
                PriorIntegrationStatus::Proven => IntegrationEvidenceStatus::TrunkRewritten,
            }
        },
        ScopedPatchIntegrationEvaluation::Unavailable => IntegrationEvidenceStatus::ObjectUnknown,
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
