//! Session states from the accounts command and their independent background refresh.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;
use std::sync::mpsc;
use std::sync::mpsc::Receiver;
use std::sync::mpsc::RecvTimeoutError;
use std::sync::mpsc::Sender;
use std::thread;
use std::time::Duration;
use std::time::Instant;

use serde::Deserialize;

use super::Agent;
use super::AgentRow;
use super::Observed;
use super::SessionIdentity;
use super::SourceHealth;
use super::hierarchy;
use super::hierarchy::HierarchyError;
use crate::constants::ACCOUNTS_REFRESH_INTERVAL;
use crate::constants::HIERARCHY_TIMEOUT;
use crate::theme::Role;

/// The accounts producer classifies each session once.
/// A report from before the four-value status gives `starting` or `waiting` for a busy session.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum SessionState {
    #[serde(alias = "starting", alias = "waiting")]
    Busy,
    Idle,
    Paused,
    Exited,
}

impl SessionState {
    pub(crate) const fn text(self) -> &'static str {
        match self {
            Self::Busy => "busy",
            Self::Idle => "idle",
            Self::Paused => "paused",
            Self::Exited => "exited",
        }
    }
}

/// A missing source entry never implies an idle session.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum ReportedSessionState {
    #[default]
    Unavailable,
    Reported(SessionState),
}

impl ReportedSessionState {
    pub(crate) const fn text(self) -> &'static str {
        match self {
            Self::Unavailable => "",
            Self::Reported(state) => state.text(),
        }
    }

    pub(crate) const fn color_role(self) -> Role {
        match self {
            Self::Reported(SessionState::Busy) => Role::Busy,
            Self::Unavailable | Self::Reported(_) => Role::Idle,
        }
    }
}

/// Source states use the same session identities as the census.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct SessionStates(BTreeMap<String, SessionState>);

pub(crate) static EMPTY_SESSION_STATES: SessionStates = SessionStates(BTreeMap::new());

impl SessionStates {
    pub(crate) fn get(&self, session: &str) -> ReportedSessionState {
        self.0.get(session).copied().map_or(
            ReportedSessionState::Unavailable,
            ReportedSessionState::Reported,
        )
    }

    fn for_session(&self, identity: &super::SessionIdentity) -> ReportedSessionState {
        identity
            .ids()
            .map(|session| self.get(session))
            .find(|state| !matches!(state, ReportedSessionState::Unavailable))
            .unwrap_or_default()
    }

    /// A listed Codex process remains live before its thread is recorded.
    pub(crate) fn for_row(&self, row: &AgentRow) -> ReportedSessionState {
        match (&row.session_id, row.agent) {
            (SessionIdentity::Unrecorded, Agent::Codex) => {
                ReportedSessionState::Reported(SessionState::Idle)
            },
            _ => self.for_session(&row.session_id),
        }
    }

    /// Decode the session states of every account once at the command boundary.
    /// A session that two accounts report in different states has no state.
    pub(crate) fn parse(bytes: &[u8]) -> Result<Self, AccountsFailure> {
        let report: AccountsReport =
            serde_json::from_slice(bytes).map_err(|_| AccountsFailure::InvalidReading)?;
        let mut sessions = BTreeMap::new();
        let mut conflicts = BTreeSet::new();
        for state in report.accounts.iter().flat_map(|account| &account.sessions) {
            if state.session.is_empty() {
                return Err(AccountsFailure::InvalidReading);
            }
            if let Some(previous) = sessions.insert(state.session.clone(), state.state)
                && previous != state.state
            {
                conflicts.insert(state.session.clone());
            }
        }
        sessions.retain(|session, _| !conflicts.contains(session));
        Ok(Self(sessions))
    }
}

/// The accounts command's report; only the session states are read.
#[derive(Deserialize)]
struct AccountsReport {
    accounts: Vec<AccountWire>,
}

#[derive(Deserialize)]
struct AccountWire {
    #[serde(default)]
    sessions: Vec<SessionStateWire>,
}

#[derive(Deserialize)]
struct SessionStateWire {
    session: String,
    state:   SessionState,
}

/// A failed command never replaces usable session states with none.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AccountsFailure {
    Command(HierarchyError),
    InvalidReading,
}

impl Display for AccountsFailure {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Command(error) => write!(formatter, "accounts: {error}"),
            Self::InvalidReading => formatter.write_str("accounts: invalid JSON reading"),
        }
    }
}

/// Exactly one state describes both the usable session states and any failure.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) enum AccountsSource {
    #[default]
    NotConfigured,
    AwaitingFirstReading,
    Ready(SessionStates),
    Failed(AccountsFailure),
    FailedKeepingLast {
        last:    SessionStates,
        failure: AccountsFailure,
    },
}

impl AccountsSource {
    /// The census keeps the monitor's outcome; a failure keeps its reason.
    pub(super) fn health(&self) -> super::Observed<super::SourceHealth> {
        match self {
            Self::NotConfigured => Observed::Known(SourceHealth::Absent),
            Self::AwaitingFirstReading => {
                super::Observed::unknown("the accounts command has not answered")
            },
            Self::Ready(_) => Observed::Known(SourceHealth::Loaded),
            Self::Failed(failure) | Self::FailedKeepingLast { failure, .. } => Observed::Known(
                SourceHealth::Unreadable(super::UnknownReason(failure.to_string())),
            ),
        }
    }

