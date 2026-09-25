//! What each agent is running, laid out as the rows of its cell.
//!
//! An agent's rows come from four places. Its shell tool's commands are
//! the children of its process that are Claude Code's shell wrapper,
//! each named by the shell call [`scan`](super::scan) found for it in a
//! transcript. The Claude Code and Codex processes a command started
//! are found by walking the wrapper's subtree, and each is laid out the
//! same way in turn. Its subagents come from its transcript's
//! directory, and a Codex app server's threads from the conversation
//! files it holds open. The sessions it opened in tmux are the agents
//! that name it as their launcher.
//!
//! Everything here is a pure function over the process table and what
//! the scan read from disk, so the tests drive it from fixtures.

use std::collections::HashMap;
use std::collections::HashSet;
use std::path::Path;

use super::Agent;
use super::AgentRow;
use super::ChildKind;
use super::ChildRow;
use super::classify;
use super::classify::ProcessEntry;
use super::classify::SessionRecord;
use super::transcript::BashCall;
use super::transcript::Subagent;
use crate::constants::CALL_LOOKAHEAD;
use crate::constants::CLAUDE_AGENT;
use crate::constants::CODEX_AGENT;
use crate::constants::CODEX_APP_SERVER_LABEL;
use crate::constants::CODEX_ROLLOUT_EXTENSION;
use crate::constants::CODEX_ROLLOUT_PREFIX;
use crate::constants::CODEX_THREAD_ID_LENGTH;
use crate::constants::SHELL_EVAL_OPEN;
use crate::constants::SHELL_QUOTED_QUOTE;
use crate::constants::SHELL_SCRIPT_FLAG;
use crate::constants::SHELL_SNAPSHOT_MARKER;
use crate::constants::TREE_DEPTH_LIMIT;

/// The shell call that started one shell wrapper.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ShellCall {
    /// What the session said the command does, where it said.
    pub(super) description: Option<String>,
    /// The subagent whose transcript holds the call; none for the
    /// session's own.
    pub(super) subagent:    Option<String>,
}

/// One thread a Codex app server holds open.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ThreadEntry {
    /// The thread's label, else its id.
    pub(super) name:    String,
    /// When it was created, in unix seconds.
    pub(super) started: u64,
}

/// What the scan read from disk for the tree.
#[derive(Debug, Default)]
pub(super) struct TreeSources<'a> {
    /// Every session record, for naming a Claude Code process found
    /// under a shell.
    pub(super) sessions:    &'a [SessionRecord],
    /// The running subagents of each Claude Code process, by pid.
    pub(super) subagents:   HashMap<u32, Vec<Subagent>>,
    /// The call that started each shell wrapper, by the wrapper's pid.
    pub(super) shell_calls: HashMap<u32, ShellCall>,
    /// The threads each Codex app server holds open, by its pid.
    pub(super) threads:     HashMap<u32, Vec<ThreadEntry>>,
}

/// The command a Claude Code shell wrapper runs, when `process` is one:
/// its script sources a shell snapshot and runs the command quoted after
/// `eval '`, where `'\''` stands for one `'`. A script with no `eval '`
/// is the command itself.
pub(super) fn shell_command(process: &ProcessEntry) -> Option<String> {
    if process.arguments.get(1).map(String::as_str) != Some(SHELL_SCRIPT_FLAG) {
        return None;
    }
    let script = process.arguments.get(2)?;
    if !script.contains(SHELL_SNAPSHOT_MARKER) {
        return None;
    }
    let Some(open) = script.find(SHELL_EVAL_OPEN) else {
        return Some(script.clone());
    };
    let mut rest = &script[open + SHELL_EVAL_OPEN.len()..];
    let mut command = String::new();
    loop {
        if let Some(after) = rest.strip_prefix(SHELL_QUOTED_QUOTE) {
            command.push('\'');
            rest = after;
            continue;
        }
        let mut characters = rest.chars();
        match characters.next() {
            None | Some('\'') => return Some(command),
            Some(character) => {
                command.push(character);
                rest = characters.as_str();
            },
        }
    }
}

