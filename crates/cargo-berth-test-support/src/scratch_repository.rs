//! The uninitialized scratch history shared by ledger and drift fixtures.

use std::fs;
use std::path::Path;

use crate::GitDriver;

/// Build the main branch with one scratch commit under the caller's Git policy.
///
/// # Panics
/// Panics if a command fails or the base file cannot be written.
pub fn build_scratch_repository(root: &Path, git: GitDriver) {
    git.run(root, ["init", "--quiet", "--initial-branch", "main"]);
    git.run(root, ["config", "user.email", "test@example.invalid"]);
    git.run(root, ["config", "user.name", "Berth Test"]);
    fs::write(root.join("README.md"), "scratch\n").expect("base file should write");
    git.run(root, ["add", "README.md"]);
    git.run(root, ["commit", "--quiet", "-m", "initial"]);
}
