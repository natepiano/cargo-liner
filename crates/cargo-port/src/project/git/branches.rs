use std::path::Path;

use super::command;
use super::constants::GIT_ABBREV_REF_ARG;
use super::constants::GIT_HEAD;
use super::constants::GIT_LOCAL_BRANCH_REF_PREFIX;
use super::constants::GIT_ORIGIN_HEAD_REF;
use super::constants::GIT_QUIET_ARG;
use super::constants::GIT_REMOTE_HEAD_REF_SUFFIX;
use super::constants::GIT_REMOTE_ORIGIN_PREFIX;
use super::constants::GIT_REMOTE_REF_PREFIX;
use super::constants::GIT_REV_PARSE_COMMAND;
use super::constants::GIT_SHORT_ARG;
use super::constants::GIT_SHOW_REF_COMMAND;
use super::constants::GIT_SYMBOLIC_FULL_NAME_ARG;
use super::constants::GIT_SYMBOLIC_REF_COMMAND;
use super::constants::GIT_UPSTREAM_REF;
use super::constants::GIT_VERIFY_ARG;
use crate::config;
use crate::config::CargoPortConfig;

pub(super) fn get_current_branch(repo_root: &Path) -> Option<String> {
    command::git_output_logged(
        repo_root,
        "rev_parse_head",
        [GIT_REV_PARSE_COMMAND, GIT_ABBREV_REF_ARG, GIT_HEAD],
    )
    .ok()
    .and_then(|o| {
        let b = String::from_utf8_lossy(&o.stdout).trim().to_string();
        if b.is_empty() { None } else { Some(b) }
    })
}

pub(super) fn get_upstream_branch(project_dir: &Path) -> Option<String> {
    command::git_output_logged(
        project_dir,
        "rev_parse_upstream_name",
        [
            GIT_REV_PARSE_COMMAND,
            GIT_ABBREV_REF_ARG,
            GIT_SYMBOLIC_FULL_NAME_ARG,
            GIT_UPSTREAM_REF,
        ],
    )
    .ok()
    .and_then(|o| {
        let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
        if s.is_empty() { None } else { Some(s) }
    })
}

/// Resolve the repo's default branch from `origin/HEAD` (e.g. `main`).
pub(super) fn get_default_branch(repo_root: &Path) -> Option<String> {
    command::git_output_logged(
        repo_root,
        "symbolic_ref_origin_head",
        [GIT_SYMBOLIC_REF_COMMAND, GIT_ORIGIN_HEAD_REF, GIT_SHORT_ARG],
    )
    .ok()
    .and_then(|o| {
        let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
        s.strip_prefix(GIT_REMOTE_ORIGIN_PREFIX)
            .filter(|b| !b.is_empty())
            .map(str::to_string)
    })
}

/// Resolve the tracked ref for a remote with a fallback chain.
///
/// Tries, in order:
/// 1. The current branch's `@{upstream}` if it belongs to this remote.
/// 2. `symbolic-ref refs/remotes/<remote>/HEAD`.
/// 3. `<remote>/<default_branch>` (from `origin/HEAD`) if the ref exists.
/// 4. `<remote>/<current_branch>` if the ref exists.
/// 5. `<remote>/<cfg.tui.main_branch>` and each `other_primary_branches` entry if the ref exists.
pub(super) fn resolve_tracked_ref(
    repo_root: &Path,
    remote_name: &str,
    current_upstream: Option<&str>,
    default_branch: Option<&str>,
    current_branch: Option<&str>,
    cargo_port_config: &CargoPortConfig,
) -> Option<String> {
    let prefix = format!("{remote_name}/");
    if let Some(us) = current_upstream
        && us.starts_with(&prefix)
    {
        return Some(us.to_string());
    }
    if let Ok(out) = command::git_output_logged(
        repo_root,
        &format!("symbolic_ref_{remote_name}_head"),
        [
            GIT_SYMBOLIC_REF_COMMAND,
            &format!("{GIT_REMOTE_REF_PREFIX}{remote_name}{GIT_REMOTE_HEAD_REF_SUFFIX}"),
            GIT_SHORT_ARG,
        ],
    ) {
        let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if s.starts_with(&prefix) {
            return Some(s);
        }
    }
    if let Some(db) = default_branch
        && remote_ref_exists(repo_root, remote_name, db)
    {
        return Some(format!("{remote_name}/{db}"));
    }
    if let Some(cb) = current_branch
        && remote_ref_exists(repo_root, remote_name, cb)
    {
        return Some(format!("{remote_name}/{cb}"));
    }
    std::iter::once(cargo_port_config.tui.main_branch.as_str())
        .chain(
            cargo_port_config
                .tui
                .other_primary_branches
                .iter()
                .map(String::as_str),
        )
        .find(|b| remote_ref_exists(repo_root, remote_name, b))
        .map(|b| format!("{remote_name}/{b}"))
}

fn remote_ref_exists(repo_root: &Path, remote_name: &str, branch: &str) -> bool {
    command::git_output_logged(
        repo_root,
        &format!("show_ref_{remote_name}"),
        [
            GIT_SHOW_REF_COMMAND,
            GIT_VERIFY_ARG,
            GIT_QUIET_ARG,
            &format!("{GIT_REMOTE_REF_PREFIX}{remote_name}/{branch}"),
        ],
    )
    .is_ok()
}

pub(super) fn resolve_local_main_branch(project_dir: &Path) -> Option<String> {
    let cargo_port_config = config::active_config();
    std::iter::once(cargo_port_config.tui.main_branch.as_str())
        .chain(
            cargo_port_config
                .tui
                .other_primary_branches
                .iter()
                .map(String::as_str),
        )
        .find(|branch| local_branch_exists(project_dir, branch))
        .map(str::to_string)
}

fn local_branch_exists(project_dir: &Path, branch: &str) -> bool {
    command::git_output_logged(
        project_dir,
        "show_ref_local_main",
        [
            GIT_SHOW_REF_COMMAND,
            GIT_VERIFY_ARG,
            GIT_QUIET_ARG,
            &format!("{GIT_LOCAL_BRANCH_REF_PREFIX}{branch}"),
        ],
    )
    .is_ok()
}
