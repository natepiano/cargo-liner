//! Named git command, flag, reference, and status spellings.

// commands
/// The git executable.
pub(super) const GIT_BINARY: &str = "git";
/// The `cat-file` subcommand.
pub(super) const GIT_CAT_FILE_COMMAND: &str = "cat-file";
/// The `merge-base` subcommand.
pub(super) const GIT_MERGE_BASE_COMMAND: &str = "merge-base";
/// The `rev-list` subcommand.
pub(super) const GIT_REV_LIST_COMMAND: &str = "rev-list";
/// The `rev-parse` subcommand.
pub(super) const GIT_REV_PARSE_COMMAND: &str = "rev-parse";
/// The `update-ref` subcommand.
pub(super) const GIT_UPDATE_REF_COMMAND: &str = "update-ref";
/// The `worktree` subcommand.
pub(super) const GIT_WORKTREE_COMMAND: &str = "worktree";

// flags
/// Ask `cat-file` to classify one object per input line.
pub(super) const GIT_BATCH_CHECK_ARG: &str = "--batch-check";
/// Include excluded boundary commits so unrelated histories remain distinguishable.
pub(super) const GIT_BOUNDARY_ARG: &str = "--boundary";
/// Ask `rev-parse` for the shared administrative directory.
pub(super) const GIT_COMMON_DIRECTORY_ARG: &str = "--git-common-dir";
/// Prefix selecting commits on descendant paths from one protected tip.
pub(super) const GIT_ANCESTRY_PATH_ARG_PREFIX: &str = "--ancestry-path=";
/// Test whether an object can be read without printing it.
pub(super) const GIT_EXISTS_ARG: &str = "-e";
/// Delete the named ref through `update-ref`.
pub(super) const GIT_DELETE_REF_ARG: &str = "-d";
/// Ask `merge-base` to test commit ancestry.
pub(super) const GIT_IS_ANCESTOR_ARG: &str = "--is-ancestor";
/// Mark commits that have a patch-equivalent on the other side of a symmetric difference.
pub(super) const GIT_CHERRY_MARK_ARG: &str = "--cherry-mark";
/// Report only the number of selected commits.
pub(super) const GIT_COUNT_ARG: &str = "--count";
/// Follow only the first parent, so a walk stays on one branch's own line.
pub(super) const GIT_FIRST_PARENT_ARG: &str = "--first-parent";
/// Prefix bounding a revision walk to a number of commits.
pub(super) const GIT_MAX_COUNT_ARG_PREFIX: &str = "--max-count=";
/// Omit merge commits, which carry no patch of their own to compare.
pub(super) const GIT_NO_MERGES_ARG: &str = "--no-merges";
/// Prefix symmetric-difference commits by their left or right side.
pub(super) const GIT_LEFT_RIGHT_ARG: &str = "--left-right";
/// Disable git's optional locks for read-only calls.
pub(super) const GIT_NO_OPTIONAL_LOCKS_ARG: &str = "--no-optional-locks";
/// Ask `rev-parse` to resolve its path result to an absolute path.
pub(super) const GIT_PATH_FORMAT_ABSOLUTE_ARG: &str = "--path-format=absolute";
/// Ask `rev-parse` to resolve a repository path after configuration overrides.
pub(super) const GIT_PATH_ARG: &str = "--git-path";
/// Request a stable machine-readable worktree listing.
pub(super) const GIT_PORCELAIN_ARG: &str = "--porcelain";
/// Terminate each porcelain field with NUL so worktree paths remain verbatim.
pub(super) const GIT_NUL_TERMINATED_ARG: &str = "-z";
/// Ask `rev-parse` for the repository worktree root.
pub(super) const GIT_SHOW_TOPLEVEL_ARG: &str = "--show-toplevel";
/// List registered worktrees.
pub(super) const GIT_WORKTREE_LIST_ARG: &str = "list";

// output
/// Prefix `--cherry-mark` gives a commit that has a patch-equivalent on the other side.
pub(super) const GIT_EQUIVALENT_COMMIT_MARK: char = '=';
/// Suffix reported by `cat-file --batch-check` for an unresolved object expression.
pub(super) const GIT_MISSING_OBJECT_SUFFIX: &str = " missing";

// references
/// The current worktree commit.
pub(super) const GIT_HEAD_REVISION: &str = "HEAD";
/// Git's configured hook directory selector.
pub(super) const GIT_HOOKS_PATH: &str = "hooks";
/// The prefix for local branch refs.
pub(super) const GIT_LOCAL_BRANCH_REF_PREFIX: &str = "refs/heads/";
/// Prefix excluding a revision while retaining its descendants.
pub(super) const GIT_EXCLUDE_REVISION_PREFIX: &str = "^";
/// Infix selecting the commits reachable from the second revision but not the first.
pub(super) const GIT_ANCESTOR_RANGE_INFIX: &str = "..";
/// Infix selecting the commits reachable from exactly one of two revisions.
pub(super) const GIT_SYMMETRIC_RANGE_INFIX: &str = "...";
/// Suffix selecting a commit's nth first-parent ancestor.
pub(super) const GIT_FIRST_PARENT_ANCESTOR_INFIX: &str = "~";
/// Suffix that requires a revision to resolve as a commit.
pub(super) const GIT_COMMIT_PEEL_SUFFIX: &str = "^{commit}";
/// Git's per-worktree state directory for a rebase running on the merge backend.
pub(super) const GIT_REBASE_MERGE_STATE_PATH: &str = "rebase-merge";
/// Git's per-worktree state directory for a rebase or `am` running on the apply backend.
pub(super) const GIT_REBASE_APPLY_STATE_PATH: &str = "rebase-apply";
/// The private namespace used to retain reservation commits.
pub(super) const RESERVATION_RETENTION_REF_PREFIX: &str = "refs/cargo-berth/reservations/";

// statuses
/// Git's documented result when `merge-base --is-ancestor` finds no ancestry.
pub(super) const GIT_NOT_ANCESTOR_EXIT_CODE: i32 = 1;
