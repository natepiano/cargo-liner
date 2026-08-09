//! Neutral `App` adapter for the shared process-refresh executor.
//!
//! Everything here is common to every refresh consumer: the executor's
//! deadline, its request dispatch, its result receiver, the correlation of a
//! completed cycle back to the request that asked for it, and the split of that
//! cycle into independent consumer outcomes. Running Targets cadence and
//! attribution live in [`crate::tui::running_targets`]; compile-monitor scope
//! lifetime lives in [`crate::tui::compile_visibility`].

use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;

use super::app::App;
use super::background::BuildClassifyingRefreshCycle;
use super::background::ProcessTerminatorAvailability;
use super::compile_visibility::CompileVisibilityState;
use super::compile_visibility::InFlightMonitorGeneration;
use super::messages::ProcessRefreshMsg;
use super::startup_services::StartupEffect;
use super::state::Inflight;
use super::state::OwnedRunTermination;
use crate::build_monitor::BuildClassificationExecutionFailure;
use crate::build_monitor::BuildMonitoringRefreshCycleDemand;
use crate::build_monitor::BuildMonitoringRefreshCycleExecution;
use crate::build_monitor::BuildScopeActionability;
use crate::build_monitor::BuildTerminationObservationExecution;
use crate::build_monitor::CompileClassificationCancellation;
use crate::build_monitor::CompileClassificationDemand;
use crate::build_monitor::CompileClassificationExecution;
use crate::build_monitor::CompileMonitorGeneration;
use crate::build_monitor::OwnedTerminationSupport;
use crate::process_observation::ProcessRefreshConsumerDemand;
use crate::process_observation::ProcessRefreshDeadline;
use crate::process_observation::ProcessRefreshDispatchOutcome;
use crate::process_observation::ProcessRefreshExecutionOutcome;
use crate::process_observation::ProcessRefreshExecutor;
use crate::process_observation::ProcessRefreshResultPoll;
use crate::process_observation::ProcessRefreshResultReceiver;
use crate::process_observation::snapshot::ProcessRefreshExecutionFailure;
use crate::project::CargoWorkspaceIndex;

/// The one executor `App` owns, bound to the classifier the worker owns.
pub(super) type AppProcessRefreshExecutor = ProcessRefreshExecutor<BuildClassifyingRefreshCycle>;

/// The borrowed worker receiver the event loop registers wakeups on.
pub(super) type AppProcessRefreshResultReceiver<'a> =
    ProcessRefreshResultReceiver<'a, BuildMonitoringRefreshCycleExecution>;

/// Whether the foreground tick received one completed observer refresh and
/// therefore has an observer duration to instrument.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum ObserverRefreshTiming {
    #[default]
    NoCompletedRefresh,
    Completed(Duration),
}

