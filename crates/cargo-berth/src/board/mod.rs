//! Headless reservation-board projection and its machine-readable sections.

pub(crate) mod tui;

use std::collections::HashMap;
use std::collections::HashSet;
use std::error::Error;
use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;
use std::path::Path;

use serde::Deserialize;
use serde::Serialize;

use crate::alert::Alert;
use crate::alert::BranchRefStatus;
use crate::alert::ObjectAvailability;
use crate::alert::RecoverabilityVerdict;
use crate::alert::RetentionRefStatus;
use crate::answer::AuthorizedOverlap;
use crate::answer::AuthorizedOverlapSet;
use crate::answer::ConflictAuthorization;
use crate::answer::OverlapAuthorizationReason;
use crate::edge::EdgeDeclaration;
use crate::edge::EdgeHold;
use crate::edge::EdgeReadiness;
use crate::edge::EdgeReplayError;
use crate::edge::IntegrationConstraintProjection;
use crate::edge::IntegrationDeferralStatus;
use crate::edge::MissingReadinessFact;
use crate::edge::OrderingReason;
use crate::edge::RepositoryReservationEvidence;
use crate::edge::RepositorySnapshot;
use crate::edge::RepositoryTrunk;
use crate::edge::UnintegratedPredecessorEvidence;
use crate::gate::permit;
use crate::gate::permit::ForcedIntegrationPermitReplayError;
use crate::git;
use crate::git::AheadBehind;
use crate::ids::EdgeId;
use crate::ids::EventId;
use crate::ids::ForcedIntegrationPermitId;
use crate::ids::GitObjectId;
use crate::ids::JournalByteOffset;
use crate::ids::ProjectionGeneration;
use crate::ids::RecordedAt;
use crate::ids::ReservationId;
use crate::ids::ReservationScopePath;
use crate::ids::WorktreeId;
use crate::ledger::BypassCause;
use crate::ledger::BypassOccurrenceTime;
use crate::ledger::BypassedMergeIdentity;
use crate::ledger::CanonicalWorktreeRoot;
use crate::ledger::ClaimHeadSnapshot;
use crate::ledger::ClaimSource;
use crate::ledger::ForcedIntegrationReason;
use crate::ledger::FullRefName;
use crate::ledger::IncursionIncidentId;
use crate::ledger::JournalEvent;
use crate::ledger::JournalOperation;
use crate::ledger::OrderingDirection;
use crate::ledger::PendingBypassMarkerId;
use crate::ledger::ReservationPurpose;
use crate::ledger::SkippedDeferral;
use crate::ledger::SkippedIntegrationHoldSet;
use crate::ledger::SkippedOrderingEdge;
use crate::ledger::WidenCause;
use crate::reconcile::ReconciliationGitCost;
use crate::reconcile::ReconciliationReport;
use crate::reservation::EditBlockingStatus;
use crate::reservation::IncursionIncidentStatus;
use crate::reservation::IntegrationEvidenceStatus;
use crate::reservation::ProtectedReservationTip;
use crate::reservation::ReleaseDisposition;
use crate::reservation::Reservation;
use crate::reservation::ReservationFreshness;
use crate::reservation::ReservationLifecycle;
use crate::reservation::ReservationReplayError;
use crate::reservation::RetainedReservationSet;
use crate::scope::ReservationScope;
use crate::scope::ReservationScopeSet;
use crate::worktree::WorktreeHead;
use crate::worktree::WorktreeLiveness;

/// One complete, terminal-independent board assembled from a coherent locked replay.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct BoardModel {
    journal_position:                   BoardJournalPosition,
    recovered_bypasses_this_invocation: RecoveredBypassesThisInvocation,
    integration_order:                  IntegrationOrderDeclaration,
    ready_now:                          BoardSection<ReadyReservation>,
    waiting:                            BoardSection<WaitingConstraint>,
    settled_ordering_constraints:       BoardSection<SettledOrderingConstraint>,
    unresolved_overlaps:                BoardSection<UnresolvedOverlap>,
    recorded_overlap_answers:           BoardSection<RecordedAnswer>,
    unconstrained_reservations:         BoardSection<ReservationRow>,
    resolved:                           BoardSection<ReservationRow>,
    available_forced_permits:           BoardSection<AvailableForcedPermit>,
    bypass_audit:                       BoardSection<BypassAuditEntry>,
    outstanding_incursions:             BoardSection<OutstandingIncursion>,
    recorded_incursion_answers:         BoardSection<RecordedIncursionAnswer>,
    alerts:                             BoardSection<BoardAlert>,
    git_cost:                           BoardGitCost,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct BoardJournalPosition {
    generation:          ProjectionGeneration,
    journal_byte_offset: JournalByteOffset,
}

