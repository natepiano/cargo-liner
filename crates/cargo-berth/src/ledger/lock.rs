//! Descriptor-held serialization for every ledger mutation.

use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;
use std::fs::File;
use std::fs::OpenOptions;
use std::fs::TryLockError;
use std::io::Write;
use std::net::UdpSocket;
use std::path::Path;
use std::sync::mpsc;
use std::sync::mpsc::Receiver;
use std::sync::mpsc::RecvTimeoutError;
use std::thread;
use std::time::Duration;

use super::constants;
use super::constants::MUTATION_LOCK_CONTENTION_TOLERANCE_ENVIRONMENT;
use super::constants::MUTATION_LOCK_READY_PATH_ENVIRONMENT;
#[cfg(test)]
use crate::test_work;
#[cfg(test)]
use crate::test_work::Work;

/// A held advisory lock whose descriptor releases automatically on process death.
#[derive(Debug)]
pub(super) struct MutationLock {
    descriptor: File,
}

/// An immediate grant or one kernel-queued acquisition, before applying the caller's deadline.
enum MutationLockRequest {
    Granted(MutationLock),
    Queued(Receiver<std::io::Result<MutationLock>>),
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
        match Self::request(lock_path, acquisition_timeout)? {
            MutationLockRequest::Granted(lock) => Ok(lock),
            MutationLockRequest::Queued(receiver) => {
                match receiver.recv_timeout(acquisition_timeout) {
                    Ok(acquired) => acquired.map_err(MutationLockError::Io),
                    Err(RecvTimeoutError::Timeout) => Err(MutationLockError::AcquisitionTimedOut),
                    Err(RecvTimeoutError::Disconnected) => {
                        Err(MutationLockError::Io(std::io::Error::other(
                            "ledger lock waiter stopped before acquiring the lock",
                        )))
                    },
                }
            },
        }
    }

    fn request(
        lock_path: &Path,
        acquisition_timeout: Duration,
    ) -> Result<MutationLockRequest, MutationLockError> {
        let descriptor = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(lock_path)?;
        if let Some(ready_path) = std::env::var_os(MUTATION_LOCK_READY_PATH_ENVIRONMENT) {
            let mut acquisitions = OpenOptions::new()
                .create(true)
                .append(true)
                .open(Path::new(&ready_path).with_extension("acquisitions"))?;
            writeln!(acquisitions, "acquire")?;
        }
        #[cfg(test)]
        test_work::record(Work::MutationLockAttempt);
        match descriptor.try_lock() {
            Ok(()) => return Ok(MutationLockRequest::Granted(Self { descriptor })),
            Err(TryLockError::Error(error)) => return Err(error.into()),
            Err(TryLockError::WouldBlock) => {},
        }
        if acquisition_timeout.is_zero() {
            return Err(MutationLockError::AcquisitionTimedOut);
        }
        let (sender, receiver) = mpsc::sync_channel(1);
        #[cfg(test)]
        test_work::record(Work::MutationLockWait);
        // Queue in the kernel instead of competing with later arrivals after a backoff.
        // On timeout the receiver closes; the worker then drops the acquired descriptor,
        // so an abandoned request can never leave a lock held in the background.
        thread::Builder::new()
            .name("ledger-lock-wait".to_owned())
            .spawn(move || {
                let acquired = descriptor.lock().map(|()| Self { descriptor });
                std::mem::drop(sender.send(acquired));
            })?;
        if let Some(ready_path) = std::env::var_os(MUTATION_LOCK_READY_PATH_ENVIRONMENT) {
            std::fs::write(ready_path, b"ready")?;
        }
        if let Some(socket_address) =
            std::env::var_os("CARGO_BERTH_TEST_MUTATION_LOCK_READY_SOCKET")
        {
            UdpSocket::bind("127.0.0.1:0")?
                .send_to(b"queued", socket_address.to_string_lossy().as_ref())?;
        }
        Ok(MutationLockRequest::Queued(receiver))
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
    clippy::panic,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::env;
    use std::fs;
    use std::path::Path;
    use std::thread;
    use std::time::Duration;

    use cargo_berth_test_support::ChildDeadlineExt as _;
    use cargo_berth_test_support::FixtureCommand as Command;
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
        let counter = crate::test_work::WorkCounter::start();
        let super::MutationLockRequest::Queued(receiver) =
            MutationLock::request(&path, LOCK_ACQUISITION_TIMEOUT).expect("queued request")
        else {
            panic!("a second descriptor must queue while the holder owns the lock");
        };
        std::mem::drop(holder);
        let lock = receiver
            .recv()
            .expect("waiter reports")
            .expect("queued writer acquires");
        let work = counter.finish();
        assert_eq!(work.mutation_lock_attempts, 1);
        assert_eq!(work.mutation_lock_waits, 1);
        std::mem::drop(lock);
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
