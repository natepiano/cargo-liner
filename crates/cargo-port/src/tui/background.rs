//! The `Background` subsystem.
//!
//! Owns the four channel pairs plus the watcher handle:
//! - background `sender` / `receiver` (replaced wholesale on every rescan — see
//!   [`Background::swap_background_channel`])
//! - `ci_fetch_tx` / `ci_fetch_rx`
//! - `clean_tx` / `clean_rx`
//! - `example_tx` / `example_rx`
//! - `watcher`
//!
//! Spawn / poll facade methods live on `App` (and inside
//! [`Inflight`]) because they thread cross-subsystem dependencies
//! (`Scan`, `Net`, framework toasts).
//!
//! [`Inflight`]: super::state::Inflight

use std::time::Instant;

use tui_pane::PERF_LOG_TARGET;

use super::messages::ProcessTerminationOutcomeMsg;
use super::messages::ProcessTerminationPlanMsg;
use super::process_refresh::AppProcessRefreshExecutor;
use super::startup_services::WatcherHandle;
use super::terminal::CiFetchMsg;
use super::terminal::CleanMsg;
use super::terminal::OwnedRunEvent;
use crate::build_monitor;
use crate::build_monitor::BuildClassifier;
use crate::build_monitor::BuildMonitoringRefreshCycleDemand;
use crate::build_monitor::BuildMonitoringRefreshCycleExecution;
use crate::channel::Receiver;
use crate::channel::SendError;
use crate::channel::Sender;
use crate::process_observation::CompileMonitorRefreshSchedule;
use crate::process_observation::ProcessObserver;
use crate::process_observation::ProcessRefreshExecutionBackendSelection;
use crate::process_observation::ProcessRefreshExecutor;
use crate::process_observation::RefreshCycleClassifier;
use crate::process_observation::RunningTargetsRefreshSchedule;
use crate::process_observation::snapshot::ProcessObservationSnapshot;
use crate::process_termination::ProcessTerminator;
use crate::process_termination::TerminationDispatchOutcome;
use crate::process_termination::TerminationPlanCreation;
use crate::process_termination::TerminationRequestId;
use crate::process_termination::TerminationResultPoll;
use crate::project;
use crate::project::AbsolutePath;
use crate::project::RootItem;
use crate::scan::BackgroundMsg;
use crate::watcher::WatchRequest;
use crate::watcher::WatcherMsg;

/// Bundle the four channel pairs plus the watcher handle that
/// [`Background`] owns. Single argument to [`Background::new`].
pub struct BackgroundChannels {
    pub background: (Sender<BackgroundMsg>, Receiver<BackgroundMsg>),
    pub ci_fetch:   (Sender<CiFetchMsg>, Receiver<CiFetchMsg>),
    pub clean:      (Sender<CleanMsg>, Receiver<CleanMsg>),
    pub example:    (Sender<OwnedRunEvent>, Receiver<OwnedRunEvent>),
    pub watcher:    WatcherHandle,
}

/// The termination worker's one startup handshake.
///
/// The execution plan contains no targets. It proves that the dedicated
/// request/result channel is operating without observing or signaling a
/// process; authority-bearing plans are submitted only after readiness.
enum ProcessTerminationWorkerReadiness {
    Awaiting(TerminationRequestId),
    Available,
    Unavailable,
    #[cfg(test)]
    StartingForTest,
}

/// Deterministic worker readiness used by App interaction fixtures.
#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ProcessTerminatorReadinessForTest {
    Starting,
    Available,
    Unavailable,
}

/// Owns every long-lived I/O channel App holds. App holds a single
/// `background: Background` field.
pub(super) struct Background {
    sender:                               Sender<BackgroundMsg>,
    receiver:                             Receiver<BackgroundMsg>,
    ci_fetch_tx:                          Sender<CiFetchMsg>,
    ci_fetch_rx:                          Receiver<CiFetchMsg>,
    clean_tx:                             Sender<CleanMsg>,
    clean_rx:                             Receiver<CleanMsg>,
    example_tx:                           Sender<OwnedRunEvent>,
    example_rx:                           Receiver<OwnedRunEvent>,
    process_terminator:                   ProcessTerminator,
    process_termination_worker_readiness: ProcessTerminationWorkerReadiness,
    watcher:                              WatcherHandle,
}

