//! Bounded hierarchy commands and their per-session-set cache, run off the UI thread.

#[path = "command_inputs.rs"]
mod command_inputs;

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::error::Error;
use std::fmt::Display;
use std::fmt::Formatter;
#[cfg(test)]
use std::fs;
use std::io::Read;
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::process::Child;
use std::process::Command;
use std::process::Stdio;
use std::sync::Mutex;
use std::sync::mpsc;
use std::sync::mpsc::RecvTimeoutError;
use std::thread;
use std::time::Duration;
use std::time::Instant;

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
use super::SessionIdentity;
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
            Self::UnknownSchema(schema) => write!(
                formatter,
                "staff schema {schema}; set hierarchy_command to staff.py show --json --live (schema 3)"
            ),
            Self::MalformedEdge => formatter
                .write_str("malformed staff list; hierarchy_command must return staff schema 3"),
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

/// Schema 3 staff members, before their links are validated together.
#[derive(Deserialize)]
struct StaffReport {
    members: Vec<StaffMember>,
}

/// Parent membership is distinct from a root in the staff list.
#[derive(Deserialize)]
#[serde(untagged)]
enum StaffParent {
    Managed(String),
    Root(()),
}

/// A live staff identity belongs to one session family.
#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum StaffIdentity {
    Claude(String),
    Codex(String),
}

impl StaffIdentity {
    fn session(&self) -> &str {
        match self {
            Self::Claude(id) | Self::Codex(id) => id,
        }
    }
}

/// The member id joins parent links; its identity locates the live process.
#[derive(Deserialize)]
struct StaffMember {
    id:       String,
    parent:   StaffParent,
    identity: StaffIdentity,
    role:     StaffRole,
}

/// Roles known to the hierarchy; future staff roles keep their own identity.
#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum StaffRole {
    Showrunner,
    Unit,
    Impl,
    Test,
    #[serde(alias = "astra-reviewer")]
    Review,
    Fix,
    #[serde(rename = "astra-designer")]
    Designer,
    #[serde(other)]
    Unknown,
}

enum StaffLinkRole {
    Known(HierarchyRole),
    Future,
}

impl StaffRole {
    const fn hierarchy_role(&self) -> StaffLinkRole {
        match self {
            Self::Showrunner => StaffLinkRole::Known(HierarchyRole::Showrunner),
            Self::Unit => StaffLinkRole::Known(HierarchyRole::Unit),
            Self::Impl | Self::Designer => StaffLinkRole::Known(HierarchyRole::Impl),
            Self::Test => StaffLinkRole::Known(HierarchyRole::Test),
            Self::Review => StaffLinkRole::Known(HierarchyRole::Review),
            Self::Fix => StaffLinkRole::Known(HierarchyRole::Fix),
            Self::Unknown => StaffLinkRole::Future,
        }
    }
}

/// Decode one complete staff list; invalid links never publish a partial answer.
pub(super) fn parse(output: &[u8]) -> Result<HierarchyEdges, HierarchyError> {
    #[derive(Deserialize)]
    struct SchemaHeader {
        schema: u32,
    }
    let header: SchemaHeader =
        serde_json::from_slice(output).map_err(|_| HierarchyError::OutputUnreadable)?;
    if header.schema != HIERARCHY_SCHEMA {
        return Err(HierarchyError::UnknownSchema(header.schema));
    }
    let report: StaffReport =
        serde_json::from_slice(output).map_err(|_| HierarchyError::MalformedEdge)?;
    let mut members = BTreeMap::new();
    let mut identities = BTreeSet::new();
    for member in &report.members {
        if member.id.is_empty() || member.identity.session().is_empty() {
            return Err(HierarchyError::EmptySessionId);
        }
        if members.insert(member.id.as_str(), member).is_some()
            || !identities.insert(member.identity.session())
        {
            return Err(HierarchyError::MalformedEdge);
        }
    }
    let mut edges = Vec::new();
    for member in &report.members {
        let StaffParent::Managed(parent) = &member.parent else {
            continue;
        };
        let manager = members
            .get(parent.as_str())
            .ok_or(HierarchyError::MalformedEdge)?;
        if !matches!(manager.identity, StaffIdentity::Claude(_)) || parent == &member.id {
            return Err(HierarchyError::MalformedEdge);
        }
        let mut ancestors = BTreeSet::from([member.id.as_str()]);
        let mut ancestor = manager;
        loop {
            if !ancestors.insert(ancestor.id.as_str()) {
                return Err(HierarchyError::MalformedEdge);
            }
            let StaffParent::Managed(parent) = &ancestor.parent else {
                break;
            };
            ancestor = members
                .get(parent.as_str())
                .ok_or(HierarchyError::MalformedEdge)?;
        }
        let StaffLinkRole::Known(role) = member.role.hierarchy_role() else {
            continue;
        };
        edges.push(HierarchyEdge {
            manager: manager.identity.session().to_owned(),
            member: member.identity.session().to_owned(),
            role,
        });
    }
    Ok(HierarchyEdges(edges))
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
}

impl HierarchyCache {
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

