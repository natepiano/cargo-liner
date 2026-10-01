//! Reading the threads interactive Codex sessions started, from the
//! `threads` table of Codex's database `~/.codex/state_<n>.sqlite`.
//!
//! An interactive `codex` hands its threads to an app server -- since
//! Codex 0.157 a daemon every `codex` on the machine shares -- so the
//! `codex` process holds no file that names its thread. What ties the
//! two together is where and when: Codex records each thread's
//! directory, creation time and the program that started it, and a
//! `codex` creates its first thread in its own directory as it starts.
//! [`classify`](super::classify) makes that match; this module reads
//! the rows it matches against.

use std::collections::HashMap;
use std::fs;
use std::ops::RangeInclusive;
use std::path::Path;
use std::path::PathBuf;

use rusqlite::Connection;
use rusqlite::Error;
use rusqlite::OpenFlags;
use rusqlite::params;

use crate::constants::CODEX_PROMPT_LABEL_MAX;
use crate::constants::CODEX_STATE_BUSY_TIMEOUT;
use crate::constants::CODEX_STATE_EXTENSION;
use crate::constants::CODEX_STATE_PREFIX;
use crate::constants::CODEX_THREAD_BY_ID_QUERY;
use crate::constants::CODEX_THREADS_QUERY;
use crate::constants::CODEX_TUI_ORIGINATOR;
use crate::constants::TRUNCATION_MARK;

/// One thread an interactive Codex started, as the census reads it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct CodexThread {
    /// The directory the thread runs in.
    pub(super) cwd:          PathBuf,
    /// When the thread was created, in unix milliseconds.
    pub(super) created_ms:   u64,
    /// The name the thread was given, where it has one.
    pub(super) name:         Option<String>,
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
        if let Some(name) = self.name.as_deref().map(one_line)
            && !name.is_empty()
        {
            return Some(name);
        }
        let prompt = one_line(&self.first_prompt);
        if prompt.is_empty() {
            return None;
        }
        if prompt.chars().count() <= CODEX_PROMPT_LABEL_MAX {
            return Some(prompt);
        }
        Some(
            prompt
                .chars()
                .take(CODEX_PROMPT_LABEL_MAX - 1)
                .chain(std::iter::once(TRUNCATION_MARK))
                .collect(),
        )
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
    let connection = open_read_only(path)?;
    let mut statement = connection.prepare(CODEX_THREAD_BY_ID_QUERY)?;
    let mut threads = HashMap::new();
    for id in ids {
        let found = statement.query_row(params![id], |row| {
            Ok(CodexThread {
                cwd:          PathBuf::from(row.get::<_, String>(0)?),
                created_ms:   u64::try_from(row.get::<_, i64>(1)?).unwrap_or_default(),
                name:         row.get(2)?,
                first_prompt: row.get(3)?,
                rollout:      PathBuf::from(row.get::<_, String>(4)?),
            })
        });
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
                .strip_prefix(CODEX_STATE_PREFIX)?
                .parse()
                .ok()?;
            Some((version, path))
        })
        .max_by_key(|(version, _)| *version)
        .map(|(_, path)| path)
}

