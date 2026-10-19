use std::path::Path;

use serde::Serialize;

use super::history;
use super::push;
use super::remote;
use super::remote::GitOrigin;
use super::remote::RemoteInfo;
use super::remote::RemoteResolveContext;
use super::remote::UpstreamRemote;
use super::workflow;
use super::workflow::WorkflowPresence;
use crate::config;
use crate::project::git::branches;
use crate::project::git::branches::ResolvedBranch;
use crate::project::git::checkout::CommitDate;
use crate::project::git::checkout::{self};
use crate::project::git::command::GitReadError;
use crate::project::git::constants::GIT_REMOTE_UPSTREAM;
use crate::project::git::discovery;
use crate::project::git::probes;

/// Remote configuration is one read: a failure invalidates the listing.
#[derive(Debug, Clone, Serialize)]
pub(crate) enum RemoteList {
    Listed(Vec<RemoteInfo>),
    Unreadable,
}

impl Default for RemoteList {
    fn default() -> Self { Self::Listed(Vec::new()) }
}

impl RemoteList {
    pub(crate) fn listed(&self) -> &[RemoteInfo] {
        match self {
            Self::Listed(remotes) => remotes,
            Self::Unreadable => &[],
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub(crate) enum FirstCommit {
    #[default]
    AwaitingProbe,
    Probed(CommitDate),
}

/// Repo-level metadata: state that is the same across every checkout of
/// the same git repo. Lives on `GitRepo::repo_info` so siblings cannot
/// drift.
#[derive(Debug, Clone, Default, Serialize)]
pub(crate) struct RepoInfo {
    /// All remotes declared for this repo.
    pub remotes:           RemoteList,
    /// Whether `.github/workflows/` contains any `.yml` or `.yaml` files.
    pub workflows:         WorkflowPresence,
    /// ISO 8601 date of the first commit (inception).
    pub first_commit:      FirstCommit,
    /// ISO 8601 timestamp of the last `git fetch` against any remote,
    /// derived from the mtime of `FETCH_HEAD` in the common git dir.
    pub last_fetched:      Option<String>,
    /// The repo's default branch name resolved from `origin/HEAD`.
    pub default_branch:    ResolvedBranch,
    /// The local branch name used for `M` comparisons.
    pub local_main_branch: ResolvedBranch,
}

impl RepoInfo {
    /// Repo-level origin classification derived from `remotes`.
    pub(crate) fn origin_kind(&self) -> GitOrigin {
        match &self.remotes {
            RemoteList::Unreadable => GitOrigin::Unreadable,
            RemoteList::Listed(remotes) if remotes.is_empty() => GitOrigin::Local,
            RemoteList::Listed(remotes)
                if remotes
                    .iter()
                    .any(|remote| remote.name == GIT_REMOTE_UPSTREAM) =>
            {
                GitOrigin::Fork
            },
            RemoteList::Listed(_) => GitOrigin::Clone,
        }
    }

    /// Probe per-repo git metadata. Run once per repo (typically on the
    /// primary checkout's path) and shared across every linked
    /// worktree. Excludes `first_commit`, which is handled by
    /// `schedule_git_first_commit_refreshes` batched by repo root.
    pub(crate) fn get(probe_path: &Path) -> Option<Self> {
        let repo_root = discovery::git_repo_root(probe_path)?;
        let active_config = config::active_config();

        // Branch / upstream / default-branch context is probed here
        // because `build_remote_info` uses it to resolve each remote's
        // `tracked_ref` and compute `ahead_behind`. Siblings reuse this
        // work; the canonical source is the primary checkout's view.
        let ((head, current_upstream), default_branch, local_main_branch) =
            std::thread::scope(|scope| {
                let current = scope.spawn(|| {
                    let head = checkout::resolve_head_state(&repo_root);
                    let upstream = branches::get_upstream_branch(&repo_root, &head);
                    (head, upstream)
                });
                let default = scope.spawn(|| branches::get_default_branch(&repo_root));
                let main =
                    scope.spawn(|| branches::resolve_local_main_branch(&repo_root, &active_config));
                (
                    probes::finish_probe(current),
                    probes::finish_probe(default),
                    probes::finish_probe(main),
                )
            });

        let remotes = (|| {
            let (remote_names, pushurls) = probes::run_independent_probes(
                || remote::list_remote_names(&repo_root),
                || push::list_remote_pushurls(&repo_root),
            );
            let remote_names = remote_names?;
            let upstream_remote = UpstreamRemote::from(remote_names.as_slice());
            let pushurls = pushurls?;
            let context = RemoteResolveContext {
                repo_root: &repo_root,
                upstream_remote,
                current_upstream: &current_upstream,
                default_branch: &default_branch,
                current_branch: head.branch_name(),
                cargo_port_config: &active_config,
            };
            remote_names
                .iter()
                .map(|name| {
                    remote::build_remote_info(
                        &context,
                        name,
                        pushurls.get(name.as_str()).map(String::as_str),
                    )
                })
                .collect::<Result<Vec<_>, GitReadError>>()
        })()
        .map_or(RemoteList::Unreadable, RemoteList::Listed);

        Some(Self {
            remotes,
            workflows: workflow::get_workflow_presence(&repo_root),
            first_commit: FirstCommit::AwaitingProbe,
            last_fetched: history::get_last_fetched(&repo_root),
            default_branch,
            local_main_branch,
        })
    }
}
