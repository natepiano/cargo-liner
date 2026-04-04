use std::path::Path;

use chrono::DateTime;
use chrono::Utc;

use super::history;
use super::paths;
use super::read_write;
use super::status;
use super::types::PortReportCommand;
use super::types::PortReportCommandStatus;
use super::*;

fn run(status: PortReportRunStatus) -> PortReportRun {
    PortReportRun {
        run_id: "run-1".to_string(),
        started_at: "2026-03-30T14:22:01-05:00".to_string(),
        finished_at: Some("2026-03-30T14:22:18-05:00".to_string()),
        duration_ms: Some(17_000),
        status,
        commands: Vec::new(),
    }
}

// ── parse_run ───────────────────────────────────────────────────

#[test]
fn parse_run_cases() {
    let mut running = run(PortReportRunStatus::Running);
    running.started_at = Utc::now().format("%+").to_string();
    running.finished_at = None;

    let mut stale = run(PortReportRunStatus::Running);
    stale.started_at = "2020-01-01T00:00:00+00:00".to_string();
    stale.finished_at = None;

    let mut garbage = run(PortReportRunStatus::Passed);
    garbage.started_at = "not a valid timestamp".to_string();
    garbage.finished_at = Some("not a valid timestamp".to_string());

    let mut empty = run(PortReportRunStatus::Passed);
    empty.started_at.clear();
    empty.finished_at = None;

    let cases = [
        ("passed", run(PortReportRunStatus::Passed)),
        ("failed", run(PortReportRunStatus::Failed)),
        ("running", running),
        ("stale", stale),
        ("garbage", garbage),
        ("empty", empty),
    ];

    for (name, run) in cases {
        let status = status::parse_run(&run);
        match name {
            "passed" => assert!(matches!(status, LintStatus::Passed(_)), "{name}"),
            "failed" => assert!(matches!(status, LintStatus::Failed(_)), "{name}"),
            "running" => assert!(matches!(status, LintStatus::Running(_)), "{name}"),
            "stale" => assert!(matches!(status, LintStatus::Stale), "{name}"),
            "garbage" | "empty" => assert!(matches!(status, LintStatus::NoLog), "{name}"),
            _ => unreachable!("unexpected case"),
        }
    }
}

#[test]
fn aggregate_prefers_highest_severity() {
    let ts = DateTime::parse_from_rfc3339("2026-03-30T14:22:18-05:00").expect("timestamp");
    let status = LintStatus::aggregate([
        LintStatus::Passed(ts),
        LintStatus::Stale,
        LintStatus::Running(ts),
        LintStatus::Failed(ts),
    ]);
    assert!(matches!(status, LintStatus::Failed(_)));
}

#[test]
fn aggregate_keeps_latest_timestamp_within_variant() {
    let older = DateTime::parse_from_rfc3339("2026-03-30T14:22:18-05:00").expect("older");
    let newer = DateTime::parse_from_rfc3339("2026-03-30T15:22:18-05:00").expect("newer");
    let status = LintStatus::aggregate([LintStatus::Passed(older), LintStatus::Passed(newer)]);
    assert_eq!(status, LintStatus::Passed(newer));
}

// ── read_status (end-to-end) ────────────────────────────────────

fn write_latest(root: &Path, run: &PortReportRun) {
    read_write::write_latest_under(&cache_root(), root, run).expect("write latest");
}

#[test]
fn read_status_cases() {
    let mut running = run(PortReportRunStatus::Running);
    running.started_at = Utc::now().format("%+").to_string();
    running.finished_at = None;

    let mut stale = run(PortReportRunStatus::Running);
    stale.started_at = "2020-01-01T00:00:00+00:00".to_string();
    stale.finished_at = None;

    let cases = [
        ("passed", Some(run(PortReportRunStatus::Passed))),
        ("failed", Some(run(PortReportRunStatus::Failed))),
        ("running", Some(running)),
        ("stale", Some(stale)),
        ("no_log", None),
    ];

    for (name, latest) in cases {
        let dir = tempfile::tempdir().expect("tempdir");
        if let Some(run) = latest.as_ref() {
            write_latest(dir.path(), run);
        }
        let status = read_status(dir.path());
        match name {
            "passed" => assert!(matches!(status, LintStatus::Passed(_))),
            "failed" => assert!(matches!(status, LintStatus::Failed(_))),
            "running" => assert!(matches!(status, LintStatus::Running(_))),
            "stale" => assert!(matches!(status, LintStatus::Stale)),
            "no_log" => assert!(matches!(status, LintStatus::NoLog)),
            _ => unreachable!("unexpected case"),
        }
    }
}

