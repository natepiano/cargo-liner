//! The configured session that chooses overlap answers, as a refusal names it.

use std::fs;
use std::path::Path;
use std::path::PathBuf;

use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;

/// The worktree whose session chooses overlap answers, relative to one refused caller.
///
/// This is routing information only: the engine records an answer from whichever session runs
/// the claim command.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
pub(crate) struct OverlapApprover {
    /// The configured approver worktree path.
    pub(crate) worktree:           PathBuf,
    /// Whether the refused caller works in that worktree.
    pub(crate) caller_is_approver: bool,
}

impl OverlapApprover {
    /// Compare the configured approver worktree with the caller's canonical worktree root.
    ///
    /// The configured path is canonicalized when it exists, so a symlinked spelling of the
    /// caller's own worktree still names it.
    pub(crate) fn for_caller(approver_worktree: &Path, caller_worktree_root: &Path) -> Self {
        let caller_is_approver = fs::canonicalize(approver_worktree)
            .map_or(approver_worktree == caller_worktree_root, |canonical| {
                canonical == caller_worktree_root
            });
        Self {
            worktree: approver_worktree.to_path_buf(),
            caller_is_approver,
        }
    }
}
