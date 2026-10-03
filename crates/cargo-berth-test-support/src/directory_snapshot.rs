//! A directory tree held in memory, so a fixture built once can start several cases.
//!
//! A test that runs the same scenario under several cases pays for the shared
//! setup in every case when each builds its own repository. `DirectorySnapshot`
//! captures what that setup left under one root and writes it back before each
//! case, with no processes. Git records a linked worktree and its repository by
//! absolute path, so a snapshot is restored at the root it was captured from,
//! unless `DirectorySnapshot::replace_text` first rewrites each recorded path.

use std::fs;
use std::fs::Permissions;
use std::path::Path;
use std::path::PathBuf;

/// The files and directories under one root, with each file's permissions.
pub struct DirectorySnapshot {
    entries: Vec<SnapshotEntry>,
}

/// One path relative to a `DirectorySnapshot` root; a directory precedes its entries.
enum SnapshotEntry {
    Directory(PathBuf),
    File {
        path:        PathBuf,
        contents:    Vec<u8>,
        permissions: Permissions,
    },
}

impl DirectorySnapshot {
    /// Capture every file and directory under `root`.
    ///
    /// # Panics
    ///
    /// Panics when an entry cannot be read, or when `root` holds anything other
    /// than files and directories.
    #[must_use]
    pub fn capture(root: &Path) -> Self {
        let mut entries = Vec::new();
        Self::capture_directory(root, Path::new(""), &mut entries);
        Self { entries }
    }

    fn capture_directory(root: &Path, relative: &Path, entries: &mut Vec<SnapshotEntry>) {
        for entry in fs::read_dir(root.join(relative)).expect("snapshot directory should read") {
            let entry = entry.expect("snapshot entry should read");
            let path = relative.join(entry.file_name());
            let file_type = entry.file_type().expect("snapshot entry type should read");
            if file_type.is_dir() {
                entries.push(SnapshotEntry::Directory(path.clone()));
                Self::capture_directory(root, &path, entries);
            } else {
                assert!(
                    file_type.is_file(),
                    "a snapshot holds only files and directories: {}",
                    path.display()
                );
                let absolute = root.join(&path);
                entries.push(SnapshotEntry::File {
                    contents: fs::read(&absolute).expect("snapshot file should read"),
                    permissions: fs::metadata(&absolute)
                        .expect("snapshot file metadata should read")
                        .permissions(),
                    path,
                });
            }
        }
    }

    /// Replace `from` with `to` in every captured file that contains it.
    ///
    /// Returns the path of each file it changed, relative to the captured root.
    ///
    /// # Panics
    ///
    /// Panics when a file that contains `from` is not UTF-8 text.
    #[must_use]
    pub fn replace_text(&mut self, from: &str, to: &str) -> Vec<PathBuf> {
        self.entries
            .iter_mut()
            .filter_map(|entry| match entry {
                SnapshotEntry::File { path, contents, .. }
                    if contents
                        .windows(from.len())
                        .any(|window| window == from.as_bytes()) =>
                {
                    let replaced = str::from_utf8(contents)
                        .expect("a file that holds replaced text should be UTF-8")
                        .replace(from, to);
                    *contents = replaced.into_bytes();
                    Some(path.clone())
                },
                SnapshotEntry::File { .. } | SnapshotEntry::Directory(_) => None,
            })
            .collect()
    }

    /// Replace everything under `root` with the captured entries.
    ///
    /// # Panics
    ///
    /// Panics when the current contents of `root` cannot be removed or a captured
    /// entry cannot be written.
    pub fn restore(&self, root: &Path) {
        for entry in fs::read_dir(root).expect("snapshot root should read") {
            let entry = entry.expect("snapshot root entry should read");
            if entry
                .file_type()
                .expect("snapshot root entry type should read")
                .is_dir()
            {
                fs::remove_dir_all(entry.path())
            } else {
                fs::remove_file(entry.path())
            }
            .expect("previous snapshot contents should remove");
        }
        for entry in &self.entries {
            match entry {
                SnapshotEntry::Directory(path) => {
                    fs::create_dir(root.join(path)).expect("snapshot directory should restore");
                },
                SnapshotEntry::File {
                    path,
                    contents,
                    permissions,
                } => {
                    let absolute = root.join(path);
                    fs::write(&absolute, contents).expect("snapshot file should restore");
                    fs::set_permissions(&absolute, permissions.clone())
                        .expect("snapshot file permissions should restore");
                },
            }
        }
    }
}
