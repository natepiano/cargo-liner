use std::time::Duration;

// tui_pane src theme blend
/// The xterm palette for ANSI 0-15, which is what a named
/// [`Color`](ratatui::style::Color) and the first sixteen entries of
/// [`Color::Indexed`](ratatui::style::Color::Indexed) both stand for.
///
/// A terminal profile is free to set its own sixteen and there is no
/// way to read them back, so a blend against a named colour is worked
/// out against these. The values a theme states outright are exact
/// either way.
pub(super) const ANSI_BASE_PALETTE: [(u8, u8, u8); 16] = [
    (0, 0, 0),
    (205, 0, 0),
    (0, 205, 0),
    (205, 205, 0),
    (0, 0, 238),
    (205, 0, 205),
    (0, 205, 205),
    (229, 229, 229),
    (127, 127, 127),
    (255, 0, 0),
    (0, 255, 0),
    (255, 255, 0),
    (92, 92, 255),
    (255, 0, 255),
    (0, 255, 255),
    (255, 255, 255),
];
/// First entry of the 6x6x6 colour cube, which follows the sixteen.
pub(super) const ANSI_CUBE_BASE: u8 = 16;
/// The six levels each channel of the colour cube steps through.
pub(super) const ANSI_CUBE_LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];
/// First entry of the grayscale ramp closing the palette.
pub(super) const ANSI_GRAYSCALE_BASE: u8 = 232;
/// Level the grayscale ramp opens at, on every channel at once.
pub(super) const ANSI_GRAYSCALE_START: u8 = 8;
/// How far each entry of the grayscale ramp stands above the last.
pub(super) const ANSI_GRAYSCALE_STEP: u8 = 10;
/// Fixed point the redmean weights are carried in, so
/// [`color_distance`](super::color_distance) reaches its answer without
/// a float anywhere.
pub(super) const COLOR_DISTANCE_SCALE: u32 = 256;
/// The redmean weight red and blue start from, before the shift that
/// adds to one of them whatever it takes from the other.
pub(super) const COLOR_DISTANCE_SIDE_WEIGHT: u32 = 2 * COLOR_DISTANCE_SCALE;
/// The redmean weight on green, which is the same at every point of
/// the scale -- green is where the eye reads most of a difference, and
/// it reads it the same however red the pair is.
pub(super) const COLOR_DISTANCE_GREEN_WEIGHT: u32 = 4 * COLOR_DISTANCE_SCALE;

// tui_pane src theme poller
pub(super) const BACKOFF_INTERVAL: Duration = Duration::from_secs(30);
pub(super) const BACKOFF_THRESHOLD: u32 = 10;
pub(super) const POLL_INTERVAL: Duration = Duration::from_millis(1500);
