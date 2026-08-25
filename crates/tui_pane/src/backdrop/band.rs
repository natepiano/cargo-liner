//! The attract-mode animation: a lit strip of characters crossing the
//! grid, each cell wearing the colour of the desktop behind it.
//!
//! The strip travels one of the four ways a [`BandDirection`] names,
//! and it wraps: its tail is still leaving one edge while its leading
//! edge is coming back in at the other, so the grid is never empty
//! between one pass and the next.
//!
//! Every cell it covers is drawn in exactly the colour the [`Backdrop`]
//! has there -- no lift at the front, no ramp along the tail. A
//! terminal cell is opaque and carries no alpha, so anything done to
//! that colour is done to what the reader came to look at: the strip's
//! one subject is the desktop the window is standing on, and a cell
//! wearing a mixture is a cell showing something that is not there.
//!
//! What gives the strip edges to read, then, is where it stops. The
//! leading edge is flat across every line. So is the trailing edge,
//! until [`TravelingBand::toggle_variable_tail`] is called -- after
//! which the strip runs back its own distance at every offset across
//! itself, and those distances grow and shrink between a third of its
//! width and all of it while it travels.
//!
//! Position is tracked in whole numbers throughout. A strip that moves
//! a fraction of a cell per frame wants sub-cell precision, and
//! carrying that as a float would put a truncating cast in the middle
//! of every cell's colour.

use std::time::Duration;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;

use super::Backdrop;
use super::constants::CHURN_CELLS_PER_FRAME;
use super::constants::DEFAULT_BAND_SPEED;
use super::constants::DEFAULT_BAND_WIDTH;
use super::constants::GLYPHS;
use super::constants::MAX_BAND_SPEED;
use super::constants::MAX_BAND_WIDTH;
use super::constants::MILLIS_PER_SECOND;
use super::constants::MIN_BAND_SPEED;
use super::constants::MIN_BAND_WIDTH;
use super::constants::SUBCELLS_PER_CELL;
use super::constants::VARIABLE_TAIL_FLOOR_PERCENT;
use super::constants::VARIABLE_TAIL_HOLD;
use super::constants::VARIABLE_TAIL_TRAVEL_PER_SECOND;
use super::constants::WHOLE_PERCENT;
use super::constants::XORSHIFT_FALLBACK_SEED;
use super::constants::XORSHIFT_FIRST;
use super::constants::XORSHIFT_SECOND;
use super::constants::XORSHIFT_THIRD;
use crate::theme::blend_color;

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

/// Xorshift64, seeded from the clock.
///
/// The character churn needs a cheap varying number and nothing more,
/// so this stands in for a dependency on a real generator.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Xorshift(u64);

impl Default for Xorshift {
    fn default() -> Self {
        let seed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since_epoch| {
                u64::try_from(since_epoch.as_nanos()).unwrap_or(0)
            });
        Self(if seed == 0 {
            XORSHIFT_FALLBACK_SEED
        } else {
            seed
        })
    }
}

impl Xorshift {
    /// The next number in the sequence.
    const fn roll(&mut self) -> u64 {
        self.0 ^= self.0 << XORSHIFT_FIRST;
        self.0 ^= self.0 >> XORSHIFT_SECOND;
        self.0 ^= self.0 << XORSHIFT_THIRD;
        self.0
    }

    /// A number in `0..len`, or zero where `len` is zero.
    fn index(&mut self, len: usize) -> usize {
        let Ok(len) = u64::try_from(len) else {
            return 0;
        };
        if len == 0 {
            return 0;
        }
        usize::try_from(self.roll() % len).unwrap_or(0)
    }
}

/// How far back the strip runs at one offset across itself, and where
/// that is heading.
///
/// A depth drawn at random and taken up on the next frame would read as
/// a trailing edge boiling rather than as one moving, so a fresh draw
/// is a place to travel to: the offset walks there over as many frames
/// as [`VARIABLE_TAIL_TRAVEL_PER_SECOND`] takes, stands at it for
/// [`VARIABLE_TAIL_HOLD`], and only then draws again.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TailRun {
    /// How far back the strip runs here now, on the scale
    /// [`TravelingBand::tail_at`] reads.
    depth:   u8,
    /// Where that is heading.
    target:  u8,
    /// What is left of the stand at [`Self::target`] before a fresh one
    /// is drawn. Only counts down once the target has been reached.
    holding: Duration,
}

