//! Complete-board and single-reservation reports, and the presentation they render into.

use serde::Serialize;

use crate::ids::ReservationId;
use crate::output::ReservationReportSnapshot;
use crate::presentation;
use crate::presentation::EnvelopePresentation;
use crate::reconcile::ReconciliationReport;
use crate::reservation::EffectiveMergeExtent;
use crate::reservation::MergeExtent;
use crate::reservation::RaceExtent;
use crate::reservation::ReservationLifecycleSnapshot;
use crate::reservation::ReservationReplayError;

/// One reservation's placement-independent lifecycle and extent report.
#[derive(Serialize)]
struct ReservationReport<'reservation> {
    #[serde(rename = "Reservation")]
    reservation_id: ReservationId,
    #[serde(rename = "Lifecycle")]
    lifecycle:      &'reservation ReservationLifecycleSnapshot,
    #[serde(rename = "Race extent")]
    race_extent:    &'reservation RaceExtent,
    #[serde(flatten)]
    merge_extent:   HumanMergeExtent<'reservation>,
}

/// The single-reservation report's merge extent, labeled by whether it still protects anything.
#[derive(Serialize)]
enum HumanMergeExtent<'reservation> {
    /// An active or outstanding reservation's current merge protection.
    #[serde(rename = "Merge extent")]
    Live(&'reservation MergeExtent),
    /// The merge extent a released reservation last observed, which refuses nothing.
    #[serde(rename = "Merge extent at release (not blocking)")]
    AtRelease(&'reservation MergeExtent),
}

impl<'reservation> From<&'reservation EffectiveMergeExtent> for HumanMergeExtent<'reservation> {
    fn from(effective_merge_extent: &'reservation EffectiveMergeExtent) -> Self {
        match effective_merge_extent {
            EffectiveMergeExtent::Live(merge_extent) => Self::Live(merge_extent),
            EffectiveMergeExtent::Released { at_release } => Self::AtRelease(at_release),
        }
    }
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
        merge_extent: HumanMergeExtent::from(&snapshot.merge_extent),
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
    let reservation = report
        .journal_snapshot
        .reservations()
        .reservation(reservation_id)?;
    Ok(ReservationReportSnapshot {
        reservation_id,
        lifecycle: ReservationLifecycleSnapshot::from(reservation.evidence_state()?),
        race_extent: reservation.race_extent(),
        merge_extent: reservation.effective_merge_extent(),
    })
}
