//! The staff file reader and the hierarchy publication state.

#[path = "command_inputs.rs"]
mod command_inputs;

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::error::Error;
use std::fmt::Display;
use std::fmt::Formatter;
use std::fs;
use std::io::Read;
use std::ops::Deref;
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::Child;
use std::process::Command;
use std::process::Stdio;
use std::sync::Arc;
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

use super::AgentRow;
use super::CodexThreadId;
use super::HierarchyEdge;
use super::HierarchyEdges;
use super::HierarchyRole;
use super::HierarchySource;
use super::RoleFallback;
#[cfg(test)]
use super::SessionIdentity;
use super::worker_dispatch::WorkerDispatch;
use super::worker_dispatch::WorkerIdentity;
use crate::constants::HIERARCHY_OUTPUT_LIMIT;
use crate::constants::HIERARCHY_REFRESH_INTERVAL;
use crate::constants::HIERARCHY_SCHEMA;
use crate::constants::HIERARCHY_TIMEOUT;
#[cfg(unix)]
use crate::constants::KILL_GROUP_OPTIONS;
#[cfg(unix)]
use crate::constants::KILL_PROGRAM;
use crate::constants::PROBE_EXIT_CHECK;
use crate::constants::SHELL_PROGRAM;
use crate::constants::SHELL_SCRIPT_FLAG;
use crate::startup_log;

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

impl HierarchyEdges {
    /// Staff kinds preserve the roles that the census already resolved from mesh rosters.
    pub(super) fn with_worker_roles(mut self, rows: &[AgentRow]) -> Self {
        let mut roles = BTreeMap::new();
        for row in rows {
            if let Some(session) = row.session_id.ids().next()
                && let RoleFallback::Worker(role) = row.role_fallback
            {
                roles.insert(session, role);
            }
            for child in &row.children {
                if let WorkerDispatch::Worker(dispatch) = &child.dispatch
                    && let RoleFallback::Worker(role) = dispatch.fallback()
                {
                    let session = match &dispatch.identity {
                        WorkerIdentity::ClaudeSession(session)
                        | WorkerIdentity::CodexThread(session) => session,
                    };
                    roles.insert(session.as_str(), role);
                }
            }
        }
        for edge in &mut self.0 {
            if edge.role == HierarchyRole::Worker
                && let Some(role) = roles.get(edge.member.as_str())
            {
                edge.role = *role;
            }
        }
        self
    }
}

