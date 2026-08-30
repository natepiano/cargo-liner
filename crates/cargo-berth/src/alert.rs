//! Durable alerts derived from retained journal state and current git evidence.

use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;
use std::path::Path;

use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;

use crate::edge::RepositoryTrunk;
use crate::git;
use crate::git::GitError;
use crate::git::Reachability;
use crate::git::ReferenceLookup;
use crate::ids::GitObjectId;
use crate::ids::ReservationId;
use crate::ledger::ClaimHeadSnapshot;
use crate::reservation::IntegrationEvidenceStatus;
use crate::reservation::ProtectedReservationTip;
use crate::reservation::ReleaseRevalidationSubject;
use crate::reservation::Reservation;
use crate::reservation::ReservationEvidenceState;
use crate::reservation::ReservationLifecycle;
use crate::reservation::ReservationReplayError;
use crate::worktree::WorktreeLiveness;

/// A persistent coordination condition that remains until journal state resolves it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub(crate) enum Alert {
    /// A released reservation no longer has affirmative integration evidence.
    LostIntegrationEvidence(LostIntegrationEvidenceAlert),
    /// A protected reservation has no validated worktree holder.
    OrphanedOutstanding(OrphanedOutstandingAlert),
}

impl Alert {
    /// Return the reservation whose retained state keeps this alert active.
    pub(crate) const fn reservation_id(&self) -> ReservationId {
        match self {
            Self::LostIntegrationEvidence(alert) => alert.reservation_id,
            Self::OrphanedOutstanding(alert) => alert.reservation_id,
        }
    }

    /// Count the git queries that established this orphan recovery verdict.
    pub(crate) const fn recovery_evidence_query_count(&self) -> u64 {
        match self {
            Self::LostIntegrationEvidence(_) => 0,
            Self::OrphanedOutstanding(alert) => match alert.branch_ref_status {
                BranchRefStatus::Present { .. } => 4,
                BranchRefStatus::Missing { .. } => 3,
                BranchRefStatus::Detached => 2,
            },
        }
    }
}

impl Display for Alert {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::LostIntegrationEvidence(alert) => match &alert.recovery {
                LostEvidenceRecovery::VerifyResolvedTrunk { trunk_oid, .. } => write!(
                    formatter,
                    "INTEGRATION EVIDENCE LOST: released reservation {} remains non-blocking, but trunk {} no longer proves protected tip {}. If trunk {} contains the released work, run `cargo-berth resolve {} --integrated-as {}`. Otherwise restore the work first. Inspect `cargo-berth board --json`.",
                    alert.reservation_id,
                    trunk_oid,
                    alert.protected_tip,
                    trunk_oid,
                    alert.reservation_id,
                    trunk_oid,
                ),
                LostEvidenceRecovery::ResolveTrunkFirst { .. } => write!(
                    formatter,
                    "INTEGRATION EVIDENCE LOST: released reservation {} remains non-blocking, and trunk does not currently resolve to a known object, so protected tip {} cannot be proved either way. Resolve trunk first, then rerun. Inspect `cargo-berth board --json`.",
                    alert.reservation_id, alert.protected_tip,
                ),
            },
            Self::OrphanedOutstanding(alert) => write!(
                formatter,
                "Alert: orphaned outstanding reservation {} at protected tip {}; branch {}; object {}; retention {}; recovery {}.",
                alert.reservation_id,
                alert.protected_tip,
                alert.branch_ref_status,
                alert.object_availability,
                alert.retention_ref,
                alert.recoverability,
            ),
        }
    }
}

/// Lost affirmative Git evidence for a terminal reservation.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[schemars(rename = "lost_integration_evidence_alert")]
pub(crate) struct LostIntegrationEvidenceAlert {
    /// The released reservation whose evidence no longer proves integration.
    #[schemars(with = "String")]
    reservation_id:  ReservationId,
    /// The fixed checkpoint commit whose released work needs confirmation.
    protected_tip:   ProtectedReservationTip,
    /// What the current repository observation proves about the released evidence.
    evidence_status: LostIntegrationEvidenceStatus,
    /// The recovery path selected by whether trunk resolved.
    recovery:        LostEvidenceRecovery,
}

