//! Semantic activity resolved while scanning, shared by agents and workers.

use std::borrow::Borrow;
use std::collections::HashSet;
use std::fs;
use std::path::Path;

use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;

use super::classify::ProcessEntry;
use super::transcript;
use super::transcript::JsonlCursor;
use super::transcript::RecordKind;
use crate::constants::TRANSCRIPT_TAIL_START;
use crate::constants::VERIFY_SCRIPT;
use crate::theme::Role;

/// One activity, with no contradictory separate turn state.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AgentActivity {
    /// No activity source reports this row.
    #[default]
    Unreported,
    /// The last turn completed or aborted.
    Idle,
    /// Work in progress, including detached verification.
    Active(Activity),
}

/// A worker's current work.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Activity {
    Thinking,
    Writing,
    Reading,
    Git,
    Delegating,
    Web,
    Running,
    Asking,
    Waiting,
    /// The gate and the machine executing it.
    Verify {
        step:     VerifyStep,
        location: RunLocation,
    },
}

/// Supported cargo and formatting gates.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum VerifyStep {
    Fmt,
    Clippy,
    Nextest,
    Build,
    Check,
    Doc,
    Mend,
    Taplo,
}

/// Where the selected verification executes.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RunLocation {
    Local,
    Mac,
}

impl AgentActivity {
    /// Omit the compatibility default on child probe rows.
    pub(crate) fn is_idle(activity: impl Borrow<Self>) -> bool {
        matches!(activity.borrow(), Self::Idle)
    }

    /// One word, also used as the Status sort key.
    pub(crate) const fn text(self) -> &'static str {
        match self {
            Self::Unreported => "—",
            Self::Idle => "idle",
            Self::Active(activity) => match activity {
                Activity::Thinking => "thinking",
                Activity::Writing => "writing",
                Activity::Reading => "reading",
                Activity::Git => "git",
                Activity::Delegating => "delegating",
                Activity::Web => "web",
                Activity::Running => "running",
                Activity::Asking => "asking",
                Activity::Waiting => "waiting",
                Activity::Verify { step, location } => match (step, location) {
                    (VerifyStep::Fmt, RunLocation::Local) => "fmt",
                    (VerifyStep::Clippy, RunLocation::Local) => "clippy",
                    (VerifyStep::Nextest, RunLocation::Local) => "nextest",
                    (VerifyStep::Build, RunLocation::Local) => "build",
                    (VerifyStep::Check, RunLocation::Local) => "check",
                    (VerifyStep::Doc, RunLocation::Local) => "doc",
                    (VerifyStep::Mend, RunLocation::Local) => "mend",
                    (VerifyStep::Taplo, RunLocation::Local) => "taplo",
                    (VerifyStep::Fmt, RunLocation::Mac) => "fmt (mac)",
                    (VerifyStep::Clippy, RunLocation::Mac) => "clippy (mac)",
                    (VerifyStep::Nextest, RunLocation::Mac) => "nextest (mac)",
                    (VerifyStep::Build, RunLocation::Mac) => "build (mac)",
                    (VerifyStep::Check, RunLocation::Mac) => "check (mac)",
                    (VerifyStep::Doc, RunLocation::Mac) => "doc (mac)",
                    (VerifyStep::Mend, RunLocation::Mac) => "mend (mac)",
                    (VerifyStep::Taplo, RunLocation::Mac) => "taplo (mac)",
                },
            },
        }
    }

    /// Gates share an accent; waits and closed turns are dim.
    pub(crate) const fn color_role(self) -> Role {
        match self {
            Self::Active(Activity::Verify { .. }) => Role::Codex,
            Self::Active(Activity::Waiting) | Self::Idle | Self::Unreported => Role::Idle,
            Self::Active(_) => Role::Busy,
        }
    }
}

