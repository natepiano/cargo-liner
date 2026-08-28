//! Checkpoint, resnapshot, release, and evidence revalidation.

use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;
use std::path::Path;

use crate::config::BerthConfig;
use crate::config::ConfigError;
use crate::config::Enrollment;
use crate::edge::EdgeReplayError;
use crate::edge::OrderingGraph;
use crate::git;
use crate::git::GitError;
use crate::ids::CoordinationRunId;
use crate::ids::GitObjectId;
use crate::ids::ReservationId;
use crate::ids::WorktreeId;
use crate::ledger;
use crate::ledger::CommittedActionValidation;
use crate::ledger::CoordinationRunMarkerRemoval;
use crate::ledger::JournalOperation;
use crate::ledger::Ledger;
use crate::ledger::LedgerCommittedActionError;
use crate::ledger::LedgerCommittedActionOutcome;
use crate::ledger::LedgerError;
use crate::ledger::LedgerTransactionError;
use crate::ledger::ProtectedPhaseStartHead;
use crate::ledger::ReplayedLedgerState;
use crate::ledger::ReservationSnapshot;
use crate::ledger::WorktreeContext;
use crate::output::CommandVerb;
use crate::output::CoordinationRunMarkerRetirement;
use crate::output::OutputEnvelope;
use crate::output::ReleasePayload;
use crate::reconcile;
use crate::reconcile::RecoveredBypassReporting;
use crate::reservation;
use crate::reservation::EditBlockingStatus;
use crate::reservation::IntegrationEvidenceStatus;
use crate::reservation::PriorIntegrationStatus;
use crate::reservation::ProtectedReservationTip;
use crate::reservation::ReleaseDisposition;
use crate::reservation::ReleaseRevalidationSubject;
use crate::reservation::Reservation;
use crate::reservation::ReservationEvidenceState;
use crate::reservation::ReservationReplayError;
use crate::reservation::RetainedReservationSet;
use crate::scope::ReservationScopeSet;
use crate::session::SessionIdentityMappingPublication;

/// A parsed request to checkpoint or revalidate one reservation.
#[derive(Clone, Copy)]
pub(crate) struct ReleaseRequest {
    /// The reservation named at the command line.
    pub(crate) reservation_id: ReservationId,
}

#[derive(Clone, Copy)]
struct ReleaseTransactionContext<'context> {
    repository_root:      &'context Path,
    trunk_branch:         &'context str,
    reservation_id:       ReservationId,
    invoking_worktree_id: WorktreeId,
}

/// Execute the release lifecycle operation and map every failure to the public envelope.
pub(crate) fn execute(release_request: ReleaseRequest) -> OutputEnvelope {
    let invocation_directory = match std::env::current_dir() {
        Ok(invocation_directory) => invocation_directory,
        Err(error) => {
            return OutputEnvelope::ledger_unreadable(CommandVerb::Release, &error.to_string());
        },
    };
    let mut reconciliation_report =
        match reconcile::reconcile(&invocation_directory, RecoveredBypassReporting::Defer) {
            Ok(Enrollment::Enrolled(reconciliation_report)) => reconciliation_report,
            Ok(Enrollment::Unconfigured {
                expected_configuration_path,
            }) => {
                return OutputEnvelope::unconfigured(
                    CommandVerb::Release,
                    &expected_configuration_path,
                );
            },
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
        Ok(Enrollment::Enrolled(release_payload)) => {
            if matches!(&release_payload, ReleasePayload::Released { .. }) {
                reconciliation_report
                    .alerts
                    .retain(|alert| alert.reservation_id() != release_request.reservation_id);
            }
            OutputEnvelope::released(release_payload)
        },
        Ok(Enrollment::Unconfigured {
            expected_configuration_path,
        }) => OutputEnvelope::unconfigured(CommandVerb::Release, &expected_configuration_path),
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
                OutputEnvelope::ledger_error(CommandVerb::Release, &error)
            },
        },
        Err(ReleaseError::Config(error)) => {
            OutputEnvelope::ledger_error(CommandVerb::Release, &LedgerError::Config(error))
        },
        Err(ReleaseError::Ledger(error)) => {
            OutputEnvelope::ledger_error(CommandVerb::Release, &error)
        },
        Err(error) => OutputEnvelope::ledger_unreadable(CommandVerb::Release, &error.to_string()),
    };
    output_envelope.with_alerts(reconciliation_report.alerts)
}

