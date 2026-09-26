//! Durable alerts derived from retained journal state and current git evidence.

use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;
use std::path::Path;

use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;

use crate::edge::RepositoryReservationEvidence;
use crate::edge::RepositoryTrunk;
use crate::git;
use crate::git::GitError;
use crate::git::Reachability;
use crate::git::ReferenceLookup;
use crate::ids::GitObjectId;
use crate::ids::ReservationId;
use crate::ledger::ClaimHeadSnapshot;
use crate::ledger::IntegrationTarget;
use crate::reservation::IntegrationEvidenceStatus;
use crate::reservation::ProtectedReservationTip;
use crate::reservation::ReleaseRevalidationSubject;
use crate::reservation::Reservation;
use crate::reservation::ReservationEvidenceState;
use crate::reservation::ReservationLifecycle;
use crate::reservation::ReservationReplayError;
use crate::reservation::RewrittenIntegrationTrunkCommit;
use crate::worktree::WorktreeLiveness;

/// A persistent coordination condition that remains until journal state resolves it.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[schemars(rename = "alert")]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub(crate) enum Alert {
    /// A live reservation's recorded integration branch no longer exists.
    TargetMissing {
        /// The unreleased reservation still assigned to the missing target.
        reservation_id: ReservationId,
        /// The recorded integration branch that no longer resolves.
        target:         IntegrationTarget,
        /// The retarget command that restores a resolvable integration branch.
        commands:       Vec<String>,
    },
    /// A failed branch observation retains protection until the repository can answer again.
    MergeExtentUnavailable {
        /// The reservation whose last successful surface remains protected.
        #[schemars(with = "String")]
        reservation_id: ReservationId,
        /// The failed observation reported by Git or holder validation.
        failure:        String,
    },
    /// A released reservation no longer has affirmative integration evidence.
    LostIntegrationEvidence(LostIntegrationEvidenceAlert),
    /// A protected reservation has no validated worktree holder.
    OrphanedOutstanding(OrphanedOutstandingAlert),
}

impl Alert {
    /// Return the reservation whose retained state keeps this alert active.
    pub(crate) const fn reservation_id(&self) -> ReservationId {
        match self {
            Self::TargetMissing { reservation_id, .. }
            | Self::MergeExtentUnavailable { reservation_id, .. } => *reservation_id,
            Self::LostIntegrationEvidence(alert) => alert.reservation_id,
            Self::OrphanedOutstanding(alert) => alert.reservation_id,
        }
    }