/// Why one refresh cycle left the compile monitor without a classification.
#[derive(Clone, Copy, Debug)]
enum CompileMonitorCycleFailure<'a> {
    /// The whole refresh cycle failed, so classification never ran.
    ProcessRefresh(&'a ProcessRefreshExecutionFailure),
    /// The cycle completed and the classification it carried did not.
    BuildClassification(&'a BuildClassificationExecutionFailure),
}

/// Whether the shared worker currently holds a compile classification, and the
/// capability that stops it.
///
/// The capability is generation-bound, so keeping the one for the request that
/// is actually in flight is what lets a toggle or scope invalidation cancel
/// that cycle and nothing else.
#[derive(Clone, Debug, Default)]
pub(super) enum CompileClassificationInFlight {
    #[default]
    NotRequested,
    Requested(CompileClassificationCancellation),
}

impl App {
    /// Dispatch due process work and reconcile completed immutable results.
    pub fn process_refresh_tick(&mut self, now: Instant) -> ObserverRefreshTiming {
        let mut observer_refresh_timing = ObserverRefreshTiming::NoCompletedRefresh;
        self.push_compile_monitor_schedule();
        self.push_termination_transaction_schedule();
        match self.process_refresh_executor.poll_result() {
            ProcessRefreshResultPoll::Ready(process_refresh_execution) => {
                observer_refresh_timing =
                    self.reconcile_process_refresh_execution(now, *process_refresh_execution);
            },
            ProcessRefreshResultPoll::Pending => {},
        }
        self.push_termination_transaction_schedule();

        let running_targets_polling_effect = self.startup_services.running_targets_polling_effect();
        if running_targets_polling_effect == StartupEffect::Suppressed {
            self.startup_services
                .record_running_targets_polling(running_targets_polling_effect);
            return observer_refresh_timing;
        }

        match self.dispatch_due_process_refresh(now) {
            ProcessRefreshDispatchOutcome::Finished(process_refresh_execution) => {
                self.startup_services
                    .record_running_targets_polling(running_targets_polling_effect);
                observer_refresh_timing =
                    self.reconcile_process_refresh_execution(now, *process_refresh_execution);
            },
            ProcessRefreshDispatchOutcome::AwaitingWorker(_) => {
                self.startup_services
                    .record_running_targets_polling(running_targets_polling_effect);
            },
            ProcessRefreshDispatchOutcome::NotDue => {},
        }
        observer_refresh_timing
    }

    /// Hand the executor whatever deadline the monitor's own scheduling intent
    /// currently implies. A disabled monitor pushes `NotScheduled`, so no
    /// compile-specific wakeup survives a toggle off.
    pub(super) fn push_compile_monitor_schedule(&mut self) {
        let compile_monitor_refresh_schedule = self
            .compile_visibility_state
            .compile_monitor_refresh_schedule();
        self.process_refresh_executor
            .rearm_compile_monitor(compile_monitor_refresh_schedule);
    }

    const fn push_termination_transaction_schedule(&mut self) {
        self.process_refresh_executor
            .rearm_termination_transaction(self.build_monitor.termination_refresh_schedule());
    }

    pub fn process_refresh_next_deadline(&self) -> ProcessRefreshDeadline {
        self.process_refresh_executor.next_deadline()
    }

    pub const fn process_refresh_result_receiver(&self) -> AppProcessRefreshResultReceiver<'_> {
        self.process_refresh_executor.result_receiver()
    }

    /// Stop the in-flight classification when the caller owns the generation it
    /// was requested under. A toggle or scope invalidation that advances past a
    /// different generation leaves the cycle running.
    pub(super) fn cancel_compile_classification(
        &self,
        compile_monitor_generation: CompileMonitorGeneration,
    ) {
        if let CompileClassificationInFlight::Requested(cancellation) =
            &self.compile_classification_in_flight
        {
            let _ = cancellation.cancel(compile_monitor_generation);
        }
    }

    /// Build the per-cycle demand and remember the cancellation capability for
    /// whichever request actually goes out.
    fn dispatch_due_process_refresh(
        &mut self,
        now: Instant,
    ) -> ProcessRefreshDispatchOutcome<BuildMonitoringRefreshCycleExecution> {
        let compile_classification_demand = |demand| {
            compile_classification_demand(
                demand,
                &self.compile_visibility_state,
                &self.cargo_workspace_index,
                &self.inflight,
            )
        };
        let mut requested_cancellation = CompileClassificationInFlight::NotRequested;
        let process_refresh_dispatch_outcome =
            self.process_refresh_executor.refresh_due(now, |demand| {
                let compile_classification_demand = compile_classification_demand(demand);
                if let CompileClassificationDemand::Requested { cancellation, .. } =
                    &compile_classification_demand
                {
                    requested_cancellation =
                        CompileClassificationInFlight::Requested(cancellation.clone());
                }
                BuildMonitoringRefreshCycleDemand::new(
                    compile_classification_demand,
                    self.build_monitor.termination_observation_demand(demand),
                )
            });
        if !matches!(
            process_refresh_dispatch_outcome,
            ProcessRefreshDispatchOutcome::NotDue
        ) {
            if matches!(
                requested_cancellation,
                CompileClassificationInFlight::Requested(_)
            ) {
                self.compile_visibility_state.record_refresh_dispatch(now);
                self.push_compile_monitor_schedule();
            }
            self.compile_classification_in_flight = requested_cancellation;
        }
        process_refresh_dispatch_outcome
    }

    /// Split one completed cycle into its independent consumer outcomes. A
    /// failed or cancelled classification never withholds the observation from
    /// Running Targets.
    fn reconcile_process_refresh_execution(
        &mut self,
        now: Instant,
        process_refresh_execution: ProcessRefreshMsg,
    ) -> ObserverRefreshTiming {
        let demand = process_refresh_execution.demand();
        let completed_process_refresh_execution = match process_refresh_execution.into_outcome() {
            ProcessRefreshExecutionOutcome::Completed(completed_process_refresh_execution) => {
                completed_process_refresh_execution
            },
            ProcessRefreshExecutionOutcome::Failed(failure) => {
                tracing::warn!(?failure, "process_refresh_execution_failed");
                self.compile_classification_in_flight = CompileClassificationInFlight::NotRequested;
                if let InFlightMonitorGeneration::InFlight(compile_monitor_generation) =
                    self.compile_visibility_state.in_flight_generation()
                {
                    self.age_compile_monitor_for_failed_cycle(
                        now,
                        compile_monitor_generation,
                        CompileMonitorCycleFailure::ProcessRefresh(&failure),
                    );
                }
                return ObserverRefreshTiming::NoCompletedRefresh;
            },
        };
        let observer_refresh_timing =
            ObserverRefreshTiming::Completed(completed_process_refresh_execution.elapsed());
        let (process_observation_snapshot, build_monitoring_refresh_cycle_execution) =
            completed_process_refresh_execution.into_parts();
        let (compile_classification_execution, build_termination_observation_execution) =
            build_monitoring_refresh_cycle_execution.into_parts();

        self.compile_classification_in_flight = CompileClassificationInFlight::NotRequested;
        self.store_compile_classification_execution(now, compile_classification_execution);
        self.reconcile_build_termination_observation(build_termination_observation_execution);

        if demand.includes_running_targets() {
            self.apply_running_targets_observation(now, &process_observation_snapshot);
        }
        observer_refresh_timing
    }

    /// Store what the cycle's classification produced, refusing any result
    /// whose generation the monitor has already moved past. A cancelled or
    /// superseded generation neither updates the snapshot nor rearms the
    /// cadence, which is what stops a cancelled cycle from scheduling more.
    fn store_compile_classification_execution(
        &mut self,
        now: Instant,
        compile_classification_execution: CompileClassificationExecution,
    ) {
        match compile_classification_execution {
            CompileClassificationExecution::NotRequested => {},
            CompileClassificationExecution::Completed(completed_build_classification) => {
                if !self
                    .compile_visibility_state
                    .accepts_generation(completed_build_classification.compile_monitor_generation())
                {
                    tracing::debug!("compile_classification_superseded");
                    return;
                }
                tracing::debug!(
                    build_sessions = completed_build_classification
                        .classification()
                        .build_sessions()
                        .len(),
                    compile_activities = completed_build_classification
                        .classification()
                        .compile_activities()
                        .len(),
                    unattributed_compile_activities = completed_build_classification
                        .classification()
                        .unattributed_compile_activities()
                        .len(),
                    "compile_classification_completed"
                );
                self.build_monitor
                    .record_classification(completed_build_classification);
                self.compile_visibility_state.record_refresh_settled(now);
                self.push_compile_monitor_schedule();
            },
            CompileClassificationExecution::Failed {
                compile_monitor_generation,
                build_classification_execution_failure,
            } => {
                self.age_compile_monitor_for_failed_cycle(
                    now,
                    compile_monitor_generation,
                    CompileMonitorCycleFailure::BuildClassification(
                        &build_classification_execution_failure,
                    ),
                );
            },
            CompileClassificationExecution::Cancelled(compile_monitor_generation) => {
                tracing::debug!(
                    ?compile_monitor_generation,
                    "compile_classification_cancelled"
                );
            },
        }
    }

    fn reconcile_build_termination_observation(
        &mut self,
        build_termination_observation_execution: BuildTerminationObservationExecution,
    ) {
        let ProcessTerminatorAvailability::Available(process_terminator) =
            self.background.available_process_terminator()
        else {
            return;
        };
        let build_termination_completion_transition =
            self.build_monitor.reconcile_termination_observation(
                build_termination_observation_execution,
                process_terminator,
            );
        self.consume_build_termination_completion_transition(
            build_termination_completion_transition,
        );
    }

    /// Age what the monitor shows for a cycle that produced no classification,
    /// but only when the monitor is still waiting on the generation that cycle
    /// ran under. A failure the monitor has moved past would otherwise blank the
    /// rows the new generation is retaining.
    fn age_compile_monitor_for_failed_cycle(
        &mut self,
        now: Instant,
        compile_monitor_generation: CompileMonitorGeneration,
        compile_monitor_cycle_failure: CompileMonitorCycleFailure<'_>,
    ) {
        if !self
            .compile_visibility_state
            .accepts_generation(compile_monitor_generation)
        {
            tracing::debug!(
                ?compile_monitor_cycle_failure,
                "compile_classification_failure_superseded"
            );
            return;
        }
        match compile_monitor_cycle_failure {
            CompileMonitorCycleFailure::ProcessRefresh(process_refresh_execution_failure) => {
                tracing::warn!(
                    ?process_refresh_execution_failure,
                    "compile_monitor_cycle_failed"
                );
            },
            CompileMonitorCycleFailure::BuildClassification(
                build_classification_execution_failure,
            ) => {
                tracing::warn!(
                    ?build_classification_execution_failure,
                    "compile_classification_failed"
                );
            },
        }
        self.build_monitor.record_classification_failure();
        self.compile_visibility_state.record_refresh_settled(now);
        self.push_compile_monitor_schedule();
    }
}

