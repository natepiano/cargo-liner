//! Which processes are top-level agents.
//!
//! An agent is top level when nothing above it is another agent or a
//! tmux server. Under another agent it is a delegate that agent
//! started; under a tmux server it is a worker some tool drives
//! rather than a session someone opened. Both are left out.
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
//! session records and the Codex threads, so the tests drive it from
//! fixtures.

use std::collections::HashMap;
use std::ops::RangeInclusive;
use std::path::Path;
use std::path::PathBuf;

use serde::Deserialize;

use super::Agent;
use super::AgentRow;
use super::codex::CodexThread;
use crate::constants::CLAUDE_AGENT;
use crate::constants::CODEX_AGENT;
use crate::constants::CODEX_APP_SERVER_ARGUMENT;
use crate::constants::CODEX_DESKTOP_APP;
use crate::constants::CODEX_THREAD_START_SLACK;
use crate::constants::CODEX_THREAD_START_WINDOW;
use crate::constants::HOME_ABBREVIATION;
use crate::constants::MISSING_VALUE;
use crate::constants::SESSION_ID_PREFIX_LENGTH;
use crate::constants::TMUX_SERVER_NAMES;

/// One process as the classification reads it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ProcessEntry {
    /// The process id.
    pub(super) pid:       u32,
    /// The parent's process id; none for the first process.
    pub(super) parent:    Option<u32>,
    /// The process name: `/proc/<pid>/comm` on Linux, which a process
    /// may set for itself.
    pub(super) name:      String,
    /// The whole command line, program first. Empty where it was not
    /// read.
    pub(super) arguments: Vec<String>,
    /// When the process started, in unix seconds.
    pub(super) started:   u64,
    /// The process's working directory, where it could be read.
    pub(super) directory: Option<PathBuf>,
}

impl ProcessEntry {
    /// Whether this is Claude Code. The name alone is not enough on
    /// macOS, where the name comes from the executable's path and the
    /// installed executable is named for its version, so the program
    /// named on the command line counts as well.
    fn is_claude(&self) -> bool {
        self.name == CLAUDE_AGENT
            || self
                .arguments
                .first()
                .and_then(|program| Path::new(program).file_name())
                .is_some_and(|program| program == CLAUDE_AGENT)
    }

    /// Whether this is any `codex` process, app server or not.
    fn is_codex(&self) -> bool { self.name == CODEX_AGENT }

    /// Whether this is a Claude Code or Codex process of any kind.
    fn is_agent(&self) -> bool { self.is_claude() || self.is_codex() }

    /// Whether this is a tmux server.
    fn is_tmux_server(&self) -> bool { TMUX_SERVER_NAMES.contains(&self.name.as_str()) }

