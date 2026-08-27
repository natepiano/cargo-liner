//! Reservation policy constants.

// scoped patch evaluation

/// The actual trunk and one proposed trunk are the two live reconciliation targets.
pub(super) const SCOPED_PATCH_TARGET_RETENTION_LIMIT: usize = 2;
