//! Semantic activity resolved while scanning, shared by agents and workers.

use std::borrow::Borrow;
use std::collections::HashMap;
use std::collections::HashSet;
use std::ffi::OsString;
use std::fs;
use std::io::ErrorKind;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::time::Instant;

use notify::Config;
use notify::Event;
use notify::RecommendedWatcher;
use notify::Watcher;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;

use super::classify::ProcessEntry;
use super::hierarchy::FileRevision;
use super::transcript;
use super::transcript::JsonlCursor;
use super::transcript::RecordKind;
use super::transcript::WatchedDirectoryIdentity;
use crate::constants::RUN_WATCH_RECOVERY;
use crate::constants::RUN_WATCH_RETRY;
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

/// Verification progress for the process rows in one census answer.
pub(super) struct VerificationProcesses<'a> {
    processes:      &'a [ProcessEntry],
    by_pid:         HashMap<u32, usize>,
    children:       HashMap<u32, Vec<usize>>,
    progress:       HashMap<u32, VerificationProgress>,
    #[cfg(test)]
    process_visits: usize,
}

impl<'a> VerificationProcesses<'a> {
    pub(super) fn new(processes: &'a [ProcessEntry]) -> Self {
        let mut by_pid = HashMap::new();
        let mut children: HashMap<_, Vec<_>> = HashMap::new();
        for (index, process) in processes.iter().enumerate() {
            by_pid.insert(process.pid, index);
            if let Some(parent) = process.parent {
                children.entry(parent).or_default().push(index);
            }
        }
        Self {
            processes,
            by_pid,
            children,
            progress: HashMap::new(),
            #[cfg(test)]
            process_visits: 0,
        }
    }

    pub(super) fn progress(&mut self, root: u32) -> VerificationProgress {
        if let Some(progress) = self.progress.get(&root) {
            return *progress;
        }
        // Resolve descendants without assuming detachment ancestry.
        let mut pending = vec![root];
        let mut seen = HashSet::new();
        let mut indices = Vec::new();
        while let Some(pid) = pending.pop() {
            if !seen.insert(pid) {
                continue;
            }
            if let Some(index) = self.by_pid.get(&pid) {
                indices.push(*index);
            }
            #[cfg(test)]
            {
                self.process_visits += 1;
            }
            if let Some(children) = self.children.get(&pid) {
                pending.extend(children.iter().map(|index| self.processes[*index].pid));
            }
        }
        // Keep the original table order used to select the last matching step.
        indices.sort_unstable();
        let tree: Vec<_> = indices
            .into_iter()
            .map(|index| &self.processes[index])
            .collect();
        let progress = verification_progress(&tree);
        self.progress.insert(root, progress);
        progress
    }

    /// Latest eligible live verification wins for this worker.
    pub(super) fn run(&mut self, records: &[RunRecord], thread_id: &str) -> AgentActivity {
        let latest = records
            .iter()
            .filter_map(|record| match record {
                RunRecord::Verify(run)
                    if run.thread_id == thread_id && self.by_pid.contains_key(&run.pid) =>
                {
                    Some(run)
                },
                _ => None,
            })
            .max_by(|left, right| {
                left.started_at
                    .total_cmp(&right.started_at)
                    .then(left.pid.cmp(&right.pid))
            });
        latest.map_or(AgentActivity::Unreported, |run| {
            self.progress(run.pid).with_requested(run.requested)
        })
    }
}

/// Walk once, preserving whether an unrecognized step is remote.
#[cfg(test)]
pub(super) fn verify_activity(processes: &[ProcessEntry], root_pid: u32) -> VerificationProgress {
    VerificationProcesses::new(processes).progress(root_pid)
}