    /// Whether this `codex` serves a client rather than taking input
    /// itself.
    fn is_app_server(&self) -> bool {
        self.arguments
            .iter()
            .skip(1)
            .any(|argument| argument == CODEX_APP_SERVER_ARGUMENT)
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

/// The top-level agents among `processes`, oldest first, with pid
/// breaking a tie. An interactive Codex is named for its thread among
/// `codex_threads`, and directories are written against `home`.
pub(super) fn top_level_rows(
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
/// to be the one some top-level interactive Codex in `processes`
/// started with; none when there is no such Codex.
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
fn pid_table(processes: &[ProcessEntry]) -> HashMap<u32, &ProcessEntry> {
    processes
        .iter()
        .map(|process| (process.pid, process))
        .collect()
}

/// The top-level `codex` processes that are not app servers.
fn interactive_codex<'a>(
    table: &HashMap<u32, &ProcessEntry>,
    processes: &'a [ProcessEntry],
) -> Vec<&'a ProcessEntry> {
    processes
        .iter()
        .filter(|process| {
            process.is_codex() && !process.is_app_server() && is_top_level(table, process)
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
/// and is top level.
fn claude_row(
    table: &HashMap<u32, &ProcessEntry>,
    session: &SessionRecord,
    home: Option<&Path>,
) -> Option<AgentRow> {
    let process = table.get(&session.pid)?;
    if !process.is_claude() || !is_top_level(table, process) {
        return None;
    }
    let name = session
        .name
        .clone()
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| {
            session
                .session_id
                .chars()
                .take(SESSION_ID_PREFIX_LENGTH)
                .collect()
        });
    let directory = session.cwd.as_deref().or(process.directory.as_deref());
    Some(AgentRow {
        agent: Agent::Claude,
        name,
        status: session.status.clone(),
        started: process.started,
        pid: process.pid,
        directory: directory_label(directory, home),
    })
}

/// The row for `process`, when it is a top-level interactive Codex or
/// the desktop app's app server. An interactive Codex is named for
/// `thread`, the one it started with, else for its command line after
/// `codex`, else for its pid.
fn codex_row(
    table: &HashMap<u32, &ProcessEntry>,
    process: &ProcessEntry,
    thread: Option<&CodexThread>,
    home: Option<&Path>,
) -> Option<AgentRow> {
    if !process.is_codex() || !is_top_level(table, process) {
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
        let arguments = process.arguments.get(1..).unwrap_or_default().join(" ");
        if arguments.is_empty() {
            format!("pid {}", process.pid)
        } else {
            arguments
        }
    };
    Some(AgentRow {
        agent: Agent::Codex,
        name,
        status: None,
        started: process.started,
        pid: process.pid,
        directory: directory_label(process.directory.as_deref(), home),
    })
}

/// Whether no process above `process` is an agent or a tmux server.
///
/// The walk stops at a parent missing from the table, and after as
/// many steps as the table has processes, so a parent link that loops
/// cannot hold it.
fn is_top_level(table: &HashMap<u32, &ProcessEntry>, process: &ProcessEntry) -> bool {
    let mut parent = process.parent;
    for _ in 0..table.len() {
        let Some(ancestor) = parent.and_then(|pid| table.get(&pid)) else {
            return true;
        };
        if ancestor.is_agent() || ancestor.is_tmux_server() {
            return false;
        }
        parent = ancestor.parent;
    }
    true
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

    /// The sessions opened in terminal windows are listed, oldest
    /// first; those a tmux server holds, the Claude and Codex sessions
    /// another Claude session started, the app servers and an ended
    /// session are not.
    #[test]
    fn natedev_lists_its_terminal_sessions_oldest_first() {
        let (processes, sessions) = natedev();

        let rows = top_level_rows(&processes, &sessions, &[], Some(Path::new(HOME)));

        assert_eq!(
            names(&rows),
            [
                "scarlett",
                "berth-fix",
                "natedev",
                "boss of bosses",
                "tmp cleanup then merge to berth and handler",
                "enh/handler",
                "soft body physics investigation",
                "--model gpt-5",
            ]
        );
        let handler = &rows[5];
        assert_eq!(handler.agent, Agent::Claude);
        assert_eq!(handler.status.as_deref(), Some("busy"));
        assert_eq!(handler.started, 800);
        assert_eq!(handler.pid, 428_044);
        assert_eq!(handler.directory, "~/rust/handler");
        assert_eq!(rows[6].directory, "~");
        assert_eq!(rows[0].directory, "/etc/nixos");
        let codex = &rows[7];
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

        assert!(top_level_rows(&processes, &sessions, &[], Some(Path::new(HOME))).is_empty());
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

        let rows = top_level_rows(&processes, &sessions, &[], Some(Path::new(HOME)));

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

        let rows = top_level_rows(&processes, &sessions, &[], Some(Path::new(HOME)));

        assert_eq!(names(&rows), ["first", "second"]);
    }

    /// A Codex started with no arguments is named by its pid.
    #[test]
    fn a_bare_codex_is_named_by_its_pid() {
        let processes = [
            process(1, 0, "systemd", 0),
            detailed(900, 1, "codex", 10, &["codex"], "/tmp"),
        ];

        let rows = top_level_rows(&processes, &[], &[], Some(Path::new(HOME)));

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
        let rows = top_level_rows(&processes, &[], &threads, Some(Path::new(HOME)));

        assert_eq!(
            names(&rows),
            ["resume", "codex test", "fix the build", "pid 40"]
        );
    }

    /// The Mac as `ps` showed it: the desktop app running its Codex app
    /// server, a Claude Code session whose process is named for the
    /// installed version, and one a tmux server holds, which on macOS
    /// keeps the name `tmux`.
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

        let rows = top_level_rows(&processes, &sessions, &[], Some(Path::new(home)));

        assert_eq!(names(&rows), ["ChatGPT", "natemccoy-30"]);
        let desktop = &rows[0];
        assert_eq!(desktop.agent, Agent::Codex);
        assert_eq!(desktop.status, None);
        assert_eq!(desktop.started, 1_005);
        assert_eq!(desktop.directory, "/");
        assert_eq!(rows[1].directory, "~");
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
