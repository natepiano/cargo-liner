//! Reading Claude Code's transcripts: the shell commands a session ran
//! around a moment, and the subagents running inside it.
//!
//! A session's transcript is one JSON object per line, appended as the
//! session goes, at `~/.claude/projects/<directory>/<session id>.jsonl`,
//! where `<directory>` is the directory the session started in with
//! every character but an ASCII letter or digit written as `-`. Each of
//! its subagents writes a transcript of its own under
//! `<session id>/subagents/`, beside a `.meta.json` saying what the
//! subagent was asked to do.
//!
//! A day-long session's transcript runs to hundreds of megabytes, so
//! nothing here reads one whole. Shell commands are read only from a
//! span of time, found by bisecting the file on its lines' timestamps,
//! and a subagent's state is read from the end of its file.

use std::collections::HashMap;
use std::fs;
use std::fs::File;
use std::io::BufRead;
use std::io::BufReader;
use std::io::Read;
use std::io::Seek;
use std::io::SeekFrom;
use std::ops::RangeInclusive;
use std::path::Path;
use std::path::PathBuf;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use chrono::DateTime;
use serde::Deserialize;

use super::AgentActivity;
use crate::constants::ASSISTANT_LINE;
use crate::constants::BASH_TOOL;
use crate::constants::BASH_TOOL_MARKER;
use crate::constants::END_TURN;
use crate::constants::SUBAGENT_META_SUFFIX;
use crate::constants::SUBAGENT_PREFIX;
use crate::constants::SUBAGENT_QUIET_LIMIT;
use crate::constants::SUBAGENTS_DIRNAME;
use crate::constants::TOOL_USE_BLOCK;
use crate::constants::TRANSCRIPT_BEGIN_READ_LIMIT;
use crate::constants::TRANSCRIPT_BISECT_GRAIN;
use crate::constants::TRANSCRIPT_EXTENSION;
use crate::constants::TRANSCRIPT_SPAN_READ_LIMIT;
use crate::constants::TRANSCRIPT_TAIL_GROWTH;
use crate::constants::TRANSCRIPT_TAIL_START;
use crate::constants::USER_LINE;

/// A cursor owns the bytes fetched in one path generation.
/// In `Following`, `offset` is the next unread file position and `pending`
/// holds the final record fragment. In `AfterHead`, `offset` is the end of
/// the last head record and `buffered` holds bytes already fetched after it.
/// Both buffers prevent a second read of those bytes.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) enum JsonlCursor {
    #[default]
    NeverRead,
    AfterHead {
        offset:   u64,
        buffered: Vec<u8>,
    },
    Following {
        offset:  u64,
        pending: Vec<u8>,
    },
}

/// Whether a read starts on a whole record or inside a record.
#[derive(Clone, Copy)]
enum RecordBoundary {
    Whole,
    Fragment,
}

impl RecordBoundary {
    const fn at(start: u64, floor: u64) -> Self {
        if start == floor {
            Self::Whole
        } else {
            Self::Fragment
        }
    }

    fn complete(self, bytes: &[u8]) -> &[u8] {
        let from = match self {
            Self::Whole => 0,
            Self::Fragment => bytes
                .iter()
                .position(|byte| *byte == b'\n')
                .map_or(bytes.len(), |index| index + 1),
        };
        let to = bytes
            .iter()
            .rposition(|byte| *byte == b'\n')
            .map_or(0, |index| index + 1);
        &bytes[from.min(to)..to]
    }
}

/// Record families that can supply conversation state.
#[derive(Default, Deserialize)]
pub(super) enum RecordKind {
    #[serde(rename = "assistant", alias = "user")]
    Claude,
    #[serde(rename = "event_msg", alias = "response_item")]
    Codex,
    #[serde(rename = "session_meta")]
    Session,
    #[default]
    #[serde(other)]
    Other,
}

#[derive(Deserialize)]
struct RecordHeader {
    #[serde(rename = "type", default)]
    kind: RecordKind,
}

/// Read the record family without allocating its large content fields.
pub(super) fn record_kind(line: &[u8]) -> RecordKind {
    serde_json::from_slice::<RecordHeader>(line).map_or(RecordKind::Other, |header| header.kind)
}

/// Complete records fetched by one read, and its actual file traffic.
#[derive(Debug, Default)]
pub(super) struct ReadBatch {
    pub(super) lines:      Vec<Vec<u8>>,
    pub(super) bytes_read: u64,
}

