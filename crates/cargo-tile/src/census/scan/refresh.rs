//! Refresh deadlines and the scan worker.

#[cfg(test)]
#[path = "refresh_tests.rs"]
mod refresh_tests;

use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::mpsc::Receiver;
use std::thread;
use std::thread::JoinHandle;
use std::time::Duration;
use std::time::Instant;

use sysinfo::Pid;
#[cfg(any(not(target_os = "linux"), test))]
use sysinfo::ProcessRefreshKind;
#[cfg(any(not(target_os = "linux"), test))]
use sysinfo::ProcessesToUpdate;
#[cfg(any(not(target_os = "linux"), test))]
use sysinfo::System;
#[cfg(any(not(target_os = "linux"), test))]
use sysinfo::UpdateKind;

use super::census::Census;
#[cfg(not(target_os = "linux"))]
use super::discovery::DiscoveryProcess;
#[cfg(target_os = "macos")]
use super::discovery::InvocationLifetimeReads;
use super::discovery::ProcessDiscovery;
#[cfg(target_os = "linux")]
use super::linux_details;
use super::observation::ProcessObservations;
use super::scope::CensusScope;
use super::scope::ExcludedCommands;
#[cfg(test)]
use super::tests;
#[cfg(target_os = "macos")]
use crate::birth_stamp::LifetimeEvidence;
use crate::census::command_text::ScannerHome;
use crate::census::invocation_cpu_accounting::CensusCadence;
use crate::census::invocation_cpu_accounting::CpuBaseline;
use crate::census::invocation_cpu_accounting::InvocationCpuAccounting;
#[cfg(target_os = "macos")]
use crate::census::invocation_cpu_accounting::Measurement;
use crate::constants::PROCESS_IDLE_POLL_MILLIS;
use crate::progress::capture::Capture;
use crate::progress::capture_roots::CaptureRoots;
use crate::terminal::Scan;

/// The last census chooses discovery's active or idle pace.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RefreshPace {
    Active,
    Idle,
}

impl RefreshPace {
    fn for_census(census: &Census, capture: &Capture) -> Self {
        if census.live_cargo().next().is_some() || !capture.confirmed().is_empty() {
            Self::Active
        } else {
            Self::Idle
        }
    }
}

/// Start deadlines share the cadence's minimum spacing, even when idle is interrupted.
/// Idle discovery reserves one publication pass before its maximum interval elapses.
struct NextRefresh {
    poll:   Duration,
    active: Instant,
    idle:   Instant,
}

impl NextRefresh {
    const fn new(cadence: CensusCadence, now: Instant) -> Self {
        Self {
            poll:   cadence.poll,
            active: now,
            idle:   now,
        }
    }

    const fn deadline(&self, pace: RefreshPace) -> Instant {
        match pace {
            RefreshPace::Active => self.active,
            RefreshPace::Idle => self.idle,
        }
    }

    fn start(&mut self, now: Instant, pace: RefreshPace) -> Result<(), RefreshNotDue> {
        if now < self.deadline(pace) {
            return Err(RefreshNotDue);
        }
        self.active = now + self.poll;
        self.idle = now
            + Duration::from_millis(PROCESS_IDLE_POLL_MILLIS)
                .saturating_sub(self.poll / 2)
                .max(self.poll);
        Ok(())
    }
}

struct RefreshNotDue;

/// A refresh source owns the process table and publishes independent snapshots.
trait ProcessRefreshSource {
    type Snapshot;

    fn refresh(&mut self) -> Self::Snapshot;
}

/// One table survives every refresh; the worker's actual start time sets its next deadline.
struct RefreshWorker<Source> {
    source: Source,
    next:   NextRefresh,
}

impl<Source: ProcessRefreshSource> RefreshWorker<Source> {
    fn new(build: impl FnOnce() -> Source, cadence: CensusCadence, now: Instant) -> Self {
        Self {
            source: build(),
            next:   NextRefresh::new(cadence, now),
        }
    }

    fn sample(&mut self, now: Instant, pace: RefreshPace) -> StageAnswer<Source::Snapshot> {
        if self.next.start(now, pace).is_err() {
            return StageAnswer::Pending;
        }
        StageAnswer::Completed(self.source.refresh())
    }
}

