//! Which processes are agents someone can talk to, and which agent
//! opened a session tmux holds.
//!
//! An agent counts unless a process above it is another agent: under
//! another agent it is a delegate that agent started, and its cell's
//! tree shows it instead. An agent a tmux server holds counts too, since
//! someone can attach to it; when another agent's transcript shows that
//! agent opening the tmux session, [`pick_launcher`] names it as the
//! session's launcher, and the session is no longer top level.
//!
//! A Claude Code session is found through the record it writes for its
//! process, and counts only while that process is alive and is still
//! Claude Code. A Codex session has no record, so an interactive Codex
//! is any `codex` process that is not an app server. It is named for the
//! thread it started with: the first thread an interactive Codex created
//! in its directory from its start until [`CODEX_THREAD_START_WINDOW`]
//! later, and before the next one started there. The one app server
//! that counts is the macOS desktop app's, which stands for the app.
//!
//! Everything here is a pure function over a process table, the
//! session records, the Codex threads and the shell calls read from
//! transcripts, so the tests drive it from fixtures.

use std::collections::HashMap;
use std::ops::RangeInclusive;
use std::path::Path;
use std::path::PathBuf;

use serde::Deserialize;

use super::Agent;
use super::AgentRow;
use super::codex::CodexThread;
use super::tmux;
use super::transcript::BashCall;
use crate::constants::CALL_LOOKAHEAD;
use crate::constants::CALL_LOOKBACK;
use crate::constants::CLAUDE_AGENT;
use crate::constants::CODEX_AGENT;
use crate::constants::CODEX_APP_SERVER_ARGUMENT;
use crate::constants::CODEX_DESKTOP_APP;
use crate::constants::CODEX_THREAD_START_SLACK;
use crate::constants::CODEX_THREAD_START_WINDOW;
use crate::constants::HOME_ABBREVIATION;
use crate::constants::MISSING_VALUE;
use crate::constants::SESSION_ID_PREFIX_LENGTH;
use crate::constants::TMUX_NEW_SESSION;
use crate::constants::TMUX_SERVER_NAMES;

/// One process as the classification reads it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ProcessEntry {
    /// The process id.
    pub(super) pid:        u32,
    /// The parent's process id; none for the first process.
    pub(super) parent:     Option<u32>,
    /// The process name: `/proc/<pid>/comm` on Linux, which a process
    /// may set for itself.
    pub(super) name:       String,
    /// The whole command line, program first. Empty where it was not
    /// read.
    pub(super) arguments:  Vec<String>,
    /// When the process started, in unix seconds.
    pub(super) started:    u64,
    /// The process's working directory, where it could be read.
    pub(super) directory:  Option<PathBuf>,
    /// The Claude Code process named by the process's `CLAUDE_PID`,
    /// read for `codex` processes alone.
    pub(super) claude_pid: Option<u32>,
}

impl ProcessEntry {
    /// Whether this is Claude Code. The name alone is not enough on
    /// macOS, where the name comes from the executable's path and the
    /// installed executable is named for its version, so the program
    /// named on the command line counts as well.
    pub(super) fn is_claude(&self) -> bool {
        self.name == CLAUDE_AGENT
            || self
                .arguments
                .first()
                .and_then(|program| Path::new(program).file_name())
                .is_some_and(|program| program == CLAUDE_AGENT)
    }

    /// Whether this is any `codex` process, app server or not.
    pub(super) fn is_codex(&self) -> bool { self.name == CODEX_AGENT }

    /// Whether this is a Claude Code or Codex process of any kind.
    pub(super) fn is_agent(&self) -> bool { self.is_claude() || self.is_codex() }

    /// Whether this is a tmux server.
    pub(super) fn is_tmux_server(&self) -> bool { TMUX_SERVER_NAMES.contains(&self.name.as_str()) }

    /// Whether this `codex` serves a client rather than taking input
    /// itself.
    pub(super) fn is_app_server(&self) -> bool {
        self.arguments
            .iter()
            .skip(1)
            .any(|argument| argument == CODEX_APP_SERVER_ARGUMENT)
    }

    /// The command line after the program, on one line; none when the
    /// command line was not read or names the program alone.
    pub(super) fn arguments_label(&self) -> Option<String> {
        let arguments = self.arguments.get(1..).unwrap_or_default().join(" ");
        (!arguments.is_empty()).then_some(arguments)
    }
}

