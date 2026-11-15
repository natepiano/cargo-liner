use std::fs::File;
use std::io;
use std::io::BufRead;
use std::io::BufReader;
use std::io::ErrorKind;
use std::path::Path;

use super::cache_size_index;
use super::paths;
use super::run::LintRun;

/// A `latest.json` read: the run it holds, or no record that can be used.
pub(super) enum LatestLintRecord {
    /// The file opened and decoded as a `LintRun`.
    Readable(LintRun),
    /// The file is missing, cannot be opened, or does not decode.
    Unavailable,
}

pub(super) fn write_latest_bytes_under(
    cache_root: &Path,
    project_root: &Path,
    bytes: &[u8],
) -> io::Result<()> {
    #[cfg(test)]
    paths::assert_not_default_user_cache_root(cache_root);

    let path = paths::latest_path_under(cache_root, project_root);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp_path = path.with_extension("json.tmp");
    let old_size = tui_pane::file_size_or_zero(&path);
    std::fs::write(&tmp_path, bytes)?;
    std::fs::rename(tmp_path, &path)?;
    let new_size = tui_pane::file_size_or_zero(&path);
    cache_size_index::apply_write_delta(cache_root, old_size, new_size);
    Ok(())
}

pub fn clear_latest_under(cache_root: &Path, project_root: &Path) -> io::Result<()> {
    #[cfg(test)]
    paths::assert_not_default_user_cache_root(cache_root);

    let path = paths::latest_path_under(cache_root, project_root);
    let old_size = tui_pane::file_size_or_zero(&path);
    match std::fs::remove_file(&path) {
        Ok(()) => {
            cache_size_index::apply_write_delta(cache_root, old_size, 0);
            Ok(())
        },
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err),
    }
}

pub fn read_latest_file(path: &Path) -> LatestLintRecord {
    let Ok(file) = File::open(path) else {
        return LatestLintRecord::Unavailable;
    };
    serde_json::from_reader(file).map_or(LatestLintRecord::Unavailable, LatestLintRecord::Readable)
}

pub fn read_history_file(path: &Path) -> Vec<LintRun> {
    let Ok(file) = File::open(path) else {
        return Vec::new();
    };
    let reader = BufReader::new(file);
    reader
        .lines()
        .map_while(Result::ok)
        .filter_map(|line| serde_json::from_str::<LintRun>(&line).ok())
        .collect()
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use super::*;
    use crate::cache_paths;
    use crate::constants::LINTS_CACHE_DIR;

    #[test]
    #[should_panic(expected = "tests must write lint artifacts under a temp cache root")]
    fn writes_reject_default_user_cache_root() {
        let project_dir = tempfile::tempdir().expect("tempdir");
        let default_lint_root = cache_paths::default_app_cache_root().join(LINTS_CACHE_DIR);

        write_latest_bytes_under(default_lint_root.as_path(), project_dir.path(), b"{}")
            .expect("guard should panic before write_latest returns");
    }
}
