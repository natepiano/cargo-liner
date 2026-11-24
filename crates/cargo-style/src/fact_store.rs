//! Store immutable blob facts in the main repository's selection database.

use std::env;
use std::fs;
use std::io;
use std::path::Component;
use std::path::MAIN_SEPARATOR_STR;
use std::path::Path;
use std::path::PathBuf;
use std::str;
use std::thread;
use std::time::Instant;

use gix::ObjectId;
use gix::config::path::interpolate;
use rusqlite::Connection;
use rusqlite::ErrorCode;
use rusqlite::OptionalExtension;
use rusqlite::TransactionBehavior;
use rusqlite::params;

use crate::cache_constants::FACT_DATABASE_NAME;
use crate::constants::BUSY_TIMEOUT;
use crate::constants::WAL_RETRY_INTERVAL;
use crate::facts::FactReading;
use crate::sqlite_read;
use crate::sqlite_read::DatabaseFiles;
use crate::sqlite_read::DatabaseReadMode;
use crate::state_directory::STATE_DIRECTORY;

pub(crate) struct FactStore {
    database:       Connection,
    reader_version: ReaderVersion,
    access:         FactStoreAccess,
}

enum FactStoreAccess {
    Writable,
    QuietCache {
        path:  PathBuf,
        files: Box<DatabaseFiles>,
    },
}

impl FactStoreAccess {
    fn verify(&self) -> Result<(), CacheFailure> {
        if let Self::QuietCache { path, files } = self
            && DatabaseFiles::read(path)? != **files
        {
            return Err(CacheFailure::CacheChanged);
        }
        Ok(())
    }
}

const CREATE_FACTS: &str = "CREATE TABLE IF NOT EXISTS blob_facts (
    blob_id TEXT NOT NULL,
    reader_version TEXT NOT NULL,
    reading TEXT NOT NULL,
    PRIMARY KEY (blob_id, reader_version)
) WITHOUT ROWID";
const SELECT_FACT: &str =
    "SELECT reading FROM blob_facts WHERE blob_id = ?1 AND reader_version = ?2";
const INSERT_FACT: &str =
    "INSERT OR IGNORE INTO blob_facts (blob_id, reader_version, reading) VALUES (?1, ?2, ?3)";