/// The rows [`CODEX_THREADS_QUERY`] finds in the database at `path`,
/// opened read-only.
fn query_threads(path: &Path, created: &RangeInclusive<u64>) -> rusqlite::Result<Vec<CodexThread>> {
    let connection = open_read_only(path)?;
    let mut statement = connection.prepare(CODEX_THREADS_QUERY)?;
    let from = i64::try_from(*created.start()).unwrap_or(i64::MAX);
    let until = i64::try_from(*created.end()).unwrap_or(i64::MAX);
    statement
        .query_map(params![CODEX_TUI_ORIGINATOR, from, until], |row| {
            Ok(CodexThread {
                cwd:          PathBuf::from(row.get::<_, String>(0)?),
                created_ms:   u64::try_from(row.get::<_, i64>(1)?).unwrap_or_default(),
                name:         row.get(2)?,
                first_prompt: row.get(3)?,
                rollout:      PathBuf::from(row.get::<_, String>(4)?),
            })
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
                        thread.name,
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
    fn thread(created_ms: u64, name: Option<&str>, first_prompt: &str) -> CodexThread {
        CodexThread {
            cwd: PathBuf::from("/work"),
            created_ms,
            name: name.map(str::to_string),
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
                originator: CODEX_TUI_ORIGINATOR,
                thread:     thread(1_500, Some("from the old layout"), ""),
            }],
        );
        write_database(
            directory.path(),
            "state_5.sqlite",
            &[
                StoredThread {
                    id:         "named",
                    originator: CODEX_TUI_ORIGINATOR,
                    thread:     thread(1_500, Some("codex test"), ""),
                },
                StoredThread {
                    id:         "typed",
                    originator: CODEX_TUI_ORIGINATOR,
                    thread:     other(thread(2_000, None, "fix the build")),
                },
                StoredThread {
                    id:         "delegate",
                    originator: "tool-impl",
                    thread:     thread(1_600, Some("tool-impl"), "phase 1"),
                },
                StoredThread {
                    id:         "early",
                    originator: CODEX_TUI_ORIGINATOR,
                    thread:     thread(999, Some("before"), ""),
                },
                StoredThread {
                    id:         "late",
                    originator: CODEX_TUI_ORIGINATOR,
                    thread:     thread(2_001, Some("after"), ""),
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
                thread(1_500, Some("codex test"), ""),
                other(thread(2_000, None, "fix the build")),
            ]
        );
    }

    /// A directory with no database, and a database whose table lacks a
    /// column the query names, both give no threads rather than an
    /// error.
    #[test]
    fn a_missing_or_changed_database_gives_no_threads() {
        let directory = TempDir::new().expect("a temporary directory should open");
        assert!(read_threads(directory.path(), &(0..=u64::MAX)).is_empty());

        let connection = Connection::open(directory.path().join("state_6.sqlite"))
            .expect("the fixture database should open");
        connection
            .execute(
                "CREATE TABLE threads (id TEXT PRIMARY KEY, cwd TEXT NOT NULL)",
                [],
            )
            .expect("the fixture table should create");

        assert!(read_threads(directory.path(), &(0..=u64::MAX)).is_empty());
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
                originator: CODEX_TUI_ORIGINATOR,
                thread:     thread(1_500, Some("from the old layout"), ""),
            }],
        );
        write_database(
            directory.path(),
            "state_5.sqlite",
            &[
                StoredThread {
                    id:         "0199-named",
                    originator: CODEX_TUI_ORIGINATOR,
                    thread:     thread(1_500, Some("codex test"), ""),
                },
                StoredThread {
                    id:         "0199-mesh",
                    originator: "codex_mesh",
                    thread:     thread(2_000, None, "phase 1"),
                },
                StoredThread {
                    id:         "0199-other",
                    originator: CODEX_TUI_ORIGINATOR,
                    thread:     thread(2_500, Some("not asked for"), ""),
                },
            ],
        );
        let ids = ["0199-named", "0199-mesh", "0199-gone"].map(str::to_string);

        assert_eq!(
            read_threads_by_id(directory.path(), &ids),
            HashMap::from([
                (
                    "0199-named".to_string(),
                    thread(1_500, Some("codex test"), ""),
                ),
                ("0199-mesh".to_string(), thread(2_000, None, "phase 1")),
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
        assert_eq!(
            thread(0, Some("codex test"), "fix the build")
                .label()
                .as_deref(),
            Some("codex test")
        );
        assert_eq!(
            thread(0, Some("  "), "fix the\n  build\n")
                .label()
                .as_deref(),
            Some("fix the build")
        );
        assert_eq!(thread(0, None, " \n").label(), None);

        let long = "word ".repeat(CODEX_PROMPT_LABEL_MAX);
        let label = thread(0, None, &long)
            .label()
            .expect("a prompt should label");
        assert_eq!(label.chars().count(), CODEX_PROMPT_LABEL_MAX);
        assert!(label.ends_with(TRUNCATION_MARK));
    }
}
