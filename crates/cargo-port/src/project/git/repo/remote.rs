use std::path::Path;

use serde::Serialize;

use super::branches;
use super::push;
use super::push::PushState;
use crate::config::CargoPortConfig;
use crate::constants::GIT_REMOTE_SUFFIX;
use crate::project::git::checkout;
use crate::project::git::command;
use crate::project::git::constants::GIT_GET_URL_ARG;
use crate::project::git::constants::GIT_HEAD_REVSPEC_PREFIX;
use crate::project::git::constants::GIT_REMOTE_COMMAND;
use crate::project::git::constants::GIT_REMOTE_ORIGIN;
use crate::project::git::constants::GIT_REMOTE_UPSTREAM;

/// Whether a project is a plain clone or a fork (has an "upstream" remote).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum GitOrigin {
    /// A local-only repo (no origin remote).
    Local,
    /// A plain git clone (has "origin" remote).
    Clone,
    /// A fork (has an "upstream" remote).
    Fork,
}

/// How a single git remote relates to the repo: a plain clone or the fork
/// origin when an `upstream` remote also exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum RemoteKind {
    Clone,
    Fork,
}

/// Per-remote metadata. A repo may have any number of these (`origin`,
/// `upstream`, and others).
#[derive(Debug, Clone, Serialize)]
pub(crate) struct RemoteInfo {
    pub name:         String,
    pub url:          Option<String>,
    pub owner:        Option<String>,
    pub repo:         Option<String>,
    pub tracked_ref:  Option<String>,
    pub ahead_behind: Option<(usize, usize)>,
    pub kind:         RemoteKind,
    pub push:         PushState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum UpstreamRemote {
    Present,
    Missing,
}

impl UpstreamRemote {
    pub(super) const fn is_present(self) -> bool { matches!(self, Self::Present) }
}

impl From<&[String]> for UpstreamRemote {
    fn from(remote_names: &[String]) -> Self {
        if remote_names.iter().any(|name| name == GIT_REMOTE_UPSTREAM) {
            Self::Present
        } else {
            Self::Missing
        }
    }
}

pub(super) struct RemoteResolveContext<'a> {
    pub(super) repo_root:         &'a Path,
    pub(super) upstream_remote:   UpstreamRemote,
    pub(super) current_upstream:  Option<&'a str>,
    pub(super) default_branch:    Option<&'a str>,
    pub(super) current_branch:    Option<&'a str>,
    pub(super) cargo_port_config: &'a CargoPortConfig,
}

pub(super) fn build_remote_info(
    context: &RemoteResolveContext<'_>,
    name: &str,
    pushurl: Option<&str>,
) -> RemoteInfo {
    let (owner, url, repo) = remote_url_info(context.repo_root, name);
    let tracked_ref = branches::resolve_tracked_ref(
        context.repo_root,
        name,
        context.current_upstream,
        context.default_branch,
        context.current_branch,
        context.cargo_port_config,
    );
    let ahead_behind = tracked_ref.as_deref().and_then(|r| {
        checkout::parse_ahead_behind(
            context.repo_root,
            &format!("{GIT_HEAD_REVSPEC_PREFIX}{r}"),
            &format!("tracked_{name}"),
        )
    });
    let kind = if name == GIT_REMOTE_ORIGIN && context.upstream_remote.is_present() {
        RemoteKind::Fork
    } else {
        RemoteKind::Clone
    };
    let push = push::resolve_push_state(url.as_deref(), pushurl);
    RemoteInfo {
        name: name.to_string(),
        url,
        owner,
        repo,
        tracked_ref,
        ahead_behind,
        kind,
        push,
    }
}

pub(super) fn list_remote_names(repo_root: &Path) -> Vec<String> {
    command::git_output_logged(repo_root, "remote", [GIT_REMOTE_COMMAND])
        .ok()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(String::from)
                .collect()
        })
        .unwrap_or_default()
}

fn remote_url_info(
    repo_root: &Path,
    name: &str,
) -> (Option<String>, Option<String>, Option<String>) {
    command::git_output_logged(
        repo_root,
        &format!("remote_get_url_{name}"),
        [GIT_REMOTE_COMMAND, GIT_GET_URL_ARG, name],
    )
    .ok()
    .map_or((None, None, None), |out| {
        let raw = String::from_utf8_lossy(&out.stdout).trim().to_string();
        parse_remote_url(&raw)
    })
}

/// Extract `(owner, url, repo)` from a git remote URL.
///
/// Handles:
/// - `https://github.com/owner/repo.git`
/// - `git@github.com:owner/repo.git`
///
/// SSH forms are canonicalized to HTTPS so downstream prefix-matching against
/// `default_remote_host_url` works uniformly.
fn parse_remote_url(raw: &str) -> (Option<String>, Option<String>, Option<String>) {
    if let Some(after_at) = raw.strip_prefix("git@")
        && let Some((host, path)) = after_at.split_once(':')
    {
        let path = path.strip_suffix(GIT_REMOTE_SUFFIX).unwrap_or(path);
        let mut parts = path.splitn(2, '/');
        let owner = parts.next().map(String::from);
        let repo = parts.next().map(String::from);
        let url = format!("https://{host}/{path}");
        return (owner, Some(url), repo);
    }

    if raw.starts_with("https://") || raw.starts_with("http://") {
        let clean = raw.strip_suffix(GIT_REMOTE_SUFFIX).unwrap_or(raw);
        let mut segments = clean.split('/').skip(3);
        let owner = segments.next().map(String::from);
        let repo = segments.next().map(String::from);
        return (owner, Some(clean.to_string()), repo);
    }

    (None, None, None)
}
