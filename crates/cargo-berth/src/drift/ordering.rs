//! Canonical ordering for the paths a drift report names.
//!
//! The reservation identities a report names are ordered by
//! [`crate::ids::WireOrderedReservationIds`] instead, which carries the ordering as a
//! property of the collection rather than as a call every producer must remember.

use crate::ids::ReservationScopePath;

pub(super) fn normalize_paths(paths: &mut Vec<ReservationScopePath>) {
    paths.sort_by_key(ToString::to_string);
    paths.dedup_by(|left, right| left.to_string() == right.to_string());
}
