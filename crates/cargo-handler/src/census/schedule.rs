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
use std::sync::mpsc::SendError;
use std::sync::mpsc::Sender;
use std::thread;
use std::time::Duration;
use std::time::Instant;

use super::AgentRow;
use super::CensusUpdate;
use super::HierarchyCommand;
use super::RemoteMachines;
use super::hierarchy::CommandRunner;
use super::hierarchy::HierarchyCache;
use super::hierarchy::HierarchyRunner;
use super::remote;
use super::remote::RemoteRunner;
use super::remote::SshRunner;
use super::scan::LocalScanner;
use super::scan::ScanStage;
use crate::constants::LOCAL_SCAN_INTERVAL;
use crate::constants::REMOTE_PROBE_INTERVAL;
use crate::startup_log;

/// How often the scheduler looks at each machine.
#[derive(Clone, Copy, Debug)]
struct Cadence {
    /// Between scans of this machine.
    local:  Duration,
    /// Between probes of each remote machine.
    remote: Duration,
}

impl Cadence {
    /// The cadence the app runs at.
    const LIVE: Self = Self {
        local:  LOCAL_SCAN_INTERVAL,
        remote: REMOTE_PROBE_INTERVAL,
    };
}

/// Hosts with a probe out, shared between the scheduler and the probe
/// threads.
type InFlight = Arc<Mutex<HashSet<String>>>;

/// Whether a completed local scan permits another scheduler round.
enum ScanSchedule {
    NextRound,
    Finished,
}

/// Publishes staged local rows with the current cached hierarchy on the scheduler thread.
pub(crate) struct LocalAnswers {
    /// The event loop's census channel.
    sender:    Sender<CensusUpdate>,
    /// The live setting, read for each publication.
    hierarchy: HierarchyCommand,
    /// The command runner, replaceable in scheduler tests.
    runner:    Arc<dyn HierarchyRunner>,
    /// The most recent hierarchy attempt for this session set.
    cache:     HierarchyCache,
}

impl LocalAnswers {
    /// Publish classification immediately; only completed scans discover hierarchy edges.
    pub(super) fn publish(
        &mut self,
        stage: ScanStage,
        rows: Vec<AgentRow>,
    ) -> Result<(), SendError<CensusUpdate>> {
        match stage {
            ScanStage::Classified => self
                .sender
                .send(CensusUpdate::Local(rows, super::HierarchyEdges::default())),
            ScanStage::Completed => self.send(rows),
        }
    }

    /// Attach this machine's management links to a completed scan before delivering it.
    pub(crate) fn send(&mut self, rows: Vec<AgentRow>) -> Result<(), SendError<CensusUpdate>> {
        let edges = self.cache.edges(
            self.hierarchy.snapshot(),
            &rows,
            self.runner.as_ref(),
            Instant::now(),
        );
        self.sender.send(CensusUpdate::Local(rows, edges))
    }
}

