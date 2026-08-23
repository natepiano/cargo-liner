use std::thread;
use std::thread::JoinHandle;
use std::time::Duration;
use std::time::Instant;

use super::ProcessObserver;
use super::snapshot::CompletedProcessRefreshExecution;
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

enum ProcessRefreshWorkerCommand {
    Execute(ProcessRefreshRequestId),
    Shutdown,
}

/// One correlated observer execution result.
#[derive(Debug, PartialEq)]
pub(crate) struct ProcessRefreshExecution {
    request_id: ProcessRefreshRequestId,
    outcome:    ProcessRefreshExecutionOutcome,
}

impl ProcessRefreshExecution {
    pub(crate) fn into_outcome(self) -> ProcessRefreshExecutionOutcome { self.outcome }

    const fn failed(
        request_id: ProcessRefreshRequestId,
        failure: ProcessRefreshExecutionFailure,
    ) -> Self {
        Self {
            request_id,
            outcome: ProcessRefreshExecutionOutcome::Failed(failure),
        }
    }

    #[cfg(test)]
    pub(crate) const fn failed_for_test(failure: ProcessRefreshExecutionFailure) -> Self {
        Self::failed(ProcessRefreshRequestId(0), failure)
    }

    /// A correlated completed execution over an empty observation, so a
    /// consumer's reconciliation can be tested without a host refresh.
    #[cfg(test)]
    pub(crate) fn completed_for_test() -> Self {
        Self {
            request_id: ProcessRefreshRequestId(0),
            outcome:    ProcessRefreshExecutionOutcome::Completed(Box::new(
                CompletedProcessRefreshExecution::new(
                    crate::process_observation::snapshot::ProcessObservationSnapshot::empty_for_test(
                    ),
                    Duration::ZERO,
                ),
            )),
        }
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
            let mut process_observer = ProcessObserver::default();
            process_refresh_worker(&mut process_observer, &command_receiver, &result_sender);
        });
        Self {
            command_sender,
            result_receiver,
            thread_state: ProcessRefreshWorkerThreadState::Running(join_handle),
        }
    }

    fn dispatch(
        &self,
        request_id: ProcessRefreshRequestId,
    ) -> Result<(), ProcessRefreshExecutionFailure> {
        self.command_sender
            .send(ProcessRefreshWorkerCommand::Execute(request_id))
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
    Awaiting(ProcessRefreshRequestId),
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
    in_flight:                ProcessRefreshInFlight,
    next_request_id:          ProcessRefreshRequestId,
}

impl ProcessRefreshExecutor {
    pub(crate) fn new(
        backend_selection: ProcessRefreshExecutionBackendSelection,
        running_targets_schedule: RunningTargetsRefreshSchedule,
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
            in_flight: ProcessRefreshInFlight::Idle,
            next_request_id: ProcessRefreshRequestId(0),
        }
    }

    pub(crate) const fn next_deadline(&self) -> ProcessRefreshDeadline {
        if matches!(self.in_flight, ProcessRefreshInFlight::Awaiting(_)) {
            return ProcessRefreshDeadline::AwaitingWorker;
        }
        self.running_targets_deadline
    }

    /// Dispatch a due cycle. A tick that finds nothing due does no work.
    pub(crate) fn refresh_due(&mut self, now: Instant) -> ProcessRefreshDispatchOutcome {
        if matches!(self.in_flight, ProcessRefreshInFlight::Awaiting(_)) {
            return ProcessRefreshDispatchOutcome::NotDue;
        }
        if !matches!(
            self.running_targets_deadline,
            ProcessRefreshDeadline::At(deadline) if deadline <= now
        ) {
            return ProcessRefreshDispatchOutcome::NotDue;
        }
        let request_id = self.next_request_id.next();
        self.running_targets_deadline = match self.running_targets_schedule {
            RunningTargetsRefreshSchedule::Every(interval) => {
                ProcessRefreshDeadline::At(now + interval)
            },
            RunningTargetsRefreshSchedule::Suppressed => ProcessRefreshDeadline::NotScheduled,
        };
        match &mut self.backend {
            ProcessRefreshExecutionBackend::Synchronous(process_observer) => {
                ProcessRefreshDispatchOutcome::Finished(Box::new(execute_refresh(
                    process_observer,
                    request_id,
                )))
            },
            ProcessRefreshExecutionBackend::DedicatedWorker(worker) => {
                match worker.dispatch(request_id) {
                    Ok(()) => {
                        self.in_flight = ProcessRefreshInFlight::Awaiting(request_id);
                        ProcessRefreshDispatchOutcome::AwaitingWorker(request_id)
                    },
                    Err(failure) => ProcessRefreshDispatchOutcome::Finished(Box::new(
                        ProcessRefreshExecution::failed(request_id, failure),
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
        let ProcessRefreshInFlight::Awaiting(request_id) = self.in_flight else {
            return ProcessRefreshResultPoll::Pending;
        };
        match worker_poll {
            ProcessRefreshWorkerResultPoll::Received(process_refresh_execution)
                if process_refresh_execution.request_id == request_id =>
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
                    ProcessRefreshExecutionFailure::ResultChannelDisconnected,
                )))
            },
        }
    }

    pub(crate) const fn result_receiver(&self) -> ProcessRefreshResultReceiver<'_> {
        match (&self.backend, self.in_flight) {
            (
                ProcessRefreshExecutionBackend::DedicatedWorker(worker),
                ProcessRefreshInFlight::Awaiting(_),
            ) => ProcessRefreshResultReceiver::DedicatedWorker(worker.result_receiver()),
            (ProcessRefreshExecutionBackend::Synchronous(_), _)
            | (ProcessRefreshExecutionBackend::DedicatedWorker(_), ProcessRefreshInFlight::Idle) => {
                ProcessRefreshResultReceiver::NoWorkerResultExpected
            },
        }
    }
}

