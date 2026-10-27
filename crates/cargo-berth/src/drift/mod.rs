//! Working-tree drift observation, pre-lock commit attribution, and locked reservation
//! reconciliation.

mod classification;
mod constants;
mod execution;
mod fingerprint;
mod git_output;
mod identity;
mod observation;
mod ordering;
mod provenance;
mod report;
mod selection;
mod widening;

pub(crate) use execution::execute;
pub(crate) use fingerprint::WorkingTreeFingerprint;
pub(crate) use identity::DriftScopeAcquisition;
pub(crate) use observation::observe_merge_working_tree;
pub(crate) use observation::working_tree_changed_paths;
pub(crate) use report::DriftEffect;
pub(crate) use report::DriftPathAttributionOutcome;
pub(crate) use report::DriftReport;
pub(crate) use report::IncursionCommit;
pub(crate) use report::IncursionCommitOrigin;
pub(crate) use report::PostWriteFreePathProtection;
pub(crate) use report::ReservationDriftResult;
pub(crate) use selection::DriftComparisonChoice;
pub(crate) use selection::DriftRequest;
pub(crate) use selection::DriftReservationSelection;
pub(crate) use selection::PostCommitWideningSelection;
