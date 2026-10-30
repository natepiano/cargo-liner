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
use std::sync::mpsc::RecvTimeoutError;
use std::sync::mpsc::SendError;
use std::sync::mpsc::Sender;
use std::thread;
use std::time::Duration;
use std::time::Instant;

use super::Agent;
use super::AgentRow;
use super::CensusUpdate;
use super::CodexThreadId;
use super::HierarchyCommand;
use super::HierarchyEdges;
use super::HierarchySnapshot;
use super::HierarchySource;
use super::RemoteHierarchyAnswers;
use super::RemoteMachines;
use super::SessionIdentity;
use super::account::AccountLabels;
use super::hierarchy;
#[cfg(test)]
use super::hierarchy::CommandRunner;
use super::hierarchy::HierarchyCache;
use super::hierarchy::HierarchyError;
use super::hierarchy::HierarchyNotice;
use super::hierarchy::HierarchyPreparation;
use super::hierarchy::HierarchyPublication;
use super::hierarchy::HierarchyRefreshScope;
use super::hierarchy::HierarchyRequestKind;
use super::hierarchy::HierarchyRunner;
use super::hierarchy::InputDrivenHierarchyRunner;
use super::hierarchy::ReviewerPools;
use super::probe;
use super::remote;
use super::remote::ProbeOutcome;
use super::remote::RemoteRunner;
use super::remote::SshRunner;
use super::scan::LocalScanner;
use super::scan::ScanStage;
use crate::constants::HIERARCHY_FIRST_PUBLISH_BUDGET;
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

/// Events wake the scheduler without waiting for the next local scan.
enum ScheduleEvent {
    HierarchyReady,
    Scanned(ScanSchedule),
}

/// A scan runs once at a time while the scheduler accepts hierarchy outcomes.
enum LocalScanState {
    Ready,
    Running,
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

/// A row stays held if its session or any hosted thread can be placed by hierarchy.
enum PendingPlacementRow {
    Independent(AgentRow),
    AwaitingHierarchy(AgentRow),
}

/// Keep row order while separating safe publications from unresolved placement.
struct PendingPlacementRows(Vec<PendingPlacementRow>);

impl PendingPlacementRows {
    fn new(rows: Vec<AgentRow>) -> Self {
        Self(
            rows.into_iter()
                .map(|row| {
                    if matches!(row.agent, Agent::Claude)
                        || row
                            .children
                            .iter()
                            .any(|child| matches!(child.thread_id, CodexThreadId::Recorded(_)))
                    {
                        PendingPlacementRow::AwaitingHierarchy(row)
                    } else {
                        PendingPlacementRow::Independent(row)
                    }
                })
                .collect(),
        )
    }

    /// One unresolved identity holds every row in its retained linked component.
    fn refreshing(rows: Vec<AgentRow>, scope: &HierarchyRefreshScope) -> Self {
        let mut waiting: HashSet<String> = rows
            .iter()
            .flat_map(HierarchyRefreshScope::placement_ids)
            .filter(|id| scope.waits_for(id))
            .map(str::to_owned)
            .collect();
        loop {
            let before = waiting.len();
            for row in &rows {
                if HierarchyRefreshScope::placement_ids(row).any(|id| waiting.contains(id)) {
                    waiting.extend(HierarchyRefreshScope::placement_ids(row).map(str::to_owned));
                }
            }
            for edge in &scope.links().0 {
                if waiting.contains(edge.manager.as_str()) || waiting.contains(edge.member.as_str())
                {
                    waiting.insert(edge.manager.clone());
                    waiting.insert(edge.member.clone());
                }
            }
            if waiting.len() == before {
                break;
            }
        }
        Self(
            rows.into_iter()
                .map(|row| {
                    if HierarchyRefreshScope::placement_ids(&row).any(|id| waiting.contains(id)) {
                        PendingPlacementRow::AwaitingHierarchy(row)
                    } else {
                        PendingPlacementRow::Independent(row)
                    }
                })
                .collect(),
        )
    }

