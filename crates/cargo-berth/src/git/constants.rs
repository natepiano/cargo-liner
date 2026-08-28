//! Named git command, flag, reference, and status spellings.

// commands
/// The git executable.
pub(super) const GIT_BINARY: &str = "git";
/// The `cat-file` subcommand.
pub(super) const GIT_CAT_FILE_COMMAND: &str = "cat-file";
/// The `diff` subcommand.
pub(super) const GIT_DIFF_COMMAND: &str = "diff";
/// The `for-each-ref` subcommand.
pub(super) const GIT_FOR_EACH_REF_COMMAND: &str = "for-each-ref";
/// The `merge-base` subcommand.
pub(super) const GIT_MERGE_BASE_COMMAND: &str = "merge-base";
/// The `merge-tree` subcommand.
pub(super) const GIT_MERGE_TREE_COMMAND: &str = "merge-tree";
/// The `read-tree` subcommand.
pub(super) const GIT_READ_TREE_COMMAND: &str = "read-tree";
/// The `reflog` subcommand.
pub(super) const GIT_REFLOG_COMMAND: &str = "reflog";
/// The `rev-list` subcommand.
pub(super) const GIT_REV_LIST_COMMAND: &str = "rev-list";
/// The `rev-parse` subcommand.
pub(super) const GIT_REV_PARSE_COMMAND: &str = "rev-parse";
/// The `update-index` subcommand.
pub(super) const GIT_UPDATE_INDEX_COMMAND: &str = "update-index";
/// The `update-ref` subcommand.
pub(super) const GIT_UPDATE_REF_COMMAND: &str = "update-ref";
/// The `worktree` subcommand.
pub(super) const GIT_WORKTREE_COMMAND: &str = "worktree";
/// The `write-tree` subcommand.
pub(super) const GIT_WRITE_TREE_COMMAND: &str = "write-tree";

// environment
/// Override git's index with the scoped protected tree construction index.
pub(super) const GIT_INDEX_FILE_ENV: &str = "GIT_INDEX_FILE";

// flags
/// Ask `cat-file` to classify one object per input line.
pub(super) const GIT_BATCH_CHECK_ARG: &str = "--batch-check";
/// Mark commits that have a patch-equivalent on the other side of a symmetric difference.
pub(super) const GIT_CHERRY_MARK_ARG: &str = "--cherry-mark";
/// Ask `rev-parse` for the shared administrative directory.
pub(super) const GIT_COMMON_DIRECTORY_ARG: &str = "--git-common-dir";
/// Report only the number of selected commits.
pub(super) const GIT_COUNT_ARG: &str = "--count";
/// Test whether an object can be read without printing it.
pub(super) const GIT_EXISTS_ARG: &str = "-e";
/// Follow only the first parent, so a walk stays on one branch's own line.
pub(super) const GIT_FIRST_PARENT_ARG: &str = "--first-parent";
/// Continue a revision walk when one supplied object cannot be resolved.
pub(super) const GIT_IGNORE_MISSING_ARG: &str = "--ignore-missing";
/// Read NUL-delimited cache entries from standard input.
pub(super) const GIT_INDEX_INFO_ARG: &str = "--index-info";
/// Ask `merge-base` to test commit ancestry.
pub(super) const GIT_IS_ANCESTOR_ARG: &str = "--is-ancestor";
/// Prefix symmetric-difference commits by their left or right side.
pub(super) const GIT_LEFT_RIGHT_ARG: &str = "--left-right";
/// Prefix bounding a revision walk to a number of commits.
pub(super) const GIT_MAX_COUNT_ARG_PREFIX: &str = "--max-count=";
/// Read only the newest reflog entry.
pub(super) const GIT_MAX_COUNT_ONE_ARG: &str = "--max-count=1";
/// Supply an explicit merge base to `merge-tree`.
pub(super) const GIT_MERGE_BASE_ARG_PREFIX: &str = "--merge-base=";
/// Print only affected repository paths.
pub(super) const GIT_NAME_ONLY_ARG: &str = "--name-only";
/// Print full object ids in raw diff records.
pub(super) const GIT_NO_ABBREV_ARG: &str = "--no-abbrev";
/// Print the complete reference name from `for-each-ref`.
pub(super) const GIT_FULL_REF_FORMAT_ARG: &str = "--format=%(refname)";
/// Omit merge commits, which carry no patch of their own to compare.
pub(super) const GIT_NO_MERGES_ARG: &str = "--no-merges";
/// Disable git's optional locks for read-only calls.
pub(super) const GIT_NO_OPTIONAL_LOCKS_ARG: &str = "--no-optional-locks";
/// Compare renames as their deletion and addition patches.
pub(super) const GIT_NO_RENAMES_ARG: &str = "--no-renames";
/// Terminate each porcelain field with NUL so worktree paths remain verbatim.
pub(super) const GIT_NUL_TERMINATED_ARG: &str = "-z";
/// Print each selected commit with its direct parents.
pub(super) const GIT_PARENTS_ARG: &str = "--parents";
/// Ask `rev-parse` to resolve its path result to an absolute path.
pub(super) const GIT_PATH_FORMAT_ABSOLUTE_ARG: &str = "--path-format=absolute";
/// Ask `rev-parse` to resolve a repository path after configuration overrides.
pub(super) const GIT_PATH_ARG: &str = "--git-path";
/// Separate revision arguments from repository pathspecs.
pub(super) const GIT_PATHSPEC_SEPARATOR: &str = "--";
/// Prefix a `for-each-ref` object-tip filter.
pub(super) const GIT_POINTS_AT_ARG_PREFIX: &str = "--points-at=";
/// Request a stable machine-readable worktree listing.
pub(super) const GIT_PORCELAIN_ARG: &str = "--porcelain";
/// Print raw diff records with object modes and ids.
pub(super) const GIT_RAW_ARG: &str = "--raw";
/// Show entries from the named reflog.
pub(super) const GIT_REFLOG_SHOW_ARG: &str = "show";
/// Print only each reflog entry's subject.
pub(super) const GIT_REFLOG_SUBJECT_FORMAT_ARG: &str = "--format=%gs";
/// Ask `rev-parse` for the repository worktree root.
pub(super) const GIT_SHOW_TOPLEVEL_ARG: &str = "--show-toplevel";
/// Read revision arguments from standard input.
pub(super) const GIT_STDIN_ARG: &str = "--stdin";
/// List registered worktrees.
pub(super) const GIT_WORKTREE_LIST_ARG: &str = "list";
/// Write the merged tree even when it contains conflicts.
pub(super) const GIT_WRITE_TREE_ARG: &str = "--write-tree";

