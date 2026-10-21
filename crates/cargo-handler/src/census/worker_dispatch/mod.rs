//! Literal worker launcher commands and their roster-backed thread identities.

use std::collections::BTreeMap;
use std::collections::btree_map::Entry;
use std::fs;
use std::iter::Peekable;
use std::path::Path;
use std::path::PathBuf;
use std::str::Chars;
use std::time::SystemTime;

use serde::Deserialize;
use serde::Deserializer;
use serde::Serialize;
use serde::de::IgnoredAny;
use serde::ser::SerializeStruct;

use super::HierarchyRole;
use super::RoleFallback;
use super::classify::ProcessEntry;
use super::tier::ThreadWorktree;
use crate::constants::REVIEW_JOB_HEADING;
use crate::constants::SESSION_ROOTS;
use crate::constants::UUID_TEXT_LENGTH;
use crate::constants::WORKER_DOCUMENT_EXTENSION;
use crate::constants::WORKER_LABEL;

/// The roster filename inside a dispatch's session directory.
const ROSTER_FILE: &str = "mesh_roster.json";
const WORKER_FILE_PREFIX: &str = "impl_worker_";
const CLAIM_FILE_PREFIX: &str = "impl_claim_";
const POOL_PREAMBLE: &str = "You are a pooled reviewer for ";
const POOL_PHASE_DESCRIPTION: &str = "this is phase ";
/// The launcher interpreter, including when it is given as an absolute path.
const LAUNCHER_INTERPRETER: &str = "bash";
/// The launcher path suffix shared by literal commands and process argv.
const LAUNCHER_SCRIPT: &str = ".claude/scripts/delegate/implement.sh";
/// Prompt boilerplate introduces a prescribed board role call rather than a task.
const ROLE_PREAMBLE: &str = "You are slot ";
const BOARD_SCRIPT: &str = "board.sh";
const BOARD_ROLE: &str = "role";
/// Arguments after board.sh: role, session directory, slot, responsibility, job.
const BOARD_JOB_INDEX: usize = 4;

/// The responsibilities a mesh worker seat can carry.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum WorkerRole {
    /// Write the implementation.
    Impl,
    /// Exercise the specification.
    Test,
    /// Inspect a proposed change.
    Review,
    /// Repair a finding.
    Fix,
}

impl WorkerRole {
    /// The role text shared by folded and managed worker rows.
    const fn text(self) -> &'static str {
        match self {
            Self::Impl => "impl",
            Self::Test => "test",
            Self::Review => "review",
            Self::Fix => "fix",
        }
    }
}

/// A seat's known worker responsibility, or an ordinary thread's role.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum DispatchRole {
    /// The roster or short seat name identifies a worker role.
    Worker(WorkerRole),
    /// No supported worker role was recorded or inferred.
    #[default]
    Unroled,
}

impl DispatchRole {
    /// The first role token wins after removing each token's trailing seat digits.
    fn from_short_seat(seat: &str) -> Self {
        seat.split(['-', '_'])
            .find_map(|token| {
                match token.trim_end_matches(|character: char| character.is_ascii_digit()) {
                    "impl" => Some(Self::Worker(WorkerRole::Impl)),
                    "test" => Some(Self::Worker(WorkerRole::Test)),
                    "review" => Some(Self::Worker(WorkerRole::Review)),
                    "fix" => Some(Self::Worker(WorkerRole::Fix)),
                    _ => None,
                }
            })
            .unwrap_or(Self::Unroled)
    }
}

/// The available authority for a Codex worker's role.
#[derive(Clone, Copy, Debug)]
pub(crate) enum WorkerRoleInput<'a> {
    /// A live pool lease identifies the current review.
    ReviewerLease,
    /// A management edge is authoritative over any dispatch seat.
    Hierarchy(HierarchyRole),
    /// An unmanaged worker may carry a resolved dispatch seat.
    Dispatch(DispatchLabel<'a>),
}

/// The authoritative source of one visible Codex worker's responsibility.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WorkerRoleSource {
    /// The source validated a busy reviewer lease.
    ReviewerLease,
    /// A management edge outranks every seat source.
    Hierarchy(HierarchyRole),
    /// The session roster or its seat token resolved the dispatch's role.
    Dispatch(WorkerRole),
    /// An unroled dispatch still has a recognizable seat token.
    SeatToken(WorkerRole),
    /// No hierarchy or dispatch seat names a supported role.
    Unroled,
}

impl WorkerRoleSource {
    /// Resolve hierarchy first, then the session's dispatch role and seat token.
    pub(crate) fn resolve(input: WorkerRoleInput<'_>) -> Self {
        match input {
            WorkerRoleInput::ReviewerLease => Self::ReviewerLease,
            WorkerRoleInput::Hierarchy(role) => Self::Hierarchy(role),
            WorkerRoleInput::Dispatch(DispatchLabel::Worker(dispatch)) => match dispatch.role {
                DispatchRole::Worker(role) => Self::Dispatch(role),
                DispatchRole::Unroled => {
                    match DispatchRole::from_short_seat(&dispatch.short_seat) {
                        DispatchRole::Worker(role) => Self::SeatToken(role),
                        DispatchRole::Unroled => Self::Unroled,
                    }
                },
            },
            WorkerRoleInput::Dispatch(DispatchLabel::Original) => Self::Unroled,
        }
    }

    /// Every Codex row is a worker, including one with no recorded responsibility.
    pub(crate) const fn text(self) -> &'static str {
        match self {
            Self::ReviewerLease => WorkerRole::Review.text(),
            Self::Hierarchy(role) => role.text(),
            Self::SeatToken(role) | Self::Dispatch(role) => role.text(),
            Self::Unroled => WORKER_LABEL,
        }
    }
}