/// Available bytes on the file system that holds the working directory.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum FreeDiskSpace {
    Available(u64),
    #[default]
    Unavailable,
}

impl FreeDiskSpace {
    /// Read available blocks once for each census publication.
    pub(crate) fn read(directory: &Path) -> Self {
        rustix::fs::statvfs(directory).map_or(Self::Unavailable, |stat| {
            stat.f_bavail
                .checked_mul(stat.f_frsize)
                .map_or(Self::Unavailable, Self::Available)
        })
    }
}

/// This table belongs only to the refresh worker, never to a published snapshot.
#[derive(Default)]
struct SystemRefresh {
    #[cfg(not(target_os = "linux"))]
    system: System,
}

impl ProcessRefreshSource for SystemRefresh {
    type Snapshot = ProcessDiscovery;

    #[cfg(target_os = "linux")]
    fn refresh(&mut self) -> Self::Snapshot { ProcessDiscovery::linux() }

    #[cfg(not(target_os = "linux"))]
    fn refresh(&mut self) -> Self::Snapshot {
        self.system.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            process_discovery_refresh_kind(),
        );
        ProcessDiscovery {
            processes: self
                .system
                .processes()
                .iter()
                .map(|(&pid, process)| (pid, DiscoveryProcess::from_process(process)))
                .collect(),
            baselines: self
                .system
                .processes()
                .iter()
                .map(|(&pid, process)| {
                    #[cfg(target_os = "macos")]
                    let baseline = CpuBaseline {
                        lifetime:    LifetimeEvidence::Unavailable,
                        accumulated: process.accumulated_cpu_time(),
                    };
                    #[cfg(not(target_os = "macos"))]
                    let baseline = CpuBaseline::from(process);
                    #[cfg(test)]
                    tests::SYSINFO_BASELINE_READS.with(|count| count.set(count.get() + 1));
                    (pid, baseline)
                })
                .collect(),
        }
    }
}

#[cfg(all(test, target_os = "linux"))]
mod system_refresh_tests {
    use super::Pid;
    use super::ProcessRefreshSource;
    use super::SystemRefresh;
    use super::tests::SYSINFO_BASELINE_READS;

    #[cfg(target_os = "linux")]
    #[test]
    fn native_discovery_uses_the_stat_birth_without_independent_baseline_reads() {
        SYSINFO_BASELINE_READS.with(|count| count.set(0));
        let mut source = SystemRefresh::default();
        let discovery = source.refresh();
        assert!(
            discovery
                .processes
                .contains_key(&Pid::from_u32(std::process::id()))
        );
        SYSINFO_BASELINE_READS.with(|count| assert_eq!(count.get(), 0));
    }
}

/// A refresh has either no preceding sample or counters from its actual predecessor.
pub(super) enum RefreshHistory {
    FirstObservation,
    Previous(HashMap<Pid, CpuBaseline>),
}

impl RefreshHistory {
    #[cfg(target_os = "macos")]
    pub(super) fn measure(&self, pid: Pid, rate: f32, baseline: &CpuBaseline) -> Measurement<f32> {
        match self {
            Self::FirstObservation => Census::measure_cpu(pid, rate, baseline, &HashMap::new()),
            Self::Previous(previous) => Census::measure_cpu(pid, rate, baseline, previous),
        }
    }

    fn observations<'scan>(
        &self,
        discovery: &'scan ProcessDiscovery,
    ) -> ProcessObservations<'scan> {
        match self {
            Self::FirstObservation => discovery.observations_since(&HashMap::new()),
            Self::Previous(baselines) => discovery.observations_since(baselines),
        }
    }
}

/// A declined refresh never manufactures a process snapshot.
enum StageAnswer<T> {
    Completed(T),
    Pending,
}

