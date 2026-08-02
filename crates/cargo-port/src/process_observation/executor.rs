use std::thread;
use std::thread::JoinHandle;
use std::time::Duration;
use std::time::Instant;

use super::ProcessObserver;
use super::snapshot::CompletedProcessRefreshExecution;
use super::snapshot::ProcessRefreshConsumerDemand;
use super::snapshot::ProcessRefreshExecutionFailure;
use super::snapshot::ProcessRefreshExecutionOutcome;
use crate::channel;
use crate::channel::Receiver;
use crate::channel::Sender;
use crate::channel::TryRecvError;

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
    #[cfg(test)]
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ProcessRefreshPlan {
    demand: ProcessRefreshConsumerDemand,
}

enum DueProcessRefreshDemand {
    Due(ProcessRefreshConsumerDemand),
    NotDue,
}

impl ProcessRefreshPlan {
    const fn new(demand: ProcessRefreshConsumerDemand) -> Self { Self { demand } }
}

struct ProcessRefreshWorkerRequest {
    request_id: ProcessRefreshRequestId,
    plan:       ProcessRefreshPlan,
}

enum ProcessRefreshWorkerCommand {
    Execute(ProcessRefreshWorkerRequest),
    Shutdown,
}

/// One correlated observer execution result.
#[derive(Debug, PartialEq)]
pub(crate) struct ProcessRefreshExecution {
    request_id: ProcessRefreshRequestId,
    demand:     ProcessRefreshConsumerDemand,
    outcome:    ProcessRefreshExecutionOutcome,
}

impl ProcessRefreshExecution {
    pub(crate) const fn demand(&self) -> ProcessRefreshConsumerDemand { self.demand }

    pub(crate) fn into_outcome(self) -> ProcessRefreshExecutionOutcome { self.outcome }

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
}

struct DedicatedProcessRefreshWorker {
    command_sender:  Sender<ProcessRefreshWorkerCommand>,
    result_receiver: Receiver<Box<ProcessRefreshExecution>>,
    thread_state:    ProcessRefreshWorkerThreadState,
}

enum ProcessRefreshWorkerThreadState {
    Running(JoinHandle<()>),
    Joined,
}

impl DedicatedProcessRefreshWorker {
    fn spawn() -> Self {
        let (command_sender, command_receiver) = channel::unbounded();
        let (result_sender, result_receiver) = channel::unbounded();
        let join_handle = thread::spawn(move || {
            process_refresh_worker(&command_receiver, &result_sender);
        });
        Self {
            command_sender,
            result_receiver,
            thread_state: ProcessRefreshWorkerThreadState::Running(join_handle),
        }
    }

    fn dispatch(
        &self,
        process_refresh_worker_request: ProcessRefreshWorkerRequest,
    ) -> Result<(), ProcessRefreshExecutionFailure> {
        self.command_sender
            .send(ProcessRefreshWorkerCommand::Execute(
                process_refresh_worker_request,
            ))
            .map_err(|_| ProcessRefreshExecutionFailure::RequestChannelDisconnected)
    }

    fn poll(&self) -> ProcessRefreshWorkerResultPoll {
        match self.result_receiver.try_recv() {
            Ok(process_refresh_execution) => {
                ProcessRefreshWorkerResultPoll::Received(process_refresh_execution)
            },
            Err(TryRecvError::Empty) => ProcessRefreshWorkerResultPoll::Pending,
            Err(TryRecvError::Disconnected) => ProcessRefreshWorkerResultPoll::Disconnected,
        }
    }

    const fn result_receiver(&self) -> &Receiver<Box<ProcessRefreshExecution>> {
        &self.result_receiver
    }
}

