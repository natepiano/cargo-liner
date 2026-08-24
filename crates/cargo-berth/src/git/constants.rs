//! Named git command, flag, reference, and status spellings.

// commands
/// The git executable.
pub(super) const GIT_BINARY: &str = "git";
/// The `cat-file` subcommand.
pub(super) const GIT_CAT_FILE_COMMAND: &str = "cat-file";
/// The `merge-base` subcommand.
pub(super) const GIT_MERGE_BASE_COMMAND: &str = "merge-base";
/// The `rev-parse` subcommand.
pub(super) const GIT_REV_PARSE_COMMAND: &str = "rev-parse";
/// The `update-ref` subcommand.
pub(super) const GIT_UPDATE_REF_COMMAND: &str = "update-ref";
/// The `worktree` subcommand.
pub(super) const GIT_WORKTREE_COMMAND: &str = "worktree";

// flags
/// Ask `rev-parse` for the shared administrative directory.
pub(super) const GIT_COMMON_DIRECTORY_ARG: &str = "--git-common-dir";
/// Test whether an object can be read without printing it.
pub(super) const GIT_EXISTS_ARG: &str = "-e";
/// Delete the named ref through `update-ref`.
pub(super) const GIT_DELETE_REF_ARG: &str = "-d";
/// Ask `merge-base` to test commit ancestry.
pub(super) const GIT_IS_ANCESTOR_ARG: &str = "--is-ancestor";
/// Disable git's optional locks for read-only calls.
pub(super) const GIT_NO_OPTIONAL_LOCKS_ARG: &str = "--no-optional-locks";
/// Request a stable machine-readable worktree listing.
pub(super) const GIT_PORCELAIN_ARG: &str = "--porcelain";
/// Terminate each porcelain field with NUL so worktree paths remain verbatim.
pub(super) const GIT_NUL_TERMINATED_ARG: &str = "-z";
/// Ask `rev-parse` for the repository worktree root.
pub(super) const GIT_SHOW_TOPLEVEL_ARG: &str = "--show-toplevel";
/// List registered worktrees.
pub(super) const GIT_WORKTREE_LIST_ARG: &str = "list";

// references
/// The current worktree commit.
pub(super) const GIT_HEAD_REVISION: &str = "HEAD";
/// The prefix for local branch refs.
pub(super) const GIT_LOCAL_BRANCH_REF_PREFIX: &str = "refs/heads/";
/// Suffix that requires a revision to resolve as a commit.
pub(super) const GIT_COMMIT_PEEL_SUFFIX: &str = "^{commit}";
/// The private namespace used to retain reservation commits.
pub(super) const RESERVATION_RETENTION_REF_PREFIX: &str = "refs/cargo-berth/reservations/";

// statuses
/// Git's documented result when `merge-base --is-ancestor` finds no ancestry.
pub(super) const GIT_NOT_ANCESTOR_EXIT_CODE: i32 = 1;
