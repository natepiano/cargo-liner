//! Worktree identity validation and registered-holder liveness.

mod constants;
mod identity;
pub(crate) mod liveness;

pub(crate) use liveness::WorktreeLiveness;
pub(crate) use liveness::WorktreeRegistry;
pub(crate) use liveness::WorktreeRelocation;