// output
/// Prefix `--cherry-mark` gives a commit that has a patch-equivalent on the other side.
pub(super) const GIT_EQUIVALENT_COMMIT_MARK: char = '=';
/// Prefix that removes a path through `update-index --index-info`.
pub(super) const GIT_INDEX_REMOVAL_RECORD_PREFIX: &[u8] =
    b"0 0000000000000000000000000000000000000000\t";
/// Suffix reported by `cat-file --batch-check` for an unresolved object expression.
pub(super) const GIT_MISSING_OBJECT_SUFFIX: &str = " missing";

// pathspecs
/// Prefix that makes a repository-root-relative pathspec literal.
pub(super) const GIT_LITERAL_TOP_PATHSPEC_PREFIX: &str = ":(top,literal)";

// references
/// Infix selecting the commits reachable from the second revision but not the first.
pub(super) const GIT_ANCESTOR_RANGE_INFIX: &str = "..";
/// Suffix that requires a revision to resolve as a commit.
pub(super) const GIT_COMMIT_PEEL_SUFFIX: &str = "^{commit}";
/// Prefix excluding a revision while retaining its descendants.
pub(super) const GIT_EXCLUDE_REVISION_PREFIX: &str = "^";
/// Suffix selecting a commit's nth first-parent ancestor.
pub(super) const GIT_FIRST_PARENT_ANCESTOR_INFIX: &str = "~";
/// The current worktree commit.
pub(super) const GIT_HEAD_REVISION: &str = "HEAD";
/// Git's configured hook directory selector.
pub(super) const GIT_HOOKS_PATH: &str = "hooks";
/// The prefix for local branch refs.
pub(super) const GIT_LOCAL_BRANCH_REF_PREFIX: &str = "refs/heads/";
/// Git's per-worktree state directory for a rebase or `am` running on the apply backend.
pub(super) const GIT_REBASE_APPLY_STATE_PATH: &str = "rebase-apply";
/// Git's per-worktree state directory for a rebase running on the merge backend.
pub(super) const GIT_REBASE_MERGE_STATE_PATH: &str = "rebase-merge";
/// Infix selecting the commits reachable from exactly one of two revisions.
pub(super) const GIT_SYMMETRIC_RANGE_INFIX: &str = "...";
/// The private namespace used to retain reservation commits.
pub(super) const RESERVATION_RETENTION_REF_PREFIX: &str = "refs/cargo-berth/reservations/";

// statuses
/// Raw-diff status for an added path.
pub(super) const GIT_ADDED_STATUS: &[u8] = b"A";
/// Raw-diff status for a deleted path.
pub(super) const GIT_DELETED_STATUS: &[u8] = b"D";
/// Git's documented result when `merge-tree` completes without conflicts.
pub(super) const GIT_MERGE_TREE_CLEAN_EXIT_CODE: i32 = 0;
/// Git's documented result when `merge-tree` writes a tree containing conflicts.
pub(super) const GIT_MERGE_TREE_CONFLICT_EXIT_CODE: i32 = 1;
/// Raw-diff status for a modified path.
pub(super) const GIT_MODIFIED_STATUS: &[u8] = b"M";
/// Git's documented result when two commits have no merge base.
pub(super) const GIT_NO_MERGE_BASE_EXIT_CODE: i32 = 1;
/// Git's documented result when `merge-base --is-ancestor` finds no ancestry.
pub(super) const GIT_NOT_ANCESTOR_EXIT_CODE: i32 = 1;
/// Raw-diff status for a path whose object kind or mode changed.
pub(super) const GIT_TYPE_CHANGED_STATUS: &[u8] = b"T";
