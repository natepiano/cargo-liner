use std::process::Command;
use std::sync::OnceLock;
use std::time::UNIX_EPOCH;

use unicode_width::UnicodeWidthStr;

use super::DetailField;
use crate::constants::GIT_STATUS_UNREADABLE;
use crate::constants::GIT_UNREADABLE_LABEL;
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
use crate::project::AheadBehind;
use crate::project::BisectProgress;
use crate::project::CheckoutInfo;
use crate::project::CommitDate;
use crate::project::Comparison;
use crate::project::FirstCommit;
use crate::project::GitStatus;
use crate::project::HeadState;
use crate::project::RemoteList;
use crate::project::RepoInfo;
use crate::project::ResolvedBranch;
use crate::project::TrackedRef;
use crate::tui::panes::constants::FIT_TEXT_ELLIPSIS;
use crate::tui::render;
use crate::tui::state::AvailabilityStatus;

/// Whether any completed git read failed, independently of path status.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::tui) enum GitReadHealth {
    Readable,
    Unreadable,
}

impl GitReadHealth {
    pub(in crate::tui) fn of(checkout: Option<&CheckoutInfo>, repo: Option<&RepoInfo>) -> Self {
        if GitReadFailures::of(checkout, repo).rows.is_empty() {
            Self::Readable
        } else {
            Self::Unreadable
        }
    }
}

/// The pane row or remote cell that owns a failed git read.
#[derive(Clone, PartialEq, Eq)]
enum UnreadableGitRow {
    Field(DetailField),
    RemoteTracked(usize),
    RemoteSync(usize),
}

/// Failed reads and their owning rows, shared by the pane and list markers.
#[derive(Clone, Default)]
pub(in crate::tui) struct GitReadFailures {
    rows: Vec<UnreadableGitRow>,
}

impl GitReadFailures {
    pub(super) fn of(checkout: Option<&CheckoutInfo>, repo: Option<&RepoInfo>) -> Self {
        let mut failures = Self::default();
        if let Some(info) = checkout {
            failures.observe_checkout(info);
        }
        if let Some(info) = repo {
            failures.observe_repo(info);
        }
        failures
    }

    fn observe_checkout(&mut self, info: &CheckoutInfo) {
        for (failed, field) in [
            (
                matches!(info.status, GitStatus::Unreadable),
                DetailField::GitStatus,
            ),
            (
                matches!(info.head, HeadState::Unreadable),
                DetailField::Head,
            ),
            (
                matches!(info.last_commit, CommitDate::Unreadable),
                DetailField::LastCommit,
            ),
            (
                matches!(info.primary_tracked_ref, TrackedRef::Unreadable),
                DetailField::Head,
            ),
            (
                matches!(info.ahead_behind_local, Comparison::Unreadable),
                DetailField::VsLocal,
            ),
            (
                matches!(info.bisect, Some(BisectProgress::Unreadable)),
                DetailField::Bisect,
            ),
        ] {
            if failed {
                self.rows.push(UnreadableGitRow::Field(field));
            }
        }
    }

    fn observe_repo(&mut self, info: &RepoInfo) {
        for (failed, field) in [
            (
                matches!(
                    info.first_commit,
                    FirstCommit::Probed(CommitDate::Unreadable)
                ),
                DetailField::Inception,
            ),
            (
                matches!(info.default_branch, ResolvedBranch::Unreadable),
                DetailField::Head,
            ),
            (
                matches!(info.local_main_branch, ResolvedBranch::Unreadable),
                DetailField::Head,
            ),
        ] {
            if failed {
                self.rows.push(UnreadableGitRow::Field(field));
            }
        }
        match &info.remotes {
            RemoteList::Unreadable => self
                .rows
                .push(UnreadableGitRow::Field(DetailField::RemotesUnreadable)),
            RemoteList::Listed(remotes) => {
                for (index, remote) in remotes.iter().enumerate() {
                    if matches!(remote.tracked_ref, TrackedRef::Unreadable) {
                        self.rows.push(UnreadableGitRow::RemoteTracked(index));
                    }
                    if matches!(remote.ahead_behind, Comparison::Unreadable) {
                        self.rows.push(UnreadableGitRow::RemoteSync(index));
                    }
                }
            },
        }
    }