/// Borrowed external-worker receiver used only for event-loop wakeups.
pub(super) enum ProcessTerminationResultReceiver<'a> {
    NoCompletedResultExpected,
    Worker(&'a Receiver<ProcessTerminationOutcomeMsg>),
}

impl Background {
    pub(super) fn new(channels: BackgroundChannels) -> Self {
        let BackgroundChannels {
            background: (background_tx, background_rx),
            ci_fetch: (ci_fetch_tx, ci_fetch_rx),
            clean: (clean_tx, clean_rx),
            example: (example_tx, example_rx),
            watcher,
        } = channels;
        let (process_terminator, process_termination_worker_readiness) =
            Self::start_process_termination_worker();
        Self {
            sender: background_tx,
            receiver: background_rx,
            ci_fetch_tx,
            ci_fetch_rx,
            clean_tx,
            clean_rx,
            example_tx,
            example_rx,
            process_terminator,
            process_termination_worker_readiness,
            watcher,
        }
    }

    /// Start the observer backend and transfer its worker ownership to App's executor.
    pub(super) fn start_process_refresh_executor(
        backend_selection: ProcessRefreshExecutionBackendSelection,
        running_targets_refresh_schedule: RunningTargetsRefreshSchedule,
        compile_monitor_refresh_schedule: CompileMonitorRefreshSchedule,
        started_at: Instant,
    ) -> AppProcessRefreshExecutor {
        ProcessRefreshExecutor::new(
            backend_selection,
            BuildClassifyingRefreshCycle::default(),
            running_targets_refresh_schedule,
            compile_monitor_refresh_schedule,
            started_at,
        )
    }

    // ── Senders (cloned by spawn paths) ──────────────────────────────

    pub(super) fn background_sender(&self) -> Sender<BackgroundMsg> { self.sender.clone() }

    pub(super) fn ci_fetch_sender(&self) -> Sender<CiFetchMsg> { self.ci_fetch_tx.clone() }

    pub(super) fn clean_sender(&self) -> Sender<CleanMsg> { self.clean_tx.clone() }

    pub(super) fn example_sender(&self) -> Sender<OwnedRunEvent> { self.example_tx.clone() }

    /// Reap the target-free startup handshake without consuming later
    /// authority-bearing termination outcomes after the worker reports readiness.
    pub(super) fn poll_process_termination_worker_readiness(&mut self) {
        let ProcessTerminationWorkerReadiness::Awaiting(termination_request_id) =
            self.process_termination_worker_readiness
        else {
            return;
        };
        match self.process_terminator.poll_outcome() {
            TerminationResultPoll::Completed(termination_outcome_summary) => {
                self.reconcile_process_termination_worker_handshake(
                    termination_request_id,
                    &termination_outcome_summary,
                );
            },
            TerminationResultPoll::NoCompletedRequest => {},
            TerminationResultPoll::WorkerUnavailable => {
                self.process_termination_worker_readiness =
                    ProcessTerminationWorkerReadiness::Unavailable;
            },
        }
    }

    /// Poll one correlated external result after the target-free startup
    /// handshake has established that the worker is available.
    pub(super) fn poll_process_termination_outcome(&mut self) -> TerminationResultPoll {
        match self.process_termination_worker_readiness {
            ProcessTerminationWorkerReadiness::Available => {
                let termination_result_poll = self.process_terminator.poll_outcome();
                if matches!(
                    termination_result_poll,
                    TerminationResultPoll::WorkerUnavailable
                ) {
                    self.process_termination_worker_readiness =
                        ProcessTerminationWorkerReadiness::Unavailable;
                }
                termination_result_poll
            },
            ProcessTerminationWorkerReadiness::Awaiting(_) => {
                TerminationResultPoll::NoCompletedRequest
            },
            #[cfg(test)]
            ProcessTerminationWorkerReadiness::StartingForTest => {
                TerminationResultPoll::NoCompletedRequest
            },
            ProcessTerminationWorkerReadiness::Unavailable => {
                TerminationResultPoll::WorkerUnavailable
            },
        }
    }

