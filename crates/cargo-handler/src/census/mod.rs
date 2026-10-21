//! The census of agents: the Claude Code and Codex sessions someone can
//! talk to on this machine and on each remote machine named in
//! `[machines] remote`, each with what it is running.
//!
//! [`classify`] decides which processes count, over a process table,
//! the session records Claude Code writes, and the threads [`codex`]
//! reads from Codex's database, and which agent opened a session held
//! by tmux, reading the sessions a shell call opens through [`tmux`].
//! [`tree`] lays out what each agent is running, from the
//! process table and what [`transcript`] reads from Claude Code's
//! transcripts. [`scan`] reads all of it on this machine. [`probe`] is
//! the JSON a machine prints about itself, and [`remote`] runs that
//! probe on another machine over ssh. [`schedule`] runs the scans and
//! probes on threads of their own and hands each answer to the event
//! loop as a [`CensusUpdate`].

pub(crate) mod account;
pub(crate) mod accounts;
pub(crate) mod activity;
mod branch;
#[cfg(test)]
mod catch_up_hierarchy_tests;
pub(crate) mod classify;
#[cfg(test)]
mod claude_seat_tests;
pub(crate) mod codex;
mod codex_names;
pub(crate) mod desktop;
pub(crate) mod hierarchy;
#[cfg(test)]
mod incremental_read_tests;
mod placement;
pub(crate) mod probe;
pub(crate) mod remote;
#[cfg(test)]
mod restart_hierarchy_tests;
#[cfg(test)]
mod reviewer_tests;
pub(crate) mod scan;
pub(crate) mod schedule;
pub(crate) mod tier;
pub(crate) mod tmux;
pub(crate) mod transcript;
pub(crate) mod tree;
pub(crate) mod worker_dispatch;

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::PoisonError;
use std::sync::RwLock;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

pub(crate) use account::Account;
pub(crate) use account::AccountColumn;
#[cfg(test)]
pub(crate) use activity::Activity;
pub(crate) use activity::AgentActivity;
#[cfg(test)]
pub(crate) use activity::RunLocation;
#[cfg(test)]
pub(crate) use activity::VerifyStep;
use hierarchy::HierarchyAnswer;
pub(crate) use hierarchy::HierarchyNotice;
pub(crate) use hierarchy::HierarchySnapshot;
use hierarchy::ReviewerState;
pub(crate) use placement::AgentPlacement;
pub(crate) use placement::ManagedThread;
pub(crate) use placement::PlacedAgent;
pub(crate) use placement::Placement;
pub(crate) use placement::RoleLabel;
pub(crate) use placement::RootPresentation;
pub(crate) use placement::RowIdentity;
#[cfg(test)]
pub(crate) use placement::mesh_worker_fixture;
use scan::ScanStage;
use serde::Deserialize;
use serde::Serialize;
use sysinfo::System;
use worker_dispatch::WorkerDispatch;
use worker_dispatch::WorkerTask;

use crate::constants::CLAUDE_AGENT;
use crate::constants::CODEX_AGENT;
use crate::constants::CODEX_FAST_TIERS;
use crate::constants::COMMAND_RUNS;
use crate::constants::FAST_TIER_LABEL;
use crate::constants::LOCAL_MACHINE_FALLBACK;
use crate::constants::TIMER_RUNS;

/// The program a census row represents, including transport-only worker hosts.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Agent {
    /// A Claude Code session.
    Claude,
    /// A Codex session, or the Codex desktop app.
    Codex,
    /// A non-desktop app server retained only to transport its workers.
    #[serde(rename = "codex_thread_host")]
    CodexThreadHost,
}

impl Agent {
    /// The row's `agent` cell.
    const fn label(self) -> &'static str {
        match self {
            Self::Claude => CLAUDE_AGENT,
            Self::Codex | Self::CodexThreadHost => CODEX_AGENT,
        }
    }
}

/// The service tier a Codex agent or thread asked for. No source reports
/// the tier served, so this is always the one requested.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ServiceTier {
    /// The fast tier: `fast`, or `priority` as a rollout records it.
    Fast,
    /// Any other tier: `default`, a rollout's `null`, or one such as
    /// `flex`.
    Standard,
    /// No source names a tier: every Claude Code row, every row that is
    /// not a Codex thread or process, and a Codex one nothing recorded a
    /// tier for.
    #[default]
    Unrecorded,
}

