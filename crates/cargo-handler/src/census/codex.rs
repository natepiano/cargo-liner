//! Reading the threads interactive Codex sessions started, from the
//! `threads` table of Codex's database `~/.codex/state_<n>.sqlite`.
//!
//! Log rows identify resumed threads by process. A managed daemon can serve
//! several clients. Only a daemon with one current client supplies an exact link.
//! The startup window remains the fallback when no log link exists.

use std::collections::HashMap;
use std::collections::HashSet;
use std::fs;
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
use crate::constants::CODEX_DIRNAME;
use crate::constants::CODEX_LAST_THREAD_QUERY;
use crate::constants::CODEX_LEGACY_THREADS_QUERY;
use crate::constants::CODEX_LEGACY_TUI_ORIGINATOR;
use crate::constants::CODEX_LOG_INITIAL_QUERY;
use crate::constants::CODEX_LOG_MAX_ID_QUERY;
use crate::constants::CODEX_LOG_NEW_QUERY;
use crate::constants::CODEX_LOG_PREFIX;
use crate::constants::CODEX_LOG_PROCESS_PREFIX;
use crate::constants::CODEX_MANAGED_DAEMON_ARGUMENT;
use crate::constants::CODEX_PROMPT_LABEL_MAX;
use crate::constants::CODEX_RESUME_LAST_ARGUMENT;
use crate::constants::CODEX_RESUME_SUBCOMMAND;
use crate::constants::CODEX_STATE_BUSY_TIMEOUT;
use crate::constants::CODEX_STATE_EXTENSION;
use crate::constants::CODEX_STATE_PREFIX;
use crate::constants::CODEX_THREAD_BY_ID_QUERY;
use crate::constants::CODEX_THREADS_QUERY;
#[cfg(test)]
use crate::constants::TRUNCATION_MARK;
use crate::constants::UUID_TEXT_LENGTH;
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

/// A resume can identify a thread explicitly or select the latest in the cwd.
pub(super) fn resume_request(process: &ProcessEntry) -> ResumeRequest<'_> {
    process
        .arguments
        .windows(2)
        .find_map(|pair| {
            if pair[0] != CODEX_RESUME_SUBCOMMAND {
                return None;
            }
            match pair[1].as_str() {
                CODEX_RESUME_LAST_ARGUMENT => Some(ResumeRequest::LastThread),
                id if id.len() == UUID_TEXT_LENGTH => Some(ResumeRequest::NamedId(id)),
                _ => None,
            }
        })
        .unwrap_or(ResumeRequest::NewSession)
}