/// The part of a Claude Code session record the summary reads. The
/// records hold more, which is ignored.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(super) struct SessionRecord {
    /// The session's process.
    pub(super) pid:        u32,
    /// The session id, whose start names a session with no name.
    #[serde(default)]
    pub(super) session_id: String,
    /// The directory the session was started in.
    #[serde(default)]
    pub(super) cwd:        Option<PathBuf>,
    /// The session's name, where it has one.
    #[serde(default)]
    pub(super) name:       Option<String>,
    /// `idle`, `busy` or `shell`, where the record says.
    #[serde(default)]
    pub(super) status:     Option<String>,
}

impl SessionRecord {
    /// What a row calls the session: its name, else the start of its id.
    pub(super) fn label(&self) -> String {
        self.name
            .clone()
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| {
                self.session_id
                    .chars()
                    .take(SESSION_ID_PREFIX_LENGTH)
                    .collect()
            })
    }
}

/// The agents among `processes` that no other agent started, oldest
/// first, with pid breaking a tie. An interactive Codex is named for its
/// thread among `codex_threads`, and directories are written against
/// `home`. Every row is top level and runs nothing until the scan says
/// otherwise.
pub(super) fn agent_rows(
    processes: &[ProcessEntry],
    sessions: &[SessionRecord],
    codex_threads: &[CodexThread],
    home: Option<&Path>,
) -> Vec<AgentRow> {
    let table = pid_table(processes);
    let threads = startup_threads(&interactive_codex(&table, processes), codex_threads);
    let mut rows: Vec<AgentRow> = sessions
        .iter()
        .filter_map(|session| claude_row(&table, session, home))
        .chain(processes.iter().filter_map(|process| {
            codex_row(&table, process, threads.get(&process.pid).copied(), home)
        }))
        .collect();
    rows.sort_by_key(|row| (row.started, row.pid));
    rows
}

/// The creation times, in unix milliseconds, that a thread must fall in
/// to be the one some listed interactive Codex in `processes` started
/// with; none when there is no such Codex.
pub(super) fn codex_thread_window(processes: &[ProcessEntry]) -> Option<RangeInclusive<u64>> {
    let table = pid_table(processes);
    let windows: Vec<_> = interactive_codex(&table, processes)
        .into_iter()
        .map(thread_window)
        .collect();
    let from = windows.iter().map(|window| *window.start()).min()?;
    let until = windows.iter().map(|window| *window.end()).max()?;
    Some(from..=until)
}

/// `processes` by pid.
pub(super) fn pid_table(processes: &[ProcessEntry]) -> HashMap<u32, &ProcessEntry> {
    processes
        .iter()
        .map(|process| (process.pid, process))
        .collect()
}

/// The `codex` processes under no other agent that are not app servers.
fn interactive_codex<'a>(
    table: &HashMap<u32, &ProcessEntry>,
    processes: &'a [ProcessEntry],
) -> Vec<&'a ProcessEntry> {
    processes
        .iter()
        .filter(|process| {
            process.is_codex() && !process.is_app_server() && !under_an_agent(table, process)
        })
        .collect()
}

/// The creation times, in unix milliseconds, of a thread `process`
/// could have started with: from [`CODEX_THREAD_START_SLACK`] before its
/// recorded start to [`CODEX_THREAD_START_WINDOW`] after.
fn thread_window(process: &ProcessEntry) -> RangeInclusive<u64> {
    let started = process.started.saturating_mul(1_000);
    let slack = u64::try_from(CODEX_THREAD_START_SLACK.as_millis()).unwrap_or(u64::MAX);
    let window = u64::try_from(CODEX_THREAD_START_WINDOW.as_millis()).unwrap_or(u64::MAX);
    started.saturating_sub(slack)..=started.saturating_add(window)
}

/// The thread each Codex in `codex` started with, by pid.
///
/// A thread belongs to the Codex in its directory that started last
/// before it within [`thread_window`], so a Codex that resumed an old
/// thread, and so created none, does not take the first thread of one
/// started after it. Of the threads a Codex owns, the first is the one
/// it started with.
fn startup_threads<'a>(
    codex: &[&ProcessEntry],
    threads: &'a [CodexThread],
) -> HashMap<u32, &'a CodexThread> {
    let mut first: HashMap<u32, &CodexThread> = HashMap::new();
    for thread in threads {
        let owner = codex
            .iter()
            .filter(|process| {
                process.directory.as_deref() == Some(thread.cwd.as_path())
                    && thread_window(process).contains(&thread.created_ms)
            })
            .max_by_key(|process| (process.started, process.pid));
        if let Some(owner) = owner {
            let kept = first.entry(owner.pid).or_insert(thread);
            if thread.created_ms < kept.created_ms {
                *kept = thread;
            }
        }
    }
    first
}

