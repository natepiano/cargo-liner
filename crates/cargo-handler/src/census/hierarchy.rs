//! Bounded hierarchy commands and their per-session-set cache, run off the UI thread.

#[path = "command_inputs.rs"]
mod command_inputs;

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::error::Error;
use std::fmt::Display;
use std::fmt::Formatter;
use std::fs;
use std::io::Read;
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::path::PathBuf;
use std::process::Child;
use std::process::Command;
use std::process::Stdio;
use std::sync::Mutex;
use std::sync::mpsc;
use std::sync::mpsc::RecvTimeoutError;
use std::thread;
use std::time::Duration;
use std::time::Instant;
use std::time::SystemTime;

pub(super) use command_inputs::FileRevision;
pub(super) use command_inputs::InputDrivenCommand;
use serde::Deserialize;
use serde::Serialize;
use sysinfo::Pid;
use sysinfo::ProcessRefreshKind;
use sysinfo::ProcessesToUpdate;
use sysinfo::System;

use super::AgentRow;
use super::CodexThreadId;
use super::HierarchyEdge;
use super::HierarchyEdges;
use super::HierarchyRole;
use super::HierarchySource;
use super::SessionIdentity;
use super::worker_dispatch::JobText;
use super::worker_dispatch::WorkerTask;
use crate::constants::HIERARCHY_OUTPUT_LIMIT;
use crate::constants::HIERARCHY_REFRESH_INTERVAL;
use crate::constants::HIERARCHY_SCHEMA;
use crate::constants::HIERARCHY_TIMEOUT;
#[cfg(unix)]
use crate::constants::KILL_GROUP_OPTIONS;
#[cfg(unix)]
use crate::constants::KILL_PROGRAM;
use crate::constants::PROBE_EXIT_CHECK;
use crate::constants::REVIEW_POOL_DIRECTORY;
use crate::constants::REVIEW_POOL_FILE;
use crate::constants::SHELL_PROGRAM;
use crate::constants::SHELL_SCRIPT_FLAG;
use crate::startup_log;

/// The source machine decides whether a stored reviewer can draw.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ReviewerState {
    #[default]
    NotPool,
    Idle,
    /// A command identified a pool edge. Its process still needs validation.
    Unverified,
    Busy {
        name:    String,
        pid:     u32,
        started: u64,
    },
}

impl ReviewerState {
    pub(super) const fn is_drawn(&self) -> bool {
        matches!(self, Self::NotPool | Self::Busy { .. })
    }
}

/// Source-side reviewer state travels independently of retained manager links.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(super) struct ReviewerOutcome {
    pub(super) thread_id: String,
    pub(super) state:     ReviewerState,
}

/// The pool location is unavailable when the user's home is unknown.
pub(super) enum ReviewerPoolLocation {
    MissingHome,
    Directory(PathBuf),
}

impl ReviewerPoolLocation {
    pub(super) fn update(&self, pools: &mut ReviewerPools) -> Vec<(PathBuf, SystemTime)> {
        match self {
            Self::MissingHome => {
                pools.documents.clear();
                Vec::new()
            },
            Self::Directory(path) => pools.update(path),
        }
    }

    pub(super) fn read(&self) -> ReviewerPools {
        match self {
            Self::MissingHome => ReviewerPools::default(),
            Self::Directory(path) => ReviewerPools::read(path),
        }
    }
}

/// Missing leases do not authorize a stored reviewer to draw.
#[derive(Debug, Default, Deserialize)]
#[serde(from = "Option<PoolLeaseRecord>")]
enum PoolLease {
    #[default]
    Unleased,
    Leased,
}

#[derive(Deserialize)]
struct PoolLeaseRecord {
    unit: String,
}

impl From<Option<PoolLeaseRecord>> for PoolLease {
    fn from(record: Option<PoolLeaseRecord>) -> Self {
        match record {
            Some(record) if !record.unit.is_empty() => Self::Leased,
            _ => Self::Unleased,
        }
    }
}

/// A process claim still needs a liveness check on each scan.
#[derive(Debug, Default, Deserialize)]
#[serde(from = "Option<u32>")]
enum ReviewerProcessClaim {
    #[default]
    Unclaimed,
    Claimed(u32),
}

impl From<Option<u32>> for ReviewerProcessClaim {
    fn from(pid: Option<u32>) -> Self { pid.map_or(Self::Unclaimed, Self::Claimed) }
}

/// A missing allocation time uses the claimed process's start.
#[derive(Debug, Default, Deserialize)]
#[serde(from = "Option<f64>")]
enum ReviewAllocation {
    #[default]
    ProcessStart,
    Recorded(u64),
}

impl From<Option<f64>> for ReviewAllocation {
    fn from(seconds: Option<f64>) -> Self {
        seconds
            .and_then(|seconds| Duration::try_from_secs_f64(seconds).ok())
            .and_then(|duration| duration.as_millis().try_into().ok())
            .map_or(Self::ProcessStart, Self::Recorded)
    }
}

#[derive(Debug, Deserialize)]
struct PoolReviewer {
    thread_id:      String,
    #[serde(default)]
    lease:          PoolLease,
    #[serde(default)]
    busy_pid:       ReviewerProcessClaim,
    #[serde(default)]
    last_allocated: ReviewAllocation,
}

#[derive(Debug, Deserialize)]
struct PoolTemplate {
    thread_id: String,
}

/// Unknown pool fields stay at the JSON boundary.
#[derive(Debug, Default, Deserialize)]
struct ReviewerPoolDocument {
    #[serde(default)]
    reviewers: BTreeMap<String, PoolReviewer>,
    #[serde(default)]
    templates: BTreeMap<String, PoolTemplate>,
}

/// Pool documents are reread only when their modification time changes.
#[derive(Debug, Default)]
pub(super) struct ReviewerPools {
    documents: BTreeMap<PathBuf, (SystemTime, ReviewerPoolDocument)>,
    processes: Mutex<System>,
}

impl ReviewerPools {
    pub(super) fn root() -> ReviewerPoolLocation {
        dirs::home_dir().map_or(ReviewerPoolLocation::MissingHome, |home| {
            ReviewerPoolLocation::Directory(home.join(REVIEW_POOL_DIRECTORY))
        })
    }

    pub(super) fn read(root: &Path) -> Self {
        let mut pools = Self::default();
        pools.update(root);
        pools
    }

    /// Return a stable key for hierarchy cache invalidation.
    pub(super) fn update(&mut self, root: &Path) -> Vec<(PathBuf, SystemTime)> {
        let mut current = BTreeSet::new();
        for directory in fs::read_dir(root).into_iter().flatten().flatten() {
            let path = directory.path().join(REVIEW_POOL_FILE);
            let Ok(modified) = fs::metadata(&path).and_then(|metadata| metadata.modified()) else {
                continue;
            };
            current.insert(path.clone());
            if self
                .documents
                .get(&path)
                .is_none_or(|(previous, _)| *previous != modified)
            {
                let document = fs::read(&path)
                    .ok()
                    .and_then(|bytes| serde_json::from_slice(&bytes).ok())
                    .unwrap_or_default();
                self.documents.insert(path, (modified, document));
            }
        }
        self.documents.retain(|path, _| current.contains(path));
        self.documents
            .iter()
            .map(|(path, (modified, _))| (path.clone(), *modified))
            .collect()
    }

