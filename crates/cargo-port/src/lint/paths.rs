use std::path::Path;
use std::path::PathBuf;

use crate::cache_paths;
use crate::constants::PORT_REPORT_HISTORY_JSONL;
use crate::constants::PORT_REPORT_LATEST_JSON;

/// Canonical cache directory for all per-project lint status files.
pub fn cache_root() -> PathBuf { cache_paths::port_report_root() }

/// Stable per-project cache key used by both cargo-port and external scripts.
pub fn project_key(project_root: &Path) -> String {
    let mut encoded = String::new();
    for byte in project_root.to_string_lossy().as_bytes() {
        use std::fmt::Write as _;
        let _ = write!(&mut encoded, "{byte:02x}");
    }
    encoded
}

/// Cache-rooted directory for the project's lint watcher protocol files.
pub fn project_dir(project_root: &Path) -> PathBuf { cache_root().join(project_key(project_root)) }

/// Cache-rooted directory for the project's lint watcher protocol files under
/// an explicit cache root.
pub fn project_dir_under(cache_root: &Path, project_root: &Path) -> PathBuf {
    cache_root.join(project_key(project_root))
}

/// Cache-rooted raw command output directory for the project under an explicit
/// cache root.
pub fn output_dir_under(cache_root: &Path, project_root: &Path) -> PathBuf {
    project_dir_under(cache_root, project_root).join("port-report")
}

pub fn latest_path_under(cache_root: &Path, project_root: &Path) -> PathBuf {
    project_dir_under(cache_root, project_root).join(PORT_REPORT_LATEST_JSON)
}

pub fn history_path_under(cache_root: &Path, project_root: &Path) -> PathBuf {
    project_dir_under(cache_root, project_root).join(PORT_REPORT_HISTORY_JSONL)
}
