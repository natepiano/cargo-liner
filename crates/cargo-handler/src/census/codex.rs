//! Reading the threads interactive Codex sessions started, from the
//! `threads` table of Codex's database `~/.codex/state_<n>.sqlite`.
//!
//! Log rows identify resumed threads by process. A managed daemon can serve
//! several clients. A child daemon supplies confirmed selection events.
//! Detached daemon connections have no verified join to a TUI process.

#[cfg(test)]
use std::cell::Cell;
use std::collections::HashMap;
use std::collections::HashSet;
use std::fs;
#[cfg(test)]
use std::ops::RangeInclusive;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::path::PathBuf;

use rusqlite::Connection;
use rusqlite::Error;
use rusqlite::OpenFlags;
use rusqlite::OptionalExtension;
use rusqlite::Row;
use rusqlite::params;

use super::account::ConfigHome;
use super::account::Program;
use super::classify;
use super::classify::ProcessEntry;
use super::codex_names::SessionName;
use super::hierarchy::FileRevision;
use crate::config::CodexStartWindow;
use crate::constants::CODEX_DIRNAME;
use crate::constants::CODEX_LAST_THREAD_QUERY;
use crate::constants::CODEX_LEGACY_TUI_ORIGINATOR;
use crate::constants::CODEX_LOG_INITIAL_QUERY;
use crate::constants::CODEX_LOG_MAX_ID_QUERY;
use crate::constants::CODEX_LOG_NEW_QUERY;
use crate::constants::CODEX_LOG_PREFIX;
use crate::constants::CODEX_LOG_PROCESS_PREFIX;
use crate::constants::CODEX_PROMPT_LABEL_MAX;
use crate::constants::CODEX_RESUME_LAST_ARGUMENT;
use crate::constants::CODEX_RESUME_SUBCOMMAND;
use crate::constants::CODEX_STATE_BUSY_TIMEOUT;
use crate::constants::CODEX_STATE_EXTENSION;
use crate::constants::CODEX_STATE_PREFIX;
use crate::constants::CODEX_THREAD_BY_ID_QUERY;
#[cfg(test)]
use crate::constants::TRUNCATION_MARK;
use crate::constants::UUID_TEXT_LENGTH;
use crate::startup_log;
use crate::summary;

/// Resolve the process's own configuration home, never another account's index.
pub(super) fn session_home(process: &ProcessEntry, home: Option<&Path>) -> Option<PathBuf> {
    match &process.config_home {
        ConfigHome::Named(path) => Some(path.clone()),
        ConfigHome::Default(Program::Codex) => home.map(|home| home.join(CODEX_DIRNAME)),
        ConfigHome::Unread | ConfigHome::Default(_) => None,
    }
}

/// The conversation an interactive Codex asks to open.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ResumeRequest<'a> {
    NewSession,
    NamedId(&'a str),
    LastThread,
}

/// Codex options whose next argument is a value, including values named resume.
const CODEX_VALUE_OPTIONS: &[&str] = &[
    "-c",
    "--config",
    "--enable",
    "--disable",
    "--remote",
    "--remote-auth-token-env",
    "-i",
    "--image",
    "-m",
    "--model",
    "--local-provider",
    "-p",
    "--profile",
    "-s",
    "--sandbox",
    "-a",
    "--ask-for-approval",
    "-C",
    "--cd",
    "--add-dir",
];

/// Separator positions in a complete UUID.
const UUID_HYPHEN_POSITIONS: [usize; 4] = [8, 13, 18, 23];

/// `ResumePosition` separates the subcommand from its requested thread.
#[derive(Clone, Copy)]
enum ResumePosition {
    Command,
    Thread,
}

/// A resume identifies a thread only after the first positional argument names the command.
pub(super) fn resume_request(process: &ProcessEntry) -> ResumeRequest<'_> {
    let mut arguments = process.arguments.iter().skip(1);
    let mut position = ResumePosition::Command;
    while let Some(argument) = arguments.next() {
        let argument = argument.as_str();
        if CODEX_VALUE_OPTIONS.contains(&argument) {
            if arguments.next().is_none() {
                return ResumeRequest::NewSession;
            }
            continue;
        }
        match (position, argument) {
            (ResumePosition::Thread, CODEX_RESUME_LAST_ARGUMENT) => {
                return ResumeRequest::LastThread;
            },
            (_, switch) if switch.starts_with('-') => {},
            (ResumePosition::Command, CODEX_RESUME_SUBCOMMAND) => position = ResumePosition::Thread,
            (ResumePosition::Thread, id)
                if id.len() == UUID_TEXT_LENGTH
                    && id.bytes().enumerate().all(|(index, byte)| {
                        if UUID_HYPHEN_POSITIONS.contains(&index) {
                            byte == b'-'
                        } else {
                            byte.is_ascii_hexdigit()
                        }
                    }) =>
            {
                return ResumeRequest::NamedId(id);
            },
            _ => return ResumeRequest::NewSession,
        }
    }
    ResumeRequest::NewSession
}

/// Older resume candidates from this process's own home, outside the startup window.
#[cfg(test)]
pub(super) fn read_resumed_threads(process: &ProcessEntry, codex_home: &Path) -> Vec<CodexThread> {
    match resume_request(process) {
        ResumeRequest::NewSession => Vec::new(),
        ResumeRequest::NamedId(id) => OpenThreadRows::default()
            .read(codex_home, &[id.to_owned()])
            .values()
            .cloned()
            .collect(),
        ResumeRequest::LastThread => process
            .directory
            .as_deref()
            .and_then(|cwd| {
                state_database(codex_home)
                    .and_then(|path| query_last_thread(&path, cwd).ok().flatten())
            })
            .into_iter()
            .collect(),
    }
}

