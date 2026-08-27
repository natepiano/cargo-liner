//! Shared liveness, evidence, retention-ref, and marker reconciliation.

use std::collections::HashMap;
use std::error::Error;
use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;
use std::path::Path;
use std::path::PathBuf;

use crate::alert;
use crate::alert::Alert;
use crate::config::BerthConfig;
use crate::config::ConfigError;
use crate::config::Enrollment;
use crate::edge::EdgeReplayError;
use crate::edge::IntegrationConstraintProjection;
use crate::edge::MissingReadinessFact;
use crate::edge::OrderingGraph;
use crate::edge::PredecessorReachability;
use crate::edge::RepositoryReservationEvidence;
use crate::edge::RepositoryReservationSnapshot;
use crate::edge::RepositorySnapshot;
use crate::edge::RepositoryTrunk;
use crate::edge::SuccessorHeadReachability;
use crate::gate::permit;
use crate::gate::permit::PendingBypassMarkerImport;
use crate::gate::permit::RecoveredPendingBypassMarker;
use crate::git;
use crate::git::CandidateHeadReachability;
use crate::git::DescendantCommitQuery;
use crate::git::GitError;
use crate::ids::CoordinationRunId;
use crate::ids::GitObjectId;
use crate::ids::JournalByteOffset;
use crate::ids::ProjectionGeneration;
use crate::ids::RepoInstanceId;
use crate::ids::ReservationId;
use crate::ids::WorktreeId;
use crate::ledger;
use crate::ledger::BypassOccurrenceTime;
use crate::ledger::JournalEvent;
use crate::ledger::JournalOperation;
use crate::ledger::Ledger;
use crate::ledger::LedgerCommittedActionError;
use crate::ledger::LedgerCommittedActionOutcome;
use crate::ledger::LedgerError;
use crate::ledger::LedgerTransactionError;
use crate::ledger::ReconciliationValidation;
use crate::ledger::RecoverableReconciliationAppendFailures;
use crate::ledger::ReplayedLedgerState;
use crate::ledger::WorktreeContext;
use crate::output::CommandVerb;
use crate::output::OutputEnvelope;
use crate::reservation;
use crate::reservation::EditBlockingStatus;
use crate::reservation::IntegrationEvidenceStatus;
use crate::reservation::PriorIntegrationStatus;
use crate::reservation::ProtectedReservationTip;
use crate::reservation::ReleaseRevalidationSubject;
use crate::reservation::Reservation;
use crate::reservation::ReservationEvidenceState;
use crate::reservation::ReservationLifecycle;
use crate::reservation::ReservationReplayError;
use crate::reservation::RetainedReservationSet;
use crate::worktree::WorktreeHead;
use crate::worktree::WorktreeLiveness;
use crate::worktree::WorktreeRegistry;
use crate::worktree::WorktreeRelocation;
use crate::worktree::liveness::WorktreeRegistryError;

/// Whether this reconciliation caller will announce recovered bypass markers.
#[derive(Clone, Copy)]
pub(crate) enum RecoveredBypassReporting {
    /// Retire recovered markers under the reconciliation lock and return their identities.
    Report,
    /// Leave recovered markers for a later reporting consumer and return no identities.
    Defer,
}

/// Alerts that remain after one complete reconciliation.
pub(crate) struct ReconciliationReport {
    /// Durable alerts derived from retained journal state.
    pub(crate) alerts:                        Vec<Alert>,
    /// Integration conclusions appended by this reconciliation.
    pub(crate) evidence:                      Vec<ReconciledEvidence>,
    /// The one complete repository observation shared by edge and board consumers.
    pub(crate) repository_snapshot:           RepositorySnapshot,
    /// Complete edge and answer state derived from the same committed locked replay.
    pub(crate) constraints:                   IntegrationConstraintProjection,
    /// The exact locked replay point from which board state is projected.
    pub(crate) journal_snapshot:              ReconciledJournalSnapshot,
    /// Pending bypass markers that could not yet become ordinary audit records.
    pub(crate) unrecorded_bypass_occurrences: Vec<BypassOccurrenceTime>,
    /// Pending bypass markers retired and claimed for reporting by this reconciliation.
    pub(crate) recovered_bypass_markers:      Vec<RecoveredPendingBypassMarker>,
    /// Git query dimensions observed while reconciliation assembled this report.
    pub(crate) git_cost:                      ReconciliationGitCost,
}

/// Git query dimensions owned by reconciliation rather than board row projection.
pub(crate) struct ReconciliationGitCost {
    /// Calls that attempted to resolve the configured trunk.
    pub(crate) trunk_resolution_calls:           u64,
    /// Calls used to establish orphan recovery evidence.
    pub(crate) orphan_recovery_evidence_queries: u64,
}

/// Complete journal truth retained from one reconciliation lock acquisition.
pub(crate) struct ReconciledJournalSnapshot {
    events:             Vec<JournalEvent>,
    generation:         ProjectionGeneration,
    journal_end_offset: JournalByteOffset,
}

