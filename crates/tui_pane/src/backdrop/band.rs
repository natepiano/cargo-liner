//! The attract-mode animation: a lit strip of characters crossing the
//! grid, each cell wearing the colour of the desktop behind it.
//!
//! The strip travels one of the four ways a [`BandDirection`] names,
//! and it wraps: its tail is still leaving one edge while its leading
//! edge is coming back in at the other, so the grid is never empty
//! between one pass and the next.
//!
//! Every cell the strip stands wholly on is drawn in exactly the colour
//! the [`Backdrop`] has there -- no ramp along its length. A terminal
//! cell is opaque and carries no alpha, so anything done to that colour
//! is done to what the reader came to look at: the strip's one subject
//! is the desktop the window is standing on, and a cell wearing a
//! mixture is a cell showing something that is not there.
//!
//! The exception is the one line at each end that the strip stands only
//! part way across, which is lit by however much of it the strip covers.
//! Whole cells are the only places a strip on a character grid can
//! stand, so without that the strip could only step from cell to cell,
//! and stepping is what the eye reads as stop motion rather than travel.
//!
//! What gives the strip edges to read, then, is where it stops -- and
//! either edge can fray rather than standing flat across every line.
//! A fraying edge runs back its own distance at every offset across the
//! strip, and those distances grow and shrink while it travels.
//! [`BandFraying`] names the four ways the two edges can be set, and
//! how fast they fray is steerable on top of that.
//!
//! A strip that has not been steered sets off left to right with both
//! of its edges fraying, standing across the whole window. What each
//! offset across it stands on is its own two edges, so the lines it is
//! made of end at different places rather than all together, and how
//! much grid is left empty behind it changes while it travels.
//!
//! Position is tracked in whole numbers throughout. A strip that moves
//! a fraction of a cell per frame wants sub-cell precision, and
//! carrying that as a float would put a truncating cast in the middle
//! of every cell's colour.

use std::time::Duration;

use crossterm::terminal;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;

use super::Backdrop;
use super::constants::CHURN_CELLS_PER_FRAME;
use super::constants::DEFAULT_BAND_SPEED;
use super::constants::DEFAULT_TAIL_SPEED;
use super::constants::MAX_BAND_SPEED;
use super::constants::MAX_BAND_WIDTH;
use super::constants::MAX_BAND_WIDTH_PERCENT;
use super::constants::MAX_TAIL_SPEED;
use super::constants::MICROS_PER_SECOND;
use super::constants::MILLIS_PER_SECOND;
use super::constants::MIN_BAND_SPEED;
use super::constants::MIN_BAND_WIDTH;
use super::constants::MIN_TAIL_SPEED;
use super::constants::PIXEL_PRECISION;
use super::constants::SUBCELLS_PER_CELL;
use super::constants::VARIABLE_HEAD_CEILING_PERCENT;
use super::constants::VARIABLE_TAIL_FLOOR_PERCENT;
use super::constants::VARIABLE_TAIL_HOLD_PERCENT;
use super::constants::WHOLE_PERCENT;
use super::random;
use super::random::Xorshift;
use crate::theme;

/// Which way a [`TravelingBand`] crosses the grid.
///
/// Travel is along one axis at a time: sideways the strip is a column
/// the full height of the area, up or down it is a row the full width
/// of it. The direction also says which edge the strip enters by, and
/// so which end of the area its tail trails toward.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum BandDirection {
    /// Enters at the right edge and travels toward the left.
    Left,
    /// Enters at the left edge and travels toward the right. Where a
    /// strip that has not been steered starts.
    #[default]
    Right,
    /// Enters at the bottom edge and travels toward the top.
    Up,
    /// Enters at the top edge and travels toward the bottom.
    Down,
}

/// Which of a [`TravelingBand`]'s two edges fray, rather than standing
/// flat across every line.
///
/// An edge that frays runs back a different distance at every offset
/// across the strip, and those distances grow and shrink while it
/// travels. Which edge is doing it changes what the strip reads as: a
/// flat leading edge is the one the eye tracks and a fraying trailing
/// one is the strip coming apart behind it, and swapping the two puts
/// the ragged end in front.
///
/// [`next`](Self::next) steps through all four, which is what one key
/// cycling them walks along.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum BandFraying {
    /// The trailing edge frays and the leading edge stays flat.
    Trailing,
    /// Both edges fray. Where a strip that has not been steered starts.
    #[default]
    Both,
    /// The leading edge frays and the trailing edge stays flat.
    Leading,
    /// Neither frays: both edges are flat across every line.
    Neither,
}

impl BandFraying {
    /// The next of the four, wrapping back to the first.
    ///
    /// Ordered so that each step changes exactly one edge -- the
    /// trailing edge alone, then both, then the leading edge alone,
    /// then neither -- which is what makes what a press did readable
    /// from the screen without being told. A strip starts partway
    /// along the ring rather than at its head, so the first press
    /// takes the leading edge on its own.
    #[must_use]
    pub const fn next(self) -> Self {
        match self {
            Self::Trailing => Self::Both,
            Self::Both => Self::Leading,
            Self::Leading => Self::Neither,
            Self::Neither => Self::Trailing,
        }
    }

    /// Whether the leading edge frays.
    const fn leading(self) -> bool { matches!(self, Self::Leading | Self::Both) }

    /// Whether the trailing edge frays.
    const fn trailing(self) -> bool { matches!(self, Self::Trailing | Self::Both) }
}

/// How much of the run of `length` starting at `start` falls inside
/// `0..inside`, on a ring of circumference `span`.
///
/// The run is allowed to pass the wrap point, in which case what it is
/// owed is whatever it picks up before the wrap plus whatever it picks
/// up after it.
const fn ring_overlap(start: u32, length: u32, inside: u32, span: u32) -> u32 {
    let before_wrap = span - start;
    let first = if length < before_wrap {
        length
    } else {
        before_wrap
    };
    let head = if start < inside {
        let room = inside - start;
        if first < room { first } else { room }
    } else {
        0
    };
    let wrapped = length - first;
    let foot = if wrapped < inside { wrapped } else { inside };
    head + foot
}

/// How far back one of the strip's edges stands at one offset across
/// it, and where that is heading.
///
/// A depth drawn at random and taken up on the next frame would read as
/// a trailing edge boiling rather than as one moving, so a fresh draw
/// is a place to travel to: the offset walks there, stands at it for a
/// while, and only then draws again. How long both of those take comes
/// from [`TravelingBand::tail_speed`], so one key governs the whole of
/// how fast the trailing edge changes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct EdgeRun {
    /// How far back the strip runs here now, on the scale
    /// [`TravelingBand::tail_at`] reads.
    depth:   u8,
    /// Where that is heading.
    target:  u8,
    /// What is left of the stand at [`Self::target`] before a fresh one
    /// is drawn. Only counts down once the target has been reached.
    holding: Duration,
}

impl EdgeRun {
    /// An offset standing at `depth` with nowhere to travel.
    ///
    /// Both edges start at the end of their range that is not frayed
    /// at all -- the trailing edge at the full width, the leading one
    /// flush with where the travel says it is -- so a strip sets off
    /// as flat as it will ever be and frays outward from there.
    const fn at(depth: u8) -> Self {
        Self {
            depth,
            target: depth,
            holding: Duration::ZERO,
        }
    }

    /// Carry the offset one frame on: `travel` further toward its
    /// target, or `elapsed` further through the stand of `hold` it is
    /// keeping at one, taking `drawn` as its next target when that
    /// stand runs out.
    ///
    /// `drawn` is handed in already rolled rather than rolled here, so
    /// the strip's one generator stays where the rest of its randomness
    /// comes from. Most frames it goes unused.
    fn advance(&mut self, elapsed: Duration, travel: u8, hold: Duration, drawn: u8) {
        if self.depth != self.target {
            self.depth = if self.depth < self.target {
                self.depth.saturating_add(travel).min(self.target)
            } else {
                self.depth.saturating_sub(travel).max(self.target)
            };
            if self.depth == self.target {
                self.holding = hold;
            }
            return;
        }
        self.holding = self.holding.saturating_sub(elapsed);
        if self.holding.is_zero() {
            self.target = drawn;
        }
    }
}

