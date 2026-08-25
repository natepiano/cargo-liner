//! Constants for the backdrop capture and the attract-mode band.

use std::time::Duration;

// attract band
/// How many cells are re-rolled to a new character each frame, on top
/// of the whole line the leading edge re-rolls as it arrives. Enough
/// to read as a shimmer without the strip looking like static.
pub(super) const CHURN_CELLS_PER_FRAME: usize = 3;
/// How far the strip travels each second, in cells, before anything
/// has sped it up or slowed it down.
pub(super) const DEFAULT_BAND_SPEED: u32 = 30;
/// How deep the lit strip stands, in cells along the axis it travels,
/// before anything has widened or thinned it. Cells this far behind
/// the leading edge are the last ones drawn.
pub(super) const DEFAULT_BAND_WIDTH: u32 = 20;
/// Fastest the strip travels. Past this it crosses the grid inside a
/// couple of frames, which reads as a flicker rather than as travel.
pub(super) const MAX_BAND_SPEED: u32 = 400;
/// Widest the strip stands. The strip wraps, so past the number of
/// lines the grid has there is no gap left between the tail and the
/// leading edge and the whole grid is lit at once -- which is a
/// reasonable place to be able to get to, and not a band.
pub(super) const MAX_BAND_WIDTH: u32 = 200;
/// Slowest the strip travels. Zero is not offered: a strip that never
/// moves is one the reader cannot tell from a frozen display.
pub(super) const MIN_BAND_SPEED: u32 = 1;
/// Thinnest the strip stands: a single line at full strength with no
/// tail behind it.
pub(super) const MIN_BAND_WIDTH: u32 = 1;
/// How finely the strip's position is tracked between one cell and the
/// next.
///
/// The strip has to move a fraction of a cell per frame and its
/// trailing fade has to be smooth, and both are worked out in whole
/// numbers -- a float would put a truncating cast in the middle of
/// every cell's colour.
pub(super) const SUBCELLS_PER_CELL: u32 = 256;
/// How shallow the strip can run back at one offset across itself once
/// its trailing edge is varying, as a percentage of its width.
///
/// The strip keeps a core this deep everywhere and varies only behind
/// that, so a ragged trailing edge frays the strip rather than breaking
/// it into pieces the eye reads as separate.
pub(super) const VARIABLE_TAIL_FLOOR_PERCENT: u32 = 30;
/// How long one offset across the strip stands at the depth it was last
/// sent to before a new one is drawn for it.
///
/// Long enough to be read as a depth the strip is holding rather than
/// as a moment it passed through on the way somewhere else.
pub(super) const VARIABLE_TAIL_HOLD: Duration = Duration::from_secs(2);
/// How fast an offset travels toward the depth it was last sent to, on
/// the [`u8`] scale that depth is held in, per second.
///
/// The whole range takes a little under three seconds to cross, so a
/// varying trailing edge is something the eye follows rather than
/// something that has already changed by the time it is looked at.
pub(super) const VARIABLE_TAIL_TRAVEL_PER_SECOND: u32 = 90;
/// The whole of something, as a percentage.
pub(super) const WHOLE_PERCENT: u32 = 100;

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