impl ServiceTier {
    /// This tier, else the one `other` names when this one is
    /// [`Self::Unrecorded`].
    fn or_else(self, other: impl FnOnce() -> Self) -> Self {
        match self {
            Self::Unrecorded => other(),
            Self::Fast | Self::Standard => self,
        }
    }

    /// `program` marked with this tier, as an `agent` or `runs` cell shows
    /// it: `codex fast`, or `program` alone when
    /// [`Self::Unrecorded`].
    pub(crate) fn mark(self, program: impl Into<Cow<'static, str>>) -> Cow<'static, str> {
        let program = program.into();
        match self {
            Self::Fast => Cow::Owned(format!("{program} {FAST_TIER_LABEL}")),
            Self::Standard | Self::Unrecorded => program,
        }
    }
}

impl From<&str> for ServiceTier {
    /// A tier as Codex, its rollouts or the pacer name it: one of
    /// [`CODEX_FAST_TIERS`] is [`Self::Fast`], any other
    /// [`Self::Standard`].
    fn from(tier: &str) -> Self {
        if CODEX_FAST_TIERS.contains(&tier) {
            Self::Fast
        } else {
            Self::Standard
        }
    }
}

/// A Claude Code session's stable identity, absent for other agents or old probes.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(untagged)]
pub(crate) enum SessionIdentity {
    /// No Claude Code session record identifies this row.
    #[default]
    Unrecorded,
    /// The `sessionId` in a Claude Code session record.
    Claude(String),
}

impl From<String> for SessionIdentity {
    fn from(id: String) -> Self {
        if id.is_empty() {
            Self::Unrecorded
        } else {
            Self::Claude(id)
        }
    }
}

/// The member's responsibility under its manager.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum HierarchyRole {
    /// Runs a production.
    Showrunner,
    /// Directs a unit.
    Unit,
    /// Implements a change.
    Impl,
    /// Tests a change.
    Test,
    /// Reviews a change.
    Review,
    /// Repairs a defect.
    Fix,
}

impl HierarchyRole {
    /// The role value the hierarchy command reports, shown in titles and member tree lines.
    const fn text(self) -> &'static str {
        match self {
            Self::Showrunner => "showrunner",
            Self::Unit => "unit",
            Self::Impl => "impl",
            Self::Test => "test",
            Self::Review => "review",
            Self::Fix => "fix",
        }
    }
}

/// A manager and its member, identified across machines by session id.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct HierarchyEdge {
    /// The manager's Claude Code session id, possibly on another machine.
    pub(crate) manager:  String,
    /// The member's Claude session id or Codex app-server thread id.
    pub(crate) member:   String,
    /// What the member does for its manager.
    pub(crate) role:     HierarchyRole,
    #[serde(default)]
    pub(crate) reviewer: ReviewerState,
}

/// The management links reported by one machine.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub(crate) struct HierarchyEdges(pub(crate) Vec<HierarchyEdge>);

/// A Codex app-server thread's stable identity, absent in older probes.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(untagged)]
pub(crate) enum CodexThreadId {
    /// The thread has no recorded identity.
    #[default]
    Unrecorded,
    /// The id in its rollout filename.
    Recorded(String),
}

impl CodexThreadId {
    /// Whether the wire can omit this identity.
    const fn is_unrecorded(&self) -> bool { matches!(self, Self::Unrecorded) }
}

/// Where this machine obtains its management links.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) enum HierarchySource {
    /// Hierarchy discovery is disabled.
    #[default]
    Off,
    /// A shell command printing the schema-1 hierarchy contract.
    Command(String),
}

impl From<String> for HierarchySource {
    fn from(command: String) -> Self {
        if command.trim().is_empty() {
            Self::Off
        } else {
            Self::Command(command)
        }
    }
}

/// The hierarchy setting shared between the app and the scheduler.
#[derive(Clone, Debug)]
pub(crate) struct HierarchyCommand(Arc<RwLock<HierarchySource>>);

