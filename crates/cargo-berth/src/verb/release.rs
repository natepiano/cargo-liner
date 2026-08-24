//! Checkpoint, resnapshot, release, and evidence revalidation.

use std::fmt;

use crate::config::BerthConfig;
use crate::config::ConfigError;
use crate::git::GitError;
use crate::ids::CoordinationRunId;
use crate::ids::GitObjectId;
use crate::ids::ReservationId;
use crate::ids::WorktreeId;
use crate::ledger::CommittedActionValidation;
use crate::ledger::CoordinationRunMarkerRemoval;
use crate::ledger::EditAuthorization;
use crate::ledger::JournalOperation;
use crate::ledger::Ledger;
use crate::ledger::LedgerCommittedActionError;
use crate::ledger::LedgerCommittedActionOutcome;
use crate::ledger::LedgerError;
use crate::ledger::LedgerTransactionError;
use crate::ledger::ReservationSnapshot;
use crate::ledger::WorktreeContext;
use crate::ledger::worktree_identity;
use crate::output::CommandVerb;
use crate::output::CoordinationRunMarkerRetirement;
use crate::output::OutputEnvelope;
use crate::output::ReleasePayload;
use crate::reservation::IntegrationEvidenceStatus;
use crate::reservation::PriorIntegrationStatus;
use crate::reservation::ProtectedReservationTip;
use crate::reservation::ReleaseDisposition;
use crate::reservation::ReleaseRevalidationSubject;
use crate::reservation::ReservationEvidenceState;
use crate::reservation::ReservationReplayError;
use crate::reservation::RetainedReservationSet;
use crate::reservation::current_head;
use crate::reservation::current_trunk;
use crate::reservation::integration_status;
use crate::reservation::outstanding_integration_status;
use crate::reservation::retain_protected_tip;

/// A parsed request to checkpoint or revalidate one reservation.
#[derive(Clone, Copy)]
pub(crate) struct ReleaseRequest {
    /// The reservation named at the command line.
    pub(crate) reservation_id: ReservationId,
}

/// Execute the release lifecycle operation and map every failure to the public envelope.
pub(crate) fn execute(release_request: ReleaseRequest) -> OutputEnvelope {
    let invocation_directory = match std::env::current_dir() {
        Ok(invocation_directory) => invocation_directory,
        Err(error) => {
            return OutputEnvelope::ledger_unreadable(CommandVerb::Release, &error.to_string());
        },
    };
    let mut reconciliation_report = match crate::reconcile::reconcile(&invocation_directory) {
        Ok(reconciliation_report) => reconciliation_report,
        Err(error) => return error.into_output(CommandVerb::Release),
    };
    for reconciled_evidence in &reconciliation_report.evidence {
        if reconciled_evidence.reservation_id == release_request.reservation_id {
            return OutputEnvelope::released(ReleasePayload::EvidenceRevalidated {
                reservation_id: release_request.reservation_id,
                evidence:       reconciled_evidence.status.clone(),
                marker:         CoordinationRunMarkerRetirement::AlreadyAbsent,
            })
            .with_alerts(reconciliation_report.alerts);
        }
    }
    let output_envelope = match execute_release(release_request) {
        Ok(release_payload) => {
            if matches!(&release_payload, ReleasePayload::Released { .. }) {
                reconciliation_report
                    .alerts
                    .retain(|alert| alert.reservation_id() != release_request.reservation_id);
            }
            OutputEnvelope::released(release_payload)
        },
        Err(ReleaseError::UnknownReservation(reservation_id)) => OutputEnvelope::invalid_input(
            CommandVerb::Release,
            &format!("reservation {reservation_id} does not exist"),
        ),
        Err(ReleaseError::AlreadyReleased(reservation_id)) => OutputEnvelope::invalid_input(
            CommandVerb::Release,
            &format!("reservation {reservation_id} already has a final disposition"),
        ),
        Err(ReleaseError::ForeignActiveReservation(reservation_id)) => {
            OutputEnvelope::invalid_input(
                CommandVerb::Release,
                &format!(
                    "reservation {reservation_id} is active in another worktree and must be checkpointed from its holder"
                ),
            )
        },
        Err(ReleaseError::Transaction(error)) => match error {
            LedgerTransactionError::CorrectableInput(error) => {
                OutputEnvelope::invalid_input(CommandVerb::Release, &error.to_string())
            },
            LedgerTransactionError::LockContention => {
                OutputEnvelope::contention(CommandVerb::Release, &error.to_string())
            },
            LedgerTransactionError::LedgerUnreadable(error) => {
                OutputEnvelope::ledger_unreadable(CommandVerb::Release, &error.to_string())
            },
        },
        Err(error) => OutputEnvelope::ledger_unreadable(CommandVerb::Release, &error.to_string()),
    };
    output_envelope.with_alerts(reconciliation_report.alerts)
}