impl JsonlCursor {
    /// Start with a small tail, then expand through disjoint ranges until all
    /// requested values are known or the head boundary is reached.
    /// Later calls fetch only appended bytes.
    pub(super) fn read_until(
        &mut self,
        path: &Path,
        initial_tail_bytes: u64,
        decisive: impl Fn(&[u8]) -> bool,
    ) -> ReadBatch {
        let Ok(mut file) = File::open(path) else {
            return ReadBatch::default();
        };
        let Ok(length) = file.metadata().map(|metadata| metadata.len()) else {
            return ReadBatch::default();
        };
        if matches!(self, Self::Following { offset, .. } if *offset > length) {
            *self = Self::NeverRead;
        }
        let floor = match self {
            Self::AfterHead { offset, buffered } => *offset + buffered.len() as u64,
            _ => 0,
        };
        let mut start = match self {
            Self::Following { offset, .. } => *offset,
            _ => length.saturating_sub(initial_tail_bytes).max(floor),
        };
        let mut boundary = match self {
            Self::Following { .. } => RecordBoundary::Whole,
            _ => RecordBoundary::at(start, floor),
        };
        let mut bytes = match self {
            Self::Following { pending, .. } => pending.clone(),
            Self::AfterHead { buffered, .. } if start == floor => buffered.clone(),
            _ => Vec::new(),
        };
        if file.seek(SeekFrom::Start(start)).is_err() {
            return ReadBatch::default();
        }
        let previous = bytes.len();
        if std::io::Read::by_ref(&mut file)
            .take(length - start)
            .read_to_end(&mut bytes)
            .is_err()
        {
            return ReadBatch::default();
        }
        let mut bytes_read = (bytes.len() - previous) as u64;
        if !matches!(self, Self::Following { .. }) {
            while start > floor && !decisive(boundary.complete(&bytes)) {
                let span = (length - start).max(initial_tail_bytes);
                let next = length
                    .saturating_sub(span.saturating_mul(TRANSCRIPT_TAIL_GROWTH))
                    .max(floor);
                let mut prefix = Vec::new();
                if file.seek(SeekFrom::Start(next)).is_err()
                    || std::io::Read::by_ref(&mut file)
                        .take(start - next)
                        .read_to_end(&mut prefix)
                        .is_err()
                {
                    return ReadBatch::default();
                }
                bytes_read += prefix.len() as u64;
                if next == floor
                    && let Self::AfterHead { buffered, .. } = self
                {
                    let mut head = buffered.clone();
                    head.extend(prefix);
                    prefix = head;
                }
                prefix.extend(bytes);
                bytes = prefix;
                start = next;
                boundary = RecordBoundary::at(start, floor);
            }
        }
        let mut batch = ReadBatch {
            lines: Vec::new(),
            bytes_read,
        };
        let mut remaining = bytes.as_slice();
        let mut pending = Vec::new();
        while !remaining.is_empty() {
            let mut line = Vec::new();
            if remaining.read_until(b'\n', &mut line).is_err() {
                return ReadBatch::default();
            }
            if !line.ends_with(b"\n") {
                pending = line;
                break;
            }
            line.pop();
            match boundary {
                RecordBoundary::Whole => batch.lines.push(line),
                RecordBoundary::Fragment => boundary = RecordBoundary::Whole,
            }
        }
        *self = Self::Following {
            offset: length,
            pending,
        };
        batch
    }
}

/// The transcript of the session `session_id` started in `cwd`, under
/// Claude Code's `projects` directory.
pub(super) fn transcript_path(projects: &Path, cwd: &Path, session_id: &str) -> PathBuf {
    let directory: String = cwd
        .to_string_lossy()
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character
            } else {
                '-'
            }
        })
        .collect();
    projects
        .join(directory)
        .join(format!("{session_id}.{TRANSCRIPT_EXTENSION}"))
}

/// The transcript of the session `session_id` under Claude Code's
/// `projects` directory: where `cwd`, the session's directory, says when
/// it is there, else in whichever project holds it, as the transcript of
/// a session resumed or moved away from where it started does. None
/// when no project holds it yet.
pub(super) fn find_transcript(
    projects: &Path,
    cwd: Option<&Path>,
    session_id: &str,
) -> Option<PathBuf> {
    if let Some(path) = cwd
        .map(|cwd| transcript_path(projects, cwd, session_id))
        .filter(|path| path.is_file())
    {
        return Some(path);
    }
    let file_name = format!("{session_id}.{TRANSCRIPT_EXTENSION}");
    fs::read_dir(projects)
        .ok()?
        .filter_map(Result::ok)
        .map(|project| project.path().join(&file_name))
        .find(|path| path.is_file())
}

