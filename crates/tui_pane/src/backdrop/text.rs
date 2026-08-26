//! The other attract-mode animation: the whole window filled with
//! bars, drifting line by line in the colours of the desktop behind
//! it.
//!
//! Where a [`TravelingBand`](super::TravelingBand) is one strip with
//! two edges and empty grid either side of it, this leaves no cell
//! undrawn. Every cell takes the colour the [`Backdrop`] has for it, so
//! the desktop reads through a window that is entirely filled rather
//! than through a strip crossing it.
//!
//! Every cell keeps the colour of whatever it is over, always. What
//! travels is only the pattern drawn on top of it: each line is dealt a
//! ring of numbers, and the ring turns. Nothing about the desktop moves,
//! because moving it is the one thing that would stop it being the
//! desktop -- a cell wearing one place's colour and another place's
//! pattern reads as smoke rather than as a window.
//!
//! [`TextFill`] says how that number is drawn. As a character it is the
//! animation this started as. As a bar it is how much of the cell is
//! lit, which is the reading that can be drawn part way.
//!
//! A line is a row while the field drifts sideways and a column while
//! it drifts up or down, and every line is a ring: what leaves one edge
//! comes round at the other, so the field never runs out.
//!
//! The bars are what let a line be drawn where it actually is. Eight
//! steps of fill are eight positions inside one cell, so the ring is
//! read between two of its numbers rather than at one of them. Drawn on
//! whole cells only -- which is all a character can do -- a line at a
//! few cells a second holds still and then jumps, and the eye reads
//! that as stepping rather than as travel.
//!
//! What keeps it from reading as one rigid sheet sliding past is that
//! the lines need not travel together. [`TextDrift`] says whether they
//! do, and while they do not, each line's own speed stands somewhere in
//! a spread around the field's -- so lines that started flush come
//! apart, and how fast they do is steerable. Where in that spread each
//! line stands is not drawn line by line but dealt in lanes: see
//! [`DriftingText::lanes`].
//!
//! Position is tracked in whole numbers throughout, for the same
//! reason it is in the band: a line moving a fraction of a cell per
//! frame wants sub-cell precision, and carrying that as a float would
//! put a truncating cast in the middle of every frame.

use std::time::Duration;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;

use super::Backdrop;
use super::BandDirection;
use super::constants::BAR_LEVELS;
use super::constants::BARS_ACROSS;
use super::constants::BARS_UP;
use super::constants::DEFAULT_TEXT_SPEED;
use super::constants::DEFAULT_TEXT_SPREAD;
use super::constants::GLYPHS;
use super::constants::LANE_FRACTION_UNIT;
use super::constants::MAX_TEXT_SPEED;
use super::constants::MAX_TEXT_SPREAD;
use super::constants::MICROS_PER_SECOND;
use super::constants::MIN_TEXT_SPEED;
use super::constants::SUBCELLS_PER_CELL;
use super::constants::TEXT_BEHIND_FADE;
use super::constants::TEXT_LANE_BODY_PERCENT;
use super::constants::TEXT_LANE_COLUMNS;
use super::constants::TEXT_LANE_GIVE_PERCENT;
use super::constants::TEXT_LANE_ROWS;
use super::constants::TEXT_LANE_SPREAD_PERCENT;
use super::constants::TEXT_RIPPLE_LINES;
use super::constants::TEXT_RIPPLE_PERCENT;
use super::constants::TEXT_WAVE_SUBLINES_PER_SECOND;
use super::constants::WHOLE_PERCENT;
use super::random::Xorshift;
use crate::theme;

/// Whether the lines of a [`DriftingText`] travel as one or at speeds
/// of their own.
///
/// Two answers rather than a spread of them, because the difference is
/// not one of degree: lines at one speed hold whatever arrangement they
/// were put in forever, and lines at speeds of their own never hold one
/// twice. How far apart the second sends them is a separate question --
/// see [`DriftingText::spread_wider`].
///
/// [`next`](Self::next) is what one key toggling them walks along.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum TextDrift {
    /// Every line at the field's own speed, and all of them put back
    /// flush when this is turned on, so the window slides as one piece.
    Together,
    /// Each line at a speed of its own, drawn from the spread around
    /// the field's. Where text that has not been steered starts: it is
    /// the livelier of the two, and the one worth showing a reader who
    /// has pressed nothing.
    #[default]
    Apart,
}

impl TextDrift {
    /// The other of the two.
    #[must_use]
    pub const fn next(self) -> Self {
        match self {
            Self::Together => Self::Apart,
            Self::Apart => Self::Together,
        }
    }
}

/// One line of the field: the characters on it, how far it has drifted,
/// and where its own speed stands in the spread.
#[derive(Clone, Debug, Eq, PartialEq)]
struct TextLine {
    /// The number each cell of this line was dealt, as a ring. Index
    /// zero is the cell the line entered by before it had drifted at
    /// all, and [`DriftingText::draw_at`] turns the ring by how far it
    /// has come since.
    ///
    /// One ring for both readings: a character and a fill are two ways
    /// of drawing the same number, so switching between them leaves
    /// every line exactly where it stood.
    draws:    Vec<u8>,
    /// How many whole cells the line has drifted, modulo its own
    /// length.
    drifted:  u32,
    /// How far into the next cell it has come, in sub-cells.
    fraction: u32,
    /// This line's own share of the range, on top of whatever the lanes
    /// carry it at: see [`TEXT_LANE_GIVE_PERCENT`]. Two lines dealt
    /// exactly one speed never come apart however varied the rest of
    /// the field is, and the lanes hand neighbours very nearly the same
    /// number by design.
    ///
    /// Drawn once, when the line is, and held from then on. Where the
    /// line sits among the lanes is worked out afresh every frame --
    /// [`DriftingText::waved`] moves it -- but this is the one part of
    /// its speed that must not, or the field would seethe rather than
    /// drift.
    give:     u8,
}

impl TextLine {
    /// Carry the line `elapsed_micros` further at `speed` cells a
    /// second, dealing a fresh number for each whole cell it has
    /// entered by.
    fn advance(&mut self, elapsed_micros: u64, speed: u32, xorshift: &mut Xorshift) {
        let Ok(length) = u32::try_from(self.draws.len()) else {
            return;
        };
        if length == 0 {
            return;
        }
        let travel = u64::from(speed)
            .saturating_mul(u64::from(SUBCELLS_PER_CELL))
            .saturating_mul(elapsed_micros)
            / MICROS_PER_SECOND;
        // A frame long enough to carry the line a whole lap has already
        // dealt every cell on it afresh, and one carrying it further
        // would only deal them again. Stopping the travel there is what
        // keeps the loop below bounded by the line's own length.
        let lap = length.saturating_mul(SUBCELLS_PER_CELL);
        let travel = u32::try_from(travel).unwrap_or(u32::MAX).min(lap);
        let crossed = self.fraction.saturating_add(travel);
        self.fraction = crossed % SUBCELLS_PER_CELL;
        for _ in 0..(crossed / SUBCELLS_PER_CELL) {
            self.drifted = (self.drifted + 1) % length;
            // The cell the line enters by is index zero turned back by
            // however far it has drifted, which is where the number
            // that just left the far end has come round to.
            let entering = usize::try_from((length - self.drifted) % length).unwrap_or(0);
            let drawn = xorshift.byte();
            if let Some(slot) = self.draws.get_mut(entering) {
                *slot = drawn;
            }
        }
    }
}

/// What a [`DriftingText`] draws with the number each of its cells was
/// dealt.
///
/// The same ring either way, read two ways: as an index into the
/// characters, or as how much of the cell is lit. Bars are where the
/// display starts, because only they can be drawn part way into a cell
/// -- see the module docs.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum TextFill {
    /// How much of the cell is lit, on eighths.
    #[default]
    Bars,
    /// One character out of the field's own set, which is the
    /// animation this started as.
    Glyphs,
}

impl TextFill {
    /// The other of the two, which is all the key that toggles them
    /// asks for.
    #[must_use]
    pub const fn next(self) -> Self {
        match self {
            Self::Bars => Self::Glyphs,
            Self::Glyphs => Self::Bars,
        }
    }
}

