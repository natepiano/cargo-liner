use std::collections::HashMap;
use std::fs::File;
use std::io;
use std::io::BufRead;
use std::io::BufReader;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::time::SystemTime;

use walkdir::WalkDir;

use super::paths;
use super::read_write;
use super::status;
use super::types::PortReportRun;
use crate::constants::PORT_REPORT_HISTORY_JSONL;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HistoryUsage {
    pub bytes:        u64,
    pub budget_bytes: Option<u64>,
}

#[derive(Debug)]
struct PrunableHistoryLine {
    path:       PathBuf,
    line_index: usize,
    sort_key:   i64,
    byte_len:   u64,
}

type HistoryFileLines = (PathBuf, Vec<String>);
type CollectedHistoryLines = (Vec<PrunableHistoryLine>, Vec<HistoryFileLines>);

pub fn retained_history_usage(history_budget_bytes: Option<u64>) -> HistoryUsage {
    retained_history_usage_under(&paths::cache_root(), history_budget_bytes)
}

pub(super) fn retained_history_usage_under(
    cache_root: &Path,
    history_budget_bytes: Option<u64>,
) -> HistoryUsage {
    HistoryUsage {
        bytes:        total_bytes_under(cache_root),
        budget_bytes: history_budget_bytes,
    }
}

pub fn append_history_under(
    cache_root: &Path,
    project_root: &Path,
    run: &PortReportRun,
    history_budget_bytes: Option<u64>,
) -> io::Result<()> {
    let path = paths::history_path_under(cache_root, project_root);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    let json = serde_json::to_string(run)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    writeln!(file, "{json}")?;
    enforce_history_budget_under(cache_root, history_budget_bytes)
}

pub fn read_history(project_root: &Path) -> Vec<PortReportRun> {
    read_history_under(&paths::cache_root(), project_root)
}

pub(super) fn read_history_under(cache_root: &Path, project_root: &Path) -> Vec<PortReportRun> {
    let mut runs =
        read_write::read_history_file(&paths::history_path_under(cache_root, project_root));
    let latest = read_write::read_latest_file(&paths::latest_path_under(cache_root, project_root));

    if let Some(latest_run) = latest
        && runs
            .last()
            .is_none_or(|run| run.run_id != latest_run.run_id)
    {
        runs.push(latest_run);
    }

    runs.reverse();
    runs
}

fn enforce_history_budget_under(
    cache_root: &Path,
    history_budget_bytes: Option<u64>,
) -> io::Result<()> {
    let Some(history_budget_bytes) = history_budget_bytes else {
        return Ok(());
    };
    prune_history_lines_under(cache_root, history_budget_bytes)?;
    prune_legacy_log_files_under(cache_root, history_budget_bytes)
}

pub(super) fn total_bytes_under(root: &Path) -> u64 {
    WalkDir::new(root)
        .into_iter()
        .filter_map(Result::ok)
        .filter_map(|entry| {
            entry
                .file_type()
                .is_file()
                .then(|| entry.metadata().ok().map(|metadata| metadata.len()))
                .flatten()
        })
        .sum()
}

fn history_line_sort_key(run: &PortReportRun) -> i64 {
    run.finished_at
        .as_deref()
        .and_then(status::parse_timestamp)
        .or_else(|| status::parse_timestamp(&run.started_at))
        .map_or(i64::MIN, |timestamp| timestamp.timestamp_millis())
}

fn collect_history_lines_under(cache_root: &Path) -> io::Result<CollectedHistoryLines> {
    let mut entries = Vec::new();
    let mut files = Vec::new();

    for history_path in WalkDir::new(cache_root)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .filter(|entry| entry.file_name() == PORT_REPORT_HISTORY_JSONL)
        .map(walkdir::DirEntry::into_path)
    {
        let file = File::open(&history_path)?;
        let reader = BufReader::new(file);
        let mut lines = Vec::new();
        for (line_index, line) in reader.lines().enumerate() {
            let line = line?;
            let byte_len = u64::try_from(line.len() + 1).unwrap_or(u64::MAX);
            let sort_key = serde_json::from_str::<PortReportRun>(&line)
                .map(|run| history_line_sort_key(&run))
                .unwrap_or(i64::MIN);
            entries.push(PrunableHistoryLine {
                path: history_path.clone(),
                line_index,
                sort_key,
                byte_len,
            });
            lines.push(line);
        }
        files.push((history_path, lines));
    }

    Ok((entries, files))
}