fn execute_release(release_request: ReleaseRequest) -> Result<ReleasePayload, ReleaseError> {
    let invocation_directory = std::env::current_dir()?;
    let worktree_context = WorktreeContext::discover(&invocation_directory)?;
    let worktree_identity = worktree_identity(
        worktree_context.administrative_directory(),
        worktree_context.worktree_kind(),
    )?;
    let coordination_run_id = mutation_run_id(worktree_context.administrative_directory());
    let berth_config = BerthConfig::read(worktree_context.repository_root())?;
    let ledger = Ledger::open(worktree_context.repository_root())?;
    let outcome = ledger
        .transact_with_committed_action(
            worktree_identity.id,
            coordination_run_id,
            |state| {
                let reservations = match RetainedReservationSet::replay(state.events()) {
                    Ok(reservations) => reservations,
                    Err(error) => {
                        return CommittedActionValidation::Reject(ReleaseRejection::Replay(error));
                    },
                };
                let reservation = match reservations.reservation(release_request.reservation_id) {
                    Ok(reservation) => reservation,
                    Err(ReservationReplayError::UnknownReservation(_)) => {
                        return CommittedActionValidation::Reject(
                            ReleaseRejection::UnknownReservation,
                        );
                    },
                    Err(error) => {
                        return CommittedActionValidation::Reject(ReleaseRejection::Replay(error));
                    },
                };
                let marker_plan = marker_plan_for(
                    &reservations,
                    reservation.actor().run,
                    reservation.actor().worktree,
                    worktree_identity.id,
                    release_request.reservation_id,
                );
                let evidence_state = match reservation.evidence_state() {
                    Ok(evidence_state) => evidence_state,
                    Err(error) => {
                        return CommittedActionValidation::Reject(ReleaseRejection::Replay(error));
                    },
                };
                let release_append = match operation_for_state(
                    worktree_context.repository_root(),
                    &berth_config.trunk,
                    release_request.reservation_id,
                    worktree_identity.id,
                    reservation.actor().worktree,
                    evidence_state,
                ) {
                    Ok(release_append) => release_append,
                    Err(error) => return CommittedActionValidation::Reject(error),
                };
                CommittedActionValidation::Append {
                    operation: Box::new(release_append.operation),
                    action:    ReleaseCommittedAction {
                        payload_seed: release_append.payload_seed,
                        protected_tip_retention: release_append.protected_tip_retention,
                        marker_plan,
                    },
                }
            },
            |committed_action| {
                committed_action.commit(worktree_context.repository_root(), &worktree_context)
            },
        )
        .map_err(|error| match error {
            LedgerCommittedActionError::Transaction(error) => ReleaseError::Transaction(error),
            LedgerCommittedActionError::Action(error) => error,
        })?;
    match outcome {
        LedgerCommittedActionOutcome::Appended(output) => Ok(output),
        LedgerCommittedActionOutcome::Rejected(ReleaseRejection::UnknownReservation) => Err(
            ReleaseError::UnknownReservation(release_request.reservation_id),
        ),
        LedgerCommittedActionOutcome::Rejected(ReleaseRejection::AlreadyReleased) => Err(
            ReleaseError::AlreadyReleased(release_request.reservation_id),
        ),
        LedgerCommittedActionOutcome::Rejected(ReleaseRejection::ForeignActiveReservation) => Err(
            ReleaseError::ForeignActiveReservation(release_request.reservation_id),
        ),
        LedgerCommittedActionOutcome::Rejected(ReleaseRejection::Replay(error)) => {
            Err(ReleaseError::ReservationReplay(error))
        },
        LedgerCommittedActionOutcome::Rejected(ReleaseRejection::Git(error)) => {
            Err(ReleaseError::Git(error))
        },
    }
}

