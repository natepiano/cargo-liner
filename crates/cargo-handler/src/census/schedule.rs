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
use super::HierarchySnapshot;
use super::HierarchySource;
use super::RemoteHierarchyAnswers;
use super::RemoteMachines;
use super::account::AccountLabels;
use super::hierarchy::CommandRunner;
use super::hierarchy::HierarchyCache;
use super::hierarchy::HierarchyRunner;
use super::probe;
use super::remote;
use super::remote::ProbeOutcome;
use super::remote::RemoteRunner;
use super::remote::SshRunner;
use super::scan::LocalScanner;
use super::scan::ScanStage;
#[cfg(test)]
use crate::constants::HIERARCHY_REFRESH_INTERVAL;
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

/// Discovery completed during classification belongs to this scan until it expires.
#[derive(Default)]
enum ScanDiscovery {
    #[default]
    Unstarted,
    Classified {
        source:  HierarchySource,
        expires: Instant,
    },
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
    /// Whether this scan already obtained a usable hierarchy outcome.
    discovery: ScanDiscovery,
}

impl LocalAnswers {
    /// Publish classification immediately, then discover roles before slow detail reads.
    pub(super) fn publish(
        &mut self,
        stage: ScanStage,
        rows: Vec<AgentRow>,
    ) -> Result<(), SendError<CensusUpdate>> {
        let source = self.hierarchy.snapshot();
        match stage {
            ScanStage::Classified => {
                let hierarchy = self.cache.retained(source.clone());
                self.deliver(rows.clone(), hierarchy, stage)?;
                let started = Instant::now();
                if matches!(source, HierarchySource::Command(_)) {
                    self.discover(rows, stage)?;
                }
                let expires = self.cache.reuse_deadline(started);
                self.discovery = ScanDiscovery::Classified { source, expires };
                Ok(())
            },
            ScanStage::Completed => {
                let hierarchy = self.cache.retained(source.clone());
                self.deliver(rows.clone(), hierarchy.clone(), ScanStage::Classified)?;
                match std::mem::take(&mut self.discovery) {
                    ScanDiscovery::Classified {
                        source: classified,
                        expires,
                    } if classified == source && Instant::now() < expires => {
                        self.deliver(rows, hierarchy, stage)
                    },
                    _ => self.discover(rows, stage),
                }
            },
        }
    }

    /// Completed-only fixture sources represent one whole scan per call.
    #[cfg(test)]
    pub(crate) fn send(&mut self, rows: Vec<AgentRow>) -> Result<(), SendError<CensusUpdate>> {
        self.discover(rows, ScanStage::Completed)
    }

    /// Discover links at either stage without delaying the first visible rows.
    fn discover(
        &mut self,
        rows: Vec<AgentRow>,
        stage: ScanStage,
    ) -> Result<(), SendError<CensusUpdate>> {
        let hierarchy = self.cache.snapshot(
            self.hierarchy.snapshot(),
            &rows,
            self.runner.as_ref(),
            Instant::now(),
        );
        self.deliver(rows, hierarchy, stage)
    }

    fn deliver(
        &self,
        rows: Vec<AgentRow>,
        hierarchy: HierarchySnapshot,
        stage: ScanStage,
    ) -> Result<(), SendError<CensusUpdate>> {
        let update = CensusUpdate::Local {
            rows,
            hierarchy,
            stage,
        };
        self.sender.send(update)
    }
}

