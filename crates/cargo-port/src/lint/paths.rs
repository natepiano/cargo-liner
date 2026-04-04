use std::path::Path;
use std::path::PathBuf;

use crate::cache_paths;
use crate::constants::LINTS_HISTORY_JSONL;
use crate::constants::LINTS_LATEST_JSON;

/// Canonical cache directory for all per-project lint status files.
pub fn cache_root() -> PathBuf { cache_paths::lint_runs_root() }

/// Stable per-project cache key: `{name}-{hash}` where name is the last
/// path component and hash is 8 hex chars derived from the full path.
pub fn project_key(project_root: &Path) -> String {
    use std::hash::Hash as _;
    use std::hash::Hasher as _;

    let path_str = project_root.to_string_lossy();
    let name = project_root
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("project");

    let mut hasher = std::hash::DefaultHasher::new();
    path_str.hash(&mut hasher);
    let hash = hasher.finish();

    format!("{name}-{hash:08x}")
}

/// Cache-rooted directory for the project's lint watcher protocol files.
pub fn project_dir(project_root: &Path) -> PathBuf { cache_root().join(project_key(project_root)) }

/// Cache-rooted directory for the project's lint watcher protocol files under
/// an explicit cache root.
pub fn project_dir_under(cache_root: &Path, project_root: &Path) -> PathBuf {
    cache_root.join(project_key(project_root))
}

/// Cache-rooted raw command output directory for the project under an explicit
/// cache root. This is the same as the project directory — command logs live
/// directly alongside `latest.json` and `history.jsonl`.
pub fn output_dir_under(cache_root: &Path, project_root: &Path) -> PathBuf {
    project_dir_under(cache_root, project_root)
}

pub fn latest_path_under(cache_root: &Path, project_root: &Path) -> PathBuf {
    project_dir_under(cache_root, project_root).join(LINTS_LATEST_JSON)
}

pub fn history_path_under(cache_root: &Path, project_root: &Path) -> PathBuf {
    project_dir_under(cache_root, project_root).join(LINTS_HISTORY_JSONL)
}