/// Resolve what this cycle owes the compile monitor from the monitor's own
/// enablement and scope state. Anything short of an enabled, actionable scope
/// owes nothing, which is what keeps a disabled monitor free of classification
/// work.
fn compile_classification_demand(
    demand: ProcessRefreshConsumerDemand,
    compile_visibility_state: &CompileVisibilityState,
    cargo_workspace_index: &Arc<CargoWorkspaceIndex>,
    inflight: &Inflight,
) -> CompileClassificationDemand {
    if !demand.includes_compile_monitor() {
        return CompileClassificationDemand::NotRequested;
    }
    let CompileVisibilityState::On(active_monitor_state) = compile_visibility_state else {
        return CompileClassificationDemand::NotRequested;
    };
    let BuildScopeActionability::Actionable(build_scope_key) =
        active_monitor_state.build_scope_actionability()
    else {
        return CompileClassificationDemand::NotRequested;
    };
    let compile_monitor_generation = active_monitor_state.compile_monitor_generation();
    CompileClassificationDemand::Requested {
        compile_monitor_generation,
        build_scope_key,
        cargo_workspace_index: Arc::clone(cargo_workspace_index),
        owned_root_evidence: inflight.owned_run().owned_root_evidence(),
        owned_termination_support: match inflight.owned_run_termination() {
            OwnedRunTermination::Available {
                owned_run_id,
                owned_run_termination_token,
            } => OwnedTerminationSupport::Actionable {
                owned_run_id,
                owned_run_termination_token,
            },
            OwnedRunTermination::RequestPending { .. } | OwnedRunTermination::NoRunningRun => {
                OwnedTerminationSupport::Unavailable
            },
        },
        cancellation: CompileClassificationCancellation::for_generation(compile_monitor_generation),
    }
}

