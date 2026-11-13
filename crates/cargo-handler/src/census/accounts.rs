//! Stored account readings and their independent background refresh.

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

use chrono::DateTime;
use chrono::Utc;
use serde::Deserialize;
use serde::Deserializer;
use serde::de::Error;

use super::Agent;
use super::AgentRow;
use super::Observed;
use super::SessionIdentity;
use super::SourceHealth;
use super::SourceRead;
use super::hierarchy;
use super::hierarchy::HierarchyError;
use crate::constants::ACCOUNTS_REFRESH_INTERVAL;
use crate::constants::HIERARCHY_TIMEOUT;
use crate::theme::Role;

/// A nullable reading is unavailable rather than zero.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Reading<T> {
    Known(T),
    Unavailable,
}

impl<T> From<Option<T>> for Reading<T> {
    fn from(value: Option<T>) -> Self { value.map_or(Self::Unavailable, Self::Known) }
}

/// Roles are supplied by accounts.py, including multiple groups per account.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum AccountRole {
    Units,
    Showrunners,
    Workers,
    Spare,
}

impl AccountRole {
    const fn text(self) -> &'static str {
        match self {
            Self::Units => "units",
            Self::Showrunners => "showrunners",
            Self::Workers => "workers",
            Self::Spare => "spare",
        }
    }
}

/// The accounts producer classifies each session once.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum SessionState {
    Busy,
    Starting,
    Waiting,
    Idle,
    Paused,
    Exited,
}

impl SessionState {
    pub(crate) const fn text(self) -> &'static str {
        match self {
            Self::Busy => "busy",
            Self::Starting => "starting",
            Self::Waiting => "waiting",
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
            Self::Reported(SessionState::Busy | SessionState::Starting) => Role::Busy,
            Self::Reported(SessionState::Waiting) => Role::Waiting,
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
}

#[derive(Deserialize)]
struct SessionStateWire {
    session: String,
    state:   SessionState,
}

/// Source counts for one role across the reported machines.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RoleSessionCounts {
    pub(crate) busy:     u64,
    pub(crate) starting: u64,
    pub(crate) waiting:  u64,
    pub(crate) idle:     u64,
    pub(crate) paused:   u64,
    pub(crate) exited:   u64,
    pub(crate) total:    u64,
}

/// One source count exceeds the range of a combined count.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RoleCountOverflow;

impl From<Result<RoleSessionCounts, RoleCountOverflow>> for Reading<RoleSessionCounts> {
    fn from(value: Result<RoleSessionCounts, RoleCountOverflow>) -> Self {
        match value {
            Ok(counts) => Self::Known(counts),
            Err(RoleCountOverflow) => Self::Unavailable,
        }
    }
}

impl RoleSessionCounts {
    /// `RoleSessionCounts::checked_add` sums all seven source counts without overflow.
    pub(crate) fn checked_add(self, other: &Self) -> Result<Self, RoleCountOverflow> {
        Ok(Self {
            busy:     self.busy.checked_add(other.busy).ok_or(RoleCountOverflow)?,
            starting: self
                .starting
                .checked_add(other.starting)
                .ok_or(RoleCountOverflow)?,
            waiting:  self
                .waiting
                .checked_add(other.waiting)
                .ok_or(RoleCountOverflow)?,
            idle:     self.idle.checked_add(other.idle).ok_or(RoleCountOverflow)?,
            paused:   self
                .paused
                .checked_add(other.paused)
                .ok_or(RoleCountOverflow)?,
            exited:   self
                .exited
                .checked_add(other.exited)
                .ok_or(RoleCountOverflow)?,
            total:    self
                .total
                .checked_add(other.total)
                .ok_or(RoleCountOverflow)?,
        })
    }
}

/// `AccountRoleUsage` associates role with its counts.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct AccountRoleUsage {
    pub(crate) role:   AccountRole,
    pub(crate) counts: Reading<RoleSessionCounts>,
}

impl AccountRoleUsage {
    pub(crate) const fn text(&self) -> &'static str { self.role.text() }
}

/// Credit balances have three distinct source meanings.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Credits {
    Amount(f64),
    Unlimited,
    Unavailable,
}

/// Whether this account may spend its available credits.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CreditPermission {
    Enabled,
    Disabled,
}

/// Whether the latest usage read succeeded and a stored reading exists.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AccountReadStatus {
    Ok,
    Failed,
}