/// A shell either remains ordinary work or names a roster-backed worker.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum WorkerDispatch {
    /// No worker identity could be established at scan time.
    #[default]
    Plain,
    /// The launcher's seat and thread were found in its roster.
    Worker(ResolvedWorkerDispatch),
}

impl WorkerDispatch {
    /// Ordinary shells need no additional probe field.
    pub(super) const fn is_plain(&self) -> bool { matches!(self, Self::Plain) }
}

/// The agent family is part of the worker identity.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum WorkerIdentity {
    CodexThread(String),
    ClaudeSession(String),
}

/// A worker seat with the thread identity needed to join its shell and row.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(from = "ProbedWorkerDispatch")]
pub(crate) struct ResolvedWorkerDispatch {
    /// The complete seat key in the mesh roster.
    seat:                String,
    /// The recorded thread belonging to that seat.
    pub(super) identity: WorkerIdentity,
    /// The extracted responsibility, or a preamble with no usable job.
    job:                 JobText,
    /// The seat with its worktree directory prefix removed.
    short_seat:          String,
    /// The roster responsibility, falling back to the short seat's role stem.
    role:                DispatchRole,
}

impl Serialize for ResolvedWorkerDispatch {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut record = serializer.serialize_struct("ResolvedWorkerDispatch", 6)?;
        record.serialize_field("seat", &self.seat)?;
        record.serialize_field(
            "thread_id",
            match &self.identity {
                WorkerIdentity::CodexThread(id) => id.as_str(),
                WorkerIdentity::ClaudeSession(_) => "",
            },
        )?;
        record.serialize_field("identity", &self.identity)?;
        record.serialize_field("job", &self.job)?;
        record.serialize_field("short_seat", &self.short_seat)?;
        record.serialize_field("role", &self.role)?;
        record.end()
    }
}

/// Probe compatibility boundary: old payloads did not record a seat role.
#[derive(Deserialize)]
struct ProbedWorkerDispatch {
    /// The full seat key preserved by the probe.
    seat:       String,
    /// The thread joined with the dispatch shell.
    #[serde(default)]
    thread_id:  String,
    #[serde(default)]
    identity:   Option<WorkerIdentity>,
    /// The dispatched responsibility text.
    job:        String,
    /// The role stem source when an older probe omitted its role.
    short_seat: String,
    /// Absence exists only at this external boundary; explicit unroled stays unroled.
    #[serde(default)]
    role:       ProbedRole,
}

/// Only an omitted probe field triggers inference; explicit values must parse.
#[derive(Default)]
enum ProbedRole {
    /// The sender predates role metadata.
    #[default]
    Unrecorded,
    /// The sender supplied an authoritative role, including unroled.
    Recorded(DispatchRole),
}

impl<'de> Deserialize<'de> for ProbedRole {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        DispatchRole::deserialize(deserializer).map(Self::Recorded)
    }
}

impl From<ProbedWorkerDispatch> for ResolvedWorkerDispatch {
    fn from(probe: ProbedWorkerDispatch) -> Self {
        let role = match probe.role {
            ProbedRole::Unrecorded => DispatchRole::from_short_seat(&probe.short_seat),
            ProbedRole::Recorded(role) => role,
        };
        Self {
            seat: probe.seat,
            identity: probe
                .identity
                .unwrap_or(WorkerIdentity::CodexThread(probe.thread_id)),
            job: JobText::parse(&probe.job, JobSource::DispatchJob),
            short_seat: probe.short_seat,
            role,
        }
    }
}

impl ResolvedWorkerDispatch {
    /// The worker row's display name, on one line however the job was written.
    pub(crate) fn label(&self) -> String { self.job.label(&self.short_seat) }
    pub(super) const fn fallback(&self) -> super::RoleFallback {
        match self.role {
            DispatchRole::Unroled => RoleFallback::None,
            DispatchRole::Worker(role) => RoleFallback::Worker(match role {
                WorkerRole::Impl => HierarchyRole::Impl,
                WorkerRole::Test => HierarchyRole::Test,
                WorkerRole::Review => HierarchyRole::Review,
                WorkerRole::Fix => HierarchyRole::Fix,
            }),
        }
    }
}

/// The task-backed fallback name, resolved with the final hierarchy role at draw time.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum WorkerTask {
    /// An older probe has only its original name.
    #[default]
    Unrecorded,
    /// No readable first task prompt exists.
    RoleOnly,
    /// The first task line; neither a UUID nor an arbitrary later prompt.
    Task(String),
    /// A pool name belongs to the current review, not to the first task.
    Reviewer { name: String, job: JobText },
}
impl WorkerTask {
    /// Resolve a task line once at the rollout boundary.
    pub(crate) fn from_prompt(prompt: &str) -> Self {
        match JobText::parse(prompt, JobSource::FirstTaskPrompt) {
            JobText::Unrecorded => Self::RoleOnly,
            JobText::Recorded(job) => Self::Task(job),
        }
    }
    /// A final role names every thread, including unreadable or old raw-id rows.
    pub(crate) fn label(&self, role: WorkerRoleSource, original: &str) -> String {
        match self {
            Self::Reviewer { name, job } => job.label(name),
            Self::RoleOnly => role.text().to_owned(),
            Self::Task(job) => format!("{} · {job}", role.text()),
            Self::Unrecorded => {
                let uuid = original.len() == UUID_TEXT_LENGTH
                    && original.chars().all(|c| c.is_ascii_hexdigit() || c == '-');
                if uuid {
                    role.text().to_owned()
                } else {
                    original.to_owned()
                }
            },
        }
    }
}