/// The newest database conversation in this directory, regardless of process age.
#[cfg(test)]
fn query_last_thread(path: &Path, cwd: &Path) -> rusqlite::Result<Option<CodexThread>> {
    let account_home = CodexAccountHome::from_database(path)?;
    let connection = open_read_only(path)?;
    connection
        .query_row(
            CODEX_LAST_THREAD_QUERY,
            params![cwd.to_string_lossy()],
            |row| decode_thread(row, &account_home),
        )
        .optional()
}

/// The lexical configuration home whose database supplied a thread.
/// Shared rollout links do not change the account that owns the database.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct CodexAccountHome(PathBuf);

impl From<&Path> for CodexAccountHome {
    fn from(home: &Path) -> Self { Self(home.to_path_buf()) }
}

impl AsRef<Path> for CodexAccountHome {
    fn as_ref(&self) -> &Path { &self.0 }
}

impl CodexAccountHome {
    fn from_database(path: &Path) -> rusqlite::Result<Self> {
        path.parent()
            .map(Self::from)
            .ok_or_else(|| Error::InvalidPath(path.to_path_buf()))
    }
}

/// One thread an interactive Codex started, as the census reads it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct CodexThread {
    /// The account home of the database that supplied this record.
    pub(super) account_home: CodexAccountHome,
    /// The directory the thread runs in.
    pub(super) cwd:          PathBuf,
    /// When the thread was created, in unix milliseconds.
    pub(super) created_ms:   u64,
    /// The name the thread was given, where it has one.
    pub(super) name:         SessionName,
    /// The first prompt typed into the thread; empty before the first.
    pub(super) first_prompt: String,
    /// The file the thread's conversation is written to.
    pub(super) rollout:      PathBuf,
}

impl CodexThread {
    /// What a row shows for this thread: its name, else its first
    /// prompt, each on one line and the prompt cut to
    /// [`CODEX_PROMPT_LABEL_MAX`] characters; nothing when the thread
    /// has neither.
    pub(super) fn label(&self) -> Option<String> {
        if let SessionName::Named(name) = &self.name {
            return Some(name.clone());
        }
        let prompt = one_line(&self.first_prompt);
        if prompt.is_empty() {
            return None;
        }
        Some(summary::truncated(&prompt, CODEX_PROMPT_LABEL_MAX))
    }
}

/// `text` with every run of whitespace, line breaks included, written
/// as one space and none at either end.
fn one_line(text: &str) -> String { text.split_whitespace().collect::<Vec<_>>().join(" ") }

/// The threads interactive Codex sessions created with a creation time
/// in `created`, in unix milliseconds, read from the newest thread
/// database in `codex_dir`.
///
/// None when there is no database, or when it cannot be read as this
/// build expects -- a layout Codex has since changed included. A row
/// then goes without a thread's name.
#[cfg(test)]
pub(super) fn read_threads(codex_dir: &Path, created: &RangeInclusive<u64>) -> Vec<CodexThread> {
    state_database(codex_dir)
        .and_then(|path| query_threads(&path, created).ok())
        .unwrap_or_default()
}

/// The rows [`CODEX_THREAD_BY_ID_QUERY`] finds for each of `ids` in the
/// database at `path`, opened read-only.
fn query_threads_by_id(path: &Path, ids: &[String]) -> rusqlite::Result<OpenThreadAccount> {
    let account_home = CodexAccountHome::from_database(path)?;
    let connection = open_read_only(path)?;
    let mut statement = connection.prepare(CODEX_THREAD_BY_ID_QUERY)?;
    // SQLite can create the WAL during query preparation. Record that revision
    // before reading rows, so a later write still invalidates this answer.
    let revision = ThreadDatabaseRevision::of_database(path);
    let mut threads = HashMap::new();
    for id in ids {
        let found = statement.query_row(params![id], |row| decode_thread(row, &account_home));
        match found {
            Ok(thread) => {
                threads.insert(id.clone(), thread);
            },
            Err(Error::QueryReturnedNoRows) => {},
            Err(error) => return Err(error),
        }
    }
    Ok(OpenThreadAccount {
        revision,
        requested: ids.iter().cloned().collect(),
        rows: threads,
    })
}

/// Decode the shared column order of every thread query.
fn decode_thread(row: &Row<'_>, account: &CodexAccountHome) -> rusqlite::Result<CodexThread> {
    Ok(CodexThread {
        account_home: account.clone(),
        cwd:          PathBuf::from(row.get::<_, String>(0)?),
        created_ms:   u64::try_from(row.get::<_, i64>(1)?).unwrap_or_default(),
        name:         SessionName::from(row.get::<_, Option<String>>(2)?),
        first_prompt: row.get(3)?,
        rollout:      PathBuf::from(row.get::<_, String>(4)?),
    })
}

/// The database at `path`, opened read-only, waiting on a writer for at
/// most [`CODEX_STATE_BUSY_TIMEOUT`].
fn open_read_only(path: &Path) -> rusqlite::Result<Connection> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    connection.busy_timeout(CODEX_STATE_BUSY_TIMEOUT)?;
    Ok(connection)
}

/// The `state_<n>.sqlite` in `codex_dir` with the highest `<n>`.
fn state_database(codex_dir: &Path) -> Option<PathBuf> {
    versioned_database(codex_dir, CODEX_STATE_PREFIX)
}

/// Select the highest database version without following account-home links.
fn versioned_database(codex_dir: &Path, prefix: &str) -> Option<PathBuf> {
    fs::read_dir(codex_dir)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter_map(|path| {
            if path.extension()? != CODEX_STATE_EXTENSION {
                return None;
            }
            let version: u32 = path
                .file_stem()?
                .to_str()?
                .strip_prefix(prefix)?
                .parse()
                .ok()?;
            Some((version, path))
        })
        .max_by_key(|(version, _)| *version)
        .map(|(_, path)| path)
}

/// App-server thread names retained while the account database files stay equal.
#[derive(Debug, Default)]
pub(super) struct OpenThreadRows {
    homes:                     HashMap<PathBuf, OpenThreadAccount>,
    #[cfg(test)]
    pub(super) database_reads: usize,
}

