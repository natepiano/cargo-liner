use std::fs::File;
use std::io;
use std::io::BufRead;
use std::io::BufReader;
use std::io::ErrorKind;
use std::path::Path;

use super::paths;
use super::types::LintRun;
use super::types::LintRunStatus;
use crate::constants::LINTS_HISTORY_JSONL;
use crate::constants::LINTS_LATEST_JSON;

pub fn write_latest_under(cache_root: &Path, project_root: &Path, run: &LintRun) -> io::Result<()> {
    let path = paths::latest_path_under(cache_root, project_root);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_vec_pretty(run)
        .map_err(|err| io::Error::new(ErrorKind::InvalidData, err))?;
    let tmp_path = path.with_extension("json.tmp");
    std::fs::write(&tmp_path, json)?;
    std::fs::rename(tmp_path, path)
}

pub fn clear_latest_under(cache_root: &Path, project_root: &Path) -> io::Result<()> {
    let path = paths::latest_path_under(cache_root, project_root);
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
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

pub fn clear_running_latest_files_under(cache_root: &Path) -> io::Result<usize> {
    let entries = match std::fs::read_dir(cache_root) {
        Ok(entries) => entries,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(0),
        Err(err) => return Err(err),
    };

    let mut cleared = 0;
    for entry in entries {
        let Ok(entry) = entry else {
            continue;
        };
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_dir() {
            continue;
        }

        let latest_path = entry.path().join(LINTS_LATEST_JSON);
        let Some(run) = read_latest_file(&latest_path) else {
            continue;
        };
        if !matches!(run.status, LintRunStatus::Running) {
            continue;
        }

        // If the app was killed mid-lint, `latest.json` is stuck at "running".
        // Deleting it loses the status icon until the next lint run completes.
        // Recover by replacing it with the last completed run from history.
        let history_path = entry.path().join(LINTS_HISTORY_JSONL);
        let last_completed = read_history_file(&history_path)
            .into_iter()
            .rev()
            .find(|r| !matches!(r.status, LintRunStatus::Running));
        if let Some(run) = last_completed {
            let json = serde_json::to_vec_pretty(&run)
                .map_err(|err| io::Error::new(ErrorKind::InvalidData, err))?;
            let tmp_path = latest_path.with_extension("json.tmp");
            std::fs::write(&tmp_path, json)?;
            std::fs::rename(tmp_path, &latest_path)?;
            cleared += 1;
        } else {
            match std::fs::remove_file(&latest_path) {
                Ok(()) => cleared += 1,
                Err(err) if err.kind() == ErrorKind::NotFound => {},
                Err(err) => return Err(err),
            }
        }
    }

    Ok(cleared)
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
