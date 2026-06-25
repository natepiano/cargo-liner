use std::path::Path;

use sha2::Digest as _;

use crate::cache_paths;
#[cfg(test)]
use crate::constants::LINTS_CACHE_DIR;
use crate::constants::LINTS_HISTORY_JSONL;
use crate::constants::LINTS_LATEST_JSON;
use crate::project::AbsolutePath;

/// Canonical cache directory for all per-project lint status files.
pub fn cache_root() -> AbsolutePath { cache_paths::lint_runs_root() }

#[cfg(test)]
pub(super) fn assert_not_default_user_cache_root(cache_root: &Path) {
    let default_lint_root = cache_paths::default_app_cache_root().join(LINTS_CACHE_DIR);
    assert_ne!(
        cache_root,
        default_lint_root.as_path(),
        "tests must write lint artifacts under a temp cache root, not {}",
        default_lint_root.display(),
    );
}

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

/// Render an id into a single path segment that is legal on every platform:
/// keep ASCII alphanumerics plus `-`, `.`, `_`; replace anything else with `-`.
/// A lint `run_id` is used verbatim as the `runs/{run_id}` archive directory
/// name, and the default id is an RFC3339 timestamp whose `:` separators are
/// illegal in Windows paths — so the id must be path-safe where it is created.
pub fn sanitize_run_id(id: &str) -> String {
    id.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_') {
                c
            } else {
                '-'
            }
        })
        .collect()
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
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_run_id_replaces_path_illegal_chars() {
        // RFC3339 colons (illegal on Windows) become dashes; digits, dots,
        // and existing dashes survive, so the id stays unique per timestamp.
        assert_eq!(
            sanitize_run_id("2026-05-25T17:20:44.592-04:00"),
            "2026-05-25T17-20-44.592-04-00"
        );
        assert_eq!(sanitize_run_id("run-abc"), "run-abc");
    }

    #[test]
    fn project_key_matches_shasum_cli() {
        // echo -n "/Users/natemccoy/rust/cargo-mend" | shasum -a 256 | cut -c1-16
        // => c76947976a369618
        let key = project_key(Path::new("/Users/natemccoy/rust/cargo-mend"));
        assert_eq!(key, "cargo-mend-c76947976a369618");
    }

    #[test]
    fn cache_latest_path_does_not_live_under_project_dir() {
        let cache_dir = tempfile::tempdir().expect("tempdir");
        let dir = tempfile::tempdir().expect("tempdir");
        let path = latest_path_under(cache_dir.path(), dir.path());
        assert!(
            !path.starts_with(dir.path()),
            "cache latest path should not recreate project directories"
        );
    }
}