#[cfg(test)]
#[allow(
    clippy::panic,
    reason = "tests should fail on unexpected reconciliation states"
)]
mod tests {
    use super::*;
    use crate::build_monitor::BuildScopeKey;
    use crate::build_monitor::BuildTerminationObservationDemand;
    use crate::build_monitor::CompletedBuildClassification;
    use crate::build_monitor::MonitorSnapshot;
    use crate::process_observation::CompileMonitorRefreshSchedule;
    use crate::process_observation::ProcessRefreshExecution;
    use crate::process_observation::ProcessRefreshExecutionBackendSelection;
    use crate::process_observation::ProcessRefreshExecutor;
    use crate::process_observation::RunningTargetsRefreshSchedule;
    use crate::process_observation::snapshot::ProcessRefreshExecutionFailure;
    use crate::project::AbsolutePath;
    use crate::tui::startup_services::StartupServices;

    fn completed_cycle(
        compile_classification_execution: CompileClassificationExecution,
    ) -> BuildMonitoringRefreshCycleExecution {
        BuildMonitoringRefreshCycleExecution::new(
            compile_classification_execution,
            BuildTerminationObservationExecution::NotRequested,
        )
    }

    #[test]
    fn subsecond_app_ticks_skip_attribution_collection_until_due() {
        let mut app = crate::tui::test_support::make_app(&[]);
        let poll_interval = Duration::from_secs(1);
        let first_poll = Instant::now();
        app.startup_services = StartupServices::production();
        app.process_refresh_executor = ProcessRefreshExecutor::new(
            ProcessRefreshExecutionBackendSelection::Synchronous,
            BuildClassifyingRefreshCycle::default(),
            RunningTargetsRefreshSchedule::Every(poll_interval),
            CompileMonitorRefreshSchedule::NotScheduled,
            first_poll,
        );

        assert!(matches!(
            app.process_refresh_tick(first_poll),
            ObserverRefreshTiming::Completed(_)
        ));
        let rebuild_count = app.cargo_workspace_index.rebuild_count();
        assert_eq!(app.running_target_attribution_collection_count, 1);
        assert_eq!(
            app.process_refresh_next_deadline(),
            crate::process_observation::ProcessRefreshDeadline::At(first_poll + poll_interval)
        );

        app.process_refresh_tick(first_poll + poll_interval / 4);
        app.process_refresh_tick(
            first_poll + poll_interval.saturating_sub(Duration::from_millis(1)),
        );

        assert_eq!(app.running_target_attribution_collection_count, 1);
        assert_eq!(app.cargo_workspace_index.rebuild_count(), rebuild_count);

        app.process_refresh_tick(first_poll + poll_interval);

        assert_eq!(app.running_target_attribution_collection_count, 2);
        assert_eq!(app.cargo_workspace_index.rebuild_count(), rebuild_count);
    }

