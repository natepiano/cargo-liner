//! Reservation rows, the board model they compose, and the locked replay that assembles it.

use std::collections::HashMap;
use std::collections::HashSet;
use std::path::Path;

use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;

use super::alerts;
use super::alerts::AvailableForcedPermit;
use super::alerts::BoardAlert;
use super::alerts::BoardGitCost;
use super::alerts::BypassAuditEntry;
use super::alerts::OutstandingIncursion;
use super::alerts::RecordedIncursionAnswer;
use super::answers;
use super::answers::RecordedAnswer;
use super::error::BoardError;
use super::report::CompleteBoardReport;
use crate::answer::OverlapAuthorizationReason;
use crate::edge::EdgeDeclaration;
use crate::edge::EdgeHold;
use crate::edge::EdgeReadiness;
use crate::edge::IntegrationConstraintProjection;
use crate::edge::IntegrationDeferralStatus;
use crate::edge::OrderingReason;
use crate::edge::RepositoryReservationEvidence;
use crate::edge::RepositorySnapshot;
use crate::edge::RepositoryTrunk;
use crate::edge::UnintegratedPredecessorEvidence;
use crate::git;
use crate::git::AheadBehind;
use crate::ids::EdgeId;
use crate::ids::EventId;
use crate::ids::GitObjectId;
use crate::ids::JournalByteOffset;
use crate::ids::ProjectionGeneration;
use crate::ids::RecordedAt;
use crate::ids::ReservationId;
use crate::ids::WireOrderedReservationIds;
use crate::ids::WorktreeId;
use crate::ledger::CanonicalWorktreeRoot;
use crate::ledger::ClaimHeadSnapshot;
use crate::ledger::ClaimSource;
use crate::ledger::FullRefName;
use crate::ledger::IncursionIncidentId;
use crate::ledger::PendingBypassMarkerId;
use crate::ledger::ReservationPurpose;
use crate::presentation;
use crate::presentation::EmptyRenderedBlocks;
use crate::presentation::EnvelopePresentation;
use crate::presentation::NonEmptyRenderedBlocks;
use crate::presentation::RenderedOutputBlock;
use crate::reconcile::ReconciliationReport;
use crate::reservation::EditBlockingStatus;
use crate::reservation::IntegrationEvidenceStatus;
use crate::reservation::Reservation;
use crate::reservation::ReservationFreshness;
use crate::reservation::ReservationLifecycle;
use crate::reservation::RetainedReservationSet;
use crate::scope::ReservationScopeSet;
use crate::worktree::WorktreeHead;
use crate::worktree::WorktreeLiveness;

/// One complete, terminal-independent board assembled from a coherent locked replay.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
pub(crate) struct BoardModel {
    journal_position:                      BoardJournalPosition,
    recovered_bypasses_this_invocation:    RecoveredBypassesThisInvocation,
    integration_order:                     IntegrationOrderDeclaration,
    pub(super) ready_now:                  BoardSection<ReadyReservation>,
    waiting:                               BoardSection<WaitingConstraint>,
    settled_ordering_constraints:          BoardSection<SettledOrderingConstraint>,
    unresolved_overlaps:                   BoardSection<UnresolvedOverlap>,
    pub(super) recorded_overlap_answers:   BoardSection<RecordedAnswer>,
    pub(super) unconstrained_reservations: BoardSection<BoardReservationSnapshot>,
    pub(super) resolved:                   BoardSection<BoardReservationSnapshot>,
    available_forced_permits:              BoardSection<AvailableForcedPermit>,
    bypass_audit:                          BoardSection<BypassAuditEntry>,
    outstanding_incursions:                BoardSection<OutstandingIncursion>,
    recorded_incursion_answers:            BoardSection<RecordedIncursionAnswer>,
    alerts:                                BoardSection<BoardAlert>,
    git_cost:                              BoardGitCost,
}

/// Whether the complete board has retained facts beyond its journal position and read cost.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BoardReportContent {
    Empty,
    Populated,
}