impl HierarchySnapshot {
    pub(crate) fn matched(mut self, rows: &[AgentRow]) -> Self {
        if self.notice == HierarchyNotice::Clear && !self.edges.0.iter().any(|edge| {
            rows.iter().any(|row| {
                row.session_id.ids().any(|id| id == edge.member || id == edge.manager)
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
            Self::UnknownSchema(schema) => {
                write!(formatter, "staff schema {schema}; expected schema 2")
            },
            Self::MalformedEdge => formatter.write_str("malformed staff file"),
            Self::EmptySessionId => formatter.write_str("empty hierarchy session id"),
        }
    }
}

impl Error for HierarchyError {}

/// A source of hierarchy edges, replaceable by fixtures in scheduler tests.
pub(crate) trait HierarchyRunner: Send + Sync {
    /// Read links for the configured source.
    fn run(&self, source: &str, timeout: Duration) -> Result<HierarchyEdges, HierarchyError>;

    #[cfg(test)]
    fn publication(&self, _source: &HierarchySource) -> StaffPublication {
        StaffPublication::Deferred
    }
}

/// A file answer can publish without an asynchronous discovery request.
#[cfg(test)]
pub(crate) enum StaffPublication {
    Deferred,
    Ready(HierarchySnapshot),
}

/// Classification can run without staff inputs in isolated census fixtures.
#[derive(Debug, Default)]
pub(super) enum StaffDiscovery {
    #[default]
    Disabled,
    File {
        setting: super::StaffFileSetting,
        reader:  StaffFileReader,
    },
}

impl StaffDiscovery {
    pub(super) fn snapshot(&self) -> StaffSnapshot {
        match self {
            Self::Disabled => StaffSnapshot::default(),
            Self::File { setting, reader } => {
                let source = setting.snapshot();
                let roster = reader.roster(&source);
                StaffSnapshot { source, roster }
            },
        }
    }
}

/// One scan uses one source and one parsed revision for names and links.
#[derive(Clone, Debug, Default)]
pub(super) struct StaffSnapshot {
    source:            HierarchySource,
    pub(super) roster: Arc<StaffRoster>,
}

impl StaffSnapshot {
    pub(super) fn publication(&self) -> HierarchySnapshot {
        match self.source {
            HierarchySource::Off => HierarchyEdges::default().into(),
            HierarchySource::File(_) => self.roster.edges.clone().into(),
        }
    }
}

/// One reader shares the parsed staff file with classification and publication.
#[derive(Clone, Debug, Default)]
pub(crate) struct StaffFileReader(Arc<Mutex<StaffFileCache>>);

#[derive(Debug, Default)]
enum StaffFileCache {
    #[default]
    Unread,
    Read {
        path:     PathBuf,
        revision: SystemTime,
        roster:   Arc<StaffRoster>,
    },
}

impl StaffFileReader {
    /// Check the mtime before reading and parsing the file.
    pub(super) fn roster(&self, source: &HierarchySource) -> Arc<StaffRoster> {
        let HierarchySource::File(path) = source else {
            return Arc::default();
        };
        let path = path
            .strip_prefix("~/")
            .and_then(|relative| dirs::home_dir().map(|home| home.join(relative)))
            .unwrap_or_else(|| PathBuf::from(path));
        let mut cache = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Ok(mut file) = fs::File::open(&path) else {
            *cache = StaffFileCache::Unread;
            return Arc::default();
        };
        let Ok(metadata) = file.metadata() else {
            *cache = StaffFileCache::Unread;
            return Arc::default();
        };
        let Ok(revision) = metadata.modified() else {
            *cache = StaffFileCache::Unread;
            return Arc::default();
        };
        if !metadata.is_file() {
            *cache = StaffFileCache::Unread;
            return Arc::default();
        }
        if let StaffFileCache::Read {
            path: previous,
            revision: modified,
            roster,
        } = &*cache
            && previous == &path
            && modified == &revision
        {
            return Arc::clone(roster);
        }
        let mut bytes = Vec::new();
        if file.read_to_end(&mut bytes).is_err() {
            *cache = StaffFileCache::Unread;
            return Arc::default();
        }
        let roster = Arc::new(StaffRoster::parse(&bytes).unwrap_or_default());
        *cache = StaffFileCache::Read {
            path,
            revision,
            roster: Arc::clone(&roster),
        };
        roster
    }
}

impl HierarchyRunner for StaffFileReader {
    fn run(&self, path: &str, _: Duration) -> Result<HierarchyEdges, HierarchyError> {
        Ok(self
            .roster(&HierarchySource::File(path.to_owned()))
            .edges
            .clone())
    }

    #[cfg(test)]
    fn publication(&self, source: &HierarchySource) -> StaffPublication {
        StaffPublication::Ready(self.roster(source).edges.clone().into())
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

/// The staff file owns names and links, while the census owns liveness.
#[derive(Debug, Default)]
pub(super) struct StaffRoster {
    pub(super) edges: HierarchyEdges,
    names:            BTreeMap<String, String>,
    members:          BTreeMap<String, StaffMembership>,
}

/// A checked member retains its current session and its current manager.
#[derive(Debug)]
struct StaffMembership {
    session: String,
    parent:  StaffManager,
}

#[derive(Debug)]
enum StaffManager {
    Root,
    Session(String),
}

/// Stable staff identities and legacy thread identities have separate meanings.
#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub(super) enum WorkerReference {
    Staff { staff_id: String },
    LegacyThread { thread_id: String },
}

impl WorkerReference {
    pub(super) fn session<'a>(&'a self, staff: &'a StaffRoster) -> Option<&'a str> {
        match self {
            Self::Staff { staff_id } => staff
                .members
                .get(staff_id)
                .map(|member| member.session.as_str()),
            Self::LegacyThread { thread_id } => {
                (!thread_id.trim().is_empty()).then_some(thread_id.trim())
            },
        }
    }

    pub(super) fn parent_session<'a>(&self, staff: &'a StaffRoster) -> Option<&'a str> {
        let Self::Staff { staff_id } = self else {
            return None;
        };
        match &staff.members.get(staff_id)?.parent {
            StaffManager::Root => None,
            StaffManager::Session(session) => Some(session),
        }
    }
}

#[derive(Deserialize)]
struct StaffDocument {
    rows: Vec<StaffDocumentRow>,
}

/// The parent field distinguishes a root from a managed row.
#[derive(Deserialize)]
#[serde(untagged)]
enum StaffParent {
    Managed(String),
    Root(()),
}

/// Unknown kinds have no required fields in this reader.
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum StaffDocumentRow {
    Showrunner(StaffFields),
    Unit(StaffFields),
    Worker(StaffFields),
    Agent(StaffFields),
    #[serde(other)]
    Unknown,
}

#[derive(Deserialize)]
struct StaffFields {
    id:              String,
    name:            String,
    parent:          StaffParent,
    current_session: String,
}

struct StaffRow {
    fields: StaffFields,
    kind:   StaffKind,
}

impl Deref for StaffRow {
    type Target = StaffFields;
    fn deref(&self) -> &Self::Target { &self.fields }
}

impl StaffDocumentRow {
    fn known(self) -> Option<StaffRow> {
        let (kind, fields) = match self {
            Self::Showrunner(fields) => (StaffKind::Showrunner, fields),
            Self::Unit(fields) => (StaffKind::Unit, fields),
            Self::Worker(fields) => (StaffKind::Worker, fields),
            Self::Agent(fields) => (StaffKind::Agent, fields),
            Self::Unknown => return None,
        };
        Some(StaffRow { fields, kind })
    }
}

/// IDs and sessions are unique, parent kinds are valid, and parent chains have no cycles.
struct ValidatedStaffForest<'a>(BTreeMap<&'a str, &'a StaffRow>);

impl<'a> ValidatedStaffForest<'a> {
    fn new(rows: &'a [StaffRow]) -> Result<Self, HierarchyError> {
        let members: BTreeMap<_, _> = rows.iter().map(|row| (row.id.as_str(), row)).collect();
        if members.len() != rows.len() {
            return Err(HierarchyError::MalformedEdge);
        }
        let mut sessions = BTreeSet::new();
        for row in members.values() {
            if !sessions.insert(row.current_session.as_str()) {
                return Err(HierarchyError::MalformedEdge);
            }
            match &row.parent {
                StaffParent::Root(())
                    if matches!(row.kind, StaffKind::Showrunner | StaffKind::Agent) => {},
                StaffParent::Managed(parent) => {
                    let manager = members
                        .get(parent.as_str())
                        .ok_or(HierarchyError::MalformedEdge)?;
                    if !row.kind.accepts_parent(manager.kind) {
                        return Err(HierarchyError::MalformedEdge);
                    }
                },
                StaffParent::Root(()) => return Err(HierarchyError::MalformedEdge),
            }
            if row.id.is_empty() || row.current_session.is_empty() {
                return Err(HierarchyError::EmptySessionId);
            }
            let mut ancestors = BTreeSet::from([row.id.as_str()]);
            let mut ancestor = *row;
            while let StaffParent::Managed(parent) = &ancestor.parent {
                if !ancestors.insert(parent.as_str()) {
                    return Err(HierarchyError::MalformedEdge);
                }
                ancestor = members
                    .get(parent.as_str())
                    .ok_or(HierarchyError::MalformedEdge)?;
            }
        }
        Ok(Self(members))
    }
}

#[derive(Clone, Copy)]
enum StaffKind {
    Showrunner,
    Unit,
    Worker,
    Agent,
}

impl StaffKind {
    const fn accepts_parent(self, parent: Self) -> bool {
        matches!(
            (self, parent),
            (
                Self::Showrunner | Self::Unit | Self::Worker,
                Self::Showrunner
            ) | (Self::Worker, Self::Unit)
        )
    }

    const fn role(self) -> HierarchyRole {
        match self {
            Self::Showrunner => HierarchyRole::Showrunner,
            Self::Unit => HierarchyRole::Unit,
            Self::Worker => HierarchyRole::Worker,
            Self::Agent => HierarchyRole::Agent,
        }
    }
}

/// A missing staff name leaves the existing census name unchanged.
pub(super) enum StaffName<'a> {
    Named(&'a str),
    Unlisted,
}

/// `StaffSessionMembership` names whether a session belongs to this staff snapshot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum StaffSessionMembership {
    Listed,
    Unlisted,
}

impl StaffRoster {
    /// `StaffRoster` admits only sessions listed in this scan's staff snapshot.
    pub(super) fn session_membership(&self, id: &str) -> StaffSessionMembership {
        self.members
            .values()
            .find(|member| member.session == id)
            .map_or(StaffSessionMembership::Unlisted, |_| {
                StaffSessionMembership::Listed
            })
    }

