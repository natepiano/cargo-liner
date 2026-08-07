use std::fmt::Debug;
use std::thread;
use std::thread::JoinHandle;
use std::time::Duration;
use std::time::Instant;

use super::ProcessObserver;
use super::snapshot::CompletedProcessRefreshExecution;
use super::snapshot::ProcessObservationSnapshot;
use super::snapshot::ProcessRefreshConsumerDemand;
use super::snapshot::ProcessRefreshExecutionFailure;
use super::snapshot::ProcessRefreshExecutionOutcome;
use crate::channel;
use crate::channel::Receiver;
use crate::channel::Sender;
use crate::channel::TryRecvError;

/// How a refresh consumer classifies the cycle the observer just produced.
///
/// This is the one point where observation hands a completed snapshot to code
/// that knows what a build is. Observation moves the consumer's own demand in
/// and its own outcome back out without naming either type, which is what keeps
/// `process_observation` free of any dependency on `build_monitor` while the
/// sole runtime classifier still lives beside the sole observer.
pub(crate) trait RefreshCycleClassifier: Send + 'static {
    /// Immutable consumer evidence attached to one refresh request.
    type CycleDemand: Send + 'static;
    /// What classifying one refresh cycle produced for that consumer.
    type CycleOutcome: Debug + Send + 'static;

    fn classify_refresh_cycle(
        &mut self,
        process_observer: &mut ProcessObserver,
        process_observation_snapshot: &ProcessObservationSnapshot,
        cycle_demand: Self::CycleDemand,
    ) -> Self::CycleOutcome;
}

/// The single mutable owner of everything one refresh cycle runs against.
///
/// Whichever backend executes the cycle holds exactly one of these, so the
/// observer and the classifier can never be reached from two places.
struct ProcessRefreshCycleOwner<C> {
    process_observer:         ProcessObserver,
    refresh_cycle_classifier: C,
}

impl<C> ProcessRefreshCycleOwner<C> {
    fn new(refresh_cycle_classifier: C) -> Self {
        Self {
            process_observer: ProcessObserver::default(),
            refresh_cycle_classifier,
        }
    }
}

/// The benchmark-selected location for observer work.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ProcessRefreshExecutionBackendSelection {
    Synchronous,
    DedicatedWorker,
}

/// Whether Running Targets contributes a one-second process deadline.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RunningTargetsRefreshSchedule {
    Every(Duration),
    Suppressed,
}

/// Whether compile monitoring contributes a process deadline.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CompileMonitorRefreshSchedule {
    At(Instant),
    NotScheduled,
}

/// Whether an active termination transaction contributes a process deadline.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TerminationTransactionRefreshSchedule {
    At(Instant),
    NotScheduled,
}

/// The next reason the event loop should wake for process work.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ProcessRefreshDeadline {
    At(Instant),
    AwaitingWorker,
    NotScheduled,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct ProcessRefreshRequestId(u64);

impl ProcessRefreshRequestId {
    const fn next(&mut self) -> Self {
        let current = *self;
        self.0 = self.0.wrapping_add(1);
        current
    }
}

struct ProcessRefreshPlan<CycleDemand> {
    demand:       ProcessRefreshConsumerDemand,
    cycle_demand: CycleDemand,
}

enum DueProcessRefreshDemand {
    Due(ProcessRefreshConsumerDemand),
    NotDue,
}

impl<CycleDemand> ProcessRefreshPlan<CycleDemand> {
    const fn new(demand: ProcessRefreshConsumerDemand, cycle_demand: CycleDemand) -> Self {
        Self {
            demand,
            cycle_demand,
        }
    }
}

struct ProcessRefreshWorkerRequest<CycleDemand> {
    request_id: ProcessRefreshRequestId,
    plan:       ProcessRefreshPlan<CycleDemand>,
}

enum ProcessRefreshWorkerCommand<CycleDemand> {
    Execute(ProcessRefreshWorkerRequest<CycleDemand>),
    Shutdown,
}

/// One correlated observer execution result.
#[derive(Debug, PartialEq)]
pub(crate) struct ProcessRefreshExecution<CycleOutcome> {
    request_id: ProcessRefreshRequestId,
    demand:     ProcessRefreshConsumerDemand,
    outcome:    ProcessRefreshExecutionOutcome<CycleOutcome>,
}

impl<CycleOutcome> ProcessRefreshExecution<CycleOutcome> {
    pub(crate) const fn demand(&self) -> ProcessRefreshConsumerDemand { self.demand }

    pub(crate) fn into_outcome(self) -> ProcessRefreshExecutionOutcome<CycleOutcome> {
        self.outcome
    }

    const fn failed(
        request_id: ProcessRefreshRequestId,
        demand: ProcessRefreshConsumerDemand,
        failure: ProcessRefreshExecutionFailure,
    ) -> Self {
        Self {
            request_id,
            demand,
            outcome: ProcessRefreshExecutionOutcome::Failed(failure),
        }
    }

    #[cfg(test)]
    pub(crate) const fn failed_for_test(
        demand: ProcessRefreshConsumerDemand,
        failure: ProcessRefreshExecutionFailure,
    ) -> Self {
        Self::failed(ProcessRefreshRequestId(0), demand, failure)
    }

