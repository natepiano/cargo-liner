//! Shared liveness, evidence, retention-ref, and marker reconciliation.

use std::collections::HashMap;
use std::collections::HashSet;
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
use crate::edge::PredecessorSuccessorIncorporation;
use crate::edge::RepositoryReservationEvidence;
use crate::edge::RepositoryReservationSnapshot;
use crate::edge::RepositorySnapshot;
use crate::edge::RepositoryTrunk;
use crate::edge::SuccessorIncorporationEvidence;
use crate::gate::permit;
use crate::gate::permit::PendingBypassMarkerImport;
use crate::gate::permit::RecoveredPendingBypassMarker;
use crate::git;
use crate::git::CandidateHeadReachability;
use crate::git::DescendantCommitQuery;
use crate::git::GitError;
use crate::git::ProtectedTipSuccessorHeads;
use crate::git::Reachability;
use crate::git::ScopedPatchComparison;
use crate::ids::CoordinationRunId;
use crate::ids::GitObjectId;
use crate::ids::JournalByteOffset;
use crate::ids::ProjectionGeneration;
use crate::ids::RepoInstanceId;
use crate::ids::ReservationId;
use crate::ids::ReservationScopePath;
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
use crate::reservation::DeferredScopedPatchIntegrationStatus;
use crate::reservation::DurableScopedPatchComparison;
use crate::reservation::EditBlockingStatus;
use crate::reservation::IntegrationEvidenceObservation;
use crate::reservation::IntegrationEvidenceStatus;
use crate::reservation::IntegrationProof;
use crate::reservation::IntegrationProofSubjectRevision;
use crate::reservation::PriorIntegrationStatus;
use crate::reservation::ProtectedReservationTip;
use crate::reservation::ReleaseDisposition;
use crate::reservation::ReleaseRevalidationSubject;
use crate::reservation::Reservation;
use crate::reservation::ReservationEvidenceState;
use crate::reservation::ReservationLifecycle;
use crate::reservation::ReservationReplayError;
use crate::reservation::RetainedReservationSet;
use crate::reservation::ScopedPatchComparisonObservation;
use crate::reservation::ScopedPatchEquivalenceCacheLookup;
use crate::reservation::ScopedPatchEquivalenceVerdict;
use crate::reservation::ScopedPatchEvaluationPriority;
use crate::reservation::SuccessorScopedPatchEquivalenceCacheLookup;
use crate::reservation::SuccessorScopedPatchEquivalenceVerdict;
use crate::scope::ScopeKind;
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

struct ReconciliationEvidenceContext<'context> {
    berth_config:                             &'context BerthConfig,
    scoped_patch_evaluation_memo:             &'context mut ScopedPatchEvaluationMemo,
    successor_scoped_patch_evaluation_budget: &'context mut SuccessorScopedPatchEvaluationBudget,
}

struct TargetIntegrationEvidenceContext<'context> {
    repository_root:              &'context Path,
    repository_trunk:             &'context RepositoryTrunk,
    integration_reachability:     &'context BatchedIntegrationReachability,
    scoped_patch_evaluation_memo: &'context mut ScopedPatchEvaluationMemo,
}

#[derive(Clone)]
struct RepositoryEvidenceObservation {
    evidence:                RepositoryReservationEvidence,
    revalidation:            EvidenceRevalidationObservation,
    scoped_patch_comparison: ScopedPatchComparisonJournalUpdate,
}

struct IntegrationStatusObservation {
    status:                  IntegrationEvidenceStatus,
    revalidation:            EvidenceRevalidationObservation,
    scoped_patch_comparison: ScopedPatchComparisonJournalUpdate,
}

impl From<DeferredScopedPatchIntegrationStatus> for IntegrationStatusObservation {
    fn from(deferred_status: DeferredScopedPatchIntegrationStatus) -> Self {
        match deferred_status {
            DeferredScopedPatchIntegrationStatus::StillValid(status) => Self {
                status,
                revalidation: EvidenceRevalidationObservation::PreserveMaterialized,
                scoped_patch_comparison: ScopedPatchComparisonJournalUpdate::Unchanged,
            },
            DeferredScopedPatchIntegrationStatus::Degraded(status) => Self {
                status,
                revalidation: EvidenceRevalidationObservation::Apply,
                scoped_patch_comparison: ScopedPatchComparisonJournalUpdate::Unchanged,
            },
        }
    }
}

/// Every integration-proof ancestor classified against one immutable trunk target.
struct BatchedIntegrationReachability {
    by_ancestor: HashMap<GitObjectId, Reachability>,
}

impl BatchedIntegrationReachability {
    fn observe(
        repository_root: &Path,
        reservations: &RetainedReservationSet,
        repository_trunk: &RepositoryTrunk,
    ) -> Result<Self, ReservationReplayError> {
        let RepositoryTrunk::Resolved(target) = repository_trunk else {
            return Ok(Self {
                by_ancestor: HashMap::new(),
            });
        };
        let mut candidate_ancestors = HashSet::new();
        for reservation in reservations.iter() {
            match reservation.evidence_state()? {
                ReservationEvidenceState::Outstanding {
                    protected_tip,
                    trunk_snapshot,
                    ..
                } => {
                    candidate_ancestors.insert(protected_tip.as_ref().clone());
                    candidate_ancestors.insert(trunk_snapshot);
                },
                ReservationEvidenceState::Released {
                    protected_tip,
                    disposition,
                    ..
                } => match disposition.revalidation_subject() {
                    ReleaseRevalidationSubject::ProtectedTip => {
                        candidate_ancestors.insert(protected_tip.as_ref().clone());
                    },
                    ReleaseRevalidationSubject::RewrittenIntegration(trunk_commit) => {
                        candidate_ancestors.insert(trunk_commit.as_ref().clone());
                    },
                    ReleaseRevalidationSubject::None => {},
                },
                ReservationEvidenceState::Active { .. }
                | ReservationEvidenceState::ReleasedWithoutCheckpoint { .. } => {},
            }
        }
        let candidate_ancestors = candidate_ancestors.into_iter().collect::<Vec<_>>();
        let reachability =
            git::reachability_to_target(repository_root, &candidate_ancestors, target)
                .unwrap_or_else(|_| vec![Reachability::ObjectUnknown; candidate_ancestors.len()]);
        Ok(Self {
            by_ancestor: candidate_ancestors.into_iter().zip(reachability).collect(),
        })
    }

