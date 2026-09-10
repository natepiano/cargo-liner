//! Named values owned by flat top-level modules.

// claim overlap arguments
pub(crate) const OVERLAP_WHY_ARGUMENT: &str = "overlap-why";
pub(crate) const OVERLAP_WHY_ARGUMENT_ID: &str = "overlap_why";
pub(crate) const OVERLAP_WHY_VALUE_NAME: &str = "REASON";
pub(crate) const PROPOSAL_ARGUMENT: &str = "proposal";
pub(crate) const PROPOSAL_VALUE_NAME: &str = "TOKEN";

/// A missing trunk cannot prove that a branch's merge surface is empty.
pub(crate) const MERGE_EXTENT_TRUNK_UNAVAILABLE: &str =
    "cannot derive merge extent: trunk is unavailable";
/// Failed holder validation preserves the prior surface instead of observing a recycled path.
pub(crate) const MERGE_EXTENT_WORKTREE_UNAVAILABLE: &str =
    "cannot derive merge extent: holder worktree is unavailable";
