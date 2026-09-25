//! Reading this machine: the session records Claude Code writes under
//! `~/.claude/sessions`, the process table, the threads interactive
//! Codex sessions started, and the transcripts that name what each
//! agent is running and which agent opened a session tmux holds, handed
//! to [`classify`] and [`tree`].

use std::collections::HashMap;
use std::collections::HashSet;
use std::ffi::OsString;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::time::SystemTime;

use sysinfo::Pid;
use sysinfo::Process;
use sysinfo::ProcessRefreshKind;
use sysinfo::ProcessesToUpdate;
use sysinfo::System;
use sysinfo::UpdateKind;

use super::Agent;
use super::AgentRow;
use super::classify;
use super::classify::HeldSession;
use super::classify::ProcessEntry;
use super::classify::SessionRecord;
use super::codex;
use super::transcript;
use super::transcript::BashCall;
use super::transcript::Subagent;
use super::tree;
use super::tree::ShellCall;
use super::tree::ThreadEntry;
use super::tree::TreeSources;
use super::unix_now;
use crate::constants::CALL_SEARCH_SETTLE;
use crate::constants::CLAUDE_AGENT;
use crate::constants::CLAUDE_DIRNAME;
use crate::constants::CLAUDE_PID_VARIABLE;
use crate::constants::CLAUDE_SESSIONS_DIRNAME;
use crate::constants::CODEX_AGENT;
use crate::constants::CODEX_DIRNAME;
use crate::constants::CODEX_SESSIONS_DIRNAME;
use crate::constants::PROC_DIRNAME;
use crate::constants::PROC_FD_DIRNAME;
use crate::constants::PROJECTS_DIRNAME;
use crate::constants::SESSION_RECORD_EXTENSION;

/// A process as a cache knows it: its pid and start, so a pid handed
/// out again is a new key.
type ProcessKey = (u32, u64);

/// Scans this machine for its agents and what each is running.
#[derive(Debug)]
pub(super) struct LocalScanner {
    /// This machine's home directory: where the session records live,
    /// and what a row's directory is written against.
    home:        Option<PathBuf>,
    /// The call found for each shell wrapper, or none found once the
    /// wrapper was [`CALL_SEARCH_SETTLE`] old.
    shell_calls: HashMap<ProcessKey, Option<ShellCall>>,
    /// The launcher found for each session tmux holds, by the launcher's
    /// pid and start, or none found once the session was
    /// [`CALL_SEARCH_SETTLE`] old.
    launchers:   HashMap<ProcessKey, Option<ProcessKey>>,
}

impl LocalScanner {
    /// A scanner of the user running this process.
    pub(super) fn new() -> Self {
        Self {
            home:        dirs::home_dir(),
            shell_calls: HashMap::new(),
            launchers:   HashMap::new(),
        }
    }