impl From<Option<&str>> for AgentActivity {
    /// Convert an old optional status at a file or probe boundary.
    fn from(status: Option<&str>) -> Self {
        match status {
            Some("idle") => Self::Idle,
            Some("busy" | "thinking") => Self::Active(Activity::Thinking),
            Some("shell" | "running") => Self::Active(Activity::Running),
            Some("writing") => Self::Active(Activity::Writing),
            Some("reading") => Self::Active(Activity::Reading),
            Some("git") => Self::Active(Activity::Git),
            Some("delegating") => Self::Active(Activity::Delegating),
            Some("web") => Self::Active(Activity::Web),
            Some("asking") => Self::Active(Activity::Asking),
            Some("waiting") => Self::Active(Activity::Waiting),
            _ => Self::Unreported,
        }
    }
}

impl<'de> Deserialize<'de> for AgentActivity {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        match value {
            Value::String(ref text) => Ok(Self::from(Some(text.as_str()))),
            Value::Null => Ok(Self::Unreported),
            Value::Object(mut fields) => {
                let active = fields
                    .remove("active")
                    .ok_or_else(|| serde::de::Error::custom("missing active activity"))?;
                serde_json::from_value(active)
                    .map(Self::Active)
                    .map_err(serde::de::Error::custom)
            },
            _ => Err(serde::de::Error::custom("invalid activity")),
        }
    }
}

/// Incremental turn and tool observations from one conversation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct ActivityRead {
    current: AgentActivity,
}
impl Default for ActivityRead {
    fn default() -> Self {
        Self {
            current: AgentActivity::Idle,
        }
    }
}
impl ActivityRead {
    /// Seed a transcript read with the session boundary's reported turn state.
    pub(super) const fn from_activity(current: AgentActivity) -> Self { Self { current } }

    /// The last event's semantic activity.
    pub(super) const fn activity(self) -> AgentActivity { self.current }
    /// Observe a whole Codex rollout line.
    pub(super) fn take_codex(&mut self, line: &Value) {
        let payload = &line["payload"];
        match (line["type"].as_str(), payload["type"].as_str()) {
            (Some("event_msg"), Some("task_started"))
            | (Some("response_item"), Some("reasoning")) => {
                self.current = AgentActivity::Active(Activity::Thinking);
            },
            (Some("event_msg"), Some("task_complete" | "turn_aborted")) => {
                self.current = AgentActivity::Idle;
            },
            (Some("response_item"), Some("function_call" | "custom_tool_call")) => {
                let raw = payload
                    .get("arguments")
                    .or_else(|| payload.get("input"))
                    .unwrap_or(&Value::Null);
                let arguments = raw
                    .as_str()
                    .and_then(|text| serde_json::from_str(text).ok())
                    .unwrap_or_else(|| raw.clone());
                self.current =
                    tool_activity(payload["name"].as_str().unwrap_or_default(), &arguments);
            },
            _ => {},
        }
    }
    /// Observe a Claude transcript message.
    pub(super) fn take_claude(&mut self, line: &Value) {
        match line["type"].as_str() {
            Some("user")
                if line["message"]["content"].as_str().is_some()
                    || line["message"]["content"].as_array().is_some_and(|blocks| {
                        blocks.iter().any(|block| block["type"] == "text")
                    }) =>
            {
                self.current = AgentActivity::Active(Activity::Thinking);
            },
            Some("assistant") => {
                if let Some(content) = line["message"]["content"].as_array() {
                    for block in content {
                        match block["type"].as_str() {
                            Some("tool_use") => {
                                self.current = tool_activity(
                                    block["name"].as_str().unwrap_or_default(),
                                    &block["input"],
                                );
                            },
                            Some("thinking") => {
                                self.current = AgentActivity::Active(Activity::Thinking);
                            },
                            _ => {},
                        }
                    }
                }
                if line["message"]["stop_reason"] == "end_turn" {
                    self.current = AgentActivity::Idle;
                }
            },
            _ => {},
        }
    }
}