impl ReconciledJournalSnapshot {
    /// Borrow every event visible at the reconciled replay point.
    pub(crate) fn events(&self) -> &[JournalEvent] { &self.events }

    /// Return the projection generation shared by every board section.
    pub(crate) const fn generation(&self) -> ProjectionGeneration { self.generation }

    /// Return the journal byte offset shared by every board section.
    pub(crate) const fn journal_end_offset(&self) -> JournalByteOffset { self.journal_end_offset }
}

/// One evidence conclusion appended before the requesting stateful verb ran.
pub(crate) struct ReconciledEvidence {
    /// The reservation whose evidence changed.
    pub(crate) reservation_id: ReservationId,
    /// The newly materialized integration result.
    pub(crate) status:         IntegrationEvidenceStatus,
}

struct ReconciliationPlan {
    operations: Vec<JournalOperation>,
    action:     ReconciliationAction,
}

/// Actual-trunk reconciliation plus proposed-trunk constraints prepared under one lock.
pub(crate) struct GateReconciliation {
    reconciliation: ReconciliationPlan,
    constraints:    IntegrationConstraintProjection,
    reservations:   RetainedReservationSet,
}

/// A gate decision committed together with any reconciliation and permit records.
pub(crate) struct GateReconciliationAction<Decision> {
    reconciliation: ReconciliationAction,
    decision:       Decision,
}

#[derive(Default)]
struct ReconciliationChanges {
    operations:          Vec<JournalOperation>,
    retention_repairs:   Vec<RetentionRepair>,
    retention_deletions: Vec<ReservationId>,
    evidence:            Vec<ReconciledEvidence>,
}

struct ReconciliationAction {
    active_holders:                Vec<ActiveHolder>,
    marker_contexts:               Vec<WorktreeContext>,
    repository_root:               PathBuf,
    retention_repairs:             Vec<RetentionRepair>,
    retention_deletions:           Vec<ReservationId>,
    alert_subjects:                Vec<AlertSubject>,
    evidence:                      Vec<ReconciledEvidence>,
    repository_snapshot:           RepositorySnapshot,
    recovered_bypass_reporting:    RecoveredBypassReporting,
    recovered_bypass_markers:      Vec<RecoveredPendingBypassMarker>,
    pending_bypass_imports:        Vec<PendingBypassMarkerImport>,
    unrecorded_bypass_occurrences: Vec<BypassOccurrenceTime>,
    trunk_resolution_calls:        u64,
}

#[derive(Clone, Copy)]
struct ActiveHolder {
    worktree_id:         WorktreeId,
    coordination_run_id: CoordinationRunId,
}

struct RetentionRepair {
    reservation_id: ReservationId,
    protected_tip:  ProtectedReservationTip,
}

struct AlertSubject {
    reservation:       Reservation,
    worktree_liveness: WorktreeLiveness,
}

#[derive(Clone, Copy)]
enum RepositoryObservationScope {
    CurrentOrderingGraph,
    RequestedOrderingEdge {
        before: ReservationId,
        after:  ReservationId,
    },
}

/// Reconcile every retained reservation before a stateful command consumes it.
pub(crate) fn reconcile(
    invocation_directory: &Path,
    recovered_bypass_reporting: RecoveredBypassReporting,
) -> Result<Enrollment<ReconciliationReport>, ReconcileError> {
    reconcile_with_scope(
        invocation_directory,
        RepositoryObservationScope::CurrentOrderingGraph,
        recovered_bypass_reporting,
    )
}

/// Reconcile with the ordering graph that would result if one request is admitted.
pub(crate) fn reconcile_for_sequence(
    invocation_directory: &Path,
    before: ReservationId,
    after: ReservationId,
) -> Result<Enrollment<ReconciliationReport>, ReconcileError> {
    reconcile_with_scope(
        invocation_directory,
        RepositoryObservationScope::RequestedOrderingEdge { before, after },
        RecoveredBypassReporting::Defer,
    )
}

fn reconcile_with_scope(
    invocation_directory: &Path,
    repository_observation_scope: RepositoryObservationScope,
    recovered_bypass_reporting: RecoveredBypassReporting,
) -> Result<Enrollment<ReconciliationReport>, ReconcileError> {
    let worktree_context = WorktreeContext::discover(invocation_directory)?;
    match BerthConfig::read(worktree_context.repository_root())? {
        Enrollment::Enrolled(berth_config) => reconcile_enrolled(
            &worktree_context,
            &berth_config,
            repository_observation_scope,
            recovered_bypass_reporting,
        )
        .map(Enrollment::Enrolled),
        Enrollment::Unconfigured {
            expected_configuration_path,
        } => Ok(Enrollment::Unconfigured {
            expected_configuration_path,
        }),
    }
}