/// A lit strip of characters travelling across the grid.
///
/// [`advance`](Self::advance) moves it on by one frame's worth of time
/// and [`render`](Self::render) draws it over a [`Backdrop`]. The strip
/// sizes itself to whatever [`Rect`] it is advanced against, so a
/// resize costs a fresh set of characters and nothing more.
///
/// Which way it goes, how deep it stands and how fast it travels are
/// all steerable while it runs --
/// [`set_direction`](Self::set_direction), [`widen`](Self::widen) and
/// [`narrow`](Self::narrow), [`speed_up`](Self::speed_up) and
/// [`slow_down`](Self::slow_down). Each is clamped here rather than at
/// the call site, so an app can hand a held key straight through
/// without working out where the limits are.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TravelingBand {
    /// Where the leading edge stands, in sub-cells from the edge the
    /// strip enters by.
    leading_edge:   u32,
    /// The character each cell is drawing, row-major over
    /// `columns * rows`.
    glyphs:         Vec<char>,
    /// How far back the strip's trailing edge stands at each offset
    /// across itself while it is fraying, read by [`Self::tail_at`].
    /// Zero is the shallowest it goes and [`u8::MAX`] is the full
    /// width, and each of them is travelling between the two rather
    /// than sitting still -- see [`EdgeRun`].
    ///
    /// Across the strip rather than along it. A strip crossing sideways
    /// is a run of cells on every row, and it is how far back those
    /// runs reach that varies. Indexed by line instead, this would be
    /// asking whether a whole column is drawn at all -- and a column
    /// near the trailing edge answering no punches a hole through the
    /// strip rather than shortening it.
    ///
    /// Long enough for either axis, so turning the strip round costs
    /// nothing.
    tails:          Vec<EdgeRun>,
    /// How far back the strip's leading edge stands from where its
    /// travel says it is, at each offset across it, while that edge is
    /// fraying. Read by [`Self::head_at`], on the same scale as
    /// [`Self::tails`] but running the other way: zero is flush with
    /// the travel and [`u8::MAX`] is as far back as that edge goes.
    heads:          Vec<EdgeRun>,
    /// Where each offset across the strip carries its own leading edge,
    /// as a share of one lap of the ring, read by [`Self::phase_at`].
    ///
    /// Without this the strip's two ends stand at the same place on
    /// every offset, give or take how far the edges have frayed -- and
    /// a strip standing less deep than the window is wide then leaves
    /// the same run of grid empty on every one of them. That shared run
    /// is a band of nothing down the screen, which is the one thing an
    /// animation filling a window should not have.
    ///
    /// Held as a share rather than in sub-cells so that turning the
    /// strip between the two axes, where a lap is a different length,
    /// leaves every offset where it stood relative to the others.
    /// Drawn once with the area, because an offset whose start moved
    /// from frame to frame would not read as travelling at all.
    ///
    /// Only read while the leading edge frays: the other two settings
    /// are the ones whose whole point is an edge the eye can track
    /// across the window, and a staggered start is not one.
    phases:         Vec<u8>,
    /// Cells across the area the strip was last sized to.
    columns:        u16,
    /// Cells down that same area.
    rows:           u16,
    /// Which way the strip is travelling.
    direction:      BandDirection,
    /// How deep the strip stands, in cells along the axis it travels.
    width:          u32,
    /// One character cell across and down, in pixels scaled by
    /// [`PIXEL_PRECISION`], or zeroes where the terminal will not say.
    ///
    /// What this is for is turning the strip between the two axes: a
    /// cell is taller than it is wide, so the same count of them is a
    /// different depth on the screen depending on which way they stack.
    cell_pixels:    (u32, u32),
    /// How far the strip travels each second, in cells.
    speed:          u32,
    /// How fast the trailing edge frays, on the [`u8`] scale one
    /// offset's depth is held in, per second. Governs both the walk
    /// toward a fresh depth and the stand at it -- see [`EdgeRun`].
    tail_speed:     u32,
    /// How many lines the leading edge has re-rolled on this pass. A
    /// line is a column while the strip travels sideways and a row
    /// while it travels up or down.
    rolled_through: u16,
    /// Source of the character churn.
    xorshift:       Xorshift,
    /// How far the whole strip is carried toward the ground it is drawn
    /// on, on the alpha scale [`blend_color`] reads: zero draws it at
    /// full strength, [`u8::MAX`] draws nothing.
    faded:          u8,
    /// Which of the strip's two edges fray.
    fraying:        BandFraying,
}

impl Default for TravelingBand {
    fn default() -> Self {
        Self {
            leading_edge:   0,
            glyphs:         Vec::new(),
            tails:          Vec::new(),
            heads:          Vec::new(),
            phases:         Vec::new(),
            columns:        0,
            rows:           0,
            direction:      BandDirection::default(),
            // Deeper than any grid, so the first draw clamps it to
            // whatever the window turns out to be: the strip starts
            // standing across the whole of it.
            width:          MAX_BAND_WIDTH,
            cell_pixels:    (0, 0),
            speed:          DEFAULT_BAND_SPEED,
            tail_speed:     DEFAULT_TAIL_SPEED,
            rolled_through: 0,
            xorshift:       Xorshift::default(),
            faded:          0,
            fraying:        BandFraying::default(),
        }
    }
}

impl TravelingBand {
    /// A strip that has not been sized yet. The first
    /// [`advance`](Self::advance) settles its area.
    #[must_use]
    pub fn new() -> Self { Self::default() }

    /// Move the strip on by `elapsed`, sizing it to `area` and
    /// re-rolling the characters its leading edge has reached.
    pub fn advance(&mut self, area: Rect, elapsed: Duration) {
        self.resize(area);
        if self.columns == 0 || self.rows == 0 {
            return;
        }
        let elapsed_micros = u64::try_from(elapsed.as_micros()).unwrap_or(u64::MAX);
        let travel = u64::from(self.speed)
            .saturating_mul(u64::from(SUBCELLS_PER_CELL))
            .saturating_mul(elapsed_micros)
            / MICROS_PER_SECOND;
        self.leading_edge = self
            .leading_edge
            .saturating_add(u32::try_from(travel).unwrap_or(u32::MAX));

        // The strip wraps rather than running clear of the grid and
        // starting over: the position is measured modulo the lines
        // there are, so the tail is still leaving the far edge while
        // the leading edge is back at the near one. A strip that
        // finished each pass would leave the screen empty for as long
        // as it took to cross it again, which on a wide grid at a slow
        // speed is most of the time the reader is watching.
        let span = self.span();
        if span > 0 && self.leading_edge >= span {
            self.leading_edge %= span;
            self.rolled_through = 0;
        }
        self.advance_edges(elapsed);
        self.roll_reached_lines();
        self.churn();
    }

    /// Carry the whole strip `faded` of the way toward the ground it is
    /// drawn on, which is how it leaves when the screen it decorates
    /// has something real to show. Zero is full strength, [`u8::MAX`]
    /// draws nothing at all.
    pub const fn fade(&mut self, faded: u8) { self.faded = faded; }

