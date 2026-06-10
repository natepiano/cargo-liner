use std::process::Command;
use std::sync::OnceLock;

use crate::constants::IN_SYNC;
use crate::constants::MINUTES_PER_DAY;
use crate::constants::MINUTES_PER_HOUR;
use crate::constants::NO_REMOTE_SYNC;
use crate::constants::SECONDS_PER_DAY;
use crate::constants::SECONDS_PER_HOUR;
use crate::constants::SECONDS_PER_MINUTE;
use crate::constants::SYNC_DOWN;
use crate::constants::SYNC_UP;
use crate::http::RateLimitQuota;
use crate::project::BisectProgress;

/// Get the local UTC offset in seconds (e.g., -28800 for PST).
fn local_utc_offset_secs() -> i64 {
    static OFFSET: OnceLock<i64> = OnceLock::new();
    *OFFSET.get_or_init(|| {
        Command::new("date")
            .arg("+%z")
            .output()
            .ok()
            .and_then(|output| {
                let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if value.len() >= 5 {
                    let sign: i64 = if value.starts_with('-') { -1 } else { 1 };
                    let hours: i64 = value[1..3].parse().ok()?;
                    let mins: i64 = value[3..5].parse().ok()?;
                    Some(
                        sign * (hours * SECONDS_PER_HOUR.cast_signed()
                            + mins * SECONDS_PER_MINUTE.cast_signed()),
                    )
                } else {
                    None
                }
            })
            .unwrap_or(0)
    })
}

const fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        2 => {
            if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) {
                29
            } else {
                28
            }
        },
        _ => 30,
    }
}

/// Extract the local date from an ISO 8601 timestamp as `yyyy-mm-dd`.
///
/// If the timestamp has an embedded timezone offset, the date portion is
/// already local and is returned directly. For UTC timestamps, the local
/// offset is applied via `format_timestamp`.
pub fn format_date(iso: &str) -> String {
    let stripped = iso.trim_end_matches('Z');
    if let Some((date, after_t)) = stripped.split_once('T') {
        let has_offset = after_t.rfind(['+', '-']).is_some_and(|p| p > 0);
        if has_offset {
            return date.to_string();
        }
    }
    let full = format_timestamp(iso);
    full.split(' ').next().unwrap_or(&full).to_string()
}

/// Extract the local time portion from an ISO 8601 timestamp as `hh:mm:ss`.
///
/// If the timestamp has an embedded timezone offset (e.g., `-04:00`), the
/// time is already local and no offset is applied. If it ends in `Z` or has
/// no offset, the local UTC offset is applied.
pub fn format_time(iso: &str) -> String {
    let is_utc = iso.ends_with('Z');
    let stripped = iso.trim_end_matches('Z');
    let Some((_, time_and_offset)) = stripped.split_once('T') else {
        return "—".to_string();
    };

    let (time_str, has_embedded_offset) =
        time_and_offset
            .rfind(['+', '-'])
            .map_or((time_and_offset, false), |pos| {
                if pos > 0 {
                    (&time_and_offset[..pos], true)
                } else {
                    (time_and_offset, false)
                }
            });

    let time_parts: Vec<&str> = time_str.split(':').collect();
    if time_parts.len() < 3 {
        return time_str.to_string();
    }
    let (Ok(hour), Ok(minute), Ok(second)) = (
        time_parts[0].parse::<i64>(),
        time_parts[1].parse::<i64>(),
        time_parts[2]
            .split('.')
            .next()
            .unwrap_or("0")
            .parse::<i64>(),
    ) else {
        return time_str.to_string();
    };

    let offset = if has_embedded_offset || !is_utc {
        0
    } else {
        local_utc_offset_secs()
    };

    let seconds_per_day = SECONDS_PER_DAY.cast_signed();
    let seconds_per_hour = SECONDS_PER_HOUR.cast_signed();
    let seconds_per_minute = SECONDS_PER_MINUTE.cast_signed();
    let total_secs = hour * seconds_per_hour + minute * seconds_per_minute + second + offset;
    let mut adj = total_secs % seconds_per_day;
    if adj < 0 {
        adj += seconds_per_day;
    }
    let h = adj / seconds_per_hour;
    let m = (adj % seconds_per_hour) / seconds_per_minute;
    let s = adj % seconds_per_minute;
    format!("{h:02}:{m:02}:{s:02}")
}

/// Format a duration in milliseconds as a compact string.
pub fn format_duration(duration_ms: Option<u64>) -> String {
    let Some(ms) = duration_ms else {
        return "—".to_string();
    };
    let total_secs = ms / 1000;
    let minutes = total_secs / SECONDS_PER_MINUTE;
    let seconds = total_secs % SECONDS_PER_MINUTE;
    if minutes > 0 {
        format!("{minutes}m {seconds:02}s")
    } else {
        format!("{seconds}s")
    }
}