/// One call a session made to its shell tool.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct BashCall {
    /// When the call was written, in unix milliseconds.
    pub(super) at_ms:       u64,
    /// The command it ran, as the session wrote it.
    pub(super) command:     String,
    /// What the session said the command does, where it said.
    pub(super) description: Option<String>,
}

/// The part of any transcript line a bisection reads.
#[derive(Deserialize)]
struct Stamped {
    /// When the line was written; some kinds of line carry none.
    #[serde(default)]
    timestamp: Option<String>,
}

/// A line that may hold shell tool calls.
#[derive(Deserialize)]
struct CallLine {
    /// When the line was written.
    #[serde(default)]
    timestamp: Option<String>,
    /// `assistant` for a line the session wrote itself.
    #[serde(rename = "type", default)]
    kind:      String,
    /// The message the line carries.
    #[serde(default)]
    message:   Option<CallMessage>,
}

/// An assistant message: a list of blocks, some of them tool calls.
#[derive(Deserialize)]
struct CallMessage {
    /// The message's blocks.
    #[serde(default)]
    content: Vec<ContentBlock>,
}

/// One block of an assistant message.
#[derive(Deserialize)]
struct ContentBlock {
    /// `tool_use` for a tool call.
    #[serde(rename = "type", default)]
    kind:  String,
    /// The tool called.
    #[serde(default)]
    name:  Option<String>,
    /// What the tool was handed.
    #[serde(default)]
    input: Option<BashInput>,
}

/// What the shell tool is handed.
#[derive(Deserialize)]
struct BashInput {
    /// The command.
    #[serde(default)]
    command:     Option<String>,
    /// What the session said the command does.
    #[serde(default)]
    description: Option<String>,
}

/// The shell tool calls in the transcript at `path` written within
/// `span`, in unix milliseconds, in the order they were written.
///
/// None when there is no transcript. The read stops at the first line
/// past the span, or [`TRANSCRIPT_SPAN_READ_LIMIT`] bytes in.
pub(super) fn bash_calls(path: &Path, span: &RangeInclusive<u64>) -> Vec<BashCall> {
    let Ok(file) = File::open(path) else {
        return Vec::new();
    };
    let Ok(length) = file.metadata().map(|metadata| metadata.len()) else {
        return Vec::new();
    };
    let start = span_start(&file, length, *span.start());
    let mut reader = BufReader::new(&file);
    let Ok(mut read) = line_after(&mut reader, start) else {
        return Vec::new();
    };
    let mut calls = Vec::new();
    let mut line = Vec::new();
    while read <= TRANSCRIPT_SPAN_READ_LIMIT {
        line.clear();
        match reader.read_until(b'\n', &mut line) {
            Ok(0) | Err(_) => break,
            Ok(bytes) => read = read.saturating_add(bytes as u64),
        }
        // A line with no shell call is read for its time alone, which
        // is all it can end the span with.
        if !contains(&line, BASH_TOOL_MARKER.as_bytes()) {
            match line_time(&line) {
                Some(at) if at > *span.end() => break,
                _ => continue,
            }
        }
        let Ok(parsed) = serde_json::from_slice::<CallLine>(&line) else {
            continue;
        };
        let Some(at_ms) = parsed.timestamp.as_deref().and_then(unix_ms) else {
            continue;
        };
        if at_ms > *span.end() {
            break;
        }
        if at_ms < *span.start() || parsed.kind != ASSISTANT_LINE {
            continue;
        }
        let blocks = parsed.message.map(|message| message.content);
        calls.extend(
            blocks
                .into_iter()
                .flatten()
                .filter(|block| {
                    block.kind == TOOL_USE_BLOCK && block.name.as_deref() == Some(BASH_TOOL)
                })
                .filter_map(|block| block.input)
                .filter_map(|input| {
                    Some(BashCall {
                        at_ms,
                        command: input.command?,
                        description: input.description,
                    })
                }),
        );
    }
    calls
}