/// A window filled with characters, every line of them drifting in the
/// colours of the desktop behind it.
///
/// An app holds one, hands it a [`Rect`] and the time since the last
/// frame through [`advance`](Self::advance), and draws it with
/// [`render`](Self::render). What the reader steers is
/// [`set_direction`](Self::set_direction), [`speed_up`](Self::speed_up)
/// and [`slow_down`](Self::slow_down),
/// [`cycle_drift`](Self::cycle_drift), and
/// [`spread_wider`](Self::spread_wider) and
/// [`spread_narrower`](Self::spread_narrower). Each is clamped here
/// rather than at the call site, so an app can hand a held key straight
/// through without working out where the limits are.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DriftingText {
    /// Cells across the area the field was last sized to.
    columns:   u16,
    /// Cells down that same area.
    rows:      u16,
    /// Which way every line drifts.
    direction: BandDirection,
    /// How far the field travels each second, in cells, before any
    /// line's own share of the spread is taken off it.
    speed:     u32,
    /// How far the lines' own speeds may stand from that, as a
    /// percentage of it either way.
    spread:    u32,
    /// Whether the lines travel as one or at speeds of their own.
    drift:     TextDrift,
    /// One entry per line, indexed the way
    /// [`line_of`](Self::line_of) counts them.
    lines:     Vec<TextLine>,
    /// The drawn speeds the field's lanes are read off, one every
    /// [`lines_per_lane`](Self::lines_per_lane) lines, pushed toward the
    /// ends of the range by [`toward_the_ends`].
    ///
    /// A speed drawn for each line on its own is varied by the numbers
    /// and reads as noise: neighbouring lines are the only ones the eye
    /// can compare, a field of characters carries no landmark to measure
    /// a line against anything further off, and independent draws leave
    /// the display without a single run of lines going anywhere
    /// together. Dealing alternate lines to opposite ends of the range
    /// answers that with a comb, which is legible but is a texture
    /// rather than motion.
    ///
    /// Lanes are the answer to both. Every line between two drawn points
    /// takes a speed interpolated from the pair -- see [`speed_at`] --
    /// so a slow point holds the lines around it back while a fast one
    /// carries its own along, and the display reads as bodies of text
    /// travelling together rather than as a field of separate lines.
    /// Pushing the points outward is what makes them read as a slow
    /// group and a fast group rather than as one long gradient.
    lanes:     Vec<LanePoint>,
    /// A second, finer run of drawn speeds read at
    /// [`TEXT_RIPPLE_PERCENT`] of its strength, which is what puts
    /// visible variation *inside* a lane: short runs of lines easing
    /// ahead of their group and falling back, without any of them
    /// leaving it.
    ripple:    Vec<LanePoint>,
    /// How far the lanes have travelled along the field, in
    /// [`LANE_FRACTION_UNIT`] sub-lines, wrapping at its far end.
    ///
    /// The lanes are dealt once and this moves where each line reads
    /// them, so the pattern slides along the field and a line is carried
    /// from a slow group into a fast one and back. Dealing fresh
    /// speeds instead would step the whole field at once; moving the
    /// read point is what makes the change a wave crossing the lines
    /// rather than a new hand.
    waved:     u32,
    /// Source of the numbers each line is dealt and of where it sits in
    /// the spread.
    xorshift:  Xorshift,
    /// What those numbers are drawn as.
    fill:      TextFill,
    /// How far the whole field is carried toward the ground it is drawn
    /// on, on the alpha scale [`blend_color`] reads: zero draws it at
    /// full strength, [`u8::MAX`] draws nothing.
    faded:     u8,
}

impl Default for DriftingText {
    fn default() -> Self {
        Self {
            columns:   0,
            rows:      0,
            direction: BandDirection::default(),
            speed:     DEFAULT_TEXT_SPEED,
            spread:    DEFAULT_TEXT_SPREAD,
            drift:     TextDrift::default(),
            lines:     Vec::new(),
            lanes:     Vec::new(),
            ripple:    Vec::new(),
            waved:     0,
            xorshift:  Xorshift::default(),
            fill:      TextFill::default(),
            faded:     0,
        }
    }
}

impl DriftingText {
    /// A field that has not been sized yet. The first
    /// [`advance`](Self::advance) settles its area.
    #[must_use]
    pub fn new() -> Self { Self::default() }

    /// Carry every line one frame further along, sizing the field to
    /// `area` first.
    pub fn advance(&mut self, area: Rect, elapsed: Duration) {
        self.resize(area);
        if self.columns == 0 || self.rows == 0 {
            return;
        }
        let elapsed_micros = u64::try_from(elapsed.as_micros()).unwrap_or(u64::MAX);
        let together = matches!(self.drift, TextDrift::Together);
        let speed = self.speed;
        let spread = self.spread;
        let count = u32::try_from(self.lines.len()).unwrap_or(u32::MAX);
        self.waved = waved_on(self.waved, count, elapsed_micros);
        let waved = self.waved;
        let lanes = &self.lanes;
        let ripple = &self.ripple;
        let xorshift = &mut self.xorshift;
        for (index, line) in self.lines.iter_mut().enumerate() {
            // Where this line stands among the lanes now, which is its
            // own place plus however far they have travelled since the
            // field was dealt.
            let at = u32::try_from(index)
                .unwrap_or(u32::MAX)
                .saturating_mul(LANE_FRACTION_UNIT)
                .wrapping_add(waved);
            let carried = speed_at(lanes, at, count);
            let within = nudged(carried, speed_at(ripple, at, count), TEXT_RIPPLE_PERCENT);
            let own = line_speed(
                speed,
                spread,
                together,
                nudged(within, line.give, TEXT_LANE_GIVE_PERCENT),
            );
            line.advance(elapsed_micros, own, xorshift);
        }
    }

    /// Carry the whole field this far toward the ground it is drawn on.
    /// Zero draws it at full strength and [`u8::MAX`] draws nothing.
    pub const fn fade(&mut self, faded: u8) { self.faded = faded; }

    /// Draw the field as bars or as characters, whichever it is not
    /// drawing now.
    ///
    /// Costs the field nothing: the numbers are already dealt and this
    /// only changes how they are read, so every line carries on from
    /// exactly where it stood.
    pub const fn cycle_fill(&mut self) { self.fill = self.fill.next(); }

    /// Turn the lines' speeds together or send them apart again.
    ///
    /// Together is the lines moving as one, which they cannot do from
    /// wherever their own speeds have carried them -- so turning it on
    /// puts every line back flush and the field sets off from there.
    pub fn cycle_drift(&mut self) {
        self.drift = self.drift.next();
        match self.drift {
            TextDrift::Together => {
                for line in &mut self.lines {
                    line.drifted = 0;
                    line.fraction = 0;
                }
            },
            // Sending the lines apart with the spread drawn shut is a
            // key that answers by doing nothing, so this opens it far
            // enough to be read. Only ever upward: a reader who has
            // already sent the speeds further apart than the default
            // is not asking to be brought back to it.
            TextDrift::Apart => self.spread = self.spread.max(DEFAULT_TEXT_SPREAD),
        }
    }

    /// Which way the lines drift, and so which edge fresh characters
    /// enter by.
    ///
    /// Sideways a line is a row and up or down it is a column, so a
    /// turn between the two axes is a different set of lines
    /// altogether. Every cell keeps what it was drawing across the
    /// turn, dealt back into whichever new line now runs through it.
    /// Re-dealing instead would replace the whole field in one frame,
    /// and a field of numbers replaced at once averages out -- the
    /// reader sees the picture wash pale for as long as the new numbers
    /// take to travel, which reads as a fault rather than as a turn.
    pub fn set_direction(&mut self, direction: BandDirection) {
        if self.direction == direction {
            return;
        }
        let carried: Vec<(u16, u16, u8)> = (0..self.rows)
            .flat_map(|row| (0..self.columns).map(move |column| (column, row)))
            .filter_map(|(column, row)| {
                self.draw_at(column, row, 0)
                    .map(|drawn| (column, row, drawn))
            })
            .collect();
        self.direction = direction;
        self.rebuild();
        // rebuild leaves every line un-drifted, so a cell's own number
        // belongs at the index it sits at along its new line.
        for (column, row, drawn) in carried {
            let line = usize::from(self.line_of(column, row));
            let at = usize::from(self.along(column, row));
            if let Some(slot) = self
                .lines
                .get_mut(line)
                .and_then(|line| line.draws.get_mut(at))
            {
                *slot = drawn;
            }
        }
    }

    /// Slow the field down by `cells_per_second`, never past the
    /// slowest the text is allowed to drift: a field stopped dead is
    /// one the reader cannot tell from a frozen display.
    pub fn slow_down(&mut self, cells_per_second: u32) {
        self.set_speed(self.speed.saturating_sub(cells_per_second));
    }

    /// Speed the field up by `cells_per_second`, never past the
    /// fastest the text is allowed to drift.
    pub fn speed_up(&mut self, cells_per_second: u32) {
        self.set_speed(self.speed.saturating_add(cells_per_second));
    }

