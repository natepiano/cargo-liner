//! Reservation policy constants.

// `SCOPED_PATCH_TARGET_RETENTION_LIMIT` and `SUCCESSOR_SCOPED_PATCH_TARGET_RETENTION_LIMIT` bound
// scoped patch evaluation.

/// The actual trunk and one proposed trunk are the two live reconciliation targets.
pub(super) const SCOPED_PATCH_TARGET_RETENTION_LIMIT: usize = 2;

/// The default graph permits at most this many durable successor-target verdicts per proof subject.
pub(super) const SUCCESSOR_SCOPED_PATCH_TARGET_RETENTION_LIMIT: usize = 512;