    /// Every agent running now, oldest first, with what it is running.
    ///
    /// Each scan reads a fresh process table, so a pid handed out again
    /// since the last scan cannot keep the command line or directory of
    /// the process that had it before.
    pub(super) fn scan(&mut self) -> Vec<AgentRow> {
        let home = self.home.clone();
        let sessions = home
            .as_deref()
            .map(|home| read_sessions(&home.join(CLAUDE_DIRNAME).join(CLAUDE_SESSIONS_DIRNAME)))
            .unwrap_or_default();
        let processes = process_table(&sessions);
        // Codex's database is opened only while an interactive Codex runs,
        // and read only for the span its threads could have been created in.
        let codex_threads = home
            .as_deref()
            .zip(classify::codex_thread_window(&processes))
            .map(|(home, window)| codex::read_threads(&home.join(CODEX_DIRNAME), &window))
            .unwrap_or_default();
        let mut rows = classify::agent_rows(&processes, &sessions, &codex_threads, home.as_deref());
        let table = classify::pid_table(&processes);
        let transcripts: HashMap<u32, PathBuf> = home
            .as_deref()
            .map(|home| home.join(CLAUDE_DIRNAME).join(PROJECTS_DIRNAME))
            .map(|projects| {
                sessions
                    .iter()
                    .filter(|session| {
                        table
                            .get(&session.pid)
                            .is_some_and(|process| process.is_claude())
                    })
                    .filter_map(|session| {
                        let cwd = session.cwd.as_deref()?;
                        Some((
                            session.pid,
                            transcript::transcript_path(&projects, cwd, &session.session_id),
                        ))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let now = unix_now();
        self.find_launchers(&processes, &sessions, &transcripts, &mut rows, now);
        let subagents: HashMap<u32, Vec<Subagent>> = transcripts
            .iter()
            .map(|(pid, path)| (*pid, transcript::running_subagents(path, SystemTime::now())))
            .collect();
        let shell_calls = self.find_shell_calls(&processes, &transcripts, &subagents, now);
        let threads = home
            .as_deref()
            .map(|home| app_server_threads(&processes, &home.join(CODEX_DIRNAME)))
            .unwrap_or_default();
        let sources = TreeSources {
            sessions: &sessions,
            subagents,
            shell_calls,
            threads,
        };
        tree::attach_children(&mut rows, &processes, &sources);
        rows
    }

    /// Set `launched_by` on each of `rows` a tmux server holds, from the
    /// transcripts of the Claude Code rows started before it.
    fn find_launchers(
        &mut self,
        processes: &[ProcessEntry],
        sessions: &[SessionRecord],
        transcripts: &HashMap<u32, PathBuf>,
        rows: &mut [AgentRow],
        now: u64,
    ) {
        let listed: HashSet<ProcessKey> = rows.iter().map(|row| (row.pid, row.started)).collect();
        let mut seen = HashSet::new();
        let mut found = Vec::new();
        for (index, row) in rows.iter().enumerate() {
            if !classify::held_by_tmux(processes, row.pid) {
                continue;
            }
            let key = (row.pid, row.started);
            seen.insert(key);
            let launcher = if let Some(cached) = self.launchers.get(&key) {
                *cached
            } else {
                let launcher = search_launcher(processes, sessions, transcripts, rows, row);
                if launcher.is_some() || settled(row.started, now) {
                    self.launchers.insert(key, launcher);
                }
                launcher
            };
            // A launcher no longer listed leaves the session top level, as
            // a scan starting afresh would.
            if let Some(launcher) = launcher.filter(|launcher| listed.contains(launcher)) {
                found.push((index, launcher.0));
            }
        }
        self.launchers.retain(|key, _| seen.contains(key));
        for (index, launcher) in found {
            rows[index].launched_by = Some(launcher);
        }
    }

    /// The call that started each shell wrapper under a Claude Code
    /// process with a transcript, by the wrapper's pid.
    fn find_shell_calls(
        &mut self,
        processes: &[ProcessEntry],
        transcripts: &HashMap<u32, PathBuf>,
        subagents: &HashMap<u32, Vec<Subagent>>,
        now: u64,
    ) -> HashMap<u32, ShellCall> {
        let mut seen = HashSet::new();
        let mut found = HashMap::new();
        for process in processes {
            let Some(main) = process.parent.and_then(|parent| transcripts.get(&parent)) else {
                continue;
            };
            let Some(command) = tree::shell_command(process) else {
                continue;
            };
            let key = (process.pid, process.started);
            seen.insert(key);
            let call = if let Some(cached) = self.shell_calls.get(&key) {
                cached.clone()
            } else {
                let theirs = process
                    .parent
                    .and_then(|parent| subagents.get(&parent))
                    .map_or(&[][..], Vec::as_slice);
                let call = search_shell_call(process, &command, main, theirs);
                if call.is_some() || settled(process.started, now) {
                    self.shell_calls.insert(key, call.clone());
                }
                call
            };
            if let Some(call) = call {
                found.insert(process.pid, call);
            }
        }
        self.shell_calls.retain(|key, _| seen.contains(key));
        found
    }
}

/// The launcher of `held`, a row tmux holds, by its pid and start: the
/// Claude Code row among `rows` started before it whose transcript
/// holds the call that opened it.
fn search_launcher(
    processes: &[ProcessEntry],
    sessions: &[SessionRecord],
    transcripts: &HashMap<u32, PathBuf>,
    rows: &[AgentRow],
    held: &AgentRow,
) -> Option<ProcessKey> {
    let directory = sessions
        .iter()
        .find(|session| session.pid == held.pid)
        .and_then(|session| session.cwd.clone())
        .or_else(|| {
            processes
                .iter()
                .find(|process| process.pid == held.pid)?
                .directory
                .clone()
        });
    let session = HeldSession {
        name:      &held.name,
        directory: directory.as_deref(),
        started:   held.started,
    };
    let span = session.call_span();
    let calls: Vec<(u32, BashCall)> = rows
        .iter()
        .filter(|other| {
            other.agent == Agent::Claude && other.pid != held.pid && other.started < held.started
        })
        .filter_map(|other| Some((other.pid, transcripts.get(&other.pid)?)))
        .flat_map(|(pid, path)| {
            transcript::bash_calls(path, &span)
                .into_iter()
                .map(move |call| (pid, call))
        })
        .collect();
    let launcher = classify::pick_launcher(&session, &calls)?;
    rows.iter()
        .find(|other| other.pid == launcher)
        .map(|other| (other.pid, other.started))
}

/// The call that started the shell wrapper `process`, running `command`,
/// among the calls in `main`, its session's transcript, and in the
/// transcripts of `subagents`, the session's running subagents.
fn search_shell_call(
    process: &ProcessEntry,
    command: &str,
    main: &Path,
    subagents: &[Subagent],
) -> Option<ShellCall> {
    let span = classify::call_span(process.started);
    let theirs: Vec<(String, Vec<BashCall>)> = subagents
        .iter()
        .map(|agent| {
            (
                agent.id.clone(),
                transcript::bash_calls(&agent.transcript, &span),
            )
        })
        .collect();
    let own = transcript::bash_calls(main, &span);
    tree::match_shell_call(command, process.started, &own, &theirs)
}

/// Whether a process started at `started` is old enough, at `now`, that
/// a search that found nothing for it will find nothing later.
const fn settled(started: u64, now: u64) -> bool {
    now.saturating_sub(started) >= CALL_SEARCH_SETTLE.as_secs()
}

/// The threads each Codex app server in `processes` holds open, by its
/// pid, named from the thread database in `codex_dir`. Only Linux lists
/// a process's open files, so elsewhere there are none.
fn app_server_threads(
    processes: &[ProcessEntry],
    codex_dir: &Path,
) -> HashMap<u32, Vec<ThreadEntry>> {
    if !cfg!(target_os = "linux") {
        return HashMap::new();
    }
    let conversations = codex_dir.join(CODEX_SESSIONS_DIRNAME);
    let held: Vec<(&ProcessEntry, Vec<String>)> = processes
        .iter()
        .filter(|process| process.is_codex() && process.is_app_server())
        .map(|process| {
            let descriptors = Path::new(PROC_DIRNAME)
                .join(process.pid.to_string())
                .join(PROC_FD_DIRNAME);
            let mut ids: Vec<String> = fs::read_dir(descriptors)
                .into_iter()
                .flatten()
                .filter_map(Result::ok)
                .filter_map(|entry| fs::read_link(entry.path()).ok())
                .filter(|target| target.starts_with(&conversations))
                .filter_map(|target| tree::thread_id(&target))
                .collect();
            ids.sort();
            ids.dedup();
            (process, ids)
        })
        .filter(|(_, ids)| !ids.is_empty())
        .collect();
    let every_id: Vec<String> = held
        .iter()
        .flat_map(|(_, ids)| ids.iter().cloned())
        .collect();
    let named = codex::read_threads_by_id(codex_dir, &every_id);
    held.into_iter()
        .map(|(process, ids)| {
            let threads = ids
                .into_iter()
                .map(|id| match named.get(&id) {
                    Some(thread) => ThreadEntry {
                        name:    thread.label().unwrap_or(id),
                        started: thread.created_ms / 1_000,
                    },
                    None => ThreadEntry {
                        name:    id,
                        started: process.started,
                    },
                })
                .collect();
            (process.pid, threads)
        })
        .collect()
}

/// Every session record in `directory` that parses. The directory holds
/// other files beside the records, and a record being written as it is
/// read fails to parse; both are skipped.
fn read_sessions(directory: &Path) -> Vec<SessionRecord> {
    let Ok(entries) = fs::read_dir(directory) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == SESSION_RECORD_EXTENSION)
        })
        .filter_map(|path| fs::read_to_string(path).ok())
        .filter_map(|text| serde_json::from_str(&text).ok())
        .collect()
}

/// The process table the classification and the tree read.
///
/// Three passes. The first reads every process's name, parent and
/// start time, which is all the ancestry walk needs. The second reads
/// the command line and directory, and only for the processes
/// [`detailed_pids`] names. The third reads the environment of every
/// `codex`, for the Claude Code process its `CLAUDE_PID` names; no other
/// process's environment is read.
fn process_table(sessions: &[SessionRecord]) -> Vec<ProcessEntry> {
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing().without_tasks(),
    );
    let detailed = detailed_pids(&system, sessions);
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&detailed),
        false,
        ProcessRefreshKind::nothing()
            .without_tasks()
            .with_cmd(UpdateKind::OnlyIfNotSet)
            .with_cwd(UpdateKind::OnlyIfNotSet),
    );
    let codex: Vec<Pid> = system
        .processes()
        .values()
        .filter(|process| process.name() == CODEX_AGENT)
        .map(Process::pid)
        .collect();
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&codex),
        false,
        ProcessRefreshKind::nothing()
            .without_tasks()
            .with_environ(UpdateKind::OnlyIfNotSet),
    );
    system
        .processes()
        .values()
        .map(|process| ProcessEntry {
            pid:        process.pid().as_u32(),
            parent:     process.parent().map(Pid::as_u32),
            name:       process.name().to_string_lossy().into_owned(),
            arguments:  process
                .cmd()
                .iter()
                .map(|argument| argument.to_string_lossy().into_owned())
                .collect(),
            started:    process.start_time(),
            directory:  process.cwd().map(Path::to_path_buf),
            claude_pid: if process.name() == CODEX_AGENT {
                claude_pid(process.environ())
            } else {
                None
            },
        })
        .collect()
}

