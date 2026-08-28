//! Named drift cache file spellings and the git commands one observation runs.

// cache
/// The prefix every per-worktree drift fingerprint cache file carries.
pub(super) const DRIFT_CACHE_FILE_PREFIX: &str = "drift-fingerprint-";
/// The suffix every per-worktree drift fingerprint cache file carries.
pub(super) const DRIFT_CACHE_FILE_SUFFIX: &str = ".json";

// commands
/// The `diff` subcommand.
pub(super) const GIT_DIFF_COMMAND: &str = "diff";
/// The `ls-files` subcommand.
pub(super) const GIT_LS_FILES_COMMAND: &str = "ls-files";
/// The `status` subcommand.
pub(super) const GIT_STATUS_COMMAND: &str = "status";

// flags
/// Compare the index instead of the working tree.
pub(super) const GIT_CACHED_ARGUMENT: &str = "--cached";
/// Apply the standard ignore rules when listing untracked paths.
pub(super) const GIT_EXCLUDE_STANDARD_ARGUMENT: &str = "--exclude-standard";
/// Report one status letter and path per change instead of a patch.
pub(super) const GIT_NAME_STATUS_ARGUMENT: &str = "--name-status";
/// Report a rename as its separate deletion and addition paths.
pub(super) const GIT_NO_RENAMES_ARGUMENT: &str = "--no-renames";
/// Terminate each field with NUL so paths remain verbatim.
pub(super) const GIT_NUL_TERMINATED_ARGUMENT: &str = "-z";
/// List untracked paths.
pub(super) const GIT_OTHERS_ARGUMENT: &str = "--others";
/// Request the stable machine-readable status format.
pub(super) const GIT_PORCELAIN_ARGUMENT: &str = "--porcelain";

// references
/// The current worktree commit.
pub(super) const GIT_HEAD_REVISION: &str = "HEAD";