/// The row for `session`, when its process is alive, is Claude Code,
/// and is under no other agent.
fn claude_row(
    table: &HashMap<u32, &ProcessEntry>,
    session: &SessionRecord,
    home: Option<&Path>,
) -> Option<AgentRow> {
    let process = table.get(&session.pid)?;
    if !process.is_claude() || under_an_agent(table, process) {
        return None;
    }
    let directory = session.cwd.as_deref().or(process.directory.as_deref());
    Some(AgentRow {
        agent:       Agent::Claude,
        name:        session.label(),
        status:      session.status.clone(),
        started:     process.started,
        pid:         process.pid,
        desktop:     None,
        directory:   directory_label(directory, home),
        launched_by: None,
        children:    Vec::new(),
    })
}

/// The row for `process`, when it is an interactive Codex under no other
/// agent or the desktop app's app server. An interactive Codex is named for
/// `thread`, the one it started with, else for its command line after
/// `codex`, else for its pid.
fn codex_row(
    table: &HashMap<u32, &ProcessEntry>,
    process: &ProcessEntry,
    thread: Option<&CodexThread>,
    home: Option<&Path>,
) -> Option<AgentRow> {
    if !process.is_codex() || under_an_agent(table, process) {
        return None;
    }
    let name = if process.is_app_server() {
        let parent = process.parent.and_then(|parent| table.get(&parent))?;
        if parent.name != CODEX_DESKTOP_APP {
            return None;
        }
        CODEX_DESKTOP_APP.to_string()
    } else if let Some(label) = thread.and_then(CodexThread::label) {
        label
    } else {
        process
            .arguments_label()
            .unwrap_or_else(|| format!("pid {}", process.pid))
    };
    Some(AgentRow {
        agent: Agent::Codex,
        name,
        status: None,
        started: process.started,
        pid: process.pid,
        desktop: None,
        directory: directory_label(process.directory.as_deref(), home),
        launched_by: None,
        children: Vec::new(),
    })
}

/// Whether a process above `process` is an agent.
pub(super) fn under_an_agent(table: &HashMap<u32, &ProcessEntry>, process: &ProcessEntry) -> bool {
    ancestors(table, process).any(ProcessEntry::is_agent)
}

/// Whether the process `pid` is held by a tmux server: a process above
/// it is one.
pub(super) fn held_by_tmux(processes: &[ProcessEntry], pid: u32) -> bool {
    let table = pid_table(processes);
    table
        .get(&pid)
        .is_some_and(|process| ancestors(&table, process).any(ProcessEntry::is_tmux_server))
}

/// The processes above `process`, its parent first.
///
/// The walk stops at a parent missing from the table, and after as
/// many steps as the table has processes, so a parent link that loops
/// cannot hold it.
pub(super) fn ancestors<'a>(
    table: &'a HashMap<u32, &'a ProcessEntry>,
    process: &ProcessEntry,
) -> impl Iterator<Item = &'a ProcessEntry> {
    let mut parent = process.parent;
    std::iter::from_fn(move || {
        let ancestor = *table.get(&parent?)?;
        parent = ancestor.parent;
        Some(ancestor)
    })
    .take(table.len())
}

/// A session tmux holds, as the search for the agent that opened it
/// reads it.
#[derive(Clone, Copy, Debug)]
pub(super) struct HeldSession<'a> {
    /// The session's name as its row shows it.
    pub(super) name:       &'a str,
    /// Whether `name` is one the session was given, as a Claude Code
    /// session's own name is, rather than a stand-in such as the start
    /// of its id or a Codex thread's name. Only a given name rules out a
    /// call that opened a tmux session under another literal name.
    pub(super) given_name: bool,
    /// The directory it runs in, written out in full, where known.
    pub(super) directory:  Option<&'a Path>,
    /// When its process started, in unix seconds.
    pub(super) started:    u64,
}

impl HeldSession<'_> {
    /// The span of unix milliseconds the call that opened the session
    /// could have been written in: from [`CALL_LOOKBACK`] before its
    /// process started to [`CALL_LOOKAHEAD`] after.
    pub(super) fn call_span(&self) -> RangeInclusive<u64> { call_span(self.started) }
}

/// The span of unix milliseconds the call that started a process at
/// `started`, in unix seconds, could have been written in: from
/// [`CALL_LOOKBACK`] before to [`CALL_LOOKAHEAD`] after.
pub(super) fn call_span(started: u64) -> RangeInclusive<u64> {
    let started = started.saturating_mul(1_000);
    let lookback = u64::try_from(CALL_LOOKBACK.as_millis()).unwrap_or(u64::MAX);
    let lookahead = u64::try_from(CALL_LOOKAHEAD.as_millis()).unwrap_or(u64::MAX);
    started.saturating_sub(lookback)..=started.saturating_add(lookahead)
}