fn operation_for_state(
    repository_root: &std::path::Path,
    trunk_branch: &str,
    reservation_id: ReservationId,
    invoking_worktree_id: WorktreeId,
    holder_worktree_id: WorktreeId,
    evidence_state: ReservationEvidenceState,
) -> Result<ReleaseAppend, ReleaseRejection> {
    let release_repository_context = ReleaseRepositoryContext {
        repository_root,
        trunk_branch,
        holder_worktree: HolderWorktree::classify(invoking_worktree_id, holder_worktree_id),
    };
    match evidence_state {
        ReservationEvidenceState::Active { .. } => {
            checkpoint_operation(&release_repository_context, reservation_id)
        },
        ReservationEvidenceState::Outstanding {
            protected_tip,
            trunk_snapshot,
            integration_status,
        } => outstanding_operation(
            &release_repository_context,
            reservation_id,
            &protected_tip,
            &trunk_snapshot,
            &integration_status,
        ),
        ReservationEvidenceState::Released {
            protected_tip,
            trunk_snapshot: _,
            disposition,
            integration_status,
        } => released_evidence_operation(
            &release_repository_context,
            reservation_id,
            &protected_tip,
            &disposition,
            &integration_status,
        ),
        ReservationEvidenceState::ReleasedWithoutCheckpoint { .. } => {
            Err(ReleaseRejection::AlreadyReleased)
        },
    }
}

fn checkpoint_operation(
    release_repository_context: &ReleaseRepositoryContext<'_>,
    reservation_id: ReservationId,
) -> Result<ReleaseAppend, ReleaseRejection> {
    if release_repository_context.holder_worktree == HolderWorktree::Different {
        return Err(ReleaseRejection::ForeignActiveReservation);
    }
    let protected_tip = ProtectedReservationTip::from(
        current_head(release_repository_context.repository_root).map_err(ReleaseRejection::Git)?,
    );
    let trunk_oid = current_trunk(
        release_repository_context.repository_root,
        release_repository_context.trunk_branch,
    )
    .map_err(ReleaseRejection::Git)?;
    Ok(ReleaseAppend::new(
        JournalOperation::Checkpoint {
            reservation_id,
            protected_tip: protected_tip.clone(),
            trunk_snapshot: trunk_oid.clone(),
        },
        ReleasePayloadSeed::Checkpointed {
            reservation_id,
            protected_tip: protected_tip.clone(),
            trunk_oid,
        },
        reservation_id,
        protected_tip,
    ))
}

fn outstanding_operation(
    release_repository_context: &ReleaseRepositoryContext<'_>,
    reservation_id: ReservationId,
    protected_tip: &ProtectedReservationTip,
    trunk_snapshot: &GitObjectId,
    materialized_status: &IntegrationEvidenceStatus,
) -> Result<ReleaseAppend, ReleaseRejection> {
    let Ok(current_trunk) = current_trunk(
        release_repository_context.repository_root,
        release_repository_context.trunk_branch,
    ) else {
        return Ok(evidence_operation(
            reservation_id,
            IntegrationEvidenceStatus::ObjectUnknown,
            protected_tip.clone(),
        ));
    };
    let evidence = if matches!(
        materialized_status,
        IntegrationEvidenceStatus::Integrated { .. }
    ) {
        integration_status(
            release_repository_context.repository_root,
            protected_tip,
            &current_trunk,
            PriorIntegrationStatus::Proven,
        )
    } else {
        outstanding_integration_status(
            release_repository_context.repository_root,
            protected_tip,
            trunk_snapshot,
            &current_trunk,
        )
    }
    .unwrap_or(IntegrationEvidenceStatus::ObjectUnknown);
    if matches!(
        (materialized_status, &evidence),
        (
            IntegrationEvidenceStatus::Integrated { .. },
            IntegrationEvidenceStatus::Integrated { .. }
        )
    ) {
        return Ok(ReleaseAppend::new(
            JournalOperation::Release {
                reservation_id,
                disposition: ReleaseDisposition::Integrated,
            },
            ReleasePayloadSeed::Released {
                reservation_id,
                disposition: ReleaseDisposition::Integrated,
            },
            reservation_id,
            protected_tip.clone(),
        ));
    }
    if !matches!(
        materialized_status,
        IntegrationEvidenceStatus::Integrated { .. }
    ) && release_repository_context.holder_worktree == HolderWorktree::Invoking
        && matches!(
            evidence,
            IntegrationEvidenceStatus::NotIntegrated | IntegrationEvidenceStatus::TrunkRewritten
        )
        && current_head(release_repository_context.repository_root)
            .is_ok_and(|current_head| current_head != *protected_tip.as_ref())
    {
        return resnapshot_operation(release_repository_context, reservation_id, &current_trunk);
    }
    Ok(evidence_operation(
        reservation_id,
        evidence,
        protected_tip.clone(),
    ))
}

