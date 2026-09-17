//! Shared liveness, evidence, retention-ref, and marker reconciliation.

use std::collections::HashMap;
use std::collections::HashSet;
use std::error::Error;
use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;
use std::path::Path;
use std::path::PathBuf;
use std::thread;

use crate::alert;
use crate::alert::Alert;
use crate::config::BerthConfig;
use crate::config::ConfigError;
use crate::config::Enrollment;
use crate::constants::MERGE_EXTENT_TRUNK_UNAVAILABLE;
use crate::constants::MERGE_EXTENT_WORKTREE_UNAVAILABLE;
use crate::drift;
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
use crate::gate;
use crate::gate::RewriteCreatedCommits;
use crate::gate::permit;
use crate::gate::permit::CompletedRewriteSubject;
use crate::gate::permit::PendingBranchRewriteMarker;
use crate::gate::permit::PendingBypassMarkerImport;
use crate::gate::permit::RecoveredPendingBypassMarker;
use crate::gate::rewrite_map;
use crate::gate::rewrite_map::MappedPhaseInterval;
use crate::gate::rewrite_map::PhaseRewriteMapping;
use crate::git;
use crate::git::CandidateHeadReachability;
use crate::git::CommitCandidateReachability;
use crate::git::CommitTargetReachability;
use crate::git::CommitTargetReachabilityObservation;
use crate::git::GitError;
use crate::git::HistoricalIntegrationCandidateDiscovery;
use crate::git::PhaseStartTargetFirstParentHistories;
use crate::git::ProtectedTipSuccessorHeadClassification;
use crate::git::ProtectedTipSuccessorHeads;
use crate::git::Reachability;
use crate::git::ReservationRetentionRefRepair;
use crate::git::ResolvedBatchCommitCandidates;
use crate::git::ScopedPatchComparison;
use crate::git::ScopedPatchTargetHistory;
use crate::ids::CoordinationRunId;
use crate::ids::GitObjectId;
use crate::ids::JournalByteOffset;
use crate::ids::ProjectionGeneration;
use crate::ids::RepoInstanceId;
use crate::ids::ReservationId;
use crate::ids::ReservationScopePath;
use crate::ids::WireOrderedReservationIds;
use crate::ids::WorktreeId;
use crate::ledger;
use crate::ledger::BypassOccurrenceTime;
use crate::ledger::ClaimSource;
use crate::ledger::JournalEvent;
use crate::ledger::JournalOperation;
use crate::ledger::Ledger;
use crate::ledger::LedgerCommittedActionError;
use crate::ledger::LedgerCommittedActionOutcome;
use crate::ledger::LedgerError;
use crate::ledger::LedgerTransactionError;
use crate::ledger::LedgerTransactionOutcome;
use crate::ledger::ProtectedPhaseStartHead;
use crate::ledger::ReconciliationValidation;
use crate::ledger::RecoverableReconciliationAppendFailures;
use crate::ledger::ReplayedLedgerState;
use crate::ledger::ReservationSnapshot;
use crate::ledger::TransactionValidation;
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
use crate::reservation::IntegrationWitness;
use crate::reservation::MergeExtent;
use crate::reservation::MergeExtentKey;
use crate::reservation::PriorIntegrationStatus;
use crate::reservation::ProtectedReservationTip;
use crate::reservation::ReleaseDisposition;
use crate::reservation::ReleaseRevalidationSubject;
use crate::reservation::Reservation;
use crate::reservation::ReservationEvidenceState;
use crate::reservation::ReservationLifecycle;
use crate::reservation::ReservationReplayError;
use crate::reservation::RetainedReservationSet;
use crate::reservation::RewrittenIntegrationTrunkCommit;
use crate::reservation::ScopedPatchComparisonObservation;
use crate::reservation::ScopedPatchEquivalenceVerdict;
use crate::reservation::ScopedPatchEvaluationPriority;
use crate::reservation::ScopedPatchEvaluatorVersion;
use crate::reservation::ScopedPatchIntegrationEvaluation;
use crate::reservation::ScopedPatchTargetVerdictAvailability;
use crate::reservation::SuccessorScopedPatchEquivalenceVerdict;
use crate::reservation::SuccessorScopedPatchTargetVerdictAvailability;
use crate::scope::ReservationScope;
use crate::scope::ReservationScopeSet;
use crate::scope::ScopeKind;
use crate::session::SessionIdentityMappingPublication;
use crate::worktree::WorktreeHead;
use crate::worktree::WorktreeLiveness;
use crate::worktree::WorktreeMarkerSweepContext;
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
    /// Reservations released by this actual-trunk reconciliation transaction.
    pub(crate) settlements:                   Vec<ReconciledSettlement>,
    /// Mapping publication returned by the requesting reconciliation transaction.
    pub(crate) session_mapping_publication:   SessionIdentityMappingPublication,
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

impl ReconciliationReport {
    /// Return the trunk observation admitted by this reconciliation pass.
    pub(crate) const fn repository_trunk(&self) -> &RepositoryTrunk {
        self.repository_snapshot.trunk()
    }
}

/// Reconciliation state retained for the drift observation that immediately follows it.
pub(crate) struct ReconciledDriftPreflight<ConcurrentObservation> {
    /// The one complete reconciliation result whose alerts accompany the drift response.
    pub(crate) report:           ReconciliationReport,
    /// The filesystem-discovered worktree identity shared with drift execution.
    pub(crate) worktree_context: WorktreeContext,
    /// The already-located ledger used by drift's later mutation transaction.
    pub(crate) ledger:           Ledger,
    /// Drift facts read concurrently after current-marker reconciliation.
    pub(crate) observation:      ConcurrentObservation,
}

/// Git query dimensions owned by reconciliation rather than board row projection.
pub(crate) struct ReconciliationGitCost {
    /// Calls that attempted to resolve the configured trunk.
    pub(crate) trunk_resolution_calls:               u64,
    /// Calls used to establish orphan recovery evidence.
    pub(crate) orphan_recovery_evidence_queries:     u64,
    /// Status observations shared across the reservations of each live holder.
    pub(crate) merge_extent_worktree_status_queries: u64,
    /// Net merge-base path queries, including separation of committed paths for settlement.
    pub(crate) merge_extent_path_queries:            u64,
}

/// Attempted Git queries made while deriving all holder merge extents under one lock.
#[derive(Default)]
struct MergeExtentGitCost {
    /// Includes attempted status reads that report an observation failure.
    worktree_status_queries: u64,
    /// Includes failed reads, fresh committed-path reads needed to settle a cached extent, and
    /// the ancestry read that ends a clean run whose head trunk contains.
    path_queries:            u64,
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

/// One terminal integration disposition appended by this reconciliation.
pub(crate) struct ReconciledSettlement {
    /// The outstanding reservation this transaction released.
    pub(crate) reservation_id: ReservationId,
    /// The complete integration proof retained by the release.
    pub(crate) disposition:    ReleaseDisposition,
}

struct ReconciliationPlan {
    operations: Vec<JournalOperation>,
    action:     ReconciliationAction,
}

struct ReconciliationEvidenceContext<'context> {
    merged_run_endings:                       MergedRunEndings,
    berth_config:                             &'context BerthConfig,
    scoped_patch_evaluation_budget: &'context mut ReconciliationScopedPatchEvaluationBudget,
    successor_scoped_patch_evaluation_budget:
        &'context mut ReconciliationSuccessorScopedPatchEvaluationBudget,
}

struct TargetIntegrationEvidenceContext<'context> {
    repository_root:                &'context Path,
    repository_trunk:               &'context RepositoryTrunk,
    integration_reachability:       &'context BatchedIntegrationReachability,
    scoped_patch_evaluation_budget: &'context mut ReconciliationScopedPatchEvaluationBudget,
}

/// Identifies the checkout whose recorded worktree roots a reconciliation classifies against.
#[derive(Clone, Copy)]
struct LedgerCheckoutIdentity<'directory> {
    repository:           RepoInstanceId,
    common_git_directory: &'directory Path,
}

/// Repository facts read once, before any retained reservation is judged against them.
struct ObservedRepositoryFacts {
    worktree_registry:                WorktreeRegistry,
    repository_trunk:                 RepositoryTrunk,
    integration_reachability:         BatchedIntegrationReachability,
    repository_evidence_observations: Vec<RepositoryEvidenceObservation>,
    /// How many times resolving the trunk queried git, reported as this reconciliation's cost.
    trunk_resolution_calls:           u64,
}

/// What every retained reservation contributed once the observed facts were applied to it.
struct ReconciledReservations {
    changes:        ReconciliationChanges,
    alert_subjects: Vec<AlertSubject>,
    snapshots:      Vec<RepositoryReservationSnapshot>,
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
    by_ancestor:         HashMap<GitObjectId, Reachability>,
    resolved_candidates: ResolvedBatchCommitCandidates,
    target_histories:    PhaseStartTargetFirstParentHistories,
}

impl BatchedIntegrationReachability {
    fn observe_configured_trunk(
        repository_root: &Path,
        reservations: &RetainedReservationSet,
        ordering_graph: &OrderingGraph,
        trunk_branch: &str,
    ) -> Result<(RepositoryTrunk, Self), ReservationReplayError> {
        let candidate_ancestors = Self::candidate_ancestors(reservations, ordering_graph)?;
        let observation =
            git::branch_commit_reachability(repository_root, trunk_branch, &candidate_ancestors);
        let Ok(observation) = observation else {
            return Ok((
                RepositoryTrunk::ObjectUnknown,
                Self {
                    by_ancestor:         HashMap::new(),
                    resolved_candidates: git::ResolvedBatchCommitCandidates::default(),
                    target_histories:    git::PhaseStartTargetFirstParentHistories::default(),
                },
            ));
        };
        let CommitTargetReachabilityObservation {
            reachability,
            resolved_candidates,
            target_histories,
        } = observation;
        let CommitTargetReachability::Resolved { target, candidates } = reachability else {
            return Ok((
                RepositoryTrunk::ObjectUnknown,
                Self {
                    by_ancestor: HashMap::new(),
                    resolved_candidates,
                    target_histories,
                },
            ));
        };
        let by_ancestor = candidate_ancestors
            .into_iter()
            .zip(candidates)
            .map(|(candidate_ancestor, reachability)| {
                let reachability = match reachability {
                    CommitCandidateReachability::Ancestor => Reachability::Ancestor,
                    CommitCandidateReachability::NotAncestor => Reachability::NotAncestor,
                    CommitCandidateReachability::Missing
                    | CommitCandidateReachability::Ambiguous
                    | CommitCandidateReachability::WrongType { .. } => Reachability::ObjectUnknown,
                };
                (candidate_ancestor, reachability)
            })
            .collect();
        Ok((
            RepositoryTrunk::Resolved(target),
            Self {
                by_ancestor,
                resolved_candidates,
                target_histories,
            },
        ))
    }

    fn observe(
        repository_root: &Path,
        reservations: &RetainedReservationSet,
        ordering_graph: &OrderingGraph,
        repository_trunk: &RepositoryTrunk,
    ) -> Result<Self, ReservationReplayError> {
        let RepositoryTrunk::Resolved(target) = repository_trunk else {
            return Ok(Self {
                by_ancestor:         HashMap::new(),
                resolved_candidates: git::ResolvedBatchCommitCandidates::default(),
                target_histories:    git::PhaseStartTargetFirstParentHistories::default(),
            });
        };
        let candidate_ancestors = Self::candidate_ancestors(reservations, ordering_graph)?;
        let reachability =
            git::reachability_to_target(repository_root, &candidate_ancestors, target)
                .unwrap_or_else(|_| vec![Reachability::ObjectUnknown; candidate_ancestors.len()]);
        Ok(Self {
            by_ancestor:         candidate_ancestors.into_iter().zip(reachability).collect(),
            resolved_candidates: git::ResolvedBatchCommitCandidates::default(),
            target_histories:    git::PhaseStartTargetFirstParentHistories::default(),
        })
    }

    fn candidate_ancestors(
        reservations: &RetainedReservationSet,
        ordering_graph: &OrderingGraph,
    ) -> Result<Vec<GitObjectId>, ReservationReplayError> {
        let mut candidate_ancestors = HashSet::new();
        for reservation in reservations.iter() {
            match reservation.evidence_state()? {
                ReservationEvidenceState::Outstanding {
                    protected_tip,
                    trunk_snapshot,
                    ..
                } => {
                    candidate_ancestors.insert(reservation.phase_start_head().as_ref().clone());
                    candidate_ancestors.insert(protected_tip.as_ref().clone());
                    candidate_ancestors.insert(trunk_snapshot);
                },
                ReservationEvidenceState::Released {
                    protected_tip,
                    disposition,
                    ..
                } => {
                    if !matches!(
                        disposition.revalidation_subject(),
                        ReleaseRevalidationSubject::None
                    ) {
                        candidate_ancestors.insert(reservation.phase_start_head().as_ref().clone());
                    }
                    match disposition.revalidation_subject() {
                        ReleaseRevalidationSubject::ProtectedTip => {
                            candidate_ancestors.insert(protected_tip.as_ref().clone());
                        },
                        ReleaseRevalidationSubject::RewrittenIntegration(trunk_commit) => {
                            candidate_ancestors.insert(trunk_commit.as_ref().clone());
                        },
                        ReleaseRevalidationSubject::None => {},
                    }
                    if ordering_graph.has_nonterminal_dependent(reservation.id(), reservations)? {
                        candidate_ancestors.insert(protected_tip.as_ref().clone());
                    }
                },
                ReservationEvidenceState::Active { .. }
                | ReservationEvidenceState::ReleasedWithoutCheckpoint { .. } => {},
            }
        }
        Ok(candidate_ancestors.into_iter().collect())
    }

    fn for_ancestor(&self, ancestor: &GitObjectId) -> Reachability {
        self.by_ancestor
            .get(ancestor)
            .copied()
            .unwrap_or(Reachability::ObjectUnknown)
    }

    fn target_history_after_phase_start(
        &self,
        phase_start: &GitObjectId,
    ) -> ScopedPatchTargetHistory<'_> {
        self.target_histories.after_phase_start(phase_start)
    }
}

#[derive(Clone, Copy)]
enum EvidenceRevalidationObservation {
    /// Current git evidence or a retained durable verdict supplies the status.
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
    /// A comparison ran but produced no durable verdict to retain.
    Attempted {
        subject: IntegrationProofSubjectRevision,
        target:  GitObjectId,
    },
    /// No retained verdict existed, and the comparison produced a definitive result.
    Checked {
        subject: IntegrationProofSubjectRevision,
        target:  GitObjectId,
        verdict: ScopedPatchEquivalenceVerdict,
        witness: IntegrationWitness,
    },
}

#[derive(Eq, Hash, PartialEq)]
struct ScopedPatchEvaluationKey {
    phase_start_head: GitObjectId,
    protected_tip:    GitObjectId,
    target_trunk:     GitObjectId,
    scopes:           Vec<ScopedPatchEvaluationScope>,
    context:          ScopedPatchEvaluationContext,
    /// The comparison destination and location source, distinct from trunk admission.
    destination:      ScopedPatchComparisonDestination,
}

/// The immutable history and location evidence used by one scoped comparison.
#[derive(Eq, Hash, PartialEq)]
enum ScopedPatchComparisonDestination {
    /// Ordinary integration evidence compares to the admission trunk itself.
    Trunk,
    /// Rewrite acceptance compares to this mapped tip with explicitly located commits.
    Mapped {
        tip:          GitObjectId,
        destinations: Vec<GitObjectId>,
    },
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
struct ReconciliationScopedPatchEvaluationBudget {
    evaluations:       HashMap<ScopedPatchEvaluationKey, ScopedPatchIntegrationEvaluation>,
    evaluated_targets: HashSet<GitObjectId>,
}

/// A fixed per-reconciliation budget shared by every pending successor target.
///
/// Reachability remains batched for all heads. Scoped equivalence is deliberately admitted once,
/// and durable attempt generations rotate the next cold target to the front on later passes.
#[derive(Default)]
enum ReconciliationSuccessorScopedPatchEvaluationBudget {
    /// The one admitted scoped comparison is still available.
    #[default]
    Unspent,
    /// The one admitted scoped comparison has already been performed.
    Spent,
}

impl ReconciliationSuccessorScopedPatchEvaluationBudget {
    fn evaluate(
        &mut self,
        evaluate: impl FnOnce() -> ScopedPatchComparison,
    ) -> SuccessorScopedPatchComparisonObservation {
        match self {
            Self::Spent => SuccessorScopedPatchComparisonObservation::Deferred,
            Self::Unspent => {
                *self = Self::Spent;
                SuccessorScopedPatchComparisonObservation::Observed(evaluate())
            },
        }
    }
}

enum SuccessorScopedPatchComparisonObservation {
    Observed(ScopedPatchComparison),
    Deferred,
}

/// The commit whose incorporation a successor must prove.
enum SuccessorIncorporationSubject {
    /// A fixed checkpoint permits ancestry or complete scoped equivalence.
    CheckpointTip(ProtectedReservationTip),
    /// A rewritten-integration witness requires the witness itself in successor ancestry.
    RewrittenIntegrationWitness(RewrittenIntegrationTrunkCommit),
}

impl SuccessorIncorporationSubject {
    /// Borrow the commit used by the grouped successor ancestry query.
    fn commit(&self) -> &GitObjectId {
        match self {
            Self::CheckpointTip(tip) => tip.as_ref(),
            Self::RewrittenIntegrationWitness(witness) => witness.as_ref(),
        }
    }
}

struct PredecessorSuccessorEvidenceSubject<'reservation> {
    incorporation_subject:    SuccessorIncorporationSubject,
    reservation:              &'reservation Reservation,
    prior_integration_status: PriorIntegrationStatus,
    successor_heads:          Vec<GitObjectId>,
}

