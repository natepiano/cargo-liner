use std::collections::HashMap;
use std::collections::HashSet;
use std::fs::File;
use std::io;
use std::io::BufRead;
use std::io::BufReader;
use std::io::ErrorKind;
use std::io::Write;
use std::path::Path;

use walkdir::WalkDir;

use super::cache_size_index;
use super::paths;
use super::read_write;
use super::run::LintRun;
use super::run::LintRunStatus;
use super::status;
use crate::constants::LINTS_HISTORY_JSONL;
use crate::project::AbsolutePath;

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

/// Total bytes on disk under a run's archived output directory
/// (`{cache}/{project_key}/runs/{run_id}`).
///
/// Called only when seeding `LintRuns::archive_bytes` — UI reads go through
/// the in-memory cache.
pub(super) fn retained_cache_usage_under(
    cache_root: &Path,
    cache_size_bytes: Option<u64>,
) -> CacheUsage {
    let bytes = cache_size_index::read(cache_root).unwrap_or_else(|| {
        let bytes = total_bytes_under(cache_root);
        let _ = cache_size_index::write(cache_root, bytes);
        bytes
    });
    CacheUsage {
        bytes,
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
    #[cfg(test)]
    paths::assert_not_default_user_cache_root(cache_root);

    let project_dir = paths::project_dir_under(cache_root, project_root);
    let output_dir = paths::output_dir_under(cache_root, project_root);
    let run_dir = output_dir.join("runs").join(&run.run_id);

    let mut archived = run.clone();
    let mut any_copied = false;

    let mut archived_bytes: u64 = 0;
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
            let dest = run_dir.join(&archived_name);
            archived_bytes = archived_bytes.saturating_add(std::fs::copy(&source, &dest)?);
        }
    }

    if archived_bytes > 0 {
        cache_size_index::adjust(
            cache_root,
            i64::try_from(archived_bytes).unwrap_or(i64::MAX),
        );
    }
    archived.archive_bytes = archived_bytes;
    Ok(archived)
}

pub fn append_history_under(
    cache_root: &Path,
    project_root: &Path,
    run: &LintRun,
    cache_size_bytes: Option<u64>,
) -> io::Result<PruneStats> {
    #[cfg(test)]
    paths::assert_not_default_user_cache_root(cache_root);

    let path = paths::history_path_under(cache_root, project_root);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json =
        serde_json::to_string(run).map_err(|err| io::Error::new(ErrorKind::InvalidData, err))?;
    {
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;
        writeln!(file, "{json}")?;
    }
    // The handle is closed above before pruning: `prune_runs_under` re-walks
    // the cache via `total_bytes_under`, which stats this file by path. On
    // Windows the directory-entry length of a file held open for append
    // updates only on close, so a still-open handle would hide the just
    // appended line and make pruning under-trigger.
    let line_bytes = u64::try_from(json.len().saturating_add(1)).unwrap_or(u64::MAX);
    cache_size_index::adjust(cache_root, i64::try_from(line_bytes).unwrap_or(i64::MAX));
    enforce_cache_size_under(cache_root, cache_size_bytes, Some((&path, &run.run_id)))
}

pub fn read_history(project_root: &Path) -> Vec<LintRun> {
    read_history_under(&paths::cache_root(), project_root)
}

pub(super) fn read_history_under(cache_root: &Path, project_root: &Path) -> Vec<LintRun> {
    let mut runs: Vec<LintRun> =
        read_write::read_history_file(&paths::history_path_under(cache_root, project_root))
            .into_iter()
            .filter(|run| !matches!(run.status, LintRunStatus::Running))
            .collect();
    let latest = read_write::read_latest_file(&paths::latest_path_under(cache_root, project_root))
        .filter(|run| !matches!(run.status, LintRunStatus::Running));

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
    protected: Option<(&Path, &str)>,
) -> io::Result<PruneStats> {
    let Some(cache_size) = cache_size_bytes else {
        return Ok(PruneStats::default());
    };
    prune_runs_under(cache_root, cache_size, protected)
}