#[derive(Debug)]
struct OpenThreadAccount {
    revision:  ThreadDatabaseRevision,
    requested: HashSet<String>,
    rows:      HashMap<String, CodexThread>,
}

#[derive(Debug, Eq, PartialEq)]
enum ThreadDatabaseRevision {
    Missing,
    Present {
        path:     PathBuf,
        database: FileRevision,
        wal:      FileRevision,
    },
}

impl ThreadDatabaseRevision {
    fn read(codex_home: &Path) -> Self {
        let Some(path) = state_database(codex_home) else {
            return Self::Missing;
        };
        Self::of_database(&path)
    }

    fn of_database(path: &Path) -> Self {
        let revision = |file: &Path| {
            fs::metadata(file).map_or(FileRevision::Missing, |metadata| {
                FileRevision::of(&metadata)
            })
        };
        let wal = revision(&PathBuf::from(format!("{}-wal", path.display())));
        Self::Present {
            database: revision(path),
            path: path.to_owned(),
            wal,
        }
    }
}

impl OpenThreadRows {
    /// A new requested id or a database revision refreshes the account answer.
    pub(super) fn read(
        &mut self,
        codex_home: &Path,
        ids: &[String],
    ) -> &HashMap<String, CodexThread> {
        let revision = ThreadDatabaseRevision::read(codex_home);
        let requested: HashSet<_> = ids.iter().cloned().collect();
        if self.homes.get(codex_home).is_none_or(|cached| {
            cached.revision != revision || !requested.is_subset(&cached.requested)
        }) {
            #[cfg(test)]
            {
                self.database_reads += 1;
            }
            let answer = match &revision {
                ThreadDatabaseRevision::Missing => None,
                ThreadDatabaseRevision::Present { path, .. } => query_threads_by_id(path, ids).ok(),
            }
            .unwrap_or(OpenThreadAccount {
                revision,
                requested,
                rows: HashMap::new(),
            });
            self.homes.insert(codex_home.to_owned(), answer);
        }
        &self.homes[codex_home].rows
    }

    pub(super) fn retain_homes<'h>(&mut self, homes: impl Iterator<Item = &'h PathBuf>) {
        let live: HashSet<_> = homes.collect();
        self.homes.retain(|codex_home, _| live.contains(codex_home));
    }
}

/// The current database answer for a requested thread.
#[derive(Debug)]
pub(super) enum ThreadLookup {
    User(CodexThread),
    NonUser,
    Missing,
}

/// Current thread rows, separated by the database account.
#[derive(Debug, Default)]
pub(super) struct ThreadRows {
    pub(super) candidates: Vec<CodexThread>,
    by_id:                 HashMap<(PathBuf, String), ThreadLookup>,
}

impl ThreadRows {
    pub(super) fn get(&self, account: &Path, id: &str) -> Option<&ThreadLookup> {
        self.by_id.get(&(account.to_owned(), id.to_owned()))
    }
}

/// Retained cursors and raw records. Thread rows have a separate owner.
#[derive(Debug, Default)]
pub(super) struct CodexLogs {
    databases: HashMap<PathBuf, LogDatabase>,
}

impl CodexLogs {
    /// Advance each account log once.
    pub(super) fn refresh(&mut self, processes: &[ProcessEntry], home: Option<&Path>) {
        let started = std::time::Instant::now();
        self.refresh_logs(processes, home);
        startup_log::startup_event(&format!(
            "codex_log_read elapsed_us={}",
            started.elapsed().as_micros()
        ));
    }

    fn refresh_logs(&mut self, processes: &[ProcessEntry], home: Option<&Path>) {
        let table = classify::pid_table(processes);
        let clients = classify::interactive_codex(&table, processes);
        let homes: HashSet<_> = clients
            .iter()
            .filter_map(|p| session_home(p, home))
            .collect();
        self.databases.retain(|account, _| homes.contains(account));
        for account in homes {
            let oldest = clients
                .iter()
                .filter(|p| session_home(p, home).as_ref() == Some(&account))
                .map(|p| p.started)
                .min()
                .unwrap_or_default();
            if self.refresh_account(&account, processes, oldest).is_err() {
                self.databases.remove(&account);
            }
        }
    }

    fn refresh_account(
        &mut self,
        account: &Path,
        processes: &[ProcessEntry],
        oldest: u64,
    ) -> rusqlite::Result<()> {
        if !self.databases.contains_key(account) {
            self.databases
                .insert(account.to_owned(), LogDatabase::open(account, oldest)?);
        }
        let database = self.databases.get_mut(account).ok_or(Error::InvalidQuery)?;
        database.refresh_files(account, oldest)?;
        database.advance(processes)
    }

    /// Resolve all log and argv ids with one current database connection per account.
    pub(super) fn thread_rows(
        &self,
        processes: &[ProcessEntry],
        home: Option<&Path>,
        window: CodexStartWindow,
    ) -> ThreadRows {
        let mut ids: HashMap<PathBuf, HashSet<String>> = HashMap::new();
        for process in processes
            .iter()
            .filter(|p| p.is_codex() && !p.is_app_server())
        {
            let Some(account) = session_home(process, home) else {
                continue;
            };
            let requested = ids.entry(account.clone()).or_default();
            if let ResumeRequest::NamedId(id) = resume_request(process) {
                requested.insert(id.to_owned());
            }
            if let Some(database) = self.databases.get(&account) {
                for record in database.records.values().flatten() {
                    if processes.iter().any(|p| record.belongs_to(p)) {
                        requested.insert(record.thread_id.clone());
                    }
                }
            }
        }
        let mut rows = ThreadRows::default();
        for (account, requested) in ids {
            let path = self
                .databases
                .get(&account)
                .and_then(|db| db.files.threads.path().map(Path::to_path_buf))
                .or_else(|| state_database(&account));
            if let Some(path) = path {
                let _ =
                    read_current_threads(&path, &account, requested, processes, window, &mut rows);
            }
        }
        rows
    }