/// Classify a tool at the external tool-name boundary.
pub(super) fn tool_activity(name: &str, arguments: &Value) -> AgentActivity {
    let activity = match name.rsplit('.').next().unwrap_or(name) {
        "apply_patch" | "Edit" | "Write" | "NotebookEdit" => Activity::Writing,
        "Read" | "Grep" | "Glob" => Activity::Reading,
        "Agent" | "Task" => Activity::Delegating,
        "WebFetch" | "WebSearch" => Activity::Web,
        _ => {
            let command = arguments["cmd"]
                .as_str()
                .or_else(|| arguments["command"].as_str())
                .or_else(|| arguments.as_str())
                .unwrap_or_default();
            if name.rsplit('.').next() == Some("exec") && command.contains("tools.apply_patch(") {
                Activity::Writing
            } else {
                let shell = if name.rsplit('.').next() == Some("exec") {
                    ["cmd:", "\"cmd\":", "command:", "\"command\":"]
                        .iter()
                        .find_map(|key| {
                            let at = command.rfind(key)? + key.len();
                            serde_json::Deserializer::from_str(command[at..].trim_start())
                                .into_iter::<String>()
                                .next()?
                                .ok()
                        })
                        .unwrap_or_else(|| command.to_owned())
                } else {
                    command.to_owned()
                };
                let words = shell_words(&shell);
                let program = words
                    .first()
                    .and_then(|word| Path::new(word).file_name())
                    .and_then(|word| word.to_str())
                    .unwrap_or_default();
                match program {
                    "rg" | "grep" | "cat" | "ls" | "find" => Activity::Reading,
                    "sed" if words.get(1).is_some_and(|word| word == "-n") => Activity::Reading,
                    "git"
                        if words.get(1).is_some_and(|word| {
                            ["commit", "merge", "rebase", "push"].contains(&word.as_str())
                        }) =>
                    {
                        Activity::Git
                    },
                    _ => Activity::Running,
                }
            }
        },
    };
    AgentActivity::Active(activity)
}

/// Literal words of the first shell command. Arguments may contain expansions;
/// operators end the command and redirections do not change its program.
fn shell_words(command: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut quote = None;
    let mut escaped = false;
    for character in command.chars() {
        if escaped {
            word.push(character);
            escaped = false;
            continue;
        }
        match (quote, character) {
            (Some('\''), '\'') | (Some('"'), '"') => quote = None,
            (None, '\'' | '"') => quote = Some(character),
            (Some('\''), _) => word.push(character),
            (_, '\\') => escaped = true,
            (None, ';' | '&' | '|' | '<' | '>' | '\n') => break,
            (None, c) if c.is_whitespace() => {
                if !word.is_empty() {
                    words.push(std::mem::take(&mut word));
                }
            },
            _ => word.push(character),
        }
    }
    if !word.is_empty() {
        words.push(word);
    }
    // A leading assignment belongs to the shell rather than the invoked program.
    let assignments = words.iter().take_while(|word| word.contains('=')).count();
    words.drain(..assignments);
    words
}

/// A whole-line transcript cursor, retained across scans of the same path.
#[derive(Debug)]
pub(super) struct TranscriptRead {
    cursor:     JsonlCursor,
    bytes_read: u64,
    activity:   ActivityRead,
}

impl TranscriptRead {
    pub(super) const fn new(reported: AgentActivity) -> Self {
        Self {
            cursor:     JsonlCursor::NeverRead,
            bytes_read: 0,
            activity:   ActivityRead::from_activity(reported),
        }
    }

    pub(super) const fn activity(&self) -> AgentActivity { self.activity.activity() }

    /// File bytes fetched on the last advance, including unfinished records.
    pub(super) const fn bytes_read(&self) -> u64 { self.bytes_read }

