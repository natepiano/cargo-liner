//! Shared liveness, evidence, retention-ref, and marker reconciliation.

use std::collections::BTreeMap;
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
use crate::alert::OrphanIntegrationEvidence;
use crate::config::BerthConfig;
use crate::config::ConfigError;
use crate::config::Enrollment;
use crate::constants::MERGE_EXTENT_TRUNK_UNAVAILABLE;
use crate::constants::MERGE_EXTENT_WORKTREE_UNAVAILABLE;
use crate::constants::UNMERGED_BRANCH_PATH_GIT_QUERIES;
use crate::drift;
use crate::edge::CrossTargetPredecessorEvidence;
use crate::edge::CrossTargetPredecessorReachability;
use crate::edge::EdgeReplayError;
use crate::edge::IntegrationConstraintProjection;
use crate::edge::JudgedTargetTip;
use crate::edge::MissingReadinessFact;
use crate::edge::OrderingGraph;
use crate::edge::PredecessorSuccessorIncorporation;
use crate::edge::RepositoryReservationEvidence;
use crate::edge::RepositoryReservationSnapshot;
use crate::edge::RepositorySnapshot;
use crate::edge::SuccessorIncorporationEvidence;
use crate::edge::TargetObservation;
use crate::gate;
use crate::gate::ProposedTargetMove;
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
use crate::ledger::ClaimHeadSnapshot;
use crate::ledger::ClaimSource;
use crate::ledger::CoverClaimActor;
use crate::ledger::ExistingCoordinationRun;
use crate::ledger::IntegrationTarget;
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
use crate::ledger::ReservationScope;
use crate::ledger::ReservationScopeSet;
use crate::ledger::ReservationSnapshot;
use crate::ledger::ScopeKind;
use crate::ledger::TargetSelectionRequest;
use crate::ledger::TransactionValidation;
use crate::ledger::TrunkObservationAtClaim;
use crate::ledger::WorktreeContext;
use crate::output::CommandVerb;
use crate::output::OutputEnvelope;
use crate::reservation;
use crate::reservation::DurableScopedPatchComparison;
use crate::reservation::EditBlockingStatus;
use crate::reservation::IntegrationEvidenceObservation;
use crate::reservation::IntegrationEvidenceStatus;
use crate::reservation::IntegrationProof;
use crate::reservation::IntegrationProofSubjectRevision;
use crate::reservation::IntegrationWitness;
use crate::reservation::MergeExtent;
use crate::reservation::MergeExtentKey;
use crate::reservation::OrphanRetirementReason;
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
use crate::scope::PathCase;
use crate::session::SessionIdentityMappingPublication;
use crate::worktree;
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
    /// Present integration branches whose checkout carried a fresh cover extent this pass.
    covered_targets:                          HashSet<IntegrationTarget>,
    cover_creation:                           CoverCreation,
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
    pub(crate) fn cover_is_missing(&self, reservation_id: ReservationId) -> bool {
        cover_is_missing(
            &self.repository_snapshot,
            &self.covered_targets,
            reservation_id,
        )
    }

    /// Return the observation at which this reservation was judged.
    pub(crate) fn target_for(&self, reservation_id: ReservationId) -> &JudgedTargetTip {
        self.repository_snapshot.target_for(reservation_id)
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
    /// Net branch path queries, including separation of committed paths for settlement.
    pub(crate) merge_extent_path_queries:            u64,
}

/// Attempted Git queries made while deriving all holder merge extents under one lock.
#[derive(Default)]
struct MergeExtentGitCost {
    /// Includes attempted status reads that report an observation failure.
    worktree_status_queries: u64,
    /// Counts both invocations of each `git::unmerged_branch_paths` read, including failed
    /// reads and fresh committed-path reads needed to settle a cached extent, plus the ancestry
    /// read that ends a clean run whose head trunk contains.
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
    operations:   Vec<JournalOperation>,
    cover_actors: HashMap<ReservationId, CoverClaimActor>,
    action:       ReconciliationAction,
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
    repository_trunk:               &'context JudgedTargetTip,
    integration_reachability:       &'context TargetIntegrationReachability,
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
    repository_trunk:                 IntegrationTarget,
    targets:                          BTreeMap<IntegrationTarget, TargetObservation>,
    reservation_targets:              HashMap<ReservationId, IntegrationTarget>,
    resolved_candidates:              ResolvedBatchCommitCandidates,
    repository_evidence_observations: Vec<RepositoryEvidenceObservation>,
    trunk_reachability:               TargetIntegrationReachability,
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

impl IntegrationStatusObservation {
    const fn new(
        status: IntegrationEvidenceStatus,
        revalidation: EvidenceRevalidationObservation,
        scoped_patch_comparison: ScopedPatchComparisonJournalUpdate,
    ) -> Self {
        Self {
            status,
            revalidation,
            scoped_patch_comparison,
        }
    }
}

/// What an affirmative proof already on file settles about the trunk this pass observed.
///
/// Every `IntegrationProof` is a statement about the one trunk commit its `trunk_oid` names:
/// `ProtectedTipAncestor` and `RewrittenWitnessAncestor` place a commit in that trunk's history,
/// and `ScopedPatchEquivalent` matched the reservation's scoped content against it. A trunk that
/// still contains that commit has only added history since, so each of those statements holds in
/// every descendant, and a proof is re-derived only when the commit that carried it is gone --
/// a reset or a rebase. Deriving one afresh each pass could only ever disagree with what the
/// ledger already records, which is what turned a rebased tip into a permanent lost-evidence
/// directive.
enum IntegrationProofStanding {
    /// The trunk that proved integration is still in the observed trunk's history.
    Holds,
    /// No proof covers the observed trunk, so evidence is derived against `previous_trunk`.
    Derive { previous_trunk: GitObjectId },
    /// Git could not classify the trunk commit the proof names.
    ProvingTrunkUnknown,
}

/// Carry a proof that still stands forward onto the trunk this pass observed.
///
/// `trunk_oid` names the trunk a proof was measured against, and `settlement_selection` releases an
/// outstanding reservation only when that commit is the actual trunk -- so a proof left anchored to
/// where it was first taken would keep its reservation open forever. Re-anchoring states what
/// [`IntegrationProofStanding::Holds`] established: trunk has only added commits since, so the same
/// `proof` and `witness` hold here too. For `IntegrationProof::ScopedPatchEquivalent` that also
/// settles what integration means -- the work reached trunk, and a later commit reverting it does
/// not un-reach it, exactly as an ancestry proof already treats the same revert.
///
/// A tip the observed trunk now contains outranks whatever was recorded, so the proof is stated as
/// `IntegrationProof::ProtectedTipAncestor` instead. That is not a re-derivation: the tip is one of
/// the commits `TargetIntegrationReachability` already classified, and the stronger proof keeps
/// `settlement_selection` releasing as `ReleaseDisposition::Integrated` rather than retaining a
/// rewritten-integration witness the repository no longer needs.
fn reanchored_proof(
    materialized: IntegrationEvidenceStatus,
    observed_trunk: &GitObjectId,
    protected_tip_reachability: Reachability,
) -> IntegrationEvidenceStatus {
    let IntegrationEvidenceStatus::Integrated { proof, witness, .. } = materialized else {
        return materialized;
    };
    let (proof, witness) = match protected_tip_reachability {
        Reachability::Ancestor => (
            IntegrationProof::ProtectedTipAncestor,
            IntegrationWitness::EvaluatedTrunk,
        ),
        Reachability::NotAncestor | Reachability::ObjectUnknown => (proof, witness),
    };
    IntegrationEvidenceStatus::Integrated {
        trunk_oid: observed_trunk.clone(),
        proof,
        witness,
    }
}

/// The trunk commit an affirmative proof names, whose ancestry decides whether the proof stands.
const fn proving_trunk(materialized: &IntegrationEvidenceStatus) -> Option<&GitObjectId> {
    match materialized {
        IntegrationEvidenceStatus::Integrated { trunk_oid, .. } => Some(trunk_oid),
        IntegrationEvidenceStatus::NotIntegrated
        | IntegrationEvidenceStatus::TrunkRewritten
        | IntegrationEvidenceStatus::ObjectUnknown => None,
    }
}

impl IntegrationProofStanding {
    fn observe(
        materialized: &IntegrationEvidenceStatus,
        trunk_snapshot: &GitObjectId,
        integration_reachability: &TargetIntegrationReachability,
    ) -> Self {
        let Some(trunk_oid) = proving_trunk(materialized) else {
            return Self::Derive {
                previous_trunk: trunk_snapshot.clone(),
            };
        };
        match integration_reachability.for_ancestor(trunk_oid) {
            Reachability::Ancestor => Self::Holds,
            // The proving trunk left history, so the work has to be located again, and that same
            // commit is the baseline the comparison measures from: trunk moved away from where
            // this reservation was last proven, not from wherever it stood at checkpoint.
            Reachability::NotAncestor => Self::Derive {
                previous_trunk: trunk_oid.clone(),
            },
            Reachability::ObjectUnknown => Self::ProvingTrunkUnknown,
        }
    }
}

/// Every integration-proof ancestor classified against one immutable trunk target.
#[derive(Default)]
struct TargetIntegrationReachability {
    by_ancestor:         HashMap<GitObjectId, Reachability>,
    resolved_candidates: ResolvedBatchCommitCandidates,
    target_histories:    PhaseStartTargetFirstParentHistories,
}

/// Edge ancestors added only when the batch judges the repository trunk.
#[derive(Clone, Copy)]
enum TrunkEdgeCandidates<'candidates> {
    Excluded,
    Included {
        scope:       RepositoryObservationScope,
        target_tips: &'candidates [GitObjectId],
    },
}

impl<'candidates> TrunkEdgeCandidates<'candidates> {
    const fn including(
        scope: RepositoryObservationScope,
        target_tips: &'candidates [GitObjectId],
    ) -> Self {
        Self::Included { scope, target_tips }
    }
}