/// Reconcile only after [`BerthConfig::read`] proves that this worktree is enrolled.
fn reconcile_enrolled(
    worktree_context: &WorktreeContext,
    berth_config: &BerthConfig,
    repository_observation_scope: RepositoryObservationScope,
    recovered_bypass_reporting: RecoveredBypassReporting,
) -> Result<ReconciliationReport, ReconcileError> {
    let ledger = Ledger::open(worktree_context.repository_root())?;
    let ledger_repository = ledger.repository_identity()?;
    let worktree_identity = ledger::worktree_identity(
        worktree_context.administrative_directory(),
        worktree_context.worktree_kind(),
    )?;
    let coordination_run_id = CoordinationRunId::new();
    let outcome = ledger
        .transact_reconciliation(
            worktree_identity.id,
            coordination_run_id,
            |state| {
                let reservations = match RetainedReservationSet::replay(state.events()) {
                    Ok(reservations) => reservations,
                    Err(error) => {
                        return ReconciliationValidation::Reject(
                            ReconciliationPlanningError::Reservation(error),
                        );
                    },
                };
                let ordering_graph = match OrderingGraph::replay(state.events()) {
                    Ok(ordering_graph) => ordering_graph,
                    Err(error) => {
                        return ReconciliationValidation::Reject(
                            ReconciliationPlanningError::Edge(error),
                        );
                    },
                };
                let worktree_registry =
                    match WorktreeRegistry::read(worktree_context.repository_root()) {
                        Ok(worktree_registry) => worktree_registry,
                        Err(error) => {
                            return ReconciliationValidation::Reject(
                                ReconciliationPlanningError::WorktreeRegistry(error),
                            );
                        },
                    };
                let mut reconciliation_plan = match build_plan(
                    &reservations,
                    &ordering_graph,
                    repository_observation_scope,
                    &worktree_registry,
                    ledger_repository,
                    worktree_context,
                    berth_config,
                ) {
                    Ok(reconciliation_plan) => reconciliation_plan,
                    Err(error) => {
                        return ReconciliationValidation::Reject(
                            ReconciliationPlanningError::Reservation(error),
                        );
                    },
                };
                let mut pending_bypasses = match permit::prepare_pending_bypass_recovery(
                    worktree_context.common_git_directory(),
                    state.events(),
                ) {
                    Ok(pending_bypasses) => pending_bypasses,
                    Err(error) => {
                        return ReconciliationValidation::Reject(
                            ReconciliationPlanningError::PendingBypass(error),
                        );
                    },
                };
                let pending_bypass_imports = pending_bypasses.take_imports();
                let recoverable_operations = pending_bypass_imports
                    .iter()
                    .map(|pending_import| pending_import.operation().clone())
                    .collect();
                reconciliation_plan.action.pending_bypass_imports = pending_bypass_imports;
                reconciliation_plan.action.recovered_bypass_reporting = recovered_bypass_reporting;
                reconciliation_plan.action.recovered_bypass_markers =
                    pending_bypasses.take_completed_markers();
                reconciliation_plan.action.unrecorded_bypass_occurrences =
                    pending_bypasses.take_unrecorded_occurrences();
                ReconciliationValidation::Apply {
                    operations: reconciliation_plan.operations,
                    recoverable_operations,
                    action: reconciliation_plan.action,
                }
            },
            ReconciliationAction::commit,
        )
        .map_err(|error| match error {
            LedgerCommittedActionError::Transaction(error) => ReconcileError::Transaction(error),
            LedgerCommittedActionError::Action(error) => error,
        })?;
    match outcome {
        LedgerCommittedActionOutcome::Appended { output: report, .. } => Ok(report),
        LedgerCommittedActionOutcome::Rejected(error) => Err(error.into()),
    }
}