struct SuccessorScopedPatchEvaluationCandidate {
    predecessor_index:          usize,
    predecessor_reservation_id: ReservationId,
    subject:                    IntegrationProofSubjectRevision,
    phase_start_head:           GitObjectId,
    scopes:                     ReservationScopeSet,
    protected_tip:              GitObjectId,
    successor_head:             GitObjectId,
    target_history:             SuccessorScopedPatchTargetHistory,
    priority:                   ScopedPatchEvaluationPriority,
}

enum SuccessorScopedPatchTargetHistory {
    ProvenFirstParentInterval { commits: Vec<GitObjectId> },
    NeedsGitQueries,
}

impl SuccessorScopedPatchTargetHistory {
    fn as_git_evidence(&self) -> ScopedPatchTargetHistory<'_> {
        match self {
            Self::ProvenFirstParentInterval { commits } => {
                ScopedPatchTargetHistory::ProvenFirstParentInterval { commits }
            },
            Self::NeedsGitQueries => ScopedPatchTargetHistory::NeedsGitQueries,
        }
    }
}

/// What a predecessor's own recorded evidence says before any successor is consulted.
enum PredecessorEvidenceStanding {
    /// The predecessor holds a protected tip its successors can be measured against.
    Measurable {
        incorporation_subject:    SuccessorIncorporationSubject,
        prior_integration_status: PriorIntegrationStatus,
    },
    /// The predecessor never reached a checkpoint, so no successor evidence applies to it.
    NoProtectedTip,
}

impl PredecessorEvidenceStanding {
    fn of(evidence: &RepositoryReservationEvidence) -> Self {
        let (incorporation_subject, integration_status) = match evidence {
            RepositoryReservationEvidence::Outstanding {
                protected_tip,
                integration_status,
            } => (
                SuccessorIncorporationSubject::CheckpointTip(protected_tip.clone()),
                integration_status,
            ),
            RepositoryReservationEvidence::Released {
                protected_tip,
                disposition,
                integration_status,
            } => {
                let subject = match disposition.revalidation_subject() {
                    ReleaseRevalidationSubject::RewrittenIntegration(witness) => {
                        SuccessorIncorporationSubject::RewrittenIntegrationWitness(witness.clone())
                    },
                    ReleaseRevalidationSubject::ProtectedTip | ReleaseRevalidationSubject::None => {
                        SuccessorIncorporationSubject::CheckpointTip(protected_tip.clone())
                    },
                };
                (subject, integration_status)
            },
            RepositoryReservationEvidence::Active
            | RepositoryReservationEvidence::ReleasedWithoutCheckpoint { .. } => {
                return Self::NoProtectedTip;
            },
        };
        Self::Measurable {
            incorporation_subject,
            prior_integration_status: if matches!(
                integration_status,
                IntegrationEvidenceStatus::Integrated { .. }
            ) {
                PriorIntegrationStatus::Proven
            } else {
                PriorIntegrationStatus::Unproven
            },
        }
    }
}

/// The grouped ancestry answers for the predecessor's incorporation subject and phase start.
#[derive(Clone, Copy)]
struct PredecessorSuccessorReachability<'classification> {
    from_incorporation_subject: &'classification ProtectedTipSuccessorHeadClassification,
    from_phase_start:           &'classification ProtectedTipSuccessorHeadClassification,
}

impl PredecessorSuccessorReachability<'_> {
    /// First-parent commits each successor head gained since the predecessor's phase start.
    fn phase_start_target_histories(&self) -> HashMap<GitObjectId, Vec<GitObjectId>> {
        match self.from_phase_start {
            ProtectedTipSuccessorHeadClassification::AncestorObjectUnknown => HashMap::new(),
            ProtectedTipSuccessorHeadClassification::Classified(classified_heads) => {
                classified_heads
                    .iter()
                    .filter_map(|classified_head| match classified_head {
                        CandidateHeadReachability::Descendant {
                            head,
                            first_parent_commits_after_ancestor,
                        } => Some((head.clone(), first_parent_commits_after_ancestor.clone())),
                        CandidateHeadReachability::NotDescendant(_)
                        | CandidateHeadReachability::ObjectUnknown(_) => None,
                    })
                    .collect()
            },
        }
    }
}

/// Everything a pending scoped-patch candidate takes from its predecessor, fixed for the whole
/// classification of that predecessor's successor heads.
struct PendingScopedPatchCandidateContext<'subject> {
    evidence_subject:  &'subject PredecessorSuccessorEvidenceSubject<'subject>,
    predecessor_index: usize,
    subject_revision:  IntegrationProofSubjectRevision,
    target_histories:  HashMap<GitObjectId, Vec<GitObjectId>>,
}

impl PendingScopedPatchCandidateContext<'_> {
    fn candidate(
        &self,
        protected_tip: &ProtectedReservationTip,
        successor_head: &GitObjectId,
    ) -> SuccessorScopedPatchEvaluationCandidate {
        let predecessor = self.evidence_subject.reservation;
        SuccessorScopedPatchEvaluationCandidate {
            predecessor_index:          self.predecessor_index,
            predecessor_reservation_id: predecessor.id(),
            subject:                    self.subject_revision,
            phase_start_head:           predecessor.phase_start_head().as_ref().clone(),
            scopes:                     predecessor.scopes().clone(),
            protected_tip:              protected_tip.as_ref().clone(),
            successor_head:             successor_head.clone(),
            target_history:             self.target_histories.get(successor_head).map_or(
                SuccessorScopedPatchTargetHistory::NeedsGitQueries,
                |commits| SuccessorScopedPatchTargetHistory::ProvenFirstParentInterval {
                    commits: commits.clone(),
                },
            ),
            priority:                   predecessor
                .successor_scoped_patch_evaluation_priority(successor_head),
        }
    }
}

/// Incorporation verdicts reachability alone settled, plus the targets still awaiting one scoped
/// comparison.
///
/// The two fields are positionally coupled: every pending candidate's `predecessor_index` is a
/// position in `by_predecessor`, resolved by indexing when its comparison settles. Filtering,
/// reordering, or deduplicating `by_predecessor` between construction and settlement misaddresses
/// every later candidate, so rebuild the indices with it or do neither.
struct SuccessorIncorporationClassification {
    by_predecessor:      Vec<(ReservationId, PredecessorSuccessorIncorporation)>,
    pending_comparisons: Vec<SuccessorScopedPatchEvaluationCandidate>,
}

struct SuccessorIncorporationObservation {
    by_predecessor: Vec<(ReservationId, PredecessorSuccessorIncorporation)>,
    operations:     Vec<JournalOperation>,
}

impl ReconciliationScopedPatchEvaluationBudget {
    fn evaluate(
        &mut self,
        scoped_patch_evaluation_key: ScopedPatchEvaluationKey,
        evaluate: impl FnOnce() -> ScopedPatchIntegrationEvaluation,
    ) -> ScopedPatchComparisonObservation {
        if let Some(scoped_patch_comparison) = self.evaluations.get(&scoped_patch_evaluation_key) {
            return ScopedPatchComparisonObservation::Observed(scoped_patch_comparison.clone());
        }
        if !self
            .evaluated_targets
            .insert(scoped_patch_evaluation_key.target_trunk.clone())
        {
            return ScopedPatchComparisonObservation::Deferred;
        }
        let scoped_patch_comparison = evaluate();
        self.evaluations
            .insert(scoped_patch_evaluation_key, scoped_patch_comparison.clone());
        ScopedPatchComparisonObservation::Observed(scoped_patch_comparison)
    }
}

/// The gate caller whose lifecycle rules govern the proposed-trunk decision.
#[derive(Clone, Copy)]
pub(crate) enum GateReconciliationPurpose {
    /// Settle actual-trunk work before deciding whether a proposed integration may proceed.
    PreparedDecision,
    /// Preserve reservation lifecycles so the committed integration consumes its forced permits.
    CommittedAudit,
}

/// Actual-trunk reconciliation plus proposed-trunk constraints prepared under one lock.
pub(crate) struct GateReconciliation {
    reconciliation:            ReconciliationPlan,
    constraints:               IntegrationConstraintProjection,
    reservations:              RetainedReservationSet,
    /// Unaccepted rewrite destinations still subject to the reservation's ordering holds.
    deferred_rewrite_subjects: Vec<DeferredRewriteIntegrationSubject>,
}

/// A gate decision committed together with any reconciliation and permit records.
pub(crate) struct GateReconciliationAction<Decision> {
    reconciliation: ReconciliationAction,
    decision:       Decision,
}

#[derive(Default)]
struct ReconciliationChanges {
    operations:          Vec<JournalOperation>,
    retention_repairs:   Vec<ReservationRetentionRefRepair>,
    retention_deletions: Vec<ReservationId>,
    evidence:            Vec<ReconciledEvidence>,
}

struct ReconciliationAction {
    active_holders:                Vec<ActiveHolder>,
    marker_contexts:               Vec<WorktreeMarkerSweepContext>,
    repository_root:               PathBuf,
    retention_repairs:             Vec<ReservationRetentionRefRepair>,
    retention_deletions:           Vec<ReservationId>,
    retention_commit_resolution:   RetentionCommitResolution,
    alert_subjects:                Vec<AlertSubject>,
    evidence:                      Vec<ReconciledEvidence>,
    repository_snapshot:           RepositorySnapshot,
    recovered_bypass_reporting:    RecoveredBypassReporting,
    recovered_bypass_markers:      Vec<RecoveredPendingBypassMarker>,
    pending_bypass_imports:        Vec<PendingBypassMarkerImport>,
    /// Rewrite markers fully handled by the appended reconciliation.
    completed_rewrite_markers:     Vec<PendingBranchRewriteMarker>,
    /// Deferred markers whose definitive refusals must not consume future budgets.
    updated_rewrite_markers:       Vec<PendingBranchRewriteMarker>,
    unrecorded_bypass_occurrences: Vec<BypassOccurrenceTime>,
    trunk_resolution_calls:        u64,
    merge_extent_git_cost:         MergeExtentGitCost,
    settlements:                   Vec<ReconciledSettlement>,
}

/// Which commit batch establishes availability for the final retention repairs.
enum RetentionCommitResolution {
    /// The initial observation includes every commit the final repairs name.
    InitialObservation(ResolvedBatchCommitCandidates),
    /// Settlement introduced a witness, so resolve the final repairs together after append.
    IncludeSettlementWitness,
}

impl RetentionCommitResolution {
    /// Write all final repairs and deletions after the journal append succeeds.
    fn apply(
        &self,
        repository_root: &Path,
        repairs: &[ReservationRetentionRefRepair],
        deletions: &[ReservationId],
    ) -> Result<(), GitError> {
        match self {
            Self::InitialObservation(resolved_candidates) => {
                git::update_reservation_retention_refs_from_resolved_batch(
                    repository_root,
                    repairs,
                    deletions,
                    resolved_candidates,
                )
            },
            Self::IncludeSettlementWitness => {
                git::update_reservation_retention_refs(repository_root, repairs, deletions)
            },
        }
    }
}

#[derive(Clone, Copy)]
struct ActiveHolder {
    worktree_id:         WorktreeId,
    coordination_run_id: CoordinationRunId,
}

