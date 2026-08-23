//! Named git command and flag spellings.

/// The git executable.
pub(super) const GIT_BINARY: &str = "git";
/// Disable git's optional locks for read-only calls.
pub(super) const GIT_NO_OPTIONAL_LOCKS_ARG: &str = "--no-optional-locks";
/// The `rev-parse` subcommand.
pub(super) const GIT_REV_PARSE_COMMAND: &str = "rev-parse";
/// Ask `rev-parse` for the shared administrative directory.
pub(super) const GIT_COMMON_DIRECTORY_ARG: &str = "--git-common-dir";
/// Ask `rev-parse` for the repository worktree root.
pub(super) const GIT_SHOW_TOPLEVEL_ARG: &str = "--show-toplevel";