fn resnapshot_operation(
    release_repository_context: &ReleaseRepositoryContext<'_>,
    reservation_id: ReservationId,
    current_trunk: &GitObjectId,
) -> Result<ReleaseAppend, ReleaseRejection> {
    let replacement_tip = ProtectedReservationTip::from(
        current_head(release_repository_context.repository_root).map_err(ReleaseRejection::Git)?,
    );
    Ok(ReleaseAppend::new(
        JournalOperation::Resnapshot {
            reservation_id,
            snapshot: ReservationSnapshot::Outstanding {
                protected_tip: replacement_tip.clone(),
                trunk_oid:     current_trunk.clone(),
            },
        },
        ReleasePayloadSeed::Resnapshotted {
            reservation_id,
            protected_tip: replacement_tip.clone(),
            trunk_oid: current_trunk.clone(),
        },
        reservation_id,
        replacement_tip,
    ))
}

fn released_evidence_operation(
    release_repository_context: &ReleaseRepositoryContext<'_>,
    reservation_id: ReservationId,
    protected_tip: &ProtectedReservationTip,
    disposition: &ReleaseDisposition,
    materialized_status: &IntegrationEvidenceStatus,
) -> Result<ReleaseAppend, ReleaseRejection> {
    let revalidation_tip = match disposition.revalidation_subject() {
        ReleaseRevalidationSubject::ProtectedTip => protected_tip.clone(),
        ReleaseRevalidationSubject::RewrittenIntegration(trunk_commit) => {
            ProtectedReservationTip::from(trunk_commit.as_ref().clone())
        },
        ReleaseRevalidationSubject::None => return Err(ReleaseRejection::AlreadyReleased),
    };
    let Ok(current_trunk) = current_trunk(
        release_repository_context.repository_root,
        release_repository_context.trunk_branch,
    ) else {
        return Ok(evidence_operation(
            reservation_id,
            IntegrationEvidenceStatus::ObjectUnknown,
            protected_tip.clone(),
        ));
    };
    let evidence = integration_status(
        release_repository_context.repository_root,
        &revalidation_tip,
        &current_trunk,
        PriorIntegrationStatus::Proven,
    )
    .unwrap_or(IntegrationEvidenceStatus::ObjectUnknown);
    if release_repository_context.holder_worktree == HolderWorktree::Invoking
        && matches!(
            materialized_status,
            IntegrationEvidenceStatus::NotIntegrated | IntegrationEvidenceStatus::TrunkRewritten
        )
        && matches!(
            evidence,
            IntegrationEvidenceStatus::NotIntegrated | IntegrationEvidenceStatus::TrunkRewritten
        )
        && current_head(release_repository_context.repository_root)
            .is_ok_and(|current_head| current_head != *protected_tip.as_ref())
    {
        return resnapshot_operation(release_repository_context, reservation_id, &current_trunk);
    }
    Ok(evidence_operation(
        reservation_id,
        evidence,
        protected_tip.clone(),
    ))
}

struct ReleaseRepositoryContext<'repository> {
    repository_root: &'repository std::path::Path,
    trunk_branch:    &'repository str,
    holder_worktree: HolderWorktree,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum HolderWorktree {
    Invoking,
    Different,
}

impl HolderWorktree {
    fn classify(invoking_worktree_id: WorktreeId, holder_worktree_id: WorktreeId) -> Self {
        if invoking_worktree_id == holder_worktree_id {
            Self::Invoking
        } else {
            Self::Different
        }
    }
}