    /// Draw the lines' own speeds `percent` closer to the field's.
    ///
    /// At zero every line travels at the field's speed. That is not the
    /// same as [`TextDrift::Together`], which also puts them back flush
    /// -- lines that have already come apart stay where they are and
    /// keep the arrangement they drifted into.
    pub const fn spread_narrower(&mut self, percent: u32) {
        self.spread = self.spread.saturating_sub(percent);
    }

    /// Send the lines' own speeds `percent` further from the field's,
    /// never past the width at which the slowest line would stop and
    /// the fastest would run at twice the field's speed.
    pub fn spread_wider(&mut self, percent: u32) {
        self.spread = self.spread.saturating_add(percent).min(MAX_TEXT_SPREAD);
    }

    /// Draw the field where it currently stands, moving nothing.
    ///
    /// Every cell is drawn, which is the whole of what separates this
    /// from the band: what the reader is looking at is the colours, and
    /// a cell left out is a piece of the desktop missing rather than an
    /// edge to read.
    ///
    /// The colour is the one the backdrop has for the cell on the
    /// screen, not for the character standing on it -- the characters
    /// stream through a field of colour that is holding still, which is
    /// what keeps the desktop legible while they move. A cell the
    /// backdrop has no colour for is skipped, so whatever the terminal
    /// shows through stays visible.
    ///
    /// Both the character and the cell behind it are painted, from the
    /// one colour: the character at the desktop's own, the rest of the
    /// cell carried `TEXT_BEHIND_FADE` of the way toward the
    /// background. Painting only the character left every cell's
    /// remainder at the background, so the desktop arrived through the
    /// ink alone -- an eighth of the cell at the narrowest bar -- and
    /// what the reader saw was a pinstripe over the desktop instead of
    /// the desktop.
    pub fn render(&self, area: Rect, backdrop: &Backdrop, ground: Color, buffer: &mut Buffer) {
        if self.faded == u8::MAX {
            return;
        }
        for row in 0..self.rows.min(area.height) {
            for column in 0..self.columns.min(area.width) {
                let Some(color) = backdrop.color_at(column, row) else {
                    continue;
                };
                let Some(drawn) = self.drawn_at(column, row) else {
                    continue;
                };
                if let Some(cell) = buffer.cell_mut((area.x + column, area.y + row)) {
                    let toward = match cell.bg {
                        Color::Reset => ground,
                        background => background,
                    };
                    let foreground = theme::blend_color(color, toward, self.faded);
                    cell.set_char(drawn);
                    cell.set_fg(foreground);
                    cell.set_bg(theme::blend_color(foreground, toward, TEXT_BEHIND_FADE));
                }
            }
        }
    }

    /// What the cell at `column`, `row` draws this frame, or [`None`]
    /// where the field has no line running through it.
    ///
    /// The ramp this field's cells are filled from: the one that
    /// subdivides a cell along the axis its lines travel on.
    const fn bars(&self) -> &'static [char] {
        match self.direction {
            BandDirection::Left | BandDirection::Right => BARS_ACROSS,
            BandDirection::Up | BandDirection::Down => BARS_UP,
        }
    }

    /// The darkest and the brightest the desktop is anywhere the field
    /// covers.
    ///
    /// The bars are drawn across that rather than across the whole of
    /// what a colour could be. A desktop of dark greys occupies a
    /// narrow band near the bottom of the absolute scale, and read
    /// against the absolute scale every cell of it rounds to the same
    /// sliver -- so the picture that is there goes undrawn. Stretched,
    /// the same desktop fills the ramp.
    fn drawn_at(&self, column: u16, row: u16) -> Option<char> {
        match self.fill {
            TextFill::Bars => {
                let level = self.level_at(column, row)?;
                self.bars()
                    .get(usize::from(level).saturating_sub(1))
                    .copied()
            },
            TextFill::Glyphs => self.glyph_at(column, row),
        }
    }

    /// How far along its line the cell at `column`, `row` sits, counted
    /// from the edge fresh characters enter by.
    const fn along(&self, column: u16, row: u16) -> u16 {
        match self.direction {
            BandDirection::Right => column,
            BandDirection::Left => self.columns.saturating_sub(1).saturating_sub(column),
            BandDirection::Down => row,
            BandDirection::Up => self.rows.saturating_sub(1).saturating_sub(row),
        }
    }

    /// How much of the cell at `column`, `row` is lit this frame, on
    /// the one-to-[`BAR_LEVELS`] scale a bar is drawn from, or [`None`]
    /// where the field has no line running through it.
    ///
    /// Between the number standing here and the one due to arrive next
    /// the two are mixed by how far into the cell the line has come,
    /// and that mix is what puts the travel on eighths of a cell rather
    /// than on whole ones. Reading the ring at one index only -- all a
    /// character can do -- is the same journey taken in jumps.
    fn level_at(&self, column: u16, row: u16) -> Option<u8> {
        let fraction = self
            .lines
            .get(usize::from(self.line_of(column, row)))?
            .fraction;
        let here = u32::from(self.draw_at(column, row, 0)?);
        let next = u32::from(self.draw_at(column, row, 1)?);
        let mixed = (here * (SUBCELLS_PER_CELL - fraction) + next * fraction) / SUBCELLS_PER_CELL;
        // A byte spread over the levels, from one so that no cell is
        // ever left blank: this is the animation whose subject is the
        // desktop, and an empty cell is a piece of it missing.
        let level = mixed * BAR_LEVELS / (u32::from(u8::MAX) + 1) + 1;
        u8::try_from(level).ok()
    }

    /// Which of [`GLYPHS`] the cell at `column`, `row` draws, or
    /// [`None`] where the field has no line running through it.
    fn glyph_at(&self, column: u16, row: u16) -> Option<char> {
        let drawn = self.draw_at(column, row, 0)?;
        GLYPHS.get(usize::from(drawn) % GLYPHS.len()).copied()
    }

    /// The number standing at the cell at `column`, `row`, or the one
    /// `back` places further along the ring behind it -- which is the
    /// number due to arrive here in another `back` cells of travel.
    fn draw_at(&self, column: u16, row: u16, back: u32) -> Option<u8> {
        let line = self.lines.get(usize::from(self.line_of(column, row)))?;
        let length = u32::try_from(line.draws.len()).ok()?;
        if length == 0 {
            return None;
        }
        let here = (u32::from(self.along(column, row)) + length - line.drifted % length) % length;
        let at = (here + length - back % length) % length;
        line.draws.get(usize::try_from(at).ok()?).copied()
    }

    /// How many lines the field is made of: its rows while the text
    /// drifts sideways, its columns while it drifts up or down.
    const fn line_count(&self) -> u16 {
        match self.direction {
            BandDirection::Left | BandDirection::Right => self.rows,
            BandDirection::Up | BandDirection::Down => self.columns,
        }
    }

    /// Cells on one of those lines, which is the other extent of the
    /// area.
    const fn line_length(&self) -> u16 {
        match self.direction {
            BandDirection::Left | BandDirection::Right => self.columns,
            BandDirection::Up | BandDirection::Down => self.rows,
        }
    }

    /// Which line the cell at `column`, `row` belongs to.
    const fn line_of(&self, column: u16, row: u16) -> u16 {
        match self.direction {
            BandDirection::Left | BandDirection::Right => row,
            BandDirection::Up | BandDirection::Down => column,
        }
    }

    /// How many lines one lane of speeds covers, which is the target for
    /// the axis the lines happen to lie on.
    ///
    /// Wider across columns than deep across rows, and by the ratio a
    /// character cell stands at -- see [`TEXT_LANE_COLUMNS`]. A single
    /// figure for both would give the same lane a thickness on screen
    /// that changed with the direction key.
    const fn lines_per_lane(&self) -> usize {
        match self.direction {
            BandDirection::Left | BandDirection::Right => TEXT_LANE_ROWS,
            BandDirection::Up | BandDirection::Down => TEXT_LANE_COLUMNS,
        }
    }

    /// Draw a fresh set of lines for the area and the axis they cross
    /// it on, every one of them flush with the others.
    fn rebuild(&mut self) {
        let count = usize::from(self.line_count());
        let length = usize::from(self.line_length());
        self.lanes = draw_points(count, self.lines_per_lane(), &mut self.xorshift)
            .into_iter()
            .map(LanePoint::toward_the_ends)
            .collect();
        self.ripple = draw_points(count, TEXT_RIPPLE_LINES, &mut self.xorshift);
        // The lanes are a fresh hand, so how far the old one had
        // travelled says nothing about this one.
        self.waved = 0;
        let xorshift = &mut self.xorshift;
        self.lines = (0..count)
            .map(|_| TextLine {
                draws:    (0..length).map(|_| xorshift.byte()).collect(),
                drifted:  0,
                fraction: 0,
                give:     xorshift.byte(),
            })
            .collect();
    }

    /// Re-size to `area`, drawing a fresh set of lines. Does nothing
    /// when the area has not changed.
    fn resize(&mut self, area: Rect) {
        if self.columns == area.width && self.rows == area.height {
            return;
        }
        self.columns = area.width;
        self.rows = area.height;
        self.rebuild();
    }

    /// Travel the field `speed` cells a second, clamped to what it is
    /// allowed.
    fn set_speed(&mut self, speed: u32) {
        self.speed = speed.clamp(MIN_TEXT_SPEED, MAX_TEXT_SPEED);
    }
}