    pub(super) const fn process_termination_result_receiver(
        &self,
    ) -> ProcessTerminationResultReceiver<'_> {
        match self.process_termination_worker_readiness {
            ProcessTerminationWorkerReadiness::Available => {
                ProcessTerminationResultReceiver::Worker(self.process_terminator.outcome_receiver())
            },
            ProcessTerminationWorkerReadiness::Awaiting(_)
            | ProcessTerminationWorkerReadiness::Unavailable => {
                ProcessTerminationResultReceiver::NoCompletedResultExpected
            },
            #[cfg(test)]
            ProcessTerminationWorkerReadiness::StartingForTest => {
                ProcessTerminationResultReceiver::NoCompletedResultExpected
            },
        }
    }

    /// Borrow the dedicated external worker only after its startup handshake.
    ///
    /// The returned terminator still accepts only frozen opaque capabilities;
    /// no caller receives a platform adapter or PID signal path.
    pub(super) const fn available_process_terminator(
        &mut self,
    ) -> ProcessTerminatorAvailability<'_> {
        match self.process_termination_worker_readiness {
            ProcessTerminationWorkerReadiness::Available => {
                ProcessTerminatorAvailability::Available(&mut self.process_terminator)
            },
            ProcessTerminationWorkerReadiness::Awaiting(_) => {
                ProcessTerminatorAvailability::Starting
            },
            #[cfg(test)]
            ProcessTerminationWorkerReadiness::StartingForTest => {
                ProcessTerminatorAvailability::Starting
            },
            ProcessTerminationWorkerReadiness::Unavailable => {
                ProcessTerminatorAvailability::Unavailable
            },
        }
    }

    fn reconcile_process_termination_worker_handshake(
        &mut self,
        termination_request_id: TerminationRequestId,
        termination_outcome_summary: &ProcessTerminationOutcomeMsg,
    ) {
        if termination_outcome_summary.termination_request_id() == termination_request_id {
            self.process_termination_worker_readiness =
                ProcessTerminationWorkerReadiness::Available;
        }
    }

    fn start_process_termination_worker() -> (ProcessTerminator, ProcessTerminationWorkerReadiness)
    {
        let mut process_terminator = ProcessTerminator::start();
        let termination_execution_plan: ProcessTerminationPlanMsg =
            match process_terminator.plan_termination(Vec::new()) {
                TerminationPlanCreation::Planned(termination_execution_plan) => {
                    termination_execution_plan
                },
                TerminationPlanCreation::RequestIdsExhausted => {
                    return (
                        process_terminator,
                        ProcessTerminationWorkerReadiness::Unavailable,
                    );
                },
            };
        let termination_request_id = termination_execution_plan.termination_request_id();
        match process_terminator.request_termination(termination_execution_plan) {
            TerminationDispatchOutcome::Dispatched(dispatched_request_id)
                if dispatched_request_id == termination_request_id =>
            {
                (
                    process_terminator,
                    ProcessTerminationWorkerReadiness::Awaiting(termination_request_id),
                )
            },
            TerminationDispatchOutcome::Dispatched(_)
            | TerminationDispatchOutcome::WorkerUnavailable => (
                process_terminator,
                ProcessTerminationWorkerReadiness::Unavailable,
            ),
        }
    }

    // ── Receiver access ──────────────────────────────────────────────

    pub(super) const fn background_receiver(&self) -> &Receiver<BackgroundMsg> { &self.receiver }

    pub(super) const fn ci_fetch_rx(&self) -> &Receiver<CiFetchMsg> { &self.ci_fetch_rx }

    pub(super) const fn clean_rx(&self) -> &Receiver<CleanMsg> { &self.clean_rx }

    pub(super) const fn example_rx(&self) -> &Receiver<OwnedRunEvent> { &self.example_rx }

    /// Send `msg` on the watcher channel. Convenience for the
    /// common watcher-registration pattern. Disabled watcher handles
    /// accept the message without starting a watcher thread.
    pub(super) fn send_watcher(&self, msg: WatcherMsg) -> Result<(), SendError<WatcherMsg>> {
        self.watcher.send(msg)
    }

    /// Replace the background channel pair wholesale. Called from
    /// `App::rescan` — the background channel is rebuilt for each scan run
    /// while the other three channel pairs outlive any single
    /// rescan. The asymmetry stays explicit in the API rather than
    /// getting smoothed over (see plan note "Background channel-
    /// rescan caveat").
    pub(super) fn swap_background_channel(
        &mut self,
        sender: Sender<BackgroundMsg>,
        receiver: Receiver<BackgroundMsg>,
    ) {
        self.sender = sender;
        self.receiver = receiver;
    }

    /// Replace the watcher handle, used by `App::respawn_watcher`
    /// after a config reload changes the watch roots.
    pub(super) fn replace_watcher(&mut self, watcher: WatcherHandle) { self.watcher = watcher; }

    #[cfg(test)]
    pub(super) const fn watcher_is_active(&self) -> bool { self.watcher.is_active() }

    /// Set worker readiness without driving the background handshake.
    #[cfg(test)]
    pub(super) const fn set_process_terminator_readiness_for_test(
        &mut self,
        process_terminator_readiness_for_test: ProcessTerminatorReadinessForTest,
    ) {
        self.process_termination_worker_readiness = match process_terminator_readiness_for_test {
            ProcessTerminatorReadinessForTest::Starting => {
                ProcessTerminationWorkerReadiness::StartingForTest
            },
            ProcessTerminatorReadinessForTest::Available => {
                ProcessTerminationWorkerReadiness::Available
            },
            ProcessTerminatorReadinessForTest::Unavailable => {
                ProcessTerminationWorkerReadiness::Unavailable
            },
        };
    }

    pub(super) fn register_item_background_services(&self, item: &RootItem) {
        let started = std::time::Instant::now();
        let abs_path = AbsolutePath::from(item.path().to_path_buf());
        let repo_root = project::git_repo_root(&abs_path);
        let has_repo_root = repo_root.is_some();
        let _ = self.send_watcher(WatcherMsg::Register(WatchRequest {
            project_label: abs_path.to_string_lossy().to_string(),
            abs_path: abs_path.clone(),
            repo_root,
        }));
        tracing::trace!(
            target: PERF_LOG_TARGET,
            elapsed_ms = tui_pane::perf_log_ms(started.elapsed().as_millis()),
            path = %item.display_path(),
            has_repo_root,
            "app_register_project_background_services"
        );
    }
}

