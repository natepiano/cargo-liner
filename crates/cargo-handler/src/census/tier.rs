//! The service tier each Codex thread or process asked for, from the
//! first source that names one: the last `thread_settings_applied`
//! event in the thread's rollout, then `-c service_tier=` on the
//! process's command line, then the codex-pacer's history at the time
//! the thread began. A mesh's threads get their tier from the pacer
//! over the app server's websocket, which no file records, so Codex's
//! `config.toml` is never read: those threads override it.
//!
//! A rollout runs to tens of megabytes, so [`Rollouts`] reads each one
//! only as far as it has been written since the last scan.

use std::collections::HashMap;
use std::collections::HashSet;
use std::fs;
use std::fs::File;
use std::io::BufRead;
use std::io::BufReader;
use std::io::Seek;
use std::io::SeekFrom;
use std::path::Path;
use std::path::PathBuf;

use chrono::DateTime;
use serde::Deserialize;

use super::ServiceTier;
use crate::constants::CODEX_CONFIG_FLAGS;
use crate::constants::CODEX_CONFIG_PREFIX;
use crate::constants::CODEX_EVENT_LINE;
use crate::constants::CODEX_SERVICE_TIER_KEY;
use crate::constants::CODEX_SESSION_META_LINE;
use crate::constants::CODEX_THREAD_SETTINGS_EVENT;

/// What one rollout holds, read up to [`RolloutRead::read_to`].
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct RolloutRead {
    /// The offset just past the last whole line read.
    read_to:          u64,
    /// When the thread began, in unix seconds, from its `session_meta`
    /// line; none before that line is written whole.
    pub(super) began: Option<u64>,
    /// The tier the last `thread_settings_applied` event names.
    service_tier:     ServiceTier,
}

impl RolloutRead {
    /// Read the rollout at `path` on from [`Self::read_to`], from the
    /// start again when it has shrunk. A last line still being written
    /// is left for the next read.
    fn advance(&mut self, path: &Path) {
        let Ok(mut file) = File::open(path) else {
            return;
        };
        let Ok(length) = file.metadata().map(|metadata| metadata.len()) else {
            return;
        };
        if length < self.read_to {
            *self = Self::default();
        }
        if length == self.read_to || file.seek(SeekFrom::Start(self.read_to)).is_err() {
            return;
        }
        let mut reader = BufReader::new(file);
        let mut line = Vec::new();
        loop {
            line.clear();
            match reader.read_until(b'\n', &mut line) {
                Ok(bytes) if bytes > 0 && line.ends_with(b"\n") => {
                    self.take(&line);
                    self.read_to += bytes as u64;
                },
                _ => return,
            }
        }
    }

    /// Take what `line`, a whole line starting at [`Self::read_to`],
    /// says: when the thread began on the first line, its tier on a
    /// `thread_settings_applied` event.
    fn take(&mut self, line: &[u8]) {
        let first = self.read_to == 0;
        let event =
            std::str::from_utf8(line).is_ok_and(|text| text.contains(CODEX_THREAD_SETTINGS_EVENT));
        if !first && !event {
            return;
        }
        let Ok(parsed) = serde_json::from_slice::<RolloutLine>(line) else {
            return;
        };
        if first && parsed.kind == CODEX_SESSION_META_LINE {
            self.began = parsed
                .payload
                .timestamp
                .as_deref()
                .or(parsed.timestamp.as_deref())
                .and_then(unix_seconds);
        }
        if parsed.kind == CODEX_EVENT_LINE
            && parsed.payload.kind.as_deref() == Some(CODEX_THREAD_SETTINGS_EVENT)
        {
            self.service_tier = parsed
                .payload
                .thread_settings
                .and_then(|settings| settings.service_tier)
                .map_or(ServiceTier::Standard, |tier| {
                    ServiceTier::from(tier.as_str())
                });
        }
    }
}