    pub(super) fn name(&self, session: &str) -> StaffName<'_> {
        self.names
            .get(session)
            .map_or(StaffName::Unlisted, |name| StaffName::Named(name))
    }

    fn parse(output: &[u8]) -> Result<Self, HierarchyError> {
        #[derive(Deserialize)]
        struct SchemaHeader {
            schema: u32,
        }
        let header: SchemaHeader =
            serde_json::from_slice(output).map_err(|_| HierarchyError::OutputUnreadable)?;
        if header.schema != HIERARCHY_SCHEMA {
            return Err(HierarchyError::UnknownSchema(header.schema));
        }
        let document: StaffDocument =
            serde_json::from_slice(output).map_err(|_| HierarchyError::MalformedEdge)?;
        let known: Vec<_> = document
            .rows
            .into_iter()
            .filter_map(StaffDocumentRow::known)
            .collect();
        let ValidatedStaffForest(members) = ValidatedStaffForest::new(&known)?;
        let mut roster = Self::default();
        for member in members.values() {
            let parent = match &member.parent {
                StaffParent::Root(()) => StaffManager::Root,
                StaffParent::Managed(id) => {
                    StaffManager::Session(members[id.as_str()].current_session.clone())
                },
            };
            roster.members.insert(
                member.id.clone(),
                StaffMembership {
                    session: member.current_session.clone(),
                    parent,
                },
            );
            let role = member.kind.role();
            if !member.name.is_empty() {
                roster
                    .names
                    .insert(member.current_session.clone(), member.name.clone());
            }
            let StaffParent::Managed(parent) = &member.parent else {
                continue;
            };
            let Some(manager) = members.get(parent.as_str()) else {
                continue;
            };
            roster.edges.0.push(HierarchyEdge {
                manager: manager.current_session.clone(),
                member: member.current_session.clone(),
                role,
            });
        }
        Ok(roster)
    }
}