    /// Select records from this process or its child daemon.
    pub(super) fn link<'a>(
        &self,
        process: &ProcessEntry,
        processes: &[ProcessEntry],
        home: Option<&Path>,
        rows: &'a ThreadRows,
    ) -> Option<&'a CodexThread> {
        let account = session_home(process, home)?;
        let database = self.databases.get(&account)?;
        database
            .records
            .values()
            .flatten()
            .filter(|record| {
                record.logged_at >= process.started
                    && processes.iter().any(|source| {
                        record.belongs_to(source)
                            && match record.selection.proof {
                                SelectionProof::InProcessSnapshot => source.pid == process.pid,
                                SelectionProof::MatchedResponse => {
                                    source.pid == process.pid || source.parent == Some(process.pid)
                                },
                            }
                    })
            })
            .filter_map(|record| match rows.get(&account, &record.thread_id) {
                Some(ThreadLookup::User(thread)) => Some((record.id, thread)),
                Some(ThreadLookup::Missing | ThreadLookup::NonUser) | None => None,
            })
            .max_by_key(|(id, _)| *id)
            .map(|(_, thread)| thread)
    }

    #[cfg(test)]
    pub(super) fn rows_read(&self) -> usize { self.databases.values().map(|db| db.rows_read).sum() }
}