/// When the session whose transcript is at `path` began, in unix
/// seconds: the time of its first stamped line. None when there is no
/// transcript, or no stamped line in its first
/// [`TRANSCRIPT_BEGIN_READ_LIMIT`] bytes.
pub(super) fn began(path: &Path) -> Option<u64> {
    let file = File::open(path).ok()?;
    let length = file.metadata().ok()?.len();
    first_time_after(&file, 0, length.min(TRANSCRIPT_BEGIN_READ_LIMIT)).map(|at| at / 1_000)
}

/// A byte offset at or before the first line written at or after
/// `from_ms`, found by bisecting the file on its lines' timestamps to
/// within [`TRANSCRIPT_BISECT_GRAIN`] bytes.
fn span_start(file: &File, length: u64, from_ms: u64) -> u64 {
    let (mut low, mut high) = (0, length);
    while high.saturating_sub(low) > TRANSCRIPT_BISECT_GRAIN {
        let middle = low + (high - low) / 2;
        match first_time_after(file, middle, high) {
            Some(at) if at < from_ms => low = middle,
            _ => high = middle,
        }
    }
    low
}

/// The time of the first stamped line that starts after `offset` and
/// before `limit`.
fn first_time_after(file: &File, offset: u64, limit: u64) -> Option<u64> {
    let mut reader = BufReader::new(file);
    let mut at = offset.saturating_add(line_after(&mut reader, offset).ok()?);
    let mut line = Vec::new();
    while at < limit {
        line.clear();
        let bytes = reader.read_until(b'\n', &mut line).ok()?;
        if bytes == 0 {
            return None;
        }
        if let Some(time) = line_time(&line) {
            return Some(time);
        }
        at = at.saturating_add(bytes as u64);
    }
    None
}

/// Position `reader` at the start of the first line that starts at or
/// after `offset`, answering how many bytes past `offset` that is.
fn line_after(reader: &mut BufReader<&File>, offset: u64) -> std::io::Result<u64> {
    if offset == 0 {
        reader.seek(SeekFrom::Start(0))?;
        return Ok(0);
    }
    // The byte before `offset` says whether a line starts at `offset`.
    reader.seek(SeekFrom::Start(offset - 1))?;
    let mut partial = Vec::new();
    let skipped = reader.read_until(b'\n', &mut partial)?;
    Ok((skipped as u64).saturating_sub(1))
}

/// When `line` was written, in unix milliseconds, where it says.
fn line_time(line: &[u8]) -> Option<u64> {
    serde_json::from_slice::<Stamped>(line)
        .ok()?
        .timestamp
        .as_deref()
        .and_then(unix_ms)
}

/// An RFC 3339 time in unix milliseconds.
fn unix_ms(text: &str) -> Option<u64> {
    u64::try_from(DateTime::parse_from_rfc3339(text).ok()?.timestamp_millis()).ok()
}

/// Whether `needle` occurs in `haystack`.
fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

/// A subagent running inside a session.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct Subagent {
    /// The subagent's id, which a subagent it started names as its
    /// parent.
    pub(super) id:          String,
    /// The id of the subagent that started this one, where one did.
    pub(super) parent:      Option<String>,
    /// What the subagent was asked to do, else its kind.
    pub(super) description: String,
    /// When it started, in unix seconds.
    pub(super) started:     u64,
    /// Its own transcript, where the shell commands it runs are written.
    pub(super) transcript:  PathBuf,
    /// The current tool activity from this subagent's own transcript.
    pub(super) activity:    super::AgentActivity,
}

/// What a subagent's `.meta.json` says.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SubagentMeta {
    /// What the subagent was asked to do.
    #[serde(default)]
    description:     Option<String>,
    /// The subagent's kind, such as `general-purpose`.
    #[serde(default)]
    agent_type:      Option<String>,
    /// The subagent that started this one.
    #[serde(default)]
    parent_agent_id: Option<String>,
}

