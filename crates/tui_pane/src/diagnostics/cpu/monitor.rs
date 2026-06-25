use super::CpuPoller;
use super::CpuUsage;
use super::Duration;
use super::JoinHandle;
use super::Receiver;
use super::RecvTimeoutError;
use super::Sender;
use super::thread;

/// Background CPU/GPU sampler.
///
/// Spawns a worker thread that runs a [`CpuPoller`] on a fixed cadence
/// and forwards each [`CpuUsage`] over a channel. The render thread
/// calls [`latest`](Self::latest) — a non-blocking drain — so it never
/// blocks on the syscalls a poll performs. Dropping the monitor stops
/// the worker and joins it.
#[derive(Debug)]
pub struct CpuMonitor {
    samples:     Receiver<CpuUsage>,
    stop_sender: Sender<()>,
    handle:      Option<JoinHandle<()>>,
    core_count:  usize,
}

impl CpuMonitor {
    /// Spawn the worker thread, sampling at most every
    /// `poll_interval_ms` milliseconds (floored at 1ms).
    ///
    /// The poller is primed on the calling thread so [`core_count`]
    /// is available immediately; the first sample arrives one interval
    /// later, by which point the worker has a real delta window.
    ///
    /// [`core_count`]: Self::core_count
    #[must_use]
    pub fn new(poll_interval_ms: u64) -> Self {
        let mut poller = CpuPoller::new();
        let core_count = poller.core_count();
        let interval = Duration::from_millis(poll_interval_ms.max(1));
        let (sample_sender, samples) = crossbeam_channel::unbounded();
        let (stop_sender, stop_receiver) = crossbeam_channel::unbounded();
        let handle = thread::Builder::new()
            .name("cpu-monitor".to_string())
            .spawn(move || cpu_poll_loop(&mut poller, &sample_sender, &stop_receiver, interval))
            .ok();
        Self {
            samples,
            stop_sender,
            handle,
            core_count,
        }
    }

    /// Number of CPU cores, captured when the worker was spawned.
    #[must_use]
    pub const fn core_count(&self) -> usize { self.core_count }

    /// Whether the worker thread spawned successfully and is producing
    /// samples. When `false` (the `thread::Builder::spawn` failed and
    /// the sample `Sender` was dropped with the unrun closure), the
    /// [`receiver`](Self::receiver) is permanently disconnected — the
    /// event loop must not register it in a `Select`, or the loop would
    /// busy-spin on a perpetually-ready dead channel.
    #[must_use]
    pub const fn is_sampling(&self) -> bool { self.handle.is_some() }

    /// The sample channel receiver, for registering in a render-loop
    /// `crossbeam_channel::Select` so a new sample wakes the loop.
    ///
    /// Register only — draining is exclusive to [`latest`](Self::latest),
    /// which the render thread calls each frame. Registering does not
    /// consume; the `Select` merely signals readiness. Gate registration
    /// on [`is_sampling`](Self::is_sampling).
    #[must_use]
    pub const fn receiver(&self) -> &Receiver<CpuUsage> { &self.samples }

    /// Zero-filled [`CpuUsage`] sized to the current core count.
    #[must_use]
    pub fn placeholder_cpu_usage(&self) -> CpuUsage { CpuUsage::placeholder(self.core_count) }

    /// Drain the channel, returning the most recent sample if any
    /// arrived since the last call. Never blocks.
    #[must_use]
    pub fn latest(&self) -> Option<CpuUsage> {
        let mut newest = None;
        while let Ok(usage) = self.samples.try_recv() {
            newest = Some(usage);
        }
        newest
    }
}

impl Drop for CpuMonitor {
    fn drop(&mut self) {
        // Wake the worker out of its timed wait so the join is prompt;
        // a closed channel (worker already exited) is fine to ignore.
        let _ = self.stop_sender.send(());
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

/// Worker loop: wait up to `interval` for a stop signal; on timeout,
/// sample and forward. Exits when stop is signaled, when the monitor
/// is dropped (stop channel disconnects), or when the sample channel
/// is closed.
fn cpu_poll_loop(
    poller: &mut CpuPoller,
    samples: &Sender<CpuUsage>,
    stop: &Receiver<()>,
    interval: Duration,
) {
    // `recv_timeout` returns `Timeout` each interval (sample and forward);
    // `Ok`/`Disconnected` means stop was signaled or the monitor dropped.
    while stop.recv_timeout(interval) == Err(RecvTimeoutError::Timeout) {
        if samples.send(poller.poll()).is_err() {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::CpuMonitor;

    #[test]
    fn spawned_monitor_reports_sampling_with_a_connected_receiver() {
        // The event loop gates `Select` registration on `is_sampling()`.
        // A spawned worker holds the sample sender for the monitor's life,
        // so the receiver is connected (try_recv is Empty, not
        // Disconnected) and registering it will not busy-spin.
        let monitor = CpuMonitor::new(1000);
        assert!(monitor.is_sampling());
        assert!(matches!(
            monitor.receiver().try_recv(),
            Err(crossbeam_channel::TryRecvError::Empty)
        ));
    }
}
