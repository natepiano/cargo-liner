pub(super) const BYTES_PER_MIB: u64 = 1024 * 1024;
pub(super) const BYTES_PER_GIB: u64 = 1024 * 1024 * 1024;

/// Block border costs 2 horizontal characters (left + right).
pub(super) const BLOCK_BORDER_WIDTH: usize = 2;

pub(super) const OFFLINE_PULSE_CYCLE: usize = 120;
pub(super) const OFFLINE_PULSE_AMPLITUDE: f64 = 0.3;
pub(super) const OFFLINE_PULSE_OFFSET: f64 = 0.7;
pub(super) const OFFLINE_PULSE_RED: f64 = 200.0;
pub(super) const OFFLINE_PULSE_GREEN: f64 = 60.0;
pub(super) const OFFLINE_PULSE_BLUE: u8 = 60;

pub(super) const FRAME_POLL_MILLIS: u64 = 16;
pub(super) const CI_FETCH_DISPLAY_COUNT: u32 = 5;

pub(super) const SEARCH_BAR_HEIGHT: u16 = 3;
pub(super) const DETAIL_PANEL_HEIGHT: u16 = 14;
pub(super) const FINDER_POPUP_HEIGHT: u16 = 28;
pub(super) const SETTINGS_POPUP_WIDTH: u16 = 60;
pub(super) const SETTINGS_POPUP_PADDING: u16 = 6;
pub(super) const CONFIRM_DIALOG_HEIGHT: u16 = 3;
pub(super) const CI_TIMESTAMP_WIDTH: u16 = 16;

pub(super) const MAX_FINDER_RESULTS: usize = 50;

pub(super) const CI_EXTRA_ROWS: usize = 1;

pub(super) const SPINNER_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
