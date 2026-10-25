//! Private fixture directories on the shared executable cache's filesystem.

use std::env;
use std::fs;

use tempfile::TempDir;

use crate::repository_template;

/// Create a private directory whose cached executables can be hard-linked.
///
/// Unlike `tempfile::tempdir`, this ignores `TMPDIR`: the fixture cache and
/// its copies share a filesystem. The returned owner removes the directory
/// on drop; only the build-owned parent outlives a test.
///
/// # Panics
/// Panics if the executable, fixture parent or private directory cannot be resolved.
#[must_use]
pub fn fixture_tempdir(prefix: &str) -> TempDir {
    let executable = env::current_exe().expect("fixture executable should resolve");
    let parent = repository_template::fixture_cache_root(&executable).join("private");
    fs::create_dir_all(&parent).expect("private fixture parent should exist");
    tempfile::Builder::new()
        .prefix(prefix)
        .tempdir_in(parent)
        .expect("private fixture directory should exist on the cache filesystem")
}