    #[test]
    fn request_channel_failure_has_no_completed_observer_timing() {
        let mut app = crate::tui::test_support::make_app(&[]);
        let process_refresh_execution = ProcessRefreshExecution::failed_for_test(
            ProcessRefreshConsumerDemand::RunningTargets,
            ProcessRefreshExecutionFailure::RequestChannelDisconnected,
        );

        assert_eq!(
            app.reconcile_process_refresh_execution(Instant::now(), process_refresh_execution),
            ObserverRefreshTiming::NoCompletedRefresh
        );
        assert_eq!(app.running_target_attribution_collection_count, 0);
    }

    #[test]
    fn result_channel_failure_has_no_completed_observer_timing() {
        let mut app = crate::tui::test_support::make_app(&[]);
        let process_refresh_execution = ProcessRefreshExecution::failed_for_test(
            ProcessRefreshConsumerDemand::RunningTargets,
            ProcessRefreshExecutionFailure::ResultChannelDisconnected,
        );

        assert_eq!(
            app.reconcile_process_refresh_execution(Instant::now(), process_refresh_execution),
            ObserverRefreshTiming::NoCompletedRefresh
        );
        assert_eq!(app.running_target_attribution_collection_count, 0);
    }

    /// Enable the monitor and hand back the generation the toggle advanced to.
    fn enable_compile_monitor(app: &mut App, now: Instant) -> CompileMonitorGeneration {
        app.toggle_compile_visibility(now);
        match &app.compile_visibility_state {
            CompileVisibilityState::On(active_monitor_state) => {
                active_monitor_state.compile_monitor_generation()
            },
            CompileVisibilityState::Off => panic!("the toggle enables the monitor"),
        }
    }

    /// The failure belongs to the classification alone: Running Targets still
    /// applies the same cycle's observation, and the monitor the failure does
    /// belong to ages what it was showing by one step.
    #[test]
    fn a_failed_compile_classification_still_delivers_the_running_observation() {
        let mut app = crate::tui::test_support::make_app(&[]);
        let now = Instant::now();
        let compile_monitor_generation = enable_compile_monitor(&mut app, now);
        let process_refresh_execution = ProcessRefreshExecution::completed_for_test(
            ProcessRefreshConsumerDemand::RunningTargetsAndCompileMonitor,
            completed_cycle(CompileClassificationExecution::Failed {
                compile_monitor_generation,
                build_classification_execution_failure:
                    BuildClassificationExecutionFailure::NoIndexedWorkspace,
            }),
        );

        assert!(matches!(
            app.reconcile_process_refresh_execution(now, process_refresh_execution),
            ObserverRefreshTiming::Completed(_)
        ));
        assert_eq!(app.running_target_attribution_collection_count, 1);
        assert_eq!(
            *app.build_monitor.monitor_snapshot(),
            MonitorSnapshot::Unavailable,
            "a failure the monitor is still waiting on ages what it shows"
        );
    }

