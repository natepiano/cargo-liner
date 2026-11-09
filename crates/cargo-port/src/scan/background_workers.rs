//! Reuse workers for jobs that allocate whole project trees and metadata.

use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::OnceLock;
use std::sync::mpsc;
use std::thread;
use std::thread::JoinHandle;

use rayon::ThreadPool;
use rayon::ThreadPoolBuilder;

use super::pending_work::NextWork;
use super::pending_work::PendingWork;
use super::pending_work::PendingWorker;
use super::pending_work::WorkSubmission;
use crate::constants::SCAN_DISCOVERY_CONCURRENCY;
use crate::constants::SCAN_DISK_CONCURRENCY;
use crate::constants::SCAN_METADATA_CONCURRENCY;
use crate::constants::SCAN_WATCHER_CONCURRENCY;
use crate::project::AbsolutePath;

#[derive(Clone, Copy)]
enum BackgroundWork {
    Discovery,
    Watching,
    Metadata,
    FileMeasurement,
}

enum BackgroundWorkers {
    Ready(ThreadPool),
    Unavailable,
}

impl BackgroundWorkers {
    fn new(work: BackgroundWork) -> Self {
        let (name, threads) = match work {
            BackgroundWork::Discovery => ("port-discovery", SCAN_DISCOVERY_CONCURRENCY),
            BackgroundWork::Watching => ("port-watcher", SCAN_WATCHER_CONCURRENCY),
            BackgroundWork::Metadata => ("port-metadata", SCAN_METADATA_CONCURRENCY),
            BackgroundWork::FileMeasurement => ("port-files", SCAN_DISK_CONCURRENCY),
        };
        match ThreadPoolBuilder::new()
            .num_threads(threads)
            .thread_name(move |index| format!("{name}-{index}"))
            .build()
        {
            Ok(pool) => Self::Ready(pool),
            Err(error) => {
                tracing::warn!(%error, name, "background_workers_unavailable");
                Self::Unavailable
            },
        }
    }

    fn for_work(work: BackgroundWork) -> &'static Self {
        static DISCOVERY: OnceLock<BackgroundWorkers> = OnceLock::new();
        static WATCHING: OnceLock<BackgroundWorkers> = OnceLock::new();
        static METADATA: OnceLock<BackgroundWorkers> = OnceLock::new();
        static FILES: OnceLock<BackgroundWorkers> = OnceLock::new();
        let slot = match work {
            BackgroundWork::Discovery => &DISCOVERY,
            BackgroundWork::Watching => &WATCHING,
            BackgroundWork::Metadata => &METADATA,
            BackgroundWork::FileMeasurement => &FILES,
        };
        slot.get_or_init(|| Self::new(work))
    }

    fn spawn(&self, operation: impl FnOnce() + Send + 'static) {
        match self {
            Self::Ready(pool) => pool.spawn(operation),
            Self::Unavailable => {
                thread::spawn(operation);
            },
        }
    }
}

pub(super) fn spawn_discovery(operation: impl FnOnce() + Send + 'static) {
    BackgroundWorkers::for_work(BackgroundWork::Discovery).spawn(operation);
}

pub(super) fn spawn_discovery_for_roots(
    roots: Vec<AbsolutePath>,
    operation: impl FnOnce() + Send + 'static,
) {
    type DiscoveryJob = Box<dyn FnOnce() + Send>;
    type PendingDiscovery = Arc<Mutex<PendingWork<Vec<AbsolutePath>, DiscoveryJob>>>;
    static PENDING: OnceLock<PendingDiscovery> = OnceLock::new();
    let pending = PENDING.get_or_init(Arc::default);
    let submission = pending
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .submit(roots.clone(), Box::new(operation));
    if matches!(submission, WorkSubmission::JoinedWorker) {
        return;
    }
    let mut worker = PendingWorker::new(Arc::clone(pending), roots);
    spawn_discovery(move || {
        while let NextWork::Run(operation) = worker.next() {
            operation();
        }
    });
}

pub(crate) fn spawn_watcher(operation: impl FnOnce() + Send + 'static) -> JoinHandle<()> {
    let (sender, receiver) = mpsc::sync_channel(1);
    BackgroundWorkers::for_work(BackgroundWork::Watching).spawn(move || {
        let result = std::panic::catch_unwind(AssertUnwindSafe(operation));
        let _ = sender.send(result);
    });
    // Preserve the existing join contract. This thread only waits; the retained
    // watcher worker owns the large registration maps and directory buffers.
    thread::spawn(move || {
        if let Ok(Err(payload)) = receiver.recv() {
            std::panic::resume_unwind(payload);
        }
    })
}

pub(super) fn run_metadata<R: Send>(operation: impl FnOnce() -> R + Send) -> R {
    match BackgroundWorkers::for_work(BackgroundWork::Metadata) {
        BackgroundWorkers::Ready(pool) => pool.install(operation),
        BackgroundWorkers::Unavailable => operation(),
    }
}

pub(super) fn measure_files<R: Send>(operation: impl FnOnce() -> R + Send) -> R {
    match BackgroundWorkers::for_work(BackgroundWork::FileMeasurement) {
        BackgroundWorkers::Ready(pool) => pool.install(operation),
        BackgroundWorkers::Unavailable => operation(),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::sync::mpsc::RecvError;

    use super::*;

    #[test]
    fn rescan_bursts_replace_pending_discovery_for_the_same_roots() -> anyhow::Result<()> {
        let roots = vec![crate::project::AbsolutePath::from("/tmp/pending-discovery")];
        let (started, start) = mpsc::channel();
        let (release, resume) = mpsc::channel();
        spawn_discovery_for_roots(roots.clone(), move || {
            let _ = started.send(());
            let _ = resume.recv();
        });
        start.recv()?;
        let (sender, receiver) = mpsc::channel();
        for request in 0..128 {
            let sender = sender.clone();
            spawn_discovery_for_roots(roots.clone(), move || {
                let _ = sender.send(request);
            });
        }
        drop(sender);
        release.send(())?;
        let completed: Vec<_> = receiver.iter().collect();
        assert_eq!(
            completed,
            vec![127],
            "only the latest pending rescan must run"
        );
        Ok(())
    }

    #[test]
    fn repeated_metadata_jobs_reuse_the_same_worker_set() {
        let threads: HashSet<_> = (0..1_024)
            .map(|_| run_metadata(|| thread::current().id()))
            .collect();
        assert!(threads.len() <= SCAN_METADATA_CONCURRENCY);
        assert!(!threads.contains(&thread::current().id()));
    }

    #[test]
    fn discovery_jobs_reuse_one_worker() -> Result<(), RecvError> {
        let (sender, receiver) = mpsc::channel();
        for _ in 0..128 {
            let sender = sender.clone();
            spawn_discovery(move || {
                let _ = sender.send(thread::current().id());
            });
        }
        let mut threads = HashSet::new();
        for _ in 0..128 {
            threads.insert(receiver.recv()?);
        }
        assert_eq!(threads.len(), 1);
        Ok(())
    }
}