/// Identity and claim documents are complete snapshots written by the launcher.
#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
enum SeatDocument {
    Worker(SeatWorker),
    Claim(SeatClaim),
    Unavailable(IgnoredAny),
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum SeatFamily {
    Claude,
    Codex,
    #[serde(other)]
    Unsupported,
}

#[derive(Clone, Debug, Deserialize)]
struct SeatWorker {
    name:      String,
    slot:      String,
    family:    SeatFamily,
    worker_id: String,
}

#[derive(Clone, Debug, Deserialize)]
struct SeatClaim {
    pid:    u32,
    worker: String,
}

/// A roster cached by its file modification time, never searched globally.
#[derive(Debug, Default)]
pub(super) struct WorkerNames {
    rosters: BTreeMap<PathBuf, (SystemTime, WorkerRoster)>,
    seats:   BTreeMap<PathBuf, (SystemTime, SeatDocument)>,
}

impl WorkerNames {
    /// Rewritten identity files are fetched once per observed modification time.
    fn seat_document(&mut self, path: PathBuf) -> &SeatDocument {
        let modified = fs::metadata(&path)
            .and_then(|metadata| metadata.modified())
            .unwrap_or(std::time::UNIX_EPOCH);
        let read = |path: &Path| {
            fs::read(path)
                .ok()
                .and_then(|bytes| serde_json::from_slice(&bytes).ok())
                .unwrap_or(SeatDocument::Unavailable(IgnoredAny))
        };
        let cached = match self.seats.entry(path) {
            Entry::Vacant(entry) => {
                let document = read(entry.key());
                entry.insert((modified, document))
            },
            Entry::Occupied(mut entry) => {
                if entry.get().0 != modified {
                    let document = read(entry.key());
                    entry.insert((modified, document));
                }
                entry.into_mut()
            },
        };
        &cached.1
    }

    /// Join a durable Claude seat only while its named claim process is live.
    pub(super) fn resolve_claude(
        &mut self,
        dispatch: &ParsedDispatch,
        processes: &[ProcessEntry],
        dispatch_pid: u32,
    ) -> WorkerDispatch {
        let mut found = Vec::new();
        for entry in fs::read_dir(&dispatch.session_dir)
            .into_iter()
            .flatten()
            .flatten()
        {
            let filename = entry.file_name();
            let Some(filename) = filename.to_str() else {
                continue;
            };
            if !filename.starts_with(WORKER_FILE_PREFIX)
                || !entry.path().extension().is_some_and(|extension| {
                    extension.eq_ignore_ascii_case(WORKER_DOCUMENT_EXTENSION)
                })
            {
                continue;
            }
            let SeatDocument::Worker(seat) = self.seat_document(entry.path()).clone() else {
                continue;
            };
            if !matches!(seat.family, SeatFamily::Claude) || seat.worker_id.is_empty() {
                continue;
            }
            let SeatDocument::Claim(claim) =
                self.seat_document(dispatch.session_dir.join(format!(
                    "{CLAIM_FILE_PREFIX}{}.{WORKER_DOCUMENT_EXTENSION}",
                    seat.slot
                )))
            else {
                continue;
            };
            if claim.worker != seat.name
                || !processes.iter().any(|process| process.pid == claim.pid)
            {
                continue;
            }
            let selected = match &dispatch.seat {
                SeatSelection::Named(name) => name == &seat.name,
                SeatSelection::Launcher => claim.pid == dispatch_pid,
            };
            if selected {
                found.push(ResolvedWorkerDispatch {
                    seat:       seat.name,
                    identity:   WorkerIdentity::ClaudeSession(seat.worker_id),
                    job:        JobText::parse(&dispatch.job, JobSource::DispatchJob),
                    short_seat: seat.slot.clone(),
                    role:       match dispatch.pass_role {
                        DispatchRole::Worker(role) => DispatchRole::Worker(role),
                        DispatchRole::Unroled => DispatchRole::from_short_seat(&seat.slot),
                    },
                });
            }
        }
        if found.len() == 1 {
            WorkerDispatch::Worker(found.remove(0))
        } else {
            WorkerDispatch::Plain
        }
    }

    /// Session paths mentioned by the first task prompt, in either supported root.
    pub(super) fn session_dirs(prompt: &str) -> Vec<PathBuf> {
        let mut paths = Vec::new();
        for root in SESSION_ROOTS {
            for (index, _) in prompt.match_indices(root) {
                let suffix = &prompt[index + root.len()..];
                let id: String = suffix
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '-')
                    .collect();
                if !id.is_empty() {
                    paths.push(PathBuf::from(format!("{root}{id}")));
                }
            }
        }
        paths.sort();
        paths.dedup();
        paths
    }

    /// Read a discovered roster only when its modification time changes.
    pub(super) fn roster(&mut self, directory: &Path) -> &WorkerRoster {
        &self.roster_snapshot(directory).1
    }

    /// The roster and its timestamp come from the same metadata observation.
    fn roster_snapshot(&mut self, directory: &Path) -> &(SystemTime, WorkerRoster) {
        let path = directory.join(ROSTER_FILE);
        let modified = fs::metadata(&path)
            .and_then(|metadata| metadata.modified())
            .unwrap_or(std::time::UNIX_EPOCH);
        let cached = self
            .rosters
            .entry(path)
            .or_insert_with(|| (modified, WorkerRoster::read(directory)));
        if cached.0 != modified {
            *cached = (modified, WorkerRoster::read(directory));
        }
        cached
    }