    fn independent(&self) -> Vec<AgentRow> {
        self.0
            .iter()
            .filter_map(|row| match row {
                PendingPlacementRow::Independent(row) => Some(row.clone()),
                PendingPlacementRow::AwaitingHierarchy(_) => None,
            })
            .collect()
    }

    fn into_rows(self) -> Vec<AgentRow> {
        self.0
            .into_iter()
            .map(|row| match row {
                PendingPlacementRow::Independent(row)
                | PendingPlacementRow::AwaitingHierarchy(row) => row,
            })
            .collect()
    }
}

/// Session ids observed at request launch, before later scan results.
struct HierarchyRequestSessions(HashSet<String>);

impl HierarchyRequestSessions {
    fn new(rows: &[AgentRow]) -> Self {
        Self(
            rows.iter()
                .filter_map(|row| match &row.session_id {
                    SessionIdentity::Claude(id) => Some(id.clone()),
                    SessionIdentity::Unrecorded => None,
                })
                .collect(),
        )
    }

    fn newcomers(&self, rows: &[AgentRow]) -> Vec<AgentRow> {
        rows.iter()
            .filter(|row| match &row.session_id {
                SessionIdentity::Claude(id) => !self.0.contains(id),
                SessionIdentity::Unrecorded => false,
            })
            .cloned()
            .collect()
    }
}

/// Rows whose required hierarchy outcome has not arrived yet.
struct HeldHierarchyRows {
    rows:     PendingPlacementRows,
    stage:    ScanStage,
    observed: Instant,
}

/// A cache is available only after its pending command has returned.
enum LocalHierarchy {
    Available(Box<HierarchyCache>),
    Pending {
        cache:   Box<HierarchyCache>,
        source:  HierarchySource,
        request: Box<PendingHierarchyRequest>,
        publish: HeldHierarchyRows,
    },
}

/// A request owns its receiver and the session ids its command could observe.
struct HierarchyRequestAnswer {
    answer:    Receiver<Result<HierarchyEdges, HierarchyError>>,
    requested: HierarchyRequestSessions,
}

/// Initial outcomes initialize checked ids; refresh outcomes must retain them.
enum PendingHierarchyRequest {
    Initial(HierarchyRequestAnswer),
    Refresh {
        answer:    HierarchyRequestAnswer,
        placement: HierarchyRefreshScope,
    },
}

impl PendingHierarchyRequest {
    const fn answer(&self) -> &HierarchyRequestAnswer {
        match self {
            Self::Initial(answer) | Self::Refresh { answer, .. } => answer,
        }
    }
}

impl Default for LocalHierarchy {
    fn default() -> Self { Self::Available(Box::default()) }
}

/// A scan publishes through shared state so detail reads cannot delay a ready hierarchy answer.
pub(crate) struct LocalAnswers {
    /// Share one cache owner so scheduler wakeups can publish during scan detail reads.
    publications: Arc<Mutex<LocalPublications>>,
}

impl LocalAnswers {
    /// Publish a stage without holding the publication owner during detail reads.
    pub(super) fn publish(
        &self,
        stage: ScanStage,
        rows: Vec<AgentRow>,
    ) -> Result<(), SendError<CensusUpdate>> {
        self.publish_at(stage, rows, Instant::now())
    }

    /// Synthetic observation times let cache tests check expiry without sleeping.
    pub(super) fn publish_at(
        &self,
        stage: ScanStage,
        rows: Vec<AgentRow>,
        now: Instant,
    ) -> Result<(), SendError<CensusUpdate>> {
        self.publications
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .publish_at(stage, rows, now)
    }

    /// Older sequential fixtures accept the worker reply before they publish the next scan.
    #[cfg(test)]
    pub(super) fn settle_test_request(&self) -> Result<(), SendError<CensusUpdate>> {
        let mut publications = self
            .publications
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let budget = publications.first_budget;
        publications.finish_pending(budget)
    }