fn evidence_operation(
    reservation_id: ReservationId,
    evidence: IntegrationEvidenceStatus,
    protected_tip: ProtectedReservationTip,
) -> ReleaseAppend {
    let edit_blocking_status = evidence.edit_blocking_status();
    ReleaseAppend::new(
        JournalOperation::EvidenceRevalidated {
            reservation_id,
            status: evidence.clone(),
            edit_blocking_status,
        },
        ReleasePayloadSeed::EvidenceRevalidated {
            reservation_id,
            evidence,
        },
        reservation_id,
        protected_tip,
    )
}

fn mutation_run_id(administrative_directory: &std::path::Path) -> CoordinationRunId {
    match EditAuthorization::resolve(administrative_directory) {
        EditAuthorization::Environment(coordination_run_id)
        | EditAuthorization::Marker {
            coordination_run_id,
            ..
        } => coordination_run_id,
        EditAuthorization::Unidentified => CoordinationRunId::new(),
    }
}

fn marker_plan_for(
    reservations: &RetainedReservationSet,
    holder_run_id: CoordinationRunId,
    holder_worktree_id: WorktreeId,
    invoking_worktree_id: WorktreeId,
    reservation_id: ReservationId,
) -> CoordinationRunMarkerPlan {
    if holder_worktree_id != invoking_worktree_id {
        CoordinationRunMarkerPlan::PreserveDifferentWorktree
    } else if reservations.has_other_active_reservation(holder_run_id, reservation_id) {
        CoordinationRunMarkerPlan::PreserveForActiveReservation
    } else {
        CoordinationRunMarkerPlan::Remove(holder_run_id)
    }
}

struct ReleaseAppend {
    operation:               JournalOperation,
    payload_seed:            ReleasePayloadSeed,
    protected_tip_retention: ProtectedTipRetention,
}

impl ReleaseAppend {
    const fn new(
        operation: JournalOperation,
        payload_seed: ReleasePayloadSeed,
        reservation_id: ReservationId,
        protected_tip: ProtectedReservationTip,
    ) -> Self {
        Self {
            operation,
            payload_seed,
            protected_tip_retention: ProtectedTipRetention {
                reservation_id,
                protected_tip,
            },
        }
    }
}

struct ProtectedTipRetention {
    reservation_id: ReservationId,
    protected_tip:  ProtectedReservationTip,
}

impl ProtectedTipRetention {
    fn commit(self, repository_root: &std::path::Path) -> Result<(), GitError> {
        retain_protected_tip(repository_root, self.reservation_id, &self.protected_tip)
    }
}

struct ReleaseCommittedAction {
    payload_seed:            ReleasePayloadSeed,
    protected_tip_retention: ProtectedTipRetention,
    marker_plan:             CoordinationRunMarkerPlan,
}

impl ReleaseCommittedAction {
    fn commit(
        self,
        repository_root: &std::path::Path,
        worktree_context: &WorktreeContext,
    ) -> Result<ReleasePayload, ReleaseError> {
        self.protected_tip_retention.commit(repository_root)?;
        let marker = self.marker_plan.finish(worktree_context);
        Ok(self.payload_seed.into_payload(marker))
    }
}

enum ReleasePayloadSeed {
    Checkpointed {
        reservation_id: ReservationId,
        protected_tip:  ProtectedReservationTip,
        trunk_oid:      GitObjectId,
    },
    Resnapshotted {
        reservation_id: ReservationId,
        protected_tip:  ProtectedReservationTip,
        trunk_oid:      GitObjectId,
    },
    EvidenceRevalidated {
        reservation_id: ReservationId,
        evidence:       IntegrationEvidenceStatus,
    },
    Released {
        reservation_id: ReservationId,
        disposition:    ReleaseDisposition,
    },
}