/// The subagents of the session whose transcript is `session_transcript`
/// that are still running at `now`: those whose own transcript does not
/// end on a finished turn and was written within
/// [`SUBAGENT_QUIET_LIMIT`]. One quiet longer than that was stopped
/// before it could finish.
pub(super) fn running_subagents(
    session_transcript: &Path,
    now: SystemTime,
    reads: &mut SubagentReads,
) -> Vec<Subagent> {
    let directory = session_transcript
        .with_extension("")
        .join(SUBAGENTS_DIRNAME);
    let Ok(entries) = fs::read_dir(&directory) else {
        return Vec::new();
    };
    let mut running: Vec<Subagent> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let file_name = entry.file_name();
            let id = file_name
                .to_str()?
                .strip_suffix(SUBAGENT_META_SUFFIX)?
                .strip_prefix(SUBAGENT_PREFIX)?
                .to_string();
            let transcript =
                directory.join(format!("{SUBAGENT_PREFIX}{id}.{TRANSCRIPT_EXTENSION}"));
            let metadata = fs::metadata(&transcript).ok()?;
            let quiet = now
                .duration_since(metadata.modified().ok()?)
                .unwrap_or_default();
            if quiet > SUBAGENT_QUIET_LIMIT
                || matches!(reads.has_finished(&transcript), SubagentTurn::Finished)
            {
                return None;
            }
            let meta: SubagentMeta =
                serde_json::from_str(&fs::read_to_string(entry.path()).ok()?).ok()?;
            let started = metadata
                .created()
                .or_else(|_| entry.metadata().and_then(|meta| meta.modified()))
                .ok()
                .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                .map_or(0, |since| since.as_secs());
            let description = meta
                .description
                .filter(|description| !description.is_empty())
                .or(meta.agent_type)
                .unwrap_or_else(|| id.clone());
            Some(Subagent {
                id,
                parent: meta.parent_agent_id,
                description,
                started,
                transcript,
                activity: AgentActivity::Idle,
            })
        })
        .collect();
    running.sort_by(|left, right| (left.started, &left.id).cmp(&(right.started, &right.id)));
    running
}

/// The part of a transcript line that says whose turn it was and how
/// it ended.
#[derive(Deserialize)]
struct Turn {
    /// `assistant` or `user` for a line of the conversation; other kinds
    /// of line are bookkeeping.
    #[serde(rename = "type", default)]
    kind:    String,
    /// The message the line carries.
    #[serde(default)]
    message: Option<TurnMessage>,
}

/// How an assistant message ended.
#[derive(Deserialize)]
struct TurnMessage {
    /// `end_turn` once the assistant has said its last word.
    #[serde(default)]
    stop_reason: Option<String>,
}

/// Tail positions and last turn outcomes retained for each subagent transcript.
#[derive(Debug, Default)]
pub(super) struct SubagentReads {
    reads: HashMap<PathBuf, SubagentRead>,
}

#[derive(Debug, Default)]
struct SubagentRead {
    cursor: JsonlCursor,
    turn:   SubagentTurn,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum SubagentTurn {
    #[default]
    Running,
    Finished,
}

impl SubagentReads {
    pub(super) fn has_finished(&mut self, path: &Path) -> SubagentTurn {
        let read = self.reads.entry(path.to_path_buf()).or_default();
        if matches!(&read.cursor, JsonlCursor::Following { offset, .. }
            if fs::metadata(path).is_ok_and(|metadata| metadata.len() < *offset))
        {
            *read = SubagentRead::default();
        }
        let batch = read.cursor.read_until(path, TRANSCRIPT_TAIL_START, |line| {
            line.split(|byte| *byte == b'\n').any(|record| {
                serde_json::from_slice::<Turn>(record)
                    .is_ok_and(|turn| turn.kind == ASSISTANT_LINE || turn.kind == USER_LINE)
            })
        });
        for line in batch.lines {
            let Ok(turn) = serde_json::from_slice::<Turn>(&line) else {
                continue;
            };
            if turn.kind == ASSISTANT_LINE {
                read.turn = if turn
                    .message
                    .and_then(|message| message.stop_reason)
                    .is_some_and(|reason| reason == END_TURN)
                {
                    SubagentTurn::Finished
                } else {
                    SubagentTurn::Running
                };
            } else if turn.kind == USER_LINE {
                read.turn = SubagentTurn::Running;
            }
        }
        read.turn
    }
}

#[cfg(test)]
#[expect(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::time::Duration;

    use serde_json::Value;
    use serde_json::json;
    use tempfile::TempDir;

    use super::*;

    /// An RFC 3339 time `ms` milliseconds after the unix epoch.
    fn stamp(ms: u64) -> String {
        DateTime::from_timestamp_millis(i64::try_from(ms).expect("the fixture time should fit"))
            .expect("the fixture time should be representable")
            .to_rfc3339()
    }

