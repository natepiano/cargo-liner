//! Running the census: one scheduler thread scans this machine and
//! sends each remote machine its probe, each probe on a thread of its
//! own, and every answer goes to the event loop over one channel.
//!
//! A remote that is slow to answer holds only its own thread. The
//! scheduler skips a host whose last probe is still running, so a host
//! that stops answering never has more than one probe out.

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::PoisonError;
use std::sync::mpsc;
use std::sync::mpsc::Receiver;
use std::sync::mpsc::Sender;
use std::thread;
use std::time::Duration;
use std::time::Instant;

use super::AgentRow;
use super::CensusUpdate;
use super::RemoteMachines;
use super::remote;
use super::remote::RemoteRunner;
use super::remote::SshRunner;
use super::scan::LocalScanner;
use crate::constants::LOCAL_SCAN_INTERVAL;
use crate::constants::REMOTE_PROBE_INTERVAL;

/// How often the scheduler looks at each machine.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Cadence {
    /// Between scans of this machine.
    pub(crate) local:  Duration,
    /// Between probes of each remote machine.
    pub(crate) remote: Duration,
}

impl Cadence {
    /// The cadence the app runs at.
    pub(crate) const LIVE: Self = Self {
        local:  LOCAL_SCAN_INTERVAL,
        remote: REMOTE_PROBE_INTERVAL,
    };
}

/// Hosts with a probe out, shared between the scheduler and the probe
/// threads.
type InFlight = Arc<Mutex<HashSet<String>>>;

/// Start the census of this machine and of the hosts `remotes` names,
/// probing over ssh, and hand back the channel it answers on.
pub(crate) fn spawn(remotes: RemoteMachines) -> Receiver<CensusUpdate> {
    let scanner = LocalScanner::new();
    spawn_with(
        remotes,
        Arc::new(SshRunner::new()),
        move || scanner.scan(),
        Cadence::LIVE,
    )
}

/// Start the scheduler with `runner` probing the remotes and
/// `scan_local` scanning this machine, at `cadence`.
///
/// `remotes` is read afresh at every round of probes, so a host added
/// in the settings overlay is probed from the next round on. The
/// scheduler ends at the first answer it cannot send, which is once the
/// receiver is gone.
pub(crate) fn spawn_with(
    remotes: RemoteMachines,
    runner: Arc<dyn RemoteRunner>,
    mut scan_local: impl FnMut() -> Vec<AgentRow> + Send + 'static,
    cadence: Cadence,
) -> Receiver<CensusUpdate> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let in_flight = InFlight::default();
        let mut next_local = Instant::now();
        let mut next_remote = next_local;
        loop {
            let now = Instant::now();
            if now >= next_local {
                if sender.send(CensusUpdate::Local(scan_local())).is_err() {
                    return;
                }
                next_local = now + cadence.local;
            }
            if now >= next_remote {
                for host in remotes.snapshot() {
                    dispatch(host, &runner, &in_flight, &sender);
                }
                next_remote = now + cadence.remote;
            }
            thread::sleep(
                next_local
                    .min(next_remote)
                    .saturating_duration_since(Instant::now()),
            );
        }
    });
    receiver
}

/// Probe `host` on a thread of its own, unless its last probe is still
/// out.
fn dispatch(
    host: String,
    runner: &Arc<dyn RemoteRunner>,
    in_flight: &InFlight,
    sender: &Sender<CensusUpdate>,
) {
    if !in_flight
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(host.clone())
    {
        return;
    }
    let runner = Arc::clone(runner);
    let in_flight = Arc::clone(in_flight);
    let sender = sender.clone();
    thread::spawn(move || {
        let state = remote::machine_state(runner.probe(&host));
        in_flight
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&host);
        // No receiver means the app has quit, and the answer has nowhere
        // to go.
        let _ = sender.send(CensusUpdate::Remote { host, state });
    });
}

#[cfg(test)]
#[expect(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::census::MachineState;
    use crate::census::remote::ProbeOutcome;

    /// The host whose probe waits until the test lets it finish.
    const BLOCKED: &str = "blocked";
    /// The host whose probe answers at once.
    const QUICK: &str = "quick";
    /// How long the test waits for any one answer before failing.
    const PATIENCE: Duration = Duration::from_secs(5);

    /// A runner whose [`BLOCKED`] probe waits on `release` and whose
    /// every other probe fails at once as unreachable, counting how
    /// often each host is probed.
    struct Runner {
        /// Lets the blocked probe finish.
        release: Mutex<Receiver<()>>,
        /// Probes started, by host.
        started: Mutex<HashMap<String, usize>>,
    }

    impl RemoteRunner for Runner {
        fn probe(&self, host: &str) -> ProbeOutcome {
            *self
                .started
                .lock()
                .expect("the count should lock")
                .entry(host.to_string())
                .or_default() += 1;
            if host == BLOCKED {
                let _ = self.release.lock().expect("the release should lock").recv();
            }
            ProbeOutcome::Finished {
                status: Some(255),
                stdout: Vec::new(),
            }
        }
    }

    /// While one host's probe hangs, this machine keeps being scanned
    /// and the other host keeps being probed; the hung host is not
    /// probed again until its probe returns.
    #[test]
    fn a_hung_host_delays_neither_this_machine_nor_another_host() {
        let (release, released) = mpsc::channel();
        let runner = Arc::new(Runner {
            release: Mutex::new(released),
            started: Mutex::new(HashMap::new()),
        });
        let cadence = Cadence {
            local:  Duration::from_millis(5),
            remote: Duration::from_millis(5),
        };
        let updates = spawn_with(
            RemoteMachines::new(vec![BLOCKED.to_string(), QUICK.to_string()]),
            Arc::clone(&runner) as Arc<dyn RemoteRunner>,
            Vec::new,
            cadence,
        );

        let mut local = 0;
        let mut quick = 0;
        while local < 3 || quick < 3 {
            match updates
                .recv_timeout(PATIENCE)
                .expect("the census should keep answering")
            {
                CensusUpdate::Local(rows) => {
                    assert!(rows.is_empty());
                    local += 1;
                },
                CensusUpdate::Remote { host, state } => {
                    assert_eq!(
                        host, QUICK,
                        "the blocked host answered before it was let go"
                    );
                    assert_eq!(state, MachineState::Failed("unreachable".to_string()));
                    quick += 1;
                },
            }
        }
        let blocked_probes = |runner: &Runner| {
            runner
                .started
                .lock()
                .expect("the count should lock")
                .get(BLOCKED)
                .copied()
        };
        assert_eq!(blocked_probes(&runner), Some(1));

        release
            .send(())
            .expect("the blocked probe should be waiting");
        let answered = std::iter::from_fn(|| updates.recv_timeout(PATIENCE).ok())
            .any(|update| matches!(update, CensusUpdate::Remote { host, .. } if host == BLOCKED));
        assert!(answered, "the blocked host should answer once let go");
    }
}