/// The pid `CLAUDE_PID` names in `environment`, one `NAME=value` per
/// entry; none when it is not set or is not a pid.
fn claude_pid(environment: &[OsString]) -> Option<u32> {
    environment.iter().find_map(|variable| {
        variable
            .to_str()?
            .strip_prefix(CLAUDE_PID_VARIABLE)?
            .strip_prefix('=')?
            .parse()
            .ok()
    })
}

/// The processes whose command line and directory the second pass
/// reads, and every process above one of them.
///
/// A row can come from each session record's process and every
/// `codex`. The tree reads the command line of each child of a Claude
/// Code process, which says whether the child is its shell wrapper, so
/// every process named `claude` and each such child are read too. On
/// macOS a Claude Code process is named for its version, so an ancestor
/// is only recognised as Claude Code by its command line.
fn detailed_pids(system: &System, sessions: &[SessionRecord]) -> Vec<Pid> {
    let processes = system.processes();
    let mut children: HashMap<Pid, Vec<Pid>> = HashMap::new();
    for process in processes.values() {
        if let Some(parent) = process.parent() {
            children.entry(parent).or_default().push(process.pid());
        }
    }
    let claude: Vec<Pid> = sessions
        .iter()
        .map(|session| Pid::from_u32(session.pid))
        .chain(
            processes
                .values()
                .filter(|process| process.name() == CLAUDE_AGENT)
                .map(Process::pid),
        )
        .collect();
    let starts = claude
        .iter()
        .copied()
        .chain(
            claude
                .iter()
                .filter_map(|pid| children.get(pid))
                .flatten()
                .copied(),
        )
        .chain(
            processes
                .values()
                .filter(|process| process.name() == CODEX_AGENT)
                .map(Process::pid),
        );
    let mut detailed = HashSet::new();
    for start in starts {
        let mut next = Some(start);
        while let Some(pid) = next {
            // A pid already taken was reached from another start, and so
            // was everything above it.
            if !detailed.insert(pid) {
                break;
            }
            next = processes.get(&pid).and_then(Process::parent);
        }
    }
    detailed.into_iter().collect()
}

