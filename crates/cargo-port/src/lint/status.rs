use std::path::Path;

use chrono::DateTime;
use chrono::FixedOffset;
use chrono::Utc;

use super::paths;
use super::read_write;
use super::types::LintStatus;
use super::types::PortReportRun;
use super::types::PortReportRunStatus;
use crate::constants::STALE_TIMEOUT;

/// Read the last line of the project's lint status log and parse it.
pub fn read_status(project_root: &Path) -> LintStatus {
    read_status_from_path(&paths::latest_path_under(
        &paths::cache_root(),
        project_root,
    ))
}

pub fn read_status_under(cache_root: &Path, project_root: &Path) -> LintStatus {
    read_status_from_path(&paths::latest_path_under(cache_root, project_root))
}

fn read_status_from_path(path: &Path) -> LintStatus {
    let Some(run) = read_write::read_latest_file(path) else {
        return LintStatus::NoLog;
    };
    parse_run(&run)
}

pub fn parse_timestamp(value: &str) -> Option<DateTime<FixedOffset>> {
    DateTime::parse_from_rfc3339(value.trim()).ok()
}

pub(super) fn parse_run(run: &PortReportRun) -> LintStatus {
    let timestamp = run
        .finished_at
        .as_deref()
        .and_then(parse_timestamp)
        .or_else(|| parse_timestamp(&run.started_at));
    let Some(ts) = timestamp else {
        return LintStatus::NoLog;
    };

    match run.status {
        PortReportRunStatus::Passed => LintStatus::Passed(ts),
        PortReportRunStatus::Failed => LintStatus::Failed(ts),
        PortReportRunStatus::Running => {
            let elapsed = Utc::now().signed_duration_since(ts);
            if elapsed > chrono::Duration::from_std(STALE_TIMEOUT).unwrap_or_default() {
                LintStatus::Stale
            } else {
                LintStatus::Running(ts)
            }
        },
    }
}