/// One of the field's drawn speeds, and how much of the field it holds.
///
/// The speeds were once read off slices of one thickness apiece, which
/// draws every lane the same size -- and a field of bands all one size
/// reads as a ruled grid, because the eye finds the repeat and then
/// stops seeing anything else. Each point carries its own extent
/// instead, drawn within [`TEXT_LANE_SPREAD_PERCENT`] of the nominal
/// thickness, so no two lanes running together come out alike.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct LanePoint {
    /// The speed the lines nearest this point drift at.
    speed:  u8,
    /// How much of the field this point holds, against what its
    /// neighbours were dealt. Nominally [`LANE_FRACTION_UNIT`].
    extent: u32,
}

impl LanePoint {
    /// This point with its speed pushed toward the ends of the range,
    /// its extent untouched. See [`toward_the_ends`].
    fn toward_the_ends(self) -> Self {
        Self {
            speed: toward_the_ends(self.speed),
            ..self
        }
    }
}

/// How much of the field one lane holds, in [`LANE_FRACTION_UNIT`]s.
///
/// Nominally one unit, drawn within [`TEXT_LANE_SPREAD_PERCENT`] of it
/// either way. Held to at least one: a lane drawn away to nothing would
/// put two speeds side by side with no gradient between them, which is
/// the wall the interpolation exists to avoid.
fn draw_extent(xorshift: &mut Xorshift) -> u32 {
    let spread = LANE_FRACTION_UNIT.saturating_mul(TEXT_LANE_SPREAD_PERCENT) / WHOLE_PERCENT;
    let range = usize::try_from(spread.saturating_mul(2).saturating_add(1)).unwrap_or(1);
    let drawn = u32::try_from(xorshift.index(range)).unwrap_or(0);
    LANE_FRACTION_UNIT
        .saturating_sub(spread)
        .saturating_add(drawn)
        .max(1)
}

/// A run of drawn speeds spaced one every `every` lines down a field of
/// `count` of them, with a point at either end.
///
/// Spacing rather than a count of them, so the same call gives a lane
/// the same nominal thickness whatever size the field is -- a window
/// twice as deep gets twice as many lanes rather than lanes twice as
/// deep. Nominal because each point is then dealt its own extent
/// either side of it: see [`LanePoint`].
/// Always at least one span, so a field shorter than one lane is still
/// dealt something to interpolate across.
///
/// One point per slice of the range rather than each drawn freely, and
/// then shuffled. Free draws leave a display with no fast lane on it
/// whenever the numbers happen to come out low, which is an ordinary
/// hand rather than a rare one and reads as a field that is simply
/// slow -- and it is the very thing the lanes exist to avoid, since a
/// reader with nothing quick on screen has nothing to measure the slow
/// against. A point per slice makes a slow lane, a fast one and
/// something in between certain whatever is drawn; the shuffle is what
/// keeps them from arriving in order down the display.
fn draw_points(count: usize, every: usize, xorshift: &mut Xorshift) -> Vec<LanePoint> {
    let whole = usize::from(u8::MAX).saturating_add(1);
    let points = (count / every.max(1)).max(1).saturating_add(1);
    let slice = (whole / points).max(1);
    let mut drawn: Vec<LanePoint> = (0..points)
        .map(|index| {
            let low = index.saturating_mul(whole) / points;
            LanePoint {
                speed:  u8::try_from(low.saturating_add(xorshift.index(slice))).unwrap_or(u8::MAX),
                extent: draw_extent(xorshift),
            }
        })
        .collect();
    for index in (1..drawn.len()).rev() {
        drawn.swap(index, xorshift.index(index.saturating_add(1)));
    }
    drawn
}

/// Which lane `along` falls in, and how much extent the lanes ahead of
/// it take up, so [`speed_at`] can read how far into its own lane a
/// line stands.
///
/// Walked rather than divided: the lanes are dealt uneven extents, so
/// there is no slice size to divide by. The run is short -- one point
/// every [`TEXT_LANE_ROWS`] or [`TEXT_LANE_COLUMNS`] lines -- and this
/// is read once per line per frame.
///
/// `along` is always inside the run, since [`speed_at`] scales it by
/// the extents before calling. The first lane answers for a value that
/// somehow is not, which is where a field with nothing dealt would land.
fn span_at(points: &[LanePoint], along: u64) -> (usize, u64) {
    let mut before: u64 = 0;
    for (index, point) in points.iter().enumerate() {
        let next = before.saturating_add(u64::from(point.extent));
        if along < next {
            return (index, before);
        }
        before = next;
    }
    (0, 0)
}

/// The speed `points` carry a line standing `at` sub-lines along a field
/// of `count` of them, read off the two of them either side of it.
///
/// Read as a ring: the last point runs back into the first, so the
/// pattern has no end for [`DriftingText::waved`] to carry a line over.
/// A run with two ends would hand every line a jump each time the wave
/// came round.
///
/// The weight between two points is a smoothstep rather than the plain
/// fraction, which is what gives a lane a body: the curve barely moves
/// at either end and does the whole of its travel in the middle, so the
/// lines nearest a point sit at very nearly its speed and the ones
/// halfway between two are where the field changes hands.
fn speed_at(points: &[LanePoint], at: u32, count: u32) -> u8 {
    let Some(first) = points.first().copied() else {
        return u8::MAX / 2;
    };
    let spans = points.len();
    let total = u64::from(count).saturating_mul(u64::from(LANE_FRACTION_UNIT));
    let dealt: u64 = points.iter().map(|point| u64::from(point.extent)).sum();
    if total == 0 || dealt == 0 {
        return first.speed;
    }
    // The lanes hold uneven shares of the field, so where a line stands
    // among them is measured against the extents they were dealt rather
    // than against equal slices of the whole.
    let along = (u64::from(at) % total).saturating_mul(dealt) / total;
    let (between, before) = span_at(points, along);
    let held = points.get(between).copied().unwrap_or(first);
    let fraction = u32::try_from(
        along
            .saturating_sub(before)
            .saturating_mul(u64::from(LANE_FRACTION_UNIT))
            / u64::from(held.extent).max(1),
    )
    .unwrap_or(0)
    .min(LANE_FRACTION_UNIT);
    let weight = merged(fraction);
    let from = u32::from(held.speed);
    let to = u32::from(
        points
            .get(between.saturating_add(1) % spans)
            .copied()
            .unwrap_or(first)
            .speed,
    );
    let travelled = from.abs_diff(to).saturating_mul(weight) / LANE_FRACTION_UNIT;
    let value = if to >= from {
        from.saturating_add(travelled)
    } else {
        from.saturating_sub(travelled)
    };
    u8::try_from(value).unwrap_or(u8::MAX)
}

/// [`DriftingText::waved`] carried `elapsed_micros` further along a
/// field of `count` lines, wrapping at its far end.
///
/// Wrapped against the field rather than left to climb, so the value
/// stays where [`speed_at`] can read it whatever the field has been
/// sized to and however long the screen has been up.
fn waved_on(waved: u32, count: u32, elapsed_micros: u64) -> u32 {
    let total = count.saturating_mul(LANE_FRACTION_UNIT);
    if total == 0 {
        return 0;
    }
    let travelled = u32::try_from(
        u64::from(TEXT_WAVE_SUBLINES_PER_SECOND).saturating_mul(elapsed_micros) / MICROS_PER_SECOND,
    )
    .unwrap_or(u32::MAX);
    waved.wrapping_add(travelled) % total
}

