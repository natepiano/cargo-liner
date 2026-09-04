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
    // Git runs `maintenance run --auto --detach` after a commit, and this machine leaves that
    // default on. Several detached runs then repack one repository at once, and the geometric
    // repack deletes a pack a commit still in flight is reading, which surfaces as
    // `invalid object <oid> for '<path>'` from a commit that did nothing wrong. A fixture
    // repository is short-lived and never needs maintenance, so it opts out of both schedulers.
    for setting in [["maintenance.auto", "false"], ["gc.auto", "0"]] {
        let configured = Command::new("git")
            .args(["config"])
            .args(setting)
            .current_dir(repository.path())
            .status()
            .expect("git should configure a scratch repository");
        assert!(configured.success());
    }
    repository
}
