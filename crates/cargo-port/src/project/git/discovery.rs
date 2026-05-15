use std::path::Path;

use crate::project::info::WorktreeHealth;
use crate::project::paths::AbsolutePath;

/// Whether a project path lives inside a git repository.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GitRepoPresence {
    InRepo,
    OutsideRepo,
}

impl GitRepoPresence {
    pub const fn is_in_repo(self) -> bool { matches!(self, Self::InRepo) }
}

/// The git worktree status of a project directory.
///
/// Captures the mutually exclusive ways a project can relate to git:
/// not in a repo at all, inside a primary (unlinked) repo, or inside a
/// linked worktree. `Primary.root` and `Linked.primary` are both the
/// canonical path of the repo where `.git/` (a directory) lives —
/// distinguishing the two ensures we always know whether this project
/// sits on the main checkout or on a linked one.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum WorktreeStatus {
    #[default]
    NotGit,
    Primary {
        root: AbsolutePath,
    },
    Linked {
        primary: AbsolutePath,
    },
}

impl WorktreeStatus {
    pub const fn is_linked_worktree(&self) -> bool { matches!(self, Self::Linked { .. }) }

    /// Canonical path of the primary repo root (where `.git/` is a
    /// directory). For `NotGit` returns `None`; for both `Primary` and
    /// `Linked` returns the primary repo's root.
    pub const fn primary_root(&self) -> Option<&AbsolutePath> {
        match self {
            Self::NotGit => None,
            Self::Primary { root } => Some(root),
            Self::Linked { primary } => Some(primary),
        }
    }
}

pub(crate) fn git_repo_root(project_dir: &Path) -> Option<AbsolutePath> {
    project_dir
        .ancestors()
        .find(|dir| {
            let git_path = dir.join(".git");
            git_path.is_dir() || git_path.is_file()
        })
        .map(AbsolutePath::from)
}

/// Resolve the on-disk git directory for a repo root.
///
/// For normal repos, returns `repo_root/.git`.
/// For worktrees, `.git` is a file containing `gitdir: <path>` — this
/// function reads that file and returns the resolved path.
pub(crate) fn resolve_git_dir(repo_root: &Path) -> Option<AbsolutePath> {
    let git_path = repo_root.join(".git");
    if git_path.is_dir() {
        return Some(git_path.into());
    }
    if git_path.is_file() {
        let contents = std::fs::read_to_string(&git_path).ok()?;
        let target = contents.strip_prefix("gitdir: ")?.trim();
        return Some(AbsolutePath::resolve(target, repo_root));
    }
    None
}

/// Resolve the common git directory for a repo root.
///
/// For normal repos this is the same path as [`resolve_git_dir`]. For linked
/// worktrees, the resolved git dir may contain a `commondir` file pointing back
/// to the shared `<primary>/.git` directory where branch refs are updated.
pub(crate) fn resolve_common_git_dir(repo_root: &Path) -> Option<AbsolutePath> {
    let git_dir = resolve_git_dir(repo_root)?;
    let commondir_path = git_dir.join("commondir");
    if !commondir_path.is_file() {
        return Some(git_dir);
    }

    let contents = std::fs::read_to_string(&commondir_path).ok()?;
    let target = contents.trim();
    Some(AbsolutePath::resolve(target, &git_dir))
}

/// Check if a project directory is a broken worktree — `.git` is a file whose
/// gitdir target does not exist on disk.
pub(crate) fn get_worktree_health(project_dir: &Path) -> WorktreeHealth {
    let git_path = project_dir.join(".git");
    if !git_path.is_file() {
        return WorktreeHealth::Normal;
    }
    let Ok(contents) = std::fs::read_to_string(&git_path) else {
        return WorktreeHealth::Broken;
    };
    let Some(gitdir_str) = contents.strip_prefix("gitdir: ") else {
        return WorktreeHealth::Broken;
    };
    let gitdir = AbsolutePath::resolve_no_canonicalize(gitdir_str.trim(), project_dir);
    if gitdir.exists() {
        WorktreeHealth::Normal
    } else {
        WorktreeHealth::Broken
    }
}