/// Whether App may submit a frozen external transaction to the worker.
pub(super) enum ProcessTerminatorAvailability<'a> {
    Available(&'a mut ProcessTerminator),
    Starting,
    Unavailable,
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use super::*;
    use crate::channel;

    fn make_msg() -> BackgroundMsg {
        BackgroundMsg::RepoFetchQueued {
            repo: crate::ci::OwnerRepo::new("owner", "repo"),
        }
    }

    fn fresh() -> Background {
        let (watch_tx, _watch_rx) = channel::unbounded();
        Background::new(BackgroundChannels {
            background: channel::unbounded(),
            ci_fetch:   channel::unbounded(),
            clean:      channel::unbounded(),
            example:    channel::unbounded(),
            watcher:    WatcherHandle::active(watch_tx),
        })
    }

    #[test]
    fn bg_sender_clone_round_trips_through_rx() {
        let background = fresh();
        let sender = background.background_sender();
        sender
            .send(make_msg())
            .expect("send through cloned bg sender");
        let received = background
            .background_receiver()
            .recv()
            .expect("recv on background_rx");
        assert!(matches!(received, BackgroundMsg::RepoFetchQueued { .. }));
    }

    #[test]
    fn swap_bg_channel_routes_to_new_pair_only() {
        let mut background = fresh();
        let original_sender = background.background_sender();

        let (new_tx, new_rx) = channel::unbounded();
        background.swap_background_channel(new_tx, new_rx);

        // Sender cloned before the swap can still send (it's tied to
        // the dropped receiver), but the swapped-in receiver must
        // not see anything from it.
        let _ = original_sender.send(make_msg());
        assert!(
            background.background_receiver().try_recv().is_err(),
            "stale sender must not reach the swapped-in rx"
        );

        // A fresh send via the new sender DOES reach the new rx.
        background
            .background_sender()
            .send(make_msg())
            .expect("send through post-swap bg sender");
        let received = background
            .background_receiver()
            .recv()
            .expect("recv on swapped background_rx");
        assert!(matches!(received, BackgroundMsg::RepoFetchQueued { .. }));
    }

    #[test]
    fn send_watcher_delivers_to_watcher_channel() {
        let (watch_tx, watch_rx) = channel::unbounded();
        let background = Background::new(BackgroundChannels {
            background: channel::unbounded(),
            ci_fetch:   channel::unbounded(),
            clean:      channel::unbounded(),
            example:    channel::unbounded(),
            watcher:    WatcherHandle::active(watch_tx),
        });

        background
            .send_watcher(WatcherMsg::InitialRegistrationComplete)
            .expect("send_watcher succeeds");
        let received = watch_rx.recv().expect("recv on watch_rx");
        assert!(matches!(received, WatcherMsg::InitialRegistrationComplete));
    }

    #[test]
    fn replace_watcher_handle_redirects_send_watcher() {
        let mut background = fresh();
        let (new_watch_tx, new_watch_rx) = channel::unbounded();
        background.replace_watcher(WatcherHandle::active(new_watch_tx));
        background
            .send_watcher(WatcherMsg::InitialRegistrationComplete)
            .expect("send_watcher succeeds post-replace");
        let received = new_watch_rx.recv().expect("recv on new watcher rx");
        assert!(matches!(received, WatcherMsg::InitialRegistrationComplete));
    }

    #[test]
    fn disabled_watcher_handle_ignores_registration_messages() {
        let background = Background::new(BackgroundChannels {
            background: channel::unbounded(),
            ci_fetch:   channel::unbounded(),
            clean:      channel::unbounded(),
            example:    channel::unbounded(),
            watcher:    WatcherHandle::disabled(),
        });

        background
            .send_watcher(WatcherMsg::InitialRegistrationComplete)
            .expect("disabled watcher accepts completion");

        assert!(!background.watcher_is_active());
    }

    #[test]
    fn observed_worker_disconnection_stops_termination_receiver_registration() {
        let mut background = fresh();
        background.process_terminator = ProcessTerminator::disconnected_for_test();
        background.process_termination_worker_readiness =
            ProcessTerminationWorkerReadiness::Available;

        assert!(matches!(
            background.poll_process_termination_outcome(),
            TerminationResultPoll::WorkerUnavailable
        ));
        assert!(matches!(
            background.process_termination_result_receiver(),
            ProcessTerminationResultReceiver::NoCompletedResultExpected
        ));
    }

    #[test]
    fn test_readiness_control_reports_each_submission_state() {
        let mut background = fresh();

        background
            .set_process_terminator_readiness_for_test(ProcessTerminatorReadinessForTest::Starting);
        assert!(matches!(
            background.available_process_terminator(),
            ProcessTerminatorAvailability::Starting
        ));

        background.set_process_terminator_readiness_for_test(
            ProcessTerminatorReadinessForTest::Available,
        );
        assert!(matches!(
            background.available_process_terminator(),
            ProcessTerminatorAvailability::Available(_)
        ));

        background.set_process_terminator_readiness_for_test(
            ProcessTerminatorReadinessForTest::Unavailable,
        );
        assert!(matches!(
            background.available_process_terminator(),
            ProcessTerminatorAvailability::Unavailable
        ));
    }
}

