use std::fs::File;
use std::io;
use std::io::BufRead;
use std::io::BufReader;
use std::io::ErrorKind;
use std::path::Path;

use super::cache_size_index;
use super::paths;
use super::run::LintRun;
use super::run::LintRunStatus;

pub fn write_latest_under(cache_root: &Path, project_root: &Path, run: &LintRun) -> io::Result<()> {
    #[cfg(test)]
    paths::assert_not_default_user_cache_root(cache_root);

    let path = paths::latest_path_under(cache_root, project_root);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_vec_pretty(run)
        .map_err(|err| io::Error::new(ErrorKind::InvalidData, err))?;
    let tmp_path = path.with_extension("json.tmp");
    let old_size = cache_size_index::file_size_or_zero(&path);
    std::fs::write(&tmp_path, json)?;
    std::fs::rename(tmp_path, &path)?;
    let new_size = cache_size_index::file_size_or_zero(&path);
    cache_size_index::apply_write_delta(cache_root, old_size, new_size);
    Ok(())
}

pub fn clear_latest_under(cache_root: &Path, project_root: &Path) -> io::Result<()> {
    #[cfg(test)]
    paths::assert_not_default_user_cache_root(cache_root);

    let path = paths::latest_path_under(cache_root, project_root);
    let old_size = cache_size_index::file_size_or_zero(&path);
    match std::fs::remove_file(&path) {
        Ok(()) => {
            cache_size_index::apply_write_delta(cache_root, old_size, 0);
            Ok(())
        },
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err),
    }
}

/// Remove `latest.json` only if it is still marked `Running`. Used to
/// finalize a run that never reached a terminal write (the worker was joined
/// mid-command on shutdown, an early return, or a panic) without clobbering a
/// completed run. Returns whether a `Running` marker was cleared.
pub fn clear_latest_if_running_under(cache_root: &Path, project_root: &Path) -> io::Result<bool> {
    let path = paths::latest_path_under(cache_root, project_root);
    let Some(run) = read_latest_file(&path) else {
        return Ok(false);
    };
    if matches!(run.status, LintRunStatus::Running) {
        clear_latest_under(cache_root, project_root)?;
        return Ok(true);
    }
    Ok(false)
}

pub fn read_latest_file(path: &Path) -> Option<LintRun> {
    let file = File::open(path).ok()?;
    serde_json::from_reader(file).ok()
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

    fn run(status: LintRunStatus) -> LintRun {
        LintRun {
            run_id: "run-1".to_string(),
            started_at: "2026-03-30T14:22:01-05:00".to_string(),
            finished_at: Some("2026-03-30T14:22:18-05:00".to_string()),
            duration_ms: Some(17_000),
            status,
            commands: Vec::new(),
            archive_bytes: 0,
        }
    }

    #[test]
    #[should_panic(expected = "tests must write lint artifacts under a temp cache root")]
    fn writes_reject_default_user_cache_root() {
        let project_dir = tempfile::tempdir().expect("tempdir");
        let default_lint_root = cache_paths::default_app_cache_root().join(LINTS_CACHE_DIR);

        write_latest_under(
            default_lint_root.as_path(),
            project_dir.path(),
            &run(LintRunStatus::Passed),
        )
        .expect("guard should panic before write_latest returns");
    }

    #[test]
    fn clear_latest_if_running_removes_running_latest() {
        let cache_dir = tempfile::tempdir().expect("tempdir");
        let project_dir = tempfile::tempdir().expect("tempdir");
        let mut running = run(LintRunStatus::Running);
        running.started_at = chrono::Utc::now().format("%+").to_string();
        running.finished_at = None;
        write_latest_under(cache_dir.path(), project_dir.path(), &running).expect("write latest");

        let cleared =
            clear_latest_if_running_under(cache_dir.path(), project_dir.path()).expect("clear");

        assert!(cleared);
        assert!(!paths::latest_path_under(cache_dir.path(), project_dir.path()).exists());
    }
}