/// Get the git worktree status for a project directory by walking up
/// until a `.git` entry is found: file → `Linked`, directory → `Primary`,
/// nothing found → `NotGit`.
pub(crate) fn get_worktree_status(project_dir: &Path) -> WorktreeStatus {
    let mut dir = project_dir;
    loop {
        let git_path = dir.join(".git");
        if git_path.is_file() {
            return linked_status_from_gitfile(&git_path, dir);
        }
        if git_path.is_dir() {
            return dir
                .canonicalize()
                .map_or(WorktreeStatus::NotGit, |canonical| {
                    WorktreeStatus::Primary {
                        root: AbsolutePath::from(canonical),
                    }
                });
        }
        let Some(parent) = dir.parent() else {
            return WorktreeStatus::NotGit;
        };
        dir = parent;
    }
}

fn linked_status_from_gitfile(git_path: &Path, dir: &Path) -> WorktreeStatus {
    let Ok(contents) = std::fs::read_to_string(git_path) else {
        return WorktreeStatus::NotGit;
    };
    let Some(gitdir_str) = contents.strip_prefix("gitdir: ") else {
        return WorktreeStatus::NotGit;
    };
    let gitdir = AbsolutePath::resolve(gitdir_str.trim(), dir);
    // gitdir is `<primary>/.git/worktrees/<name>` — go up 3 levels
    let Some(primary_root) = gitdir
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
    else {
        return WorktreeStatus::NotGit;
    };
    WorktreeStatus::Linked {
        primary: AbsolutePath::from(primary_root.to_path_buf()),
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use super::*;

    #[test]
    fn git_repo_root_finds_ancestor_git_directory() {
        let tmp = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
        let repo_root = tmp.path().join("repo");
        let nested = repo_root.join("crates").join("demo");
        std::fs::create_dir_all(repo_root.join(".git")).unwrap_or_else(|_| std::process::abort());
        std::fs::create_dir_all(&nested).unwrap_or_else(|_| std::process::abort());

        assert_eq!(git_repo_root(&nested).as_deref(), Some(repo_root.as_path()));
    }

    #[test]
    fn git_repo_root_finds_worktree_git_file() {
        let tmp = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
        let repo_root = tmp.path().join("repo");
        let nested = repo_root.join("crates").join("demo");
        std::fs::create_dir_all(&nested).unwrap_or_else(|_| std::process::abort());
        std::fs::write(repo_root.join(".git"), "gitdir: /tmp/fake\n")
            .unwrap_or_else(|_| std::process::abort());

        assert_eq!(git_repo_root(&nested).as_deref(), Some(repo_root.as_path()));
    }

    #[test]
    fn resolve_git_dir_returns_dot_git_for_normal_repo() {
        let tmp = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(repo.join(".git")).unwrap_or_else(|_| std::process::abort());

        assert_eq!(
            resolve_git_dir(&repo).as_deref(),
            Some(repo.join(".git").as_path())
        );
    }

    #[test]
    fn resolve_git_dir_follows_worktree_gitdir_file() {
        let tmp = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
        let main_git = tmp
            .path()
            .join("main")
            .join(".git")
            .join("worktrees")
            .join("wt");
        std::fs::create_dir_all(&main_git).unwrap_or_else(|_| std::process::abort());

        let wt = tmp.path().join("wt");
        std::fs::create_dir_all(&wt).unwrap_or_else(|_| std::process::abort());
        std::fs::write(wt.join(".git"), format!("gitdir: {}\n", main_git.display()))
            .unwrap_or_else(|_| std::process::abort());

        let resolved = resolve_git_dir(&wt).expect("should resolve");
        assert_eq!(resolved.canonicalize().ok(), main_git.canonicalize().ok());
    }

    #[test]
    fn resolve_git_dir_returns_none_without_git() {
        let tmp = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
        assert_eq!(resolve_git_dir(tmp.path()), None);
    }
}
