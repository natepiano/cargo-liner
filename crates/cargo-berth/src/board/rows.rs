//! Reservation rows, the board model they compose, and the locked replay that assembles it.

use std::collections::HashMap;
use std::collections::HashSet;
use std::path::Path;

use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Map;
use serde_json::Value;

use super::alerts;
use super::alerts::AvailableForcedPermit;
use super::alerts::BoardAlert;
use super::alerts::BoardGitCost;
use super::alerts::BypassAuditEntry;
use super::alerts::OutstandingIncursion;
use super::alerts::RecordedIncursionAnswer;
use super::answers;
use super::answers::BoardOverlapAnswers;
use super::answers::RecordedOverlapAnswer;
use super::answers::ReleasedOverlapAnswerCount;
use super::error::BoardError;
use super::overview;
use crate::alert::AlertRouting;
use crate::answer::OverlapAuthorizationReason;
use crate::edge::DeferralOrigin;
use crate::edge::EdgeDeclaration;
use crate::edge::EdgeHold;
use crate::edge::EdgeOrderingTarget;
use crate::edge::EdgeReadiness;
use crate::edge::IntegrationConstraintProjection;
use crate::edge::IntegrationDeferralStatus;
use crate::edge::JudgedTargetTip;
use crate::edge::OrderingReason;
use crate::edge::RepositoryReservationEvidence;
use crate::edge::RepositorySnapshot;
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
use crate::ledger::IntegrationTarget;
use crate::ledger::PendingBypassMarkerId;
use crate::ledger::ReservationPurpose;
use crate::ledger::ReservationScopeSet;
use crate::output::EngineAnswerOccasion;
use crate::output::TargetView;
use crate::presentation;
use crate::presentation::EmptyRenderedBlocks;
use crate::presentation::EnvelopePresentation;
use crate::presentation::NonEmptyRenderedBlocks;
use crate::presentation::RenderedOutputBlock;
use crate::reconcile::ReconciliationReport;
use crate::reservation::EditBlockingStatus;
use crate::reservation::EffectiveMergeExtent;
use crate::reservation::IntegrationEvidenceStatus;
use crate::reservation::RaceExtent;
use crate::reservation::Reservation;
use crate::reservation::ReservationFreshness;
use crate::reservation::ReservationLifecycle;
use crate::reservation::RetainedReservationSet;
use crate::worktree::WorktreeHead;
use crate::worktree::WorktreeLiveness;

/// One complete, terminal-independent board assembled from a coherent locked replay.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
pub(crate) struct BoardModel {
    journal_position:                      BoardJournalPosition,
    recovered_bypasses_this_invocation:    RecoveredBypassesThisInvocation,
    integration_order:                     IntegrationOrderDeclaration,
    targets:                               Vec<BoardTarget>,
    pub(super) ready_now:                  BoardSection<ReadyReservation>,
    pub(super) waiting:                    BoardSection<WaitingEntry>,
    settled_ordering_constraints:          BoardSection<SettledOrderingConstraint>,
    pub(super) unresolved_overlaps:        BoardSection<UnresolvedOverlap>,
    pub(super) live_overlap_answers:       BoardSection<RecordedOverlapAnswer>,
    released_overlap_answer_count:         ReleasedOverlapAnswerCount,
    pub(super) unconstrained_reservations: BoardSection<BoardReservationSnapshot>,
    pub(super) resolved:                   BoardSection<BoardReservationSnapshot>,
    available_forced_permits:              BoardSection<AvailableForcedPermit>,
    bypass_audit:                          BoardSection<BypassAuditEntry>,
    outstanding_incursions:                BoardSection<OutstandingIncursion>,
    recorded_incursion_answers:            BoardSection<RecordedIncursionAnswer>,
    alerts:                                BoardSection<BoardAlert>,
    git_cost:                              BoardGitCost,
}

/// A recorded integration branch and its nonterminal reservations.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
pub(super) struct BoardTarget {
    #[serde(rename = "ref")]
    reference:    IntegrationTarget,
    commit:       String,
    reservations: Vec<ReservationId>,
}

/// What a board response's presentation states about the board beside its actionable notices.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BoardReportRendering {
    /// Nothing beyond the notices: the envelope's payload carries the complete board.
    PayloadAlone,
    /// The integration order an opening session may act on, and the commands that read the rest.
    ///
    /// Most of the board is resolved history, which a session that just opened has no use for,
    /// and the whole board can run to megabytes.
    SessionOverview,
}

