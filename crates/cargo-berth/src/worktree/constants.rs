//! Git porcelain field names and enrollment spellings used by worktree code.

// enrollment
/// The reason recorded when an enrolled overlap is ordered from the suggested command.
pub(super) const ENROLLMENT_SEQUENCE_REASON: &str = "Order enrolled work";
/// `git merge-base` exits with this code when the commits share no ancestor.
pub(super) const MERGE_BASE_NO_COMMON_ANCESTOR_EXIT_CODE: i32 = 1;

// porcelain fields
pub(super) const HEAD_FIELD_PREFIX: &str = "HEAD ";
pub(super) const BRANCH_FIELD_PREFIX: &str = "branch ";
pub(super) const LOCKED_FIELD: &str = "locked";
pub(super) const PRUNABLE_FIELD: &str = "prunable";
pub(super) const WORKTREE_FIELD_PREFIX: &str = "worktree ";