    /// Find this thread only in rosters its own task names; newest match wins.
    pub(super) fn resolve(
        &mut self,
        thread_id: &str,
        prompt: &str,
        worktree: &ThreadWorktree,
    ) -> WorkerDispatch {
        let mut selected = None;
        for directory in Self::session_dirs(prompt) {
            let (modified, roster) = self.roster_snapshot(&directory);
            let modified = *modified;
            for (seat, worker) in &roster.seats {
                if worker.thread_id != thread_id {
                    continue;
                }
                if selected.as_ref().is_some_and(|(at, _)| *at >= modified) {
                    continue;
                }
                let short = worktree
                    .seat_prefix()
                    .and_then(|prefix| seat.strip_prefix(&format!("{prefix}-")))
                    .unwrap_or(seat);
                selected = Some((
                    modified,
                    ResolvedWorkerDispatch {
                        seat:       seat.clone(),
                        identity:   WorkerIdentity::CodexThread(thread_id.to_owned()),
                        job:        JobText::parse(prompt, JobSource::FirstTaskPrompt),
                        short_seat: short.to_owned(),
                        role:       match worker.role {
                            DispatchRole::Unroled => DispatchRole::from_short_seat(short),
                            role @ DispatchRole::Worker(_) => role,
                        },
                    },
                ));
            }
        }
        selected.map_or(WorkerDispatch::Plain, |(_, dispatch)| {
            WorkerDispatch::Worker(dispatch)
        })
    }

    /// Name a launcher-free worker using the same seat and job contract as a fold.
    #[cfg(test)]
    pub(super) fn name(
        &mut self,
        thread_id: &str,
        prompt: &str,
        worktree: &ThreadWorktree,
        role: WorkerRoleSource,
    ) -> String {
        match self.resolve(thread_id, prompt, worktree) {
            WorkerDispatch::Worker(dispatch) => dispatch.label(),
            WorkerDispatch::Plain => {
                JobText::parse(prompt, JobSource::FirstTaskPrompt).label(role.text())
            },
        }
    }
}

/// A usable job, distinct from a preamble whose opening names no task.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum JobText {
    Recorded(String),
    Unrecorded,
}

/// Rollouts supply a first task line; dispatch arguments already contain the full job.
#[derive(Clone, Copy)]
enum JobSource {
    FirstTaskPrompt,
    DispatchJob,
}

/// Prompt openings name either surrounding context, a job, or role instructions.
#[derive(Clone, Copy)]
enum PromptLine<'a> {
    Blank,
    Context,
    RoleInstructions,
    Heading(&'a str),
    Title(&'a str),
    Body(&'a str),
}

impl<'a> PromptLine<'a> {
    fn classify(line: &'a str) -> Self {
        let line = line.trim();
        if line.is_empty() {
            return Self::Blank;
        }
        if line.starts_with("Worktree:")
            || (line.starts_with("From ") && line.contains("(unit director)."))
        {
            return Self::Context;
        }
        if line.starts_with(ROLE_PREAMBLE) {
            return Self::RoleInstructions;
        }
        if line.starts_with('#') {
            return Self::Heading(line);
        }
        if line.starts_with("Phase ") || line.starts_with("Ticket:") {
            return Self::Title(line);
        }
        let first = line.split_whitespace().next().unwrap_or_default();
        if let Some((project, number)) = first.trim_end_matches(':').split_once('-')
            && !project.is_empty()
            && project
                .chars()
                .all(|character| character.is_ascii_uppercase())
            && !number.is_empty()
            && number.chars().all(|character| character.is_ascii_digit())
        {
            return Self::Title(line);
        }
        Self::Body(line)
    }
}

impl JobText {
    /// Validate and flatten extracted text once, before it becomes a drawn job.
    fn from_line(line: &str) -> Self {
        let text = line
            .trim()
            .trim_start_matches('#')
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if text.is_empty() {
            Self::Unrecorded
        } else {
            Self::Recorded(text)
        }
    }

    /// One extraction serves roster, fallback and dispatch names.
    fn parse(prompt: &str, source: JobSource) -> Self {
        if let Some((_, review)) = prompt.split_once(POOL_PREAMBLE) {
            // Pool memory precedes the role preamble. The phase title names the job.
            if let Some((_, phase)) = review.split_once(POOL_PHASE_DESCRIPTION)
                && let Some((_, title)) = phase.split_once('"')
                && let Some((title, _)) = title.split_once('"')
            {
                return Self::from_line(title);
            }
            if let Some(heading) = review
                .lines()
                .skip(1)
                .map(str::trim)
                .find(|line| line.starts_with(REVIEW_JOB_HEADING))
            {
                return Self::from_line(heading);
            }
            return Self::Unrecorded;
        }
        Self::from_task_prompt(prompt, source)
    }

    /// Context opens a search for a title; role instructions carry their board job.
    fn from_task_prompt(prompt: &str, source: JobSource) -> Self {
        let first = prompt
            .lines()
            .map(PromptLine::classify)
            .find(|line| !matches!(line, PromptLine::Blank))
            .unwrap_or(PromptLine::Blank);
        match first {
            PromptLine::Heading(title) => return Self::from_line(title),
            PromptLine::Title(line) | PromptLine::Body(line) => {
                return Self::from_line(match source {
                    JobSource::FirstTaskPrompt => line,
                    JobSource::DispatchJob => prompt,
                });
            },
            PromptLine::Context => {
                if let Some(title) =
                    prompt
                        .lines()
                        .find_map(|line| match PromptLine::classify(line) {
                            PromptLine::Heading(title) | PromptLine::Title(title) => Some(title),
                            _ => None,
                        })
                {
                    return Self::from_line(title);
                }
                if !prompt
                    .lines()
                    .any(|line| matches!(PromptLine::classify(line), PromptLine::RoleInstructions))
                {
                    return Self::Unrecorded;
                }
            },
            PromptLine::RoleInstructions | PromptLine::Blank => {},
        }
        // The first prescribed `BOARD_SCRIPT` `BOARD_ROLE` call carries the director's actual job.
        for (offset, _) in prompt.match_indices(BOARD_SCRIPT) {
            let command = &prompt[offset + BOARD_SCRIPT.len()..];
            if let Ok(words) = shell_words(command, CommandBoundary::PromptLine)
                && words.first().is_some_and(|word| word == BOARD_ROLE)
                && let Some(job) = words.get(BOARD_JOB_INDEX)
            {
                return Self::from_line(job);
            }
        }
        // Some dispatches put a title in `prompt` after the role instructions instead.
        if let Some(heading) = prompt
            .lines()
            .skip(1)
            .map(str::trim)
            .find(|line| line.starts_with(REVIEW_JOB_HEADING))
        {
            return Self::from_line(heading);
        }
        Self::Unrecorded
    }

    /// Absence stays semantic until the final role-only label is drawn.
    fn label(&self, role: &str) -> String {
        match self {
            Self::Unrecorded => role.to_owned(),
            Self::Recorded(job) => format!("{role} · {job}"),
        }
    }
}

impl<'de> Deserialize<'de> for JobText {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(|text| Self::from_line(&text))
    }
}

impl Serialize for JobText {
    /// Keep the legacy probe's string field; empty text exists only on the wire.
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(match self {
            Self::Recorded(job) => job,
            Self::Unrecorded => "",
        })
    }
}