impl TailRun {
    /// An offset standing at the full width with nowhere to travel,
    /// which is where every one of them starts: the strip sets off as
    /// flat behind as it is in front and frays from there.
    const fn full() -> Self {
        Self {
            depth:   u8::MAX,
            target:  u8::MAX,
            holding: Duration::ZERO,
        }
    }

    /// Carry the offset one frame on: `travel` further toward its
    /// target, or `elapsed` further through the stand it is keeping at
    /// one, taking `drawn` as its next target when that stand runs out.
    ///
    /// `drawn` is handed in already rolled rather than rolled here, so
    /// the strip's one generator stays where the rest of its randomness
    /// comes from. Most frames it goes unused.
    fn advance(&mut self, elapsed: Duration, travel: u8, drawn: u8) {
        if self.depth != self.target {
            self.depth = if self.depth < self.target {
                self.depth.saturating_add(travel).min(self.target)
            } else {
                self.depth.saturating_sub(travel).max(self.target)
            };
            if self.depth == self.target {
                self.holding = VARIABLE_TAIL_HOLD;
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
    /// How far back the strip runs at each offset across itself while
    /// [`Self::variable_tail`] is on, read by [`Self::tail_at`]. Zero
    /// is the shallowest it goes and [`u8::MAX`] is the full width, and
    /// each of them is travelling between the two rather than sitting
    /// still -- see [`TailRun`].
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
    tails:          Vec<TailRun>,
    /// Cells across the area the strip was last sized to.
    columns:        u16,
    /// Cells down that same area.
    rows:           u16,
    /// Which way the strip is travelling.
    direction:      BandDirection,
    /// How deep the strip stands, in cells along the axis it travels.
    width:          u32,
    /// How far the strip travels each second, in cells.
    speed:          u32,
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
    /// Whether how far the strip runs back varies across it, which is
    /// what makes the trailing edge ragged. Off, it stands the full
    /// width the whole way across and the trailing edge is as flat as
    /// the leading one.
    variable_tail:  bool,
}

impl Default for TravelingBand {
    fn default() -> Self {
        Self {
            leading_edge:   0,
            glyphs:         Vec::new(),
            tails:          Vec::new(),
            columns:        0,
            rows:           0,
            direction:      BandDirection::default(),
            width:          DEFAULT_BAND_WIDTH,
            speed:          DEFAULT_BAND_SPEED,
            rolled_through: 0,
            xorshift:       Xorshift::default(),
            faded:          0,
            variable_tail:  false,
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
        let elapsed_millis = u32::try_from(elapsed.as_millis()).unwrap_or(u32::MAX);
        let travel = self
            .speed
            .saturating_mul(SUBCELLS_PER_CELL)
            .saturating_mul(elapsed_millis)
            / MILLIS_PER_SECOND;
        self.leading_edge = self.leading_edge.saturating_add(travel);

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
        if self.variable_tail {
            self.advance_tails(elapsed);
        }
        self.roll_reached_lines();
        self.churn();
    }

    /// Carry the whole strip `faded` of the way toward the ground it is
    /// drawn on, which is how it leaves when the screen it decorates
    /// has something real to show. Zero is full strength, [`u8::MAX`]
    /// draws nothing at all.
    pub const fn fade(&mut self, faded: u8) { self.faded = faded; }

    /// Turn the ragged trailing edge on or off.
    ///
    /// On, the strip runs back its own distance at each offset across
    /// itself -- every row of a strip crossing sideways, every column
    /// of one crossing up or down -- and those distances grow and
    /// shrink between a third of its width and the whole of it while it
    /// travels. Each is a walk toward a depth drawn for it and a stand
    /// of a couple of seconds once it arrives, so the trailing edge
    /// moves rather than boiling.
    ///
    /// The leading edge stays flat either way: it is the edge the eye
    /// tracks, and one that arrived at a different moment on every row
    /// reads as noise rather than as travel.
    pub const fn toggle_variable_tail(&mut self) { self.variable_tail = !self.variable_tail; }

    /// Send the strip a different way.
    ///
    /// It restarts from the edge it now enters by rather than carrying
    /// its position across: the position is measured from that edge, so
    /// a reversal read the old number as a strip most of the way to the
    /// far side rather than one just setting off.
    pub const fn set_direction(&mut self, direction: BandDirection) {
        if matches!(
            (self.direction, direction),
            (BandDirection::Left, BandDirection::Left)
                | (BandDirection::Right, BandDirection::Right)
                | (BandDirection::Up, BandDirection::Up)
                | (BandDirection::Down, BandDirection::Down)
        ) {
            return;
        }
        self.direction = direction;
        self.leading_edge = 0;
        self.rolled_through = 0;
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
                if !self.covers(column, row) {
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
                    cell.set_char(glyph);
                    cell.set_fg(blend_color(color, toward, self.faded));
                }
            }
        }
    }

    /// Whether the strip covers the cell at `column`, `row` this frame.
    ///
    /// Distance behind the leading edge is measured the long way round,
    /// so a line the edge has not reached on this pass is read as one
    /// its tail has not finished leaving on the last -- which is what
    /// the wrap means.
    fn covers(&self, column: u16, row: u16) -> bool {
        let span = self.span();
        if span == 0 {
            return false;
        }
        let line = self.line_of(column, row);
        let behind = (self.leading_edge + span - u32::from(line) * SUBCELLS_PER_CELL) % span;
        behind <= self.tail_at(self.offset_of(column, row))
    }

    /// How far back the strip runs at `offset` across itself, in
    /// sub-cells behind the leading edge.
    ///
    /// The full width unless the trailing edge is varying, in which
    /// case that offset's own draw carries it from the floor
    /// [`VARIABLE_TAIL_FLOOR_DIVISOR`] sets up to that full width.
    fn tail_at(&self, offset: u16) -> u32 {
        let full = self.width * SUBCELLS_PER_CELL;
        if !self.variable_tail {
            return full;
        }
        let floor = full * VARIABLE_TAIL_FLOOR_PERCENT / WHOLE_PERCENT;
        let depth = self
            .tails
            .get(usize::from(offset))
            .map_or(u8::MAX, |run| run.depth);
        floor + (full - floor) * u32::from(depth) / u32::from(u8::MAX)
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
    fn set_width(&mut self, width: u32) {
        self.width = width.clamp(MIN_BAND_WIDTH, MAX_BAND_WIDTH);
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
        let cells = usize::from(area.width) * usize::from(area.height);
        let mut glyphs = Vec::with_capacity(cells);
        for _ in 0..cells {
            glyphs.push(random_glyph(&mut self.xorshift));
        }
        self.glyphs = glyphs;
        // Long enough for the longer of the two axes, so turning the
        // strip round needs no second draw: the offsets it runs over
        // are the rows one way and the columns the other.
        self.tails = vec![TailRun::full(); usize::from(area.width.max(area.height))];
    }

    /// Carry every offset across the strip one frame further along.
    ///
    /// Each is travelling toward a depth of its own or standing at one,
    /// so what the trailing edge does over a second is grow and shrink
    /// rather than jump. One draw per offset is rolled up front,
    /// whether or not the offset has run out of stand to use it.
    fn advance_tails(&mut self, elapsed: Duration) {
        let elapsed_millis = u32::try_from(elapsed.as_millis()).unwrap_or(u32::MAX);
        let travel = u8::try_from(
            elapsed_millis.saturating_mul(VARIABLE_TAIL_TRAVEL_PER_SECOND) / MILLIS_PER_SECOND,
        )
        .unwrap_or(u8::MAX);
        for index in 0..self.tails.len() {
            let drawn = random_tail(&mut self.xorshift);
            if let Some(run) = self.tails.get_mut(index) {
                run.advance(elapsed, travel, drawn);
            }
        }
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
                let glyph = random_glyph(&mut self.xorshift);
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
            let glyph = random_glyph(&mut self.xorshift);
            if let Some(slot) = self.glyphs.get_mut(index) {
                *slot = glyph;
            }
        }
    }
}

/// One character drawn at random from [`GLYPHS`].
fn random_glyph(xorshift: &mut Xorshift) -> char {
    let index = xorshift.index(GLYPHS.len());
    GLYPHS.get(index).copied().unwrap_or(' ')
}

/// One depth drawn at random, on the scale [`TravelingBand::tail_at`]
/// reads. Where an offset across the strip is sent next.
fn random_tail(xorshift: &mut Xorshift) -> u8 {
    u8::try_from(xorshift.index(usize::from(u8::MAX) + 1)).unwrap_or(u8::MAX)
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use super::*;

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
    #[test]
    fn the_strip_stops_at_its_own_width() {
        let band = entered(BandDirection::Right);
        let edge = u16::try_from(band.width).unwrap_or(u16::MAX);

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

    /// A cell the strip covers wears the colour of the backdrop under
    /// it, which is the whole point of drawing over one.
    #[test]
    fn a_covered_cell_is_drawn_in_the_colour_behind_it() {
        let color = Color::Rgb(200, 100, 50);
        let mut band = TravelingBand::new();
        band.advance(AREA, Duration::ZERO);
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
    /// behind the window looks like, so every cell of it is that colour
    /// exactly -- the leading edge, the tail, and everything between.
    /// A cell carried any distance toward the ground shows something
    /// that is not behind the window at all, and where the window is
    /// transparent the ground is not what the reader is looking at
    /// either.
    #[test]
    fn the_whole_strip_is_the_desktop_colour_front_to_back() {
        let color = Color::Rgb(200, 100, 50);
        let band = entered(BandDirection::Right);
        let backdrop = Backdrop::flat(AREA, color);
        let mut buffer = Buffer::empty(AREA);

        band.render(AREA, &backdrop, Color::Black, &mut buffer);

        for column in 0..=u16::try_from(band.width).unwrap_or(u16::MAX) {
            let cell = buffer
                .cell((AREA.x + column, AREA.y))
                .expect("area covers the strip");
            assert_eq!(
                cell.fg, color,
                "column {column} should wear the desktop's colour"
            );
        }
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
        band.toggle_variable_tail();
        band.tails = vec![TailRun::full(); usize::from(AREA.height)];
        band.tails[0].depth = 0;
        let edge = u16::try_from(band.width).unwrap_or(u16::MAX);

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

        band.toggle_variable_tail();
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
            u32::try_from(frame.as_millis()).unwrap_or(u32::MAX) * VARIABLE_TAIL_TRAVEL_PER_SECOND
                / MILLIS_PER_SECOND,
        )
        .unwrap_or(u8::MAX);
        let mut run = TailRun::full();

        // The first frame has nothing to stand out, so it draws.
        run.advance(frame, travel, 0);
        assert_eq!(run.target, 0);
        assert_eq!(
            run.depth,
            u8::MAX,
            "the draw is where to go, not where to be"
        );

        // And it walks there, one frame's travel at a time.
        run.advance(frame, travel, u8::MAX);
        assert_eq!(run.depth, u8::MAX - travel);
        let frames_to_arrive = u32::from(u8::MAX).div_ceil(u32::from(travel));
        for _ in 0..frames_to_arrive {
            run.advance(frame, travel, u8::MAX);
        }
        assert_eq!(run.depth, 0, "it should have arrived by now");
        assert_eq!(run.holding, VARIABLE_TAIL_HOLD.saturating_sub(frame));

        // Arrived, it stands. A draw offered mid-stand is not taken.
        run.advance(frame, travel, u8::MAX);
        assert_eq!(run.target, 0, "the stand is not over yet");
        while !run.holding.is_zero() {
            run.advance(frame, travel, 0);
        }

        run.advance(frame, travel, u8::MAX);
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

    /// Width and speed are clamped where they are set, so a caller can
    /// hand a held key straight through without knowing the limits.
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