    /// A correlated completed execution over an empty observation, so a
    /// consumer's reconciliation can be tested against any cycle outcome.
    #[cfg(test)]
    pub(crate) fn completed_for_test(
        demand: ProcessRefreshConsumerDemand,
        cycle_outcome: CycleOutcome,
    ) -> Self {
        Self {
            request_id: ProcessRefreshRequestId(0),
            demand,
            outcome: ProcessRefreshExecutionOutcome::Completed(Box::new(
                CompletedProcessRefreshExecution::new(
                    crate::process_observation::snapshot::ProcessObservationSnapshot::empty_for_test(
                    ),
                    Duration::ZERO,
                    cycle_outcome,
                ),
            )),
        }
    }
}

struct DedicatedProcessRefreshWorker<C: RefreshCycleClassifier> {
    command_sender:  Sender<ProcessRefreshWorkerCommand<C::CycleDemand>>,
    result_receiver: Receiver<Box<ProcessRefreshExecution<C::CycleOutcome>>>,
    thread_state:    ProcessRefreshWorkerThreadState,
}

enum ProcessRefreshWorkerThreadState {
    Running(JoinHandle<()>),
    Joined,
}

impl<C: RefreshCycleClassifier> DedicatedProcessRefreshWorker<C> {
    fn spawn(refresh_cycle_classifier: C) -> Self {
        let (command_sender, command_receiver) = channel::unbounded();
        let (result_sender, result_receiver) = channel::unbounded();
        let join_handle = thread::spawn(move || {
            let mut process_refresh_cycle_owner =
                ProcessRefreshCycleOwner::new(refresh_cycle_classifier);
            process_refresh_worker(
                &mut process_refresh_cycle_owner,
                &command_receiver,
                &result_sender,
            );
        });
        Self {
            command_sender,
            result_receiver,
            thread_state: ProcessRefreshWorkerThreadState::Running(join_handle),
        }
    }

    fn dispatch(
        &self,
        process_refresh_worker_request: ProcessRefreshWorkerRequest<C::CycleDemand>,
    ) -> Result<(), ProcessRefreshExecutionFailure> {
        self.command_sender
            .send(ProcessRefreshWorkerCommand::Execute(
                process_refresh_worker_request,
            ))
            .map_err(|_| ProcessRefreshExecutionFailure::RequestChannelDisconnected)
    }

    fn poll(&self) -> ProcessRefreshWorkerResultPoll<C::CycleOutcome> {
        match self.result_receiver.try_recv() {
            Ok(process_refresh_execution) => {
                ProcessRefreshWorkerResultPoll::Received(process_refresh_execution)
            },
            Err(TryRecvError::Empty) => ProcessRefreshWorkerResultPoll::Pending,
            Err(TryRecvError::Disconnected) => ProcessRefreshWorkerResultPoll::Disconnected,
        }
    }

    const fn result_receiver(&self) -> &Receiver<Box<ProcessRefreshExecution<C::CycleOutcome>>> {
        &self.result_receiver
    }
}

impl<C: RefreshCycleClassifier> Drop for DedicatedProcessRefreshWorker<C> {
    fn drop(&mut self) {
        let _ = self
            .command_sender
            .send(ProcessRefreshWorkerCommand::Shutdown);
        let thread_state = std::mem::replace(
            &mut self.thread_state,
            ProcessRefreshWorkerThreadState::Joined,
        );
        if let ProcessRefreshWorkerThreadState::Running(join_handle) = thread_state {
            let _ = join_handle.join();
        }
    }
}