    pub(super) fn fields(&self) -> impl Iterator<Item = DetailField> + '_ {
        self.rows.iter().filter_map(|row| match row {
            UnreadableGitRow::Field(field) => Some(*field),
            UnreadableGitRow::RemoteTracked(_) | UnreadableGitRow::RemoteSync(_) => None,
        })
    }

    /// Readable means no failure is recorded for this row; it does not promise a completed probe.
    pub(in crate::tui::panes) fn field_health(&self, field: DetailField) -> GitReadHealth {
        if self.rows.contains(&UnreadableGitRow::Field(field)) {
            GitReadHealth::Unreadable
        } else {
            GitReadHealth::Readable
        }
    }

    pub(super) fn remote_tracked_health(&self, index: usize) -> GitReadHealth {
        if self.rows.contains(&UnreadableGitRow::RemoteTracked(index)) {
            GitReadHealth::Unreadable
        } else {
            GitReadHealth::Readable
        }
    }

    pub(super) fn remote_sync_health(&self, index: usize) -> GitReadHealth {
        if self.rows.contains(&UnreadableGitRow::RemoteSync(index)) {
            GitReadHealth::Unreadable
        } else {
            GitReadHealth::Readable
        }
    }
}

/// The two compact sync cells share all of their unreadable evidence.
pub(in crate::tui) struct GitSyncLabels {
    pub(in crate::tui) origin: String,
    pub(in crate::tui) main:   String,
}

impl GitSyncLabels {
    pub(in crate::tui) fn of(checkout: Option<&CheckoutInfo>, repo: Option<&RepoInfo>) -> Self {
        match GitReadHealth::of(checkout, repo) {
            GitReadHealth::Unreadable => Self {
                origin: GIT_STATUS_UNREADABLE.into(),
                main:   GIT_STATUS_UNREADABLE.into(),
            },
            GitReadHealth::Readable => checkout.map_or_else(
                || Self {
                    origin: String::new(),
                    main:   String::new(),
                },
                |info| match info.status {
                    GitStatus::Untracked | GitStatus::Ignored => Self {
                        origin: String::new(),
                        main:   String::new(),
                    },
                    GitStatus::Clean | GitStatus::Modified | GitStatus::Unreadable => Self {
                        origin: format_sync(repo.map_or(Comparison::NotCompared, |repo| {
                            info.primary_ahead_behind(repo)
                        })),
                        main:   format_sync(info.ahead_behind_local),
                    },
                },
            ),
        }
    }
}

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

/// `TimestampZone` retains the time slice and whether an embedded offset already
/// makes it local; only Utc needs the local offset when formatting its time.
enum TimestampZone<'a> {
    Embedded { date: &'a str, time: &'a str },
    Utc(&'a str),
    Unzoned(&'a str),
    MissingTime,
}

impl<'a> From<&'a str> for TimestampZone<'a> {
    fn from(iso: &'a str) -> Self {
        let stripped = iso.trim_end_matches('Z');
        let Some((date, time)) = stripped.split_once('T') else {
            return Self::MissingTime;
        };
        if let Some(pos) = time.rfind(['+', '-']).filter(|pos| *pos > 0) {
            return Self::Embedded {
                date,
                time: &time[..pos],
            };
        }
        if iso.ends_with('Z') {
            Self::Utc(time)
        } else {
            Self::Unzoned(time)
        }
    }
}