/// How the launcher selects its seat before its roster is read.
#[derive(Clone, Debug, Eq, PartialEq)]
enum SeatSelection {
    /// An explicit `--to` names the roster entry.
    Named(String),
    /// A new dispatch is the entry bearing its launcher pid.
    Launcher,
}

/// A literal implement.sh invocation, before roster resolution.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ParsedDispatch {
    /// The directory containing `mesh_roster.json`.
    pub(super) session_dir: PathBuf,
    /// The worktree whose basename prefixes its seat.
    worktree:               PathBuf,
    /// How this invocation locates its roster entry.
    seat:                   SeatSelection,
    /// The launcher's responsibility argument.
    job:                    String,
    /// The pass role can differ from the durable seat slot.
    pass_role:              DispatchRole,
}

/// The command cannot safely be interpreted as one worker invocation.
#[derive(Debug, Eq, PartialEq)]
pub(super) struct NotWorkerDispatch;

impl ParsedDispatch {
    /// The live mesh child names a fresh or follow-up seat authoritatively.
    pub(super) fn select_seat(&mut self, seat: String) { self.seat = SeatSelection::Named(seat); }

    /// A live launcher's wrapper may set variables before its final dispatch.
    /// Commands following the dispatch still belong to the shell and must stay visible.
    pub(super) fn is_final_command_of(&self, command: &str) -> bool {
        if Self::parse(command).is_ok_and(|parsed| parsed == *self) {
            return true;
        }
        let mut quote = None;
        let mut escaped = false;
        let mut start = 0;
        for (offset, character) in command.char_indices() {
            if escaped {
                escaped = false;
                continue;
            }
            match (quote, character) {
                (Some('\''), '\'') | (Some('"'), '"') => quote = None,
                (None, '\'' | '"') => quote = Some(character),
                (Some('\''), _) => {},
                (_, '\\') => escaped = true,
                (None, ';' | '&' | '|' | '\n') => start = offset + character.len_utf8(),
                _ => {},
            }
        }
        // Values are already resolved in the observed launcher argv. In `final_command`, variable
        // spellings only establish that the wrapper ends with one dispatch command.
        let final_command = command[start..].replace('$', "_");
        Self::parse(&final_command).is_ok_and(|parsed| parsed == *self)
    }

    /// Read literal assignments followed by bash implement.sh without executing them.
    pub(super) fn parse(command: &str) -> Result<Self, NotWorkerDispatch> {
        Self::from_arguments(&command_words(command)?)
    }