/// Total bytes under a single project's cache subdirectory. Used by
/// [`crate::lint::reclaim_project_cache`] to compute the delta before
/// the directory is removed so the cache-size index can be decremented.
pub(super) fn project_dir_bytes(project_dir: &Path) -> u64 { total_bytes_under(project_dir) }

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
    history_path:      AbsolutePath,
    line_index:        usize,
    sort_key:          i64,
    run_id:            String,
    /// Project cache directory containing `runs/{run_id}/` archives.
    project_cache_dir: AbsolutePath,
}

/// Collect all runs across all history files, paired with their archive
/// output directory.
fn collect_prunable_runs(cache_root: &Path) -> io::Result<Vec<PrunableRun>> {
    let mut runs = Vec::new();

    for history_pb in WalkDir::new(cache_root)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .filter(|entry| entry.file_name() == LINTS_HISTORY_JSONL)
        .map(walkdir::DirEntry::into_path)
    {
        let history_path = AbsolutePath::from(history_pb);
        let project_cache_dir = history_path
            .parent()
            .map_or_else(|| "/".into(), AbsolutePath::from);

        let file = File::open(&*history_path)?;
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
                project_cache_dir: project_cache_dir.clone(),
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
/// in fixed 5%-of-cache-size batches until total bytes fall within the limit.
///
/// Why: trimming to exactly the cache limit causes the very next append to
/// re-trigger eviction. Reclaiming a fixed chunk per call leaves headroom.
/// The chunk is 5% of the cache limit, recorded in bytes up-front so repeated
/// batches do not shrink as the cache drains.
///
/// `protected` is the (`history_path`, `run_id`) of a run that must not be
/// evicted — typically the run that was just appended. If reclaiming every
/// other run still leaves the cache over its limit, the protected run stays
/// and the cache remains over the limit.
fn prune_runs_under(
    cache_root: &Path,
    cache_size: u64,
    protected: Option<(&Path, &str)>,
) -> io::Result<PruneStats> {
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

    let batch_bytes = (cache_size / 20).max(1);
    let mut target = batch_bytes;
    let mut reclaimed: u64 = 0;

    // Track which runs to remove, keyed by history file path.
    let mut removed: HashMap<AbsolutePath, Vec<usize>> = HashMap::new();
    let mut runs_evicted: usize = 0;

    for run in &runs {
        if reclaimed >= target && total_bytes <= cache_size {
            break;
        }

        if let Some((protected_history, protected_id)) = protected
            && run.history_path.as_path() == protected_history
            && run.run_id == protected_id
        {
            continue;
        }

        // Remove the archived output directory for this run.
        let run_dir = run.project_cache_dir.join("runs").join(&run.run_id);
        if run_dir.is_dir() {
            let dir_bytes = total_bytes_under(&run_dir);
            std::fs::remove_dir_all(&run_dir).ok();
            total_bytes = total_bytes.saturating_sub(dir_bytes);
            reclaimed = reclaimed.saturating_add(dir_bytes);
        }

        removed
            .entry(run.history_path.clone())
            .or_default()
            .push(run.line_index);
        runs_evicted += 1;

        // Hit the current batch target but still over the cache limit —
        // advance to the next fixed 5% multiple.
        if reclaimed >= target && total_bytes > cache_size {
            let batches_done = reclaimed / batch_bytes;
            target = batches_done.saturating_add(1).saturating_mul(batch_bytes);
        }
    }

    // Rewrite each affected history file, keeping only non-removed lines.
    for (history_path, removed_indices) in &removed {
        let file = File::open(history_path)?;
        let reader = BufReader::new(file);
        let line_count = reader.lines().count();

        let removed_set: HashSet<usize> = removed_indices.iter().copied().collect();
        let kept: Vec<usize> = (0..line_count)
            .filter(|index| !removed_set.contains(index))
            .collect();

        // Subtract the removed history line bytes from total.
        let file_before = std::fs::metadata(history_path).map_or(0, |m| m.len());
        rewrite_history_file(history_path, &kept)?;
        let file_after = std::fs::metadata(history_path).map_or(0, |m| m.len());
        total_bytes = total_bytes.saturating_sub(file_before.saturating_sub(file_after));
    }

    let _ = cache_size_index::write(cache_root, total_bytes);
    Ok(PruneStats {
        runs_evicted,
        bytes_reclaimed: bytes_before.saturating_sub(total_bytes),
    })
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::lint::run::LintCommand;
    use crate::lint::run::LintCommandStatus;

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
    fn reads_newest_first_and_excludes_running_records() {
        let cache_dir = tempfile::tempdir().expect("tempdir");
        let project_dir = tempfile::tempdir().expect("tempdir");
        let completed = LintRun {
            run_id:        "completed".to_string(),
            started_at:    "2026-04-01T18:00:00-04:00".to_string(),
            finished_at:   Some("2026-04-01T18:00:10-04:00".to_string()),
            duration_ms:   Some(10_000),
            status:        LintRunStatus::Passed,
            commands:      Vec::new(),
            archive_bytes: 0,
        };
        let running = LintRun {
            run_id:        "running".to_string(),
            started_at:    "2026-04-01T18:05:00-04:00".to_string(),
            finished_at:   None,
            duration_ms:   None,
            status:        LintRunStatus::Running,
            commands:      Vec::new(),
            archive_bytes: 0,
        };

        append_history_under(cache_dir.path(), project_dir.path(), &completed, None)
            .expect("append history");
        append_history_under(cache_dir.path(), project_dir.path(), &running, None)
            .expect("append running history");
        read_write::write_latest_under(cache_dir.path(), project_dir.path(), &running)
            .expect("write latest");

        let runs = read_history_under(cache_dir.path(), project_dir.path());
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].run_id, "completed");
    }

    #[test]
    fn latest_final_run_does_not_duplicate_completed_history() {
        let cache_dir = tempfile::tempdir().expect("tempdir");
        let project_dir = tempfile::tempdir().expect("tempdir");
        let completed = LintRun {
            run_id:        "same-run".to_string(),
            started_at:    "2026-04-01T18:00:00-04:00".to_string(),
            finished_at:   Some("2026-04-01T18:00:10-04:00".to_string()),
            duration_ms:   Some(10_000),
            status:        LintRunStatus::Passed,
            commands:      Vec::new(),
            archive_bytes: 0,
        };

        append_history_under(cache_dir.path(), project_dir.path(), &completed, None)
            .expect("append history");
        read_write::write_latest_under(cache_dir.path(), project_dir.path(), &completed)
            .expect("write latest");

        let runs = read_history_under(cache_dir.path(), project_dir.path());
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].run_id, "same-run");
    }

    #[test]
    fn retained_cache_usage_counts_latest_and_history_bytes() {
        let cache_dir = tempfile::tempdir().expect("tempdir");
        let project_dir = tempfile::tempdir().expect("tempdir");
        let completed = run(LintRunStatus::Passed);

        read_write::write_latest_under(cache_dir.path(), project_dir.path(), &completed)
            .expect("write latest");
        append_history_under(cache_dir.path(), project_dir.path(), &completed, None)
            .expect("append history");

        let usage = retained_cache_usage_under(cache_dir.path(), Some(1024));
        assert!(usage.bytes > 0);
        assert_eq!(usage.cache_size_bytes, Some(1024));
    }

    fn run_with_commands(run_id: &str, started_at: &str) -> LintRun {
        LintRun {
            run_id:        run_id.to_string(),
            started_at:    started_at.to_string(),
            finished_at:   Some(started_at.to_string()),
            duration_ms:   Some(5_000),
            status:        LintRunStatus::Passed,
            commands:      vec![
                LintCommand {
                    name:        "clippy".to_string(),
                    command:     "cargo clippy".to_string(),
                    status:      LintCommandStatus::Passed,
                    duration_ms: Some(3_000),
                    exit_code:   Some(0),
                    log_file:    "clippy-latest.log".to_string(),
                },
                LintCommand {
                    name:        "mend".to_string(),
                    command:     "cargo mend".to_string(),
                    status:      LintCommandStatus::Passed,
                    duration_ms: Some(2_000),
                    exit_code:   Some(0),
                    log_file:    "mend-latest.log".to_string(),
                },
            ],
            archive_bytes: 0,
        }
    }

    fn write_fake_logs(cache_root: &Path, project_root: &Path, content: &str) {
        let output_dir = paths::output_dir_under(cache_root, project_root);
        std::fs::create_dir_all(&output_dir).expect("create output dir");
        std::fs::write(
            output_dir.join("clippy-latest.log"),
            format!("clippy: {content}\n"),
        )
        .expect("write clippy log");
        std::fs::write(
            output_dir.join("mend-latest.log"),
            format!("mend: {content}\n"),
        )
        .expect("write mend log");
    }

    fn archive_run_with_logs(
        cache_root: &Path,
        project_root: &Path,
        run_id: &str,
        started_at: &str,
        content: &str,
    ) -> LintRun {
        let run = run_with_commands(run_id, started_at);
        write_fake_logs(cache_root, project_root, content);
        archive_run_output(cache_root, project_root, &run).expect("archive run")
    }

    fn append_archived_run(
        cache_root: &Path,
        project_root: &Path,
        run: &LintRun,
        cache_size: Option<u64>,
    ) -> PruneStats {
        append_history_under(cache_root, project_root, run, cache_size).expect("append run")
    }

    #[test]
    fn archive_run_copies_logs_to_run_id_directory() {
        let cache_dir = tempfile::tempdir().expect("tempdir");
        let project_dir = tempfile::tempdir().expect("tempdir");
        let completed = run_with_commands("run-abc", "2026-04-04T10:00:00-04:00");

        write_fake_logs(cache_dir.path(), project_dir.path(), "test output");
        let archived =
            archive_run_output(cache_dir.path(), project_dir.path(), &completed).expect("archive");

        // Archived run should have updated log_file paths pointing at runs/{run_id}/
        assert_eq!(archived.commands.len(), 2);
        assert_eq!(archived.commands[0].log_file, "runs/run-abc/clippy.log");
        assert_eq!(archived.commands[1].log_file, "runs/run-abc/mend.log");

        // Archived files should exist on disk
        let project_cache = paths::project_dir_under(cache_dir.path(), project_dir.path());
        let run_dir = project_cache.join("runs/run-abc");
        assert!(run_dir.join("clippy.log").exists());
        assert!(run_dir.join("mend.log").exists());

        // Content should match originals
        let clippy_content = std::fs::read_to_string(run_dir.join("clippy.log")).expect("read");
        assert_eq!(clippy_content, "clippy: test output\n");

        // The archived run carries the total bytes of its copied logs, summed
        // once at archive time so reading history never walks the directory.
        let clippy_bytes = std::fs::metadata(run_dir.join("clippy.log"))
            .expect("clippy meta")
            .len();
        let mend_bytes = std::fs::metadata(run_dir.join("mend.log"))
            .expect("mend meta")
            .len();
        assert!(clippy_bytes > 0 && mend_bytes > 0);
        assert_eq!(archived.archive_bytes, clippy_bytes + mend_bytes);

        // Latest logs should still exist (convenience copies)
        let output_dir = paths::output_dir_under(cache_dir.path(), project_dir.path());
        assert!(output_dir.join("clippy-latest.log").exists());
        assert!(output_dir.join("mend-latest.log").exists());
    }

    #[test]
    fn archive_run_with_missing_logs_still_succeeds() {
        let cache_dir = tempfile::tempdir().expect("tempdir");
        let project_dir = tempfile::tempdir().expect("tempdir");
        let completed = run_with_commands("run-missing", "2026-04-04T10:00:00-04:00");

        // Don't write any log files — archive should still succeed gracefully
        let archived =
            archive_run_output(cache_dir.path(), project_dir.path(), &completed).expect("archive");

        // Paths updated even if files don't exist
        assert_eq!(archived.commands[0].log_file, "runs/run-missing/clippy.log");

        // No archived file on disk (nothing to copy)
        let project_cache = paths::project_dir_under(cache_dir.path(), project_dir.path());
        let run_dir = project_cache.join("runs/run-missing");
        assert!(!run_dir.join("clippy.log").exists());

        // Nothing copied, so the persisted archive size is zero.
        assert_eq!(archived.archive_bytes, 0);
    }

    #[test]
    fn prune_removes_oldest_run_directory_and_history_line() {
        let cache_dir = tempfile::tempdir().expect("tempdir");
        let project_dir = tempfile::tempdir().expect("tempdir");

        let older = archive_run_with_logs(
            cache_dir.path(),
            project_dir.path(),
            "run-older",
            "2026-04-01T18:00:00-04:00",
            "older output with padding to exceed batch size",
        );
        append_archived_run(cache_dir.path(), project_dir.path(), &older, None);

        let newer = archive_run_with_logs(
            cache_dir.path(),
            project_dir.path(),
            "run-newer",
            "2026-04-01T19:00:00-04:00",
            "newer output with padding to exceed batch size",
        );

        // Measure total bytes with both runs fully on disk. The cache size must be
        // small enough that keeping both exceeds it, but large enough that the
        // newer run alone fits. We subtract the older run's archived log bytes
        // to create that pressure.
        let total_before_append = total_bytes_under(cache_dir.path());
        let newer_line_bytes = serde_json::to_string(&newer).expect("serialize").len() as u64 + 1;
        let cache_size = total_before_append + newer_line_bytes - 1;

        append_archived_run(
            cache_dir.path(),
            project_dir.path(),
            &newer,
            Some(cache_size),
        );

        // Only newer run should remain in history
        let runs = read_history_under(cache_dir.path(), project_dir.path());
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].run_id, "run-newer");

        // Older run's archived directory should be deleted
        let project_cache = paths::project_dir_under(cache_dir.path(), project_dir.path());
        assert!(
            !project_cache.join("runs/run-older").exists(),
            "older run directory should be pruned"
        );

        // Newer run's archived directory should still exist
        assert!(
            project_cache.join("runs/run-newer").exists(),
            "newer run directory should survive"
        );
    }

    #[test]
    fn prune_across_projects_removes_globally_oldest() {
        let cache_dir = tempfile::tempdir().expect("tempdir");
        let project_a = tempfile::tempdir().expect("tempdir");
        let project_b = tempfile::tempdir().expect("tempdir");

        let old_a = archive_run_with_logs(
            cache_dir.path(),
            project_a.path(),
            "run-old-a",
            "2026-04-01T17:00:00-04:00",
            "project-a output with padding to exceed batch size",
        );
        append_archived_run(cache_dir.path(), project_a.path(), &old_a, None);

        let new_b = archive_run_with_logs(
            cache_dir.path(),
            project_b.path(),
            "run-new-b",
            "2026-04-01T20:00:00-04:00",
            "project-b output with padding to exceed batch size",
        );

        // Budget: total with both archived + room for B's history line, minus 1
        // byte so the pruner must delete A's run to fit.
        let total_before_append = total_bytes_under(cache_dir.path());
        let new_b_line_bytes = serde_json::to_string(&new_b).expect("serialize").len() as u64 + 1;
        let cache_size = total_before_append + new_b_line_bytes - 1;

        append_archived_run(cache_dir.path(), project_b.path(), &new_b, Some(cache_size));

        // Project A's older run should be pruned
        let runs_a = read_history_under(cache_dir.path(), project_a.path());
        assert!(runs_a.is_empty(), "older project A run should be pruned");

        // Project B's newer run should survive
        let runs_b = read_history_under(cache_dir.path(), project_b.path());
        assert_eq!(runs_b.len(), 1);
        assert_eq!(runs_b[0].run_id, "run-new-b");

        // Project A's archived directory should be deleted
        let cache_a = paths::project_dir_under(cache_dir.path(), project_a.path());
        assert!(
            !cache_a.join("runs/run-old-a").exists(),
            "pruned run directory should be deleted"
        );
    }

    #[test]
    fn prune_no_op_when_under_cache_size() {
        let cache_dir = tempfile::tempdir().expect("tempdir");
        let project_dir = tempfile::tempdir().expect("tempdir");

        let completed = archive_run_with_logs(
            cache_dir.path(),
            project_dir.path(),
            "run-keep",
            "2026-04-01T18:00:00-04:00",
            "keep this output",
        );

        // Generous cache size — nothing should be pruned
        append_archived_run(
            cache_dir.path(),
            project_dir.path(),
            &completed,
            Some(10 * 1024 * 1024),
        );

        let runs = read_history_under(cache_dir.path(), project_dir.path());
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].run_id, "run-keep");

        let project_cache = paths::project_dir_under(cache_dir.path(), project_dir.path());
        assert!(project_cache.join("runs/run-keep").exists());
    }

    #[test]
    fn prune_returns_stats_about_evicted_runs() {
        let cache_dir = tempfile::tempdir().expect("tempdir");
        let project_dir = tempfile::tempdir().expect("tempdir");

        let older = archive_run_with_logs(
            cache_dir.path(),
            project_dir.path(),
            "run-older",
            "2026-04-01T18:00:00-04:00",
            "older output with padding to exceed batch size",
        );
        append_archived_run(cache_dir.path(), project_dir.path(), &older, None);

        let newer = archive_run_with_logs(
            cache_dir.path(),
            project_dir.path(),
            "run-newer",
            "2026-04-01T19:00:00-04:00",
            "newer output with padding to exceed batch size",
        );

        let total_before = total_bytes_under(cache_dir.path());
        let newer_line = serde_json::to_string(&newer).expect("serialize").len() as u64 + 1;
        let cache_size = total_before + newer_line - 1;

        let stats = append_archived_run(
            cache_dir.path(),
            project_dir.path(),
            &newer,
            Some(cache_size),
        );

        assert_eq!(stats.runs_evicted, 1);
        assert!(stats.bytes_reclaimed > 0);
    }

    #[test]
    fn prune_protects_just_appended_run_even_when_larger_than_cache() {
        let cache_dir = tempfile::tempdir().expect("tempdir");
        let project_dir = tempfile::tempdir().expect("tempdir");

        let older = archive_run_with_logs(
            cache_dir.path(),
            project_dir.path(),
            "run-older",
            "2026-04-01T18:00:00-04:00",
            "older output",
        );
        append_archived_run(cache_dir.path(), project_dir.path(), &older, None);

        // Newer run whose archived logs alone far exceed the cache budget.
        let huge_content = "x".repeat(10_000);
        let newer = archive_run_with_logs(
            cache_dir.path(),
            project_dir.path(),
            "run-newer",
            "2026-04-01T19:00:00-04:00",
            &huge_content,
        );

        // Tiny cache forces eviction; even wiping the older run cannot get
        // below cache_size because the newer run alone is far larger. The
        // newer run should survive because it was just appended.
        let stats = append_archived_run(cache_dir.path(), project_dir.path(), &newer, Some(500));

        let runs = read_history_under(cache_dir.path(), project_dir.path());
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].run_id, "run-newer");

        let project_cache = paths::project_dir_under(cache_dir.path(), project_dir.path());
        assert!(
            project_cache.join("runs/run-newer").exists(),
            "just-appended run directory should survive"
        );
        assert!(
            !project_cache.join("runs/run-older").exists(),
            "older run directory should be evicted"
        );
        assert_eq!(stats.runs_evicted, 1);
    }

    #[test]
    fn no_prune_returns_zero_stats() {
        let cache_dir = tempfile::tempdir().expect("tempdir");
        let project_dir = tempfile::tempdir().expect("tempdir");

        let completed = archive_run_with_logs(
            cache_dir.path(),
            project_dir.path(),
            "run-keep",
            "2026-04-01T18:00:00-04:00",
            "keep this",
        );

        let stats = append_archived_run(
            cache_dir.path(),
            project_dir.path(),
            &completed,
            Some(10 * 1024 * 1024),
        );

        assert_eq!(stats.runs_evicted, 0);
        assert_eq!(stats.bytes_reclaimed, 0);
    }

    #[test]
    fn no_cache_size_returns_zero_stats() {
        let cache_dir = tempfile::tempdir().expect("tempdir");
        let project_dir = tempfile::tempdir().expect("tempdir");

        let completed = run_with_commands("run-unlimited", "2026-04-01T18:00:00-04:00");

        let stats = append_history_under(cache_dir.path(), project_dir.path(), &completed, None)
            .expect("append");

        assert_eq!(stats.runs_evicted, 0);
        assert_eq!(stats.bytes_reclaimed, 0);
    }
}
