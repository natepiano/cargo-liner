//! Read the WAL beside a ledger without opening the database.

use std::fs;
use std::io;
use std::path::Path;

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum WalSnapshot {
    Absent,
    Present(Vec<u8>),
}

pub(crate) fn read_wal(ledger: &Path) -> io::Result<WalSnapshot> {
    let mut path = ledger.as_os_str().to_os_string();
    path.push("-wal");
    match fs::read(path) {
        Ok(bytes) => Ok(WalSnapshot::Present(bytes)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(WalSnapshot::Absent),
        Err(error) => Err(error),
    }
}