/// One signed-in account, in the source's order.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct AccountUsage {
    pub(crate) label:               String,
    pub(crate) roles:               Vec<AccountRoleUsage>,
    pub(crate) weekly_left_percent: Reading<f64>,
    pub(crate) resets_at:           Reading<DateTime<Utc>>,
    pub(crate) credits:             Credits,
    pub(crate) credit_permission:   CreditPermission,
    pub(crate) read_status:         AccountReadStatus,
    pub(crate) read_at:             Reading<DateTime<Utc>>,
}

/// A complete source answer; Rust does not infer its roles or filter its rows.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct AccountsReading {
    pub(crate) received_at: u64,
    pub(crate) sessions:    SessionStates,
    pub(crate) accounts:    Vec<AccountUsage>,
    pub(crate) problems:    Vec<String>,
}

#[derive(Deserialize)]
struct AccountsReport {
    accounts: Vec<AccountWire>,
    problems: Vec<String>,
}

#[derive(Deserialize)]
struct AccountWire {
    #[serde(default)]
    sessions:            Vec<SessionStateWire>,
    label:               String,
    roles:               Vec<AccountRole>,
    #[serde(default)]
    role_counts:         Vec<Option<RoleCountsWire>>,
    weekly_left_percent: Option<f64>,
    resets_at:           Option<String>,
    credits:             Option<CreditWire>,
    use_credits:         bool,
    #[serde(default)]
    read_ok:             bool,
    read_at:             Option<String>,
}

/// The wire reader constructs this proof only when the retired key is absent.
#[derive(Default)]
struct RetiredShellAbsent;

impl<'de> Deserialize<'de> for RetiredShellAbsent {
    fn deserialize<D>(_: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Err(D::Error::custom("the shell count is retired"))
    }
}

#[derive(Deserialize)]
struct RoleCountsWire {
    busy:          u64,
    #[serde(default)]
    starting:      u64,
    #[serde(default)]
    waiting:       u64,
    idle:          u64,
    paused:        u64,
    exited:        u64,
    total:         u64,
    #[serde(default, rename = "shell")]
    retired_shell: RetiredShellAbsent,
}

impl From<RoleCountsWire> for RoleSessionCounts {
    fn from(counts: RoleCountsWire) -> Self {
        let RoleCountsWire {
            busy,
            starting,
            waiting,
            idle,
            paused,
            exited,
            total,
            retired_shell: RetiredShellAbsent,
        } = counts;
        Self {
            busy,
            starting,
            waiting,
            idle,
            paused,
            exited,
            total,
        }
    }
}

/// Pair each role with its combined source count.
fn role_usages(
    roles: Vec<AccountRole>,
    combined_counts: Vec<Option<RoleCountsWire>>,
) -> Vec<AccountRoleUsage> {
    let mut counts = combined_counts.into_iter();
    roles
        .into_iter()
        .map(|role| AccountRoleUsage {
            role,
            counts: counts.next().flatten().map(RoleSessionCounts::from).into(),
        })
        .collect()
}

#[derive(Deserialize)]
#[serde(untagged)]
enum CreditWire {
    Amount(f64),
    Word(Unlimited),
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum Unlimited {
    Unlimited,
}

impl AccountsReading {
    /// Decode nullable wire readings once at the command boundary.
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
        Ok(Self {
            received_at: u64::try_from(Utc::now().timestamp()).unwrap_or_default(),
            sessions:    SessionStates(sessions),
            accounts:    report
                .accounts
                .into_iter()
                .map(|row| {
                    let roles = role_usages(row.roles, row.role_counts);
                    Ok(AccountUsage {
                        label: row.label,
                        roles,
                        weekly_left_percent: row.weekly_left_percent.into(),
                        resets_at: timestamp(row.resets_at.as_deref())?,
                        credits: match row.credits {
                            Some(CreditWire::Amount(amount)) => Credits::Amount(amount),
                            Some(CreditWire::Word(Unlimited::Unlimited)) => Credits::Unlimited,
                            None => Credits::Unavailable,
                        },
                        credit_permission: if row.use_credits {
                            CreditPermission::Enabled
                        } else {
                            CreditPermission::Disabled
                        },
                        read_status: if row.read_ok {
                            AccountReadStatus::Ok
                        } else {
                            AccountReadStatus::Failed
                        },
                        read_at: timestamp(row.read_at.as_deref())?,
                    })
                })
                .collect::<Result<_, AccountsFailure>>()?,
            problems:    report.problems,
        })
    }
}

