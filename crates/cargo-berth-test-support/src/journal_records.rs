//! Predicates over decoded `journal.ndjson` records.
//!
//! Reconciliation records one holder checkout's merge extent once, listing every
//! reservation of that holder it applies to; journals written before that carry
//! one record per reservation. These predicates read both forms, so a test that
//! inspects the journal directly asks about a reservation, not a record layout.

use serde_json::Value;

/// The operation reconciliation writes for one holder's observed merge extent.
pub const HOLDER_MERGE_EXTENT_OBSERVED: &str = "holder_merge_extent_observed";
/// The single-reservation operation earlier binaries wrote for the same observation.
pub const MERGE_EXTENT_OBSERVED: &str = "merge_extent_observed";

/// Whether `event` records an automatic merge extent observation, in either form.
#[must_use]
pub fn is_merge_extent_observation(event: &Value) -> bool {
    event["op"] == HOLDER_MERGE_EXTENT_OBSERVED || event["op"] == MERGE_EXTENT_OBSERVED
}

/// Whether `event` records a merge extent observation for `reservation_id`, in either form.
#[must_use]
pub fn observes_merge_extent_of(event: &Value, reservation_id: &str) -> bool {
    match event["op"].as_str() {
        Some(HOLDER_MERGE_EXTENT_OBSERVED) => {
            event["reservations"]
                .as_array()
                .is_some_and(|reservations| {
                    reservations
                        .iter()
                        .any(|observed| observed["reservation_id"] == reservation_id)
                })
        },
        Some(MERGE_EXTENT_OBSERVED) => event["reservation_id"] == reservation_id,
        _ => false,
    }
}
