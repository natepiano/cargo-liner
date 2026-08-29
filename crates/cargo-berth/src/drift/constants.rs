//! Named drift cache file spellings and the git commands one observation runs.

// cache
/// The prefix every per-worktree drift fingerprint cache file carries.
pub(super) const DRIFT_CACHE_FILE_PREFIX: &str = "drift-fingerprint-";
/// The suffix every per-worktree drift fingerprint cache file carries.
pub(super) const DRIFT_CACHE_FILE_SUFFIX: &str = ".json";

// commands
/// The `status` subcommand.
pub(super) const GIT_STATUS_COMMAND: &str = "status";

// flags
/// Report a rename as its separate deletion and addition paths.
pub(super) const GIT_NO_RENAMES_ARGUMENT: &str = "--no-renames";
/// Terminate each field with NUL so paths remain verbatim.
pub(super) const GIT_NUL_TERMINATED_ARGUMENT: &str = "-z";
/// Request the stable machine-readable status format.
pub(super) const GIT_PORCELAIN_ARGUMENT: &str = "--porcelain";
/// Include individual untracked files rather than collapsing untracked directories.
pub(super) const GIT_UNTRACKED_FILES_ALL_ARGUMENT: &str = "--untracked-files=all";