fn build_plan(
    reservations: &RetainedReservationSet,
    ordering_graph: &OrderingGraph,
    repository_observation_scope: RepositoryObservationScope,
    worktree_registry: &WorktreeRegistry,
    ledger_repository: RepoInstanceId,
    worktree_context: &WorktreeContext,
    berth_config: &BerthConfig,
) -> Result<ReconciliationPlan, ReservationReplayError> {
    let common_git_directory = worktree_context.common_git_directory();
    let repository_root = worktree_context.repository_root();
    let mut changes = ReconciliationChanges::default();
    let mut alert_subjects = Vec::new();
    let mut trunk_resolution_calls = 0;
    trunk_resolution_calls += 1;
    let repository_trunk = reservation::current_trunk(repository_root, &berth_config.trunk)
        .map_or(RepositoryTrunk::ObjectUnknown, RepositoryTrunk::Resolved);
    let mut reservation_snapshots = Vec::new();
    for reservation in reservations.iter() {
        let observation =
            worktree_registry.classify(ledger_repository, common_git_directory, reservation);
        if let WorktreeRelocation::Relocated { current_root } = &observation.relocation {
            changes.operations.push(JournalOperation::RelocateWorktree {
                reservation_id: reservation.id(),
                worktree_id:    reservation.actor().worktree,
                previous_root:  reservation.worktree_root().clone(),
                current_root:   current_root.clone(),
            });
        }
        alert_subjects.push(AlertSubject {
            reservation:       reservation.clone(),
            worktree_liveness: observation.liveness,
        });
        let repository_evidence =
            repository_evidence(repository_root, reservation, &repository_trunk)?;
        append_evidence_and_retention(
            reservation,
            &repository_evidence,
            reservations,
            ordering_graph,
            &mut changes,
        )?;
        reservation_snapshots.push(RepositoryReservationSnapshot {
            reservation_id:    reservation.id(),
            worktree_liveness: observation.liveness,
            worktree_head:     observation.head,
            evidence:          repository_evidence,
        });
    }
    let predecessor_reachability = predecessor_descendants(
        repository_root,
        ordering_graph,
        repository_observation_scope,
        &reservation_snapshots,
    );
    let repository_snapshot = RepositorySnapshot::new(
        repository_trunk,
        reservation_snapshots,
        predecessor_reachability,
    );
    let active_holders = reservations
        .iter()
        .filter(|reservation| matches!(reservation.lifecycle(), ReservationLifecycle::Active))
        .map(|reservation| ActiveHolder {
            worktree_id:         reservation.actor().worktree,
            coordination_run_id: reservation.actor().run,
        })
        .collect();
    Ok(ReconciliationPlan {
        operations: changes.operations,
        action:     ReconciliationAction {
            active_holders,
            marker_contexts: worktree_registry.marker_sweep_contexts(common_git_directory),
            repository_root: repository_root.to_path_buf(),
            retention_repairs: changes.retention_repairs,
            retention_deletions: changes.retention_deletions,
            alert_subjects,
            evidence: changes.evidence,
            repository_snapshot,
            recovered_bypass_reporting: RecoveredBypassReporting::Defer,
            recovered_bypass_markers: Vec::new(),
            pending_bypass_imports: Vec::new(),
            unrecorded_bypass_occurrences: Vec::new(),
            trunk_resolution_calls,
        },
    })
}

/// Prepare the actual reconciliation and proposed-ref constraint read from one replay.
///
/// Beyond the one fixed `rev-list` used to identify newly reachable commits, the hook performs
/// one `cat-file` batch and at most one grouped `rev-list` for each protected graph predecessor,
/// independent of the total retained-reservation count. The proposed-trunk view reuses those
/// reachability facts and changes only trunk evidence.
pub(crate) fn prepare_gate_reconciliation(
    events: &[JournalEvent],
    generation: ProjectionGeneration,
    worktree_context: &WorktreeContext,
    ledger_repository: RepoInstanceId,
    berth_config: &BerthConfig,
    proposed_trunk: GitObjectId,
) -> Result<GateReconciliation, GateReconciliationError> {
    let reservations =
        RetainedReservationSet::replay(events).map_err(GateReconciliationError::Reservation)?;
    let ordering_graph = OrderingGraph::replay(events).map_err(GateReconciliationError::Edge)?;
    let worktree_registry = WorktreeRegistry::read(worktree_context.repository_root())
        .map_err(GateReconciliationError::WorktreeRegistry)?;
    let reconciliation = build_plan(
        &reservations,
        &ordering_graph,
        RepositoryObservationScope::CurrentOrderingGraph,
        &worktree_registry,
        ledger_repository,
        worktree_context,
        berth_config,
    )
    .map_err(GateReconciliationError::Reservation)?;
    let proposed_snapshot = observe_proposed_trunk(
        &reservations,
        &reconciliation.action.repository_snapshot,
        worktree_context,
        proposed_trunk,
    )?;
    let constraints = ordering_graph
        .integration_constraints(&reservations, &proposed_snapshot, generation)
        .map_err(GateReconciliationError::MissingReadinessFact)?;
    Ok(GateReconciliation {
        reconciliation,
        constraints,
        reservations,
    })
}

fn observe_proposed_trunk(
    reservations: &RetainedReservationSet,
    actual_snapshot: &RepositorySnapshot,
    worktree_context: &WorktreeContext,
    proposed_trunk: GitObjectId,
) -> Result<RepositorySnapshot, GateReconciliationError> {
    let repository_trunk = RepositoryTrunk::Resolved(proposed_trunk);
    let reservation_snapshots = reservations
        .iter()
        .map(|reservation| {
            let actual_reservation = actual_snapshot
                .reservation(reservation.id())
                .map_err(GateReconciliationError::MissingReadinessFact)?;
            Ok(RepositoryReservationSnapshot {
                reservation_id:    reservation.id(),
                worktree_liveness: actual_reservation.worktree_liveness,
                worktree_head:     actual_reservation.worktree_head.clone(),
                evidence:          repository_evidence(
                    worktree_context.repository_root(),
                    reservation,
                    &repository_trunk,
                )
                .map_err(GateReconciliationError::Reservation)?,
            })
        })
        .collect::<Result<Vec<_>, GateReconciliationError>>()?;
    let predecessor_reachability = actual_snapshot
        .predecessor_reachability()
        .map(|(reservation_id, reachability)| (*reservation_id, reachability.clone()))
        .collect();
    Ok(RepositorySnapshot::new(
        repository_trunk,
        reservation_snapshots,
        predecessor_reachability,
    ))
}

