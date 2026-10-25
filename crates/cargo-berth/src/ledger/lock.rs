//! Descriptor-held serialization for every ledger mutation.

use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;
use std::fs::File;
use std::fs::OpenOptions;
use std::fs::TryLockError;
use std::io::Write;
use std::path::Path;
use std::sync::mpsc;
use std::sync::mpsc::RecvTimeoutError;
use std::thread;
use std::time::Duration;
use std::time::Instant;

use super::constants;
use super::constants::MUTATION_LOCK_CONTENTION_TOLERANCE_ENVIRONMENT;
use super::constants::MUTATION_LOCK_READY_PATH_ENVIRONMENT;

/// A held advisory lock whose descriptor releases automatically on process death.
#[derive(Debug)]
pub(super) struct MutationLock {
    descriptor: File,
}

impl MutationLock {
    /// Open and acquire the ledger's mutation lock.
    pub(super) fn acquire(
        lock_path: &Path,
        acquisition_timeout: Duration,
    ) -> Result<Self, MutationLockError> {
        let acquisition_timeout = constants::shortened_by_environment(
            MUTATION_LOCK_CONTENTION_TOLERANCE_ENVIRONMENT,
            acquisition_timeout,
        );
        let descriptor = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(lock_path)?;
        match descriptor.try_lock() {
            Ok(()) => return Ok(Self { descriptor }),
            Err(TryLockError::Error(error)) => return Err(error.into()),
            Err(TryLockError::WouldBlock) => {},
        }
        if acquisition_timeout.is_zero() {
            return Err(MutationLockError::AcquisitionTimedOut);
        }
        let (sender, receiver) = mpsc::sync_channel(1);
        // Queue in the kernel instead of competing with later arrivals after a backoff.
        // On timeout the receiver closes; the worker then drops the acquired descriptor,
        // so an abandoned request can never leave a lock held in the background.
        let queued = Instant::now();
        thread::Builder::new()
            .name("ledger-lock-wait".to_owned())
            .spawn(move || {
                let acquired = descriptor.lock().and_then(|()| {
                    let waited = queued.elapsed();
                    if let Some(ready_path) = std::env::var_os(MUTATION_LOCK_READY_PATH_ENVIRONMENT)
                    {
                        let mut waits = OpenOptions::new()
                            .create(true)
                            .append(true)
                            .open(Path::new(&ready_path).with_extension("waits"))?;
                        writeln!(waits, "{}", waited.as_nanos())?;
                    }
                    Ok(Self { descriptor })
                });
                std::mem::drop(sender.send(acquired));
            })?;
        if let Some(ready_path) = std::env::var_os(MUTATION_LOCK_READY_PATH_ENVIRONMENT) {
            std::fs::write(ready_path, b"ready")?;
        }
        match receiver.recv_timeout(acquisition_timeout) {
            Ok(acquired) => acquired.map_err(MutationLockError::Io),
            Err(RecvTimeoutError::Timeout) => Err(MutationLockError::AcquisitionTimedOut),
            Err(RecvTimeoutError::Disconnected) => Err(MutationLockError::Io(
                std::io::Error::other("ledger lock waiter stopped before acquiring the lock"),
            )),
        }
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
    /// Another live holder retained the descriptor for the full retry window.
    AcquisitionTimedOut,
}

impl Display for MutationLockError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "could not acquire ledger mutation lock: {error}"),
            Self::AcquisitionTimedOut => formatter.write_str(
                "another cargo-berth operation is still running; wait for it to finish, then retry",
            ),
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

    use cargo_berth_test_support::ChildDeadlineExt as _;
    use tempfile::tempdir;

    use super::MutationLock;
    use super::MutationLockError;

    const HOLDER_LOCK_PATH_ENVIRONMENT: &str = "CARGO_BERTH_TEST_LOCK_PATH";
    const HOLDER_READY_PATH_ENVIRONMENT: &str = "CARGO_BERTH_TEST_READY_PATH";
    const HOLDER_STOP_PATH_ENVIRONMENT: &str = "CARGO_BERTH_TEST_STOP_PATH";
    const HOLDER_TEST_NAME: &str = "ledger::lock::tests::terminated_process_releases_the_lock";
    const LOCK_ACQUISITION_TIMEOUT: Duration = Duration::from_millis(100);
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
        holder
            .bounded_wait()
            .expect("terminated holder should reap");

        assert!(MutationLock::acquire(&lock_path, LOCK_ACQUISITION_TIMEOUT).is_ok());
    }

    #[test]
    fn a_second_holder_times_out_with_an_actionable_fact_free_error() {
        let temporary_directory = tempdir().expect("temporary directory should exist");
        let lock_path = temporary_directory.path().join("mutation.lock");
        let first_holder = MutationLock::acquire(&lock_path, LOCK_ACQUISITION_TIMEOUT)
            .expect("first holder should acquire");

        let error = MutationLock::acquire(&lock_path, Duration::ZERO)
            .expect_err("second holder should time out");
        let message = error.to_string();

        assert!(matches!(error, MutationLockError::AcquisitionTimedOut));
        assert!(message.contains("wait for it to finish"));
        assert!(message.contains("retry"));
        assert!(!message.contains("delete"));
        assert!(!message.contains(lock_path.to_string_lossy().as_ref()));
        std::mem::drop(first_holder);
    }

    #[test]
    fn queued_writer_acquires_without_a_retry_sleep_after_release() {
        let directory = tempdir().expect("scratch lock directory");
        let path = directory.path().join("mutation.lock");
        let holder =
            MutationLock::acquire(&path, LOCK_ACQUISITION_TIMEOUT).expect("initial holder");
        let (started, ready) = std::sync::mpsc::sync_channel(1);
        let (acquired, received) = std::sync::mpsc::sync_channel(1);
        let waiter = thread::spawn(move || {
            started.send(()).expect("report waiter start");
            let lock =
                MutationLock::acquire(&path, LOCK_ACQUISITION_TIMEOUT).expect("queued writer");
            acquired.send(()).expect("report acquisition");
            std::mem::drop(lock);
        });
        ready.recv().expect("waiter started");
        thread::sleep(Duration::from_millis(15));
        std::mem::drop(holder);
        received
            .recv_timeout(Duration::from_millis(25))
            .expect("a queued writer wakes on release, without a polling interval");
        waiter.join().expect("writer finishes");
    }

    #[test]
    fn timed_out_waiter_cannot_keep_a_later_lock_held() {
        let directory = tempdir().expect("scratch lock directory");
        let path = directory.path().join("mutation.lock");
        let holder =
            MutationLock::acquire(&path, LOCK_ACQUISITION_TIMEOUT).expect("initial holder");
        assert!(matches!(
            MutationLock::acquire(&path, Duration::from_millis(5)),
            Err(MutationLockError::AcquisitionTimedOut)
        ));
        std::mem::drop(holder);
        let next = MutationLock::acquire(&path, LOCK_ACQUISITION_TIMEOUT)
            .expect("abandoned waiter releases its descriptor");
        std::mem::drop(next);
    }

    fn hold_lock_until_terminated(lock_path: &Path) {
        let lock_holder = MutationLock::acquire(lock_path, LOCK_ACQUISITION_TIMEOUT)
            .expect("holder should acquire lock");
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