impl HierarchyCommand {
    /// Share the configured source with the scheduler.
    pub(crate) fn new(source: HierarchySource) -> Self { Self(Arc::new(RwLock::new(source))) }

    /// Set the source read at the next local scan.
    pub(crate) fn replace(&self, source: HierarchySource) {
        *self.0.write().unwrap_or_else(PoisonError::into_inner) = source;
    }

    /// The source as it stands for this round.
    pub(crate) fn snapshot(&self) -> HierarchySource {
        self.0
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

/// A process role used only when no production relationship names a role.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RoleFallback {
    #[default]
    None,
    Review,
    Worker(HierarchyRole),
}

/// One agent someone can talk to, or an app server transporting workers
/// for placement without appearing in the summary or its own cell.
/// The field order is the
/// probe's JSON order.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(from = "probe::ProbeAgentRow")]
pub(crate) struct AgentRow {
    /// Claude Code, Codex, or an app server retained only for worker placement.
    pub(crate) agent:           Agent,
    /// Claude Code session identity, or unrecorded for other rows.
    pub(crate) session_id:      SessionIdentity,
    pub(crate) role_fallback:   RoleFallback,
    /// The tier a Codex agent asked for; [`ServiceTier::Unrecorded`] for
    /// Claude Code. Missing from a probe printed before it existed.
    pub(crate) service_tier:    ServiceTier,
    /// The home the agent's process runs under. Missing from a probe printed before it existed.
    pub(crate) account:         Account,
    /// The session's name, or what stands in for one.
    pub(crate) name:            String,
    /// One current activity, resolved from conversation events and live verification.
    pub(crate) activity:        AgentActivity,
    /// When the agent's session began, in unix seconds: for Claude Code
    /// the first line of its transcript, so a session resumed or
    /// restarted keeps its age, else when its process started.
    pub(crate) started:         u64,
    /// When the agent's process itself started, in unix seconds, for
    /// stable cell identity even when its session predates the process.
    /// An older probe omits this field; its session start supplies the
    /// identity used by that probe version at the wire boundary.
    pub(crate) process_started: u64,
    /// The agent's process id on its own machine.
    pub(crate) pid:             u32,
    /// The names of the KDE virtual desktops the agent's terminal window
    /// is on, or [`ALL_DESKTOPS_LABEL`](crate::constants::ALL_DESKTOPS_LABEL)
    /// for a window on every one; none where no window was matched to
    /// the agent. Read on Linux under `KWin` alone, and missing from a
    /// probe printed before it existed.
    pub(crate) desktop:         Option<String>,
    /// The directory the agent runs in, with its machine's home
    /// directory written as `~`.
    pub(crate) directory:       String,
    /// The branch checked out where the agent runs, or for a detached
    /// `HEAD` the start of its commit; none outside a repository.
    /// Missing from a probe printed before it existed.
    pub(crate) branch:          Option<String>,
    /// The pid of the row that opened this one: the agent that opened it
    /// in a tmux session, or the row standing for the Claude desktop app
    /// that holds it. None for an agent a person started outside the
    /// app, which is what makes it top level.
    pub(crate) launched_by:     Option<u32>,
    /// What the agent is running, in the order its cell draws it: each
    /// row is followed by the rows it started, one level deeper.
    pub(crate) children:        Vec<ChildRow>,
}

impl AgentRow {
    /// Whether this row represents an agent someone can talk to, rather than a carrier.
    pub(crate) const fn is_visible(&self) -> bool { !matches!(self.agent, Agent::CodexThreadHost) }

    /// The `agent` cell: the program, marked with its tier.
    pub(crate) fn agent_label(&self, account_column: AccountColumn) -> Cow<'static, str> {
        match (account_column, &self.account) {
            (AccountColumn::Shown, Account::Home(home)) => {
                self.service_tier.mark(home.label().to_owned())
            },
            _ => self.service_tier.mark(self.agent.label()),
        }
    }
}