impl<'board> From<&'board BoardModel> for CompleteBoardReport<'board> {
    fn from(board: &'board BoardModel) -> Self {
        Self {
            journal_position:                   &board.journal_position,
            recovered_bypasses_this_invocation: &board.recovered_bypasses_this_invocation,
            integration_order:                  &board.integration_order,
            ready_now:                          &board.ready_now,
            waiting:                            &board.waiting,
            settled_ordering_constraints:       &board.settled_ordering_constraints,
            unresolved_overlaps:                &board.unresolved_overlaps,
            recorded_overlap_answers:           &board.recorded_overlap_answers,
            unconstrained_reservations:         &board.unconstrained_reservations,
            resolved_reservations:              &board.resolved,
            available_forced_permits:           &board.available_forced_permits,
            bypass_audit:                       &board.bypass_audit,
            outstanding_incursions:             &board.outstanding_incursions,
            recorded_incursion_answers:         &board.recorded_incursion_answers,
            alerts:                             &board.alerts,
            git_cost:                           &board.git_cost,
        }
    }
}

/// Exclusive live-board membership for one drift-reported incursion incident.
pub(crate) enum LiveIncursionMembership {
    /// The incident remains outstanding and still requires feedback.
    Outstanding,
    /// A recorded answer resolved the incident before feedback rendering.
    Recorded,
    /// The board omitted the incident or represented it in both sections.
    Unverifiable,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
pub(super) struct BoardJournalPosition {
    generation:          ProjectionGeneration,
    journal_byte_offset: JournalByteOffset,
}

/// Pending bypass markers whose durable recovery completed during this board invocation.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(transparent)]
pub(super) struct RecoveredBypassesThisInvocation(Vec<PendingBypassMarkerId>);

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
pub(super) struct BoardSection<Entry> {
    journal_position:   BoardJournalPosition,
    pub(super) entries: Vec<Entry>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum IntegrationOrderDeclaration {
    Undeclared,
    ConstraintsRecorded,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
pub(super) struct BoardReservationSnapshot {
    pub(super) reservation_id: ReservationId,
    holder:                    ReservationHolder,
    source:                    ClaimSource,
    purpose:                   ReservationPurpose,
    scopes:                    ReservationScopeSet,
    lifecycle:                 ReservationLifecycle,
    integration_evidence:      BoardIntegrationEvidence,
    edit_blocking_status:      EditBlockingStatus,
    pub(super) visibility:     BoardReservationVisibility,
    pub(super) freshness:      ReservationFreshness,
    ahead_behind_main:         AheadBehind,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
struct ReservationHolder {
    worktree_id:   WorktreeId,
    worktree_root: CanonicalWorktreeRoot,
    branch:        HolderBranch,
    liveness:      WorktreeLiveness,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum HolderBranch {
    Attached { reference: FullRefName },
    Detached { head: GitObjectId },
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum BoardIntegrationEvidence {
    ActiveWork,
    Current { status: IntegrationEvidenceStatus },
    ReleasedWithoutCheckpoint,
}

/// Where one retained reservation belongs on the board.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum BoardReservationVisibility {
    /// Live or outstanding work still participates in active constraints.
    ActiveConstraint,
    /// A cleanly released reservation belongs only to retained audit history.
    ResolvedAudit,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
pub(super) struct ReadyReservation {
    relation:               ReadinessTie,
    pub(super) reservation: BoardReservationSnapshot,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum ReadinessTie {
    Unordered,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
pub(super) struct WaitingConstraint {
    edge_id:              EdgeId,
    predecessor:          ReservationId,
    successor:            ReservationId,
    scopes:               ReservationScopeSet,
    reason:               OrderingReason,
    action:               WaitingAction,
    provenance:           EdgeDeclaration,
    declaration_event_id: EventId,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub(super) enum WaitingAction {
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

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
pub(super) struct SettledOrderingConstraint {
    edge_id:              EdgeId,
    predecessor:          ReservationId,
    successor:            ReservationId,
    scopes:               ReservationScopeSet,
    reason:               OrderingReason,
    settlement:           EdgeSettlement,
    provenance:           EdgeDeclaration,
    declaration_event_id: EventId,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum EdgeSettlement {
    CancelledConstraintEnded,
    FulfilledSuccessorContainsPredecessor,
    SuccessorNoLongerActive,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
pub(super) struct UnresolvedOverlap {
    declaration_event_id: EventId,
    deferred:             ReservationId,
    blocker:              ReservationId,
    scopes:               ReservationScopeSet,
    reason:               OverlapAuthorizationReason,
    consequence:          SymmetricDeferralConsequence,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum SymmetricDeferralConsequence {
    BothIntegrationsHeldUntilSequence,
}

/// Declared ordering constraints split by whether they still hold a live successor.
struct DeclaredOrderingConstraints {
    waiting:                  Vec<WaitingConstraint>,
    settled:                  Vec<SettledOrderingConstraint>,
    /// Every reservation named by an ordering constraint. Callers that also
    /// place unresolved overlaps extend this with those endpoints.
    constrained_reservations: HashSet<ReservationId>,
}

/// The three mutually exclusive sections one board places its reservation rows into.
struct PlacedReservationSections {
    ready_now:                  Vec<ReadyReservation>,
    unconstrained_reservations: Vec<BoardReservationSnapshot>,
    resolved:                   Vec<BoardReservationSnapshot>,
}

impl BoardModel {
    /// Classify one incident across the board's mutually exclusive live sections.
    pub(crate) fn live_incursion_membership(
        &self,
        incident_id: IncursionIncidentId,
    ) -> LiveIncursionMembership {
        let is_outstanding = self
            .outstanding_incursions
            .entries
            .iter()
            .any(|incident| incident.incident_id == incident_id);
        let is_recorded = self
            .recorded_incursion_answers
            .entries
            .iter()
            .any(|incident| incident.incident_id == incident_id);
        match (is_outstanding, is_recorded) {
            (true, false) => LiveIncursionMembership::Outstanding,
            (false, true) => LiveIncursionMembership::Recorded,
            (false, false) | (true, true) => LiveIncursionMembership::Unverifiable,
        }
    }

    /// Render the complete board and every actionable notice without payload interpretation.
    pub(crate) fn envelope_presentation(&self) -> EnvelopePresentation {
        let mut blocks = self.actionable_notice_blocks();
        match self.report_content() {
            BoardReportContent::Empty => {},
            BoardReportContent::Populated => blocks.push(self.complete_report_block()),
        }
        match NonEmptyRenderedBlocks::try_from(blocks) {
            Ok(non_empty_rendered_blocks) => EnvelopePresentation::RenderedBlocks {
                blocks: non_empty_rendered_blocks,
            },
            Err(EmptyRenderedBlocks) => EnvelopePresentation::nothing_to_show(),
        }
    }

    fn actionable_notice_blocks(&self) -> Vec<RenderedOutputBlock> {
        let mut immediate_stop_details = self
            .outstanding_incursions
            .entries
            .iter()
            .map(alerts::outstanding_incursion_detail)
            .collect::<Vec<_>>();
        let actionable_notice_details = self
            .recovered_bypasses_this_invocation
            .0
            .iter()
            .map(PendingBypassMarkerId::file_name)
            .map(presentation::recovered_bypass_block)
            .chain(self.alerts.entries.iter().map(alerts::board_alert_detail))
            .collect::<Vec<_>>();
        if immediate_stop_details.is_empty() {
            return match actionable_notice_details.as_slice() {
                [] => Vec::new(),
                [_, ..] => vec![presentation::actionable_board_notices_block(
                    &actionable_notice_details,
                )],
            };
        }
        immediate_stop_details.extend(actionable_notice_details);
        vec![presentation::engine_message_block(
            "cargo-berth detected drift that requires an immediate stop.",
            &immediate_stop_details.join("\n"),
        )]
    }

    fn report_content(&self) -> BoardReportContent {
        if self.recovered_bypasses_this_invocation.0.is_empty()
            && self.integration_order == IntegrationOrderDeclaration::Undeclared
            && self.ready_now.entries.is_empty()
            && self.waiting.entries.is_empty()
            && self.settled_ordering_constraints.entries.is_empty()
            && self.unresolved_overlaps.entries.is_empty()
            && self.recorded_overlap_answers.entries.is_empty()
            && self.unconstrained_reservations.entries.is_empty()
            && self.resolved.entries.is_empty()
            && self.available_forced_permits.entries.is_empty()
            && self.bypass_audit.entries.is_empty()
            && self.outstanding_incursions.entries.is_empty()
            && self.recorded_incursion_answers.entries.is_empty()
            && self.alerts.entries.is_empty()
        {
            BoardReportContent::Empty
        } else {
            BoardReportContent::Populated
        }
    }

    fn complete_report_block(&self) -> RenderedOutputBlock {
        let complete_board_report = CompleteBoardReport::from(self);
        serde_json::to_string_pretty(&complete_board_report).map_or_else(
            |error| {
                presentation::engine_message_block(
                    "cargo-berth could not render the reservation board report.",
                    &format!("BOARD REPORT SERIALIZATION FAILED: {error}"),
                )
            },
            |detail| {
                presentation::engine_message_block(
                    "cargo-berth read the complete reservation board report.",
                    &detail,
                )
            },
        )
    }

    /// Project the reconciled repository observation and its exact locked journal replay.
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
        let (reservation_snapshots, ahead_behind_computations) = board_reservation_snapshots(
            repository_root,
            &reservations,
            &report.repository_snapshot,
            &observed_at,
        )?;
        let active_ids = reservation_snapshots
            .iter()
            .filter(|snapshot| snapshot.visibility != BoardReservationVisibility::ResolvedAudit)
            .map(|snapshot| snapshot.reservation_id)
            .collect::<HashSet<_>>();

        let DeclaredOrderingConstraints {
            waiting,
            settled,
            mut constrained_reservations,
        } = declared_ordering_constraints(&report.constraints, &active_ids);
        let unresolved_overlaps = unresolved_overlaps(&report.constraints);
        constrained_reservations.extend(
            unresolved_overlaps
                .iter()
                .flat_map(|overlap| [overlap.deferred, overlap.blocker]),
        );
        let PlacedReservationSections {
            ready_now,
            unconstrained_reservations,
            resolved,
        } = place_reservation_sections(
            &reservation_snapshots,
            &constrained_reservations,
            &waiting,
            &unresolved_overlaps,
        );
        let recorded_overlap_answers = answers::recorded_answers(events, &report.constraints)?;
        let available_forced_permits = alerts::available_forced_permits(events)?;
        let bypass_audit = alerts::bypass_audit(events);
        let (outstanding_incursions, recorded_incursion_answers) =
            alerts::incursion_sections(&reservations);
        let alerts = alerts::board_alerts(
            &report.alerts,
            &reservation_snapshots,
            &report.unrecorded_bypass_occurrences,
        )?;
        let git_cost = alerts::board_git_cost(
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
    pub(crate) fn reservation_ids(&self) -> WireOrderedReservationIds {
        let reservation_ids = self
            .ready_now
            .entries
            .iter()
            .map(|entry| entry.reservation.reservation_id)
            .chain(
                self.unconstrained_reservations
                    .entries
                    .iter()
                    .map(|snapshot| snapshot.reservation_id),
            )
            .chain(
                self.resolved
                    .entries
                    .iter()
                    .map(|snapshot| snapshot.reservation_id),
            )
            .chain(self.waiting.entries.iter().map(|entry| entry.successor))
            .chain(
                self.unresolved_overlaps
                    .entries
                    .iter()
                    .flat_map(|entry| [entry.deferred, entry.blocker]),
            )
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        WireOrderedReservationIds::sorted(reservation_ids)
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

fn declared_ordering_constraints(
    constraints: &IntegrationConstraintProjection,
    active_ids: &HashSet<ReservationId>,
) -> DeclaredOrderingConstraints {
    let mut waiting = Vec::new();
    let mut settled = Vec::new();
    let mut involved = HashSet::new();
    for edge in &constraints.ordering_constraints {
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

    DeclaredOrderingConstraints {
        waiting,
        settled,
        constrained_reservations: involved,
    }
}

fn unresolved_overlaps(constraints: &IntegrationConstraintProjection) -> Vec<UnresolvedOverlap> {
    constraints
        .deferrals
        .iter()
        .filter(|deferral| deferral.status == IntegrationDeferralStatus::Unresolved)
        .map(|deferral| UnresolvedOverlap {
            declaration_event_id: deferral.declaration_event_id,
            deferred:             deferral.deferred,
            blocker:              deferral.blocker,
            scopes:               deferral.scopes.clone(),
            reason:               deferral.reason.clone(),
            consequence:          SymmetricDeferralConsequence::BothIntegrationsHeldUntilSequence,
        })
        .collect()
}

fn place_reservation_sections(
    reservation_snapshots: &[BoardReservationSnapshot],
    involved: &HashSet<ReservationId>,
    waiting: &[WaitingConstraint],
    unresolved_overlaps: &[UnresolvedOverlap],
) -> PlacedReservationSections {
    let waiting_successors = waiting
        .iter()
        .map(|constraint| constraint.successor)
        .collect::<HashSet<_>>();
    let deferred_endpoints = unresolved_overlaps
        .iter()
        .flat_map(|overlap| [overlap.deferred, overlap.blocker])
        .collect::<HashSet<_>>();
    let ready_now = reservation_snapshots
        .iter()
        .filter(|snapshot| snapshot.visibility != BoardReservationVisibility::ResolvedAudit)
        .filter(|snapshot| involved.contains(&snapshot.reservation_id))
        .filter(|snapshot| !waiting_successors.contains(&snapshot.reservation_id))
        .filter(|snapshot| !deferred_endpoints.contains(&snapshot.reservation_id))
        .cloned()
        .map(|reservation| ReadyReservation {
            relation: ReadinessTie::Unordered,
            reservation,
        })
        .collect();
    let unconstrained_reservations = reservation_snapshots
        .iter()
        .filter(|snapshot| snapshot.visibility != BoardReservationVisibility::ResolvedAudit)
        .filter(|snapshot| !involved.contains(&snapshot.reservation_id))
        .cloned()
        .collect();
    let resolved = reservation_snapshots
        .iter()
        .filter(|snapshot| snapshot.visibility == BoardReservationVisibility::ResolvedAudit)
        .cloned()
        .collect();
    PlacedReservationSections {
        ready_now,
        unconstrained_reservations,
        resolved,
    }
}

fn board_reservation_snapshots(
    repository_root: &Path,
    reservations: &RetainedReservationSet,
    snapshot: &RepositorySnapshot,
    observed_at: &RecordedAt,
) -> Result<(Vec<BoardReservationSnapshot>, u64), BoardError> {
    let (ahead_by_worktree, ahead_behind_computations) =
        ahead_behind_by_worktree(repository_root, reservations, snapshot)?;
    let mut reservation_snapshots = Vec::new();
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
        reservation_snapshots.push(BoardReservationSnapshot {
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
    Ok((reservation_snapshots, ahead_behind_computations))
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

pub(super) fn waiting_action(hold: EdgeHold) -> WaitingAction {
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

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io;

    use super::BoardIntegrationEvidence;
    use super::BoardModel;
    use super::WaitingAction;
    use crate::answer::ConflictAuthorization;
    use crate::board::alerts::BypassAuditEntry;
    use crate::board::test_support;
    use crate::board::test_support::BoardFixture;
    use crate::board::test_support::FixtureResult;
    use crate::board::test_support::OrderedBoardFixture;
    use crate::config::Enrollment;
    use crate::ids::GitObjectId;
    use crate::ledger::JournalOperation;
    use crate::ledger::ReservationSnapshot;
    use crate::reconcile;
    use crate::reconcile::RecoveredBypassReporting;
    use crate::reservation::AbandonmentReason;
    use crate::reservation::IntegrationEvidenceStatus;
    use crate::reservation::IntegrationProof;
    use crate::reservation::OrphanRetirementReason;
    use crate::reservation::ProtectedReservationTip;
    use crate::reservation::ReleaseDisposition;
    use crate::reservation::ReservationLifecycle;
    use crate::reservation::RewrittenIntegrationTrunkCommit;

    const PENDING_BYPASS_NAME: &str =
        "cargo-berth-pending-bypass-01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a99.json";
    const UNKNOWN_OBJECT_ID: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    #[test]
    fn deferring_reconciliation_leaves_recovery_for_one_reporting_board() -> FixtureResult<()> {
        let fixture = BoardFixture::new()?;
        let marker_path = fixture
            .repository
            .path()
            .join(".git")
            .join(PENDING_BYPASS_NAME);
        fs::write(
            &marker_path,
            r#"{"cause":{"kind":"environment_override","bypassed_merge":"model-recovery"},"occurrence_time":{"status":"unavailable"}}
"#,
        )?;

        let deferred =
            match reconcile::reconcile(fixture.repository.path(), RecoveredBypassReporting::Defer)?
            {
                Enrollment::Enrolled(deferred) => deferred,
                Enrollment::Unconfigured { .. } => {
                    return Err("initialized board fixture is not enrolled".into());
                },
            };
        assert!(deferred.recovered_bypass_markers.is_empty());
        assert!(marker_path.exists());

        let recovered = fixture.model()?;
        assert_eq!(
            serde_json::to_value(&recovered.recovered_bypasses_this_invocation)?,
            serde_json::json!([PENDING_BYPASS_NAME])
        );
        assert!(matches!(
            recovered.bypass_audit.entries.as_slice(),
            [BypassAuditEntry::EnvironmentOverride { .. }]
        ));
        assert!(!marker_path.exists());

        let later_read = fixture.model()?;
        assert_eq!(
            serde_json::to_value(&later_read.recovered_bypasses_this_invocation)?,
            serde_json::json!([])
        );
        assert!(matches!(
            later_read.bypass_audit.entries.as_slice(),
            [BypassAuditEntry::EnvironmentOverride { .. }]
        ));
        assert!(!marker_path.exists());
        Ok(())
    }

    #[test]
    fn release_dispositions_remain_typed_in_resolved_rows() -> FixtureResult<()> {
        let fixture = BoardFixture::new()?;
        let actor = fixture.main_actor();
        let trunk = fixture.trunk()?;

        let integrated =
            fixture.claim(&actor, "integrated.rs", ConflictAuthorization::NoConflict)?;
        fixture.checkpoint(
            &actor,
            integrated.reservation_id,
            trunk.clone(),
            trunk.clone(),
        )?;
        fixture.record_evidence(
            &actor,
            integrated.reservation_id,
            IntegrationEvidenceStatus::Integrated {
                trunk_oid: trunk.clone(),
                proof:     IntegrationProof::ProtectedTipAncestor,
            },
        )?;
        fixture.release(
            &actor,
            integrated.reservation_id,
            ReleaseDisposition::Integrated,
        )?;

        let rewritten = fixture.claim(&actor, "rewritten.rs", ConflictAuthorization::NoConflict)?;
        fixture.checkpoint(
            &actor,
            rewritten.reservation_id,
            trunk.clone(),
            trunk.clone(),
        )?;
        fixture.record_evidence(
            &actor,
            rewritten.reservation_id,
            IntegrationEvidenceStatus::Integrated {
                trunk_oid: trunk.clone(),
                proof:     IntegrationProof::ProtectedTipAncestor,
            },
        )?;
        fixture.release(
            &actor,
            rewritten.reservation_id,
            ReleaseDisposition::RewrittenIntegration(RewrittenIntegrationTrunkCommit::from(trunk)),
        )?;

        let abandoned = fixture.claim(&actor, "abandoned.rs", ConflictAuthorization::NoConflict)?;
        fixture.release(
            &actor,
            abandoned.reservation_id,
            ReleaseDisposition::Abandoned("discarded deliberately".parse::<AbandonmentReason>()?),
        )?;

        let retired = fixture.claim(&actor, "retired.rs", ConflictAuthorization::NoConflict)?;
        fixture.release(
            &actor,
            retired.reservation_id,
            ReleaseDisposition::RetiredOrphan(
                "retired after review".parse::<OrphanRetirementReason>()?,
            ),
        )?;

        let model = fixture.model()?;
        assert!(matches!(
            &test_support::board_reservation_snapshot(&model, integrated.reservation_id)?.lifecycle,
            ReservationLifecycle::Released {
                disposition: ReleaseDisposition::Integrated,
            }
        ));
        assert!(matches!(
            &test_support::board_reservation_snapshot(&model, rewritten.reservation_id)?.lifecycle,
            ReservationLifecycle::Released {
                disposition: ReleaseDisposition::RewrittenIntegration(_),
            }
        ));
        assert!(matches!(
            &test_support::board_reservation_snapshot(&model, abandoned.reservation_id)?.lifecycle,
            ReservationLifecycle::Released {
                disposition: ReleaseDisposition::Abandoned(_),
            }
        ));
        assert!(matches!(
            &test_support::board_reservation_snapshot(&model, retired.reservation_id)?.lifecycle,
            ReservationLifecycle::Released {
                disposition: ReleaseDisposition::RetiredOrphan(_),
            }
        ));
        Ok(())
    }

    #[test]
    fn waiting_reasons_pair_typed_evidence_with_actions() -> FixtureResult<()> {
        assert_checkpoint_not_integrated_and_incorporation_actions()?;
        assert_trunk_rewritten_action()?;
        assert_object_unknown_action()?;
        Ok(())
    }

    fn assert_checkpoint_not_integrated_and_incorporation_actions() -> FixtureResult<()> {
        let initial = OrderedBoardFixture::new()?;
        let initial_model = initial.model()?;
        assert_waiting_endpoints(&initial_model, &initial);
        let WaitingAction::PredecessorCheckpoint { instruction } = waiting_action(&initial_model)?
        else {
            return Err(io::Error::other("active predecessor should require a checkpoint").into());
        };
        assert!(instruction.contains("nobody can act yet"));

        let protected_tip = initial.commit_predecessor()?;
        let checkpoint_trunk = initial.board.trunk()?;
        initial.board.checkpoint(
            &initial.predecessor_actor,
            initial.predecessor.reservation_id,
            protected_tip,
            checkpoint_trunk,
        )?;
        let not_integrated = initial.model()?;
        let WaitingAction::PredecessorNotIntegrated { instruction } =
            waiting_action(&not_integrated)?
        else {
            return Err(io::Error::other("unmerged predecessor should be not integrated").into());
        };
        assert!(instruction.contains("reach trunk"));
        assert!(matches!(
            &test_support::board_reservation_snapshot(
                &not_integrated,
                initial.predecessor.reservation_id
            )?
            .integration_evidence,
            BoardIntegrationEvidence::Current {
                status: IntegrationEvidenceStatus::NotIntegrated,
            }
        ));

        initial.merge_predecessor()?;
        let integrated = initial.model()?;
        let WaitingAction::SuccessorMustIncorporatePredecessor { instruction } =
            waiting_action(&integrated)?
        else {
            return Err(io::Error::other(
                "integrated predecessor should require the successor's own rebase",
            )
            .into());
        };
        assert!(instruction.contains("reader's own rebase"));
        assert!(matches!(
            &test_support::board_reservation_snapshot(
                &integrated,
                initial.predecessor.reservation_id
            )?
            .integration_evidence,
            BoardIntegrationEvidence::Current {
                status: IntegrationEvidenceStatus::Integrated { .. },
            }
        ));
        Ok(())
    }

    fn assert_trunk_rewritten_action() -> FixtureResult<()> {
        let rewritten = OrderedBoardFixture::new()?;
        let rewritten_tip = rewritten.board.trunk()?;
        rewritten.board.checkpoint(
            &rewritten.predecessor_actor,
            rewritten.predecessor.reservation_id,
            rewritten_tip.clone(),
            rewritten_tip.clone(),
        )?;
        rewritten.board.record_evidence(
            &rewritten.predecessor_actor,
            rewritten.predecessor.reservation_id,
            IntegrationEvidenceStatus::Integrated {
                trunk_oid: rewritten_tip,
                proof:     IntegrationProof::ProtectedTipAncestor,
            },
        )?;
        rewritten.board.release(
            &rewritten.predecessor_actor,
            rewritten.predecessor.reservation_id,
            ReleaseDisposition::Integrated,
        )?;
        rewritten.board.amend_trunk()?;
        let rewritten_model = rewritten.model()?;
        let WaitingAction::TrunkEvidenceRewritten {
            instruction,
            resolve_flag,
        } = waiting_action(&rewritten_model)?
        else {
            return Err(io::Error::other("rewritten trunk should require new evidence").into());
        };
        assert!(instruction.contains("trunk rewrite"));
        assert_eq!(resolve_flag, "resolve --integrated-as <trunk-oid>");
        assert!(matches!(
            &test_support::board_reservation_snapshot(
                &rewritten_model,
                rewritten.predecessor.reservation_id
            )?
            .integration_evidence,
            BoardIntegrationEvidence::Current {
                status: IntegrationEvidenceStatus::TrunkRewritten,
            }
        ));
        Ok(())
    }

    fn assert_object_unknown_action() -> FixtureResult<()> {
        let unknown = OrderedBoardFixture::new()?;
        let known_tip = unknown.board.trunk()?;
        unknown.board.checkpoint(
            &unknown.predecessor_actor,
            unknown.predecessor.reservation_id,
            known_tip.clone(),
            known_tip.clone(),
        )?;
        unknown.model()?;
        let unknown_tip = UNKNOWN_OBJECT_ID.parse::<GitObjectId>()?;
        unknown.board.append_as(
            &unknown.predecessor_actor,
            JournalOperation::Resnapshot {
                reservation_id: unknown.predecessor.reservation_id,
                snapshot:       ReservationSnapshot::Outstanding {
                    protected_tip: ProtectedReservationTip::from(unknown_tip),
                    trunk_oid:     known_tip,
                },
            },
        )?;
        let unknown_model = unknown.model()?;
        let WaitingAction::PredecessorObjectUnknown { instruction } =
            waiting_action(&unknown_model)?
        else {
            return Err(io::Error::other("missing predecessor object should be reported").into());
        };
        assert!(instruction.contains("does not resolve"));
        assert!(matches!(
            &test_support::board_reservation_snapshot(
                &unknown_model,
                unknown.predecessor.reservation_id
            )?
            .integration_evidence,
            BoardIntegrationEvidence::Current {
                status: IntegrationEvidenceStatus::ObjectUnknown,
            }
        ));
        Ok(())
    }

    fn waiting_action(model: &BoardModel) -> FixtureResult<&WaitingAction> {
        if model.waiting.entries.len() != 1 {
            return Err(io::Error::other("fixture should produce one waiting constraint").into());
        }
        Ok(&model.waiting.entries[0].action)
    }

    fn assert_waiting_endpoints(model: &BoardModel, fixture: &OrderedBoardFixture) {
        assert_eq!(model.waiting.entries.len(), 1);
        assert_eq!(
            model.waiting.entries[0].predecessor,
            fixture.predecessor.reservation_id
        );
        assert_eq!(
            model.waiting.entries[0].successor,
            fixture.successor.reservation_id
        );
    }
}