    pub(super) fn outcomes(&self) -> Vec<ReviewerOutcome> {
        let mut edges = HierarchyEdges(
            self.documents
                .values()
                .flat_map(|(_, document)| document.reviewers.values())
                .map(|record| record.thread_id.as_str())
                .map(|id| HierarchyEdge {
                    manager:  String::new(),
                    member:   id.to_owned(),
                    role:     HierarchyRole::Review,
                    reviewer: ReviewerState::Unverified,
                })
                .collect(),
        );
        self.refresh(&mut edges);
        let mut outcomes: Vec<_> = edges
            .0
            .into_iter()
            .map(|edge| ReviewerOutcome {
                thread_id: edge.member,
                state:     edge.reviewer,
            })
            .collect();
        outcomes.extend(
            self.documents
                .values()
                .flat_map(|(_, document)| document.templates.values())
                .map(|record| record.thread_id.as_str())
                .map(|id| ReviewerOutcome {
                    thread_id: id.to_owned(),
                    state:     ReviewerState::Idle,
                }),
        );
        outcomes
    }

    pub(super) fn apply_outcomes(rows: &mut [AgentRow], outcomes: &[ReviewerOutcome]) {
        for row in rows {
            row.children.retain(|child| !matches!(&child.thread_id, CodexThreadId::Recorded(id)
                if outcomes.iter().any(|outcome| outcome.thread_id == *id && !outcome.state.is_drawn())));
        }
    }

    /// Recheck process identity even when the hierarchy command failed.
    pub(super) fn refresh(&self, edges: &mut HierarchyEdges) {
        let reviewers: Vec<_> = self
            .documents
            .values()
            .flat_map(|(_, document)| document.reviewers.iter())
            .collect();
        let pids: Vec<_> = reviewers
            .iter()
            .filter_map(|(_, record)| match record.busy_pid {
                ReviewerProcessClaim::Claimed(pid) => Some(Pid::from_u32(pid)),
                ReviewerProcessClaim::Unclaimed => None,
            })
            .collect();
        let mut system = self
            .processes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // sysinfo exposes no removal API. Rebuild only when a claim is retired,
        // refreshing the current claims rather than the whole process table.
        if system.processes().keys().any(|pid| !pids.contains(pid)) {
            *system = System::new();
        }
        system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&pids),
            true,
            ProcessRefreshKind::nothing().without_tasks(),
        );
        for edge in &mut edges.0 {
            let record = reviewers
                .iter()
                .find(|(_, record)| record.thread_id == edge.member);
            if let Some((name, record)) = record {
                edge.reviewer = match (&record.lease, &record.busy_pid) {
                    (PoolLease::Leased, ReviewerProcessClaim::Claimed(pid)) => system
                        .process(Pid::from_u32(*pid))
                        .map_or(ReviewerState::Idle, |process| ReviewerState::Busy {
                            name:    (*name).clone(),
                            pid:     *pid,
                            started: match record.last_allocated {
                                ReviewAllocation::Recorded(time) => time,
                                ReviewAllocation::ProcessStart => {
                                    process.start_time().saturating_mul(1000)
                                },
                            },
                        }),
                    _ => ReviewerState::Idle,
                };
            } else if !matches!(edge.reviewer, ReviewerState::NotPool) {
                edge.reviewer = ReviewerState::Idle;
            }
        }
        drop(system);
    }

    /// Current source outcomes replace stale reviewer state on retained links.
    pub(super) fn update_edges(edges: &mut HierarchyEdges, outcomes: &[ReviewerOutcome]) {
        for edge in &mut edges.0 {
            if let Some(outcome) = outcomes
                .iter()
                .find(|outcome| outcome.thread_id == edge.member)
            {
                edge.reviewer = outcome.state.clone();
            }
        }
    }

    /// Idle pool threads must not leak through ordinary app-server rows.
    pub(super) fn apply(rows: &mut [AgentRow], edges: &HierarchyEdges) {
        for row in rows {
            row.children.retain_mut(|child| {
                let CodexThreadId::Recorded(id) = &child.thread_id else {
                    return true;
                };
                let Some(edge) = edges.0.iter().find(|edge| edge.member == *id) else {
                    return true;
                };
                if let ReviewerState::Busy { name, .. } = &edge.reviewer
                    && !matches!(child.worker_task, WorkerTask::Reviewer { .. })
                {
                    child.worker_task = WorkerTask::Reviewer {
                        name: name.clone(),
                        job:  JobText::Unrecorded,
                    };
                }
                edge.reviewer.is_drawn()
            });
        }
    }
}

/// Why hierarchy discovery could not produce a usable answer.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum HierarchyError {
    /// The configured shell could not start.
    CouldNotStart,
    /// The command or its output exceeded the deadline.
    TimedOut,
    /// The command exited with a failure status.
    ExitedUnsuccessfully,
    /// Stdout could not be read or was not a hierarchy report.
    OutputUnreadable,
    /// Stdout exceeded the permitted answer size.
    OutputTooLarge,
    /// The command's exit status could not be inspected.
    ExitUnreadable,
    /// The answer uses another hierarchy contract version.
    UnknownSchema(u32),
    /// An edge is missing a required field.
    MalformedEdge,
    /// An edge names an empty manager or member session.
    EmptySessionId,
}

/// Discovery health travels with retained links instead of disappearing on failure.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum HierarchyNotice {
    #[default]
    Clear,
    Failed(HierarchyError),
    NoSessionMatched,
}

impl HierarchyNotice {
    pub(crate) const fn is_visible(self) -> bool { !matches!(self, Self::Clear) }

    /// One summary line; clear discovery contributes no line at all.
    pub(crate) fn text(self) -> String {
        match self {
            Self::Clear => String::new(),
            Self::Failed(HierarchyError::TimedOut) => "hierarchy: timed out".to_owned(),
            Self::Failed(reason) => format!("hierarchy: {reason}"),
            Self::NoSessionMatched => "hierarchy: no session matched".to_owned(),
        }
    }
}

/// Usable management links and the latest discovery outcome are one publication.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct HierarchySnapshot {
    pub(crate) edges:  HierarchyEdges,
    pub(crate) notice: HierarchyNotice,
}

impl From<HierarchyEdges> for HierarchySnapshot {
    fn from(edges: HierarchyEdges) -> Self {
        Self {
            edges,
            notice: HierarchyNotice::Clear,
        }
    }
}

impl HierarchySnapshot {
    pub(crate) fn matched(mut self, rows: &[AgentRow]) -> Self {
        if self.notice == HierarchyNotice::Clear && !self.edges.0.iter().any(|edge| {
            rows.iter().any(|row| {
                matches!(&row.session_id, SessionIdentity::Claude(id) if id == &edge.member || id == &edge.manager)
                || row.children.iter().any(|child| matches!(&child.thread_id, CodexThreadId::Recorded(id) if id == &edge.member))
            })
        }) {
            self.notice = HierarchyNotice::NoSessionMatched;
        }
        self
    }
}

impl Display for HierarchyError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CouldNotStart => formatter.write_str("command could not start"),
            Self::TimedOut => formatter.write_str("command timed out"),
            Self::ExitedUnsuccessfully => formatter.write_str("command failed"),
            Self::OutputUnreadable => formatter.write_str("unreadable hierarchy output"),
            Self::OutputTooLarge => formatter.write_str("hierarchy output too large"),
            Self::ExitUnreadable => formatter.write_str("command exit status unreadable"),
            Self::UnknownSchema(schema) => write!(formatter, "unknown hierarchy schema {schema}"),
            Self::MalformedEdge => formatter.write_str("malformed hierarchy edge"),
            Self::EmptySessionId => formatter.write_str("empty hierarchy session id"),
        }
    }
}