/// Test fixtures use the same parser as the file reader.
#[cfg(test)]
pub(super) fn parse(output: &[u8]) -> Result<HierarchyEdges, HierarchyError> {
    StaffRoster::parse(output).map(|roster| roster.edges)
}

/// Whether discovery answered or must preserve the last successful links.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum HierarchyAnswer {
    /// A successful command, or discovery explicitly turned off.
    Answered(HierarchyEdges),
    /// A configured source answered, but none of its identities occur in this scan.
    Unmatched(HierarchyEdges),
    /// Discovery failed; absence of edges is not an authoritative answer.
    Failed { reason: HierarchyError },
}

/// Run one configured discovery, logging a single line for each failed command.
pub(super) fn read(
    source: &HierarchySource,
    runner: &dyn HierarchyRunner,
) -> Result<HierarchyEdges, HierarchyError> {
    let HierarchySource::File(command) = source else {
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
    Ready,
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
        row.session_id.ids().chain(
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
}

impl HierarchyCache {
    /// Cached links, cleared immediately when the setting changes or turns off.
    pub(super) fn cached(&mut self, source: &HierarchySource) -> &LastSuccessfulHierarchy {
        if &self.source != source || matches!(source, HierarchySource::Off) {
            self.source.clone_from(source);
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
        source: &HierarchySource,
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
            .flat_map(|row| row.session_id.ids())
            .filter(|id| {
                !checked.0.contains(*id)
                    && !edges
                        .0
                        .iter()
                        .any(|edge| edge.manager == *id || edge.member == *id)
            })
            .map(str::to_owned)
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
        source: &HierarchySource,
        rows: &[AgentRow],
        now: Instant,
    ) -> HierarchyPreparation {
        self.cached(source);
        if matches!(source, HierarchySource::Off) {
            return HierarchyPreparation::Ready(self.retained(source));
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
        source: &HierarchySource,
        rows: &[AgentRow],
        outcome: Result<HierarchyEdges, HierarchyError>,
        now: Instant,
    ) -> HierarchySnapshot {
        self.cached(source);
        self.accept(outcome, now);
        let snapshot = self.retained(source).matched(rows);
        self.notice = snapshot.notice;
        snapshot
    }

    /// Accept the bounded first attempt after its worker returns.
    pub(super) fn first_outcome(
        &mut self,
        source: &HierarchySource,
        rows: &[AgentRow],
        outcome: Result<HierarchyEdges, HierarchyError>,
        now: Instant,
    ) -> HierarchySnapshot {
        self.cached(source);
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
        source: &HierarchySource,
        rows: &[AgentRow],
        runner: &dyn HierarchyRunner,
        now: Instant,
    ) -> HierarchySnapshot {
        match self.prepare_snapshot(source, rows, now) {
            HierarchyPreparation::Ready(snapshot) => snapshot,
            HierarchyPreparation::Request(HierarchyRequestKind::Initial) => {
                let outcome = read(source, runner);
                self.first_outcome(source, rows, outcome, now)
            },
            HierarchyPreparation::Request(HierarchyRequestKind::Refresh(_)) => {
                let outcome = read(source, runner);
                self.refresh_outcome(source, rows, outcome, now)
            },
        }
    }

    /// A cached success keeps its original deadline; a failed attempt belongs to this scan.
    #[cfg(test)]
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
        source: &HierarchySource,
        rows: &[AgentRow],
    ) -> HierarchyPublication {
        self.cached(source);
        match (&self.source, &self.discovery) {
            (HierarchySource::File(_), HierarchyDiscovery::Pending) => {
                HierarchyPublication::Pending
            },
            (HierarchySource::File(_), HierarchyDiscovery::Answered(_))
                if !self.unchecked_sessions(rows).is_empty() =>
            {
                HierarchyPublication::RefreshNeeded
            },
            _ => HierarchyPublication::Ready,
        }
    }

    pub(super) fn retained(&mut self, source: &HierarchySource) -> HierarchySnapshot {
        let edges = self.cached(source).edges();
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

    use serde_json::Value;

    use super::*;

    #[test]
    fn malformed_staff_never_returns_the_valid_part_of_its_links() {
        fn publishes_no_links(report: &Value) -> bool {
            let directory = tempfile::tempdir().expect("staff directory");
            let path = directory.path().join("staff.json");
            fs::write(&path, serde_json::to_vec(report).expect("JSON")).expect("staff file");
            let reader = StaffFileReader::default();
            let source = HierarchySource::File(path.to_string_lossy().into_owned());
            matches!(
                reader.publication(&source),
                StaffPublication::Ready(HierarchySnapshot {
                    edges: HierarchyEdges(edges),
                    notice: HierarchyNotice::Clear,
                }) if edges.is_empty()
            ) && reader.roster(&source).names.is_empty()
        }

        let valid: Value = serde_json::from_slice(include_bytes!("fixtures/staff_members.json"))
            .expect("staff fixture");
        for field in ["id", "parent", "current_session", "kind", "name"] {
            let mut report = valid.clone();
            report["rows"][2]
                .as_object_mut()
                .expect("worker")
                .remove(field);
            assert!(publishes_no_links(&report), "missing {field}");
        }
        for invalid in ["missing-parent", "worker-staff-id"] {
            let mut report = valid.clone();
            report["rows"][2]["parent"] = invalid.into();
            assert!(publishes_no_links(&report), "invalid parent {invalid}");
        }
        let mut cycle = valid.clone();
        cycle["rows"][0]["parent"] = "unit-staff-id".into();
        assert!(
            publishes_no_links(&cycle),
            "a cycle must invalidate every link"
        );
        let mut duplicate = valid.clone();
        duplicate["rows"]
            .as_array_mut()
            .expect("rows")
            .push(valid["rows"][0].clone());
        assert!(
            publishes_no_links(&duplicate),
            "duplicate IDs must invalidate every link"
        );
        let mut wrong_manager = valid;
        wrong_manager["rows"][0]["current_session"] = serde_json::json!({"codex":"boss-session"});
        assert!(
            publishes_no_links(&wrong_manager),
            "a malformed manager session must invalidate every link"
        );
    }

    #[test]
    fn command_exit_keeps_its_reason_without_a_shell_startup_deadline() {
        assert_eq!(
            run_output("exit 7", Duration::from_secs(60)),
            Err(HierarchyError::ExitedUnsuccessfully)
        );
    }

    #[test]
    fn command_timeout_keeps_its_reason() {
        assert_eq!(
            run_output("exec sleep 10", Duration::from_millis(100)),
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
            run_output(&command, Duration::from_millis(500)),
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
            br#"{"schema":2,"rows":[{"id":"a","kind":"showrunner","name":"unit","parent":null,"current_session":"unit-session"},{"id":"b","kind":"worker","name":"worker","parent":"a","current_session":"worker-thread"}]}"#
                .to_vec();
        output.resize(HIERARCHY_OUTPUT_LIMIT, b' ');
        let bytes = read_output(output.as_slice()).expect("an answer at the limit should be read");
        assert_eq!(
            parse(&bytes),
            Ok(HierarchyEdges(vec![HierarchyEdge {
                manager: "unit-session".to_string(),
                member:  "worker-thread".to_string(),
                role:    HierarchyRole::Worker,
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
                manager: command.to_string(),
                member:  "worker-thread".to_string(),
                role:    HierarchyRole::Impl,
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
            manager: "boss".to_string(),
            member:  "worker".to_string(),
            role:    HierarchyRole::Unit,
        }]);
        let replacement = HierarchyEdges(vec![HierarchyEdge {
            manager: "new-boss".to_string(),
            ..first.0[0].clone()
        }]);
        let runner = ScriptedRunner(Mutex::new(std::collections::VecDeque::from([
            Ok(first.clone()),
            Err(HierarchyError::TimedOut),
            Err(HierarchyError::ExitedUnsuccessfully),
            Ok(replacement.clone()),
        ])));
        let source = HierarchySource::File("unchanged".to_string());
        let mut cache = HierarchyCache::default();
        let start = Instant::now();
        assert_eq!(cache.edges(&source, &[], &runner, start), first);
        let expired = start + HIERARCHY_REFRESH_INTERVAL;
        assert_eq!(cache.edges(&source, &[], &runner, expired), first);
        assert_eq!(
            cache.edges(&source, &[], &runner, expired + Duration::from_millis(1)),
            first
        );
        assert_eq!(
            cache.edges(&source, &[], &runner, expired + Duration::from_millis(2)),
            replacement
        );
        assert_eq!(
            cache.edges(&source, &[], &runner, expired + Duration::from_millis(3)),
            replacement
        );
        assert!(runner.0.lock().expect("answer queue").is_empty());
        assert_eq!(
            cache.edges(&HierarchySource::Off, &[], &runner, expired),
            HierarchyEdges::default()
        );
    }

    /// Time, identities and configuration invalidate cached success and failure alike.
    #[test]
    fn hierarchy_cache_expires_and_configuration_changes_clear_previous_success() {
        let runner = RecordingRunner(Mutex::new(Vec::new()));
        let mut cache = HierarchyCache::default();
        let now = Instant::now();
        let source = HierarchySource::File("good".to_string());
        let initial = cache.edges(&source, &[], &runner, now);
        assert_eq!(
            cache.edges(&source, &[], &runner, now + Duration::from_secs(29)),
            initial
        );
        assert_eq!(runner.0.lock().expect("calls should lock").len(), 1);
        assert_eq!(
            cache.edges(&source, &[], &runner, now + HIERARCHY_REFRESH_INTERVAL),
            initial
        );
        assert_eq!(runner.0.lock().expect("calls should lock").len(), 2);
        assert_eq!(
            cache.edges(
                &HierarchySource::File("fail".to_string()),
                &[],
                &runner,
                now
            ),
            HierarchyEdges::default()
        );
        assert_eq!(
            cache.edges(
                &HierarchySource::File("fail".to_string()),
                &[],
                &runner,
                now
            ),
            HierarchyEdges::default()
        );
        assert_eq!(runner.0.lock().expect("calls should lock").len(), 4);
        assert_eq!(
            cache.edges(&HierarchySource::Off, &[], &runner, now),
            HierarchyEdges::default()
        );
        assert_eq!(runner.0.lock().expect("calls should lock").len(), 4);
        assert_eq!(
            cache.edges(
                &HierarchySource::File("good".to_string()),
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
        use crate::census::Machine;
        use crate::census::MachineState;
        use crate::census::StaffFileSetting;
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
                    "schema":2,"rows":[{"id":"boss","name":"boss","parent":null,"current_session":SHOWRUNNER,"kind":"showrunner"},{"id":"unit","name":"unit","parent":"boss","current_session":UNIT,"kind":"unit"}]
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
            let source = HierarchySource::File("fixture production command".into());
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
                let discovered = cache.snapshot(&source, &rows, &runner, moment);
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
                manager: SHOWRUNNER.into(),
                member:  UNIT.into(),
                role:    HierarchyRole::Unit,
            }]);
            let runner = Arc::new(PublicationAttempts(Mutex::new(VecDeque::from([
                Ok(links.clone()),
                Err(HierarchyError::TimedOut),
                Ok(links),
            ]))));
            let updates = schedule::spawn_with_test_source(
                StaffFileSetting::new(HierarchySource::File("fixture hierarchy".into())),
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
                        manager: "another-production".into(),
                        member:  "another-session".into(),
                        role:    HierarchyRole::Unit,
                    }]))
                }
            }
            let mut cache = HierarchyCache::default();
            let rows = production_rows();
            let now = Instant::now();
            let unmatched = cache.snapshot(
                &HierarchySource::File("fixture command".into()),
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
            let disabled = cache.snapshot(&HierarchySource::Off, &rows, &ForeignSessions, now);
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
                StaffFileSetting::new(HierarchySource::File("fixture production tree".into())),
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
                        manager: "foreign-manager".into(),
                        member:  "foreign-unit".into(),
                        role:    HierarchyRole::Unit,
                    }]))
                }
            }
            let mut cache = HierarchyCache::default();
            let source = HierarchySource::File("fixture foreign hierarchy".into());
            cache.snapshot(
                &source,
                &production_rows(),
                &ForeignHierarchy,
                Instant::now()
                    .checked_sub(HIERARCHY_REFRESH_INTERVAL)
                    .expect("the fixture clock supports an expired hierarchy"),
            );
            // Simulate the classified publication before a slow refresh by reading
            // `HierarchyCache::retained` with its expired links and notice.
            let retained = cache.retained(&source);
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
                cache.retained(&HierarchySource::Off).notice,
                HierarchyNotice::Clear,
                "a replacement source clears the old outcome"
            );
        }
    }
}

#[cfg(test)]
#[path = "hierarchy_file_tests.rs"]
mod file_tests;