fn read_current_threads(
    path: &Path,
    account: &Path,
    ids: HashSet<String>,
    processes: &[ProcessEntry],
    window: CodexStartWindow,
    rows: &mut ThreadRows,
) -> rusqlite::Result<()> {
    let connection = open_read_only(path)?;
    let layout = ThreadsLayout::read(&connection)?;
    let owner = CodexAccountHome::from(account);
    if let Some(created) = classify::codex_thread_window(processes, window) {
        let mut statement = connection.prepare(&layout.query())?;
        let from = i64::try_from(*created.start()).unwrap_or(i64::MAX);
        let until = i64::try_from(*created.end()).unwrap_or(i64::MAX);
        rows.candidates.extend(
            statement
                .query_map(params![CODEX_LEGACY_TUI_ORIGINATOR, from, until], |row| {
                    decode_thread(row, &owner)
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?,
        );
    }
    let mut last = connection.prepare(CODEX_LAST_THREAD_QUERY)?;
    for process in processes
        .iter()
        .filter(|p| matches!(resume_request(p), ResumeRequest::LastThread))
    {
        if let Some(cwd) = &process.directory {
            rows.candidates.extend(
                last.query_row(params![cwd.to_string_lossy()], |row| {
                    decode_thread(row, &owner)
                })
                .optional()?,
            );
        }
    }
    let predicate = layout.user_predicate();
    let query = format!(
        "SELECT cwd, created_at_ms, name, first_user_message, rollout_path, coalesce(({predicate}), 0) FROM threads WHERE id = ?2"
    );
    let mut statement = connection.prepare(&query)?;
    for id in ids {
        let result = statement
            .query_row(params![CODEX_LEGACY_TUI_ORIGINATOR, id], |row| {
                if row.get::<_, bool>(5)? {
                    Ok(ThreadLookup::User(decode_thread(row, &owner)?))
                } else {
                    Ok(ThreadLookup::NonUser)
                }
            })
            .optional()?
            .unwrap_or(ThreadLookup::Missing);
        rows.by_id.insert((account.to_owned(), id), result);
    }
    Ok(())
}

/// A file identity changes when another file replaces the selected path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FileIdentity(u64, u64);

impl FileIdentity {
    fn read(path: &Path) -> rusqlite::Result<Self> {
        path.metadata()
            .map(|m| Self(m.dev(), m.ino()))
            .map_err(|_| Error::InvalidPath(path.to_owned()))
    }
}

/// A directory can contain logs before it contains a thread database.
#[derive(Debug)]
enum ThreadDatabasePath {
    Selected(PathBuf),
    Unavailable,
}

impl ThreadDatabasePath {
    fn path(&self) -> Option<&Path> {
        match self {
            Self::Selected(path) => Some(path),
            Self::Unavailable => None,
        }
    }
}

/// One directory pass selects both database versions.
#[derive(Debug)]
struct AccountFiles {
    revision: FileRevision,
    logs:     PathBuf,
    threads:  ThreadDatabasePath,
}

#[cfg(test)]
thread_local! {
    static ACCOUNT_DIRECTORY_READS: Cell<usize> = const { Cell::new(0) };
}

impl AccountFiles {
    fn read(account: &Path) -> rusqlite::Result<Self> {
        let metadata = account
            .metadata()
            .map_err(|_| Error::InvalidPath(account.to_owned()))?;
        let mut logs = None;
        let mut threads = None;
        #[cfg(test)]
        ACCOUNT_DIRECTORY_READS.with(|reads| reads.set(reads.get() + 1));
        for entry in fs::read_dir(account)
            .map_err(|_| Error::InvalidPath(account.to_owned()))?
            .flatten()
        {
            let path = entry.path();
            if path
                .extension()
                .is_none_or(|extension| extension != CODEX_STATE_EXTENSION)
            {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            for (prefix, selected) in [
                (CODEX_LOG_PREFIX, &mut logs),
                (CODEX_STATE_PREFIX, &mut threads),
            ] {
                if let Some(version) = stem
                    .strip_prefix(prefix)
                    .and_then(|n| n.parse::<u32>().ok())
                    && selected.as_ref().is_none_or(|(kept, _)| version > *kept)
                {
                    *selected = Some((version, path.clone()));
                }
            }
        }
        Ok(Self {
            revision: super::hierarchy::FileRevision::of(&metadata),
            logs:     logs
                .map(|(_, p)| p)
                .ok_or_else(|| Error::InvalidPath(account.to_owned()))?,
            threads:  threads.map_or(ThreadDatabasePath::Unavailable, |(_, path)| {
                ThreadDatabasePath::Selected(path)
            }),
        })
    }
}

#[derive(Debug)]
enum LogCursor {
    FromProcessStart(u64),
    AfterId(i64),
}

/// A request identity stays within its daemon process and connection.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct SelectionRequest {
    daemon:     String,
    connection: u64,
    request:    String,
}

impl SelectionRequest {
    fn request(daemon: &str, body: &str) -> Option<Self> {
        if !body.contains("app_server.client_name=\"codex-tui\"")
            || !(body.contains("rpc.method=\"thread/start\"")
                || body.contains("rpc.method=\"thread/resume\""))
        {
            return None;
        }
        Some(Self {
            daemon:     daemon.into(),
            connection: log_number(body, "app_server.connection_id=")?,
            request:    log_request_id(body, "rpc.request_id=")?,
        })
    }

    fn response(daemon: &str, body: &str) -> Option<Self> {
        if !(body.contains("thread resume response") || body.contains("thread start response")) {
            return None;
        }
        let request = if body.contains("request_id: Integer(") {
            format!("integer:{}", log_number(body, "request_id: Integer(")?)
        } else {
            log_request_id(body, "request_id: String(")?
        };
        Some(Self {
            daemon: daemon.into(),
            connection: log_number(body, "connection_id: ConnectionId(")?,
            request,
        })
    }
}

fn log_number(body: &str, key: &str) -> Option<u64> {
    body.split_once(key)?
        .1
        .split(|c: char| !c.is_ascii_digit())
        .next()?
        .parse()
        .ok()
}

fn log_request_id(body: &str, key: &str) -> Option<String> {
    let value = body.split_once(key)?.1;
    if let Some(quoted) = value.strip_prefix('"') {
        let end = quoted.find('"')?;
        Some(format!("text:{}", &quoted[..end]))
    } else {
        Some(format!(
            "integer:{}",
            value
                .split(|c: char| !c.is_ascii_digit())
                .next()?
                .parse::<u64>()
                .ok()?
        ))
    }
}

/// The evidence that binds a selection to its log process.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SelectionProof {
    MatchedResponse,
    InProcessSnapshot,
}

#[derive(Debug)]
struct ConfirmedSelection {
    connection: u64,
    request:    String,
    proof:      SelectionProof,
}

impl ConfirmedSelection {
    fn snapshot(identity: &str, body: &str, thread: &str) -> Option<Self> {
        if !body.contains("rpc.transport=\"in-process\"")
            || !body.contains(&format!("shell_snapshot{{thread_id={thread}}}"))
            || !body.contains("}: Shell snapshot successfully created:")
        {
            return None;
        }
        let request = SelectionRequest::request(identity, body)?;
        Some(Self {
            connection: request.connection,
            request:    request.request,
            proof:      SelectionProof::InProcessSnapshot,
        })
    }

    fn rejects_older_request(&self, other: &Self) -> bool {
        match (
            self.request
                .strip_prefix("integer:")
                .and_then(|id| id.parse::<u64>().ok()),
            other
                .request
                .strip_prefix("integer:")
                .and_then(|id| id.parse::<u64>().ok()),
        ) {
            (Some(previous), Some(current)) => previous > current,
            _ => false,
        }
    }
}

#[derive(Debug)]
struct LoggedThread {
    pid:       u32,
    id:        i64,
    logged_at: u64,
    thread_id: String,
    selection: ConfirmedSelection,
}

impl LoggedThread {
    const fn belongs_to(&self, process: &ProcessEntry) -> bool {
        if self.pid != process.pid {
            return false;
        }
        self.logged_at >= process.started
    }
}

#[derive(Debug)]
struct LogDatabase {
    files:     AccountFiles,
    identity:  FileIdentity,
    logs:      Connection,
    cursor:    LogCursor,
    records:   HashMap<u32, Vec<LoggedThread>>,
    rows_read: usize,
    requests:  HashMap<SelectionRequest, u64>,
}

impl LogDatabase {
    fn open(account: &Path, start: u64) -> rusqlite::Result<Self> {
        Self::from_files(AccountFiles::read(account)?, start)
    }

    fn from_files(files: AccountFiles, start: u64) -> rusqlite::Result<Self> {
        Ok(Self {
            identity: FileIdentity::read(&files.logs)?,
            logs: open_read_only(&files.logs)?,
            files,
            cursor: LogCursor::FromProcessStart(start),
            records: HashMap::new(),
            rows_read: 0,
            requests: HashMap::new(),
        })
    }

    fn refresh_files(&mut self, account: &Path, start: u64) -> rusqlite::Result<()> {
        let metadata = account
            .metadata()
            .map_err(|_| Error::InvalidPath(account.to_owned()))?;
        if self.files.revision == super::hierarchy::FileRevision::of(&metadata) {
            return Ok(());
        }
        let files = AccountFiles::read(account)?;
        let identity = FileIdentity::read(&files.logs)?;
        if identity == self.identity {
            self.files = files;
        } else {
            *self = Self::from_files(files, start)?;
        }
        Ok(())
    }

    fn advance(&mut self, processes: &[ProcessEntry]) -> rusqlite::Result<()> {
        self.records.retain(|pid, records| {
            processes
                .iter()
                .find(|p| p.pid == *pid)
                .is_some_and(|process| {
                    records.retain(|record| record.logged_at >= process.started);
                    !records.is_empty()
                })
        });
        self.requests.retain(|request, at| {
            processes.iter().any(|process| {
                request.daemon.starts_with(&format!("pid:{}:", process.pid))
                    && *at >= process.started
            })
        });
        let (query, after) = match self.cursor {
            LogCursor::FromProcessStart(start) => (
                CODEX_LOG_INITIAL_QUERY,
                i64::try_from(start).unwrap_or(i64::MAX),
            ),
            LogCursor::AfterId(id) => (CODEX_LOG_NEW_QUERY, id),
        };
        // The initial maximum and time range need the same SQLite snapshot.
        let initial = matches!(self.cursor, LogCursor::FromProcessStart(_));
        if initial {
            self.logs.execute_batch("BEGIN")?;
        }
        let result = self.read_new(query, after);
        if initial {
            self.logs.execute_batch("ROLLBACK")?;
        }
        result
    }

    fn read_new(&mut self, query: &str, after: i64) -> rusqlite::Result<()> {
        self.rows_read = 0;
        let mut last_id = match self.cursor {
            LogCursor::AfterId(id) => id,
            LogCursor::FromProcessStart(_) => {
                self.logs
                    .query_row(CODEX_LOG_MAX_ID_QUERY, [], |row| row.get(0))?
            },
        };
        let mut statement = self.logs.prepare_cached(query)?;
        let mut rows = statement.query([after])?;
        while let Some(row) = rows.next()? {
            self.rows_read += 1;
            let id: i64 = row.get(0)?;
            last_id = last_id.max(id);
            let Some(identity) = row.get::<_, Option<String>>(2)? else {
                continue;
            };
            let Some(pid) = identity
                .strip_prefix(CODEX_LOG_PROCESS_PREFIX)
                .and_then(|s| s.split_once(':'))
                .and_then(|(pid, _)| pid.parse().ok())
            else {
                continue;
            };
            let logged_at = u64::try_from(row.get::<_, i64>(1)?).unwrap_or_default();
            let body = row.get::<_, Option<String>>(4)?.unwrap_or_default();
            if let Some(request) = SelectionRequest::request(&identity, &body) {
                self.requests.entry(request).or_insert(logged_at);
            }
            let Some(thread_id) = row.get::<_, Option<String>>(3)? else {
                continue;
            };
            let selection = match SelectionRequest::response(&identity, &body) {
                Some(request) if self.requests.remove(&request).is_some() => ConfirmedSelection {
                    connection: request.connection,
                    request:    request.request,
                    proof:      SelectionProof::MatchedResponse,
                },
                _ => match ConfirmedSelection::snapshot(&identity, &body, &thread_id) {
                    Some(selection) => {
                        self.requests.remove(&SelectionRequest {
                            daemon:     identity.clone(),
                            connection: selection.connection,
                            request:    selection.request.clone(),
                        });
                        selection
                    },
                    None => continue,
                },
            };
            let record = LoggedThread {
                pid,
                id,
                logged_at,
                thread_id,
                selection,
            };
            let records = self.records.entry(pid).or_default();
            let kept = records
                .iter_mut()
                .find(|r| r.selection.connection == record.selection.connection);
            if let Some(kept) = kept {
                if kept.id < record.id && !kept.selection.rejects_older_request(&record.selection) {
                    *kept = record;
                }
            } else {
                records.push(record);
            }
        }
        self.cursor = LogCursor::AfterId(last_id);
        Ok(())
    }
}

/// Which columns identify interactive threads in a database.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ThreadsLayout {
    WithThreadSource,
    OriginatorOnly,
}