#[test]
fn read_status_uses_latest_over_history() {
    let dir = tempfile::tempdir().expect("tempdir");
    history::append_history_under(
        &cache_root(),
        dir.path(),
        &run(PortReportRunStatus::Failed),
        None,
    )
    .expect("append history");
    write_latest(dir.path(), &run(PortReportRunStatus::Passed));
    assert!(
        matches!(read_status(dir.path()), LintStatus::Passed(_)),
        "should read latest.json, not older history"
    );
}

#[test]
fn cache_latest_path_does_not_live_under_project_dir() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = latest_path_under(&cache_root(), dir.path());
    assert!(
        !path.starts_with(dir.path()),
        "cache latest path should not recreate project directories"
    );
}

#[test]
fn history_reads_newest_first_and_includes_latest() {
    let cache_dir = tempfile::tempdir().expect("tempdir");
    let project_dir = tempfile::tempdir().expect("tempdir");
    let completed = PortReportRun {
        run_id:      "completed".to_string(),
        started_at:  "2026-04-01T18:00:00-04:00".to_string(),
        finished_at: Some("2026-04-01T18:00:10-04:00".to_string()),
        duration_ms: Some(10_000),
        status:      PortReportRunStatus::Passed,
        commands:    Vec::new(),
    };
    let running = PortReportRun {
        run_id:      "running".to_string(),
        started_at:  "2026-04-01T18:05:00-04:00".to_string(),
        finished_at: None,
        duration_ms: None,
        status:      PortReportRunStatus::Running,
        commands:    Vec::new(),
    };

    history::append_history_under(cache_dir.path(), project_dir.path(), &completed, None)
        .expect("append history");
    read_write::write_latest_under(cache_dir.path(), project_dir.path(), &running)
        .expect("write latest");

    let runs = history::read_history_under(cache_dir.path(), project_dir.path());
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0].run_id, "running");
    assert_eq!(runs[1].run_id, "completed");
}

#[test]
fn clear_latest_if_running_removes_running_latest() {
    let cache_dir = tempfile::tempdir().expect("tempdir");
    let project_dir = tempfile::tempdir().expect("tempdir");
    let running = PortReportRun {
        run_id:      "running".to_string(),
        started_at:  Utc::now().format("%+").to_string(),
        finished_at: None,
        duration_ms: None,
        status:      PortReportRunStatus::Running,
        commands:    Vec::new(),
    };
    read_write::write_latest_under(cache_dir.path(), project_dir.path(), &running)
        .expect("write latest");

    let cleared = read_write::clear_latest_if_running_under(cache_dir.path(), project_dir.path())
        .expect("clear");

    assert!(cleared);
    assert!(!latest_path_under(cache_dir.path(), project_dir.path()).exists());
}

#[test]
fn latest_final_run_does_not_duplicate_completed_history() {
    let cache_dir = tempfile::tempdir().expect("tempdir");
    let project_dir = tempfile::tempdir().expect("tempdir");
    let completed = PortReportRun {
        run_id:      "same-run".to_string(),
        started_at:  "2026-04-01T18:00:00-04:00".to_string(),
        finished_at: Some("2026-04-01T18:00:10-04:00".to_string()),
        duration_ms: Some(10_000),
        status:      PortReportRunStatus::Passed,
        commands:    Vec::new(),
    };

    history::append_history_under(cache_dir.path(), project_dir.path(), &completed, None)
        .expect("append history");
    read_write::write_latest_under(cache_dir.path(), project_dir.path(), &completed)
        .expect("write latest");

    let runs = history::read_history_under(cache_dir.path(), project_dir.path());
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].run_id, "same-run");
}

#[test]
fn append_history_prunes_oldest_runs_under_budget() {
    let cache_dir = tempfile::tempdir().expect("tempdir");
    let project_dir = tempfile::tempdir().expect("tempdir");

    let mut older = run_with_commands("older", "2026-04-01T18:00:00-04:00");
    let mut newer = run_with_commands("newer", "2026-04-01T19:00:00-04:00");

    // Archive and append the older run without a budget
    write_fake_logs(cache_dir.path(), project_dir.path(), "older logs");
    older = history::archive_run_output(cache_dir.path(), project_dir.path(), &older)
        .expect("archive older");
    history::append_history_under(cache_dir.path(), project_dir.path(), &older, None)
        .expect("append older");

    // Archive the newer run, then set a budget that forces older out
    write_fake_logs(cache_dir.path(), project_dir.path(), "newer logs");
    newer = history::archive_run_output(cache_dir.path(), project_dir.path(), &newer)
        .expect("archive newer");

    let total_before = history::total_bytes_under(cache_dir.path());
    let newer_line = serde_json::to_string(&newer).expect("serialize").len() as u64 + 1;
    let budget = total_before + newer_line - 1;

    history::append_history_under(cache_dir.path(), project_dir.path(), &newer, Some(budget))
        .expect("append newer");

    let runs = history::read_history_under(cache_dir.path(), project_dir.path());
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].run_id, "newer");
}