/// Start the scanner thread and hand back the channel it publishes on.
///
/// The thread ends when the receiver is dropped.
///
/// Each scan reads `excluded` afresh, `cadence` sets the pause between
/// scans and how the `cpu` column follows them, and `scope` sets which
/// cargo processes they count. Parent resolution runs once inside the
/// worker before its scan loop, so a slow filesystem cannot block terminal
/// startup; each scan still opens every root to recheck access and
/// ownership. Keep root resolution on the worker even when it stalls. The
/// resolver and join handle let tests hold resolution and observe repeated
/// scans and shutdown.
pub(crate) fn spawn_with_resolver(
    excluded: ExcludedCommands,
    cadence: CensusCadence,
    scope: CensusScope,
    resolve: impl FnOnce() -> CaptureRoots + Send + 'static,
) -> (Receiver<Scan>, JoinHandle<()>) {
    spawn_resolved_scan_loop(cadence.poll / 2, resolve, move || {
        let mut worker = ScanWorker {
            refresh: RefreshWorker::new(SystemRefresh::default, cadence, Instant::now()),
            history: RefreshHistory::FirstObservation,
            pace: RefreshPace::Active,
            smoothing: InvocationCpuAccounting::new(cadence),
            home: dirs::home_dir()
                .map_or(ScannerHomeLocation::Unavailable, ScannerHomeLocation::Known),
            excluded,
            scope,
            cadence,
        };
        move |roots| worker.scan(roots)
    })
}

/// Resolve capture roots once, then publish every scan produced from them.
fn spawn_resolved_scan_loop<Published, Prepare, TakeScan>(
    poll: Duration,
    resolve: impl FnOnce() -> CaptureRoots + Send + 'static,
    prepare: Prepare,
) -> (Receiver<Published>, JoinHandle<()>)
where
    Published: Send + 'static,
    Prepare: FnOnce() -> TakeScan + Send + 'static,
    TakeScan: FnMut(&CaptureRoots) -> Published + Send + 'static,
{
    let (sender, receiver) = mpsc::channel();
    let worker = thread::spawn(move || {
        let roots = resolve();
        let mut take_scan = prepare();
        loop {
            if sender.send(take_scan(&roots)).is_err() {
                return;
            }
            thread::sleep(poll);
        }
    });
    (receiver, worker)
}

/// The scanner account's home, retained across scan publications.
enum ScannerHomeLocation {
    Known(PathBuf),
    Unavailable,
}

impl ScannerHomeLocation {
    fn borrowed(&self) -> ScannerHome<'_> {
        match self {
            Self::Known(home) => ScannerHome::Known(home.as_path()),
            Self::Unavailable => ScannerHome::Unavailable,
        }
    }
}

/// Only a fresh refresh can supply the next scan.
enum ScanInput {
    AwaitingRefresh,
    Fresh(FreshProcessScan),
}
struct FreshProcessScan {
    discovery: ProcessDiscovery,
}
impl From<StageAnswer<ProcessDiscovery>> for ScanInput {
    fn from(answer: StageAnswer<ProcessDiscovery>) -> Self {
        match answer {
            StageAnswer::Pending => Self::AwaitingRefresh,
            StageAnswer::Completed(discovery) => Self::Fresh(FreshProcessScan { discovery }),
        }
    }
}

/// The worker owns discovery, accounting and its last completed census's pace.
struct ScanWorker {
    refresh:   RefreshWorker<SystemRefresh>,
    history:   RefreshHistory,
    pace:      RefreshPace,
    smoothing: InvocationCpuAccounting,
    home:      ScannerHomeLocation,
    excluded:  ExcludedCommands,
    scope:     CensusScope,
    cadence:   CensusCadence,
}

impl ScanWorker {
    /// Check registrations between refreshes so an idle worker wakes within one poll.
    fn scan(&mut self, roots: &CaptureRoots) -> Scan {
        loop {
            let capture = Capture::take(roots);
            if !capture.confirmed().is_empty() {
                self.pace = RefreshPace::Active;
            }
            let now = Instant::now();
            match ScanInput::from(self.refresh.sample(now, self.pace)) {
                ScanInput::Fresh(fresh) => return self.scan_discovery(fresh, capture, now),
                ScanInput::AwaitingRefresh => {},
            }
            let deadline = self
                .refresh
                .next
                .deadline(self.pace)
                .min(now + self.cadence.poll);
            thread::sleep(deadline.saturating_duration_since(Instant::now()));
        }
    }

