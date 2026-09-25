//! Reading this machine: the session records Claude Code writes under
//! `~/.claude/sessions`, and the process table, handed together to
//! [`classify::top_level_rows`].

use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

use sysinfo::Pid;
use sysinfo::Process;
use sysinfo::ProcessRefreshKind;
use sysinfo::ProcessesToUpdate;
use sysinfo::System;
use sysinfo::UpdateKind;

use super::AgentRow;
use super::classify;
use super::classify::ProcessEntry;
use super::classify::SessionRecord;
use crate::constants::CLAUDE_DIRNAME;
use crate::constants::CLAUDE_SESSIONS_DIRNAME;
use crate::constants::CODEX_AGENT;
use crate::constants::SESSION_RECORD_EXTENSION;

/// Scans this machine for its top-level agents.
#[derive(Debug)]
pub(crate) struct LocalScanner {
    /// This machine's home directory: where the session records live,
    /// and what a row's directory is written against.
    home: Option<PathBuf>,
}

impl LocalScanner {
    /// A scanner of the user running this process.
    pub(crate) fn new() -> Self {
        Self {
            home: dirs::home_dir(),
        }
    }

    /// The top-level agents running now, oldest first.
    ///
    /// Each scan reads a fresh process table, so a pid handed out again
    /// since the last scan cannot keep the command line or directory of
    /// the process that had it before.
    pub(crate) fn scan(&self) -> Vec<AgentRow> {
        let sessions = self
            .home
            .as_deref()
            .map(|home| read_sessions(&home.join(CLAUDE_DIRNAME).join(CLAUDE_SESSIONS_DIRNAME)))
            .unwrap_or_default();
        let processes = process_table(&sessions);
        classify::top_level_rows(&processes, &sessions, self.home.as_deref())
    }
}

/// Every session record in `directory` that parses. The directory holds
/// other files beside the records, and a record being written as it is
/// read fails to parse; both are skipped.
pub(crate) fn read_sessions(directory: &Path) -> Vec<SessionRecord> {
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

/// The process table the classification reads.
///
/// Two passes. The first reads every process's name, parent and start
/// time, which is all the ancestry walk needs. The second reads the
/// command line and directory, and only for the processes a row can
/// come from -- each session record's process and every `codex` -- and
/// the processes above them: on macOS a Claude Code process is named
/// for its version, so an ancestor is only recognised as Claude Code
/// by its command line.
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
    system
        .processes()
        .values()
        .map(|process| ProcessEntry {
            pid:       process.pid().as_u32(),
            parent:    process.parent().map(Pid::as_u32),
            name:      process.name().to_string_lossy().into_owned(),
            arguments: process
                .cmd()
                .iter()
                .map(|argument| argument.to_string_lossy().into_owned())
                .collect(),
            started:   process.start_time(),
            directory: process.cwd().map(Path::to_path_buf),
        })
        .collect()
}

/// The processes whose command line and directory the second pass
/// reads: each session record's process, every `codex`, and every
/// process above one of them.
fn detailed_pids(system: &System, sessions: &[SessionRecord]) -> Vec<Pid> {
    let processes = system.processes();
    let starts = sessions
        .iter()
        .map(|session| Pid::from_u32(session.pid))
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
}