/// One rollout line, as far as the tier read needs it.
#[derive(Deserialize)]
struct RolloutLine {
    /// `session_meta`, `event_msg`, and others.
    #[serde(rename = "type", default)]
    kind:      String,
    /// When the line was written.
    #[serde(default)]
    timestamp: Option<String>,
    /// The line's content.
    #[serde(default)]
    payload:   RolloutPayload,
}

/// The parts of a rollout line's `payload` the tier read needs.
#[derive(Default, Deserialize)]
struct RolloutPayload {
    /// An event's type.
    #[serde(rename = "type", default)]
    kind:            Option<String>,
    /// When a `session_meta` line's thread began.
    #[serde(default)]
    timestamp:       Option<String>,
    /// A `thread_settings_applied` event's settings.
    #[serde(default)]
    thread_settings: Option<ThreadSettings>,
}

/// The settings a `thread_settings_applied` event records.
#[derive(Deserialize)]
struct ThreadSettings {
    /// `priority`, `default`, or null.
    #[serde(default)]
    service_tier: Option<String>,
}

/// Every rollout the scanner reads, each as far as it has been read.
#[derive(Debug, Default)]
pub(super) struct Rollouts {
    /// What each rollout holds so far, by path.
    reads:   HashMap<PathBuf, RolloutRead>,
    /// The rollouts read since the last [`Rollouts::forget_unread`].
    current: HashSet<PathBuf>,
}

impl Rollouts {
    /// What the rollout at `path` holds, reading only what was written
    /// since the last read of it.
    pub(super) fn read(&mut self, path: &Path) -> RolloutRead {
        self.current.insert(path.to_path_buf());
        let read = self.reads.entry(path.to_path_buf()).or_default();
        read.advance(path);
        *read
    }

    /// Forget every rollout not read since the last call, as a scan
    /// ends.
    pub(super) fn forget_unread(&mut self) {
        let current = std::mem::take(&mut self.current);
        self.reads.retain(|path, _| current.contains(path));
    }
}

/// Each change of the tier the codex-pacer launches threads at, oldest
/// first, in unix seconds.
#[derive(Debug, Default)]
pub(super) struct PacerHistory {
    /// When each change took effect, and the tier it set.
    changes: Vec<(u64, ServiceTier)>,
}

/// The part of the pacer's state the history is read from.
#[derive(Deserialize)]
struct PacerState {
    /// `[[<RFC 3339 time>, <tier>], …]`.
    #[serde(default)]
    history: Vec<(String, String)>,
}

impl PacerHistory {
    /// The history in the pacer's state at `path`; none where the file is
    /// absent or cannot be read.
    pub(super) fn read(path: &Path) -> Self {
        let mut changes: Vec<(u64, ServiceTier)> = fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str::<PacerState>(&text).ok())
            .map(|state| {
                state
                    .history
                    .iter()
                    .filter_map(|(at, tier)| {
                        Some((unix_seconds(at)?, ServiceTier::from(tier.as_str())))
                    })
                    .collect()
            })
            .unwrap_or_default();
        changes.sort_by_key(|(at, _)| *at);
        Self { changes }
    }

    /// The tier in force at `at`, in unix seconds; unrecorded before the
    /// first change.
    fn tier_at(&self, at: u64) -> ServiceTier {
        self.changes
            .iter()
            .rev()
            .find(|(changed, _)| *changed <= at)
            .map_or(ServiceTier::Unrecorded, |(_, tier)| *tier)
    }
}

/// The tier a Codex thread or process asked for: `rollout`'s last
/// `thread_settings_applied` event, else `-c service_tier=` in
/// `arguments`, else the tier `pacer` had in force when the thread
/// began -- its rollout's `session_meta` time, else `started`.
pub(super) fn requested(
    rollout: Option<RolloutRead>,
    arguments: &[String],
    pacer: &PacerHistory,
    started: u64,
) -> ServiceTier {
    let began = rollout.and_then(|read| read.began).unwrap_or(started);
    rollout
        .map_or(ServiceTier::Unrecorded, |read| read.service_tier)
        .or_else(|| argument_tier(arguments))
        .or_else(|| pacer.tier_at(began))
}

