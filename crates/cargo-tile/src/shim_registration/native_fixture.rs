//! Share native fixture preparation across the unit and integration tests.

use std::ffi::OsStr;
use std::io;
use std::io::Write;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::sync::LazyLock;

/// Prepare and retain one real fixture executable for this test process.
pub(crate) fn executable() -> io::Result<&'static Path> {
    static EXECUTABLE: LazyLock<io::Result<PathBuf>> = LazyLock::new(|| {
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
            if prepared.is_file() {
                return Ok(prepared);
            }
        }
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
        std::fs::create_dir_all(&cache)?;
        let mut staged = tempfile::NamedTempFile::new_in(cache)?;
        staged.write_all(prepared.as_os_str().as_bytes())?;
        staged.persist(receipt)?;
        Ok(prepared)
    });
    EXECUTABLE
        .as_ref()
        .map(PathBuf::as_path)
        .map_err(|error| io::Error::new(error.kind(), error.to_string()))
}