enum ProcessRefreshExecutionBackend<C: RefreshCycleClassifier> {
    Synchronous(Box<ProcessRefreshCycleOwner<C>>),
    DedicatedWorker(DedicatedProcessRefreshWorker<C>),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ProcessRefreshInFlight {
    Idle,
    Awaiting {
        request_id: ProcessRefreshRequestId,
        demand:     ProcessRefreshConsumerDemand,
    },
}

/// Result of asking the executor to perform due work.
#[derive(Debug, PartialEq)]
pub(crate) enum ProcessRefreshDispatchOutcome<CycleOutcome> {
    NotDue,
    AwaitingWorker(ProcessRefreshRequestId),
    Finished(Box<ProcessRefreshExecution<CycleOutcome>>),
}

/// Nonblocking state of the dedicated worker result channel.
#[derive(Debug, PartialEq)]
pub(crate) enum ProcessRefreshResultPoll<CycleOutcome> {
    Pending,
    Ready(Box<ProcessRefreshExecution<CycleOutcome>>),
}

enum ProcessRefreshWorkerResultPoll<CycleOutcome> {
    Pending,
    Received(Box<ProcessRefreshExecution<CycleOutcome>>),
    Disconnected,
}

/// App-owned scheduler and execution backend for the sole `ProcessObserver`.
pub(crate) struct ProcessRefreshExecutor<C: RefreshCycleClassifier> {
    backend:                          ProcessRefreshExecutionBackend<C>,
    running_targets_schedule:         RunningTargetsRefreshSchedule,
    running_targets_deadline:         ProcessRefreshDeadline,
    compile_monitor_schedule:         CompileMonitorRefreshSchedule,
    termination_transaction_schedule: TerminationTransactionRefreshSchedule,
    in_flight:                        ProcessRefreshInFlight,
    next_request_id:                  ProcessRefreshRequestId,
}

impl<C: RefreshCycleClassifier> ProcessRefreshExecutor<C> {
    pub(crate) fn new(
        backend_selection: ProcessRefreshExecutionBackendSelection,
        refresh_cycle_classifier: C,
        running_targets_schedule: RunningTargetsRefreshSchedule,
        compile_monitor_schedule: CompileMonitorRefreshSchedule,
        started_at: Instant,
    ) -> Self {
        let backend = match backend_selection {
            ProcessRefreshExecutionBackendSelection::Synchronous => {
                ProcessRefreshExecutionBackend::Synchronous(Box::new(
                    ProcessRefreshCycleOwner::new(refresh_cycle_classifier),
                ))
            },
            ProcessRefreshExecutionBackendSelection::DedicatedWorker => {
                ProcessRefreshExecutionBackend::DedicatedWorker(
                    DedicatedProcessRefreshWorker::spawn(refresh_cycle_classifier),
                )
            },
        };
        let running_targets_deadline = match running_targets_schedule {
            RunningTargetsRefreshSchedule::Every(_) => ProcessRefreshDeadline::At(started_at),
            RunningTargetsRefreshSchedule::Suppressed => ProcessRefreshDeadline::NotScheduled,
        };
        Self {
            backend,
            running_targets_schedule,
            running_targets_deadline,
            compile_monitor_schedule,
            termination_transaction_schedule: TerminationTransactionRefreshSchedule::NotScheduled,
            in_flight: ProcessRefreshInFlight::Idle,
            next_request_id: ProcessRefreshRequestId(0),
        }
    }

    /// Hand the executor the compile monitor's current dispatch deadline. The
    /// monitor owns the scheduling intent; this deadline is derived from it and
    /// is never read back as truth.
    pub(crate) const fn rearm_compile_monitor(
        &mut self,
        compile_monitor_schedule: CompileMonitorRefreshSchedule,
    ) {
        self.compile_monitor_schedule = compile_monitor_schedule;
    }

    /// Hand the executor the active transaction's next observation deadline.
    pub(crate) const fn rearm_termination_transaction(
        &mut self,
        termination_transaction_refresh_schedule: TerminationTransactionRefreshSchedule,
    ) {
        self.termination_transaction_schedule = termination_transaction_refresh_schedule;
    }

    pub(crate) fn next_deadline(&self) -> ProcessRefreshDeadline {
        if matches!(self.in_flight, ProcessRefreshInFlight::Awaiting { .. }) {
            return ProcessRefreshDeadline::AwaitingWorker;
        }
        minimum_deadline(
            minimum_deadline(
                self.running_targets_deadline,
                match self.compile_monitor_schedule {
                    CompileMonitorRefreshSchedule::At(deadline) => {
                        ProcessRefreshDeadline::At(deadline)
                    },
                    CompileMonitorRefreshSchedule::NotScheduled => {
                        ProcessRefreshDeadline::NotScheduled
                    },
                },
            ),
            match self.termination_transaction_schedule {
                TerminationTransactionRefreshSchedule::At(deadline) => {
                    ProcessRefreshDeadline::At(deadline)
                },
                TerminationTransactionRefreshSchedule::NotScheduled => {
                    ProcessRefreshDeadline::NotScheduled
                },
            },
        )
    }

    /// Dispatch a due cycle, asking the consumer for its per-cycle demand only
    /// once a cycle is actually going out. A tick that finds nothing due costs
    /// no demand construction.
    pub(crate) fn refresh_due(
        &mut self,
        now: Instant,
        cycle_demand: impl FnOnce(ProcessRefreshConsumerDemand) -> C::CycleDemand,
    ) -> ProcessRefreshDispatchOutcome<C::CycleOutcome> {
        if matches!(self.in_flight, ProcessRefreshInFlight::Awaiting { .. }) {
            return ProcessRefreshDispatchOutcome::NotDue;
        }
        let demand = match self.due_demand(now) {
            DueProcessRefreshDemand::Due(demand) => demand,
            DueProcessRefreshDemand::NotDue => return ProcessRefreshDispatchOutcome::NotDue,
        };
        let request_id = self.next_request_id.next();
        self.advance_dispatched_deadlines(now, demand);
        let plan = ProcessRefreshPlan::new(demand, cycle_demand(demand));
        match &mut self.backend {
            ProcessRefreshExecutionBackend::Synchronous(process_refresh_cycle_owner) => {
                ProcessRefreshDispatchOutcome::Finished(Box::new(execute_refresh(
                    process_refresh_cycle_owner,
                    request_id,
                    plan,
                )))
            },
            ProcessRefreshExecutionBackend::DedicatedWorker(worker) => {
                let process_refresh_worker_request =
                    ProcessRefreshWorkerRequest { request_id, plan };
                match worker.dispatch(process_refresh_worker_request) {
                    Ok(()) => {
                        self.in_flight = ProcessRefreshInFlight::Awaiting { request_id, demand };
                        ProcessRefreshDispatchOutcome::AwaitingWorker(request_id)
                    },
                    Err(failure) => ProcessRefreshDispatchOutcome::Finished(Box::new(
                        ProcessRefreshExecution::failed(request_id, demand, failure),
                    )),
                }
            },
        }
    }

