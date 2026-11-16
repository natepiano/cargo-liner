//! Share native fixture preparation across the unit and integration tests.

use std::ffi::OsStr;
use std::fs::File;
use std::fs::OpenOptions;
use std::io;
use std::io::Write;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::sync::LazyLock;

/// Extension for the lock that keeps a native test fixture in use.
const NATIVE_FIXTURE_LOCK_EXTENSION: &str = "lock";

/// Keep the direct source or a prepared fixture with its shared lock.
enum RetainedNativeExecutable {
    /// The platform runs the build output directly.
    Direct(PathBuf),
    /// The shared lock prevents cleanup until this value is dropped.
    Locked { path: PathBuf, lock: File },
}

impl RetainedNativeExecutable {
    /// Read the path while this value retains the fixture lock.
    fn path(&self) -> &Path {
        match self {
            Self::Direct(path) | Self::Locked { path, .. } => path,
        }
    }
}

impl Drop for RetainedNativeExecutable {
    fn drop(&mut self) {
        if let Self::Locked { lock, .. } = self {
            let _ = lock.unlock();
        }
    }
}

/// Prepare and retain one real fixture executable for this test process.
pub(crate) fn executable() -> io::Result<&'static Path> {
    static EXECUTABLE: LazyLock<io::Result<RetainedNativeExecutable>> = LazyLock::new(|| {
        let source = Path::new(env!("CARGO_BIN_EXE_cargo-tile"));
        let metadata = source.metadata()?;
        let cache = dirs::cache_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join("cargo-tile/test-native-fixtures");
        let receipt = cache.join(format!(
            "{}-{}-{}-{}-{}-{}-{}",
            metadata.dev(),
            metadata.ino(),
            metadata.len(),
            metadata.mtime(),
            metadata.mtime_nsec(),
            metadata.ctime(),
            metadata.ctime_nsec(),
        ));
        if let Ok(bytes) = std::fs::read(&receipt) {
            let prepared = PathBuf::from(OsStr::from_bytes(&bytes));
            if prepared == source {
                return Ok(RetainedNativeExecutable::Direct(prepared));
            }
            if let Some(guard) = lock_fixture(&prepared)? {
                return Ok(RetainedNativeExecutable::Locked {
                    path: prepared,
                    lock: guard,
                });
            }
        }
        loop {
            let output = Command::new("python3")
                .args([
                    "-S",
                    concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/src/shim_registration/setup_reader.py"
                    ),
                    "native-fixture",
                ])
                .arg(env!("CARGO_BIN_EXE_cargo-tile"))
                .output()?;
            if !output.status.success() {
                return Err(io::Error::other(format!(
                    "native fixture preparation failed: {}: {}",
                    output.status,
                    String::from_utf8_lossy(&output.stderr),
                )));
            }
            let prepared = PathBuf::from(OsStr::from_bytes(
                output.stdout.strip_suffix(b"\n").unwrap_or(&output.stdout),
            ));
            let retained = if prepared == source {
                RetainedNativeExecutable::Direct(prepared)
            } else if let Some(guard) = lock_fixture(&prepared)? {
                RetainedNativeExecutable::Locked {
                    path: prepared,
                    lock: guard,
                }
            } else {
                continue;
            };
            std::fs::create_dir_all(&cache)?;
            let mut staged = tempfile::NamedTempFile::new_in(&cache)?;
            staged.write_all(retained.path().as_os_str().as_bytes())?;
            staged.persist(&receipt)?;
            return Ok(retained);
        }
    });
    EXECUTABLE
        .as_ref()
        .map(RetainedNativeExecutable::path)
        .map_err(|error| io::Error::new(error.kind(), error.to_string()))
}

/// Keep the fixture while its lock path still names the locked inode.
fn lock_fixture(prepared: &Path) -> io::Result<Option<File>> {
    let lock = prepared.with_extension(NATIVE_FIXTURE_LOCK_EXTENSION);
    let guard = match OpenOptions::new().read(true).write(true).open(&lock) {
        Ok(guard) => guard,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    guard.lock_shared()?;
    let current = match lock.metadata() {
        Ok(current) => current,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let locked = guard.metadata()?;
    if locked.dev() != current.dev() || locked.ino() != current.ino() || !prepared.is_file() {
        return Ok(None);
    }
    Ok(Some(guard))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io;
    use std::path::Path;
    use std::process::Command;

    use super::NATIVE_FIXTURE_LOCK_EXTENSION;
    use super::lock_fixture;

    /// Run the real cleanup against the private cache while Rust owns its reader lock.
    #[test]
    fn a_rust_reader_lock_retains_a_stale_native_fixture_until_release() -> io::Result<()> {
        let cache = tempfile::tempdir()?;
        let fixture = cache.path().join("a".repeat(64));
        let record = fixture.with_extension("source");
        let lock = fixture.with_extension(NATIVE_FIXTURE_LOCK_EXTENSION);
        fs::write(&fixture, b"fixture bytes")?;
        fs::write(&record, b"source that no longer exists")?;
        fs::write(&lock, b"")?;
        let guard = lock_fixture(&fixture)?
            .ok_or_else(|| io::Error::other("the private fixture must have a reader lock"))?;
        retire_native_cache(cache.path())?;
        assert_eq!(fs::read(&fixture)?, b"fixture bytes");
        assert_eq!(fs::read(&record)?, b"source that no longer exists");
        assert!(lock.is_file());
        drop(guard);
        retire_native_cache(cache.path())?;
        assert!(!fixture.exists());
        assert!(!record.exists());
        assert!(!lock.exists());
        Ok(())
    }

    /// A failed receipt lookup must not create a lock without a fixture.
    #[test]
    fn a_native_fixture_lookup_does_not_create_a_missing_lock() -> io::Result<()> {
        let cache = tempfile::tempdir()?;
        let fixture = cache.path().join("b".repeat(64));
        let lock = fixture.with_extension(NATIVE_FIXTURE_LOCK_EXTENSION);
        assert!(lock_fixture(&fixture)?.is_none());
        assert!(!lock.exists());
        fs::write(&fixture, b"fixture without a lock")?;
        assert!(lock_fixture(&fixture)?.is_none());
        assert!(!lock.exists());
        Ok(())
    }

    /// Use the retained Python case entry without the user cache or native preparation tools.
    fn retire_native_cache(cache: &Path) -> io::Result<()> {
        let output = Command::new("python3")
            .args([
                "-S",
                concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/src/shim_registration/reader_staging_cases.py"
                ),
                "retire-native-cache",
            ])
            .arg(cache)
            .output()?;
        if !output.status.success() {
            return Err(io::Error::other(
                String::from_utf8_lossy(&output.stderr).into_owned(),
            ));
        }
        Ok(())
    }
}
