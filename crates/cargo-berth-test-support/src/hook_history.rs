//! Evidence records shared by hook tests and recorded fixture builders.

use std::path::PathBuf;

/// One incident and the foreign reservation that it enters.
pub struct ObservedIncursion {
    /// The incident that an answer resolves.
    pub incident_id:            String,
    /// The foreign reservation used by the hook corpus.
    pub foreign_reservation_id: String,
}

/// Released work and the checkout that can report its evidence.
pub struct ReleasedWorkEvidence {
    /// The reservation that owns the released work.
    pub reservation_id: String,
    /// The protected commit recorded at release.
    pub protected_tip:  String,
    /// The readable checkout used for hook reports.
    pub reporting_root: PathBuf,
}
