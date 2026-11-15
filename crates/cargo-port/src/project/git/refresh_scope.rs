use std::path::Path;

use super::constants::GIT_HEAD;
use super::constants::GIT_LOCAL_BRANCH_REF_PREFIX;
use super::constants::GIT_REFLOG_DIR;
use super::constants::GIT_REMOTE_REF_PREFIX;
use super::constants::GIT_SYMBOLIC_REF_PREFIX;

/// What a checkout probe must read again after a Git event.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum GitRefreshScope {
    /// A branch or remote ref that the checkout's `HEAD` does not name changed.
    /// The Git status cannot change, so the probe runs no `git status`.
    RefsOnly,
    /// Each checkout field, the Git status included.
    Full,
}

impl GitRefreshScope {
    /// The scope of a change of `event_path` for the checkout with `git_dir`. A
    /// change of a branch or remote ref that the checkout's `HEAD` does not name
    /// cannot change its Git status. Each other Git event can.
    pub(crate) fn of_event(event_path: &Path, git_dir: &Path, common_git_dir: &Path) -> Self {
        let Ok(in_git_dir) = event_path.strip_prefix(common_git_dir) else {
            return Self::Full;
        };
        // Git writes the reflog of a ref under `logs/` when it writes the ref.
        let ref_path = in_git_dir
            .strip_prefix(GIT_REFLOG_DIR)
            .unwrap_or(in_git_dir);
        if ref_path.starts_with(GIT_REMOTE_REF_PREFIX) {
            return Self::RefsOnly;
        }
        let Ok(branch) = ref_path.strip_prefix(GIT_LOCAL_BRANCH_REF_PREFIX) else {
            return Self::Full;
        };
        if branch.as_os_str().is_empty() {
            return Self::Full;
        }
        let Ok(head) = std::fs::read_to_string(git_dir.join(GIT_HEAD)) else {
            return Self::Full;
        };
        match head.trim_end().strip_prefix(GIT_SYMBOLIC_REF_PREFIX) {
            // `HEAD` names `refs/heads/<branch>`, so a change of that branch moves `HEAD`. An
            // event can name a folder that holds the branch of `HEAD`, as `refs/heads/feature`
            // holds `refs/heads/feature/x`.
            Some(head_ref)
                if Path::new(head_ref)
                    .strip_prefix(GIT_LOCAL_BRANCH_REF_PREFIX)
                    .is_ok_and(|head_branch| head_branch.starts_with(branch)) =>
            {
                Self::Full
            },
            // `HEAD` names another ref, or holds a commit id (a detached `HEAD`).
            Some(_) | None => Self::RefsOnly,
        }
    }
}
