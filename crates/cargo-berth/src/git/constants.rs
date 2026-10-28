//! Named git command, flag, reference, and status spellings.

/// Keep internally spawned hooks on this berth executable unless the caller selected another.
pub(super) const BERTH_EXECUTABLE_ENVIRONMENT: &str = "CARGO_BERTH_EXECUTABLE";

/// Hook-local repository selectors must not redirect Git away from its requested checkout.
pub(super) const GIT_REPOSITORY_ENVIRONMENT: [&str; 5] = [
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_COMMON_DIR",
    "GIT_PREFIX",
];

// Configuration spellings: `GIT_BRANCH_CONFIGURATION_SECTION` through `GIT_SPLIT_INDEX_KEY`.
/// Configuration section for branch tracking rules.
pub(super) const GIT_BRANCH_CONFIGURATION_SECTION: &str = "branch";
/// Branch upstream merge reference key.
pub(super) const GIT_BRANCH_MERGE_KEY: &str = "merge";
/// Branch upstream remote key.
pub(super) const GIT_BRANCH_REMOTE_KEY: &str = "remote";
/// Explicit disabled configuration value.
pub(super) const GIT_CONFIG_FALSE_VALUE: &str = "false";
/// Shell-encoded inherited command configuration.
pub(super) const GIT_CONFIG_PARAMETERS_ENVIRONMENT: &str = "GIT_CONFIG_PARAMETERS";
/// Numeric disabled configuration value.
pub(super) const GIT_CONFIG_ZERO_VALUE: &str = "0";
/// Filesystem monitor configuration.
pub(super) const GIT_FILESYSTEM_MONITOR_KEY: &str = "core.fsmonitor";
/// Automatic garbage collection configuration.
pub(super) const GIT_GC_AUTO_KEY: &str = "gc.auto";
/// Remote name for the same repository.
pub(super) const GIT_LOCAL_REMOTE_VALUE: &str = ".";
/// Automatic maintenance configuration.
pub(super) const GIT_MAINTENANCE_AUTO_KEY: &str = "maintenance.auto";
/// Repository object hash configuration.
pub(super) const GIT_OBJECT_FORMAT_KEY: &str = "extensions.objectFormat";
/// Reftable reference storage format.
pub(super) const GIT_REFTABLE_STORAGE_VALUE: &str = "reftable";
/// Repository reference storage configuration.
pub(super) const GIT_REF_STORAGE_KEY: &str = "extensions.refStorage";
/// SHA-256 object hash format.
pub(super) const GIT_SHA256_OBJECT_FORMAT_VALUE: &str = "sha256";
/// Sparse checkout configuration.
pub(super) const GIT_SPARSE_CHECKOUT_KEY: &str = "core.sparseCheckout";
/// Split index configuration.
pub(super) const GIT_SPLIT_INDEX_KEY: &str = "core.splitIndex";

// File-mode bits: `GIT_FILE_TYPE_MASK`.
/// Bits distinguishing regular files, symlinks, trees and gitlinks.
pub(super) const GIT_FILE_TYPE_MASK: u32 = 0o170_000;