    pub(crate) fn poll_result(&mut self) -> ProcessRefreshResultPoll<C::CycleOutcome> {
        if matches!(self.in_flight, ProcessRefreshInFlight::Idle) {
            return ProcessRefreshResultPoll::Pending;
        }
        let worker_poll = match &self.backend {
            ProcessRefreshExecutionBackend::Synchronous(_) => {
                ProcessRefreshWorkerResultPoll::Disconnected
            },
            ProcessRefreshExecutionBackend::DedicatedWorker(worker) => worker.poll(),
        };
        self.handle_worker_result_poll(worker_poll)
    }

    fn handle_worker_result_poll(
        &mut self,
        worker_poll: ProcessRefreshWorkerResultPoll<C::CycleOutcome>,
    ) -> ProcessRefreshResultPoll<C::CycleOutcome> {
        let ProcessRefreshInFlight::Awaiting { request_id, demand } = self.in_flight else {
            return ProcessRefreshResultPoll::Pending;
        };
        match worker_poll {
            ProcessRefreshWorkerResultPoll::Received(process_refresh_execution)
                if process_refresh_execution.request_id == request_id
                    && process_refresh_execution.demand == demand =>
            {
                self.in_flight = ProcessRefreshInFlight::Idle;
                ProcessRefreshResultPoll::Ready(process_refresh_execution)
            },
            ProcessRefreshWorkerResultPoll::Pending
            | ProcessRefreshWorkerResultPoll::Received(_) => ProcessRefreshResultPoll::Pending,
            ProcessRefreshWorkerResultPoll::Disconnected => {
                self.in_flight = ProcessRefreshInFlight::Idle;
                ProcessRefreshResultPoll::Ready(Box::new(ProcessRefreshExecution::failed(
                    request_id,
                    demand,
                    ProcessRefreshExecutionFailure::ResultChannelDisconnected,
                )))
            },
        }
    }

    pub(crate) const fn result_receiver(
        &self,
    ) -> ProcessRefreshResultReceiver<'_, C::CycleOutcome> {
        match (&self.backend, self.in_flight) {
            (
                ProcessRefreshExecutionBackend::DedicatedWorker(worker),
                ProcessRefreshInFlight::Awaiting { .. },
            ) => ProcessRefreshResultReceiver::DedicatedWorker(worker.result_receiver()),
            (ProcessRefreshExecutionBackend::Synchronous(_), _)
            | (ProcessRefreshExecutionBackend::DedicatedWorker(_), ProcessRefreshInFlight::Idle) => {
                ProcessRefreshResultReceiver::NoWorkerResultExpected
            },
        }
    }

    fn due_demand(&self, now: Instant) -> DueProcessRefreshDemand {
        let running_targets_due = matches!(
            self.running_targets_deadline,
            ProcessRefreshDeadline::At(deadline) if deadline <= now
        );
        let compile_monitor_due = match self.compile_monitor_schedule {
            CompileMonitorRefreshSchedule::At(deadline) => deadline <= now,
            CompileMonitorRefreshSchedule::NotScheduled => false,
        };
        let termination_transaction_due = match self.termination_transaction_schedule {
            TerminationTransactionRefreshSchedule::At(deadline) => deadline <= now,
            TerminationTransactionRefreshSchedule::NotScheduled => false,
        };
        match (
            running_targets_due,
            compile_monitor_due,
            termination_transaction_due,
        ) {
            (true, true, true) => {
                DueProcessRefreshDemand::Due(ProcessRefreshConsumerDemand::AllConsumers)
            },
            (true, true, false) => DueProcessRefreshDemand::Due(
                ProcessRefreshConsumerDemand::RunningTargetsAndCompileMonitor,
            ),
            (true, false, true) => DueProcessRefreshDemand::Due(
                ProcessRefreshConsumerDemand::RunningTargetsAndTerminationTransaction,
            ),
            (false, true, true) => DueProcessRefreshDemand::Due(
                ProcessRefreshConsumerDemand::CompileMonitorAndTerminationTransaction,
            ),
            (true, false, false) => {
                DueProcessRefreshDemand::Due(ProcessRefreshConsumerDemand::RunningTargets)
            },
            (false, true, false) => {
                DueProcessRefreshDemand::Due(ProcessRefreshConsumerDemand::CompileMonitor)
            },
            (false, false, true) => {
                DueProcessRefreshDemand::Due(ProcessRefreshConsumerDemand::TerminationTransaction)
            },
            (false, false, false) => DueProcessRefreshDemand::NotDue,
        }
    }

    fn advance_dispatched_deadlines(&mut self, now: Instant, demand: ProcessRefreshConsumerDemand) {
        if demand.includes_running_targets() {
            self.running_targets_deadline = match self.running_targets_schedule {
                RunningTargetsRefreshSchedule::Every(interval) => {
                    ProcessRefreshDeadline::At(now + interval)
                },
                RunningTargetsRefreshSchedule::Suppressed => ProcessRefreshDeadline::NotScheduled,
            };
        }
        if demand.includes_compile_monitor() {
            self.compile_monitor_schedule = CompileMonitorRefreshSchedule::NotScheduled;
        }
        if demand.includes_termination_transaction() {
            self.termination_transaction_schedule =
                TerminationTransactionRefreshSchedule::NotScheduled;
        }
    }
}