/// The tier a `codex` command line `arguments` sets with `-c`,
/// `--config` or `--config=`, the last setting winning; unrecorded
/// where none does.
fn argument_tier(arguments: &[String]) -> ServiceTier {
    let mut tier = ServiceTier::Unrecorded;
    let mut rest = arguments.iter().skip(1);
    while let Some(argument) = rest.next() {
        let setting = if CODEX_CONFIG_FLAGS.contains(&argument.as_str()) {
            rest.next().map(String::as_str)
        } else {
            argument.strip_prefix(CODEX_CONFIG_PREFIX)
        };
        if let Some((key, value)) = setting.and_then(|setting| setting.split_once('='))
            && key.trim() == CODEX_SERVICE_TIER_KEY
        {
            tier = ServiceTier::from(unquoted(value.trim()));
        }
    }
    tier
}

/// `value` without one pair of surrounding `"` or `'`.
fn unquoted(value: &str) -> &str {
    ['"', '\'']
        .into_iter()
        .find_map(|quote| value.strip_prefix(quote)?.strip_suffix(quote))
        .unwrap_or(value)
}

/// An RFC 3339 time in unix seconds.
fn unix_seconds(text: &str) -> Option<u64> {
    u64::try_from(DateTime::parse_from_rfc3339(text).ok()?.timestamp()).ok()
}