fn rewrite_history_file(path: &Path, lines: &[String], removed: &[bool]) -> io::Result<()> {
    let kept: Vec<&String> = lines
        .iter()
        .zip(removed.iter())
        .filter_map(|(line, is_removed)| (!*is_removed).then_some(line))
        .collect();

    if kept.is_empty() {
        match std::fs::remove_file(path) {
            Ok(()) => return Ok(()),
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(err) => return Err(err),
        }
    }

    let tmp_path = path.with_extension("jsonl.tmp");
    let mut file = File::create(&tmp_path)?;
    for line in kept {
        writeln!(file, "{line}")?;
    }
    std::fs::rename(tmp_path, path)
}

fn prune_history_lines_under(cache_root: &Path, history_budget_bytes: u64) -> io::Result<()> {
    let mut total_bytes = total_bytes_under(cache_root);
    if total_bytes <= history_budget_bytes {
        return Ok(());
    }

    let (mut entries, files) = collect_history_lines_under(cache_root)?;
    if entries.is_empty() {
        return Ok(());
    }

    entries.sort_unstable_by(|lhs, rhs| {
        lhs.sort_key
            .cmp(&rhs.sort_key)
            .then_with(|| lhs.path.cmp(&rhs.path))
            .then_with(|| lhs.line_index.cmp(&rhs.line_index))
    });

    let mut removed_by_path = HashMap::<PathBuf, Vec<bool>>::new();
    for (path, lines) in &files {
        removed_by_path.insert(path.clone(), vec![false; lines.len()]);
    }

    for entry in entries {
        if total_bytes <= history_budget_bytes {
            break;
        }
        let Some(removed) = removed_by_path.get_mut(&entry.path) else {
            continue;
        };
        if removed.get(entry.line_index).copied().unwrap_or(false) {
            continue;
        }
        removed[entry.line_index] = true;
        total_bytes = total_bytes.saturating_sub(entry.byte_len);
    }

    for (path, lines) in files {
        let Some(removed) = removed_by_path.get(&path) else {
            continue;
        };
        if removed.iter().all(|is_removed| !*is_removed) {
            continue;
        }
        rewrite_history_file(&path, &lines, removed)?;
    }

    Ok(())
}

fn prune_legacy_log_files_under(cache_root: &Path, history_budget_bytes: u64) -> io::Result<()> {
    let mut total_bytes = total_bytes_under(cache_root);
    if total_bytes <= history_budget_bytes {
        return Ok(());
    }

    let mut logs: Vec<(PathBuf, u64, SystemTime)> = WalkDir::new(cache_root)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .filter_map(|entry| {
            let file_name = entry.file_name().to_string_lossy();
            (file_name == "port-report.log").then(|| {
                entry.metadata().ok().map(|metadata| {
                    (
                        entry.into_path(),
                        metadata.len(),
                        metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                    )
                })
            })?
        })
        .collect();

    logs.sort_unstable_by(|lhs, rhs| lhs.2.cmp(&rhs.2).then_with(|| lhs.0.cmp(&rhs.0)));
    for (path, byte_len, _) in logs {
        if total_bytes <= history_budget_bytes {
            break;
        }
        match std::fs::remove_file(&path) {
            Ok(()) => {
                total_bytes = total_bytes.saturating_sub(byte_len);
            },
            Err(err) if err.kind() == io::ErrorKind::NotFound => {},
            Err(err) => return Err(err),
        }
    }

    Ok(())
}
