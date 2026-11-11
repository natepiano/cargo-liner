//! Private fixture directories owned by each test.

use std::env;
use std::fs;

use tempfile::TempDir;

use crate::repository_template;

/// Create a private directory under the test scratch root.
///
/// Nextest's scratch wrapper uses executable memory storage when available.
/// Outside that wrapper, this uses the cache's `private` directory.
/// Cached executables use hard links on the same filesystem and copies across filesystems.
/// The returned owner removes the directory on drop.
///
/// # Panics
/// Panics if the executable, fixture parent or private directory cannot be resolved.
#[must_use]
pub fn fixture_tempdir(prefix: &str) -> TempDir {
    let parent = env::var_os("CARGO_BERTH_TEST_SCRATCH_ROOT").map_or_else(
        || {
            let executable = env::current_exe().expect("fixture executable should resolve");
            repository_template::fixture_cache_root(&executable).join("private")
        },
        Into::into,
    );
    fs::create_dir_all(&parent).expect("private fixture parent should exist");
    tempfile::Builder::new()
        .prefix(prefix)
        .tempdir_in(parent)
        .expect("private fixture directory should exist under its scratch parent")
}