/// UTC instants enter the domain only after their external text has parsed.
fn timestamp(value: Option<&str>) -> Result<Reading<DateTime<Utc>>, AccountsFailure> {
    value
        .map(|text| {
            DateTime::parse_from_rfc3339(text)
                .map(|at| at.with_timezone(&Utc))
                .map_err(|_| AccountsFailure::InvalidReading)
        })
        .transpose()
        .map(Reading::from)
}

/// A failed command never replaces a usable reading with empty rows.
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

/// Exactly one state describes both the visible reading and any failure.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) enum AccountsTable {
    #[default]
    NotConfigured,
    AwaitingFirstReading,
    Ready(AccountsReading),
    Failed(AccountsFailure),
    FailedKeepingLast {
        last:    AccountsReading,
        failure: AccountsFailure,
    },
}

impl AccountsTable {
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
            Self::Ready(reading) | Self::FailedKeepingLast { last: reading, .. } => {
                &reading.sessions
            },
            Self::NotConfigured | Self::AwaitingFirstReading | Self::Failed(_) => {
                &EMPTY_SESSION_STATES
            },
        }
    }

    /// Visible account content keeps the summary present even without agents.
    pub(crate) const fn is_visible(&self) -> bool {
        matches!(
            self,
            Self::Ready(_) | Self::Failed(_) | Self::FailedKeepingLast { .. }
        )
    }

    /// Preserve the last successful answer across any number of failures.
    pub(crate) fn apply(&mut self, result: Result<AccountsReading, AccountsFailure>) -> bool {
        let next = match result {
            Ok(reading) => Self::Ready(reading),
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

/// Read and decode the configured stored-account command off the frame thread.
pub(crate) fn read_accounts(command: &str) -> Result<AccountsReading, AccountsFailure> {
    let bytes =
        hierarchy::run_output(command, HIERARCHY_TIMEOUT).map_err(AccountsFailure::Command)?;
    AccountsReading::parse(&bytes)
}

/// A missing command is a valid empty source rather than a failed command.
pub(super) fn read_accounts_source(command: &str) -> super::SourceRead<AccountsReading> {
    if command.trim().is_empty() {
        return SourceRead::Absent;
    }
    read_accounts(command).map_or_else(
        |error| SourceRead::Unreadable(super::UnknownReason(error.to_string())),
        SourceRead::Loaded,
    )
}

#[cfg(test)]
#[path = "accounts_source_tests.rs"]
mod source_tests;

/// A stored-account command whose answer is delivered by the refresh thread.
pub(crate) type AccountsCommand =
    Box<dyn Fn(&str) -> Result<AccountsReading, AccountsFailure> + Send>;

/// Only a configured source owns a refresh thread and its shutdown channel.
enum AccountsUpdates {
    Disabled,
    Running {
        readings: Receiver<Result<AccountsReading, AccountsFailure>>,
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
    pub(crate) table: AccountsTable,
    updates:          AccountsUpdates,
}

impl AccountsMonitor {
    pub(crate) fn new(command: String) -> Self {
        Self::spawn_with(command, Box::new(read_accounts), ACCOUNTS_REFRESH_INTERVAL)
    }

    /// `AccountsMonitor::spawn_with` runs `AccountsCommand` on a refresh thread
    /// at `cadence` and publishes readings through `AccountsUpdates::Running`.
    /// An empty `command` selects `AccountsUpdates::Disabled` without a thread.
    pub(crate) fn spawn_with(command: String, runner: AccountsCommand, cadence: Duration) -> Self {
        if command.trim().is_empty() {
            return Self {
                table:   AccountsTable::NotConfigured,
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
            table:   AccountsTable::AwaitingFirstReading,
            updates: AccountsUpdates::Running { readings, stop },
        }
    }

    /// Drain every completed refresh; the next frame sees the latest result.
    pub(crate) fn refresh(&mut self) -> bool {
        let mut changed = false;
        if let AccountsUpdates::Running { readings, .. } = &self.updates {
            while let Ok(result) = readings.try_recv() {
                changed |= self.table.apply(result);
            }
        }
        changed
    }
}