    fn for_ancestor(&self, ancestor: &GitObjectId) -> Reachability {
        self.by_ancestor
            .get(ancestor)
            .copied()
            .unwrap_or(Reachability::ObjectUnknown)
    }
}

#[derive(Clone, Copy)]
enum EvidenceRevalidationObservation {
    /// Current git evidence or a durable cache verdict supplies the status.
    Apply,
    /// The bounded scoped comparison did not run, so materialized evidence was retained.
    PreserveMaterialized,
    /// The lifecycle has no git evidence subject to revalidate.
    NotApplicable,
}

#[derive(Clone)]
enum ScopedPatchComparisonJournalUpdate {
    /// No new definitive verdict needs a journal record.
    Unchanged,
    /// A comparison ran but produced no durable cache verdict.
    Attempted {
        subject: IntegrationProofSubjectRevision,
        target:  GitObjectId,
    },
    /// A cache miss produced a definitive verdict for this subject and target.
    Checked {
        subject: IntegrationProofSubjectRevision,
        target:  GitObjectId,
        verdict: ScopedPatchEquivalenceVerdict,
    },
}

#[derive(Eq, Hash, PartialEq)]
struct ScopedPatchEvaluationKey {
    phase_start_head: GitObjectId,
    protected_tip:    GitObjectId,
    target_trunk:     GitObjectId,
    scopes:           Vec<ScopedPatchEvaluationScope>,
    context:          ScopedPatchEvaluationContext,
}

struct ProposedTrunkObservation {
    snapshot:   RepositorySnapshot,
    operations: Vec<JournalOperation>,
}

#[derive(Eq, Hash, PartialEq)]
struct ScopedPatchEvaluationScope {
    path:       ReservationScopePath,
    scope_kind: ScopedPatchEvaluationScopeKind,
}

#[derive(Clone, Copy, Eq, Hash, PartialEq)]
enum ScopedPatchEvaluationScopeKind {
    File,
    Tree,
}

impl From<ScopeKind> for ScopedPatchEvaluationScopeKind {
    fn from(scope_kind: ScopeKind) -> Self {
        match scope_kind {
            ScopeKind::File => Self::File,
            ScopeKind::Tree => Self::Tree,
        }
    }
}

#[derive(Clone, Eq, Hash, PartialEq)]
enum ScopedPatchEvaluationContext {
    PriorIntegrationProven,
    Outstanding { previous_trunk: GitObjectId },
}

/// Reuses identical proof inputs and admits one scoped comparison per trunk target.
#[derive(Default)]
struct ScopedPatchEvaluationMemo {
    comparisons:       HashMap<ScopedPatchEvaluationKey, ScopedPatchComparison>,
    evaluated_targets: HashSet<GitObjectId>,
}

/// A fixed per-reconciliation budget shared by every pending successor target.
///
/// Reachability remains batched for all heads. Scoped equivalence is deliberately admitted once,
/// and durable attempt generations rotate the next cold target to the front on later passes.
#[derive(Default)]
struct SuccessorScopedPatchEvaluationBudget {
    comparison_performed: bool,
}

impl SuccessorScopedPatchEvaluationBudget {
    fn evaluate(
        &mut self,
        evaluate: impl FnOnce() -> ScopedPatchComparison,
    ) -> SuccessorScopedPatchComparisonObservation {
        if self.comparison_performed {
            SuccessorScopedPatchComparisonObservation::Deferred
        } else {
            self.comparison_performed = true;
            SuccessorScopedPatchComparisonObservation::Observed(evaluate())
        }
    }
}

enum SuccessorScopedPatchComparisonObservation {
    Observed(ScopedPatchComparison),
    Deferred,
}

struct PredecessorSuccessorEvidenceSubject<'reservation> {
    reservation:               &'reservation Reservation,
    prior_integration_status:  PriorIntegrationStatus,
    protected_reservation_tip: ProtectedReservationTip,
    successor_heads:           Vec<GitObjectId>,
}

struct SuccessorScopedPatchEvaluationCandidate {
    predecessor_index:          usize,
    predecessor_reservation_id: ReservationId,
    subject:                    IntegrationProofSubjectRevision,
    phase_start_head:           GitObjectId,
    scopes:                     crate::scope::ReservationScopeSet,
    protected_tip:              GitObjectId,
    successor_head:             GitObjectId,
    priority:                   ScopedPatchEvaluationPriority,
}

struct SuccessorIncorporationObservation {
    by_predecessor: Vec<(ReservationId, PredecessorSuccessorIncorporation)>,
    operations:     Vec<JournalOperation>,
}

impl ScopedPatchEvaluationMemo {
    fn evaluate(
        &mut self,
        scoped_patch_evaluation_key: ScopedPatchEvaluationKey,
        evaluate: impl FnOnce() -> ScopedPatchComparison,
    ) -> ScopedPatchComparisonObservation {
        if let Some(scoped_patch_comparison) = self.comparisons.get(&scoped_patch_evaluation_key) {
            return ScopedPatchComparisonObservation::Observed(*scoped_patch_comparison);
        }
        if !self
            .evaluated_targets
            .insert(scoped_patch_evaluation_key.target_trunk.clone())
        {
            return ScopedPatchComparisonObservation::Deferred;
        }
        let scoped_patch_comparison = evaluate();
        self.comparisons
            .insert(scoped_patch_evaluation_key, scoped_patch_comparison);
        ScopedPatchComparisonObservation::Observed(scoped_patch_comparison)
    }
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
    retention_repairs:   Vec<git::ReservationRetentionRefRepair>,
    retention_deletions: Vec<ReservationId>,
    evidence:            Vec<ReconciledEvidence>,
}