impl Error for HierarchyError {}

/// A source of hierarchy edges, replaceable by fixtures in scheduler tests.
pub(crate) trait HierarchyRunner: Send + Sync {
    /// Run `command`, allowing at most `timeout` for output and exit.
    fn run(&self, command: &str, timeout: Duration) -> Result<HierarchyEdges, HierarchyError>;
}

/// Runs the configured shell command on this machine.
pub(crate) struct CommandRunner;

impl HierarchyRunner for CommandRunner {
    fn run(&self, command: &str, timeout: Duration) -> Result<HierarchyEdges, HierarchyError> {
        parse(&run_output(command, timeout)?)
    }
}

/// Production hierarchy reads retain an answer while known script inputs stay unchanged.
#[derive(Default)]
pub(super) struct InputDrivenHierarchyRunner(
    Mutex<BTreeMap<String, InputDrivenCommand<Result<HierarchyEdges, HierarchyError>>>>,
);

impl HierarchyRunner for InputDrivenHierarchyRunner {
    fn run(&self, command: &str, timeout: Duration) -> Result<HierarchyEdges, HierarchyError> {
        let mut readings = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let inputs = readings.entry(command.to_owned()).or_insert_with(|| {
            InputDrivenCommand::for_command(command, HIERARCHY_REFRESH_INTERVAL)
        });
        let answer = inputs.read(Instant::now(), || CommandRunner.run(command, timeout));
        if answer.is_err() {
            inputs.invalidate();
        }
        drop(readings);
        answer
    }
}

/// Run a bounded JSON-producing command away from the frame thread.
pub(super) fn run_output(command: &str, timeout: Duration) -> Result<Vec<u8>, HierarchyError> {
    let deadline = Instant::now() + timeout;
    let mut process = Command::new(SHELL_PROGRAM);
    process
        .arg(SHELL_SCRIPT_FLAG)
        .arg(command)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .stdout(Stdio::piped());
    // Set `process.process_group(0)` so `end` can terminate the shell's
    // descendants when `deadline` expires.
    #[cfg(unix)]
    {
        process.process_group(0);
    }
    let mut child = process.spawn().map_err(|_| HierarchyError::CouldNotStart)?;
    let Some(stdout) = child.stdout.take() else {
        end(&mut child);
        return Err(HierarchyError::OutputUnreadable);
    };
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let _ = sender.send(read_output(stdout));
    });
    let output = match receiver.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
        Ok(Ok(output)) => output,
        Ok(Err(error)) => {
            end(&mut child);
            return Err(error);
        },
        Err(error) => {
            end(&mut child);
            return Err(match error {
                RecvTimeoutError::Timeout => HierarchyError::TimedOut,
                RecvTimeoutError::Disconnected => HierarchyError::OutputUnreadable,
            });
        },
    };
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(output),
            Ok(Some(_)) => return Err(HierarchyError::ExitedUnsuccessfully),
            Ok(None) if Instant::now() < deadline => thread::sleep(
                PROBE_EXIT_CHECK.min(deadline.saturating_duration_since(Instant::now())),
            ),
            Ok(None) => {
                end(&mut child);
                return Err(HierarchyError::TimedOut);
            },
            Err(_) => {
                end(&mut child);
                return Err(HierarchyError::ExitUnreadable);
            },
        }
    }
}