    /// An assistant line at `ms` calling the shell tool with `command`.
    fn bash_line(ms: u64, command: &str, description: &str) -> String {
        json!({
            "type": "assistant",
            "timestamp": stamp(ms),
            "message": {"content": [
                {"type": "text", "text": "running it"},
                {"type": "tool_use", "name": "Bash",
                 "input": {"command": command, "description": description}},
            ]},
        })
        .to_string()
    }

    /// A user line at `ms` carrying a tool result that mentions `Bash`.
    fn result_line(ms: u64) -> String {
        json!({
            "type": "user",
            "timestamp": stamp(ms),
            "message": {"content": [{"type": "tool_result", "content": "\"name\":\"Bash\""}]},
        })
        .to_string()
    }

    /// Write `lines` to `path`, one per line.
    fn write_lines(path: &Path, lines: &[String]) {
        fs::write(path, lines.join("\n") + "\n").expect("the fixture transcript should write");
    }

    /// The directory a session's transcript sits in is its starting
    /// directory with everything but letters and digits made `-`.
    #[test]
    fn the_transcript_path_follows_the_starting_directory() {
        let path = transcript_path(
            Path::new("/home/me/.claude/projects"),
            Path::new("/home/me/rust/hana_catalyst/docs.v2"),
            "439f",
        );

        assert_eq!(
            path,
            Path::new("/home/me/.claude/projects/-home-me-rust-hana-catalyst-docs-v2/439f.jsonl")
        );
    }