impl ThreadsLayout {
    fn read(connection: &Connection) -> rusqlite::Result<Self> {
        connection.query_row(
            "SELECT COUNT(*) FROM pragma_table_info('threads') WHERE name IN ('source', 'thread_source')",
            [],
            |row| {
                Ok(match row.get::<_, i64>(0)? {
                    2 => Self::WithThreadSource,
                    _ => Self::OriginatorOnly,
                })
            },
        )
    }

    const fn user_predicate(self) -> &'static str {
        match self {
            Self::WithThreadSource => {
                "(thread_source = 'user' OR (thread_source IS NULL AND originator = ?1)) AND (source IS NULL OR source <> 'exec')"
            },
            Self::OriginatorOnly => "originator = ?1",
        }
    }

    fn query(self) -> String {
        format!(
            "SELECT cwd, created_at_ms, name, first_user_message, rollout_path FROM threads WHERE ({}) AND created_at_ms BETWEEN ?2 AND ?3",
            self.user_predicate()
        )
    }
}

/// The interactive threads in the database at `path`,
/// opened read-only.
#[cfg(test)]
fn query_threads(path: &Path, created: &RangeInclusive<u64>) -> rusqlite::Result<Vec<CodexThread>> {
    let account_home = CodexAccountHome::from_database(path)?;
    let connection = open_read_only(path)?;
    let layout = ThreadsLayout::read(&connection)?;
    let mut statement = connection.prepare(&layout.query())?;
    let from = i64::try_from(*created.start()).unwrap_or(i64::MAX);
    let until = i64::try_from(*created.end()).unwrap_or(i64::MAX);
    statement
        .query_map(params![CODEX_LEGACY_TUI_ORIGINATOR, from, until], |row| {
            decode_thread(row, &account_home)
        })?
        .collect()
}

