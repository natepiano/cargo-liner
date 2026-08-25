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
/// How fast the trailing edge frays, on the [`u8`] scale one offset's
/// depth is held in, per second, before anything has sped it up or
/// slowed it down.
///
/// The whole range takes a second to cross here, which is fast enough
/// that the edge is visibly working at whatever the eye happens to be
/// resting on rather than only where it is not.
pub(super) const DEFAULT_TAIL_SPEED: u32 = 260;
/// Fastest the strip travels. Past this it crosses the grid inside a
/// couple of frames, which reads as a flicker rather than as travel.
pub(super) const MAX_BAND_SPEED: u32 = 400;
/// How deep the strip stands before the grid it crosses is known,
/// which is also where it starts.
///
/// Once the grid is known it is the ceiling: the strip wraps, so at
/// exactly the number of lines the grid has there is no gap left
/// between the tail and the leading edge and the whole grid is lit at
/// once, and past that there is nothing further to show. Starting here
/// stands the strip across the whole window, since the first draw
/// clamps it to whatever that turns out to be.
pub(super) const MAX_BAND_WIDTH: u32 = 200;
/// Fastest the trailing edge frays. Past this an offset crosses the
/// whole range inside a few frames, and a trailing edge that arrives
/// somewhere new every frame is the boiling the travel was there to
/// avoid.
pub(super) const MAX_TAIL_SPEED: u32 = 2000;
/// Slowest the strip travels. Zero is not offered: a strip that never
/// moves is one the reader cannot tell from a frozen display.
pub(super) const MIN_BAND_SPEED: u32 = 1;
/// Thinnest the strip stands: a single line at full strength with no
/// tail behind it.
pub(super) const MIN_BAND_WIDTH: u32 = 1;
/// Slowest the trailing edge frays: the whole range over half a minute,
/// which is as near to a flat trailing edge as the key goes without
/// turning the fraying off.
pub(super) const MIN_TAIL_SPEED: u32 = 8;
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
/// sent to, as a percentage of what crossing the whole range costs at
/// the speed it is travelling.
///
/// Taken from the speed rather than fixed so that one key governs the
/// whole of how fast the trailing edge changes. A stand of its own
/// length would otherwise outlast the travel at the top of the range
/// and leave the fastest setting looking no livelier than the middle
/// of it.
pub(super) const VARIABLE_TAIL_HOLD_PERCENT: u32 = 40;
/// What a character cell's pixel measurements are scaled by before
/// they are divided into each other.
///
/// A cell is a whole number of pixels across only by accident -- eight
/// and a half is as ordinary as eight -- and the strip's depth is
/// carried between the two axes by the ratio of the two. Rounding each
/// side to whole pixels first would put a percent or so of error into
/// every turn.
pub(super) const PIXEL_PRECISION: u32 = 256;
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
/// How many times the window server is asked which window is wearing
/// the marker title before the attempt is given up on.
///
/// Nothing paces these: each one is a full round trip and the title
/// has only to travel to the emulator and back out to the window
/// server, so asking again is already asking later.
pub(super) const IDENTIFY_ATTEMPTS: u32 = 5;
/// What the marker title this app briefly wears begins with, before
/// the process id that makes it this process's alone.
pub(super) const IDENTIFY_MARKER: &str = "tui-pane-window-";
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
/// Microseconds in one second.
///
/// The strip's travel is worked out at this resolution rather than in
/// milliseconds: a frame is a little under twenty milliseconds and
/// rounding it down to nineteen loses a twentieth of the distance, and
/// loses a different fraction of it whenever a frame arrives early.
pub(super) const MICROS_PER_SECOND: u64 = 1_000_000;
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