/// How an agent holds a child, including the work kinds shown in tree role columns.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ChildKind {
    /// A command the agent's shell tool is running.
    Shell,
    /// A subagent running inside the agent's own process.
    Subagent,
    /// An agent this one opened, in a tmux session or as a session of
    /// the Claude desktop app, which has a row of its own.
    Session(Agent),
    /// A Claude Code or Codex process one of the agent's shells started.
    UnderShell(Agent),
    /// A Claude Code or Codex process the agent started itself, not
    /// through a shell: an MCP server, or a command Codex runs.
    Direct(Agent),
    /// A Codex app server that has left the shell that started it and
    /// names the agent in `CLAUDE_PID`.
    Detached(Agent),
    /// A thread a Codex app server is running.
    Thread,
}

impl ChildKind {
    /// The agent the row runs: none for a shell, which runs a command.
    const fn agent(self) -> Option<Agent> {
        match self {
            Self::Shell => None,
            Self::Subagent => Some(Agent::Claude),
            Self::Session(agent)
            | Self::UnderShell(agent)
            | Self::Direct(agent)
            | Self::Detached(agent) => Some(agent),
            Self::Thread => Some(Agent::Codex),
        }
    }

    /// The row's `runs` cell: its agent's program, or `command` for a
    /// shell.
    const fn runs(self) -> &'static str {
        match self.agent() {
            Some(agent) => agent.label(),
            None => COMMAND_RUNS,
        }
    }
}

/// The span of the `sleep` a shell row's command is waiting out, in unix
/// seconds.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct Timer {
    /// When the `sleep` started.
    pub(crate) started:  u64,
    /// When the `sleep` ends: [`Self::started`] plus the whole seconds
    /// its arguments add up to.
    pub(crate) deadline: u64,
}

/// Old probe children had no activity unless their thread was busy.
const fn idle_activity() -> AgentActivity { AgentActivity::Idle }

/// One row of an agent's cell: something the agent started that is
/// still running.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(from = "probe::ProbeChildRow", into = "probe::ProbeChildRow")]
pub(crate) struct ChildRow {
    /// Task naming survives placement so the final hierarchy role supplies its label.
    pub(crate) worker_task:  WorkerTask,
    /// The last activity, read incrementally from a thread rollout or subagent transcript.
    pub(crate) activity:     AgentActivity,
    /// A roster-backed launcher identity, resolved only during the scan.
    pub(crate) dispatch:     WorkerDispatch,
    /// The Codex thread this child represents, when recorded.
    pub(crate) thread_id:    CodexThreadId,
    /// How many levels below the agent the row sits; the rows directly
    /// under the agent are at 0.
    pub(crate) depth:        u8,
    /// What the row is.
    pub(crate) kind:         ChildKind,
    /// The tier a Codex thread or process asked for;
    /// [`ServiceTier::Unrecorded`] for any other row. Missing from a
    /// probe printed before it existed.
    pub(crate) service_tier: ServiceTier,
    /// The app server's home, for a Codex thread; unrecorded for any other row.
    /// Missing from a probe printed before it existed.
    pub(crate) account:      Account,
    /// Its process; none for a subagent or a thread, which have none of
    /// their own.
    pub(crate) pid:          Option<u32>,
    /// What the row says it is doing: a shell's description or command,
    /// a subagent's description, a session's or thread's name, or a
    /// process's.
    pub(crate) name:         String,
    /// When it started, in unix seconds.
    pub(crate) started:      u64,
    /// The `sleep` a shell row's command is waiting out, where that
    /// `sleep` is the one process at the bottom of the shell's subtree;
    /// none for any other row. Missing from a probe printed before it
    /// existed, and left out of one for a row that has none.
    pub(crate) timer:        Option<Timer>,
}

impl ChildRow {
    /// The `runs` cell: what the row runs, marked with its tier;
    /// [`TIMER_RUNS`] for a shell row with a [`Timer`].
    pub(crate) fn runs_label(&self) -> Cow<'static, str> {
        let runs = self.timer.map_or_else(|| self.kind.runs(), |_| TIMER_RUNS);
        self.service_tier.mark(runs)
    }
}

/// What is known about one machine's agents.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum MachineState {
    /// No answer yet.
    Scanning,
    /// Every agent someone can talk to, oldest first.
    Answered(Vec<AgentRow>, HierarchySnapshot),
    /// The machine gave no answer, and why.
    Failed(String),
}