// Command spellings: `GIT_BINARY` through `GIT_WORKTREE_COMMAND`.
/// The git executable.
pub(super) const GIT_BINARY: &str = "git";
/// The `cat-file` subcommand.
pub(super) const GIT_CAT_FILE_COMMAND: &str = "cat-file";
/// The `diff` subcommand.
pub(super) const GIT_DIFF_COMMAND: &str = "diff";
/// The `diff-tree` subcommand.
pub(super) const GIT_DIFF_TREE_COMMAND: &str = "diff-tree";
/// The `for-each-ref` subcommand.
pub(super) const GIT_FOR_EACH_REF_COMMAND: &str = "for-each-ref";
/// The `log` subcommand.
pub(super) const GIT_LOG_COMMAND: &str = "log";
/// The `merge-base` subcommand.
pub(super) const GIT_MERGE_BASE_COMMAND: &str = "merge-base";
/// The `merge-tree` subcommand.
pub(super) const GIT_MERGE_TREE_COMMAND: &str = "merge-tree";
/// The `reflog` subcommand.
pub(super) const GIT_REFLOG_COMMAND: &str = "reflog";
/// The `rev-list` subcommand.
pub(super) const GIT_REV_LIST_COMMAND: &str = "rev-list";
/// The `rev-parse` subcommand.
pub(super) const GIT_REV_PARSE_COMMAND: &str = "rev-parse";
/// The `symbolic-ref` subcommand.
pub(super) const GIT_SYMBOLIC_REF_COMMAND: &str = "symbolic-ref";
/// The `update-ref` subcommand.
pub(super) const GIT_UPDATE_REF_COMMAND: &str = "update-ref";
/// The `worktree` subcommand.
pub(super) const GIT_WORKTREE_COMMAND: &str = "worktree";

// Argument spellings: `GIT_BATCH_CHECK_OBJECT_FORMAT_ARG` through `GIT_WORKTREE_LIST_ARG`.
/// Ask `cat-file` to classify one object per input line.
/// Ask `cat-file` for the resolved object id and object type of each input expression.
pub(super) const GIT_BATCH_CHECK_OBJECT_FORMAT_ARG: &str =
    "--batch-check=%(objectname) %(objecttype)";
/// Mark commits that have a patch-equivalent on the other side of a symmetric difference.
pub(super) const GIT_CHERRY_MARK_ARG: &str = "--cherry-mark";
/// Ask `rev-parse` for the shared administrative directory.
/// Report only the number of selected commits.
pub(super) const GIT_COUNT_ARG: &str = "--count";
/// Show a merge path only when its result differs from every parent.
pub(super) const GIT_DENSE_COMBINED_ARG: &str = "--diff-merges=dense-combined";
/// Follow only the first parent, so a walk stays on one branch's own line.
pub(super) const GIT_FIRST_PARENT_ARG: &str = "--first-parent";
/// Continue a revision walk when one supplied object cannot be resolved.
pub(super) const GIT_IGNORE_MISSING_ARG: &str = "--ignore-missing";
/// Delimit incursion-log commit fields with NUL and start each record with NUL.
/// Ask `merge-base` to test commit ancestry.
pub(super) const GIT_IS_ANCESTOR_ARG: &str = "--is-ancestor";
/// Prefix symmetric-difference commits by their left or right side.
pub(super) const GIT_LEFT_RIGHT_ARG: &str = "--left-right";
/// Prefix bounding a revision walk to a number of commits.
pub(super) const GIT_MAX_COUNT_ARG_PREFIX: &str = "--max-count=";
/// Read only the newest reflog entry.
pub(super) const GIT_MAX_COUNT_ONE_ARG: &str = "--max-count=1";
/// Print only affected repository paths.
pub(super) const GIT_NAME_ONLY_ARG: &str = "--name-only";
/// Print each affected repository path with its change status.
pub(super) const GIT_NAME_STATUS_ARG: &str = "--name-status";
/// Print the complete reference name from `for-each-ref`.
pub(super) const GIT_FULL_REF_FORMAT_ARG: &str = "--format=%(refname)";
/// Supply an explicit merge base to `merge-tree`.
pub(super) const GIT_MERGE_BASE_ARG_PREFIX: &str = "--merge-base=";
/// Omit merge commits, which carry no patch of their own to compare.
pub(super) const GIT_NO_MERGES_ARG: &str = "--no-merges";
/// Omit `merge-tree`'s informational messages, leaving the tree id and conflicted paths.
pub(super) const GIT_NO_MESSAGES_ARG: &str = "--no-messages";
/// Disable git's optional locks for read-only calls.
pub(super) const GIT_NO_OPTIONAL_LOCKS_ARG: &str = "--no-optional-locks";
/// Compare renames as their deletion and addition patches.
pub(super) const GIT_NO_RENAMES_ARG: &str = "--no-renames";
/// Terminate each porcelain field with NUL so worktree paths remain verbatim.
pub(super) const GIT_NUL_TERMINATED_ARG: &str = "-z";
/// Find one common ancestor across every supplied commit.
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
/// Recurse into subtrees while comparing tree objects.
pub(super) const GIT_RECURSIVE_ARG: &str = "-r";
/// Compare a parentless commit with the empty tree.
pub(super) const GIT_ROOT_ARG: &str = "--root";
/// Show entries from the named reflog.
pub(super) const GIT_REFLOG_SHOW_ARG: &str = "show";
/// Print only each reflog entry's subject.
pub(super) const GIT_REFLOG_SUBJECT_FORMAT_ARG: &str = "--format=%gs";
/// Ask `rev-parse` for the repository worktree root.
/// Read revision arguments from standard input.
pub(super) const GIT_STDIN_ARG: &str = "--stdin";
/// Disable rename detection through the merge strategy option boundary.
pub(super) const GIT_STRATEGY_OPTION_NO_RENAMES_ARG: &str = "-Xno-renames";
/// Ask `merge-tree` to write its result tree object.
pub(super) const GIT_WRITE_TREE_ARG: &str = "--write-tree";
/// List registered worktrees.
pub(super) const GIT_WORKTREE_LIST_ARG: &str = "list";

