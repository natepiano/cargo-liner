//! Named drift cache file spellings and the git commands one observation runs.

// cache
/// The prefix every per-worktree drift fingerprint cache file carries.
pub(super) const DRIFT_CACHE_FILE_PREFIX: &str = "drift-fingerprint-";
/// The suffix every per-worktree drift fingerprint cache file carries.
pub(super) const DRIFT_CACHE_FILE_SUFFIX: &str = ".json";

// commands
/// The git executable.
pub(super) const GIT_BINARY: &str = "git";
/// The `diff` subcommand.
pub(super) const GIT_DIFF_COMMAND: &str = "diff";
/// The `log` subcommand.
pub(super) const GIT_LOG_COMMAND: &str = "log";
/// The `ls-files` subcommand.
pub(super) const GIT_LS_FILES_COMMAND: &str = "ls-files";
/// The `status` subcommand.
pub(super) const GIT_STATUS_COMMAND: &str = "status";

// flags
/// Compare the index instead of the working tree.
pub(super) const GIT_CACHED_ARGUMENT: &str = "--cached";
/// Apply the standard ignore rules when listing untracked paths.
pub(super) const GIT_EXCLUDE_STANDARD_ARGUMENT: &str = "--exclude-standard";
/// The unit separator `%x1f` writes between a commit's fields.
pub(super) const GIT_FIELD_SEPARATOR: char = '\u{1f}';
/// Report one commit per line as its full object id and subject.
pub(super) const GIT_LOG_FORMAT_ARGUMENT: &str = "--format=%H%x1f%s";
/// Report one status letter and path per change instead of a patch.
pub(super) const GIT_NAME_STATUS_ARGUMENT: &str = "--name-status";
/// Disable git's optional locks for read-only calls.
pub(super) const GIT_NO_OPTIONAL_LOCKS_ARGUMENT: &str = "--no-optional-locks";
/// Report a rename as its separate deletion and addition paths.
pub(super) const GIT_NO_RENAMES_ARGUMENT: &str = "--no-renames";
/// Terminate each field with NUL so paths remain verbatim.
pub(super) const GIT_NUL_TERMINATED_ARGUMENT: &str = "-z";
/// List untracked paths.
pub(super) const GIT_OTHERS_ARGUMENT: &str = "--others";
/// Separate the arguments naming revisions from the ones naming paths.
pub(super) const GIT_PATHSPEC_SEPARATOR: &str = "--";
/// Request the stable machine-readable status format.
pub(super) const GIT_PORCELAIN_ARGUMENT: &str = "--porcelain";

// references
/// The current worktree commit.
pub(super) const GIT_HEAD_REVISION: &str = "HEAD";