impl LostIntegrationEvidenceAlert {
    /// Return the released reservation whose integration evidence was lost.
    pub(crate) const fn reservation_id(&self) -> ReservationId { self.reservation_id }

    /// Borrow the released reservation's fixed checkpoint commit.
    pub(crate) const fn protected_tip(&self) -> &ProtectedReservationTip { &self.protected_tip }

    /// Borrow the current non-affirmative evidence status.
    pub(crate) const fn evidence_status(&self) -> &LostIntegrationEvidenceStatus {
        &self.evidence_status
    }

    /// Borrow the recovery path supported by the current trunk observation.
    pub(crate) const fn recovery(&self) -> &LostEvidenceRecovery { &self.recovery }
}

declare_wire_enum! {
    /// A non-affirmative integration status eligible for a lost-evidence alert.
    #[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
    #[schemars(rename = "lost_integration_evidence_status")]
    #[serde(tag = "status", rename_all = "snake_case")]
    pub(crate) enum LostIntegrationEvidenceStatus {
        /// The protected work is not reachable from the configured trunk.
        NotIntegrated => "not_integrated";
        /// Trunk no longer contains evidence that was verified earlier.
        TrunkRewritten => "trunk_rewritten";
        /// Git could not resolve an object required by the evidence query.
        ObjectUnknown => "object_unknown";
    }
}

/// Recovery instructions distinguished by whether the configured trunk resolved.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[schemars(rename = "lost_evidence_recovery")]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum LostEvidenceRecovery {
    /// Trunk resolved; the operator can confirm it carries the released work.
    VerifyResolvedTrunk {
        /// The current configured trunk commit.
        #[schemars(with = "String")]
        trunk_oid: GitObjectId,
        /// The typed resolution available after the operator verifies the work.
        action:    LostEvidenceRecoveryCommand,
    },
    /// No trunk object resolved; trunk must resolve before any repair is available.
    ResolveTrunkFirst {
        /// The typed resolution that becomes available after trunk resolves.
        action: LostEvidenceRecoveryCommand,
    },
}

/// The reservation recovery command represented without a stringly typed flag.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[schemars(rename = "lost_evidence_recovery_action")]
#[serde(tag = "action", rename_all = "snake_case")]
pub(crate) enum LostEvidenceRecoveryCommand {
    /// Replace lost Git-backed evidence with an operator-verified trunk commit.
    ResolveIntegratedAs {
        #[schemars(with = "String")]
        reservation_id: ReservationId,
    },
}

/// Recovery evidence for an outstanding reservation whose worktree was pruned.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct OrphanedOutstandingAlert {
    /// The reservation that still retains scopes and ordering edges.
    reservation_id:      ReservationId,
    /// The fixed checkpoint commit whose availability was tested.
    protected_tip:       ProtectedReservationTip,
    /// Whether the acquisition-time branch reference survives.
    branch_ref_status:   BranchRefStatus,
    /// Whether git can still read the protected commit object.
    object_availability: ObjectAvailability,
    /// Whether the private retention ref still protects the expected commit.
    retention_ref:       RetentionRefStatus,
    /// The strongest recovery route established by current evidence.
    recoverability:      RecoverabilityVerdict,
}

impl OrphanedOutstandingAlert {
    /// Return the outstanding reservation that requires a disposition.
    pub(crate) const fn reservation_id(&self) -> ReservationId { self.reservation_id }

    /// Borrow the fixed checkpoint commit protected by this alert.
    pub(crate) const fn protected_tip(&self) -> &ProtectedReservationTip { &self.protected_tip }

    /// Borrow the acquisition-time branch observation.
    pub(crate) const fn branch_ref_status(&self) -> &BranchRefStatus { &self.branch_ref_status }

    /// Return whether git can read the protected commit.
    pub(crate) const fn object_availability(&self) -> ObjectAvailability {
        self.object_availability
    }

    /// Borrow the retention reference evidence relevant to recoverability.
    pub(crate) const fn retention_ref_status(&self) -> &RetentionRefStatus { &self.retention_ref }

    /// Return the recovery conclusion already established by reconciliation.
    pub(crate) const fn recoverability(&self) -> RecoverabilityVerdict { self.recoverability }
}

