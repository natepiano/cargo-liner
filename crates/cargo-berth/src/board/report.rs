//! Complete-board and single-reservation reports, and the presentation they render into.

use serde::Serialize;

use super::alerts::AvailableForcedPermit;
use super::alerts::BoardAlert;
use super::alerts::BoardGitCost;
use super::alerts::BypassAuditEntry;
use super::alerts::OutstandingIncursion;
use super::alerts::RecordedIncursionAnswer;
use super::answers::RecordedAnswer;
use super::rows::BoardJournalPosition;
use super::rows::BoardReservationSnapshot;
use super::rows::BoardSection;
use super::rows::IntegrationOrderDeclaration;
use super::rows::ReadyReservation;
use super::rows::RecoveredBypassesThisInvocation;
use super::rows::SettledOrderingConstraint;
use super::rows::UnresolvedOverlap;
use super::rows::WaitingConstraint;
use crate::ids::ReservationId;
use crate::output::ReservationReportSnapshot;
use crate::presentation;
use crate::presentation::EnvelopePresentation;
use crate::reconcile::ReconciliationReport;
use crate::reservation::MergeExtent;
use crate::reservation::RaceExtent;
use crate::reservation::ReservationLifecycleSnapshot;
use crate::reservation::ReservationReplayError;
use crate::reservation::RetainedReservationSet;

/// User-facing complete-board sections, named as the text report presents them.
#[derive(Serialize)]
pub(super) struct CompleteBoardReport<'board> {
    #[serde(rename = "Journal position")]
    pub(super) journal_position:                   &'board BoardJournalPosition,
    #[serde(rename = "Recovered bypasses this invocation")]
    pub(super) recovered_bypasses_this_invocation: &'board RecoveredBypassesThisInvocation,
    #[serde(rename = "Integration order")]
    pub(super) integration_order:                  &'board IntegrationOrderDeclaration,
    #[serde(rename = "Ready now")]
    pub(super) ready_now:                          &'board BoardSection<ReadyReservation>,
    #[serde(rename = "Waiting")]
    pub(super) waiting:                            &'board BoardSection<WaitingConstraint>,
    #[serde(rename = "Settled ordering constraints")]
    pub(super) settled_ordering_constraints:       &'board BoardSection<SettledOrderingConstraint>,
    #[serde(rename = "Unresolved overlaps")]
    pub(super) unresolved_overlaps:                &'board BoardSection<UnresolvedOverlap>,
    #[serde(rename = "Recorded overlap answers")]
    pub(super) recorded_overlap_answers:           &'board BoardSection<RecordedAnswer>,
    #[serde(rename = "Unconstrained reservations")]
    pub(super) unconstrained_reservations:         &'board BoardSection<BoardReservationSnapshot>,
    #[serde(rename = "Resolved reservations")]
    pub(super) resolved_reservations:              &'board BoardSection<BoardReservationSnapshot>,
    #[serde(rename = "Available forced permits")]
    pub(super) available_forced_permits:           &'board BoardSection<AvailableForcedPermit>,
    #[serde(rename = "Bypass audit")]
    pub(super) bypass_audit:                       &'board BoardSection<BypassAuditEntry>,
    #[serde(rename = "Outstanding incursions")]
    pub(super) outstanding_incursions:             &'board BoardSection<OutstandingIncursion>,
    #[serde(rename = "Recorded incursion answers")]
    pub(super) recorded_incursion_answers:         &'board BoardSection<RecordedIncursionAnswer>,
    #[serde(rename = "Alerts")]
    pub(super) alerts:                             &'board BoardSection<BoardAlert>,
    #[serde(rename = "Git cost")]
    pub(super) git_cost:                           &'board BoardGitCost,
}

/// One reservation's placement-independent lifecycle and extent report.
#[derive(Serialize)]
struct ReservationReport<'reservation> {
    #[serde(rename = "Reservation")]
    reservation_id: ReservationId,
    #[serde(rename = "Lifecycle")]
    lifecycle:      &'reservation ReservationLifecycleSnapshot,
    #[serde(rename = "Race extent")]
    race_extent:    &'reservation RaceExtent,
    #[serde(rename = "Merge extent")]
    merge_extent:   &'reservation MergeExtent,
}

/// Render one retained reservation's lifecycle and extents without restating the complete board.
pub(crate) fn reservation_lifecycle_presentation(
    snapshot: &ReservationReportSnapshot,
) -> EnvelopePresentation {
    let reservation_id = snapshot.reservation_id;
    let reservation_report = ReservationReport {
        reservation_id,
        lifecycle: &snapshot.lifecycle,
        race_extent: &snapshot.race_extent,
        merge_extent: &snapshot.merge_extent,
    };
    serde_json::to_string_pretty(&reservation_report).map_or_else(
        |error| {
            presentation::engine_message_block(
                "cargo-berth could not render the reservation lifecycle report.",
                &format!("RESERVATION LIFECYCLE SERIALIZATION FAILED: {error}"),
            )
            .into()
        },
        |detail| {
            presentation::engine_message_block(
                &format!("cargo-berth read reservation {reservation_id} lifecycle."),
                &detail,
            )
            .into()
        },
    )
}

/// Read one retained reservation independently of its complete-board placement.
pub(crate) fn reservation_lifecycle_snapshot(
    report: &ReconciliationReport,
    reservation_id: ReservationId,
) -> Result<ReservationReportSnapshot, ReservationReplayError> {
    let reservations = RetainedReservationSet::replay(report.journal_snapshot.events())?;
    let reservation = reservations.reservation(reservation_id)?;
    Ok(ReservationReportSnapshot {
        reservation_id,
        lifecycle: ReservationLifecycleSnapshot::from(reservation.evidence_state()?),
        race_extent: reservation.race_extent(),
        merge_extent: reservation.merge_extent().clone(),
    })
}
