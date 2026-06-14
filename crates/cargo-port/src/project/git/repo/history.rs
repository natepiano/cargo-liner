use std::path::Path;
use std::time::SystemTime;

use crate::constants::SECONDS_PER_DAY;
use crate::constants::SECONDS_PER_HOUR;
use crate::constants::SECONDS_PER_MINUTE;
use crate::project::git::command;
use crate::project::git::constants::GIT_FORMAT_ISO8601_ARG;
use crate::project::git::constants::GIT_HEAD;
use crate::project::git::constants::GIT_LOG_COMMAND;
use crate::project::git::constants::GIT_MAX_PARENTS_ZERO_ARG;
use crate::project::git::constants::GIT_REVERSE_ARG;
use crate::project::git::discovery;

pub(crate) fn get_first_commit(project_dir: &Path) -> Option<String> {
    let repo_root = discovery::git_repo_root(project_dir)?;
    command::git_output_logged(
        &repo_root,
        "log_first_commit",
        [
            GIT_LOG_COMMAND,
            GIT_MAX_PARENTS_ZERO_ARG,
            GIT_REVERSE_ARG,
            GIT_FORMAT_ISO8601_ARG,
            GIT_HEAD,
        ],
    )
    .ok()
    .and_then(|o| {
        String::from_utf8_lossy(&o.stdout)
            .lines()
            .next()
            .filter(|s| !s.is_empty())
            .map(std::string::ToString::to_string)
    })
}

/// Read `FETCH_HEAD` mtime from the common git dir and render it as UTC ISO
/// 8601. `FETCH_HEAD` is rewritten on every `git fetch` regardless of whether
/// refs changed, so its mtime is the most reliable "last fetched" signal.
pub(super) fn get_last_fetched(repo_root: &Path) -> Option<String> {
    let common_dir = discovery::resolve_common_git_dir(repo_root)?;
    let fetch_head = common_dir.join("FETCH_HEAD");
    let modified = std::fs::metadata(&fetch_head).ok()?.modified().ok()?;
    system_time_to_iso8601_utc(modified)
}

fn system_time_to_iso8601_utc(t: SystemTime) -> Option<String> {
    let secs = i64::try_from(t.duration_since(SystemTime::UNIX_EPOCH).ok()?.as_secs()).ok()?;
    let seconds_per_day = SECONDS_PER_DAY.cast_signed();
    let seconds_per_hour = SECONDS_PER_HOUR.cast_signed();
    let seconds_per_minute = SECONDS_PER_MINUTE.cast_signed();
    let days = secs.div_euclid(seconds_per_day);
    let time_of_day = secs.rem_euclid(seconds_per_day);
    let hour = time_of_day / seconds_per_hour;
    let min = (time_of_day % seconds_per_hour) / seconds_per_minute;
    let sec = time_of_day % seconds_per_minute;
    let (year, month, day) = civil_from_days(days);
    Some(format!(
        "{year:04}-{month:02}-{day:02}T{hour:02}:{min:02}:{sec:02}Z"
    ))
}

/// Inverse of `days_from_civil`: days since Unix epoch → (year, month, day).
/// Howard Hinnant's algorithm.
#[allow(
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::cast_possible_truncation,
    reason = "Hinnant's algorithm bounces between signed/unsigned; month/day always 1..=12 / 1..=31"
)]
const fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };
    (year, m as u32, d as u32)
}