    /// Read argv from the actual launcher process without re-splitting it.
    pub(super) fn from_arguments(words: &[String]) -> Result<Self, NotWorkerDispatch> {
        let mut args = words.iter().map(String::as_str);
        let program = args.next().ok_or(NotWorkerDispatch)?;
        if Path::new(program)
            .file_name()
            .is_none_or(|name| name != LAUNCHER_INTERPRETER)
        {
            return Err(NotWorkerDispatch);
        }
        let script = args.next().ok_or(NotWorkerDispatch)?;
        if !Path::new(script).ends_with(LAUNCHER_SCRIPT) {
            return Err(NotWorkerDispatch);
        }
        let mut args = args.peekable();
        let mut seat = SeatSelection::Launcher;
        while args.peek().is_some_and(|word| word.starts_with("--")) {
            match args.next() {
                Some("--to") => {
                    let name = args
                        .next()
                        .filter(|name| !name.is_empty())
                        .ok_or(NotWorkerDispatch)?;
                    seat = SeatSelection::Named(name.to_owned());
                },
                Some("--phase") => {
                    args.next()
                        .filter(|phase| !phase.is_empty())
                        .ok_or(NotWorkerDispatch)?;
                },
                _ => return Err(NotWorkerDispatch),
            }
        }
        let session_dir = args
            .next()
            .filter(|dir| !dir.is_empty())
            .ok_or(NotWorkerDispatch)?;
        let worktree = args
            .next()
            .filter(|dir| !dir.is_empty())
            .ok_or(NotWorkerDispatch)?;
        args.next()
            .filter(|prompt| !prompt.is_empty())
            .ok_or(NotWorkerDispatch)?;
        match args.next() {
            Some("impl" | "test" | "review" | "fix") => {},
            _ => return Err(NotWorkerDispatch),
        }
        let job = args
            .next()
            .filter(|job| !job.is_empty())
            .ok_or(NotWorkerDispatch)?;
        // The launcher positions in `remaining` are pass kind, activity, fix pass,
        // team role, and an optional mesh prefix. Nothing may follow them.
        let remaining: Vec<_> = args.collect();
        match remaining.as_slice() {
            ["impl" | "test" | "review" | "fix", _, _, team_role]
            | ["impl" | "test" | "review" | "fix", _, _, team_role, _]
                if !team_role.is_empty() => {},
            _ => return Err(NotWorkerDispatch),
        }
        Ok(Self {
            session_dir: PathBuf::from(session_dir),
            worktree: PathBuf::from(worktree),
            seat,
            job: job.to_owned(),
            pass_role: DispatchRole::from_short_seat(remaining[0]),
        })
    }

    /// Join the explicit seat, or this dispatch's pid, to a recorded thread.
    pub(super) fn resolve(&self, roster: &WorkerRoster, dispatch_pid: u32) -> WorkerDispatch {
        let entry = match &self.seat {
            SeatSelection::Named(seat) => roster.seats.get_key_value(seat),
            SeatSelection::Launcher => {
                let mut matches = roster.seats.iter().filter(|(_, value)| {
                    value.launcher == LauncherIdentity::Recorded(dispatch_pid)
                });
                let first = matches.next();
                if matches.next().is_some() {
                    return WorkerDispatch::Plain;
                }
                first
            },
        };
        let Some((seat, entry)) = entry else {
            return WorkerDispatch::Plain;
        };

        let short_seat = self
            .worktree
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| seat.strip_prefix(&format!("{name}-")))
            .unwrap_or(seat)
            .to_owned();
        WorkerDispatch::Worker(ResolvedWorkerDispatch {
            seat: seat.clone(),
            identity: WorkerIdentity::CodexThread(entry.thread_id.clone()),
            job: JobText::parse(&self.job, JobSource::DispatchJob),
            role: match entry.role {
                DispatchRole::Unroled => DispatchRole::from_short_seat(&short_seat),
                role @ DispatchRole::Worker(_) => role,
            },
            short_seat,
        })
    }
}

/// Whether a roster entry records its launcher's process identity.
#[derive(Clone, Debug, Eq, PartialEq)]
enum LauncherIdentity {
    /// Finished seats may no longer have a launcher pid.
    Unrecorded,
    /// The pid can identify a dispatch without an explicit seat.
    Recorded(u32),
}

/// A roster entry with a usable thread identity.
#[derive(Clone, Debug)]
struct RosterWorker {
    /// The nonempty thread id to join to the census.
    thread_id: String,
    /// The dispatch pid, where the roster records it.
    launcher:  LauncherIdentity,
    /// The supported worker responsibility, if the roster records one.
    role:      DispatchRole,
}

/// A typed JSON boundary that ignores an absent or malformed field.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RosterField<T> {
    /// A field with the type expected by this reader.
    Recorded(T),
    /// Unknown values do not invalidate the seat's other fields.
    Unrecorded(IgnoredAny),
}

impl<T> Default for RosterField<T> {
    fn default() -> Self { Self::Unrecorded(IgnoredAny) }
}

/// The fields read from a seat, independent of unrelated roster metadata.
#[derive(Debug, Deserialize)]
struct RosterRecord {
    /// A thread is required before the seat can participate in a fold.
    thread_id:    String,
    /// Some completed seats no longer have a launcher process.
    #[serde(default)]
    launcher_pid: RosterField<u32>,
    /// Older rosters may omit a worker responsibility.
    #[serde(default)]
    role:         RosterField<WorkerRole>,
}

/// A malformed seat must not hide valid neighbours in the same roster.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RosterEntry {
    /// A seat with the expected thread identity field.
    Worker(RosterRecord),
    /// Unrelated or malformed entries contribute no worker identity.
    Invalid(IgnoredAny),
}

/// The seat entries read once per session directory during a scan.
#[derive(Clone, Debug, Default)]
pub(super) struct WorkerRoster {
    /// Entries whose thread identity was validated at the JSON boundary.
    seats: BTreeMap<String, RosterWorker>,
}

impl WorkerRoster {
    /// Malformed or unavailable rosters provide no worker identities.
    pub(super) fn parse(text: &str) -> Self {
        let entries: BTreeMap<String, RosterEntry> = serde_json::from_str(text).unwrap_or_default();
        let seats = entries
            .into_iter()
            .filter_map(|(seat, entry)| {
                let RosterEntry::Worker(record) = entry else {
                    return None;
                };
                let thread_id = record.thread_id.trim();
                if seat.is_empty() || thread_id.is_empty() {
                    return None;
                }
                let launcher = match record.launcher_pid {
                    RosterField::Recorded(pid) => LauncherIdentity::Recorded(pid),
                    RosterField::Unrecorded(_) => LauncherIdentity::Unrecorded,
                };
                let role = match record.role {
                    RosterField::Recorded(role) => DispatchRole::Worker(role),
                    RosterField::Unrecorded(_) => DispatchRole::Unroled,
                };
                Some((
                    seat,
                    RosterWorker {
                        thread_id: thread_id.to_owned(),
                        launcher,
                        role,
                    },
                ))
            })
            .collect();
        Self { seats }
    }

