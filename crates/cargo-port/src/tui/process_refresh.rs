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

use crate::build_monitor::BuildScopeActionability;
use crate::build_monitor::CompileClassificationCancellation;
use crate::build_monitor::CompileClassificationDemand;
use crate::build_monitor::CompileClassificationExecution;
use crate::build_monitor::CompileMonitorGeneration;
use crate::process_observation::ProcessRefreshConsumerDemand;
use crate::process_observation::ProcessRefreshDeadline;
use crate::process_observation::ProcessRefreshDispatchOutcome;
use crate::process_observation::ProcessRefreshExecutionOutcome;
use crate::process_observation::ProcessRefreshExecutor;
use crate::process_observation::ProcessRefreshResultPoll;
use crate::process_observation::ProcessRefreshResultReceiver;
use crate::tui::app::App;
use crate::tui::background::BuildClassifyingRefreshCycle;
use crate::tui::compile_visibility::CompileVisibilityState;
use crate::tui::messages::ProcessRefreshMsg;
use crate::tui::startup_services::StartupEffect;

/// The one executor `App` owns, bound to the classifier the worker owns.
pub(crate) type AppProcessRefreshExecutor = ProcessRefreshExecutor<BuildClassifyingRefreshCycle>;

/// The borrowed worker receiver the event loop registers wakeups on.
pub(crate) type AppProcessRefreshResultReceiver<'a> =
    ProcessRefreshResultReceiver<'a, CompileClassificationExecution>;

/// Whether the foreground tick received one completed observer refresh and
/// therefore has an observer duration to instrument.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum ObserverRefreshTiming {
    #[default]
    NoCompletedRefresh,
    Completed(Duration),
}

/// Whether the shared worker currently holds a compile classification, and the
/// capability that stops it.
///
/// The capability is generation-bound, so keeping the one for the request that
/// is actually in flight is what lets a toggle or scope invalidation cancel
/// that cycle and nothing else.
#[derive(Clone, Debug, Default)]
pub(crate) enum CompileClassificationInFlight {
    #[default]
    NotRequested,
    Requested(CompileClassificationCancellation),
}

impl App {
    /// Dispatch due process work and reconcile completed immutable results.
    pub fn process_refresh_tick(&mut self, now: Instant) -> ObserverRefreshTiming {
        let mut observer_refresh_timing = ObserverRefreshTiming::NoCompletedRefresh;
        match self.process_refresh_executor.poll_result() {
            ProcessRefreshResultPoll::Ready(process_refresh_execution) => {
                observer_refresh_timing =
                    self.reconcile_process_refresh_execution(now, *process_refresh_execution);
            },
            ProcessRefreshResultPoll::Pending => {},
        }

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

    pub fn process_refresh_next_deadline(&self) -> ProcessRefreshDeadline {
        self.process_refresh_executor.next_deadline()
    }

    pub const fn process_refresh_result_receiver(&self) -> AppProcessRefreshResultReceiver<'_> {
        self.process_refresh_executor.result_receiver()
    }

    /// Stop the in-flight classification when the caller owns the generation it
    /// was requested under. A toggle or scope invalidation that advances past a
    /// different generation leaves the cycle running.
    pub(crate) fn cancel_compile_classification(
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
    ) -> ProcessRefreshDispatchOutcome<CompileClassificationExecution> {
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
                compile_classification_demand
            });
        if !matches!(
            process_refresh_dispatch_outcome,
            ProcessRefreshDispatchOutcome::NotDue
        ) {
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
                return ObserverRefreshTiming::NoCompletedRefresh;
            },
        };
        let observer_refresh_timing =
            ObserverRefreshTiming::Completed(completed_process_refresh_execution.elapsed());
        let (process_observation_snapshot, compile_classification_execution) =
            completed_process_refresh_execution.into_parts();

        self.compile_classification_in_flight = CompileClassificationInFlight::NotRequested;
        record_compile_classification_execution(compile_classification_execution);

        if demand.includes_running_targets() {
            self.apply_running_targets_observation(now, &process_observation_snapshot);
        }
        observer_refresh_timing
    }
}

