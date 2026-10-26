//! Scratch repositories shared by the unit tests of this module's submodules.

use cargo_berth_test_support::CommandDeadlineExt as _;
use tempfile::TempDir;
use tempfile::tempdir;

/// Initialize an empty git repository in a temporary directory.
pub(super) fn scratch_repository() -> TempDir {
    let repository = tempdir().expect("temporary repository should exist");
    let git_init = cargo_berth_test_support::fixture_git_command()
        .args(["init", "--quiet"])
        .current_dir(repository.path())
        .bounded_status()
        .expect("git should initialize a scratch repository");
    assert!(git_init.success());
    repository
}