impl MachineState {
    /// The rows the machine answered with; none before an answer or
    /// after a failure.
    pub(crate) fn rows(&self) -> &[AgentRow] {
        match self {
            Self::Answered(rows, _) => rows,
            Self::Scanning | Self::Failed(_) => &[],
        }
    }

    /// Management links from this machine's latest completed answer.
    pub(crate) fn edges(&self) -> &[HierarchyEdge] {
        match self {
            Self::Answered(_, hierarchy) => &hierarchy.edges.0,
            Self::Scanning | Self::Failed(_) => &[],
        }
    }

    pub(crate) const fn hierarchy_notice(&self) -> HierarchyNotice {
        match self {
            Self::Answered(_, hierarchy) => hierarchy.notice,
            _ => HierarchyNotice::Clear,
        }
    }
}

/// Last successful links for each remote, independent of its fresh row answers.
#[derive(Default)]
struct RemoteHierarchyAnswers {
    /// Cleared when a host leaves the configured set.
    edges: HashMap<String, HierarchyEdges>,
}

impl RemoteHierarchyAnswers {
    /// Apply a probe answer, retaining successful hierarchy through discovery failures.
    fn apply(
        &mut self,
        host: &str,
        answer: Result<(Vec<AgentRow>, HierarchyAnswer), String>,
    ) -> MachineState {
        match answer {
            Ok((rows, HierarchyAnswer::Answered(edges))) => {
                self.edges.insert(host.to_string(), edges.clone());
                MachineState::Answered(rows, edges.into())
            },
            Ok((rows, HierarchyAnswer::Unmatched(edges))) => {
                self.edges.insert(host.to_string(), edges.clone());
                MachineState::Answered(
                    rows,
                    HierarchySnapshot {
                        edges,
                        notice: HierarchyNotice::NoSessionMatched,
                    },
                )
            },
            Ok((rows, HierarchyAnswer::Failed { reason, reviewers })) => {
                let edges = self.edges.entry(host.to_string()).or_default();
                hierarchy::ReviewerPools::update_edges(edges, &reviewers);
                MachineState::Answered(
                    rows,
                    HierarchySnapshot {
                        edges:  edges.clone(),
                        notice: HierarchyNotice::Failed(reason),
                    },
                )
            },
            Err(reason) => MachineState::Failed(reason),
        }
    }

    /// A removed host must not recover links from its previous configuration.
    fn retain(&mut self, configured: &[String]) {
        self.edges.retain(|host, _| configured.contains(host));
    }
}

/// A remote machine before its first answer.
static SCANNING: MachineState = MachineState::Scanning;

/// One answer from the scheduler.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum CensusUpdate {
    /// This machine's rows and hierarchy at one stage of its scan.
    Local {
        /// Classified or detailed agent rows.
        rows:      Vec<AgentRow>,
        /// Retained links and the latest discovery outcome.
        hierarchy: HierarchySnapshot,
        /// Whether detail collection has finished.
        stage:     ScanStage,
    },
    /// A remote machine's answer, or why there was none.
    Remote {
        /// The ssh name the machine is configured under.
        host:  String,
        /// What it answered.
        state: MachineState,
    },
}

/// What the summary shows: this machine and every remote that has
/// answered, each as of its latest answer.
#[derive(Debug)]
pub(crate) struct Census {
    /// This machine's heading.
    local_name: String,
    /// This machine's agents.
    local:      MachineState,
    /// Each remote's latest answer, by ssh name. A remote configured
    /// but absent here has not answered yet.
    remotes:    HashMap<String, MachineState>,
}

impl Census {
    /// Diagnostics keep an otherwise empty watcher visible.
    pub(crate) fn has_hierarchy_notice(&self, configured: &[String]) -> bool {
        self.machines(configured)
            .iter()
            .any(|machine| machine.state.hierarchy_notice().is_visible())
    }

    /// A census of `local_name` that nothing has answered yet.
    pub(crate) fn new(local_name: String) -> Self {
        Self {
            local_name,
            local: MachineState::Scanning,
            remotes: HashMap::new(),
        }
    }