    /// Assemble a publication from one fresh discovery and independently read details.
    fn scan_discovery(
        &mut self,
        fresh: FreshProcessScan,
        mut capture: Capture,
        now: Instant,
    ) -> Scan {
        let FreshProcessScan { discovery } = fresh;
        #[cfg(target_os = "macos")]
        let mut discovery = discovery;
        let mut observations = self.history.observations(&discovery);
        let mut census = Census::take(observations.records.values());
        census.restrict_to(self.scope);
        #[cfg(target_os = "macos")]
        {
            let invocation_lifetime_reads = InvocationLifetimeReads::for_census(&census, &capture);
            invocation_lifetime_reads.apply(
                &mut census,
                &mut observations,
                &discovery,
                &self.history,
            );
        }
        // Excluded cargo still counts as live work for choosing the refresh pace.
        self.pace = RefreshPace::for_census(&census, &capture);
        census.identities = self.smoothing.identities.observe(&census.lifetimes);

        // Registration pids receive fresh argv even when they exec between refreshes.
        let mut detailed = census.detailed();
        for pid in capture.registered_pids().into_iter().map(Pid::from_u32) {
            for candidate in
                std::iter::once(pid).chain(census.ancestor_pids(pid).as_slice().iter().copied())
            {
                if !detailed.contains(&candidate) {
                    detailed.push(candidate);
                }
            }
        }
        #[cfg(target_os = "linux")]
        let details = linux_details::CurrentProcessDetails::read(&detailed, &discovery);
        #[cfg(target_os = "linux")]
        details.overlay(&mut observations);
        #[cfg(not(target_os = "linux"))]
        let details = process_details(&detailed);
        #[cfg(not(target_os = "linux"))]
        observations.overlay_details(&details);
        census.prepare(&observations, &capture, &self.excluded.snapshot());
        let attributed = census.attribute(&observations, &mut self.smoothing, now);
        let groups = census.groups(&observations, &attributed, self.home.borrowed(), &capture);
        census.associate_status(&mut capture, &groups);
        #[cfg(target_os = "macos")]
        for (pid, baseline) in &mut discovery.baselines {
            if let Some(lifetime) = census.lifetimes.remove(pid) {
                baseline.lifetime = lifetime;
            }
        }
        self.history = RefreshHistory::Previous(discovery.baselines);
        Scan {
            #[cfg(test)]
            cpu_scan_publication: attributed.cpu_scan_publication,
            memory_shares: attributed.memory_shares,
            sccache: census.sccache(),
            free_disk_space: FreeDiskSpace::read(Path::new(".")),
            groups,
            root_status: capture.root_status,
            shared_directory: capture.shared_directory,
        }
    }
}

/// Fields read for the full-system pass.
///
/// Tasks are Linux threads. They inherit a process's name and command,
/// so including them would draw one cargo invocation more than once.
#[cfg(any(not(target_os = "linux"), test))]
pub(super) fn process_discovery_refresh_kind() -> ProcessRefreshKind {
    let kind = ProcessRefreshKind::nothing()
        .without_tasks()
        .with_cpu()
        .with_memory();
    #[cfg(target_os = "macos")]
    let kind = kind.with_exe(UpdateKind::Always);
    kind
}

/// Fields read for cargo, its ancestors, and compiler ownership evidence.
#[cfg(any(not(target_os = "linux"), test))]
pub(super) fn process_detail_refresh_kind() -> ProcessRefreshKind {
    ProcessRefreshKind::nothing()
        .without_tasks()
        .with_cwd(UpdateKind::OnlyIfNotSet)
        .with_cmd(UpdateKind::OnlyIfNotSet)
        .with_exe(UpdateKind::OnlyIfNotSet)
        .with_environ(UpdateKind::OnlyIfNotSet)
        .with_user(UpdateKind::OnlyIfNotSet)
}

/// sysinfo retains populated metadata even when a requested reread fails.
/// Fresh detail records cannot carry cwd, argv or exe across a pid replacement
/// that its whole-second start time cannot distinguish.
#[cfg(any(not(target_os = "linux"), test))]
pub(super) fn process_details(pids: &[Pid]) -> System {
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(pids),
        false,
        process_detail_refresh_kind(),
    );
    system
}

#[cfg(test)]
#[path = "../refresh_worker_tests.rs"]
mod refresh_worker_tests;