    /// Step to the next of the four ways the strip's edges can fray --
    /// see [`BandFraying::next`].
    ///
    /// A fraying edge runs back its own distance at each offset across
    /// the strip -- every row of one crossing sideways, every column of
    /// one crossing up or down -- and those distances grow and shrink
    /// while it travels. Each is a walk toward a depth drawn for it and
    /// a stand once it arrives, so an edge moves rather than boiling.
    ///
    /// An edge that has stopped fraying is put back flat, so the next
    /// time round it starts from flat and frays outward rather than
    /// snapping to wherever it was left.
    pub fn cycle_fraying(&mut self) {
        self.fraying = self.fraying.next();
        if !self.fraying.trailing() {
            self.tails.fill(EdgeRun::at(u8::MAX));
        }
        if !self.fraying.leading() {
            self.heads.fill(EdgeRun::at(0));
        }
    }

    /// Send the strip a different way.
    ///
    /// It restarts from the edge it now enters by rather than carrying
    /// its position across: the position is measured from that edge, so
    /// a reversal read the old number as a strip most of the way to the
    /// far side rather than one just setting off.
    pub fn set_direction(&mut self, direction: BandDirection) {
        if matches!(
            (self.direction, direction),
            (BandDirection::Left, BandDirection::Left)
                | (BandDirection::Right, BandDirection::Right)
                | (BandDirection::Up, BandDirection::Up)
                | (BandDirection::Down, BandDirection::Down)
        ) {
            return;
        }
        // A turn from one axis to the other is a turn between two
        // rulers. The depth is carried across it rather than the count
        // of cells, so a strip a ruler measures at an inch across the
        // screen is still an inch after it turns.
        let turning =
            self.sideways() != matches!(direction, BandDirection::Left | BandDirection::Right);
        if turning {
            self.width = self.turned_depth();
        }
        self.direction = direction;
        self.leading_edge = 0;
        self.rolled_through = 0;
        // The ceiling is the grid's extent along the axis travelled,
        // and that is a different number after the turn.
        self.set_width(self.width);
    }

    /// Whether the strip travels along the rows rather than down the
    /// columns.
    const fn sideways(&self) -> bool {
        matches!(self.direction, BandDirection::Left | BandDirection::Right)
    }

    /// The strip's depth in the cells of the axis it is turning on to,
    /// standing as deep on the screen as it does in the ones it leaves.
    ///
    /// Where the terminal will not say how big a cell is, the count is
    /// carried across unchanged -- which is what it did before there
    /// was anything to scale it by.
    fn turned_depth(&self) -> u32 {
        let (across, down) = self.cell_pixels;
        if across == 0 || down == 0 {
            return self.width;
        }
        // Leaving the sideways axis the strip is counted in columns and
        // arrives counted in rows, and the other way round coming back.
        let (from, to) = if self.sideways() {
            (across, down)
        } else {
            (down, across)
        };
        // Rounded rather than truncated, so a turn and a turn back land
        // on the depth they started at rather than a cell shallower
        // every time round.
        self.width
            .saturating_mul(from)
            .saturating_add(to / 2)
            .checked_div(to)
            .unwrap_or(self.width)
    }

    /// Stand the strip `cells` deeper, up to the widest it goes.
    pub fn widen(&mut self, cells: u32) { self.set_width(self.width.saturating_add(cells)); }

    /// Stand the strip `cells` shallower, down to the thinnest it goes.
    pub fn narrow(&mut self, cells: u32) { self.set_width(self.width.saturating_sub(cells)); }

    /// Travel `cells_per_second` faster, up to the fastest it goes.
    pub fn speed_up(&mut self, cells_per_second: u32) {
        self.speed = self
            .speed
            .saturating_add(cells_per_second)
            .clamp(MIN_BAND_SPEED, MAX_BAND_SPEED);
    }

    /// Travel `cells_per_second` slower, down to the slowest it goes.
    pub fn slow_down(&mut self, cells_per_second: u32) {
        self.speed = self
            .speed
            .saturating_sub(cells_per_second)
            .clamp(MIN_BAND_SPEED, MAX_BAND_SPEED);
    }

    /// Fray the trailing edge `per_second` faster, up to the fastest it
    /// goes. Does nothing visible while the trailing edge is flat.
    pub fn tail_faster(&mut self, per_second: u32) {
        self.tail_speed = self
            .tail_speed
            .saturating_add(per_second)
            .clamp(MIN_TAIL_SPEED, MAX_TAIL_SPEED);
    }

    /// Fray the trailing edge `per_second` slower, down to the slowest
    /// it goes.
    pub fn tail_slower(&mut self, per_second: u32) {
        self.tail_speed = self
            .tail_speed
            .saturating_sub(per_second)
            .clamp(MIN_TAIL_SPEED, MAX_TAIL_SPEED);
    }

    /// Draw the strip over `area`, colouring each cell by the
    /// [`Backdrop`] underneath it.
    ///
    /// Every covered cell is that colour exactly, front to back. The
    /// strip is not a gradient and there is nothing here to fade: what
    /// separates it from the rest of the grid is that it is drawn at
    /// all, and where it stops.
    ///
    /// Leaving is the one moment the strip is meant to stop being
    /// visible, and it goes toward whatever each cell is already
    /// painted on to do it -- `ground` only standing in where the cell
    /// is painted on nothing at all. A cell keeps its own background
    /// here, so a strip drawn over something opaque settles into that
    /// something rather than into a colour guessed for the whole grid.
    ///
    /// Cells outside the strip are left untouched rather than painted,
    /// so whatever the terminal shows through stays visible. A cell the
    /// backdrop has no colour for is skipped for the same reason.
    pub fn render(&self, area: Rect, backdrop: &Backdrop, ground: Color, buffer: &mut Buffer) {
        if self.faded == u8::MAX {
            return;
        }
        for row in 0..self.rows.min(area.height) {
            for column in 0..self.columns.min(area.width) {
                let Some(strength) = self.coverage(column, row) else {
                    continue;
                };
                // A cell the edge has only just entered is drawn no
                // more strongly than it has been entered, and a cell it
                // has not entered at all is left alone rather than
                // painted in the colour it would be invisible in.
                if strength == 0 {
                    continue;
                }
                let Some(color) = backdrop.color_at(column, row) else {
                    continue;
                };
                let Some(&glyph) = self.glyphs.get(self.cell_index(column, row)) else {
                    continue;
                };
                if let Some(cell) = buffer.cell_mut((area.x + column, area.y + row)) {
                    let toward = match cell.bg {
                        Color::Reset => ground,
                        background => background,
                    };
                    // The strip's own fade and this cell's share of it
                    // compound: what is left of the colour is the one
                    // scaled by the other, and the alpha handed on is
                    // whatever that leaves.
                    let visible =
                        u32::from(u8::MAX - self.faded) * u32::from(strength) / u32::from(u8::MAX);
                    let alpha = u8::MAX - u8::try_from(visible).unwrap_or(u8::MAX);
                    cell.set_char(glyph);
                    cell.set_fg(theme::blend_color(color, toward, alpha));
                }
            }
        }
    }

    /// Whether the strip reaches the cell at `column`, `row` at all
    /// this frame.
    ///
    /// Where the strip reaches is a separate question from how strongly
    /// it lights what it reaches, and the tests below ask it directly.
    /// Rendering asks only [`Self::coverage`], which answers both at
    /// once.
    #[cfg(test)]
    fn covers(&self, column: u16, row: u16) -> bool { self.coverage(column, row).is_some() }