/// Extract the local date from an ISO 8601 timestamp as `yyyy-mm-dd`.
///
/// If the timestamp has an embedded timezone offset, the date portion is
/// already local and is returned directly. For UTC timestamps, the local
/// offset is applied via `format_timestamp`.
pub fn format_date(iso: &str) -> String {
    match TimestampZone::from(iso) {
        TimestampZone::Embedded { date, .. } => return date.to_string(),
        TimestampZone::Utc(_) | TimestampZone::Unzoned(_) | TimestampZone::MissingTime => {},
    }
    let full = format_timestamp(iso);
    full.split(' ').next().unwrap_or(&full).to_string()
}

/// Extract the local time portion from an ISO 8601 timestamp as `hh:mm:ss`.
///
/// If the timestamp has an embedded timezone offset (e.g., `-04:00`), the
/// time is already local and no offset is applied. A trailing `Z` requests
/// the local UTC offset; an unzoned time keeps its existing clock value.
pub fn format_time(iso: &str) -> String {
    let timestamp_zone = TimestampZone::from(iso);
    let time_str = match timestamp_zone {
        TimestampZone::Embedded { time, .. }
        | TimestampZone::Unzoned(time)
        | TimestampZone::Utc(time) => time,
        TimestampZone::MissingTime => return "—".to_string(),
    };

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

    let offset = match timestamp_zone {
        TimestampZone::Embedded { .. } | TimestampZone::Unzoned(_) => 0,
        TimestampZone::Utc(_) => local_utc_offset_secs(),
        TimestampZone::MissingTime => return "—".to_string(),
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

/// `format_rate_limit_bucket` formats `quota.remaining`, `quota.limit`, and an active reset
/// countdown. The countdown stays hidden when `quota.used == 0` or `quota.reset_at` has expired.
/// GitHub resets the window on each poll when `quota.used == 0`.
/// An unused bucket's countdown alternates between `HH:00:00` and `(HH-1):59:59` each second.
pub(super) fn format_rate_limit_bucket(quota: RateLimitQuota) -> String {
    let base = format!("{}/{}", quota.remaining, quota.limit);
    if quota.used == 0 {
        return base;
    }
    let Some(reset_at) = quota.reset_at else {
        return base;
    };
    let now = std::time::SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let secs = reset_at.saturating_sub(now);
    if secs == 0 {
        return base;
    }
    format!("{base} resets {}", tui_pane::format_progressive(secs))
}

pub(super) fn format_ahead_behind_against(comparison: Comparison, branch: &str) -> String {
    let (ahead, behind) = match comparison {
        Comparison::Counted(AheadBehind { ahead, behind }) => (ahead, behind),
        Comparison::NotCompared => return String::new(),
        Comparison::Unreadable => return GIT_UNREADABLE_LABEL.to_string(),
    };
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
        BisectProgress::Unreadable => format!("bisecting — progress {GIT_UNREADABLE_LABEL}"),
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
pub fn format_ahead_behind(comparison: Comparison) -> String {
    match comparison {
        Comparison::Counted(AheadBehind {
            ahead: 0,
            behind: 0,
        }) => IN_SYNC.to_string(),
        Comparison::Counted(AheadBehind { ahead, behind: 0 }) => format!("{SYNC_UP}{ahead} ahead"),
        Comparison::Counted(AheadBehind { ahead: 0, behind }) => {
            format!("{SYNC_DOWN}{behind} behind")
        },
        Comparison::Counted(AheadBehind { ahead, behind }) => {
            format!("{SYNC_UP}{ahead} ahead {SYNC_DOWN}{behind} behind")
        },
        Comparison::NotCompared => NO_REMOTE_SYNC.to_string(),
        Comparison::Unreadable => GIT_UNREADABLE_LABEL.to_string(),
    }
}

/// Fit sync text with whole counts and whole words.
pub(in crate::tui::panes) fn fit_sync_text(text: &str, width: usize) -> String {
    if text.width() <= width {
        return text.to_string();
    }
    if width == 0 {
        return String::new();
    }
    let mut prefix =
        render::truncate_with_ellipsis(text, width.saturating_sub(FIT_TEXT_ELLIPSIS.width()), "");
    let remaining = &text[prefix.len()..];
    if !remaining.starts_with(char::is_whitespace)
        && !remaining.starts_with(SYNC_UP)
        && !remaining.starts_with(SYNC_DOWN)
    {
        let start = prefix
            .rfind(char::is_whitespace)
            .into_iter()
            .chain(prefix.rfind(SYNC_UP))
            .chain(prefix.rfind(SYNC_DOWN))
            .max()
            .unwrap_or(0);
        prefix.truncate(start);
    }
    format!("{}{FIT_TEXT_ELLIPSIS}", prefix.trim_end())
}

/// Compact sync text shared by the project rows and their fitted widths.
fn format_sync(comparison: Comparison) -> String {
    match comparison {
        Comparison::Counted(AheadBehind {
            ahead: 0,
            behind: 0,
        }) => IN_SYNC.to_string(),
        Comparison::Counted(AheadBehind { ahead, behind: 0 }) => format!("{SYNC_UP}{ahead}"),
        Comparison::Counted(AheadBehind { ahead: 0, behind }) => format!("{SYNC_DOWN}{behind}"),
        Comparison::Counted(AheadBehind { ahead, behind }) => {
            format!("{SYNC_UP}{ahead}{SYNC_DOWN}{behind}")
        },
        Comparison::NotCompared => NO_REMOTE_SYNC.to_string(),
        Comparison::Unreadable => GIT_STATUS_UNREADABLE.to_string(),
    }
}

pub(super) fn format_commit_date(date: &CommitDate) -> String {
    match date {
        CommitDate::At(timestamp) => format_timestamp(timestamp),
        CommitDate::NoCommits => String::new(),
        CommitDate::Unreadable => GIT_UNREADABLE_LABEL.to_string(),
    }
}

pub(super) fn format_tracked_ref(reference: &TrackedRef) -> String {
    match reference {
        TrackedRef::Named(name) => name.clone(),
        TrackedRef::Untracked => NO_REMOTE_SYNC.to_string(),
        TrackedRef::Unreadable => GIT_UNREADABLE_LABEL.to_string(),
    }
}

/// Shared GitHub status text for the Git pane.
pub(super) const fn github_status_text(status: AvailabilityStatus) -> &'static str {
    match status {
        AvailabilityStatus::Reachable => "",
        AvailabilityStatus::Unreachable => "GitHub unreachable",
        AvailabilityStatus::RateLimited => "github rate-limited",
        AvailabilityStatus::Unauthenticated => "unauthenticated — gh auth login",
        AvailabilityStatus::NotInstalled => "gh not installed",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sync_value_uses_synced_label_when_in_sync() {
        assert_eq!(
            format_ahead_behind(Comparison::Counted(AheadBehind {
                ahead:  0,
                behind: 0,
            })),
            "☑️"
        );
    }

    #[test]
    fn local_ahead_behind_values_name_the_compared_branch() {
        let cases = [
            ((8, 0), "↑8 ahead of main"),
            ((0, 2), "↓2 behind main"),
            ((8, 2), "↑8 ↓2 diverged from main"),
            ((0, 0), "☑️ up to date with main"),
        ];

        for (ahead_behind, expected) in cases {
            assert_eq!(
                format_ahead_behind_against(
                    Comparison::Counted(AheadBehind {
                        ahead:  ahead_behind.0,
                        behind: ahead_behind.1,
                    }),
                    "main"
                ),
                expected
            );
        }
        for (value, width, expected) in [
            ("↑1234 ↓5678 diverged from main", 11, "↑1234…"),
            ("↑1234 ↓5678 diverged from main", 12, "↑1234 ↓5678…"),
            ("↑1 ↓91 diverged from main", 12, "↑1 ↓91…"),
            ("↑1 ↓91 diverged from main", 25, "↑1 ↓91 diverged from main"),
            ("↑1234↓5678", 6, "↑1234…"),
        ] {
            assert_eq!(fit_sync_text(value, width), expected);
        }
    }
}
