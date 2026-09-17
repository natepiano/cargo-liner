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

// orphan retirement
/// The explanation `OrphanRetirementReason::derived` records when reconciliation, rather than the
/// user, ends an orphaned reservation. It names the two facts that authorized it, because the
/// journal entry is the only account a later reader gets.
pub(crate) const DERIVED_ORPHAN_RETIREMENT_REASON: &str = "berth retired this orphan: \
     git no longer registers the holder worktree, and the last completed merge-extent \
     observation proved the branch level with trunk and nothing uncommitted, so the \
     reservation protected nothing and no later observation could change that";