    /// How strongly the strip lights the cell at `column`, `row` this
    /// frame, or [`None`] where it does not reach the cell at all.
    ///
    /// [`u8::MAX`] is the strip at full strength, and anything less is
    /// a cell the leading edge has only partly entered.
    ///
    /// That partial line is what makes the strip travel rather than
    /// step. The edge moves a fraction of a cell per frame -- a little
    /// over half of one at the default speed -- so a cell that could
    /// only be lit or unlit holds still for a frame or two and then
    /// changes all at once, and a whole cell arriving on an uneven beat
    /// is what the eye reads as stop motion. Lighting the line in
    /// proportion to how far the edge has come into it gives every
    /// frame something to show.
    ///
    /// Both edges are read the same way, and they have to be. A strip
    /// travelling across a character grid can only ever stand on whole
    /// cells, but how brightly a cell is lit is not quantised at all --
    /// so where the strip has reached between one cell and the next is
    /// carried by the brightness of the line it is part way through.
    /// Doing that at the leading edge alone leaves the trailing one
    /// dropping a whole line at a time, and the strip has only two
    /// edges to read its travel from: one of them stepping is half the
    /// motion stepping.
    ///
    /// Distance behind the leading edge is measured the long way round,
    /// so a line the edge has not reached on this pass is read as one
    /// its tail has not finished leaving on the last -- which is what
    /// the wrap means.
    fn coverage(&self, column: u16, row: u16) -> Option<u8> {
        let span = self.span();
        if span == 0 {
            return None;
        }
        let offset = self.offset_of(column, row);
        let head = self.head_at(offset);
        let tail = self.tail_at(offset);
        // What the strip actually stands on at this offset, once both
        // of its edges have been asked where they are.
        let depth = tail.saturating_sub(head);
        // A strip standing as deep as the grid has lines is the whole
        // ring: its tail has met its own leading edge, and there is no
        // line left anywhere for either edge to be part way across.
        if depth >= span {
            return Some(u8::MAX);
        }
        let line = self.line_of(column, row);
        // Both the cell and the strip are runs on the same ring -- the
        // cell one line long ending at `near`, the strip `tail` long
        // ending at the leading edge -- so a cell near the wrap can have
        // one end of it inside the strip's head and the other inside
        // its tail, and owning only the first is what left a line unlit
        // at every width.
        let leading = (self.leading_edge + self.phase_at(offset)) % span;
        let near = (leading + span - u32::from(line) * SUBCELLS_PER_CELL) % span;
        let start = (near + span - SUBCELLS_PER_CELL) % span;
        // Turning the ring back by `head` puts this offset's own
        // leading edge at zero, so a strip that starts somewhere behind
        // the travel is read by the same overlap as one that starts at
        // it.
        let shifted = (start + span - head % span) % span;
        let covered = ring_overlap(shifted, SUBCELLS_PER_CELL, depth, span);
        if covered == 0 {
            return None;
        }
        if covered >= SUBCELLS_PER_CELL {
            return Some(u8::MAX);
        }
        u8::try_from(covered * u32::from(u8::MAX) / SUBCELLS_PER_CELL).ok()
    }

    /// How far back the strip runs at `offset` across itself, in
    /// sub-cells behind the leading edge.
    ///
    /// The full width unless the trailing edge is fraying, in which
    /// case that offset's own draw carries it from the floor
    /// [`VARIABLE_TAIL_FLOOR_PERCENT`] sets up to that full width.
    fn tail_at(&self, offset: u16) -> u32 {
        let full = self.width * SUBCELLS_PER_CELL;
        if !self.fraying.trailing() {
            return full;
        }
        let floor = full * VARIABLE_TAIL_FLOOR_PERCENT / WHOLE_PERCENT;
        let depth = self
            .tails
            .get(usize::from(offset))
            .map_or(u8::MAX, |run| run.depth);
        floor + (full - floor) * u32::from(depth) / u32::from(u8::MAX)
    }

    /// How far behind the strip's travel its own leading edge stands
    /// at `offset` across it, in sub-cells.
    ///
    /// Zero -- flush with the travel -- unless that edge is fraying, in
    /// which case the offset's own draw carries it back as far as
    /// [`VARIABLE_HEAD_CEILING_PERCENT`] of the width. That ceiling
    /// sits under the trailing edge's floor, so however the two are
    /// drawn the strip keeps a core at every offset.
    fn head_at(&self, offset: u16) -> u32 {
        if !self.fraying.leading() {
            return 0;
        }
        let ceiling =
            self.width * SUBCELLS_PER_CELL * VARIABLE_HEAD_CEILING_PERCENT / WHOLE_PERCENT;
        let depth = self
            .heads
            .get(usize::from(offset))
            .map_or(0, |run| run.depth);
        ceiling * u32::from(depth) / u32::from(u8::MAX)
    }

    /// How far round the ring `offset` carries its own leading edge,
    /// in sub-cells.
    ///
    /// Zero unless that edge frays, which is the setting the stagger
    /// belongs to: the other two are the ones the eye tracks a single
    /// edge across, and there is no edge to track once every offset
    /// starts somewhere else.
    fn phase_at(&self, offset: u16) -> u32 {
        if !self.fraying.leading() {
            return 0;
        }
        let share = self.phases.get(usize::from(offset)).copied().unwrap_or(0);
        self.span() * u32::from(share) / (u32::from(u8::MAX) + 1)
    }

    /// How far the leading edge travels before it is back where it
    /// started, in sub-cells. Zero for a strip that has no area yet.
    fn span(&self) -> u32 { u32::from(self.lines()) * SUBCELLS_PER_CELL }

    /// Lines the strip crosses on one pass: the columns of the area
    /// while it travels sideways, its rows while it travels up or down.
    const fn lines(&self) -> u16 {
        match self.direction {
            BandDirection::Left | BandDirection::Right => self.columns,
            BandDirection::Up | BandDirection::Down => self.rows,
        }
    }

    /// Cells on one of those lines: the rows of the area while the
    /// strip travels sideways, its columns while it travels up or down.
    const fn cells_per_line(&self) -> u16 {
        match self.direction {
            BandDirection::Left | BandDirection::Right => self.rows,
            BandDirection::Up | BandDirection::Down => self.columns,
        }
    }

    /// How far along its line the cell at `column`, `row` sits, which
    /// is the axis how far back the strip runs varies over. The
    /// inverse of the `offset` [`cell_on_line`](Self::cell_on_line)
    /// takes.
    const fn offset_of(&self, column: u16, row: u16) -> u16 {
        match self.direction {
            BandDirection::Left | BandDirection::Right => row,
            BandDirection::Up | BandDirection::Down => column,
        }
    }

    /// Which line the cell at `column`, `row` sits on, counted from the
    /// edge the strip enters by.
    const fn line_of(&self, column: u16, row: u16) -> u16 {
        match self.direction {
            BandDirection::Right => column,
            BandDirection::Left => self.columns.saturating_sub(1).saturating_sub(column),
            BandDirection::Down => row,
            BandDirection::Up => self.rows.saturating_sub(1).saturating_sub(row),
        }
    }

    /// The area column and row of the cell `offset` along `line`, the
    /// inverse of [`line_of`](Self::line_of).
    const fn cell_on_line(&self, line: u16, offset: u16) -> (u16, u16) {
        match self.direction {
            BandDirection::Right => (line, offset),
            BandDirection::Left => (self.columns.saturating_sub(1).saturating_sub(line), offset),
            BandDirection::Down => (offset, line),
            BandDirection::Up => (offset, self.rows.saturating_sub(1).saturating_sub(line)),
        }
    }

    /// Where the cell at `column`, `row` sits in [`Self::glyphs`].
    fn cell_index(&self, column: u16, row: u16) -> usize {
        usize::from(row) * usize::from(self.columns) + usize::from(column)
    }

    /// Stand the strip `width` deep, clamped to what it is allowed.
    ///
    /// Never deeper than the grid it crosses -- see
    /// [`MAX_BAND_WIDTH_PERCENT`]. At exactly that depth an offset
    /// whose trailing edge is at full stretch has met its own leading
    /// edge and lights its whole line, which is a reasonable place to
    /// be able to get to; past it more and more offsets are in that
    /// state at once and there is too little grid left empty to read
    /// the strip against.
    fn set_width(&mut self, width: u32) {
        let lines = self.lines();
        let widest = if lines == 0 {
            MAX_BAND_WIDTH
        } else {
            u32::from(lines) * MAX_BAND_WIDTH_PERCENT / WHOLE_PERCENT
        };
        self.width = width.clamp(MIN_BAND_WIDTH, widest.max(MIN_BAND_WIDTH));
    }

