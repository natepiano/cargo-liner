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

mod branch;
pub(crate) mod classify;
pub(crate) mod codex;
pub(crate) mod desktop;
pub(crate) mod probe;
pub(crate) mod remote;
pub(crate) mod scan;
pub(crate) mod schedule;
pub(crate) mod tier;
pub(crate) mod tmux;
pub(crate) mod transcript;
pub(crate) mod tree;

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::PoisonError;
use std::sync::RwLock;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use serde::Deserialize;
use serde::Serialize;
use sysinfo::System;

use crate::constants::CLAUDE_AGENT;
use crate::constants::CODEX_AGENT;
use crate::constants::CODEX_FAST_TIERS;
use crate::constants::COMMAND_RUNS;
use crate::constants::DETACHED_VIA;
use crate::constants::DIRECT_VIA;
use crate::constants::FAST_TIER_LABEL;
use crate::constants::LOCAL_MACHINE_FALLBACK;
use crate::constants::SESSION_VIA;
use crate::constants::SHELL_VIA;
use crate::constants::STANDARD_TIER_LABEL;
use crate::constants::SUBAGENT_VIA;
use crate::constants::THREAD_VIA;

/// Which program an agent row is.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Agent {
    /// A Claude Code session.
    Claude,
    /// A Codex session, or the Codex desktop app.
    Codex,
}

impl Agent {
    /// The row's `agent` cell.
    const fn label(self) -> &'static str {
        match self {
            Self::Claude => CLAUDE_AGENT,
            Self::Codex => CODEX_AGENT,
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
    /// it: `codex fast`, `codex --`, or `program` alone when
    /// [`Self::Unrecorded`].
    fn mark(self, program: &'static str) -> Cow<'static, str> {
        match self {
            Self::Fast => Cow::Owned(format!("{program} {FAST_TIER_LABEL}")),
            Self::Standard => Cow::Owned(format!("{program} {STANDARD_TIER_LABEL}")),
            Self::Unrecorded => Cow::Borrowed(program),
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

/// One agent someone can talk to, as the summary lists it, as its own
/// cell draws it, and as the probe prints it. The field order is the
/// probe's JSON order.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct AgentRow {
    /// Claude Code or Codex.
    pub(crate) agent:        Agent,
    /// The tier a Codex agent asked for; [`ServiceTier::Unrecorded`] for
    /// Claude Code. Missing from a probe printed before it existed.
    #[serde(default)]
    pub(crate) service_tier: ServiceTier,
    /// The session's name, or what stands in for one.
    pub(crate) name:         String,
    /// What the session says it is doing: `idle`, `busy` or `shell`
    /// for Claude Code, and nothing for Codex, which reports none.
    pub(crate) status:       Option<String>,
    /// When the agent's session began, in unix seconds: for Claude Code
    /// the first line of its transcript, so a session resumed or
    /// restarted keeps its age, else when its process started.
    pub(crate) started:      u64,
    /// The agent's process id on its own machine.
    pub(crate) pid:          u32,
    /// The names of the KDE virtual desktops the agent's terminal window
    /// is on, or [`ALL_DESKTOPS_LABEL`](crate::constants::ALL_DESKTOPS_LABEL)
    /// for a window on every one; none where no window was matched to
    /// the agent. Read on Linux under `KWin` alone, and missing from a
    /// probe printed before it existed.
    #[serde(default)]
    pub(crate) desktop:      Option<String>,
    /// The directory the agent runs in, with its machine's home
    /// directory written as `~`.
    pub(crate) directory:    String,
    /// The branch checked out where the agent runs, or for a detached
    /// `HEAD` the start of its commit; none outside a repository.
    /// Missing from a probe printed before it existed.
    #[serde(default)]
    pub(crate) branch:       Option<String>,
    /// The pid of the agent that opened this one in a tmux session; none
    /// for an agent a person started, which is what makes it top level.
    pub(crate) launched_by:  Option<u32>,
    /// What the agent is running, in the order its cell draws it: each
    /// row is followed by the rows it started, one level deeper.
    pub(crate) children:     Vec<ChildRow>,
}

impl AgentRow {
    /// Whether a person started this agent rather than another agent.
    const fn is_top_level(&self) -> bool { self.launched_by.is_none() }

    /// The `agent` cell: the program, marked with its tier.
    pub(crate) fn agent_label(&self) -> Cow<'static, str> {
        self.service_tier.mark(self.agent.label())
    }
}

/// What one row of an agent's cell is: how the agent holds it, its
/// `via`, and what it runs.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ChildKind {
    /// A command the agent's shell tool is running.
    Shell,
    /// A subagent running inside the agent's own process.
    Subagent,
    /// An agent this one opened in a tmux session, which has a cell of
    /// its own.
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
    /// The row's `via` cell: how the agent holds it.
    pub(crate) const fn via(self) -> &'static str {
        match self {
            Self::Shell | Self::UnderShell(_) => SHELL_VIA,
            Self::Subagent => SUBAGENT_VIA,
            Self::Session(_) => SESSION_VIA,
            Self::Direct(_) => DIRECT_VIA,
            Self::Detached(_) => DETACHED_VIA,
            Self::Thread => THREAD_VIA,
        }
    }

    /// The agent the row runs: none for a shell, which runs a command.
    pub(crate) const fn agent(self) -> Option<Agent> {
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

/// One row of an agent's cell: something the agent started that is
/// still running.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct ChildRow {
    /// How many levels below the agent the row sits; the rows directly
    /// under the agent are at 0.
    pub(crate) depth:        u8,
    /// What the row is.
    pub(crate) kind:         ChildKind,
    /// The tier a Codex thread or process asked for;
    /// [`ServiceTier::Unrecorded`] for any other row. Missing from a
    /// probe printed before it existed.
    #[serde(default)]
    pub(crate) service_tier: ServiceTier,
    /// Its process; none for a subagent or a thread, which have none of
    /// their own.
    pub(crate) pid:          Option<u32>,
    /// What the row says it is doing: a shell's description or command,
    /// a subagent's description, a session's or thread's name, or a
    /// process's.
    pub(crate) name:         String,
    /// When it started, in unix seconds.
    pub(crate) started:      u64,
}

impl ChildRow {
    /// The `runs` cell: what the row runs, marked with its tier.
    pub(crate) fn runs_label(&self) -> Cow<'static, str> {
        self.service_tier.mark(self.kind.runs())
    }
}

/// What is known about one machine's agents.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum MachineState {
    /// No answer yet.
    Scanning,
    /// Every agent someone can talk to, oldest first.
    Answered(Vec<AgentRow>),
    /// The machine gave no answer, and why.
    Failed(String),
}

