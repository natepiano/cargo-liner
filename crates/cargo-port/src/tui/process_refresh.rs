//! Neutral `App` adapter for the shared process-refresh executor.
//!
//! Everything here is common to the refresh itself: the executor's deadline,
//! its request dispatch, its result receiver, and the correlation of a
//! completed cycle back to the request that asked for it. Running Targets
//! cadence and attribution live in [`crate::tui::running_targets`].

use std::time::Duration;
use std::time::Instant;

use super::app::App;
use super::startup_services::StartupEffect;
use crate::process_observation::ProcessRefreshDeadline;
use crate::process_observation::ProcessRefreshDispatchOutcome;
use crate::process_observation::ProcessRefreshExecution;
use crate::process_observation::ProcessRefreshExecutionOutcome;
use crate::process_observation::ProcessRefreshResultPoll;
use crate::process_observation::ProcessRefreshResultReceiver;

/// Whether the foreground tick received one completed observer refresh and
/// therefore has an observer duration to instrument.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum ObserverRefreshTiming {
    #[default]
    NoCompletedRefresh,
    Completed(Duration),
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

        match self.process_refresh_executor.refresh_due(now) {
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

    pub const fn process_refresh_next_deadline(&self) -> ProcessRefreshDeadline {
        self.process_refresh_executor.next_deadline()
    }

    pub const fn process_refresh_result_receiver(&self) -> ProcessRefreshResultReceiver<'_> {
        self.process_refresh_executor.result_receiver()
    }

    /// Hand one completed cycle's observation to Running Targets. A failed
    /// cycle produces no observation and no timing.
    fn reconcile_process_refresh_execution(
        &mut self,
        now: Instant,
        process_refresh_execution: ProcessRefreshExecution,
    ) -> ObserverRefreshTiming {
        let completed_process_refresh_execution = match process_refresh_execution.into_outcome() {
            ProcessRefreshExecutionOutcome::Completed(completed_process_refresh_execution) => {
                completed_process_refresh_execution
            },
            ProcessRefreshExecutionOutcome::Failed(failure) => {
                tracing::warn!(?failure, "process_refresh_execution_failed");
                return ObserverRefreshTiming::NoCompletedRefresh;
            },
        };
        let observer_refresh_timing =
            ObserverRefreshTiming::Completed(completed_process_refresh_execution.elapsed());
        let process_observation_snapshot = completed_process_refresh_execution.into_snapshot();
        self.apply_running_targets_observation(now, &process_observation_snapshot);
        observer_refresh_timing
    }
}

#[cfg(test)]
#[allow(
    clippy::panic,
    reason = "tests should fail on unexpected reconciliation states"
)]
mod tests {
    use super::*;
    use crate::process_observation::ProcessRefreshExecutionBackendSelection;
    use crate::process_observation::ProcessRefreshExecutor;
    use crate::process_observation::RunningTargetsRefreshSchedule;
    use crate::process_observation::snapshot::ProcessRefreshExecutionFailure;
    use crate::tui::startup_services::StartupServices;

    #[test]
    fn subsecond_app_ticks_skip_attribution_collection_until_due() {
        let mut app = crate::tui::test_support::make_app(&[]);
        let poll_interval = Duration::from_secs(1);
        let first_poll = Instant::now();
        app.startup_services = StartupServices::production();
        app.process_refresh_executor = ProcessRefreshExecutor::new(
            ProcessRefreshExecutionBackendSelection::Synchronous,
            RunningTargetsRefreshSchedule::Every(poll_interval),
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
            ProcessRefreshDeadline::At(first_poll + poll_interval)
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
            ProcessRefreshExecutionFailure::ResultChannelDisconnected,
        );

        assert_eq!(
            app.reconcile_process_refresh_execution(Instant::now(), process_refresh_execution),
            ObserverRefreshTiming::NoCompletedRefresh
        );
        assert_eq!(app.running_target_attribution_collection_count, 0);
    }

    #[test]
    fn a_completed_cycle_delivers_its_observation_to_running_targets() {
        let mut app = crate::tui::test_support::make_app(&[]);

        assert!(matches!(
            app.reconcile_process_refresh_execution(
                Instant::now(),
                ProcessRefreshExecution::completed_for_test()
            ),
            ObserverRefreshTiming::Completed(_)
        ));
        assert_eq!(app.running_target_attribution_collection_count, 1);
    }
}