/// Convert a UTC ISO 8601 timestamp to local time, formatted as `yyyy-mm-dd hh:mm`.
pub fn format_timestamp(iso: &str) -> String {
    let utc_offset_secs = local_utc_offset_secs();
    let stripped = iso.trim_end_matches('Z');
    match stripped.split_once('T') {
        Some((date, time)) => {
            let date_parts: Vec<&str> = date.split('-').collect();
            let time_parts: Vec<&str> = time.split(':').collect();
            if date_parts.len() >= 3
                && time_parts.len() >= 2
                && let (Ok(y), Ok(month), Ok(day), Ok(hour), Ok(minute)) = (
                    date_parts[0].parse::<i64>(),
                    date_parts[1].parse::<i64>(),
                    date_parts[2].parse::<i64>(),
                    time_parts[0].parse::<i64>(),
                    time_parts[1].parse::<i64>(),
                )
            {
                let seconds_per_minute = SECONDS_PER_MINUTE.cast_signed();
                let minutes_per_day = MINUTES_PER_DAY.cast_signed();
                let minutes_per_hour = MINUTES_PER_HOUR.cast_signed();
                let total_mins =
                    hour * minutes_per_hour + minute + utc_offset_secs / seconds_per_minute;
                let mut day = day;
                let mut month = month;
                let mut year = y;
                let mut adj_mins = total_mins % minutes_per_day;
                if adj_mins < 0 {
                    adj_mins += minutes_per_day;
                    day -= 1;
                    if day < 1 {
                        month -= 1;
                        if month < 1 {
                            month = 12;
                            year -= 1;
                        }
                        day = days_in_month(year, month);
                    }
                } else if adj_mins >= minutes_per_day {
                    adj_mins -= minutes_per_day;
                    day += 1;
                    if day > days_in_month(year, month) {
                        day = 1;
                        month += 1;
                        if month > 12 {
                            month = 1;
                            year += 1;
                        }
                    }
                }
                let local_h = adj_mins / minutes_per_hour;
                let local_m = adj_mins % minutes_per_hour;
                return format!("{year:04}-{month:02}-{day:02} {local_h:02}:{local_m:02}");
            }
            let short_time = if time.len() >= 5 { &time[..5] } else { time };
            format!("{date} {short_time}")
        },
        None => stripped.to_string(),
    }
}

/// Render a rate-limit bucket as `"remaining/limit resets HH:MM:SS"`.
/// Returns the empty string when the bucket has not been observed yet;
/// drops the `resets …` suffix when no reset timestamp is available
/// **or** when the bucket is fully unused (`used == 0`). GitHub
/// re-bases the reset window on every `/rate_limit` poll for an
/// unused bucket, so including the countdown for those rows makes
/// the value oscillate between `HH:00:00` and `(HH-1):59:59` every
/// second. Nothing has been consumed there — no countdown to show.
pub(super) fn format_rate_limit_bucket(quota: Option<RateLimitQuota>) -> String {
    let Some(quota) = quota else {
        return String::new();
    };
    let base = format!("{}/{}", quota.remaining, quota.limit);
    if quota.used == 0 {
        return base;
    }
    let Some(reset_at) = quota.reset_at else {
        return base;
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let secs = reset_at.saturating_sub(now);
    format!("{base} resets {}", tui_pane::format_progressive(secs))
}

pub fn format_ahead_behind_against((ahead, behind): (usize, usize), branch: &str) -> String {
    match (ahead, behind) {
        (0, 0) => format!("{IN_SYNC} up to date with {branch}"),
        (ahead, 0) => format!("{SYNC_UP}{ahead} ahead of {branch}"),
        (0, behind) => format!("{SYNC_DOWN}{behind} behind {branch}"),
        (ahead, behind) => format!("{SYNC_UP}{ahead} {SYNC_DOWN}{behind} diverged from {branch}"),
    }
}

/// Render the Git pane's `Bisect` row, mirroring git's own
/// "N revisions left to test (roughly M steps)" phrasing.
pub(super) fn format_bisect_progress(progress: &BisectProgress) -> String {
    match progress {
        BisectProgress::Awaiting => "bisecting — mark a known-good & known-bad commit".to_string(),
        BisectProgress::Narrowing { revisions, steps } => {
            format!(
                "{} left · ~{}",
                pluralize(*revisions, "revision"),
                pluralize(*steps, "step"),
            )
        },
    }
}

fn pluralize(count: usize, noun: &str) -> String {
    if count == 1 {
        format!("{count} {noun}")
    } else {
        format!("{count} {noun}s")
    }
}

/// Spell out an ahead/behind pair for the Git pane's Remotes "Sync" and
/// Worktrees "vs primary" columns. Both columns share this phrasing; the
/// reference each is measured against is named by its column header (the
/// Remotes "Tracked" ref, the primary worktree) rather than inline.
pub fn format_ahead_behind(ahead_behind: Option<(usize, usize)>) -> String {
    match ahead_behind {
        Some((0, 0)) => IN_SYNC.to_string(),
        Some((ahead, 0)) => format!("{SYNC_UP}{ahead} ahead"),
        Some((0, behind)) => format!("{SYNC_DOWN}{behind} behind"),
        Some((ahead, behind)) => format!("{SYNC_UP}{ahead} ahead {SYNC_DOWN}{behind} behind"),
        None => NO_REMOTE_SYNC.to_string(),
    }
}