#[test]
fn retained_history_usage_counts_latest_and_history_bytes() {
    let cache_dir = tempfile::tempdir().expect("tempdir");
    let project_dir = tempfile::tempdir().expect("tempdir");
    let completed = run(PortReportRunStatus::Passed);

    read_write::write_latest_under(cache_dir.path(), project_dir.path(), &completed)
        .expect("write latest");
    history::append_history_under(cache_dir.path(), project_dir.path(), &completed, None)
        .expect("append history");

    let usage = history::retained_history_usage_under(cache_dir.path(), Some(1024));
    assert!(usage.bytes > 0);
    assert_eq!(usage.budget_bytes, Some(1024));
}

// ── run archival ────────────────────────────────────────────────

fn run_with_commands(run_id: &str, started_at: &str) -> PortReportRun {
    PortReportRun {
        run_id:      run_id.to_string(),
        started_at:  started_at.to_string(),
        finished_at: Some(started_at.to_string()),
        duration_ms: Some(5_000),
        status:      PortReportRunStatus::Passed,
        commands:    vec![
            PortReportCommand {
                name:        "clippy".to_string(),
                command:     "cargo clippy".to_string(),
                status:      PortReportCommandStatus::Passed,
                duration_ms: Some(3_000),
                exit_code:   Some(0),
                log_file:    "port-report/clippy-latest.log".to_string(),
            },
            PortReportCommand {
                name:        "mend".to_string(),
                command:     "cargo mend".to_string(),
                status:      PortReportCommandStatus::Passed,
                duration_ms: Some(2_000),
                exit_code:   Some(0),
                log_file:    "port-report/mend-latest.log".to_string(),
            },
        ],
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

#[test]
fn archive_run_copies_logs_to_run_id_directory() {
    let cache_dir = tempfile::tempdir().expect("tempdir");
    let project_dir = tempfile::tempdir().expect("tempdir");
    let completed = run_with_commands("run-abc", "2026-04-04T10:00:00-04:00");

    write_fake_logs(cache_dir.path(), project_dir.path(), "test output");
    let archived = history::archive_run_output(cache_dir.path(), project_dir.path(), &completed)
        .expect("archive");

    // Archived run should have updated log_file paths pointing at runs/{run_id}/
    assert_eq!(archived.commands.len(), 2);
    assert_eq!(
        archived.commands[0].log_file,
        "port-report/runs/run-abc/clippy.log"
    );
    assert_eq!(
        archived.commands[1].log_file,
        "port-report/runs/run-abc/mend.log"
    );

    // Archived files should exist on disk
    let project_cache = paths::project_dir_under(cache_dir.path(), project_dir.path());
    let run_dir = project_cache.join("port-report/runs/run-abc");
    assert!(run_dir.join("clippy.log").exists());
    assert!(run_dir.join("mend.log").exists());

    // Content should match originals
    let clippy_content = std::fs::read_to_string(run_dir.join("clippy.log")).expect("read");
    assert_eq!(clippy_content, "clippy: test output\n");

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
    let archived = history::archive_run_output(cache_dir.path(), project_dir.path(), &completed)
        .expect("archive");

    // Paths updated even if files don't exist
    assert_eq!(
        archived.commands[0].log_file,
        "port-report/runs/run-missing/clippy.log"
    );

    // No archived file on disk (nothing to copy)
    let project_cache = paths::project_dir_under(cache_dir.path(), project_dir.path());
    let run_dir = project_cache.join("port-report/runs/run-missing");
    assert!(!run_dir.join("clippy.log").exists());
}

// ── run-based pruning ──────────────────────────────────────────

#[test]
fn prune_removes_oldest_run_directory_and_history_line() {
    let cache_dir = tempfile::tempdir().expect("tempdir");
    let project_dir = tempfile::tempdir().expect("tempdir");

    let mut older = run_with_commands("run-older", "2026-04-01T18:00:00-04:00");
    let mut newer = run_with_commands("run-newer", "2026-04-01T19:00:00-04:00");

    // Archive and append the older run (no budget — always succeeds)
    write_fake_logs(cache_dir.path(), project_dir.path(), "older output");
    older = history::archive_run_output(cache_dir.path(), project_dir.path(), &older)
        .expect("archive older");
    history::append_history_under(cache_dir.path(), project_dir.path(), &older, None)
        .expect("append older");

    // Archive the newer run (not yet appended to history)
    write_fake_logs(cache_dir.path(), project_dir.path(), "newer output");
    newer = history::archive_run_output(cache_dir.path(), project_dir.path(), &newer)
        .expect("archive newer");

    // Measure total bytes with both runs fully on disk. The budget must be
    // small enough that keeping both exceeds it, but large enough that the
    // newer run alone fits. We subtract the older run's archived log bytes
    // to create that pressure.
    let total_before_append = history::total_bytes_under(cache_dir.path());
    let newer_line_bytes = serde_json::to_string(&newer).expect("serialize").len() as u64 + 1;
    let budget = total_before_append + newer_line_bytes - 1;

    history::append_history_under(cache_dir.path(), project_dir.path(), &newer, Some(budget))
        .expect("append newer");

    // Only newer run should remain in history
    let runs = history::read_history_under(cache_dir.path(), project_dir.path());
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].run_id, "run-newer");

    // Older run's archived directory should be deleted
    let project_cache = paths::project_dir_under(cache_dir.path(), project_dir.path());
    assert!(
        !project_cache.join("port-report/runs/run-older").exists(),
        "older run directory should be pruned"
    );

    // Newer run's archived directory should still exist
    assert!(
        project_cache.join("port-report/runs/run-newer").exists(),
        "newer run directory should survive"
    );
}