/// Pending bypass markers whose durable recovery completed during this board invocation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(transparent)]
struct RecoveredBypassesThisInvocation(Vec<PendingBypassMarkerId>);

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct BoardSection<Entry> {
    journal_position: BoardJournalPosition,
    entries:          Vec<Entry>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum IntegrationOrderDeclaration {
    Undeclared,
    ConstraintsRecorded,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct ReservationRow {
    reservation_id:       ReservationId,
    holder:               ReservationHolder,
    source:               ClaimSource,
    purpose:              ReservationPurpose,
    scopes:               ReservationScopeSet,
    lifecycle:            ReservationLifecycle,
    integration_evidence: BoardIntegrationEvidence,
    edit_blocking_status: EditBlockingStatus,
    visibility:           BoardReservationVisibility,
    freshness:            ReservationFreshness,
    ahead_behind_main:    AheadBehind,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct ReservationHolder {
    worktree_id:   WorktreeId,
    worktree_root: CanonicalWorktreeRoot,
    branch:        HolderBranch,
    liveness:      WorktreeLiveness,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum HolderBranch {
    Attached { reference: FullRefName },
    Detached { head: GitObjectId },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum BoardIntegrationEvidence {
    ActiveWork,
    Current { status: IntegrationEvidenceStatus },
    ReleasedWithoutCheckpoint,
}

/// Where one retained reservation belongs on the board.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum BoardReservationVisibility {
    /// Live or outstanding work still participates in active constraints.
    ActiveConstraint,
    /// Reserved v1 wire value no longer produced for released reservations.
    ReblockedActiveConstraint,
    /// A cleanly released reservation belongs only to retained audit history.
    ResolvedAudit,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct ReadyReservation {
    relation:    ReadinessTie,
    reservation: ReservationRow,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum ReadinessTie {
    Unordered,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct WaitingConstraint {
    edge_id:              EdgeId,
    predecessor:          ReservationId,
    successor:            ReservationId,
    scopes:               ReservationScopeSet,
    reason:               OrderingReason,
    action:               WaitingAction,
    provenance:           EdgeDeclaration,
    declaration_event_id: EventId,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
enum WaitingAction {
    PredecessorCheckpoint {
        instruction: String,
    },
    PredecessorNotIntegrated {
        instruction: String,
    },
    TrunkEvidenceRewritten {
        instruction:  String,
        resolve_flag: String,
    },
    PredecessorObjectUnknown {
        instruction: String,
    },
    SuccessorMustIncorporatePredecessor {
        instruction: String,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct SettledOrderingConstraint {
    edge_id:              EdgeId,
    predecessor:          ReservationId,
    successor:            ReservationId,
    scopes:               ReservationScopeSet,
    reason:               OrderingReason,
    settlement:           EdgeSettlement,
    provenance:           EdgeDeclaration,
    declaration_event_id: EventId,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum EdgeSettlement {
    CancelledConstraintEnded,
    FulfilledSuccessorContainsPredecessor,
    SuccessorNoLongerActive,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct UnresolvedOverlap {
    declaration_event_id: EventId,
    deferred:             ReservationId,
    blocker:              ReservationId,
    scopes:               ReservationScopeSet,
    reason:               OverlapAuthorizationReason,
    consequence:          SymmetricDeferralConsequence,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum SymmetricDeferralConsequence {
    BothIntegrationsHeldUntilSequence,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "answer", rename_all = "snake_case")]
enum RecordedAnswer {
    Sequence {
        reservation_id:        ReservationId,
        blocker:               ReservationId,
        direction:             OrderingDirection,
        exact_approved_scopes: AuthorizedOverlapSet,
        authorization_reason:  OverlapAuthorizationReason,
        acquisition:           AnswerAcquisition,
        consequence:           OrderingConsequence,
    },
    Defer {
        reservation_id:        ReservationId,
        blocker:               ReservationId,
        exact_approved_scopes: AuthorizedOverlapSet,
        authorization_reason:  OverlapAuthorizationReason,
        acquisition:           AnswerAcquisition,
        consequence:           SymmetricDeferralConsequence,
    },
    Override {
        reservation_id:        ReservationId,
        blocker:               ReservationId,
        exact_approved_scopes: AuthorizedOverlapSet,
        authorization_reason:  OverlapAuthorizationReason,
        acquisition:           AnswerAcquisition,
        consequence:           OverrideConsequence,
    },
    OrderingCreatedFromDeferral {
        edge_id:               EdgeId,
        deferred:              ReservationId,
        blocker:               ReservationId,
        direction:             OrderingDirection,
        exact_approved_scopes: Vec<AuthorizedOverlap>,
        deferral_reasons:      Vec<OverlapAuthorizationReason>,
        ordering_reason:       OrderingReason,
        consequence:           OrderingConsequence,
    },
    ExistingAnswersCoverEveryOverlap {
        reservation_id:          ReservationId,
        exact_existing_bindings: AuthorizedOverlapSet,
        added_scopes:            Vec<ReservationScope>,
        cause:                   WidenCause,
        edit_blocking_status:    EditBlockingStatus,
        consequence:             RevalidationConsequence,
    },
    WidenWithoutForeignOverlap {
        reservation_id:       ReservationId,
        added_scopes:         Vec<ReservationScope>,
        cause:                WidenCause,
        edit_blocking_status: EditBlockingStatus,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "origin", rename_all = "snake_case")]
enum AnswerAcquisition {
    Claim,
    Widen {
        added_scopes:         Vec<ReservationScope>,
        cause:                WidenCause,
        edit_blocking_status: EditBlockingStatus,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
enum OrderingConsequence {
    Holding { action: WaitingAction },
    Cancelled,
    Fulfilled,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum OverrideConsequence {
    EditingAuthorizedWithoutIntegrationOrder,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum RevalidationConsequence {
    ExistingAnswersStillCoverWidenedScopesNoNewEdge,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct AvailableForcedPermit {
    permit_id:      ForcedIntegrationPermitId,
    reservation_id: ReservationId,
    reason:         ForcedIntegrationReason,
    skipped_holds:  SkippedIntegrationHoldSet,
    instruction:    String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum BypassAuditEntry {
    ForcedOrderingEdges {
        permit_id:     ForcedIntegrationPermitId,
        reason:        ForcedIntegrationReason,
        skipped_edges: Vec<SkippedOrderingEdge>,
        occurrence:    BoardBypassTime,
    },
    ForcedUnresolvedDeferrals {
        permit_id:         ForcedIntegrationPermitId,
        reason:            ForcedIntegrationReason,
        skipped_deferrals: Vec<SkippedDeferral>,
        occurrence:        BoardBypassTime,
    },
    ForcedEdgesAndDeferrals {
        permit_id:         ForcedIntegrationPermitId,
        reason:            ForcedIntegrationReason,
        skipped_edges:     Vec<SkippedOrderingEdge>,
        skipped_deferrals: Vec<SkippedDeferral>,
        occurrence:        BoardBypassTime,
    },
    EnvironmentOverride {
        override_name:                  String,
        occurrences:                    Vec<BoardBypassTime>,
        grouped_reference_transactions: u64,
        skipped_holds:                  UnrecordedSkippedHolds,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum BoardBypassTime {
    Known { at: RecordedAt },
    Unknown,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum UnrecordedSkippedHolds {
    OverridePrecededLedgerRead,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct OutstandingIncursion {
    incident_id:             IncursionIncidentId,
    straying_reservation_id: ReservationId,
    foreign_reservation_ids: Vec<ReservationId>,
    entered_paths:           Vec<ReservationScopePath>,
    /// How many incidents stand outstanding for the straying reservation, this one included.
    ///
    /// A notice naming one incident reads as though answering it ends the matter, and a
    /// backlog accumulated before the dedup landed stays invisible without this.
    outstanding_count:       usize,
    resolution:              IncursionResolutionAction,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct IncursionResolutionAction {
    reservation_id: ReservationId,
    incident_id:    IncursionIncidentId,
    flag:           String,
    /// The disposition that clears the reservation's whole outstanding set.
    every_flag:     String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct RecordedIncursionAnswer {
    incident_id:             IncursionIncidentId,
    straying_reservation_id: ReservationId,
    foreign_reservation_ids: Vec<ReservationId>,
    entered_paths:           Vec<ReservationScopePath>,
    resolution_event_id:     EventId,
    resolved_at:             RecordedAt,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum BoardAlert {
    OrphanedOutstanding {
        reservation_id:       ReservationId,
        protected_tip:        ProtectedReservationTip,
        branch:               BoardBranchRefStatus,
        object_availability:  ObjectAvailability,
        retention_ref:        BoardRetentionRefStatus,
        recoverability:       RecoverabilityVerdict,
        recovery_consequence: OrphanRecoveryConsequence,
        resolution:           OrphanResolutionAction,
    },
    StaleReservation {
        reservation_id: ReservationId,
        freshness:      ReservationFreshness,
        resolution:     StaleReservationResolutionAction,
    },
    UnrecordedBypasses {
        count:            u64,
        occurrence_times: Vec<BoardBypassTime>,
        instruction:      String,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum BoardBranchRefStatus {
    Present {
        reference: FullRefName,
        tip:       GitObjectId,
    },
    Missing {
        reference: FullRefName,
    },
    Detached,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(transparent)]
struct ReservationRetentionRef(String);

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum BoardRetentionRefStatus {
    Present {
        reference: ReservationRetentionRef,
    },
    Missing {
        reference: ReservationRetentionRef,
    },
    Mismatched {
        reference: ReservationRetentionRef,
        actual:    GitObjectId,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "action", rename_all = "snake_case")]
enum OrphanResolutionAction {
    Recover { flag: String },
    RetireOrAbandon { flags: Vec<String> },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "action", rename_all = "snake_case")]
enum StaleReservationResolutionAction {
    Renew { reservation_id: ReservationId },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum OrphanRecoveryConsequence {
    WorkRecoverable,
    CommitsLost,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct BoardGitCost {
    trunk_resolution_calls:                 u64,
    worktree_list_calls:                    u64,
    reservation_evidence_revalidations:     u64,
    protected_predecessor_ancestry_queries: u64,
    worktree_ahead_behind_computations:     u64,
    orphan_recovery_evidence_queries:       u64,
}

impl BoardModel {
    /// Project the reconciled repository observation and its exact locked journal replay.
    #[allow(
        clippy::too_many_lines,
        reason = "the constructor assigns each coherent board section from one locked replay"
    )]
    pub(crate) fn build(
        repository_root: &Path,
        report: &ReconciliationReport,
    ) -> Result<Self, BoardError> {
        let position = BoardJournalPosition {
            generation:          report.journal_snapshot.generation(),
            journal_byte_offset: report.journal_snapshot.journal_end_offset(),
        };
        let events = report.journal_snapshot.events();
        let reservations = RetainedReservationSet::replay(events)?;
        if report.constraints.generation != position.generation {
            return Err(BoardError::MismatchedProjectionGeneration {
                replay:      position.generation,
                constraints: report.constraints.generation,
            });
        }
        let observed_at = RecordedAt::now();
        let (rows, ahead_behind_computations) = reservation_rows(
            repository_root,
            &reservations,
            &report.repository_snapshot,
            &observed_at,
        )?;
        let active_ids = rows
            .iter()
            .filter(|row| row.visibility != BoardReservationVisibility::ResolvedAudit)
            .map(|row| row.reservation_id)
            .collect::<HashSet<_>>();

        let mut waiting = Vec::new();
        let mut settled = Vec::new();
        let mut involved = HashSet::new();
        for edge in &report.constraints.ordering_constraints {
            involved.insert(edge.predecessor);
            involved.insert(edge.successor);
            match edge.readiness {
                EdgeReadiness::Holding { hold } if active_ids.contains(&edge.successor) => {
                    waiting.push(WaitingConstraint {
                        edge_id:              edge.edge_id,
                        predecessor:          edge.predecessor,
                        successor:            edge.successor,
                        scopes:               edge.scopes.clone(),
                        reason:               edge.reason.clone(),
                        action:               waiting_action(hold),
                        provenance:           edge.declaration,
                        declaration_event_id: edge.declaration_event_id,
                    });
                },
                EdgeReadiness::Holding { .. } => settled.push(SettledOrderingConstraint {
                    edge_id:              edge.edge_id,
                    predecessor:          edge.predecessor,
                    successor:            edge.successor,
                    scopes:               edge.scopes.clone(),
                    reason:               edge.reason.clone(),
                    settlement:           EdgeSettlement::SuccessorNoLongerActive,
                    provenance:           edge.declaration,
                    declaration_event_id: edge.declaration_event_id,
                }),
                EdgeReadiness::Cancelled => settled.push(SettledOrderingConstraint {
                    edge_id:              edge.edge_id,
                    predecessor:          edge.predecessor,
                    successor:            edge.successor,
                    scopes:               edge.scopes.clone(),
                    reason:               edge.reason.clone(),
                    settlement:           EdgeSettlement::CancelledConstraintEnded,
                    provenance:           edge.declaration,
                    declaration_event_id: edge.declaration_event_id,
                }),
                EdgeReadiness::Fulfilled => settled.push(SettledOrderingConstraint {
                    edge_id:              edge.edge_id,
                    predecessor:          edge.predecessor,
                    successor:            edge.successor,
                    scopes:               edge.scopes.clone(),
                    reason:               edge.reason.clone(),
                    settlement:           EdgeSettlement::FulfilledSuccessorContainsPredecessor,
                    provenance:           edge.declaration,
                    declaration_event_id: edge.declaration_event_id,
                }),
            }
        }

        let unresolved_overlaps = report
            .constraints
            .deferrals
            .iter()
            .filter(|deferral| deferral.status == IntegrationDeferralStatus::Unresolved)
            .map(|deferral| {
                involved.insert(deferral.deferred);
                involved.insert(deferral.blocker);
                UnresolvedOverlap {
                    declaration_event_id: deferral.declaration_event_id,
                    deferred:             deferral.deferred,
                    blocker:              deferral.blocker,
                    scopes:               deferral.scopes.clone(),
                    reason:               deferral.reason.clone(),
                    consequence:
                        SymmetricDeferralConsequence::BothIntegrationsHeldUntilSequence,
                }
            })
            .collect::<Vec<_>>();
        let waiting_successors = waiting
            .iter()
            .map(|constraint| constraint.successor)
            .collect::<HashSet<_>>();
        let deferred_endpoints = unresolved_overlaps
            .iter()
            .flat_map(|overlap| [overlap.deferred, overlap.blocker])
            .collect::<HashSet<_>>();
        let ready_now = rows
            .iter()
            .filter(|row| row.visibility != BoardReservationVisibility::ResolvedAudit)
            .filter(|row| involved.contains(&row.reservation_id))
            .filter(|row| !waiting_successors.contains(&row.reservation_id))
            .filter(|row| !deferred_endpoints.contains(&row.reservation_id))
            .cloned()
            .map(|reservation| ReadyReservation {
                relation: ReadinessTie::Unordered,
                reservation,
            })
            .collect();
        let unconstrained_reservations = rows
            .iter()
            .filter(|row| row.visibility != BoardReservationVisibility::ResolvedAudit)
            .filter(|row| !involved.contains(&row.reservation_id))
            .cloned()
            .collect();
        let resolved = rows
            .iter()
            .filter(|row| row.visibility == BoardReservationVisibility::ResolvedAudit)
            .cloned()
            .collect();
        let recorded_overlap_answers = recorded_answers(events, &report.constraints)?;
        let available_forced_permits = available_forced_permits(events)?;
        let bypass_audit = bypass_audit(events);
        let (outstanding_incursions, recorded_incursion_answers) =
            incursion_sections(&reservations);
        let alerts = board_alerts(&report.alerts, &rows, &report.unrecorded_bypass_occurrences)?;
        let git_cost = board_git_cost(
            &reservations,
            &report.constraints,
            &report.repository_snapshot,
            ahead_behind_computations,
            &report.git_cost,
        );
        let integration_order = if report.constraints.ordering_constraints.is_empty() {
            IntegrationOrderDeclaration::Undeclared
        } else {
            IntegrationOrderDeclaration::ConstraintsRecorded
        };
        let recovered_bypasses_this_invocation = RecoveredBypassesThisInvocation(
            report
                .recovered_bypass_markers
                .iter()
                .map(|marker| marker.id().clone())
                .collect(),
        );
        Ok(Self {
            journal_position: position,
            recovered_bypasses_this_invocation,
            integration_order,
            ready_now: BoardSection::new(position, ready_now),
            waiting: BoardSection::new(position, waiting),
            settled_ordering_constraints: BoardSection::new(position, settled),
            unresolved_overlaps: BoardSection::new(position, unresolved_overlaps),
            recorded_overlap_answers: BoardSection::new(position, recorded_overlap_answers),
            unconstrained_reservations: BoardSection::new(position, unconstrained_reservations),
            resolved: BoardSection::new(position, resolved),
            available_forced_permits: BoardSection::new(position, available_forced_permits),
            bypass_audit: BoardSection::new(position, bypass_audit),
            outstanding_incursions: BoardSection::new(position, outstanding_incursions),
            recorded_incursion_answers: BoardSection::new(position, recorded_incursion_answers),
            alerts: BoardSection::new(position, alerts),
            git_cost,
        })
    }

    /// Return every retained reservation represented by this board.
    pub(crate) fn reservation_ids(&self) -> Vec<ReservationId> {
        let mut reservation_ids = self
            .ready_now
            .entries
            .iter()
            .map(|entry| entry.reservation.reservation_id)
            .chain(
                self.unconstrained_reservations
                    .entries
                    .iter()
                    .map(|row| row.reservation_id),
            )
            .chain(self.resolved.entries.iter().map(|row| row.reservation_id))
            .chain(self.waiting.entries.iter().map(|entry| entry.successor))
            .chain(
                self.unresolved_overlaps
                    .entries
                    .iter()
                    .flat_map(|entry| [entry.deferred, entry.blocker]),
            )
            .collect::<HashSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        reservation_ids.sort_by_key(ToString::to_string);
        reservation_ids
    }

    /// Borrow marker filenames claimed for one-time reporting by this board.
    pub(crate) fn recovered_bypass_marker_names(&self) -> impl Iterator<Item = &str> {
        self.recovered_bypasses_this_invocation
            .0
            .iter()
            .map(PendingBypassMarkerId::file_name)
    }
}

impl<Entry> BoardSection<Entry> {
    const fn new(journal_position: BoardJournalPosition, entries: Vec<Entry>) -> Self {
        Self {
            journal_position,
            entries,
        }
    }
}

fn reservation_rows(
    repository_root: &Path,
    reservations: &RetainedReservationSet,
    snapshot: &RepositorySnapshot,
    observed_at: &RecordedAt,
) -> Result<(Vec<ReservationRow>, u64), BoardError> {
    let (ahead_by_worktree, ahead_behind_computations) =
        ahead_behind_by_worktree(repository_root, reservations, snapshot)?;
    let mut rows = Vec::new();
    for reservation in reservations.iter() {
        let repository_reservation = snapshot.reservation(reservation.id())?;
        let ahead_behind_main = *ahead_by_worktree
            .get(&reservation.actor().worktree)
            .unwrap_or(&AheadBehind::Unavailable);
        let integration_evidence = match &repository_reservation.evidence {
            RepositoryReservationEvidence::Active => BoardIntegrationEvidence::ActiveWork,
            RepositoryReservationEvidence::Outstanding {
                integration_status, ..
            }
            | RepositoryReservationEvidence::Released {
                integration_status, ..
            } => BoardIntegrationEvidence::Current {
                status: integration_status.clone(),
            },
            RepositoryReservationEvidence::ReleasedWithoutCheckpoint { .. } => {
                BoardIntegrationEvidence::ReleasedWithoutCheckpoint
            },
        };
        let visibility = reservation_visibility(reservation);
        rows.push(ReservationRow {
            reservation_id: reservation.id(),
            holder: ReservationHolder {
                worktree_id:   reservation.actor().worktree,
                worktree_root: reservation.worktree_root().clone(),
                branch:        holder_branch(reservation.head_snapshot()),
                liveness:      repository_reservation.worktree_liveness,
            },
            source: reservation.source().clone(),
            purpose: reservation.purpose().clone(),
            scopes: reservation.scopes().clone(),
            lifecycle: reservation.lifecycle().clone(),
            integration_evidence,
            edit_blocking_status: reservation.edit_blocking_status(),
            visibility,
            freshness: reservation.freshness(observed_at),
            ahead_behind_main,
        });
    }
    Ok((rows, ahead_behind_computations))
}

fn ahead_behind_by_worktree(
    repository_root: &Path,
    reservations: &RetainedReservationSet,
    snapshot: &RepositorySnapshot,
) -> Result<(HashMap<WorktreeId, AheadBehind>, u64), BoardError> {
    let RepositoryTrunk::Resolved(trunk) = snapshot.trunk() else {
        return Ok((HashMap::new(), 0));
    };
    let mut head_by_worktree = HashMap::new();
    for reservation in reservations.iter() {
        let repository_reservation = snapshot.reservation(reservation.id())?;
        if let WorktreeHead::Resolved(head) = &repository_reservation.worktree_head {
            head_by_worktree
                .entry(reservation.actor().worktree)
                .or_insert_with(|| head.clone());
        }
    }
    let mut worktree_heads = head_by_worktree.into_iter().collect::<Vec<_>>();
    worktree_heads.sort_by_key(|(worktree_id, _)| worktree_id.to_string());
    let ahead_behind_computations = u64::try_from(
        worktree_heads
            .iter()
            .filter(|(_, worktree_head)| worktree_head != trunk)
            .count(),
    )
    .unwrap_or(u64::MAX);
    let heads = worktree_heads
        .iter()
        .map(|(_, worktree_head)| worktree_head.clone())
        .collect::<Vec<_>>();
    let ahead_behind = git::ahead_behind_for_heads(repository_root, trunk, &heads);
    Ok((
        worktree_heads
            .into_iter()
            .zip(ahead_behind)
            .map(|((worktree_id, _), ahead_behind)| (worktree_id, ahead_behind))
            .collect(),
        ahead_behind_computations,
    ))
}

const fn reservation_visibility(reservation: &Reservation) -> BoardReservationVisibility {
    match reservation.lifecycle() {
        ReservationLifecycle::Released { .. } => BoardReservationVisibility::ResolvedAudit,
        ReservationLifecycle::Active | ReservationLifecycle::Outstanding { .. } => {
            BoardReservationVisibility::ActiveConstraint
        },
    }
}

fn holder_branch(snapshot: &ClaimHeadSnapshot) -> HolderBranch {
    match snapshot {
        ClaimHeadSnapshot::Branch { full_ref, .. } => HolderBranch::Attached {
            reference: full_ref.clone(),
        },
        ClaimHeadSnapshot::Detached { head } => HolderBranch::Detached {
            head: head.as_ref().clone(),
        },
    }
}

fn waiting_action(hold: EdgeHold) -> WaitingAction {
    match hold {
        EdgeHold::AwaitingPredecessorCheckpoint => WaitingAction::PredecessorCheckpoint {
            instruction: "wait for the predecessor to reach a checkpoint; nobody can act yet"
                .to_owned(),
        },
        EdgeHold::PredecessorNotOnTrunk {
            evidence: UnintegratedPredecessorEvidence::NotIntegrated,
        } => WaitingAction::PredecessorNotIntegrated {
            instruction: "wait for the predecessor to reach trunk".to_owned(),
        },
        EdgeHold::PredecessorNotOnTrunk {
            evidence: UnintegratedPredecessorEvidence::TrunkRewritten,
        } => WaitingAction::TrunkEvidenceRewritten {
            instruction: "re-record evidence invalidated by the trunk rewrite".to_owned(),
            resolve_flag: "resolve --integrated-as <trunk-oid>".to_owned(),
        },
        EdgeHold::PredecessorNotOnTrunk {
            evidence: UnintegratedPredecessorEvidence::ObjectUnknown,
        } => WaitingAction::PredecessorObjectUnknown {
            instruction: "repair the predecessor object that does not resolve".to_owned(),
        },
        EdgeHold::AwaitingSuccessorIncorporation => {
            WaitingAction::SuccessorMustIncorporatePredecessor {
                instruction: "rebase this worktree onto current main; only the reader's own rebase clears this hold"
                    .to_owned(),
            }
        },
    }
}

fn ordering_consequence(readiness: EdgeReadiness) -> OrderingConsequence {
    match readiness {
        EdgeReadiness::Holding { hold } => OrderingConsequence::Holding {
            action: waiting_action(hold),
        },
        EdgeReadiness::Cancelled => OrderingConsequence::Cancelled,
        EdgeReadiness::Fulfilled => OrderingConsequence::Fulfilled,
    }
}

#[allow(
    clippy::too_many_lines,
    reason = "one append-order pass preserves each answer's durable acquisition context"
)]
fn recorded_answers(
    events: &[JournalEvent],
    constraints: &IntegrationConstraintProjection,
) -> Result<Vec<RecordedAnswer>, BoardError> {
    let resolved_pairs = events
        .iter()
        .filter_map(|event| match &event.operation {
            JournalOperation::ResolveDefer {
                deferred_reservation_id,
                blocker_reservation_id,
                ..
            } => Some((*deferred_reservation_id, *blocker_reservation_id)),
            _ => None,
        })
        .collect::<HashSet<_>>();
    let mut answers = Vec::new();
    for event in events {
        match &event.operation {
            JournalOperation::Claim {
                reservation_id,
                authorization,
                ..
            } => append_authorization_answer(
                &mut answers,
                *reservation_id,
                authorization,
                AnswerAcquisition::Claim,
                &resolved_pairs,
                constraints,
            )?,
            JournalOperation::Widen {
                reservation_id,
                added_scopes,
                cause,
                authorization,
                edit_blocking_status,
            } => {
                let acquisition = AnswerAcquisition::Widen {
                    added_scopes:         added_scopes.as_slice().to_vec(),
                    cause:                cause.clone(),
                    edit_blocking_status: *edit_blocking_status,
                };
                match authorization {
                    ConflictAuthorization::ExistingAnswersCoverEveryOverlap { overlaps } => {
                        answers.push(RecordedAnswer::ExistingAnswersCoverEveryOverlap {
                            reservation_id: *reservation_id,
                            exact_existing_bindings: overlaps.clone(),
                            added_scopes: added_scopes.as_slice().to_vec(),
                            cause: cause.clone(),
                            edit_blocking_status: *edit_blocking_status,
                            consequence: RevalidationConsequence::ExistingAnswersStillCoverWidenedScopesNoNewEdge,
                        });
                    },
                    ConflictAuthorization::NoConflict => {
                        answers.push(RecordedAnswer::WidenWithoutForeignOverlap {
                            reservation_id:       *reservation_id,
                            added_scopes:         added_scopes.as_slice().to_vec(),
                            cause:                cause.clone(),
                            edit_blocking_status: *edit_blocking_status,
                        });
                    },
                    _ => append_authorization_answer(
                        &mut answers,
                        *reservation_id,
                        authorization,
                        acquisition,
                        &resolved_pairs,
                        constraints,
                    )?,
                }
            },
            JournalOperation::ResolveDefer {
                deferred_reservation_id,
                blocker_reservation_id,
                edge_id,
                direction,
                reason,
            } => {
                let mut exact_approved_scopes = Vec::new();
                let mut deferral_reasons = Vec::new();
                for prior in events
                    .iter()
                    .take_while(|prior| prior.event_id() != event.event_id())
                {
                    let (requester, authorization) = match &prior.operation {
                        JournalOperation::Claim {
                            reservation_id,
                            authorization,
                            ..
                        }
                        | JournalOperation::Widen {
                            reservation_id,
                            authorization,
                            ..
                        } => (*reservation_id, authorization),
                        _ => continue,
                    };
                    if requester == *deferred_reservation_id
                        && let ConflictAuthorization::Defer {
                            overlaps,
                            blocker,
                            reason,
                        } = authorization
                        && blocker == blocker_reservation_id
                    {
                        exact_approved_scopes.extend(overlaps.as_slice().iter().cloned());
                        deferral_reasons.push(reason.clone());
                    }
                }
                let edge = constraints
                    .ordering_constraints
                    .iter()
                    .find(|edge| edge.edge_id == *edge_id)
                    .ok_or(BoardError::MissingOrderingEdge(*edge_id))?;
                answers.push(RecordedAnswer::OrderingCreatedFromDeferral {
                    edge_id: *edge_id,
                    deferred: *deferred_reservation_id,
                    blocker: *blocker_reservation_id,
                    direction: *direction,
                    exact_approved_scopes,
                    deferral_reasons,
                    ordering_reason: reason.clone(),
                    consequence: ordering_consequence(edge.readiness),
                });
            },
            _ => {},
        }
    }
    Ok(answers)
}

#[allow(
    clippy::too_many_arguments,
    reason = "the audit row requires the complete durable answer and its current consequence"
)]
fn append_authorization_answer(
    answers: &mut Vec<RecordedAnswer>,
    reservation_id: ReservationId,
    authorization: &ConflictAuthorization,
    acquisition: AnswerAcquisition,
    resolved_pairs: &HashSet<(ReservationId, ReservationId)>,
    constraints: &IntegrationConstraintProjection,
) -> Result<(), BoardError> {
    match authorization {
        ConflictAuthorization::Sequence {
            overlaps,
            blocker,
            direction,
            edge_id,
            reason,
        } => {
            let edge = constraints
                .ordering_constraints
                .iter()
                .find(|edge| edge.edge_id == *edge_id)
                .ok_or(BoardError::MissingOrderingEdge(*edge_id))?;
            answers.push(RecordedAnswer::Sequence {
                reservation_id,
                blocker: *blocker,
                direction: *direction,
                exact_approved_scopes: overlaps.clone(),
                authorization_reason: reason.clone(),
                acquisition,
                consequence: ordering_consequence(edge.readiness),
            });
        },
        ConflictAuthorization::Defer {
            overlaps,
            blocker,
            reason,
        } if !resolved_pairs.contains(&(reservation_id, *blocker)) => {
            answers.push(RecordedAnswer::Defer {
                reservation_id,
                blocker: *blocker,
                exact_approved_scopes: overlaps.clone(),
                authorization_reason: reason.clone(),
                acquisition,
                consequence: SymmetricDeferralConsequence::BothIntegrationsHeldUntilSequence,
            });
        },
        ConflictAuthorization::Override {
            overlaps,
            blocker,
            reason,
        } => answers.push(RecordedAnswer::Override {
            reservation_id,
            blocker: *blocker,
            exact_approved_scopes: overlaps.clone(),
            authorization_reason: reason.clone(),
            acquisition,
            consequence: OverrideConsequence::EditingAuthorizedWithoutIntegrationOrder,
        }),
        ConflictAuthorization::NoConflict
        | ConflictAuthorization::ExistingAnswersCoverEveryOverlap { .. }
        | ConflictAuthorization::Defer { .. } => {},
    }
    Ok(())
}

fn available_forced_permits(
    events: &[JournalEvent],
) -> Result<Vec<AvailableForcedPermit>, BoardError> {
    permit::available_forced_integration_permits(events)
        .map_err(BoardError::ForcedPermitReplay)
        .map(|permits| {
            permits
                .into_iter()
                .map(|permit| AvailableForcedPermit {
                    permit_id:      permit.permit_id,
                    reservation_id: permit.reservation_id,
                    reason:         permit.reason,
                    skipped_holds:  permit.skipped_holds,
                    instruction:    "retrying the integration will consume this permit".to_owned(),
                })
                .collect()
        })
}

fn bypass_audit(events: &[JournalEvent]) -> Vec<BypassAuditEntry> {
    let permits = events
        .iter()
        .filter_map(|event| match &event.operation {
            JournalOperation::ForcedIntegrationPermit {
                permit_id,
                skipped_holds,
                ..
            } => Some((*permit_id, skipped_holds.clone())),
            _ => None,
        })
        .collect::<HashMap<_, _>>();
    let mut forced_seen = HashSet::new();
    let mut environment_groups: HashMap<BypassedMergeIdentity, Vec<BoardBypassTime>> =
        HashMap::new();
    let mut audit = Vec::new();
    for event in events {
        let JournalOperation::Bypass {
            cause,
            occurrence_time,
            ..
        } = &event.operation
        else {
            continue;
        };
        let occurrence = bypass_time(event, occurrence_time);
        match cause {
            BypassCause::EnvironmentOverride { bypassed_merge } => {
                environment_groups
                    .entry(bypassed_merge.clone())
                    .or_default()
                    .push(occurrence);
            },
            BypassCause::ForcedIntegration { permit_id, reason }
                if forced_seen.insert(*permit_id) =>
            {
                let Some(skipped_holds) = permits.get(permit_id) else {
                    continue;
                };
                audit.push(match skipped_holds {
                    SkippedIntegrationHoldSet::OrderingEdges { edges } => {
                        BypassAuditEntry::ForcedOrderingEdges {
                            permit_id: *permit_id,
                            reason: reason.clone(),
                            skipped_edges: edges.clone(),
                            occurrence,
                        }
                    },
                    SkippedIntegrationHoldSet::Deferrals { deferrals } => {
                        BypassAuditEntry::ForcedUnresolvedDeferrals {
                            permit_id: *permit_id,
                            reason: reason.clone(),
                            skipped_deferrals: deferrals.clone(),
                            occurrence,
                        }
                    },
                    SkippedIntegrationHoldSet::OrderingEdgesAndDeferrals { edges, deferrals } => {
                        BypassAuditEntry::ForcedEdgesAndDeferrals {
                            permit_id: *permit_id,
                            reason: reason.clone(),
                            skipped_edges: edges.clone(),
                            skipped_deferrals: deferrals.clone(),
                            occurrence,
                        }
                    },
                });
            },
            BypassCause::ForcedIntegration { .. } => {},
        }
    }
    audit.extend(environment_groups.into_values().map(|occurrences| {
        BypassAuditEntry::EnvironmentOverride {
            override_name: "CARGO_BERTH_BYPASS=1".to_owned(),
            grouped_reference_transactions: u64::try_from(occurrences.len()).unwrap_or(u64::MAX),
            occurrences,
            skipped_holds: UnrecordedSkippedHolds::OverridePrecededLedgerRead,
        }
    }));
    audit
}

fn bypass_time(event: &JournalEvent, occurrence: &BypassOccurrenceTime) -> BoardBypassTime {
    match occurrence {
        BypassOccurrenceTime::EventRecordedAt => BoardBypassTime::Known {
            at: event.recorded_at().clone(),
        },
        BypassOccurrenceTime::Known { at } => BoardBypassTime::Known { at: at.clone() },
        BypassOccurrenceTime::Unavailable => BoardBypassTime::Unknown,
    }
}

fn incursion_sections(
    reservations: &RetainedReservationSet,
) -> (Vec<OutstandingIncursion>, Vec<RecordedIncursionAnswer>) {
    let mut outstanding = Vec::new();
    let mut recorded = Vec::new();
    let mut outstanding_counts: HashMap<ReservationId, usize> = HashMap::new();
    for incident in reservations.outstanding_incursion_incidents() {
        *outstanding_counts
            .entry(incident.reservation_id())
            .or_default() += 1;
    }
    for incident in reservations.incursion_incidents() {
        match incident.status() {
            IncursionIncidentStatus::Outstanding => outstanding.push(OutstandingIncursion {
                incident_id:             incident.id(),
                straying_reservation_id: incident.reservation_id(),
                foreign_reservation_ids: incident.foreign_reservation_ids().as_slice().to_vec(),
                entered_paths:           incident.paths().as_slice().to_vec(),
                outstanding_count:       outstanding_counts
                    .get(&incident.reservation_id())
                    .copied()
                    .unwrap_or(1),
                resolution:              IncursionResolutionAction {
                    reservation_id: incident.reservation_id(),
                    incident_id:    incident.id(),
                    flag:           format!(
                        "resolve {} --incursion {}",
                        incident.reservation_id(),
                        incident.id()
                    ),
                    every_flag:     format!(
                        "resolve {} --every-incursion",
                        incident.reservation_id()
                    ),
                },
            }),
            IncursionIncidentStatus::Resolved {
                resolution_event_id,
                resolved_at,
            } => recorded.push(RecordedIncursionAnswer {
                incident_id:             incident.id(),
                straying_reservation_id: incident.reservation_id(),
                foreign_reservation_ids: incident.foreign_reservation_ids().as_slice().to_vec(),
                entered_paths:           incident.paths().as_slice().to_vec(),
                resolution_event_id:     *resolution_event_id,
                resolved_at:             resolved_at.clone(),
            }),
        }
    }
    (outstanding, recorded)
}

fn board_alerts(
    alerts: &[Alert],
    rows: &[ReservationRow],
    unrecorded_bypasses: &[BypassOccurrenceTime],
) -> Result<Vec<BoardAlert>, BoardError> {
    let mut board_alerts = alerts
        .iter()
        .map(board_alert)
        .collect::<Result<Vec<_>, BoardError>>()?;
    board_alerts.extend(rows.iter().filter_map(|row| match &row.freshness {
        ReservationFreshness::Stale { .. }
            if row.visibility != BoardReservationVisibility::ResolvedAudit =>
        {
            Some(BoardAlert::StaleReservation {
                reservation_id: row.reservation_id,
                freshness:      row.freshness.clone(),
                resolution:     StaleReservationResolutionAction::Renew {
                    reservation_id: row.reservation_id,
                },
            })
        },
        ReservationFreshness::Fresh { .. } | ReservationFreshness::Stale { .. } => None,
    }));
    if !unrecorded_bypasses.is_empty() {
        board_alerts.push(BoardAlert::UnrecordedBypasses {
            count: u64::try_from(unrecorded_bypasses.len()).unwrap_or(u64::MAX),
            occurrence_times: unrecorded_bypasses
                .iter()
                .map(|occurrence| match occurrence {
                    BypassOccurrenceTime::Known { at } => BoardBypassTime::Known { at: at.clone() },
                    BypassOccurrenceTime::EventRecordedAt | BypassOccurrenceTime::Unavailable => {
                        BoardBypassTime::Unknown
                    },
                })
                .collect(),
            instruction: "restore journal write access; the pending marker remains until its audit event is durable"
                .to_owned(),
        });
    }
    Ok(board_alerts)
}

fn board_alert(alert: &Alert) -> Result<BoardAlert, BoardError> {
    match alert {
        Alert::OrphanedOutstanding(orphan) => {
            let recoverability = orphan.recoverability();
            Ok(BoardAlert::OrphanedOutstanding {
                reservation_id: orphan.reservation_id(),
                protected_tip: orphan.protected_tip().clone(),
                branch: board_branch_ref_status(orphan.branch_ref_status())?,
                object_availability: orphan.object_availability(),
                retention_ref: board_retention_ref_status(orphan.retention_ref_status()),
                recoverability,
                recovery_consequence: match recoverability {
                    RecoverabilityVerdict::RecoverableFromBranch
                    | RecoverabilityVerdict::RecoverableFromProtectedTip => {
                        OrphanRecoveryConsequence::WorkRecoverable
                    },
                    RecoverabilityVerdict::CommitUnavailable => {
                        OrphanRecoveryConsequence::CommitsLost
                    },
                },
                resolution: match recoverability {
                    RecoverabilityVerdict::RecoverableFromBranch
                    | RecoverabilityVerdict::RecoverableFromProtectedTip => {
                        OrphanResolutionAction::Recover {
                            flag: "resolve --recovered".to_owned(),
                        }
                    },
                    RecoverabilityVerdict::CommitUnavailable => {
                        OrphanResolutionAction::RetireOrAbandon {
                            flags: vec![
                                "resolve --retire-orphan --why <reason>".to_owned(),
                                "resolve --abandon --why <reason>".to_owned(),
                            ],
                        }
                    },
                },
            })
        },
    }
}

fn board_branch_ref_status(status: &BranchRefStatus) -> Result<BoardBranchRefStatus, BoardError> {
    match status {
        BranchRefStatus::Present { reference, tip } => Ok(BoardBranchRefStatus::Present {
            reference: reference
                .parse()
                .map_err(|_| BoardError::InvalidBranchReference(reference.clone()))?,
            tip:       tip.clone(),
        }),
        BranchRefStatus::Missing { reference } => Ok(BoardBranchRefStatus::Missing {
            reference: reference
                .parse()
                .map_err(|_| BoardError::InvalidBranchReference(reference.clone()))?,
        }),
        BranchRefStatus::Detached => Ok(BoardBranchRefStatus::Detached),
    }
}

fn board_retention_ref_status(status: &RetentionRefStatus) -> BoardRetentionRefStatus {
    match status {
        RetentionRefStatus::Present { reference } => BoardRetentionRefStatus::Present {
            reference: ReservationRetentionRef(reference.clone()),
        },
        RetentionRefStatus::Missing { reference } => BoardRetentionRefStatus::Missing {
            reference: ReservationRetentionRef(reference.clone()),
        },
        RetentionRefStatus::Mismatched { reference, actual } => {
            BoardRetentionRefStatus::Mismatched {
                reference: ReservationRetentionRef(reference.clone()),
                actual:    actual.clone(),
            }
        },
    }
}

fn board_git_cost(
    reservations: &RetainedReservationSet,
    constraints: &IntegrationConstraintProjection,
    snapshot: &RepositorySnapshot,
    ahead_behind_computations: u64,
    reconciliation_git_cost: &ReconciliationGitCost,
) -> BoardGitCost {
    let reservation_evidence_revalidations = reservations
        .iter()
        .filter(|reservation| {
            matches!(
                reservation.lifecycle(),
                ReservationLifecycle::Outstanding { .. }
                    | ReservationLifecycle::Released {
                        disposition: ReleaseDisposition::Integrated
                            | ReleaseDisposition::RewrittenIntegration(_),
                    }
            )
        })
        .count();
    let protected_predecessors = constraints
        .ordering_constraints
        .iter()
        .map(|constraint| constraint.predecessor)
        .collect::<HashSet<_>>();
    let protected_predecessor_ancestry_queries = protected_predecessors
        .iter()
        .filter(|predecessor_id| {
            snapshot
                .reservation(**predecessor_id)
                .is_ok_and(|reservation| {
                    matches!(
                        reservation.evidence,
                        RepositoryReservationEvidence::Outstanding { .. }
                            | RepositoryReservationEvidence::Released { .. }
                    ) && constraints.ordering_constraints.iter().any(|constraint| {
                        constraint.predecessor == **predecessor_id
                            && snapshot
                                .reservation(constraint.successor)
                                .is_ok_and(|reservation| {
                                    matches!(reservation.worktree_head, WorktreeHead::Resolved(_))
                                })
                    })
                })
        })
        .count();
    BoardGitCost {
        trunk_resolution_calls:                 reconciliation_git_cost.trunk_resolution_calls,
        worktree_list_calls:                    1,
        reservation_evidence_revalidations:     u64::try_from(reservation_evidence_revalidations)
            .unwrap_or(u64::MAX),
        protected_predecessor_ancestry_queries: u64::try_from(
            protected_predecessor_ancestry_queries,
        )
        .unwrap_or(u64::MAX),
        worktree_ahead_behind_computations:     ahead_behind_computations,
        orphan_recovery_evidence_queries:       reconciliation_git_cost
            .orphan_recovery_evidence_queries,
    }
}

/// A coherent board could not be derived from retained journal and repository facts.
#[derive(Debug)]
pub(crate) enum BoardError {
    /// Reservation replay failed.
    Reservation(ReservationReplayError),
    /// Ordering graph replay failed.
    Edge(EdgeReplayError),
    /// A repository observation omitted a required edge fact.
    MissingReadiness(MissingReadinessFact),
    /// A recorded answer named an edge absent from the replayed graph.
    MissingOrderingEdge(EdgeId),
    /// Forced-permit replay found inconsistent issue or consumption records.
    ForcedPermitReplay(ForcedIntegrationPermitReplayError),
    /// An orphan alert retained a branch reference that no longer satisfies its type.
    InvalidBranchReference(String),
    /// The projection and event replay did not describe the same committed generation.
    MismatchedProjectionGeneration {
        /// The generation carried by the retained event replay.
        replay:      ProjectionGeneration,
        /// The generation carried by the shared constraint projection.
        constraints: ProjectionGeneration,
    },
}

impl Display for BoardError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Reservation(error) => error.fmt(formatter),
            Self::Edge(error) => error.fmt(formatter),
            Self::MissingReadiness(error) => error.fmt(formatter),
            Self::MissingOrderingEdge(edge_id) => {
                write!(
                    formatter,
                    "recorded overlap answer names missing edge {edge_id}"
                )
            },
            Self::ForcedPermitReplay(error) => error.fmt(formatter),
            Self::InvalidBranchReference(reference) => {
                write!(
                    formatter,
                    "orphan alert retained invalid branch reference {reference}"
                )
            },
            Self::MismatchedProjectionGeneration {
                replay,
                constraints,
            } => write!(
                formatter,
                "board replay generation {replay} does not match constraint generation {constraints}"
            ),
        }
    }
}

impl Error for BoardError {}

impl From<ReservationReplayError> for BoardError {
    fn from(error: ReservationReplayError) -> Self { Self::Reservation(error) }
}

impl From<EdgeReplayError> for BoardError {
    fn from(error: EdgeReplayError) -> Self { Self::Edge(error) }
}

impl From<MissingReadinessFact> for BoardError {
    fn from(error: MissingReadinessFact) -> Self { Self::MissingReadiness(error) }
}

#[cfg(test)]
mod tests;
