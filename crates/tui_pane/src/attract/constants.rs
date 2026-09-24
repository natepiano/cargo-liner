//! Constants for the attract screen: how fast it fades and steps, how
//! long it waits, what it says when the desktop cannot be drawn, and
//! the keymap tables its keys are read from.

use std::time::Duration;

// attract screen
/// How far the attract screen's strip is carried toward or away from
/// full strength each frame. Divides the range exactly, so the fade
/// lands on either end rather than saturating out its last frame, and
/// at the frame poll's cadence it crosses the whole thing in a little
/// over half a second -- long enough to read as the screen changing
/// hands rather than as a cut.
pub(super) const ATTRACT_FADE_STEP: u8 = 3;

/// How long the grid has to stand empty before the attract screen
/// comes back to it.
///
/// The screen leaves the moment a command turns up and does not turn
/// back part way, so without this a command that starts and stops
/// inside a couple of seconds would hand the terminal over and take it
/// again, over and over. A command that finishes keeps its cell for the
/// configured spell before it goes; this is the quiet after that, and
/// it is long enough that a watcher firing every few seconds keeps the
/// grid rather than trading it back and forth with the animation. The
/// screen the reader asks for outright waits out none of it.
pub(super) const ATTRACT_RETURN_QUIET: Duration = Duration::from_secs(3);
/// How long the attract screen may want a desktop capture and have none
/// before it says so.
///
/// The macOS backend may take several seconds to start its first capture, while the KDE wallpaper
/// backend normally answers much sooner. Ten seconds covers one stalled first attempt and the
/// retry that follows it, so a shorter gap is normal capture startup. A backdrop that never arrives
/// is still reported promptly enough for an ambient screen.
pub(super) const ATTRACT_BACKDROP_GRACE: Duration = Duration::from_secs(10);
/// What the screen says once unusable workers consume every automatic replacement.
pub(super) const ATTRACT_BACKDROP_RECOVERY_STOPPED_NOTICE: &str =
    "attract: desktop capture recovery stopped -- worker replacement limit reached";
/// What the screen says after a stalled capture worker is abandoned and replaced.
pub(super) const ATTRACT_BACKDROP_STALLED_NOTICE: &str =
    "attract: desktop capture stalled -- retrying with a replacement capture worker";
/// What the screen says when desktop capture is unavailable for a reason
/// the user cannot grant their way out of. The frame log is off unless
/// its variable is set, so the line names the variable rather than
/// promising a recording an ordinary run never makes.
pub(super) const ATTRACT_BACKDROP_UNAVAILABLE_NOTICE: &str =
    "attract: desktop capture unavailable -- set CARGO_TILE_FRAME_LOG to record why";