    /// Completed-only fixtures represent one whole scan per call.
    #[cfg(test)]
    pub(crate) fn send(&self, rows: Vec<AgentRow>) -> Result<(), SendError<CensusUpdate>> {
        self.publications
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .send_completed_fixture(rows)
    }
}

/// The scheduler and scan callbacks share one owner of cache and pending rows.
struct LocalPublications {
    /// Send only rows with a known placement outcome to the app.
    sender:       Sender<CensusUpdate>,
    /// Read the current setting so old command results cannot restore a removed source.
    hierarchy:    HierarchyCommand,
    /// Run discovery outside the scan to release ready results during detail reads.
    runner:       Arc<dyn HierarchyRunner>,
    /// Keep a pending command separate from an available cache.
    state:        LocalHierarchy,
    /// Reuse Classified discovery within the same scan until its result expires.
    discovery:    ScanDiscovery,
    /// Bound the startup wait so unrelated rows can publish before discovery ends.
    first_budget: Duration,
    /// Wake the scheduler when background discovery can release held rows.
    events:       Sender<ScheduleEvent>,
}

impl LocalPublications {
    /// Completed-only fixtures wait explicitly because they have no detail callback to hold open.
    #[cfg(test)]
    fn send_completed_fixture(
        &mut self,
        rows: Vec<AgentRow>,
    ) -> Result<(), SendError<CensusUpdate>> {
        let source = self.hierarchy.snapshot();
        let now = Instant::now();
        let LocalHierarchy::Available(cache) = &mut self.state else {
            return Ok(());
        };
        match cache.prepare_snapshot(source.clone(), &rows, now) {
            HierarchyPreparation::Request(kind) => {
                self.start_request(source, rows, ScanStage::Completed, now, kind)?;
                self.finish_pending(self.first_budget)
            },
            HierarchyPreparation::Ready(hierarchy) => {
                self.deliver(rows, hierarchy, ScanStage::Completed)
            },
        }
    }

    /// Tests can advance observation time without sleeping.
    pub(super) fn publish_at(
        &mut self,
        stage: ScanStage,
        rows: Vec<AgentRow>,
        now: Instant,
    ) -> Result<(), SendError<CensusUpdate>> {
        let source = self.hierarchy.snapshot();
        if let LocalHierarchy::Pending {
            source: pending, ..
        } = &self.state
            && *pending != source
        {
            self.state = LocalHierarchy::default();
            self.discovery = ScanDiscovery::Unstarted;
        }
        if let LocalHierarchy::Pending {
            publish, request, ..
        } = &mut self.state
        {
            let rows = match request.as_ref() {
                PendingHierarchyRequest::Initial(_) => PendingPlacementRows::new(rows),
                PendingHierarchyRequest::Refresh { placement, .. } => {
                    PendingPlacementRows::refreshing(rows, placement)
                },
            };
            *publish = HeldHierarchyRows {
                rows,
                stage,
                observed: now,
            };
            self.finish_pending(Duration::ZERO)?;
            return self.deliver_held();
        }
        let LocalHierarchy::Available(cache) = &mut self.state else {
            return Ok(());
        };
        if matches!(stage, ScanStage::Completed) {
            match std::mem::take(&mut self.discovery) {
                ScanDiscovery::Classified {
                    source: classified,
                    expires,
                } if classified == source
                    && now < expires
                    && (matches!(
                        cache.retained(source.clone()).notice,
                        HierarchyNotice::Failed(_)
                    ) || matches!(
                        cache.publication(source.clone(), &rows),
                        HierarchyPublication::Ready(_)
                    )) =>
                {
                    let hierarchy = cache.retained(source);
                    self.deliver(rows.clone(), hierarchy.clone(), ScanStage::Classified)?;
                    return self.deliver(rows, hierarchy, stage);
                },
                _ => {},
            }
        }
        match cache.publication(source.clone(), &rows) {
            HierarchyPublication::Pending => self.start_initial(source, rows, stage, now),
            HierarchyPublication::RefreshNeeded => self.discover_at(rows, stage, now),
            HierarchyPublication::Ready(hierarchy) => {
                self.deliver(rows.clone(), hierarchy, ScanStage::Classified)?;
                if matches!(source, HierarchySource::Command(_))
                    || matches!(stage, ScanStage::Completed)
                {
                    self.discover_at(rows, stage, now)?;
                } else if let LocalHierarchy::Available(cache) = &self.state {
                    self.discovery = ScanDiscovery::Classified {
                        source,
                        expires: cache.reuse_deadline(now),
                    };
                }
                Ok(())
            },
        }
    }

