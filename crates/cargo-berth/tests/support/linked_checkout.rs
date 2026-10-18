//! Resolve linked checkout administration before comparing repository paths.

use std::fs;
use std::path::Path;
use std::path::PathBuf;

/// Git's linked administration and common directory in one resolved spelling.
pub(crate) struct ResolvedLinkedAdministration {
    directory:        PathBuf,
    common_directory: PathBuf,
}

impl ResolvedLinkedAdministration {
    /// Read Git's pointer and resolve both administrative paths at the boundary.
    pub(crate) fn read(root: &Path) -> Self {
        let git_file =
            fs::read_to_string(root.join(".git")).expect("linked checkout's Git file should read");
        let pointer = Path::new(
            git_file
                .trim()
                .strip_prefix("gitdir: ")
                .expect("linked checkout should name its administrative directory"),
        );
        let directory =
            fs::canonicalize(root.join(pointer)).expect("linked administration should resolve");
        let common = fs::read_to_string(directory.join("commondir"))
            .expect("linked administration should name its common directory");
        let common_directory = fs::canonicalize(directory.join(common.trim()))
            .expect("common administration should resolve");
        Self {
            directory,
            common_directory,
        }
    }

    /// The linked administration relative to its resolved common directory.
    pub(crate) fn locator(&self) -> &Path {
        self.directory
            .strip_prefix(&self.common_directory)
            .expect("linked administration should be in common git")
    }
}

/// Resolve the administrative directory named by a linked checkout's Git file.
pub(crate) fn administrative_directory(root: &Path) -> PathBuf {
    let administration = ResolvedLinkedAdministration::read(root);
    administration
        .common_directory
        .join(administration.locator())
}
