//! A directory tree held in memory, so a fixture built once can start several cases.
//!
//! A test that runs the same scenario under several cases pays for the shared
//! setup in every case when each builds its own repository. `DirectorySnapshot`
//! captures what that setup left under one root and writes it back before each
//! case, with no processes. Git records a linked worktree and its repository by
//! absolute path, so a snapshot is restored at the root it was captured from,
//! unless `DirectorySnapshot::replace_text` first rewrites each recorded path.

use std::env;
use std::fs;
use std::fs::File;
use std::fs::Permissions;
use std::hash::DefaultHasher;
use std::hash::Hash;
use std::hash::Hasher;
use std::io::ErrorKind;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Stdio;

use crate::repository_template;

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
                    if permissions.mode() & 0o111 != 0
                        && contents.starts_with(b"#!/bin/sh\n# cargo-berth managed hook:")
                    {
                        ImmutableShellExecutable::prepare(contents, permissions.clone())
                            .link(&absolute);
                    } else {
                        fs::write(&absolute, contents).expect("snapshot file should restore");
                        fs::set_permissions(&absolute, permissions.clone())
                            .expect("snapshot file permissions should restore");
                    }
                },
            }
        }
    }
}

/// An executable inode owned by the fixture cache and never rewritten by a copy.
/// Callers replacing an installed hook must unlink its private name before writing.
struct ImmutableShellExecutable {
    source: PathBuf,
}

impl ImmutableShellExecutable {
    fn prepare(contents: &[u8], permissions: Permissions) -> Self {
        let mut identity = DefaultHasher::new();
        contents.hash(&mut identity);
        permissions.mode().hash(&mut identity);
        let binary = env::current_exe().expect("fixture executable should resolve");
        let cache = repository_template::fixture_cache_root(&binary).join("executables");
        fs::create_dir_all(&cache).expect("executable cache should exist");
        let source = cache.join(format!("{:016x}", identity.finish()));
        let lock = File::create(source.with_extension("lock"))
            .expect("executable preparation lock should open");
        lock.lock()
            .expect("one preparer should own each executable");
        if source.exists() {
            assert_eq!(
                fs::read(&source).expect("shared executable should read"),
                contents,
                "an immutable executable was modified"
            );
        } else {
            fs::write(&source, contents).expect("shared executable should write once");
            fs::set_permissions(&source, permissions).expect("shared executable should execute");
        }
        let ready = source.with_extension("ready");
        if !ready.exists() {
            let warmed = Command::new(&source)
                .arg("preparing")
                .env("CARGO_BERTH_BYPASS", "1")
                .env("CARGO_BERTH_TEST_EXECUTABLE_WARMUP", "1")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .expect("shared executable should prepare");
            assert!(warmed.success(), "shared executable preparation failed");
            File::create(ready).expect("executable preparation should record");
        }
        Self { source }
    }

    fn link(&self, destination: &Path) {
        if destination.exists() {
            fs::remove_file(destination).expect("previous executable name should unlink");
        }
        fs::hard_link(&self.source, destination)
            .or_else(|error| {
                if error.kind() == ErrorKind::CrossesDevices {
                    fs::copy(&self.source, destination).map(|_| ())
                } else {
                    Err(error)
                }
            })
            .expect("fixture should link its executable or copy it across filesystems");
    }
}

impl DirectorySnapshot {
    /// Install a shared shell script whose first execution is paid once per body.
    ///
    /// The script body is immutable. Replacing this path unlinks the old name;
    /// mutable arguments belong in environment variables or private data files.
    ///
    /// # Panics
    /// Panics if the executable cannot be prepared, linked or copied.
    pub fn write_shared_shell_script(path: &Path, script: &str) {
        let body = script
            .strip_prefix("#!/bin/sh\n")
            .expect("a shared shell script should name /bin/sh");
        let guarded = format!(
            "#!/bin/sh\nif [ \"${{CARGO_BERTH_TEST_EXECUTABLE_WARMUP:-}}\" = 1 ]; then exit 0; fi\n{body}"
        );
        ImmutableShellExecutable::prepare(guarded.as_bytes(), Permissions::from_mode(0o755))
            .link(path);
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::MetadataExt;

    use super::*;

    #[test]
    fn shared_script_replacement_keeps_its_other_copy_and_mutable_files_private() {
        let root = crate::fixture_tempdir("shared-script-");
        let first = root.path().join("first");
        let second = root.path().join("second");
        let script = "#!/bin/sh\nprintf '%s' original\n";
        DirectorySnapshot::write_shared_shell_script(&first, script);
        DirectorySnapshot::write_shared_shell_script(&second, script);
        assert_eq!(
            fs::metadata(&first).expect("first inode").ino(),
            fs::metadata(&second).expect("second inode").ino()
        );
        DirectorySnapshot::write_shared_shell_script(
            &first,
            "#!/bin/sh\nprintf '%s' replacement\n",
        );
        assert_eq!(
            Command::new(&first)
                .output()
                .expect("replacement should run")
                .stdout,
            b"replacement"
        );
        assert_eq!(
            Command::new(&second)
                .output()
                .expect("original should run")
                .stdout,
            b"original"
        );
        let source = tempfile::tempdir().expect("snapshot source should exist");
        fs::write(source.path().join("mutable"), "before").expect("mutable should write");
        let snapshot = DirectorySnapshot::capture(source.path());
        let copy = tempfile::tempdir().expect("snapshot copy should exist");
        snapshot.restore(copy.path());
        fs::write(copy.path().join("mutable"), "after").expect("copy should change");
        assert_eq!(
            fs::read_to_string(source.path().join("mutable")).expect("source should read"),
            "before"
        );
    }
}