/// `value` moved `percent` of however far `toward` stands from the
/// middle of the range, upward where that draw is above the middle and
/// downward where it is below.
///
/// A draw read as an offset rather than blended into the value, which is
/// the difference between varying a lane and diluting it: a blend pulls
/// every line toward the middle of the range and would undo the work
/// [`toward_the_ends`] does, while this leaves a slow lane slow and only
/// says where in it the line sits.
///
/// An offset that would carry the line past an end of the range is
/// turned back in instead of being clipped there. Clipping is what
/// costs the field its variation exactly where it is wanted most: the
/// lanes are pushed to the ends on purpose, so the slowest group sits
/// against the floor and every downward offset in it would flatten to
/// the same number -- a wide run of lines dealt one speed, which is what
/// the lanes are for avoiding. Turning back keeps the offset varying
/// line by line and keeps the line inside its own lane.
fn nudged(value: u8, toward: u8, percent: u32) -> u8 {
    let whole = u32::from(u8::MAX);
    let middle = whole / 2;
    let drawn = u32::from(toward);
    let moved = drawn.abs_diff(middle).saturating_mul(percent) / WHOLE_PERCENT;
    let standing = u32::from(value);
    let landed = if drawn >= middle {
        let raised = standing.saturating_add(moved);
        if raised > whole {
            whole.saturating_sub(raised.saturating_sub(whole))
        } else {
            raised
        }
    } else {
        standing.abs_diff(moved)
    };
    u8::try_from(landed).unwrap_or(u8::MAX)
}

/// A drawn speed moved toward whichever end of the range it is already
/// nearer, so two lanes drawn a little apart end up plainly apart.
///
/// The same smoothstep curve the interpolation uses, read as a curve
/// over the range rather than over the distance between two lanes: it
/// leaves the middle where it is and carries everything else outward,
/// so a field of lanes still has a medium one while its slow and fast
/// lanes are further from it than they were drawn.
///
/// Applied twice. Once is a curve the numbers can measure and the eye
/// cannot: a field of characters gives nothing to compare a line
/// against, so lanes a quarter of the range apart still read as one
/// speed. Twice sends a slow draw to roughly a fifth of where one pass
/// leaves it and a fast one nearly to the top, which is what turns the
/// display into groups rather than a gradient. The middle is a fixed
/// point of the curve, so no number of passes moves it.
fn toward_the_ends(value: u8) -> u8 {
    let whole = u32::from(u8::MAX);
    let along = u32::from(value).saturating_mul(LANE_FRACTION_UNIT) / whole;
    let eased = smoothstep(smoothstep(along));
    u8::try_from(eased.saturating_mul(whole) / LANE_FRACTION_UNIT).unwrap_or(u8::MAX)
}

/// The weight between two lanes' speeds at `fraction` of the way from
/// one to the next.
///
/// [`smoothstep`] pulled back toward the straight ramp by
/// [`TEXT_LANE_BODY_PERCENT`], which is what keeps two lanes from
/// meeting at an edge. See that constant.
fn merged(fraction: u32) -> u32 {
    let along = u64::from(fraction.min(LANE_FRACTION_UNIT));
    let eased = u64::from(smoothstep(fraction));
    let body = u64::from(TEXT_LANE_BODY_PERCENT);
    let whole = u64::from(WHOLE_PERCENT);
    let blended = (eased * body + along * whole.saturating_sub(body)) / whole;
    u32::try_from(blended).unwrap_or(LANE_FRACTION_UNIT)
}

/// Smoothstep across [`LANE_FRACTION_UNIT`]: both ends where they were,
/// and the travel between them slowest at either end and fastest in the
/// middle.
fn smoothstep(fraction: u32) -> u32 {
    let unit = u64::from(LANE_FRACTION_UNIT);
    let along = u64::from(fraction).min(unit);
    let eased = along * along * (3 * unit - 2 * along) / (unit * unit);
    u32::try_from(eased).unwrap_or(LANE_FRACTION_UNIT)
}

/// How fast one line drifts, given where its own variance stands in the
/// spread around the field's speed.
///
/// Zero variance is the slow end of the spread and [`u8::MAX`] the fast
/// end, so the field's own speed is what a line halfway along it
/// travels at and the spread is how far either side of that the ends
/// reach. Never under [`MIN_TEXT_SPEED`]: a line stopped dead is one
/// the reader reads as a rendering fault rather than as the slow end of
/// a range.
fn line_speed(speed: u32, spread: u32, together: bool, variance: u8) -> u32 {
    if together {
        return speed.max(MIN_TEXT_SPEED);
    }
    let percent = WHOLE_PERCENT
        .saturating_sub(spread)
        .saturating_add(spread.saturating_mul(2) * u32::from(variance) / u32::from(u8::MAX));
    (speed.saturating_mul(percent) / WHOLE_PERCENT).max(MIN_TEXT_SPEED)
}

