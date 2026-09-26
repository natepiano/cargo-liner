//! Worktree identity validation and registered-holder liveness.

mod constants;
mod enrollment;
mod identity;
pub(crate) mod liveness;

pub(crate) use enrollment::WorktreeEnrollmentReport;
pub(crate) use enrollment::cover_claim;
pub(crate) use enrollment::enroll_worktrees;
pub(crate) use liveness::WorktreeHead;
pub(crate) use liveness::WorktreeLiveness;
pub(crate) use liveness::WorktreeMarkerSweepContext;
pub(crate) use liveness::WorktreeRegistry;
pub(crate) use liveness::WorktreeRelocation;