impl GateReconciliation {
    /// Borrow the shared gate-and-board projection prepared at this generation.
    pub(crate) const fn constraints(&self) -> &IntegrationConstraintProjection { &self.constraints }

    /// Borrow reservations when a stateful caller validates its marker-derived actor.
    pub(crate) const fn reservations(&self) -> &RetainedReservationSet { &self.reservations }

    /// Join a gate decision and its journal records to the prepared reconciliation.
    pub(crate) fn into_action<Decision>(
        mut self,
        additional_operations: Vec<JournalOperation>,
        decision: Decision,
    ) -> (Vec<JournalOperation>, GateReconciliationAction<Decision>) {
        self.reconciliation.operations.extend(additional_operations);
        (
            self.reconciliation.operations,
            GateReconciliationAction {
                reconciliation: self.reconciliation.action,
                decision,
            },
        )
    }
}

impl<Decision> GateReconciliationAction<Decision> {
    /// Commit reconciliation repairs while retaining the already validated gate decision.
    pub(crate) fn commit(
        self,
        state: &ReplayedLedgerState<'_>,
        recoverable_failures: &RecoverableReconciliationAppendFailures,
    ) -> Result<(ReconciliationReport, Decision), ReconcileError> {
        let report = self.reconciliation.commit(state, recoverable_failures)?;
        Ok((report, self.decision))
    }
}

/// A locked gate read could not produce complete replayed constraints.
#[derive(Debug)]
pub(crate) enum GateReconciliationError {
    /// Reservation replay failed.
    Reservation(ReservationReplayError),
    /// Ordering-graph replay failed.
    Edge(EdgeReplayError),
    /// The repository's worktree registry could not be observed.
    WorktreeRegistry(WorktreeRegistryError),
    /// A derived readiness value lacked a required repository fact.
    MissingReadinessFact(MissingReadinessFact),
}

impl Display for GateReconciliationError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Reservation(error) => error.fmt(formatter),
            Self::Edge(error) => error.fmt(formatter),
            Self::WorktreeRegistry(error) => error.fmt(formatter),
            Self::MissingReadinessFact(error) => error.fmt(formatter),
        }
    }
}

impl Error for GateReconciliationError {}

fn repository_evidence(
    repository_root: &Path,
    reservation: &Reservation,
    repository_trunk: &RepositoryTrunk,
) -> Result<RepositoryReservationEvidence, ReservationReplayError> {
    match reservation.evidence_state()? {
        ReservationEvidenceState::Active { .. } => Ok(RepositoryReservationEvidence::Active),
        ReservationEvidenceState::Outstanding {
            protected_tip,
            trunk_snapshot,
            integration_status: materialized,
        } => {
            let integration_status = match repository_trunk {
                RepositoryTrunk::Resolved(current_trunk_oid) => {
                    if matches!(materialized, IntegrationEvidenceStatus::Integrated { .. }) {
                        reservation::integration_status(
                            repository_root,
                            reservation.phase_start_head(),
                            reservation.scopes(),
                            &protected_tip,
                            current_trunk_oid,
                            PriorIntegrationStatus::Proven,
                        )
                    } else {
                        reservation::outstanding_integration_status(
                            repository_root,
                            reservation.phase_start_head(),
                            reservation.scopes(),
                            &protected_tip,
                            &trunk_snapshot,
                            current_trunk_oid,
                        )
                    }
                    .unwrap_or(IntegrationEvidenceStatus::ObjectUnknown)
                },
                RepositoryTrunk::ObjectUnknown => IntegrationEvidenceStatus::ObjectUnknown,
            };
            Ok(RepositoryReservationEvidence::Outstanding {
                protected_tip,
                integration_status,
            })
        },
        ReservationEvidenceState::Released {
            protected_tip,
            disposition,
            integration_status: materialized,
            ..
        } => {
            let integration_status = match disposition.revalidation_subject() {
                ReleaseRevalidationSubject::ProtectedTip => revalidate_release(
                    repository_root,
                    reservation,
                    &protected_tip,
                    repository_trunk,
                ),
                ReleaseRevalidationSubject::RewrittenIntegration(trunk_commit) => {
                    let revalidation_tip =
                        ProtectedReservationTip::from(trunk_commit.as_ref().clone());
                    revalidate_release(
                        repository_root,
                        reservation,
                        &revalidation_tip,
                        repository_trunk,
                    )
                },
                ReleaseRevalidationSubject::None => materialized,
            };
            Ok(RepositoryReservationEvidence::Released {
                protected_tip,
                disposition,
                integration_status,
            })
        },
        ReservationEvidenceState::ReleasedWithoutCheckpoint { disposition } => {
            Ok(RepositoryReservationEvidence::ReleasedWithoutCheckpoint { disposition })
        },
    }
}