/// Borrowed worker receiver used only to register event-loop wakeups.
pub(crate) enum ProcessRefreshResultReceiver<'a, CycleOutcome> {
    NoWorkerResultExpected,
    DedicatedWorker(&'a Receiver<Box<ProcessRefreshExecution<CycleOutcome>>>),
}

fn minimum_deadline(
    left_deadline: ProcessRefreshDeadline,
    right_deadline: ProcessRefreshDeadline,
) -> ProcessRefreshDeadline {
    match (left_deadline, right_deadline) {
        (ProcessRefreshDeadline::At(left), ProcessRefreshDeadline::At(right)) => {
            ProcessRefreshDeadline::At(left.min(right))
        },
        (ProcessRefreshDeadline::At(deadline), ProcessRefreshDeadline::NotScheduled)
        | (ProcessRefreshDeadline::NotScheduled, ProcessRefreshDeadline::At(deadline)) => {
            ProcessRefreshDeadline::At(deadline)
        },
        (ProcessRefreshDeadline::AwaitingWorker, _)
        | (_, ProcessRefreshDeadline::AwaitingWorker) => ProcessRefreshDeadline::AwaitingWorker,
        (ProcessRefreshDeadline::NotScheduled, ProcessRefreshDeadline::NotScheduled) => {
            ProcessRefreshDeadline::NotScheduled
        },
    }
}

fn process_refresh_worker<C: RefreshCycleClassifier>(
    process_refresh_cycle_owner: &mut ProcessRefreshCycleOwner<C>,
    command_receiver: &Receiver<ProcessRefreshWorkerCommand<C::CycleDemand>>,
    result_sender: &Sender<Box<ProcessRefreshExecution<C::CycleOutcome>>>,
) {
    while let Ok(command) = command_receiver.recv() {
        match command {
            ProcessRefreshWorkerCommand::Execute(process_refresh_worker_request) => {
                let process_refresh_execution = execute_refresh(
                    process_refresh_cycle_owner,
                    process_refresh_worker_request.request_id,
                    process_refresh_worker_request.plan,
                );
                if result_sender
                    .send(Box::new(process_refresh_execution))
                    .is_err()
                {
                    break;
                }
            },
            ProcessRefreshWorkerCommand::Shutdown => break,
        }
    }
}

/// Observe, then classify. The observer timing stops before classification so
/// the recorded duration stays the observer-only boundary Phase 3 established.
fn execute_refresh<C: RefreshCycleClassifier>(
    process_refresh_cycle_owner: &mut ProcessRefreshCycleOwner<C>,
    request_id: ProcessRefreshRequestId,
    plan: ProcessRefreshPlan<C::CycleDemand>,
) -> ProcessRefreshExecution<C::CycleOutcome> {
    let started = Instant::now();
    let process_observation_snapshot = process_refresh_cycle_owner
        .process_observer
        .refresh_for_consumer_demand(plan.demand);
    let elapsed = started.elapsed();
    let cycle_outcome = process_refresh_cycle_owner
        .refresh_cycle_classifier
        .classify_refresh_cycle(
            &mut process_refresh_cycle_owner.process_observer,
            &process_observation_snapshot,
            plan.cycle_demand,
        );
    ProcessRefreshExecution {
        request_id,
        demand: plan.demand,
        outcome: ProcessRefreshExecutionOutcome::Completed(Box::new(
            CompletedProcessRefreshExecution::new(
                process_observation_snapshot,
                elapsed,
                cycle_outcome,
            ),
        )),
    }
}

#[cfg(test)]
#[allow(
    clippy::panic,
    reason = "tests should fail on unexpected executor states"
)]
mod tests {
    use super::*;
    use crate::process_observation::snapshot::ProcessObservationSnapshot;

    /// The demand and the outcome of a cycle nothing classified.
    ///
    /// These tests cover the executor's own scheduling, correlation and backend
    /// behavior, all of which must hold whatever a consumer classifies.
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum UnclassifiedRefreshCycle {
        NotClassified,
    }

    /// Classifier that leaves every cycle unclassified.
    #[derive(Debug)]
    struct NoRefreshCycleClassification;

    impl RefreshCycleClassifier for NoRefreshCycleClassification {
        type CycleDemand = UnclassifiedRefreshCycle;
        type CycleOutcome = UnclassifiedRefreshCycle;

        fn classify_refresh_cycle(
            &mut self,
            _: &mut ProcessObserver,
            _process_observation_snapshot: &ProcessObservationSnapshot,
            _cycle_demand: Self::CycleDemand,
        ) -> Self::CycleOutcome {
            UnclassifiedRefreshCycle::NotClassified
        }
    }

    /// Classifier that reports the thread the cycle was classified on, so the
    /// worker's sole ownership of classification is observable.
    #[derive(Debug)]
    struct ClassifyingThreadRecord;