    /// Record `update`, answering whether anything it says changed.
    pub(crate) fn apply(&mut self, update: CensusUpdate) -> bool {
        let (slot, state) = match update {
            CensusUpdate::Local {
                rows, hierarchy, ..
            } => (&mut self.local, MachineState::Answered(rows, hierarchy)),
            CensusUpdate::Remote { host, state } => (
                self.remotes.entry(host).or_insert(MachineState::Scanning),
                state,
            ),
        };
        if *slot == state {
            return false;
        }
        *slot = state;
        true
    }

    /// Forget every remote that `configured` no longer names, so one
    /// taken out and put back starts over at `scanning`.
    pub(crate) fn retain_remotes(&mut self, configured: &[String]) {
        self.remotes
            .retain(|host, _| configured.iter().any(|kept| kept == host));
    }

    /// The machines the summary draws: this one first, then each
    /// configured remote in `configured` order.
    pub(crate) fn machines<'a>(&'a self, configured: &'a [String]) -> Vec<Machine<'a>> {
        let local = Machine {
            name:  &self.local_name,
            state: &self.local,
        };
        std::iter::once(local)
            .chain(configured.iter().map(|host| Machine {
                name:  host,
                state: self.remotes.get(host).unwrap_or(&SCANNING),
            }))
            .collect()
    }

    /// Whether any machine the summary draws lists an agent, idle ones
    /// included: an idle agent is one waiting on its reader.
    pub(crate) fn lists_an_agent(&self, configured: &[String]) -> bool {
        self.local.rows().iter().any(AgentRow::is_visible)
            || configured.iter().any(|host| {
                self.remotes
                    .get(host)
                    .is_some_and(|state| state.rows().iter().any(AgentRow::is_visible))
            })
    }
}

/// One machine as the summary draws it.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Machine<'a> {
    /// The heading: this machine's short host name, or a remote's ssh
    /// name.
    pub(crate) name:  &'a str,
    /// Its latest answer.
    pub(crate) state: &'a MachineState,
}

/// `machines.remote` as the scheduler reads it, shared with the app so
/// a list edited in the settings overlay applies from the next round of
/// probes on without restarting the scheduler.
#[derive(Clone, Debug)]
pub(crate) struct RemoteMachines(Arc<RwLock<Vec<String>>>);

impl RemoteMachines {
    /// Share `hosts` with the scheduler.
    pub(crate) fn new(hosts: Vec<String>) -> Self { Self(Arc::new(RwLock::new(hosts))) }

    /// Replace the hosts the next round probes.
    pub(crate) fn replace(&self, hosts: Vec<String>) {
        *self.0.write().unwrap_or_else(PoisonError::into_inner) = hosts;
    }

    /// The hosts as they stand, copied out for one round.
    pub(crate) fn snapshot(&self) -> Vec<String> {
        self.0
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

/// This machine's heading: its host name up to the first `.`.
pub(crate) fn local_machine_name() -> String {
    System::host_name()
        .and_then(|name| name.split('.').next().map(str::to_string))
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| LOCAL_MACHINE_FALLBACK.to_string())
}

/// The current time in unix seconds, which an agent's age is measured
/// against.
pub(crate) fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

#[cfg(test)]
mod tests {
    use hierarchy::HierarchyAnswer;
    use hierarchy::HierarchyError;

    use super::*;

    /// A claude row started at `started`.
    fn row(started: u64) -> AgentRow {
        AgentRow {
            agent: Agent::Claude,
            role_fallback: crate::census::RoleFallback::None,
            session_id: SessionIdentity::Unrecorded,
            service_tier: ServiceTier::Unrecorded,
            account: Account::Unrecorded,
            name: "enh/handler".to_string(),
            activity: AgentActivity::from(Some("busy")),
            started,
            process_started: started,
            pid: 428_044,
            desktop: None,
            directory: "~/rust/handler".to_string(),
            branch: None,
            launched_by: None,
            children: Vec::new(),
        }
    }