/// Start the census of this machine and of the hosts `remotes` names,
/// probing over ssh, and hand back the channel it answers on.
pub(crate) fn spawn(
    remotes: RemoteMachines,
    hierarchy: HierarchyCommand,
    account_labels: AccountLabels,
) -> Receiver<CensusUpdate> {
    let mut scanner = LocalScanner::new(account_labels);
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
            discovery: ScanDiscovery::default(),
        };
        let in_flight = InFlight::default();
        let remote_answers = Arc::new(Mutex::new(RemoteHierarchyAnswers::default()));
        let mut next_local = Instant::now();
        let mut next_remote = next_local;
        loop {
            let now = Instant::now();
            if now >= next_remote {
                let configured = remotes.snapshot();
                remote_answers
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .retain(&configured);
                for host in configured {
                    dispatch(
                        host,
                        &remotes,
                        &runner,
                        &in_flight,
                        &remote_answers,
                        &sender,
                    );
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
        HierarchyCommand::new(HierarchySource::Off),
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
    remotes: &RemoteMachines,
    runner: &Arc<dyn RemoteRunner>,
    in_flight: &InFlight,
    answers: &Arc<Mutex<RemoteHierarchyAnswers>>,
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
    let answers = Arc::clone(answers);
    let remotes = remotes.clone();
    thread::spawn(move || {
        let outcome = runner.probe(&host);
        let mut answers = answers.lock().unwrap_or_else(PoisonError::into_inner);
        let configured = remotes.snapshot();
        answers.retain(&configured);
        if !configured.contains(&host) {
            in_flight
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .remove(&host);
            return;
        }
        let state = match outcome {
            ProbeOutcome::Finished {
                status: Some(0),
                stdout,
            } => answers.apply(&host, probe::parse_answer(&stdout)),
            outcome => remote::machine_state(outcome),
        };
        drop(answers);
        in_flight
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&host);
        // If `sender.send` finds no receiver, the app has quit and the answer
        // has nowhere to go.
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
    use std::collections::VecDeque;
    use std::sync::atomic::AtomicUsize;
    use std::sync::atomic::Ordering;

    use tempfile::TempDir;

    use super::*;
    use crate::census::Account;
    use crate::census::Agent;
    use crate::census::AgentActivity;
    use crate::census::HierarchyEdges;
    use crate::census::HierarchyNotice;
    use crate::census::HierarchySnapshot;
    use crate::census::MachineState;
    use crate::census::ServiceTier;
    use crate::census::SessionIdentity;
    use crate::census::hierarchy::HierarchyError;

    /// The host whose probe waits until the test lets it finish.
    const BLOCKED: &str = "blocked";
    /// The host whose probe answers at once.
    const QUICK: &str = "quick";
    /// How long the test waits for any one answer before failing.
    const PATIENCE: Duration = Duration::from_secs(5);

    /// A completed answer with a visible discovery diagnostic and no retained links.
    fn noted(rows: Vec<AgentRow>, notice: HierarchyNotice) -> CensusUpdate {
        CensusUpdate::Local {
            rows,
            hierarchy: HierarchySnapshot {
                edges: HierarchyEdges::default(),
                notice,
            },
            stage: ScanStage::Completed,
        }
    }

    /// Startup classification and detailed rows precede one completion per scan.
    #[test]
    fn each_local_scan_publishes_one_completion_after_its_interim_rows() {
        let updates = spawn_with_test_source(
            HierarchyCommand::new(HierarchySource::Off),
            Arc::new(CommandRunner),
            |answers| {
                answers
                    .publish(ScanStage::Classified, vec![session_row("worker", 1)])
                    .expect("publish startup classification");
                for started in [2, 3] {
                    answers
                        .publish(ScanStage::Completed, vec![session_row("worker", started)])
                        .expect("publish a completed scan");
                }
            },
        );
        let answers: Vec<_> = updates.into_iter().collect();
        assert_eq!(
            answers,
            [
                CensusUpdate::Local {
                    rows:      vec![session_row("worker", 1)],
                    hierarchy: HierarchyEdges::default().into(),
                    stage:     ScanStage::Classified,
                },
                CensusUpdate::Local {
                    rows:      vec![session_row("worker", 2)],
                    hierarchy: HierarchyEdges::default().into(),
                    stage:     ScanStage::Classified,
                },
                CensusUpdate::Local {
                    rows:      vec![session_row("worker", 2)],
                    hierarchy: HierarchyEdges::default().into(),
                    stage:     ScanStage::Completed,
                },
                CensusUpdate::Local {
                    rows:      vec![session_row("worker", 3)],
                    hierarchy: HierarchyEdges::default().into(),
                    stage:     ScanStage::Classified,
                },
                CensusUpdate::Local {
                    rows:      vec![session_row("worker", 3)],
                    hierarchy: HierarchyEdges::default().into(),
                    stage:     ScanStage::Completed,
                },
            ]
        );
    }

    /// A failed Classified attempt is reused by Completed, then retried by the next scan.
    #[test]
    fn failed_hierarchy_is_attempted_once_per_scan_and_recovers_on_the_next_scan() {
        struct Recovering(AtomicUsize);
        impl HierarchyRunner for Recovering {
            fn run(&self, _: &str, _: Duration) -> Result<HierarchyEdges, HierarchyError> {
                if self.0.fetch_add(1, Ordering::SeqCst) == 0 {
                    Err(HierarchyError::TimedOut)
                } else {
                    Ok(HierarchyEdges(vec![super::super::HierarchyEdge {
                        manager: "boss".into(),
                        member:  "worker".into(),
                        role:    super::super::HierarchyRole::Unit,
                    }]))
                }
            }
        }
        let runner = Arc::new(Recovering(AtomicUsize::new(0)));
        let observed = Arc::clone(&runner);
        let updates = spawn_with_test_source(
            HierarchyCommand::new(HierarchySource::Command("fixture".into())),
            runner,
            |answers| {
                for _ in 0..2 {
                    answers
                        .publish(ScanStage::Classified, vec![session_row("worker", 1)])
                        .expect("classification publishes");
                    answers
                        .publish(ScanStage::Completed, vec![session_row("worker", 1)])
                        .expect("details publish");
                }
            },
        );
        let completions: Vec<_> = updates
            .into_iter()
            .filter_map(|update| match update {
                CensusUpdate::Local {
                    hierarchy,
                    stage: ScanStage::Completed,
                    ..
                } => Some(hierarchy.notice),
                _ => None,
            })
            .collect();
        assert_eq!(
            completions,
            [
                HierarchyNotice::Failed(HierarchyError::TimedOut),
                HierarchyNotice::Clear
            ]
        );
        assert_eq!(observed.0.load(Ordering::SeqCst), 2);
    }

    /// A long detail phase may outlive the Classified discovery's normal lifetime.
    #[test]
    fn expired_classified_hierarchy_refreshes_before_publishing_completed_rows() {
        struct Attempts(AtomicUsize);
        impl HierarchyRunner for Attempts {
            fn run(&self, _: &str, _: Duration) -> Result<HierarchyEdges, HierarchyError> {
                self.0.fetch_add(1, Ordering::SeqCst);
                Err(HierarchyError::TimedOut)
            }
        }
        let runner = Arc::new(Attempts(AtomicUsize::new(0)));
        let observed = Arc::clone(&runner);
        let updates = spawn_with_test_source(
            HierarchyCommand::new(HierarchySource::Command("fixture".into())),
            runner,
            |answers| {
                answers
                    .publish(ScanStage::Classified, vec![session_row("worker", 1)])
                    .expect("classification publishes");
                if let ScanDiscovery::Classified { expires, .. } = &mut answers.discovery {
                    *expires -= HIERARCHY_REFRESH_INTERVAL;
                }
                answers
                    .publish(ScanStage::Completed, vec![session_row("worker", 1)])
                    .expect("expired discovery refreshes");
            },
        );
        let completions = updates
            .into_iter()
            .filter(|update| {
                matches!(
                    update,
                    CensusUpdate::Local {
                        stage: ScanStage::Completed,
                        ..
                    }
                )
            })
            .count();
        assert_eq!(completions, 1);
        assert_eq!(observed.0.load(Ordering::SeqCst), 2);
    }

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
            account: Account::Unrecorded,
            name: "worker".to_string(),
            activity: AgentActivity::Unreported,
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
    fn command_script(script: &str) -> (TempDir, HierarchySource) {
        let temp = tempfile::tempdir().expect("a fixture directory should open");
        let path = temp.path().join("hierarchy fixture.sh");
        std::fs::write(&path, script).expect("the fixture should be written");
        let source = HierarchySource::Command(format!("sh '{}'", path.display()));
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
                }])
                .into()
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
        for (script, reason) in [
            ("exit 1", HierarchyError::ExitedUnsuccessfully),
            ("printf 'malformed'", HierarchyError::OutputUnreadable),
            (
                "printf '{\"schema\":2,\"edges\":[]}'",
                HierarchyError::UnknownSchema(2),
            ),
            ("exec yes", HierarchyError::OutputTooLarge),
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
                noted(Vec::new(), HierarchyNotice::Failed(reason))
            );
            assert!(started.elapsed() < Duration::from_millis(800));
        }
        for close_stdout in ["", "exec > /dev/null;"] {
            let temp = tempfile::tempdir().expect("a fixture directory should open");
            let pid_file = temp.path().join("pid");
            let source = HierarchySource::Command(format!(
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
                noted(
                    Vec::new(),
                    HierarchyNotice::Failed(HierarchyError::TimedOut)
                )
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

    /// A pending hierarchy run cannot withhold completed row details already scanned.
    #[test]
    fn completed_rows_arrive_before_a_blocked_hierarchy_run_finishes() {
        struct BlockedHierarchy {
            entered:  Sender<Duration>,
            release:  Mutex<Receiver<()>>,
            attempts: AtomicUsize,
        }
        impl HierarchyRunner for BlockedHierarchy {
            fn run(&self, _: &str, timeout: Duration) -> Result<HierarchyEdges, HierarchyError> {
                if self.attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                    return Ok(HierarchyEdges::default());
                }
                self.entered
                    .send(timeout)
                    .expect("record the discovery deadline");
                self.release
                    .lock()
                    .expect("release lock")
                    .recv()
                    .expect("release discovery");
                Err(HierarchyError::TimedOut)
            }
        }
        let (entered, waiting) = mpsc::channel();
        let (release, released) = mpsc::channel();
        let runner = Arc::new(BlockedHierarchy {
            entered,
            release: Mutex::new(released),
            attempts: AtomicUsize::new(0),
        });
        let rows = vec![session_row("worker", 2)];
        let expected = rows.clone();
        let updates = spawn_with_test_source(
            HierarchyCommand::new(HierarchySource::Command("fixture".to_string())),
            runner,
            move |answers| {
                answers
                    .send(vec![session_row("previous", 1)])
                    .expect("prime a successful answer");
                answers
                    .publish(ScanStage::Completed, rows.clone())
                    .expect("publish rows");
            },
        );
        let timeout = waiting
            .recv_timeout(Duration::from_millis(200))
            .expect("discovery starts");
        let initial = updates.try_recv();
        let answer = updates.try_recv();
        release
            .send(())
            .expect("release the command before assertions");
        assert_eq!(timeout, Duration::from_secs(10));
        assert_eq!(
            initial.expect("initial successful answer"),
            noted(
                vec![session_row("previous", 1)],
                HierarchyNotice::NoSessionMatched
            )
        );
        assert_eq!(
            answer.expect("rows precede hierarchy completion"),
            CensusUpdate::Local {
                rows:      expected,
                hierarchy: HierarchySnapshot {
                    edges:  HierarchyEdges::default(),
                    notice: HierarchyNotice::NoSessionMatched,
                },
                stage:     ScanStage::Classified,
            }
        );
        assert!(updates.recv_timeout(Duration::from_millis(200)).is_ok());
    }

    /// First completed details must precede discovery even without an earlier success.
    #[test]
    fn uncached_completed_rows_arrive_before_hierarchy_discovery_is_released() {
        struct GatedHierarchy {
            entered: Sender<()>,
            release: Mutex<Receiver<()>>,
        }
        impl HierarchyRunner for GatedHierarchy {
            fn run(&self, _: &str, _: Duration) -> Result<HierarchyEdges, HierarchyError> {
                self.entered.send(()).expect("signal discovery");
                self.release
                    .lock()
                    .expect("release lock")
                    .recv()
                    .expect("release discovery");
                Err(HierarchyError::TimedOut)
            }
        }
        let (entered, waiting) = mpsc::channel();
        let (release, released) = mpsc::channel();
        let expected = vec![session_row("worker", 2)];
        let rows = expected.clone();
        let updates = spawn_with_test_source(
            HierarchyCommand::new(HierarchySource::Command("fixture".to_string())),
            Arc::new(GatedHierarchy {
                entered,
                release: Mutex::new(released),
            }),
            move |answers| {
                answers
                    .publish(ScanStage::Completed, rows.clone())
                    .expect("publish completion");
            },
        );
        waiting
            .recv_timeout(Duration::from_millis(200))
            .expect("discovery starts");
        let early = updates.try_recv();
        release
            .send(())
            .expect("release discovery before assertions");
        assert_eq!(
            early.expect("uncached completed rows precede discovery"),
            CensusUpdate::Local {
                rows:      expected.clone(),
                hierarchy: HierarchyEdges::default().into(),
                stage:     ScanStage::Classified,
            }
        );
        assert_eq!(
            updates
                .recv_timeout(Duration::from_millis(200))
                .expect("failed discovery answers"),
            noted(expected, HierarchyNotice::Failed(HierarchyError::TimedOut))
        );
    }

    /// An answer from a removed host cannot restore its hierarchy cache.
    #[test]
    fn removed_host_late_answer_cannot_supply_links_after_readding_it() {
        struct GatedReports {
            entered: Sender<()>,
            reports: Mutex<Receiver<Vec<u8>>>,
        }
        impl RemoteRunner for GatedReports {
            fn probe(&self, _: &str) -> ProbeOutcome {
                self.entered.send(()).expect("signal probe");
                ProbeOutcome::Finished {
                    status: Some(0),
                    stdout: self
                        .reports
                        .lock()
                        .expect("reports lock")
                        .recv()
                        .expect("release report"),
                }
            }
        }
        let (entered, waiting) = mpsc::channel();
        let (release, reports) = mpsc::channel();
        let runner: Arc<dyn RemoteRunner> = Arc::new(GatedReports {
            entered,
            reports: Mutex::new(reports),
        });
        let remotes = RemoteMachines::new(vec!["remote".to_string()]);
        let in_flight = InFlight::default();
        let answers = Arc::new(Mutex::new(RemoteHierarchyAnswers::default()));
        let (sender, receiver) = mpsc::channel();
        dispatch(
            "remote".to_string(),
            &remotes,
            &runner,
            &in_flight,
            &answers,
            &sender,
        );
        drop(sender);
        waiting
            .recv_timeout(Duration::from_millis(200))
            .expect("probe started");
        remotes.replace(Vec::new());
        release
            .send(
                serde_json::to_vec(&serde_json::json!({
                    "schema": crate::constants::PROBE_SCHEMA, "machine": "remote", "rows": [],
                    "edges": [{ "manager": "boss", "member": "worker", "role": "unit" }],
                }))
                .expect("serialize successful report"),
            )
            .expect("release removed host");
        let removed_answer = receiver.recv_timeout(Duration::from_millis(200));
        remotes.replace(vec!["remote".to_string()]);
        let (sender, receiver) = mpsc::channel();
        dispatch(
            "remote".to_string(),
            &remotes,
            &runner,
            &in_flight,
            &answers,
            &sender,
        );
        waiting
            .recv_timeout(Duration::from_millis(200))
            .expect("readded host starts probe");
        let rows = vec![session_row("worker", 2)];
        release
            .send(
                serde_json::to_vec(&serde_json::json!({
                    "schema": crate::constants::PROBE_SCHEMA, "machine": "remote", "rows": rows,
                    "hierarchy_failed": true,
                }))
                .expect("serialize failed hierarchy"),
            )
            .expect("release readded host");
        assert_eq!(
            receiver
                .recv_timeout(Duration::from_millis(200))
                .expect("fresh row answer"),
            CensusUpdate::Remote {
                host:  "remote".to_string(),
                state: MachineState::Answered(
                    rows,
                    HierarchySnapshot {
                        edges:  HierarchyEdges::default(),
                        notice: HierarchyNotice::Failed(HierarchyError::ExitedUnsuccessfully),
                    }
                ),
            }
        );
        assert_eq!(removed_answer, Err(mpsc::RecvTimeoutError::Disconnected));
    }

    /// The production remote dispatch preserves successful links, not failed empty output.
    #[test]
    fn remote_probe_hierarchy_failure_updates_rows_without_flattening_links() {
        struct Reports(Mutex<VecDeque<Vec<u8>>>);
        impl RemoteRunner for Reports {
            fn probe(&self, _: &str) -> ProbeOutcome {
                ProbeOutcome::Finished {
                    status: Some(0),
                    stdout: self
                        .0
                        .lock()
                        .expect("reports lock")
                        .pop_front()
                        .expect("queued report"),
                }
            }
        }
        let edges = HierarchyEdges(vec![super::super::HierarchyEdge {
            manager: "boss".to_string(),
            member:  "worker".to_string(),
            role:    super::super::HierarchyRole::Unit,
        }]);
        let reports = [
            (false, edges.clone()),
            (true, HierarchyEdges::default()),
            (false, HierarchyEdges::default()),
        ]
        .into_iter()
        .enumerate()
        .map(|(index, (failed, edges))| {
            serde_json::to_vec(&serde_json::json!({
                "schema": crate::constants::PROBE_SCHEMA, "machine": "remote",
                "rows": [session_row("worker", u32::try_from(index).expect("small index"))],
                "edges": edges, "hierarchy_failed": failed,
            }))
            .expect("serialize report")
        })
        .collect();
        let runner: Arc<dyn RemoteRunner> = Arc::new(Reports(Mutex::new(reports)));
        let in_flight = InFlight::default();
        let answers = Arc::new(Mutex::new(RemoteHierarchyAnswers::default()));
        let (sender, receiver) = mpsc::channel();
        let mut census = super::super::Census::new("local".to_string());
        let configured = vec!["remote".to_string()];
        let remotes = RemoteMachines::new(configured.clone());
        for (pid, expected) in [edges.clone(), edges, HierarchyEdges::default()]
            .into_iter()
            .enumerate()
        {
            dispatch(
                "remote".to_string(),
                &remotes,
                &runner,
                &in_flight,
                &answers,
                &sender,
            );
            census.apply(
                receiver
                    .recv_timeout(Duration::from_millis(200))
                    .expect("remote answer"),
            );
            assert_eq!(
                census.machines(&configured)[1].state,
                &MachineState::Answered(
                    vec![session_row(
                        "worker",
                        u32::try_from(pid).expect("small pid")
                    )],
                    HierarchySnapshot {
                        edges:  expected,
                        notice: if pid == 1 {
                            HierarchyNotice::Failed(HierarchyError::ExitedUnsuccessfully)
                        } else {
                            HierarchyNotice::Clear
                        },
                    }
                )
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
        let hierarchy = HierarchyCommand::new(HierarchySource::Command("first".to_string()));
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
                edits.replace(HierarchySource::Command("second".to_string()));
                sender
                    .send(vec![session_row("b", 2)])
                    .expect("new setting should send");
                edits.replace(HierarchySource::Off);
                sender
                    .send(vec![session_row("b", 2)])
                    .expect("disabled hierarchy should send");
            },
        );
        let answers: Vec<_> = updates.into_iter().collect();
        let managers: Vec<_> = answers
            .iter()
            .filter_map(|update| match update {
                CensusUpdate::Local { hierarchy, .. } => Some(
                    hierarchy
                        .edges
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
                CensusUpdate::Local { rows, .. } => {
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