/// Start the census of this machine and of the hosts `remotes` names,
/// probing over ssh, and hand back the channel it answers on.
pub(crate) fn spawn(
    remotes: RemoteMachines,
    hierarchy: HierarchyCommand,
) -> Receiver<CensusUpdate> {
    let mut scanner = LocalScanner::new();
    spawn_staged_with(
        remotes,
        hierarchy,
        Arc::new(CommandRunner),
        Arc::new(SshRunner::new()),
        move |sender| {
            let mut schedule = ScanSchedule::NextRound;
            scanner.scan_staged(|stage, rows| {
                startup_log::startup_local_answer(&rows);
                if sender.publish(stage, rows).is_err() {
                    schedule = ScanSchedule::Finished;
                }
            });
            schedule
        },
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
fn spawn_staged_with(
    remotes: RemoteMachines,
    hierarchy: HierarchyCommand,
    hierarchy_runner: Arc<dyn HierarchyRunner>,
    runner: Arc<dyn RemoteRunner>,
    mut scan_local: impl FnMut(&mut LocalAnswers) -> ScanSchedule + Send + 'static,
    cadence: Cadence,
) -> Receiver<CensusUpdate> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let mut local_answers = LocalAnswers {
            sender: sender.clone(),
            hierarchy,
            runner: hierarchy_runner,
            cache: HierarchyCache::default(),
        };
        let in_flight = InFlight::default();
        let mut next_local = Instant::now();
        let mut next_remote = next_local;
        loop {
            let now = Instant::now();
            if now >= next_remote {
                for host in remotes.snapshot() {
                    dispatch(host, &runner, &in_flight, &sender);
                }
                next_remote = now + cadence.remote;
            }
            if now >= next_local {
                if matches!(scan_local(&mut local_answers), ScanSchedule::Finished) {
                    return;
                }
                next_local = now + cadence.local;
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

/// Adapt the completed-answer source used by the existing cadence tests.
#[cfg(test)]
fn spawn_with(
    remotes: RemoteMachines,
    runner: Arc<dyn RemoteRunner>,
    mut scan_local: impl FnMut() -> Vec<AgentRow> + Send + 'static,
    cadence: Cadence,
) -> Receiver<CensusUpdate> {
    spawn_staged_with(
        remotes,
        HierarchyCommand::new(super::HierarchySource::Off),
        Arc::new(CommandRunner),
        runner,
        move |sender| {
            if sender.send(scan_local()).is_ok() {
                ScanSchedule::NextRound
            } else {
                ScanSchedule::Finished
            }
        },
        cadence,
    )
}

/// Run one real scheduler scan with a source the startup test can hold.
#[cfg(test)]
pub(crate) fn spawn_with_test_source(
    hierarchy: HierarchyCommand,
    hierarchy_runner: Arc<dyn HierarchyRunner>,
    mut scan_local: impl FnMut(&mut LocalAnswers) + Send + 'static,
) -> Receiver<CensusUpdate> {
    spawn_staged_with(
        RemoteMachines::new(Vec::new()),
        hierarchy,
        hierarchy_runner,
        Arc::new(SshRunner::new()),
        move |sender| {
            scan_local(sender);
            ScanSchedule::Finished
        },
        Cadence::LIVE,
    )
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
        startup_log::startup_event(&format!("remote_answer {host}"));
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
    use std::sync::atomic::AtomicUsize;
    use std::sync::atomic::Ordering;

    use tempfile::TempDir;

    use super::*;
    use crate::census::Agent;
    use crate::census::HierarchyEdges;
    use crate::census::MachineState;
    use crate::census::ServiceTier;
    use crate::census::SessionIdentity;
    use crate::census::hierarchy::HierarchyError;
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

    /// A local Claude fixture whose process identity may change independently of its session.
    fn session_row(id: &str, pid: u32) -> AgentRow {
        AgentRow {
            agent: Agent::Claude,
            session_id: SessionIdentity::Claude(id.to_string()),
            service_tier: ServiceTier::Unrecorded,
            name: "worker".to_string(),
            status: None,
            started: 1,
            process_started: 1,
            pid,
            desktop: None,
            directory: "~".to_string(),
            branch: None,
            launched_by: None,
            children: Vec::new(),
        }
    }

    /// Write a command script, keeping a space in its path to exercise shell quoting.
    fn command_script(script: &str) -> (TempDir, super::super::HierarchySource) {
        let temp = tempfile::tempdir().expect("a fixture directory should open");
        let path = temp.path().join("hierarchy fixture.sh");
        std::fs::write(&path, script).expect("the fixture should be written");
        let source = super::super::HierarchySource::Command(format!("sh '{}'", path.display()));
        (temp, source)
    }

    /// A real fixture command reaches a census machine answer through the scheduler.
    #[test]
    fn hierarchy_command_edges_reach_the_local_census() {
        let (_temp, source) = command_script(
            r#"printf '%s\n' '{"schema":1,"edges":[{"manager":"remote-boss","member":"worker","role":"impl"}]}'"#,
        );
        let updates = spawn_with_test_source(
            HierarchyCommand::new(source),
            Arc::new(CommandRunner),
            |sender| {
                sender
                    .send(vec![session_row("worker", 1)])
                    .expect("the local answer should send");
            },
        );
        let mut census = super::super::Census::new("local".to_string());
        census.apply(
            updates
                .recv_timeout(PATIENCE)
                .expect("the census should answer"),
        );
        let machines = census.machines(&[]);
        assert_eq!(
            machines[0].state,
            &MachineState::Answered(
                vec![session_row("worker", 1)],
                super::super::HierarchyEdges(vec![super::super::HierarchyEdge {
                    manager: "remote-boss".to_string(),
                    member:  "worker".to_string(),
                    role:    super::super::HierarchyRole::Impl,
                }]),
            )
        );
    }

    /// Use the production runner with a shorter permitted runtime for timeout fixtures.
    struct ShortDeadlineRunner;

    impl HierarchyRunner for ShortDeadlineRunner {
        fn run(&self, command: &str, timeout: Duration) -> Result<HierarchyEdges, HierarchyError> {
            CommandRunner.run(command, timeout.min(Duration::from_millis(120)))
        }
    }

    /// Failed, malformed, and timed-out scripts publish no links and reap the timed-out child.
    #[test]
    fn unsuccessful_commands_publish_no_edges_and_timeout_reaps_the_child() {
        for script in [
            "exit 1",
            "printf 'malformed'",
            "printf '{\"schema\":2,\"edges\":[]}'",
            "exec yes",
        ] {
            let started = Instant::now();
            let (_temp, source) = command_script(script);
            let updates = spawn_with_test_source(
                HierarchyCommand::new(source),
                Arc::new(CommandRunner),
                |sender| {
                    sender
                        .send(Vec::new())
                        .expect("the local answer should send");
                },
            );
            assert_eq!(
                updates
                    .recv_timeout(PATIENCE)
                    .expect("a failed hierarchy still answers"),
                CensusUpdate::Local(Vec::new(), super::super::HierarchyEdges::default())
            );
            assert!(started.elapsed() < Duration::from_millis(800));
        }
        for close_stdout in ["", "exec > /dev/null;"] {
            let temp = tempfile::tempdir().expect("a fixture directory should open");
            let pid_file = temp.path().join("pid");
            let source = super::super::HierarchySource::Command(format!(
                "{close_stdout} echo $$ > '{}'; exec sleep 10",
                pid_file.display()
            ));
            let started = Instant::now();
            let updates = spawn_with_test_source(
                HierarchyCommand::new(source),
                Arc::new(ShortDeadlineRunner),
                |sender| {
                    sender
                        .send(Vec::new())
                        .expect("the local answer should send");
                },
            );
            assert_eq!(
                updates
                    .recv_timeout(PATIENCE)
                    .expect("a timed-out hierarchy still answers"),
                CensusUpdate::Local(Vec::new(), super::super::HierarchyEdges::default())
            );
            assert!(started.elapsed() < Duration::from_millis(800));
            let pid =
                std::fs::read_to_string(pid_file).expect("the script should have recorded its pid");
            let status = std::process::Command::new("kill")
                .args(["-0", pid.trim()])
                .stderr(std::process::Stdio::null())
                .status()
                .expect("kill should start");
            assert!(
                !status.success(),
                "the timed-out child must be killed and reaped"
            );
        }
    }

    /// Counts actual refreshes independently of the rows supplied to the scheduler.
    struct CountingHierarchy(AtomicUsize);

    impl HierarchyRunner for CountingHierarchy {
        fn run(&self, _: &str, _: Duration) -> Result<HierarchyEdges, HierarchyError> {
            let round = self.0.fetch_add(1, Ordering::SeqCst) + 1;
            Ok(HierarchyEdges(vec![super::super::HierarchyEdge {
                manager: format!("manager-{round}"),
                member:  "worker".to_string(),
                role:    super::super::HierarchyRole::Test,
            }]))
        }
    }

    /// Staged answers reuse links across process changes and refresh on session or setting changes.
    #[test]
    fn scheduler_refreshes_on_session_sets_and_live_setting_changes() {
        let hierarchy =
            HierarchyCommand::new(super::super::HierarchySource::Command("first".to_string()));
        let edits = hierarchy.clone();
        let runner = Arc::new(CountingHierarchy(std::sync::atomic::AtomicUsize::new(0)));
        let updates = spawn_with_test_source(
            hierarchy,
            Arc::clone(&runner) as Arc<dyn HierarchyRunner>,
            move |sender| {
                sender
                    .send(vec![session_row("a", 1)])
                    .expect("first answer should send");
                sender
                    .send(vec![session_row("a", 2)])
                    .expect("same session should send");
                sender
                    .send(vec![session_row("b", 2)])
                    .expect("new session should send");
                edits.replace(super::super::HierarchySource::Command("second".to_string()));
                sender
                    .send(vec![session_row("b", 2)])
                    .expect("new setting should send");
                edits.replace(super::super::HierarchySource::Off);
                sender
                    .send(vec![session_row("b", 2)])
                    .expect("disabled hierarchy should send");
            },
        );
        let answers: Vec<_> = updates.into_iter().collect();
        let managers: Vec<_> = answers
            .iter()
            .filter_map(|update| match update {
                CensusUpdate::Local(_, edges) => Some(
                    edges
                        .0
                        .iter()
                        .map(|edge| edge.manager.as_str())
                        .collect::<Vec<_>>(),
                ),
                CensusUpdate::Remote { .. } => None,
            })
            .collect();
        assert_eq!(
            managers,
            [
                vec!["manager-1"],
                vec!["manager-1"],
                vec!["manager-2"],
                vec!["manager-3"],
                vec![]
            ]
        );
        assert_eq!(runner.0.load(std::sync::atomic::Ordering::SeqCst), 3);
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
                CensusUpdate::Local(rows, _) => {
                    assert_eq!(rows, [] as [AgentRow; 0]);
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