impl TargetIntegrationReachability {
    fn observe_configured_trunk(
        repository_root: &Path,
        reservations: &RetainedReservationSet,
        ordering_graph: &OrderingGraph,
        trunk_branch: &str,
        selected_reservations: &HashSet<ReservationId>,
        edge_candidates: TrunkEdgeCandidates<'_>,
    ) -> Result<(TargetObservation, Self), ReservationReplayError> {
        let candidate_ancestors = Self::candidate_ancestors(
            reservations,
            ordering_graph,
            selected_reservations,
            edge_candidates,
        )?;
        let observation =
            git::branch_commit_reachability(repository_root, trunk_branch, &candidate_ancestors);
        let Ok(observation) = observation else {
            return Ok((
                TargetObservation::ObjectUnknown,
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
        let (target, candidates) = match reachability {
            CommitTargetReachability::Resolved { target, candidates } => (target, candidates),
            CommitTargetReachability::Missing => {
                return Ok((
                    TargetObservation::Missing,
                    Self {
                        by_ancestor: HashMap::new(),
                        resolved_candidates,
                        target_histories,
                    },
                ));
            },
            CommitTargetReachability::Ambiguous | CommitTargetReachability::WrongType { .. } => {
                return Ok((
                    TargetObservation::ObjectUnknown,
                    Self {
                        by_ancestor: HashMap::new(),
                        resolved_candidates,
                        target_histories,
                    },
                ));
            },
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
            TargetObservation::Resolved(target),
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
        repository_trunk: &JudgedTargetTip,
        selected_reservations: &HashSet<ReservationId>,
        edge_candidates: TrunkEdgeCandidates<'_>,
    ) -> Result<Self, ReservationReplayError> {
        let JudgedTargetTip::Resolved(target) = repository_trunk else {
            return Ok(Self {
                by_ancestor:         HashMap::new(),
                resolved_candidates: git::ResolvedBatchCommitCandidates::default(),
                target_histories:    git::PhaseStartTargetFirstParentHistories::default(),
            });
        };
        let candidate_ancestors = Self::candidate_ancestors(
            reservations,
            ordering_graph,
            selected_reservations,
            edge_candidates,
        )?;
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
        selected_reservations: &HashSet<ReservationId>,
        edge_candidates: TrunkEdgeCandidates<'_>,
    ) -> Result<Vec<GitObjectId>, ReservationReplayError> {
        let mut candidate_ancestors = HashSet::new();
        for reservation in reservations
            .iter()
            .filter(|reservation| selected_reservations.contains(&reservation.id()))
        {
            match reservation.evidence_state()? {
                ReservationEvidenceState::Outstanding {
                    protected_tip,
                    trunk_snapshot,
                    integration_status,
                } => {
                    candidate_ancestors.insert(reservation.phase_start_head().as_ref().clone());
                    candidate_ancestors.insert(protected_tip.as_ref().clone());
                    candidate_ancestors.insert(trunk_snapshot);
                    candidate_ancestors.extend(proving_trunk(&integration_status).cloned());
                },
                ReservationEvidenceState::Released {
                    protected_tip,
                    disposition,
                    integration_status,
                    ..
                } => {
                    candidate_ancestors.extend(proving_trunk(&integration_status).cloned());
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
        if let TrunkEdgeCandidates::Included { scope, target_tips } = edge_candidates {
            candidate_ancestors.extend(target_tips.iter().cloned());
            for predecessor_id in observed_predecessors(ordering_graph, scope) {
                let predecessor = reservations.reservation(predecessor_id)?;
                candidate_ancestors.extend(predecessor.target_proof_commits().iter().cloned());
                match predecessor.evidence_state()? {
                    ReservationEvidenceState::Outstanding {
                        protected_tip,
                        integration_status,
                        ..
                    }
                    | ReservationEvidenceState::Released {
                        protected_tip,
                        integration_status,
                        ..
                    } => {
                        candidate_ancestors.insert(protected_tip.as_ref().clone());
                        candidate_ancestors.extend(proving_trunk(&integration_status).cloned());
                    },
                    ReservationEvidenceState::Active { .. }
                    | ReservationEvidenceState::ReleasedWithoutCheckpoint { .. } => {},
                }
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

    /// Classify commits introduced by this reconciliation after the first trunk batch.
    fn extend_for_snapshot(
        &mut self,
        repository_root: &Path,
        trunk: &JudgedTargetTip,
        snapshots: &[RepositoryReservationSnapshot],
        reservations: &RetainedReservationSet,
        ordering_graph: &OrderingGraph,
        scope: RepositoryObservationScope,
    ) -> Result<(), ReservationReplayError> {
        let mut additional = Vec::new();
        for predecessor_id in observed_predecessors(ordering_graph, scope) {
            let Some(snapshot) = snapshots
                .iter()
                .find(|snapshot| snapshot.reservation_id == predecessor_id)
            else {
                continue;
            };
            let protected_tip = match &snapshot.evidence {
                RepositoryReservationEvidence::Outstanding { protected_tip, .. }
                | RepositoryReservationEvidence::Released { protected_tip, .. } => protected_tip,
                RepositoryReservationEvidence::Active
                | RepositoryReservationEvidence::ReleasedWithoutCheckpoint { .. } => continue,
            };
            additional.push(protected_tip.as_ref().clone());
            additional.extend(
                reservations
                    .reservation(predecessor_id)?
                    .target_proof_commits()
                    .iter()
                    .cloned(),
            );
        }
        let mut seen = HashSet::new();
        additional
            .retain(|commit| !self.by_ancestor.contains_key(commit) && seen.insert(commit.clone()));
        if additional.is_empty() {
            return Ok(());
        }
        let reachability = match trunk {
            JudgedTargetTip::Resolved(target) => {
                git::reachability_to_target(repository_root, &additional, target)
                    .unwrap_or_else(|_| vec![Reachability::ObjectUnknown; additional.len()])
            },
            JudgedTargetTip::ObjectUnknown => vec![Reachability::ObjectUnknown; additional.len()],
        };
        self.by_ancestor
            .extend(additional.into_iter().zip(reachability));
        Ok(())
    }

    fn target_history_after_phase_start(
        &self,
        phase_start: &GitObjectId,
    ) -> ScopedPatchTargetHistory<'_> {
        self.target_histories.after_phase_start(phase_start)
    }
}

/// Classify graph predecessors at the repository trunk using the trunk's existing batch.
fn cross_target_predecessor_evidence(
    ordering_graph: &OrderingGraph,
    repository_observation_scope: RepositoryObservationScope,
    reservations: &RetainedReservationSet,
    snapshots: &[RepositoryReservationSnapshot],
    trunk: &JudgedTargetTip,
    reachability: &TargetIntegrationReachability,
) -> CrossTargetPredecessorEvidence {
    let by_reservation = snapshots
        .iter()
        .map(|snapshot| (snapshot.reservation_id, &snapshot.evidence))
        .collect::<HashMap<_, _>>();
    let evidence = observed_predecessors(ordering_graph, repository_observation_scope)
        .into_iter()
        .filter_map(|predecessor_id| {
            let evidence = by_reservation.get(&predecessor_id)?;
            let (protected_tip, integration_status) = match evidence {
                RepositoryReservationEvidence::Outstanding {
                    protected_tip,
                    integration_status,
                }
                | RepositoryReservationEvidence::Released {
                    protected_tip,
                    integration_status,
                    ..
                } => (protected_tip, integration_status),
                RepositoryReservationEvidence::Active
                | RepositoryReservationEvidence::ReleasedWithoutCheckpoint { .. } => return None,
            };
            let outcome = match trunk {
                JudgedTargetTip::Resolved(trunk_oid) => {
                    let tip = reachability.for_ancestor(protected_tip.as_ref());
                    let proof_commits = reservations
                        .reservation(predecessor_id)
                        .map(crate::reservation::Reservation::target_proof_commits)
                        .unwrap_or_default();
                    let proofs = proof_commits
                        .iter()
                        .map(|commit| reachability.for_ancestor(commit))
                        .chain(
                            proving_trunk(integration_status)
                                .map(|commit| reachability.for_ancestor(commit)),
                        );
                    let proof_reachability = proofs.collect::<Vec<_>>();
                    if tip == Reachability::Ancestor
                        || proof_reachability.contains(&Reachability::Ancestor)
                    {
                        CrossTargetPredecessorReachability::OnTrunk(trunk_oid.clone())
                    } else if tip == Reachability::ObjectUnknown
                        || proof_reachability.contains(&Reachability::ObjectUnknown)
                    {
                        CrossTargetPredecessorReachability::ObjectUnknown
                    } else {
                        CrossTargetPredecessorReachability::NotOnTrunk
                    }
                },
                JudgedTargetTip::ObjectUnknown => CrossTargetPredecessorReachability::ObjectUnknown,
            };
            Some((predecessor_id, outcome))
        })
        .collect();
    CrossTargetPredecessorEvidence::new(evidence)
}

/// Predecessors from the durable graph and a sequence request not yet appended to it.
fn observed_predecessors(
    ordering_graph: &OrderingGraph,
    repository_observation_scope: RepositoryObservationScope,
) -> HashSet<ReservationId> {
    let mut predecessors = ordering_graph
        .predecessors()
        .map(|predecessor| predecessor.reservation_id)
        .collect::<HashSet<_>>();
    if let RepositoryObservationScope::RequestedOrderingEdge { before, .. } =
        repository_observation_scope
    {
        predecessors.insert(before);
    }
    predecessors
}

/// Current tips that can become newly materialized integration proofs this pass.
fn observed_predecessor_target_tips(
    ordering_graph: &OrderingGraph,
    scope: RepositoryObservationScope,
    repository_trunk: &IntegrationTarget,
    reservation_targets: &HashMap<ReservationId, IntegrationTarget>,
    targets: &BTreeMap<IntegrationTarget, TargetObservation>,
) -> Vec<GitObjectId> {
    observed_predecessors(ordering_graph, scope)
        .into_iter()
        .filter_map(|id| reservation_targets.get(&id))
        .filter(|target| *target != repository_trunk)
        .filter_map(|target| targets.get(target))
        .filter_map(|observation| match observation {
            TargetObservation::Resolved(commit) => Some(commit.clone()),
            TargetObservation::Missing | TargetObservation::ObjectUnknown => None,
        })
        .collect()
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

struct ProposedTargetObservation {
    snapshot:            RepositorySnapshot,
    operations:          Vec<JournalOperation>,
    judged_reservations: HashSet<ReservationId>,
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

/// Reuses identical proof inputs and admits one scoped comparison per trunk target.
///
/// The throttle bounds git cost for reservations no proof covers, where deferring costs nothing:
/// their materialized status already says the work is not in trunk, so a pass that skips the
/// comparison records and reports exactly what the previous one did. A reservation whose proof
/// still stands never reaches this budget at all -- [`IntegrationProofStanding::Holds`] answers
/// it from the batched ancestry query -- so a deferral can no longer unseat a proof.
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

/// What the predecessor's own integration evidence contributes to a successor's incorporation.
enum PredecessorIntegrationProof {
    /// The predecessor's work reached trunk at this commit.
    ReachedTrunk(GitObjectId),
    /// Nothing on file says the work reached trunk, so no successor can inherit it from there.
    Unproven,
}

impl PredecessorIntegrationProof {
    const fn reached_trunk(&self) -> Option<&GitObjectId> {
        match self {
            Self::ReachedTrunk(trunk_oid) => Some(trunk_oid),
            Self::Unproven => None,
        }
    }
}

struct PredecessorSuccessorEvidenceSubject<'reservation> {
    incorporation_subject: SuccessorIncorporationSubject,
    reservation:           &'reservation Reservation,
    integration_proof:     PredecessorIntegrationProof,
    successor_heads:       Vec<GitObjectId>,
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
        incorporation_subject: SuccessorIncorporationSubject,
        integration_proof:     PredecessorIntegrationProof,
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
            integration_proof: match integration_status {
                IntegrationEvidenceStatus::Integrated { trunk_oid, .. } => {
                    PredecessorIntegrationProof::ReachedTrunk(trunk_oid.clone())
                },
                IntegrationEvidenceStatus::NotIntegrated
                | IntegrationEvidenceStatus::TrunkRewritten
                | IntegrationEvidenceStatus::ObjectUnknown => PredecessorIntegrationProof::Unproven,
            },
        }
    }
}

/// The grouped ancestry answers for the predecessor's incorporation subject, phase start, and the
/// trunk commit its integration proof names.
#[derive(Clone, Copy)]
struct PredecessorSuccessorReachability<'classification> {
    against_incorporation_subject: &'classification ProtectedTipSuccessorHeadClassification,
    since_phase_start:             &'classification ProtectedTipSuccessorHeadClassification,
    /// Absent when the predecessor holds no proof, so there is no trunk commit to ask about.
    against_reached_trunk:         Option<&'classification ProtectedTipSuccessorHeadClassification>,
}

impl PredecessorSuccessorReachability<'_> {
    /// Whether this successor head's history contains the commit the predecessor's proof names.
    fn reached_trunk_is_ancestor(&self, successor_head: &GitObjectId) -> bool {
        let Some(ProtectedTipSuccessorHeadClassification::Classified(classified_heads)) =
            self.against_reached_trunk
        else {
            return false;
        };
        classified_heads.iter().any(|classified_head| {
            matches!(
                classified_head,
                CandidateHeadReachability::Descendant { head, .. } if head == successor_head
            )
        })
    }

    /// First-parent commits each successor head gained since the predecessor's phase start.
    fn phase_start_target_histories(&self) -> HashMap<GitObjectId, Vec<GitObjectId>> {
        match self.since_phase_start {
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

/// Actual repository reconciliation plus proposed-target constraints prepared under one lock.
pub(crate) struct GateReconciliation {
    reconciliation:            ReconciliationPlan,
    constraints:               IntegrationConstraintProjection,
    reservations:              RetainedReservationSet,
    judged_reservations:       HashSet<ReservationId>,
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
    operations:              Vec<JournalOperation>,
    retention_repairs:       Vec<ReservationRetentionRefRepair>,
    retention_deletions:     Vec<ReservationId>,
    evidence:                Vec<ReconciledEvidence>,
    /// Reservations whose lost-evidence directive two consecutive passes have now derived.
    confirmed_lost_evidence: Vec<ReservationId>,
}

/// Whether a reconciliation pass created cover reservations; created covers need one more pass
/// so they are judged in the same reconciliation.
#[derive(Clone, Copy)]
enum CoverCreation {
    NoneCreated,
    Created,
}

struct ReconciliationAction {
    active_holders:                Vec<ActiveHolder>,
    cover_marker_publications:     Vec<(WorktreeContext, CoordinationRunId)>,
    cover_creation:                CoverCreation,
    uncovered_targets:             Vec<(IntegrationTarget, Vec<ReservationId>)>,
    covered_targets:               HashSet<IntegrationTarget>,
    cover_requirements:            HashMap<IntegrationTarget, (WorktreeId, IntegrationTarget)>,
    marker_contexts:               Vec<WorktreeMarkerSweepContext>,
    repository_root:               PathBuf,
    retention_repairs:             Vec<ReservationRetentionRefRepair>,
    retention_deletions:           Vec<ReservationId>,
    retention_commit_resolution:   RetentionCommitResolution,
    alert_subjects:                Vec<AlertSubject>,
    evidence:                      Vec<ReconciledEvidence>,
    /// Reservations whose lost-evidence directive has been derived by two consecutive passes.
    ///
    /// A single pass reads a repository that is still moving -- a push amends, a rebase lands, a
    /// pack is written -- so a judgement it derives alone is as likely to be an artifact of the
    /// moment as a fact about the work. `alert::for_lost_integration_evidence` therefore speaks
    /// only when the status a previous pass recorded and the status this pass derived agree the
    /// proof is gone. A proof genuinely lost is reported one pass late, and every transient
    /// disagreement the next pass overturns is never reported at all.
    confirmed_lost_evidence:       Vec<ReservationId>,
    repository_snapshot:           RepositorySnapshot,
    trunk_reachability:            TargetIntegrationReachability,
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

impl ReconciliationAction {
    fn cover_is_missing(&self, reservation_id: ReservationId) -> bool {
        cover_is_missing(
            &self.repository_snapshot,
            &self.covered_targets,
            reservation_id,
        )
    }
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
    let first = reconcile_with_open_ledger_once(worktree_context, ledger, berth_config, request)?;
    if matches!(first.cover_creation, CoverCreation::Created) {
        reconcile_with_open_ledger_once(worktree_context, ledger, berth_config, request)
    } else {
        Ok(first)
    }
}

fn reconcile_with_open_ledger_once(
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
                            cover_actors:           prepared.cover_actors,
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

/// The branch chosen for a rewrite subject, or a Git failure that defers it.
enum RewriteTargetJudgment {
    Available(IntegrationTarget),
    Unavailable,
}

/// Per-preflight branch judgments and the tip observations they select.
#[derive(Default)]
struct RewriteTargetObservations {
    judgments: BTreeMap<IntegrationTarget, RewriteTargetJudgment>,
    tips:      BTreeMap<IntegrationTarget, Result<GitObjectId, GitError>>,
}

/// Observe one recorded target only once across the rewrite subjects in a preflight.
fn rewrite_target_judgment<'cache, ErrorType>(
    judgments: &'cache mut BTreeMap<IntegrationTarget, RewriteTargetJudgment>,
    recorded_target: &IntegrationTarget,
    observe: impl FnOnce(&IntegrationTarget) -> Result<IntegrationTarget, ErrorType>,
) -> &'cache RewriteTargetJudgment {
    judgments.entry(recorded_target.clone()).or_insert_with(|| {
        observe(recorded_target).map_or(
            RewriteTargetJudgment::Unavailable,
            RewriteTargetJudgment::Available,
        )
    })
}

/// Reuse one target tip observation across every rewrite subject at that branch.
fn rewrite_target_tip<'cache, ErrorType>(
    target_tips: &'cache mut BTreeMap<IntegrationTarget, Result<GitObjectId, ErrorType>>,
    target: &IntegrationTarget,
    resolution_calls: &mut u64,
    resolve: impl FnOnce(&IntegrationTarget) -> Result<GitObjectId, ErrorType>,
) -> &'cache Result<GitObjectId, ErrorType> {
    target_tips.entry(target.clone()).or_insert_with(|| {
        *resolution_calls += 1;
        resolve(target)
    })
}

enum RewriteTargetTip<'cache> {
    Resolved(&'cache GitObjectId),
    Unavailable,
}

fn rewrite_target_tip_for_reservation<'cache>(
    repository_root: &Path,
    reservation_id: ReservationId,
    reservations: &RetainedReservationSet,
    repository_trunk: &IntegrationTarget,
    observations: &'cache mut RewriteTargetObservations,
    resolution_calls: &mut u64,
) -> Result<RewriteTargetTip<'cache>, ReconcileError> {
    let target = reservations
        .target_of(reservation_id, repository_trunk)
        .map_err(ReconcileError::Replay)?;
    let RewriteTargetJudgment::Available(judging_target) =
        rewrite_target_judgment(&mut observations.judgments, &target, |target| {
            target
                .judging_branch(repository_root, repository_trunk)
                .map(|judgment| judgment.target().clone())
        })
    else {
        return Ok(RewriteTargetTip::Unavailable);
    };
    Ok(rewrite_target_tip(
        &mut observations.tips,
        judging_target,
        resolution_calls,
        |target| git::branch_object_id(repository_root, target.short_name()),
    )
    .as_ref()
    .map_or(RewriteTargetTip::Unavailable, RewriteTargetTip::Resolved))
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
    let repository_trunk = berth_config.repository_trunk().map_err(|reason| {
        ReconcileError::Config(ConfigError::InvalidValue {
            key:   "trunk".to_owned(),
            value: reason,
        })
    })?;
    let mut target_observations = RewriteTargetObservations::default();
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
            &repository_trunk,
            &mut target_observations,
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
    repository_trunk: &IntegrationTarget,
    target_observations: &mut RewriteTargetObservations,
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
        let target_tip = rewrite_target_tip_for_reservation(
            repository_root,
            reservation_id,
            reservations,
            repository_trunk,
            target_observations,
            &mut preflight.trunk_resolution_calls,
        )?;
        let RewriteTargetTip::Resolved(target_tip) = target_tip else {
            deferred = true;
            preflight.defer_subject(reservation, marker, interval.destinations.into_iter());
            continue;
        };
        let comparison = compare_mapped_phase(
            repository_root,
            reservation,
            protected_tip,
            target_tip,
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
                        trunk_oid:        target_tip.clone(),
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

/// Charge rewrite acceptance to the reservation's target while comparing the mapped destination.
fn compare_mapped_phase(
    repository_root: &Path,
    reservation: &Reservation,
    protected_tip: &ProtectedReservationTip,
    target_tip: &GitObjectId,
    interval: &MappedPhaseInterval,
    budget: &mut ReconciliationScopedPatchEvaluationBudget,
) -> ScopedPatchComparisonObservation {
    let key = ScopedPatchEvaluationKey {
        phase_start_head: reservation.phase_start_head().as_ref().clone(),
        protected_tip:    protected_tip.as_ref().clone(),
        target_trunk:     target_tip.clone(),
        scopes:           reservation
            .scopes()
            .as_slice()
            .iter()
            .map(|scope| ScopedPatchEvaluationScope {
                path:       scope.path.clone(),
                scope_kind: scope.kind.into(),
            })
            .collect(),
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
    cover_actors:           HashMap<ReservationId, CoverClaimActor>,
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
        ReconciliationBuildError::Config(error) => ReconciliationPlanningError::Config(error),
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
        cover_actors: reconciliation_plan.cover_actors,
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
    committed_by_holder: HashMap<(WorktreeId, IntegrationTarget), CommittedMergeEvidence>,
    observed_by_holder:  HashMap<(WorktreeId, IntegrationTarget), MergeExtent>,
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
            .entry((
                reservation.actor().worktree,
                snapshot.recorded_target(reservation.id()).clone(),
            ))
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
    let observed_by_holder = observed_by_worktree
        .iter()
        .filter_map(|(key, observation)| {
            observation
                .as_ref()
                .ok()
                .map(|observation| (key.clone(), observation.extent.clone()))
        })
        .collect();
    Ok(MergeExtentReconciliation {
        operations,
        observed_by_holder,
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
    let JudgedTargetTip::Resolved(trunk) = snapshot.target_for(reservation.id()) else {
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
            && snapshot.recorded_target(candidate.id())
                == snapshot.recorded_target(reservation.id())
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
            && snapshot.recorded_target(holder.id()) == snapshot.recorded_target(reservation.id())
            && holder.merge_extent().matches_key(&key)
    }) && (!settlement_needs_committed_paths
        || matches!(cached.merge_extent(), MergeExtent::Empty { .. }))
    {
        return Ok(HolderMergeProtection {
            extent:          cached.merge_extent().clone(),
            committed_paths: CommittedMergeEvidence::Unavailable,
        });
    }
    git_cost.path_queries += UNMERGED_BRANCH_PATH_GIT_QUERIES;
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
        targets,
        reservation_targets,
        resolved_candidates,
        repository_evidence_observations,
        trunk_reachability,
        trunk_resolution_calls,
    } = observe_repository_facts(
        reservations,
        ordering_graph,
        repository_observation_scope,
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
    let observed_trunk = JudgedTargetTip::repository_trunk(&targets, &repository_trunk);
    let cross_target_predecessors = cross_target_predecessor_evidence(
        ordering_graph,
        repository_observation_scope,
        reservations,
        &reservation_snapshots,
        &observed_trunk,
        &trunk_reachability,
    );
    let repository_snapshot = RepositorySnapshot::new(
        repository_trunk,
        targets,
        reservation_targets,
        reservation_snapshots.clone(),
        Vec::new(),
        cross_target_predecessors,
    );
    let active_holders = reservations
        .iter()
        .filter(|reservation| matches!(reservation.lifecycle(), ReservationLifecycle::Active))
        .map(|reservation| ActiveHolder {
            worktree_id:         reservation.actor().worktree,
            coordination_run_id: reservation.actor().run,
        })
        .collect();
    let mut plan = ReconciliationPlan {
        operations:   changes.operations,
        cover_actors: HashMap::new(),
        action:       ReconciliationAction {
            active_holders,
            cover_marker_publications: Vec::new(),
            cover_creation: CoverCreation::NoneCreated,
            uncovered_targets: Vec::new(),
            covered_targets: HashSet::new(),
            cover_requirements: HashMap::new(),
            marker_contexts: worktree_registry.marker_sweep_contexts(common_git_directory),
            repository_root: repository_root.to_path_buf(),
            retention_repairs: changes.retention_repairs,
            retention_deletions: changes.retention_deletions,
            retention_commit_resolution: RetentionCommitResolution::InitialObservation(
                resolved_candidates,
            ),
            alert_subjects,
            settlements: Vec::new(),
            evidence: changes.evidence,
            confirmed_lost_evidence: changes.confirmed_lost_evidence,
            repository_snapshot,
            trunk_reachability,
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
        &worktree_registry,
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
    worktree_registry: &WorktreeRegistry,
    observation_scope: RepositoryObservationScope,
    mut snapshots: Vec<RepositoryReservationSnapshot>,
    context: &mut ReconciliationEvidenceContext<'_>,
    plan: &mut ReconciliationPlan,
) -> Result<(), ReservationReplayError> {
    append_cover_claims(reservations, worktree_registry, plan);
    let extents = derive_merge_extents(
        reservations,
        &plan.action.repository_snapshot,
        &plan.operations,
        &mut plan.action.merge_extent_git_cost,
    )?;
    plan.action.covered_targets = covered_targets_for_pass(reservations, &extents, plan);
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
    append_orphan_retirements(reservations, plan);
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
    replace_completed_repository_snapshot(
        reservations,
        ordering_graph,
        observation_scope,
        snapshots,
        successors.by_predecessor,
        plan,
    )?;
    Ok(())
}

/// Rebuild the edge view from the reservation states the completed plan will commit.
fn replace_completed_repository_snapshot(
    reservations: &RetainedReservationSet,
    ordering_graph: &OrderingGraph,
    observation_scope: RepositoryObservationScope,
    mut snapshots: Vec<RepositoryReservationSnapshot>,
    successor_incorporation: Vec<(ReservationId, PredecessorSuccessorIncorporation)>,
    plan: &mut ReconciliationPlan,
) -> Result<(), ReservationReplayError> {
    let (repository_trunk, mut targets, mut reservation_targets) =
        plan.action.repository_snapshot.target_facts();
    for operation in &plan.operations {
        if let JournalOperation::Claim {
            reservation_id,
            source: ClaimSource::Cover { .. },
            target: Some(target),
            trunk_at_claim,
            head_snapshot,
            ..
        } = operation
        {
            reservation_targets.insert(*reservation_id, target.target.clone());
            if let TrunkObservationAtClaim::Resolved(commit) = trunk_at_claim {
                targets
                    .entry(target.target.clone())
                    .or_insert_with(|| TargetObservation::Resolved(commit.clone()));
            }
            let head = match head_snapshot {
                ClaimHeadSnapshot::Branch { head, .. } | ClaimHeadSnapshot::Detached { head } => {
                    head.as_ref().clone()
                },
            };
            snapshots.push(RepositoryReservationSnapshot {
                reservation_id:    *reservation_id,
                worktree_liveness: WorktreeLiveness::Live,
                worktree_head:     WorktreeHead::Resolved(head),
                evidence:          RepositoryReservationEvidence::Active,
            });
        }
    }
    let observed_trunk = JudgedTargetTip::repository_trunk(&targets, &repository_trunk);
    plan.action.trunk_reachability.extend_for_snapshot(
        &plan.action.repository_root,
        &observed_trunk,
        &snapshots,
        reservations,
        ordering_graph,
        observation_scope,
    )?;
    let cross_target_predecessors = cross_target_predecessor_evidence(
        ordering_graph,
        observation_scope,
        reservations,
        &snapshots,
        &observed_trunk,
        &plan.action.trunk_reachability,
    );
    plan.action.repository_snapshot = RepositorySnapshot::new(
        repository_trunk,
        targets,
        reservation_targets,
        snapshots,
        successor_incorporation,
        cross_target_predecessors,
    );
    Ok(())
}

/// Give each checked-out integration branch a reservation against its own target.
fn append_cover_claims(
    reservations: &RetainedReservationSet,
    registry: &WorktreeRegistry,
    plan: &mut ReconciliationPlan,
) {
    let snapshot = &plan.action.repository_snapshot;
    let repository_trunk = snapshot.repository_trunk_target().clone();
    let waiting = waiting_cover_targets(reservations, snapshot);
    for (branch, reservation_ids) in waiting {
        let Some(context) = registry.context_for_branch(branch.reference()) else {
            plan.action
                .uncovered_targets
                .push((branch, reservation_ids));
            continue;
        };
        append_cover_for_branch(reservations, context, &branch, &repository_trunk, plan);
    }
}

fn waiting_cover_targets(
    reservations: &RetainedReservationSet,
    snapshot: &RepositorySnapshot,
) -> BTreeMap<IntegrationTarget, Vec<ReservationId>> {
    let repository_trunk = snapshot.repository_trunk_target();
    let mut waiting: BTreeMap<IntegrationTarget, Vec<ReservationId>> = BTreeMap::new();
    for reservation in reservations.iter() {
        if matches!(
            reservation.lifecycle(),
            ReservationLifecycle::Released { .. }
        ) {
            continue;
        }
        let target = snapshot.recorded_target(reservation.id());
        if target != repository_trunk
            && matches!(
                snapshot.targets().get(target),
                Some(TargetObservation::Resolved(_))
            )
        {
            waiting
                .entry(target.clone())
                .or_default()
                .push(reservation.id());
        }
    }
    waiting
}

fn append_cover_for_branch(
    reservations: &RetainedReservationSet,
    context: &WorktreeContext,
    branch: &IntegrationTarget,
    repository_trunk: &IntegrationTarget,
    plan: &mut ReconciliationPlan,
) {
    let Ok(identity) =
        ledger::worktree_identity(context.administrative_directory(), context.worktree_kind())
    else {
        return;
    };
    let parent = ledger::resolve_claim_target(
        context.common_git_directory(),
        Some(branch.reference()),
        TargetSelectionRequest::AutomaticAcquisition,
        repository_trunk,
        |candidate| {
            git::branch_object_id(context.repository_root(), candidate.short_name()).is_ok()
        },
    );
    let Ok(parent) = parent else {
        return;
    };
    plan.action
        .cover_requirements
        .insert(branch.clone(), (identity.id, parent.target.clone()));
    let has_cover = reservations.iter().any(|reservation| {
        reservation.actor().worktree == identity.id
            && !matches!(
                reservation.lifecycle(),
                ReservationLifecycle::Released { .. }
            )
            && reservations
                .target_of(reservation.id(), repository_trunk)
                .is_ok_and(|recorded| recorded == parent.target)
    });
    if has_cover {
        return;
    }
    let Ok(parent_commit) =
        git::branch_object_id(context.repository_root(), parent.target.short_name())
    else {
        return;
    };
    let Ok(path_case) = PathCase::read(context.common_git_directory()) else {
        return;
    };
    let Ok(Some(operation)) = worktree::cover_claim(
        context.clone(),
        identity.id,
        branch.clone(),
        parent,
        parent_commit,
        path_case,
    ) else {
        return;
    };
    let JournalOperation::Claim { reservation_id, .. } = &operation else {
        return;
    };
    let (run, publish) = match context.existing_coordination_run() {
        Ok(ExistingCoordinationRun::Present(run)) => (run, false),
        Ok(ExistingCoordinationRun::Absent) => (CoordinationRunId::new(), true),
        Err(_) => return,
    };
    plan.cover_actors.insert(
        *reservation_id,
        CoverClaimActor {
            worktree: identity.id,
            run,
        },
    );
    plan.action.cover_creation = CoverCreation::Created;
    plan.action.active_holders.push(ActiveHolder {
        worktree_id:         identity.id,
        coordination_run_id: run,
    });
    if publish {
        plan.action
            .cover_marker_publications
            .push((context.clone(), run));
    }
    plan.operations.push(operation);
}

/// A cover counts only when this pass observed its holder against the branch's current tip.
fn covered_targets_for_pass(
    reservations: &RetainedReservationSet,
    extents: &MergeExtentReconciliation,
    plan: &ReconciliationPlan,
) -> HashSet<IntegrationTarget> {
    plan.action
        .cover_requirements
        .iter()
        .filter_map(|(branch, (worktree, parent))| {
            let Some(TargetObservation::Resolved(branch_tip)) =
                plan.action.repository_snapshot.targets().get(branch)
            else {
                return None;
            };
            let extent = extents
                .observed_by_holder
                .get(&(*worktree, parent.clone()))?;
            let key = match extent {
                MergeExtent::Protected { key, .. } | MergeExtent::Empty { key } => key,
                MergeExtent::NotDerived { .. } | MergeExtent::Unavailable { .. } => return None,
            };
            if key.head != *branch_tip {
                return None;
            }
            reservations
                .iter()
                .any(|reservation| {
                    reservation.actor().worktree == *worktree
                        && !matches!(
                            reservation.lifecycle(),
                            ReservationLifecycle::Released { .. }
                        )
                        && plan
                            .action
                            .repository_snapshot
                            .recorded_target(reservation.id())
                            == parent
                })
                .then(|| branch.clone())
        })
        .collect()
}

fn cover_is_missing(
    repository_snapshot: &RepositorySnapshot,
    covered_targets: &HashSet<IntegrationTarget>,
    reservation_id: ReservationId,
) -> bool {
    let target = repository_snapshot.recorded_target(reservation_id);
    target != repository_snapshot.repository_trunk_target()
        && matches!(
            repository_snapshot.targets().get(target),
            Some(TargetObservation::Resolved(_))
        )
        && !covered_targets.contains(target)
}

/// Recorded reservation identities partitioned by their integration branch.
struct TargetReservationGroups {
    groups:           BTreeMap<IntegrationTarget, HashSet<ReservationId>>,
    recorded_targets: HashMap<ReservationId, IntegrationTarget>,
}

/// Replay target assignments once before observing any branch.
fn group_reservations_by_target(
    reservations: &RetainedReservationSet,
    repository_trunk: &IntegrationTarget,
) -> Result<TargetReservationGroups, ReservationReplayError> {
    let mut groups = BTreeMap::new();
    groups
        .entry(repository_trunk.clone())
        .or_insert_with(HashSet::new);
    let mut recorded_targets = HashMap::new();
    for reservation in reservations.iter() {
        let target = reservations.target_of(reservation.id(), repository_trunk)?;
        groups
            .entry(target.clone())
            .or_default()
            .insert(reservation.id());
        recorded_targets.insert(reservation.id(), target);
    }
    Ok(TargetReservationGroups {
        groups,
        recorded_targets,
    })
}

/// Read branch reachability and reservation evidence for one repository observation.
fn observe_repository_facts(
    reservations: &RetainedReservationSet,
    ordering_graph: &OrderingGraph,
    repository_observation_scope: RepositoryObservationScope,
    worktree_context: &WorktreeContext,
    reconciliation_evidence_context: &mut ReconciliationEvidenceContext<'_>,
) -> Result<ObservedRepositoryFacts, ReconciliationBuildError> {
    let repository_root = worktree_context.repository_root();
    thread::scope(|scope| {
        let worktree_registry = scope.spawn(|| WorktreeRegistry::read(worktree_context));
        let repository_trunk = reconciliation_evidence_context
            .berth_config
            .repository_trunk()
            .map_err(|_| {
                ReconciliationBuildError::Config(ConfigError::InvalidValue {
                    key:   "trunk".to_owned(),
                    value: reconciliation_evidence_context.berth_config.trunk.clone(),
                })
            })?;
        let TargetReservationGroups {
            mut groups,
            recorded_targets: reservation_targets,
        } = group_reservations_by_target(reservations, &repository_trunk)?;
        let mut target_jobs = Vec::new();
        for (target, selected) in &groups {
            if target == &repository_trunk {
                continue;
            }
            let branch = target.short_name().to_owned();
            let selected = selected.clone();
            target_jobs.push((
                target.clone(),
                selected.clone(),
                scope.spawn(move || {
                    TargetIntegrationReachability::observe_configured_trunk(
                        repository_root,
                        reservations,
                        ordering_graph,
                        &branch,
                        &selected,
                        TrunkEdgeCandidates::Excluded,
                    )
                }),
            ));
        }
        let mut observations = BTreeMap::new();
        let mut reachabilities = BTreeMap::new();
        let mut resolved_candidates = ResolvedBatchCommitCandidates::default();
        for (target, selected, job) in target_jobs {
            let (observation, reachability) = job
                .join()
                .map_err(|_| WorktreeRegistryError::ObservationWorkerPanicked)??;
            if matches!(observation, TargetObservation::Missing) {
                groups
                    .entry(repository_trunk.clone())
                    .or_default()
                    .extend(selected);
            }
            resolved_candidates.extend(reachability.resolved_candidates.clone());
            observations.insert(target.clone(), observation);
            reachabilities.insert(target, reachability);
        }
        let trunk_reservations = groups.get(&repository_trunk).cloned().unwrap_or_default();
        let target_tips = observed_predecessor_target_tips(
            ordering_graph,
            repository_observation_scope,
            &repository_trunk,
            &reservation_targets,
            &observations,
        );
        let (trunk_observation, trunk_reachability) =
            TargetIntegrationReachability::observe_configured_trunk(
                repository_root,
                reservations,
                ordering_graph,
                repository_trunk.short_name(),
                &trunk_reservations,
                TrunkEdgeCandidates::including(repository_observation_scope, &target_tips),
            )?;
        resolved_candidates.extend(trunk_reachability.resolved_candidates.clone());
        observations.insert(repository_trunk.clone(), trunk_observation);
        reachabilities.insert(repository_trunk.clone(), trunk_reachability);
        let repository_evidence_observations = observe_target_group_evidence(
            reservations,
            &groups,
            &repository_trunk,
            &observations,
            &reachabilities,
            repository_root,
            reconciliation_evidence_context.scoped_patch_evaluation_budget,
        )?;
        let trunk_reachability = reachabilities.remove(&repository_trunk).unwrap_or_default();
        let worktree_registry = worktree_registry
            .join()
            .map_err(|_| WorktreeRegistryError::ObservationWorkerPanicked)??;
        Ok::<_, ReconciliationBuildError>(ObservedRepositoryFacts {
            worktree_registry,
            repository_trunk,
            targets: observations,
            reservation_targets,
            resolved_candidates,
            repository_evidence_observations,
            trunk_reachability,
            trunk_resolution_calls: groups.len() as u64,
        })
    })
}

/// Evaluate each reservation once in the context of the branch that judges it this pass.
fn observe_target_group_evidence(
    reservations: &RetainedReservationSet,
    groups: &BTreeMap<IntegrationTarget, HashSet<ReservationId>>,
    repository_trunk: &IntegrationTarget,
    observations: &BTreeMap<IntegrationTarget, TargetObservation>,
    reachabilities: &BTreeMap<IntegrationTarget, TargetIntegrationReachability>,
    repository_root: &Path,
    budget: &mut ReconciliationScopedPatchEvaluationBudget,
) -> Result<Vec<RepositoryEvidenceObservation>, ReservationReplayError> {
    let mut indexed_evidence = Vec::new();
    let unavailable_reachability = TargetIntegrationReachability::default();
    for (target, selected) in groups {
        if target != repository_trunk
            && matches!(observations.get(target), Some(TargetObservation::Missing))
        {
            continue;
        }
        let judged_target = if matches!(observations.get(target), Some(TargetObservation::Missing))
        {
            repository_trunk
        } else {
            target
        };
        let judged_commit =
            observations
                .get(judged_target)
                .map_or(
                    JudgedTargetTip::ObjectUnknown,
                    |observation| match observation {
                        TargetObservation::Resolved(commit) => {
                            JudgedTargetTip::Resolved(commit.clone())
                        },
                        TargetObservation::Missing | TargetObservation::ObjectUnknown => {
                            JudgedTargetTip::ObjectUnknown
                        },
                    },
                );
        let mut context = TargetIntegrationEvidenceContext {
            repository_root,
            repository_trunk: &judged_commit,
            integration_reachability: reachabilities
                .get(judged_target)
                .unwrap_or(&unavailable_reachability),
            scoped_patch_evaluation_budget: budget,
        };
        indexed_evidence.extend(
            scoped_patch_evaluation_order(reservations, &judged_commit)
                .into_iter()
                .filter(|(_, reservation)| selected.contains(&reservation.id()))
                .map(|(index, reservation)| {
                    repository_evidence(reservation, &mut context)
                        .map(|observation| (index, observation))
                })
                .collect::<Result<Vec<_>, ReservationReplayError>>()?,
        );
    }
    indexed_evidence.sort_by_key(|(index, _)| *index);
    Ok(indexed_evidence
        .into_iter()
        .map(|(_, observation)| observation)
        .collect())
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
    repository_trunk: &JudgedTargetTip,
) -> Vec<(usize, &'reservation Reservation)> {
    let mut evaluation_order = reservations.iter().enumerate().collect::<Vec<_>>();
    if let JudgedTargetTip::Resolved(target) = repository_trunk {
        evaluation_order
            .sort_by_key(|(_, reservation)| reservation.scoped_patch_evaluation_priority(target));
    }
    evaluation_order
}

/// Prepare the actual reconciliation and proposed-ref constraint read from one replay.
///
/// Prepared decisions project actual-trunk settlements before observing the proposal. Committed
/// audits keep replayed lifecycles so newly integrated reservations still enter the permit audit.
/// Proposed-target evidence never settles reservations.
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
    proposed_move: &ProposedTargetMove,
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
        ReconciliationBuildError::Config(error) => GateReconciliationError::Config(error),
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
    let proposed_observation = observe_proposed_target(
        &reservations,
        &ordering_graph,
        RepositoryObservationScope::CurrentOrderingGraph,
        &reconciliation.action.repository_snapshot,
        worktree_context,
        proposed_move,
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
        judged_reservations: proposed_observation.judged_reservations,
        deferred_rewrite_subjects: rewrite_preflight.deferred_subjects,
    })
}

fn reservations_judged_at_proposed_target(
    reservations: &RetainedReservationSet,
    actual_snapshot: &RepositorySnapshot,
    proposed_target: &IntegrationTarget,
) -> HashSet<ReservationId> {
    reservations
        .iter()
        .filter(|reservation| {
            let target = actual_snapshot.recorded_target(reservation.id());
            target == proposed_target
                || (proposed_target == actual_snapshot.repository_trunk_target()
                    && matches!(
                        actual_snapshot.targets().get(target),
                        Some(TargetObservation::Missing)
                    ))
        })
        .map(Reservation::id)
        .collect()
}

fn observe_proposed_target(
    reservations: &RetainedReservationSet,
    ordering_graph: &OrderingGraph,
    repository_observation_scope: RepositoryObservationScope,
    actual_snapshot: &RepositorySnapshot,
    worktree_context: &WorktreeContext,
    proposed_move: &ProposedTargetMove,
    reconciliation_evidence_context: &mut ReconciliationEvidenceContext<'_>,
) -> Result<ProposedTargetObservation, GateReconciliationError> {
    let proposed_target = &proposed_move.target;
    let target_tip = JudgedTargetTip::Resolved(proposed_move.proposed.clone());
    let selected =
        reservations_judged_at_proposed_target(reservations, actual_snapshot, proposed_target);
    let (trunk_target, mut targets, reservation_targets) = actual_snapshot.target_facts();
    let integration_reachability = proposed_target_reachability(
        reservations,
        ordering_graph,
        repository_observation_scope,
        actual_snapshot,
        worktree_context,
        proposed_move,
        &selected,
    )?;
    let mut target_evidence_context = TargetIntegrationEvidenceContext {
        repository_root:                worktree_context.repository_root(),
        repository_trunk:               &target_tip,
        integration_reachability:       &integration_reachability,
        scoped_patch_evaluation_budget: reconciliation_evidence_context
            .scoped_patch_evaluation_budget,
    };
    let mut indexed_evidence = scoped_patch_evaluation_order(reservations, &target_tip)
        .into_iter()
        .filter(|(_, reservation)| selected.contains(&reservation.id()))
        .map(|(index, reservation)| {
            repository_evidence(reservation, &mut target_evidence_context)
                .map(|observation| (index, reservation, observation))
                .map_err(GateReconciliationError::Reservation)
        })
        .collect::<Result<Vec<_>, GateReconciliationError>>()?;
    indexed_evidence.sort_by_key(|(index, _, _)| *index);
    let mut operations = Vec::new();
    let mut reservation_snapshots = indexed_evidence
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
    for reservation in reservations
        .iter()
        .filter(|reservation| !selected.contains(&reservation.id()))
    {
        reservation_snapshots.push(
            actual_snapshot
                .reservation(reservation.id())
                .map_err(GateReconciliationError::MissingReadinessFact)?
                .clone(),
        );
    }
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
    targets.insert(
        proposed_target.clone(),
        TargetObservation::Resolved(proposed_move.proposed.clone()),
    );
    let cross_target_predecessors = if proposed_target == &trunk_target {
        cross_target_predecessor_evidence(
            ordering_graph,
            repository_observation_scope,
            reservations,
            &reservation_snapshots,
            &target_tip,
            &integration_reachability,
        )
    } else {
        actual_snapshot.cross_target_predecessor_evidence().clone()
    };
    Ok(ProposedTargetObservation {
        snapshot: RepositorySnapshot::new(
            trunk_target,
            targets,
            reservation_targets,
            reservation_snapshots,
            successor_incorporation.by_predecessor,
            cross_target_predecessors,
        ),
        operations,
        judged_reservations: selected,
    })
}

fn proposed_target_reachability(
    reservations: &RetainedReservationSet,
    ordering_graph: &OrderingGraph,
    scope: RepositoryObservationScope,
    actual_snapshot: &RepositorySnapshot,
    worktree_context: &WorktreeContext,
    proposed_move: &ProposedTargetMove,
    selected: &HashSet<ReservationId>,
) -> Result<TargetIntegrationReachability, GateReconciliationError> {
    let (trunk_target, targets, reservation_targets) = actual_snapshot.target_facts();
    let target_tips = if proposed_move.target == trunk_target {
        observed_predecessor_target_tips(
            ordering_graph,
            scope,
            &trunk_target,
            &reservation_targets,
            &targets,
        )
    } else {
        Vec::new()
    };
    let edge_candidates = if proposed_move.target == trunk_target {
        TrunkEdgeCandidates::including(scope, &target_tips)
    } else {
        TrunkEdgeCandidates::Excluded
    };
    TargetIntegrationReachability::observe(
        worktree_context.repository_root(),
        reservations,
        ordering_graph,
        &JudgedTargetTip::Resolved(proposed_move.proposed.clone()),
        selected,
        edge_candidates,
    )
    .map_err(GateReconciliationError::Reservation)
}

impl GateReconciliation {
    /// Whether this proposed branch update judges the reservation.
    pub(crate) fn judges(&self, reservation_id: ReservationId) -> bool {
        self.judged_reservations.contains(&reservation_id)
    }
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
    ) -> (
        Vec<JournalOperation>,
        HashMap<ReservationId, CoverClaimActor>,
        CommittedHookReconciliationAction,
    ) {
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
            // The committed audit retains no claim from the reconciliation plan.
            HashMap::new(),
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
    ) -> (
        Vec<JournalOperation>,
        HashMap<ReservationId, CoverClaimActor>,
        GateReconciliationAction<Decision>,
    ) {
        self.reconciliation.operations.extend(additional_operations);
        (
            self.reconciliation.operations,
            self.reconciliation.cover_actors,
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
    Config(ConfigError),
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
            Self::Config(error) => error.fmt(formatter),
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
            materialized,
        )),
        ReservationEvidenceState::Released {
            protected_tip,
            trunk_snapshot,
            disposition,
            integration_status: materialized,
        } => Ok(observe_released_repository_evidence(
            target_evidence_context,
            reservation,
            protected_tip,
            &trunk_snapshot,
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
    materialized: IntegrationEvidenceStatus,
) -> RepositoryEvidenceObservation {
    let observation = revalidate_protected_tip(
        target_evidence_context,
        reservation,
        &protected_tip,
        trunk_snapshot,
        materialized,
    );
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
    trunk_snapshot: &GitObjectId,
    disposition: ReleaseDisposition,
    materialized: IntegrationEvidenceStatus,
) -> RepositoryEvidenceObservation {
    let observation = match disposition.revalidation_subject() {
        ReleaseRevalidationSubject::ProtectedTip => revalidate_protected_tip(
            target_evidence_context,
            reservation,
            &protected_tip,
            trunk_snapshot,
            materialized,
        ),
        ReleaseRevalidationSubject::RewrittenIntegration(trunk_commit) => {
            IntegrationStatusObservation::new(
                match target_evidence_context.repository_trunk {
                    JudgedTargetTip::Resolved(trunk) => {
                        trunk_commit.revalidate_ancestry(trunk, |witness| {
                            target_evidence_context
                                .integration_reachability
                                .for_ancestor(witness)
                        })
                    },
                    JudgedTargetTip::ObjectUnknown => IntegrationEvidenceStatus::ObjectUnknown,
                },
                EvidenceRevalidationObservation::Apply,
                ScopedPatchComparisonJournalUpdate::Unchanged,
            )
        },
        ReleaseRevalidationSubject::None => IntegrationStatusObservation::new(
            materialized,
            EvidenceRevalidationObservation::NotApplicable,
            ScopedPatchComparisonJournalUpdate::Unchanged,
        ),
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

/// Answer one reservation's integration against the observed trunk, re-deriving only when needed.
fn revalidate_protected_tip(
    target_evidence_context: &mut TargetIntegrationEvidenceContext<'_>,
    reservation: &Reservation,
    protected_tip: &ProtectedReservationTip,
    trunk_snapshot: &GitObjectId,
    materialized: IntegrationEvidenceStatus,
) -> IntegrationStatusObservation {
    let JudgedTargetTip::Resolved(current_trunk_oid) = target_evidence_context.repository_trunk
    else {
        return IntegrationStatusObservation::new(
            IntegrationEvidenceStatus::ObjectUnknown,
            EvidenceRevalidationObservation::Apply,
            ScopedPatchComparisonJournalUpdate::Unchanged,
        );
    };
    match IntegrationProofStanding::observe(
        &materialized,
        trunk_snapshot,
        target_evidence_context.integration_reachability,
    ) {
        IntegrationProofStanding::Holds => IntegrationStatusObservation::new(
            reanchored_proof(
                materialized,
                current_trunk_oid,
                target_evidence_context
                    .integration_reachability
                    .for_ancestor(protected_tip.as_ref()),
            ),
            EvidenceRevalidationObservation::Apply,
            ScopedPatchComparisonJournalUpdate::Unchanged,
        ),
        IntegrationProofStanding::ProvingTrunkUnknown => IntegrationStatusObservation::new(
            IntegrationEvidenceStatus::ObjectUnknown,
            EvidenceRevalidationObservation::Apply,
            ScopedPatchComparisonJournalUpdate::Unchanged,
        ),
        IntegrationProofStanding::Derive { previous_trunk } => {
            integration_status_with_retained_verdict(
                target_evidence_context,
                reservation,
                protected_tip,
                current_trunk_oid,
                &previous_trunk,
                materialized,
            )
        },
    }
}

fn integration_status_with_retained_verdict(
    target_evidence_context: &mut TargetIntegrationEvidenceContext<'_>,
    reservation: &Reservation,
    protected_tip: &ProtectedReservationTip,
    target: &GitObjectId,
    previous_trunk: &GitObjectId,
    materialized: IntegrationEvidenceStatus,
) -> IntegrationStatusObservation {
    let subject = reservation.integration_proof_subject_revision();
    match reservation
        .retained_scoped_patch_target_verdicts()
        .lookup(subject, target)
    {
        ScopedPatchTargetVerdictAvailability::Hit {
            comparison,
            witness,
        } => IntegrationStatusObservation::new(
            integration_status_from_retained_scoped_patch_comparison(
                comparison,
                witness,
                target,
                previous_trunk,
                target_evidence_context.integration_reachability,
            ),
            EvidenceRevalidationObservation::Apply,
            ScopedPatchComparisonJournalUpdate::Unchanged,
        ),
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
                destination:      ScopedPatchComparisonDestination::Trunk,
            };
            let evidence_observation = reservation::observe_outstanding_integration_status(
                integration_reachability.for_ancestor(protected_tip.as_ref()),
                integration_reachability.for_ancestor(previous_trunk),
                target,
                || {
                    scoped_patch_evaluation_budget.evaluate(scoped_patch_evaluation_key, || {
                        evaluate_reservation_scoped_integration(
                            repository_root,
                            reservation,
                            protected_tip,
                            target,
                            integration_reachability,
                        )
                    })
                },
            );
            match evidence_observation {
                IntegrationEvidenceObservation::Reachability(status) => {
                    IntegrationStatusObservation::new(
                        status,
                        EvidenceRevalidationObservation::Apply,
                        ScopedPatchComparisonJournalUpdate::Unchanged,
                    )
                },
                IntegrationEvidenceObservation::ScopedPatchComparison { status, evaluation } => {
                    let scoped_patch_comparison =
                        scoped_patch_journal_update(subject, target, &status, &evaluation);
                    IntegrationStatusObservation::new(
                        status,
                        EvidenceRevalidationObservation::Apply,
                        scoped_patch_comparison,
                    )
                },
                // Nothing this pass ran judged the content, so that answer is never written down:
                // a journaled status reads as settled, and the next pass would report a proof lost
                // that its own comparison was about to restore. Every pass re-derives the same
                // answer until the budget admits the comparison and records what it found.
                IntegrationEvidenceObservation::ScopedPatchComparisonDeferred => {
                    IntegrationStatusObservation::new(
                        deferred_integration_status(
                            materialized,
                            previous_trunk,
                            integration_reachability,
                        ),
                        EvidenceRevalidationObservation::PreserveMaterialized,
                        ScopedPatchComparisonJournalUpdate::Unchanged,
                    )
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
    integration_reachability: &TargetIntegrationReachability,
) -> ScopedPatchIntegrationEvaluation {
    evaluate_historical_then_current_trunk(
        target,
        || {
            git::discover_historical_integration_candidate(
                repository_root,
                reservation.phase_start_head().as_ref(),
                reservation.scopes(),
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
    previous_trunk: &GitObjectId,
    integration_reachability: &TargetIntegrationReachability,
) -> IntegrationEvidenceStatus {
    match scoped_patch_comparison {
        DurableScopedPatchComparison::Equivalent => IntegrationEvidenceStatus::Integrated {
            trunk_oid: target.clone(),
            proof: IntegrationProof::ScopedPatchEquivalent,
            witness,
        },
        DurableScopedPatchComparison::Different => {
            unproven_integration_status(previous_trunk, integration_reachability)
        },
    }
}

/// What to report when the budget admitted no comparison, so this pass judged no content.
///
/// A status already on file that names no proving trunk was derived by an earlier pass from the
/// same ancestry answers this one holds, so repeating it verbatim keeps the board reading the same
/// across every deferred pass -- and keeps an `IntegrationEvidenceStatus::ObjectUnknown` saying
/// that git could not answer instead of degrading to a `NotIntegrated` nothing established. An
/// affirmative proof whose trunk left history is the one thing this pass did refute, so it gives
/// way to whatever ancestry alone now supports.
fn deferred_integration_status(
    materialized: IntegrationEvidenceStatus,
    previous_trunk: &GitObjectId,
    integration_reachability: &TargetIntegrationReachability,
) -> IntegrationEvidenceStatus {
    if proving_trunk(&materialized).is_none() {
        return materialized;
    }
    unproven_integration_status(previous_trunk, integration_reachability)
}

/// Separate work trunk never took from work a rewritten trunk dropped.
///
/// Both answers say no proof stands; they differ in whether the trunk this reservation was last
/// measured against is still in the observed trunk's history.
fn unproven_integration_status(
    previous_trunk: &GitObjectId,
    integration_reachability: &TargetIntegrationReachability,
) -> IntegrationEvidenceStatus {
    match integration_reachability.for_ancestor(previous_trunk) {
        Reachability::Ancestor => IntegrationEvidenceStatus::NotIntegrated,
        Reachability::NotAncestor => IntegrationEvidenceStatus::TrunkRewritten,
        Reachability::ObjectUnknown => IntegrationEvidenceStatus::ObjectUnknown,
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
    if proving_trunk(&materialized).is_none() && proving_trunk(evidence).is_none() {
        changes.confirmed_lost_evidence.push(reservation.id());
    }
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
    actual_trunk: &JudgedTargetTip,
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
        JudgedTargetTip::Resolved(actual),
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
    committed_by_holder: &HashMap<(WorktreeId, IntegrationTarget), CommittedMergeEvidence>,
    reconciliation: &mut ReconciliationPlan,
) -> Result<(), ReservationReplayError> {
    let mut settled = HashMap::new();
    for reservation in reservations.iter() {
        if reconciliation.action.cover_is_missing(reservation.id()) {
            continue;
        }
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
            reconciliation
                .action
                .repository_snapshot
                .target_for(reservation.id()),
            extent,
            committed_by_holder
                .get(&(
                    reservation.actor().worktree,
                    reconciliation
                        .action
                        .repository_snapshot
                        .recorded_target(reservation.id())
                        .clone(),
                ))
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
        if reconciliation.action.cover_is_missing(reservation.id()) {
            continue;
        }
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
        let did_work = matches!(
            reservation.source(),
            ClaimSource::Enrolled | ClaimSource::Cover { .. }
        ) || reservation.merge_extent().observed_unmerged_work()
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

/// End an `Active` run whose worktree git no longer registers and whose last completed observation
/// proved its branch had nothing left to integrate.
///
/// `Reservation::is_terminal` cannot reach such a reservation: it requires `!is_active()`, and
/// `ReservationLifecycle` leaves `Active` only at a checkpoint or a disposition, so an orphaned
/// `Active` holder has no transition available and its row stays on the board permanently. The
/// retirement is journaled rather than filtered during replay because its evidence does not exist
/// at replay time -- liveness, the trunk the key was taken against, and the working-tree
/// fingerprint all come from the repository observation this plan is built on.
///
/// Every condition is required and doubt retains, because the two errors are not symmetric: a
/// retained dead row costs a reader one puzzled look, while retiring a live holder stops
/// protecting that holder's work in every session at once. So `WorktreeLiveness::Orphaned` is the
/// only liveness accepted -- a locked registration reads `Unavailable`, a prunable one
/// `OrphanCandidate`, an unestablished identity `Unknown`, and each of those keeps the
/// reservation, since a worktree that cannot be read now is not a worktree that is gone. An
/// `Outstanding` orphan is kept as well: it checkpointed a commit, and
/// `alert::for_orphaned_outstanding` reports it with a recoverability verdict, so retiring it here
/// would end the reservation that alert exists to recover.
fn append_orphan_retirements(
    reservations: &RetainedReservationSet,
    reconciliation: &mut ReconciliationPlan,
) {
    let mut retired = Vec::new();
    for reservation in reservations.iter() {
        if !matches!(reservation.lifecycle(), ReservationLifecycle::Active) {
            continue;
        }
        let Ok(holder) = reconciliation
            .action
            .repository_snapshot
            .reservation(reservation.id())
        else {
            continue;
        };
        if holder.worktree_liveness != WorktreeLiveness::Orphaned {
            continue;
        }
        // `append_merged_run_endings` runs first and releases a holder whose work reached trunk.
        // That disposition records the integration this one cannot, so never write a second
        // ending over it.
        if planned_release(reservation.id(), &reconciliation.operations).is_some() {
            continue;
        }
        // Read the extent this pass will commit rather than the one replay produced: observation
        // fails for an orphaned holder, and `derive_merge_extents` has already planned the
        // `MergeExtent::Unavailable` that retains the proof.
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
        if extent
            .proved_empty_key()
            .is_some_and(MergeExtentKey::proves_nothing_outstanding)
        {
            retired.push(reservation.id());
        }
    }
    for reservation_id in retired {
        reconciliation.operations.push(JournalOperation::Release {
            reservation_id,
            disposition: ReleaseDisposition::RetiredOrphan(OrphanRetirementReason::derived()),
        });
    }
}

/// The disposition this plan already records for a reservation, if it ends one.
fn planned_release(
    reservation_id: ReservationId,
    operations: &[JournalOperation],
) -> Option<&ReleaseDisposition> {
    operations.iter().find_map(|operation| match operation {
        JournalOperation::Release {
            reservation_id: released,
            disposition,
        } if *released == reservation_id => Some(disposition),
        _ => None,
    })
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
            integration_proof,
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
            integration_proof,
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

/// Where one predecessor's grouped ancestry answers sit in the single batched query.
///
/// Positions rather than a fixed stride, because a predecessor holding no proof contributes no
/// reached-trunk query and the batch is ragged.
struct SubjectQueryPositions {
    incorporation_subject: usize,
    phase_start:           usize,
    reached_trunk:         Option<usize>,
}

/// Add one ancestor-against-heads question to the batch and report where its answer will land.
fn push_successor_head_query<'commits>(
    queries: &mut Vec<ProtectedTipSuccessorHeads<'commits>>,
    ancestor: &'commits GitObjectId,
    successor_heads: &'commits [GitObjectId],
) -> usize {
    queries.push(ProtectedTipSuccessorHeads::new(ancestor, successor_heads));
    queries.len() - 1
}

/// Decide every successor head reachability can settle, queueing the rest for one scoped
/// comparison apiece.
fn classify_successor_incorporation(
    repository_root: &Path,
    evidence_subjects: Vec<PredecessorSuccessorEvidenceSubject<'_>>,
) -> SuccessorIncorporationClassification {
    let mut subject_successor_heads = Vec::new();
    let mut query_positions = Vec::new();
    for subject in &evidence_subjects {
        let successor_heads = subject.successor_heads.as_slice();
        query_positions.push(SubjectQueryPositions {
            incorporation_subject: push_successor_head_query(
                &mut subject_successor_heads,
                subject.incorporation_subject.commit(),
                successor_heads,
            ),
            phase_start:           push_successor_head_query(
                &mut subject_successor_heads,
                subject.reservation.phase_start_head().as_ref(),
                successor_heads,
            ),
            reached_trunk:         subject.integration_proof.reached_trunk().map(|trunk_oid| {
                push_successor_head_query(&mut subject_successor_heads, trunk_oid, successor_heads)
            }),
        });
    }
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
    let mut by_predecessor = Vec::new();
    let mut pending_comparisons = Vec::new();
    for (evidence_subject, positions) in evidence_subjects.into_iter().zip(query_positions) {
        let predecessor_id = evidence_subject.reservation.id();
        let incorporation = predecessor_successor_incorporation(
            &evidence_subject,
            PredecessorSuccessorReachability {
                against_incorporation_subject: &subject_successor_classifications
                    [positions.incorporation_subject],
                since_phase_start:             &subject_successor_classifications
                    [positions.phase_start],
                against_reached_trunk:         positions
                    .reached_trunk
                    .map(|position| &subject_successor_classifications[position]),
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
        reachability.against_incorporation_subject
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
                unreached_successor_evidence(
                    &candidate_context,
                    reachability,
                    head,
                    pending_comparisons,
                ),
            ),
        };
        evidence_by_head.insert(head.clone(), evidence);
    }
    PredecessorSuccessorIncorporation::Classified(evidence_by_head)
}

/// Decide a successor head the predecessor's incorporation subject does not reach.
///
/// The predecessor's own proof is asked first: it names the trunk commit the protected work landed
/// as, and a successor whose history contains that commit contains the work. Nothing about the
/// successor's current content can unmake that, so this answer is as final as an ancestry proof and
/// costs no comparison -- the batched query already classified the commit. Reaching only for the
/// protected tip left the branch that merged trunk indistinguishable from the branch that ignored
/// it, because an amending push rewrites the tip away while the commit it landed as stays put, so
/// every pass re-ran a content comparison that could only answer `Different` as the successor
/// developed its own work on those paths -- a hold with no terminal condition.
///
/// Failing that, a retained verdict settles it outright; otherwise it joins the queue competing for
/// the one scoped comparison this reconciliation admits.
fn unreached_successor_evidence(
    candidate_context: &PendingScopedPatchCandidateContext<'_>,
    reachability: PredecessorSuccessorReachability<'_>,
    successor_head: &GitObjectId,
    pending_comparisons: &mut Vec<SuccessorScopedPatchEvaluationCandidate>,
) -> SuccessorIncorporationEvidence {
    if reachability.reached_trunk_is_ancestor(successor_head) {
        return SuccessorIncorporationEvidence::IntegratedTrunkAncestor;
    }
    let evidence_subject = candidate_context.evidence_subject;
    let SuccessorIncorporationSubject::CheckpointTip(protected_tip) =
        &evidence_subject.incorporation_subject
    else {
        return SuccessorIncorporationEvidence::NotIncorporated;
    };
    if evidence_subject.integration_proof.reached_trunk().is_none() {
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

/// Collect every alert the retained reservations raise against the observed repository.
fn reservation_alerts(
    reservations: &RetainedReservationSet,
    repository_snapshot: &RepositorySnapshot,
    confirmed_lost_evidence: &[ReservationId],
) -> Result<Vec<Alert>, ReconcileError> {
    let mut alerts = Vec::new();
    for reservation in reservations.iter() {
        // Only a directive two consecutive passes derived reaches a reader. See
        // `ReconciliationAction::confirmed_lost_evidence`.
        if confirmed_lost_evidence.contains(&reservation.id()) {
            alerts.extend(
                alert::for_lost_integration_evidence(
                    reservation,
                    repository_snapshot.target_for(reservation.id()),
                )
                .map_err(ReconcileError::Replay)?,
            );
        }
        // Released reservations no longer refresh merge extents, but their integration
        // evidence must still report when the proof for released work is lost.
        if matches!(
            reservation.lifecycle(),
            ReservationLifecycle::Released { .. }
        ) {
            continue;
        }
        let target = repository_snapshot.recorded_target(reservation.id());
        if target != repository_snapshot.repository_trunk_target()
            && matches!(
                repository_snapshot.targets().get(target),
                Some(TargetObservation::Missing)
            )
        {
            alerts.push(Alert::TargetMissing {
                reservation_id: reservation.id(),
                target:         target.clone(),
                commands:       vec![format!(
                    "cargo-berth retarget {} --target <branch>",
                    reservation.id()
                )],
            });
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
    Ok(alerts)
}

impl ReconciliationAction {
    fn finish_cover_markers(
        marker_contexts: &[WorktreeMarkerSweepContext],
        active_holders: &[ActiveHolder],
        cover_marker_publications: &[(WorktreeContext, CoordinationRunId)],
    ) -> Result<(), ReconcileError> {
        for marker_context in marker_contexts {
            marker_context.sweep_coordination_run_marker(|worktree_id, coordination_run_id| {
                active_holders.iter().any(|active_holder| {
                    active_holder.worktree_id == worktree_id
                        && active_holder.coordination_run_id == coordination_run_id
                })
            })?;
        }
        for (context, run) in cover_marker_publications {
            context.publish_coordination_run_marker(*run)?;
        }
        Ok(())
    }

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
        Self::finish_cover_markers(
            &self.marker_contexts,
            &self.active_holders,
            &self.cover_marker_publications,
        )?;
        let mut alerts = reservation_alerts(
            &reservations,
            &self.repository_snapshot,
            &self.confirmed_lost_evidence,
        )?;
        alerts.extend(
            self.uncovered_targets
                .into_iter()
                .map(|(target, waiting_reservations)| Alert::TargetUncovered {
                    target,
                    waiting_reservations,
                }),
        );
        for alert_subject in self.alert_subjects {
            alerts.extend(alert::for_orphaned_outstanding(
                &self.repository_root,
                reservations
                    .reservation(alert_subject.reservation_id)
                    .map_err(ReconcileError::Replay)?,
                alert_subject.worktree_liveness,
                OrphanIntegrationEvidence::from(
                    &self
                        .repository_snapshot
                        .reservation(alert_subject.reservation_id)
                        .map_err(ReconcileError::MissingReadinessFact)?
                        .evidence,
                ),
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
            covered_targets: self.covered_targets,
            cover_creation: self.cover_creation,
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
    Config(ConfigError),
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
    Config(ConfigError),
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
            Self::Transaction(error @ LedgerTransactionError::DerivedRecordTooLarge { .. }) => {
                OutputEnvelope::ledger_unreadable(command_verb, &error.to_string())
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
            ReconciliationPlanningError::Config(error) => Self::Config(error),
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
    use std::collections::BTreeMap;
    use std::process::Command;

    use serde_json::Value;
    use serde_json::json;

    use super::HistoricalIntegrationCandidateDiscovery;
    use super::HistoricalIntegrationCandidateDiscovery as Discovery;
    use super::ReconciliationScopedPatchEvaluationBudget;
    use super::RewriteTargetJudgment;
    use super::ScopedPatchComparisonDestination;
    use super::ScopedPatchEvaluationKey;
    use super::SettlementSelection;
    use super::rewrite_target_judgment;
    use super::rewrite_target_tip;
    use crate::edge::JudgedTargetTip;
    use crate::gate::permit::PendingBranchRewriteMarker;
    use crate::git::Reachability;
    use crate::git::ScopedPatchComparison;
    use crate::git::ScopedPatchComparison as Comparison;
    use crate::ids::GitObjectId;
    use crate::ids::ReservationId;
    use crate::ledger::IntegrationTarget;
    use crate::ledger::JournalEvent;
    use crate::ledger::ReservationJudgingBranch;
    use crate::output::CommandVerb;
    use crate::reservation::IntegrationEvidenceObservation;
    use crate::reservation::IntegrationEvidenceStatus;
    use crate::reservation::IntegrationProof;
    use crate::reservation::IntegrationWitness;
    use crate::reservation::MergeExtent;
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
    fn rewrite_target_tips_are_resolved_once_per_distinct_branch()
    -> Result<(), Box<dyn std::error::Error>> {
        let integration = IntegrationTarget::from_branch_argument("integration")?;
        let other = IntegrationTarget::from_branch_argument("other")?;
        let integration_tip = TRUNK.parse::<GitObjectId>()?;
        let other_tip = TIP.parse::<GitObjectId>()?;
        let mut judgments = BTreeMap::new();
        let mut existence_checks = 0;
        for _ in 0..2 {
            let judgment = rewrite_target_judgment(&mut judgments, &integration, |target| {
                existence_checks += 1;
                Ok::<_, ()>(target.clone())
            });
            assert!(
                matches!(judgment, RewriteTargetJudgment::Available(target) if target == &integration)
            );
        }
        let mut tips = BTreeMap::<_, Result<GitObjectId, ()>>::new();
        let mut calls = 0;
        assert_eq!(
            rewrite_target_tip(&mut tips, &integration, &mut calls, |_| {
                Ok(integration_tip.clone())
            }),
            &Ok(integration_tip.clone())
        );
        assert_eq!(
            rewrite_target_tip(&mut tips, &integration, &mut calls, |_| Err(())),
            &Ok(integration_tip)
        );
        assert_eq!(
            rewrite_target_tip(&mut tips, &other, &mut calls, |_| Ok(other_tip.clone())),
            &Ok(other_tip)
        );
        assert_eq!(calls, 2);
        assert_eq!(existence_checks, 1);
        let repository_trunk = IntegrationTarget::from_branch_argument("main")?;
        assert!(!tips.contains_key(&repository_trunk));
        Ok(())
    }

    #[test]
    fn missing_rewrite_target_resolves_the_trunk_tip_once() -> Result<(), Box<dyn std::error::Error>>
    {
        let repository = tempfile::tempdir()?;
        let initialized = Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(repository.path())
            .output()?;
        assert!(initialized.status.success());
        let integration = IntegrationTarget::from_branch_argument("integration")?;
        let repository_trunk = IntegrationTarget::from_branch_argument("main")?;
        let mut judgments = BTreeMap::new();
        let mut existence_checks = 0;
        let trunk_tip = TRUNK.parse::<GitObjectId>()?;
        let mut tips = BTreeMap::<_, Result<GitObjectId, ()>>::new();
        let mut calls = 0;
        for _ in 0..2 {
            let judgment = rewrite_target_judgment(&mut judgments, &integration, |target| {
                existence_checks += 1;
                target
                    .judging_branch(repository.path(), &repository_trunk)
                    .map(|judgment| {
                        assert!(matches!(
                            judgment,
                            ReservationJudgingBranch::MissingTargetFallback(_)
                        ));
                        judgment.target().clone()
                    })
            });
            let RewriteTargetJudgment::Available(judging_target) = judgment else {
                return Err(std::io::Error::other("missing target must use trunk").into());
            };
            assert_eq!(
                rewrite_target_tip(&mut tips, judging_target, &mut calls, |_| {
                    Ok(trunk_tip.clone())
                }),
                &Ok(trunk_tip.clone())
            );
        }
        assert_eq!(calls, 1);
        assert_eq!(existence_checks, 1);
        assert!(!tips.contains_key(&integration));
        Ok(())
    }

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
            destination:      ScopedPatchComparisonDestination::Trunk,
        };
        let mut budget = ReconciliationScopedPatchEvaluationBudget::default();
        let calls = RefCell::new(Vec::new());
        let observation = crate::reservation::observe_outstanding_integration_status(
            {
                calls.borrow_mut().push("ancestry");
                Reachability::NotAncestor
            },
            Reachability::Ancestor,
            &target,
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
    fn scoped_patch_budget_admits_one_subject_per_target() -> Result<(), Box<dyn std::error::Error>>
    {
        let first: GitObjectId = TRUNK.parse()?;
        let second: GitObjectId = TIP.parse()?;
        let key = |target: GitObjectId, tip: GitObjectId| ScopedPatchEvaluationKey {
            phase_start_head: first.clone(),
            protected_tip:    tip,
            target_trunk:     target,
            scopes:           Vec::new(),
            destination:      ScopedPatchComparisonDestination::Trunk,
        };
        let mut budget = ReconciliationScopedPatchEvaluationBudget::default();
        assert!(matches!(
            budget.evaluate(key(first.clone(), first.clone()), || Evaluation::Different),
            ScopedPatchComparisonObservation::Observed(Evaluation::Different)
        ));
        assert!(matches!(
            budget.evaluate(key(first.clone(), second.clone()), || Evaluation::Different),
            ScopedPatchComparisonObservation::Deferred
        ));
        assert!(matches!(
            budget.evaluate(key(second.clone(), second), || Evaluation::Different),
            ScopedPatchComparisonObservation::Observed(Evaluation::Different)
        ));
        assert_eq!(budget.evaluated_targets.len(), 2);
        Ok(())
    }

    #[test]
    fn settlement_selection() -> Result<(), Box<dyn std::error::Error>> {
        let reservation_id = RESERVATION_ID.parse::<ReservationId>()?;
        let [claim, checkpoint] = checkpoint_events()?;
        let actual_trunk = JudgedTargetTip::Resolved(TRUNK.parse()?);
        let other_trunk = JudgedTargetTip::Resolved(TIP.parse()?);
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
                    &JudgedTargetTip::Resolved(trunk.parse()?),
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