impl BoardReportRendering {
    /// The rendering a response on `occasion` states.
    pub(crate) const fn for_occasion(occasion: EngineAnswerOccasion) -> Self {
        match occasion {
            EngineAnswerOccasion::OpeningSession => Self::SessionOverview,
            EngineAnswerOccasion::DirectInvocation | EngineAnswerOccasion::CompletedBashCall => {
                Self::PayloadAlone
            },
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
    target:                    TargetView,
    holder:                    ReservationHolder,
    source:                    ClaimSource,
    purpose:                   ReservationPurpose,
    scopes:                    ReservationScopeSet,
    /// The run's effective editing scope, independent of branch integration.
    race_extent:               RaceExtent,
    /// The live branch surface, its evidence retained on failure, or the record kept at release.
    merge_extent:              EffectiveMergeExtent,
    lifecycle:                 ReservationLifecycle,
    integration_evidence:      BoardIntegrationEvidence,
    edit_blocking_status:      EditBlockingStatus,
    pub(super) visibility:     BoardReservationVisibility,
    pub(super) freshness:      ReservationFreshness,
    ahead_behind_main:         AheadBehind,
}

/// Whether human board presentations show reservation targets.
#[derive(Clone, Copy)]
pub(super) enum HumanTargetVisibility {
    TrunkOnly,
    MultipleTargets,
}

impl HumanTargetVisibility {
    pub(super) const fn for_target_count(count: usize) -> Self {
        if count <= 1 {
            Self::TrunkOnly
        } else {
            Self::MultipleTargets
        }
    }

    pub(super) fn omit_json_targets(self, fields: &mut Map<String, Value>) {
        if !matches!(self, Self::TrunkOnly) {
            return;
        }
        fields.remove("targets");
        for section in [
            "ready_now",
            "waiting",
            "unconstrained_reservations",
            "resolved",
        ] {
            let Some(entries) = fields
                .get_mut(section)
                .and_then(|value| value.get_mut("entries"))
                .and_then(serde_json::Value::as_array_mut)
            else {
                continue;
            };
            for entry in entries {
                let reservation = if matches!(section, "ready_now" | "waiting") {
                    &mut entry["reservation"]
                } else {
                    entry
                };
                if let Some(reservation) = reservation.as_object_mut() {
                    reservation.remove("target");
                }
            }
        }
    }
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

/// One hold on a live reservation, carrying that reservation's row.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
pub(super) struct WaitingEntry {
    #[serde(flatten)]
    pub(super) hold:        WaitingHold,
    pub(super) reservation: BoardReservationSnapshot,
}

/// Why one live reservation cannot integrate yet.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(tag = "hold", rename_all = "snake_case")]
pub(super) enum WaitingHold {
    /// A declared ordering edge holds its successor, the reservation this entry carries.
    OrderingEdge {
        edge_id:              EdgeId,
        predecessor:          ReservationId,
        successor:            ReservationId,
        scopes:               ReservationScopeSet,
        reason:               OrderingReason,
        action:               WaitingAction,
        provenance:           EdgeDeclaration,
        declaration_event_id: EventId,
    },
    /// An unresolved overlap holds each live side, the reservation this entry carries, until
    /// `sequence` orders the pair.
    UnresolvedOverlap {
        declaration_event_id: EventId,
        deferred:             ReservationId,
        blocker:              ReservationId,
        action:               OverlapWaitingAction,
    },
}

/// What releases a hold that an unresolved overlap places.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub(super) enum OverlapWaitingAction {
    OverlapNotSequenced { instruction: String },
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
    declaration_event_id:   EventId,
    pub(super) deferred:    ReservationId,
    pub(super) blocker:     ReservationId,
    scopes:                 ReservationScopeSet,
    reason:                 OverlapAuthorizationReason,
    origin:                 DeferralOrigin,
    pub(super) consequence: DeferralConsequence,
}

/// Which integrations one unresolved deferral holds until `sequence` orders the pair.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
pub(super) enum DeferralConsequence {
    /// Neither side is released, so both integrations are held.
    #[serde(rename = "both_integrations_held_until_sequence")]
    Both,
    /// The blocker is released, so only the deferred side's integration is held.
    #[serde(rename = "deferred_integration_held_until_sequence")]
    DeferredOnly,
    /// The deferred side is released, so only the blocker's integration is held.
    #[serde(rename = "blocker_integration_held_until_sequence")]
    BlockerOnly,
}

impl DeferralConsequence {
    /// The integrations a deferral holds, or `None` once both of its sides are released.
    fn from_lifecycles(
        deferred: &ReservationLifecycle,
        blocker: &ReservationLifecycle,
    ) -> Option<Self> {
        if is_released(deferred) {
            (!is_released(blocker)).then_some(Self::BlockerOnly)
        } else {
            Some(Self::with_live_deferred([blocker]))
        }
    }

    /// The integrations held while the deferred side is live: the blockers' as well, until every
    /// blocker is released.
    pub(super) fn with_live_deferred<'lifecycle>(
        blockers: impl IntoIterator<Item = &'lifecycle ReservationLifecycle>,
    ) -> Self {
        if blockers.into_iter().all(is_released) {
            Self::DeferredOnly
        } else {
            Self::Both
        }
    }
}

const fn is_released(lifecycle: &ReservationLifecycle) -> bool {
    matches!(lifecycle, ReservationLifecycle::Released { .. })
}

/// Declared ordering constraints split by whether they still hold a live successor.
struct DeclaredOrderingConstraints {
    holds:                    Vec<ReservationHold>,
    settled:                  Vec<SettledOrderingConstraint>,
    /// Every reservation named by an ordering constraint.
    constrained_reservations: HashSet<ReservationId>,
}

/// One hold on a live reservation, before the board attaches that reservation's row.
struct ReservationHold {
    held: ReservationId,
    hold: WaitingHold,
}

/// The four mutually exclusive sections one board places its reservation rows into.
struct PlacedReservationSections {
    ready_now:                  Vec<ReadyReservation>,
    waiting:                    Vec<WaitingEntry>,
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

