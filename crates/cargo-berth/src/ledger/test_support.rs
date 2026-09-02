//! Scratch repositories shared by the unit tests of this module's submodules.

use std::process::Command;

use tempfile::TempDir;
use tempfile::tempdir;

/// Initialize an empty git repository in a temporary directory.
pub(super) fn scratch_repository() -> TempDir {
    let repository = tempdir().expect("temporary repository should exist");
    let git_init = Command::new("git")
        .args(["init", "--quiet"])
        .current_dir(repository.path())
        .status()
        .expect("git should initialize a scratch repository");
    assert!(git_init.success());
    repository
}
