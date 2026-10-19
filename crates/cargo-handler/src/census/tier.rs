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
use std::path::Path;
use std::path::PathBuf;

use chrono::DateTime;
use serde::Deserialize;
use serde_json::Value;

use super::AgentActivity;
use super::ServiceTier;
use super::activity::ActivityRead;
use super::transcript;
use super::transcript::JsonlCursor;
use super::transcript::RecordKind;
use super::tree;
use super::worker_dispatch::JobText;
use super::worker_dispatch::WorkerTask;
use crate::constants::CODEX_CONFIG_FLAGS;
use crate::constants::CODEX_CONFIG_PREFIX;
use crate::constants::CODEX_EVENT_LINE;
use crate::constants::CODEX_RESPONSE_LINE;
use crate::constants::CODEX_SERVICE_TIER_KEY;
use crate::constants::CODEX_SESSION_META_LINE;
use crate::constants::CODEX_THREAD_SETTINGS_EVENT;
use crate::constants::CODEX_USER_MARKER;
use crate::constants::ROLLOUT_CONTEXT_PREFIXES;
use crate::constants::TRANSCRIPT_TAIL_START;
use crate::startup_log;

/// Cached metadata and the current conversation state of one rollout.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct RolloutRead {
    pub(super) current_prompt: CurrentReviewPrompt,
    activity_read:             ActivityRead,
    pub(super) first_prompt:   FirstTaskPrompt,
    pub(super) worktree:       ThreadWorktree,
    cursor:                    JsonlCursor,
    bytes_read:                u64,
    head:                      HeadValues,
    /// The rollout's session start in Unix seconds, or an unavailable start.
    pub(super) began:          RolloutStart,
    /// The tier the last `thread_settings_applied` event names.
    service_tier:              ServiceTier,
}

impl Default for RolloutRead {
    fn default() -> Self {
        Self {
            activity_read:  ActivityRead::default(),
            first_prompt:   FirstTaskPrompt::Unread,
            worktree:       ThreadWorktree::Unrecorded,
            cursor:         JsonlCursor::NeverRead,
            bytes_read:     0,
            head:           HeadValues::AwaitingSessionAndTask,
            began:          RolloutStart::Unavailable,
            current_prompt: CurrentReviewPrompt::Unread,
            service_tier:   ServiceTier::Unrecorded,
        }
    }
}

/// A session start recorded by the rollout, separate from the process start.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum RolloutStart {
    #[default]
    Unavailable,
    Recorded(u64),
}

/// The latest real task and its Unix time in milliseconds.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) enum CurrentReviewPrompt {
    #[default]
    Unread,
    Recorded {
        text:        String,
        recorded_at: u64,
    },
}

impl CurrentReviewPrompt {
    /// A new lease cannot reuse the previous review's task.
    pub(super) fn for_lease(&self, started: u64) -> WorkerTask {
        match self {
            Self::Recorded { text, recorded_at } if *recorded_at >= started => {
                super::worker_dispatch::WorkerTask::from_prompt(text)
            },
            Self::Unread | Self::Recorded { .. } => WorkerTask::RoleOnly,
        }
    }
}

/// Read user tasks at the JSON boundary and discard injected context.
fn task_text(value: &Value) -> Option<String> {
    let payload = &value["payload"];
    if value["type"] != "response_item" || payload["role"] != "user" {
        return None;
    }
    let text = payload["content"]
        .as_array()?
        .iter()
        .filter_map(|part| part["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n");
    (!text.is_empty()
        && !ROLLOUT_CONTEXT_PREFIXES
            .iter()
            .any(|prefix| text.starts_with(prefix)))
    .then_some(text)
}

/// A task remains unread until a real user prompt has been observed.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) enum FirstTaskPrompt {
    #[default]
    Unread,
    Recorded(String),
}
impl FirstTaskPrompt {
    /// Text passed to the task and roster parsers; unread input names only a role.
    pub(super) fn text(&self) -> &str {
        match self {
            Self::Unread => "",
            Self::Recorded(text) => text,
        }
    }
}

/// A missing cwd must not act as an empty filesystem path.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) enum ThreadWorktree {
    #[default]
    Unrecorded,
    Recorded(PathBuf),
}
impl ThreadWorktree {
    pub(super) fn seat_prefix(&self) -> Option<&str> {
        match self {
            Self::Unrecorded => None,
            Self::Recorded(path) => path.file_name().and_then(|name| name.to_str()),
        }
    }
}

/// The immutable head records that still need a complete line.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HeadValues {
    AwaitingSessionAndTask,
    AwaitingSession,
    AwaitingTask,
    Complete,
}

impl HeadValues {
    const fn with_session(self) -> Self {
        match self {
            Self::AwaitingSessionAndTask => Self::AwaitingTask,
            Self::AwaitingSession => Self::Complete,
            state => state,
        }
    }

    const fn with_task(self) -> Self {
        match self {
            Self::AwaitingSessionAndTask => Self::AwaitingSession,
            Self::AwaitingTask => Self::Complete,
            state => state,
        }
    }
}