    /// Read the roster only during census scanning.
    pub(super) fn read(session_dir: &Path) -> Self {
        fs::read_to_string(session_dir.join(ROSTER_FILE))
            .map(|text| Self::parse(&text))
            .unwrap_or_default()
    }
}

/// A thread's original label, or a matched dispatch's responsibility.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) enum DispatchLabel<'a> {
    /// No shell was folded into this row.
    #[default]
    Original,
    /// The same tile contains this worker's dispatch.
    Worker(&'a ResolvedWorkerDispatch),
}

/// A complete literal word, classified before its position in the command is known.
#[derive(Debug)]
enum LiteralWord {
    /// An unquoted shell identifier followed by an unquoted equals sign.
    Assignment(String),
    /// A word that cannot introduce an environment assignment.
    Argument(String),
}

impl LiteralWord {
    /// Both kinds retain their full text for use after the command word.
    const fn text_mut(&mut self) -> &mut String {
        match self {
            Self::Assignment(text) | Self::Argument(text) => text,
        }
    }

    /// After the assignment prefix, every word is an ordinary argv entry.
    fn into_argument(self) -> String {
        match self {
            Self::Assignment(text) | Self::Argument(text) => text,
        }
    }
}

/// Quoting before the first equals sign rules out an assignment name.
#[derive(Default)]
enum WordBeingLexed {
    /// Whitespace has not yet been followed by a word.
    #[default]
    Empty,
    /// No quote, escape, or equals sign has appeared in this word yet.
    UnquotedPrefix(String),
    /// The word's assignment syntax has been determined.
    Classified(LiteralWord),
}

impl WordBeingLexed {
    /// Only an unquoted equals sign after a shell identifier makes an assignment.
    fn push_unquoted(&mut self, character: char) {
        match self {
            Self::Empty if character == '=' => {
                *self = Self::Classified(LiteralWord::Argument(character.to_string()));
            },
            Self::Empty => *self = Self::UnquotedPrefix(character.to_string()),
            Self::UnquotedPrefix(text) if character == '=' => {
                let mut characters = text.chars();
                let valid_name = characters
                    .next()
                    .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
                    && characters
                        .all(|character| character.is_ascii_alphanumeric() || character == '_');
                let mut text = std::mem::take(text);
                text.push(character);
                *self = Self::Classified(if valid_name {
                    LiteralWord::Assignment(text)
                } else {
                    LiteralWord::Argument(text)
                });
            },
            Self::UnquotedPrefix(text) => text.push(character),
            Self::Classified(word) => word.text_mut().push(character),
        }
    }

    /// Quoted or escaped text can belong to an assignment value, but never its name.
    fn push_quoted(&mut self, quoted: &str) {
        match self {
            Self::Empty => *self = Self::Classified(LiteralWord::Argument(quoted.to_owned())),
            Self::UnquotedPrefix(text) => {
                let mut text = std::mem::take(text);
                text.push_str(quoted);
                *self = Self::Classified(LiteralWord::Argument(text));
            },
            Self::Classified(word) => word.text_mut().push_str(quoted),
        }
    }

    /// Complete a word without turning whitespace into an empty argument.
    fn finish(&mut self, words: &mut Vec<LiteralWord>) {
        match std::mem::take(self) {
            Self::Empty => {},
            Self::UnquotedPrefix(text) => words.push(LiteralWord::Argument(text)),
            Self::Classified(word) => words.push(word),
        }
    }
}

/// The literal quoting forms accepted inside a shell word.
#[derive(Clone, Copy)]
enum QuotedSegment {
    /// Single quotes preserve every character until the closing quote.
    Single,
    /// Double quotes allow escaped punctuation, but expansions remain forbidden.
    Double,
    /// Dollar-single quotes decode Bash's ANSI-C escapes without expansion.
    AnsiC,
}

impl QuotedSegment {
    /// Read after the opening delimiter, consuming the closing delimiter too.
    fn read(self, chars: &mut Peekable<Chars<'_>>) -> Result<String, NotWorkerDispatch> {
        if matches!(self, Self::AnsiC) {
            return read_ansi_c_segment(chars);
        }
        let mut quoted = String::new();
        loop {
            match (self, chars.next()) {
                (Self::Single, Some('\'')) | (Self::Double, Some('"')) => return Ok(quoted),
                (Self::Double, Some('\\')) => match chars.next() {
                    Some(escaped @ ('$' | '`' | '"' | '\\')) => quoted.push(escaped),
                    Some('\n') => {},
                    Some(other) => {
                        quoted.push('\\');
                        quoted.push(other);
                    },
                    None => return Err(NotWorkerDispatch),
                },
                (Self::Double, Some('$' | '`')) | (_, None) => return Err(NotWorkerDispatch),
                (_, Some(inner)) => quoted.push(inner),
            }
        }
    }
}