    /// Remote hierarchy failures retain that host's links while its row details keep changing.
    #[test]
    fn local_notice_changes_publish_even_when_rows_and_links_stay_the_same() {
        let mut census = Census::new("fixture".into());
        for stage in [ScanStage::Classified, ScanStage::Completed] {
            for notice in [
                HierarchyNotice::Failed(HierarchyError::TimedOut),
                HierarchyNotice::Clear,
            ] {
                let rows = vec![row(1)];
                let hierarchy = HierarchySnapshot {
                    edges: HierarchyEdges::default(),
                    notice,
                };
                assert!(census.apply(CensusUpdate::Local {
                    rows: rows.clone(),
                    hierarchy,
                    stage
                }));
                let machines = census.machines(&[]);
                assert_eq!(machines[0].state.rows(), rows);
                assert_eq!(machines[0].state.hierarchy_notice(), notice);
            }
        }
    }

    #[test]
    fn failed_remote_hierarchy_preserves_links_and_updates_rows_until_answered_empty() {
        let mut answers = RemoteHierarchyAnswers::default();
        let mut census = Census::new("local".to_string());
        let edges = HierarchyEdges(vec![HierarchyEdge {
            reviewer: crate::census::hierarchy::ReviewerState::NotPool,
            manager:  "boss".to_string(),
            member:   "worker".to_string(),
            role:     HierarchyRole::Unit,
        }]);
        for (host, started, hierarchy, expected) in [
            (
                "remote",
                1,
                HierarchyAnswer::Answered(edges.clone()),
                edges.clone(),
            ),
            (
                "remote",
                2,
                HierarchyAnswer::Failed {
                    reason:    HierarchyError::ExitedUnsuccessfully,
                    reviewers: Vec::new(),
                },
                edges.clone(),
            ),
            (
                "other",
                3,
                HierarchyAnswer::Failed {
                    reason:    HierarchyError::ExitedUnsuccessfully,
                    reviewers: Vec::new(),
                },
                HierarchyEdges::default(),
            ),
            (
                "remote",
                4,
                HierarchyAnswer::Answered(HierarchyEdges::default()),
                HierarchyEdges::default(),
            ),
            (
                "remote",
                5,
                HierarchyAnswer::Failed {
                    reason:    HierarchyError::ExitedUnsuccessfully,
                    reviewers: Vec::new(),
                },
                HierarchyEdges::default(),
            ),
        ] {
            let notice = match &hierarchy {
                HierarchyAnswer::Failed { reason, .. } => HierarchyNotice::Failed(*reason),
                HierarchyAnswer::Answered(_) => HierarchyNotice::Clear,
                HierarchyAnswer::Unmatched(_) => HierarchyNotice::NoSessionMatched,
            };
            let state = answers.apply(host, Ok((vec![row(started)], hierarchy)));
            assert!(census.apply(CensusUpdate::Remote {
                host: host.to_string(),
                state
            }));
            let configured = vec![host.to_string()];
            assert_eq!(
                census.machines(&configured)[1].state,
                &MachineState::Answered(
                    vec![row(started)],
                    HierarchySnapshot {
                        edges: expected,
                        notice
                    }
                )
            );
        }
        answers.apply("removed", Ok((vec![], HierarchyAnswer::Answered(edges))));
        answers.retain(&["remote".to_string()]);
        assert_eq!(
            answers.apply(
                "removed",
                Ok((
                    vec![row(6)],
                    HierarchyAnswer::Failed {
                        reason:    HierarchyError::ExitedUnsuccessfully,
                        reviewers: Vec::new(),
                    }
                ))
            ),
            MachineState::Answered(
                vec![row(6)],
                HierarchySnapshot {
                    edges:  HierarchyEdges::default(),
                    notice: HierarchyNotice::Failed(HierarchyError::ExitedUnsuccessfully),
                }
            )
        );
    }