impl RolloutRead {
    /// The turn reader is the sole source of conversation activity.
    pub(super) const fn activity(&self) -> AgentActivity { self.activity_read.activity() }

    /// The pool name and task belong to the same busy lease.
    pub(super) fn reviewer_task(&self, name: &str, lease_started: u64) -> WorkerTask {
        let job = match self.current_prompt.for_lease(lease_started) {
            WorkerTask::Task(text) => JobText::Recorded(text),
            _ => JobText::Unrecorded,
        };
        WorkerTask::Reviewer {
            name: name.to_owned(),
            job,
        }
    }

    /// Bytes fetched by the last read, including the one-time metadata bootstrap.
    pub(super) const fn bytes_read(&self) -> u64 { self.bytes_read }

    /// Bootstrap head metadata once, then consume only a tail and appended records.
    fn advance(&mut self, path: &Path) {
        if matches!(&self.cursor, JsonlCursor::Following { offset, .. }
            if fs::metadata(path).is_ok_and(|metadata| metadata.len() < *offset))
        {
            *self = Self::default();
        }
        self.bytes_read = 0;
        if matches!(self.cursor, JsonlCursor::NeverRead) {
            let Ok(file) = File::open(path) else { return };
            let mut reader = BufReader::new(file);
            let mut line = Vec::new();
            let mut offset = 0;
            while self.head != HeadValues::Complete {
                let Ok(bytes) = reader.read_until(b'\n', &mut line) else {
                    return;
                };
                if bytes == 0 || !line.ends_with(b"\n") {
                    break;
                }
                offset += bytes as u64;
                if [
                    CODEX_SESSION_META_LINE.as_bytes(),
                    CODEX_EVENT_LINE.as_bytes(),
                    CODEX_USER_MARKER,
                    CODEX_RESPONSE_LINE.as_bytes(),
                ]
                .iter()
                .any(|marker| line.windows(marker.len()).any(|part| part == *marker))
                {
                    self.take(&line);
                }
                line.clear();
            }
            line.extend_from_slice(reader.buffer());
            self.bytes_read = offset + line.len() as u64;
            self.cursor = JsonlCursor::AfterHead {
                offset,
                buffered: line,
            };
        }
        let batch = self.cursor.read_until(path, TRANSCRIPT_TAIL_START, |span| {
            let activity_known =
                std::str::from_utf8(span)
                    .unwrap_or_default()
                    .lines()
                    .any(|line| {
                        let line = line.as_bytes();
                        if !matches!(transcript::record_kind(line), RecordKind::Codex) {
                            return false;
                        }
                        let mut activity = ActivityRead::from_activity(AgentActivity::Unreported);
                        if let Ok(value) = serde_json::from_slice(line) {
                            activity.take_codex(&value);
                        }
                        activity.activity() != AgentActivity::Unreported
                    });
            let task_known = std::str::from_utf8(span)
                .unwrap_or_default()
                .lines()
                .any(|line| {
                    serde_json::from_str(line)
                        .ok()
                        .and_then(|value| task_text(&value))
                        .is_some()
                });
            activity_known
                && task_known
                && std::str::from_utf8(span)
                    .unwrap_or_default()
                    .lines()
                    .any(|line| {
                        let line = line.as_bytes();
                        if !line
                            .windows(CODEX_THREAD_SETTINGS_EVENT.len())
                            .any(|part| part == CODEX_THREAD_SETTINGS_EVENT.as_bytes())
                        {
                            return false;
                        }
                        serde_json::from_slice::<RolloutLine>(line).is_ok_and(|record| {
                            record.kind == CODEX_EVENT_LINE
                                && record.payload.kind.as_deref()
                                    == Some(CODEX_THREAD_SETTINGS_EVENT)
                        })
                    })
        });
        self.bytes_read += batch.bytes_read;
        for line in batch.lines {
            self.take(&line);
        }
    }