    impl RefreshCycleClassifier for ClassifyingThreadRecord {
        type CycleDemand = UnclassifiedRefreshCycle;
        type CycleOutcome = thread::ThreadId;

        fn classify_refresh_cycle(
            &mut self,
            _: &mut ProcessObserver,
            _process_observation_snapshot: &ProcessObservationSnapshot,
            _cycle_demand: Self::CycleDemand,
        ) -> Self::CycleOutcome {
            thread::current().id()
        }
    }

    fn unclassified_cycle(_demand: ProcessRefreshConsumerDemand) -> UnclassifiedRefreshCycle {
        UnclassifiedRefreshCycle::NotClassified
    }

    fn executor_awaiting(
        request_id: ProcessRefreshRequestId,
        demand: ProcessRefreshConsumerDemand,
    ) -> ProcessRefreshExecutor<NoRefreshCycleClassification> {
        let mut process_refresh_executor = ProcessRefreshExecutor::new(
            ProcessRefreshExecutionBackendSelection::Synchronous,
            NoRefreshCycleClassification,
            RunningTargetsRefreshSchedule::Suppressed,
            CompileMonitorRefreshSchedule::NotScheduled,
            Instant::now(),
        );
        process_refresh_executor.in_flight =
            ProcessRefreshInFlight::Awaiting { request_id, demand };
        process_refresh_executor
    }

    fn received_completed_execution(
        request_id: ProcessRefreshRequestId,
        demand: ProcessRefreshConsumerDemand,
    ) -> ProcessRefreshWorkerResultPoll<UnclassifiedRefreshCycle> {
        ProcessRefreshWorkerResultPoll::Received(Box::new(ProcessRefreshExecution {
            request_id,
            demand,
            outcome: ProcessRefreshExecutionOutcome::Completed(Box::new(
                CompletedProcessRefreshExecution::new(
                    ProcessObservationSnapshot::empty_for_test(),
                    Duration::ZERO,
                    UnclassifiedRefreshCycle::NotClassified,
                ),
            )),
        }))
    }

    fn disconnected_worker() -> DedicatedProcessRefreshWorker<NoRefreshCycleClassification> {
        let (command_sender, command_receiver) = channel::unbounded();
        let (result_sender, result_receiver) = channel::unbounded();
        drop(command_receiver);
        drop(result_sender);
        DedicatedProcessRefreshWorker {
            command_sender,
            result_receiver,
            thread_state: ProcessRefreshWorkerThreadState::Joined,
        }
    }

    #[test]
    fn simultaneous_consumers_produce_one_coalesced_execution() {
        let now = Instant::now();
        let mut process_refresh_executor = ProcessRefreshExecutor::new(
            ProcessRefreshExecutionBackendSelection::Synchronous,
            NoRefreshCycleClassification,
            RunningTargetsRefreshSchedule::Every(Duration::from_secs(1)),
            CompileMonitorRefreshSchedule::At(now),
            now,
        );

        let ProcessRefreshDispatchOutcome::Finished(process_refresh_execution) =
            process_refresh_executor.refresh_due(now, unclassified_cycle)
        else {
            panic!("coalesced synchronous refresh should complete");
        };

        assert_eq!(
            process_refresh_execution.demand(),
            ProcessRefreshConsumerDemand::RunningTargetsAndCompileMonitor
        );
        assert!(matches!(
            process_refresh_execution.into_outcome(),
            ProcessRefreshExecutionOutcome::Completed(_)
        ));
        assert_eq!(
            process_refresh_executor.refresh_due(now, unclassified_cycle),
            ProcessRefreshDispatchOutcome::NotDue
        );
    }

    #[test]
    fn transaction_demand_uses_the_sole_executor_and_a_full_system_snapshot() {
        let now = Instant::now();
        let mut process_refresh_executor = ProcessRefreshExecutor::new(
            ProcessRefreshExecutionBackendSelection::Synchronous,
            NoRefreshCycleClassification,
            RunningTargetsRefreshSchedule::Suppressed,
            CompileMonitorRefreshSchedule::NotScheduled,
            now,
        );
        process_refresh_executor
            .rearm_termination_transaction(TerminationTransactionRefreshSchedule::At(now));

        let ProcessRefreshDispatchOutcome::Finished(process_refresh_execution) =
            process_refresh_executor.refresh_due(now, unclassified_cycle)
        else {
            panic!("transaction refresh should finish synchronously");
        };
        assert_eq!(
            process_refresh_execution.demand(),
            ProcessRefreshConsumerDemand::TerminationTransaction
        );
        let ProcessRefreshExecutionOutcome::Completed(completed_execution) =
            process_refresh_execution.into_outcome()
        else {
            panic!("transaction refresh should complete successfully");
        };
        assert_eq!(
            completed_execution.snapshot().scope(),
            &crate::process_observation::snapshot::ProcessSnapshotScope::FullSystem
        );
        assert!(matches!(
            completed_execution.snapshot().running_process_metrics(),
            crate::process_observation::snapshot::RunningProcessMetricsObservation::NotRequested
        ));
        assert_eq!(
            process_refresh_executor.next_deadline(),
            ProcessRefreshDeadline::NotScheduled
        );
    }