/// Older resume candidates from this process's own home, outside the startup window.
pub(super) fn read_resumed_threads(process: &ProcessEntry, codex_home: &Path) -> Vec<CodexThread> {
    match resume_request(process) {
        ResumeRequest::NewSession => Vec::new(),
        ResumeRequest::NamedId(id) => read_threads_by_id(codex_home, &[id.to_owned()])
            .into_values()
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
pub(super) fn read_threads(codex_dir: &Path, created: &RangeInclusive<u64>) -> Vec<CodexThread> {
    state_database(codex_dir)
        .and_then(|path| query_threads(&path, created).ok())
        .unwrap_or_default()
}

/// The threads among `ids` found in the newest thread database in
/// `codex_dir`, whoever started them, by id.
///
/// None when there is no database or it cannot be read as this build
/// expects; a thread's row then goes by its id.
pub(super) fn read_threads_by_id(codex_dir: &Path, ids: &[String]) -> HashMap<String, CodexThread> {
    if ids.is_empty() {
        return HashMap::new();
    }
    state_database(codex_dir)
        .and_then(|path| query_threads_by_id(&path, ids).ok())
        .unwrap_or_default()
}

/// The rows [`CODEX_THREAD_BY_ID_QUERY`] finds for each of `ids` in the
/// database at `path`, opened read-only.
fn query_threads_by_id(
    path: &Path,
    ids: &[String],
) -> rusqlite::Result<HashMap<String, CodexThread>> {
    let account_home = CodexAccountHome::from_database(path)?;
    let connection = open_read_only(path)?;
    let mut statement = connection.prepare(CODEX_THREAD_BY_ID_QUERY)?;
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
    Ok(threads)
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

/// A log link can be absent, exact, or unsafe because the daemon has clients.
pub(super) enum LogThreadLink<'a> {
    Linked(&'a CodexThread),
    NoRecord,
    SharedDaemon,
}

/// Retained log cursors and process links, separated by lexical account home.
#[derive(Debug, Default)]
pub(super) struct CodexLogs {
    databases: HashMap<PathBuf, LogDatabase>,
}

impl CodexLogs {
    /// Advance each account log once. Unchanged logs return no old rows.
    pub(super) fn refresh(&mut self, processes: &[ProcessEntry], home: Option<&Path>) {
        let homes = log_clients(processes, home);
        self.databases
            .retain(|account, _| homes.contains_key(account));
        for (account, clients) in homes {
            if self.refresh_account(&account, &clients).is_err() {
                self.databases.remove(&account);
            }
        }
    }

    fn refresh_account(&mut self, account: &Path, clients: &LogClients) -> rusqlite::Result<()> {
        let metadata = account
            .metadata()
            .map_err(|_| Error::InvalidPath(account.to_owned()))?;
        let directory_stamp = (metadata.mtime(), metadata.mtime_nsec());
        let retained = self.databases.get(account);
        let thread_path = if let Some(database) = retained
            && database.directory_stamp == directory_stamp
        {
            database.thread_path.clone()
        } else {
            state_database(account).ok_or_else(|| Error::InvalidPath(account.to_owned()))?
        };
        let path = if let Some(database) = retained
            && database.directory_stamp == directory_stamp
        {
            database.path.clone()
        } else {
            versioned_database(account, CODEX_LOG_PREFIX)
                .ok_or_else(|| Error::InvalidPath(account.to_owned()))?
        };
        let metadata = path
            .metadata()
            .map_err(|_| Error::InvalidPath(path.clone()))?;
        let identity = (metadata.dev(), metadata.ino());
        if retained.is_none_or(|database| database.path != path || database.identity != identity) {
            let database = LogDatabase::open(
                account,
                path,
                identity,
                directory_stamp,
                clients.oldest_start,
            )?;
            self.databases.insert(account.to_owned(), database);
        }
        if let Some(database) = self.databases.get_mut(account) {
            database.directory_stamp = directory_stamp;
            database.refresh_thread_database(thread_path)?;
            database.advance(clients)?;
        }
        Ok(())
    }

    /// Choose the newest exact record across the process and its children.
    pub(super) fn link<'a>(
        &'a self,
        process: &ProcessEntry,
        processes: &[ProcessEntry],
        home: Option<&Path>,
    ) -> LogThreadLink<'a> {
        let Some(account) = session_home(process, home) else {
            return LogThreadLink::NoRecord;
        };
        let Some(database) = self.databases.get(&account) else {
            return LogThreadLink::NoRecord;
        };
        let sources = std::iter::once(process).chain(processes.iter().filter(|child| {
            child.parent == Some(process.pid)
                && child.is_codex()
                && child.is_app_server()
                && session_home(child, home).as_ref() == Some(&account)
        }));
        let mut latest = None;
        let mut ambiguity = LogThreadLink::NoRecord;
        for source in sources {
            let Some(record) = database.latest.get(&source.pid) else {
                continue;
            };
            if record.logged_at < source.started.max(process.started) {
                continue;
            }
            match record.ownership {
                LogOwnership::SharedDaemon => ambiguity = LogThreadLink::SharedDaemon,
                LogOwnership::Exact
                    if database.user_thread(&record.thread_id).is_some()
                        && latest.is_none_or(|kept: &LoggedThread| kept.id < record.id) =>
                {
                    latest = Some(record);
                },
                LogOwnership::Exact | LogOwnership::AwaitingSnapshot => {},
            }
        }
        latest
            .and_then(|record| database.user_thread(&record.thread_id))
            .map_or(ambiguity, LogThreadLink::Linked)
    }

    #[cfg(test)]
    pub(super) fn thread_lookups(&self) -> usize {
        self.databases
            .values()
            .map(|database| database.thread_lookups)
            .sum()
    }

    #[cfg(test)]
    pub(super) fn rows_read(&self) -> usize {
        self.databases
            .values()
            .map(|database| database.rows_read)
            .sum()
    }
}

/// The processes whose records can name a live interactive client.
struct LogClients {
    oldest_start: u64,
    processes:    HashMap<u32, LogProcess>,
}

struct LogProcess {
    started:   u64,
    ownership: LogOwnership,
}

/// Ambiguity belongs to the record, even after a client exits.
#[derive(Clone, Copy, Debug)]
enum LogOwnership {
    Exact,
    SharedDaemon,
    AwaitingSnapshot,
}

fn log_clients(processes: &[ProcessEntry], home: Option<&Path>) -> HashMap<PathBuf, LogClients> {
    let table = classify::pid_table(processes);
    let interactive = classify::interactive_codex(&table, processes);
    let mut homes = HashMap::<PathBuf, LogClients>::new();
    for process in interactive {
        if let Some(account) = session_home(process, home) {
            let clients = homes.entry(account).or_insert_with(|| LogClients {
                oldest_start: process.started,
                processes:    HashMap::new(),
            });
            clients.oldest_start = clients.oldest_start.min(process.started);
            clients.processes.insert(
                process.pid,
                LogProcess {
                    started:   process.started,
                    ownership: LogOwnership::Exact,
                },
            );
        }
    }
    for (account, clients) in &mut homes {
        let client_count = clients.processes.len();
        let children: Vec<_> = processes
            .iter()
            .filter(|child| {
                child.is_codex()
                    && child.is_app_server()
                    && child
                        .parent
                        .is_some_and(|parent| clients.processes.contains_key(&parent))
                    && session_home(child, home).as_ref() == Some(account)
            })
            .collect();
        for child in children {
            // A managed daemon supplies an exact link only for one current client.
            let ownership = if client_count > 1
                && child
                    .arguments
                    .iter()
                    .any(|arg| arg == CODEX_MANAGED_DAEMON_ARGUMENT)
            {
                LogOwnership::SharedDaemon
            } else {
                LogOwnership::Exact
            };
            clients.processes.insert(
                child.pid,
                LogProcess {
                    started: child.started,
                    ownership,
                },
            );
        }
    }
    homes
}

/// The first read uses process time. Later reads use the retained primary key.
#[derive(Debug)]
enum LogCursor {
    FromProcessStart(u64),
    AfterId(i64),
}

#[derive(Debug)]
struct LoggedThread {
    id:        i64,
    logged_at: u64,
    thread_id: String,
    ownership: LogOwnership,
}

/// A missing row can arrive after its log record commits.
#[derive(Debug)]
enum LoggedThreadKind {
    User(CodexThread),
    Other,
    Unresolved,
}

/// Open databases and resolved identities for one version of an account log.
#[derive(Debug)]
struct LogDatabase {
    path:            PathBuf,
    identity:        (u64, u64),
    directory_stamp: (i64, i64),
    logs:            Connection,
    threads:         Connection,
    thread_path:     PathBuf,
    thread_identity: (u64, u64),
    layout:          ThreadsLayout,
    account:         CodexAccountHome,
    cursor:          LogCursor,
    latest:          HashMap<u32, LoggedThread>,
    pending:         Vec<(u32, LoggedThread)>,
    unseen:          Vec<(u32, LoggedThread)>,
    known:           HashMap<String, LoggedThreadKind>,
    thread_revision: ThreadRevision,
    rows_read:       usize,
    #[cfg(test)]
    thread_lookups:  usize,
}

/// SQLite changes this value when another connection commits.
#[derive(Debug, Default, PartialEq)]
enum ThreadRevision {
    #[default]
    Unread,
    Read(i64),
}

impl LogDatabase {
    fn open(
        account: &Path,
        path: PathBuf,
        identity: (u64, u64),
        directory_stamp: (i64, i64),
        start: u64,
    ) -> rusqlite::Result<Self> {
        let state =
            state_database(account).ok_or_else(|| Error::InvalidPath(account.to_owned()))?;
        let metadata = state
            .metadata()
            .map_err(|_| Error::InvalidPath(state.clone()))?;
        let thread_identity = (metadata.dev(), metadata.ino());
        let threads = open_read_only(&state)?;
        let layout = ThreadsLayout::read(&threads)?;
        Ok(Self {
            logs: open_read_only(&path)?,
            path,
            identity,
            directory_stamp,
            threads,
            thread_path: state,
            thread_identity,
            layout,
            account: CodexAccountHome::from(account),
            cursor: LogCursor::FromProcessStart(start),
            latest: HashMap::new(),
            pending: Vec::new(),
            unseen: Vec::new(),
            known: HashMap::new(),
            thread_revision: ThreadRevision::Unread,
            rows_read: 0,
            #[cfg(test)]
            thread_lookups: 0,
        })
    }

    /// Reopen the selected thread database after a version or file change.
    fn refresh_thread_database(&mut self, path: PathBuf) -> rusqlite::Result<()> {
        let metadata = path
            .metadata()
            .map_err(|_| Error::InvalidPath(path.clone()))?;
        let identity = (metadata.dev(), metadata.ino());
        if self.thread_path != path || self.thread_identity != identity {
            let threads = open_read_only(&path)?;
            let layout = ThreadsLayout::read(&threads)?;
            self.threads = threads;
            self.layout = layout;
            self.thread_path = path;
            self.thread_identity = identity;
            self.known.clear();
            self.thread_revision = ThreadRevision::Unread;
        }
        Ok(())
    }

    fn advance(&mut self, clients: &LogClients) -> rusqlite::Result<()> {
        #[cfg(test)]
        {
            self.thread_lookups = 0;
        }
        self.latest.retain(|pid, record| {
            clients
                .processes
                .get(pid)
                .is_some_and(|process| record.logged_at >= process.started)
        });
        self.pending.retain(|(pid, record)| {
            clients
                .processes
                .get(pid)
                .is_some_and(|process| record.logged_at >= process.started)
        });
        for (pid, record) in self
            .latest
            .iter_mut()
            .chain(self.pending.iter_mut().map(|(pid, record)| (&*pid, record)))
        {
            if clients
                .processes
                .get(pid)
                .is_some_and(|process| matches!(process.ownership, LogOwnership::SharedDaemon))
            {
                record.ownership = LogOwnership::SharedDaemon;
            }
        }
        // Retain an unknown pid until the next process snapshot can identify it.
        for (pid, mut record) in self.unseen.drain(..) {
            if let Some(process) = clients.processes.get(&pid)
                && record.logged_at >= process.started
            {
                record.ownership = process.ownership;
                self.pending.push((pid, record));
            }
        }
        self.read_new(clients)?;
        self.refresh_threads()?;
        let mut unresolved = Vec::new();
        for (pid, record) in self.pending.drain(..) {
            match self.known.get(&record.thread_id) {
                Some(LoggedThreadKind::User(_)) => {
                    if self.latest.get(&pid).is_none_or(|kept| kept.id < record.id) {
                        self.latest.insert(pid, record);
                    }
                },
                Some(LoggedThreadKind::Other) => {},
                Some(LoggedThreadKind::Unresolved) | None => unresolved.push((pid, record)),
            }
        }
        self.pending = unresolved;
        Ok(())
    }

    fn read_new(&mut self, clients: &LogClients) -> rusqlite::Result<()> {
        self.rows_read = 0;
        let (query, after) = match self.cursor {
            LogCursor::FromProcessStart(start) => (
                CODEX_LOG_INITIAL_QUERY,
                i64::try_from(start).unwrap_or(i64::MAX),
            ),
            LogCursor::AfterId(id) => (CODEX_LOG_NEW_QUERY, id),
        };
        let snapshot = self.logs.unchecked_transaction()?;
        let mut last_id = match self.cursor {
            LogCursor::AfterId(id) => id,
            LogCursor::FromProcessStart(_) => {
                snapshot.query_row(CODEX_LOG_MAX_ID_QUERY, [], |row| row.get(0))?
            },
        };
        let mut statement = snapshot.prepare_cached(query)?;
        let mut rows = statement.query([after])?;
        while let Some(row) = rows.next()? {
            self.rows_read += 1;
            let id: i64 = row.get(0)?;
            last_id = last_id.max(id);
            let Ok(process) = row.get_ref(2)?.as_str() else {
                continue;
            };
            let Some(pid) = process
                .strip_prefix(CODEX_LOG_PROCESS_PREFIX)
                .and_then(|process| process.split_once(':'))
                .and_then(|(pid, _)| pid.parse::<u32>().ok())
            else {
                continue;
            };
            let client = clients.processes.get(&pid);
            let logged_at = u64::try_from(row.get::<_, i64>(1)?).unwrap_or_default();
            if client.is_some_and(|client| logged_at < client.started) {
                continue;
            }
            let Some(thread_id) = row.get::<_, Option<String>>(3)? else {
                continue;
            };
            let record = LoggedThread {
                id,
                logged_at,
                thread_id,
                ownership: client.map_or(LogOwnership::AwaitingSnapshot, |client| client.ownership),
            };
            if client.is_some() {
                self.pending.push((pid, record));
            } else {
                self.unseen.push((pid, record));
            }
        }
        drop(rows);
        drop(statement);
        snapshot.commit()?;
        self.cursor = LogCursor::AfterId(last_id);
        Ok(())
    }

    fn refresh_threads(&mut self) -> rusqlite::Result<()> {
        let revision = ThreadRevision::Read(self.threads.query_row(
            "PRAGMA data_version",
            [],
            |row| row.get(0),
        )?);
        if revision != self.thread_revision {
            self.known.clear();
            self.thread_revision = revision;
        }
        let ids: HashSet<_> = self
            .latest
            .values()
            .chain(self.pending.iter().map(|(_, record)| record))
            .map(|record| record.thread_id.as_str())
            .collect();
        self.known.retain(|id, _| ids.contains(id.as_str()));
        if ids.iter().all(|id| {
            matches!(
                self.known.get(*id),
                Some(LoggedThreadKind::User(_) | LoggedThreadKind::Other)
            )
        }) {
            return Ok(());
        }
        let user_query = format!("{} AND id = ?4", self.layout.query());
        let mut users = self.threads.prepare_cached(&user_query)?;
        let mut any_thread = self.threads.prepare_cached(CODEX_THREAD_BY_ID_QUERY)?;
        for id in ids {
            if matches!(
                self.known.get(id),
                Some(LoggedThreadKind::User(_) | LoggedThreadKind::Other)
            ) {
                continue;
            }
            #[cfg(test)]
            {
                self.thread_lookups += 1;
            }
            let user = users
                .query_row(
                    params![CODEX_LEGACY_TUI_ORIGINATOR, 0, i64::MAX, id],
                    |row| decode_thread(row, &self.account),
                )
                .optional()?;
            let kind = if let Some(thread) = user {
                LoggedThreadKind::User(thread)
            } else if any_thread.exists([id])? {
                LoggedThreadKind::Other
            } else {
                LoggedThreadKind::Unresolved
            };
            self.known.insert(id.to_owned(), kind);
        }
        Ok(())
    }

    fn user_thread(&self, id: &str) -> Option<&CodexThread> {
        match self.known.get(id)? {
            LoggedThreadKind::User(thread) => Some(thread),
            LoggedThreadKind::Other | LoggedThreadKind::Unresolved => None,
        }
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

    const fn query(self) -> &'static str {
        match self {
            Self::WithThreadSource => CODEX_THREADS_QUERY,
            Self::OriginatorOnly => CODEX_LEGACY_THREADS_QUERY,
        }
    }
}

/// The interactive threads in the database at `path`,
/// opened read-only.
fn query_threads(path: &Path, created: &RangeInclusive<u64>) -> rusqlite::Result<Vec<CodexThread>> {
    let account_home = CodexAccountHome::from_database(path)?;
    let connection = open_read_only(path)?;
    let layout = ThreadsLayout::read(&connection)?;
    let mut statement = connection.prepare(layout.query())?;
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
            read_threads_by_id(directory.path(), &ids),
            HashMap::from([
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
        assert!(read_threads_by_id(directory.path(), &[]).is_empty());

        let changed = TempDir::new().expect("a temporary directory should open");
        assert!(read_threads_by_id(changed.path(), &ids).is_empty());
        Connection::open(changed.path().join("state_6.sqlite"))
            .expect("the fixture database should open")
            .execute(
                "CREATE TABLE threads (id TEXT PRIMARY KEY, cwd TEXT NOT NULL)",
                [],
            )
            .expect("the fixture table should create");
        assert!(read_threads_by_id(changed.path(), &ids).is_empty());
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