    /// A valid edge plus a role introduced by a future command implementation.
    #[test]
    fn future_roles_do_not_hide_known_edges_and_foreign_schemas_fail() {
        let output = br#"{"schema":3,"inputs":[],"members":[{"id":"s0","name":"boss","kind":"showrunner","parent":null,"identity":{"claude":"boss"},"role":"showrunner"},{"id":"s1","name":"worker","kind":"worker","parent":"s0","identity":{"codex":"worker"},"role":"impl"},{"id":"s2","name":"future","kind":"worker","parent":"s0","identity":{"claude":"future"},"role":"future"}]}"#;
        assert_eq!(
            parse(output),
            Ok(HierarchyEdges(vec![HierarchyEdge {
                manager: "boss".to_string(),
                member:  "worker".to_string(),
                role:    HierarchyRole::Impl,
            }]))
        );
        assert!(parse(br#"{"schema":2,"edges":[]}"#).is_err());
        assert!(parse(br#"{"schema":3,"members":[{"id":"broken","role":"impl"}]}"#).is_err());
    }

    #[test]
    fn staff_parent_ids_resolve_claude_managers_and_codex_workers() {
        let edges =
            parse(include_bytes!("fixtures/staff_members.json")).expect("schema 3 staff list");
        assert_eq!(
            edges,
            HierarchyEdges(vec![
                HierarchyEdge {
                    manager: "boss-session".into(),
                    member:  "unit-session".into(),
                    role:    HierarchyRole::Unit,
                },
                HierarchyEdge {
                    manager: "unit-session".into(),
                    member:  "worker-thread".into(),
                    role:    HierarchyRole::Review,
                },
            ])
        );
        assert_eq!(
            parse(br#"{"schema":3,"inputs":[],"members":[]}"#),
            Ok(HierarchyEdges::default())
        );
        assert_eq!(
            parse(br#"{"schema":1,"edges":[]}"#),
            Err(HierarchyError::UnknownSchema(1))
        );
    }

    #[test]
    fn malformed_staff_never_returns_the_valid_part_of_its_links() {
        let valid: Value = serde_json::from_slice(include_bytes!("fixtures/staff_members.json"))
            .expect("staff fixture");
        for field in ["id", "parent", "identity", "role"] {
            let mut report = valid.clone();
            report["members"][2]
                .as_object_mut()
                .expect("worker")
                .remove(field);
            assert!(
                parse(&serde_json::to_vec(&report).expect("JSON")).is_err(),
                "missing {field}"
            );
        }
        for invalid in ["missing-parent", "worker-staff-id"] {
            let mut report = valid.clone();
            report["members"][2]["parent"] = invalid.into();
            assert_eq!(
                parse(&serde_json::to_vec(&report).expect("JSON")),
                Err(HierarchyError::MalformedEdge)
            );
        }
        let mut cycle = valid.clone();
        cycle["members"][0]["parent"] = "unit-staff-id".into();
        assert_eq!(
            parse(&serde_json::to_vec(&cycle).expect("JSON")),
            Err(HierarchyError::MalformedEdge)
        );
        let mut duplicate = valid.clone();
        duplicate["members"]
            .as_array_mut()
            .expect("members")
            .push(valid["members"][0].clone());
        assert_eq!(
            parse(&serde_json::to_vec(&duplicate).expect("JSON")),
            Err(HierarchyError::MalformedEdge)
        );
        let mut wrong_manager = valid;
        wrong_manager["members"][0]["identity"] = serde_json::json!({"codex":"boss-session"});
        assert_eq!(
            parse(&serde_json::to_vec(&wrong_manager).expect("JSON")),
            Err(HierarchyError::MalformedEdge)
        );
    }

    #[test]
    fn staff_command_discards_stderr_and_reads_only_the_staff_answer() {
        let output = include_str!("fixtures/staff_members.json");
        let command =
            format!("printf '%s' 'not part of the staff list' >&2; printf '%s' '{output}'");
        assert_eq!(
            CommandRunner.run(&command, Duration::from_secs(1)),
            parse(output.as_bytes())
        );
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
                br#"{"schema":3,"members":[{"id":"broken","role":"impl"}]}"#,
                HierarchyError::MalformedEdge,
            ),
            (
                br#"{"schema":3,"inputs":[],"members":[{"id":"s0","name":"boss","kind":"showrunner","parent":null,"identity":{"claude":"boss"},"role":"showrunner"},{"id":"s1","name":"worker","kind":"worker","parent":"s0","identity":{"claude":"worker"}}]}"#,
                HierarchyError::MalformedEdge,
            ),
            (
                br#"{"schema":3,"inputs":[],"members":[{"id":"s0","name":"","kind":"showrunner","parent":null,"identity":{"claude":""},"role":"showrunner"},{"id":"s1","name":"worker","kind":"worker","parent":"s0","identity":{"codex":"worker"},"role":"impl"}]}"#,
                HierarchyError::EmptySessionId,
            ),
            (
                br#"{"schema":3,"inputs":[],"members":[{"id":"s0","name":"boss","kind":"showrunner","parent":null,"identity":{"claude":"boss"},"role":"showrunner"},{"id":"s1","name":"","kind":"worker","parent":"s0","identity":{"codex":""},"role":"impl"}]}"#,
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
            br#"{"schema":3,"inputs":[],"members":[{"id":"s0","name":"boss","kind":"showrunner","parent":null,"identity":{"claude":"boss"},"role":"showrunner"},{"id":"s1","name":"worker","kind":"worker","parent":"s0","identity":{"codex":"worker"},"role":"review"}]}"#
                .to_vec();
        output.resize(HIERARCHY_OUTPUT_LIMIT, b' ');
        let bytes = read_output(output.as_slice()).expect("an answer at the limit should be read");
        assert_eq!(
            parse(&bytes),
            Ok(HierarchyEdges(vec![HierarchyEdge {
                manager: "boss".to_string(),
                member:  "worker".to_string(),
                role:    HierarchyRole::Review,
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
                member:  "worker".to_string(),
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
                    "schema":3,"members":[{"id":"boss","parent":null,"identity":{"claude":SHOWRUNNER},"role":"showrunner"},{"id":"unit","parent":"boss","identity":{"claude":UNIT},"role":"unit"}]
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
                        manager: "foreign-manager".into(),
                        member:  "foreign-unit".into(),
                        role:    HierarchyRole::Unit,
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