fn verification_progress(tree: &[&ProcessEntry]) -> VerificationProgress {
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
#[cfg(test)]
pub(super) fn run_activity(
    records: &[RunRecord],
    processes: &[ProcessEntry],
    thread_id: &str,
) -> AgentActivity {
    VerificationProcesses::new(processes).run(records, thread_id)
}

/// Parsed detached-run records retained by one local census scanner.
#[derive(Debug)]
pub(super) struct RunRecords {
    sessions:        HashMap<PathBuf, HashMap<OsString, RunFile>>,
    watches:         HashMap<PathBuf, RunDirectoryWatch>,
    install_watch:   fn(&Path, Arc<Mutex<RunChanges>>, Arc<AtomicBool>) -> RunWatchBackend,
    #[cfg(test)]
    directory_reads: usize,
}

#[derive(Debug)]
enum RunWatchBackend {
    Watching {
        _watcher: RecommendedWatcher,
    },
    Unavailable,
    #[cfg(test)]
    Controlled,
}

/// File events name only the records to refresh; recovery requests a full listing.
#[derive(Debug, Default)]
enum RunChanges {
    #[default]
    Unchanged,
    Records(HashSet<OsString>),
    Reconcile,
}

impl RunChanges {
    fn record(&mut self, name: OsString) {
        match self {
            Self::Unchanged => *self = Self::Records(HashSet::from([name])),
            Self::Records(names) => {
                names.insert(name);
            },
            Self::Reconcile => {},
        }
    }
}

/// One session's pending changes and bounded recovery deadline.
#[derive(Debug)]
struct RunDirectoryWatch {
    backend:       RunWatchBackend,
    changed:       Arc<Mutex<RunChanges>>,
    watch_lost:    Arc<AtomicBool>,
    identity:      WatchedDirectoryIdentity,
    recovery:      Instant,
    install:       fn(&Path, Arc<Mutex<RunChanges>>, Arc<AtomicBool>) -> RunWatchBackend,
    #[cfg(test)]
    installations: usize,
}

impl RunDirectoryWatch {
    fn config() -> Config {
        // Reading unchanged records must not create work in this or another handler.
        notify::Config::default().with_event_kinds(notify::EventKindMask::CORE)
    }

    fn new(
        session: &Path,
        now: Instant,
        install: fn(&Path, Arc<Mutex<RunChanges>>, Arc<AtomicBool>) -> RunWatchBackend,
    ) -> Self {
        let mut watch = Self {
            backend: RunWatchBackend::Unavailable,
            changed: Arc::new(Mutex::new(RunChanges::Reconcile)),
            watch_lost: Arc::new(AtomicBool::new(false)),
            identity: WatchedDirectoryIdentity::Unavailable,
            recovery: now,
            install,
            #[cfg(test)]
            installations: 0,
        };
        watch.rearm(session, now);
        watch
    }

    fn install(
        session: &Path,
        changed: Arc<Mutex<RunChanges>>,
        watch_lost: Arc<AtomicBool>,
    ) -> RunWatchBackend {
        let directory = session.join("mesh_runs");
        let watched_directory = directory.clone();
        let backend = RecommendedWatcher::new(
            move |answer: notify::Result<Event>| {
                let mut pending = changed
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if let Ok(event) = answer {
                    if matches!(event.kind, notify::EventKind::Access(_)) {
                        return;
                    }
                    if event.need_rescan()
                        || event.paths.iter().any(|path| path == &watched_directory)
                    {
                        *pending = RunChanges::Reconcile;
                    } else {
                        for path in event.paths {
                            if path.parent() == Some(watched_directory.as_path())
                                && path
                                    .extension()
                                    .is_some_and(|extension| extension == "json")
                                && let Some(name) = path.file_name()
                            {
                                pending.record(name.to_owned());
                            }
                        }
                    }
                } else {
                    watch_lost.store(true, Ordering::Release);
                    *pending = RunChanges::Reconcile;
                }
            },
            Self::config(),
        );
        backend
            .and_then(|mut watcher| {
                watcher
                    .update_paths(vec![
                        notify::PathOp::watch_non_recursive(session),
                        notify::PathOp::watch_non_recursive(&directory),
                    ])
                    .map_err(|error| error.source)?;
                Ok(watcher)
            })
            .map_or_else(
                |_| RunWatchBackend::Unavailable,
                |watcher| RunWatchBackend::Watching { _watcher: watcher },
            )
    }

    fn rearm(&mut self, session: &Path, now: Instant) {
        let identity = WatchedDirectoryIdentity::read(&session.join("mesh_runs"));
        if !self.watch_lost.swap(false, Ordering::AcqRel)
            && identity != WatchedDirectoryIdentity::Unavailable
            && identity == self.identity
            && !matches!(self.backend, RunWatchBackend::Unavailable)
        {
            self.recovery = now + RUN_WATCH_RECOVERY;
            return;
        }
        self.identity = identity;
        #[cfg(test)]
        {
            self.installations += 1;
        }
        self.backend = (self.install)(
            session,
            Arc::clone(&self.changed),
            Arc::clone(&self.watch_lost),
        );
        self.recovery = now
            + match self.backend {
                RunWatchBackend::Watching { .. } => RUN_WATCH_RECOVERY,
                RunWatchBackend::Unavailable => RUN_WATCH_RETRY,
                #[cfg(test)]
                RunWatchBackend::Controlled => RUN_WATCH_RECOVERY,
            };
    }

    fn take_changes(&mut self, session: &Path, now: Instant) -> RunChanges {
        if now >= self.recovery || self.watch_lost.load(Ordering::Acquire) {
            self.rearm(session, now);
            self.invalidate();
        }
        std::mem::take(
            &mut *self
                .changed
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        )
    }

    fn invalidate(&self) {
        *self
            .changed
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = RunChanges::Reconcile;
    }

    fn record_changed(&self, name: OsString) {
        self.changed
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .record(name);
    }
}

impl Default for RunRecords {
    fn default() -> Self {
        Self {
            sessions:                     HashMap::new(),
            watches:                      HashMap::new(),
            install_watch:                RunDirectoryWatch::install,
            #[cfg(test)]
            directory_reads:              0,
        }
    }
}

/// A file revision and its decoded outcome, including a malformed partial write.
#[derive(Debug)]
struct RunFile {
    revision: FileRevision,
    content:  RunFileContent,
}

#[derive(Debug)]
enum RunFileContent {
    Parsed(RunRecord),
    MalformedJson,
    ReadFailed,
}

impl RunRecords {
    /// Deliver a fixture's file event before the next scan.
    #[cfg(test)]
    pub(super) fn record_changed(&self, session: &Path, name: OsString) {
        self.watches[session].record_changed(name);
    }

    /// Count cache work with explicit event delivery, without OS service startup time.
    #[cfg(test)]
    fn for_test() -> Self {
        Self {
            install_watch: |session, _, _| {
                if session.join("mesh_runs").is_dir() {
                    RunWatchBackend::Controlled
                } else {
                    RunWatchBackend::Unavailable
                }
            },
            ..Self::default()
        }
    }
    pub(super) fn read_for_processes(
        &mut self,
        session: &Path,
        live: &HashSet<u32>,
    ) -> Vec<RunRecord> {
        self.read_matching(
            session,
            |bytes| serde_json::from_slice(bytes).ok(),
            |path| fs::read(path),
            |record| matches!(record, RunRecord::Verify(run) if live.contains(&run.pid)),
        )
    }

    #[cfg(test)]
    pub(super) fn read(&mut self, session: &Path) -> Vec<RunRecord> {
        self.read_with(session, |bytes| serde_json::from_slice(bytes).ok())
    }

    /// Retire sessions no current worker refers to.
    pub(super) fn retain_sessions(&mut self, seen: &HashSet<PathBuf>) {
        self.sessions.retain(|path, _| seen.contains(path));
        self.watches.retain(|path, _| seen.contains(path));
    }

    #[cfg(test)]
    fn read_with(
        &mut self,
        session: &Path,
        mut parse: impl FnMut(&[u8]) -> Option<RunRecord>,
    ) -> Vec<RunRecord> {
        self.read_with_io(session, &mut parse, |path| fs::read(path))
    }

    #[cfg(test)]
    fn read_with_io(
        &mut self,
        session: &Path,
        parse: impl FnMut(&[u8]) -> Option<RunRecord>,
        read: impl FnMut(&Path) -> std::io::Result<Vec<u8>>,
    ) -> Vec<RunRecord> {
        // These fixtures supply synchronous I/O revisions; OS event delivery is
        // exercised through read_for_processes and the watch recovery fixtures.
        if let Some(watch) = self.watches.get(session) {
            watch.invalidate();
        }
        self.read_matching(session, parse, read, |_| true)
    }

    fn read_matching(
        &mut self,
        session: &Path,
        mut parse: impl FnMut(&[u8]) -> Option<RunRecord>,
        mut read: impl FnMut(&Path) -> std::io::Result<Vec<u8>>,
        accept: impl Fn(&RunRecord) -> bool,
    ) -> Vec<RunRecord> {
        let now = Instant::now();
        let watch = self
            .watches
            .entry(session.to_owned())
            .or_insert_with(|| RunDirectoryWatch::new(session, now, self.install_watch));
        let events = watch.take_changes(session, now);
        let records = self.sessions.entry(session.to_owned()).or_default();
        let directory = session.join("mesh_runs");
        let changed = match events {
            RunChanges::Unchanged => HashSet::new(),
            RunChanges::Records(names) => names,
            RunChanges::Reconcile => {
                #[cfg(test)]
                {
                    self.directory_reads += 1;
                }
                let seen: HashSet<_> = fs::read_dir(&directory)
                    .into_iter()
                    .flatten()
                    .flatten()
                    .map(|entry| entry.file_name())
                    .filter(|name| Path::new(name).extension().is_some_and(|ext| ext == "json"))
                    .collect();
                records.retain(|name, _| seen.contains(name));
                seen
            },
        };
        for name in changed {
            let path = directory.join(&name);
            let metadata = match fs::metadata(&path) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == ErrorKind::NotFound => {
                    records.remove(&name);
                    continue;
                },
                Err(_) => {
                    watch.record_changed(name);
                    continue;
                },
            };
            let revision = super::hierarchy::FileRevision::of(&metadata);
            if records.get(&name).is_some_and(|record| {
                record.revision == revision && !matches!(record.content, RunFileContent::ReadFailed)
            }) {
                continue;
            }
            let content = read(&path).map_or(RunFileContent::ReadFailed, |bytes| {
                parse(&bytes).map_or(RunFileContent::MalformedJson, RunFileContent::Parsed)
            });
            if matches!(content, RunFileContent::ReadFailed) {
                watch.record_changed(name.clone());
            }
            records.insert(name, RunFile { revision, content });
        }
        records
            .values()
            .filter_map(|record| match &record.content {
                RunFileContent::Parsed(record) if accept(record) => Some(record.clone()),
                RunFileContent::Parsed(_)
                | RunFileContent::MalformedJson
                | RunFileContent::ReadFailed => None,
            })
            .collect()
    }
}

#[cfg(test)]
#[path = "activity_regression_tests.rs"]
mod regression_tests;
