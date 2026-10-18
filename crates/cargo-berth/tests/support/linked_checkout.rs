//! Administrative directory pointers for linked-checkout fixtures.

use std::fs;
use std::path::Path;
use std::path::PathBuf;

/// Resolve the administrative directory named by a linked checkout's Git file.
pub(crate) fn administrative_directory(root: &Path) -> PathBuf {
    let git_file =
        fs::read_to_string(root.join(".git")).expect("linked checkout's Git file should read");
    let path = PathBuf::from(
        git_file
            .trim()
            .strip_prefix("gitdir: ")
            .expect("linked checkout should name its administrative directory"),
    );
    if path.is_absolute() {
        path
    } else {
        root.join(path)
    }
}
