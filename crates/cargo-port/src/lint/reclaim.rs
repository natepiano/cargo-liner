use std::path::Path;

use super::cache_size_index;
use super::history;
use super::paths;

/// Reclaim a project's lint cache directory. Best-effort: silent
/// on missing or locked paths. Called from the dismiss flow when
/// the project at `project_root` is gone from disk so a future
/// worktree/branch reusing this exact path starts clean.
pub(super) fn reclaim_project_cache(project_root: &Path) {
    reclaim_project_cache_under(paths::cache_root().as_path(), project_root);
}

/// Cache-root-explicit variant of [`reclaim_project_cache`].
/// Matches the `_under` pattern used by the rest of the module so
/// tests can target a tempdir.
pub(super) fn reclaim_project_cache_under(cache_root: &Path, project_root: &Path) {
    let project_dir = paths::project_dir_under(cache_root, project_root);
    let bytes = history::project_dir_bytes(&project_dir);
    if std::fs::remove_dir_all(&project_dir).is_ok() && bytes > 0 {
        cache_size_index::adjust(cache_root, -i64::try_from(bytes).unwrap_or(i64::MAX));
    }
}
