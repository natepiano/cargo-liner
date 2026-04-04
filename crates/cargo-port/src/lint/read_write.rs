use std::fs::File;
use std::io;
use std::io::BufRead;
use std::io::BufReader;
use std::path::Path;

use super::paths;
use super::types::LintRun;
use super::types::LintRunStatus;

pub fn write_latest_under(cache_root: &Path, project_root: &Path, run: &LintRun) -> io::Result<()> {
    let path = paths::latest_path_under(cache_root, project_root);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_vec_pretty(run)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    let tmp_path = path.with_extension("json.tmp");
    std::fs::write(&tmp_path, json)?;
    std::fs::rename(tmp_path, path)
}

pub fn clear_latest_under(cache_root: &Path, project_root: &Path) -> io::Result<()> {
    let path = paths::latest_path_under(cache_root, project_root);
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err),
    }
}

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