/// The call among `main`, the session's own, and `subagents`, each
/// subagent's calls by its id, that started a shell wrapper running
/// `command` at `started`, in unix seconds: the latest one written no
/// later than [`CALL_LOOKAHEAD`] after it whose command is `command`.
pub(super) fn match_shell_call(
    command: &str,
    started: u64,
    main: &[BashCall],
    subagents: &[(String, Vec<BashCall>)],
) -> Option<ShellCall> {
    let until = started
        .saturating_mul(1_000)
        .saturating_add(u64::try_from(CALL_LOOKAHEAD.as_millis()).unwrap_or(u64::MAX));
    let own = main.iter().map(|call| (None, call));
    let theirs = subagents
        .iter()
        .flat_map(|(id, calls)| calls.iter().map(move |call| (Some(id), call)));
    own.chain(theirs)
        .filter(|(_, call)| call.at_ms <= until && call.command == command)
        .max_by_key(|(_, call)| call.at_ms)
        .map(|(subagent, call)| ShellCall {
            description: call.description.clone(),
            subagent:    subagent.cloned(),
        })
}

/// The id of the thread whose conversation file is `path`:
/// `rollout-<time>-<thread id>.jsonl`, the id its last
/// [`CODEX_THREAD_ID_LENGTH`] characters.
pub(super) fn thread_id(path: &Path) -> Option<String> {
    if path.extension()? != CODEX_ROLLOUT_EXTENSION {
        return None;
    }
    let stem = path.file_stem()?.to_str()?;
    if !stem.starts_with(CODEX_ROLLOUT_PREFIX) {
        return None;
    }
    let start = stem.len().checked_sub(CODEX_THREAD_ID_LENGTH)?;
    stem.get(start..).map(str::to_string)
}

/// One row before it is laid out: what it is and the rows under it.
#[derive(Debug)]
struct Node {
    /// What the row is.
    kind:     ChildKind,
    /// Its process, where it has one.
    pid:      Option<u32>,
    /// What the row says.
    name:     String,
    /// When it started, in unix seconds.
    started:  u64,
    /// The rows it started.
    children: Vec<Self>,
}

/// Fill in each of `rows`' children from `processes` and `sources`.
pub(super) fn attach_children(
    rows: &mut [AgentRow],
    processes: &[ProcessEntry],
    sources: &TreeSources<'_>,
) {
    let table = classify::pid_table(processes);
    let mut children: HashMap<u32, Vec<&ProcessEntry>> = HashMap::new();
    for process in processes {
        if let Some(parent) = process.parent {
            children.entry(parent).or_default().push(process);
        }
    }
    let walker = Walker {
        children: &children,
        sources,
    };
    let trees: Vec<Vec<ChildRow>> = rows
        .iter()
        .map(|row| {
            let mut visited = HashSet::from([row.pid]);
            let mut nodes = table
                .get(&row.pid)
                .map(|process| walker.agent_nodes(process, 0, &mut visited))
                .unwrap_or_default();
            nodes.extend(
                rows.iter()
                    .filter(|other| other.launched_by == Some(row.pid))
                    .map(|session| Node {
                        kind:     ChildKind::Session(session.agent),
                        pid:      Some(session.pid),
                        name:     session.name.clone(),
                        started:  session.started,
                        children: Vec::new(),
                    }),
            );
            // An app server a mesh moved out from under the agent names it
            // in `CLAUDE_PID`. A Claude Code process is never attached this
            // way: a terminal window an agent opened carries the variable
            // too, and the session in it is top level.
            let detached: Vec<&ProcessEntry> = processes
                .iter()
                .filter(|process| {
                    process.is_codex()
                        && process.is_app_server()
                        && process.claude_pid == Some(row.pid)
                        && process.started >= row.started
                        && !classify::under_an_agent(&table, process)
                })
                .collect();
            for server in detached {
                if visited.insert(server.pid) {
                    nodes.push(walker.process_node(server, 1, &mut visited));
                }
            }
            let mut laid_out = Vec::new();
            flatten(nodes, 0, &mut laid_out);
            laid_out
        })
        .collect();
    for (row, tree) in rows.iter_mut().zip(trees) {
        row.children = tree;
    }
}

