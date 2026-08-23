//! Descriptor-held serialization for every ledger mutation.

use std::fmt;
use std::fs::File;
use std::fs::OpenOptions;
use std::path::Path;

/// A held advisory lock whose descriptor releases automatically on process death.
pub(super) struct MutationLock {
    descriptor: File,
}

impl MutationLock {
    /// Open and acquire the ledger's mutation lock.
    pub(super) fn acquire(lock_path: &Path) -> Result<Self, MutationLockError> {
        let descriptor = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(lock_path)?;
        descriptor.lock()?;
        Ok(Self { descriptor })
    }
}

impl Drop for MutationLock {
    fn drop(&mut self) {
        // Closing the descriptor is sufficient if this explicit unlock fails.
        std::mem::drop(self.descriptor.unlock());
    }
}

/// A failure while acquiring or releasing a descriptor-held mutation lock.
#[derive(Debug)]
pub(crate) enum MutationLockError {
    /// Opening or locking the descriptor failed.
    Io(std::io::Error),
}

impl fmt::Display for MutationLockError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "could not acquire ledger mutation lock: {error}"),
        }
    }
}

impl std::error::Error for MutationLockError {}

impl From<std::io::Error> for MutationLockError {
    fn from(error: std::io::Error) -> Self { Self::Io(error) }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::env;
    use std::fs;
    use std::path::Path;
    use std::process::Command;
    use std::thread;
    use std::time::Duration;

    use tempfile::tempdir;

    use super::MutationLock;

    const HOLDER_LOCK_PATH_ENVIRONMENT: &str = "CARGO_BERTH_TEST_LOCK_PATH";
    const HOLDER_READY_PATH_ENVIRONMENT: &str = "CARGO_BERTH_TEST_READY_PATH";
    const HOLDER_STOP_PATH_ENVIRONMENT: &str = "CARGO_BERTH_TEST_STOP_PATH";
    const HOLDER_TEST_NAME: &str = "ledger::lock::tests::terminated_process_releases_the_lock";
    const READY_WAIT_ATTEMPTS: usize = 100;
    const READY_WAIT_INTERVAL: Duration = Duration::from_millis(10);

    #[test]
    fn terminated_process_releases_the_lock() {
        if let Some(lock_path) = env::var_os(HOLDER_LOCK_PATH_ENVIRONMENT) {
            hold_lock_until_terminated(Path::new(&lock_path));
            return;
        }

        let temporary_directory = tempdir().expect("temporary directory should exist");
        let lock_path = temporary_directory.path().join("mutation.lock");
        let ready_path = temporary_directory.path().join("ready");
        let stop_path = temporary_directory.path().join("stop");
        let mut holder = Command::new(env::current_exe().expect("test executable should resolve"))
            .args(["--exact", HOLDER_TEST_NAME, "--nocapture"])
            .env(HOLDER_LOCK_PATH_ENVIRONMENT, &lock_path)
            .env(HOLDER_READY_PATH_ENVIRONMENT, &ready_path)
            .env(HOLDER_STOP_PATH_ENVIRONMENT, &stop_path)
            .spawn()
            .expect("lock holder should start");

        for _ in 0..READY_WAIT_ATTEMPTS {
            if ready_path.is_file() {
                break;
            }
            thread::sleep(READY_WAIT_INTERVAL);
        }
        assert!(ready_path.is_file());
        holder.kill().expect("lock holder should terminate");
        holder.wait().expect("terminated holder should reap");

        assert!(MutationLock::acquire(&lock_path).is_ok());
    }

    fn hold_lock_until_terminated(lock_path: &Path) {
        let lock_holder = MutationLock::acquire(lock_path).expect("holder should acquire lock");
        let ready_path = env::var_os(HOLDER_READY_PATH_ENVIRONMENT)
            .expect("holder ready path should be provided");
        let stop_path =
            env::var_os(HOLDER_STOP_PATH_ENVIRONMENT).expect("holder stop path should be provided");
        fs::write(ready_path, b"ready").expect("holder should report readiness");
        while !Path::new(&stop_path).exists() {
            thread::sleep(READY_WAIT_INTERVAL);
        }
        std::mem::drop(lock_holder);
    }
}
