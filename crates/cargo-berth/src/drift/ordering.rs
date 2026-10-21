//! Canonical ordering for the paths a drift report names.
//!
//! The reservation identities a report names are ordered by
//! [`crate::ids::WireOrderedReservationIds`] instead, which carries the ordering as a
//! property of the collection rather than as a call every producer must remember.

use std::borrow::Borrow;

use crate::ids::ReservationScopePath;

pub(super) fn normalize_paths<Path: Borrow<ReservationScopePath>>(paths: &mut Vec<Path>) {
    paths.sort_by_key(|path| Path::borrow(path).to_string());
    paths.dedup_by(|left, right| Path::borrow(left).to_string() == Path::borrow(right).to_string());
}