/// Walks the process table below an agent.
struct Walker<'a> {
    /// Each process's children, by the parent's pid.
    children: &'a HashMap<u32, Vec<&'a ProcessEntry>>,
    /// What the scan read from disk.
    sources:  &'a TreeSources<'a>,
}

impl Walker<'_> {
    /// The rows directly under the agent `process`, `level` agents below
    /// the cell's own, skipping anything in `visited`.
    fn agent_nodes(
        &self,
        process: &ProcessEntry,
        level: u8,
        visited: &mut HashSet<u32>,
    ) -> Vec<Node> {
        if level >= TREE_DEPTH_LIMIT {
            return Vec::new();
        }
        if process.is_codex() && process.is_app_server() {
            return self
                .sources
                .threads
                .get(&process.pid)
                .into_iter()
                .flatten()
                .map(|thread| Node {
                    kind:     ChildKind::Thread,
                    pid:      None,
                    name:     thread.name.clone(),
                    started:  thread.started,
                    children: Vec::new(),
                })
                .collect();
        }
        let mut shells_by_subagent: HashMap<String, Vec<Node>> = HashMap::new();
        let mut nodes = Vec::new();
        for child in self.children.get(&process.pid).into_iter().flatten() {
            if !visited.insert(child.pid) {
                continue;
            }
            if process.is_claude()
                && let Some(command) = shell_command(child)
            {
                let call = self.sources.shell_calls.get(&child.pid);
                let name = call
                    .and_then(|call| call.description.clone())
                    .unwrap_or_else(|| command.split_whitespace().collect::<Vec<_>>().join(" "));
                let shell = Node {
                    kind: ChildKind::Shell,
                    pid: Some(child.pid),
                    name,
                    started: child.started,
                    children: self.agents_below(child, level, visited),
                };
                match call.and_then(|call| call.subagent.clone()) {
                    Some(subagent) => shells_by_subagent.entry(subagent).or_default().push(shell),
                    None => nodes.push(shell),
                }
            } else if child.is_agent() {
                nodes.push(self.process_node(child, level.saturating_add(1), visited));
            }
        }
        if process.is_claude() {
            let subagents = self
                .sources
                .subagents
                .get(&process.pid)
                .map_or(&[][..], Vec::as_slice);
            let running: HashSet<&str> = subagents.iter().map(|agent| agent.id.as_str()).collect();
            for agent in subagents {
                if agent
                    .parent
                    .as_deref()
                    .is_none_or(|parent| !running.contains(parent))
                {
                    nodes.push(subagent_node(agent, subagents, &mut shells_by_subagent, 0));
                }
            }
            // A shell whose subagent has since finished stays under the agent.
            nodes.extend(shells_by_subagent.into_values().flatten());
        }
        nodes
    }

    /// The Claude Code and Codex processes in the subtree under the
    /// shell wrapper `shell`, each as a row with its own rows under it.
    /// The walk stops at each agent, whose rows come from its own walk.
    fn agents_below(
        &self,
        shell: &ProcessEntry,
        level: u8,
        visited: &mut HashSet<u32>,
    ) -> Vec<Node> {
        let mut found = Vec::new();
        let mut pending: Vec<&ProcessEntry> =
            self.children.get(&shell.pid).cloned().unwrap_or_default();
        while let Some(process) = pending.pop() {
            if !visited.insert(process.pid) {
                continue;
            }
            if process.is_agent() {
                found.push(self.process_node(process, level.saturating_add(1), visited));
            } else {
                pending.extend(
                    self.children
                        .get(&process.pid)
                        .into_iter()
                        .flatten()
                        .copied(),
                );
            }
        }
        found
    }

    /// The row for the agent process `process`, `level` agents below the
    /// cell's own, with the rows under it.
    fn process_node(&self, process: &ProcessEntry, level: u8, visited: &mut HashSet<u32>) -> Node {
        let (agent, name) = if process.is_codex() {
            let name = if process.is_app_server() {
                CODEX_APP_SERVER_LABEL.to_string()
            } else {
                process
                    .arguments_label()
                    .unwrap_or_else(|| CODEX_AGENT.to_string())
            };
            (Agent::Codex, name)
        } else {
            let record = self
                .sources
                .sessions
                .iter()
                .find(|session| session.pid == process.pid);
            let name = record
                .map(SessionRecord::label)
                .or_else(|| process.arguments_label());
            (
                Agent::Claude,
                name.unwrap_or_else(|| CLAUDE_AGENT.to_string()),
            )
        };
        Node {
            kind: ChildKind::Process(agent),
            pid: Some(process.pid),
            name,
            started: process.started,
            children: self.agent_nodes(process, level, visited),
        }
    }
}