/// Borrowed worker receiver used only to register event-loop wakeups.
pub(crate) enum ProcessRefreshResultReceiver<'a> {
    NoWorkerResultExpected,
    DedicatedWorker(&'a Receiver<Box<ProcessRefreshExecution>>),
}

fn process_refresh_worker(
    process_observer: &mut ProcessObserver,
    command_receiver: &Receiver<ProcessRefreshWorkerCommand>,
    result_sender: &Sender<Box<ProcessRefreshExecution>>,
) {
    while let Ok(command) = command_receiver.recv() {
        match command {
            ProcessRefreshWorkerCommand::Execute(request_id) => {
                let process_refresh_execution = execute_refresh(process_observer, request_id);
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

/// Observe the host and time only the observation, which is the boundary the
/// event loop instruments and the benchmarks report against.
fn execute_refresh(
    process_observer: &mut ProcessObserver,
    request_id: ProcessRefreshRequestId,
) -> ProcessRefreshExecution {
    let started = Instant::now();
    let process_observation_snapshot = process_observer.refresh_running_targets_cycle();
    let elapsed = started.elapsed();
    ProcessRefreshExecution {
        request_id,
        outcome: ProcessRefreshExecutionOutcome::Completed(Box::new(
            CompletedProcessRefreshExecution::new(process_observation_snapshot, elapsed),
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

    fn executor_awaiting(request_id: ProcessRefreshRequestId) -> ProcessRefreshExecutor {
        let mut process_refresh_executor = ProcessRefreshExecutor::new(
            ProcessRefreshExecutionBackendSelection::Synchronous,
            RunningTargetsRefreshSchedule::Suppressed,
            Instant::now(),
        );
        process_refresh_executor.in_flight = ProcessRefreshInFlight::Awaiting(request_id);
        process_refresh_executor
    }

    fn received_completed_execution(
        request_id: ProcessRefreshRequestId,
    ) -> ProcessRefreshWorkerResultPoll {
        ProcessRefreshWorkerResultPoll::Received(Box::new(ProcessRefreshExecution {
            request_id,
            outcome: ProcessRefreshExecutionOutcome::Completed(Box::new(
                CompletedProcessRefreshExecution::new(
                    ProcessObservationSnapshot::empty_for_test(),
                    Duration::ZERO,
                ),
            )),
        }))
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
    fn a_due_cycle_dispatches_once_and_rearms_for_the_next_interval() {
        let now = Instant::now();
        let interval = Duration::from_secs(1);
        let mut process_refresh_executor = ProcessRefreshExecutor::new(
            ProcessRefreshExecutionBackendSelection::Synchronous,
            RunningTargetsRefreshSchedule::Every(interval),
            now,
        );

        let ProcessRefreshDispatchOutcome::Finished(process_refresh_execution) =
            process_refresh_executor.refresh_due(now)
        else {
            panic!("a due synchronous refresh should complete");
        };

        assert!(matches!(
            process_refresh_execution.into_outcome(),
            ProcessRefreshExecutionOutcome::Completed(_)
        ));
        assert_eq!(
            process_refresh_executor.refresh_due(now),
            ProcessRefreshDispatchOutcome::NotDue
        );
        assert_eq!(
            process_refresh_executor.next_deadline(),
            ProcessRefreshDeadline::At(now + interval)
        );
    }

    #[test]
    fn a_suppressed_schedule_has_no_deadline_and_nothing_due() {
        let now = Instant::now();
        let mut process_refresh_executor = ProcessRefreshExecutor::new(
            ProcessRefreshExecutionBackendSelection::Synchronous,
            RunningTargetsRefreshSchedule::Suppressed,
            now,
        );

        assert_eq!(
            process_refresh_executor.next_deadline(),
            ProcessRefreshDeadline::NotScheduled
        );
        assert_eq!(
            process_refresh_executor.refresh_due(now),
            ProcessRefreshDispatchOutcome::NotDue
        );
    }

    #[test]
    fn synchronous_completion_contains_successful_execution_timing() {
        let now = Instant::now();
        let mut process_refresh_executor = ProcessRefreshExecutor::new(
            ProcessRefreshExecutionBackendSelection::Synchronous,
            RunningTargetsRefreshSchedule::Every(Duration::from_secs(1)),
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
        let completed_empty = ProcessRefreshExecutionOutcome::Completed(Box::new(
            CompletedProcessRefreshExecution::new(
                ProcessObservationSnapshot::empty_for_test(),
                Duration::ZERO,
            ),
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
                    thread::yield_now();
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
    fn an_awaiting_executor_registers_the_worker_receiver_for_wakeups() {
        let now = Instant::now();
        let mut process_refresh_executor = ProcessRefreshExecutor::new(
            ProcessRefreshExecutionBackendSelection::DedicatedWorker,
            RunningTargetsRefreshSchedule::Every(Duration::from_secs(1)),
            now,
        );

        assert!(matches!(
            process_refresh_executor.result_receiver(),
            ProcessRefreshResultReceiver::NoWorkerResultExpected
        ));
        assert!(matches!(
            process_refresh_executor.refresh_due(now),
            ProcessRefreshDispatchOutcome::AwaitingWorker(_)
        ));
        assert!(matches!(
            process_refresh_executor.result_receiver(),
            ProcessRefreshResultReceiver::DedicatedWorker(_)
        ));
        assert_eq!(
            process_refresh_executor.next_deadline(),
            ProcessRefreshDeadline::AwaitingWorker
        );
    }

    #[test]
    fn request_channel_failure_has_no_completed_execution_timing() {
        let now = Instant::now();
        let mut process_refresh_executor = ProcessRefreshExecutor::new(
            ProcessRefreshExecutionBackendSelection::Synchronous,
            RunningTargetsRefreshSchedule::Every(Duration::from_secs(1)),
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
        let mut process_refresh_executor = executor_awaiting(request_id);
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
        let mut process_refresh_executor = executor_awaiting(current_request_id);

        assert_eq!(
            process_refresh_executor.handle_worker_result_poll(received_completed_execution(
                ProcessRefreshRequestId(6)
            )),
            ProcessRefreshResultPoll::Pending
        );
        assert_eq!(
            process_refresh_executor.in_flight,
            ProcessRefreshInFlight::Awaiting(current_request_id)
        );

        let ProcessRefreshResultPoll::Ready(process_refresh_execution) = process_refresh_executor
            .handle_worker_result_poll(received_completed_execution(current_request_id))
        else {
            panic!("matching request result should complete the current request");
        };
        assert_eq!(process_refresh_execution.request_id, current_request_id);
        assert_eq!(
            process_refresh_executor.in_flight,
            ProcessRefreshInFlight::Idle
        );
    }
}