/// The agent whose shell call opened `held`, among `calls`, each paired
/// with the pid of the agent that made it; none when no call opened a
/// tmux session in [`HeldSession::call_span`].
///
/// A call that opens only sessions named by a literal other than
/// `held`'s name is passed over. Among the rest, a call that names the
/// session or its directory wins over one that names neither -- a
/// session opened in a loop is named by a variable -- and among calls
/// alike in that, the latest does.
pub(super) fn pick_launcher(held: &HeldSession<'_>, calls: &[(u32, BashCall)]) -> Option<u32> {
    let span = held.call_span();
    let directory = held.directory.and_then(Path::to_str);
    calls
        .iter()
        .filter(|(_, call)| {
            span.contains(&call.at_ms)
                && call.command.contains(TMUX_NEW_SESSION)
                && !opens_another_session(held, &call.command)
        })
        .max_by_key(|(_, call)| {
            let named = mentions(&call.command, held.name)
                || directory.is_some_and(|directory| mentions(&call.command, directory));
            (named, call.at_ms)
        })
        .map(|(launcher, _)| *launcher)
}

/// Whether `command` opens only tmux sessions named by a literal other
/// than `held`'s name, so it cannot have opened `held`.
///
/// Only a name the session was given counts, and a call that mentions
/// that name anywhere is kept: a Claude Code session is named by its
/// own `-n`, which need not match the tmux session holding it.
fn opens_another_session(held: &HeldSession<'_>, command: &str) -> bool {
    if !held.given_name || mentions(command, held.name) {
        return false;
    }
    let opened = tmux::opened_sessions(command);
    !opened.is_empty()
        && opened
            .iter()
            .all(|name| name.as_deref().is_some_and(|name| name != held.name))
}

/// Whether `text` holds `word` standing on its own: with no letter,
/// digit, `-`, `_`, `.` or `/` right before or after it, so a session
/// named `trunk` is not found in `ui-trunk`, nor a directory in one of
/// its subdirectories.
fn mentions(text: &str, word: &str) -> bool {
    if word.is_empty() {
        return false;
    }
    let continues = |character: Option<char>| {
        character.is_some_and(|character| {
            character.is_alphanumeric() || matches!(character, '-' | '_' | '.' | '/')
        })
    };
    text.match_indices(word).any(|(start, _)| {
        !continues(text[..start].chars().next_back())
            && !continues(text[start + word.len()..].chars().next())
    })
}