/// The row for the subagent `agent`, with the shells found in its
/// transcript and the running subagents it started under it.
fn subagent_node(
    agent: &Subagent,
    subagents: &[Subagent],
    shells: &mut HashMap<String, Vec<Node>>,
    level: u8,
) -> Node {
    let mut children = shells.remove(&agent.id).unwrap_or_default();
    if level < TREE_DEPTH_LIMIT {
        children.extend(
            subagents
                .iter()
                .filter(|other| other.parent.as_deref() == Some(agent.id.as_str()))
                .map(|other| subagent_node(other, subagents, shells, level.saturating_add(1))),
        );
    }
    Node {
        kind: ChildKind::Subagent,
        pid: None,
        name: agent.description.clone(),
        started: agent.started,
        children,
    }
}

/// Lay `nodes` out at `depth` into `rows`, each followed by the rows
/// under it, siblings ordered by when they started.
fn flatten(mut nodes: Vec<Node>, depth: u8, rows: &mut Vec<ChildRow>) {
    nodes.sort_by(|left, right| {
        (left.started, left.pid, &left.name).cmp(&(right.started, right.pid, &right.name))
    });
    for node in nodes {
        rows.push(ChildRow {
            depth,
            kind: node.kind,
            pid: node.pid,
            name: node.name,
            started: node.started,
        });
        flatten(node.children, depth.saturating_add(1), rows);
    }
}