struct ReconciliationAction {
    active_holders:                Vec<ActiveHolder>,
    marker_contexts:               Vec<WorktreeContext>,
    repository_root:               PathBuf,
    retention_repairs:             Vec<git::ReservationRetentionRefRepair>,
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
    let journal_mutation_actor = ledger::resolve_identity(worktree_context)?
        .journal_mutation_actor_for(CoordinationRunId::new());
    let outcome = ledger
        .transact_reconciliation(
            journal_mutation_actor.worktree_id,
            journal_mutation_actor.coordination_run_id,
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
                let mut scoped_patch_evaluation_memo = ScopedPatchEvaluationMemo::default();
                let mut successor_scoped_patch_evaluation_budget =
                    SuccessorScopedPatchEvaluationBudget::default();
                let mut reconciliation_evidence_context = ReconciliationEvidenceContext {
                    berth_config,
                    scoped_patch_evaluation_memo: &mut scoped_patch_evaluation_memo,
                    successor_scoped_patch_evaluation_budget:
                        &mut successor_scoped_patch_evaluation_budget,
                };
                let mut reconciliation_plan = match build_plan(
                    &reservations,
                    &ordering_graph,
                    repository_observation_scope,
                    &worktree_registry,
                    ledger_repository,
                    worktree_context,
                    &mut reconciliation_evidence_context,
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

#[allow(
    clippy::too_many_lines,
    reason = "one locked observation keeps repository facts and their journal updates coherent"
)]
fn build_plan(
    reservations: &RetainedReservationSet,
    ordering_graph: &OrderingGraph,
    repository_observation_scope: RepositoryObservationScope,
    worktree_registry: &WorktreeRegistry,
    ledger_repository: RepoInstanceId,
    worktree_context: &WorktreeContext,
    reconciliation_evidence_context: &mut ReconciliationEvidenceContext<'_>,
) -> Result<ReconciliationPlan, ReservationReplayError> {
    let common_git_directory = worktree_context.common_git_directory();
    let repository_root = worktree_context.repository_root();
    let mut changes = ReconciliationChanges::default();
    let mut alert_subjects = Vec::new();
    let mut trunk_resolution_calls = 0;
    trunk_resolution_calls += 1;
    let repository_trunk = reservation::current_trunk(
        repository_root,
        &reconciliation_evidence_context.berth_config.trunk,
    )
    .map_or(RepositoryTrunk::ObjectUnknown, RepositoryTrunk::Resolved);
    let integration_reachability =
        BatchedIntegrationReachability::observe(repository_root, reservations, &repository_trunk)?;
    let mut target_evidence_context = TargetIntegrationEvidenceContext {
        repository_root,
        repository_trunk: &repository_trunk,
        integration_reachability: &integration_reachability,
        scoped_patch_evaluation_memo: reconciliation_evidence_context.scoped_patch_evaluation_memo,
    };
    let mut indexed_evidence = scoped_patch_evaluation_order(reservations, &repository_trunk)
        .into_iter()
        .map(|(index, reservation)| {
            repository_evidence(reservation, &mut target_evidence_context)
                .map(|observation| (index, observation))
        })
        .collect::<Result<Vec<_>, ReservationReplayError>>()?;
    indexed_evidence.sort_by_key(|(index, _)| *index);
    let repository_evidence_observations = indexed_evidence
        .into_iter()
        .map(|indexed_observation| indexed_observation.1);
    let mut reservation_snapshots = Vec::new();
    for (reservation, repository_evidence_observation) in
        reservations.iter().zip(repository_evidence_observations)
    {
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
        append_evidence_and_retention(
            reservation,
            &repository_evidence_observation,
            reservations,
            ordering_graph,
            &mut changes,
        )?;
        reservation_snapshots.push(RepositoryReservationSnapshot {
            reservation_id:    reservation.id(),
            worktree_liveness: observation.liveness,
            worktree_head:     observation.head,
            evidence:          repository_evidence_observation.evidence,
        });
    }
    let successor_incorporation = successor_incorporation_evidence(
        repository_root,
        reservations,
        ordering_graph,
        repository_observation_scope,
        &reservation_snapshots,
        reconciliation_evidence_context.successor_scoped_patch_evaluation_budget,
    )?;
    changes
        .operations
        .extend(successor_incorporation.operations);
    let repository_snapshot = RepositorySnapshot::new(
        repository_trunk,
        reservation_snapshots,
        successor_incorporation.by_predecessor,
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

fn scoped_patch_evaluation_order<'reservation>(
    reservations: &'reservation RetainedReservationSet,
    repository_trunk: &RepositoryTrunk,
) -> Vec<(usize, &'reservation Reservation)> {
    let mut evaluation_order = reservations.iter().enumerate().collect::<Vec<_>>();
    if let RepositoryTrunk::Resolved(target) = repository_trunk {
        evaluation_order
            .sort_by_key(|(_, reservation)| reservation.scoped_patch_evaluation_priority(target));
    }
    evaluation_order
}

/// Prepare the actual reconciliation and proposed-ref constraint read from one replay.
///
/// Each observed trunk target uses one `cat-file` batch and one grouped `rev-list` to classify all
/// integration-proof ancestors. Graph predecessor queries use one grouped `rev-list` for every
/// protected tip and successor head. Retention repair uses one `cat-file` batch and one
/// `update-ref` transaction. These invocation counts are independent of the total retained-
/// reservation count. Scoped patch comparisons reuse identical proof inputs and evaluate at most
/// one distinct proof subject for each observed trunk target.
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
    let mut scoped_patch_evaluation_memo = ScopedPatchEvaluationMemo::default();
    let mut successor_scoped_patch_evaluation_budget =
        SuccessorScopedPatchEvaluationBudget::default();
    let mut reconciliation_evidence_context = ReconciliationEvidenceContext {
        berth_config,
        scoped_patch_evaluation_memo: &mut scoped_patch_evaluation_memo,
        successor_scoped_patch_evaluation_budget: &mut successor_scoped_patch_evaluation_budget,
    };
    let mut reconciliation = build_plan(
        &reservations,
        &ordering_graph,
        RepositoryObservationScope::CurrentOrderingGraph,
        &worktree_registry,
        ledger_repository,
        worktree_context,
        &mut reconciliation_evidence_context,
    )
    .map_err(GateReconciliationError::Reservation)?;
    let proposed_observation = observe_proposed_trunk(
        &reservations,
        &ordering_graph,
        RepositoryObservationScope::CurrentOrderingGraph,
        &reconciliation.action.repository_snapshot,
        worktree_context,
        proposed_trunk,
        &mut reconciliation_evidence_context,
    )?;
    for operation in proposed_observation.operations {
        if !reconciliation.operations.contains(&operation) {
            reconciliation.operations.push(operation);
        }
    }
    let constraints = ordering_graph
        .integration_constraints(&reservations, &proposed_observation.snapshot, generation)
        .map_err(GateReconciliationError::MissingReadinessFact)?;
    Ok(GateReconciliation {
        reconciliation,
        constraints,
        reservations,
    })
}

fn observe_proposed_trunk(
    reservations: &RetainedReservationSet,
    ordering_graph: &OrderingGraph,
    repository_observation_scope: RepositoryObservationScope,
    actual_snapshot: &RepositorySnapshot,
    worktree_context: &WorktreeContext,
    proposed_trunk: GitObjectId,
    reconciliation_evidence_context: &mut ReconciliationEvidenceContext<'_>,
) -> Result<ProposedTrunkObservation, GateReconciliationError> {
    let repository_trunk = RepositoryTrunk::Resolved(proposed_trunk);
    let integration_reachability = BatchedIntegrationReachability::observe(
        worktree_context.repository_root(),
        reservations,
        &repository_trunk,
    )
    .map_err(GateReconciliationError::Reservation)?;
    let mut target_evidence_context = TargetIntegrationEvidenceContext {
        repository_root:              worktree_context.repository_root(),
        repository_trunk:             &repository_trunk,
        integration_reachability:     &integration_reachability,
        scoped_patch_evaluation_memo: reconciliation_evidence_context.scoped_patch_evaluation_memo,
    };
    let mut indexed_evidence = scoped_patch_evaluation_order(reservations, &repository_trunk)
        .into_iter()
        .map(|(index, reservation)| {
            repository_evidence(reservation, &mut target_evidence_context)
                .map(|observation| (index, reservation, observation))
                .map_err(GateReconciliationError::Reservation)
        })
        .collect::<Result<Vec<_>, GateReconciliationError>>()?;
    indexed_evidence.sort_by_key(|(index, _, _)| *index);
    let mut operations = Vec::new();
    let reservation_snapshots = indexed_evidence
        .into_iter()
        .map(|(_, reservation, repository_evidence_observation)| {
            let actual_reservation = actual_snapshot
                .reservation(reservation.id())
                .map_err(GateReconciliationError::MissingReadinessFact)?;
            append_scoped_patch_journal_update(
                reservation,
                &repository_evidence_observation.scoped_patch_comparison,
                &mut operations,
            );
            Ok(RepositoryReservationSnapshot {
                reservation_id:    reservation.id(),
                worktree_liveness: actual_reservation.worktree_liveness,
                worktree_head:     actual_reservation.worktree_head.clone(),
                evidence:          repository_evidence_observation.evidence,
            })
        })
        .collect::<Result<Vec<_>, GateReconciliationError>>()?;
    let successor_incorporation = successor_incorporation_evidence(
        worktree_context.repository_root(),
        reservations,
        ordering_graph,
        repository_observation_scope,
        &reservation_snapshots,
        reconciliation_evidence_context.successor_scoped_patch_evaluation_budget,
    )
    .map_err(GateReconciliationError::Reservation)?;
    operations.extend(successor_incorporation.operations);
    Ok(ProposedTrunkObservation {
        snapshot: RepositorySnapshot::new(
            repository_trunk,
            reservation_snapshots,
            successor_incorporation.by_predecessor,
        ),
        operations,
    })
}

impl GateReconciliation {
    /// Borrow the shared gate-and-board projection prepared at this generation.
    pub(crate) const fn constraints(&self) -> &IntegrationConstraintProjection { &self.constraints }