/// Current status of the branch reference recorded at claim time.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum BranchRefStatus {
    /// The full branch reference still resolves.
    Present {
        /// The full reference name.
        reference: String,
        /// The commit currently named by the branch.
        tip:       GitObjectId,
    },
    /// The recorded full branch reference no longer resolves.
    Missing { reference: String },
    /// The reservation was acquired from a detached worktree.
    Detached,
}

impl Display for BranchRefStatus {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Present { reference, tip } => write!(formatter, "{reference} present at {tip}"),
            Self::Missing { reference } => write!(formatter, "{reference} missing"),
            Self::Detached => formatter.write_str("detached at claim"),
        }
    }
}

/// Whether git can read the protected checkpoint commit.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ObjectAvailability {
    /// Git can read the commit.
    Available,
    /// Git cannot read the commit.
    Unavailable,
}

impl Display for ObjectAvailability {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Available => formatter.write_str("available"),
            Self::Unavailable => formatter.write_str("unavailable"),
        }
    }
}

/// Current status of the reservation's private retention reference.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum RetentionRefStatus {
    /// The retention ref points to the protected tip.
    Present { reference: String },
    /// No retention ref resolves for this reservation.
    Missing { reference: String },
    /// The retention ref resolves to a different object.
    Mismatched {
        /// The private reference name.
        reference: String,
        /// The unexpected object currently named by the reference.
        actual:    GitObjectId,
    },
}

impl Display for RetentionRefStatus {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Present { reference } => write!(formatter, "{reference} present"),
            Self::Missing { reference } => write!(formatter, "{reference} missing"),
            Self::Mismatched { reference, actual } => {
                write!(formatter, "{reference} mismatched at {actual}")
            },
        }
    }
}

/// The recovery conclusion current git evidence supports.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RecoverabilityVerdict {
    /// The acquisition-time branch remains available.
    RecoverableFromBranch,
    /// The branch does not retain the tip, but the private retention ref does.
    RecoverableFromProtectedTip,
    /// Neither the branch nor retained protected commit is available.
    CommitUnavailable,
}

/// Whether the acquisition-time branch still contains the protected checkpoint.
enum BranchProtectedTipStatus {
    /// The branch ref retains the protected tip in its ancestry.
    Reachable,
    /// The branch is absent or no longer retains the protected tip.
    Unreachable,
}

/// Derive an alert when a released Git-backed disposition has no affirmative proof.
pub(crate) fn for_lost_integration_evidence(
    reservation: &Reservation,
    repository_trunk: &RepositoryTrunk,
) -> Result<Vec<Alert>, ReservationReplayError> {
    let ReservationEvidenceState::Released {
        protected_tip,
        disposition,
        integration_status,
        ..
    } = reservation.evidence_state()?
    else {
        return Ok(Vec::new());
    };
    if matches!(
        disposition.revalidation_subject(),
        ReleaseRevalidationSubject::None
    ) {
        return Ok(Vec::new());
    }
    let evidence_status = match integration_status {
        IntegrationEvidenceStatus::NotIntegrated => LostIntegrationEvidenceStatus::NotIntegrated,
        IntegrationEvidenceStatus::TrunkRewritten => LostIntegrationEvidenceStatus::TrunkRewritten,
        IntegrationEvidenceStatus::ObjectUnknown => LostIntegrationEvidenceStatus::ObjectUnknown,
        IntegrationEvidenceStatus::Integrated { .. } => return Ok(Vec::new()),
    };

    let action = LostEvidenceRecoveryCommand::ResolveIntegratedAs {
        reservation_id: reservation.id(),
    };
    let recovery = match repository_trunk {
        RepositoryTrunk::Resolved(trunk_oid) => LostEvidenceRecovery::VerifyResolvedTrunk {
            trunk_oid: trunk_oid.clone(),
            action,
        },
        RepositoryTrunk::ObjectUnknown => LostEvidenceRecovery::ResolveTrunkFirst { action },
    };
    Ok(vec![Alert::LostIntegrationEvidence(
        LostIntegrationEvidenceAlert {
            reservation_id: reservation.id(),
            protected_tip,
            evidence_status,
            recovery,
        },
    )])
}