impl Drop for DedicatedProcessRefreshWorker {
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

enum ProcessRefreshExecutionBackend {
    Synchronous(Box<ProcessObserver>),
    DedicatedWorker(DedicatedProcessRefreshWorker),
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
pub(crate) enum ProcessRefreshDispatchOutcome {
    NotDue,
    AwaitingWorker(ProcessRefreshRequestId),
    Finished(Box<ProcessRefreshExecution>),
}

/// Nonblocking state of the dedicated worker result channel.
#[derive(Debug, PartialEq)]
pub(crate) enum ProcessRefreshResultPoll {
    Pending,
    Ready(Box<ProcessRefreshExecution>),
}

enum ProcessRefreshWorkerResultPoll {
    Pending,
    Received(Box<ProcessRefreshExecution>),
    Disconnected,
}

/// App-owned scheduler and execution backend for the sole `ProcessObserver`.
pub(crate) struct ProcessRefreshExecutor {
    backend:                  ProcessRefreshExecutionBackend,
    running_targets_schedule: RunningTargetsRefreshSchedule,
    running_targets_deadline: ProcessRefreshDeadline,
    compile_monitor_schedule: CompileMonitorRefreshSchedule,
    in_flight:                ProcessRefreshInFlight,
    next_request_id:          ProcessRefreshRequestId,
}

impl ProcessRefreshExecutor {
    pub(crate) fn new(
        backend_selection: ProcessRefreshExecutionBackendSelection,
        running_targets_schedule: RunningTargetsRefreshSchedule,
        compile_monitor_schedule: CompileMonitorRefreshSchedule,
        started_at: Instant,
    ) -> Self {
        let backend = match backend_selection {
            ProcessRefreshExecutionBackendSelection::Synchronous => {
                ProcessRefreshExecutionBackend::Synchronous(Box::default())
            },
            ProcessRefreshExecutionBackendSelection::DedicatedWorker => {
                ProcessRefreshExecutionBackend::DedicatedWorker(
                    DedicatedProcessRefreshWorker::spawn(),
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
            in_flight: ProcessRefreshInFlight::Idle,
            next_request_id: ProcessRefreshRequestId(0),
        }
    }

    pub(crate) fn next_deadline(&self) -> ProcessRefreshDeadline {
        if matches!(self.in_flight, ProcessRefreshInFlight::Awaiting { .. }) {
            return ProcessRefreshDeadline::AwaitingWorker;
        }
        minimum_deadline(self.running_targets_deadline, self.compile_monitor_schedule)
    }

    pub(crate) fn refresh_due(&mut self, now: Instant) -> ProcessRefreshDispatchOutcome {
        if matches!(self.in_flight, ProcessRefreshInFlight::Awaiting { .. }) {
            return ProcessRefreshDispatchOutcome::NotDue;
        }
        let demand = match self.due_demand(now) {
            DueProcessRefreshDemand::Due(demand) => demand,
            DueProcessRefreshDemand::NotDue => return ProcessRefreshDispatchOutcome::NotDue,
        };
        let request_id = self.next_request_id.next();
        self.advance_dispatched_deadlines(now, demand);
        let plan = ProcessRefreshPlan::new(demand);
        match &mut self.backend {
            ProcessRefreshExecutionBackend::Synchronous(process_observer) => {
                ProcessRefreshDispatchOutcome::Finished(Box::new(execute_refresh(
                    process_observer,
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

    pub(crate) fn poll_result(&mut self) -> ProcessRefreshResultPoll {
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
        worker_poll: ProcessRefreshWorkerResultPoll,
    ) -> ProcessRefreshResultPoll {
        let ProcessRefreshInFlight::Awaiting { request_id, demand } = self.in_flight else {
            return ProcessRefreshResultPoll::Pending;
        };
        match worker_poll {
            ProcessRefreshWorkerResultPoll::Pending => ProcessRefreshResultPoll::Pending,
            ProcessRefreshWorkerResultPoll::Received(process_refresh_execution)
                if process_refresh_execution.request_id == request_id
                    && process_refresh_execution.demand == demand =>
            {
                self.in_flight = ProcessRefreshInFlight::Idle;
                ProcessRefreshResultPoll::Ready(process_refresh_execution)
            },
            ProcessRefreshWorkerResultPoll::Received(_) => ProcessRefreshResultPoll::Pending,
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

    pub(crate) const fn result_receiver(&self) -> ProcessRefreshResultReceiver<'_> {
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
            #[cfg(test)]
            CompileMonitorRefreshSchedule::At(deadline) => deadline <= now,
            CompileMonitorRefreshSchedule::NotScheduled => false,
        };
        match (running_targets_due, compile_monitor_due) {
            (true, true) => DueProcessRefreshDemand::Due(
                ProcessRefreshConsumerDemand::RunningTargets
                    .coalesce(ProcessRefreshConsumerDemand::CompileMonitor),
            ),
            (true, false) => {
                DueProcessRefreshDemand::Due(ProcessRefreshConsumerDemand::RunningTargets)
            },
            (false, true) => {
                DueProcessRefreshDemand::Due(ProcessRefreshConsumerDemand::CompileMonitor)
            },
            (false, false) => DueProcessRefreshDemand::NotDue,
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
        if matches!(
            demand,
            ProcessRefreshConsumerDemand::CompileMonitor
                | ProcessRefreshConsumerDemand::RunningTargetsAndCompileMonitor
        ) {
            self.compile_monitor_schedule = CompileMonitorRefreshSchedule::NotScheduled;
        }
    }
}

/// Borrowed worker receiver used only to register event-loop wakeups.
pub(crate) enum ProcessRefreshResultReceiver<'a> {
    NoWorkerResultExpected,
    DedicatedWorker(&'a Receiver<Box<ProcessRefreshExecution>>),
}

fn minimum_deadline(
    running_targets_deadline: ProcessRefreshDeadline,
    compile_monitor_schedule: CompileMonitorRefreshSchedule,
) -> ProcessRefreshDeadline {
    let compile_monitor_deadline = match compile_monitor_schedule {
        #[cfg(test)]
        CompileMonitorRefreshSchedule::At(deadline) => ProcessRefreshDeadline::At(deadline),
        CompileMonitorRefreshSchedule::NotScheduled => ProcessRefreshDeadline::NotScheduled,
    };
    match (running_targets_deadline, compile_monitor_deadline) {
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

fn process_refresh_worker(
    command_receiver: &Receiver<ProcessRefreshWorkerCommand>,
    result_sender: &Sender<Box<ProcessRefreshExecution>>,
) {
    let mut process_observer = ProcessObserver::default();
    while let Ok(command) = command_receiver.recv() {
        match command {
            ProcessRefreshWorkerCommand::Execute(process_refresh_worker_request) => {
                let process_refresh_execution = execute_refresh(
                    &mut process_observer,
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

fn execute_refresh(
    process_observer: &mut ProcessObserver,
    request_id: ProcessRefreshRequestId,
    plan: ProcessRefreshPlan,
) -> ProcessRefreshExecution {
    let started = Instant::now();
    let process_observation_snapshot = process_observer.refresh_for_consumer_demand(plan.demand);
    ProcessRefreshExecution {
        request_id,
        demand: plan.demand,
        outcome: ProcessRefreshExecutionOutcome::Completed(CompletedProcessRefreshExecution::new(
            process_observation_snapshot,
            started.elapsed(),
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

    fn executor_awaiting(
        request_id: ProcessRefreshRequestId,
        demand: ProcessRefreshConsumerDemand,
    ) -> ProcessRefreshExecutor {
        let mut process_refresh_executor = ProcessRefreshExecutor::new(
            ProcessRefreshExecutionBackendSelection::Synchronous,
            RunningTargetsRefreshSchedule::Suppressed,
            CompileMonitorRefreshSchedule::NotScheduled,
            Instant::now(),
        );
        process_refresh_executor.in_flight =
            ProcessRefreshInFlight::Awaiting { request_id, demand };
        process_refresh_executor
    }

    fn completed_execution(
        request_id: ProcessRefreshRequestId,
        demand: ProcessRefreshConsumerDemand,
    ) -> Box<ProcessRefreshExecution> {
        Box::new(ProcessRefreshExecution {
            request_id,
            demand,
            outcome: ProcessRefreshExecutionOutcome::Completed(
                CompletedProcessRefreshExecution::new(
                    ProcessObservationSnapshot::empty_for_test(),
                    Duration::ZERO,
                ),
            ),
        })
    }

    fn disconnected_worker() -> DedicatedProcessRefreshWorker {
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
            RunningTargetsRefreshSchedule::Every(Duration::from_secs(1)),
            CompileMonitorRefreshSchedule::At(now),
            now,
        );

        let ProcessRefreshDispatchOutcome::Finished(process_refresh_execution) =
            process_refresh_executor.refresh_due(now)
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
            process_refresh_executor.refresh_due(now),
            ProcessRefreshDispatchOutcome::NotDue
        );
    }

    #[test]
    fn no_compile_deadline_exists_when_monitoring_is_not_scheduled() {
        let now = Instant::now();
        let process_refresh_executor = ProcessRefreshExecutor::new(
            ProcessRefreshExecutionBackendSelection::Synchronous,
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
            RunningTargetsRefreshSchedule::Every(Duration::from_secs(1)),
            CompileMonitorRefreshSchedule::NotScheduled,
            now,
        );

        let ProcessRefreshDispatchOutcome::Finished(process_refresh_execution) =
            process_refresh_executor.refresh_due(now)
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
        let completed_empty =
            ProcessRefreshExecutionOutcome::Completed(CompletedProcessRefreshExecution::new(
                ProcessObservationSnapshot::empty_for_test(),
                Duration::ZERO,
            ));
        let failed = ProcessRefreshExecutionOutcome::Failed(
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
        let process_refresh_execution = ProcessRefreshExecution::failed(
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
            RunningTargetsRefreshSchedule::Every(Duration::from_secs(1)),
            CompileMonitorRefreshSchedule::NotScheduled,
            now,
        );
        assert!(matches!(
            process_refresh_executor.refresh_due(now),
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
    fn request_channel_failure_has_no_completed_execution_timing() {
        let now = Instant::now();
        let mut process_refresh_executor = ProcessRefreshExecutor::new(
            ProcessRefreshExecutionBackendSelection::Synchronous,
            RunningTargetsRefreshSchedule::Every(Duration::from_secs(1)),
            CompileMonitorRefreshSchedule::NotScheduled,
            now,
        );
        process_refresh_executor.backend =
            ProcessRefreshExecutionBackend::DedicatedWorker(disconnected_worker());

        let ProcessRefreshDispatchOutcome::Finished(process_refresh_execution) =
            process_refresh_executor.refresh_due(now)
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
            process_refresh_executor.handle_worker_result_poll(
                ProcessRefreshWorkerResultPoll::Received(completed_execution(
                    ProcessRefreshRequestId(6),
                    demand,
                )),
            ),
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
            .handle_worker_result_poll(ProcessRefreshWorkerResultPoll::Received(
                completed_execution(current_request_id, demand),
            ))
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
            process_refresh_executor.handle_worker_result_poll(
                ProcessRefreshWorkerResultPoll::Received(completed_execution(
                    request_id,
                    ProcessRefreshConsumerDemand::CompileMonitor,
                )),
            ),
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
            .handle_worker_result_poll(ProcessRefreshWorkerResultPoll::Received(
                completed_execution(request_id, current_demand),
            ))
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