#[cfg(test)]
#[expect(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::io::Write;

    use tempfile::TempDir;

    use super::*;

    /// A rollout's first line, for a thread that began at unix second
    /// [`BEGAN`].
    const META: &str = r#"{"timestamp":"2026-09-21T14:13:21Z","type":"session_meta","payload":{"timestamp":"2026-09-21T14:13:20Z"}}"#;
    /// The unix second [`META`] says its thread began.
    const BEGAN: u64 = 1_790_000_000;

    /// A `thread_settings_applied` line naming `tier`, a JSON value.
    fn settings(tier: &str) -> String {
        format!(
            r#"{{"type":"event_msg","payload":{{"type":"thread_settings_applied","thread_settings":{{"service_tier":{tier}}}}}}}"#
        )
    }

    /// Append `lines`, each ending in a newline, to the file at `path`.
    fn append(path: &Path, lines: &[&str]) {
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .expect("the fixture rollout should open");
        for line in lines {
            writeln!(file, "{line}").expect("the fixture line should write");
        }
    }

    /// `arguments` as a command line.
    fn command(arguments: &[&str]) -> Vec<String> {
        arguments
            .iter()
            .map(|argument| (*argument).to_string())
            .collect()
    }

    /// The last event wins, `priority` is fast, `default` and null are
    /// standard, and a rollout with no event records nothing.
    #[test]
    fn the_last_settings_event_names_the_tier() {
        let directory = TempDir::new().expect("a temporary directory should open");
        let path = directory.path().join("rollout.jsonl");
        let mut rollouts = Rollouts::default();
        append(&path, &[META, r#"{"type":"response_item","payload":{}}"#]);
        let read = rollouts.read(&path);
        assert_eq!(
            (read.began, read.service_tier),
            (Some(BEGAN), ServiceTier::Unrecorded)
        );

        for (tier, expected) in [
            (r#""default""#, ServiceTier::Standard),
            (r#""priority""#, ServiceTier::Fast),
            ("null", ServiceTier::Standard),
            (r#""priority""#, ServiceTier::Fast),
        ] {
            append(&path, &[&settings(tier)]);
            assert_eq!(rollouts.read(&path).service_tier, expected, "after {tier}");
        }
    }

    /// Each read starts where the last stopped, a last line still being
    /// written is left for later, a shrunk file is read again from the
    /// start, and a rollout not read in a scan is forgotten.
    #[test]
    fn rollouts_are_read_on_from_where_they_stopped() {
        let directory = TempDir::new().expect("a temporary directory should open");
        let path = directory.path().join("rollout.jsonl");
        append(&path, &[META]);
        let mut rollouts = Rollouts::default();
        let first = rollouts.read(&path);
        assert_eq!(first.read_to, META.len() as u64 + 1);

        let event = settings(r#""priority""#);
        let mut file = fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .expect("the fixture rollout should open");
        write!(file, "{event}").expect("the partial line should write");
        assert_eq!(
            rollouts.read(&path),
            first,
            "a partial line is not consumed"
        );
        writeln!(file).expect("the line should end");
        let whole = rollouts.read(&path);
        assert_eq!(whole.read_to, first.read_to + event.len() as u64 + 1);
        assert_eq!(whole.service_tier, ServiceTier::Fast);

        fs::write(&path, format!("{META}\n")).expect("the rollout should rewrite");
        assert_eq!(
            rollouts.read(&path),
            first,
            "a shrunk rollout is read again"
        );

        rollouts.forget_unread();
        rollouts.forget_unread();
        assert!(rollouts.reads.is_empty());
    }

    /// `-c service_tier=` sets the tier quoted or not, through `-c`,
    /// `--config` or `--config=`, the last one winning; an app server's
    /// command line names none.
    #[test]
    fn the_command_line_names_a_tier() {
        let pacer = PacerHistory::default();
        let tier = |arguments: &[&str]| requested(None, &command(arguments), &pacer, BEGAN);
        assert_eq!(
            tier(&["codex", "exec", "-c", r#"service_tier="fast""#]),
            ServiceTier::Fast
        );
        assert_eq!(
            tier(&["codex", "exec", "-c", "service_tier=fast"]),
            ServiceTier::Fast
        );
        assert_eq!(
            tier(&["codex", "--config=service_tier='flex'"]),
            ServiceTier::Standard
        );
        assert_eq!(
            tier(&[
                "codex",
                "-c",
                "service_tier=fast",
                "--config",
                "service_tier = \"default\""
            ]),
            ServiceTier::Standard
        );
        assert_eq!(
            tier(&["codex", "app-server", "--listen", "ws://127.0.0.1:43483"]),
            ServiceTier::Unrecorded
        );
    }

    /// A thread takes the tier the pacer had in force when it began: none
    /// before the first change, and a rollout's tier or the command
    /// line's wins over the pacer's. An absent state file records
    /// nothing.
    #[test]
    fn the_pacer_names_the_tier_in_force_when_a_thread_began() {
        let directory = TempDir::new().expect("a temporary directory should open");
        let path = directory.path().join("state.json");
        fs::write(
            &path,
            r#"{"files":{},"history":[["2026-09-21T09:30:00-04:00","default"],["2026-09-21T14:13:00Z","fast"]]}"#,
        )
        .expect("the fixture state should write");
        let pacer = PacerHistory::read(&path);
        let tier = |started: u64| requested(None, &[], &pacer, started);
        assert_eq!(tier(BEGAN - 3_000), ServiceTier::Unrecorded);
        assert_eq!(tier(BEGAN - 21), ServiceTier::Standard);
        assert_eq!(tier(BEGAN - 20), ServiceTier::Fast);
        assert_eq!(tier(BEGAN + 9_000), ServiceTier::Fast);

        let began_late = RolloutRead {
            began: Some(BEGAN - 21),
            ..RolloutRead::default()
        };
        assert_eq!(
            requested(Some(began_late), &[], &pacer, BEGAN),
            ServiceTier::Standard
        );
        let standard = RolloutRead {
            service_tier: ServiceTier::Standard,
            ..RolloutRead::default()
        };
        assert_eq!(
            requested(Some(standard), &[], &pacer, BEGAN),
            ServiceTier::Standard
        );
        assert_eq!(
            requested(
                None,
                &command(&["codex", "-c", "service_tier=flex"]),
                &pacer,
                BEGAN
            ),
            ServiceTier::Standard
        );

        let absent = PacerHistory::read(&directory.path().join("absent.json"));
        assert_eq!(
            requested(None, &[], &absent, BEGAN),
            ServiceTier::Unrecorded
        );
    }
}
