use std::fs::File;
use std::io;
use std::io::BufRead;
use std::io::BufReader;
use std::io::ErrorKind;
use std::path::Path;

use super::cache_size_index;
use super::paths;
use super::types::LintRun;
#[cfg(test)]
use super::types::LintRunStatus;

pub fn write_latest_under(cache_root: &Path, project_root: &Path, run: &LintRun) -> io::Result<()> {
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

#[cfg(test)]
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