// Output spellings: `GIT_AMBIGUOUS_OBJECT_SUFFIX` through `GIT_RIGHT_COMMIT_MARK`.
/// Suffix reported by `cat-file --batch-check` for an ambiguous object expression.
pub(super) const GIT_AMBIGUOUS_OBJECT_SUFFIX: &str = " ambiguous";
/// Object type required by commit-resolution queries.
pub(super) const GIT_COMMIT_OBJECT_TYPE: &str = "commit";
/// Prefix `--cherry-mark` gives a commit that has a patch-equivalent on the other side.
pub(super) const GIT_EQUIVALENT_COMMIT_MARK: char = '=';
/// Prefix `--left-right` gives a commit found only on the left side.
pub(super) const GIT_LEFT_COMMIT_MARK: char = '<';
/// Suffix reported by `cat-file --batch-check` for an unresolved object expression.
pub(super) const GIT_MISSING_OBJECT_SUFFIX: &str = " missing";
/// Prefix `--left-right` gives a commit found only on the right side.
pub(super) const GIT_RIGHT_COMMIT_MARK: char = '>';

// Pathspec spelling: `GIT_LITERAL_TOP_PATHSPEC_PREFIX`.
/// Prefix that makes a repository-root-relative pathspec literal.
pub(super) const GIT_LITERAL_TOP_PATHSPEC_PREFIX: &str = ":(top,literal)";

// Reference spellings: `GIT_ANCESTOR_RANGE_INFIX` through `RESERVATION_RETENTION_REF_PREFIX`.
/// Infix selecting the commits reachable from the second revision but not the first.
pub(super) const GIT_ANCESTOR_RANGE_INFIX: &str = "..";
/// Git's per-worktree marker for a stopped cherry-pick.
pub(super) const GIT_CHERRY_PICK_STATE_PATH: &str = "CHERRY_PICK_HEAD";
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
/// Git's per-worktree lock while the index and checked-out paths are being replaced.
pub(super) const GIT_INDEX_LOCK_PATH: &str = "index.lock";
/// The prefix for local branch refs.
pub(super) const GIT_LOCAL_BRANCH_REF_PREFIX: &str = "refs/heads/";
/// Git's per-worktree marker for a merge awaiting completion.
pub(super) const GIT_MERGE_STATE_PATH: &str = "MERGE_HEAD";
/// Git's per-worktree state directory for a rebase or `am` running on the apply backend.
pub(super) const GIT_REBASE_APPLY_STATE_PATH: &str = "rebase-apply";
/// Git's per-worktree state directory for a rebase running on the merge backend.
pub(super) const GIT_REBASE_MERGE_STATE_PATH: &str = "rebase-merge";
/// Git's per-worktree marker for a stopped revert.
pub(super) const GIT_REVERT_STATE_PATH: &str = "REVERT_HEAD";
/// Infix selecting the commits reachable from exactly one of two revisions.
pub(super) const GIT_SYMMETRIC_RANGE_INFIX: &str = "...";
/// Suffix selecting a local branch's configured upstream.
/// The private namespace used to retain reservation commits.
pub(super) const RESERVATION_RETENTION_REF_PREFIX: &str = "refs/cargo-berth/reservations/";