    /// Only the scan worker spends the startup budget; scheduler wakeups never wait.
    fn start_initial(
        &mut self,
        source: HierarchySource,
        rows: Vec<AgentRow>,
        stage: ScanStage,
        now: Instant,
    ) -> Result<(), SendError<CensusUpdate>> {
        self.start_request(source, rows, stage, now, HierarchyRequestKind::Initial)?;
        self.finish_pending(self.first_budget)?;
        self.deliver_held()
    }

    /// Every command runs on its own worker, without blocking publication or the scheduler.
    fn start_request(
        &mut self,
        source: HierarchySource,
        rows: Vec<AgentRow>,
        stage: ScanStage,
        now: Instant,
        kind: HierarchyRequestKind,
    ) -> Result<(), SendError<CensusUpdate>> {
        let LocalHierarchy::Available(cache) = std::mem::take(&mut self.state) else {
            return Ok(());
        };
        let (sender, answer) = mpsc::channel();
        let runner = Arc::clone(&self.runner);
        let command = source.clone();
        let events = self.events.clone();
        let requested = HierarchyRequestSessions::new(&rows);
        thread::spawn(move || {
            let _ = sender.send(hierarchy::read(&command, runner.as_ref()));
            let _ = events.send(ScheduleEvent::HierarchyReady);
        });
        let answer = HierarchyRequestAnswer { answer, requested };
        let (request, rows) = match kind {
            HierarchyRequestKind::Initial => (
                PendingHierarchyRequest::Initial(answer),
                PendingPlacementRows::new(rows),
            ),
            HierarchyRequestKind::Refresh(placement) => {
                let rows = PendingPlacementRows::refreshing(rows, &placement);
                (PendingHierarchyRequest::Refresh { answer, placement }, rows)
            },
        };
        self.state = LocalHierarchy::Pending {
            cache,
            source,
            request: Box::new(request),
            publish: HeldHierarchyRows {
                rows,
                stage,
                observed: now,
            },
        };
        self.finish_pending(Duration::ZERO)
    }

    /// Publish independent rows while either hierarchy request is pending.
    /// Refreshes use `Classified` because `Completed` reloads footer colors in
    /// terminal.rs `drain_census` and must wait for the full row set.
    fn deliver_held(&mut self) -> Result<(), SendError<CensusUpdate>> {
        let LocalHierarchy::Pending {
            publish,
            cache,
            source,
            request,
        } = &mut self.state
        else {
            return Ok(());
        };
        let rows = publish.rows.independent();
        let hierarchy = cache.retained(source.clone());
        let stage = match request.as_ref() {
            PendingHierarchyRequest::Initial(_) => publish.stage,
            PendingHierarchyRequest::Refresh { .. } => ScanStage::Classified,
        };
        self.deliver(rows, hierarchy, stage)?;
        Ok(())
    }