/// Decode bytes first, since octal and hex escapes may form one UTF-8 character.
fn read_ansi_c_segment(chars: &mut Peekable<Chars<'_>>) -> Result<String, NotWorkerDispatch> {
    let mut quoted = Vec::new();
    loop {
        match chars.next() {
            Some('\'') => break,
            Some('\\') => read_ansi_c_escape(chars, &mut quoted)?,
            Some(inner) => quoted.extend_from_slice(inner.encode_utf8(&mut [0; 4]).as_bytes()),
            None => return Err(NotWorkerDispatch),
        }
    }
    // Bash words cannot contain NUL: it ends `quoted`'s decoded text.
    if let Some(end) = quoted.iter().position(|byte| *byte == 0) {
        quoted.truncate(end);
    }
    String::from_utf8(quoted).map_err(|_| NotWorkerDispatch)
}

/// Consume one escape after its backslash; unknown escapes keep the backslash.
fn read_ansi_c_escape(
    chars: &mut Peekable<Chars<'_>>,
    quoted: &mut Vec<u8>,
) -> Result<(), NotWorkerDispatch> {
    match chars.next().ok_or(NotWorkerDispatch)? {
        'a' => quoted.push(7),
        'b' => quoted.push(8),
        'e' | 'E' => quoted.push(27),
        'f' => quoted.push(12),
        'n' => quoted.push(b'\n'),
        'r' => quoted.push(b'\r'),
        't' => quoted.push(b'\t'),
        'v' => quoted.push(11),
        escaped @ ('\\' | '\'' | '"' | '?') => {
            quoted.extend_from_slice(escaped.encode_utf8(&mut [0; 4]).as_bytes());
        },
        first @ '0'..='7' => {
            let digits: String = std::iter::once(first)
                .chain(std::iter::from_fn(|| chars.next_if(|c| c.is_digit(8))).take(2))
                .collect();
            let value = u32::from_str_radix(&digits, 8).map_err(|_| NotWorkerDispatch)?;
            quoted.push(value.to_le_bytes()[0]);
        },
        escape @ ('x' | 'u' | 'U') => {
            let limit = match escape {
                'x' => 2,
                'u' => 4,
                _ => 8,
            };
            let digits: String = std::iter::from_fn(|| chars.next_if(char::is_ascii_hexdigit))
                .take(limit)
                .collect();
            if digits.is_empty() {
                quoted.push(b'\\');
                quoted.extend_from_slice(escape.encode_utf8(&mut [0; 4]).as_bytes());
            } else {
                let value = u32::from_str_radix(&digits, 16).map_err(|_| NotWorkerDispatch)?;
                if escape == 'x' {
                    quoted.push(value.to_le_bytes()[0]);
                } else {
                    let character = char::from_u32(value).ok_or(NotWorkerDispatch)?;
                    quoted.extend_from_slice(character.encode_utf8(&mut [0; 4]).as_bytes());
                }
            }
        },
        'c' => {
            if chars.peek().is_none_or(|next| *next == '\'') {
                quoted.extend_from_slice(b"\\c");
                return Ok(());
            }
            let control = chars
                .next()
                .filter(char::is_ascii)
                .ok_or(NotWorkerDispatch)?;
            if control == '\\' {
                chars.next_if(|next| *next == '\\');
            }
            let byte = u8::try_from(u32::from(control)).map_err(|_| NotWorkerDispatch)?;
            quoted.push(if byte == b'?' { 127 } else { byte & 31 });
        },
        other => {
            quoted.push(b'\\');
            quoted.extend_from_slice(other.encode_utf8(&mut [0; 4]).as_bytes());
        },
    }
    Ok(())
}

/// Split one literal command into launcher argv, consuming leading assignments.
/// Expansions and command operators cannot establish a safe worker identity.
pub(super) fn command_words(command: &str) -> Result<Vec<String>, NotWorkerDispatch> {
    shell_words(command, CommandBoundary::WholeInvocation)
}

/// Literal launchers consume the whole input; prompt examples stop at prose delimiters.
#[derive(Clone, Copy)]
enum CommandBoundary {
    WholeInvocation,
    PromptLine,
}

/// Quoted segments consume their own delimiters before command boundaries are checked.
fn shell_words(command: &str, boundary: CommandBoundary) -> Result<Vec<String>, NotWorkerDispatch> {
    let mut words = Vec::new();
    let mut word = WordBeingLexed::Empty;
    let mut chars = command.chars().peekable();
    while let Some(character) = chars.next() {
        match character {
            ' ' | '\t' => word.finish(&mut words),
            '\'' => word.push_quoted(&QuotedSegment::Single.read(&mut chars)?),
            '"' => word.push_quoted(&QuotedSegment::Double.read(&mut chars)?),
            '$' => {
                if chars.next() != Some('\'') {
                    return Err(NotWorkerDispatch);
                }
                word.push_quoted(&QuotedSegment::AnsiC.read(&mut chars)?);
            },
            '\\' => match chars.next() {
                Some('\n') => {},
                Some(escaped) => word.push_quoted(&escaped.to_string()),
                None => return Err(NotWorkerDispatch),
            },
            '\n' | '`' if matches!(boundary, CommandBoundary::PromptLine) => break,
            '\n' | '`' | ';' | '&' | '|' | '(' | ')' | '<' | '>' => {
                return Err(NotWorkerDispatch);
            },
            other => word.push_unquoted(other),
        }
    }
    word.finish(&mut words);
    Ok(words
        .into_iter()
        .skip_while(|word| matches!(word, LiteralWord::Assignment(_)))
        .map(LiteralWord::into_argument)
        .collect())
}

#[cfg(test)]
mod tests;

#[cfg(test)]
#[path = "naming_tests.rs"]
mod naming_tests;

#[cfg(test)]
mod dispatch_form_tests;

#[cfg(test)]
mod prompt_opening_tests;

#[cfg(test)]
mod context_job_tests;