    /// Consume only newly completed lines. The count makes cursor reuse observable.
    pub(super) fn advance(&mut self, path: &Path, reported: AgentActivity) -> usize {
        if matches!(&self.cursor, JsonlCursor::Following { offset, .. }
            if fs::metadata(path).is_ok_and(|metadata| metadata.len() < *offset))
        {
            *self = Self::new(reported);
        }
        let batch = self.cursor.read_until(path, TRANSCRIPT_TAIL_START, |line| {
            std::str::from_utf8(line)
                .unwrap_or_default()
                .lines()
                .any(|record| {
                    let record = record.as_bytes();
                    if !matches!(transcript::record_kind(record), RecordKind::Claude) {
                        return false;
                    }
                    let mut activity = ActivityRead::from_activity(AgentActivity::Unreported);
                    if let Ok(value) = serde_json::from_slice(record) {
                        activity.take_claude(&value);
                    }
                    activity.activity() != AgentActivity::Unreported
                })
        });
        self.bytes_read = batch.bytes_read;
        for line in &batch.lines {
            if !matches!(transcript::record_kind(line), RecordKind::Claude) {
                continue;
            }
            if let Ok(value) = serde_json::from_slice(line) {
                self.activity.take_claude(&value);
            }
        }
        batch.lines.len()
    }
}

/// A record at the file boundary; incomplete identity never enters a live run.
#[derive(Deserialize)]
struct RunPayload {
    thread_id:  Option<String>,
    pid:        u32,
    command:    Vec<String>,
    status:     RunStatus,
    started_at: Option<f64>,
}

/// Only validated, running verification records may compete for a worker's activity.
#[derive(Clone, Debug, Deserialize)]
#[serde(from = "RunPayload")]
pub(super) enum RunRecord {
    Ignored,
    Verify(VerificationRun),
}

#[derive(Clone, Debug)]
pub(super) struct VerificationRun {
    thread_id:  String,
    pid:        u32,
    requested:  RequestedStep,
    started_at: f64,
}

impl From<RunPayload> for RunRecord {
    fn from(payload: RunPayload) -> Self {
        let Some(thread_id) = payload.thread_id.filter(|id| !id.is_empty()) else {
            return Self::Ignored;
        };
        let Some(started_at) = payload.started_at.filter(|at| at.is_finite()) else {
            return Self::Ignored;
        };
        let Some(arguments) = verification_arguments(&payload.command) else {
            return Self::Ignored;
        };
        if payload.status != RunStatus::Running {
            return Self::Ignored;
        }
        Self::Verify(VerificationRun {
            thread_id,
            pid: payload.pid,
            requested: arguments
                .iter()
                .find_map(|word| VerifyStep::from_word(word))
                .map_or(RequestedStep::Unknown, RequestedStep::Step),
            started_at,
        })
    }
}

/// Recognize the invoked verification script, never a script mentioned as data.
fn verification_arguments(command: &[String]) -> Option<&[String]> {
    let (program, arguments) = command.split_first()?;
    match Path::new(program).file_name()?.to_str()? {
        VERIFY_SCRIPT => Some(arguments),
        "bash" | "sh" | "zsh" => {
            let (script, rest) = arguments.split_first()?;
            (Path::new(script).file_name()? == VERIFY_SCRIPT).then_some(rest)
        },
        _ => None,
    }
}

/// A verification invocation may not specify a recognizable remote step.
#[derive(Clone, Copy, Debug)]
enum RequestedStep {
    Unknown,
    Step(VerifyStep),
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum RunStatus {
    Running,
    #[serde(other)]
    Finished,
}

impl VerifyStep {
    /// Shared word classification for cargo argv, ssh commands and verify invocations.
    fn from_word(word: &str) -> Option<Self> {
        match word {
            "fmt" => Some(Self::Fmt),
            "clippy" | "lint" => Some(Self::Clippy),
            "nextest" | "test" | "mac_test" | "mac_test.sh" => Some(Self::Nextest),
            "build" => Some(Self::Build),
            "check" => Some(Self::Check),
            "doc" => Some(Self::Doc),
            "mend" => Some(Self::Mend),
            "taplo" => Some(Self::Taplo),
            _ => None,
        }
    }
}

/// Unknown remote work remains distinct until the run's requested step is available.
#[derive(Clone, Copy, Debug)]
pub(super) enum VerificationProgress {
    Waiting,
    RemoteUnknown,
    Step {
        step:     VerifyStep,
        location: RunLocation,
    },
}

impl VerificationProgress {
    pub(super) const fn activity(self) -> AgentActivity {
        match self {
            Self::Waiting | Self::RemoteUnknown => AgentActivity::Active(Activity::Waiting),
            Self::Step { step, location } => {
                AgentActivity::Active(Activity::Verify { step, location })
            },
        }
    }