#[test]
fn prune_across_projects_removes_globally_oldest() {
    let cache_dir = tempfile::tempdir().expect("tempdir");
    let project_a = tempfile::tempdir().expect("tempdir");
    let project_b = tempfile::tempdir().expect("tempdir");

    let mut old_a = run_with_commands("run-old-a", "2026-04-01T17:00:00-04:00");
    let mut new_b = run_with_commands("run-new-b", "2026-04-01T20:00:00-04:00");

    // Archive and append run for project A (no budget)
    write_fake_logs(cache_dir.path(), project_a.path(), "project-a output");
    old_a =
        history::archive_run_output(cache_dir.path(), project_a.path(), &old_a).expect("archive a");
    history::append_history_under(cache_dir.path(), project_a.path(), &old_a, None)
        .expect("append a");

    // Archive run for project B (not yet appended)
    write_fake_logs(cache_dir.path(), project_b.path(), "project-b output");
    new_b =
        history::archive_run_output(cache_dir.path(), project_b.path(), &new_b).expect("archive b");

    // Budget: total with both archived + room for B's history line, minus 1
    // byte so the pruner must delete A's run to fit.
    let total_before_append = history::total_bytes_under(cache_dir.path());
    let new_b_line_bytes = serde_json::to_string(&new_b).expect("serialize").len() as u64 + 1;
    let budget = total_before_append + new_b_line_bytes - 1;

    history::append_history_under(cache_dir.path(), project_b.path(), &new_b, Some(budget))
        .expect("append b");

    // Project A's older run should be pruned
    let runs_a = history::read_history_under(cache_dir.path(), project_a.path());
    assert!(runs_a.is_empty(), "older project A run should be pruned");

    // Project B's newer run should survive
    let runs_b = history::read_history_under(cache_dir.path(), project_b.path());
    assert_eq!(runs_b.len(), 1);
    assert_eq!(runs_b[0].run_id, "run-new-b");

    // Project A's archived directory should be deleted
    let cache_a = paths::project_dir_under(cache_dir.path(), project_a.path());
    assert!(
        !cache_a.join("port-report/runs/run-old-a").exists(),
        "pruned run directory should be deleted"
    );
}

#[test]
fn prune_no_op_when_under_budget() {
    let cache_dir = tempfile::tempdir().expect("tempdir");
    let project_dir = tempfile::tempdir().expect("tempdir");

    let mut completed = run_with_commands("run-keep", "2026-04-01T18:00:00-04:00");
    write_fake_logs(cache_dir.path(), project_dir.path(), "keep this output");
    completed = history::archive_run_output(cache_dir.path(), project_dir.path(), &completed)
        .expect("archive");

    // Generous budget — nothing should be pruned
    history::append_history_under(
        cache_dir.path(),
        project_dir.path(),
        &completed,
        Some(10 * 1024 * 1024),
    )
    .expect("append");

    let runs = history::read_history_under(cache_dir.path(), project_dir.path());
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].run_id, "run-keep");

    let project_cache = paths::project_dir_under(cache_dir.path(), project_dir.path());
    assert!(project_cache.join("port-report/runs/run-keep").exists());
}
