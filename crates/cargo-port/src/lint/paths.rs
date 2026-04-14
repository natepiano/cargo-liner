use std::path::Path;

use crate::cache_paths;
use crate::constants::LINTS_HISTORY_JSONL;
use crate::constants::LINTS_LATEST_JSON;
use crate::project::AbsolutePath;

/// Canonical cache directory for all per-project lint status files.
pub fn cache_root() -> AbsolutePath { cache_paths::lint_runs_root() }

/// Stable per-project cache key: `{name}-{sha256_prefix}` where name is the
/// last path component and the suffix is the first 16 hex chars of the SHA-256
/// digest of the full path. This is trivially reproducible in any language:
///
/// ```bash
/// echo -n "/path/to/project" | shasum -a 256 | cut -c1-16
/// ```
///
/// SYNC: must match `project_key()` in `~/.claude/scripts/clippy/check_cache.sh`.
pub fn project_key(project_root: &Path) -> String {
    use sha2::Digest as _;

    let path_str = project_root.to_string_lossy();
    let name = project_root
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("project");

    let digest = sha2::Sha256::digest(path_str.as_bytes());
    let hex = digest
        .iter()
        .take(8)
        .fold(String::with_capacity(16), |mut acc, b| {
            use std::fmt::Write as _;
            let _ = write!(acc, "{b:02x}");
            acc
        });

    format!("{name}-{hex}")
}

/// Cache-rooted directory for the project's lint watcher protocol files.
pub fn project_dir(project_root: &Path) -> AbsolutePath {
    cache_root().join(project_key(project_root)).into()
}

/// Cache-rooted directory for the project's lint watcher protocol files under
/// an explicit cache root.
pub fn project_dir_under(cache_root: &Path, project_root: &Path) -> AbsolutePath {
    cache_root.join(project_key(project_root)).into()
}

/// Cache-rooted raw command output directory for the project under an explicit
/// cache root. This is the same as the project directory — command logs live
/// directly alongside `latest.json` and `history.jsonl`.
pub fn output_dir_under(cache_root: &Path, project_root: &Path) -> AbsolutePath {
    project_dir_under(cache_root, project_root)
}

pub fn latest_path_under(cache_root: &Path, project_root: &Path) -> AbsolutePath {
    project_dir_under(cache_root, project_root)
        .join(LINTS_LATEST_JSON)
        .into()
}

pub fn history_path_under(cache_root: &Path, project_root: &Path) -> AbsolutePath {
    project_dir_under(cache_root, project_root)
        .join(LINTS_HISTORY_JSONL)
        .into()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "tests")]
mod tests {
    use super::*;

    #[test]
    fn project_key_matches_shasum_cli() {
        // echo -n "/Users/natemccoy/rust/cargo-mend" | shasum -a 256 | cut -c1-16
        // => c76947976a369618
        let key = project_key(Path::new("/Users/natemccoy/rust/cargo-mend"));
        assert_eq!(key, "cargo-mend-c76947976a369618");
    }
}