    /// Borrow reservations when a stateful caller validates its marker-derived actor.
    pub(crate) const fn reservations(&self) -> &RetainedReservationSet { &self.reservations }

    /// Retain durable content checks while leaving actual-trunk lifecycle updates to commands.
    pub(crate) fn into_committed_hook_operations(
        self,
        additional_operations: Vec<JournalOperation>,
    ) -> Vec<JournalOperation> {
        let mut operations = self
            .reconciliation
            .operations
            .into_iter()
            .filter(|operation| {
                // These records contain immutable subject/target facts or comparison ordering
                // only. Applying them without `EvidenceRevalidated` cannot affirm integration or
                // change lifecycle state, while retaining them prevents the committed hook from
                // repeating the same bounded comparison.
                matches!(
                    operation,
                    JournalOperation::ScopedPatchEquivalenceChecked { .. }
                        | JournalOperation::ScopedPatchComparisonAttempted { .. }
                        | JournalOperation::SuccessorScopedPatchEquivalenceChecked { .. }
                        | JournalOperation::SuccessorScopedPatchComparisonAttempted { .. }
                )
            })
            .collect::<Vec<_>>();
        operations.extend(additional_operations);
        operations
    }

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
    reservation: &Reservation,
    target_evidence_context: &mut TargetIntegrationEvidenceContext<'_>,
) -> Result<RepositoryEvidenceObservation, ReservationReplayError> {
    match reservation.evidence_state()? {
        ReservationEvidenceState::Active { .. } => Ok(RepositoryEvidenceObservation {
            evidence:                RepositoryReservationEvidence::Active,
            revalidation:            EvidenceRevalidationObservation::NotApplicable,
            scoped_patch_comparison: ScopedPatchComparisonJournalUpdate::Unchanged,
        }),
        ReservationEvidenceState::Outstanding {
            protected_tip,
            trunk_snapshot,
            integration_status: materialized,
        } => Ok(observe_outstanding_repository_evidence(
            target_evidence_context,
            reservation,
            protected_tip,
            &trunk_snapshot,
            &materialized,
        )),
        ReservationEvidenceState::Released {
            protected_tip,
            disposition,
            integration_status: materialized,
            ..
        } => Ok(observe_released_repository_evidence(
            target_evidence_context,
            reservation,
            protected_tip,
            disposition,
            materialized,
        )),
        ReservationEvidenceState::ReleasedWithoutCheckpoint { disposition } => {
            Ok(RepositoryEvidenceObservation {
                evidence:                RepositoryReservationEvidence::ReleasedWithoutCheckpoint {
                    disposition,
                },
                revalidation:            EvidenceRevalidationObservation::NotApplicable,
                scoped_patch_comparison: ScopedPatchComparisonJournalUpdate::Unchanged,
            })
        },
    }
}

fn observe_outstanding_repository_evidence(
    target_evidence_context: &mut TargetIntegrationEvidenceContext<'_>,
    reservation: &Reservation,
    protected_tip: ProtectedReservationTip,
    trunk_snapshot: &GitObjectId,
    materialized: &IntegrationEvidenceStatus,
) -> RepositoryEvidenceObservation {
    let observation = match target_evidence_context.repository_trunk {
        RepositoryTrunk::Resolved(current_trunk_oid) => {
            let scoped_patch_evaluation_context =
                if matches!(materialized, IntegrationEvidenceStatus::Integrated { .. }) {
                    ScopedPatchEvaluationContext::PriorIntegrationProven
                } else {
                    ScopedPatchEvaluationContext::Outstanding {
                        previous_trunk: trunk_snapshot.clone(),
                    }
                };
            integration_status_with_cache(
                target_evidence_context,
                reservation,
                &protected_tip,
                current_trunk_oid,
                scoped_patch_evaluation_context,
                materialized,
            )
        },
        RepositoryTrunk::ObjectUnknown => IntegrationStatusObservation {
            status:                  IntegrationEvidenceStatus::ObjectUnknown,
            revalidation:            EvidenceRevalidationObservation::Apply,
            scoped_patch_comparison: ScopedPatchComparisonJournalUpdate::Unchanged,
        },
    };
    RepositoryEvidenceObservation {
        evidence:                RepositoryReservationEvidence::Outstanding {
            protected_tip,
            integration_status: observation.status,
        },
        revalidation:            observation.revalidation,
        scoped_patch_comparison: observation.scoped_patch_comparison,
    }
}

fn observe_released_repository_evidence(
    target_evidence_context: &mut TargetIntegrationEvidenceContext<'_>,
    reservation: &Reservation,
    protected_tip: ProtectedReservationTip,
    disposition: ReleaseDisposition,
    materialized: IntegrationEvidenceStatus,
) -> RepositoryEvidenceObservation {
    let observation = match disposition.revalidation_subject() {
        ReleaseRevalidationSubject::ProtectedTip => revalidate_release(
            target_evidence_context,
            reservation,
            &protected_tip,
            &materialized,
        ),
        ReleaseRevalidationSubject::RewrittenIntegration(trunk_commit) => {
            let revalidation_tip = ProtectedReservationTip::from(trunk_commit.as_ref().clone());
            revalidate_release(
                target_evidence_context,
                reservation,
                &revalidation_tip,
                &materialized,
            )
        },
        ReleaseRevalidationSubject::None => IntegrationStatusObservation {
            status:                  materialized,
            revalidation:            EvidenceRevalidationObservation::NotApplicable,
            scoped_patch_comparison: ScopedPatchComparisonJournalUpdate::Unchanged,
        },
    };
    RepositoryEvidenceObservation {
        evidence:                RepositoryReservationEvidence::Released {
            protected_tip,
            disposition,
            integration_status: observation.status,
        },
        revalidation:            observation.revalidation,
        scoped_patch_comparison: observation.scoped_patch_comparison,
    }
}

fn revalidate_release(
    target_evidence_context: &mut TargetIntegrationEvidenceContext<'_>,
    reservation: &Reservation,
    protected_tip: &ProtectedReservationTip,
    materialized: &IntegrationEvidenceStatus,
) -> IntegrationStatusObservation {
    match target_evidence_context.repository_trunk {
        RepositoryTrunk::Resolved(current_trunk_oid) => integration_status_with_cache(
            target_evidence_context,
            reservation,
            protected_tip,
            current_trunk_oid,
            ScopedPatchEvaluationContext::PriorIntegrationProven,
            materialized,
        ),
        RepositoryTrunk::ObjectUnknown => IntegrationStatusObservation {
            status:                  IntegrationEvidenceStatus::ObjectUnknown,
            revalidation:            EvidenceRevalidationObservation::Apply,
            scoped_patch_comparison: ScopedPatchComparisonJournalUpdate::Unchanged,
        },
    }
}

fn integration_status_with_cache(
    target_evidence_context: &mut TargetIntegrationEvidenceContext<'_>,
    reservation: &Reservation,
    protected_tip: &ProtectedReservationTip,
    target: &GitObjectId,
    scoped_patch_evaluation_context: ScopedPatchEvaluationContext,
    materialized: &IntegrationEvidenceStatus,
) -> IntegrationStatusObservation {
    let subject = reservation.integration_proof_subject_revision();
    match reservation
        .scoped_patch_equivalence_cache()
        .lookup(subject, target)
    {
        ScopedPatchEquivalenceCacheLookup::Hit(scoped_patch_comparison) => {
            IntegrationStatusObservation {
                status:                  integration_status_from_cached_scoped_patch_comparison(
                    scoped_patch_comparison,
                    target,
                    &scoped_patch_evaluation_context,
                    target_evidence_context.integration_reachability,
                ),
                revalidation:            EvidenceRevalidationObservation::Apply,
                scoped_patch_comparison: ScopedPatchComparisonJournalUpdate::Unchanged,
            }
        },
        ScopedPatchEquivalenceCacheLookup::Miss => {
            let repository_root = target_evidence_context.repository_root;
            let integration_reachability = target_evidence_context.integration_reachability;
            let scoped_patch_evaluation_memo =
                &mut *target_evidence_context.scoped_patch_evaluation_memo;
            let scoped_patch_evaluation_key = ScopedPatchEvaluationKey {
                phase_start_head: reservation.phase_start_head().as_ref().clone(),
                protected_tip:    protected_tip.as_ref().clone(),
                target_trunk:     target.clone(),
                scopes:           reservation
                    .scopes()
                    .as_slice()
                    .iter()
                    .map(|scope| ScopedPatchEvaluationScope {
                        path:       scope.path.clone(),
                        scope_kind: scope.kind.into(),
                    })
                    .collect(),
                context:          scoped_patch_evaluation_context.clone(),
            };
            let observe_scoped_patch_comparison = || {
                scoped_patch_evaluation_memo.evaluate(scoped_patch_evaluation_key, || {
                    git::scoped_patch_equivalence(
                        repository_root,
                        reservation.phase_start_head().as_ref(),
                        reservation.scopes(),
                        protected_tip.as_ref(),
                        target,
                    )
                    .unwrap_or(ScopedPatchComparison::Unavailable)
                })
            };
            let evidence_observation = match scoped_patch_evaluation_context {
                ScopedPatchEvaluationContext::PriorIntegrationProven => {
                    reservation::observe_integration_status(
                        integration_reachability.for_ancestor(protected_tip.as_ref()),
                        target,
                        PriorIntegrationStatus::Proven,
                        materialized,
                        observe_scoped_patch_comparison,
                    )
                },
                ScopedPatchEvaluationContext::Outstanding { previous_trunk } => {
                    reservation::observe_outstanding_integration_status(
                        integration_reachability.for_ancestor(protected_tip.as_ref()),
                        integration_reachability.for_ancestor(&previous_trunk),
                        target,
                        materialized,
                        observe_scoped_patch_comparison,
                    )
                },
            };
            match evidence_observation {
                IntegrationEvidenceObservation::Reachability(status) => {
                    IntegrationStatusObservation {
                        status,
                        revalidation: EvidenceRevalidationObservation::Apply,
                        scoped_patch_comparison: ScopedPatchComparisonJournalUpdate::Unchanged,
                    }
                },
                IntegrationEvidenceObservation::ScopedPatchComparison(status) => {
                    let scoped_patch_comparison =
                        scoped_patch_journal_update(subject, target, &status);
                    IntegrationStatusObservation {
                        status,
                        revalidation: EvidenceRevalidationObservation::Apply,
                        scoped_patch_comparison,
                    }
                },
                IntegrationEvidenceObservation::ScopedPatchComparisonDeferred(status) => {
                    status.into()
                },
            }
        },
    }
}

fn scoped_patch_journal_update(
    subject: IntegrationProofSubjectRevision,
    target: &GitObjectId,
    status: &IntegrationEvidenceStatus,
) -> ScopedPatchComparisonJournalUpdate {
    match status {
        IntegrationEvidenceStatus::Integrated {
            proof: IntegrationProof::ScopedPatchEquivalent,
            ..
        } => ScopedPatchComparisonJournalUpdate::Checked {
            subject,
            target: target.clone(),
            verdict: ScopedPatchEquivalenceVerdict::Integrated,
        },
        IntegrationEvidenceStatus::TrunkRewritten => ScopedPatchComparisonJournalUpdate::Checked {
            subject,
            target: target.clone(),
            verdict: ScopedPatchEquivalenceVerdict::TrunkRewritten,
        },
        IntegrationEvidenceStatus::NotIntegrated => ScopedPatchComparisonJournalUpdate::Checked {
            subject,
            target: target.clone(),
            verdict: ScopedPatchEquivalenceVerdict::NotIntegrated,
        },
        IntegrationEvidenceStatus::ObjectUnknown => ScopedPatchComparisonJournalUpdate::Attempted {
            subject,
            target: target.clone(),
        },
        IntegrationEvidenceStatus::Integrated {
            proof: IntegrationProof::ProtectedTipAncestor,
            ..
        } => ScopedPatchComparisonJournalUpdate::Unchanged,
    }
}

fn integration_status_from_cached_scoped_patch_comparison(
    scoped_patch_comparison: DurableScopedPatchComparison,
    target: &GitObjectId,
    scoped_patch_evaluation_context: &ScopedPatchEvaluationContext,
    integration_reachability: &BatchedIntegrationReachability,
) -> IntegrationEvidenceStatus {
    match scoped_patch_comparison {
        DurableScopedPatchComparison::Equivalent => IntegrationEvidenceStatus::Integrated {
            trunk_oid: target.clone(),
            proof:     IntegrationProof::ScopedPatchEquivalent,
        },
        DurableScopedPatchComparison::Different => match scoped_patch_evaluation_context {
            ScopedPatchEvaluationContext::PriorIntegrationProven => {
                IntegrationEvidenceStatus::TrunkRewritten
            },
            ScopedPatchEvaluationContext::Outstanding { previous_trunk } => {
                match integration_reachability.for_ancestor(previous_trunk) {
                    Reachability::Ancestor => IntegrationEvidenceStatus::NotIntegrated,
                    Reachability::NotAncestor => IntegrationEvidenceStatus::TrunkRewritten,
                    Reachability::ObjectUnknown => IntegrationEvidenceStatus::ObjectUnknown,
                }
            },
        },
    }
}

fn append_evidence_and_retention(
    reservation: &Reservation,
    repository_evidence_observation: &RepositoryEvidenceObservation,
    reservations: &RetainedReservationSet,
    ordering_graph: &OrderingGraph,
    changes: &mut ReconciliationChanges,
) -> Result<(), ReservationReplayError> {
    let (protected_tip, evidence, retention) = match &repository_evidence_observation.evidence {
        RepositoryReservationEvidence::Active
        | RepositoryReservationEvidence::ReleasedWithoutCheckpoint { .. } => return Ok(()),
        RepositoryReservationEvidence::Outstanding {
            protected_tip,
            integration_status,
        } => (protected_tip, integration_status, RetentionDecision::Repair),
        RepositoryReservationEvidence::Released {
            protected_tip,
            integration_status,
            ..
        } => {
            let retention =
                if ordering_graph.has_nonterminal_dependent(reservation.id(), reservations)? {
                    RetentionDecision::Repair
                } else {
                    RetentionDecision::Delete
                };
            (protected_tip, integration_status, retention)
        },
    };
    match retention {
        RetentionDecision::Repair => {
            changes
                .retention_repairs
                .push(git::ReservationRetentionRefRepair::new(
                    reservation.id(),
                    protected_tip.as_ref().clone(),
                ));
        },
        RetentionDecision::Delete => changes.retention_deletions.push(reservation.id()),
    }
    let evidence_revalidation = match repository_evidence_observation.revalidation {
        EvidenceRevalidationObservation::Apply => EvidenceRevalidation::Required(evidence),
        EvidenceRevalidationObservation::PreserveMaterialized => {
            EvidenceRevalidation::PreserveMaterialized
        },
        EvidenceRevalidationObservation::NotApplicable => EvidenceRevalidation::NotApplicable,
    };
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
    append_scoped_patch_journal_update(
        reservation,
        &repository_evidence_observation.scoped_patch_comparison,
        &mut changes.operations,
    );
    Ok(())
}

fn append_scoped_patch_journal_update(
    reservation: &Reservation,
    scoped_patch_comparison: &ScopedPatchComparisonJournalUpdate,
    operations: &mut Vec<JournalOperation>,
) {
    match scoped_patch_comparison {
        ScopedPatchComparisonJournalUpdate::Unchanged => {},
        ScopedPatchComparisonJournalUpdate::Attempted { subject, target } => {
            match reservation.scoped_patch_evaluation_priority(target) {
                ScopedPatchEvaluationPriority::NotAttempted => {
                    operations.push(JournalOperation::ScopedPatchComparisonAttempted {
                        reservation_id: reservation.id(),
                        subject:        *subject,
                        target:         target.clone(),
                    });
                },
                ScopedPatchEvaluationPriority::LastAttemptedAt(_) => {},
            }
        },
        ScopedPatchComparisonJournalUpdate::Checked {
            subject,
            target,
            verdict,
        } => operations.push(JournalOperation::ScopedPatchEquivalenceChecked {
            reservation_id: reservation.id(),
            subject:        *subject,
            target:         target.clone(),
            verdict:        *verdict,
        }),
    }
}

#[derive(Clone, Copy)]
enum RetentionDecision {
    Repair,
    Delete,
}

enum EvidenceRevalidation<'evidence> {
    Required(&'evidence IntegrationEvidenceStatus),
    PreserveMaterialized,
    NotApplicable,
}

#[allow(
    clippy::too_many_lines,
    reason = "one grouped pass keeps reachability, cache lookup, scheduling, and snapshot facts coherent"
)]
fn successor_incorporation_evidence(
    repository_root: &Path,
    reservations: &RetainedReservationSet,
    ordering_graph: &OrderingGraph,
    repository_observation_scope: RepositoryObservationScope,
    reservation_snapshots: &[RepositoryReservationSnapshot],
    evaluation_budget: &mut SuccessorScopedPatchEvaluationBudget,
) -> Result<SuccessorIncorporationObservation, ReservationReplayError> {
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
    let mut predecessor_groups = successors_by_predecessor.into_iter().collect::<Vec<_>>();
    predecessor_groups.sort_by_key(|(predecessor_id, _)| predecessor_id.to_string());
    let mut evidence_subjects = Vec::new();
    for (predecessor_id, successors) in predecessor_groups {
        let Some(predecessor_snapshot) = snapshots_by_reservation.get(&predecessor_id) else {
            continue;
        };
        let (protected_reservation_tip, prior_integration_status) =
            match &predecessor_snapshot.evidence {
                RepositoryReservationEvidence::Outstanding {
                    protected_tip,
                    integration_status,
                }
                | RepositoryReservationEvidence::Released {
                    protected_tip,
                    integration_status,
                    ..
                } => {
                    let prior_integration_status = if matches!(
                        integration_status,
                        IntegrationEvidenceStatus::Integrated { .. }
                    ) {
                        PriorIntegrationStatus::Proven
                    } else {
                        PriorIntegrationStatus::Unproven
                    };
                    (protected_tip.clone(), prior_integration_status)
                },
                RepositoryReservationEvidence::Active
                | RepositoryReservationEvidence::ReleasedWithoutCheckpoint { .. } => continue,
            };
        let mut candidate_heads = Vec::new();
        for successor in successors {
            let Some(successor_snapshot) = snapshots_by_reservation.get(&successor) else {
                continue;
            };
            let WorktreeHead::Resolved(head) = &successor_snapshot.worktree_head else {
                continue;
            };
            if !candidate_heads.contains(head) {
                candidate_heads.push(head.clone());
            }
        }
        if candidate_heads.is_empty() {
            continue;
        }
        let predecessor = reservations.reservation(predecessor_id)?;
        evidence_subjects.push(PredecessorSuccessorEvidenceSubject {
            reservation: predecessor,
            prior_integration_status,
            protected_reservation_tip,
            successor_heads: candidate_heads,
        });
    }
    let protected_tip_successor_heads = evidence_subjects
        .iter()
        .map(|subject| {
            ProtectedTipSuccessorHeads::new(
                subject.protected_reservation_tip.as_ref(),
                &subject.successor_heads,
            )
        })
        .collect::<Vec<_>>();
    let descendant_commit_results =
        git::descendant_commits(repository_root, &protected_tip_successor_heads);
    let mut by_predecessor = Vec::new();
    let mut pending_comparisons = Vec::new();
    match descendant_commit_results {
        Err(_) => {
            by_predecessor.extend(evidence_subjects.into_iter().map(|subject| {
                (
                    subject.reservation.id(),
                    PredecessorSuccessorIncorporation::QueryFailed,
                )
            }));
        },
        Ok(descendant_commit_queries) => {
            for (evidence_subject, descendant_commit_query) in
                evidence_subjects.into_iter().zip(descendant_commit_queries)
            {
                let predecessor = evidence_subject.reservation;
                let predecessor_id = predecessor.id();
                let subject = predecessor.integration_proof_subject_revision();
                let predecessor_index = by_predecessor.len();
                let incorporation = match descendant_commit_query {
                    DescendantCommitQuery::AncestorObjectUnknown => {
                        PredecessorSuccessorIncorporation::PredecessorObjectUnknown
                    },
                    DescendantCommitQuery::Classified(classified_heads) => {
                        let mut evidence_by_head = HashMap::new();
                        for classified_head in classified_heads {
                            match classified_head {
                                CandidateHeadReachability::Descendant(head) => {
                                    evidence_by_head.insert(
                                        head,
                                        SuccessorIncorporationEvidence::ProtectedTipAncestor,
                                    );
                                },
                                CandidateHeadReachability::ObjectUnknown(head) => {
                                    evidence_by_head.insert(
                                        head,
                                        SuccessorIncorporationEvidence::ObjectUnknown,
                                    );
                                },
                                CandidateHeadReachability::NotDescendant(head) => {
                                    let evidence = if matches!(
                                        evidence_subject.prior_integration_status,
                                        PriorIntegrationStatus::Proven
                                    ) {
                                        match predecessor
                                            .successor_scoped_patch_equivalence_cache()
                                            .lookup(subject, &head)
                                        {
                                            SuccessorScopedPatchEquivalenceCacheLookup::Hit(
                                                SuccessorScopedPatchEquivalenceVerdict::Equivalent,
                                            ) => {
                                                SuccessorIncorporationEvidence::ScopedPatchEquivalent
                                            },
                                            SuccessorScopedPatchEquivalenceCacheLookup::Hit(
                                                SuccessorScopedPatchEquivalenceVerdict::Different,
                                            ) => {
                                                SuccessorIncorporationEvidence::NotIncorporated
                                            },
                                            SuccessorScopedPatchEquivalenceCacheLookup::Miss => {
                                                pending_comparisons.push(
                                                    SuccessorScopedPatchEvaluationCandidate {
                                                        predecessor_index,
                                                        predecessor_reservation_id: predecessor_id,
                                                        subject,
                                                        phase_start_head: predecessor
                                                            .phase_start_head()
                                                            .as_ref()
                                                            .clone(),
                                                        scopes: predecessor.scopes().clone(),
                                                        protected_tip: evidence_subject
                                                            .protected_reservation_tip
                                                            .as_ref()
                                                            .clone(),
                                                        successor_head: head.clone(),
                                                        priority: predecessor
                                                            .successor_scoped_patch_evaluation_priority(
                                                                &head,
                                                            ),
                                                    },
                                                );
                                                SuccessorIncorporationEvidence::NotIncorporated
                                            },
                                        }
                                    } else {
                                        SuccessorIncorporationEvidence::NotIncorporated
                                    };
                                    evidence_by_head.insert(head, evidence);
                                },
                            }
                        }
                        PredecessorSuccessorIncorporation::Classified(evidence_by_head)
                    },
                };
                by_predecessor.push((predecessor_id, incorporation));
            }
        },
    }

    pending_comparisons.sort_by_key(|candidate| {
        (
            candidate.priority,
            candidate.predecessor_reservation_id.to_string(),
            candidate.successor_head.to_string(),
        )
    });
    let mut operations = Vec::new();
    for candidate in pending_comparisons {
        let comparison = evaluation_budget.evaluate(|| {
            git::scoped_patch_equivalence(
                repository_root,
                &candidate.phase_start_head,
                &candidate.scopes,
                &candidate.protected_tip,
                &candidate.successor_head,
            )
            .unwrap_or(ScopedPatchComparison::Unavailable)
        });
        let SuccessorScopedPatchComparisonObservation::Observed(comparison) = comparison else {
            continue;
        };
        let PredecessorSuccessorIncorporation::Classified(evidence_by_head) =
            &mut by_predecessor[candidate.predecessor_index].1
        else {
            continue;
        };
        match comparison {
            ScopedPatchComparison::Equivalent => {
                evidence_by_head.insert(
                    candidate.successor_head.clone(),
                    SuccessorIncorporationEvidence::ScopedPatchEquivalent,
                );
                operations.push(JournalOperation::SuccessorScopedPatchEquivalenceChecked {
                    predecessor_reservation_id: candidate.predecessor_reservation_id,
                    subject:                    candidate.subject,
                    successor_head:             candidate.successor_head,
                    verdict:                    SuccessorScopedPatchEquivalenceVerdict::Equivalent,
                });
            },
            ScopedPatchComparison::Different => {
                operations.push(JournalOperation::SuccessorScopedPatchEquivalenceChecked {
                    predecessor_reservation_id: candidate.predecessor_reservation_id,
                    subject:                    candidate.subject,
                    successor_head:             candidate.successor_head,
                    verdict:                    SuccessorScopedPatchEquivalenceVerdict::Different,
                });
            },
            ScopedPatchComparison::Unavailable => {
                evidence_by_head.insert(
                    candidate.successor_head.clone(),
                    SuccessorIncorporationEvidence::ObjectUnknown,
                );
                operations.push(JournalOperation::SuccessorScopedPatchComparisonAttempted {
                    predecessor_reservation_id: candidate.predecessor_reservation_id,
                    subject:                    candidate.subject,
                    successor_head:             candidate.successor_head,
                });
            },
        }
    }
    Ok(SuccessorIncorporationObservation {
        by_predecessor,
        operations,
    })
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
        git::update_reservation_retention_refs(
            &self.repository_root,
            &self.retention_repairs,
            &self.retention_deletions,
        )?;
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
        for reservation in reservations.iter() {
            alerts.extend(
                alert::for_lost_integration_evidence(reservation, self.repository_snapshot.trunk())
                    .map_err(ReconcileError::Replay)?,
            );
        }
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
            Self::Replay(error) => OutputEnvelope::replay_failure(command_verb, &error),
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
