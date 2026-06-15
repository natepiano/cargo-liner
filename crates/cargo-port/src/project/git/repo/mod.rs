use std::path::Path;

use serde::Serialize;

use super::constants::GIT_REMOTE_UPSTREAM;
use super::discovery;
use crate::config;

mod history;
mod push;
mod remote;
mod workflow;

pub(crate) use history::get_first_commit;
pub(crate) use push::PushDisabledReason;
pub(crate) use push::PushState;
pub(crate) use remote::GitOrigin;
pub(crate) use remote::RemoteInfo;
pub(crate) use remote::RemoteKind;
use remote::RemoteResolveContext;
use remote::UpstreamRemote;
use remote::list_remote_names;
pub(crate) use workflow::WorkflowPresence;

use super::branches;

/// Repo-level metadata: state that is the same across every checkout of
/// the same git repo. Lives on `GitRepo::repo_info` so siblings cannot
/// drift.
#[derive(Debug, Clone, Default, Serialize)]
pub(crate) struct RepoInfo {
    /// All remotes declared for this repo.
    pub remotes:           Vec<RemoteInfo>,
    /// Whether `.github/workflows/` contains any `.yml` or `.yaml` files.
    pub workflows:         WorkflowPresence,
    /// ISO 8601 date of the first commit (inception).
    pub first_commit:      Option<String>,
    /// ISO 8601 timestamp of the last `git fetch` against any remote,
    /// derived from the mtime of `FETCH_HEAD` in the common git dir.
    pub last_fetched:      Option<String>,
    /// The repo's default branch name resolved from `origin/HEAD`.
    pub default_branch:    Option<String>,
    /// The local branch name used for `M` comparisons.
    pub local_main_branch: Option<String>,
}

impl RepoInfo {
    /// Repo-level origin classification derived from `remotes`.
    pub fn origin_kind(&self) -> GitOrigin {
        if self.remotes.is_empty() {
            GitOrigin::Local
        } else if self.remotes.iter().any(|r| r.name == GIT_REMOTE_UPSTREAM) {
            GitOrigin::Fork
        } else {
            GitOrigin::Clone
        }
    }

    /// Probe per-repo git metadata. Run once per repo (typically on the
    /// primary checkout's path) and shared across every linked
    /// worktree. Excludes `first_commit`, which is handled by
    /// `schedule_git_first_commit_refreshes` batched by repo root.
    pub fn get(probe_path: &Path) -> Option<Self> {
        let repo_root = discovery::git_repo_root(probe_path)?;
        let active_config = config::active_config();

        // Branch / upstream / default-branch context is probed here
        // because `build_remote_info` uses it to resolve each remote's
        // `tracked_ref` and compute `ahead_behind`. Siblings reuse this
        // work; the canonical source is the primary checkout's view.
        let branch = branches::get_current_branch(&repo_root);
        let current_upstream = branches::get_upstream_branch(&repo_root);
        let default_branch = branches::get_default_branch(&repo_root);
        let local_main_branch = branches::resolve_local_main_branch(&repo_root);

        let remote_names = list_remote_names(&repo_root);
        let upstream_remote = UpstreamRemote::from(remote_names.as_slice());
        let pushurls = push::list_remote_pushurls(&repo_root);
        let remote_context = RemoteResolveContext {
            repo_root: &repo_root,
            upstream_remote,
            current_upstream: current_upstream.as_deref(),
            default_branch: default_branch.as_deref(),
            current_branch: branch.as_deref(),
            config: &active_config,
        };
        let remotes: Vec<RemoteInfo> = remote_names
            .iter()
            .map(|name| {
                remote::build_remote_info(
                    &remote_context,
                    name,
                    pushurls.get(name.as_str()).map(String::as_str),
                )
            })
            .collect();

        Some(Self {
            remotes,
            workflows: workflow::get_workflow_presence(&repo_root),
            first_commit: None,
            last_fetched: history::get_last_fetched(&repo_root),
            default_branch,
            local_main_branch,
        })
    }
}