    #[cfg(test)]
    pub(super) const fn text(self) -> &'static str { self.activity().text() }

    const fn with_requested(self, requested: RequestedStep) -> AgentActivity {
        match (self, requested) {
            (Self::RemoteUnknown, RequestedStep::Step(step)) => {
                AgentActivity::Active(Activity::Verify {
                    step,
                    location: RunLocation::Mac,
                })
            },
            _ => self.activity(),
        }
    }
}

/// Descendants of a live root, resolved without assuming detachment ancestry.
fn descendants(processes: &[ProcessEntry], root: u32) -> Vec<&ProcessEntry> {
    let mut pids = HashSet::from([root]);
    loop {
        let before = pids.len();
        for process in processes {
            if process.parent.is_some_and(|pid| pids.contains(&pid)) {
                pids.insert(process.pid);
            }
        }
        if before == pids.len() {
            break;
        }
    }
    processes
        .iter()
        .filter(|process| pids.contains(&process.pid))
        .collect()
}

/// Walk once, preserving whether an unrecognized step is remote.
pub(super) fn verify_activity(processes: &[ProcessEntry], root_pid: u32) -> VerificationProgress {
    let tree = descendants(processes, root_pid);
    let remote = tree.iter().find(|process| {
        process.arguments.first().is_some_and(|word| {
            Path::new(word)
                .file_name()
                .is_some_and(|name| name == "ssh")
        })
    });
    let location = if remote.is_some() {
        RunLocation::Mac
    } else {
        RunLocation::Local
    };
    for process in tree.iter().rev() {
        let args = &process.arguments;
        let program = args
            .first()
            .and_then(|word| Path::new(word).file_name())
            .and_then(|word| word.to_str())
            .unwrap_or_default();
        let step = match program {
            "taplo" => Some(VerifyStep::Taplo),
            "cargo" => args
                .iter()
                .skip(1)
                .find_map(|word| VerifyStep::from_word(word)),
            _ => None,
        };
        if let Some(step) = step {
            return VerificationProgress::Step { step, location };
        }
    }
    if let Some(ssh) = remote {
        // ssh may store the entire remote command in one argv element.
        let step = ssh
            .arguments
            .iter()
            .skip(1)
            .flat_map(|argument| {
                argument.split(|character: char| {
                    character.is_whitespace()
                        || matches!(character, '\'' | '"' | ';' | '&' | '|' | '(' | ')')
                })
            })
            .find_map(|word| {
                let name = Path::new(word).file_name()?.to_str()?;
                VerifyStep::from_word(name)
            });
        return step.map_or(VerificationProgress::RemoteUnknown, |step| {
            VerificationProgress::Step { step, location }
        });
    }
    VerificationProgress::Waiting
}

/// Latest eligible live verification wins; unrelated detached commands contribute nothing.
pub(super) fn run_activity(
    records: &[RunRecord],
    processes: &[ProcessEntry],
    thread_id: &str,
) -> AgentActivity {
    records
        .iter()
        .filter_map(|record| match record {
            RunRecord::Verify(run)
                if run.thread_id == thread_id
                    && processes.iter().any(|process| process.pid == run.pid) =>
            {
                Some(run)
            },
            _ => None,
        })
        .max_by(|left, right| {
            left.started_at
                .total_cmp(&right.started_at)
                .then(left.pid.cmp(&right.pid))
        })
        .map_or(AgentActivity::Unreported, |run| {
            verify_activity(processes, run.pid).with_requested(run.requested)
        })
}

/// Read records in the discovered session only, converting at the JSON boundary.
pub(super) fn read_runs(session: &Path) -> Vec<RunRecord> {
    fs::read_dir(session.join("mesh_runs"))
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
        .filter_map(|entry| serde_json::from_slice(&fs::read(entry.path()).ok()?).ok())
        .collect()
}

#[cfg(test)]
#[path = "activity_regression_tests.rs"]
mod regression_tests;