// Exit statuses: `GIT_NO_MERGE_BASE_EXIT_CODE` through `GIT_NOT_ANCESTOR_EXIT_CODE`.
/// Git's documented result when two commits have no merge base.
pub(super) const GIT_NO_MERGE_BASE_EXIT_CODE: i32 = 1;
/// Git's documented result when `merge-tree` completes without conflicts.
pub(super) const GIT_MERGE_TREE_CLEAN_EXIT_CODE: i32 = 0;
/// Git's documented result when `merge-tree` writes a tree containing conflicts.
pub(super) const GIT_MERGE_TREE_CONFLICT_EXIT_CODE: i32 = 1;
/// Git's documented result when `merge-base --is-ancestor` finds no ancestry.
pub(super) const GIT_NOT_ANCESTOR_EXIT_CODE: i32 = 1;

// refusal diagnostics and process inspection
/// Prefix surrounding the refused path in Git's stable English diagnostic.
pub(super) const GIT_LOCK_REFUSAL_PREFIX: &str = "Unable to create '";
/// Suffix identifying an existing lock rather than another write error.
pub(super) const GIT_LOCK_REFUSAL_SUFFIX: &str = "': File exists";
/// Extension of a Git lock file.
pub(super) const GIT_LOCK_EXTENSION: &str = "lock";
/// Linux process inspection root.
#[cfg(target_os = "linux")]
pub(super) const PROCESS_DIRECTORY: &str = "/proc";
/// Per-process open file descriptors.
#[cfg(target_os = "linux")]
pub(super) const PROCESS_DESCRIPTORS_DIRECTORY: &str = "fd";
/// Process holder inspection on other supported systems.
#[cfg(not(target_os = "linux"))]
pub(super) const LSOF_BINARY: &str = "/usr/sbin/lsof";
/// Request process ids alone from lsof.
#[cfg(not(target_os = "linux"))]
pub(super) const LSOF_PROCESS_IDS_ARG: &str = "-t";
/// Locale selector used for machine-readable diagnostics.
pub(super) const GIT_LOCALE_ENVIRONMENT: &str = "LC_ALL";
/// Stable English diagnostics.
pub(super) const GIT_DIAGNOSTIC_LOCALE: &str = "C";
/// Number of inherited configuration overlay entries.
pub(super) const GIT_CONFIG_COUNT_ENVIRONMENT: &str = "GIT_CONFIG_COUNT";
/// Prefix of an overlay configuration key variable.
pub(super) const GIT_CONFIG_KEY_PREFIX: &str = "GIT_CONFIG_KEY_";
/// Prefix of an overlay configuration value variable.
pub(super) const GIT_CONFIG_VALUE_PREFIX: &str = "GIT_CONFIG_VALUE_";
/// Configuration overlay for explicit private-ref writes.
pub(super) const RETENTION_REF_CONFIG: &[(&str, &str)] = &[("core.hooksPath", "/dev/null")];
/// Only deferrable maintenance yields immediately when another ref writer holds a lock.
pub(super) const RETENTION_MAINTENANCE_CONFIG: &[(&str, &str)] = &[
    ("core.hooksPath", "/dev/null"),
    ("core.packedRefsTimeout", "0"),
    ("core.filesRefLockTimeout", "0"),
];