    /// Render the actionable notices `alert_routing` delivers, then the board as `rendering`
    /// states it.
    ///
    /// Only the notices are routed: the payload still carries every alert, so the board data a
    /// reader parses stays whole.
    pub(crate) fn envelope_presentation(
        &self,
        alert_routing: &AlertRouting,
        rendering: BoardReportRendering,
    ) -> EnvelopePresentation {
        let mut blocks = self.actionable_notice_blocks(alert_routing);
        match rendering {
            BoardReportRendering::PayloadAlone => {},
            BoardReportRendering::SessionOverview => {
                blocks.extend(overview::session_overview_block(self, alert_routing));
            },
        }
        match NonEmptyRenderedBlocks::try_from(blocks) {
            Ok(non_empty_rendered_blocks) => EnvelopePresentation::RenderedBlocks {
                blocks: non_empty_rendered_blocks,
            },
            Err(EmptyRenderedBlocks) => EnvelopePresentation::nothing_to_show(),
        }
    }

    fn actionable_notice_blocks(&self, alert_routing: &AlertRouting) -> Vec<RenderedOutputBlock> {
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
            .chain(
                self.alerts
                    .entries
                    .iter()
                    .filter(|alert| alert_routing.delivers(alert.reservation_ids()))
                    .map(alerts::board_alert_detail),
            )
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

    /// Project the reconciled repository observation and its exact locked journal replay.
    pub(crate) fn build(
        repository_root: &Path,
        report: &ReconciliationReport,
    ) -> Result<Self, BoardError> {
        let position = BoardJournalPosition {
            generation:          report.journal_snapshot.generation(),
            journal_byte_offset: report.journal_snapshot.journal_end_offset(),
        };
        let events = report.journal_snapshot.coordination_events();
        let reservations = report.journal_snapshot.reservations();
        if report.constraints.generation != position.generation {
            return Err(BoardError::MismatchedProjectionGeneration {
                replay:      position.generation,
                constraints: report.constraints.generation,
            });
        }
        let observed_at = RecordedAt::now();
        let (reservation_snapshots, ahead_behind_computations) = board_reservation_snapshots(
            repository_root,
            reservations,
            &report.repository_snapshot,
            &observed_at,
        )?;
        let active_ids = reservation_snapshots
            .iter()
            .filter(|snapshot| snapshot.visibility != BoardReservationVisibility::ResolvedAudit)
            .map(|snapshot| snapshot.reservation_id)
            .collect::<HashSet<_>>();

        let DeclaredOrderingConstraints {
            holds,
            settled,
            constrained_reservations,
        } = declared_ordering_constraints(&report.constraints, &active_ids);
        let unresolved_overlaps = unresolved_overlaps(&report.constraints)?;
        let PlacedReservationSections {
            ready_now,
            waiting,
            unconstrained_reservations,
            resolved,
        } = place_reservation_sections(
            &reservation_snapshots,
            &constrained_reservations,
            holds,
            &unresolved_overlaps,
        );
        let BoardOverlapAnswers {
            live: live_overlap_answers,
            released: released_overlap_answer_count,
        } = answers::board_overlap_answers(events, &report.constraints, &active_ids)?;
        let available_forced_permits = alerts::available_forced_permits(events)?;
        let bypass_audit = alerts::bypass_audit(events);
        let (outstanding_incursions, recorded_incursion_answers) =
            alerts::incursion_sections(reservations);
        let alerts = alerts::board_alerts(
            &report.alerts,
            &reservation_snapshots,
            &report.unrecorded_bypass_occurrences,
            |id| report.target_for(id).clone(),
        )?;
        let targets = board_targets(reservations, &report.repository_snapshot);
        let git_cost = alerts::board_git_cost(
            reservations,
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
            targets,
            ready_now: BoardSection::new(position, ready_now),
            waiting: BoardSection::new(position, waiting),
            settled_ordering_constraints: BoardSection::new(position, settled),
            unresolved_overlaps: BoardSection::new(position, unresolved_overlaps),
            live_overlap_answers: BoardSection::new(position, live_overlap_answers),
            released_overlap_answer_count,
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
            .chain(
                self.waiting
                    .entries
                    .iter()
                    .map(|entry| entry.reservation.reservation_id),
            )
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
    let mut holds = Vec::new();
    let mut settled = Vec::new();
    let mut involved = HashSet::new();
    for edge in &constraints.ordering_constraints {
        involved.insert(edge.predecessor);
        involved.insert(edge.successor);
        match edge.readiness {
            EdgeReadiness::Holding { hold } if active_ids.contains(&edge.successor) => {
                holds.push(ReservationHold {
                    held: edge.successor,
                    hold: WaitingHold::OrderingEdge {
                        edge_id:              edge.edge_id,
                        predecessor:          edge.predecessor,
                        successor:            edge.successor,
                        scopes:               edge.scopes.clone(),
                        reason:               edge.reason.clone(),
                        action:               waiting_action(hold, &edge.ordering_target),
                        provenance:           edge.declaration,
                        declaration_event_id: edge.declaration_event_id,
                    },
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
        holds,
        settled,
        constrained_reservations: involved,
    }
}

/// Every unresolved deferral that still holds an integration.
///
/// The filter reads lifecycles, not board visibility: an outstanding reservation with an empty
/// merge extent is resolved audit on the board, yet the gate still holds it.
fn unresolved_overlaps(
    constraints: &IntegrationConstraintProjection,
) -> Result<Vec<UnresolvedOverlap>, BoardError> {
    let mut overlaps = Vec::new();
    for deferral in constraints
        .deferrals
        .iter()
        .filter(|deferral| deferral.status == IntegrationDeferralStatus::Unresolved)
    {
        let deferred = &constraints.reservation(deferral.deferred)?.lifecycle;
        let blocker = &constraints.reservation(deferral.blocker)?.lifecycle;
        // The gate skips released reservations, so a pair whose sides both ended holds nothing.
        let Some(consequence) = DeferralConsequence::from_lifecycles(deferred, blocker) else {
            continue;
        };
        overlaps.push(UnresolvedOverlap {
            declaration_event_id: deferral.declaration_event_id,
            deferred: deferral.deferred,
            blocker: deferral.blocker,
            scopes: deferral.scopes.clone(),
            reason: deferral.reason.clone(),
            origin: deferral.origin,
            consequence,
        });
    }
    Ok(overlaps)
}

/// Place each reservation row in exactly one section: resolved audit, one waiting entry per hold,
/// ready as an ordering-edge endpoint, or unconstrained.
fn place_reservation_sections(
    reservation_snapshots: &[BoardReservationSnapshot],
    ordering_endpoints: &HashSet<ReservationId>,
    ordering_holds: Vec<ReservationHold>,
    unresolved_overlaps: &[UnresolvedOverlap],
) -> PlacedReservationSections {
    let active_rows = reservation_snapshots
        .iter()
        .filter(|snapshot| snapshot.visibility != BoardReservationVisibility::ResolvedAudit)
        .map(|snapshot| (snapshot.reservation_id, snapshot))
        .collect::<HashMap<_, _>>();
    let overlap_holds = unresolved_overlaps.iter().flat_map(|overlap| {
        [overlap.deferred, overlap.blocker]
            .into_iter()
            .map(|held| ReservationHold {
                held,
                hold: overlap_hold(overlap),
            })
    });
    let waiting = ordering_holds
        .into_iter()
        .chain(overlap_holds)
        .filter_map(|ReservationHold { held, hold }| {
            active_rows.get(&held).map(|row| WaitingEntry {
                hold,
                reservation: (*row).clone(),
            })
        })
        .collect::<Vec<_>>();
    let held = waiting
        .iter()
        .map(|entry| entry.reservation.reservation_id)
        .collect::<HashSet<_>>();
    let (ready_now, unconstrained_reservations): (Vec<_>, Vec<_>) = reservation_snapshots
        .iter()
        .filter(|snapshot| snapshot.visibility != BoardReservationVisibility::ResolvedAudit)
        .filter(|snapshot| !held.contains(&snapshot.reservation_id))
        .cloned()
        .partition(|snapshot| ordering_endpoints.contains(&snapshot.reservation_id));
    let ready_now = ready_now
        .into_iter()
        .map(|reservation| ReadyReservation {
            relation: ReadinessTie::Unordered,
            reservation,
        })
        .collect();
    let resolved = reservation_snapshots
        .iter()
        .filter(|snapshot| snapshot.visibility == BoardReservationVisibility::ResolvedAudit)
        .cloned()
        .collect();
    PlacedReservationSections {
        ready_now,
        waiting,
        unconstrained_reservations,
        resolved,
    }
}

fn overlap_hold(overlap: &UnresolvedOverlap) -> WaitingHold {
    WaitingHold::UnresolvedOverlap {
        declaration_event_id: overlap.declaration_event_id,
        deferred:             overlap.deferred,
        blocker:              overlap.blocker,
        action:               OverlapWaitingAction::OverlapNotSequenced {
            instruction: format!(
                "order this pair: cargo berth sequence <first> <then> --why '<reason>', naming {} and {} in the order they must integrate",
                overlap.deferred, overlap.blocker
            ),
        },
    }
}

fn board_targets(
    reservations: &RetainedReservationSet,
    snapshot: &RepositorySnapshot,
) -> Vec<BoardTarget> {
    snapshot
        .targets()
        .iter()
        .map(|(reference, observation)| BoardTarget {
            reference:    reference.clone(),
            commit:       observation.commit(),
            reservations: reservations
                .iter()
                .filter(|reservation| {
                    !matches!(
                        reservation.lifecycle(),
                        ReservationLifecycle::Released { .. }
                    ) && snapshot.recorded_target(reservation.id()) == reference
                })
                .map(Reservation::id)
                .collect(),
        })
        .collect()
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
            target: TargetView::from_recorded(
                reservation.target(),
                snapshot.repository_trunk_target(),
                reservation.comparison_snapshot(),
            )
            .with_observed_commit(
                snapshot
                    .targets()
                    .get(snapshot.recorded_target(reservation.id()))
                    .map_or_else(
                        || "unresolved".to_owned(),
                        crate::edge::TargetObservation::commit,
                    ),
            ),
            holder: ReservationHolder {
                worktree_id:   reservation.actor().worktree,
                worktree_root: reservation.worktree_root().clone(),
                branch:        holder_branch(reservation.head_snapshot()),
                liveness:      repository_reservation.worktree_liveness,
            },
            source: reservation.source().clone(),
            purpose: reservation.purpose().clone(),
            scopes: reservation.scopes().clone(),
            race_extent: reservation.race_extent(),
            merge_extent: reservation.effective_merge_extent(),
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
    let JudgedTargetTip::Resolved(trunk) = snapshot.repository_trunk() else {
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
    if reservation.is_terminal() {
        BoardReservationVisibility::ResolvedAudit
    } else {
        BoardReservationVisibility::ActiveConstraint
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

pub(super) fn waiting_action(
    hold: EdgeHold,
    ordering_target: &EdgeOrderingTarget,
) -> WaitingAction {
    match hold {
        EdgeHold::AwaitingPredecessorCheckpoint => WaitingAction::PredecessorCheckpoint {
            instruction: "wait for the predecessor to reach a checkpoint; nobody can act yet"
                .to_owned(),
        },
        EdgeHold::PredecessorNotOnOrderingTarget {
            evidence: UnintegratedPredecessorEvidence::NotIntegrated,
        } => WaitingAction::PredecessorNotIntegrated {
            instruction: format!(
                "wait for the predecessor to reach {}",
                ordering_target.wait_name()
            ),
        },
        EdgeHold::PredecessorNotOnOrderingTarget {
            evidence: UnintegratedPredecessorEvidence::TrunkRewritten,
        } => WaitingAction::TrunkEvidenceRewritten {
            instruction:  format!(
                "re-record evidence invalidated by a rewrite of {}",
                ordering_target.wait_name()
            ),
            resolve_flag: "resolve --integrated-as <target-oid>".to_owned(),
        },
        EdgeHold::PredecessorNotOnOrderingTarget {
            evidence: UnintegratedPredecessorEvidence::ObjectUnknown,
        } => WaitingAction::PredecessorObjectUnknown {
            instruction: "repair the predecessor object that does not resolve".to_owned(),
        },
        EdgeHold::AwaitingSuccessorIncorporation => {
            WaitingAction::SuccessorMustIncorporatePredecessor {
                instruction: format!(
                    "rebase this worktree onto current {}; only the reader's own rebase clears this hold",
                    ordering_target.branch_name()
                ),
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::fs;
    use std::io;

    use super::BoardIntegrationEvidence;
    use super::BoardModel;
    use super::DeferralConsequence;
    use super::WaitingAction;
    use super::WaitingEntry;
    use super::WaitingHold;
    use crate::answer::AuthorizedOverlap;
    use crate::answer::ConflictAuthorization;
    use crate::answer::OverlapScopeRevision;
    use crate::board::alerts::BypassAuditEntry;
    use crate::board::answers::RecordedOverlapAnswer;
    use crate::board::test_support;
    use crate::board::test_support::BoardFixture;
    use crate::board::test_support::FixtureResult;
    use crate::board::test_support::OrderedBoardFixture;
    use crate::board::test_support::OverlapAnswerFixture;
    use crate::board::test_support::TestActor;
    use crate::config::Enrollment;
    use crate::edge::DeferralOrigin;
    use crate::ids::GitObjectId;
    use crate::ids::ReservationId;
    use crate::ledger::JournalOperation;
    use crate::ledger::ReservationScope;
    use crate::ledger::ReservationScopeSet;
    use crate::ledger::ReservationSnapshot;
    use crate::ledger::ScopeKind;
    use crate::reconcile;
    use crate::reconcile::RecoveredBypassReporting;
    use crate::reservation::AbandonmentReason;
    use crate::reservation::IntegrationEvidenceStatus;
    use crate::reservation::IntegrationProof;
    use crate::reservation::IntegrationWitness;
    use crate::reservation::OrphanRetirementReason;
    use crate::reservation::ProtectedReservationTip;
    use crate::reservation::ReleaseDisposition;
    use crate::reservation::ReservationLifecycle;
    use crate::reservation::RewrittenIntegrationTrunkCommit;

    const PENDING_BYPASS_NAME: &str =
        "cargo-berth-pending-bypass-01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a99.json";
    const UNKNOWN_OBJECT_ID: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    #[test]
    fn unresolved_overlaps_distinguish_enrollment_from_user_answers() -> FixtureResult<()> {
        let fixture = BoardFixture::new()?;
        let actor = fixture.main_actor();
        let blocker = fixture.claim(&actor, "shared.rs", ConflictAuthorization::NoConflict)?;
        let scopes = ReservationScopeSet::try_from(vec![ReservationScope {
            path: "shared.rs".parse()?,
            kind: ScopeKind::File,
        }])?;
        let overlaps = AuthorizedOverlap {
            reservation_id: blocker.reservation_id,
            scope_revision: OverlapScopeRevision::from(&scopes),
            scopes:         scopes.into(),
        };
        let enrolled = fixture.claim(
            &actor,
            "shared.rs",
            ConflictAuthorization::Enrollment {
                overlaps: overlaps.clone().into(),
            },
        )?;
        let model = fixture.model()?;
        let [enrollment] = model.unresolved_overlaps.entries.as_slice() else {
            return Err(
                io::Error::other("enrollment should project one unresolved overlap").into(),
            );
        };
        assert_eq!(enrollment.origin, DeferralOrigin::Enrollment);
        assert_eq!(enrollment.deferred, enrolled.reservation_id);
        assert_eq!(enrollment.blocker, blocker.reservation_id);
        assert_eq!(enrollment.consequence, DeferralConsequence::Both);
        assert_eq!(serde_json::to_value(enrollment)?["origin"], "enrollment");

        let user = fixture.claim(
            &actor,
            "shared.rs",
            ConflictAuthorization::Defer {
                overlaps: overlaps.into(),
                blocker:  blocker.reservation_id,
                reason:   enrollment.reason.clone(),
            },
        )?;
        let model = fixture.model()?;
        let user_overlap = model
            .unresolved_overlaps
            .entries
            .iter()
            .find(|overlap| overlap.deferred == user.reservation_id)
            .ok_or_else(|| {
                io::Error::other("user deferral should project an unresolved overlap")
            })?;
        assert_eq!(user_overlap.origin, DeferralOrigin::UserAnswer);
        assert_eq!(serde_json::to_value(user_overlap)?["origin"], "user_answer");
        Ok(())
    }

    /// A blocker and a reservation that deferred its overlap with that blocker.
    struct DeferredPairFixture {
        board:    BoardFixture,
        actor:    TestActor,
        deferred: ReservationId,
        blocker:  ReservationId,
    }

    impl DeferredPairFixture {
        fn new() -> FixtureResult<Self> {
            let board = BoardFixture::new()?;
            let actor = board.main_actor();
            let blocker = board.claim(&actor, "shared.rs", ConflictAuthorization::NoConflict)?;
            let deferred = board.claim(
                &actor,
                "shared.rs",
                test_support::conflict_authorization(OverlapAnswerFixture::Defer, &blocker)?,
            )?;
            Ok(Self {
                board,
                actor,
                deferred: deferred.reservation_id,
                blocker: blocker.reservation_id,
            })
        }

        fn abandon(&self, reservation_id: ReservationId) -> FixtureResult<()> {
            self.board.release(
                &self.actor,
                reservation_id,
                ReleaseDisposition::Abandoned(
                    "the overlapping work was dropped".parse::<AbandonmentReason>()?,
                ),
            )
        }

        fn model(&self) -> FixtureResult<BoardModel> {
            let model = self.board.model()?;
            assert_placed_once(&model, &[self.deferred, self.blocker]);
            Ok(model)
        }
    }

    #[test]
    fn an_overlap_leaves_the_board_once_both_sides_end() -> FixtureResult<()> {
        let pair = DeferredPairFixture::new()?;
        pair.abandon(pair.deferred)?;
        pair.abandon(pair.blocker)?;
        let model = pair.model()?;
        assert!(model.unresolved_overlaps.entries.is_empty());
        assert!(model.waiting.entries.is_empty());
        Ok(())
    }

    #[test]
    fn a_released_blocker_leaves_the_deferred_side_waiting() -> FixtureResult<()> {
        let pair = DeferredPairFixture::new()?;
        pair.abandon(pair.blocker)?;
        let model = pair.model()?;
        assert_eq!(
            overlap_consequence(&model)?,
            DeferralConsequence::DeferredOnly
        );
        assert_eq!(overlap_holds(&model), [pair.deferred]);
        assert!(matches!(
            model.live_overlap_answers.entries.as_slice(),
            [RecordedOverlapAnswer::Defer {
                consequence: DeferralConsequence::DeferredOnly,
                ..
            }]
        ));

        let wire = serde_json::to_value(&model)?;
        assert_eq!(
            wire["unresolved_overlaps"]["entries"][0]["consequence"],
            "deferred_integration_held_until_sequence"
        );
        let entry = &wire["waiting"]["entries"][0];
        assert_eq!(entry["hold"], "unresolved_overlap");
        assert_eq!(entry["deferred"], serde_json::to_value(pair.deferred)?);
        assert_eq!(entry["blocker"], serde_json::to_value(pair.blocker)?);
        assert_eq!(entry["action"]["reason"], "overlap_not_sequenced");
        assert_eq!(
            entry["reservation"]["reservation_id"],
            serde_json::to_value(pair.deferred)?
        );
        Ok(())
    }

    #[test]
    fn a_released_deferred_side_leaves_the_blocker_waiting() -> FixtureResult<()> {
        let pair = DeferredPairFixture::new()?;
        pair.abandon(pair.deferred)?;
        let model = pair.model()?;
        assert_eq!(
            overlap_consequence(&model)?,
            DeferralConsequence::BlockerOnly
        );
        assert_eq!(overlap_holds(&model), [pair.blocker]);
        Ok(())
    }

    #[test]
    fn a_live_overlap_holds_both_sides() -> FixtureResult<()> {
        let pair = DeferredPairFixture::new()?;
        let model = pair.model()?;
        assert_eq!(overlap_consequence(&model)?, DeferralConsequence::Both);
        assert_eq!(overlap_holds(&model), [pair.deferred, pair.blocker]);
        Ok(())
    }

    fn overlap_consequence(model: &BoardModel) -> FixtureResult<DeferralConsequence> {
        let [overlap] = model.unresolved_overlaps.entries.as_slice() else {
            return Err(io::Error::other("fixture should list one unresolved overlap").into());
        };
        Ok(overlap.consequence)
    }

    /// The reservations an unresolved overlap holds, in waiting order.
    fn overlap_holds(model: &BoardModel) -> Vec<ReservationId> {
        model
            .waiting
            .entries
            .iter()
            .filter(|entry| matches!(entry.hold, WaitingHold::UnresolvedOverlap { .. }))
            .map(|entry| entry.reservation.reservation_id)
            .collect()
    }

    /// Each reservation sits in exactly one of ready now, waiting, unconstrained, and resolved.
    fn assert_placed_once(model: &BoardModel, reservations: &[ReservationId]) {
        let sections = [
            model
                .ready_now
                .entries
                .iter()
                .map(|entry| entry.reservation.reservation_id)
                .collect::<HashSet<_>>(),
            model
                .waiting
                .entries
                .iter()
                .map(|entry| entry.reservation.reservation_id)
                .collect(),
            model
                .unconstrained_reservations
                .entries
                .iter()
                .map(|row| row.reservation_id)
                .collect(),
            model
                .resolved
                .entries
                .iter()
                .map(|row| row.reservation_id)
                .collect(),
        ];
        for reservation_id in reservations {
            assert_eq!(
                sections
                    .iter()
                    .filter(|section| section.contains(reservation_id))
                    .count(),
                1,
                "reservation {reservation_id} should sit in exactly one section"
            );
        }
    }

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
                witness:   IntegrationWitness::EvaluatedTrunk,
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
                witness:   IntegrationWitness::EvaluatedTrunk,
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
        assert_waiting_endpoints(&initial_model, &initial)?;
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
        // The phase must protect real scoped work: a rewrite only costs a reservation its
        // evidence when trunk actually loses the content that evidence rested on.
        let protected_tip = rewritten.commit_predecessor()?;
        rewritten.merge_predecessor()?;
        let integrated_trunk = rewritten.board.trunk()?;
        rewritten.board.checkpoint(
            &rewritten.predecessor_actor,
            rewritten.predecessor.reservation_id,
            protected_tip,
            integrated_trunk.clone(),
        )?;
        rewritten.board.record_evidence(
            &rewritten.predecessor_actor,
            rewritten.predecessor.reservation_id,
            IntegrationEvidenceStatus::Integrated {
                trunk_oid: integrated_trunk,
                proof:     IntegrationProof::ProtectedTipAncestor,
                witness:   IntegrationWitness::EvaluatedTrunk,
            },
        )?;
        rewritten.board.release(
            &rewritten.predecessor_actor,
            rewritten.predecessor.reservation_id,
            ReleaseDisposition::Integrated,
        )?;
        rewritten.board.discard_trunk_tip()?;
        let rewritten_model = rewritten.model()?;
        let WaitingAction::TrunkEvidenceRewritten {
            instruction,
            resolve_flag,
        } = waiting_action(&rewritten_model)?
        else {
            return Err(io::Error::other("rewritten trunk should require new evidence").into());
        };
        assert!(instruction.contains("rewrite of trunk"));
        assert_eq!(resolve_flag, "resolve --integrated-as <target-oid>");
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
        let known_tip = unknown.commit_predecessor()?;
        let checkpoint_trunk = unknown.board.trunk()?;
        unknown.board.checkpoint(
            &unknown.predecessor_actor,
            unknown.predecessor.reservation_id,
            known_tip,
            checkpoint_trunk.clone(),
        )?;
        unknown.model()?;
        let unknown_tip = UNKNOWN_OBJECT_ID.parse::<GitObjectId>()?;
        unknown.board.append_as(
            &unknown.predecessor_actor,
            JournalOperation::Resnapshot {
                reservation_id: unknown.predecessor.reservation_id,
                snapshot:       ReservationSnapshot::Outstanding {
                    phase_start_head: None,
                    protected_tip:    ProtectedReservationTip::from(unknown_tip),
                    trunk_oid:        checkpoint_trunk,
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
        let [
            WaitingEntry {
                hold: WaitingHold::OrderingEdge { action, .. },
                ..
            },
        ] = model.waiting.entries.as_slice()
        else {
            return Err(
                io::Error::other("fixture should produce one waiting ordering edge").into(),
            );
        };
        Ok(action)
    }

    fn assert_waiting_endpoints(
        model: &BoardModel,
        fixture: &OrderedBoardFixture,
    ) -> FixtureResult<()> {
        let [
            WaitingEntry {
                hold:
                    WaitingHold::OrderingEdge {
                        predecessor,
                        successor,
                        ..
                    },
                reservation,
            },
        ] = model.waiting.entries.as_slice()
        else {
            return Err(
                io::Error::other("fixture should produce one waiting ordering edge").into(),
            );
        };
        assert_eq!(*predecessor, fixture.predecessor.reservation_id);
        assert_eq!(*successor, fixture.successor.reservation_id);
        assert_eq!(reservation.reservation_id, fixture.successor.reservation_id);
        Ok(())
    }
}
