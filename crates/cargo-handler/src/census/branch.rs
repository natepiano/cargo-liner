//! The git branch each agent works on: read from the `HEAD` of the
//! repository its directory sits in, without running git.
//!
//! The repository is the nearest directory at or above the agent's that
//! holds a `.git` entry. A `.git` directory holds `HEAD` itself; a
//! `.git` file, as a worktree or a submodule has, names the directory
//! that does in its `gitdir:` line.

use std::fs;
use std::path::Path;
use std::path::PathBuf;

use super::AgentRow;
use crate::constants::BRANCH_REF_PREFIX;
use crate::constants::DETACHED_HEAD_LENGTH;
use crate::constants::GIT_DIRNAME;
use crate::constants::GIT_HEAD_FILENAME;
use crate::constants::GITDIR_PREFIX;
use crate::constants::HEAD_REF_PREFIX;
use crate::constants::HOME_ABBREVIATION;

/// Set the branch of each of `rows` from its directory, written against
/// `home`.
pub(super) fn attach_branches(rows: &mut [AgentRow], home: Option<&Path>) {
    for row in rows {
        row.branch = directory_path(&row.directory, home).and_then(|directory| branch(&directory));
    }
}

/// The directory a row's `directory` names, with a leading `~` read as
/// `home`; none for a directory that could not be read, or one under a
/// home directory that is not known.
fn directory_path(directory: &str, home: Option<&Path>) -> Option<PathBuf> {
    match directory.strip_prefix(HOME_ABBREVIATION) {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => {
            Some(home?.join(rest.trim_start_matches('/')))
        },
        _ => Path::new(directory)
            .is_absolute()
            .then(|| PathBuf::from(directory)),
    }
}

/// The branch checked out in the repository `directory` sits in: the
/// branch's name, or for a detached `HEAD` the start of the commit it
/// points at. None outside a repository, or where `HEAD` cannot be
/// read.
fn branch(directory: &Path) -> Option<String> {
    let dot_git = directory
        .ancestors()
        .map(|ancestor| ancestor.join(GIT_DIRNAME))
        .find(|dot_git| dot_git.exists())?;
    let git_directory = if dot_git.is_dir() {
        dot_git
    } else {
        let named = fs::read_to_string(&dot_git).ok()?;
        let named = named.trim().strip_prefix(GITDIR_PREFIX)?.trim();
        dot_git.parent()?.join(named)
    };
    let head = fs::read_to_string(git_directory.join(GIT_HEAD_FILENAME)).ok()?;
    let head = head.trim();
    let branch: String = head.strip_prefix(HEAD_REF_PREFIX).map_or_else(
        || head.chars().take(DETACHED_HEAD_LENGTH).collect(),
        |reference| {
            reference
                .strip_prefix(BRANCH_REF_PREFIX)
                .unwrap_or(reference)
                .to_string()
        },
    );
    (!branch.is_empty()).then_some(branch)
}

#[cfg(test)]
#[expect(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    /// Write `contents` to `path`, making the directories above it.
    fn write(path: &Path, contents: &str) {
        fs::create_dir_all(path.parent().expect("the path should have a parent"))
            .expect("the directories should be made");
        fs::write(path, contents).expect("the file should be written");
    }

    /// A repository's own directory, any directory below it, a worktree
    /// named by an absolute `gitdir:` and a submodule by a relative one
    /// each read the branch their `HEAD` names; a detached `HEAD` reads
    /// as the start of its commit.
    #[test]
    fn a_directory_reads_the_branch_of_the_repository_it_sits_in() {
        let root = TempDir::new().expect("a temporary directory should open");
        let repository = root.path().join("cargo-liner");
        write(&repository.join(".git/HEAD"), "ref: refs/heads/main\n");
        let worktree_git = repository.join(".git/worktrees/transparent");
        write(
            &worktree_git.join("HEAD"),
            "ref: refs/heads/enh/handler-groups\n",
        );
        let worktree = root.path().join("transparent");
        write(
            &worktree.join(".git"),
            &format!("gitdir: {}\n", worktree_git.display()),
        );
        write(
            &repository.join(".git/modules/vendor/HEAD"),
            "1f71d290e2c4a5b6c7d8e9f0a1b2c3d4e5f6a7b8\n",
        );
        let submodule = repository.join("vendor");
        write(&submodule.join(".git"), "gitdir: ../.git/modules/vendor\n");
        let deep = repository.join("crates/cargo-handler/src");
        fs::create_dir_all(&deep).expect("the directories should be made");

        assert_eq!(branch(&repository).as_deref(), Some("main"));
        assert_eq!(branch(&deep).as_deref(), Some("main"));
        assert_eq!(branch(&worktree).as_deref(), Some("enh/handler-groups"));
        assert_eq!(branch(&submodule).as_deref(), Some("1f71d290"));
    }

    /// A `.git` file naming a directory with no `HEAD` has no branch, and
    /// the search stops there rather than reading a repository above.
    #[test]
    fn a_head_that_cannot_be_read_has_no_branch() {
        let root = TempDir::new().expect("a temporary directory should open");
        write(&root.path().join(".git/HEAD"), "ref: refs/heads/main\n");
        let broken = root.path().join("broken");
        write(&broken.join(".git"), "gitdir: /nowhere/at/all\n");

        assert_eq!(branch(&broken), None);
    }

    /// `~` and a path under it are read against the home directory, an
    /// absolute path as it stands, and a directory that could not be
    /// read, or `~` with no home directory known, as none.
    #[test]
    fn a_directory_label_is_read_back_against_home() {
        let home = Path::new("/home/natepiano");

        assert_eq!(directory_path("~", Some(home)), Some(home.to_path_buf()));
        assert_eq!(
            directory_path("~/rust/handler", Some(home)),
            Some(home.join("rust/handler"))
        );
        assert_eq!(
            directory_path("/etc/nixos", Some(home)),
            Some(PathBuf::from("/etc/nixos"))
        );
        assert_eq!(directory_path("—", Some(home)), None);
        assert_eq!(directory_path("~/rust", None), None);
        assert_eq!(directory_path("~other/rust", Some(home)), None);
    }
}
