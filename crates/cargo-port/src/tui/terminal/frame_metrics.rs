use std::time::Duration;

use tui_pane::SLOW_FRAME_MS;

use crate::tui::process_refresh::ObserverRefreshTiming;

#[derive(Clone, Copy)]
pub(super) struct FrameMetrics {
    pub(super) frame_elapsed:           Duration,
    pub(super) input_elapsed:           Duration,
    pub(super) bg_elapsed:              Duration,
    pub(super) cpu_elapsed:             Duration,
    pub(super) process_refresh_elapsed: Duration,
    pub(super) observer_refresh_timing: ObserverRefreshTiming,
    pub(super) rows_elapsed:            Duration,
    pub(super) disk_elapsed:            Duration,
    pub(super) fit_elapsed:             Duration,
    pub(super) detail_elapsed:          Duration,
    pub(super) draw_elapsed:            Duration,
    pub(super) input_count:             usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ForegroundFrameInstrumentation {
    BelowSlowThreshold,
    SlowFrame,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct FrameInstrumentationPlan {
    pub(super) observer_refresh_timing: ObserverRefreshTiming,
    pub(super) foreground_frame:        ForegroundFrameInstrumentation,
}

impl FrameMetrics {
    pub(super) const fn instrumentation_plan(&self) -> FrameInstrumentationPlan {
        let foreground_frame = if self.frame_elapsed.as_millis() < SLOW_FRAME_MS {
            ForegroundFrameInstrumentation::BelowSlowThreshold
        } else {
            ForegroundFrameInstrumentation::SlowFrame
        };
        FrameInstrumentationPlan {
            observer_refresh_timing: self.observer_refresh_timing,
            foreground_frame,
        }
    }

    #[cfg(test)]
    const fn for_test(
        frame_elapsed: Duration,
        observer_refresh_timing: ObserverRefreshTiming,
    ) -> Self {
        Self {
            frame_elapsed,
            input_elapsed: Duration::ZERO,
            bg_elapsed: Duration::ZERO,
            cpu_elapsed: Duration::ZERO,
            process_refresh_elapsed: Duration::ZERO,
            observer_refresh_timing,
            rows_elapsed: Duration::ZERO,
            disk_elapsed: Duration::ZERO,
            fit_elapsed: Duration::ZERO,
            detail_elapsed: Duration::ZERO,
            draw_elapsed: Duration::ZERO,
            input_count: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completed_observer_refresh_is_instrumented_when_foreground_frame_is_not_slow() {
        let below_slow_threshold = Duration::from_millis(
            u64::try_from(SLOW_FRAME_MS.saturating_sub(1)).unwrap_or_default(),
        );
        let observer_elapsed = Duration::from_millis(80);
        let frame_metrics = FrameMetrics::for_test(
            below_slow_threshold,
            ObserverRefreshTiming::Completed(observer_elapsed),
        );

        assert_eq!(
            frame_metrics.instrumentation_plan(),
            FrameInstrumentationPlan {
                observer_refresh_timing: ObserverRefreshTiming::Completed(observer_elapsed),
                foreground_frame:        ForegroundFrameInstrumentation::BelowSlowThreshold,
            }
        );
    }

    #[test]
    fn no_completed_observer_refresh_produces_no_performance_event_timing() {
        let frame_metrics = FrameMetrics::for_test(
            Duration::from_millis(1),
            ObserverRefreshTiming::NoCompletedRefresh,
        );

        assert_eq!(
            frame_metrics.instrumentation_plan().observer_refresh_timing,
            ObserverRefreshTiming::NoCompletedRefresh
        );
    }
}