/// Report what the cycle's classification produced. The monitor's own snapshot
/// state arrives with the first scheduled compile demand, so this phase records
/// the outcome rather than storing it.
fn record_compile_classification_execution(
    compile_classification_execution: CompileClassificationExecution,
) {
    match compile_classification_execution {
        CompileClassificationExecution::NotRequested => {},
        CompileClassificationExecution::Completed(build_classification) => {
            tracing::debug!(
                build_sessions = build_classification.build_sessions().len(),
                compile_activities = build_classification.compile_activities().len(),
                unattributed_compile_activities =
                    build_classification.unattributed_compile_activities().len(),
                "compile_classification_completed"
            );
        },
        #[cfg(test)]
        CompileClassificationExecution::Failed(failure) => {
            tracing::warn!(?failure, "compile_classification_failed");
        },
        CompileClassificationExecution::Cancelled(compile_monitor_generation) => {
            tracing::debug!(
                ?compile_monitor_generation,
                "compile_classification_cancelled"
            );
        },
    }
}

/// Resolve what this cycle owes the compile monitor from the monitor's own
/// enablement and scope state. Anything short of an enabled, actionable scope
/// owes nothing, which is what keeps a disabled monitor free of classification
/// work.
fn compile_classification_demand(
    demand: ProcessRefreshConsumerDemand,
    compile_visibility_state: &CompileVisibilityState,
    cargo_workspace_index: &Arc<crate::project::CargoWorkspaceIndex>,
    inflight: &crate::tui::state::Inflight,
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
    use crate::build_monitor::BuildClassificationExecutionFailure;
    use crate::build_monitor::BuildScopeKey;
    use crate::process_observation::CompileMonitorRefreshSchedule;
    use crate::process_observation::ProcessRefreshExecution;
    use crate::process_observation::ProcessRefreshExecutionBackendSelection;
    use crate::process_observation::ProcessRefreshExecutor;
    use crate::process_observation::RunningTargetsRefreshSchedule;
    use crate::process_observation::snapshot::ProcessRefreshExecutionFailure;
    use crate::project::AbsolutePath;
    use crate::tui::startup_services::StartupServices;

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

    #[test]
    fn a_failed_compile_classification_still_delivers_the_running_observation() {
        let mut app = crate::tui::test_support::make_app(&[]);
        let process_refresh_execution = ProcessRefreshExecution::completed_for_test(
            ProcessRefreshConsumerDemand::RunningTargetsAndCompileMonitor,
            CompileClassificationExecution::Failed(
                BuildClassificationExecutionFailure::AwaitingReachableCause,
            ),
        );

        assert!(matches!(
            app.reconcile_process_refresh_execution(Instant::now(), process_refresh_execution),
            ObserverRefreshTiming::Completed(_)
        ));
        assert_eq!(app.running_target_attribution_collection_count, 1);
    }

    #[test]
    fn a_cancelled_compile_classification_still_delivers_the_running_observation() {
        let mut app = crate::tui::test_support::make_app(&[]);
        let process_refresh_execution = ProcessRefreshExecution::completed_for_test(
            ProcessRefreshConsumerDemand::RunningTargetsAndCompileMonitor,
            CompileClassificationExecution::Cancelled(CompileMonitorGeneration::default()),
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
        let ProcessRefreshDispatchOutcome::Finished(process_refresh_execution) = app
            .process_refresh_executor
            .refresh_due(now, |_| compile_classification_demand)
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
        let (process_observation_snapshot, compile_classification_execution) =
            completed_process_refresh_execution.into_parts();

        assert_eq!(
            compile_classification_execution,
            CompileClassificationExecution::Cancelled(compile_monitor_generation)
        );
        app.apply_running_targets_observation(now, &process_observation_snapshot);
        assert_eq!(app.running_target_attribution_collection_count, 1);
    }

    #[test]
    fn a_compile_monitor_only_cycle_delivers_no_running_observation() {
        let mut app = crate::tui::test_support::make_app(&[]);
        let process_refresh_execution = ProcessRefreshExecution::completed_for_test(
            ProcessRefreshConsumerDemand::CompileMonitor,
            CompileClassificationExecution::NotRequested,
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