    #[test]
    fn no_compile_deadline_exists_when_monitoring_is_not_scheduled() {
        let now = Instant::now();
        let process_refresh_executor = ProcessRefreshExecutor::new(
            ProcessRefreshExecutionBackendSelection::Synchronous,
            NoRefreshCycleClassification,
            RunningTargetsRefreshSchedule::Suppressed,
            CompileMonitorRefreshSchedule::NotScheduled,
            now,
        );

        assert_eq!(
            process_refresh_executor.next_deadline(),
            ProcessRefreshDeadline::NotScheduled
        );
    }

    #[test]
    fn synchronous_completion_contains_successful_execution_timing() {
        let now = Instant::now();
        let mut process_refresh_executor = ProcessRefreshExecutor::new(
            ProcessRefreshExecutionBackendSelection::Synchronous,
            NoRefreshCycleClassification,
            RunningTargetsRefreshSchedule::Every(Duration::from_secs(1)),
            CompileMonitorRefreshSchedule::NotScheduled,
            now,
        );

        let ProcessRefreshDispatchOutcome::Finished(process_refresh_execution) =
            process_refresh_executor.refresh_due(now, unclassified_cycle)
        else {
            panic!("synchronous refresh should finish in the dispatch call");
        };
        let ProcessRefreshExecutionOutcome::Completed(completed_process_refresh_execution) =
            process_refresh_execution.into_outcome()
        else {
            panic!("synchronous refresh should complete successfully");
        };

        assert!(
            completed_process_refresh_execution.elapsed() <= Duration::from_secs(5),
            "completed synchronous timing should describe the bounded execution"
        );
    }

    #[test]
    fn completed_empty_snapshot_is_not_a_failed_execution() {
        let completed_empty = ProcessRefreshExecutionOutcome::Completed(Box::new(
            CompletedProcessRefreshExecution::new(
                ProcessObservationSnapshot::empty_for_test(),
                Duration::ZERO,
                UnclassifiedRefreshCycle::NotClassified,
            ),
        ));
        let failed: ProcessRefreshExecutionOutcome<UnclassifiedRefreshCycle> =
            ProcessRefreshExecutionOutcome::Failed(
                ProcessRefreshExecutionFailure::ResultChannelDisconnected,
            );

        assert!(matches!(
            completed_empty,
            ProcessRefreshExecutionOutcome::Completed(completed_process_refresh_execution)
                if completed_process_refresh_execution
                    .snapshot()
                    .strongly_identified_processes()
                    .is_empty()
        ));
        assert!(matches!(failed, ProcessRefreshExecutionOutcome::Failed(_)));
    }

    #[test]
    fn failure_outcome_retains_its_correlated_request() {
        let process_refresh_execution: ProcessRefreshExecution<UnclassifiedRefreshCycle> =
            ProcessRefreshExecution::failed(
                ProcessRefreshRequestId(7),
                ProcessRefreshConsumerDemand::RunningTargets,
                ProcessRefreshExecutionFailure::RequestChannelDisconnected,
            );

        assert_eq!(
            process_refresh_execution.request_id,
            ProcessRefreshRequestId(7)
        );
        assert_eq!(
            process_refresh_execution.into_outcome(),
            ProcessRefreshExecutionOutcome::Failed(
                ProcessRefreshExecutionFailure::RequestChannelDisconnected
            )
        );
    }

    #[test]
    fn dedicated_worker_completion_contains_successful_execution_timing() {
        let now = Instant::now();
        let mut process_refresh_executor = ProcessRefreshExecutor::new(
            ProcessRefreshExecutionBackendSelection::DedicatedWorker,
            NoRefreshCycleClassification,
            RunningTargetsRefreshSchedule::Every(Duration::from_secs(1)),
            CompileMonitorRefreshSchedule::NotScheduled,
            now,
        );
        assert!(matches!(
            process_refresh_executor.refresh_due(now, unclassified_cycle),
            ProcessRefreshDispatchOutcome::AwaitingWorker(_)
        ));
        let deadline = Instant::now() + Duration::from_secs(5);

        let completed_process_refresh_execution = loop {
            match process_refresh_executor.poll_result() {
                ProcessRefreshResultPoll::Ready(process_refresh_execution) => {
                    let ProcessRefreshExecutionOutcome::Completed(
                        completed_process_refresh_execution,
                    ) = process_refresh_execution.into_outcome()
                    else {
                        panic!("worker refresh should complete successfully");
                    };
                    break completed_process_refresh_execution;
                },
                ProcessRefreshResultPoll::Pending if Instant::now() < deadline => {
                    std::thread::yield_now();
                },
                ProcessRefreshResultPoll::Pending => {
                    panic!("worker refresh should finish before the test deadline");
                },
            }
        };

        assert!(
            completed_process_refresh_execution.elapsed() <= Duration::from_secs(5),
            "completed worker timing should describe the bounded execution"
        );
    }