/// Kill the command's descendants as well as the shell, then reap the shell.
fn end(child: &mut Child) {
    #[cfg(unix)]
    {
        // `run_output` created `child`'s process group; `KILL_PROGRAM` with
        // `KILL_GROUP_OPTIONS` and negative `child.id()` terminates that group.
        let _ = Command::new(KILL_PROGRAM)
            .args(KILL_GROUP_OPTIONS)
            .arg(format!("-{}", child.id()))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// Read at most one byte beyond the limit, so oversized output fails without waiting for EOF.
fn read_output(stdout: impl Read) -> Result<Vec<u8>, HierarchyError> {
    let mut bytes = Vec::new();
    stdout
        .take((HIERARCHY_OUTPUT_LIMIT + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| HierarchyError::OutputUnreadable)?;
    if bytes.len() > HIERARCHY_OUTPUT_LIMIT {
        return Err(HierarchyError::OutputTooLarge);
    }
    Ok(bytes)
}

/// The command output before wire fields become validated domain edges.
#[derive(Deserialize)]
struct HierarchyReport {
    /// Version of the command contract, independent of the probe schema.
    schema: u32,
    /// Each edge is decoded once, including roles a newer command may introduce.
    edges:  Vec<HierarchyWireEdge>,
}

/// Optional wire fields become a named failure at the domain boundary.
#[derive(Deserialize)]
struct HierarchyWireEdge {
    /// Managing Claude session, required for known roles.
    manager: Option<String>,
    /// Local managed Claude session, required for known roles.
    member:  Option<String>,
    /// An absent role fails the answer; a future role discards only its edge.
    role:    Option<HierarchyWireRole>,
    #[serde(default)]
    state:   HierarchyWireReviewerState,
}

/// A producer state is decoded before it can affect placement.
#[derive(Default, Deserialize)]
#[serde(rename_all = "lowercase")]
enum HierarchyWireReviewerState {
    #[default]
    NotPool,
    Busy,
    Idle,
    #[serde(other)]
    Unknown,
}

/// Unknown roles discard only their own edge.
#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum HierarchyWireRole {
    Showrunner,
    Unit,
    Impl,
    Test,
    Review,
    Fix,
    #[serde(other)]
    Unknown,
}

/// Decode a command answer, rejecting a foreign schema or malformed JSON.
pub(super) fn parse(output: &[u8]) -> Result<HierarchyEdges, HierarchyError> {
    let report: HierarchyReport =
        serde_json::from_slice(output).map_err(|_| HierarchyError::OutputUnreadable)?;
    if report.schema != HIERARCHY_SCHEMA {
        return Err(HierarchyError::UnknownSchema(report.schema));
    }
    report
        .edges
        .into_iter()
        .filter(|edge| !matches!(edge.role, Some(HierarchyWireRole::Unknown)))
        .map(|edge| {
            let manager = edge.manager.ok_or(HierarchyError::MalformedEdge)?;
            let member = edge.member.ok_or(HierarchyError::MalformedEdge)?;
            let role = match edge.role {
                Some(HierarchyWireRole::Showrunner) => HierarchyRole::Showrunner,
                Some(HierarchyWireRole::Unit) => HierarchyRole::Unit,
                Some(HierarchyWireRole::Impl) => HierarchyRole::Impl,
                Some(HierarchyWireRole::Test) => HierarchyRole::Test,
                Some(HierarchyWireRole::Review) => HierarchyRole::Review,
                Some(HierarchyWireRole::Fix) => HierarchyRole::Fix,
                None | Some(HierarchyWireRole::Unknown) => {
                    return Err(HierarchyError::MalformedEdge);
                },
            };
            if manager.is_empty() || member.is_empty() {
                return Err(HierarchyError::EmptySessionId);
            }
            Ok(HierarchyEdge {
                reviewer: match edge.state {
                    HierarchyWireReviewerState::Busy => ReviewerState::Unverified,
                    HierarchyWireReviewerState::Idle => ReviewerState::Idle,
                    HierarchyWireReviewerState::NotPool | HierarchyWireReviewerState::Unknown => {
                        ReviewerState::NotPool
                    },
                },
                manager,
                member,
                role,
            })
        })
        .collect::<Result<Vec<_>, _>>()
        .map(HierarchyEdges)
}

/// Whether discovery answered or must preserve the last successful links.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum HierarchyAnswer {
    /// A successful command, or discovery explicitly turned off.
    Answered(HierarchyEdges),
    /// A configured source answered, but none of its identities occur in this scan.
    Unmatched(HierarchyEdges),
    /// Discovery failed; absence of edges is not an authoritative answer.
    Failed {
        reason:    HierarchyError,
        reviewers: Vec<ReviewerOutcome>,
    },
}

/// Run one configured discovery, logging a single line for each failed command.
pub(super) fn read(
    source: &HierarchySource,
    runner: &dyn HierarchyRunner,
) -> Result<HierarchyEdges, HierarchyError> {
    let HierarchySource::Command(command) = source else {
        return Ok(HierarchyEdges::default());
    };
    match runner.run(command, HIERARCHY_TIMEOUT) {
        Ok(edges) => Ok(edges),
        Err(reason) => {
            startup_log::startup_event(&format!("hierarchy_failed {reason}"));
            Err(reason)
        },
    }
}

/// The last success for one source, retained across unsuccessful attempts.
#[derive(Default)]
pub(super) enum LastSuccessfulHierarchy {
    /// This source has never answered successfully.
    #[default]
    Unanswered,
    /// The last authoritative links and the time they were obtained.
    Answered {
        /// When discovery successfully answered.
        refreshed: Instant,
        /// Links from that successful answer.
        edges:     HierarchyEdges,
    },
}

impl LastSuccessfulHierarchy {
    /// Links usable while the next attempt is running or has failed.
    pub(super) fn edges(&self) -> HierarchyEdges {
        match self {
            Self::Unanswered => HierarchyEdges::default(),
            Self::Answered { edges, .. } => edges.clone(),
        }
    }
}

/// Whether the next scheduler tick must retry discovery.
#[derive(Default)]
enum HierarchyRetry {
    /// A successful answer may be reused until its normal expiration.
    #[default]
    OnExpiration,
    /// The latest attempt failed; retry on the next tick.
    NextTick,
}

/// The source directory used to validate reviewer leases.
#[derive(Default)]
enum ReviewerPoolRoot {
    #[default]
    UserState,
    #[cfg(test)]
    Fixture(PathBuf),
}

impl ReviewerPoolRoot {
    fn path(&self) -> ReviewerPoolLocation {
        match self {
            Self::UserState => ReviewerPools::root(),
            #[cfg(test)]
            Self::Fixture(path) => ReviewerPoolLocation::Directory(path.clone()),
        }
    }
}

/// Unnamed sessions that already caused their extra hierarchy check.
#[derive(Default)]
struct CheckedUnlinkedSessions(BTreeSet<String>);

/// The first outcome establishes the set of sessions eligible for an extra check.
#[derive(Default)]
enum HierarchyDiscovery {
    #[default]
    Pending,
    Answered(CheckedUnlinkedSessions),
}

/// Rows stay behind the scanning frame until the source has its first outcome.
pub(super) enum HierarchyPublication {
    Pending,
    RefreshNeeded,
    Ready(HierarchySnapshot),
}

/// Placement identities that one refresh must resolve, with its retained links.
pub(super) struct HierarchyRefreshScope {
    /// Only unchecked, unnamed identities require this request's answer.
    waiting:  BTreeSet<String>,
    /// Later arrivals must wait for a catch-up instead of using an older answer.
    observed: BTreeSet<String>,
    /// Keep linked rows together even if only one identity needs an answer.
    links:    HierarchyEdges,
}

impl HierarchyRefreshScope {
    fn new(rows: &[AgentRow], checked: &CheckedUnlinkedSessions, links: HierarchyEdges) -> Self {
        let mut settled = checked.0.clone();
        for edge in &links.0 {
            settled.insert(edge.manager.clone());
            settled.insert(edge.member.clone());
        }
        let waiting = rows
            .iter()
            .flat_map(Self::placement_ids)
            .filter(|id| !settled.contains(*id))
            .map(str::to_owned)
            .collect();
        let mut observed = settled;
        observed.extend(rows.iter().flat_map(Self::placement_ids).map(str::to_owned));
        Self {
            waiting,
            observed,
            links,
        }
    }

    /// A row carries one session and any threads whose placement follows it.
    pub(super) fn placement_ids(row: &AgentRow) -> impl Iterator<Item = &str> {
        let session = match &row.session_id {
            SessionIdentity::Claude(id) => Some(id.as_str()),
            SessionIdentity::Unrecorded => None,
        };
        session.into_iter().chain(
            row.children
                .iter()
                .filter_map(|child| match &child.thread_id {
                    CodexThreadId::Recorded(id) => Some(id.as_str()),
                    CodexThreadId::Unrecorded => None,
                }),
        )
    }

    /// Frozen eligibility keeps checked sessions visible across later scan stages.
    pub(super) fn waits_for(&self, id: &str) -> bool {
        self.waiting.contains(id) || !self.observed.contains(id)
    }

    /// Retained links define the components that must publish together.
    pub(super) const fn links(&self) -> &HierarchyEdges { &self.links }
}

/// The command role determines whether checked session ids are initialized or retained.
pub(super) enum HierarchyRequestKind {
    Initial,
    Refresh(HierarchyRefreshScope),
}

/// Prepared cache work never runs a command while its owner is locked.
pub(super) enum HierarchyPreparation {
    Request(HierarchyRequestKind),
    Ready(HierarchySnapshot),
}

/// Edges reused until their source, local sessions, or freshness changes.
#[derive(Default)]
pub(super) struct HierarchyCache {
    /// Source to which the retained success belongs.
    source:    HierarchySource,
    /// First-outcome readiness and unnamed sessions already checked.
    discovery: HierarchyDiscovery,
    /// Last successful answer from this source, never a failed empty answer.
    success:   LastSuccessfulHierarchy,
    /// Failure invalidates the ordinary refresh interval.
    retry:     HierarchyRetry,
    notice:    HierarchyNotice,
    pools:     ReviewerPools,
    pool_key:  Vec<(PathBuf, SystemTime)>,
    pool_root: ReviewerPoolRoot,
}

impl HierarchyCache {
    #[cfg(test)]
    pub(super) fn with_pool_root(root: PathBuf) -> Self {
        Self {
            pool_root: ReviewerPoolRoot::Fixture(root),
            ..Self::default()
        }
    }

    /// Cached links, cleared immediately when the setting changes or turns off.
    pub(super) fn cached(&mut self, source: HierarchySource) -> &LastSuccessfulHierarchy {
        if self.source != source || matches!(source, HierarchySource::Off) {
            self.source = source;
            self.success = LastSuccessfulHierarchy::Unanswered;
            self.retry = HierarchyRetry::OnExpiration;
            self.discovery = HierarchyDiscovery::Pending;
            self.notice = HierarchyNotice::Clear;
        }
        &self.success
    }

    /// Sequential cache fixtures use the same preparation and acceptance path as workers.
    #[cfg(test)]
    pub(super) fn edges(
        &mut self,
        source: HierarchySource,
        rows: &[AgentRow],
        runner: &dyn HierarchyRunner,
        now: Instant,
    ) -> HierarchyEdges {
        self.snapshot(source, rows, runner, now).edges
    }

    /// New ids may cause one extra attempt; process changes do not create new ids.
    fn unchecked_sessions(&self, rows: &[AgentRow]) -> BTreeSet<String> {
        let HierarchyDiscovery::Answered(checked) = &self.discovery else {
            return BTreeSet::new();
        };
        let edges = self.success.edges();
        rows.iter()
            .filter_map(|row| match &row.session_id {
                SessionIdentity::Claude(id)
                    if !checked.0.contains(id)
                        && !edges
                            .0
                            .iter()
                            .any(|edge| &edge.manager == id || &edge.member == id) =>
                {
                    Some(id.clone())
                },
                _ => None,
            })
            .collect()
    }

    fn accept(&mut self, outcome: Result<HierarchyEdges, HierarchyError>, now: Instant) {
        match outcome {
            Ok(edges) => {
                self.notice = HierarchyNotice::Clear;
                self.success = LastSuccessfulHierarchy::Answered {
                    refreshed: now,
                    edges,
                };
                self.retry = HierarchyRetry::OnExpiration;
            },
            Err(reason) => {
                self.retry = HierarchyRetry::NextTick;
                self.notice = HierarchyNotice::Failed(reason);
            },
        }
    }

    /// Decide and reserve a request before its worker starts, so checked ids survive chains.
    pub(super) fn prepare_snapshot(
        &mut self,
        source: HierarchySource,
        rows: &[AgentRow],
        now: Instant,
    ) -> HierarchyPreparation {
        self.cached(source.clone());
        if matches!(source, HierarchySource::Off) {
            return HierarchyPreparation::Ready(self.retained(source));
        }
        let pool_key = self.pool_root.path().update(&mut self.pools);
        if self.pool_key != pool_key {
            self.retry = HierarchyRetry::NextTick;
            self.pool_key = pool_key;
        }
        if matches!(self.discovery, HierarchyDiscovery::Pending) {
            return HierarchyPreparation::Request(HierarchyRequestKind::Initial);
        }
        let unchecked = self.unchecked_sessions(rows);
        if let LastSuccessfulHierarchy::Answered { refreshed, .. } = &self.success
            && unchecked.is_empty()
            && matches!(self.retry, HierarchyRetry::OnExpiration)
            && now.saturating_duration_since(*refreshed) < HIERARCHY_REFRESH_INTERVAL
        {
            let snapshot = self.retained(source).matched(rows);
            self.notice = snapshot.notice;
            return HierarchyPreparation::Ready(snapshot);
        }
        match &mut self.discovery {
            HierarchyDiscovery::Pending => {
                HierarchyPreparation::Request(HierarchyRequestKind::Initial)
            },
            HierarchyDiscovery::Answered(checked) => {
                let scope = HierarchyRefreshScope::new(rows, checked, self.success.edges());
                checked.0.extend(unchecked);
                HierarchyPreparation::Request(HierarchyRequestKind::Refresh(scope))
            },
        }
    }

    /// Accept a follow-up without making previously checked sessions eligible again.
    pub(super) fn refresh_outcome(
        &mut self,
        source: HierarchySource,
        rows: &[AgentRow],
        outcome: Result<HierarchyEdges, HierarchyError>,
        now: Instant,
    ) -> HierarchySnapshot {
        self.cached(source.clone());
        self.pool_key = self.pool_root.path().update(&mut self.pools);
        self.accept(outcome, now);
        let snapshot = self.retained(source).matched(rows);
        self.notice = snapshot.notice;
        snapshot
    }

    /// Accept the bounded first attempt after its worker returns.
    pub(super) fn first_outcome(
        &mut self,
        source: HierarchySource,
        rows: &[AgentRow],
        outcome: Result<HierarchyEdges, HierarchyError>,
        now: Instant,
    ) -> HierarchySnapshot {
        self.cached(source.clone());
        self.pool_key = self.pool_root.path().update(&mut self.pools);
        self.discovery = HierarchyDiscovery::Answered(CheckedUnlinkedSessions::default());
        self.accept(outcome, now);
        let snapshot = self.retained(source).matched(rows);
        self.notice = snapshot.notice;
        snapshot
    }

    /// Sequential fixtures execute a prepared request and accept its distinct outcome.
    #[cfg(test)]
    pub(super) fn snapshot(
        &mut self,
        source: HierarchySource,
        rows: &[AgentRow],
        runner: &dyn HierarchyRunner,
        now: Instant,
    ) -> HierarchySnapshot {
        match self.prepare_snapshot(source.clone(), rows, now) {
            HierarchyPreparation::Ready(snapshot) => snapshot,
            HierarchyPreparation::Request(HierarchyRequestKind::Initial) => {
                let outcome = read(&source, runner);
                self.first_outcome(source, rows, outcome, now)
            },
            HierarchyPreparation::Request(HierarchyRequestKind::Refresh(_)) => {
                let outcome = read(&source, runner);
                self.refresh_outcome(source, rows, outcome, now)
            },
        }
    }

    /// A cached success keeps its original deadline; a failed attempt belongs to this scan.
    pub(super) fn reuse_deadline(&self, attempted: Instant) -> Instant {
        let refreshed = match (&self.success, self.notice) {
            (
                LastSuccessfulHierarchy::Answered { refreshed, .. },
                HierarchyNotice::Clear | HierarchyNotice::NoSessionMatched,
            ) => *refreshed,
            _ => attempted,
        };
        refreshed + HIERARCHY_REFRESH_INTERVAL
    }

    /// Get an outcome before publishing a new source or a new unchecked session.
    pub(super) fn publication(
        &mut self,
        source: HierarchySource,
        rows: &[AgentRow],
    ) -> HierarchyPublication {
        let snapshot = self.retained(source);
        match (&self.source, &self.discovery) {
            (HierarchySource::Command(_), HierarchyDiscovery::Pending) => {
                HierarchyPublication::Pending
            },
            (HierarchySource::Command(_), HierarchyDiscovery::Answered(_))
                if !self.unchecked_sessions(rows).is_empty() =>
            {
                HierarchyPublication::RefreshNeeded
            },
            _ => HierarchyPublication::Ready(snapshot),
        }
    }

    pub(super) fn retained(&mut self, source: HierarchySource) -> HierarchySnapshot {
        let mut edges = self.cached(source).edges();
        self.pool_root.path().update(&mut self.pools);
        self.pools.refresh(&mut edges);
        HierarchySnapshot {
            edges,
            notice: self.notice,
        }
    }
}

#[cfg(test)]
#[expect(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::Mutex;

    use super::*;

    #[test]
    fn reviewer_missing_home_clears_cached_pools_instead_of_using_a_relative_root() {
        let root = tempfile::tempdir().expect("pool root");
        let production = root.path().join("production");
        fs::create_dir(&production).expect("production directory");
        fs::write(
            production.join(REVIEW_POOL_FILE),
            br#"{"templates":{"code":{"thread_id":"template-thread"}}}"#,
        )
        .expect("pool document");
        let mut pools = ReviewerPools::read(root.path());
        assert_eq!(pools.outcomes().len(), 1, "the fixture pool was read");
        assert_eq!(ReviewerPoolLocation::MissingHome.update(&mut pools), []);
        assert_eq!(
            pools.outcomes().len(),
            0,
            "missing home cannot retain a discovered pool"
        );
    }

    #[test]
    fn idle_reviewer_processes_drop_pids_no_pool_claims() {
        let pools = ReviewerPools::default();
        let pid = Pid::from_u32(std::process::id());
        pools
            .processes
            .lock()
            .expect("process table")
            .refresh_processes_specifics(
                ProcessesToUpdate::Some(&[pid]),
                true,
                ProcessRefreshKind::nothing().without_tasks(),
            );
        assert!(
            pools
                .processes
                .lock()
                .expect("process table")
                .process(pid)
                .is_some()
        );
        pools.refresh(&mut HierarchyEdges(Vec::new()));
        assert!(
            pools
                .processes
                .lock()
                .expect("process table")
                .processes()
                .is_empty(),
            "processes outside the current reviewer claims release their entries"
        );
    }

    /// A valid edge plus a role introduced by a future command implementation.
    #[test]
    fn future_roles_do_not_hide_known_edges_and_foreign_schemas_fail() {
        let output = br#"{"schema":1,"edges":[{"manager":"boss","member":"worker","role":"impl"},{"manager":"boss","member":"future","role":"future"}]}"#;
        assert_eq!(
            parse(output),
            Ok(HierarchyEdges(vec![HierarchyEdge {
                reviewer: crate::census::hierarchy::ReviewerState::NotPool,
                manager:  "boss".to_string(),
                member:   "worker".to_string(),
                role:     HierarchyRole::Impl,
            }]))
        );
        assert!(parse(br#"{"schema":2,"edges":[]}"#).is_err());
        assert!(parse(br#"{"schema":1,"edges":[{"role":"impl"}]}"#).is_err());
    }

    /// Keep error categories distinct for failed commands and malformed contracts.
    #[test]
    fn command_and_contract_failures_keep_their_named_reasons() {
        for (output, reason) in [
            (b"not json".as_slice(), HierarchyError::OutputUnreadable),
            (
                br#"{"schema":2,"edges":[]}"#,
                HierarchyError::UnknownSchema(2),
            ),
            (
                br#"{"schema":1,"edges":[{"role":"impl"}]}"#,
                HierarchyError::MalformedEdge,
            ),
            (
                br#"{"schema":1,"edges":[{"manager":"boss","member":"worker"}]}"#,
                HierarchyError::MalformedEdge,
            ),
            (
                br#"{"schema":1,"edges":[{"manager":"","member":"worker","role":"impl"}]}"#,
                HierarchyError::EmptySessionId,
            ),
            (
                br#"{"schema":1,"edges":[{"manager":"boss","member":"","role":"impl"}]}"#,
                HierarchyError::EmptySessionId,
            ),
        ] {
            assert_eq!(parse(output), Err(reason));
        }
    }

    #[test]
    fn command_exit_keeps_its_reason_without_a_shell_startup_deadline() {
        assert_eq!(
            CommandRunner.run("exit 7", Duration::from_secs(60)),
            Err(HierarchyError::ExitedUnsuccessfully)
        );
    }

    #[test]
    fn command_timeout_keeps_its_reason() {
        assert_eq!(
            CommandRunner.run("exec sleep 10", Duration::from_millis(100)),
            Err(HierarchyError::TimedOut)
        );
    }

    /// An endless producer fails at the output limit and its process is reaped promptly.
    #[test]
    fn oversized_output_fails_before_the_deadline_and_reaps_the_command() {
        let temp = tempfile::tempdir().expect("a fixture directory should open");
        let pid_file = temp.path().join("pid");
        let command = format!("echo $$ > '{}'; exec yes", pid_file.display());
        let started = Instant::now();
        assert_eq!(
            CommandRunner.run(&command, Duration::from_millis(500)),
            Err(HierarchyError::OutputTooLarge)
        );
        assert!(started.elapsed() < Duration::from_millis(800));
        let pid = fs::read_to_string(pid_file).expect("the command should record its pid");
        let status = Command::new("kill")
            .args(["-0", pid.trim()])
            .stderr(Stdio::null())
            .status()
            .expect("kill should start");
        assert!(
            !status.success(),
            "the overflowing command must be killed and reaped"
        );
    }

    /// A full-size valid answer is accepted; even one byte more is rejected.
    #[test]
    fn output_limit_accepts_a_complete_answer_at_the_boundary() {
        let mut output =
            br#"{"schema":1,"edges":[{"manager":"boss","member":"worker","role":"review"}]}"#
                .to_vec();
        output.resize(HIERARCHY_OUTPUT_LIMIT, b' ');
        let bytes = read_output(output.as_slice()).expect("an answer at the limit should be read");
        assert_eq!(
            parse(&bytes),
            Ok(HierarchyEdges(vec![HierarchyEdge {
                reviewer: crate::census::hierarchy::ReviewerState::NotPool,
                manager:  "boss".to_string(),
                member:   "worker".to_string(),
                role:     HierarchyRole::Review,
            }]))
        );
        output.push(b' ');
        assert_eq!(
            read_output(output.as_slice()),
            Err(HierarchyError::OutputTooLarge)
        );
    }

    /// Records refreshes; one setting deliberately fails after a previous success.
    struct RecordingRunner(Mutex<Vec<String>>);

    impl HierarchyRunner for RecordingRunner {
        fn run(&self, command: &str, _: Duration) -> Result<HierarchyEdges, HierarchyError> {
            self.0
                .lock()
                .expect("calls should lock")
                .push(command.to_string());
            if command == "fail" {
                return Err(HierarchyError::ExitedUnsuccessfully);
            }
            Ok(HierarchyEdges(vec![HierarchyEdge {
                reviewer: crate::census::hierarchy::ReviewerState::NotPool,
                manager:  command.to_string(),
                member:   "worker".to_string(),
                role:     HierarchyRole::Impl,
            }]))
        }
    }

    /// Failures preserve links for an unchanged source and retry on the very next tick.
    #[test]
    fn hierarchy_failures_keep_success_and_retry_next_tick_until_disabled() {
        struct ScriptedRunner(Mutex<VecDeque<Result<HierarchyEdges, HierarchyError>>>);
        impl HierarchyRunner for ScriptedRunner {
            fn run(&self, _: &str, _: Duration) -> Result<HierarchyEdges, HierarchyError> {
                self.0
                    .lock()
                    .expect("answer queue")
                    .pop_front()
                    .expect("unexpected extra discovery")
            }
        }
        let first = HierarchyEdges(vec![HierarchyEdge {
            reviewer: crate::census::hierarchy::ReviewerState::NotPool,
            manager:  "boss".to_string(),
            member:   "worker".to_string(),
            role:     HierarchyRole::Unit,
        }]);
        let replacement = HierarchyEdges(vec![HierarchyEdge {
            reviewer: crate::census::hierarchy::ReviewerState::NotPool,
            manager: "new-boss".to_string(),
            ..first.0[0].clone()
        }]);
        let runner = ScriptedRunner(Mutex::new(std::collections::VecDeque::from([
            Ok(first.clone()),
            Err(HierarchyError::TimedOut),
            Err(HierarchyError::ExitedUnsuccessfully),
            Ok(replacement.clone()),
        ])));
        let source = HierarchySource::Command("unchanged".to_string());
        let mut cache = HierarchyCache::default();
        let start = Instant::now();
        assert_eq!(cache.edges(source.clone(), &[], &runner, start), first);
        let expired = start + HIERARCHY_REFRESH_INTERVAL;
        assert_eq!(cache.edges(source.clone(), &[], &runner, expired), first);
        assert_eq!(
            cache.edges(
                source.clone(),
                &[],
                &runner,
                expired + Duration::from_millis(1)
            ),
            first
        );
        assert_eq!(
            cache.edges(
                source.clone(),
                &[],
                &runner,
                expired + Duration::from_millis(2)
            ),
            replacement
        );
        assert_eq!(
            cache.edges(source, &[], &runner, expired + Duration::from_millis(3)),
            replacement
        );
        assert!(runner.0.lock().expect("answer queue").is_empty());
        assert_eq!(
            cache.edges(HierarchySource::Off, &[], &runner, expired),
            HierarchyEdges::default()
        );
    }

    /// Time, identities and configuration invalidate cached success and failure alike.
    #[test]
    fn hierarchy_cache_expires_and_configuration_changes_clear_previous_success() {
        let runner = RecordingRunner(Mutex::new(Vec::new()));
        let mut cache = HierarchyCache::default();
        let now = Instant::now();
        let source = HierarchySource::Command("good".to_string());
        let initial = cache.edges(source.clone(), &[], &runner, now);
        assert_eq!(
            cache.edges(source.clone(), &[], &runner, now + Duration::from_secs(29)),
            initial
        );
        assert_eq!(runner.0.lock().expect("calls should lock").len(), 1);
        assert_eq!(
            cache.edges(source, &[], &runner, now + HIERARCHY_REFRESH_INTERVAL),
            initial
        );
        assert_eq!(runner.0.lock().expect("calls should lock").len(), 2);
        assert_eq!(
            cache.edges(
                HierarchySource::Command("fail".to_string()),
                &[],
                &runner,
                now
            ),
            HierarchyEdges::default()
        );
        assert_eq!(
            cache.edges(
                HierarchySource::Command("fail".to_string()),
                &[],
                &runner,
                now
            ),
            HierarchyEdges::default()
        );
        assert_eq!(runner.0.lock().expect("calls should lock").len(), 4);
        assert_eq!(
            cache.edges(HierarchySource::Off, &[], &runner, now),
            HierarchyEdges::default()
        );
        assert_eq!(runner.0.lock().expect("calls should lock").len(), 4);
        assert_eq!(
            cache.edges(
                HierarchySource::Command("good".to_string()),
                &[],
                &runner,
                now
            ),
            initial
        );
        assert_eq!(runner.0.lock().expect("calls should lock").len(), 5);
    }

    #[expect(
        clippy::expect_used,
        reason = "fixture rows and scripted answers must exist"
    )]
    mod snapshot {
        //! Hierarchy discovery keeps production sessions placed through transient failures.

        use std::collections::VecDeque;
        use std::sync::Arc;
        use std::sync::Mutex;
        use std::sync::mpsc;

        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        use ratatui::buffer::Buffer;
        use ratatui::layout::Rect;
        use tui_pane::TerminalApp;

        use super::super::*;
        use crate::agent_cell;
        use crate::app::App;
        use crate::census::AccountColumn;
        use crate::census::AgentPlacement;
        use crate::census::Census;
        use crate::census::CensusUpdate;
        use crate::census::HierarchyCommand;
        use crate::census::Machine;
        use crate::census::MachineState;
        use crate::census::scan::ScanStage;
        use crate::census::schedule;
        use crate::footer_colors::FooterColors;

        const SHOWRUNNER: &str = "4dbdb78e-a83b-4248-b8f9-ff278dcbc60c";
        const UNIT: &str = "fc0ab8f8-54e4-4c8e-9eed-89b6be9766d0";

        fn production_rows() -> Vec<AgentRow> {
            [
                (3_208_779, SHOWRUNNER, "production-showrunner"),
                (2_778_913, UNIT, "cargo-handler-unit"),
            ]
            .map(|(pid, session, name)| {
                serde_json::from_value(serde_json::json!({
                    "agent":"claude", "session_id":session, "name":name,
                    "started":100,"pid":pid,"directory":"/fixture", "children":[]
                }))
                .expect("session ids from the live natedev scan")
            })
            .into()
        }

        fn assert_unit_draws_under_showrunner(rows: Vec<AgentRow>, edges: HierarchyEdges) {
            let state = MachineState::Answered(rows, edges.into());
            let machines = [Machine {
                name:  "natedev",
                state: &state,
            }];
            let placement = AgentPlacement::from_machines(&machines);
            assert_eq!(
                placement.roots().len(),
                1,
                "the unit is not an unassigned root"
            );
            let cells = agent_cell::cell_order_placed(&placement);
            let area = Rect::new(0, 0, 180, 10);
            let mut buffer = Buffer::empty(area);
            agent_cell::draw(
                &mut buffer,
                area,
                &cells[0],
                &cells,
                200,
                AccountColumn::Hidden,
                &FooterColors::default(),
            );
            let lines: Vec<String> = (0..area.height)
                .map(|y| (0..area.width).map(|x| buffer[(x, y)].symbol()).collect())
                .collect();
            let member = lines
                .iter()
                .find(|line| line.contains("cargo-handler-unit"))
                .expect("the managed unit remains visible in its showrunner tile");
            assert!(
                member
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .windows(2)
                    .any(|pair| pair == ["cargo-handler-unit", "unit"]),
                "{member}"
            );
            assert!(
                member.contains('└'),
                "the unit has a managed tree rail: {member}"
            );
        }

        #[test]
        fn hierarchy_live_session_ids_keep_the_unit_nested_across_failure_and_recovery() {
            struct Attempts(Mutex<VecDeque<Result<HierarchyEdges, HierarchyError>>>);
            impl HierarchyRunner for Attempts {
                fn run(&self, _: &str, _: Duration) -> Result<HierarchyEdges, HierarchyError> {
                    self.0
                        .lock()
                        .expect("attempt queue")
                        .pop_front()
                        .expect("next attempt")
                }
            }
            let edges = parse(
                &serde_json::to_vec(&serde_json::json!({
                    "schema":1,"edges":[{"manager":SHOWRUNNER,"member":UNIT,"role":"unit"}]
                }))
                .expect("production edge encodes"),
            )
            .expect("production edge parses");
            let runner = Attempts(Mutex::new(VecDeque::from([
                Ok(edges.clone()),
                Err(HierarchyError::TimedOut),
                Ok(edges),
            ])));
            let mut cache = HierarchyCache::default();
            let source = HierarchySource::Command("fixture production command".into());
            let now = Instant::now();
            for (moment, expected) in [
                (now, HierarchyNotice::Clear),
                (
                    now + HIERARCHY_REFRESH_INTERVAL,
                    HierarchyNotice::Failed(HierarchyError::TimedOut),
                ),
                (
                    now + HIERARCHY_REFRESH_INTERVAL + Duration::from_millis(1),
                    HierarchyNotice::Clear,
                ),
            ] {
                let rows = production_rows();
                let discovered = cache.snapshot(source.clone(), &rows, &runner, moment);
                assert_eq!(discovered.notice, expected);
                assert_unit_draws_under_showrunner(rows, discovered.edges);
            }
            assert!(runner.0.lock().expect("attempt queue").is_empty());
        }

        #[test]
        fn hierarchy_scheduler_publishes_failure_and_recovery_without_losing_managed_rows() {
            struct PublicationAttempts(Mutex<VecDeque<Result<HierarchyEdges, HierarchyError>>>);
            impl HierarchyRunner for PublicationAttempts {
                fn run(&self, _: &str, _: Duration) -> Result<HierarchyEdges, HierarchyError> {
                    self.0
                        .lock()
                        .expect("publication queue")
                        .pop_front()
                        .expect("next publication")
                }
            }
            let links = HierarchyEdges(vec![HierarchyEdge {
                reviewer: crate::census::hierarchy::ReviewerState::NotPool,
                manager:  SHOWRUNNER.into(),
                member:   UNIT.into(),
                role:     HierarchyRole::Unit,
            }]);
            let runner = Arc::new(PublicationAttempts(Mutex::new(VecDeque::from([
                Ok(links.clone()),
                Err(HierarchyError::TimedOut),
                Ok(links),
            ]))));
            let updates = schedule::spawn_with_test_source(
                HierarchyCommand::new(HierarchySource::Command("fixture hierarchy".into())),
                runner,
                |sender| {
                    let now = Instant::now();
                    sender
                        .publish_at(ScanStage::Completed, production_rows(), now)
                        .expect("successful publication");
                    sender
                        .settle_test_request()
                        .expect("accept fixture hierarchy reply");
                    let mut changed = production_rows();
                    let mut additional = changed[0].clone();
                    additional.pid = 99;
                    additional.session_id =
                        SessionIdentity::Claude("new-session-triggers-discovery".into());
                    changed.push(additional);
                    sender
                        .publish_at(
                            ScanStage::Completed,
                            changed.clone(),
                            now + Duration::from_secs(2),
                        )
                        .expect("failed discovery publication");
                    sender
                        .settle_test_request()
                        .expect("accept fixture hierarchy reply");
                    sender
                        .publish_at(ScanStage::Completed, changed, now + Duration::from_secs(4))
                        .expect("recovered publication");
                    sender
                        .settle_test_request()
                        .expect("accept fixture hierarchy reply");
                },
            );
            let mut completions = updates.into_iter().filter(|update| {
                matches!(
                    update,
                    CensusUpdate::Local {
                        stage: ScanStage::Completed,
                        ..
                    }
                )
            });
            let mut census = Census::new("natedev".into());
            for expected in [
                HierarchyNotice::Clear,
                HierarchyNotice::Failed(HierarchyError::TimedOut),
                HierarchyNotice::Clear,
            ] {
                let update = completions
                    .next()
                    .expect("background scheduler completes each scan");
                census.apply(update);
                let machines = census.machines(&[]);
                assert_eq!(machines[0].state.hierarchy_notice(), expected);
                let placement = AgentPlacement::from_machines(&machines);
                assert!(
                    !placement.is_root("natedev", 2_778_913),
                    "failure must not unassign the unit"
                );
                assert_eq!(machines[0].state.edges().len(), 1);
            }
        }

        #[test]
        fn hierarchy_configured_unmatched_and_disabled_sources_publish_distinct_outcomes() {
            struct ForeignSessions;
            impl HierarchyRunner for ForeignSessions {
                fn run(&self, _: &str, _: Duration) -> Result<HierarchyEdges, HierarchyError> {
                    Ok(HierarchyEdges(vec![HierarchyEdge {
                        reviewer: crate::census::hierarchy::ReviewerState::NotPool,
                        manager:  "another-production".into(),
                        member:   "another-session".into(),
                        role:     HierarchyRole::Unit,
                    }]))
                }
            }
            let mut cache = HierarchyCache::default();
            let rows = production_rows();
            let now = Instant::now();
            let unmatched = cache.snapshot(
                HierarchySource::Command("fixture command".into()),
                &rows,
                &ForeignSessions,
                now,
            );
            assert_eq!(unmatched.notice, HierarchyNotice::NoSessionMatched);
            assert_eq!(
                unmatched.edges.0.len(),
                1,
                "unmatched does not discard a valid command answer"
            );
            let disabled = cache.snapshot(HierarchySource::Off, &rows, &ForeignSessions, now);
            assert_eq!(disabled.notice, HierarchyNotice::Clear);
            assert!(
                disabled.edges.0.is_empty(),
                "disabling discovery clears prior links"
            );
        }

        #[test]
        fn hierarchy_classified_sessions_draw_roles_before_slow_details_complete() {
            struct ProductionTree;
            impl HierarchyRunner for ProductionTree {
                fn run(&self, _: &str, _: Duration) -> Result<HierarchyEdges, HierarchyError> {
                    parse(include_bytes!("fixtures/hierarchy_startup.json"))
                }
            }
            let (release_details, details_released) = mpsc::channel();
            let (completed, completion) = mpsc::channel();
            let updates = schedule::spawn_with_test_source(
                HierarchyCommand::new(HierarchySource::Command("fixture production tree".into())),
                Arc::new(ProductionTree),
                move |sender| {
                    let rows = production_rows();
                    sender
                        .publish(ScanStage::Classified, rows.clone())
                        .expect("publish classified sessions");
                    details_released
                        .recv()
                        .expect("hold slow detail reads until roles arrive");
                    sender
                        .publish(ScanStage::Completed, rows)
                        .expect("publish completed details");
                    completed.send(()).expect("record detail completion");
                },
            );
            let classified = updates.recv_timeout(Duration::from_millis(200));
            // `release_details.send(())` unblocks `details_released.recv()` before
            // assertions, even when the update receive operations timed out.
            release_details.send(()).expect("release detail reads");
            completion
                .recv_timeout(Duration::from_millis(200))
                .expect("the source finishes promptly");
            let mut census = Census::new("natedev".into());
            census.apply(classified.expect("classified roles reach the frame before details"));
            let machines = census.machines(&[]);
            assert_eq!(machines[0].state.rows().len(), 2);
            let machines = census.machines(&[]);
            assert_eq!(machines[0].state.hierarchy_notice(), HierarchyNotice::Clear);
            assert_unit_draws_under_showrunner(
                machines[0].state.rows().to_vec(),
                HierarchyEdges(machines[0].state.edges().to_vec()),
            );
        }

        #[test]
        fn unmatched_hierarchy_note_stays_drawn_while_expired_links_await_refresh() {
            struct ForeignHierarchy;
            impl HierarchyRunner for ForeignHierarchy {
                fn run(&self, _: &str, _: Duration) -> Result<HierarchyEdges, HierarchyError> {
                    Ok(HierarchyEdges(vec![HierarchyEdge {
                        reviewer: crate::census::hierarchy::ReviewerState::NotPool,
                        manager:  "foreign-manager".into(),
                        member:   "foreign-unit".into(),
                        role:     HierarchyRole::Unit,
                    }]))
                }
            }
            let mut cache = HierarchyCache::default();
            let source = HierarchySource::Command("fixture foreign hierarchy".into());
            cache.snapshot(
                source.clone(),
                &production_rows(),
                &ForeignHierarchy,
                Instant::now()
                    .checked_sub(HIERARCHY_REFRESH_INTERVAL)
                    .expect("the fixture clock supports an expired hierarchy"),
            );
            // Simulate the classified publication before a slow refresh by reading
            // `HierarchyCache::retained` with its expired links and notice.
            let retained = cache.retained(source);
            let mut app = App::new_for_test().expect("test app");
            app.census = Census::new("natedev".into());
            app.census.apply(CensusUpdate::Local {
                rows:      production_rows(),
                hierarchy: retained,
                stage:     ScanStage::Classified,
            });
            app.before_draw();
            let keymap = app.keymap();
            let mut terminal = Terminal::new(TestBackend::new(180, 40)).expect("test terminal");
            terminal
                .draw(|frame| app.draw(frame, &keymap))
                .expect("draw retained notice");
            let buffer = terminal.backend().buffer();
            let output = (0..buffer.area.height)
                .map(|y| {
                    (0..buffer.area.width)
                        .map(|x| buffer[(x, y)].symbol())
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join("\n");
            assert!(output.contains("hierarchy: no session matched"), "{output}");
            assert_eq!(
                cache.retained(HierarchySource::Off).notice,
                HierarchyNotice::Clear,
                "a replacement source clears the old outcome"
            );
        }
    }
}
