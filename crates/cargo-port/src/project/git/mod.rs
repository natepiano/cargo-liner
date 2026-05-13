mod checkout;
mod command;
mod discovery;
mod repo;
mod submodule;
mod worktree_group;

pub(crate) use checkout::CheckoutInfo;
pub(crate) use checkout::GitStatus;
pub(crate) use checkout::LocalGitState;
pub(crate) use checkout::worktree_ahead_behind_primary;
pub(crate) use discovery::GitRepoPresence;
pub(crate) use discovery::WorktreeStatus;
pub(crate) use discovery::get_worktree_health;
pub(crate) use discovery::get_worktree_status;
pub(crate) use discovery::git_repo_root;
pub(crate) use discovery::resolve_common_git_dir;
pub(crate) use discovery::resolve_git_dir;
pub(crate) use repo::GitOrigin;
#[cfg(test)]
pub(crate) use repo::RemoteInfo;
pub(crate) use repo::RemoteKind;
pub(crate) use repo::RepoInfo;
#[cfg(test)]
pub(crate) use repo::WorkflowPresence;
pub(crate) use repo::get_first_commit;
pub(crate) use submodule::Submodule;
pub(crate) use submodule::get_submodules;
pub(crate) use worktree_group::WorktreeGroup;
