//! Canonical ordering for the paths and reservation identities a drift report names.

use crate::ids::ReservationId;
use crate::ids::ReservationScopePath;

pub(super) fn normalize_paths(paths: &mut Vec<ReservationScopePath>) {
    paths.sort_by_key(ToString::to_string);
    paths.dedup_by(|left, right| left.to_string() == right.to_string());
}

pub(super) fn sort_reservation_ids(reservation_ids: &mut [ReservationId]) {
    reservation_ids.sort_by_key(ToString::to_string);
}

pub(super) fn sort_and_deduplicate_reservation_ids(reservation_ids: &mut Vec<ReservationId>) {
    sort_reservation_ids(reservation_ids);
    reservation_ids.dedup();
}