    pub(crate) fn session_states(&self) -> &SessionStates {
        match self {
            Self::Ready(states) | Self::FailedKeepingLast { last: states, .. } => states,
            Self::NotConfigured | Self::AwaitingFirstReading | Self::Failed(_) => {
                &EMPTY_SESSION_STATES
            },
        }
    }

    /// One read of `command` within `limit` for a process with no monitor; an empty command is
    /// not configured.
    pub(super) fn read_once(command: &str, limit: Duration) -> Self {
        if command.trim().is_empty() {
            return Self::NotConfigured;
        }
        let mut source = Self::AwaitingFirstReading;
        source.apply(read_accounts_within(command, limit));
        source
    }

    /// A one-time read that gave no answer in its time.
    pub(super) fn timed_out() -> Self {
        let mut source = Self::AwaitingFirstReading;
        source.apply(Err(AccountsFailure::Command(HierarchyError::TimedOut)));
        source
    }

    /// Preserve the last successful answer across any number of failures.
    fn apply(&mut self, result: Result<SessionStates, AccountsFailure>) -> bool {
        let next = match result {
            Ok(states) => Self::Ready(states),
            Err(failure) => match self {
                Self::Ready(last) | Self::FailedKeepingLast { last, .. } => {
                    Self::FailedKeepingLast {
                        last: last.clone(),
                        failure,
                    }
                },
                Self::NotConfigured | Self::AwaitingFirstReading | Self::Failed(_) => {
                    Self::Failed(failure)
                },
            },
        };
        if *self == next {
            return false;
        }
        *self = next;
        true
    }
}

/// Read and decode the configured accounts command off the frame thread.
fn read_accounts(command: &str) -> Result<SessionStates, AccountsFailure> {
    read_accounts_within(command, HIERARCHY_TIMEOUT)
}

/// Read and decode `command`, which is stopped at `HIERARCHY_TIMEOUT` or `limit`, whichever is
/// sooner.
fn read_accounts_within(command: &str, limit: Duration) -> Result<SessionStates, AccountsFailure> {
    let bytes = hierarchy::run_output(command, limit.min(HIERARCHY_TIMEOUT))
        .map_err(AccountsFailure::Command)?;
    SessionStates::parse(&bytes)
}

#[cfg(test)]
#[path = "accounts_source_tests.rs"]
mod source_tests;

/// An accounts command whose answer is delivered by the refresh thread.
type AccountsCommand = Box<dyn Fn(&str) -> Result<SessionStates, AccountsFailure> + Send>;

/// Only a configured source owns a refresh thread and its shutdown channel.
enum AccountsUpdates {
    Disabled,
    Running {
        readings: Receiver<Result<SessionStates, AccountsFailure>>,
        stop:     Sender<()>,
    },
}

impl Drop for AccountsUpdates {
    fn drop(&mut self) {
        if let Self::Running { stop, .. } = self {
            let _ = stop.send(());
        }
    }
}

/// UI-owned state updated by nonblocking channel reads, never by a frame's command.
pub(crate) struct AccountsMonitor {
    pub(crate) source: AccountsSource,
    updates:           AccountsUpdates,
}

impl AccountsMonitor {
    pub(crate) fn new(command: String) -> Self {
        Self::spawn_with(command, Box::new(read_accounts), ACCOUNTS_REFRESH_INTERVAL)
    }

    /// `AccountsMonitor::spawn_with` runs `AccountsCommand` on a refresh thread
    /// at `cadence` and publishes readings through `AccountsUpdates::Running`.
    /// An empty `command` selects `AccountsUpdates::Disabled` without a thread.
    fn spawn_with(command: String, runner: AccountsCommand, cadence: Duration) -> Self {
        if command.trim().is_empty() {
            return Self {
                source:  AccountsSource::NotConfigured,
                updates: AccountsUpdates::Disabled,
            };
        }
        let (sender, readings) = mpsc::channel();
        let (stop, shutdown) = mpsc::channel();
        thread::spawn(move || {
            let mut inputs = hierarchy::InputDrivenCommand::for_command(&command, cadence);
            loop {
                let answer = inputs.read(Instant::now(), || runner(&command));
                if answer.is_err() {
                    inputs.invalidate();
                }
                if sender.send(answer).is_err() {
                    return;
                }
                if shutdown.recv_timeout(cadence) != Err(RecvTimeoutError::Timeout) {
                    return;
                }
            }
        });
        Self {
            source:  AccountsSource::AwaitingFirstReading,
            updates: AccountsUpdates::Running { readings, stop },
        }
    }

    /// Drain every completed refresh; the next frame sees the latest result.
    pub(crate) fn refresh(&mut self) -> bool {
        let mut changed = false;
        if let AccountsUpdates::Running { readings, .. } = &self.updates {
            while let Ok(result) = readings.try_recv() {
                changed |= self.source.apply(result);
            }
        }
        changed
    }
}