    /// Ask the terminal how big one character cell is, and keep the
    /// answer where a turn between the axes can read it.
    ///
    /// A terminal that will not say leaves the last answer standing, so
    /// a single refusal does not undo a size already learned.
    fn read_cell_pixels(&mut self) {
        let Ok(size) = terminal::window_size() else {
            return;
        };
        if size.width == 0 || size.height == 0 || size.columns == 0 || size.rows == 0 {
            return;
        }
        self.cell_pixels = (
            u32::from(size.width) * PIXEL_PRECISION / u32::from(size.columns),
            u32::from(size.height) * PIXEL_PRECISION / u32::from(size.rows),
        );
    }

    /// Re-size to `area`, drawing a fresh set of characters and putting
    /// the strip back at the edge it enters by. Does nothing when the
    /// area has not changed.
    fn resize(&mut self, area: Rect) {
        if self.columns == area.width && self.rows == area.height {
            return;
        }
        self.columns = area.width;
        self.rows = area.height;
        self.leading_edge = 0;
        self.rolled_through = 0;
        self.read_cell_pixels();
        // The grid is the ceiling on how deep the strip stands, so a
        // grid that has just become smaller than the strip lowers it.
        self.set_width(self.width);
        let cells = usize::from(area.width) * usize::from(area.height);
        let mut glyphs = Vec::with_capacity(cells);
        for _ in 0..cells {
            glyphs.push(random::random_glyph(&mut self.xorshift));
        }
        self.glyphs = glyphs;
        // Long enough for the longer of the two axes, so turning the
        // strip round needs no second draw: the offsets it runs over
        // are the rows one way and the columns the other.
        let offsets = usize::from(area.width.max(area.height));
        self.tails = vec![EdgeRun::at(u8::MAX); offsets];
        self.heads = vec![EdgeRun::at(0); offsets];
        self.phases = (0..offsets).map(|_| self.xorshift.byte()).collect();
    }

    /// Carry every offset of every fraying edge one frame further
    /// along.
    ///
    /// Each is travelling toward a depth of its own or standing at one,
    /// so what an edge does over a second is grow and shrink rather
    /// than jump. An edge that is not fraying is left where it stands.
    fn advance_edges(&mut self, elapsed: Duration) {
        let elapsed_millis = u32::try_from(elapsed.as_millis()).unwrap_or(u32::MAX);
        let travel =
            u8::try_from(elapsed_millis.saturating_mul(self.tail_speed) / MILLIS_PER_SECOND)
                .unwrap_or(u8::MAX);
        let hold = self.tail_hold();
        if self.fraying.trailing() {
            advance_runs(&mut self.tails, &mut self.xorshift, elapsed, travel, hold);
        }
        if self.fraying.leading() {
            advance_runs(&mut self.heads, &mut self.xorshift, elapsed, travel, hold);
        }
    }

    /// How long an offset stands at the depth it reached before drawing
    /// a fresh one, which is a share of what the walk across the whole
    /// range costs at the speed the trailing edge is fraying.
    fn tail_hold(&self) -> Duration {
        let full_range_millis =
            u32::from(u8::MAX).saturating_mul(MILLIS_PER_SECOND) / self.tail_speed.max(1);
        Duration::from_millis(u64::from(
            full_range_millis.saturating_mul(VARIABLE_TAIL_HOLD_PERCENT) / WHOLE_PERCENT,
        ))
    }

    /// Draw a fresh character for every cell on every line the leading
    /// edge has reached since the last frame.
    fn roll_reached_lines(&mut self) {
        while self.rolled_through < self.lines()
            && u32::from(self.rolled_through) * SUBCELLS_PER_CELL <= self.leading_edge
        {
            for offset in 0..self.cells_per_line() {
                let (column, row) = self.cell_on_line(self.rolled_through, offset);
                let index = self.cell_index(column, row);
                let glyph = random::random_glyph(&mut self.xorshift);
                if let Some(slot) = self.glyphs.get_mut(index) {
                    *slot = glyph;
                }
            }
            self.rolled_through += 1;
        }
    }

    /// Re-draw a few cells at random, which is what keeps the strip
    /// shimmering between one leading edge and the next.
    fn churn(&mut self) {
        for _ in 0..CHURN_CELLS_PER_FRAME {
            let index = self.xorshift.index(self.glyphs.len());
            let glyph = random::random_glyph(&mut self.xorshift);
            if let Some(slot) = self.glyphs.get_mut(index) {
                *slot = glyph;
            }
        }
    }
}

