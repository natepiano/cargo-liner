use std::time::Duration;

use tui_pane::SLOW_FRAME_MS;

#[derive(Clone, Copy)]
pub(super) struct FrameMetrics {
    pub(super) frame_elapsed:  Duration,
    pub(super) input_elapsed:  Duration,
    pub(super) bg_elapsed:     Duration,
    pub(super) cpu_elapsed:    Duration,
    pub(super) rows_elapsed:   Duration,
    pub(super) disk_elapsed:   Duration,
    pub(super) fit_elapsed:    Duration,
    pub(super) detail_elapsed: Duration,
    pub(super) draw_elapsed:   Duration,
    pub(super) input_count:    usize,
}

impl FrameMetrics {
    pub(super) const fn is_slow(&self) -> bool { self.frame_elapsed.as_millis() >= SLOW_FRAME_MS }
}