    /// A failure from a generation the monitor has moved past leaves the new
    /// generation's snapshot alone: it would otherwise blank rows the current
    /// scope is still waiting to fill.
    #[test]
    fn a_superseded_compile_classification_failure_ages_nothing() {
        let mut app = crate::tui::test_support::make_app(&[]);
        let now = Instant::now();
        let superseded_generation = enable_compile_monitor(&mut app, now);
        app.toggle_compile_visibility(now);
        let current_generation = enable_compile_monitor(&mut app, now);
        let process_refresh_execution = ProcessRefreshExecution::completed_for_test(
            ProcessRefreshConsumerDemand::RunningTargetsAndCompileMonitor,
            completed_cycle(CompileClassificationExecution::Failed {
                compile_monitor_generation:             superseded_generation,
                build_classification_execution_failure:
                    BuildClassificationExecutionFailure::NoIndexedWorkspace,
            }),
        );

        assert_ne!(superseded_generation, current_generation);
        app.reconcile_process_refresh_execution(now, process_refresh_execution);

        assert_eq!(
            *app.build_monitor.monitor_snapshot(),
            MonitorSnapshot::Pending,
            "the monitor has moved past the generation that failed"
        );
    }

    /// A completion carries the generation it was classified under, so one that
    /// arrives after a toggle is dropped rather than shown under the scope the
    /// monitor moved to.
    #[test]
    fn a_superseded_completed_classification_is_refused() {
        let mut app = crate::tui::test_support::make_app(&[]);
        let now = Instant::now();
        let superseded_generation = enable_compile_monitor(&mut app, now);
        app.toggle_compile_visibility(now);
        let current_generation = enable_compile_monitor(&mut app, now);
        let superseded_completion = ProcessRefreshExecution::completed_for_test(
            ProcessRefreshConsumerDemand::RunningTargetsAndCompileMonitor,
            completed_cycle(CompileClassificationExecution::Completed(
                CompletedBuildClassification::empty_for_test(
                    superseded_generation,
                    BuildScopeKey::for_test(AbsolutePath::from(std::path::Path::new("/"))),
                    now,
                ),
            )),
        );

        app.reconcile_process_refresh_execution(now, superseded_completion);

        assert_eq!(
            *app.build_monitor.monitor_snapshot(),
            MonitorSnapshot::Pending,
            "a completion from a superseded generation records nothing"
        );

        let current_completion = ProcessRefreshExecution::completed_for_test(
            ProcessRefreshConsumerDemand::RunningTargetsAndCompileMonitor,
            completed_cycle(CompileClassificationExecution::Completed(
                CompletedBuildClassification::empty_for_test(
                    current_generation,
                    BuildScopeKey::for_test(AbsolutePath::from(std::path::Path::new("/"))),
                    now,
                ),
            )),
        );

        app.reconcile_process_refresh_execution(now, current_completion);

        assert!(
            matches!(
                app.build_monitor.monitor_snapshot(),
                MonitorSnapshot::Fresh(_)
            ),
            "the generation the monitor accepts is the one it stores"
        );
    }

    #[test]
    fn a_cancelled_compile_classification_still_delivers_the_running_observation() {
        let mut app = crate::tui::test_support::make_app(&[]);
        let process_refresh_execution = ProcessRefreshExecution::completed_for_test(
            ProcessRefreshConsumerDemand::RunningTargetsAndCompileMonitor,
            completed_cycle(CompileClassificationExecution::Cancelled(
                CompileMonitorGeneration::default(),
            )),
        );

        assert!(matches!(
            app.reconcile_process_refresh_execution(Instant::now(), process_refresh_execution),
            ObserverRefreshTiming::Completed(_)
        ));
        assert_eq!(app.running_target_attribution_collection_count, 1);
    }