impl Display for RecoverabilityVerdict {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::RecoverableFromBranch => formatter.write_str("recoverable from branch"),
            Self::RecoverableFromProtectedTip => {
                formatter.write_str("recoverable from protected tip")
            },
            Self::CommitUnavailable => formatter.write_str("commit unavailable"),
        }
    }
}

/// Derive an alert only for an outstanding reservation proven orphaned.
pub(crate) fn for_orphaned_outstanding(
    repository_root: &Path,
    reservation: &Reservation,
    worktree_liveness: WorktreeLiveness,
) -> Result<Vec<Alert>, GitError> {
    let ReservationLifecycle::Outstanding { protected_tip } = reservation.lifecycle() else {
        return Ok(Vec::new());
    };
    if worktree_liveness != WorktreeLiveness::Orphaned {
        return Ok(Vec::new());
    }

    let branch_ref_status = branch_status(repository_root, reservation)?;
    let object_availability = if git::commit_is_available(repository_root, protected_tip.as_ref())?
    {
        ObjectAvailability::Available
    } else {
        ObjectAvailability::Unavailable
    };
    let retention_ref = retention_status(repository_root, reservation.id(), protected_tip)?;
    let branch_protected_tip_status =
        branch_protected_tip_status(repository_root, &branch_ref_status, protected_tip)?;
    let recoverability = match (
        branch_protected_tip_status,
        object_availability,
        &retention_ref,
    ) {
        (BranchProtectedTipStatus::Reachable, _, _) => RecoverabilityVerdict::RecoverableFromBranch,
        (
            BranchProtectedTipStatus::Unreachable,
            ObjectAvailability::Available,
            RetentionRefStatus::Present { .. },
        ) => RecoverabilityVerdict::RecoverableFromProtectedTip,
        _ => RecoverabilityVerdict::CommitUnavailable,
    };
    Ok(vec![Alert::OrphanedOutstanding(OrphanedOutstandingAlert {
        reservation_id: reservation.id(),
        protected_tip: protected_tip.clone(),
        branch_ref_status,
        object_availability,
        retention_ref,
        recoverability,
    })])
}

fn branch_protected_tip_status(
    repository_root: &Path,
    branch_ref_status: &BranchRefStatus,
    protected_tip: &ProtectedReservationTip,
) -> Result<BranchProtectedTipStatus, GitError> {
    let BranchRefStatus::Present { tip, .. } = branch_ref_status else {
        return Ok(BranchProtectedTipStatus::Unreachable);
    };
    match git::reachability(repository_root, protected_tip.as_ref(), tip)? {
        Reachability::Ancestor => Ok(BranchProtectedTipStatus::Reachable),
        Reachability::NotAncestor | Reachability::ObjectUnknown => {
            Ok(BranchProtectedTipStatus::Unreachable)
        },
    }
}

fn branch_status(
    repository_root: &Path,
    reservation: &Reservation,
) -> Result<BranchRefStatus, GitError> {
    match reservation.head_snapshot() {
        ClaimHeadSnapshot::Branch { full_ref, .. } => {
            let reference = full_ref.to_string();
            match git::reference_lookup(repository_root, &reference)? {
                ReferenceLookup::Present(tip) => Ok(BranchRefStatus::Present { reference, tip }),
                ReferenceLookup::Missing => Ok(BranchRefStatus::Missing { reference }),
            }
        },
        ClaimHeadSnapshot::Detached { .. } => Ok(BranchRefStatus::Detached),
    }
}

fn retention_status(
    repository_root: &Path,
    reservation_id: ReservationId,
    protected_tip: &ProtectedReservationTip,
) -> Result<RetentionRefStatus, GitError> {
    let reference = git::reservation_retention_ref_name(reservation_id);
    match git::reference_lookup(repository_root, &reference)? {
        ReferenceLookup::Present(actual) if actual == *protected_tip.as_ref() => {
            Ok(RetentionRefStatus::Present { reference })
        },
        ReferenceLookup::Present(actual) => {
            Ok(RetentionRefStatus::Mismatched { reference, actual })
        },
        ReferenceLookup::Missing => Ok(RetentionRefStatus::Missing { reference }),
    }
}