impl MachineState {
    /// The rows the machine answered with; none before an answer or
    /// after a failure.
    pub(crate) fn rows(&self) -> &[AgentRow] {
        match self {
            Self::Answered(rows) => rows,
            Self::Scanning | Self::Failed(_) => &[],
        }
    }

    /// The rows a person started, oldest first: the ones the summary
    /// lists.
    pub(crate) fn top_level(&self) -> impl Iterator<Item = &AgentRow> {
        self.rows().iter().filter(|row| row.is_top_level())
    }
}

/// A remote machine before its first answer.
static SCANNING: MachineState = MachineState::Scanning;

/// One answer from the scheduler.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum CensusUpdate {
    /// This machine's agents.
    Local(Vec<AgentRow>),
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
            CensusUpdate::Local(rows) => (&mut self.local, MachineState::Answered(rows)),
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
        !self.local.rows().is_empty()
            || configured.iter().any(|host| {
                self.remotes
                    .get(host)
                    .is_some_and(|state| !state.rows().is_empty())
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
    use super::*;

    /// A claude row started at `started`.
    fn row(started: u64) -> AgentRow {
        AgentRow {
            agent: Agent::Claude,
            service_tier: ServiceTier::Unrecorded,
            name: "enh/handler".to_string(),
            status: Some("busy".to_string()),
            started,
            pid: 428_044,
            desktop: None,
            directory: "~/rust/handler".to_string(),
            branch: None,
            launched_by: None,
            children: Vec::new(),
        }
    }

    /// An update that repeats what the census already holds changes
    /// nothing, so it asks for no repaint.
    #[test]
    fn only_a_changed_answer_counts_as_a_change() {
        let mut census = Census::new("natedev".to_string());
        assert!(census.apply(CensusUpdate::Local(vec![row(1)])));
        assert!(!census.apply(CensusUpdate::Local(vec![row(1)])));
        assert!(census.apply(CensusUpdate::Local(vec![row(2)])));

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
            state: MachineState::Answered(vec![row(1)]),
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

        census.apply(CensusUpdate::Local(Vec::new()));
        census.apply(CensusUpdate::Remote {
            host:  "studio".to_string(),
            state: MachineState::Failed("timed out".to_string()),
        });
        census.apply(CensusUpdate::Remote {
            host:  "old".to_string(),
            state: MachineState::Answered(vec![row(1)]),
        });
        assert!(!census.lists_an_agent(&configured));

        let idle = AgentRow {
            status: Some("idle".to_string()),
            ..row(2)
        };
        census.apply(CensusUpdate::Remote {
            host:  "mac".to_string(),
            state: MachineState::Answered(vec![idle]),
        });
        assert!(census.lists_an_agent(&configured));

        census.apply(CensusUpdate::Local(vec![row(3)]));
        assert!(census.lists_an_agent(&[]));
    }

    /// A remote taken out of the list loses its answer, so putting it
    /// back shows `scanning` rather than what it said before.
    #[test]
    fn a_remote_taken_out_forgets_its_answer() {
        let mut census = Census::new("natedev".to_string());
        census.apply(CensusUpdate::Remote {
            host:  "mac".to_string(),
            state: MachineState::Answered(vec![row(1)]),
        });

        census.retain_remotes(&[]);

        let configured = ["mac".to_string()];
        assert_eq!(
            census.machines(&configured)[1].state,
            &MachineState::Scanning
        );
    }
}
