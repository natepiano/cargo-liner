// branches and remotes
pub(super) const GIT_HEAD: &str = "HEAD";
pub(super) const GIT_HEAD_REVSPEC_PREFIX: &str = "HEAD...";
pub(super) const GIT_REMOTE_ORIGIN: &str = "origin";
pub(super) const GIT_REMOTE_ORIGIN_PREFIX: &str = "origin/";
pub(super) const GIT_REMOTE_UPSTREAM: &str = "upstream";
pub(super) const GIT_UPSTREAM_REF: &str = "@{upstream}";

// commands
pub(super) const GIT_BINARY: &str = "git";
pub(super) const GIT_CONFIG_COMMAND: &str = "config";
pub(super) const GIT_FOR_EACH_REF_COMMAND: &str = "for-each-ref";
pub(super) const GIT_LS_TREE_COMMAND: &str = "ls-tree";
pub(super) const GIT_LOG_COMMAND: &str = "log";
pub(super) const GIT_REMOTE_COMMAND: &str = "remote";
pub(super) const GIT_REV_LIST_COMMAND: &str = "rev-list";
pub(super) const GIT_REV_PARSE_COMMAND: &str = "rev-parse";
pub(super) const GIT_SHOW_REF_COMMAND: &str = "show-ref";
pub(super) const GIT_STATUS_COMMAND: &str = "status";
pub(super) const GIT_SYMBOLIC_REF_COMMAND: &str = "symbolic-ref";

// config
pub(super) const GIT_CONFIG_REMOTE_PREFIX: &str = "remote.";
pub(super) const GIT_CONFIG_REMOTE_PUSHURL_PATTERN: &str = r"^remote\..*\.pushurl$";
pub(super) const GIT_CONFIG_REMOTE_PUSHURL_SUFFIX: &str = ".pushurl";
pub(super) const GIT_GET_REGEXP_ARG: &str = "--get-regexp";
pub(super) const GIT_GET_URL_ARG: &str = "get-url";

// options
pub(super) const GIT_ABBREV_REF_ARG: &str = "--abbrev-ref";
pub(super) const GIT_BISECT_VARS_ARG: &str = "--bisect-vars";
pub(super) const GIT_COUNT_ARG: &str = "--count";
pub(super) const GIT_FORMAT_ISO8601_ARG: &str = "--format=%aI";
pub(super) const GIT_FORMAT_REFNAME_ARG: &str = "--format=%(refname)";
pub(super) const GIT_LEFT_RIGHT_ARG: &str = "--left-right";
pub(super) const GIT_LOG_LAST_COMMIT_ARG: &str = "-1";
pub(super) const GIT_MAX_PARENTS_ZERO_ARG: &str = "--max-parents=0";
pub(super) const GIT_NO_OPTIONAL_LOCKS_ARG: &str = "--no-optional-locks";
pub(super) const GIT_NOT_ARG: &str = "--not";
pub(super) const GIT_QUIET_ARG: &str = "--quiet";
pub(super) const GIT_REVERSE_ARG: &str = "--reverse";
pub(super) const GIT_SHORT_ARG: &str = "--short";
pub(super) const GIT_SHORT_HEAD_ARG: &str = "--short=8";
pub(super) const GIT_STATUS_IGNORED_MATCHING_ARG: &str = "--ignored=matching";
pub(super) const GIT_STATUS_PORCELAIN_V1_ARG: &str = "--porcelain=v1";
pub(super) const GIT_STATUS_UNTRACKED_ALL_ARG: &str = "--untracked-files=all";
pub(super) const GIT_SYMBOLIC_FULL_NAME_ARG: &str = "--symbolic-full-name";
pub(super) const GIT_VERIFY_ARG: &str = "--verify";

// paths and refs
pub(super) const GIT_BISECT_BAD_REF: &str = "refs/bisect/bad";
pub(super) const GIT_BISECT_GOOD_REF_PREFIX: &str = "refs/bisect/good-";
pub(super) const GIT_BISECT_REFS_PREFIX: &str = "refs/bisect/";
pub(super) const GIT_BISECT_START_FILE: &str = "BISECT_START";
pub(super) const GIT_LOCAL_BRANCH_REF_PREFIX: &str = "refs/heads/";
pub(super) const GIT_ORIGIN_HEAD_REF: &str = "refs/remotes/origin/HEAD";
pub(super) const GIT_REMOTE_HEAD_REF_SUFFIX: &str = "/HEAD";
pub(super) const GIT_REMOTE_REF_PREFIX: &str = "refs/remotes/";
pub(super) const GIT_TREE_SUBMODULE_MODE: &str = "160000";

// pathspec and status
pub(super) const GIT_CHECK_IGNORE_COMMAND: &str = "check-ignore";
pub(super) const GIT_DOUBLE_DASH_ARG: &str = "--";
pub(super) const GIT_IGNORED_STATUS_CODE: &str = "!!";
pub(super) const GIT_QUIET_SHORT_ARG: &str = "-q";
pub(super) const GIT_UNTRACKED_STATUS_CODE: &str = "??";

// submodule config
pub(super) const GIT_SUBMODULE_BRANCH_KEY: &str = "branch";
pub(super) const GIT_SUBMODULE_PATH_KEY: &str = "path";
pub(super) const GIT_SUBMODULE_URL_KEY: &str = "url";