    /// An update that repeats what the census already holds changes
    /// nothing, so it asks for no repaint.
    #[test]
    fn only_a_changed_answer_counts_as_a_change() {
        let mut census = Census::new("natedev".to_string());
        assert!(census.apply(CensusUpdate::Local {
            rows:      vec![row(1)],
            hierarchy: HierarchyEdges::default().into(),
            stage:     ScanStage::Classified,
        }));
        assert!(!census.apply(CensusUpdate::Local {
            rows:      vec![row(1)],
            hierarchy: HierarchyEdges::default().into(),
            stage:     ScanStage::Classified,
        }));
        assert!(census.apply(CensusUpdate::Local {
            rows:      vec![row(2)],
            hierarchy: HierarchyEdges::default().into(),
            stage:     ScanStage::Classified,
        }));
        assert!(!census.apply(CensusUpdate::Local {
            rows:      vec![row(2)],
            hierarchy: HierarchyEdges::default().into(),
            stage:     ScanStage::Completed,
        }));
        assert!(census.apply(CensusUpdate::Local {
            rows:      vec![row(3)],
            hierarchy: HierarchyEdges::default().into(),
            stage:     ScanStage::Completed,
        }));
        assert_eq!(
            census.machines(&[])[0].state,
            &MachineState::Answered(vec![row(3)], HierarchyEdges::default().into()),
        );

        let failed = CensusUpdate::Remote {
            host:  "mac".to_string(),
            state: MachineState::Failed("timed out".to_string()),
        };
        assert!(census.apply(failed.clone()));
        assert!(!census.apply(failed));
    }

    /// This machine comes first, then the remotes in configured order,
    /// each still `scanning` until it answers.
    #[test]
    fn machines_list_this_one_then_remotes_in_configured_order() {
        let mut census = Census::new("natedev".to_string());
        census.apply(CensusUpdate::Remote {
            host:  "mac".to_string(),
            state: MachineState::Answered(vec![row(1)], HierarchyEdges::default().into()),
        });
        let configured = ["studio".to_string(), "mac".to_string()];

        let machines = census.machines(&configured);

        let names: Vec<_> = machines.iter().map(|machine| machine.name).collect();
        assert_eq!(names, ["natedev", "studio", "mac"]);
        assert_eq!(machines[0].state, &MachineState::Scanning);
        assert_eq!(machines[1].state, &MachineState::Scanning);
        assert_eq!(machines[2].state.rows(), [row(1)]);
    }

    /// A listed agent on any drawn machine counts, idle or not; a
    /// machine still scanning, a failed one, or a remote no longer
    /// configured does not.
    #[test]
    fn any_drawn_machine_listing_an_agent_counts() {
        let mut census = Census::new("natedev".to_string());
        let configured = ["mac".to_string(), "studio".to_string()];
        assert!(!census.lists_an_agent(&configured));

        census.apply(CensusUpdate::Local {
            rows:      Vec::new(),
            hierarchy: HierarchyEdges::default().into(),
            stage:     ScanStage::Classified,
        });
        census.apply(CensusUpdate::Remote {
            host:  "studio".to_string(),
            state: MachineState::Failed("timed out".to_string()),
        });
        census.apply(CensusUpdate::Remote {
            host:  "old".to_string(),
            state: MachineState::Answered(vec![row(1)], HierarchyEdges::default().into()),
        });
        assert!(!census.lists_an_agent(&configured));

        let idle = AgentRow {
            role_fallback: crate::census::RoleFallback::None,
            activity: AgentActivity::from(Some("idle")),
            ..row(2)
        };
        census.apply(CensusUpdate::Remote {
            host:  "mac".to_string(),
            state: MachineState::Answered(vec![idle], HierarchyEdges::default().into()),
        });
        assert!(census.lists_an_agent(&configured));

        census.apply(CensusUpdate::Local {
            rows:      vec![row(3)],
            hierarchy: HierarchyEdges::default().into(),
            stage:     ScanStage::Classified,
        });
        assert!(census.lists_an_agent(&[]));
    }

    /// A remote taken out of the list loses its answer, so putting it
    /// back shows `scanning` rather than what it said before.
    #[test]
    fn a_remote_taken_out_forgets_its_answer() {
        let mut census = Census::new("natedev".to_string());
        census.apply(CensusUpdate::Remote {
            host:  "mac".to_string(),
            state: MachineState::Answered(vec![row(1)], HierarchyEdges::default().into()),
        });

        census.retain_remotes(&[]);

        let configured = ["mac".to_string()];
        assert_eq!(
            census.machines(&configured)[1].state,
            &MachineState::Scanning
        );
    }
}

#[cfg(test)]
#[path = "thread_state_tests.rs"]
mod thread_state_tests;

#[cfg(test)]
#[path = "activity_tests.rs"]
mod activity_tests;

#[cfg(test)]
mod command_name_tests;