    /// Observe a complete event, retaining the first task and session metadata.
    fn take(&mut self, line: &[u8]) {
        if !matches!(
            transcript::record_kind(line),
            RecordKind::Codex | RecordKind::Session
        ) {
            return;
        }
        let Ok(value) = serde_json::from_slice::<serde_json::Value>(line) else {
            return;
        };
        {
            self.activity_read.take_codex(&value);
            let payload = &value["payload"];
            if value["type"] == CODEX_SESSION_META_LINE {
                self.head = self.head.with_session();
                self.worktree = payload["cwd"]
                    .as_str()
                    .filter(|path| !path.is_empty())
                    .map_or(ThreadWorktree::Unrecorded, |path| {
                        ThreadWorktree::Recorded(PathBuf::from(path))
                    });
            }
            if let Some(text) = task_text(&value) {
                if matches!(self.first_prompt, FirstTaskPrompt::Unread) {
                    self.first_prompt = FirstTaskPrompt::Recorded(text.clone());
                    self.head = self.head.with_task();
                }
                self.current_prompt = value["timestamp"]
                    .as_str()
                    .and_then(|text| DateTime::parse_from_rfc3339(text).ok())
                    .and_then(|time| u64::try_from(time.timestamp_millis()).ok())
                    .map_or(CurrentReviewPrompt::Unread, |recorded_at| {
                        CurrentReviewPrompt::Recorded { text, recorded_at }
                    });
            }
        }
        let Ok(parsed) = serde_json::from_value::<RolloutLine>(value) else {
            return;
        };
        if self.began == RolloutStart::Unavailable && parsed.kind == CODEX_SESSION_META_LINE {
            self.began = parsed
                .payload
                .timestamp
                .as_deref()
                .or(parsed.timestamp.as_deref())
                .and_then(unix_seconds)
                .map_or(RolloutStart::Unavailable, RolloutStart::Recorded);
        }
        if parsed.kind != CODEX_EVENT_LINE {
            return;
        }
        if parsed.payload.kind.as_deref() == Some(CODEX_THREAD_SETTINGS_EVENT) {
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
    /// Exact thread identity to its currently read rollout path.
    threads: HashMap<String, PathBuf>,
}

impl Rollouts {
    /// What the rollout at `path` holds, reading only what was written
    /// since the last read of it.
    pub(super) fn read(&mut self, path: &Path) -> RolloutRead {
        self.current.insert(path.to_path_buf());
        if let Some(id) = tree::thread_id(path) {
            self.threads.insert(id, path.to_path_buf());
        }
        let read = self.reads.entry(path.to_path_buf()).or_default();
        read.advance(path);
        startup_log::startup_event(&format!("rollout_bytes_read {}", read.bytes_read()));
        read.clone()
    }

    /// Detail already read for a live thread, without reopening its rollout.
    pub(super) fn thread(&self, id: &str) -> Option<&RolloutRead> {
        self.threads.get(id).and_then(|path| self.reads.get(path))
    }

    /// Forget every rollout not read since the last call, as a scan
    /// ends.
    pub(super) fn forget_unread(&mut self) {
        let current = std::mem::take(&mut self.current);
        self.reads.retain(|path, _| current.contains(path));
        self.threads.retain(|_, path| current.contains(path));
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
    let began = rollout
        .as_ref()
        .and_then(|read| match read.began {
            RolloutStart::Recorded(started) => Some(started),
            RolloutStart::Unavailable => None,
        })
        .unwrap_or(started);
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
            (RolloutStart::Recorded(BEGAN), ServiceTier::Unrecorded)
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
        assert_eq!(
            first.cursor,
            JsonlCursor::Following {
                offset:  META.len() as u64 + 1,
                pending: Vec::new(),
            }
        );

        let event = settings(r#""priority""#);
        let mut file = fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .expect("the fixture rollout should open");
        write!(file, "{event}").expect("the partial line should write");
        let partial = rollouts.read(&path);
        assert_eq!(
            partial.service_tier, first.service_tier,
            "an unfinished event does not change the tier"
        );
        assert_eq!(partial.bytes_read(), event.len() as u64);
        writeln!(file).expect("the line should end");
        let whole = rollouts.read(&path);
        assert_eq!(
            whole.bytes_read(),
            1,
            "the unfinished event was retained in memory"
        );
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
            began: RolloutStart::Recorded(BEGAN - 21),
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
    #[test]
    fn rollout_lookup_uses_exact_thread_identity_and_forgets_expired_paths() {
        let directory = TempDir::new().expect("rollout fixture");
        let id = "01a142f0-1234-5678-9abc-0123456789ab";
        let path = directory.path().join(format!("rollout-prefix-{id}.jsonl"));
        append(
            &path,
            &[
                META,
                r#"{"type":"event_msg","payload":{"type":"task_started"}}"#,
            ],
        );
        let mut rollouts = Rollouts::default();
        rollouts.read(&path);
        assert_eq!(
            rollouts.thread(id).map(RolloutRead::activity),
            Some(AgentActivity::Active(
                super::super::activity::Activity::Thinking
            ))
        );
        assert!(rollouts.thread("prefix").is_none());
        rollouts.forget_unread();
        rollouts.forget_unread();
        assert!(rollouts.thread(id).is_none());
    }

    #[test]
    fn absent_rollout_context_stays_typed_until_real_task_and_cwd_arrive() {
        let directory = TempDir::new().expect("rollout fixture");
        let path = directory.path().join("rollout.jsonl");
        let mut rollouts = Rollouts::default();
        let absent = rollouts.read(&path);
        assert!(matches!(absent.first_prompt, FirstTaskPrompt::Unread));
        assert!(matches!(absent.worktree, ThreadWorktree::Unrecorded));
        append(
            &path,
            &[
                r#"{"type":"session_meta","payload":{"cwd":"/work/fixture"}}"#,
                r#"{"type":"response_item","payload":{"role":"user","content":[{"text":"Repair the widget"}]}}"#,
            ],
        );
        let read = rollouts.read(&path);
        assert!(
            matches!(read.first_prompt, FirstTaskPrompt::Recorded(ref text) if text == "Repair the widget")
        );
        assert_eq!(read.worktree.seat_prefix(), Some("fixture"));
    }
}