#[cfg(test)]
#[expect(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use chrono::DateTime;
    use serde_json::json;
    use tempfile::TempDir;

    use super::*;

    /// Only `.json` files that parse are records: the key files beside
    /// them and a record caught half written are skipped.
    #[test]
    fn only_json_files_that_parse_are_read() {
        let directory = tempfile::tempdir().expect("a temporary directory should open");
        let write = |name: &str, text: &str| {
            fs::write(directory.path().join(name), text).expect("the fixture should write");
        };
        write(
            "428044.json",
            r#"{"pid":428044,"sessionId":"9f1c","cwd":"/home/natepiano/rust/handler","name":"enh/handler","status":"busy","startedAt":1790000000000,"kind":"interactive","procStart":"123","parentSessionId":null}"#,
        );
        write("46560.json", r#"{"pid":46560,"sessionId":"abcd1234ef"}"#);
        write("428044.5f2e.key", "not a record");
        write("99.json", r#"{"pid":99,"sessionId":"#);

        let mut sessions = read_sessions(directory.path());
        sessions.sort_by_key(|session| session.pid);

        assert_eq!(
            sessions,
            [
                SessionRecord {
                    pid:        46_560,
                    session_id: "abcd1234ef".to_string(),
                    cwd:        None,
                    name:       None,
                    status:     None,
                },
                SessionRecord {
                    pid:        428_044,
                    session_id: "9f1c".to_string(),
                    cwd:        Some(PathBuf::from("/home/natepiano/rust/handler")),
                    name:       Some("enh/handler".to_string()),
                    status:     Some("busy".to_string()),
                },
            ]
        );
    }

    /// A directory that is not there holds no records.
    #[test]
    fn a_missing_directory_holds_no_records() {
        let directory = tempfile::tempdir().expect("a temporary directory should open");

        assert!(read_sessions(&directory.path().join("sessions")).is_empty());
    }

    /// `CLAUDE_PID` is read from among the other variables; a variable
    /// that only starts the same way, or a value that is not a pid,
    /// names nothing.
    #[test]
    fn claude_pid_is_read_from_the_environment() {
        let environment = |variables: &[&str]| -> Vec<OsString> {
            variables.iter().map(OsString::from).collect()
        };

        assert_eq!(
            claude_pid(&environment(&[
                "HOME=/home/natepiano",
                "CLAUDE_PID=1579022",
                "CLAUDE_CODE_SESSION_ID=e296",
            ])),
            Some(1_579_022)
        );
        assert_eq!(claude_pid(&environment(&["CLAUDE_PIDS=5"])), None);
        assert_eq!(claude_pid(&environment(&["CLAUDE_PID=soon"])), None);
        assert_eq!(claude_pid(&[]), None);
    }

    /// boss of bosses, the launcher in these tests.
    const BOSS: u32 = 1_579_022;
    /// A session boss opened in tmux.
    const TRUNK: u32 = 3_266_367;
    /// A session tmux holds that no transcript opened: it started more
    /// than [`CALL_LOOKBACK`](crate::constants::CALL_LOOKBACK) after
    /// trunk's call.
    const ARRANGE: u32 = 3_337_048;
    /// The unix second trunk and the first shell wrappers started.
    const LAUNCH: u64 = 1_790_000_000;

    /// A scanner with nothing found yet, reading no home directory.
    fn scanner() -> LocalScanner {
        LocalScanner {
            home:        None,
            shell_calls: HashMap::new(),
            launchers:   HashMap::new(),
        }
    }

    /// A process with its command line read.
    fn entry(pid: u32, parent: u32, name: &str, started: u64, arguments: &[&str]) -> ProcessEntry {
        ProcessEntry {
            pid,
            parent: Some(parent),
            name: name.to_string(),
            arguments: arguments
                .iter()
                .map(|argument| (*argument).to_string())
                .collect(),
            started,
            directory: None,
            claude_pid: None,
        }
    }

    /// Claude Code's shell wrapper under [`BOSS`] running `command`.
    fn wrapper(pid: u32, started: u64, command: &str) -> ProcessEntry {
        let script = format!(
            "source /home/natepiano/.claude/shell-snapshots/snapshot-zsh-1.sh && eval '{command}' \
             < /dev/null"
        );
        entry(pid, BOSS, "zsh", started, &["zsh", "-c", &script])
    }

    /// A Claude Code row.
    fn claude_row(pid: u32, name: &str, started: u64) -> AgentRow {
        AgentRow {
            agent: Agent::Claude,
            name: name.to_string(),
            status: None,
            started,
            pid,
            directory: "~".to_string(),
            launched_by: None,
            children: Vec::new(),
        }
    }

    /// Write a transcript at `path` holding one shell call, written at
    /// `at_ms`.
    fn write_call(path: &Path, at_ms: u64, command: &str, description: &str) {
        let stamp = DateTime::from_timestamp_millis(
            i64::try_from(at_ms).expect("the fixture time should fit"),
        )
        .expect("the fixture time should be representable")
        .to_rfc3339();
        let line = json!({
            "type": "assistant",
            "timestamp": stamp,
            "message": {"content": [{"type": "tool_use", "name": "Bash",
                                     "input": {"command": command, "description": description}}]},
        });
        fs::write(path, format!("{line}\n")).expect("the fixture transcript should write");
    }

    /// A session tmux holds is launched by the agent whose transcript
    /// opened it, which is kept. A miss is looked for again until the
    /// session is [`CALL_SEARCH_SETTLE`] old, then kept too. A launcher
    /// no longer listed leaves its session top level, and a session gone
    /// from the scan is forgotten.
    #[test]
    fn launchers_are_found_in_transcripts_and_kept() {
        let directory = TempDir::new().expect("a temporary directory should open");
        let transcript = directory.path().join("boss.jsonl");
        write_call(
            &transcript,
            (LAUNCH - 1) * 1_000,
            "tmux new-session -d -s tool-based-ui-trunk zsh -ic claude",
            "Open trunk",
        );
        let transcripts = HashMap::from([(BOSS, transcript)]);
        let processes = [
            entry(1, 0, "systemd", 0, &[]),
            entry(45_494, 1, "zsh", LAUNCH - 100, &[]),
            entry(BOSS, 45_494, "claude", LAUNCH - 100, &["claude"]),
            entry(3_261_729, 1, "tmux: server", LAUNCH - 50, &[]),
            entry(3_266_307, 3_261_729, "zsh", LAUNCH, &[]),
            entry(TRUNK, 3_266_307, "claude", LAUNCH, &["claude"]),
            entry(3_336_767, 3_261_729, "zsh", LAUNCH + 700, &[]),
            entry(ARRANGE, 3_336_767, "claude", LAUNCH + 700, &["claude"]),
        ];
        let rows = || {
            vec![
                claude_row(BOSS, "boss of bosses", LAUNCH - 100),
                claude_row(TRUNK, "tool-based-ui-trunk", LAUNCH),
                claude_row(ARRANGE, "tool-based-ui-arrange", LAUNCH + 700),
            ]
        };
        let launched = |rows: &[AgentRow]| -> Vec<Option<u32>> {
            rows.iter().map(|row| row.launched_by).collect()
        };
        let mut scanner = scanner();

        let mut young = rows();
        scanner.find_launchers(&processes, &[], &transcripts, &mut young, LAUNCH + 710);
        assert_eq!(launched(&young), [None, Some(BOSS), None]);
        assert_eq!(
            scanner.launchers,
            HashMap::from([((TRUNK, LAUNCH), Some((BOSS, LAUNCH - 100)))])
        );

        let mut settled = rows();
        scanner.find_launchers(&processes, &[], &transcripts, &mut settled, LAUNCH + 730);
        assert_eq!(launched(&settled), [None, Some(BOSS), None]);
        assert_eq!(scanner.launchers.get(&(ARRANGE, LAUNCH + 700)), Some(&None));

        let mut orphaned = rows().split_off(1);
        scanner.find_launchers(&processes, &[], &transcripts, &mut orphaned, LAUNCH + 730);
        assert_eq!(launched(&orphaned), [None, None]);

        let mut alone = rows();
        alone.truncate(1);
        scanner.find_launchers(&processes, &[], &transcripts, &mut alone, LAUNCH + 730);
        assert!(scanner.launchers.is_empty());
    }

    /// A shell wrapper's call is found in its session's transcript or a
    /// running subagent's, and kept. A miss is looked for again until
    /// the wrapper is [`CALL_SEARCH_SETTLE`] old, then kept too, and a
    /// wrapper gone from the scan is forgotten.
    #[test]
    fn shell_calls_are_found_in_transcripts_and_kept() {
        let directory = TempDir::new().expect("a temporary directory should open");
        let own = directory.path().join("boss.jsonl");
        write_call(&own, (LAUNCH - 1) * 1_000, "cargo build", "Build it");
        let theirs = directory.path().join("agent-a1.jsonl");
        write_call(
            &theirs,
            LAUNCH * 1_000,
            "cargo nextest run",
            "Run the tests",
        );
        let transcripts = HashMap::from([(BOSS, own)]);
        let subagents = HashMap::from([(
            BOSS,
            vec![Subagent {
                id:          "a1".to_string(),
                parent:      None,
                description: "Test it".to_string(),
                started:     LAUNCH - 10,
                transcript:  theirs,
            }],
        )]);
        let boss = entry(BOSS, 1, "claude", LAUNCH - 100, &["claude"]);
        let processes = [
            boss.clone(),
            wrapper(3_900_000, LAUNCH, "cargo build"),
            wrapper(3_900_010, LAUNCH, "cargo nextest run"),
            wrapper(3_900_020, LAUNCH + 10, "ls"),
        ];
        let mut scanner = scanner();

        let found = scanner.find_shell_calls(&processes, &transcripts, &subagents, LAUNCH + 20);

        let call = |description: &str, subagent: Option<&str>| ShellCall {
            description: Some(description.to_string()),
            subagent:    subagent.map(str::to_string),
        };
        assert_eq!(
            found,
            HashMap::from([
                (3_900_000, call("Build it", None)),
                (3_900_010, call("Run the tests", Some("a1"))),
            ])
        );
        assert_eq!(scanner.shell_calls.len(), 2);

        scanner.find_shell_calls(&processes, &transcripts, &subagents, LAUNCH + 40);
        assert_eq!(
            scanner.shell_calls.get(&(3_900_020, LAUNCH + 10)),
            Some(&None)
        );

        scanner.find_shell_calls(&[boss], &transcripts, &subagents, LAUNCH + 40);
        assert!(scanner.shell_calls.is_empty());
    }
}