    /// Publish held rows only with the command's outcome, then restore cache ownership.
    fn finish_pending(&mut self, timeout: Duration) -> Result<(), SendError<CensusUpdate>> {
        if let LocalHierarchy::Pending { source, .. } = &self.state
            && *source != self.hierarchy.snapshot()
        {
            let old = std::mem::take(&mut self.state);
            self.discovery = ScanDiscovery::Unstarted;
            if let LocalHierarchy::Pending { publish, .. } = old {
                return self.discover_at(publish.rows.into_rows(), publish.stage, publish.observed);
            }
            return Ok(());
        }
        let outcome = match &self.state {
            LocalHierarchy::Available(_) => return Ok(()),
            LocalHierarchy::Pending { request, .. } => {
                match request.answer().answer.recv_timeout(timeout) {
                    Ok(outcome) => outcome,
                    Err(RecvTimeoutError::Timeout) => return Ok(()),
                    Err(RecvTimeoutError::Disconnected) => Err(HierarchyError::OutputUnreadable),
                }
            },
        };
        if let LocalHierarchy::Pending {
            mut cache,
            source,
            publish,
            request,
        } = std::mem::take(&mut self.state)
        {
            let rows = publish.rows.into_rows();
            let (hierarchy, requested) = match *request {
                PendingHierarchyRequest::Initial(answer) => (
                    cache.first_outcome(source.clone(), &rows, outcome, publish.observed),
                    answer.requested,
                ),
                PendingHierarchyRequest::Refresh { answer, .. } => (
                    cache.refresh_outcome(source.clone(), &rows, outcome, publish.observed),
                    answer.requested,
                ),
            };
            if !matches!(hierarchy.notice, HierarchyNotice::Failed(_))
                && matches!(
                    cache.publication(source.clone(), &requested.newcomers(&rows)),
                    HierarchyPublication::RefreshNeeded
                )
            {
                self.state = LocalHierarchy::Available(cache);
                return self.discover_at(rows, publish.stage, publish.observed);
            }
            self.discovery = match publish.stage {
                ScanStage::Classified => ScanDiscovery::Classified {
                    source,
                    expires: cache.reuse_deadline(publish.observed),
                },
                ScanStage::Completed => ScanDiscovery::Unstarted,
            };
            self.state = LocalHierarchy::Available(cache);
            self.deliver(rows, hierarchy, publish.stage)?;
        }
        Ok(())
    }

    /// Refresh before publication so new sessions cannot flash with stale placement.
    fn discover_at(
        &mut self,
        rows: Vec<AgentRow>,
        stage: ScanStage,
        now: Instant,
    ) -> Result<(), SendError<CensusUpdate>> {
        let source = self.hierarchy.snapshot();
        let LocalHierarchy::Available(cache) = &mut self.state else {
            return Ok(());
        };
        match cache.prepare_snapshot(source.clone(), &rows, now) {
            HierarchyPreparation::Request(kind) => {
                self.start_request(source, rows, stage, now, kind)?;
                self.deliver_held()
            },
            HierarchyPreparation::Ready(hierarchy) => {
                self.discovery = match stage {
                    ScanStage::Classified => ScanDiscovery::Classified {
                        source,
                        expires: cache.reuse_deadline(now),
                    },
                    ScanStage::Completed => ScanDiscovery::Unstarted,
                };
                self.deliver(rows, hierarchy, stage)
            },
        }
    }