#[cfg(test)]
#[expect(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::collections::BTreeSet;
    use std::ops::Range;

    use super::*;

    /// An area with different extents each way, so a test that reads
    /// rows where it should read columns is caught by the count rather
    /// than passing on a square.
    const AREA: Rect = Rect::new(0, 0, 40, 12);
    /// Long enough for the slowest line to have crossed several cells.
    const A_WHILE: Duration = Duration::from_secs(2);

    /// A field sized to `AREA` and drifting `direction`, with every
    /// line at the one speed so a test can say where each of them
    /// stands.
    fn locked(direction: BandDirection) -> DriftingText {
        let mut text = DriftingText::new();
        text.set_direction(direction);
        text.cycle_drift();
        text.advance(AREA, Duration::ZERO);
        assert_eq!(
            text.drift,
            TextDrift::Together,
            "the helper locks the lines"
        );
        text
    }

    /// How long it takes a line at `speed` to cross `cells`, rounded up
    /// to the microsecond. The travel is worked out in whole numbers,
    /// so the exact figure truncates a sub-cell short and the line
    /// stops one shy of the boundary the test is asking about.
    fn crossing(cells: u32, speed: u32) -> Duration {
        Duration::from_micros(u64::from(cells) * MICROS_PER_SECOND / u64::from(speed) + 1)
    }

    /// How much of each cell across `columns` of row zero is lit.
    ///
    /// Read into a [`Vec`] rather than handed back as an iterator
    /// because every caller compares it against the same row *after*
    /// [`DriftingText::advance`] has moved everything: a lazy read
    /// would answer with where the light ended up rather than where it
    /// set out from.
    fn row_levels(text: &DriftingText, columns: Range<u16>) -> Vec<u8> {
        columns
            .map(|column| text.level_at(column, 0).expect("the row is filled"))
            .collect()
    }

    /// A line is a row while the text drifts sideways and a column
    /// while it drifts up or down, so the count and the length of them
    /// swap when the axis does. Reading it the other way round would
    /// draw the field on its side and only show on a square window.
    #[test]
    fn a_line_is_a_row_sideways_and_a_column_up_or_down() {
        let sideways = locked(BandDirection::Right);
        let vertical = locked(BandDirection::Down);

        assert_eq!(sideways.lines.len(), usize::from(AREA.height));
        assert_eq!(sideways.line_length(), AREA.width);
        assert_eq!(vertical.lines.len(), usize::from(AREA.width));
        assert_eq!(vertical.line_length(), AREA.height);
    }

    /// Every cell of the area carries a bar, and never one of nothing.
    /// What separates this animation from the band is that it leaves
    /// nothing out, so a hole anywhere in it is the whole point missed
    /// -- and a desktop dark enough to read as nothing is exactly where
    /// a scale starting at zero would put holes.
    #[test]
    fn every_cell_of_the_area_carries_a_bar() {
        let text = locked(BandDirection::Right);

        for row in 0..AREA.height {
            for column in 0..AREA.width {
                let level = text
                    .level_at(column, row)
                    .expect("the field covers the area");
                assert!(level >= 1, "nothing lit at {column}, {row}");
                assert!(
                    u32::from(level) <= BAR_LEVELS,
                    "more than a cell lit at {column}, {row}"
                );
            }
        }
    }

    /// A field with no area yet draws nothing, rather than reading its
    /// own emptiness as a line of length zero.
    #[test]
    fn an_unsized_field_lights_nothing() {
        assert_eq!(DriftingText::new().level_at(0, 0), None);
    }

    /// The light stands one cell further along its line for every cell
    /// the line has drifted, which is the whole of what makes the field
    /// move.
    #[test]
    fn light_travels_along_its_line_as_the_line_drifts() {
        let mut text = locked(BandDirection::Right);
        let before = row_levels(&text, 0..AREA.width - 1);

        text.advance(AREA, crossing(1, DEFAULT_TEXT_SPEED));

        for (column, level) in before.into_iter().enumerate() {
            let moved = u16::try_from(column).expect("the area is narrow") + 1;
            assert_eq!(
                text.level_at(moved, 0),
                Some(level),
                "the light at {column} should have moved one cell right"
            );
        }
    }

    /// Travel worth less than a whole cell still changes what is drawn.
    ///
    /// This is what the bars are for. Drawn on whole cells only, a line
    /// at the default speed holds the same picture for a twelfth of a
    /// second and then jumps a cell, which reads as stepping rather
    /// than as travel.
    #[test]
    fn a_line_moving_less_than_a_cell_still_changes_what_is_drawn() {
        let mut text = locked(BandDirection::Right);
        let before = row_levels(&text, 0..AREA.width);

        // Half a cell at the field's own speed, so no line has crossed
        // a boundary and the whole-cell reading is untouched.
        text.advance(
            AREA,
            Duration::from_micros(MICROS_PER_SECOND / u64::from(DEFAULT_TEXT_SPEED) / 2),
        );

        assert!(
            text.lines.iter().all(|line| line.drifted == 0),
            "the test should not have crossed a cell boundary"
        );
        assert_ne!(
            row_levels(&text, 0..AREA.width),
            before,
            "half a cell of travel should show"
        );
    }

    /// Drifting left, the light travels toward the left edge instead.
    /// The ring is turned the same way whichever direction it is read
    /// in -- what changes is which end of the line is counted from.
    #[test]
    fn drifting_left_carries_the_light_the_other_way() {
        let mut text = locked(BandDirection::Left);
        let before = row_levels(&text, 1..AREA.width);

        text.advance(AREA, crossing(1, DEFAULT_TEXT_SPEED));

        for (index, level) in before.into_iter().enumerate() {
            let moved = u16::try_from(index).expect("the area is narrow");
            assert_eq!(
                text.level_at(moved, 0),
                Some(level),
                "the light at {} should have moved one cell left",
                moved + 1
            );
        }
    }

    /// A line is a ring, so a whole lap brings it back to where it set
    /// off from -- but a fresh number was dealt at the entering edge
    /// for every cell it crossed on the way, so what it draws there is
    /// a new hand rather than the one it started with.
    #[test]
    fn a_whole_lap_deals_a_line_a_fresh_hand() {
        let mut text = locked(BandDirection::Right);
        let before = row_levels(&text, 0..AREA.width);

        text.advance(AREA, crossing(u32::from(AREA.width), DEFAULT_TEXT_SPEED));

        assert_eq!(text.lines[0].drifted, 0, "a lap should come round");
        assert_ne!(row_levels(&text, 0..AREA.width), before);
    }

    /// Both fills read the same ring, so the key that swaps them moves
    /// nothing: the field the reader was looking at is the field they
    /// keep, drawn another way.
    #[test]
    fn swapping_the_fill_leaves_every_line_where_it_stood() {
        let mut text = locked(BandDirection::Right);
        text.advance(AREA, crossing(3, DEFAULT_TEXT_SPEED));
        let before: Vec<Vec<u8>> = text.lines.iter().map(|line| line.draws.clone()).collect();
        let drifted: Vec<u32> = text.lines.iter().map(|line| line.drifted).collect();

        text.cycle_fill();

        assert_eq!(text.fill, TextFill::Glyphs);
        assert!(text.glyph_at(0, 0).is_some(), "the glyphs should draw");
        assert_eq!(
            text.lines
                .iter()
                .map(|line| line.draws.clone())
                .collect::<Vec<_>>(),
            before
        );
        assert_eq!(
            text.lines
                .iter()
                .map(|line| line.drifted)
                .collect::<Vec<_>>(),
            drifted
        );
    }

    /// A turn keeps every cell drawing what it was drawing, and only
    /// changes which way that travels from here. Dealing the field
    /// afresh instead replaces every cell at once, and a field of
    /// numbers replaced at once averages out -- the picture washes pale
    /// for as long as the new numbers take to travel.
    #[test]
    fn turning_carries_every_cell_into_the_new_direction() {
        let mut text = locked(BandDirection::Right);
        text.advance(AREA, crossing(2, DEFAULT_TEXT_SPEED));
        let before: Vec<Vec<Option<u8>>> = (0..AREA.height)
            .map(|row| {
                (0..AREA.width)
                    .map(|column| text.draw_at(column, row, 0))
                    .collect()
            })
            .collect();

        text.set_direction(BandDirection::Down);

        // Read back by reference: the numbers have to be off the field
        // before the turn, and a lazy read would answer with what the
        // cells hold after it.
        for (row, drawn) in before.iter().enumerate() {
            for (column, number) in drawn.iter().enumerate() {
                let column = u16::try_from(column).expect("the area is narrow");
                let row = u16::try_from(row).expect("the area is short");
                assert_eq!(
                    text.draw_at(column, row, 0),
                    *number,
                    "{column}, {row} should have kept its number through the turn"
                );
            }
        }
    }

    /// Every covered cell wears the colour the backdrop has for it. The
    /// characters are what moves; the colours are what is being looked
    /// at, and they hold still.
    #[test]
    fn every_cell_is_drawn_in_the_colour_behind_it() {
        let color = Color::Rgb(200, 100, 50);
        let text = locked(BandDirection::Right);
        let backdrop = Backdrop::flat(AREA, color);
        let mut buffer = Buffer::empty(AREA);

        text.render(AREA, &backdrop, Color::Black, &mut buffer);

        for row in 0..AREA.height {
            for column in 0..AREA.width {
                let cell = buffer
                    .cell((AREA.x + column, AREA.y + row))
                    .expect("the area covers the field");
                assert_eq!(cell.fg, color, "{column}, {row} should wear the desktop");
            }
        }
    }

    /// The cell behind the character wears the desktop too, dimmed.
    /// Painting only the character left the rest of every cell at the
    /// background, and since no cell is ever blank and the bars all
    /// fill from one edge, that background showed as a rule down every
    /// cell boundary rather than as the display.
    #[test]
    fn every_cell_is_backed_by_the_colour_behind_it() {
        let color = Color::Rgb(200, 100, 50);
        let ground = Color::Black;
        let text = locked(BandDirection::Right);
        let backdrop = Backdrop::flat(AREA, color);
        let mut buffer = Buffer::empty(AREA);

        text.render(AREA, &backdrop, ground, &mut buffer);

        let behind = theme::blend_color(color, ground, TEXT_BEHIND_FADE);
        for row in 0..AREA.height {
            for column in 0..AREA.width {
                let cell = buffer
                    .cell((AREA.x + column, AREA.y + row))
                    .expect("the area covers the field");
                assert_eq!(
                    cell.bg, behind,
                    "{column}, {row} should be backed by the desktop, dimmed"
                );
            }
        }
        assert_ne!(
            behind, color,
            "the cell behind the character has to part from the character, or the field is a flat capture"
        );
    }

    /// A field carried the whole way toward the ground draws nothing at
    /// all, which is what closes the frame it finishes leaving on.
    #[test]
    fn a_fully_faded_field_draws_nothing() {
        let mut text = locked(BandDirection::Right);
        let backdrop = Backdrop::flat(AREA, Color::Rgb(200, 100, 50));
        let mut buffer = Buffer::empty(AREA);
        text.fade(u8::MAX);

        text.render(AREA, &backdrop, Color::Black, &mut buffer);

        assert_eq!(buffer, Buffer::empty(AREA));
    }

    /// Lines left to their own speeds come apart from each other, and
    /// lines moving as one do not. Without the first the field is a
    /// sheet sliding past; without the second there is nothing to turn
    /// it back into one.
    #[test]
    fn lines_come_apart_only_while_they_are_drifting_apart() {
        let mut together = locked(BandDirection::Right);
        let mut apart = locked(BandDirection::Right);
        apart.cycle_drift();

        together.advance(AREA, A_WHILE);
        apart.advance(AREA, A_WHILE);

        let stands = |text: &DriftingText| {
            text.lines
                .iter()
                .map(|line| line.drifted)
                .collect::<Vec<_>>()
        };
        let locked_stands = stands(&together);
        assert!(
            locked_stands.windows(2).all(|pair| pair[0] == pair[1]),
            "lines moving as one should stand together: {locked_stands:?}"
        );
        let loose_stands = stands(&apart);
        assert!(
            loose_stands.windows(2).any(|pair| pair[0] != pair[1]),
            "lines at their own speeds should come apart: {loose_stands:?}"
        );
    }

    /// Turning the lines back together puts them flush as well as
    /// putting them on one speed. Left where their own speeds had
    /// carried them they would move as one and still be scattered,
    /// which is not what the key says it does.
    #[test]
    fn turning_the_lines_together_puts_them_back_flush() {
        let mut text = locked(BandDirection::Right);
        text.cycle_drift();
        text.advance(AREA, A_WHILE);
        assert!(
            text.lines.iter().any(|line| line.drifted != 0),
            "the lines should have come apart first"
        );

        text.cycle_drift();

        assert!(
            text.lines
                .iter()
                .all(|line| line.drifted == 0 && line.fraction == 0),
            "every line should be back where it started"
        );
    }

    /// Sending the lines apart opens the spread back to the default,
    /// so the key always has something to show. A reader who narrowed
    /// it to nothing and then asked for varied speeds would otherwise
    /// be told the field is varied while watching it move as one.
    #[test]
    fn sending_the_lines_apart_opens_a_spread_worth_seeing() {
        let mut text = locked(BandDirection::Right);
        text.spread_narrower(MAX_TEXT_SPREAD);
        assert_eq!(text.spread, 0, "the spread should be shut first");

        text.cycle_drift();

        assert_eq!(text.drift, TextDrift::Apart);
        assert_eq!(text.spread, DEFAULT_TEXT_SPREAD);
    }

    /// Lines the field has dealt, for the lane tests. Seeded rather
    /// than clock-drawn: what is being asserted is how one draw is laid
    /// out across the display, which is not a thing to read off
    /// whichever numbers the clock happened to give.
    const LANE_TEST_LINES: usize = 48;

    /// Seeds the lane tests read, so a property is asserted of the
    /// dealing rather than of one lucky draw.
    const LANE_TEST_SEEDS: [u64; 8] = [1, 7, 19, 41, 97, 233, 1021, 65537];

    /// A line travels at close to its neighbours' speed and nothing
    /// like the speed of a line across the display from it. That is the
    /// whole of what makes a lane: runs of text going somewhere
    /// together, with a slower and a faster run either side.
    ///
    /// Measured against lines half the display apart rather than
    /// against a fixed number, because the two are the same
    /// distribution once the lanes are taken away -- so the ratio
    /// between them is the correlation the lanes exist to create, and
    /// a deal that lost it fails here whatever range it happens to
    /// span.
    #[test]
    fn a_line_travels_at_close_to_its_neighbours_speed() {
        let mut neighbours = 0_u32;
        let mut across = 0_u32;
        for seed in LANE_TEST_SEEDS {
            let dealt = dealt_at_rest(LANE_TEST_LINES, TEXT_LANE_ROWS, seed);
            neighbours += dealt
                .windows(2)
                .map(|pair| u32::from(pair[0].abs_diff(pair[1])))
                .sum::<u32>();
            let half = LANE_TEST_LINES / 2;
            across += (0..half)
                .map(|line| {
                    u32::from(
                        dealt
                            .get(line)
                            .expect("the line was dealt")
                            .abs_diff(*dealt.get(line + half).expect("the line was dealt")),
                    )
                })
                .sum::<u32>();
        }
        let neighbours = neighbours / u32::try_from(LANE_TEST_SEEDS.len()).expect("a small count");
        let across = across / u32::try_from(LANE_TEST_SEEDS.len()).expect("a small count");

        assert!(
            neighbours * 3 < across,
            "neighbouring lines total {neighbours} apart against {across} across the display",
        );
    }

    /// Where every line of a field of `count` sits in the spread before
    /// the lanes have travelled at all, which is what
    /// [`DriftingText::advance`] works out per frame at
    /// [`DriftingText::waved`] of zero.
    ///
    /// The field itself deals this against a live area and a clock. The
    /// assertions below are about the hand the lanes deal, so they read
    /// it here from a seeded source instead.
    fn dealt_at_rest(count: usize, per_lane: usize, seed: u64) -> Vec<u8> {
        let mut xorshift = Xorshift::seeded(seed);
        let lanes: Vec<LanePoint> = draw_points(count, per_lane, &mut xorshift)
            .into_iter()
            .map(LanePoint::toward_the_ends)
            .collect();
        let ripple = draw_points(count, TEXT_RIPPLE_LINES, &mut xorshift);
        let total = u32::try_from(count).expect("a small count");
        (0..count)
            .map(|line| {
                let at = u32::try_from(line)
                    .expect("a small count")
                    .saturating_mul(LANE_FRACTION_UNIT);
                let carried = speed_at(&lanes, at, total);
                let within = nudged(carried, speed_at(&ripple, at, total), TEXT_RIPPLE_PERCENT);
                nudged(within, xorshift.byte(), TEXT_LANE_GIVE_PERCENT)
            })
            .collect()
    }

    /// The lanes are not all one thickness. Slices of one size read as
    /// a ruled grid -- the eye finds the repeat and then sees nothing
    /// else -- so each point is dealt its own extent, and every extent
    /// stays within [`TEXT_LANE_SPREAD_PERCENT`] of the nominal so a
    /// lane still holds enough lines to read as one body of text.
    #[test]
    fn the_lanes_are_dealt_uneven_thicknesses() {
        let mut xorshift = Xorshift::seeded(LANE_TEST_SEEDS[0]);
        let points = draw_points(LANE_TEST_LINES, TEXT_LANE_ROWS, &mut xorshift);
        let spread = LANE_FRACTION_UNIT * TEXT_LANE_SPREAD_PERCENT / WHOLE_PERCENT;
        let extents: Vec<u32> = points.iter().map(|point| point.extent).collect();
        let thinnest = extents.iter().copied().min().expect("a lane was dealt");
        let thickest = extents.iter().copied().max().expect("a lane was dealt");
        assert!(
            thinnest < thickest,
            "every lane came out {thinnest} sub-lines thick"
        );
        assert!(
            thinnest >= LANE_FRACTION_UNIT - spread,
            "a lane of {thinnest} is thinner than the spread allows"
        );
        assert!(
            thickest <= LANE_FRACTION_UNIT + spread,
            "a lane of {thickest} is thicker than the spread allows"
        );
    }

    /// One lane merges into the next rather than meeting it at an edge.
    ///
    /// Measured as how much of the range a curve crosses over the
    /// middle fifth of a span. [`smoothstep`] holds both ends flat and
    /// does the handover in the middle, so it crosses far more than its
    /// share there -- and a narrow run of lines carrying most of the
    /// speed change is the boundary the eye reads. [`merged`] pulls that
    /// back toward a straight ramp, which crosses exactly its share.
    #[test]
    fn the_lanes_merge_rather_than_meeting_at_an_edge() {
        let tenth = LANE_FRACTION_UNIT / 10;
        let low = LANE_FRACTION_UNIT / 2 - tenth;
        let high = LANE_FRACTION_UNIT / 2 + tenth;
        let crossed = |curve: fn(u32) -> u32| curve(high).abs_diff(curve(low));
        let ramp = crossed(merged);
        let curve = crossed(smoothstep);
        let straight = high - low;
        assert!(
            ramp < curve,
            "the lanes cross {ramp} in the middle where the bare curve crosses {curve}"
        );
        assert!(
            ramp >= straight,
            "the lanes cross {ramp}, under the {straight} a straight ramp would"
        );
    }

    /// Merging the curve back toward a ramp leaves both ends where they
    /// were, so a line standing on a lane's own point still drifts at
    /// that lane's speed.
    #[test]
    fn merging_leaves_the_ends_of_a_span_alone() {
        assert_eq!(merged(0), 0);
        assert_eq!(merged(LANE_FRACTION_UNIT), LANE_FRACTION_UNIT);
    }

    /// The lanes travel, so a line does not keep the speed it was dealt.
    /// Standing still they would hand one line the same number for as
    /// long as anybody watched, which is the thing the wave undoes:
    /// reading one line at every phase of a full lap is the same as
    /// watching it while the pattern goes by.
    #[test]
    fn the_lanes_carry_a_line_through_the_range_as_they_travel() {
        let mut xorshift = Xorshift::seeded(LANE_TEST_SEEDS[0]);
        let lanes: Vec<LanePoint> = draw_points(LANE_TEST_LINES, TEXT_LANE_ROWS, &mut xorshift)
            .into_iter()
            .map(LanePoint::toward_the_ends)
            .collect();
        let count = u32::try_from(LANE_TEST_LINES).expect("a small count");
        let readings: Vec<u8> = (0..LANE_TEST_LINES)
            .map(|step| {
                let waved = u32::try_from(step)
                    .expect("a small count")
                    .saturating_mul(LANE_FRACTION_UNIT);
                speed_at(&lanes, waved, count)
            })
            .collect();
        let slowest = readings.iter().copied().min().expect("a reading per step");
        let fastest = readings.iter().copied().max().expect("a reading per step");

        assert!(
            fastest.abs_diff(slowest) > u8::MAX / 2,
            "one line read {slowest} to {fastest} over a whole lap of the lanes",
        );
    }

    /// The wave wraps at the end of the field rather than climbing, so
    /// a screen left up for a long time reads its lanes at a position
    /// [`speed_at`] can still use.
    #[test]
    fn the_wave_wraps_at_the_end_of_the_field() {
        let count = u32::try_from(LANE_TEST_LINES).expect("a small count");
        let lap = count.saturating_mul(LANE_FRACTION_UNIT);
        let second = MICROS_PER_SECOND;

        assert_eq!(waved_on(0, count, second), TEXT_WAVE_SUBLINES_PER_SECOND);
        assert!(waved_on(lap.saturating_sub(1), count, second) < lap);
        assert_eq!(waved_on(0, 0, second), 0);
    }

    /// No lane is a block of lines dealt one speed between them. Two
    /// lines at exactly one speed never come apart, so a lane with no
    /// give in it would slide as a rigid sheet -- the one thing a field
    /// of drifting text must not look like.
    #[test]
    fn the_lines_of_a_lane_are_not_dealt_one_speed() {
        for seed in LANE_TEST_SEEDS {
            let dealt = dealt_at_rest(LANE_TEST_LINES, TEXT_LANE_ROWS, seed);
            let distinct: BTreeSet<u8> = dealt.iter().copied().collect();

            assert!(
                distinct.len() > LANE_TEST_LINES / 2,
                "seed {seed} dealt {} speeds across {LANE_TEST_LINES} lines",
                distinct.len(),
            );
        }
    }

    /// A lane reads its thickness off the axis the lines lie on. That
    /// the two figures differ, and which way round, is held by the
    /// assertion beside them; what this catches is the arms of the match
    /// being written the wrong way round, which would give the vertical
    /// lanes the rows' figure and come out as narrow stripes.
    #[test]
    fn a_lane_is_wider_across_columns_than_it_is_deep_across_rows() {
        let sideways = locked(BandDirection::Right);
        let vertical = locked(BandDirection::Down);

        assert_eq!(sideways.lines_per_lane(), TEXT_LANE_ROWS);
        assert_eq!(vertical.lines_per_lane(), TEXT_LANE_COLUMNS);
    }

    /// Every deal holds a plainly slow lane and a plainly fast one. The
    /// speeds are drawn one from each slice of the range for this
    /// reason: a display where the numbers all came out low reads as a
    /// field that is simply slow, and leaves the reader nothing to
    /// measure the slow lines against.
    #[test]
    fn every_deal_holds_a_slow_lane_and_a_fast_one() {
        let third = u8::MAX / 3;
        for seed in LANE_TEST_SEEDS {
            let dealt = dealt_at_rest(LANE_TEST_LINES, TEXT_LANE_ROWS, seed);
            let slowest = dealt.iter().copied().min().expect("the field was dealt");
            let fastest = dealt.iter().copied().max().expect("the field was dealt");

            assert!(slowest < third, "seed {seed} dealt nothing slow: {slowest}");
            assert!(
                fastest > third * 2,
                "seed {seed} dealt nothing fast: {fastest}"
            );
        }
    }

    /// A lane sitting against an end of the range still varies line by
    /// line. The lanes are pushed to the ends on purpose, so clipping
    /// the offsets there would flatten the slowest group into a wide run
    /// of lines at one speed -- which is the rigid block the lanes exist
    /// to avoid, arriving by the back door.
    #[test]
    fn a_lane_against_the_end_of_the_range_still_varies() {
        let ends = [0, u8::MAX];
        let offsets = [0, 40, 90, 127, 160, 210, u8::MAX];

        for end in ends {
            let landed: BTreeSet<u8> = offsets
                .into_iter()
                .map(|offset| nudged(end, offset, TEXT_RIPPLE_PERCENT))
                .collect();

            assert!(
                landed.len() > 1,
                "a lane at {end} was dealt one speed across the ripple: {landed:?}",
            );
        }
    }

    /// A lane drawn a little off the middle ends up plainly off it, so
    /// the display holds a slow group and a fast one rather than one
    /// long gradient. The middle itself does not move, which is what
    /// leaves room for a lane between the two.
    #[test]
    fn the_lanes_are_dealt_further_apart_than_they_were_drawn() {
        let middle = u8::MAX / 2;

        // Halfway to the end and back again, which one pass of the curve
        // does not manage: it leaves a draw of 64 at 40, and 40 against
        // 64 is a difference the numbers can see and the eye cannot.
        assert!(toward_the_ends(64) < 32, "a slow lane is sent slower");
        assert!(toward_the_ends(192) > 224, "a fast lane is sent faster");
        assert!(
            toward_the_ends(middle).abs_diff(middle) <= 2,
            "the middle stays where it was drawn",
        );
        assert_eq!(toward_the_ends(0), 0, "the ends have nowhere further to go");
        assert_eq!(toward_the_ends(u8::MAX), u8::MAX);
    }

    /// A spread already opened past the default survives being sent
    /// apart again: the floor only ever raises it. Anything else would
    /// undo the reader's own steering every time they toggled the key.
    #[test]
    fn a_wider_spread_is_not_drawn_back_to_the_default() {
        let mut text = locked(BandDirection::Right);
        text.spread_wider(MAX_TEXT_SPREAD);

        text.cycle_drift();

        assert_eq!(text.spread, MAX_TEXT_SPREAD);
    }

    /// A wider spread sends the ends of the range further from the
    /// field's speed without moving where any line sits in it, so the
    /// same line stays the same distance along a range that is being
    /// stretched. Re-drawing them instead would deal a fresh hand on
    /// every press.
    #[test]
    fn the_spread_stretches_the_range_rather_than_re_drawing_it() {
        let slowest = 0;
        let fastest = u8::MAX;
        let middle = u8::MAX / 2;

        let narrow = |variance| line_speed(100, 10, false, variance);
        let wide = |variance| line_speed(100, 50, false, variance);

        assert!(narrow(slowest) > wide(slowest), "the slow end goes slower");
        assert!(wide(fastest) > narrow(fastest), "the fast end goes faster");
        assert_eq!(narrow(middle), wide(middle), "the middle does not move");
    }

    /// A spread of nothing leaves every line at the field's own speed,
    /// which is what makes the key a continuous one rather than a
    /// second switch beside the one that already exists.
    #[test]
    fn a_spread_of_nothing_puts_every_line_on_the_fields_speed() {
        for variance in [0, u8::MAX / 3, u8::MAX] {
            assert_eq!(line_speed(30, 0, false, variance), 30);
        }
    }

    /// However wide the spread is opened, no line is ever stopped. A
    /// line standing still reads as a rendering fault rather than as
    /// the slow end of a range.
    #[test]
    fn the_slowest_line_still_drifts() {
        assert_eq!(line_speed(30, MAX_TEXT_SPREAD, false, 0), MIN_TEXT_SPEED);
    }

    /// Speed and spread stop at their limits rather than running past
    /// them, so an app can hand a held key straight through.
    #[test]
    fn speed_and_spread_stop_at_the_limits() {
        let mut text = DriftingText::new();

        text.speed_up(u32::MAX);
        assert_eq!(text.speed, MAX_TEXT_SPEED);
        text.slow_down(u32::MAX);
        assert_eq!(text.speed, MIN_TEXT_SPEED);
        text.spread_wider(u32::MAX);
        assert_eq!(text.spread, MAX_TEXT_SPREAD);
        text.spread_narrower(u32::MAX);
        assert_eq!(text.spread, 0);
    }

    /// A frame long enough to carry a line more than a whole lap leaves
    /// it somewhere on its own line rather than running the phase away.
    /// The loop that draws the entering characters is bounded by the
    /// same cap, so this is also what keeps a long gap cheap.
    #[test]
    fn a_frame_longer_than_a_lap_leaves_the_line_on_its_own_ring() {
        let mut text = locked(BandDirection::Right);
        text.speed_up(MAX_TEXT_SPEED);

        text.advance(AREA, Duration::from_secs(600));

        for line in &text.lines {
            assert!(
                line.drifted < u32::from(AREA.width),
                "a line stands somewhere on its own length: {}",
                line.drifted
            );
        }
    }
}