fn revalidate_release(
    repository_root: &Path,
    reservation: &Reservation,
    protected_tip: &ProtectedReservationTip,
    repository_trunk: &RepositoryTrunk,
) -> IntegrationEvidenceStatus {
    match repository_trunk {
        RepositoryTrunk::Resolved(current_trunk_oid) => reservation::integration_status(
            repository_root,
            reservation.phase_start_head(),
            reservation.scopes(),
            protected_tip,
            current_trunk_oid,
            PriorIntegrationStatus::Proven,
        )
        .unwrap_or(IntegrationEvidenceStatus::ObjectUnknown),
        RepositoryTrunk::ObjectUnknown => IntegrationEvidenceStatus::ObjectUnknown,
    }
}

fn append_evidence_and_retention(
    reservation: &Reservation,
    repository_evidence: &RepositoryReservationEvidence,
    reservations: &RetainedReservationSet,
    ordering_graph: &OrderingGraph,
    changes: &mut ReconciliationChanges,
) -> Result<(), ReservationReplayError> {
    let (protected_tip, evidence_revalidation, retention) = match repository_evidence {
        RepositoryReservationEvidence::Active
        | RepositoryReservationEvidence::ReleasedWithoutCheckpoint { .. } => return Ok(()),
        RepositoryReservationEvidence::Outstanding {
            protected_tip,
            integration_status,
        } => (
            protected_tip,
            EvidenceRevalidation::Required(integration_status),
            RetentionDecision::Repair,
        ),
        RepositoryReservationEvidence::Released {
            protected_tip,
            integration_status,
            disposition,
        } => {
            let retention =
                if ordering_graph.has_nonterminal_dependent(reservation.id(), reservations)? {
                    RetentionDecision::Repair
                } else {
                    RetentionDecision::Delete
                };
            let evidence_revalidation = match disposition.revalidation_subject() {
                ReleaseRevalidationSubject::ProtectedTip
                | ReleaseRevalidationSubject::RewrittenIntegration(_) => {
                    EvidenceRevalidation::Required(integration_status)
                },
                ReleaseRevalidationSubject::None => EvidenceRevalidation::NotApplicable,
            };
            (protected_tip, evidence_revalidation, retention)
        },
    };
    match retention {
        RetentionDecision::Repair => changes.retention_repairs.push(RetentionRepair {
            reservation_id: reservation.id(),
            protected_tip:  protected_tip.clone(),
        }),
        RetentionDecision::Delete => changes.retention_deletions.push(reservation.id()),
    }
    let EvidenceRevalidation::Required(evidence) = evidence_revalidation else {
        return Ok(());
    };
    let materialized = match reservation.evidence_state()? {
        ReservationEvidenceState::Outstanding {
            integration_status, ..
        }
        | ReservationEvidenceState::Released {
            integration_status, ..
        } => integration_status,
        ReservationEvidenceState::Active { .. }
        | ReservationEvidenceState::ReleasedWithoutCheckpoint { .. } => return Ok(()),
    };
    let edit_blocking_status = match reservation.lifecycle() {
        ReservationLifecycle::Active => EditBlockingStatus::Blocking,
        ReservationLifecycle::Outstanding { .. } => evidence.edit_blocking_status(),
        ReservationLifecycle::Released { .. } => EditBlockingStatus::Clear,
    };
    if materialized != *evidence {
        changes
            .operations
            .push(JournalOperation::EvidenceRevalidated {
                reservation_id: reservation.id(),
                status: evidence.clone(),
                edit_blocking_status,
            });
        changes.evidence.push(ReconciledEvidence {
            reservation_id: reservation.id(),
            status:         evidence.clone(),
        });
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum RetentionDecision {
    Repair,
    Delete,
}

enum EvidenceRevalidation<'evidence> {
    Required(&'evidence IntegrationEvidenceStatus),
    NotApplicable,
}

fn predecessor_descendants(
    repository_root: &Path,
    ordering_graph: &OrderingGraph,
    repository_observation_scope: RepositoryObservationScope,
    reservation_snapshots: &[RepositoryReservationSnapshot],
) -> Vec<(ReservationId, PredecessorReachability)> {
    let snapshots_by_reservation = reservation_snapshots
        .iter()
        .map(|snapshot| (snapshot.reservation_id, snapshot))
        .collect::<HashMap<_, _>>();
    let mut successors_by_predecessor = ordering_graph
        .predecessors()
        .map(|predecessor| (predecessor.reservation_id, predecessor.successors.to_vec()))
        .collect::<HashMap<_, _>>();
    if let RepositoryObservationScope::RequestedOrderingEdge { before, after } =
        repository_observation_scope
    {
        let successors = successors_by_predecessor.entry(before).or_default();
        if !successors.contains(&after) {
            successors.push(after);
        }
    }
    successors_by_predecessor
        .into_iter()
        .filter_map(|(predecessor_id, successors)| {
            let predecessor_snapshot = snapshots_by_reservation.get(&predecessor_id)?;
            let protected_tip = match &predecessor_snapshot.evidence {
                RepositoryReservationEvidence::Outstanding { protected_tip, .. }
                | RepositoryReservationEvidence::Released { protected_tip, .. } => protected_tip,
                RepositoryReservationEvidence::Active
                | RepositoryReservationEvidence::ReleasedWithoutCheckpoint { .. } => return None,
            };
            let candidate_heads = successors
                .iter()
                .filter_map(|successor| {
                    snapshots_by_reservation
                        .get(successor)
                        .and_then(|snapshot| match &snapshot.worktree_head {
                            WorktreeHead::Resolved(head) => Some(head.clone()),
                            WorktreeHead::Unavailable => None,
                        })
                })
                .collect::<Vec<_>>();
            if candidate_heads.is_empty() {
                return None;
            }
            let predecessor_reachability =
                git::descendant_commits(repository_root, protected_tip.as_ref(), &candidate_heads)
                    .map_or(PredecessorReachability::QueryFailed, |query| match query {
                        DescendantCommitQuery::Classified(candidate_heads) => {
                            PredecessorReachability::Classified(
                                candidate_heads
                                    .into_iter()
                                    .map(|candidate_head| match candidate_head {
                                        CandidateHeadReachability::Descendant(head) => {
                                            (head, SuccessorHeadReachability::ContainsPredecessor)
                                        },
                                        CandidateHeadReachability::NotDescendant(head) => (
                                            head,
                                            SuccessorHeadReachability::DoesNotContainPredecessor,
                                        ),
                                        CandidateHeadReachability::ObjectUnknown(head) => {
                                            (head, SuccessorHeadReachability::ObjectUnknown)
                                        },
                                    })
                                    .collect(),
                            )
                        },
                        DescendantCommitQuery::AncestorObjectUnknown => {
                            PredecessorReachability::ObjectUnknown
                        },
                    });
            Some((predecessor_id, predecessor_reachability))
        })
        .collect()
}

impl ReconciliationAction {
    fn commit(
        mut self,
        state: &ReplayedLedgerState<'_>,
        recoverable_failures: &RecoverableReconciliationAppendFailures,
    ) -> Result<ReconciliationReport, ReconcileError> {
        let reservations =
            RetainedReservationSet::replay(state.events()).map_err(ReconcileError::Replay)?;
        let ordering_graph =
            OrderingGraph::replay(state.events()).map_err(ReconcileError::EdgeReplay)?;
        let constraints = ordering_graph
            .integration_constraints(&reservations, &self.repository_snapshot, state.generation())
            .map_err(ReconcileError::MissingReadinessFact)?;
        for pending_import in self.pending_bypass_imports {
            if recoverable_failures.contains(pending_import.operation()) {
                self.unrecorded_bypass_occurrences
                    .push(pending_import.occurrence_time().clone());
            } else {
                self.recovered_bypass_markers
                    .push(pending_import.into_recovered_marker());
            }
        }
        for reservation_id in self.retention_deletions {
            git::delete_reservation_retention_ref(&self.repository_root, reservation_id)?;
        }
        for retention_repair in self.retention_repairs {
            if git::commit_is_available(
                &self.repository_root,
                retention_repair.protected_tip.as_ref(),
            )? {
                reservation::retain_protected_tip(
                    &self.repository_root,
                    retention_repair.reservation_id,
                    &retention_repair.protected_tip,
                )?;
            }
        }
        for marker_context in self.marker_contexts {
            let marker_worktree_id =
                ledger::read_worktree_identity(marker_context.administrative_directory());
            marker_context.sweep_coordination_run_marker(|coordination_run_id| {
                marker_worktree_id.is_ok_and(|worktree_id| {
                    self.active_holders.iter().any(|active_holder| {
                        active_holder.worktree_id == worktree_id
                            && active_holder.coordination_run_id == coordination_run_id
                    })
                })
            })?;
        }
        let mut alerts = Vec::new();
        for alert_subject in self.alert_subjects {
            alerts.extend(alert::for_orphaned_outstanding(
                &self.repository_root,
                &alert_subject.reservation,
                alert_subject.worktree_liveness,
            )?);
        }
        let orphan_recovery_evidence_queries = alerts
            .iter()
            .map(Alert::recovery_evidence_query_count)
            .sum();
        let recovered_bypass_markers = match self.recovered_bypass_reporting {
            RecoveredBypassReporting::Report => {
                permit::delete_recovered_bypass_markers(&self.recovered_bypass_markers)
                    .map_err(LedgerError::Io)?;
                self.recovered_bypass_markers
            },
            RecoveredBypassReporting::Defer => Vec::new(),
        };
        Ok(ReconciliationReport {
            alerts,
            evidence: self.evidence,
            repository_snapshot: self.repository_snapshot,
            constraints,
            journal_snapshot: ReconciledJournalSnapshot {
                events:             state.events().to_vec(),
                generation:         state.generation(),
                journal_end_offset: state.journal_end_offset(),
            },
            unrecorded_bypass_occurrences: self.unrecorded_bypass_occurrences,
            recovered_bypass_markers,
            git_cost: ReconciliationGitCost {
                trunk_resolution_calls: self.trunk_resolution_calls,
                orphan_recovery_evidence_queries,
            },
        })
    }
}

#[derive(Debug)]
enum ReconciliationPlanningError {
    Reservation(ReservationReplayError),
    Edge(EdgeReplayError),
    WorktreeRegistry(WorktreeRegistryError),
    PendingBypass(std::io::Error),
}

/// A reconciliation failure classified for command-boundary exit behavior.
#[derive(Debug)]
pub(crate) enum ReconcileError {
    Config(ConfigError),
    Git(GitError),
    Ledger(LedgerError),
    Replay(ReservationReplayError),
    EdgeReplay(EdgeReplayError),
    MissingReadinessFact(MissingReadinessFact),
    Transaction(LedgerTransactionError),
    WorktreeRegistry(WorktreeRegistryError),
}

impl ReconcileError {
    /// Convert a failed prerequisite into the requesting verb's public response.
    pub(crate) fn into_output(self, command_verb: CommandVerb) -> OutputEnvelope {
        match self {
            Self::Transaction(LedgerTransactionError::LockContention) => {
                OutputEnvelope::contention(
                    command_verb,
                    &LedgerTransactionError::LockContention.to_string(),
                )
            },
            Self::Transaction(LedgerTransactionError::CorrectableInput(error)) => {
                OutputEnvelope::invalid_input(command_verb, &error.to_string())
            },
            Self::Config(error) => {
                OutputEnvelope::ledger_error(command_verb, &LedgerError::Config(error))
            },
            Self::Git(error) => OutputEnvelope::ledger_unreadable(command_verb, &error.to_string()),
            Self::Ledger(error)
            | Self::Transaction(LedgerTransactionError::LedgerUnreadable(error)) => {
                OutputEnvelope::ledger_error(command_verb, &error)
            },
            Self::Replay(error) => {
                OutputEnvelope::ledger_unreadable(command_verb, &error.to_string())
            },
            Self::EdgeReplay(error) => {
                OutputEnvelope::ledger_unreadable(command_verb, &error.to_string())
            },
            Self::MissingReadinessFact(error) => {
                OutputEnvelope::ledger_unreadable(command_verb, &error.to_string())
            },
            Self::WorktreeRegistry(error) => {
                OutputEnvelope::ledger_unreadable(command_verb, &error.to_string())
            },
        }
    }
}

impl Display for ReconcileError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Config(error) => error.fmt(formatter),
            Self::Git(error) => error.fmt(formatter),
            Self::Ledger(error) => error.fmt(formatter),
            Self::Replay(error) => error.fmt(formatter),
            Self::EdgeReplay(error) => error.fmt(formatter),
            Self::MissingReadinessFact(error) => error.fmt(formatter),
            Self::Transaction(error) => error.fmt(formatter),
            Self::WorktreeRegistry(error) => error.fmt(formatter),
        }
    }
}

impl Error for ReconcileError {}

impl From<ConfigError> for ReconcileError {
    fn from(error: ConfigError) -> Self { Self::Config(error) }
}

impl From<GitError> for ReconcileError {
    fn from(error: GitError) -> Self { Self::Git(error) }
}

impl From<LedgerError> for ReconcileError {
    fn from(error: LedgerError) -> Self { Self::Ledger(error) }
}

impl From<WorktreeRegistryError> for ReconcileError {
    fn from(error: WorktreeRegistryError) -> Self { Self::WorktreeRegistry(error) }
}

impl From<ReconciliationPlanningError> for ReconcileError {
    fn from(error: ReconciliationPlanningError) -> Self {
        match error {
            ReconciliationPlanningError::Reservation(error) => Self::Replay(error),
            ReconciliationPlanningError::Edge(error) => Self::EdgeReplay(error),
            ReconciliationPlanningError::WorktreeRegistry(error) => Self::WorktreeRegistry(error),
            ReconciliationPlanningError::PendingBypass(error) => {
                Self::Ledger(LedgerError::Io(error))
            },
        }
    }
}
