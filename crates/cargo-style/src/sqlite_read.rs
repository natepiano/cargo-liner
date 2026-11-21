//! Read a SQLite database without creating sidecars beside it.

use std::fs;
use std::fs::Metadata;
use std::io;
use std::io::ErrorKind;
use std::os::unix::fs::MetadataExt as _;
use std::path::Path;
use std::path::PathBuf;

use rusqlite::Connection;
use rusqlite::Error as SqliteError;
use rusqlite::OpenFlags;
use url::Url;

const WAL_SUFFIX: &str = "-wal";

#[derive(PartialEq, Eq)]
pub(crate) struct DatabaseFiles {
    main:     FileVersion,
    wal:      WalFile,
    shared:   bool,
    rollback: bool,
}

#[derive(PartialEq, Eq)]
enum WalFile {
    Absent,
    Present(FileVersion),
}

#[derive(PartialEq, Eq)]
struct FileVersion {
    device:   u64,
    inode:    u64,
    length:   u64,
    modified: (i64, i64),
    changed:  (i64, i64),
}

impl From<Metadata> for FileVersion {
    fn from(file: Metadata) -> Self {
        Self {
            device:   file.dev(),
            inode:    file.ino(),
            length:   file.len(),
            modified: (file.mtime(), file.mtime_nsec()),
            changed:  (file.ctime(), file.ctime_nsec()),
        }
    }
}

impl DatabaseFiles {
    pub(crate) fn read(path: &Path) -> io::Result<Self> {
        let main = fs::metadata(path)?.into();
        let wal = match fs::metadata(sidecar(path, WAL_SUFFIX)) {
            Ok(file) => WalFile::Present(file.into()),
            Err(error) if error.kind() == ErrorKind::NotFound => WalFile::Absent,
            Err(error) => return Err(error),
        };
        Ok(Self {
            main,
            wal,
            shared: sidecar(path, "-shm").try_exists()?,
            rollback: sidecar(path, "-journal").try_exists()?,
        })
    }

    pub(crate) const fn is_quiet(&self) -> bool { !self.uses_wal() && !self.rollback }

    pub(crate) const fn uses_wal(&self) -> bool {
        !matches!(self.wal, WalFile::Absent) || self.shared
    }

    /// Let SQLite build its WAL index in the private directory.
    pub(crate) fn copy_wal_database(&self, path: &Path, copy: &Path) -> io::Result<()> {
        let _ = fs::copy(path, copy)?;
        if matches!(self.wal, WalFile::Present(_)) {
            let _ = fs::copy(sidecar(path, WAL_SUFFIX), sidecar(copy, WAL_SUFFIX))?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
pub(crate) enum DatabaseReadMode {
    Locked,
    Immutable,
}

pub(crate) fn open_read_only(
    path: &Path,
    mode: DatabaseReadMode,
) -> Result<Connection, SqliteError> {
    let mut uri =
        Url::from_file_path(path).map_err(|()| SqliteError::InvalidPath(path.to_owned()))?;
    uri.set_query(Some(match mode {
        DatabaseReadMode::Locked => "mode=ro",
        DatabaseReadMode::Immutable => "mode=ro&immutable=1",
    }));
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI;
    Connection::open_with_flags(uri.as_str(), flags)
}

fn sidecar(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    name.into()
}