    /// Only the shell calls written inside the span are read, across a
    /// transcript long enough to be bisected; calls to other tools, tool
    /// results and bookkeeping lines are passed over.
    #[test]
    fn reads_the_shell_calls_inside_a_span() {
        let directory = TempDir::new().expect("a temporary directory should open");
        let path = directory.path().join("session.jsonl");
        let padding = "x".repeat(2_000);
        let mut lines = vec![r#"{"type":"mode","mode":"normal"}"#.to_string()];
        for second in 0..400_u64 {
            let ms = 1_790_000_000_000 + second * 1_000;
            lines.push(bash_line(ms, &format!("echo {second} {padding}"), "count"));
            lines.push(
                json!({"type": "assistant", "timestamp": stamp(ms + 200),
                       "message": {"content": [{"type": "tool_use", "name": "Read",
                                                "input": {"file_path": "/tmp/x"}}]}})
                .to_string(),
            );
            lines.push(result_line(ms + 500));
        }
        write_lines(&path, &lines);

        let calls = bash_calls(&path, &(1_790_000_299_000..=1_790_000_301_000));

        let commands: Vec<_> = calls
            .iter()
            .map(|call| {
                call.command
                    .split(' ')
                    .take(2)
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect();
        assert_eq!(commands, ["echo 299", "echo 300", "echo 301"]);
        assert_eq!(calls[0].at_ms, 1_790_000_299_000);
        assert_eq!(calls[0].description.as_deref(), Some("count"));
    }

    /// A transcript that is not there, and a span before or after every
    /// line, give no calls.
    #[test]
    fn no_calls_outside_the_transcript_or_its_times() {
        let directory = TempDir::new().expect("a temporary directory should open");
        let path = directory.path().join("session.jsonl");
        assert_eq!(bash_calls(&path, &(0..=u64::MAX)), [] as [BashCall; 0]);

        write_lines(&path, &[bash_line(5_000, "ls", "list")]);
        assert_eq!(bash_calls(&path, &(0..=4_999)), [] as [BashCall; 0]);
        assert_eq!(bash_calls(&path, &(5_001..=9_000)), [] as [BashCall; 0]);
        assert_eq!(bash_calls(&path, &(5_000..=5_000)).len(), 1);
    }

    /// A transcript is found where the session's directory says, else in
    /// the project it started in, and not at all before it is written.
    #[test]
    fn a_transcript_is_found_where_its_session_started() {
        let directory = TempDir::new().expect("a temporary directory should open");
        let projects = directory.path();
        let started = projects.join("-home-me-rust-hana-catalyst-docs-hana");
        fs::create_dir_all(&started).expect("the project directory should be made");
        write_lines(&started.join("439f.jsonl"), &[result_line(1_000)]);
        fs::create_dir_all(projects.join("-home-me-rust-other"))
            .expect("the project directory should be made");
        let moved = Path::new("/home/me/rust/hana_catalyst");

        assert_eq!(
            find_transcript(projects, Some(moved), "439f"),
            Some(started.join("439f.jsonl"))
        );
        assert_eq!(
            find_transcript(
                projects,
                Some(Path::new("/home/me/rust/hana_catalyst/docs/hana")),
                "439f"
            ),
            Some(started.join("439f.jsonl"))
        );
        assert_eq!(
            find_transcript(projects, None, "439f"),
            Some(started.join("439f.jsonl"))
        );
        assert_eq!(find_transcript(projects, Some(moved), "c0de"), None);
    }

    /// A session began at its transcript's first stamped line, past the
    /// unstamped lines before it; a transcript that is not there, or
    /// holds no stamped line, gives no time.
    #[test]
    fn a_session_began_at_its_first_stamped_line() {
        let directory = TempDir::new().expect("a temporary directory should open");
        let path = directory.path().join("session.jsonl");
        assert_eq!(began(&path), None);

        write_lines(&path, &[r#"{"type":"mode","mode":"normal"}"#.to_string()]);
        assert_eq!(began(&path), None);

        write_lines(
            &path,
            &[
                r#"{"type":"mode","mode":"normal"}"#.to_string(),
                result_line(1_790_000_000_400),
                bash_line(1_790_000_009_000, "ls", "list"),
            ],
        );
        assert_eq!(began(&path), Some(1_790_000_000));
    }

    /// A subagent is running until its transcript ends on a finished
    /// turn, and only while it has written lately; one it started names
    /// it as parent, and a subagent with no description goes by its
    /// kind.
    #[test]
    fn running_subagents_are_the_unfinished_recent_ones() {
        let directory = TempDir::new().expect("a temporary directory should open");
        let session = directory.path().join("e296.jsonl");
        let subagents = directory.path().join("e296").join(SUBAGENTS_DIRNAME);
        fs::create_dir_all(&subagents).expect("the fixture directory should create");
        let write_agent = |id: &str, meta: Value, last: Value| {
            fs::write(
                subagents.join(format!("agent-{id}.meta.json")),
                meta.to_string(),
            )
            .expect("the fixture meta should write");
            write_lines(
                &subagents.join(format!("agent-{id}.jsonl")),
                &[bash_line(1_000, "ls", "list"), last.to_string()],
            );
        };
        let working = json!({"type": "assistant", "message": {"stop_reason": "tool_use"}});
        let finished = json!({"type": "assistant", "message": {"stop_reason": "end_turn"}});
        write_agent(
            "a1",
            json!({"agentType": "general-purpose", "description": "Review phase 1"}),
            working,
        );
        write_agent(
            "a2",
            json!({"agentType": "Explore", "parentAgentId": "a1"}),
            json!({"type": "user", "message": {"content": "a result"}}),
        );
        write_agent(
            "a3",
            json!({"agentType": "general-purpose", "description": "Done"}),
            finished,
        );

        let running = running_subagents(&session, SystemTime::now(), &mut SubagentReads::default());

        let described: Vec<_> = running
            .iter()
            .map(|agent| {
                (
                    agent.id.as_str(),
                    agent.description.as_str(),
                    agent.parent.as_deref(),
                )
            })
            .collect();
        assert_eq!(
            described.len(),
            2,
            "the finished subagent is left out: {described:?}"
        );
        assert!(described.contains(&("a1", "Review phase 1", None)));
        assert!(described.contains(&("a2", "Explore", Some("a1"))));

        let later = SystemTime::now() + SUBAGENT_QUIET_LIMIT + Duration::from_secs(60);
        assert_eq!(
            running_subagents(&session, later, &mut SubagentReads::default()),
            [] as [Subagent; 0]
        );
    }

    /// The end of a transcript is read far enough back to find a whole
    /// line when its last line is longer than the first read, and a
    /// bookkeeping line after the last turn does not hide it.
    #[test]
    fn a_long_last_line_is_read_whole() {
        let directory = TempDir::new().expect("a temporary directory should open");
        let path = directory.path().join("agent.jsonl");
        let long = "y".repeat(usize::try_from(TRANSCRIPT_TAIL_START).expect("fits") * 2);
        write_lines(
            &path,
            &[
                bash_line(1_000, "ls", "list"),
                json!({"type": "assistant", "text": long,
                       "message": {"stop_reason": "end_turn"}})
                .to_string(),
                json!({"type": "last-prompt"}).to_string(),
            ],
        );

        assert_eq!(
            SubagentReads::default().has_finished(&path),
            SubagentTurn::Finished
        );
    }
}