    /// Count the git queries that established this orphan recovery verdict.
    pub(crate) const fn recovery_evidence_query_count(&self) -> u64 {
        match self {
            Self::TargetMissing { .. }
            | Self::MergeExtentUnavailable { .. }
            | Self::LostIntegrationEvidence(_) => 0,
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
            Self::TargetMissing {
                reservation_id,
                target,
                ..
            } => formatter.write_str(&target_missing_detail(*reservation_id, target)),
            Self::MergeExtentUnavailable {
                reservation_id,
                failure,
            } => write!(
                formatter,
                "Reservation {reservation_id} retains its previous merge protection: {failure}."
            ),
            Self::LostIntegrationEvidence(alert) => match &alert.recovery {
                LostEvidenceRecovery::VerifyResolvedTrunk { trunk_oid, .. } => write!(
                    formatter,
                    "INTEGRATION EVIDENCE LOST: released reservation {} remains non-blocking, and trunk commit {} carries protected tip {}. Run `cargo-berth resolve {} --integrated-as {}`. Inspect `cargo-berth board --json`.",
                    alert.reservation_id,
                    trunk_oid,
                    alert.protected_tip,
                    alert.reservation_id,
                    trunk_oid,
                ),
                LostEvidenceRecovery::NameCarryingTrunkCommit { trunk_oid, .. } => write!(
                    formatter,
                    "INTEGRATION EVIDENCE LOST: released reservation {} remains non-blocking, but trunk {} no longer proves protected tip {}. If a trunk commit carries the released work, run `cargo-berth resolve {} --integrated-as <TRUNK_COMMIT>` naming that commit. Otherwise restore the work first. Inspect `cargo-berth board --json`.",
                    alert.reservation_id, trunk_oid, alert.protected_tip, alert.reservation_id,
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

/// Render the shared human guidance for a missing recorded integration branch.
pub(crate) fn target_missing_detail(
    reservation_id: ReservationId,
    target: &IntegrationTarget,
) -> String {
    format!(
        "TARGET MISSING: reservation {reservation_id} targets {}, which no longer resolves. Run `cargo-berth retarget {reservation_id} --target <branch>`.",
        target.short_name()
    )
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
    /// A trunk commit already proves the protected work; the operator can name it.
    VerifyResolvedTrunk {
        /// The trunk commit that carries the protected work.
        #[schemars(with = "String")]
        #[schemars(length(min = 1))]
        trunk_oid: GitObjectId,
        /// The typed resolution available after the operator verifies the work.
        action:    LostEvidenceRecoveryCommand,
    },
    /// Trunk resolved but does not carry the protected work, so no trunk commit can be named.
    NameCarryingTrunkCommit {
        /// The current configured trunk commit, which does not contain the protected work.
        #[schemars(with = "String")]
        #[schemars(length(min = 1))]
        trunk_oid: GitObjectId,
        /// The typed resolution available once a trunk commit carries the work.
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

/// Whether the reconciliation pass that found an orphan proved its protected work in trunk.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(tag = "status", content = "commit", rename_all = "snake_case")]
pub(crate) enum OrphanIntegrationEvidence {
    /// This trunk commit carries the protected work.
    Proven(#[schemars(with = "String")] RewrittenIntegrationTrunkCommit),
    /// No trunk commit was shown to carry the protected work.
    Unproven,
}

impl From<&RepositoryReservationEvidence> for OrphanIntegrationEvidence {
    fn from(evidence: &RepositoryReservationEvidence) -> Self {
        match evidence {
            RepositoryReservationEvidence::Outstanding {
                integration_status:
                    IntegrationEvidenceStatus::Integrated {
                        trunk_oid, witness, ..
                    },
                ..
            } => Self::Proven(witness.resolve(trunk_oid)),
            RepositoryReservationEvidence::Active
            | RepositoryReservationEvidence::Outstanding { .. }
            | RepositoryReservationEvidence::Released { .. }
            | RepositoryReservationEvidence::ReleasedWithoutCheckpoint { .. } => Self::Unproven,
        }
    }
}

/// The disposition choices supported by an orphan's retained work and observed trunk.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum OrphanResolutionAction {
    /// Recover retained work, or confirm its integration against the observed trunk.
    Recover(LostEvidenceRecovery),
    /// The unavailable commit requires explicit retirement or abandonment.
    RetireOrAbandon,
}

impl OrphanResolutionAction {
    /// Select recovery guidance without making another repository observation.
    pub(crate) fn new(
        orphan: &OrphanedOutstandingAlert,
        repository_trunk: &RepositoryTrunk,
    ) -> Self {
        match orphan.recoverability() {
            RecoverabilityVerdict::CommitUnavailable => Self::RetireOrAbandon,
            RecoverabilityVerdict::RecoverableFromBranch
            | RecoverabilityVerdict::RecoverableFromProtectedTip => {
                let action = LostEvidenceRecoveryCommand::ResolveIntegratedAs {
                    reservation_id: orphan.reservation_id(),
                };
                Self::Recover(match (orphan.integration_evidence(), repository_trunk) {
                    (OrphanIntegrationEvidence::Proven(carrying_commit), _) => {
                        LostEvidenceRecovery::VerifyResolvedTrunk {
                            trunk_oid: carrying_commit.as_ref().clone(),
                            action,
                        }
                    },
                    (OrphanIntegrationEvidence::Unproven, RepositoryTrunk::Resolved(trunk_oid)) => {
                        LostEvidenceRecovery::NameCarryingTrunkCommit {
                            trunk_oid: trunk_oid.clone(),
                            action,
                        }
                    },
                    (OrphanIntegrationEvidence::Unproven, RepositoryTrunk::ObjectUnknown) => {
                        LostEvidenceRecovery::ResolveTrunkFirst { action }
                    },
                })
            },
        }
    }

    /// Name the available commands, retaining both dispositions when work is unavailable.
    pub(crate) fn commands(&self, reservation_id: ReservationId) -> Vec<String> {
        match self {
            Self::Recover(LostEvidenceRecovery::VerifyResolvedTrunk { trunk_oid, .. }) => vec![
                format!("resolve {reservation_id} --recovered"),
                format!("resolve {reservation_id} --integrated-as {trunk_oid}"),
            ],
            Self::Recover(LostEvidenceRecovery::NameCarryingTrunkCommit { .. }) => {
                std::iter::once(format!("resolve {reservation_id} --recovered"))
                    .chain(
                        Self::retirement_flags()
                            .iter()
                            .map(|flag| format!("resolve {reservation_id} {flag}")),
                    )
                    .collect()
            },
            Self::Recover(LostEvidenceRecovery::ResolveTrunkFirst { .. }) => {
                vec![format!("resolve {reservation_id} --recovered")]
            },
            Self::RetireOrAbandon => Self::retirement_flags()
                .iter()
                .map(|flag| format!("resolve {reservation_id} {flag}"))
                .collect(),
        }
    }

    /// The paired explicit dispositions offered when the protected commit is unavailable.
    pub(crate) const fn retirement_flags() -> [&'static str; 2] {
        ["--retire-orphan --why <reason>", "--abandon --why <reason>"]
    }

    /// Explain when an integration disposition is appropriate or requires trunk repair.
    pub(crate) fn integration_guidance(&self, protected_tip: &ProtectedReservationTip) -> String {
        match self {
            Self::Recover(LostEvidenceRecovery::VerifyResolvedTrunk { trunk_oid, .. }) => format!(
                "Use --recovered after restoring the worktree; for a merged branch, use --integrated-as, since trunk commit {trunk_oid} carries the work."
            ),
            Self::Recover(LostEvidenceRecovery::NameCarryingTrunkCommit { trunk_oid, .. }) => format!(
                "Trunk {trunk_oid} does not contain protected tip {protected_tip}; --integrated-as needs a trunk commit that carries this work. Use --recovered after restoring the worktree, or --retire-orphan when the work landed where git cannot match it, such as a reworked squash or a branch other than trunk."
            ),
            Self::Recover(LostEvidenceRecovery::ResolveTrunkFirst { .. }) => {
                "For a merged branch, resolve trunk first, then rerun to name the integration commit."
                    .to_owned()
            },
            Self::RetireOrAbandon => String::new(),
        }
    }
}

/// Recovery evidence for an outstanding reservation whose worktree was pruned.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
pub(crate) struct OrphanedOutstandingAlert {
    /// The reservation that still retains scopes and ordering edges.
    reservation_id:       ReservationId,
    /// The fixed checkpoint commit whose availability was tested.
    protected_tip:        ProtectedReservationTip,
    /// Whether the acquisition-time branch reference survives.
    branch_ref_status:    BranchRefStatus,
    /// Whether git can still read the protected commit object.
    object_availability:  ObjectAvailability,
    /// Whether the private retention ref still protects the expected commit.
    retention_ref:        RetentionRefStatus,
    /// The strongest recovery route established by current evidence.
    recoverability:       RecoverabilityVerdict,
    /// Whether the same reconciliation pass proved the protected work in trunk.
    integration_evidence: OrphanIntegrationEvidence,
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

    /// Borrow the trunk integration evidence observed alongside the orphan.
    const fn integration_evidence(&self) -> &OrphanIntegrationEvidence {
        &self.integration_evidence
    }
}

/// Current status of the branch reference recorded at claim time.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
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
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
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
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
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
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
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
    // The alert fires only when trunk does not prove the work, so the resolved trunk tip is
    // never offered as the `--integrated-as` argument.
    let recovery = match repository_trunk {
        RepositoryTrunk::Resolved(trunk_oid) => LostEvidenceRecovery::NameCarryingTrunkCommit {
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
    integration_evidence: OrphanIntegrationEvidence,
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
        integration_evidence,
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

#[cfg(test)]
mod tests {
    use std::error::Error;

    use super::BranchRefStatus;
    use super::LostEvidenceRecovery;
    use super::LostEvidenceRecoveryCommand;
    use super::ObjectAvailability;
    use super::OrphanIntegrationEvidence;
    use super::OrphanResolutionAction;
    use super::OrphanedOutstandingAlert;
    use super::RecoverabilityVerdict;
    use super::RetentionRefStatus;
    use crate::edge::RepositoryReservationEvidence;
    use crate::edge::RepositoryTrunk;
    use crate::ids::GitObjectId;
    use crate::ids::ReservationId;
    use crate::reservation::IntegrationEvidenceStatus;
    use crate::reservation::IntegrationProof;
    use crate::reservation::IntegrationWitness;
    use crate::reservation::OrphanRetirementReason;
    use crate::reservation::ProtectedReservationTip;
    use crate::reservation::ReleaseDisposition;
    use crate::reservation::RewrittenIntegrationTrunkCommit;

    const CARRYING_COMMIT: &str = "cccccccccccccccccccccccccccccccccccccccc";
    const PROTECTED_TIP: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const TRUNK_TIP: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    #[test]
    fn orphan_resolution_action() -> Result<(), Box<dyn Error>> {
        let reservation_id = ReservationId::new();
        let trunk_oid: GitObjectId = TRUNK_TIP.parse()?;
        let protected_tip = ProtectedReservationTip::from(PROTECTED_TIP.parse::<GitObjectId>()?);
        let carrying_commit =
            RewrittenIntegrationTrunkCommit::from(CARRYING_COMMIT.parse::<GitObjectId>()?);
        for recoverability in [
            RecoverabilityVerdict::RecoverableFromBranch,
            RecoverabilityVerdict::RecoverableFromProtectedTip,
            RecoverabilityVerdict::CommitUnavailable,
        ] {
            for integration_evidence in [
                OrphanIntegrationEvidence::Proven(carrying_commit.clone()),
                OrphanIntegrationEvidence::Unproven,
            ] {
                for trunk in [
                    RepositoryTrunk::Resolved(trunk_oid.clone()),
                    RepositoryTrunk::ObjectUnknown,
                ] {
                    let orphan = OrphanedOutstandingAlert {
                        reservation_id,
                        protected_tip: protected_tip.clone(),
                        branch_ref_status: BranchRefStatus::Detached,
                        object_availability: if recoverability
                            == RecoverabilityVerdict::CommitUnavailable
                        {
                            ObjectAvailability::Unavailable
                        } else {
                            ObjectAvailability::Available
                        },
                        retention_ref: RetentionRefStatus::Missing {
                            reference: "refs/retained".to_owned(),
                        },
                        recoverability,
                        integration_evidence: integration_evidence.clone(),
                    };
                    let action = OrphanResolutionAction::new(&orphan, &trunk);
                    let command =
                        LostEvidenceRecoveryCommand::ResolveIntegratedAs { reservation_id };
                    let expected = match (recoverability, &integration_evidence, &trunk) {
                        (RecoverabilityVerdict::CommitUnavailable, _, _) => {
                            OrphanResolutionAction::RetireOrAbandon
                        },
                        (_, OrphanIntegrationEvidence::Proven(commit), _) => {
                            OrphanResolutionAction::Recover(
                                LostEvidenceRecovery::VerifyResolvedTrunk {
                                    trunk_oid: commit.as_ref().clone(),
                                    action:    command,
                                },
                            )
                        },
                        (
                            _,
                            OrphanIntegrationEvidence::Unproven,
                            RepositoryTrunk::Resolved(trunk_oid),
                        ) => OrphanResolutionAction::Recover(
                            LostEvidenceRecovery::NameCarryingTrunkCommit {
                                trunk_oid: trunk_oid.clone(),
                                action:    command,
                            },
                        ),
                        (
                            _,
                            OrphanIntegrationEvidence::Unproven,
                            RepositoryTrunk::ObjectUnknown,
                        ) => OrphanResolutionAction::Recover(
                            LostEvidenceRecovery::ResolveTrunkFirst { action: command },
                        ),
                    };
                    assert_eq!(action, expected);
                    assert_commands_and_guidance(&action, reservation_id, &protected_tip);
                }
            }
        }
        Ok(())
    }

    /// Assert the commands and guidance each action offers for one orphan.
    fn assert_commands_and_guidance(
        action: &OrphanResolutionAction,
        reservation_id: ReservationId,
        protected_tip: &ProtectedReservationTip,
    ) {
        let commands = action.commands(reservation_id);
        let guidance = action.integration_guidance(protected_tip);
        match *action {
            OrphanResolutionAction::RetireOrAbandon => {
                assert_eq!(
                    commands,
                    [
                        format!("resolve {reservation_id} --retire-orphan --why <reason>"),
                        format!("resolve {reservation_id} --abandon --why <reason>"),
                    ]
                );
                assert_eq!(guidance, "");
            },
            OrphanResolutionAction::Recover(LostEvidenceRecovery::VerifyResolvedTrunk {
                ..
            }) => {
                assert_eq!(
                    commands,
                    [
                        format!("resolve {reservation_id} --recovered"),
                        format!("resolve {reservation_id} --integrated-as {CARRYING_COMMIT}"),
                    ]
                );
                assert_eq!(
                    guidance,
                    format!(
                        "Use --recovered after restoring the worktree; for a merged branch, use --integrated-as, since trunk commit {CARRYING_COMMIT} carries the work."
                    )
                );
            },
            OrphanResolutionAction::Recover(LostEvidenceRecovery::NameCarryingTrunkCommit {
                ..
            }) => {
                assert_eq!(
                    commands,
                    [
                        format!("resolve {reservation_id} --recovered"),
                        format!("resolve {reservation_id} --retire-orphan --why <reason>"),
                        format!("resolve {reservation_id} --abandon --why <reason>"),
                    ]
                );
                assert_eq!(
                    guidance,
                    format!(
                        "Trunk {TRUNK_TIP} does not contain protected tip {PROTECTED_TIP}; --integrated-as needs a trunk commit that carries this work. Use --recovered after restoring the worktree, or --retire-orphan when the work landed where git cannot match it, such as a reworked squash or a branch other than trunk."
                    )
                );
            },
            OrphanResolutionAction::Recover(LostEvidenceRecovery::ResolveTrunkFirst { .. }) => {
                assert_eq!(commands, [format!("resolve {reservation_id} --recovered")]);
                assert_eq!(
                    guidance,
                    "For a merged branch, resolve trunk first, then rerun to name the integration commit."
                );
            },
        }
    }

    #[test]
    fn orphan_integration_evidence_names_the_proving_trunk_commit() -> Result<(), Box<dyn Error>> {
        let trunk_oid: GitObjectId = TRUNK_TIP.parse()?;
        let protected_tip = ProtectedReservationTip::from(PROTECTED_TIP.parse::<GitObjectId>()?);
        let carrying_commit =
            RewrittenIntegrationTrunkCommit::from(CARRYING_COMMIT.parse::<GitObjectId>()?);
        let integrated = |witness: IntegrationWitness| IntegrationEvidenceStatus::Integrated {
            trunk_oid: trunk_oid.clone(),
            proof: IntegrationProof::ScopedPatchEquivalent,
            witness,
        };
        let outstanding = |integration_status: IntegrationEvidenceStatus| {
            RepositoryReservationEvidence::Outstanding {
                protected_tip: protected_tip.clone(),
                integration_status,
            }
        };
        let cases = [
            (
                outstanding(integrated(IntegrationWitness::Historical(
                    carrying_commit.clone(),
                ))),
                OrphanIntegrationEvidence::Proven(carrying_commit),
            ),
            (
                outstanding(integrated(IntegrationWitness::EvaluatedTrunk)),
                OrphanIntegrationEvidence::Proven(RewrittenIntegrationTrunkCommit::from(
                    trunk_oid.clone(),
                )),
            ),
            (
                outstanding(IntegrationEvidenceStatus::NotIntegrated),
                OrphanIntegrationEvidence::Unproven,
            ),
            (
                outstanding(IntegrationEvidenceStatus::TrunkRewritten),
                OrphanIntegrationEvidence::Unproven,
            ),
            (
                outstanding(IntegrationEvidenceStatus::ObjectUnknown),
                OrphanIntegrationEvidence::Unproven,
            ),
            (
                RepositoryReservationEvidence::Released {
                    protected_tip:      protected_tip.clone(),
                    disposition:        ReleaseDisposition::Integrated,
                    integration_status: integrated(IntegrationWitness::EvaluatedTrunk),
                },
                OrphanIntegrationEvidence::Unproven,
            ),
            (
                RepositoryReservationEvidence::ReleasedWithoutCheckpoint {
                    disposition: ReleaseDisposition::RetiredOrphan(
                        "the work landed on another branch".parse::<OrphanRetirementReason>()?,
                    ),
                },
                OrphanIntegrationEvidence::Unproven,
            ),
            (
                RepositoryReservationEvidence::Active,
                OrphanIntegrationEvidence::Unproven,
            ),
        ];
        for (evidence, expected) in cases {
            assert_eq!(OrphanIntegrationEvidence::from(&evidence), expected);
        }
        Ok(())
    }
}