/// `directory` as a row shows it: under `home` it starts with `~`, and
/// the home directory itself is `~`. A directory that could not be read
/// is [`MISSING_VALUE`].
fn directory_label(directory: Option<&Path>, home: Option<&Path>) -> String {
    let Some(directory) = directory else {
        return MISSING_VALUE.to_string();
    };
    match home.and_then(|home| directory.strip_prefix(home).ok()) {
        Some(rest) if rest.as_os_str().is_empty() => HOME_ABBREVIATION.to_string(),
        Some(rest) => Path::new(HOME_ABBREVIATION)
            .join(rest)
            .display()
            .to_string(),
        None => directory.display().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The home directory every fixture's paths sit under.
    const HOME: &str = "/home/natepiano";

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

    /// A process with its command line and directory read.
    fn detailed(
        pid: u32,
        parent: u32,
        name: &str,
        started: u64,
        arguments: &[&str],
        directory: &str,
    ) -> ProcessEntry {
        ProcessEntry {
            arguments: arguments
                .iter()
                .map(|argument| (*argument).to_string())
                .collect(),
            directory: Some(PathBuf::from(directory)),
            ..process(pid, parent, name, started)
        }
    }

    /// A named session record.
    fn session(pid: u32, name: &str, status: &str, cwd: &str) -> SessionRecord {
        SessionRecord {
            pid,
            session_id: format!("{pid:08}-0000-0000-0000-000000000000"),
            cwd: Some(PathBuf::from(cwd)),
            name: Some(name.to_string()),
            status: Some(status.to_string()),
        }
    }

    /// A terminal window running a shell under the user's systemd,
    /// ready for something to run under the shell.
    fn terminal(window: u32, shell: u32, started: u64) -> [ProcessEntry; 2] {
        [
            process(window, 2_246, ".ghostty-wrappe", started),
            process(shell, window, "zsh", started),
        ]
    }

    /// The names of `rows`, in order.
    fn names(rows: &[AgentRow]) -> Vec<&str> { rows.iter().map(|row| row.name.as_str()).collect() }

    /// natedev as `ps` showed it: sessions opened in terminal windows,
    /// sessions a tmux server holds for a tool, the processes those
    /// sessions started, a Codex delegate under one of them, and the
    /// Codex app servers systemd runs.
    fn natedev() -> (Vec<ProcessEntry>, Vec<SessionRecord>) {
        let mut processes = vec![
            process(1, 0, "systemd", 0),
            process(2_246, 1, "systemd", 10),
            process(3_261_729, 2_246, "tmux: server", 600),
            process(3_266_307, 3_261_729, "zsh", 610),
            detailed(3_266_367, 3_266_307, "claude", 611, &["claude"], HOME),
            process(3_336_767, 3_261_729, "zsh", 900),
            detailed(3_337_048, 3_336_767, "claude", 901, &["claude"], HOME),
            detailed(
                3_532_979,
                2_246,
                "codex",
                950,
                &["codex", "app-server", "--listen", "ws://127.0.0.1:43483"],
                HOME,
            ),
            process(3_561_145, 3_532_979, "codex-code-mode", 951),
            // A Codex delegate a Claude session started through a
            // shell, a script and python.
            process(3_911_067, 428_044, "zsh", 700),
            process(3_911_070, 3_911_067, "bash", 700),
            process(3_912_063, 3_911_070, "python3", 701),
            detailed(
                3_912_100,
                3_912_063,
                "codex",
                702,
                &["codex", "exec", "--json"],
                "/home/natepiano/rust/tool-based-ui",
            ),
        ];
        let windows = [
            (46_483, 46_536, 46_560, 100),
            (3_736_400, 3_736_490, 428_044, 800),
            (46_089, 46_141, 1_341_620, 300),
            (45_725, 45_880, 1_484_649, 400),
            (45_448, 45_494, 1_579_022, 500),
            (2_573_400, 2_573_434, 2_573_656, 850),
            (1_362_900, 1_362_979, 2_747_564, 700),
        ];
        for (window, shell, claude, started) in windows {
            processes.extend(terminal(window, shell, started));
            processes.push(detailed(
                claude,
                shell,
                "claude",
                started,
                &["claude"],
                HOME,
            ));
        }
        // A Claude session another Claude session started, with a
        // record of its own.
        processes.push(process(4_999_990, 1_341_620, "zsh", 950));
        processes.push(detailed(
            5_000_000,
            4_999_990,
            "claude",
            950,
            &["claude"],
            HOME,
        ));
        // An interactive Codex opened in a terminal of its own.
        processes.extend(terminal(4_000_000, 4_000_010, 960));
        processes.push(detailed(
            4_000_020,
            4_000_010,
            "codex",
            960,
            &["codex", "--model", "gpt-5"],
            "/home/natepiano/rust/handler",
        ));
        (processes, natedev_sessions())
    }

    /// The session records natedev's `~/.claude/sessions` held for the
    /// processes in [`natedev`].
    fn natedev_sessions() -> Vec<SessionRecord> {
        vec![
            session(46_560, "scarlett", "idle", "/etc/nixos"),
            session(
                428_044,
                "enh/handler",
                "busy",
                "/home/natepiano/rust/handler",
            ),
            session(
                1_341_620,
                "berth-fix",
                "idle",
                "/home/natepiano/rust/berth-fix",
            ),
            session(1_484_649, "natedev", "idle", "/etc/nixos"),
            session(
                1_579_022,
                "boss of bosses",
                "idle",
                "/home/natepiano/rust/hana_catalyst/docs/hana",
            ),
            session(2_573_656, "soft body physics investigation", "busy", HOME),
            session(
                2_747_564,
                "tmp cleanup then merge to berth and handler",
                "shell",
                "/home/natepiano/rust/cargo-handler",
            ),
            session(3_266_367, "tool-based-ui-trunk", "busy", HOME),
            session(3_337_048, "tool-based-ui-arrange", "shell", HOME),
            session(5_000_000, "nested", "busy", HOME),
            // A session whose process has ended.
            session(3_266_368, "tool-based-ui-show-beam", "idle", HOME),
        ]
    }

    /// The sessions opened in terminal windows and those a tmux server
    /// holds are listed, oldest first; the Claude and Codex sessions
    /// another Claude session started, the app servers and an ended
    /// session are not.
    #[test]
    fn natedev_lists_its_sessions_oldest_first() {
        let (processes, sessions) = natedev();

        let rows = agent_rows(&processes, &sessions, &[], Some(Path::new(HOME)));

        assert_eq!(
            names(&rows),
            [
                "scarlett",
                "berth-fix",
                "natedev",
                "boss of bosses",
                "tool-based-ui-trunk",
                "tmp cleanup then merge to berth and handler",
                "enh/handler",
                "soft body physics investigation",
                "tool-based-ui-arrange",
                "--model gpt-5",
            ]
        );
        assert!(rows.iter().all(|row| row.launched_by.is_none()));
        let handler = &rows[6];
        assert_eq!(handler.agent, Agent::Claude);
        assert_eq!(handler.status.as_deref(), Some("busy"));
        assert_eq!(handler.started, 800);
        assert_eq!(handler.pid, 428_044);
        assert_eq!(handler.directory, "~/rust/handler");
        assert_eq!(rows[7].directory, "~");
        assert_eq!(rows[0].directory, "/etc/nixos");
        let codex = &rows[9];
        assert_eq!(codex.agent, Agent::Codex);
        assert_eq!(codex.status, None);
        assert_eq!(codex.directory, "~/rust/handler");
    }

    /// A record whose pid now belongs to something other than Claude
    /// Code is a session that ended and had its pid handed out again.
    #[test]
    fn a_record_whose_pid_is_no_longer_claude_is_left_out() {
        let processes = [
            process(1, 0, "systemd", 0),
            detailed(500, 1, "vim", 10, &["vim"], HOME),
        ];
        let sessions = [session(500, "gone", "idle", HOME)];

        assert!(agent_rows(&processes, &sessions, &[], Some(Path::new(HOME))).is_empty());
    }

    /// A session with no name shows the start of its id.
    #[test]
    fn a_session_with_no_name_shows_the_start_of_its_id() {
        let processes = [
            process(1, 0, "systemd", 0),
            detailed(500, 1, "claude", 10, &["claude"], HOME),
        ];
        let sessions = [SessionRecord {
            name: None,
            status: None,
            ..session(500, "", "", HOME)
        }];

        let rows = agent_rows(&processes, &sessions, &[], Some(Path::new(HOME)));

        assert_eq!(names(&rows), ["00000500"]);
        assert_eq!(rows[0].status, None);
    }

    /// Started in the same second, the lower pid comes first.
    #[test]
    fn pid_breaks_a_tie_in_start_time() {
        let processes = [
            process(1, 0, "systemd", 0),
            detailed(700, 1, "claude", 10, &["claude"], HOME),
            detailed(600, 1, "claude", 10, &["claude"], HOME),
        ];
        let sessions = [
            session(700, "second", "idle", HOME),
            session(600, "first", "idle", HOME),
        ];

        let rows = agent_rows(&processes, &sessions, &[], Some(Path::new(HOME)));

        assert_eq!(names(&rows), ["first", "second"]);
    }

    /// A Codex started with no arguments is named by its pid.
    #[test]
    fn a_bare_codex_is_named_by_its_pid() {
        let processes = [
            process(1, 0, "systemd", 0),
            detailed(900, 1, "codex", 10, &["codex"], "/tmp"),
        ];

        let rows = agent_rows(&processes, &[], &[], Some(Path::new(HOME)));

        assert_eq!(names(&rows), ["pid 900"]);
        assert_eq!(rows[0].directory, "/tmp");
    }

    /// Each interactive Codex is named for the first thread created in
    /// its directory after it started: a renamed thread by its name, one
    /// with no name by its first prompt. A Codex that resumed a thread
    /// does not take the thread of one started after it, a delegate's
    /// Codex takes none, and a Codex with no thread keeps its pid.
    #[test]
    fn an_interactive_codex_is_named_for_the_thread_it_started() {
        let work = "/home/natepiano/rust/handler";
        let processes = [
            process(1, 0, "systemd", 0),
            detailed(10, 1, "codex", 100, &["codex", "resume"], work),
            detailed(20, 1, "codex", 110, &["codex"], work),
            detailed(30, 1, "codex", 120, &["codex"], HOME),
            detailed(40, 1, "codex", 130, &["codex"], "/tmp"),
            detailed(50, 1, "claude", 90, &["claude"], HOME),
            detailed(60, 50, "codex", 125, &["codex"], HOME),
        ];
        let thread = |cwd: &str, created_ms, name: Option<&str>, prompt: &str| CodexThread {
            cwd: PathBuf::from(cwd),
            created_ms,
            name: name.map(str::to_string),
            first_prompt: prompt.to_string(),
        };
        let threads = [
            thread(work, 111_800, Some("codex test"), ""),
            thread(work, 150_000, Some("after /new"), ""),
            thread(HOME, 121_500, None, "fix the\nbuild"),
            thread(HOME, 126_000, Some("delegate"), ""),
            thread("/tmp", 200_000, Some("too late"), ""),
        ];

        assert_eq!(codex_thread_window(&processes), Some(99_000..=190_000));
        let rows = agent_rows(&processes, &[], &threads, Some(Path::new(HOME)));

        assert_eq!(
            names(&rows),
            ["resume", "codex test", "fix the build", "pid 40"]
        );
    }

    /// The Mac as `ps` showed it: the desktop app running its Codex app
    /// server, a Claude Code session whose process is named for the
    /// installed version, and one a tmux server holds, which on macOS
    /// keeps the name `tmux` and is listed too.
    #[test]
    fn the_mac_lists_the_desktop_app_and_its_terminal_session() {
        let home = "/Users/natemccoy";
        let processes = [
            process(1, 0, "launchd", 0),
            process(76_073, 1, "ChatGPT", 1_000),
            detailed(
                76_130,
                76_073,
                "codex",
                1_005,
                &[
                    "/Applications/ChatGPT.app/Contents/Resources/codex",
                    "--enable",
                    "app-server",
                ],
                "/",
            ),
            process(80_000, 1, "iTerm2", 2_000),
            process(80_010, 80_000, "zsh", 2_000),
            detailed(80_020, 80_010, "2.1.282", 2_001, &["claude"], home),
            process(81_000, 1, "tmux", 2_100),
            process(81_010, 81_000, "zsh", 2_100),
            detailed(81_020, 81_010, "2.1.282", 2_101, &["claude"], home),
        ];
        let sessions = [
            session(80_020, "natemccoy-30", "idle", home),
            session(81_020, "worker", "busy", home),
        ];

        let rows = agent_rows(&processes, &sessions, &[], Some(Path::new(home)));

        assert_eq!(names(&rows), ["ChatGPT", "natemccoy-30", "worker"]);
        assert!(held_by_tmux(&processes, 81_020));
        assert!(!held_by_tmux(&processes, 80_020));
        let desktop = &rows[0];
        assert_eq!(desktop.agent, Agent::Codex);
        assert_eq!(desktop.status, None);
        assert_eq!(desktop.started, 1_005);
        assert_eq!(desktop.directory, "/");
        assert_eq!(rows[1].directory, "~");
    }

    /// A shell call written `offset_ms` milliseconds from [`LAUNCH`],
    /// running `command`.
    fn call(offset_ms: i64, command: &str) -> BashCall {
        BashCall {
            at_ms:       LAUNCH
                .saturating_mul(1_000)
                .saturating_add_signed(offset_ms),
            command:     command.to_string(),
            description: None,
        }
    }

    /// The unix second each held session in the launcher tests started.
    const LAUNCH: u64 = 1_790_000_000;

    /// A held session named `name`, started at [`LAUNCH`].
    fn named_session(name: &str) -> HeldSession<'_> {
        HeldSession {
            name,
            given_name: true,
            directory: Some(Path::new("/home/natepiano/rust/tool-based-ui-x")),
            started: LAUNCH,
        }
    }

    /// boss of bosses opened three sessions: trunk from a loop, whose
    /// call names it only through a variable, one second before it
    /// started; arrange by name four seconds before; geometry-material
    /// two seconds before. Another agent's calls around the same times
    /// open no tmux session, name another session, or come too late.
    #[test]
    fn the_launcher_is_the_agent_whose_call_opened_the_session() {
        let boss = 1_579_022;
        let other = 428_044;
        let held = |name| named_session(name);
        let loop_call = r#"for name in trunk; do tmux new-session -d -s "tool-based-ui-$name" zsh -ic claude; done"#;
        let trunk = [
            (other, call(-300_000, "tmux new-session -d -s scratch")),
            (other, call(-500, "cargo build")),
            (boss, call(-1_000, loop_call)),
            (other, call(6_000, "tmux new-session -d -s late")),
        ];
        assert_eq!(
            pick_launcher(&held("tool-based-ui-trunk"), &trunk),
            Some(boss)
        );

        let arrange = [
            (
                boss,
                call(
                    -4_000,
                    "tmux new-session -d -s tool-based-ui-arrange zsh -ic claude",
                ),
            ),
            (
                other,
                call(-1_000, "tmux new-session -d -s tool-based-ui-arranger"),
            ),
        ];
        assert_eq!(
            pick_launcher(&held("tool-based-ui-arrange"), &arrange),
            Some(boss)
        );

        let geometry = [(
            boss,
            call(
                -2_000,
                "tmux new-session -d -s tool-based-ui-geometry-material",
            ),
        )];
        assert_eq!(
            pick_launcher(&held("tool-based-ui-geometry-material"), &geometry),
            Some(boss)
        );

        let outside = [
            (
                boss,
                call(-700_000, "tmux new-session -d -s tool-based-ui-trunk"),
            ),
            (
                boss,
                call(5_001, "tmux new-session -d -s tool-based-ui-trunk"),
            ),
        ];
        assert_eq!(pick_launcher(&held("tool-based-ui-trunk"), &outside), None);
    }

    /// A call opening a session named by a literal other than the held
    /// session's name is passed over, even as the latest call; one
    /// naming it by that literal wins over a later call naming nothing.
    #[test]
    fn a_call_naming_another_session_is_passed_over() {
        let boss = 1_579_022;
        let other = 428_044;
        let held = named_session("tool-based-ui-arrange");

        let elsewhere = [
            (
                boss,
                call(-3_000, "tmux new-session -d -s $name zsh -ic claude"),
            ),
            (other, call(-1_000, "tmux new-session -d -s scratch zsh")),
        ];
        assert_eq!(pick_launcher(&held, &elsewhere), Some(boss));
        assert_eq!(pick_launcher(&held, &elsewhere[1..]), None);

        let own = [
            (
                boss,
                call(-4_000, "tmux new-session -d -s 'tool-based-ui-arrange' zsh"),
            ),
            (other, call(-1_000, r#"tmux new-session -d -s "$name" zsh"#)),
        ];
        assert_eq!(pick_launcher(&held, &own), Some(boss));
    }

    /// A session named by a variable, bare or quoted, matches by time
    /// alone, so the latest call wins.
    #[test]
    fn a_session_named_by_a_variable_matches_by_time() {
        let boss = 1_579_022;
        let other = 428_044;
        let calls = [
            (other, call(-3_000, "tmux new-session -d -s $name zsh")),
            (boss, call(-1_000, r#"tmux new-session -d -s "$name" zsh"#)),
        ];

        assert_eq!(
            pick_launcher(&named_session("tool-based-ui-trunk"), &calls),
            Some(boss)
        );
    }

    /// A literal tmux name other than the session's rules nothing out
    /// when the call names the session elsewhere, as Claude Code's `-n`
    /// does, or when the session has no name of its own to compare.
    #[test]
    fn another_literal_rules_out_only_a_session_it_cannot_be() {
        let boss = 1_579_022;
        let renamed = [(
            boss,
            call(-1_000, "tmux new-session -d -s w1 'claude -n fixer'"),
        )];
        assert_eq!(pick_launcher(&named_session("fixer"), &renamed), Some(boss));

        let unnamed = HeldSession {
            name: "b83cfc96",
            given_name: false,
            ..named_session("")
        };
        let worker = [(boss, call(-1_000, "tmux new-session -d -s worker claude"))];
        assert_eq!(pick_launcher(&unnamed, &worker), Some(boss));
    }

    /// A name or directory counts only standing on its own, not inside
    /// a longer name or a subdirectory.
    #[test]
    fn a_mention_stands_on_its_own() {
        assert!(mentions("tmux new-session -s trunk", "trunk"));
        assert!(mentions("--title='trunk'", "trunk"));
        assert!(!mentions("-s ui-trunk", "trunk"));
        assert!(!mentions("-s trunk2", "trunk"));
        assert!(mentions("-c /home/natepiano zsh", "/home/natepiano"));
        assert!(!mentions("-c /home/natepiano/rust", "/home/natepiano"));
        assert!(!mentions("anything", ""));
    }

    /// Only the home directory's own components count: a sibling that
    /// shares its spelling as a prefix is written out in full.
    #[test]
    fn only_paths_under_home_are_abbreviated() {
        let home = Some(Path::new(HOME));
        assert_eq!(directory_label(Some(Path::new(HOME)), home), "~");
        assert_eq!(
            directory_label(Some(Path::new("/home/natepiano/rust")), home),
            "~/rust"
        );
        assert_eq!(
            directory_label(Some(Path::new("/home/natepianoforte")), home),
            "/home/natepianoforte"
        );
        assert_eq!(directory_label(None, home), MISSING_VALUE);
        assert_eq!(directory_label(Some(Path::new(HOME)), None), HOME);
    }
}