#[cfg(test)]
#[expect(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn a_log_database_change_reuses_the_selected_account_files() {
        let directory = TempDir::new().expect("account directory");
        let first = directory.path().join("logs_2.sqlite");
        Connection::open(&first).expect("first log database");
        ACCOUNT_DIRECTORY_READS.with(|reads| reads.set(0));
        let mut database = LogDatabase::open(directory.path(), 0).expect("open logs");
        assert_eq!(ACCOUNT_DIRECTORY_READS.with(Cell::get), 1);
        ACCOUNT_DIRECTORY_READS.with(|reads| reads.set(0));
        database
            .refresh_files(directory.path(), 0)
            .expect("unchanged files");
        assert_eq!(ACCOUNT_DIRECTORY_READS.with(Cell::get), 0);
        let next = directory.path().join("logs_3.sqlite");
        Connection::open(&next).expect("new log database");
        database
            .refresh_files(directory.path(), 0)
            .expect("new files");
        assert_eq!(database.files.logs, next);
        assert_eq!(ACCOUNT_DIRECTORY_READS.with(Cell::get), 1);
    }

    /// The columns of Codex's `threads` table the census reads, with the
    /// constraints Codex gives them, and one it does not read.
    const THREADS_TABLE: &str = "CREATE TABLE threads (
        id TEXT PRIMARY KEY,
        cwd TEXT NOT NULL,
        title TEXT NOT NULL,
        originator TEXT,
        created_at_ms INTEGER,
        name TEXT,
        first_user_message TEXT NOT NULL DEFAULT '',
        rollout_path TEXT NOT NULL
    )";

    /// One row of the fixture `threads` table: `thread` with the id and
    /// the originator Codex stores beside it.
    struct StoredThread<'a> {
        /// The thread's id, the table's key.
        id:         &'a str,
        /// The program that started the thread.
        originator: &'a str,
        /// The columns the census reads.
        thread:     CodexThread,
    }

    /// A thread database at `name` in `directory`, holding `threads`.
    fn write_database(directory: &Path, name: &str, threads: &[StoredThread<'_>]) {
        let connection =
            Connection::open(directory.join(name)).expect("the fixture database should open");
        // These disposable databases need visibility to the reader, not
        // crash durability. Avoid waiting for a disk sync after every insert.
        connection
            .pragma_update(None, "synchronous", "OFF")
            .expect("fixture writes should skip durability waits");
        connection
            .execute(THREADS_TABLE, [])
            .expect("the fixture table should create");
        for stored in threads {
            let thread = &stored.thread;
            connection
                .execute(
                    "INSERT INTO threads VALUES (?1, ?2, '', ?3, ?4, ?5, ?6, ?7)",
                    params![
                        stored.id,
                        thread
                            .cwd
                            .to_str()
                            .expect("the fixture directory should be text"),
                        stored.originator,
                        i64::try_from(thread.created_ms).expect("the fixture time should fit"),
                        match &thread.name {
                            SessionName::Named(name) => Some(name.as_str()),
                            SessionName::Unnamed => None,
                        },
                        thread.first_prompt,
                        thread
                            .rollout
                            .to_str()
                            .expect("the fixture rollout should be text"),
                    ],
                )
                .expect("the fixture thread should insert");
        }
    }

    /// A thread in `/work` created at `created_ms`.
    fn thread(home: &Path, created_ms: u64, name: Option<&str>, first_prompt: &str) -> CodexThread {
        CodexThread {
            account_home: CodexAccountHome::from(home),
            cwd: PathBuf::from("/work"),
            created_ms,
            name: SessionName::from(name.map(str::to_string)),
            first_prompt: first_prompt.to_string(),
            rollout: PathBuf::from(format!("/rollouts/{created_ms}.jsonl")),
        }
    }

    /// Only threads an interactive Codex created inside the span are
    /// read, from the newest database; the journal files beside it and
    /// an older layout's database are passed over.
    #[test]
    fn reads_interactive_threads_created_in_the_span() {
        let directory = TempDir::new().expect("a temporary directory should open");
        let other = |thread: CodexThread| CodexThread {
            cwd: PathBuf::from("/other"),
            ..thread
        };
        write_database(
            directory.path(),
            "state_4.sqlite",
            &[StoredThread {
                id:         "old",
                originator: CODEX_LEGACY_TUI_ORIGINATOR,
                thread:     thread(directory.path(), 1_500, Some("from the old layout"), ""),
            }],
        );
        write_database(
            directory.path(),
            "state_5.sqlite",
            &[
                StoredThread {
                    id:         "named",
                    originator: CODEX_LEGACY_TUI_ORIGINATOR,
                    thread:     thread(directory.path(), 1_500, Some("codex test"), ""),
                },
                StoredThread {
                    id:         "typed",
                    originator: CODEX_LEGACY_TUI_ORIGINATOR,
                    thread:     other(thread(directory.path(), 2_000, None, "fix the build")),
                },
                StoredThread {
                    id:         "delegate",
                    originator: "tool-impl",
                    thread:     thread(directory.path(), 1_600, Some("tool-impl"), "phase 1"),
                },
                StoredThread {
                    id:         "early",
                    originator: CODEX_LEGACY_TUI_ORIGINATOR,
                    thread:     thread(directory.path(), 999, Some("before"), ""),
                },
                StoredThread {
                    id:         "late",
                    originator: CODEX_LEGACY_TUI_ORIGINATOR,
                    thread:     thread(directory.path(), 2_001, Some("after"), ""),
                },
            ],
        );
        fs::write(directory.path().join("state_9.sqlite-wal"), "")
            .expect("the journal should write");

        let mut threads = read_threads(directory.path(), &(1_000..=2_000));
        threads.sort_by_key(|thread| thread.created_ms);

        assert_eq!(
            threads,
            [
                thread(directory.path(), 1_500, Some("codex test"), ""),
                other(thread(directory.path(), 2_000, None, "fix the build")),
            ]
        );
    }

    /// A directory with no database, and a database whose table lacks a
    /// column the query names, both give no threads rather than an
    /// error.
    #[test]
    fn a_missing_or_changed_database_gives_no_threads() {
        let directory = TempDir::new().expect("a temporary directory should open");
        assert_eq!(
            read_threads(directory.path(), &(0..=u64::MAX)),
            [] as [CodexThread; 0]
        );

        let connection = Connection::open(directory.path().join("state_6.sqlite"))
            .expect("the fixture database should open");
        connection
            .execute(
                "CREATE TABLE threads (id TEXT PRIMARY KEY, cwd TEXT NOT NULL)",
                [],
            )
            .expect("the fixture table should create");

        assert_eq!(
            read_threads(directory.path(), &(0..=u64::MAX)),
            [] as [CodexThread; 0]
        );
    }

    #[test]
    fn codex_interactive_threads_accept_user_sources_and_exclude_workers_and_exec() {
        let directory = TempDir::new().expect("fixture directory");
        write_database(directory.path(), "state_5.sqlite", &[]);
        let connection =
            Connection::open(directory.path().join("state_5.sqlite")).expect("fixture database");
        connection.execute_batch("ALTER TABLE threads ADD COLUMN source TEXT;
            ALTER TABLE threads ADD COLUMN thread_source TEXT;
            INSERT INTO threads (id, cwd, originator, created_at_ms, name, rollout_path, source, thread_source, title) VALUES
            ('user', '/work', 'roster', 1500, 'test-codex', '/user', 'vscode', 'user', ''),
            ('null-source', '/work', 'other', 1500, 'null-source', '/null', NULL, 'user', ''),
            ('worker', '/work', 'roster', 1500, 'worker', '/worker', 'vscode', NULL, ''),
            ('exec', '/work', 'roster', 1500, 'exec', '/exec', 'exec', 'user', ''),
            ('legacy', '/work', 'codex-tui', 1500, 'legacy', '/legacy', NULL, NULL, ''),
            ('legacy-exec', '/work', 'codex-tui', 1500, 'legacy-exec', '/legacy-exec', 'exec', NULL, ''),
            ('early', '/work', 'roster', 999, 'early', '/early', 'vscode', 'user', ''),
            ('late', '/work', 'roster', 2001, 'late', '/late', 'vscode', 'user', '');")
            .expect("modern thread fixtures");
        let mut names = read_threads(directory.path(), &(1000..=2000))
            .into_iter()
            .map(|thread| match thread.name {
                SessionName::Named(name) => name,
                SessionName::Unnamed => String::new(),
            })
            .collect::<Vec<_>>();
        names.sort();
        assert_eq!(names, ["legacy", "null-source", "test-codex"]);
    }

    #[test]
    fn codex_interactive_threads_keep_originator_only_layouts() {
        for extra_column in [
            "",
            "ALTER TABLE threads ADD COLUMN source TEXT;",
            "ALTER TABLE threads ADD COLUMN thread_source TEXT;",
        ] {
            let directory = TempDir::new().expect("fixture directory");
            let expected = thread(directory.path(), 1500, Some("legacy"), "");
            write_database(
                directory.path(),
                "state_5.sqlite",
                &[
                    StoredThread {
                        id:         "legacy",
                        originator: CODEX_LEGACY_TUI_ORIGINATOR,
                        thread:     expected.clone(),
                    },
                    StoredThread {
                        id:         "worker",
                        originator: "roster",
                        thread:     thread(directory.path(), 1500, Some("worker"), ""),
                    },
                ],
            );
            Connection::open(directory.path().join("state_5.sqlite"))
                .expect("fixture database")
                .execute_batch(extra_column)
                .expect("older layout");
            assert_eq!(read_threads(directory.path(), &(1000..=2000)), [expected]);
        }
    }

    /// Threads are read by id from the newest database whoever started
    /// them, a mesh's included; an id with no row is left out, and no
    /// ids, no database or a changed one read nothing.
    #[test]
    fn reads_threads_by_id_whoever_started_them() {
        let directory = TempDir::new().expect("a temporary directory should open");
        write_database(
            directory.path(),
            "state_4.sqlite",
            &[StoredThread {
                id:         "0199-named",
                originator: CODEX_LEGACY_TUI_ORIGINATOR,
                thread:     thread(directory.path(), 1_500, Some("from the old layout"), ""),
            }],
        );
        write_database(
            directory.path(),
            "state_5.sqlite",
            &[
                StoredThread {
                    id:         "0199-named",
                    originator: CODEX_LEGACY_TUI_ORIGINATOR,
                    thread:     thread(directory.path(), 1_500, Some("codex test"), ""),
                },
                StoredThread {
                    id:         "0199-mesh",
                    originator: "codex_mesh",
                    thread:     thread(directory.path(), 2_000, None, "phase 1"),
                },
                StoredThread {
                    id:         "0199-other",
                    originator: CODEX_LEGACY_TUI_ORIGINATOR,
                    thread:     thread(directory.path(), 2_500, Some("not asked for"), ""),
                },
            ],
        );
        let ids = ["0199-named", "0199-mesh", "0199-gone"].map(str::to_string);

        assert_eq!(
            OpenThreadRows::default().read(directory.path(), &ids),
            &HashMap::from([
                (
                    "0199-named".to_string(),
                    thread(directory.path(), 1_500, Some("codex test"), ""),
                ),
                (
                    "0199-mesh".to_string(),
                    thread(directory.path(), 2_000, None, "phase 1")
                ),
            ])
        );
        assert!(
            OpenThreadRows::default()
                .read(directory.path(), &[])
                .is_empty()
        );

        let changed = TempDir::new().expect("a temporary directory should open");
        let mut rows = OpenThreadRows::default();
        assert!(rows.read(changed.path(), &ids).is_empty());
        Connection::open(changed.path().join("state_6.sqlite"))
            .expect("the fixture database should open")
            .execute(
                "CREATE TABLE threads (id TEXT PRIMARY KEY, cwd TEXT NOT NULL)",
                [],
            )
            .expect("the fixture table should create");
        assert!(rows.read(changed.path(), &ids).is_empty());
    }

    /// A name wins over the first prompt; a prompt stands in for a
    /// missing or blank name, on one line and cut to its limit; a thread
    /// with neither has no label.
    #[test]
    fn a_label_is_the_name_else_the_first_prompt() {
        let home = Path::new("/fixture");
        assert_eq!(
            thread(home, 0, Some("codex test"), "fix the build")
                .label()
                .as_deref(),
            Some("codex test")
        );
        assert_eq!(
            thread(home, 0, Some("  "), "fix the\n  build\n")
                .label()
                .as_deref(),
            Some("fix the build")
        );
        assert_eq!(thread(home, 0, None, " \n").label(), None);

        let long = "word ".repeat(CODEX_PROMPT_LABEL_MAX);
        let label = thread(home, 0, None, &long)
            .label()
            .expect("a prompt should label");
        assert_eq!(label.chars().count(), CODEX_PROMPT_LABEL_MAX);
        assert!(label.ends_with(TRUNCATION_MARK));
    }
}

#[cfg(test)]
#[path = "codex_label_cut_tests.rs"]
mod label_cut_tests;