/// Carry one edge's offsets a frame on. One draw per offset is rolled
/// up front, whether or not the offset has run out of stand to use it,
/// so the strip's one generator stays where the rest of its randomness
/// comes from.
fn advance_runs(
    runs: &mut [EdgeRun],
    xorshift: &mut Xorshift,
    elapsed: Duration,
    travel: u8,
    hold: Duration,
) {
    for index in 0..runs.len() {
        let drawn = xorshift.byte();
        if let Some(run) = runs.get_mut(index) {
            run.advance(elapsed, travel, hold, drawn);
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use super::*;
    use crate::backdrop::constants::GLYPHS;

    /// An area big enough that the strip covers only part of it, so a
    /// covered and an uncovered line can both be asserted on.
    const AREA: Rect = Rect::new(0, 0, 80, 10);

    /// A strip shallower than the shorter side of [`AREA`].
    ///
    /// The default width stands deeper than the area is tall, and a
    /// wrapping strip that deep leaves no gap behind its tail at all --
    /// correct, and no use for asking where it stops.
    const NARROW: u32 = 4;

    /// A strip sized to [`AREA`] with its leading edge one whole band
    /// width in, so the cell it entered by is at the very end of the
    /// tail and the edge itself is inside the area.
    fn entered(direction: BandDirection) -> TravelingBand {
        let mut band = TravelingBand::new();
        // Where the strip stops is what these ask about, and a fraying
        // edge is a second thing moving it. The tests that want the
        // fraying ask for it.
        band.fraying = BandFraying::Neither;
        band.narrow(u32::MAX);
        band.widen(NARROW - MIN_BAND_WIDTH);
        band.set_direction(direction);
        band.advance(AREA, Duration::ZERO);
        band.leading_edge = band.width * SUBCELLS_PER_CELL;
        band
    }

    #[test]
    fn a_strip_sizes_itself_to_the_area_it_is_advanced_against() {
        let mut band = TravelingBand::new();

        band.advance(AREA, Duration::ZERO);

        assert_eq!(band.columns, AREA.width);
        assert_eq!(band.rows, AREA.height);
        assert_eq!(band.glyphs.len(), usize::from(AREA.width * AREA.height));
    }

    /// The strip stands its own width deep and stops: the cell one
    /// past its leading edge is not covered, and neither is the one
    /// past its tail.
    ///
    /// Its own width deep means exactly that many lines lit. The line
    /// the edge has arrived at but not yet entered is lit by nothing,
    /// so it is one of the ones past the leading edge rather than the
    /// leading edge itself.
    #[test]
    fn the_strip_stops_at_its_own_width() {
        let band = entered(BandDirection::Right);
        let edge = u16::try_from(band.width.saturating_sub(1)).unwrap_or(u16::MAX);

        assert!(
            band.covers(edge, 0),
            "the leading edge is part of the strip"
        );
        assert!(band.covers(0, 0), "so is the far end of the tail");
        assert!(!band.covers(edge + 1, 0), "the cell ahead of it is not");
        assert!(
            !band.covers(AREA.width - 1, 0),
            "nor is the cell the tail has already left",
        );
    }

    /// Standing as deep as the grid has lines, the strip is the whole
    /// ring and there is no gap anywhere in it.
    ///
    /// Both of its edges are runs on that ring, so at this depth the
    /// tail has met the leading edge and the line the edge is part way
    /// across is the same line the tail is part way off. Owning only
    /// the leading share of it left one column short of full at every
    /// position the strip could be in.
    #[test]
    fn a_strip_as_deep_as_the_grid_leaves_no_line_unlit() {
        let mut band = entered(BandDirection::Right);
        band.widen(u32::from(AREA.width) - NARROW);

        assert_eq!(band.width, u32::from(AREA.width));
        for step in 0..8 {
            band.leading_edge = step * SUBCELLS_PER_CELL / 8;
            for column in 0..AREA.width {
                assert_eq!(
                    band.coverage(column, 0),
                    Some(u8::MAX),
                    "column {column} is short of full at step {step}"
                );
            }
        }
    }

    /// A strip with no area yet covers nothing, rather than reading its
    /// own emptiness as full coverage.
    #[test]
    fn an_unsized_strip_covers_nothing() {
        assert!(!TravelingBand::new().covers(0, 0));
    }

    /// The strip wraps: its tail is still leaving the far edge while
    /// its leading edge is back at the near one. Finishing each pass
    /// before starting the next would leave the screen empty for as
    /// long again as the strip took to cross it.
    #[test]
    fn the_tail_is_still_leaving_the_far_edge_as_the_leading_edge_comes_back() {
        let mut band = entered(BandDirection::Right);
        // Two cells into a fresh pass, with most of the tail still to
        // come off the other end.
        band.leading_edge = 2 * SUBCELLS_PER_CELL;

        assert!(
            band.covers(0, 0),
            "the new pass has started at the near edge"
        );
        assert!(
            band.covers(AREA.width - 1, 0),
            "and the last pass has not finished leaving the far one",
        );
    }

    /// The position stays inside one pass's worth of travel however
    /// long the strip runs.
    #[test]
    fn the_position_wraps_rather_than_running_away() {
        let mut band = TravelingBand::new();
        band.advance(AREA, Duration::ZERO);
        let span = u32::from(AREA.width) * SUBCELLS_PER_CELL;

        // Long enough for several passes, so the wrap is exercised
        // rather than merely reached.
        for _ in 0..600 {
            band.advance(AREA, Duration::from_millis(16));
            assert!(band.leading_edge < span);
        }
    }

    /// A cell in the body of the strip wears the colour of the backdrop
    /// under it, which is the whole point of drawing over one.
    ///
    /// The edge is carried a whole cell past the origin first, so the
    /// cell read is one the strip has fully arrived on rather than the
    /// one it is still entering -- that line is the single exception,
    /// and [`the_line_the_edge_is_entering_is_lit_in_proportion_to_how_far_in_it_is`]
    /// is what covers it.
    #[test]
    fn a_covered_cell_is_drawn_in_the_colour_behind_it() {
        let color = Color::Rgb(200, 100, 50);
        let mut band = TravelingBand::new();
        band.advance(AREA, Duration::ZERO);
        band.leading_edge = SUBCELLS_PER_CELL;
        let backdrop = Backdrop::flat(AREA, color);
        let mut buffer = Buffer::empty(AREA);

        band.render(AREA, &backdrop, Color::Black, &mut buffer);

        let cell = buffer
            .cell((AREA.x, AREA.y))
            .expect("area covers its own origin");
        assert_eq!(cell.fg, color);
        assert!(GLYPHS.contains(&cell.symbol().chars().next().unwrap_or(' ')));
    }

    /// The one thing the strip is for is showing what the desktop
    /// behind the window looks like, so every cell of its body is that
    /// colour exactly -- the tail, and everything up to the line the
    /// edge is still entering. A cell carried any distance toward the
    /// ground shows something that is not behind the window at all, and
    /// where the window is transparent the ground is not what the
    /// reader is looking at either.
    ///
    /// The one line the edge is part of the way into is the exception,
    /// and it is bought deliberately: without it the strip cannot
    /// change between one whole cell and the next, and a whole cell
    /// arriving on an uneven beat is what the eye reads as stepping
    /// rather than as travel. One line of a strip twenty deep pays for
    /// the other nineteen moving smoothly.
    #[test]
    fn the_whole_strip_is_the_desktop_colour_front_to_back() {
        let color = Color::Rgb(200, 100, 50);
        let band = entered(BandDirection::Right);
        let backdrop = Backdrop::flat(AREA, color);
        let mut buffer = Buffer::empty(AREA);

        band.render(AREA, &backdrop, Color::Black, &mut buffer);

        for column in 0..u16::try_from(band.width).unwrap_or(u16::MAX) {
            let cell = buffer
                .cell((AREA.x + column, AREA.y))
                .expect("area covers the strip");
            assert_eq!(
                cell.fg, color,
                "column {column} should wear the desktop's colour"
            );
        }
    }

    /// The line the leading edge is part of the way into is lit that
    /// far and no further, and lit differently at two different points
    /// within the same cell.
    ///
    /// That second half is the whole reason the shading exists: it is
    /// what gives a frame that has not crossed a cell boundary
    /// something to show, and so what makes the strip travel rather
    /// than step.
    #[test]
    fn the_line_the_edge_is_entering_is_lit_in_proportion_to_how_far_in_it_is() {
        let color = Color::Rgb(200, 100, 50);
        let backdrop = Backdrop::flat(AREA, color);
        let leading = u16::try_from(entered(BandDirection::Right).width).unwrap_or(u16::MAX);
        let lit_at = |into: u32| {
            let mut band = entered(BandDirection::Right);
            band.leading_edge = band.width * SUBCELLS_PER_CELL + into;
            let mut buffer = Buffer::empty(AREA);
            band.render(AREA, &backdrop, Color::Black, &mut buffer);
            buffer
                .cell((AREA.x + leading, AREA.y))
                .expect("area covers the leading line")
                .fg
        };

        let quarter = lit_at(SUBCELLS_PER_CELL / 4);
        let most = lit_at(SUBCELLS_PER_CELL * 3 / 4);

        assert_ne!(
            quarter, color,
            "a line the edge has only entered is not yet at full strength"
        );
        assert_ne!(
            quarter, most,
            "and two points inside one cell are lit differently, which is \
             what a frame that crosses no boundary has to show"
        );
    }

    /// The counterpart of the leading edge shading the line it is
    /// entering. A strip whose tail leaves a whole line at a time has
    /// one edge travelling and one stepping, and the eye reads the
    /// stepping one -- so the last line is lit by however much of it
    /// the strip still stands on, exactly as the first is.
    #[test]
    fn the_line_the_tail_is_leaving_is_lit_in_proportion_to_how_much_is_left() {
        let color = Color::Rgb(200, 100, 50);
        let backdrop = Backdrop::flat(AREA, color);
        let lit_at = |past: u32| {
            let mut band = entered(BandDirection::Right);
            // Carry the strip `past` subcells beyond the point where its
            // tail sits exactly on the far edge of the first column, so
            // that column is the one being left.
            band.leading_edge = band.width * SUBCELLS_PER_CELL + past;
            let mut buffer = Buffer::empty(AREA);
            band.render(AREA, &backdrop, Color::Black, &mut buffer);
            buffer
                .cell((AREA.x, AREA.y))
                .expect("area covers the trailing line")
                .fg
        };

        let mostly_there = lit_at(SUBCELLS_PER_CELL / 4);
        let nearly_gone = lit_at(SUBCELLS_PER_CELL * 3 / 4);

        assert_ne!(
            nearly_gone, color,
            "a line the tail has most of the way off is no longer at full \
             strength"
        );
        assert_ne!(
            mostly_there, nearly_gone,
            "and two points inside one cell are lit differently, which is \
             what keeps the tail travelling rather than stepping"
        );
    }

    /// Varying the trailing edge shortens the run each row draws
    /// without shortening it at the leading edge and without dropping a
    /// cell out of the middle of a run. A depth read along the strip
    /// rather than across it does exactly that -- it asks whether a
    /// whole column is drawn, and a no near the trailing edge is a hole
    /// through the strip.
    #[test]
    fn a_varying_trailing_edge_frays_the_tail_and_leaves_the_leading_edge_flat() {
        let mut band = entered(BandDirection::Right);
        let full = band.width * SUBCELLS_PER_CELL;
        band.fraying = BandFraying::Trailing;
        band.tails = vec![EdgeRun::at(u8::MAX); usize::from(AREA.height)];
        band.tails[0].depth = 0;
        let edge = u16::try_from(band.width.saturating_sub(1)).unwrap_or(u16::MAX);

        assert_eq!(
            band.tail_at(0),
            full * VARIABLE_TAIL_FLOOR_PERCENT / WHOLE_PERCENT
        );
        assert_eq!(band.tail_at(1), full);
        // Travelling sideways the depth varies down the rows, so the
        // shallow row has given the tail cell up while the full-depth
        // row under it still holds that same cell.
        assert!(!band.covers(0, 0));
        assert!(band.covers(0, 1));

        for row in 0..AREA.height {
            assert!(
                band.covers(edge, row),
                "row {row} should reach the leading edge",
            );
            let drawn = (0..=edge)
                .filter(|&column| band.covers(column, row))
                .count();
            let unbroken = (0..=edge)
                .rev()
                .take_while(|&column| band.covers(column, row))
                .count();
            assert_eq!(drawn, unbroken, "row {row} should be one unbroken run");
        }

        band.fraying = BandFraying::Neither;
        assert_eq!(
            band.tail_at(0),
            full,
            "off, the strip stands its full width across",
        );
        assert!(band.covers(0, 0), "and the tail is back on the row it left");
    }

    /// An offset walks to the depth drawn for it over several frames
    /// rather than taking it up on the next one, stands there for a
    /// couple of seconds, and only then goes somewhere else. A
    /// trailing edge that took each draw immediately would boil rather
    /// than move.
    #[test]
    fn an_offset_walks_to_its_next_depth_and_stands_there_before_drawing_again() {
        let frame = Duration::from_millis(100);
        let travel = u8::try_from(
            u32::try_from(frame.as_millis()).unwrap_or(u32::MAX) * DEFAULT_TAIL_SPEED
                / MILLIS_PER_SECOND,
        )
        .unwrap_or(u8::MAX);
        let hold = TravelingBand::new().tail_hold();
        let mut run = EdgeRun::at(u8::MAX);

        // The first frame has nothing to stand out, so it draws.
        run.advance(frame, travel, hold, 0);
        assert_eq!(run.target, 0);
        assert_eq!(
            run.depth,
            u8::MAX,
            "the draw is where to go, not where to be"
        );

        // And it walks there, one frame's travel at a time.
        run.advance(frame, travel, hold, u8::MAX);
        assert_eq!(run.depth, u8::MAX - travel);
        let frames_to_arrive = u32::from(u8::MAX).div_ceil(u32::from(travel));
        for _ in 0..frames_to_arrive {
            run.advance(frame, travel, hold, u8::MAX);
        }
        assert_eq!(run.depth, 0, "it should have arrived by now");
        assert_eq!(run.holding, hold.saturating_sub(frame));

        // Arrived, it stands. A draw offered mid-stand is not taken.
        run.advance(frame, travel, hold, u8::MAX);
        assert_eq!(run.target, 0, "the stand is not over yet");
        while !run.holding.is_zero() {
            run.advance(frame, travel, hold, 0);
        }

        run.advance(frame, travel, hold, u8::MAX);
        assert_eq!(run.target, u8::MAX, "the stand is over, so it draws again");
    }

    /// Nothing is drawn once the strip has been carried the whole way
    /// to the ground -- that is how it leaves the screen.
    #[test]
    fn a_fully_faded_strip_draws_nothing() {
        let mut band = TravelingBand::new();
        band.advance(AREA, Duration::ZERO);
        band.fade(u8::MAX);
        let backdrop = Backdrop::flat(AREA, Color::Rgb(200, 100, 50));
        let mut buffer = Buffer::empty(AREA);

        band.render(AREA, &backdrop, Color::Black, &mut buffer);

        assert_eq!(buffer, Buffer::empty(AREA));
    }

    /// Each direction reads its distance from the edge it enters by, so
    /// the tail is at that edge and the leading edge is one band width
    /// in from it, whichever way the strip is going.
    #[test]
    fn every_direction_trails_back_to_the_edge_it_entered_by() {
        let last_column = AREA.width - 1;
        let last_row = AREA.height - 1;
        let cases = [
            (BandDirection::Right, (0, 0), (last_column, 0)),
            (BandDirection::Left, (last_column, 0), (0, 0)),
            (BandDirection::Down, (0, 0), (0, last_row)),
            (BandDirection::Up, (0, last_row), (0, 0)),
        ];

        for (direction, (column, row), (far_column, far_row)) in cases {
            let band = entered(direction);
            assert!(
                band.covers(column, row),
                "{direction:?} should trail back to the cell it entered by",
            );
            assert!(
                !band.covers(far_column, far_row),
                "{direction:?} should not have reached the far edge yet",
            );
        }
    }

    /// Travelling sideways the strip is a column the full height of the
    /// area; travelling up or down it is a row the full width of it.
    #[test]
    fn a_strip_crossing_sideways_stands_the_other_way_up_from_one_crossing_vertically() {
        let across = entered(BandDirection::Right);
        let down = entered(BandDirection::Down);
        let inside = u16::try_from(across.width).unwrap_or(u16::MAX) - 1;
        let outside = inside + 2;

        // One column of the sideways strip covers every row it crosses,
        // and the column past its leading edge covers none of them.
        assert!(across.covers(inside, 0) && across.covers(inside, 9));
        assert!(!across.covers(outside, 0) && !across.covers(outside, 9));
        // And one row of the vertical strip covers every column.
        assert!(down.covers(0, inside) && down.covers(79, inside));
        assert!(!down.covers(0, outside) && !down.covers(79, outside));
    }

    /// Turning the strip round puts it back at the edge it now enters
    /// by. Carrying the old position across would read a strip most of
    /// the way over as one that has only just set off the other way.
    #[test]
    fn turning_the_strip_round_starts_it_over_at_its_new_edge() {
        let mut band = entered(BandDirection::Right);
        assert_ne!(band.leading_edge, 0);

        band.set_direction(BandDirection::Left);

        assert_eq!(band.leading_edge, 0);
        assert_eq!(band.rolled_through, 0);
    }

    /// Sending the strip the way it is already going leaves it where it
    /// stands, so a key held down does not park it at the edge.
    #[test]
    fn sending_the_strip_the_way_it_already_goes_leaves_it_where_it_stands() {
        let mut band = entered(BandDirection::Right);
        let leading_edge = band.leading_edge;

        band.set_direction(BandDirection::Right);

        assert_eq!(band.leading_edge, leading_edge);
    }

    /// Width and both speeds are clamped where they are set, so a
    /// caller can hand a held key straight through without knowing the
    /// limits.
    #[test]
    fn width_and_speed_stop_at_the_limits_rather_than_running_past_them() {
        let mut band = TravelingBand::new();

        band.widen(u32::MAX);
        assert_eq!(band.width, MAX_BAND_WIDTH);
        band.narrow(u32::MAX);
        assert_eq!(band.width, MIN_BAND_WIDTH);

        band.speed_up(u32::MAX);
        assert_eq!(band.speed, MAX_BAND_SPEED);
        band.slow_down(u32::MAX);
        assert_eq!(band.speed, MIN_BAND_SPEED);

        band.tail_faster(u32::MAX);
        assert_eq!(band.tail_speed, MAX_TAIL_SPEED);
        band.tail_slower(u32::MAX);
        assert_eq!(band.tail_speed, MIN_TAIL_SPEED);
    }

    /// A strip nobody has steered sets off left to right across the
    /// whole window, with both of its edges already fraying.
    /// Everything the animation can do is on screen before a key is
    /// pressed.
    #[test]
    fn an_unsteered_strip_sets_off_across_the_whole_window() {
        let mut band = TravelingBand::new();

        band.advance(AREA, Duration::ZERO);

        assert_eq!(band.direction, BandDirection::Right);
        assert_eq!(
            band.width,
            u32::from(AREA.width),
            "the strip should stand across every column there is",
        );
        assert_eq!(
            band.fraying,
            BandFraying::Both,
            "and fray at both ends without being asked"
        );
    }

    /// One key walks all four ways the two edges can be set, and each
    /// step changes exactly one of them -- so what a press did is
    /// readable off the screen without being told.
    #[test]
    fn one_key_cycles_the_four_ways_the_edges_can_fray() {
        let mut band = TravelingBand::new();
        let mut seen = vec![band.fraying];

        for _ in 0..3 {
            band.cycle_fraying();
            seen.push(band.fraying);
        }

        assert_eq!(
            seen,
            vec![
                BandFraying::Both,
                BandFraying::Leading,
                BandFraying::Neither,
                BandFraying::Trailing,
            ]
        );
        assert!(
            seen.windows(2).all(|pair| {
                usize::from(pair[0].leading() != pair[1].leading())
                    + usize::from(pair[0].trailing() != pair[1].trailing())
                    == 1
            }),
            "each step should change exactly one edge: {seen:?}",
        );
        band.cycle_fraying();
        assert_eq!(
            band.fraying,
            BandFraying::Both,
            "and the fourth press comes back round"
        );
    }

    /// A fraying leading edge stands back from where the strip's travel
    /// says it is, at that offset and no other, and leaves the trailing
    /// edge where it was. The two edges are separate: fraying one is
    /// not a way of thinning the strip.
    #[test]
    fn a_fraying_leading_edge_stands_back_without_moving_the_tail() {
        let mut band = entered(BandDirection::Right);
        band.fraying = BandFraying::Leading;
        band.heads = vec![EdgeRun::at(0); usize::from(AREA.height)];
        band.heads[0].depth = u8::MAX;
        // Every offset back on the one start, so what the two rows
        // below differ by is the fraying this test is about and not
        // the stagger.
        band.phases.fill(0);
        let edge = u16::try_from(band.width.saturating_sub(1)).unwrap_or(u16::MAX);

        assert!(
            band.head_at(0) > 0,
            "the drawn row's leading edge should have come back"
        );
        assert_eq!(band.head_at(1), 0, "and its neighbour's should not have");
        assert!(
            band.coverage(edge, 0) < band.coverage(edge, 1),
            "so the leading line is lit less on the row that gave it up"
        );
        assert_eq!(
            band.coverage(0, 0),
            band.coverage(0, 1),
            "while the far end of the tail is untouched on both",
        );
    }

    /// Each offset carries its own start once the leading edge frays,
    /// so the run of empty grid between where the strip ends and where
    /// it begins again falls somewhere else on every one of them. On
    /// one place across all of them it is a band of nothing down the
    /// window, which is what a filled animation must not leave.
    ///
    /// The two settings with an edge the eye is meant to track keep
    /// their single start: there is nothing to follow across a window
    /// once every offset begins somewhere else.
    #[test]
    fn a_fraying_leading_edge_starts_each_offset_somewhere_else() {
        let mut band = entered(BandDirection::Right);

        band.fraying = BandFraying::Both;
        assert!(
            (1..AREA.height).any(|offset| band.phase_at(offset) != band.phase_at(0)),
            "the offsets should not all start together"
        );

        band.fraying = BandFraying::Trailing;
        assert!(
            (0..AREA.height).all(|offset| band.phase_at(offset) == 0),
            "an edge the eye tracks keeps its one start"
        );
    }

    /// Both edges at the far end of their ranges at once still leave a
    /// core standing. The leading edge's ceiling is held under the
    /// trailing edge's floor for exactly this: two edges free to meet
    /// would put a row of the strip out altogether, and a strip with
    /// rows missing from the middle reads as pieces rather than as one
    /// thing.
    #[test]
    fn a_strip_frayed_at_both_ends_keeps_a_core_at_every_offset() {
        let mut band = entered(BandDirection::Right);
        band.fraying = BandFraying::Both;
        band.heads = vec![EdgeRun::at(u8::MAX); usize::from(AREA.height)];
        band.tails = vec![EdgeRun::at(0); usize::from(AREA.height)];

        for row in 0..AREA.height {
            let offset = band.offset_of(0, row);
            assert!(
                band.tail_at(offset) > band.head_at(offset),
                "row {row} should keep a core with both edges at their limits",
            );
            assert!(
                (0..AREA.width).any(|column| band.covers(column, row)),
                "so row {row} should still be drawn somewhere",
            );
        }
    }

    /// An edge that has stopped fraying is put back flat, so the next
    /// time round it starts flat and frays outward rather than snapping
    /// to wherever it was left three presses ago.
    #[test]
    fn an_edge_that_stops_fraying_is_put_back_flat() {
        let mut band = entered(BandDirection::Right);
        band.fraying = BandFraying::Both;
        band.tails = vec![EdgeRun::at(0); usize::from(AREA.height)];
        band.heads = vec![EdgeRun::at(u8::MAX); usize::from(AREA.height)];

        // Both -> Leading puts the trailing edge away, and the step
        // after that puts the leading one away too.
        band.cycle_fraying();
        assert_eq!(band.tails[0].depth, u8::MAX, "the tail is flat again");
        band.cycle_fraying();
        assert_eq!(band.heads[0].depth, 0, "and so is the leading edge");
    }

    /// One key governs the whole of how fast the trailing edge changes:
    /// the stand at a depth is taken from the speed rather than fixed,
    /// so speeding the fraying up shortens the stand too.
    ///
    /// Fixing the stand instead left it outlasting the travel at the
    /// top of the range, where the fastest setting looked no livelier
    /// than the middle of it.
    #[test]
    fn fraying_faster_shortens_the_stand_as_well_as_the_walk() {
        let mut band = TravelingBand::new();
        let settled = band.tail_hold();

        band.tail_faster(u32::MAX);
        let hurried = band.tail_hold();
        band.tail_slower(u32::MAX);
        let dawdling = band.tail_hold();

        assert!(
            hurried < settled && settled < dawdling,
            "the stand should follow the speed: {hurried:?} < {settled:?} < {dawdling:?}",
        );
        assert!(!hurried.is_zero(), "even the fastest setting stands");
    }

    /// The slowest strip still moves. A strip standing still is one the
    /// reader cannot tell from a frozen display.
    #[test]
    fn the_slowest_strip_still_travels() {
        let mut band = TravelingBand::new();
        band.advance(AREA, Duration::ZERO);
        band.slow_down(u32::MAX);

        band.advance(AREA, Duration::from_secs(1));

        assert_eq!(band.leading_edge, MIN_BAND_SPEED * SUBCELLS_PER_CELL);
    }

    /// Speed is cells per second rather than a step per frame, so twice
    /// the elapsed time is twice the travel.
    #[test]
    fn the_strip_travels_its_speed_in_cells_each_second() {
        let mut band = TravelingBand::new();
        band.advance(AREA, Duration::ZERO);
        let speed = band.speed;

        band.advance(AREA, Duration::from_millis(500));

        assert_eq!(band.leading_edge, speed * SUBCELLS_PER_CELL / 2);
    }
}
