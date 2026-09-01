//! The small git subprocess surface required by the ledger.
//!
//! Each submodule owns one git concept and the types that describe its answers:
//! subprocess execution and its literal arguments, the single failure type, object
//! resolution, commit reachability, path attribution, scoped patch comparison,
//! merge-conflict coverage, reference reads and updates, and repository discovery.
//! This root declares them and re-exports the names the rest of the crate uses.

mod command;
mod conflict;
mod constants;
mod discovery;
mod error;
mod object;
mod patch;
mod paths;
mod reachability;
mod refs;

#[cfg(test)]
mod fixture;

pub(crate) use command::GitCommandOutputAvailability;
pub(crate) use command::git_execution as execute_read_only_git;
pub(crate) use discovery::common_directory;
pub(crate) use discovery::hooks_directory;
pub(crate) use discovery::repository_root;
pub(crate) use discovery::rewrite_in_progress;
pub(crate) use discovery::worktree_list_porcelain;
pub(crate) use error::GitError;
pub(crate) use object::commit_is_available;
pub(crate) use patch::ScopedPatchComparison;
pub(crate) use patch::ScopedPatchTargetHistory;
pub(crate) use patch::rewritten_phase_anchor;
pub(crate) use patch::scoped_patch_equivalence;
pub(crate) use patch::scoped_patch_equivalence_with_target_history;
pub(crate) use paths::INCURSION_ATTRIBUTION_RECORD_MARKER;
pub(crate) use paths::IncursionPathLogInvocation;
pub(crate) use paths::incursion_path_log;
pub(crate) use paths::phase_committed_path_diffs;
pub(crate) use reachability::AheadBehind;
pub(crate) use reachability::CandidateHeadReachability;
pub(crate) use reachability::CommitCandidateReachability;
pub(crate) use reachability::CommitTargetReachability;
pub(crate) use reachability::CommitTargetReachabilityObservation;
pub(crate) use reachability::PhaseStartTargetFirstParentHistories;
pub(crate) use reachability::ProtectedTipSuccessorHeadClassification;
pub(crate) use reachability::ProtectedTipSuccessorHeads;
pub(crate) use reachability::Reachability;
pub(crate) use reachability::ReservationCheckpointCommits;
pub(crate) use reachability::ResolvedBatchCommitCandidates;
pub(crate) use reachability::ahead_behind_for_heads;
pub(crate) use reachability::branch_commit_reachability;
pub(crate) use reachability::commits_outside_origin_basis;
pub(crate) use reachability::descendant_commits;
pub(crate) use reachability::head_commit_reachability;
pub(crate) use reachability::incursion_range_commits;
pub(crate) use reachability::newly_reachable_commits;
pub(crate) use reachability::reachability;
pub(crate) use reachability::reachability_to_target;
pub(crate) use reachability::reachable_commits;
pub(crate) use reachability::reservation_checkpoint_commits;
pub(crate) use refs::HeadAttachment;
pub(crate) use refs::LocalBranchRenameTargetResolution;
pub(crate) use refs::ReferenceLookup;
pub(crate) use refs::ReservationRetentionRefRepair;
pub(crate) use refs::branch_object_id;
pub(crate) use refs::head_attachment;
pub(crate) use refs::head_object_id;
pub(crate) use refs::local_branch_rename_target_resolution;
pub(crate) use refs::reference_lookup;
pub(crate) use refs::reservation_retention_ref_name;
pub(crate) use refs::symbolic_head_reference;
pub(crate) use refs::update_local_branch;
pub(crate) use refs::update_reservation_retention_refs;
pub(crate) use refs::update_reservation_retention_refs_from_resolved_batch;
pub(crate) use refs::write_reservation_retention_ref;