impl ReleasePayloadSeed {
    fn into_payload(self, marker: CoordinationRunMarkerRetirement) -> ReleasePayload {
        match self {
            Self::Checkpointed {
                reservation_id,
                protected_tip,
                trunk_oid,
            } => ReleasePayload::Checkpointed {
                reservation_id,
                protected_tip,
                trunk_oid,
                marker,
            },
            Self::Resnapshotted {
                reservation_id,
                protected_tip,
                trunk_oid,
            } => ReleasePayload::Resnapshotted {
                reservation_id,
                protected_tip,
                trunk_oid,
                marker,
            },
            Self::EvidenceRevalidated {
                reservation_id,
                evidence,
            } => ReleasePayload::EvidenceRevalidated {
                reservation_id,
                evidence,
                marker,
            },
            Self::Released {
                reservation_id,
                disposition,
            } => ReleasePayload::Released {
                reservation_id,
                disposition,
                marker,
            },
        }
    }
}

enum CoordinationRunMarkerPlan {
    Remove(CoordinationRunId),
    PreserveForActiveReservation,
    PreserveDifferentWorktree,
}

impl CoordinationRunMarkerPlan {
    fn finish(&self, worktree_context: &WorktreeContext) -> CoordinationRunMarkerRetirement {
        match self {
            Self::PreserveForActiveReservation => {
                CoordinationRunMarkerRetirement::PreservedForActiveReservation
            },
            Self::PreserveDifferentWorktree => {
                CoordinationRunMarkerRetirement::PreservedDifferentWorktree
            },
            Self::Remove(coordination_run_id) => worktree_context
                .remove_coordination_run_marker(*coordination_run_id)
                .map_or_else(
                    |error| CoordinationRunMarkerRetirement::Unavailable {
                        diagnostic: error.to_string(),
                    },
                    CoordinationRunMarkerRetirement::from,
                ),
        }
    }
}

impl From<CoordinationRunMarkerRemoval> for CoordinationRunMarkerRetirement {
    fn from(removal: CoordinationRunMarkerRemoval) -> Self {
        match removal {
            CoordinationRunMarkerRemoval::Removed => Self::Removed,
            CoordinationRunMarkerRemoval::AlreadyAbsent => Self::AlreadyAbsent,
            CoordinationRunMarkerRemoval::PreservedDifferentRun => Self::PreservedDifferentRun,
            CoordinationRunMarkerRemoval::PreservedMalformed => Self::PreservedMalformed,
        }
    }
}

enum ReleaseRejection {
    UnknownReservation,
    AlreadyReleased,
    ForeignActiveReservation,
    Replay(ReservationReplayError),
    Git(GitError),
}

#[derive(Debug)]
enum ReleaseError {
    Io(std::io::Error),
    Config(ConfigError),
    Git(GitError),
    Ledger(LedgerError),
    Transaction(LedgerTransactionError),
    ReservationReplay(ReservationReplayError),
    UnknownReservation(ReservationId),
    AlreadyReleased(ReservationId),
    ForeignActiveReservation(ReservationId),
}

impl fmt::Display for ReleaseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "release I/O failed: {error}"),
            Self::Config(error) => error.fmt(formatter),
            Self::Git(error) => error.fmt(formatter),
            Self::Ledger(error) => error.fmt(formatter),
            Self::Transaction(error) => error.fmt(formatter),
            Self::ReservationReplay(error) => {
                write!(formatter, "reservation replay failed: {error}")
            },
            Self::UnknownReservation(reservation_id) => {
                write!(formatter, "reservation {reservation_id} does not exist")
            },
            Self::AlreadyReleased(reservation_id) => {
                write!(
                    formatter,
                    "reservation {reservation_id} is already released"
                )
            },
            Self::ForeignActiveReservation(reservation_id) => write!(
                formatter,
                "reservation {reservation_id} is active in another worktree"
            ),
        }
    }
}

impl std::error::Error for ReleaseError {}

impl From<std::io::Error> for ReleaseError {
    fn from(error: std::io::Error) -> Self { Self::Io(error) }
}

impl From<ConfigError> for ReleaseError {
    fn from(error: ConfigError) -> Self { Self::Config(error) }
}

impl From<GitError> for ReleaseError {
    fn from(error: GitError) -> Self { Self::Git(error) }
}

impl From<LedgerError> for ReleaseError {
    fn from(error: LedgerError) -> Self { Self::Ledger(error) }
}

impl From<LedgerTransactionError> for ReleaseError {
    fn from(error: LedgerTransactionError) -> Self { Self::Transaction(error) }
}