    #[test]
    fn a_dedicated_worker_cycle_classifies_off_the_dispatching_thread() {
        let now = Instant::now();
        let mut process_refresh_executor = ProcessRefreshExecutor::new(
            ProcessRefreshExecutionBackendSelection::DedicatedWorker,
            ClassifyingThreadRecord,
            RunningTargetsRefreshSchedule::Every(Duration::from_secs(1)),
            CompileMonitorRefreshSchedule::NotScheduled,
            now,
        );
        assert!(matches!(
            process_refresh_executor.refresh_due(now, unclassified_cycle),
            ProcessRefreshDispatchOutcome::AwaitingWorker(_)
        ));
        let deadline = Instant::now() + Duration::from_secs(5);

        let classifying_thread = loop {
            match process_refresh_executor.poll_result() {
                ProcessRefreshResultPoll::Ready(process_refresh_execution) => {
                    let ProcessRefreshExecutionOutcome::Completed(
                        completed_process_refresh_execution,
                    ) = process_refresh_execution.into_outcome()
                    else {
                        panic!("worker refresh should complete successfully");
                    };
                    break completed_process_refresh_execution.into_parts().1;
                },
                ProcessRefreshResultPoll::Pending if Instant::now() < deadline => {
                    thread::yield_now();
                },
                ProcessRefreshResultPoll::Pending => {
                    panic!("worker refresh should finish before the test deadline");
                },
            }
        };

        assert_ne!(classifying_thread, thread::current().id());
    }

    #[test]
    fn request_channel_failure_has_no_completed_execution_timing() {
        let now = Instant::now();
        let mut process_refresh_executor = ProcessRefreshExecutor::new(
            ProcessRefreshExecutionBackendSelection::Synchronous,
            NoRefreshCycleClassification,
            RunningTargetsRefreshSchedule::Every(Duration::from_secs(1)),
            CompileMonitorRefreshSchedule::NotScheduled,
            now,
        );
        process_refresh_executor.backend =
            ProcessRefreshExecutionBackend::DedicatedWorker(disconnected_worker());

        let ProcessRefreshDispatchOutcome::Finished(process_refresh_execution) =
            process_refresh_executor.refresh_due(now, unclassified_cycle)
        else {
            panic!("disconnected request channel should return a failed execution");
        };

        assert_eq!(
            process_refresh_execution.into_outcome(),
            ProcessRefreshExecutionOutcome::Failed(
                ProcessRefreshExecutionFailure::RequestChannelDisconnected
            )
        );
    }

    #[test]
    fn result_channel_failure_has_no_completed_execution_timing() {
        let request_id = ProcessRefreshRequestId(17);
        let demand = ProcessRefreshConsumerDemand::RunningTargets;
        let mut process_refresh_executor = executor_awaiting(request_id, demand);
        process_refresh_executor.backend =
            ProcessRefreshExecutionBackend::DedicatedWorker(disconnected_worker());

        let ProcessRefreshResultPoll::Ready(process_refresh_execution) =
            process_refresh_executor.poll_result()
        else {
            panic!("disconnected result channel should return a failed execution");
        };

        assert_eq!(
            process_refresh_execution.into_outcome(),
            ProcessRefreshExecutionOutcome::Failed(
                ProcessRefreshExecutionFailure::ResultChannelDisconnected
            )
        );
    }

    #[test]
    fn stale_request_result_keeps_current_request_active_until_its_result_arrives() {
        let current_request_id = ProcessRefreshRequestId(7);
        let demand = ProcessRefreshConsumerDemand::RunningTargets;
        let mut process_refresh_executor = executor_awaiting(current_request_id, demand);

        assert_eq!(
            process_refresh_executor.handle_worker_result_poll(received_completed_execution(
                ProcessRefreshRequestId(6),
                demand
            ),),
            ProcessRefreshResultPoll::Pending
        );
        assert_eq!(
            process_refresh_executor.in_flight,
            ProcessRefreshInFlight::Awaiting {
                request_id: current_request_id,
                demand,
            }
        );

        let ProcessRefreshResultPoll::Ready(process_refresh_execution) = process_refresh_executor
            .handle_worker_result_poll(received_completed_execution(current_request_id, demand))
        else {
            panic!("matching request result should complete the current request");
        };
        assert_eq!(process_refresh_execution.request_id, current_request_id);
        assert_eq!(
            process_refresh_executor.in_flight,
            ProcessRefreshInFlight::Idle
        );
    }

    #[test]
    fn mismatched_demand_result_keeps_current_request_active_until_its_result_arrives() {
        let request_id = ProcessRefreshRequestId(7);
        let current_demand = ProcessRefreshConsumerDemand::RunningTargets;
        let mut process_refresh_executor = executor_awaiting(request_id, current_demand);

        assert_eq!(
            process_refresh_executor.handle_worker_result_poll(received_completed_execution(
                request_id,
                ProcessRefreshConsumerDemand::CompileMonitor
            ),),
            ProcessRefreshResultPoll::Pending
        );
        assert_eq!(
            process_refresh_executor.in_flight,
            ProcessRefreshInFlight::Awaiting {
                request_id,
                demand: current_demand,
            }
        );

        let ProcessRefreshResultPoll::Ready(process_refresh_execution) = process_refresh_executor
            .handle_worker_result_poll(received_completed_execution(request_id, current_demand))
        else {
            panic!("matching demand result should complete the current request");
        };
        assert_eq!(process_refresh_execution.demand(), current_demand);
        assert_eq!(
            process_refresh_executor.in_flight,
            ProcessRefreshInFlight::Idle
        );
    }
}