struct AlertSubject {
    reservation_id:    ReservationId,
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

/// Whether this reconciliation may end active runs whose work trunk already contains.
#[derive(Clone, Copy)]
enum MergedRunEndings {
    /// End every such run in this append.
    End,
    /// Leave them for a later reconciliation, so drift attributes a new commit to its run first.
    Defer,
}

/// What one reconciliation caller chooses to observe, report and end.
#[derive(Clone, Copy)]
struct ReconciliationRequest {
    repository_observation_scope: RepositoryObservationScope,
    recovered_bypass_reporting:   RecoveredBypassReporting,
    merged_run_endings:           MergedRunEndings,
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

/// Reconcile once while retaining the discovered worktree and opened ledger for drift.
pub(crate) fn reconcile_for_drift<ConcurrentObservation>(
    invocation_directory: &Path,
    observe: impl FnOnce(&WorktreeContext, &Ledger, &[JournalEvent]) -> ConcurrentObservation + Send,
) -> Result<Enrollment<ReconciledDriftPreflight<ConcurrentObservation>>, ReconcileError>
where
    ConcurrentObservation: Send,
{
    let worktree_context = WorktreeContext::discover(invocation_directory)?;
    match BerthConfig::read(&worktree_context.configuration_lookup())? {
        Enrollment::Enrolled(berth_config) => {
            let ledger = Ledger::open_from_discovered_worktree(&worktree_context)?;
            let observation_events =
                drift_observation_events_after_current_marker_sweep(&worktree_context, &ledger)?;
            let (report, observation) = thread::scope(|scope| {
                let observation_worker =
                    scope.spawn(|| observe(&worktree_context, &ledger, &observation_events));
                let report = reconcile_with_open_ledger(
                    &worktree_context,
                    &ledger,
                    &berth_config,
                    ReconciliationRequest {
                        repository_observation_scope:
                            RepositoryObservationScope::CurrentOrderingGraph,
                        recovered_bypass_reporting:   RecoveredBypassReporting::Defer,
                        // Post-commit drift must attribute the commit before its run can end.
                        merged_run_endings:           MergedRunEndings::Defer,
                    },
                );
                let observation = observation_worker
                    .join()
                    .map_err(|_| ReconcileError::ConcurrentObservationWorkerPanicked);
                (report, observation)
            });
            Ok(Enrollment::Enrolled(ReconciledDriftPreflight {
                report: report?,
                worktree_context,
                ledger,
                observation: observation?,
            }))
        },
        Enrollment::Unconfigured {
            expected_configuration_path,
        } => Ok(Enrollment::Unconfigured {
            expected_configuration_path,
        }),
    }
}

fn drift_observation_events_after_current_marker_sweep(
    worktree_context: &WorktreeContext,
    ledger: &Ledger,
) -> Result<Vec<JournalEvent>, ReconcileError> {
    let worktree_identity = ledger::worktree_identity(
        worktree_context.administrative_directory(),
        worktree_context.worktree_kind(),
    )?;
    let outcome = ledger
        .transact(worktree_identity.id, CoordinationRunId::new(), |state| {
            let prepared_events = RetainedReservationSet::replay(state.events())
                .map_err(ReconcileError::Replay)
                .and_then(|reservations| {
                    worktree_context
                        .sweep_coordination_run_marker(|coordination_run_id| {
                            reservations.iter().any(|reservation| {
                                reservation.is_active_for_coordination_run_and_worktree(
                                    coordination_run_id,
                                    worktree_identity.id,
                                )
                            })
                        })
                        .map_err(ReconcileError::Ledger)?;
                    Ok(state.events().to_vec())
                });
            TransactionValidation::Reject(prepared_events)
        })
        .map_err(ReconcileError::Transaction)?;
    match outcome {
        LedgerTransactionOutcome::Rejected(prepared_events) => prepared_events,
        LedgerTransactionOutcome::Appended { .. } => {
            Err(ReconcileError::UnexpectedDriftPreflightMutation)
        },
    }
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
    match BerthConfig::read(&worktree_context.configuration_lookup())? {
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
    let ledger = Ledger::open_from_discovered_worktree(worktree_context)?;
    reconcile_with_open_ledger(
        worktree_context,
        &ledger,
        berth_config,
        ReconciliationRequest {
            repository_observation_scope,
            recovered_bypass_reporting,
            merged_run_endings: MergedRunEndings::End,
        },
    )
}

/// Reconcile through one ledger already located from `worktree_context`.
fn reconcile_with_open_ledger(
    worktree_context: &WorktreeContext,
    ledger: &Ledger,
    berth_config: &BerthConfig,
    request: ReconciliationRequest,
) -> Result<ReconciliationReport, ReconcileError> {
    let ledger_repository = ledger.repository_identity()?;
    let journal_mutation_actor = ledger::resolve_identity(worktree_context)?
        .journal_mutation_actor_for(CoordinationRunId::new());
    retry_rewrite_reconciliation(
        || {
            let rewrite_preflight =
                prepare_rewrite_reconciliation(worktree_context, ledger, berth_config)?;
            let outcome = ledger
                .transact_reconciliation(
                    journal_mutation_actor.worktree_id,
                    journal_mutation_actor.coordination_run_id,
                    |state| match prepare_reconciliation_transaction(
                        &state,
                        berth_config,
                        request,
                        ledger_repository,
                        worktree_context,
                        rewrite_preflight,
                    ) {
                        Ok(prepared) => ReconciliationValidation::Apply {
                            operations:             prepared.operations,
                            recoverable_operations: prepared.recoverable_operations,
                            action:                 prepared.action,
                        },
                        Err(error) => ReconciliationValidation::Reject(error),
                    },
                    ReconciliationAction::commit,
                )
                .map_err(|error| match error {
                    LedgerCommittedActionError::Transaction(error) => {
                        ReconcileError::Transaction(error)
                    },
                    LedgerCommittedActionError::Action(error) => error,
                })?;
            match outcome {
                LedgerCommittedActionOutcome::Appended {
                    output: mut report,
                    session_mapping_publication,
                } => {
                    report.session_mapping_publication = session_mapping_publication;
                    Ok(report)
                },
                LedgerCommittedActionOutcome::Rejected(error) => Err(error.into()),
            }
        },
        |error| matches!(error, ReconcileError::RewriteSubjectChanged(_)),
    )
}

/// Retry a preflight invalidated by a concurrent transaction, at most three times.
/// The caller classifies only rejected validation; committed actions must never be repeated.
pub(crate) fn retry_rewrite_reconciliation<Output, Failure>(
    mut attempt: impl FnMut() -> Result<Output, Failure>,
    subject_changed: impl Fn(&Failure) -> bool,
) -> Result<Output, Failure> {
    let mut result = attempt();
    for _ in 1..3 {
        if !result.as_ref().is_err_and(&subject_changed) {
            break;
        }
        result = attempt();
    }
    result
}

/// The outstanding phase revision a rewrite candidate must still describe under the lock.
struct RewriteSubjectValidation {
    /// Reservation nominated by the rewrite map.
    reservation_id: ReservationId,
    /// The baseline, protected tip, and scopes used to nominate or accept the rewrite.
    subject:        IntegrationProofSubjectRevision,
}

impl RewriteSubjectValidation {
    /// Capture the phase revision used by a rewrite candidate.
    const fn capture(reservation: &Reservation) -> Self {
        Self {
            reservation_id: reservation.id(),
            subject:        reservation.integration_proof_subject_revision(),
        }
    }

    /// A replayed lifecycle and revision must still describe the nominated outstanding phase.
    fn accepts(&self, reservations: &RetainedReservationSet) -> bool {
        reservations
            .reservation(self.reservation_id)
            .is_ok_and(|reservation| {
                matches!(
                    reservation.lifecycle(),
                    ReservationLifecycle::Outstanding { .. }
                ) && reservation.integration_proof_subject_revision() == self.subject
            })
    }
}

/// Rewrite destinations that must retain ordering holds until their content check completes.
struct DeferredRewriteIntegrationSubject {
    /// The outstanding phase expected after all accepted preflight resnapshots.
    validation:     RewriteSubjectValidation,
    /// Candidate commits identify entry only; they never certify protected contents.
    rewritten_tips: Vec<GitObjectId>,
}

impl DeferredRewriteIntegrationSubject {
    /// Follow a later rewrite even while the reservation retains its original anchors.
    fn follow_rewrite(&mut self, marker: &PendingBranchRewriteMarker) {
        let destinations = marker
            .rewrite
            .pairs
            .iter()
            .filter(|pair| self.rewritten_tips.contains(&pair.old))
            .map(|pair| pair.new.clone())
            .collect::<Vec<_>>();
        if destinations.is_empty() {
            return;
        }
        for tip in destinations
            .into_iter()
            .chain(marker.rewrite.new_tips.iter().cloned())
        {
            if !self.rewritten_tips.contains(&tip) {
                self.rewritten_tips.push(tip);
            }
        }
    }
}

/// Accepted rewrite proofs, deferred entry candidates, and their shared comparison budget.
#[derive(Default)]
pub(crate) struct RewriteReconciliationPreflight {
    /// Sequential expected subjects; multiple maps may advance the same reservation.
    validations:            Vec<RewriteSubjectValidation>,
    /// Accepted replacements, in the same order as their expected subjects.
    operations:             Vec<JournalOperation>,
    /// Markers with no deferred candidates, retired only after append.
    completed_markers:      Vec<PendingBranchRewriteMarker>,
    /// Partial marker progress published by the committed action.
    updated_markers:        Vec<PendingBranchRewriteMarker>,
    /// Pending destinations conservatively included in the gate's entry decision.
    deferred_subjects:      Vec<DeferredRewriteIntegrationSubject>,
    /// Shared admission and comparison cache for this reconciliation pass.
    budget:                 ReconciliationScopedPatchEvaluationBudget,
    /// Extra trunk reads used to admit mapped acceptance before the locked observation.
    trunk_resolution_calls: u64,
}

/// Why a rewrite proof cannot be projected onto the locked reservation replay.
#[derive(Debug)]
enum RewriteProjectionError {
    /// A concurrent transaction changed the phase that was checked.
    RewriteSubjectChanged(ReservationId),
    /// The accepted replacement does not form a valid reservation state.
    Reservation(ReservationReplayError),
}

impl RewriteReconciliationPreflight {
    /// Retain nominated destinations without claiming that their protected contents match.
    fn defer_subject(
        &mut self,
        reservation: &Reservation,
        marker: &PendingBranchRewriteMarker,
        destinations: impl Iterator<Item = GitObjectId>,
    ) {
        self.deferred_subjects
            .push(DeferredRewriteIntegrationSubject {
                validation:     RewriteSubjectValidation::capture(reservation),
                rewritten_tips: marker
                    .rewrite
                    .new_tips
                    .iter()
                    .cloned()
                    .chain(destinations)
                    .collect(),
            });
    }

    /// Check each preflight proof under the lock before projecting any repository facts.
    fn project(
        &self,
        reservations: &RetainedReservationSet,
    ) -> Result<RetainedReservationSet, RewriteProjectionError> {
        let mut projected = reservations.clone();
        for (validation, operation) in self.validations.iter().zip(&self.operations) {
            if !validation.accepts(&projected) {
                return Err(RewriteProjectionError::RewriteSubjectChanged(
                    validation.reservation_id,
                ));
            }
            projected = projected
                .with_pending_resnapshots(std::slice::from_ref(operation))
                .map_err(RewriteProjectionError::Reservation)?;
        }
        for subject in &self.deferred_subjects {
            if !subject.validation.accepts(&projected) {
                return Err(RewriteProjectionError::RewriteSubjectChanged(
                    subject.validation.reservation_id,
                ));
            }
        }
        Ok(projected)
    }
}

/// The ordered history and introduced commits belonging to one rewritten tip.
struct RewrittenTipHistory {
    /// Complete first-parent history from the root through the rewritten tip.
    first_parent_commits: Vec<GitObjectId>,
    /// Commits introduced by this branch's rewrite, fixed when the event was recorded.
    created:              RewriteCreatedCommits,
}

/// Read ready maps and check protected contents while no ledger mutation lock is held.
pub(crate) fn prepare_rewrite_reconciliation(
    worktree_context: &WorktreeContext,
    ledger: &Ledger,
    berth_config: &BerthConfig,
) -> Result<RewriteReconciliationPreflight, ReconcileError> {
    let markers = permit::pending_branch_rewrite_markers(worktree_context.common_git_directory())
        .map_err(LedgerError::Io)?
        .into_iter()
        .filter(|marker| {
            !git::rewrite_in_progress(&marker.rewrite.worktree_administrative_directory)
        })
        .collect::<Vec<_>>();
    let mut preflight = RewriteReconciliationPreflight::default();
    if markers.is_empty() {
        return Ok(preflight);
    }
    let mut reservations = RetainedReservationSet::replay(&ledger.read_validated_events()?)
        .map_err(ReconcileError::Replay)?;
    preflight.trunk_resolution_calls = 1;
    let Ok(trunk) = git::branch_object_id(worktree_context.repository_root(), &berth_config.trunk)
    else {
        return Ok(preflight);
    };
    let mut earlier_map_deferred = false;
    for mut marker in markers {
        for subject in &mut preflight.deferred_subjects {
            subject.follow_rewrite(&marker);
        }
        let mut tips = if marker.rewrite.new_tips.is_empty() {
            marker
                .rewrite
                .pairs
                .iter()
                .map(|pair| pair.new.clone())
                .collect::<HashSet<_>>()
                .into_iter()
                .collect()
        } else {
            marker.rewrite.new_tips.clone()
        };
        let mut distinct_tips = HashSet::new();
        tips.retain(|tip| distinct_tips.insert(tip.clone()));
        let histories = tips
            .iter()
            .map(|tip| {
                Ok(RewrittenTipHistory {
                    first_parent_commits: gate::rewritten_first_parent_history(
                        worktree_context.repository_root(),
                        tip,
                    )?,
                    created:              gate::RewriteCreatedCommits::from_commits(
                        marker.rewrite.created_commits.iter().cloned(),
                    ),
                })
            })
            .collect::<Result<Vec<_>, GitError>>();
        // Unavailable first-parent histories defer affected subjects, not unrelated markers.
        let histories = histories.unwrap_or_default();
        let mut deferred = earlier_map_deferred;
        deferred |= import_rewrite_candidates(
            worktree_context.repository_root(),
            &mut marker,
            &histories,
            &mut reservations,
            &trunk,
            &mut preflight,
        )?;
        if deferred {
            earlier_map_deferred = true;
            preflight.updated_markers.push(marker);
        } else {
            preflight.completed_markers.push(marker);
        }
    }
    for subject in &mut preflight.deferred_subjects {
        // A later map may accept another replacement for this same outstanding phase.
        subject.validation = RewriteSubjectValidation::capture(
            reservations
                .reservation(subject.validation.reservation_id)
                .map_err(ReconcileError::Replay)?,
        );
    }
    Ok(preflight)
}

/// Whether a pending marker locates an outstanding phase or must retain its entry candidates.
enum OutstandingRewriteCandidate {
    /// The map does not nominate a replacement interval for this phase.
    NotMapped,
    /// Missing history leaves these destinations subject to conservative ordering holds.
    HistoryUnavailable(Vec<GitObjectId>),
    /// Located anchors still need a scoped content comparison before acceptance.
    Mapped(MappedPhaseInterval),
}

/// Locate the phase while keeping unavailable Git facts distinct from a refused mapping.
fn outstanding_rewrite_candidate(
    repository_root: &Path,
    reservation: &Reservation,
    protected_tip: &ProtectedReservationTip,
    marker: &PendingBranchRewriteMarker,
    histories: &[RewrittenTipHistory],
) -> OutstandingRewriteCandidate {
    let Ok(old_phase) = gate::rewrite_phase_commits(
        repository_root,
        reservation.phase_start_head().as_ref(),
        protected_tip.as_ref(),
    ) else {
        return OutstandingRewriteCandidate::HistoryUnavailable(
            marker
                .rewrite
                .pairs
                .iter()
                .map(|pair| pair.new.clone())
                .collect(),
        );
    };
    let destinations = marker
        .rewrite
        .pairs
        .iter()
        .filter(|pair| old_phase.contains(&pair.old))
        .map(|pair| &pair.new)
        .collect::<Vec<_>>();
    if destinations.is_empty() {
        return OutstandingRewriteCandidate::NotMapped;
    }
    if histories.is_empty() {
        return OutstandingRewriteCandidate::HistoryUnavailable(
            destinations.into_iter().cloned().collect(),
        );
    }
    histories
        .iter()
        .filter(|history| {
            destinations
                .iter()
                .all(|tip| history.first_parent_commits.contains(tip))
        })
        .find_map(|history| {
            match rewrite_map::map_phase_interval(
                &marker.rewrite.pairs,
                &old_phase,
                &history.first_parent_commits,
                &history.created,
            ) {
                PhaseRewriteMapping::Mapped(interval) => Some(interval),
                PhaseRewriteMapping::Refused => None,
            }
        })
        .map_or(
            OutstandingRewriteCandidate::NotMapped,
            OutstandingRewriteCandidate::Mapped,
        )
}

/// Compare this marker's outstanding phases and retain deferred work for a later pass.
fn import_rewrite_candidates(
    repository_root: &Path,
    marker: &mut PendingBranchRewriteMarker,
    histories: &[RewrittenTipHistory],
    reservations: &mut RetainedReservationSet,
    trunk: &GitObjectId,
    preflight: &mut RewriteReconciliationPreflight,
) -> Result<bool, ReconcileError> {
    let mut deferred = false;
    let candidates = reservations.iter().map(Reservation::id).collect::<Vec<_>>();
    for reservation_id in candidates {
        let reservation = reservations
            .reservation(reservation_id)
            .map_err(ReconcileError::Replay)?;
        let ReservationLifecycle::Outstanding { protected_tip } = reservation.lifecycle() else {
            continue;
        };
        if marker.rewrite.completed_subjects.iter().any(|completed| {
            completed.reservation_id == reservation_id
                && completed.subject == reservation.integration_proof_subject_revision()
        }) {
            continue;
        }
        let interval = match outstanding_rewrite_candidate(
            repository_root,
            reservation,
            protected_tip,
            marker,
            histories,
        ) {
            OutstandingRewriteCandidate::NotMapped => continue,
            OutstandingRewriteCandidate::HistoryUnavailable(destinations) => {
                deferred = true;
                preflight.defer_subject(reservation, marker, destinations.into_iter());
                continue;
            },
            OutstandingRewriteCandidate::Mapped(interval) => interval,
        };
        if &interval.phase_start_head == reservation.phase_start_head().as_ref()
            && &interval.protected_tip == protected_tip.as_ref()
        {
            continue;
        }
        let comparison = compare_mapped_phase(
            repository_root,
            reservation,
            protected_tip,
            trunk,
            &interval,
            &mut preflight.budget,
        );
        match comparison {
            ScopedPatchComparisonObservation::Observed(
                ScopedPatchIntegrationEvaluation::Equivalent(_),
            ) => {
                let operation = JournalOperation::Resnapshot {
                    reservation_id,
                    snapshot: ReservationSnapshot::Outstanding {
                        phase_start_head: Some(ProtectedPhaseStartHead::from(
                            interval.phase_start_head,
                        )),
                        protected_tip:    ProtectedReservationTip::from(interval.protected_tip),
                        trunk_oid:        trunk.clone(),
                    },
                };
                preflight
                    .validations
                    .push(RewriteSubjectValidation::capture(reservation));
                *reservations = reservations
                    .with_pending_resnapshots(std::slice::from_ref(&operation))
                    .map_err(ReconcileError::Replay)?;
                preflight.operations.push(operation);
            },
            ScopedPatchComparisonObservation::Observed(
                ScopedPatchIntegrationEvaluation::Different,
            ) => {
                marker
                    .rewrite
                    .completed_subjects
                    .push(CompletedRewriteSubject {
                        reservation_id,
                        subject: reservation.integration_proof_subject_revision(),
                    });
            },
            ScopedPatchComparisonObservation::Observed(
                ScopedPatchIntegrationEvaluation::Unavailable
                | ScopedPatchIntegrationEvaluation::HistoricalEvidenceUnavailable,
            )
            | ScopedPatchComparisonObservation::Deferred => {
                deferred = true;
                preflight.defer_subject(reservation, marker, interval.destinations.into_iter());
            },
        }
    }
    Ok(deferred)
}

/// Charge rewrite acceptance to actual trunk while comparing the mapped destination.
fn compare_mapped_phase(
    repository_root: &Path,
    reservation: &Reservation,
    protected_tip: &ProtectedReservationTip,
    trunk: &GitObjectId,
    interval: &MappedPhaseInterval,
    budget: &mut ReconciliationScopedPatchEvaluationBudget,
) -> ScopedPatchComparisonObservation {
    let key = ScopedPatchEvaluationKey {
        phase_start_head: reservation.phase_start_head().as_ref().clone(),
        protected_tip:    protected_tip.as_ref().clone(),
        target_trunk:     trunk.clone(),
        scopes:           reservation
            .scopes()
            .as_slice()
            .iter()
            .map(|scope| ScopedPatchEvaluationScope {
                path:       scope.path.clone(),
                scope_kind: scope.kind.into(),
            })
            .collect(),
        context:          ScopedPatchEvaluationContext::Outstanding {
            previous_trunk: trunk.clone(),
        },
        destination:      ScopedPatchComparisonDestination::Mapped {
            tip:          interval.protected_tip.clone(),
            destinations: interval.destinations.clone(),
        },
    };
    budget.evaluate(key, || {
        git::scoped_patch_equivalence_with_target_history(
            repository_root,
            reservation.phase_start_head().as_ref(),
            reservation.scopes(),
            protected_tip.as_ref(),
            &interval.protected_tip,
            ScopedPatchTargetHistory::MappedDestinations {
                commits: &interval.destinations,
            },
        )
        .unwrap_or(ScopedPatchComparison::Unavailable)
        .into()
    })
}

struct PreparedReconciliationTransaction {
    operations:             Vec<JournalOperation>,
    recoverable_operations: Vec<JournalOperation>,
    action:                 ReconciliationAction,
}

fn prepare_reconciliation_transaction(
    state: &ReplayedLedgerState<'_>,
    berth_config: &BerthConfig,
    request: ReconciliationRequest,
    ledger_repository: RepoInstanceId,
    worktree_context: &WorktreeContext,
    rewrite_preflight: RewriteReconciliationPreflight,
) -> Result<PreparedReconciliationTransaction, ReconciliationPlanningError> {
    let reservations = RetainedReservationSet::replay(state.events())
        .map_err(ReconciliationPlanningError::Reservation)?;
    let reservations = rewrite_preflight.project(&reservations)?;
    let ordering_graph =
        OrderingGraph::replay(state.events()).map_err(ReconciliationPlanningError::Edge)?;
    let mut scoped_patch_evaluation_budget = rewrite_preflight.budget;
    let mut successor_scoped_patch_evaluation_budget =
        ReconciliationSuccessorScopedPatchEvaluationBudget::default();
    let mut reconciliation_evidence_context = ReconciliationEvidenceContext {
        merged_run_endings: request.merged_run_endings,
        berth_config,
        scoped_patch_evaluation_budget: &mut scoped_patch_evaluation_budget,
        successor_scoped_patch_evaluation_budget: &mut successor_scoped_patch_evaluation_budget,
    };
    let mut reconciliation_plan = build_plan(
        &reservations,
        &ordering_graph,
        request.repository_observation_scope,
        ledger_repository,
        worktree_context,
        &mut reconciliation_evidence_context,
    )
    .map_err(|error| match error {
        ReconciliationBuildError::Reservation(error) => {
            ReconciliationPlanningError::Reservation(error)
        },
        ReconciliationBuildError::WorktreeRegistry(error) => {
            ReconciliationPlanningError::WorktreeRegistry(error)
        },
    })?;
    let mut operations = rewrite_preflight.operations;
    operations.append(&mut reconciliation_plan.operations);
    reconciliation_plan.operations = operations;
    reconciliation_plan.action.completed_rewrite_markers = rewrite_preflight.completed_markers;
    reconciliation_plan.action.updated_rewrite_markers = rewrite_preflight.updated_markers;
    reconciliation_plan.action.trunk_resolution_calls += rewrite_preflight.trunk_resolution_calls;
    let mut pending_bypasses = permit::prepare_pending_bypass_recovery(
        worktree_context.common_git_directory(),
        state.events(),
    )
    .map_err(ReconciliationPlanningError::PendingBypass)?;
    let pending_bypass_imports = pending_bypasses.take_imports();
    let recoverable_operations = pending_bypass_imports
        .iter()
        .map(|pending_import| pending_import.operation().clone())
        .collect();
    reconciliation_plan.action.pending_bypass_imports = pending_bypass_imports;
    reconciliation_plan.action.recovered_bypass_reporting = request.recovered_bypass_reporting;
    reconciliation_plan.action.recovered_bypass_markers = pending_bypasses.take_completed_markers();
    reconciliation_plan.action.unrecorded_bypass_occurrences =
        pending_bypasses.take_unrecorded_occurrences();
    Ok(PreparedReconciliationTransaction {
        operations: reconciliation_plan.operations,
        recoverable_operations,
        action: reconciliation_plan.action,
    })
}

/// Committed paths retained separately from dirty paths during this observation pass.
#[derive(Clone, Default)]
enum CommittedMergeEvidence {
    /// No current committed-path observation can support settlement of a protected extent.
    #[default]
    Unavailable,
    /// The merge-base query produced these committed paths before dirty paths were added.
    Observed(Vec<ReservationScopePath>),
}

/// One holder's independent committed-path evidence and its complete merge protection.
#[derive(Clone)]
struct HolderMergeProtection {
    extent:          MergeExtent,
    committed_paths: CommittedMergeEvidence,
}

/// Journal updates and committed-path evidence derived together under the reconciliation lock.
struct MergeExtentReconciliation {
    operations:          Vec<JournalOperation>,
    committed_by_holder: HashMap<WorktreeId, CommittedMergeEvidence>,
}

/// Share status and net branch reads across every reservation in the same holder checkout.
/// The successful key lives in the journal, so process boundaries do not defeat the cache.
fn derive_merge_extents(
    reservations: &RetainedReservationSet,
    snapshot: &RepositorySnapshot,
    planned: &[JournalOperation],
    git_cost: &mut MergeExtentGitCost,
) -> Result<MergeExtentReconciliation, ReservationReplayError> {
    let mut observed_by_worktree = HashMap::new();
    let mut operations = Vec::new();
    for reservation in reservations.iter().filter(|reservation| {
        !matches!(
            reservation.lifecycle(),
            ReservationLifecycle::Released { .. }
        )
    }) {
        let observed = observed_by_worktree
            .entry(reservation.actor().worktree)
            .or_insert_with(|| {
                observe_merge_extent(reservation, reservations, snapshot, planned, git_cost)
            });
        let extent = match observed {
            Ok(observation) => observation.extent.clone(),
            Err(failure) => reservation.merge_extent().unavailable(failure.clone()),
        };
        if &extent != reservation.merge_extent() {
            operations.push(JournalOperation::MergeExtentObserved {
                reservation_id: reservation.id(),
                extent,
                run_status: reservation.run_status(),
            });
        }
    }
    for incident in reservations.outstanding_incursion_incidents() {
        let subject = reservations.reservation(incident.reservation_id())?;
        let latest = operations
            .iter()
            .find_map(|operation| match operation {
                JournalOperation::MergeExtentObserved {
                    reservation_id,
                    extent,
                    ..
                } if *reservation_id == subject.id() => Some(extent),
                _ => None,
            })
            .unwrap_or_else(|| subject.merge_extent());
        // A distinct disposition preserves the incident's history after its branch has no work.
        if matches!(latest, reservation::MergeExtent::Empty { .. }) {
            operations.push(JournalOperation::ResolveIncursion {
                incident_id: incident.id(),
            });
        }
    }
    Ok(MergeExtentReconciliation {
        operations,
        committed_by_holder: observed_by_worktree
            .into_iter()
            .map(|(holder, observation)| {
                (
                    holder,
                    observation.map_or(CommittedMergeEvidence::Unavailable, |observation| {
                        observation.committed_paths
                    }),
                )
            })
            .collect(),
    })
}

/// Read the dirty union even on a cache hit: a fingerprint cannot be assumed unchanged.
fn observe_merge_extent(
    reservation: &Reservation,
    reservations: &RetainedReservationSet,
    snapshot: &RepositorySnapshot,
    planned: &[JournalOperation],
    git_cost: &mut MergeExtentGitCost,
) -> Result<HolderMergeProtection, String> {
    let RepositoryTrunk::Resolved(trunk) = snapshot.trunk() else {
        return Err(MERGE_EXTENT_TRUNK_UNAVAILABLE.to_owned());
    };
    let holder = snapshot
        .reservation(reservation.id())
        .map_err(|error| error.to_string())?;
    let WorktreeHead::Resolved(head) = &holder.worktree_head else {
        return Err(MERGE_EXTENT_WORKTREE_UNAVAILABLE.to_owned());
    };
    // A locked registration stays Unavailable in the liveness report, but its HEAD
    // is usable here only after the registry validates the accessible checkout's identity.
    if !matches!(
        holder.worktree_liveness,
        WorktreeLiveness::Live | WorktreeLiveness::Unavailable
    ) {
        return Err(MERGE_EXTENT_WORKTREE_UNAVAILABLE.to_owned());
    }
    let root = planned
        .iter()
        .find_map(|operation| match operation {
            JournalOperation::RelocateWorktree {
                reservation_id,
                current_root,
                ..
            } if *reservation_id == reservation.id() => Some(current_root),
            _ => None,
        })
        .unwrap_or_else(|| reservation.worktree_root());
    git_cost.worktree_status_queries += 1;
    let working_tree = drift::observe_merge_working_tree(root.as_ref())?;
    let key = MergeExtentKey {
        trunk: trunk.clone(),
        head: head.clone(),
        working_tree,
    };
    let settlement_needs_committed_paths = reservations.iter().any(|candidate| {
        candidate.actor().worktree == reservation.actor().worktree
            && snapshot.reservation(candidate.id()).is_ok_and(|observed| {
                matches!(
                    observed.evidence,
                    RepositoryReservationEvidence::Outstanding {
                        integration_status: IntegrationEvidenceStatus::Integrated { .. },
                        ..
                    }
                )
            })
    });
    if let Some(cached) = reservations.iter().find(|holder| {
        holder.actor().worktree == reservation.actor().worktree
            && holder.merge_extent().matches_key(&key)
    }) && (!settlement_needs_committed_paths
        || matches!(cached.merge_extent(), MergeExtent::Empty { .. }))
    {
        return Ok(HolderMergeProtection {
            extent:          cached.merge_extent().clone(),
            committed_paths: CommittedMergeEvidence::Unavailable,
        });
    }
    git_cost.path_queries += 1;
    let committed_paths = git::unmerged_branch_paths(root.as_ref(), trunk, head)
        .map_err(|error| error.to_string())?;
    let mut paths = committed_paths.clone();
    paths.extend(key.working_tree.tracked_paths.iter().cloned());
    paths.extend(key.working_tree.untracked_paths.iter().cloned());
    paths.sort_by_key(ToString::to_string);
    paths.dedup();
    let scopes = paths
        .into_iter()
        .map(|path| ReservationScope {
            path,
            kind: ScopeKind::File,
        })
        .collect();
    Ok(HolderMergeProtection {
        extent:          reservation::MergeExtent::derived(key, scopes),
        committed_paths: CommittedMergeEvidence::Observed(committed_paths),
    })
}

fn build_plan(
    reservations: &RetainedReservationSet,
    ordering_graph: &OrderingGraph,
    repository_observation_scope: RepositoryObservationScope,
    ledger_repository: RepoInstanceId,
    worktree_context: &WorktreeContext,
    reconciliation_evidence_context: &mut ReconciliationEvidenceContext<'_>,
) -> Result<ReconciliationPlan, ReconciliationBuildError> {
    let common_git_directory = worktree_context.common_git_directory();
    let repository_root = worktree_context.repository_root();
    let ObservedRepositoryFacts {
        worktree_registry,
        repository_trunk,
        integration_reachability,
        repository_evidence_observations,
        trunk_resolution_calls,
    } = observe_repository_facts(
        reservations,
        ordering_graph,
        worktree_context,
        reconciliation_evidence_context,
    )?;
    let ReconciledReservations {
        changes,
        alert_subjects,
        snapshots: reservation_snapshots,
    } = reconcile_observed_reservations(
        reservations,
        ordering_graph,
        LedgerCheckoutIdentity {
            repository: ledger_repository,
            common_git_directory,
        },
        &worktree_registry,
        repository_evidence_observations,
    )?;
    let repository_snapshot =
        RepositorySnapshot::new(repository_trunk, reservation_snapshots.clone(), Vec::new());
    let active_holders = reservations
        .iter()
        .filter(|reservation| matches!(reservation.lifecycle(), ReservationLifecycle::Active))
        .map(|reservation| ActiveHolder {
            worktree_id:         reservation.actor().worktree,
            coordination_run_id: reservation.actor().run,
        })
        .collect();
    let mut plan = ReconciliationPlan {
        operations: changes.operations,
        action:     ReconciliationAction {
            active_holders,
            marker_contexts: worktree_registry.marker_sweep_contexts(common_git_directory),
            repository_root: repository_root.to_path_buf(),
            retention_repairs: changes.retention_repairs,
            retention_deletions: changes.retention_deletions,
            retention_commit_resolution: RetentionCommitResolution::InitialObservation(
                integration_reachability.resolved_candidates,
            ),
            alert_subjects,
            settlements: Vec::new(),
            evidence: changes.evidence,
            repository_snapshot,
            recovered_bypass_reporting: RecoveredBypassReporting::Defer,
            recovered_bypass_markers: Vec::new(),
            pending_bypass_imports: Vec::new(),
            completed_rewrite_markers: Vec::new(),
            updated_rewrite_markers: Vec::new(),
            unrecorded_bypass_occurrences: Vec::new(),
            trunk_resolution_calls,
            merge_extent_git_cost: MergeExtentGitCost::default(),
        },
    };
    complete_reconciliation_plan(
        reservations,
        ordering_graph,
        repository_observation_scope,
        reservation_snapshots,
        reconciliation_evidence_context,
        &mut plan,
    )?;
    Ok(plan)
}

/// Observe successors against the lifecycle this same actual-trunk transaction will commit.
fn complete_reconciliation_plan(
    reservations: &RetainedReservationSet,
    ordering_graph: &OrderingGraph,
    observation_scope: RepositoryObservationScope,
    mut snapshots: Vec<RepositoryReservationSnapshot>,
    context: &mut ReconciliationEvidenceContext<'_>,
    plan: &mut ReconciliationPlan,
) -> Result<(), ReservationReplayError> {
    let extents = derive_merge_extents(
        reservations,
        &plan.action.repository_snapshot,
        &plan.operations,
        &mut plan.action.merge_extent_git_cost,
    )?;
    plan.operations.extend(extents.operations);
    append_evidence_operations(reservations, plan)?;
    append_settlement_operations(
        reservations,
        ordering_graph,
        &extents.committed_by_holder,
        plan,
    )?;
    let merged_runs = match context.merged_run_endings {
        MergedRunEndings::End => append_merged_run_endings(reservations, plan),
        MergedRunEndings::Defer => Vec::new(),
    };
    for snapshot in &mut snapshots {
        if let Some(merged_run) = merged_runs
            .iter()
            .find(|merged_run| merged_run.reservation_id == snapshot.reservation_id)
        {
            snapshot.evidence = RepositoryReservationEvidence::Released {
                protected_tip:      merged_run.protected_tip.clone(),
                disposition:        ReleaseDisposition::Integrated,
                integration_status: merged_run.integration_status.clone(),
            };
        }
        for operation in &plan.operations {
            if let JournalOperation::Release {
                reservation_id,
                disposition,
            } = operation
                && *reservation_id == snapshot.reservation_id
                && let RepositoryReservationEvidence::Outstanding {
                    protected_tip,
                    integration_status,
                } = &snapshot.evidence
            {
                snapshot.evidence = RepositoryReservationEvidence::Released {
                    protected_tip:      protected_tip.clone(),
                    integration_status: integration_status.clone(),
                    disposition:        disposition.clone(),
                };
            }
        }
    }
    let successors = successor_incorporation_evidence(
        &plan.action.repository_root,
        reservations,
        ordering_graph,
        observation_scope,
        &snapshots,
        context.successor_scoped_patch_evaluation_budget,
    )?;
    plan.operations.extend(successors.operations);
    plan.action.repository_snapshot = RepositorySnapshot::new(
        plan.action.repository_snapshot.trunk().clone(),
        snapshots,
        successors.by_predecessor,
    );
    Ok(())
}

/// Read the worktree registry, trunk reachability, and per-reservation repository evidence in one
/// pass, so every judgement the plan then makes is measured against the same repository.
fn observe_repository_facts(
    reservations: &RetainedReservationSet,
    ordering_graph: &OrderingGraph,
    worktree_context: &WorktreeContext,
    reconciliation_evidence_context: &mut ReconciliationEvidenceContext<'_>,
) -> Result<ObservedRepositoryFacts, ReconciliationBuildError> {
    let repository_root = worktree_context.repository_root();
    thread::scope(|scope| {
        let worktree_registry = scope.spawn(|| WorktreeRegistry::read(worktree_context));
        let (repository_trunk, integration_reachability) =
            BatchedIntegrationReachability::observe_configured_trunk(
                repository_root,
                reservations,
                ordering_graph,
                &reconciliation_evidence_context.berth_config.trunk,
            )?;
        let mut target_evidence_context = TargetIntegrationEvidenceContext {
            repository_root,
            repository_trunk: &repository_trunk,
            integration_reachability: &integration_reachability,
            scoped_patch_evaluation_budget: reconciliation_evidence_context
                .scoped_patch_evaluation_budget,
        };
        let mut indexed_evidence = scoped_patch_evaluation_order(reservations, &repository_trunk)
            .into_iter()
            .map(|(index, reservation)| {
                repository_evidence(reservation, &mut target_evidence_context)
                    .map(|observation| (index, observation))
            })
            .collect::<Result<Vec<_>, ReservationReplayError>>()?;
        let worktree_registry = worktree_registry
            .join()
            .map_err(|_| WorktreeRegistryError::ObservationWorkerPanicked)??;
        indexed_evidence.sort_by_key(|(index, _)| *index);
        Ok::<_, ReconciliationBuildError>(ObservedRepositoryFacts {
            worktree_registry,
            repository_trunk,
            integration_reachability,
            repository_evidence_observations: indexed_evidence
                .into_iter()
                .map(|indexed_observation| indexed_observation.1)
                .collect(),
            trunk_resolution_calls: 1,
        })
    })
}

/// Apply the observed repository facts to every retained reservation, collecting the journal
/// updates, alert subjects, and snapshot rows the plan is assembled from.
fn reconcile_observed_reservations(
    reservations: &RetainedReservationSet,
    ordering_graph: &OrderingGraph,
    ledger_checkout: LedgerCheckoutIdentity<'_>,
    worktree_registry: &WorktreeRegistry,
    repository_evidence_observations: Vec<RepositoryEvidenceObservation>,
) -> Result<ReconciledReservations, ReservationReplayError> {
    let mut changes = ReconciliationChanges::default();
    let mut alert_subjects = Vec::new();
    let mut snapshots = Vec::new();
    for (reservation, repository_evidence_observation) in
        reservations.iter().zip(repository_evidence_observations)
    {
        let observation = worktree_registry.classify(
            ledger_checkout.repository,
            ledger_checkout.common_git_directory,
            reservation,
        );
        if let WorktreeRelocation::Relocated { current_root } = &observation.relocation {
            changes.operations.push(JournalOperation::RelocateWorktree {
                reservation_id: reservation.id(),
                worktree_id:    reservation.actor().worktree,
                previous_root:  reservation.worktree_root().clone(),
                current_root:   current_root.clone(),
            });
        }
        alert_subjects.push(AlertSubject {
            reservation_id:    reservation.id(),
            worktree_liveness: observation.liveness,
        });
        append_evidence_and_retention(
            reservation,
            &repository_evidence_observation,
            reservations,
            ordering_graph,
            &mut changes,
        )?;
        snapshots.push(RepositoryReservationSnapshot {
            reservation_id:    reservation.id(),
            worktree_liveness: observation.liveness,
            worktree_head:     observation.head,
            evidence:          repository_evidence_observation.evidence,
        });
    }
    Ok(ReconciledReservations {
        changes,
        alert_subjects,
        snapshots,
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
/// Prepared decisions project actual-trunk settlements before observing the proposal. Committed
/// audits keep replayed lifecycles so newly integrated reservations still enter the permit audit.
/// Proposed-trunk evidence never settles reservations.
///
/// Each observed trunk target uses one `cat-file` batch and one grouped `rev-list` to classify all
/// integration-proof ancestors. Graph predecessor queries use one grouped `rev-list` for every
/// incorporation subject and successor head. The initial object batch also supplies retention
/// availability; newly settled witnesses require one additional batch of the final repairs.
/// One `update-ref` transaction applies every repair and deletion after append succeeds.
/// These invocation counts are independent of the total retained-reservation count. Scoped patch
/// comparisons reuse identical proof inputs and evaluate at most one distinct proof subject for
/// each observed trunk target.
pub(crate) fn prepare_gate_reconciliation(
    state: &ReplayedLedgerState<'_>,
    worktree_context: &WorktreeContext,
    ledger_repository: RepoInstanceId,
    berth_config: &BerthConfig,
    proposed_trunk: GitObjectId,
    purpose: GateReconciliationPurpose,
    rewrite_preflight: RewriteReconciliationPreflight,
) -> Result<GateReconciliation, GateReconciliationError> {
    let reservations = RetainedReservationSet::replay(state.events())
        .map_err(GateReconciliationError::Reservation)?;
    let reservations = rewrite_preflight.project(&reservations)?;
    let ordering_graph =
        OrderingGraph::replay(state.events()).map_err(GateReconciliationError::Edge)?;
    let mut scoped_patch_evaluation_budget = rewrite_preflight.budget;
    let mut successor_scoped_patch_evaluation_budget =
        ReconciliationSuccessorScopedPatchEvaluationBudget::default();
    // A gate decides one ref update; runs end on the ordinary reconciliation that follows.
    let mut reconciliation_evidence_context = ReconciliationEvidenceContext {
        merged_run_endings: MergedRunEndings::Defer,
        berth_config,
        scoped_patch_evaluation_budget: &mut scoped_patch_evaluation_budget,
        successor_scoped_patch_evaluation_budget: &mut successor_scoped_patch_evaluation_budget,
    };
    let mut reconciliation = build_plan(
        &reservations,
        &ordering_graph,
        RepositoryObservationScope::CurrentOrderingGraph,
        ledger_repository,
        worktree_context,
        &mut reconciliation_evidence_context,
    )
    .map_err(|error| match error {
        ReconciliationBuildError::Reservation(error) => GateReconciliationError::Reservation(error),
        ReconciliationBuildError::WorktreeRegistry(error) => {
            GateReconciliationError::WorktreeRegistry(error)
        },
    })?;
    let mut operations = rewrite_preflight.operations;
    operations.append(&mut reconciliation.operations);
    reconciliation.operations = operations;
    reconciliation.action.completed_rewrite_markers = rewrite_preflight.completed_markers;
    reconciliation.action.updated_rewrite_markers = rewrite_preflight.updated_markers;
    reconciliation.action.trunk_resolution_calls += rewrite_preflight.trunk_resolution_calls;
    let reservations = match purpose {
        GateReconciliationPurpose::PreparedDecision => reservations
            .with_pending_settlements(&reconciliation.operations)
            .map_err(GateReconciliationError::Reservation)?,
        GateReconciliationPurpose::CommittedAudit => reservations,
    };
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
        .integration_constraints(
            &reservations,
            &proposed_observation.snapshot,
            state.generation(),
        )
        .map_err(GateReconciliationError::MissingReadinessFact)?;
    Ok(GateReconciliation {
        reconciliation,
        constraints,
        reservations,
        deferred_rewrite_subjects: rewrite_preflight.deferred_subjects,
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
        ordering_graph,
        &repository_trunk,
    )
    .map_err(GateReconciliationError::Reservation)?;
    let mut target_evidence_context = TargetIntegrationEvidenceContext {
        repository_root:                worktree_context.repository_root(),
        repository_trunk:               &repository_trunk,
        integration_reachability:       &integration_reachability,
        scoped_patch_evaluation_budget: reconciliation_evidence_context
            .scoped_patch_evaluation_budget,
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

    /// Keep an unaccepted rewrite subject entering when any nominated destination enters trunk.
    pub(crate) fn deferred_rewrite_enters(
        &self,
        reservation_id: ReservationId,
        newly_reachable: &[GitObjectId],
    ) -> bool {
        self.deferred_rewrite_subjects.iter().any(|subject| {
            subject.validation.reservation_id == reservation_id
                && subject
                    .rewritten_tips
                    .iter()
                    .any(|tip| newly_reachable.contains(tip))
        })
    }

    /// Retain durable content checks while leaving actual-trunk lifecycle updates to commands.
    pub(crate) fn into_committed_hook_action(
        self,
        additional_operations: Vec<JournalOperation>,
    ) -> (Vec<JournalOperation>, CommittedHookReconciliationAction) {
        let mut operations = self
            .reconciliation
            .operations
            .into_iter()
            .filter(|operation| {
                // Accepted resnapshots move outstanding anchors without releasing them.
                // The remaining records retain immutable content checks and comparison ordering.
                // Evidence and lifecycle changes remain the responsibility of commands.
                matches!(
                    operation,
                    JournalOperation::Resnapshot { .. }
                        | JournalOperation::ScopedPatchEquivalenceChecked { .. }
                        | JournalOperation::ScopedPatchComparisonAttempted { .. }
                        | JournalOperation::SuccessorScopedPatchEquivalenceChecked { .. }
                        | JournalOperation::SuccessorScopedPatchComparisonAttempted { .. }
                )
            })
            .collect::<Vec<_>>();
        let retention_repairs = operations
            .iter()
            .filter_map(|operation| match operation {
                JournalOperation::Resnapshot {
                    reservation_id,
                    snapshot: ReservationSnapshot::Outstanding { protected_tip, .. },
                } => Some((*reservation_id, protected_tip.as_ref().clone())),
                _ => None,
            })
            .collect::<HashMap<_, _>>()
            .into_iter()
            .map(|(id, tip)| ReservationRetentionRefRepair::new(id, tip))
            .collect();
        operations.extend(additional_operations);
        (
            operations,
            CommittedHookReconciliationAction {
                repository_root: self.reconciliation.action.repository_root,
                retention_repairs,
                completed_markers: self.reconciliation.action.completed_rewrite_markers,
                updated_markers: self.reconciliation.action.updated_rewrite_markers,
            },
        )
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

/// Post-append repairs for an audit that preserves reservation lifecycles.
pub(crate) struct CommittedHookReconciliationAction {
    /// Repository containing the accepted rewritten commits.
    repository_root:   PathBuf,
    /// Final accepted tip for each rewritten outstanding reservation.
    retention_repairs: Vec<ReservationRetentionRefRepair>,
    /// Fully imported maps retired after their resnapshots commit.
    completed_markers: Vec<PendingBranchRewriteMarker>,
    /// Partial import progress retained for the next pass.
    updated_markers:   Vec<PendingBranchRewriteMarker>,
}

impl CommittedHookReconciliationAction {
    /// Apply only side effects supported by the filtered committed-hook records.
    pub(crate) fn commit(
        self,
        _state: &ReplayedLedgerState<'_>,
        _recoverable_failures: &RecoverableReconciliationAppendFailures,
    ) -> Result<(), ReconcileError> {
        if !self.retention_repairs.is_empty() {
            git::update_reservation_retention_refs(
                &self.repository_root,
                &self.retention_repairs,
                &[],
            )?;
        }
        permit::update_branch_rewrite_markers(&self.updated_markers).map_err(LedgerError::Io)?;
        permit::delete_branch_rewrite_markers(&self.completed_markers).map_err(LedgerError::Io)?;
        Ok(())
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
    /// A concurrent transaction changed an accepted rewrite subject.
    RewriteSubjectChanged(ReservationId),
}

impl Display for GateReconciliationError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::RewriteSubjectChanged(id) => write!(
                formatter,
                "reservation {id} changed during rewrite acceptance; retry reconciliation"
            ),
            Self::Reservation(error) => error.fmt(formatter),
            Self::Edge(error) => error.fmt(formatter),
            Self::WorktreeRegistry(error) => error.fmt(formatter),
            Self::MissingReadinessFact(error) => error.fmt(formatter),
        }
    }
}

impl From<RewriteProjectionError> for GateReconciliationError {
    fn from(error: RewriteProjectionError) -> Self {
        match error {
            RewriteProjectionError::Reservation(error) => Self::Reservation(error),
            RewriteProjectionError::RewriteSubjectChanged(id) => Self::RewriteSubjectChanged(id),
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
            integration_status_with_retained_verdict(
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
            IntegrationStatusObservation {
                status:                  match target_evidence_context.repository_trunk {
                    RepositoryTrunk::Resolved(trunk) => {
                        trunk_commit.revalidate_ancestry(trunk, |witness| {
                            target_evidence_context
                                .integration_reachability
                                .for_ancestor(witness)
                        })
                    },
                    RepositoryTrunk::ObjectUnknown => IntegrationEvidenceStatus::ObjectUnknown,
                },
                revalidation:            EvidenceRevalidationObservation::Apply,
                scoped_patch_comparison: ScopedPatchComparisonJournalUpdate::Unchanged,
            }
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
        RepositoryTrunk::Resolved(current_trunk_oid) => integration_status_with_retained_verdict(
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

fn integration_status_with_retained_verdict(
    target_evidence_context: &mut TargetIntegrationEvidenceContext<'_>,
    reservation: &Reservation,
    protected_tip: &ProtectedReservationTip,
    target: &GitObjectId,
    scoped_patch_evaluation_context: ScopedPatchEvaluationContext,
    materialized: &IntegrationEvidenceStatus,
) -> IntegrationStatusObservation {
    let subject = reservation.integration_proof_subject_revision();
    match reservation
        .retained_scoped_patch_target_verdicts()
        .lookup(subject, target)
    {
        ScopedPatchTargetVerdictAvailability::Hit {
            comparison,
            witness,
        } => IntegrationStatusObservation {
            status:                  integration_status_from_retained_scoped_patch_comparison(
                comparison,
                witness,
                target,
                &scoped_patch_evaluation_context,
                target_evidence_context.integration_reachability,
            ),
            revalidation:            EvidenceRevalidationObservation::Apply,
            scoped_patch_comparison: ScopedPatchComparisonJournalUpdate::Unchanged,
        },
        ScopedPatchTargetVerdictAvailability::Miss => {
            let repository_root = target_evidence_context.repository_root;
            let integration_reachability = target_evidence_context.integration_reachability;
            let scoped_patch_evaluation_budget =
                &mut *target_evidence_context.scoped_patch_evaluation_budget;
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
                destination:      ScopedPatchComparisonDestination::Trunk,
            };
            let observe_scoped_patch_comparison = || {
                scoped_patch_evaluation_budget.evaluate(scoped_patch_evaluation_key, || {
                    evaluate_reservation_scoped_integration(
                        repository_root,
                        reservation,
                        protected_tip,
                        target,
                        integration_reachability,
                    )
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
                IntegrationEvidenceObservation::ScopedPatchComparison { status, evaluation } => {
                    let scoped_patch_comparison =
                        scoped_patch_journal_update(subject, target, &status, &evaluation);
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

/// Evaluate historical and current locations for one admitted reservation subject.
fn evaluate_reservation_scoped_integration(
    repository_root: &Path,
    reservation: &Reservation,
    protected_tip: &ProtectedReservationTip,
    target: &GitObjectId,
    integration_reachability: &BatchedIntegrationReachability,
) -> ScopedPatchIntegrationEvaluation {
    evaluate_historical_then_current_trunk(
        target,
        || {
            git::discover_historical_integration_candidate(
                repository_root,
                reservation.phase_start_head().as_ref(),
                protected_tip.as_ref(),
                target,
                &integration_reachability.target_histories,
            )
        },
        |destination| {
            let history = if destination == target {
                integration_reachability
                    .target_history_after_phase_start(reservation.phase_start_head().as_ref())
            } else {
                ScopedPatchTargetHistory::NeedsGitQueries
            };
            git::scoped_patch_equivalence_with_target_history(
                repository_root,
                reservation.phase_start_head().as_ref(),
                reservation.scopes(),
                protected_tip.as_ref(),
                destination,
                history,
            )
            .unwrap_or(ScopedPatchComparison::Unavailable)
        },
    )
}

/// Certify a nominated historical witness before falling back to current trunk replay.
/// Both operations belong to the same admitted proof subject and observed trunk.
fn evaluate_historical_then_current_trunk(
    target: &GitObjectId,
    discover: impl FnOnce() -> HistoricalIntegrationCandidateDiscovery,
    mut compare: impl FnMut(&GitObjectId) -> ScopedPatchComparison,
) -> ScopedPatchIntegrationEvaluation {
    let historical_unavailable = match discover() {
        HistoricalIntegrationCandidateDiscovery::Nominated(candidate) => {
            match compare(&candidate) {
                ScopedPatchComparison::Equivalent => {
                    return ScopedPatchIntegrationEvaluation::Equivalent(
                        IntegrationWitness::Historical(RewrittenIntegrationTrunkCommit::from(
                            candidate,
                        )),
                    );
                },
                ScopedPatchComparison::Different => false,
                ScopedPatchComparison::Unavailable => true,
            }
        },
        HistoricalIntegrationCandidateDiscovery::NoMatch => false,
        HistoricalIntegrationCandidateDiscovery::Unavailable => true,
    };
    match compare(target) {
        ScopedPatchComparison::Different if historical_unavailable => {
            ScopedPatchIntegrationEvaluation::HistoricalEvidenceUnavailable
        },
        comparison => comparison.into(),
    }
}

fn scoped_patch_journal_update(
    subject: IntegrationProofSubjectRevision,
    target: &GitObjectId,
    status: &IntegrationEvidenceStatus,
    evaluation: &ScopedPatchIntegrationEvaluation,
) -> ScopedPatchComparisonJournalUpdate {
    if matches!(
        evaluation,
        ScopedPatchIntegrationEvaluation::HistoricalEvidenceUnavailable
    ) {
        return ScopedPatchComparisonJournalUpdate::Attempted {
            subject,
            target: target.clone(),
        };
    }
    match status {
        IntegrationEvidenceStatus::Integrated {
            proof: IntegrationProof::ScopedPatchEquivalent,
            witness,
            ..
        } => ScopedPatchComparisonJournalUpdate::Checked {
            subject,
            target: target.clone(),
            verdict: ScopedPatchEquivalenceVerdict::Integrated,
            witness: witness.clone(),
        },
        IntegrationEvidenceStatus::TrunkRewritten => ScopedPatchComparisonJournalUpdate::Checked {
            subject,
            target: target.clone(),
            verdict: ScopedPatchEquivalenceVerdict::TrunkRewritten,
            witness: IntegrationWitness::EvaluatedTrunk,
        },
        IntegrationEvidenceStatus::NotIntegrated => ScopedPatchComparisonJournalUpdate::Checked {
            subject,
            target: target.clone(),
            verdict: ScopedPatchEquivalenceVerdict::NotIntegrated,
            witness: IntegrationWitness::EvaluatedTrunk,
        },
        IntegrationEvidenceStatus::ObjectUnknown => ScopedPatchComparisonJournalUpdate::Attempted {
            subject,
            target: target.clone(),
        },
        IntegrationEvidenceStatus::Integrated {
            proof:
                IntegrationProof::ProtectedTipAncestor | IntegrationProof::RewrittenWitnessAncestor,
            ..
        } => ScopedPatchComparisonJournalUpdate::Unchanged,
    }
}

fn integration_status_from_retained_scoped_patch_comparison(
    scoped_patch_comparison: DurableScopedPatchComparison,
    witness: IntegrationWitness,
    target: &GitObjectId,
    scoped_patch_evaluation_context: &ScopedPatchEvaluationContext,
    integration_reachability: &BatchedIntegrationReachability,
) -> IntegrationEvidenceStatus {
    match scoped_patch_comparison {
        DurableScopedPatchComparison::Equivalent => IntegrationEvidenceStatus::Integrated {
            trunk_oid: target.clone(),
            proof: IntegrationProof::ScopedPatchEquivalent,
            witness,
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
    let (retained_commit, evidence, retention) = match &repository_evidence_observation.evidence {
        RepositoryReservationEvidence::Active
        | RepositoryReservationEvidence::ReleasedWithoutCheckpoint { .. } => return Ok(()),
        RepositoryReservationEvidence::Outstanding {
            protected_tip,
            integration_status,
        } => (
            protected_tip.as_ref(),
            integration_status,
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
            let retained_commit = match disposition.revalidation_subject() {
                ReleaseRevalidationSubject::RewrittenIntegration(witness) => witness.as_ref(),
                ReleaseRevalidationSubject::ProtectedTip | ReleaseRevalidationSubject::None => {
                    protected_tip.as_ref()
                },
            };
            (retained_commit, integration_status, retention)
        },
    };
    match retention {
        RetentionDecision::Repair => {
            changes
                .retention_repairs
                .push(git::ReservationRetentionRefRepair::new(
                    reservation.id(),
                    retained_commit.clone(),
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
    if materialized != *evidence {
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

/// Whether actual-trunk evidence permits ending this outstanding reservation.
#[derive(Debug, Eq, PartialEq)]
enum SettlementSelection {
    /// The lifecycle, target, or remaining branch work prevents settlement.
    Unchanged,
    /// The complete protected work reached the actual trunk under this disposition.
    Release(ReleaseDisposition),
}

/// Select from state so a previously appended proof can finish settlement after a restart.
fn settlement_selection(
    reservation: &Reservation,
    evidence: &IntegrationEvidenceStatus,
    actual_trunk: &RepositoryTrunk,
    merge_extent: &MergeExtent,
    committed_paths: &CommittedMergeEvidence,
) -> SettlementSelection {
    if !matches!(
        reservation.lifecycle(),
        ReservationLifecycle::Outstanding { .. }
    ) {
        return SettlementSelection::Unchanged;
    }
    let has_unproven_work = match committed_paths {
        CommittedMergeEvidence::Observed(paths) => {
            reservation.has_unproven_merge_work(merge_extent, paths)
        },
        CommittedMergeEvidence::Unavailable => {
            reservation.has_unproven_retained_merge_work(merge_extent)
        },
    };
    if has_unproven_work {
        return SettlementSelection::Unchanged;
    }
    let (
        IntegrationEvidenceStatus::Integrated {
            trunk_oid,
            proof,
            witness,
        },
        RepositoryTrunk::Resolved(actual),
    ) = (evidence, actual_trunk)
    else {
        return SettlementSelection::Unchanged;
    };
    if trunk_oid != actual {
        return SettlementSelection::Unchanged;
    }
    SettlementSelection::Release(match proof {
        IntegrationProof::ProtectedTipAncestor => ReleaseDisposition::Integrated,
        IntegrationProof::ScopedPatchEquivalent | IntegrationProof::RewrittenWitnessAncestor => {
            ReleaseDisposition::RewrittenIntegration(witness.resolve(trunk_oid))
        },
    })
}

/// Finish ordinary actual-trunk settlement after fresh merge extents and evidence are planned.
fn append_settlement_operations(
    reservations: &RetainedReservationSet,
    ordering_graph: &OrderingGraph,
    committed_by_holder: &HashMap<WorktreeId, CommittedMergeEvidence>,
    reconciliation: &mut ReconciliationPlan,
) -> Result<(), ReservationReplayError> {
    let mut settled = HashMap::new();
    for reservation in reservations.iter() {
        let ReservationEvidenceState::Outstanding {
            integration_status, ..
        } = reservation.evidence_state()?
        else {
            continue;
        };
        let evidence = reconciliation
            .action
            .evidence
            .iter()
            .find(|evidence| evidence.reservation_id == reservation.id())
            .map_or(&integration_status, |evidence| &evidence.status);
        let extent = reconciliation
            .operations
            .iter()
            .rev()
            .find_map(|operation| match operation {
                JournalOperation::MergeExtentObserved {
                    reservation_id,
                    extent,
                    ..
                } if *reservation_id == reservation.id() => Some(extent),
                _ => None,
            })
            .unwrap_or_else(|| reservation.merge_extent());
        let SettlementSelection::Release(disposition) = settlement_selection(
            reservation,
            evidence,
            reconciliation.action.repository_snapshot.trunk(),
            extent,
            committed_by_holder
                .get(&reservation.actor().worktree)
                .unwrap_or(&CommittedMergeEvidence::Unavailable),
        ) else {
            continue;
        };
        let edit_blocking_status = reservation
            .with_merge_extent(extent.clone())
            .edit_blocking_status();
        reconciliation.operations.retain(|operation| !matches!(operation,
            JournalOperation::EvidenceRevalidated { reservation_id, .. } if *reservation_id == reservation.id()
        ));
        reconciliation
            .operations
            .push(JournalOperation::EvidenceRevalidated {
                reservation_id: reservation.id(),
                status: evidence.clone(),
                edit_blocking_status,
            });
        reconciliation.operations.push(JournalOperation::Release {
            reservation_id: reservation.id(),
            disposition:    disposition.clone(),
        });
        reconciliation
            .action
            .settlements
            .push(ReconciledSettlement {
                reservation_id: reservation.id(),
                disposition:    disposition.clone(),
            });
        settled.insert(reservation.id(), disposition);
    }
    rebuild_settlement_retention(reservations, ordering_graph, &settled, reconciliation)
}

/// An active run this pass ended because trunk already contains all of its work.
struct MergedRunEnding {
    reservation_id:     ReservationId,
    protected_tip:      ProtectedReservationTip,
    integration_status: IntegrationEvidenceStatus,
}

/// End every active run whose work trunk now contains, with no release command.
///
/// A run qualifies once it has done work — an earlier observation found unmerged paths, or its
/// checkout has committed past the phase start — and this pass observes a clean checkout with
/// no net branch change at a head trunk contains. Nothing is left to protect, so the run
/// checkpoints at that head and settles in the same append. The result is the ordinary
/// integrated release, revalidated on later reads like any other. A fresh claim that has not
/// written yet has neither kind of work and keeps running.
fn append_merged_run_endings(
    reservations: &RetainedReservationSet,
    reconciliation: &mut ReconciliationPlan,
) -> Vec<MergedRunEnding> {
    let mut merged_runs = Vec::new();
    for reservation in reservations.iter() {
        if !matches!(reservation.lifecycle(), ReservationLifecycle::Active) {
            continue;
        }
        let extent = reconciliation
            .operations
            .iter()
            .rev()
            .find_map(|operation| match operation {
                JournalOperation::MergeExtentObserved {
                    reservation_id,
                    extent,
                    ..
                } if *reservation_id == reservation.id() => Some(extent),
                _ => None,
            })
            .unwrap_or_else(|| reservation.merge_extent());
        let MergeExtent::Empty { key } = extent else {
            continue;
        };
        // Enrollment requires a nonempty observed footprint. Its immutable source preserves
        // that evidence even if a preceding drift or gate already recorded an empty extent.
        let did_work = matches!(reservation.source(), ClaimSource::Enrolled)
            || reservation.merge_extent().observed_unmerged_work()
            || key.head != *reservation.phase_start_head().as_ref();
        if !did_work {
            continue;
        }
        if key.head != key.trunk {
            reconciliation.action.merge_extent_git_cost.path_queries += 1;
            let reachability = git::reachability(
                &reconciliation.action.repository_root,
                &key.head,
                &key.trunk,
            );
            if !matches!(reachability, Ok(Reachability::Ancestor)) {
                continue;
            }
        }
        let protected_tip = ProtectedReservationTip::from(key.head.clone());
        let integration_status = IntegrationEvidenceStatus::Integrated {
            trunk_oid: key.trunk.clone(),
            proof:     IntegrationProof::ProtectedTipAncestor,
            witness:   IntegrationWitness::EvaluatedTrunk,
        };
        let trunk_snapshot = key.trunk.clone();
        reconciliation
            .operations
            .push(JournalOperation::Checkpoint {
                reservation_id: reservation.id(),
                protected_tip: protected_tip.clone(),
                trunk_snapshot,
            });
        reconciliation
            .operations
            .push(JournalOperation::EvidenceRevalidated {
                reservation_id:       reservation.id(),
                status:               integration_status.clone(),
                edit_blocking_status: EditBlockingStatus::Clear,
            });
        reconciliation.operations.push(JournalOperation::Release {
            reservation_id: reservation.id(),
            disposition:    ReleaseDisposition::Integrated,
        });
        reconciliation
            .action
            .settlements
            .push(ReconciledSettlement {
                reservation_id: reservation.id(),
                disposition:    ReleaseDisposition::Integrated,
            });
        merged_runs.push(MergedRunEnding {
            reservation_id: reservation.id(),
            protected_tip,
            integration_status,
        });
    }
    merged_runs
}

/// Choose one final ref action per reservation after this pass selects its settlements.
fn rebuild_settlement_retention(
    reservations: &RetainedReservationSet,
    ordering_graph: &OrderingGraph,
    settled: &HashMap<ReservationId, ReleaseDisposition>,
    reconciliation: &mut ReconciliationPlan,
) -> Result<(), ReservationReplayError> {
    // Rebuild the ref actions so no transaction updates and deletes the same ref. These
    // actions still run only after the entire evidence-and-release append succeeds.
    reconciliation.action.retention_repairs.clear();
    reconciliation.action.retention_deletions.clear();
    for reservation in reservations.iter() {
        let (protected_tip, disposition) = match reservation.evidence_state()? {
            ReservationEvidenceState::Outstanding { protected_tip, .. } => {
                (protected_tip, settled.get(&reservation.id()))
            },
            ReservationEvidenceState::Released { protected_tip, .. } => {
                let ReservationLifecycle::Released { disposition } = reservation.lifecycle() else {
                    continue;
                };
                (protected_tip, Some(disposition))
            },
            ReservationEvidenceState::Active { .. }
            | ReservationEvidenceState::ReleasedWithoutCheckpoint { .. } => continue,
        };
        if disposition.is_some()
            && !ordering_graph.has_nonterminal_dependent(reservation.id(), reservations)?
        {
            reconciliation
                .action
                .retention_deletions
                .push(reservation.id());
        } else {
            let retained_commit = match disposition {
                Some(ReleaseDisposition::RewrittenIntegration(witness)) => {
                    if settled.contains_key(&reservation.id()) {
                        reconciliation.action.retention_commit_resolution =
                            RetentionCommitResolution::IncludeSettlementWitness;
                    }
                    witness.as_ref()
                },
                _ => protected_tip.as_ref(),
            };
            reconciliation
                .action
                .retention_repairs
                .push(ReservationRetentionRefRepair::new(
                    reservation.id(),
                    retained_commit.clone(),
                ));
        }
    }
    Ok(())
}

/// Construct evidence records only after the transaction has planned its merge extents.
/// Ordinary reconciliation appends settlements after these evidence records.
fn append_evidence_operations(
    reservations: &RetainedReservationSet,
    reconciliation: &mut ReconciliationPlan,
) -> Result<(), ReservationReplayError> {
    // Scheduling records follow the evidence they accompany, even though both wait
    // for extent derivation. Keep their relative order for durable retry priorities.
    let (scheduling_operations, mut operations): (Vec<_>, Vec<_>) =
        reconciliation.operations.drain(..).partition(|operation| {
            matches!(
                operation,
                JournalOperation::ScopedPatchEquivalenceChecked { .. }
                    | JournalOperation::ScopedPatchComparisonAttempted { .. }
                    | JournalOperation::SuccessorScopedPatchEquivalenceChecked { .. }
                    | JournalOperation::SuccessorScopedPatchComparisonAttempted { .. }
            )
        });
    for evidence in &reconciliation.action.evidence {
        let reservation = reservations.reservation(evidence.reservation_id)?;
        let planned_extent = operations
            .iter()
            .rev()
            .find_map(|operation| match operation {
                JournalOperation::MergeExtentObserved {
                    reservation_id,
                    extent,
                    ..
                } if *reservation_id == reservation.id() => Some(extent),
                _ => None,
            });
        let edit_blocking_status = planned_extent.map_or_else(
            || reservation.edit_blocking_status(),
            |extent| {
                reservation
                    .with_merge_extent(extent.clone())
                    .edit_blocking_status()
            },
        );
        operations.push(JournalOperation::EvidenceRevalidated {
            reservation_id: reservation.id(),
            status: evidence.status.clone(),
            edit_blocking_status,
        });
    }
    operations.extend(scheduling_operations);
    reconciliation.operations = operations;
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
            witness,
        } => operations.push(JournalOperation::ScopedPatchEquivalenceChecked {
            reservation_id:    reservation.id(),
            subject:           *subject,
            target:            target.clone(),
            verdict:           *verdict,
            witness:           witness.clone(),
            evaluator_version: ScopedPatchEvaluatorVersion::HistoricalCandidate,
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

/// Prove which successors already carry each predecessor's work.
///
/// Reachability is classified for every predecessor at once; only the successors reachability
/// leaves undecided reach the scoped-patch budget, which admits one comparison per reconciliation.
fn successor_incorporation_evidence(
    repository_root: &Path,
    reservations: &RetainedReservationSet,
    ordering_graph: &OrderingGraph,
    repository_observation_scope: RepositoryObservationScope,
    reservation_snapshots: &[RepositoryReservationSnapshot],
    evaluation_budget: &mut ReconciliationSuccessorScopedPatchEvaluationBudget,
) -> Result<SuccessorIncorporationObservation, ReservationReplayError> {
    let evidence_subjects = predecessor_successor_evidence_subjects(
        reservations,
        ordering_graph,
        repository_observation_scope,
        reservation_snapshots,
    )?;
    let classification = classify_successor_incorporation(repository_root, evidence_subjects);
    Ok(settle_pending_scoped_patch_comparisons(
        repository_root,
        classification,
        evaluation_budget,
    ))
}

/// Collect every predecessor holding a protected tip whose successors have resolvable heads.
///
/// Predecessors are visited in wire order so the candidates a later pass ranks are built in a
/// stable sequence regardless of how the ordering graph enumerated them.
fn predecessor_successor_evidence_subjects<'reservation>(
    reservations: &'reservation RetainedReservationSet,
    ordering_graph: &OrderingGraph,
    repository_observation_scope: RepositoryObservationScope,
    reservation_snapshots: &[RepositoryReservationSnapshot],
) -> Result<Vec<PredecessorSuccessorEvidenceSubject<'reservation>>, ReservationReplayError> {
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
    let ordered_predecessor_ids =
        WireOrderedReservationIds::sorted(successors_by_predecessor.keys().copied().collect());
    let mut evidence_subjects = Vec::new();
    for &predecessor_id in ordered_predecessor_ids.as_slice() {
        // Every ordered identifier is a key of the map the ordering was built from, so this
        // takes each predecessor's successors once rather than looking one up and reading a
        // miss as a skip condition it can never be.
        let successors = successors_by_predecessor
            .remove(&predecessor_id)
            .unwrap_or_default();
        let Some(predecessor_snapshot) = snapshots_by_reservation.get(&predecessor_id) else {
            continue;
        };
        let PredecessorEvidenceStanding::Measurable {
            incorporation_subject,
            prior_integration_status,
        } = PredecessorEvidenceStanding::of(&predecessor_snapshot.evidence)
        else {
            continue;
        };
        let successor_heads = resolved_successor_heads(&successors, &snapshots_by_reservation);
        if successor_heads.is_empty() {
            continue;
        }
        evidence_subjects.push(PredecessorSuccessorEvidenceSubject {
            incorporation_subject,
            reservation: reservations.reservation(predecessor_id)?,
            prior_integration_status,
            successor_heads,
        });
    }
    Ok(evidence_subjects)
}

/// The distinct resolved worktree heads of one predecessor's successors, in the order the
/// successors were supplied.
fn resolved_successor_heads(
    successors: &[ReservationId],
    snapshots_by_reservation: &HashMap<ReservationId, &RepositoryReservationSnapshot>,
) -> Vec<GitObjectId> {
    let mut successor_heads = Vec::new();
    for successor in successors {
        let Some(successor_snapshot) = snapshots_by_reservation.get(successor) else {
            continue;
        };
        let WorktreeHead::Resolved(head) = &successor_snapshot.worktree_head else {
            continue;
        };
        if !successor_heads.contains(head) {
            successor_heads.push(head.clone());
        }
    }
    successor_heads
}

/// Decide every successor head reachability can settle, queueing the rest for one scoped
/// comparison apiece.
fn classify_successor_incorporation(
    repository_root: &Path,
    evidence_subjects: Vec<PredecessorSuccessorEvidenceSubject<'_>>,
) -> SuccessorIncorporationClassification {
    let subject_successor_heads = evidence_subjects
        .iter()
        .flat_map(|subject| {
            [
                ProtectedTipSuccessorHeads::new(
                    subject.incorporation_subject.commit(),
                    &subject.successor_heads,
                ),
                ProtectedTipSuccessorHeads::new(
                    subject.reservation.phase_start_head().as_ref(),
                    &subject.successor_heads,
                ),
            ]
        })
        .collect::<Vec<_>>();
    let Ok(subject_successor_classifications) =
        git::descendant_commits(repository_root, &subject_successor_heads)
    else {
        return SuccessorIncorporationClassification {
            by_predecessor:      evidence_subjects
                .into_iter()
                .map(|subject| {
                    (
                        subject.reservation.id(),
                        PredecessorSuccessorIncorporation::QueryFailed,
                    )
                })
                .collect(),
            pending_comparisons: Vec::new(),
        };
    };
    let subject_classifications = subject_successor_classifications.iter().step_by(2);
    let phase_start_classifications = subject_successor_classifications.iter().skip(1).step_by(2);
    let mut by_predecessor = Vec::new();
    let mut pending_comparisons = Vec::new();
    for ((evidence_subject, from_incorporation_subject), from_phase_start) in evidence_subjects
        .into_iter()
        .zip(subject_classifications)
        .zip(phase_start_classifications)
    {
        let predecessor_id = evidence_subject.reservation.id();
        let incorporation = predecessor_successor_incorporation(
            &evidence_subject,
            PredecessorSuccessorReachability {
                from_incorporation_subject,
                from_phase_start,
            },
            by_predecessor.len(),
            &mut pending_comparisons,
        );
        by_predecessor.push((predecessor_id, incorporation));
    }
    SuccessorIncorporationClassification {
        by_predecessor,
        pending_comparisons,
    }
}

/// Classify successor heads against the predecessor's checkpoint or integration witness.
fn predecessor_successor_incorporation(
    evidence_subject: &PredecessorSuccessorEvidenceSubject<'_>,
    reachability: PredecessorSuccessorReachability<'_>,
    predecessor_index: usize,
    pending_comparisons: &mut Vec<SuccessorScopedPatchEvaluationCandidate>,
) -> PredecessorSuccessorIncorporation {
    let ProtectedTipSuccessorHeadClassification::Classified(classified_heads) =
        reachability.from_incorporation_subject
    else {
        return PredecessorSuccessorIncorporation::PredecessorObjectUnknown;
    };
    let candidate_context = PendingScopedPatchCandidateContext {
        evidence_subject,
        predecessor_index,
        subject_revision: evidence_subject
            .reservation
            .integration_proof_subject_revision(),
        target_histories: reachability.phase_start_target_histories(),
    };
    let mut evidence_by_head = HashMap::new();
    for classified_head in classified_heads {
        let (head, evidence) = match classified_head {
            CandidateHeadReachability::Descendant { head, .. } => {
                (head, SuccessorIncorporationEvidence::ProtectedTipAncestor)
            },
            CandidateHeadReachability::ObjectUnknown(head) => {
                (head, SuccessorIncorporationEvidence::ObjectUnknown)
            },
            CandidateHeadReachability::NotDescendant(head) => (
                head,
                unreached_successor_evidence(&candidate_context, head, pending_comparisons),
            ),
        };
        evidence_by_head.insert(head.clone(), evidence);
    }
    PredecessorSuccessorIncorporation::Classified(evidence_by_head)
}

/// Decide a successor head the predecessor's incorporation subject does not reach.
///
/// A retained verdict settles it outright; otherwise it joins the queue competing for the one
/// scoped comparison this reconciliation admits.
fn unreached_successor_evidence(
    candidate_context: &PendingScopedPatchCandidateContext<'_>,
    successor_head: &GitObjectId,
    pending_comparisons: &mut Vec<SuccessorScopedPatchEvaluationCandidate>,
) -> SuccessorIncorporationEvidence {
    let evidence_subject = candidate_context.evidence_subject;
    let SuccessorIncorporationSubject::CheckpointTip(protected_tip) =
        &evidence_subject.incorporation_subject
    else {
        return SuccessorIncorporationEvidence::NotIncorporated;
    };
    if !matches!(
        evidence_subject.prior_integration_status,
        PriorIntegrationStatus::Proven
    ) {
        return SuccessorIncorporationEvidence::NotIncorporated;
    }
    match evidence_subject
        .reservation
        .retained_successor_scoped_patch_target_verdicts()
        .lookup(candidate_context.subject_revision, successor_head)
    {
        SuccessorScopedPatchTargetVerdictAvailability::Hit(
            SuccessorScopedPatchEquivalenceVerdict::Equivalent,
        ) => SuccessorIncorporationEvidence::ScopedPatchEquivalent,
        SuccessorScopedPatchTargetVerdictAvailability::Hit(
            SuccessorScopedPatchEquivalenceVerdict::Different,
        ) => SuccessorIncorporationEvidence::NotIncorporated,
        SuccessorScopedPatchTargetVerdictAvailability::Miss => {
            pending_comparisons.push(candidate_context.candidate(protected_tip, successor_head));
            SuccessorIncorporationEvidence::NotIncorporated
        },
    }
}

/// Spend the one admitted scoped comparison on the highest-priority pending target and record
/// every verdict it settles.
fn settle_pending_scoped_patch_comparisons(
    repository_root: &Path,
    classification: SuccessorIncorporationClassification,
    evaluation_budget: &mut ReconciliationSuccessorScopedPatchEvaluationBudget,
) -> SuccessorIncorporationObservation {
    let SuccessorIncorporationClassification {
        mut by_predecessor,
        mut pending_comparisons,
    } = classification;
    pending_comparisons.sort_by_key(|candidate| {
        (
            candidate.priority,
            candidate.predecessor_reservation_id.wire_ordering_key(),
            candidate.successor_head.to_string(),
        )
    });
    let mut operations = Vec::new();
    for candidate in pending_comparisons {
        let comparison = evaluation_budget.evaluate(|| {
            git::scoped_patch_equivalence_with_target_history(
                repository_root,
                &candidate.phase_start_head,
                &candidate.scopes,
                &candidate.protected_tip,
                &candidate.successor_head,
                candidate.target_history.as_git_evidence(),
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
        record_successor_scoped_patch_verdict(
            candidate,
            comparison,
            evidence_by_head,
            &mut operations,
        );
    }
    SuccessorIncorporationObservation {
        by_predecessor,
        operations,
    }
}

/// Journal one settled scoped comparison and update the head's evidence where it changed.
fn record_successor_scoped_patch_verdict(
    candidate: SuccessorScopedPatchEvaluationCandidate,
    comparison: ScopedPatchComparison,
    evidence_by_head: &mut HashMap<GitObjectId, SuccessorIncorporationEvidence>,
    operations: &mut Vec<JournalOperation>,
) {
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
                evaluator_version:          ScopedPatchEvaluatorVersion::HistoricalCandidate,
            });
        },
        ScopedPatchComparison::Different => {
            operations.push(JournalOperation::SuccessorScopedPatchEquivalenceChecked {
                predecessor_reservation_id: candidate.predecessor_reservation_id,
                subject:                    candidate.subject,
                successor_head:             candidate.successor_head,
                verdict:                    SuccessorScopedPatchEquivalenceVerdict::Different,
                evaluator_version:          ScopedPatchEvaluatorVersion::HistoricalCandidate,
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
        self.retention_commit_resolution.apply(
            &self.repository_root,
            &self.retention_repairs,
            &self.retention_deletions,
        )?;
        permit::update_branch_rewrite_markers(&self.updated_rewrite_markers)
            .map_err(LedgerError::Io)?;
        permit::delete_branch_rewrite_markers(&self.completed_rewrite_markers)
            .map_err(LedgerError::Io)?;
        for marker_context in self.marker_contexts {
            marker_context.sweep_coordination_run_marker(|worktree_id, coordination_run_id| {
                self.active_holders.iter().any(|active_holder| {
                    active_holder.worktree_id == worktree_id
                        && active_holder.coordination_run_id == coordination_run_id
                })
            })?;
        }
        let mut alerts = Vec::new();
        for reservation in reservations.iter() {
            alerts.extend(
                alert::for_lost_integration_evidence(reservation, self.repository_snapshot.trunk())
                    .map_err(ReconcileError::Replay)?,
            );
            // Released reservations no longer refresh merge extents, but their integration
            // evidence must still report when the proof for released work is lost.
            if matches!(
                reservation.lifecycle(),
                ReservationLifecycle::Released { .. }
            ) {
                continue;
            }
            // A failed observation is worth reporting only while the evidence it retained still
            // refuses something, which is what `MergeExtent::retains_protection` asks. Matching
            // the variant alone raised the alert over `RetainedMergeEvidence::Empty` too, where
            // `MergeExtent::protection` answers `Clear` -- so the message claimed retained
            // protection for a reservation that holds none. A holder whose worktree is deleted
            // stays in that state permanently, since no later observation can succeed, so the
            // alert also had no condition under which it would ever stop.
            let merge_extent = reservation.merge_extent();
            if let MergeExtent::Unavailable { failure, .. } = merge_extent
                && merge_extent.retains_protection()
            {
                alerts.push(Alert::MergeExtentUnavailable {
                    reservation_id: reservation.id(),
                    failure:        failure.clone(),
                });
            }
        }
        for alert_subject in self.alert_subjects {
            alerts.extend(alert::for_orphaned_outstanding(
                &self.repository_root,
                reservations
                    .reservation(alert_subject.reservation_id)
                    .map_err(ReconcileError::Replay)?,
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
            settlements: self.settlements,
            session_mapping_publication: SessionIdentityMappingPublication::Published,
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
                merge_extent_worktree_status_queries: self
                    .merge_extent_git_cost
                    .worktree_status_queries,
                merge_extent_path_queries: self.merge_extent_git_cost.path_queries,
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
    /// A concurrent mutation invalidated a preflight rewrite proof.
    RewriteSubjectChanged(ReservationId),
}

impl From<RewriteProjectionError> for ReconciliationPlanningError {
    fn from(error: RewriteProjectionError) -> Self {
        match error {
            RewriteProjectionError::Reservation(error) => Self::Reservation(error),
            RewriteProjectionError::RewriteSubjectChanged(id) => Self::RewriteSubjectChanged(id),
        }
    }
}

enum ReconciliationBuildError {
    Reservation(ReservationReplayError),
    WorktreeRegistry(WorktreeRegistryError),
}

impl From<ReservationReplayError> for ReconciliationBuildError {
    fn from(error: ReservationReplayError) -> Self { Self::Reservation(error) }
}

impl From<WorktreeRegistryError> for ReconciliationBuildError {
    fn from(error: WorktreeRegistryError) -> Self { Self::WorktreeRegistry(error) }
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
    ConcurrentObservationWorkerPanicked,
    UnexpectedDriftPreflightMutation,
    /// A concurrent mutation requires rewrite acceptance to be evaluated again.
    RewriteSubjectChanged(ReservationId),
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
            Self::ConcurrentObservationWorkerPanicked => OutputEnvelope::ledger_unreadable(
                command_verb,
                "concurrent drift observation worker panicked",
            ),
            Self::RewriteSubjectChanged(reservation_id) => OutputEnvelope::contention(
                command_verb,
                &format!(
                    "reservation {reservation_id} changed during rewrite acceptance; retry reconciliation"
                ),
            ),
            Self::UnexpectedDriftPreflightMutation => OutputEnvelope::ledger_unreadable(
                command_verb,
                "drift marker preflight unexpectedly appended a journal event",
            ),
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
            Self::ConcurrentObservationWorkerPanicked => {
                formatter.write_str("concurrent drift observation worker panicked")
            },
            Self::RewriteSubjectChanged(reservation_id) => write!(
                formatter,
                "reservation {reservation_id} changed during rewrite acceptance; retry reconciliation"
            ),
            Self::UnexpectedDriftPreflightMutation => {
                formatter.write_str("drift marker preflight unexpectedly mutated the journal")
            },
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
            ReconciliationPlanningError::RewriteSubjectChanged(reservation_id) => {
                Self::RewriteSubjectChanged(reservation_id)
            },
            ReconciliationPlanningError::PendingBypass(error) => {
                Self::Ledger(LedgerError::Io(error))
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use serde_json::Value;
    use serde_json::json;

    use super::HistoricalIntegrationCandidateDiscovery;
    use super::HistoricalIntegrationCandidateDiscovery as Discovery;
    use super::ReconciliationScopedPatchEvaluationBudget;
    use super::ScopedPatchComparisonDestination;
    use super::ScopedPatchEvaluationContext;
    use super::ScopedPatchEvaluationKey;
    use super::SettlementSelection;
    use crate::edge::RepositoryTrunk;
    use crate::gate::permit::PendingBranchRewriteMarker;
    use crate::git::Reachability;
    use crate::git::ScopedPatchComparison;
    use crate::git::ScopedPatchComparison as Comparison;
    use crate::ids::GitObjectId;
    use crate::ids::ReservationId;
    use crate::ledger::JournalEvent;
    use crate::output::CommandVerb;
    use crate::reservation::IntegrationEvidenceObservation;
    use crate::reservation::IntegrationEvidenceStatus;
    use crate::reservation::IntegrationProof;
    use crate::reservation::IntegrationWitness;
    use crate::reservation::MergeExtent;
    use crate::reservation::PriorIntegrationStatus;
    use crate::reservation::ReleaseDisposition;
    use crate::reservation::ReservationEvidenceState;
    use crate::reservation::RetainedReservationSet;
    use crate::reservation::RewrittenIntegrationTrunkCommit;
    use crate::reservation::ScopedPatchComparisonObservation;
    use crate::reservation::ScopedPatchIntegrationEvaluation;
    use crate::reservation::ScopedPatchIntegrationEvaluation as Evaluation;

    const RESERVATION_ID: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1f";
    const TRUNK: &str = "1111111111111111111111111111111111111111";
    const TIP: &str = "2222222222222222222222222222222222222222";

    #[test]
    fn ancestry_precedes_candidate_and_current_trunk_inside_one_admitted_evaluation()
    -> Result<(), Box<dyn std::error::Error>> {
        let target = TRUNK.parse::<GitObjectId>()?;
        let candidate = TIP.parse::<GitObjectId>()?;
        let key = || ScopedPatchEvaluationKey {
            phase_start_head: candidate.clone(),
            protected_tip:    candidate.clone(),
            target_trunk:     target.clone(),
            scopes:           Vec::new(),
            context:          ScopedPatchEvaluationContext::PriorIntegrationProven,
            destination:      ScopedPatchComparisonDestination::Trunk,
        };
        let mut budget = ReconciliationScopedPatchEvaluationBudget::default();
        let calls = RefCell::new(Vec::new());
        let observation = crate::reservation::observe_integration_status(
            {
                calls.borrow_mut().push("ancestry");
                Reachability::NotAncestor
            },
            &target,
            PriorIntegrationStatus::Unproven,
            &IntegrationEvidenceStatus::NotIntegrated,
            || {
                budget.evaluate(key(), || {
                    calls.borrow_mut().push("admitted");
                    super::evaluate_historical_then_current_trunk(
                        &target,
                        || {
                            calls.borrow_mut().push("candidate");
                            HistoricalIntegrationCandidateDiscovery::Nominated(candidate.clone())
                        },
                        |destination| {
                            calls.borrow_mut().push(if destination == &candidate {
                                "certify"
                            } else {
                                "current_trunk"
                            });
                            ScopedPatchComparison::Different
                        },
                    )
                })
            },
        );
        assert_eq!(
            *calls.borrow(),
            [
                "ancestry",
                "admitted",
                "candidate",
                "certify",
                "current_trunk"
            ]
        );
        assert!(matches!(
            observation,
            IntegrationEvidenceObservation::ScopedPatchComparison {
                status:     IntegrationEvidenceStatus::NotIntegrated,
                evaluation: ScopedPatchIntegrationEvaluation::Different,
            }
        ));
        assert_eq!(budget.evaluated_targets.len(), 1);
        let cached = budget.evaluate(key(), || {
            calls.borrow_mut().push("unexpected_duplicate");
            ScopedPatchIntegrationEvaluation::Unavailable
        });
        assert!(matches!(
            cached,
            ScopedPatchComparisonObservation::Observed(ScopedPatchIntegrationEvaluation::Different)
        ));
        let mut other = key();
        other.protected_tip = target.clone();
        let deferred = budget.evaluate(other, || {
            calls.borrow_mut().push("unexpected_second_subject");
            ScopedPatchIntegrationEvaluation::Unavailable
        });
        assert!(matches!(
            deferred,
            ScopedPatchComparisonObservation::Deferred
        ));
        assert_eq!(calls.borrow().len(), 5);
        Ok(())
    }

    #[test]
    fn historical_witness_is_cached_and_unavailable_history_keeps_negatives_retryable()
    -> Result<(), Box<dyn std::error::Error>> {
        let target = TRUNK.parse()?;
        let candidate: GitObjectId = TIP.parse()?;
        let certified = super::evaluate_historical_then_current_trunk(
            &target,
            || Discovery::Nominated(candidate.clone()),
            |destination| {
                assert_eq!(destination, &candidate);
                Comparison::Equivalent
            },
        );
        assert_eq!(
            certified,
            Evaluation::Equivalent(IntegrationWitness::Historical(candidate.into()))
        );
        let key = || super::ScopedPatchEvaluationKey {
            phase_start_head: target.clone(),
            protected_tip:    target.clone(),
            target_trunk:     target.clone(),
            scopes:           Vec::new(),
            context:          ScopedPatchEvaluationContext::PriorIntegrationProven,
            destination:      ScopedPatchComparisonDestination::Trunk,
        };
        let mut budget = super::ReconciliationScopedPatchEvaluationBudget::default();
        let first = budget.evaluate(key(), || certified.clone());
        let reused = budget.evaluate(key(), || Evaluation::Unavailable);
        for observation in [first, reused] {
            let super::ScopedPatchComparisonObservation::Observed(evaluation) = observation else {
                return Err("identical subject should reuse admitted result".into());
            };
            assert_eq!(evaluation, certified);
        }
        let retryable = super::evaluate_historical_then_current_trunk(
            &target,
            || Discovery::Unavailable,
            |_| Comparison::Different,
        );
        assert_eq!(retryable, Evaluation::HistoricalEvidenceUnavailable);
        Ok(())
    }

    #[test]
    fn settlement_selection() -> Result<(), Box<dyn std::error::Error>> {
        let reservation_id = RESERVATION_ID.parse::<ReservationId>()?;
        let [claim, checkpoint] = checkpoint_events()?;
        let actual_trunk = RepositoryTrunk::Resolved(TRUNK.parse()?);
        let other_trunk = RepositoryTrunk::Resolved(TIP.parse()?);
        let extent = later_work_extent()?;
        for proof in [
            IntegrationProof::ProtectedTipAncestor,
            IntegrationProof::ScopedPatchEquivalent,
        ] {
            let status = IntegrationEvidenceStatus::Integrated {
                trunk_oid: TRUNK.parse()?,
                proof,
                witness: IntegrationWitness::EvaluatedTrunk,
            };
            let integrated = integrated_evidence_event(&status)?;
            let events = [claim.clone(), checkpoint.clone(), integrated.clone()];
            let retained = RetainedReservationSet::replay(&events)?;
            let reservation = retained.reservation(reservation_id)?;
            let ReservationEvidenceState::Outstanding {
                integration_status, ..
            } = reservation.evidence_state()?
            else {
                return Err("the seeded proof must remain outstanding until settlement".into());
            };
            let disposition = match proof {
                IntegrationProof::ProtectedTipAncestor => ReleaseDisposition::Integrated,
                IntegrationProof::ScopedPatchEquivalent
                | IntegrationProof::RewrittenWitnessAncestor => {
                    ReleaseDisposition::RewrittenIntegration(RewrittenIntegrationTrunkCommit::from(
                        TRUNK.parse::<GitObjectId>()?,
                    ))
                },
            };
            assert_eq!(
                super::settlement_selection(
                    reservation,
                    &integration_status,
                    &actual_trunk,
                    reservation.merge_extent(),
                    &super::CommittedMergeEvidence::Unavailable,
                ),
                SettlementSelection::Release(disposition.clone())
            );
            let unavailable_extent = extent.unavailable("holder missing".to_owned());
            for (status, trunk, extent) in [
                (
                    &integration_status,
                    &other_trunk,
                    reservation.merge_extent(),
                ),
                (&integration_status, &actual_trunk, &extent),
                (&integration_status, &actual_trunk, &unavailable_extent),
                (
                    &IntegrationEvidenceStatus::NotIntegrated,
                    &actual_trunk,
                    reservation.merge_extent(),
                ),
            ] {
                assert_eq!(
                    super::settlement_selection(
                        reservation,
                        status,
                        trunk,
                        extent,
                        &super::CommittedMergeEvidence::Unavailable,
                    ),
                    SettlementSelection::Unchanged
                );
            }
            let release = journal_event(
                4,
                &json!({
                    "op": "release", "reservation_id": RESERVATION_ID, "disposition": disposition,
                }),
            )?;
            let released = RetainedReservationSet::replay(&[
                claim.clone(),
                checkpoint.clone(),
                integrated,
                release,
            ])?;
            let reservation = released.reservation(reservation_id)?;
            assert_eq!(
                super::settlement_selection(
                    reservation,
                    &integration_status,
                    &actual_trunk,
                    reservation.merge_extent(),
                    &super::CommittedMergeEvidence::Unavailable,
                ),
                SettlementSelection::Unchanged
            );
            assert!(
                matches!(reservation.evidence_state()?, ReservationEvidenceState::Released {
                integration_status: retained_status, ..
            } if retained_status == status)
            );
        }
        Ok(())
    }

    #[test]
    fn settlement_preserves_historical_witness_only_for_actual_trunk()
    -> Result<(), Box<dyn std::error::Error>> {
        let retained = RetainedReservationSet::replay(&checkpoint_events()?)?;
        let reservation = retained.reservation(RESERVATION_ID.parse()?)?;
        let witness = RewrittenIntegrationTrunkCommit::from(
            "3333333333333333333333333333333333333333".parse::<GitObjectId>()?,
        );
        let evidence = IntegrationEvidenceStatus::Integrated {
            trunk_oid: TRUNK.parse()?,
            proof:     IntegrationProof::ScopedPatchEquivalent,
            witness:   IntegrationWitness::Historical(witness.clone()),
        };
        for (trunk, expected) in [
            (
                TRUNK,
                SettlementSelection::Release(ReleaseDisposition::RewrittenIntegration(witness)),
            ),
            (TIP, SettlementSelection::Unchanged),
        ] {
            assert_eq!(
                super::settlement_selection(
                    reservation,
                    &evidence,
                    &RepositoryTrunk::Resolved(trunk.parse()?),
                    reservation.merge_extent(),
                    &super::CommittedMergeEvidence::Unavailable,
                ),
                expected,
            );
        }
        Ok(())
    }

    #[test]
    fn rewrite_subject_races_retry_but_other_failures_do_not()
    -> Result<(), Box<dyn std::error::Error>> {
        let id = RESERVATION_ID.parse::<ReservationId>()?;
        let changed = |error: &super::ReconcileError| {
            matches!(error, super::ReconcileError::RewriteSubjectChanged(_))
        };
        let mut attempts = 0;
        super::retry_rewrite_reconciliation(
            || {
                attempts += 1;
                if attempts < 3 {
                    Err(super::ReconcileError::RewriteSubjectChanged(id))
                } else {
                    Ok(())
                }
            },
            changed,
        )?;
        assert_eq!(attempts, 3);
        attempts = 0;
        let result = super::retry_rewrite_reconciliation::<(), _>(
            || {
                attempts += 1;
                Err(super::ReconcileError::UnexpectedDriftPreflightMutation)
            },
            changed,
        );
        assert_eq!(attempts, 1);
        assert!(matches!(
            result,
            Err(super::ReconcileError::UnexpectedDriftPreflightMutation)
        ));
        Ok(())
    }

    #[test]
    fn persistent_rewrite_subject_races_report_contention() -> Result<(), Box<dyn std::error::Error>>
    {
        let id = RESERVATION_ID.parse::<ReservationId>()?;
        let mut attempts = 0;
        let result = super::retry_rewrite_reconciliation::<(), _>(
            || {
                attempts += 1;
                Err(super::ReconcileError::RewriteSubjectChanged(id))
            },
            |error| matches!(error, super::ReconcileError::RewriteSubjectChanged(_)),
        );
        assert_eq!(attempts, 3);
        let Err(error) = result else {
            return Err("persistent subject changes must remain retryable errors".into());
        };
        let output = serde_json::to_value(error.into_output(CommandVerb::Board))?;
        assert_eq!(output["status"], "contention");
        assert!(
            output["message"]
                .as_str()
                .is_some_and(|message| message.contains("retry"))
        );
        Ok(())
    }

    #[test]
    fn deferred_rewrite_entry_follows_later_markers() -> Result<(), Box<dyn std::error::Error>> {
        let outstanding = RetainedReservationSet::replay(&checkpoint_events()?)?;
        let id = RESERVATION_ID.parse::<ReservationId>()?;
        let first = "3333333333333333333333333333333333333333";
        let second = "4444444444444444444444444444444444444444";
        let third = "5555555555555555555555555555555555555555";
        let branch_tip = "6666666666666666666666666666666666666666";
        let mut subject = super::DeferredRewriteIntegrationSubject {
            validation:     super::RewriteSubjectValidation::capture(outstanding.reservation(id)?),
            rewritten_tips: vec![first.parse()?],
        };
        for (old, new, new_tips) in [
            (TRUNK, TIP, Vec::new()),
            (first, second, Vec::new()),
            (second, third, vec![branch_tip]),
        ] {
            let marker = PendingBranchRewriteMarker {
                path:    std::path::PathBuf::from("pending-rewrite.json"),
                rewrite: serde_json::from_value(json!({
                    "kind": "branch_rewrite",
                    "pairs": [{"old": old, "new": new}],
                    "new_tips": new_tips,
                    "created_commits": [new],
                    "worktree_administrative_directory": ".git",
                }))?,
            };
            subject.follow_rewrite(&marker);
        }
        assert_eq!(
            subject.rewritten_tips,
            vec![
                first.parse()?,
                second.parse()?,
                third.parse()?,
                branch_tip.parse()?,
            ]
        );
        assert!(subject.validation.accepts(&outstanding));
        Ok(())
    }

    #[test]
    fn deferred_rewrite_entry_ignores_an_independent_branch_marker()
    -> Result<(), Box<dyn std::error::Error>> {
        let outstanding = RetainedReservationSet::replay(&checkpoint_events()?)?;
        let id = RESERVATION_ID.parse::<ReservationId>()?;
        let deferred_tip = "3333333333333333333333333333333333333333".parse()?;
        let independent_old = "4444444444444444444444444444444444444444";
        let independent_new = "5555555555555555555555555555555555555555";
        let independent_tip = "6666666666666666666666666666666666666666";
        let original_tips = vec![deferred_tip];
        let mut subject = super::DeferredRewriteIntegrationSubject {
            validation:     super::RewriteSubjectValidation::capture(outstanding.reservation(id)?),
            rewritten_tips: original_tips.clone(),
        };
        let marker = PendingBranchRewriteMarker {
            path:    std::path::PathBuf::from("independent-branch-rewrite.json"),
            rewrite: serde_json::from_value(json!({
                "kind": "branch_rewrite",
                "pairs": [{"old": independent_old, "new": independent_new}],
                "new_tips": [independent_tip],
                "created_commits": [independent_new, independent_tip],
                "worktree_administrative_directory": ".git",
            }))?,
        };
        subject.follow_rewrite(&marker);
        assert_eq!(subject.rewritten_tips, original_tips);
        assert!(subject.validation.accepts(&outstanding));
        Ok(())
    }

    #[test]
    fn deferred_rewrite_subject_validation() -> Result<(), Box<dyn std::error::Error>> {
        let [claim, checkpoint] = checkpoint_events()?;
        let id = RESERVATION_ID.parse::<ReservationId>()?;
        let outstanding = RetainedReservationSet::replay(&[claim.clone(), checkpoint.clone()])?;
        let validation = super::RewriteSubjectValidation::capture(outstanding.reservation(id)?);
        let preflight = super::RewriteReconciliationPreflight {
            deferred_subjects: vec![super::DeferredRewriteIntegrationSubject {
                validation,
                rewritten_tips: vec![TRUNK.parse()?],
            }],
            ..super::RewriteReconciliationPreflight::default()
        };
        assert!(preflight.project(&outstanding).is_ok());
        for operation in [
            json!({
                "op": "resnapshot", "reservation_id": RESERVATION_ID,
                "snapshot": {"stage": "outstanding", "protected_tip": TIP, "trunk_oid": TRUNK,
                    "phase_start_head": "3333333333333333333333333333333333333333"}
            }),
            json!({
                "op": "release", "reservation_id": RESERVATION_ID,
                "disposition": {"kind": "abandoned", "evidence": "concurrent phase change"}
            }),
        ] {
            let changed = RetainedReservationSet::replay(&[
                claim.clone(),
                checkpoint.clone(),
                journal_event(3, &operation)?,
            ])?;
            assert!(matches!(
                preflight.project(&changed),
                Err(super::RewriteProjectionError::RewriteSubjectChanged(changed)) if changed == id
            ));
        }
        Ok(())
    }

    #[test]
    fn resnapshot_validation() -> Result<(), Box<dyn std::error::Error>> {
        let [claim, checkpoint] = checkpoint_events()?;
        let id = RESERVATION_ID.parse::<ReservationId>()?;
        let outstanding = RetainedReservationSet::replay(&[claim.clone(), checkpoint.clone()])?;
        let validation = super::RewriteSubjectValidation::capture(outstanding.reservation(id)?);
        assert!(validation.accepts(&outstanding));

        let resnapshot = journal_event(
            3,
            &json!({
                "op": "resnapshot", "reservation_id": RESERVATION_ID,
                "snapshot": {"stage": "outstanding", "protected_tip": TIP, "trunk_oid": TRUNK,
                    "phase_start_head": "3333333333333333333333333333333333333333"}
            }),
        )?;
        let advanced = RetainedReservationSet::replay(&[
            claim.clone(),
            checkpoint.clone(),
            resnapshot.clone(),
        ])?;
        assert!(!validation.accepts(&advanced));
        let preflight = super::RewriteReconciliationPreflight {
            validations: vec![validation],
            operations: vec![resnapshot.operation],
            ..super::RewriteReconciliationPreflight::default()
        };
        assert!(preflight.project(&outstanding).is_ok());
        assert!(
            matches!(preflight.project(&advanced), Err(super::RewriteProjectionError::RewriteSubjectChanged(changed)) if changed == id)
        );

        let released = RetainedReservationSet::replay(&[
            claim,
            checkpoint,
            journal_event(
                3,
                &json!({"op": "release", "reservation_id": RESERVATION_ID,
                "disposition": {"kind": "abandoned", "evidence": "discarded during concurrent decision"}}),
            )?,
        ])?;
        assert!(!preflight.validations[0].accepts(&released));
        assert!(
            matches!(preflight.project(&released), Err(super::RewriteProjectionError::RewriteSubjectChanged(changed)) if changed == id)
        );
        Ok(())
    }

    /// A durable proof that can settle without another evidence change.
    fn integrated_evidence_event(
        status: &IntegrationEvidenceStatus,
    ) -> Result<JournalEvent, serde_json::Error> {
        journal_event(
            3,
            &json!({
                "op": "evidence_revalidated", "reservation_id": RESERVATION_ID,
                "status": status, "edit_blocking_status": "clear",
            }),
        )
    }

    /// Retained branch work beyond the checkpoint continues to block settlement.
    fn later_work_extent() -> Result<MergeExtent, serde_json::Error> {
        serde_json::from_value(json!({
            "status": "protected",
            "key": {"trunk": TRUNK, "head": "3333333333333333333333333333333333333333",
                "working_tree": {"tracked_paths": [], "untracked_paths": []}},
            "scopes": [{"path": "src/later.rs", "kind": "file"}],
        }))
    }

    fn checkpoint_events() -> Result<[JournalEvent; 2], serde_json::Error> {
        let claim = journal_event(
            1,
            &json!({
                "op": "claim", "reservation_id": RESERVATION_ID,
                "scopes": [{"path": "src", "kind": "tree"}],
                "source": {"kind": "explicit"}, "purpose": {"kind": "not_provided_by_caller"},
                "trunk_at_claim": TRUNK,
                "head_snapshot": {"kind": "branch", "full_ref": "refs/heads/phase", "head": TIP},
                "phase_start_head": TRUNK, "worktree_root": "/repo",
                "worktree_administrative_locator": ".", "authorization": {"kind": "no_conflict"},
                "coordination_identity_provenance": "presented",
            }),
        )?;
        let checkpoint = journal_event(
            2,
            &json!({
                "op": "checkpoint", "reservation_id": RESERVATION_ID,
                "protected_tip": TIP, "trunk_snapshot": TRUNK,
            }),
        )?;
        Ok([claim, checkpoint])
    }

    fn journal_event(
        generation: u64,
        operation: &Value,
    ) -> Result<JournalEvent, serde_json::Error> {
        let mut event = json!({
            "schema_version": 2, "event_id": "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1b",
            "actor": {
                "repository": "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1c",
                "worktree": "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1d",
                "run": "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1e"
            },
            "at": "2026-08-23T17:34:54.123Z", "projection_generation": generation,
        });
        if let (Some(event), Some(operation)) = (event.as_object_mut(), operation.as_object()) {
            event.extend(operation.clone());
        }
        serde_json::from_value(event)
    }
}