    fn deliver(
        &self,
        mut rows: Vec<AgentRow>,
        hierarchy: HierarchySnapshot,
        stage: ScanStage,
    ) -> Result<(), SendError<CensusUpdate>> {
        ReviewerPools::apply(&mut rows, &hierarchy.edges);
        startup_log::startup_local_answer(&rows);
        self.sender.send(CensusUpdate::Local {
            rows,
            hierarchy,
            stage,
        })
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
        Arc::new(InputDrivenHierarchyRunner::default()),
        Arc::new(SshRunner::new()),
        move |sender| {
            let mut schedule = ScanSchedule::NextRound;
            scanner.scan_staged(|stage, rows| {
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
        let (events, ready) = mpsc::channel();
        let publications = Arc::new(Mutex::new(LocalPublications {
            events: events.clone(),
            sender: sender.clone(),
            hierarchy,
            runner: hierarchy_runner,
            state: LocalHierarchy::default(),
            discovery: ScanDiscovery::default(),
            first_budget: HIERARCHY_FIRST_PUBLISH_BUDGET,
        }));
        let (scan, rounds) = mpsc::channel();
        let shared = Arc::clone(&publications);
        thread::spawn(move || {
            let mut answers = LocalAnswers {
                publications: shared,
            };
            for () in rounds {
                if events
                    .send(ScheduleEvent::Scanned(scan_local(&mut answers)))
                    .is_err()
                {
                    return;
                }
            }
        });
        let mut local_scan = LocalScanState::Ready;
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
            if now >= next_local && matches!(local_scan, LocalScanState::Ready) {
                if scan.send(()).is_err() {
                    return;
                }
                local_scan = LocalScanState::Running;
                next_local = now + cadence.local;
            }
            let deadline = match local_scan {
                LocalScanState::Ready => next_local.min(next_remote),
                LocalScanState::Running => next_remote,
            };
            match ready.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Ok(ScheduleEvent::HierarchyReady) => {
                    if publications
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .finish_pending(Duration::ZERO)
                        .is_err()
                    {
                        return;
                    }
                },
                Ok(ScheduleEvent::Scanned(ScanSchedule::Finished))
                | Err(RecvTimeoutError::Disconnected) => return,
                Ok(ScheduleEvent::Scanned(ScanSchedule::NextRound)) => {
                    local_scan = LocalScanState::Ready;
                },
                Err(RecvTimeoutError::Timeout) => {},
            }
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
        Arc::new(InputDrivenHierarchyRunner::default()),
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

/// Run a fixture scan with a short first-answer budget.
#[cfg(test)]
pub(super) fn spawn_with_test_source_and_budget(
    hierarchy: HierarchyCommand,
    hierarchy_runner: Arc<dyn HierarchyRunner>,
    budget: Duration,
    mut scan_local: impl FnMut(&mut LocalAnswers) + Send + 'static,
) -> Receiver<CensusUpdate> {
    spawn_with_test_source(hierarchy, hierarchy_runner, move |answers| {
        answers
            .publications
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .first_budget = budget;
        scan_local(answers);
    })
}

/// Exercise remote progress while a fixture holds local discovery or detail reads.
#[cfg(test)]
pub(super) fn spawn_with_test_remote_and_budget(
    remotes: RemoteMachines,
    hierarchy: HierarchyCommand,
    hierarchy_runner: Arc<dyn HierarchyRunner>,
    remote_runner: Arc<dyn RemoteRunner>,
    budget: Duration,
    mut scan_local: impl FnMut(&mut LocalAnswers) + Send + 'static,
) -> Receiver<CensusUpdate> {
    spawn_staged_with(
        remotes,
        hierarchy,
        hierarchy_runner,
        remote_runner,
        move |answers| {
            answers
                .publications
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .first_budget = budget;
            scan_local(answers);
            ScanSchedule::Finished
        },
        Cadence {
            local:  LOCAL_SCAN_INTERVAL,
            remote: REMOTE_PROBE_INTERVAL / 500,
        },
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
    use crate::census::RoleFallback;
    use crate::census::ServiceTier;
    use crate::census::SessionIdentity;
    use crate::census::hierarchy::HierarchyError;

    /// The host whose probe waits until the test lets it finish.
    const BLOCKED: &str = "blocked";
    /// The host whose probe answers at once.
    const QUICK: &str = "quick";
    /// How long the test waits for any one answer before failing.
    const PATIENCE: Duration = Duration::from_secs(5);
    /// Bound channel waits for a late hierarchy answer while its source stays open.
    const LATE_ANSWER_WAIT: Duration = Duration::from_millis(200);

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
                        reviewer: crate::census::hierarchy::ReviewerState::NotPool,
                        manager:  "boss".into(),
                        member:   "worker".into(),
                        role:     super::super::HierarchyRole::Unit,
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
                let start = Instant::now();
                for elapsed in [Duration::ZERO, Duration::from_secs(2)] {
                    answers
                        .publish_at(
                            ScanStage::Classified,
                            vec![session_row("worker", 1)],
                            start + elapsed,
                        )
                        .expect("classification publishes");
                    answers
                        .settle_test_request()
                        .expect("accept classification discovery");
                    answers
                        .publish_at(
                            ScanStage::Completed,
                            vec![session_row("worker", 1)],
                            start + elapsed,
                        )
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
                let start = Instant::now();
                answers
                    .publish_at(ScanStage::Classified, vec![session_row("worker", 1)], start)
                    .expect("classification publishes");
                answers
                    .settle_test_request()
                    .expect("accept classification discovery");
                answers
                    .publish_at(
                        ScanStage::Completed,
                        vec![session_row("worker", 1)],
                        start + HIERARCHY_REFRESH_INTERVAL,
                    )
                    .expect("expired discovery refreshes");
                answers
                    .settle_test_request()
                    .expect("accept expired discovery");
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
            role_fallback: RoleFallback::None,
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
                    reviewer: crate::census::hierarchy::ReviewerState::NotPool,
                    manager:  "remote-boss".to_string(),
                    member:   "worker".to_string(),
                    role:     super::super::HierarchyRole::Impl,
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

    /// Require the late failure while the fixture still keeps its scan source open.
    fn expect_failed_hierarchy_completion(updates: &Receiver<CensusUpdate>) {
        let deadline = Instant::now() + LATE_ANSWER_WAIT;
        loop {
            let update = updates
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .expect("late completion publishes without ending the source");
            if matches!(
                update,
                CensusUpdate::Local {
                    stage: ScanStage::Completed,
                    hierarchy: HierarchySnapshot {
                        notice: HierarchyNotice::Failed(_),
                        ..
                    },
                    ..
                }
            ) {
                break;
            }
        }
    }

    /// A pending hierarchy run cannot withhold completed row details already scanned.
    #[test]
    fn completed_rows_arrive_before_a_blocked_hierarchy_run_finishes() {
        fn retained_edges() -> HierarchyEdges {
            HierarchyEdges(vec![super::super::HierarchyEdge {
                reviewer: crate::census::hierarchy::ReviewerState::NotPool,
                manager:  "previous".into(),
                member:   "worker".into(),
                role:     super::super::HierarchyRole::Unit,
            }])
        }
        struct BlockedHierarchy {
            entered:  Sender<Duration>,
            release:  Mutex<Receiver<()>>,
            attempts: AtomicUsize,
        }
        impl HierarchyRunner for BlockedHierarchy {
            fn run(&self, _: &str, timeout: Duration) -> Result<HierarchyEdges, HierarchyError> {
                if self.attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                    return Ok(retained_edges());
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
        let (finish, open) = mpsc::channel();
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
                    .publish_at(
                        ScanStage::Completed,
                        rows.clone(),
                        Instant::now() + HIERARCHY_REFRESH_INTERVAL,
                    )
                    .expect("publish rows");
                open.recv()
                    .expect("keep detail source open for the late answer");
            },
        );
        let timeout = waiting
            .recv_timeout(LATE_ANSWER_WAIT)
            .expect("discovery starts");
        let initial = updates.recv_timeout(LATE_ANSWER_WAIT);
        let answer = updates.recv_timeout(LATE_ANSWER_WAIT);
        release
            .send(())
            .expect("release the command before assertions");
        assert_eq!(timeout, Duration::from_secs(10));
        assert_eq!(
            initial.expect("initial successful answer"),
            CensusUpdate::Local {
                rows:      vec![session_row("previous", 1)],
                hierarchy: retained_edges().into(),
                stage:     ScanStage::Completed,
            }
        );
        assert_eq!(
            answer.expect("rows precede hierarchy completion"),
            CensusUpdate::Local {
                rows:      expected,
                hierarchy: HierarchySnapshot {
                    edges:  retained_edges(),
                    notice: HierarchyNotice::Clear,
                },
                stage:     ScanStage::Classified,
            }
        );
        expect_failed_hierarchy_completion(&updates);
        finish.send(()).expect("release the source");
    }

    /// A slow first command cannot delay safe rows or expose a possible unit as top level.
    #[test]
    fn uncached_first_publication_respects_its_budget_and_holds_claude_rows() {
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
        let budget = HIERARCHY_FIRST_PUBLISH_BUDGET / 100;
        let deadline = budget + HIERARCHY_FIRST_PUBLISH_BUDGET / 5;
        let (finish, details) = mpsc::channel();
        let started = Instant::now();
        let updates = spawn_with_test_source_and_budget(
            HierarchyCommand::new(HierarchySource::Command("fixture".to_string())),
            Arc::new(GatedHierarchy {
                entered,
                release: Mutex::new(released),
            }),
            budget,
            move |answers| {
                answers
                    .publish(ScanStage::Completed, rows.clone())
                    .expect("publish completion");
                details
                    .recv()
                    .expect("keep the scan open for the late answer");
            },
        );
        waiting
            .recv_timeout(Duration::from_millis(200))
            .expect("discovery starts");
        let early = updates.recv_timeout(deadline);
        let elapsed = started.elapsed();
        eprintln!("bounded first publication: {} ms", elapsed.as_millis());
        release
            .send(())
            .expect("release discovery before assertions");
        assert!(elapsed < deadline, "first publication took {elapsed:?}");
        assert_eq!(
            early.expect("safe rows publish inside the startup budget"),
            CensusUpdate::Local {
                rows:      Vec::new(),
                hierarchy: HierarchySnapshot::default(),
                stage:     ScanStage::Completed,
            }
        );
        let late = updates.recv_timeout(deadline);
        finish.send(()).expect("release details before assertions");
        assert_eq!(
            late.expect("failed discovery answers during detail reads"),
            noted(expected, HierarchyNotice::Failed(HierarchyError::TimedOut))
        );
    }

    /// Closing a scan does not wait for an unresolved command or retain its update sender.
    #[test]
    fn finished_scheduler_closes_updates_without_waiting_for_pending_hierarchy() {
        struct HeldCommand(Mutex<Receiver<()>>);
        impl HierarchyRunner for HeldCommand {
            fn run(&self, _: &str, _: Duration) -> Result<HierarchyEdges, HierarchyError> {
                self.0
                    .lock()
                    .expect("release lock")
                    .recv()
                    .expect("release command");
                Ok(HierarchyEdges::default())
            }
        }
        let (release, held) = mpsc::channel();
        let budget = HIERARCHY_FIRST_PUBLISH_BUDGET / 100;
        let deadline = budget + HIERARCHY_FIRST_PUBLISH_BUDGET / 5;
        let updates = spawn_with_test_source_and_budget(
            HierarchyCommand::new(HierarchySource::Command("fixture".into())),
            Arc::new(HeldCommand(Mutex::new(held))),
            budget,
            |answers| {
                answers
                    .publish(ScanStage::Classified, vec![session_row("pending", 1)])
                    .expect("publish safe rows");
            },
        );
        let safe = updates.recv_timeout(deadline);
        let closed = updates.recv_timeout(deadline);
        release.send(()).expect("release command before assertions");
        assert!(safe.is_ok(), "safe startup rows publish");
        assert_eq!(closed, Err(RecvTimeoutError::Disconnected));
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
            reviewer: crate::census::hierarchy::ReviewerState::NotPool,
            manager:  "boss".to_string(),
            member:   "worker".to_string(),
            role:     super::super::HierarchyRole::Unit,
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
                reviewer: crate::census::hierarchy::ReviewerState::NotPool,
                manager:  format!("manager-{round}"),
                member:   "worker".to_string(),
                role:     super::super::HierarchyRole::Test,
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
                    .send(vec![session_row("worker", 1)])
                    .expect("first answer should send");
                sender
                    .send(vec![session_row("worker", 2)])
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
