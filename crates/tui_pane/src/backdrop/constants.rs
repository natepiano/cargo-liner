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
/// The whole range takes a little under two seconds to cross here, and
/// the stand at each end is taken from that -- fast enough that the
/// edges are visibly working, slow enough that what they are doing is
/// something the eye can follow rather than a texture.
pub(super) const DEFAULT_TAIL_SPEED: u32 = 150;
/// Fastest the strip travels. Past this it crosses the grid inside a
/// couple of frames, which reads as a flicker rather than as travel.
pub(super) const MAX_BAND_SPEED: u32 = 400;
/// How deep the strip stands before the grid it crosses is known,
/// which is also where it starts.
///
/// Only ever a stand-in: the first draw clamps it to
/// [`MAX_BAND_WIDTH_PERCENT`] of the grid, so this has to be deeper
/// than any grid and is otherwise not a number anything reads.
pub(super) const MAX_BAND_WIDTH: u32 = 1000;
/// How deep the strip may stand, as a percentage of the grid's extent
/// along the axis it travels.
///
/// The grid's own extent, and no further. Past it the strip laps
/// itself -- its tail meets its leading edge with grid still to cross
/// -- and the offsets that have lapped light their whole line, which
/// was tried at 125 and leaves too little of the grid empty to read
/// the strip as a strip.
pub(super) const MAX_BAND_WIDTH_PERCENT: u32 = 100;
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
/// How far back the leading edge can stand from where the strip's
/// travel says it is once that edge is fraying, as a percentage of the
/// strip's width.
///
/// Under [`VARIABLE_TAIL_FLOOR_PERCENT`] on purpose, and the assertion
/// below holds it there: the trailing edge never comes closer to the
/// leading one than that floor, so a ceiling under it means the two
/// can never meet however they are drawn, and the strip keeps a core
/// at every offset rather than parting in the middle.
pub(super) const VARIABLE_HEAD_CEILING_PERCENT: u32 = 20;
const _: () = assert!(
    VARIABLE_HEAD_CEILING_PERCENT < VARIABLE_TAIL_FLOOR_PERCENT,
    "a leading edge allowed as far back as the trailing edge's floor \
     would let the two meet and the strip vanish at that offset"
);
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