/// The sole runtime [`BuildClassifier`], handed to the dedicated
/// process-refresh worker at spawn.
///
/// It lives here rather than in `process_observation` because
/// `build_monitor::classify` already reads observation's snapshot, incarnation,
/// and ancestry types; owning the classifier inside observation would point the
/// dependency both ways. Owning it here keeps observation neutral while the
/// worker still holds the classifier and the observer as one unit.
#[derive(Debug, Default)]
pub(super) struct BuildClassifyingRefreshCycle {
    build_classifier: BuildClassifier,
}

impl RefreshCycleClassifier for BuildClassifyingRefreshCycle {
    type CycleDemand = BuildMonitoringRefreshCycleDemand;
    type CycleOutcome = BuildMonitoringRefreshCycleExecution;

    fn classify_refresh_cycle(
        &mut self,
        process_observer: &mut ProcessObserver,
        process_observation_snapshot: &ProcessObservationSnapshot,
        build_monitoring_refresh_cycle_demand: BuildMonitoringRefreshCycleDemand,
    ) -> BuildMonitoringRefreshCycleExecution {
        let (compile_classification_demand, build_termination_observation_demand) =
            build_monitoring_refresh_cycle_demand.into_parts();
        let compile_classification_execution = self.build_classifier.classify_demand(
            process_observer,
            process_observation_snapshot,
            compile_classification_demand,
            Instant::now(),
        );
        let build_termination_observation_execution =
            build_monitor::observe_build_termination_demand(
                process_observer,
                process_observation_snapshot,
                build_termination_observation_demand,
            );
        BuildMonitoringRefreshCycleExecution::new(
            compile_classification_execution,
            build_termination_observation_execution,
        )
    }
}