fn execute_release(
    release_request: ReleaseRequest,
) -> Result<Enrollment<ReleasePayload>, ReleaseError> {
    let invocation_directory = std::env::current_dir()?;
    let worktree_context = WorktreeContext::discover(&invocation_directory)?;
    let journal_mutation_actor = ledger::resolve_identity(&worktree_context)?;
    let berth_config = match BerthConfig::read(worktree_context.repository_root())? {
        Enrollment::Enrolled(berth_config) => berth_config,
        Enrollment::Unconfigured {
            expected_configuration_path,
        } => {
            return Ok(Enrollment::Unconfigured {
                expected_configuration_path,
            });
        },
    };
    let ledger = Ledger::open(worktree_context.repository_root())?;
    let outcome = ledger
        .transact_with_committed_action(
            journal_mutation_actor.worktree_id,
            journal_mutation_actor.coordination_run_id,
            |state| {
                validate_release_transaction(
                    &state,
                    ReleaseTransactionContext {
                        repository_root:      worktree_context.repository_root(),
                        trunk_branch:         &berth_config.trunk,
                        reservation_id:       release_request.reservation_id,
                        invoking_worktree_id: journal_mutation_actor.worktree_id,
                    },
                )
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
        LedgerCommittedActionOutcome::Appended {
            output,
            session_mapping_publication,
        } => Ok(Enrollment::Enrolled(
            output.into_payload(session_mapping_publication),
        )),
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
        LedgerCommittedActionOutcome::Rejected(ReleaseRejection::EdgeReplay(error)) => {
            Err(ReleaseError::EdgeReplay(error))
        },
    }
}

fn validate_release_transaction(
    state: &ReplayedLedgerState<'_>,
    context: ReleaseTransactionContext<'_>,
) -> CommittedActionValidation<ReleaseRejection, ReleaseCommittedAction> {
    let reservations = match RetainedReservationSet::replay(state.events()) {
        Ok(reservations) => reservations,
        Err(error) => return CommittedActionValidation::Reject(ReleaseRejection::Replay(error)),
    };
    let ordering_graph = match OrderingGraph::replay(state.events()) {
        Ok(ordering_graph) => ordering_graph,
        Err(error) => {
            return CommittedActionValidation::Reject(ReleaseRejection::EdgeReplay(error));
        },
    };
    let reservation = match reservations.reservation(context.reservation_id) {
        Ok(reservation) => reservation,
        Err(ReservationReplayError::UnknownReservation(_)) => {
            return CommittedActionValidation::Reject(ReleaseRejection::UnknownReservation);
        },
        Err(error) => return CommittedActionValidation::Reject(ReleaseRejection::Replay(error)),
    };
    let marker_plan = marker_plan_for(
        &reservations,
        reservation.actor().run,
        reservation.actor().worktree,
        context.invoking_worktree_id,
        context.reservation_id,
    );
    let evidence_state = match reservation.evidence_state() {
        Ok(evidence_state) => evidence_state,
        Err(error) => return CommittedActionValidation::Reject(ReleaseRejection::Replay(error)),
    };
    let release_append = match operation_for_state(
        context.repository_root,
        context.trunk_branch,
        context.invoking_worktree_id,
        reservation,
        evidence_state,
    ) {
        Ok(release_append) => release_append,
        Err(error) => return CommittedActionValidation::Reject(error),
    };
    let retention_deletions = match release_retention_deletions(
        &release_append.operation,
        &ordering_graph,
        context.reservation_id,
        &reservations,
    ) {
        Ok(retention_deletions) => retention_deletions,
        Err(error) => return CommittedActionValidation::Reject(error),
    };
    CommittedActionValidation::Append {
        operation: Box::new(release_append.operation),
        action:    ReleaseCommittedAction {
            payload_seed: release_append.payload_seed,
            protected_tip_retention: release_append.protected_tip_retention,
            marker_plan,
            retention_deletions,
        },
    }
}

fn release_retention_deletions(
    operation: &JournalOperation,
    ordering_graph: &OrderingGraph,
    terminal_successor: ReservationId,
    reservations: &RetainedReservationSet,
) -> Result<Vec<ReservationId>, ReleaseRejection> {
    if !matches!(operation, JournalOperation::Release { .. }) {
        return Ok(Vec::new());
    }
    ordering_graph
        .retention_refs_retired_by_terminal(terminal_successor, reservations)
        .map_err(ReleaseRejection::Replay)
}

fn operation_for_state(
    repository_root: &Path,
    trunk_branch: &str,
    invoking_worktree_id: WorktreeId,
    reservation: &Reservation,
    evidence_state: ReservationEvidenceState,
) -> Result<ReleaseAppend, ReleaseRejection> {
    let reservation_id = reservation.id();
    let release_repository_context = ReleaseRepositoryContext {
        repository_root,
        trunk_branch,
        holder_worktree: HolderWorktree::classify(
            invoking_worktree_id,
            reservation.actor().worktree,
        ),
        phase_start_head: reservation.phase_start_head(),
        scopes: reservation.scopes(),
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
            disposition,
            ..
        } => released_evidence_operation(
            &release_repository_context,
            reservation_id,
            &protected_tip,
            &disposition,
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
        reservation::current_head(release_repository_context.repository_root)
            .map_err(ReleaseRejection::Git)?,
    );
    let trunk_oid = reservation::current_trunk(
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
    let Ok(current_trunk) = reservation::current_trunk(
        release_repository_context.repository_root,
        release_repository_context.trunk_branch,
    ) else {
        return Ok(evidence_operation(
            reservation_id,
            IntegrationEvidenceStatus::ObjectUnknown,
            EditBlockingStatus::Blocking,
            protected_tip.clone(),
        ));
    };
    let evidence = if matches!(
        materialized_status,
        IntegrationEvidenceStatus::Integrated { .. }
    ) {
        reservation::integration_status(
            release_repository_context.repository_root,
            release_repository_context.phase_start_head,
            release_repository_context.scopes,
            protected_tip,
            &current_trunk,
            PriorIntegrationStatus::Proven,
        )
    } else {
        reservation::outstanding_integration_status(
            release_repository_context.repository_root,
            release_repository_context.phase_start_head,
            release_repository_context.scopes,
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
        && reservation::current_head(release_repository_context.repository_root)
            .is_ok_and(|current_head| current_head != *protected_tip.as_ref())
    {
        return resnapshot_operation(release_repository_context, reservation_id, &current_trunk);
    }
    let edit_blocking_status = evidence.edit_blocking_status();
    Ok(evidence_operation(
        reservation_id,
        evidence,
        edit_blocking_status,
        protected_tip.clone(),
    ))
}

fn resnapshot_operation(
    release_repository_context: &ReleaseRepositoryContext<'_>,
    reservation_id: ReservationId,
    current_trunk: &GitObjectId,
) -> Result<ReleaseAppend, ReleaseRejection> {
    let replacement_tip = ProtectedReservationTip::from(
        reservation::current_head(release_repository_context.repository_root)
            .map_err(ReleaseRejection::Git)?,
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
) -> Result<ReleaseAppend, ReleaseRejection> {
    let revalidation_tip = match disposition.revalidation_subject() {
        ReleaseRevalidationSubject::ProtectedTip => protected_tip.clone(),
        ReleaseRevalidationSubject::RewrittenIntegration(trunk_commit) => {
            ProtectedReservationTip::from(trunk_commit.as_ref().clone())
        },
        ReleaseRevalidationSubject::None => return Err(ReleaseRejection::AlreadyReleased),
    };
    let Ok(current_trunk) = reservation::current_trunk(
        release_repository_context.repository_root,
        release_repository_context.trunk_branch,
    ) else {
        return Ok(evidence_operation(
            reservation_id,
            IntegrationEvidenceStatus::ObjectUnknown,
            EditBlockingStatus::Clear,
            protected_tip.clone(),
        ));
    };
    let evidence = reservation::integration_status(
        release_repository_context.repository_root,
        release_repository_context.phase_start_head,
        release_repository_context.scopes,
        &revalidation_tip,
        &current_trunk,
        PriorIntegrationStatus::Proven,
    )
    .unwrap_or(IntegrationEvidenceStatus::ObjectUnknown);
    Ok(evidence_operation(
        reservation_id,
        evidence,
        EditBlockingStatus::Clear,
        protected_tip.clone(),
    ))
}

struct ReleaseRepositoryContext<'repository> {
    repository_root:  &'repository Path,
    trunk_branch:     &'repository str,
    holder_worktree:  HolderWorktree,
    phase_start_head: &'repository ProtectedPhaseStartHead,
    scopes:           &'repository ReservationScopeSet,
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
    edit_blocking_status: EditBlockingStatus,
    protected_tip: ProtectedReservationTip,
) -> ReleaseAppend {
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
    fn commit(self, repository_root: &Path) -> Result<(), GitError> {
        reservation::retain_protected_tip(repository_root, self.reservation_id, &self.protected_tip)
    }
}

struct ReleaseCommittedAction {
    payload_seed:            ReleasePayloadSeed,
    protected_tip_retention: ProtectedTipRetention,
    marker_plan:             CoordinationRunMarkerPlan,
    retention_deletions:     Vec<ReservationId>,
}

impl ReleaseCommittedAction {
    fn commit(
        self,
        repository_root: &Path,
        worktree_context: &WorktreeContext,
    ) -> Result<ReleasePayloadPreparation, ReleaseError> {
        git::update_reservation_retention_refs(repository_root, &[], &self.retention_deletions)?;
        self.protected_tip_retention.commit(repository_root)?;
        let marker = self.marker_plan.finish(worktree_context);
        Ok(ReleasePayloadPreparation {
            payload_seed: self.payload_seed,
            marker,
        })
    }
}

struct ReleasePayloadPreparation {
    payload_seed: ReleasePayloadSeed,
    marker:       CoordinationRunMarkerRetirement,
}

impl ReleasePayloadPreparation {
    fn into_payload(
        self,
        session_mapping_publication: SessionIdentityMappingPublication,
    ) -> ReleasePayload {
        self.payload_seed
            .into_payload(self.marker, session_mapping_publication)
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
    fn into_payload(
        self,
        marker: CoordinationRunMarkerRetirement,
        session_mapping_publication: SessionIdentityMappingPublication,
    ) -> ReleasePayload {
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
                session_mapping_publication,
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
                session_mapping_publication,
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
    EdgeReplay(EdgeReplayError),
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
    EdgeReplay(EdgeReplayError),
}

impl Display for ReleaseError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
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
            Self::EdgeReplay(error) => write!(formatter, "ordering replay failed: {error}"),
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