// attract text
/// How far a line of the drifting text travels each second, in cells,
/// before anything has sped it up or slowed it down.
///
/// Well under [`DEFAULT_BAND_SPEED`]: the band is one strip and the eye
/// tracks it, while this fills the window, and a whole window of
/// characters moving at the strip's speed is a texture nothing can be
/// read out of.
pub(super) const DEFAULT_TEXT_SPEED: u32 = 12;
/// How far the lines' own speeds stand from the field's before anything
/// has changed it, as a percentage of that speed either way, and the
/// floor the spread is opened back to when the lines are sent apart.
///
/// Wide on purpose. This is the setting the reader is shown the first
/// time they ask for lines at their own speeds, and a field of
/// characters carries no landmark to measure one line against another
/// by -- the difference has to be big enough to read as different
/// speeds rather than as noise. At this width the fastest line covers
/// nine times what the slowest does, and the slowest is still plainly
/// moving: a cell every half second, which reads as slow rather than as
/// stopped.
pub(super) const DEFAULT_TEXT_SPREAD: u32 = 80;
/// Fastest the text drifts. Lower than [`MAX_BAND_SPEED`] for the same
/// reason the default is: every cell is drawn, so there is no empty
/// grid for the eye to measure the travel against.
pub(super) const MAX_TEXT_SPEED: u32 = 200;
/// Widest the lines' speeds spread around the field's. At the top of
/// the range the slowest line is stopped and the fastest is at twice
/// the speed, which is as far apart as two lines can be sent without
/// one of them ceasing to drift at all.
pub(super) const MAX_TEXT_SPREAD: u32 = 100;
/// Slowest the text drifts, and the floor every line's own speed is
/// held above however wide the spread is opened: a line that never
/// moves is one the reader cannot tell from a frozen display.
pub(super) const MIN_TEXT_SPEED: u32 = 1;
/// How far the colour behind the character is carried toward the
/// background, against the character itself standing at the desktop's
/// own colour.
///
/// Every cell of the field draws something and nothing used to paint
/// behind it, so the desktop reached the screen only through the ink
/// and the rest of every cell stayed at the background. Bars are the
/// worst of it: [`BARS_ACROSS`] fills from one edge and `level_at`
/// never returns a level below one, so every cell's first sliver is
/// lit, those slivers line up down the column, and what the reader
/// sees is a rule at every cell boundary rather than the desktop.
/// Characters are the same defect spread thinner -- ink over a
/// background, with the desktop only where the ink is.
///
/// Carrying the rest of the cell part way back gives the whole of it
/// the desktop's colour and leaves the character standing brighter
/// within it. Halfway is the setting that shows the desktop while
/// still parting the two: at zero the character is the same colour as
/// what surrounds it and the field is a flat capture, at [`u8::MAX`]
/// the cell behind the character is the background again, which is
/// where this started.
pub(super) const TEXT_BEHIND_FADE: u8 = 128;
/// How many columns one lane of the field's speeds covers while the
/// text drifts up or down.
///
/// Twice [`TEXT_LANE_ROWS`], because a character cell is about twice as
/// tall as it is wide: a lane this many columns across stands about as
/// thick on the screen as one that many rows deep, so the display reads
/// the same either way round. A count of lanes was tried instead and is
/// what makes a lane the same *fraction* of the display on both axes --
/// which is the wrong answer, since a terminal is several times wider in
/// columns than it is deep in rows and the lanes came out as blocks
/// forty columns across.
pub(super) const TEXT_LANE_COLUMNS: usize = 16;
/// How many rows one lane of the field's speeds covers while the text
/// drifts sideways.
///
/// Deep enough that a lane reads as one body of text travelling
/// together, and shallow enough that an ordinary window holds a slow
/// lane, a fast one, and something in between.
pub(super) const TEXT_LANE_ROWS: usize = 8;
const _: () = assert!(
    TEXT_LANE_COLUMNS > TEXT_LANE_ROWS,
    "a character cell is taller than it is wide, so a lane needs more \
     columns than rows to stand the same thickness on screen"
);
/// How far a line's own speed may stand from what the lane and the
/// ripple across it say, as a percentage of the whole range.
///
/// Small, and drawn per line rather than interpolated: what it is for is
/// that two lines dealt exactly one speed never come apart, so any run
/// with no give at all in it slides as a rigid block. Variation the
/// reader can actually see is [`TEXT_RIPPLE_PERCENT`]'s job -- a single
/// line drifting a little off its neighbours is below what the eye picks
/// out of a field of characters.
pub(super) const TEXT_LANE_GIVE_PERCENT: u32 = 4;
/// How many lines one rise and fall of the ripple inside a lane covers.
///
/// A few, so the ripple carries a short run of lines together rather
/// than dealing each of them separately. That is what makes it legible:
/// three lines easing ahead of their lane is something the eye reads,
/// and one line doing it is not.
pub(super) const TEXT_RIPPLE_LINES: usize = 4;
/// How much of the range the ripple inside a lane may move a line by, as
/// a percentage of how far the ripple's own draw stands from the middle.
///
/// The second and finer of the two runs of drawn speeds the field is
/// dealt from: the lanes say which group a line belongs to, and this
/// says where in its group it sits. Large enough to be read as variation
/// within a lane, small enough that it never carries a line into the
/// lane next door.
pub(super) const TEXT_RIPPLE_PERCENT: u32 = 26;
/// How far the lanes travel along the field each second, in
/// [`LANE_FRACTION_UNIT`] sub-lines.
///
/// The lanes are drawn once and then read at a point that moves, so a
/// line does not keep the speed it was dealt: the pattern slides past it
/// and carries it from a slow group into a fast one and back. Half a
/// line a second, which is slow enough that the field never reads as
/// scrolling -- the characters have their own travel and a second motion
/// at a comparable rate would fight it -- and quick enough that a line
/// crosses a whole lane in under a minute rather than holding one speed
/// for as long as anybody watches.
pub(super) const TEXT_WAVE_SUBLINES_PER_SECOND: u32 = LANE_FRACTION_UNIT / 2;
/// Fixed-point unit the lane interpolation is worked out in.
///
/// A power of two, and large enough that a lane hundreds of lines deep
/// still moves by more than one step per line. The interpolation is
/// where a lane's edge becomes a gradient rather than a wall, so it is
/// the one part of this that cannot be done in whole percentage points.
pub(super) const LANE_FRACTION_UNIT: u32 = 4096;

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
/// the marker title within one pass, before that pass gives up and
/// [`IDENTIFY_PASSES`] decides whether there is another.
///
/// These are not paced. A round trip is a fraction of a millisecond,
/// so all of them together cover a few milliseconds at most -- nowhere
/// near long enough for a title to reach the emulator, be drawn and
/// reach the window server. Waiting for that is what the passes are
/// for; this run is only there to catch a title that has already
/// arrived.
pub(super) const IDENTIFY_ATTEMPTS: u32 = 5;
/// How many passes are made before the window is given up on and the
/// size heuristic carries the run.
///
/// A pass fails for either of two reasons and they need opposite
/// answers. A terminal that will not wear a title never will, and
/// asking it again is waste. But a title also loses the race when the
/// emulator is busy -- and the animation this feeds is what makes it
/// busy, so the pass most likely to fail is the first one, taken while
/// a screen's worth of frames is still queued ahead of the marker.
/// Treating that as the settled answer is what leaves an app drawn
/// against the desktop behind somebody else's window.
pub(super) const IDENTIFY_PASSES: u32 = 10;
/// How long the app waits before looking for its window again.
///
/// Long enough that a busy emulator has drained what was queued ahead
/// of the marker, and short enough that ten of them are over inside
/// the first few seconds of the animation.
pub(super) const IDENTIFY_RETRY: Duration = Duration::from_millis(500);
/// What the marker title this app briefly wears begins with, before
/// the process id that makes it this process's alone.
pub(super) const IDENTIFY_MARKER: &str = "tui-pane-window-";
/// The environment variable a terminal emulator names itself in.
pub(super) const TERM_PROGRAM_ENV: &str = "TERM_PROGRAM";
/// How many letters a folded emulator name must carry before it is
/// matched against another by containment.
///
/// Short names match far too much: `sh` is inside a dozen bundle
/// identifiers on any machine. Five is past every accident and under
/// every real emulator name -- `iterm`, `wezterm`, `ghostty`,
/// `terminal`, `alacritty`.
pub(super) const EMULATOR_NAME_FLOOR: usize = 5;
/// What the terminal is asked when it is asked where its window
/// stands: the xterm window-position report.
///
/// Answered with `CSI 3 ; x ; y t`. A terminal that does not know the
/// query says nothing, which is why the wait for a reply is timed
/// rather than open-ended.
pub(super) const POSITION_QUERY: &str = "\u{1b}[13t";
/// How long the terminal is given to answer the position query.
///
/// Generous by an order of magnitude. The query is flushed before this
/// starts, so whatever output was queued ahead of it has already been
/// drained by the emulator and the reply has only a pty to cross.
pub(super) const POSITION_REPLY_WAIT: Duration = Duration::from_millis(100);
/// How many bytes of a reply are read before the terminal is taken to
/// be answering something with no end to it.
///
/// The longest reply a position can be is seventeen bytes, both
/// coordinates negative and four digits each. The rest is room for a
/// keystroke or two that beat the reply out of the queue.
pub(super) const POSITION_REPLY_BYTES: usize = 32;
/// The byte a position report ends on, which is what stops the read
/// before it reaches anything the reader typed.
pub(super) const POSITION_REPLY_END: u8 = b't';
/// How far, in the window server's points, a window may stand from the
/// position the terminal reported and still be taken for the one it
/// reported.
///
/// Not zero, because the two are not measured from quite the same
/// corner: an emulator may report the corner of the text area rather
/// than of the window around it, and everything it stacks above the
/// grid -- a title bar, a tab bar -- stands between them. Two hundred
/// points clears all of that and is still far short of the distance
/// between two windows the reader has put side by side.
pub(super) const POSITION_TOLERANCE: f64 = 200.0;
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

// bars
/// The bar a cell of the drifting field draws, from the narrowest
/// sliver to the whole of the cell, filling across it.
///
/// Ordered, where [`GLYPHS`] is a bag to draw from: the index is how
/// much of the cell is lit, and that is what lets a line be drawn part
/// way into a cell rather than only on whole ones. Eighths are as fine
/// as the block elements go, and the ramp fills from the left because
/// that is the only side Unicode carries the whole run from.
pub(super) const BARS_ACROSS: &[char] = &['▏', '▎', '▍', '▌', '▋', '▊', '▉', '█'];
/// The same ramp filling up the cell rather than across it.
///
/// A cell is only subdivided along the axis its line travels on -- a
/// bar that grows sideways says nothing about a line drifting
/// downward -- so the direction picks which of the two ramps is drawn.
pub(super) const BARS_UP: &[char] = &['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
/// How many steps those ramps hold, which is the scale a cell's fill is
/// read on. One is the narrowest sliver and this is the whole cell.
pub(super) const BAR_LEVELS: u32 = 8;

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