/// What the screen says while it has no desktop to draw in the colours
/// of because Screen Recording access is not granted. The permission
/// belongs to the terminal the app is drawn in rather than to the app
/// itself.
pub(super) const ATTRACT_NO_BACKDROP_NOTICE: &str = "attract: no desktop capture -- allow Screen Recording for this terminal in System Settings \u{203a} Privacy & Security";
/// TOML table the moving band's keys are read from and written back to.
/// Stable -- `keymap.toml` is hand-edited.
pub(super) const ATTRACT_MOVING_BAND_SCOPE: &str = "attract_moving_band";
/// Section heading the keymap overlay gives the moving band's keys.
pub(super) const ATTRACT_MOVING_BAND_SECTION: &str = "Attract: Moving Band";
/// TOML table the drifting text's keys are read from and written back
/// to. Stable -- `keymap.toml` is hand-edited.
pub(super) const ATTRACT_MOVING_TEXT_SCOPE: &str = "attract_moving_text";
/// Section heading the keymap overlay gives the drifting text's keys.
pub(super) const ATTRACT_MOVING_TEXT_SECTION: &str = "Attract: Moving Text";
/// TOML table the pixelate screen's keys are read from and written back
/// to. Stable -- `keymap.toml` is hand-edited.
pub(super) const ATTRACT_PIXELATE_SCOPE: &str = "attract_pixelate";
/// Section heading the keymap overlay gives the pixelate screen's keys.
pub(super) const ATTRACT_PIXELATE_SECTION: &str = "Attract: Pixelate";
/// Cells per second one step of the faster / slower keys moves the
/// band.
pub(super) const BAND_SPEED_STEP: u32 = 2;
/// How much one step of the fray-faster / fray-slower keys moves the
/// band's trailing edge, on the per-second scale that speed is held in.
pub(super) const BAND_TAIL_SPEED_STEP: u32 = 24;
/// Cells one step of the wider / thinner keys moves the band.
pub(super) const BAND_WIDTH_STEP: u32 = 1;
/// Columns one step of the coarsen / sharpen keys moves the pixelate
/// screen's blocks.
///
/// One, where the band's width step is also one: a block's rows are
/// taken from its columns, so a step of two would carry a block a whole
/// row deeper on every other press and the key would read as uneven.
pub(super) const PIXEL_BLOCK_STEP: u32 = 1;
/// Cells per second one step of the faster / slower keys moves the
/// pixelate screen's wave.
pub(super) const PIXEL_SPEED_STEP: u32 = 2;
/// Percentage points one step of the wave keys moves how much of the
/// field the pixelate screen's coarseness covers.
pub(super) const PIXEL_WAVE_STEP: u32 = 5;
/// Cells per second one step of the faster / slower keys moves the
/// drifting text. Smaller than the band's step because the range it
/// walks is half as long.
pub(super) const TEXT_SPEED_STEP: u32 = 1;
/// Percentage points one step of the spread keys moves how far the
/// text's lines' own speeds stand from the field's.
pub(super) const TEXT_SPREAD_STEP: u32 = 5;
/// Longest gap between two presses of the same steering key that still
/// reads as the key being held down. A terminal's own auto-repeat runs
/// at roughly twice this rate once it gets going, and a reader tapping
/// a key deliberately is slower than it.
pub(super) const HELD_KEY_GAP: Duration = Duration::from_millis(200);
/// Most steps one press of a held steering key is ever worth.
pub(super) const HELD_KEY_MAX_STEP: u32 = 8;
/// Presses a held steering key spends at each step size before it is
/// worth one more.
pub(super) const HELD_KEY_PRESSES_PER_STEP: u32 = 4;
/// How often the attract screen asks for a frame while it is showing.
///
/// It draws at every poll otherwise, which is 125 frames a second now
/// that a frame of it is small enough for the terminal to take them
/// that fast. Nothing it animates needs the rate: the drifting text
/// travels a dozen cells a second and the band thirty, so 30 frames a
/// second is already more than either has motion for.
///
/// What the other ninety-five cost is the emulator's, not ours. Every
/// frame of the attract screen is every cell of the window, so the
/// terminal parses a whole screen of colour changes for each one
/// whether or not the picture moved enough between two of them to be
/// told apart -- and that parsing is where the CPU goes.
pub(super) const ATTRACT_FRAME_INTERVAL: Duration = Duration::from_millis(33);

// notice toasts
/// Interior lines a notice toast keeps even when its body is one line,
/// so entrance and exit animate over a stable height.
pub(super) const NOTICE_TOAST_MIN_INTERIOR_LINES: usize = 1;
/// How long a notice toast stays visible before it starts to exit.
pub(super) const NOTICE_TOAST_VISIBLE: Duration = Duration::from_secs(5);

// random
/// Second multiplier in `SplitMix64`'s finalizer.
pub(super) const SPLITMIX_FIRST_MULTIPLIER: u64 = 0xbf58_476d_1ce4_e5b9;
/// Odd increment `SplitMix64` adds to its state before each draw.
pub(super) const SPLITMIX_INCREMENT: u64 = 0x9e37_79b9_7f4a_7c15;
/// Third multiplier in `SplitMix64`'s finalizer.
pub(super) const SPLITMIX_SECOND_MULTIPLIER: u64 = 0x94d0_49bb_1331_11eb;
