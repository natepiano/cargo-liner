use std::path::Path;

use chrono::DateTime;
use chrono::Utc;

use super::history;
use super::read_write;
use super::status;
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

    let mut older = run(PortReportRunStatus::Passed);
    older.run_id = "older".to_string();
    older.started_at = "2026-04-01T18:00:00-04:00".to_string();
    older.finished_at = Some("2026-04-01T18:00:10-04:00".to_string());

    let mut newer = run(PortReportRunStatus::Passed);
    newer.run_id = "newer".to_string();
    newer.started_at = "2026-04-01T19:00:00-04:00".to_string();
    newer.finished_at = Some("2026-04-01T19:00:10-04:00".to_string());

    let newer_json = serde_json::to_string(&newer).expect("serialize newer");
    let budget_bytes = u64::try_from(newer_json.len() + 1).unwrap_or(u64::MAX)
        + history::total_bytes_under(cache_dir.path());

    history::append_history_under(cache_dir.path(), project_dir.path(), &older, None)
        .expect("append older");
    history::append_history_under(
        cache_dir.path(),
        project_dir.path(),
        &newer,
        Some(budget_bytes),
    )
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