#[cfg(test)]
#[expect(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    /// The home directory every fixture's paths sit under.
    const HOME: &str = "/home/natepiano";
    /// boss of bosses, which opened the three tmux sessions below.
    const BOSS: u32 = 1_579_022;
    /// The session boss opened for the trunk, from a loop.
    const TRUNK: u32 = 3_266_367;
    /// The session boss opened for geometry-material.
    const GEOMETRY: u32 = 3_331_942;
    /// The session boss opened for arrange, which runs nothing.
    const ARRANGE: u32 = 3_337_048;
    /// An interactive Codex in a terminal window trunk opened.
    const TERMINAL_CODEX: u32 = 4_000_020;
    /// The unix second the shell calls in the matching test are measured
    /// from.
    const STARTED: u64 = 1_790_000_000;

    /// A process with no command line read.
    fn process(pid: u32, parent: u32, name: &str, started: u64) -> ProcessEntry {
        ProcessEntry {
            pid,
            parent: Some(parent),
            name: name.to_string(),
            arguments: Vec::new(),
            started,
            directory: None,
            claude_pid: None,
        }
    }

    /// A process with its command line read, in [`HOME`].
    fn detailed(
        pid: u32,
        parent: u32,
        name: &str,
        started: u64,
        arguments: &[&str],
    ) -> ProcessEntry {
        ProcessEntry {
            arguments: arguments
                .iter()
                .map(|argument| (*argument).to_string())
                .collect(),
            directory: Some(PathBuf::from(HOME)),
            ..process(pid, parent, name, started)
        }
    }

    /// The script Claude Code's shell tool hands zsh to run `quoted`, a
    /// command already written for the inside of single quotes.
    fn wrapper_script(quoted: &str) -> String {
        format!(
            "source {HOME}/.claude/shell-snapshots/snapshot-zsh-1790000000000-ab12cd.sh \
             2>/dev/null || true && setopt NO_EXTENDED_GLOB 2>/dev/null || true && \
             eval '{quoted}' < /dev/null && pwd -P >| /tmp/claude-5f2e-cwd"
        )
    }

    /// Claude Code's shell wrapper under `parent`, running `quoted`.
    fn wrapper(pid: u32, parent: u32, started: u64, quoted: &str) -> ProcessEntry {
        detailed(
            pid,
            parent,
            "zsh",
            started,
            &[
                "/run/current-system/sw/bin/zsh",
                "-c",
                &wrapper_script(quoted),
            ],
        )
    }

    /// A Codex app server under `parent` whose `CLAUDE_PID` is `claude_pid`.
    fn app_server(pid: u32, parent: u32, started: u64, claude_pid: Option<u32>) -> ProcessEntry {
        ProcessEntry {
            claude_pid,
            ..detailed(
                pid,
                parent,
                "codex",
                started,
                &["codex", "app-server", "--listen", "ws://127.0.0.1:43483"],
            )
        }
    }

    /// A named session record in [`HOME`].
    fn session(pid: u32, name: &str) -> SessionRecord {
        SessionRecord {
            pid,
            session_id: format!("{pid:08}-0000-0000-0000-000000000000"),
            cwd: Some(PathBuf::from(HOME)),
            name: Some(name.to_string()),
            status: Some("busy".to_string()),
        }
    }

    /// A running subagent.
    fn subagent(id: &str, parent: Option<&str>, description: &str, started: u64) -> Subagent {
        Subagent {
            id: id.to_string(),
            parent: parent.map(str::to_string),
            description: description.to_string(),
            started,
            transcript: PathBuf::from(format!("/fixture/agent-{id}.jsonl")),
        }
    }

    /// A shell call described as `description`, found in the transcript
    /// of `subagent`, else the session's own.
    fn found_call(description: &str, subagent: Option<&str>) -> ShellCall {
        ShellCall {
            description: Some(description.to_string()),
            subagent:    subagent.map(str::to_string),
        }
    }

    /// A thread an app server holds open.
    fn thread(name: &str, started: u64) -> ThreadEntry {
        ThreadEntry {
            name: name.to_string(),
            started,
        }
    }

    /// natedev as the brief found it. boss of bosses runs in a terminal
    /// window, with a Claude Code session it started through its shell
    /// tool, and has opened three sessions in tmux. geometry-material
    /// runs a mesh through its shell tool, a Codex MCP server beside a
    /// Bevy one, and two subagents. An app server trunk's mesh started
    /// now sits under systemd, beside one started before trunk and one
    /// no agent started. A Codex in a terminal window trunk opened
    /// carries trunk's `CLAUDE_PID` too.
    fn natedev() -> (Vec<ProcessEntry>, Vec<SessionRecord>) {
        let processes = vec![
            process(1, 0, "systemd", 0),
            process(2_246, 1, "systemd", 10),
            process(45_448, 2_246, ".ghostty-wrappe", 500),
            process(45_494, 45_448, "zsh", 500),
            detailed(BOSS, 45_494, "claude", 500, &["claude"]),
            wrapper(
                3_800_000,
                BOSS,
                705,
                r"claude -p '\''summarize the plan'\''",
            ),
            detailed(
                3_800_010,
                3_800_000,
                "claude",
                706,
                &["claude", "-p", "summarize the plan"],
            ),
            wrapper(
                3_800_020,
                3_800_010,
                710,
                "cd ~/rust/handler\ncargo nextest run",
            ),
            process(3_261_729, 2_246, "tmux: server", 600),
            process(3_266_307, 3_261_729, "zsh", 611),
            detailed(TRUNK, 3_266_307, "claude", 611, &["claude"]),
            process(3_331_900, 3_261_729, "zsh", 700),
            detailed(GEOMETRY, 3_331_900, "claude", 700, &["claude"]),
            process(3_336_767, 3_261_729, "zsh", 901),
            detailed(ARRANGE, 3_336_767, "claude", 901, &["claude"]),
            detailed(3_331_950, GEOMETRY, "bevy_brp_mcp", 701, &["bevy_brp_mcp"]),
            detailed(3_331_960, GEOMETRY, "codex", 702, &["codex", "mcp-server"]),
            wrapper(
                3_900_000,
                GEOMETRY,
                800,
                r"bash implement.sh --phase '\''1'\''",
            ),
            process(3_900_010, 3_900_000, "bash", 800),
            process(3_900_020, 3_900_010, "python3", 801),
            app_server(3_900_030, 3_900_020, 802, Some(GEOMETRY)),
            process(3_900_040, 3_900_030, "codex-code-mode", 803),
            detailed(
                3_900_050,
                3_900_010,
                "claude",
                805,
                &["claude", "--print", "review"],
            ),
            wrapper(3_950_000, GEOMETRY, 960, "ls tests"),
            wrapper(3_960_000, GEOMETRY, 970, "cargo build"),
            app_server(3_769_600, 2_246, 650, Some(TRUNK)),
            app_server(3_700_000, 2_246, 600, Some(TRUNK)),
            app_server(3_532_979, 2_246, 950, None),
            process(4_000_000, 2_246, ".ghostty-wrappe", 960),
            process(4_000_010, 4_000_000, "zsh", 960),
            ProcessEntry {
                claude_pid: Some(TRUNK),
                ..detailed(TERMINAL_CODEX, 4_000_010, "codex", 960, &["codex"])
            },
        ];
        let sessions = vec![
            session(BOSS, "boss of bosses"),
            session(TRUNK, "tool-based-ui-trunk"),
            session(GEOMETRY, "tool-based-ui-geometry-material"),
            session(ARRANGE, "tool-based-ui-arrange"),
            session(3_800_010, "plan summary"),
        ];
        (processes, sessions)
    }

    /// What the scan read from disk for [`natedev`]. The last shell's call
    /// was found in the transcript of a subagent that has since finished.
    fn natedev_sources(sessions: &[SessionRecord]) -> TreeSources<'_> {
        TreeSources {
            sessions,
            subagents: HashMap::from([(
                GEOMETRY,
                vec![
                    subagent("a1", None, "Review phase 1", 900),
                    subagent("a2", Some("a1"), "Explore the tests", 950),
                ],
            )]),
            shell_calls: HashMap::from([
                (3_800_000, found_call("Ask for a summary", None)),
                (3_900_000, found_call("Run phase 1", None)),
                (3_950_000, found_call("List the tests", Some("a2"))),
                (3_960_000, found_call("Build it", Some("a0"))),
            ]),
            threads: HashMap::from([
                (
                    3_900_030,
                    vec![thread("phase 1 review", 804), thread("phase 1 impl", 803)],
                ),
                (3_769_600, vec![thread("trunk mesh", 660)]),
                (3_700_000, vec![thread("stale", 601)]),
            ]),
        }
    }

    /// An agent row for `pid`, started at `started`.
    fn agent_row(pid: u32, started: u64) -> AgentRow {
        AgentRow {
            agent: Agent::Claude,
            name: format!("pid {pid}"),
            status: None,
            started,
            pid,
            desktop: None,
            directory: HOME.to_string(),
            launched_by: None,
            children: Vec::new(),
        }
    }

    /// The children of the row for `pid` in `rows`, each as its depth,
    /// kind, pid, name and start.
    fn cell(rows: &[AgentRow], pid: u32) -> Vec<(u8, ChildKind, Option<u32>, &str, u64)> {
        rows.iter()
            .find(|row| row.pid == pid)
            .expect("the agent should be listed")
            .children
            .iter()
            .map(|child| {
                (
                    child.depth,
                    child.kind,
                    child.pid,
                    child.name.as_str(),
                    child.started,
                )
            })
            .collect()
    }

    /// A wrapper's command is what its script quotes after `eval`, with
    /// each `'\''` read as one `'`; a script with no `eval` is the
    /// command itself. A shell that sources no snapshot, or runs no
    /// script, is not a wrapper.
    #[test]
    fn a_wrapper_command_is_read_from_its_eval() {
        let quoted = wrapper(10, 1, 0, r"printf '\''%s\n'\'' done && ls");
        assert_eq!(
            shell_command(&quoted).as_deref(),
            Some(r"printf '%s\n' done && ls")
        );

        let snapshot = format!("source {HOME}/.claude/shell-snapshots/snapshot-zsh-1.sh");
        let unclosed = format!("{snapshot} && eval 'ls -la");
        assert_eq!(
            shell_command(&detailed(10, 1, "zsh", 0, &["zsh", "-c", &unclosed])).as_deref(),
            Some("ls -la")
        );

        let script = format!("{snapshot} && ls");
        let bare = detailed(10, 1, "zsh", 0, &["zsh", "-c", &script]);
        assert_eq!(shell_command(&bare), Some(script.clone()));

        assert_eq!(
            shell_command(&detailed(10, 1, "zsh", 0, &["zsh", "-c", "ls"])),
            None
        );
        assert_eq!(
            shell_command(&detailed(10, 1, "zsh", 0, &["zsh", "-i", &script])),
            None
        );
        assert_eq!(shell_command(&process(10, 1, "zsh", 0)), None);
    }

    /// A shell call written `offset_ms` milliseconds from [`STARTED`].
    fn call_at(offset_ms: i64, command: &str, description: &str) -> BashCall {
        BashCall {
            at_ms:       STARTED
                .saturating_mul(1_000)
                .saturating_add_signed(offset_ms),
            command:     command.to_string(),
            description: Some(description.to_string()),
        }
    }

    /// The call that started a wrapper is the latest one running exactly
    /// its command written no later than five seconds after it started,
    /// and one in a subagent's transcript names that subagent.
    #[test]
    fn a_shell_call_is_the_latest_exact_match_up_to_five_seconds_after() {
        let main = [
            call_at(-60_000, "cargo build", "an earlier build"),
            call_at(-2_000, "cargo build", "build"),
            call_at(-1_000, "cargo build --release", "a longer command"),
            call_at(5_001, "cargo build", "too late"),
        ];

        assert_eq!(
            match_shell_call("cargo build", STARTED, &main, &[]),
            Some(found_call("build", None))
        );
        assert_eq!(match_shell_call("cargo", STARTED, &main, &[]), None);
        assert_eq!(match_shell_call("cargo build ", STARTED, &main, &[]), None);

        let subagents = [(
            "a1".to_string(),
            vec![call_at(5_000, "cargo build", "the subagent's build")],
        )];
        assert_eq!(
            match_shell_call("cargo build", STARTED, &main, &subagents),
            Some(found_call("the subagent's build", Some("a1")))
        );
    }

    /// A thread's id is the last 36 characters of its conversation
    /// file's name; any other file names no thread.
    #[test]
    fn a_thread_id_ends_the_conversation_file_name() {
        let day = Path::new("/home/natepiano/.codex/sessions/2026/09/24");
        let id = "0199a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b";

        assert_eq!(
            thread_id(&day.join(format!("rollout-2026-09-24T10-11-12-{id}.jsonl"))).as_deref(),
            Some(id)
        );
        assert_eq!(
            thread_id(&day.join(format!("rollout-2026-09-24T10-11-12-{id}.json"))),
            None
        );
        assert_eq!(
            thread_id(&day.join(format!("history-2026-09-24T10-11-12-{id}.jsonl"))),
            None
        );
        assert_eq!(thread_id(&day.join("rollout-short.jsonl")), None);
    }

    /// Each agent's cell lists what it started, depth first with
    /// siblings by start: its shells, named by their calls, each with
    /// the agents under it and what those run; a Codex app server's
    /// threads; its subagents, with the shells found in their
    /// transcripts; the sessions it opened in tmux; and an app server
    /// moved under systemd that names it in `CLAUDE_PID`. A Bevy MCP
    /// server, an app server started before the agent, one no agent
    /// started, and a Codex that is not an app server are left out.
    #[test]
    fn natedev_cells_hold_what_each_agent_runs() {
        let (processes, sessions) = natedev();
        let mut rows = classify::agent_rows(&processes, &sessions, &[], Some(Path::new(HOME)));
        for row in &mut rows {
            if [TRUNK, GEOMETRY, ARRANGE].contains(&row.pid) {
                row.launched_by = Some(BOSS);
            }
        }
        let sources = natedev_sources(&sessions);

        attach_children(&mut rows, &processes, &sources);

        let session = ChildKind::Session(Agent::Claude);
        let claude = ChildKind::Process(Agent::Claude);
        let codex = ChildKind::Process(Agent::Codex);
        assert_eq!(
            cell(&rows, BOSS),
            [
                (0, session, Some(TRUNK), "tool-based-ui-trunk", 611),
                (
                    0,
                    session,
                    Some(GEOMETRY),
                    "tool-based-ui-geometry-material",
                    700
                ),
                (
                    0,
                    ChildKind::Shell,
                    Some(3_800_000),
                    "Ask for a summary",
                    705
                ),
                (1, claude, Some(3_800_010), "plan summary", 706),
                (
                    2,
                    ChildKind::Shell,
                    Some(3_800_020),
                    "cd ~/rust/handler cargo nextest run",
                    710,
                ),
                (0, session, Some(ARRANGE), "tool-based-ui-arrange", 901),
            ]
        );
        assert_eq!(
            cell(&rows, GEOMETRY),
            [
                (0, codex, Some(3_331_960), "mcp-server", 702),
                (0, ChildKind::Shell, Some(3_900_000), "Run phase 1", 800),
                (1, codex, Some(3_900_030), "app-server", 802),
                (2, ChildKind::Thread, None, "phase 1 impl", 803),
                (2, ChildKind::Thread, None, "phase 1 review", 804),
                (1, claude, Some(3_900_050), "--print review", 805),
                (0, ChildKind::Subagent, None, "Review phase 1", 900),
                (1, ChildKind::Subagent, None, "Explore the tests", 950),
                (2, ChildKind::Shell, Some(3_950_000), "List the tests", 960),
                (0, ChildKind::Shell, Some(3_960_000), "Build it", 970),
            ]
        );
        assert_eq!(
            cell(&rows, TRUNK),
            [
                (0, codex, Some(3_769_600), "app-server", 650),
                (1, ChildKind::Thread, None, "trunk mesh", 660),
            ]
        );
        assert_eq!(cell(&rows, ARRANGE), []);
        assert_eq!(cell(&rows, TERMINAL_CODEX), []);
    }

    /// A parent link that loops back to the agent ends the walk at the
    /// agent, and a Claude Code process with no record or arguments is
    /// called `claude`.
    #[test]
    fn a_loop_in_the_process_table_ends_the_walk() {
        let looped = ProcessEntry {
            parent: Some(20),
            ..detailed(10, 1, "claude", 5, &["claude"])
        };
        let processes = [looped, detailed(20, 10, "claude", 6, &["claude"])];
        let mut rows = [agent_row(10, 5)];

        attach_children(&mut rows, &processes, &TreeSources::default());

        assert_eq!(
            cell(&rows, 10),
            [(0, ChildKind::Process(Agent::Claude), Some(20), "claude", 6)]
        );
    }

    /// A chain of agents deeper than [`TREE_DEPTH_LIMIT`] is followed to
    /// the limit and no further.
    #[test]
    fn the_walk_stops_at_the_depth_limit() {
        let mut processes = vec![detailed(100, 1, "claude", 1, &["claude"])];
        processes.extend((101..=130).map(|pid| detailed(pid, pid - 1, "claude", 2, &["claude"])));
        let mut rows = [agent_row(100, 1)];

        attach_children(&mut rows, &processes, &TreeSources::default());

        let depths: Vec<u8> = rows[0].children.iter().map(|child| child.depth).collect();
        assert_eq!(depths, (0..TREE_DEPTH_LIMIT).collect::<Vec<_>>());
    }
}
