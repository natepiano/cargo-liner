use std::collections::HashMap;
use std::fs::File;
use std::io;
use std::io::BufRead;
use std::io::BufReader;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;

use walkdir::WalkDir;

use super::paths;
use super::read_write;
use super::status;
use super::types::LintRun;
use crate::constants::LINTS_HISTORY_JSONL;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CacheUsage {
    pub bytes:            u64,
    pub cache_size_bytes: Option<u64>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PruneStats {
    pub runs_evicted:    usize,
    pub bytes_reclaimed: u64,
}

pub fn retained_cache_usage(cache_size_bytes: Option<u64>) -> CacheUsage {
    retained_cache_usage_under(&paths::cache_root(), cache_size_bytes)
}

pub(super) fn retained_cache_usage_under(
    cache_root: &Path,
    cache_size_bytes: Option<u64>,
) -> CacheUsage {
    CacheUsage {
        bytes: total_bytes_under(cache_root),
        cache_size_bytes,
    }
}

/// Archive command output from rolling `*-latest.log` files into a stable
/// per-run directory: `runs/{run_id}/{command_name}.log`.
///
/// Returns a clone of the run with `log_file` paths updated to point at the
/// archived location. The original `*-latest.log` files are left in place as
/// convenience pointers for the current run.
pub(super) fn archive_run_output(
    cache_root: &Path,
    project_root: &Path,
    run: &LintRun,
) -> io::Result<LintRun> {
    let project_dir = paths::project_dir_under(cache_root, project_root);
    let output_dir = paths::output_dir_under(cache_root, project_root);
    let run_dir = output_dir.join("runs").join(&run.run_id);

    let mut archived = run.clone();
    let mut any_copied = false;

    for command in &mut archived.commands {
        let archived_name = format!("{}.log", command.name);
        let archived_rel = format!("runs/{}/{archived_name}", run.run_id);

        // Resolve the source from the old relative log_file path
        let source = project_dir.join(&command.log_file);
        command.log_file = archived_rel;

        if source.exists() {
            if !any_copied {
                std::fs::create_dir_all(&run_dir)?;
                any_copied = true;
            }
            std::fs::copy(&source, run_dir.join(&archived_name))?;
        }
    }

    Ok(archived)
}

pub fn append_history_under(
    cache_root: &Path,
    project_root: &Path,
    run: &LintRun,
    cache_size_bytes: Option<u64>,
) -> io::Result<PruneStats> {
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
    enforce_cache_size_under(cache_root, cache_size_bytes)
}

pub fn read_history(project_root: &Path) -> Vec<LintRun> {
    read_history_under(&paths::cache_root(), project_root)
}

pub(super) fn read_history_under(cache_root: &Path, project_root: &Path) -> Vec<LintRun> {
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

fn enforce_cache_size_under(
    cache_root: &Path,
    cache_size_bytes: Option<u64>,
) -> io::Result<PruneStats> {
    let Some(cache_size) = cache_size_bytes else {
        return Ok(PruneStats::default());
    };
    prune_runs_under(cache_root, cache_size)
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

fn history_line_sort_key(run: &LintRun) -> i64 {
    run.finished_at
        .as_deref()
        .and_then(status::parse_timestamp)
        .or_else(|| status::parse_timestamp(&run.started_at))
        .map_or(i64::MIN, |timestamp| timestamp.timestamp_millis())
}

/// A single run in a single history file, with enough context to remove it
/// and its archived output directory.
#[derive(Debug)]
struct PrunableRun {
    history_path:      PathBuf,
    line_index:        usize,
    sort_key:          i64,
    run_id:            String,
    /// Project cache directory containing `runs/{run_id}/` archives.
    project_cache_dir: PathBuf,
}

/// Collect all runs across all history files, paired with their archive
/// output directory.
fn collect_prunable_runs(cache_root: &Path) -> io::Result<Vec<PrunableRun>> {
    let mut runs = Vec::new();

    for history_path in WalkDir::new(cache_root)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .filter(|entry| entry.file_name() == LINTS_HISTORY_JSONL)
        .map(walkdir::DirEntry::into_path)
    {
        let project_cache_dir = history_path.parent().unwrap_or_else(|| Path::new(""));

        let file = File::open(&history_path)?;
        let reader = BufReader::new(file);
        for (line_index, line) in reader.lines().enumerate() {
            let line = line?;
            let Ok(run) = serde_json::from_str::<LintRun>(&line) else {
                continue;
            };
            runs.push(PrunableRun {
                history_path: history_path.clone(),
                line_index,
                sort_key: history_line_sort_key(&run),
                run_id: run.run_id,
                project_cache_dir: project_cache_dir.to_path_buf(),
            });
        }
    }

    Ok(runs)
}

fn rewrite_history_file(path: &Path, kept_indices: &[usize]) -> io::Result<()> {
    if kept_indices.is_empty() {
        match std::fs::remove_file(path) {
            Ok(()) | Err(_) => return Ok(()),
        }
    }

    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let all_lines: Vec<String> = reader.lines().map_while(Result::ok).collect();

    let tmp_path = path.with_extension("jsonl.tmp");
    let mut out = File::create(&tmp_path)?;
    for &index in kept_indices {
        if let Some(line) = all_lines.get(index) {
            writeln!(out, "{line}")?;
        }
    }
    std::fs::rename(tmp_path, path)
}

/// Remove the oldest complete runs (history line + archived output directory)
/// until total bytes under the cache root are within the cache size limit.
fn prune_runs_under(cache_root: &Path, cache_size: u64) -> io::Result<PruneStats> {
    let bytes_before = total_bytes_under(cache_root);
    if bytes_before <= cache_size {
        return Ok(PruneStats::default());
    }

    let mut total_bytes = bytes_before;
    let mut runs = collect_prunable_runs(cache_root)?;
    if runs.is_empty() {
        return Ok(PruneStats::default());
    }

    // Sort oldest first so we remove the least-recent runs first.
    runs.sort_unstable_by(|lhs, rhs| {
        lhs.sort_key
            .cmp(&rhs.sort_key)
            .then_with(|| lhs.history_path.cmp(&rhs.history_path))
            .then_with(|| lhs.line_index.cmp(&rhs.line_index))
    });

    // Track which runs to remove, keyed by history file path.
    let mut removed: HashMap<PathBuf, Vec<usize>> = HashMap::new();
    let mut runs_evicted: usize = 0;

    for run in &runs {
        if total_bytes <= cache_size {
            break;
        }

        // Remove the archived output directory for this run.
        let run_dir = run.project_cache_dir.join("runs").join(&run.run_id);
        if run_dir.is_dir() {
            let dir_bytes = total_bytes_under(&run_dir);
            std::fs::remove_dir_all(&run_dir).ok();
            total_bytes = total_bytes.saturating_sub(dir_bytes);
        }

        removed
            .entry(run.history_path.clone())
            .or_default()
            .push(run.line_index);
        runs_evicted += 1;
    }

    // Rewrite each affected history file, keeping only non-removed lines.
    for (history_path, removed_indices) in &removed {
        let file = File::open(history_path)?;
        let reader = BufReader::new(file);
        let line_count = reader.lines().count();

        let removed_set: std::collections::HashSet<usize> =
            removed_indices.iter().copied().collect();
        let kept: Vec<usize> = (0..line_count)
            .filter(|index| !removed_set.contains(index))
            .collect();

        // Subtract the removed history line bytes from total.
        let file_before = std::fs::metadata(history_path)
            .map(|m| m.len())
            .unwrap_or(0);
        rewrite_history_file(history_path, &kept)?;
        let file_after = std::fs::metadata(history_path)
            .map(|m| m.len())
            .unwrap_or(0);
        total_bytes = total_bytes.saturating_sub(file_before.saturating_sub(file_after));
    }

    Ok(PruneStats {
        runs_evicted,
        bytes_reclaimed: bytes_before.saturating_sub(total_bytes_under(cache_root)),
    })
}