/// The fingerprint of the source code that defines a fact row.
struct ReaderVersion(&'static str);

impl ReaderVersion {
    const CURRENT: Self = Self(env!("CARGO_STYLE_READER_VERSION"));
}

/// The cache connection or the first failure that prevents its use.
pub(crate) enum FactCache {
    Open(FactStore),
    Unavailable(CacheFailure),
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum CacheFailure {
    #[error("{0}")]
    Location(#[from] io::Error),
    #[error("{0}")]
    Database(#[from] rusqlite::Error),
    #[error("cannot encode or decode blob facts: {0}")]
    Facts(#[from] serde_json::Error),
    #[error("SQLite returned journal mode {0} when WAL was requested")]
    JournalMode(String),
    #[error("the fact cache has an active journal or changed during the read")]
    CacheChanged,
}

/// The storage state of one requested blob at the current reader version.
pub(crate) enum CachedFact {
    Found(String),
    Missing,
    Unavailable,
}

/// A new fact row, encoded before the write transaction starts.
pub(crate) struct NewFactRow {
    blob:    ObjectId,
    reading: String,
}

impl NewFactRow {
    pub(crate) fn encode(blob: ObjectId, reading: &FactReading) -> Result<Self, CacheFailure> {
        Ok(Self {
            blob,
            reading: serde_json::to_string(reading)?,
        })
    }
}

impl FactCache {
    pub(crate) fn open(common_dir: &Path) -> Self {
        match FactStore::open(common_dir) {
            Ok(store) => Self::Open(store),
            Err(reason) => Self::Unavailable(reason),
        }
    }

    pub(crate) fn open_existing(common_dir: &Path) -> Self {
        match FactStore::open_existing(common_dir) {
            Ok(store) => Self::Open(store),
            Err(reason) => Self::Unavailable(reason),
        }
    }

    pub(crate) fn lookup(&mut self, blob: ObjectId) -> CachedFact {
        let Self::Open(store) = self else {
            return CachedFact::Unavailable;
        };
        match store.lookup(blob) {
            Ok(found) => found,
            Err(reason) => {
                *self = Self::Unavailable(reason);
                CachedFact::Unavailable
            },
        }
    }

    pub(crate) fn disable(&mut self, reason: CacheFailure) {
        if matches!(self, Self::Open(_)) {
            *self = Self::Unavailable(reason);
        }
    }

    pub(crate) fn store_batch(&mut self, rows: &[NewFactRow]) {
        if let Self::Open(store) = self
            && let Err(reason) = store.insert(rows)
        {
            *self = Self::Unavailable(reason);
        }
    }

    pub(crate) fn finish(self) {
        if let Self::Unavailable(reason) = self {
            eprintln!("cargo-style: fact cache unavailable: {reason}");
        }
    }
}

impl FactStore {
    fn open(common_dir: &Path) -> Result<Self, CacheFailure> {
        let path = database_path(common_dir)?;
        if let Some(directory) = path.parent() {
            fs::create_dir_all(directory)?;
        }
        let database = Connection::open(path)?;
        database.busy_timeout(BUSY_TIMEOUT)?;
        enable_wal(&database)?;
        database.execute_batch(CREATE_FACTS)?;
        Ok(Self {
            database,
            reader_version: ReaderVersion::CURRENT,
            access: FactStoreAccess::Writable,
        })
    }

    fn open_existing(common_dir: &Path) -> Result<Self, CacheFailure> {
        let path = database_path(common_dir)?.canonicalize()?;
        let files = DatabaseFiles::read(&path)?;
        if !files.is_quiet() {
            return Err(CacheFailure::CacheChanged);
        }
        let database = sqlite_read::open_read_only(&path, DatabaseReadMode::Immutable)?;
        Ok(Self {
            database,
            reader_version: ReaderVersion::CURRENT,
            access: FactStoreAccess::QuietCache {
                path,
                files: Box::new(files),
            },
        })
    }

    fn lookup(&self, blob: ObjectId) -> Result<CachedFact, CacheFailure> {
        self.access.verify()?;
        let mut select = self.database.prepare_cached(SELECT_FACT)?;
        let reading = select
            .query_row(params![blob.to_string(), self.reader_version.0], |row| {
                row.get::<_, String>(0)
            })
            .optional()?;
        self.access.verify()?;
        Ok(reading.map_or(CachedFact::Missing, CachedFact::Found))
    }

    fn insert(&mut self, rows: &[NewFactRow]) -> Result<(), CacheFailure> {
        if rows.is_empty() {
            return Ok(());
        }
        let transaction = self
            .database
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        {
            let mut insert = transaction.prepare_cached(INSERT_FACT)?;
            for row in rows {
                let _ = insert.execute(params![
                    row.blob.to_string(),
                    self.reader_version.0,
                    row.reading
                ])?;
            }
        }
        transaction.commit()?;
        Ok(())
    }
}

fn database_path(common_dir: &Path) -> io::Result<PathBuf> {
    let common_dir = common_dir.canonicalize()?;
    let main_repository = common_dir
        .parent()
        .ok_or_else(|| io::Error::other("the Git common directory has no parent"))?;
    let state = match env::var_os(STATE_DIRECTORY) {
        Some(path) => expand_home(path.as_ref())?,
        None => home_directory()?.join(".local/state/style"),
    };
    let directory = state.join(main_repository.file_name().unwrap_or_default());
    Ok(directory.join(FACT_DATABASE_NAME))
}

fn expand_home(path: &Path) -> io::Result<PathBuf> {
    let mut components = path.components();
    let first = match components.next() {
        Some(Component::CurDir) => components.next(),
        first => first,
    };
    let Some(Component::Normal(first)) = first else {
        return Ok(path.to_owned());
    };
    let Some(user) = first.as_encoded_bytes().strip_prefix(b"~") else {
        return Ok(path.to_owned());
    };
    let home = if user.is_empty() {
        home_directory()?
    } else {
        let user = str::from_utf8(user).map_err(io::Error::other)?;
        interpolate::home_for_user(user).ok_or_else(|| {
            io::Error::other(format!(
                "cannot expand the home directory in {STATE_DIRECTORY}"
            ))
        })?
    };
    Ok(home.join(components.as_path()))
}

fn home_directory() -> io::Result<PathBuf> {
    match env::var_os("HOME") {
        Some(home) if home.is_empty() => Ok(PathBuf::from(MAIN_SEPARATOR_STR)),
        Some(home) => Ok(home.into()),
        None => dirs::home_dir().ok_or_else(|| io::Error::other("cannot find the home directory")),
    }
}

fn enable_wal(database: &Connection) -> Result<(), CacheFailure> {
    let start = Instant::now();
    loop {
        let mode: String = database.pragma_query_value(None, "journal_mode", |row| row.get(0))?;
        if mode == "wal" {
            return Ok(());
        }

        match database
            .pragma_update_and_check(None, "journal_mode", "WAL", |row| row.get::<_, String>(0))
        {
            Ok(mode) if mode == "wal" => return Ok(()),
            Ok(mode) => return Err(CacheFailure::JournalMode(mode)),
            Err(error)
                if error.sqlite_error_code() == Some(ErrorCode::DatabaseBusy)
                    && start.elapsed() < BUSY_TIMEOUT =>
            {
                // SQLite can skip the busy handler during the first transition to WAL.
                thread::sleep(WAL_RETRY_INTERVAL);
            },
            Err(error) => return Err(error.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::env;
    use std::error::Error;
    use std::path::Path;

    use cargo_berth_test_support::FixtureCommand;
    use cargo_berth_test_support::fixture_tempdir;
    use gix::ObjectId;
    use rusqlite::Connection;

    use super::CREATE_FACTS;
    use super::CacheFailure;
    use super::CachedFact;
    use super::FactStore;
    use super::FactStoreAccess;
    use super::NewFactRow;
    use super::ReaderVersion;
    use super::enable_wal;
    use super::expand_home;
    use crate::cache_constants::FACT_DATABASE_NAME;
    use crate::facts::FactReading;

    #[test]
    fn an_empty_home_expands_a_tilde_from_the_root() -> Result<(), Box<dyn Error>> {
        const EMPTY_HOME_PROBE: &str = "CARGO_STYLE_TEST_EMPTY_HOME";
        if env::var_os(EMPTY_HOME_PROBE).is_some() {
            assert_eq!(expand_home(Path::new("./~/cache"))?, Path::new("/cache"));
            return Ok(());
        }

        let output = FixtureCommand::new(env::current_exe()?)
            .args([
                "--exact",
                "fact_store::tests::an_empty_home_expands_a_tilde_from_the_root",
            ])
            .env(EMPTY_HOME_PROBE, "1")
            .env("HOME", "")
            .output()?;
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
        Ok(())
    }

    #[test]
    fn both_writers_can_insert_the_same_keys_after_both_observe_misses()
    -> Result<(), Box<dyn Error>> {
        let directory = fixture_tempdir("cargo-style-duplicate-insert-");
        let path = directory.path().join(FACT_DATABASE_NAME);
        let open = || -> Result<FactStore, CacheFailure> {
            let database = Connection::open(&path)?;
            enable_wal(&database)?;
            database.execute_batch(CREATE_FACTS)?;
            Ok(FactStore {
                database,
                reader_version: ReaderVersion::CURRENT,
                access: FactStoreAccess::Writable,
            })
        };
        let mut first = open()?;
        let mut second = open()?;
        let blobs = [
            ObjectId::from_hex(b"1111111111111111111111111111111111111111")?,
            ObjectId::from_hex(b"2222222222222222222222222222222222222222")?,
        ];
        let reading = FactReading::parse(b"fn sample() {}\n")?;
        let [first_row, second_row] = blobs.map(|blob| NewFactRow::encode(blob, &reading));
        let rows = [first_row?, second_row?];

        for row in &rows {
            assert!(matches!(first.lookup(row.blob)?, CachedFact::Missing));
            assert!(matches!(second.lookup(row.blob)?, CachedFact::Missing));
        }

        first.insert(&rows)?;
        let changes = second.database.total_changes();
        second.insert(&rows)?;
        assert_eq!(second.database.total_changes(), changes);

        for store in [&first, &second] {
            for row in &rows {
                assert!(matches!(
                    store.lookup(row.blob)?,
                    CachedFact::Found(reading) if reading == row.reading
                ));
            }
        }
        Ok(())
    }
}
