//! Constants for the backdrop capture and the attract-mode band.

use std::time::Duration;

// attract band
/// How wide the lit strip stands, in columns. Its leading edge is at
/// full strength and the tail this far behind it has faded out.
pub(super) const BAND_COLUMNS: u32 = 20;
/// How far the strip travels each second, in columns.
pub(super) const BAND_COLUMNS_PER_SECOND: u32 = 30;
/// How many cells are re-rolled to a new character each frame, on top
/// of the whole column the leading edge re-rolls as it arrives. Enough
/// to read as a shimmer without the strip looking like static.
pub(super) const CHURN_CELLS_PER_FRAME: usize = 3;
/// How finely the strip's position is tracked between two columns.
///
/// The strip has to move a fraction of a column per frame and its
/// trailing fade has to be smooth, and both are worked out in whole
/// numbers -- a float would put a truncating cast in the middle of
/// every cell's colour.
pub(super) const SUBCOLUMNS_PER_COLUMN: u32 = 256;

// capture
/// How often the worker takes a fresh capture.
///
/// What a capture goes stale for is the desktop behind the window
/// changing -- another window opening there, a Space switch, the
/// wallpaper turning over -- and none of that happens at anything like
/// the frame rate. The window moving does not go here: that is read
/// every frame and costs a fraction of a millisecond.
pub(super) const CAPTURE_REFRESH: Duration = Duration::from_millis(1000);
/// How soon the worker is asked again after a capture that cannot be
/// used -- one whose window has closed, moved to another display, or
/// been re-gridded by a font change.
///
/// Short enough that the animation comes back quickly, long enough that
/// a window parked off every display is not asking the window server
/// for a full capture every frame.
pub(super) const CAPTURE_RETRY: Duration = Duration::from_millis(150);
/// How many pixels are captured across and down each character cell,
/// which are then averaged into the cell's one colour.
///
/// Asking the window server for one pixel per cell would leave the
/// colour to whatever filter it downsamples with, and a bilinear one
/// reading four texels out of a fifteen-pixel cell gives a noisy
/// answer. Capturing a small block per cell and averaging it here is
/// the same box filter every time.
pub(super) const SAMPLES_PER_CELL: u32 = 4;

// glyphs
/// The characters a cell draws from.
///
/// Punctuation and line-drawing rather than letters: the strip should
/// read as texture rather than as words the eye tries to finish.
pub(super) const GLYPHS: &[char] = &[
    '/', '\\', '|', '-', '_', '=', '+', '*', ':', '.', '<', '>', '^', '~', '#', '%', '&', '?', '!',
    ';', '"', '\'', '`', '(', ')', '[', ']', '{', '}',
];

// time
/// Milliseconds in one second.
pub(super) const MILLIS_PER_SECOND: u32 = 1000;

// xorshift
/// Seed the generator falls back to when the clock offers nothing to
/// vary it by. Any non-zero value will do -- xorshift64 stays at zero
/// forever if it ever reaches it.
pub(super) const XORSHIFT_FALLBACK_SEED: u64 = 0x2545_F491_4F6C_DD1D;
/// First shift of the xorshift64 round.
pub(super) const XORSHIFT_FIRST: u32 = 13;
/// Second shift of the xorshift64 round.
pub(super) const XORSHIFT_SECOND: u32 = 7;
/// Third shift of the xorshift64 round.
pub(super) const XORSHIFT_THIRD: u32 = 17;