    /// Cancelling the generation the cycle was requested under stops the
    /// classification and nothing else: the same cycle's observation is still
    /// there for Running Targets to apply.
    #[test]
    fn a_cancelled_generation_stops_only_the_classification() {
        let mut app = crate::tui::test_support::make_app(&[]);
        let now = Instant::now();
        let mut compile_monitor_generation = CompileMonitorGeneration::default();
        compile_monitor_generation.advance();
        let cancellation =
            CompileClassificationCancellation::for_generation(compile_monitor_generation);
        let compile_classification_demand = CompileClassificationDemand::Requested {
            compile_monitor_generation,
            build_scope_key: BuildScopeKey::for_test(AbsolutePath::from(std::path::Path::new("/"))),
            cargo_workspace_index: Arc::clone(&app.cargo_workspace_index),
            owned_root_evidence: app.inflight.owned_run().owned_root_evidence(),
            owned_termination_support: OwnedTerminationSupport::Unavailable,
            cancellation: cancellation.clone(),
        };
        app.process_refresh_executor = ProcessRefreshExecutor::new(
            ProcessRefreshExecutionBackendSelection::Synchronous,
            BuildClassifyingRefreshCycle::default(),
            RunningTargetsRefreshSchedule::Every(Duration::from_secs(1)),
            CompileMonitorRefreshSchedule::At(now),
            now,
        );
        app.compile_classification_in_flight =
            CompileClassificationInFlight::Requested(cancellation);

        app.cancel_compile_classification(compile_monitor_generation);
        let ProcessRefreshDispatchOutcome::Finished(process_refresh_execution) =
            app.process_refresh_executor.refresh_due(now, |_| {
                BuildMonitoringRefreshCycleDemand::new(
                    compile_classification_demand,
                    BuildTerminationObservationDemand::NotRequested,
                )
            })
        else {
            panic!("the synchronous backend finishes the cycle it dispatches")
        };

        assert!(
            process_refresh_execution
                .demand()
                .includes_running_targets()
        );
        let ProcessRefreshExecutionOutcome::Completed(completed_process_refresh_execution) =
            process_refresh_execution.into_outcome()
        else {
            panic!("a synchronous cycle completes")
        };
        let (process_observation_snapshot, build_monitoring_refresh_cycle_execution) =
            completed_process_refresh_execution.into_parts();
        let (compile_classification_execution, build_termination_observation_execution) =
            build_monitoring_refresh_cycle_execution.into_parts();

        assert!(matches!(
            compile_classification_execution,
            CompileClassificationExecution::Cancelled(returned_generation)
                if returned_generation == compile_monitor_generation
        ));
        assert!(matches!(
            build_termination_observation_execution,
            BuildTerminationObservationExecution::NotRequested
        ));
        app.apply_running_targets_observation(now, &process_observation_snapshot);
        assert_eq!(app.running_target_attribution_collection_count, 1);
    }

    #[test]
    fn a_compile_monitor_only_cycle_delivers_no_running_observation() {
        let mut app = crate::tui::test_support::make_app(&[]);
        let process_refresh_execution = ProcessRefreshExecution::completed_for_test(
            ProcessRefreshConsumerDemand::CompileMonitor,
            completed_cycle(CompileClassificationExecution::NotRequested),
        );

        assert!(matches!(
            app.reconcile_process_refresh_execution(Instant::now(), process_refresh_execution),
            ObserverRefreshTiming::Completed(_)
        ));
        assert_eq!(app.running_target_attribution_collection_count, 0);
    }

    #[test]
    fn a_disabled_compile_monitor_owes_the_cycle_no_classification() {
        let app = crate::tui::test_support::make_app(&[]);

        assert!(matches!(
            compile_classification_demand(
                ProcessRefreshConsumerDemand::RunningTargetsAndCompileMonitor,
                &app.compile_visibility_state,
                &app.cargo_workspace_index,
                &app.inflight,
            ),
            CompileClassificationDemand::NotRequested
        ));
    }

    #[test]
    fn a_running_targets_only_cycle_owes_the_compile_monitor_no_classification() {
        let app = crate::tui::test_support::make_app(&[]);

        assert!(matches!(
            compile_classification_demand(
                ProcessRefreshConsumerDemand::RunningTargets,
                &app.compile_visibility_state,
                &app.cargo_workspace_index,
                &app.inflight,
            ),
            CompileClassificationDemand::NotRequested
        ));
    }
}
