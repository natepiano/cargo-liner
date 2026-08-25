//! The other attract-mode animation: the whole window filled with
//! characters, drifting line by line in the colours of the desktop
//! behind it.
//!
//! Where a [`TravelingBand`](super::TravelingBand) is one strip with
//! two edges and empty grid either side of it, this leaves no cell
//! undrawn. What there is to look at is not where the characters stop
//! but what they are wearing: every cell takes the colour the
//! [`Backdrop`] has for it, so the desktop reads through a window of
//! moving text rather than through a strip crossing it.
//!
//! A line is a row while the text drifts sideways and a column while it
//! drifts up or down, and every line is a ring: a character leaving one
//! edge is replaced by a fresh one entering at the other, so the field
//! never runs out and never repeats.
//!
//! What keeps it from reading as one rigid sheet sliding past is that
//! the lines need not travel together. [`TextDrift`] says whether they
//! do, and while they do not, each line's own speed stands somewhere in
//! a spread around the field's -- so lines that started flush come
//! apart, and how fast they do is steerable.
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
use super::constants::DEFAULT_TEXT_SPEED;
use super::constants::DEFAULT_TEXT_SPREAD;
use super::constants::MAX_TEXT_SPEED;
use super::constants::MAX_TEXT_SPREAD;
use super::constants::MICROS_PER_SECOND;
use super::constants::MIN_TEXT_SPEED;
use super::constants::SUBCELLS_PER_CELL;
use super::constants::WHOLE_PERCENT;
use super::random::Xorshift;
use super::random::random_glyph;
use crate::theme::blend_color;

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
    /// The characters on this line, as a ring. Index zero is the cell
    /// the line entered by before it had drifted at all, and
    /// [`DriftingText::glyph_at`] turns the ring by how far it has come
    /// since.
    glyphs:   Vec<char>,
    /// How many whole cells the line has drifted, modulo its own
    /// length.
    drifted:  u32,
    /// How far into the next cell it has come, in sub-cells.
    fraction: u32,
    /// Where this line's own speed stands in the spread: zero is the
    /// slow end of it and [`u8::MAX`] the fast one.
    ///
    /// Drawn once, when the line is, and held from then on. Opening the
    /// spread stretches the range every line's speed is read off rather
    /// than re-drawing where each of them sits in it, so widening and
    /// narrowing it walks the same lines apart and back together
    /// instead of dealing the field a fresh hand each press.
    variance: u8,
}

impl TextLine {
    /// Carry the line `elapsed_micros` further at `speed` cells a
    /// second, drawing a fresh character for each whole cell it has
    /// entered by.
    fn advance(&mut self, elapsed_micros: u64, speed: u32, xorshift: &mut Xorshift) {
        let Ok(length) = u32::try_from(self.glyphs.len()) else {
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
        // replaced every character on it, and one carrying it further
        // would only replace them again. Stopping the travel there is
        // what keeps the loop below bounded by the line's own length.
        let lap = length.saturating_mul(SUBCELLS_PER_CELL);
        let travel = u32::try_from(travel).unwrap_or(u32::MAX).min(lap);
        let crossed = self.fraction.saturating_add(travel);
        self.fraction = crossed % SUBCELLS_PER_CELL;
        for _ in 0..(crossed / SUBCELLS_PER_CELL) {
            self.drifted = (self.drifted + 1) % length;
            // The cell the line enters by is index zero turned back by
            // however far it has drifted, which is where the character
            // that just left the far end has come round to.
            let entering = usize::try_from((length - self.drifted) % length).unwrap_or(0);
            let glyph = random_glyph(xorshift);
            if let Some(slot) = self.glyphs.get_mut(entering) {
                *slot = glyph;
            }
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
    /// Source of the characters and of where each line sits in the
    /// spread.
    xorshift:  Xorshift,
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
            xorshift:  Xorshift::default(),
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
        let xorshift = &mut self.xorshift;
        for line in &mut self.lines {
            let own = line_speed(speed, spread, together, line.variance);
            line.advance(elapsed_micros, own, xorshift);
        }
    }

    /// Carry the whole field this far toward the ground it is drawn on.
    /// Zero draws it at full strength and [`u8::MAX`] draws nothing.
    pub const fn fade(&mut self, faded: u8) { self.faded = faded; }

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
    /// altogether. The field is re-drawn on any change rather than only
    /// on a turn: reversing along the axis it already crosses leaves
    /// every cell reading a different place in its own ring, which is a
    /// fresh field of characters whether or not it is drawn as one.
    pub fn set_direction(&mut self, direction: BandDirection) {
        if self.direction == direction {
            return;
        }
        self.direction = direction;
        self.rebuild();
    }

    /// Slow the field down by `cells_per_second`, never past
    /// [`MIN_TEXT_SPEED`].
    pub fn slow_down(&mut self, cells_per_second: u32) {
        self.set_speed(self.speed.saturating_sub(cells_per_second));
    }

    /// Speed the field up by `cells_per_second`, never past
    /// [`MAX_TEXT_SPEED`].
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
    /// never past [`MAX_TEXT_SPREAD`].
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
    pub fn render(&self, area: Rect, backdrop: &Backdrop, ground: Color, buffer: &mut Buffer) {
        if self.faded == u8::MAX {
            return;
        }
        for row in 0..self.rows.min(area.height) {
            for column in 0..self.columns.min(area.width) {
                let Some(color) = backdrop.color_at(column, row) else {
                    continue;
                };
                let Some(glyph) = self.glyph_at(column, row) else {
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

    /// The character standing on the cell at `column`, `row` this
    /// frame, or [`None`] where the field has no line for it.
    fn glyph_at(&self, column: u16, row: u16) -> Option<char> {
        let line = self.lines.get(usize::from(self.line_of(column, row)))?;
        let length = u32::try_from(line.glyphs.len()).ok()?;
        if length == 0 {
            return None;
        }
        let index = (u32::from(self.along(column, row)) + length - line.drifted % length) % length;
        line.glyphs.get(usize::try_from(index).ok()?).copied()
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

    /// Draw a fresh set of lines for the area and the axis they cross
    /// it on, every one of them flush with the others.
    fn rebuild(&mut self) {
        let count = usize::from(self.line_count());
        let length = usize::from(self.line_length());
        let mut lines = Vec::with_capacity(count);
        for index in 0..count {
            let mut glyphs = Vec::with_capacity(length);
            for _ in 0..length {
                glyphs.push(random_glyph(&mut self.xorshift));
            }
            lines.push(TextLine {
                glyphs,
                drifted: 0,
                fraction: 0,
                variance: deal_variance(index, &mut self.xorshift),
            });
        }
        self.lines = lines;
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

/// Where one line sits in the spread, dealt so that it lands at the
/// opposite end of it from the line before.
///
/// Neighbouring lines are the only ones the eye can compare. A field
/// of characters carries no landmark to measure a line against
/// anything further off, so what the reader sees of the spread is
/// whichever pairs happen to be next to each other -- and two lines
/// drawn independently land on much the same speed as often as not.
/// Dealt that way the field is varied by the numbers and reads as
/// uniform in patches, however wide the spread is opened.
///
/// Alternate lines are therefore dealt from the slow and the fast
/// third of the range, which leaves every adjacent pair two thirds of
/// it apart at worst -- one of them always slower than the field and
/// the other always faster. The middle third is what is given up, and
/// each line still draws its own place inside its third, so the field
/// is not a comb of two speeds.
fn deal_variance(index: usize, xorshift: &mut Xorshift) -> u8 {
    let third = xorshift.byte() / 3;
    if index.is_multiple_of(2) {
        third
    } else {
        u8::MAX - third
    }
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

    /// The characters standing across `columns` of row zero.
    ///
    /// Read into a [`Vec`] rather than handed back as an iterator
    /// because every caller compares it against the same row *after*
    /// [`DriftingText::advance`] has moved everything: a lazy read
    /// would answer with where the characters ended up rather than
    /// where they set out from.
    fn row_glyphs(text: &DriftingText, columns: std::ops::Range<u16>) -> Vec<char> {
        columns
            .map(|column| text.glyph_at(column, 0).expect("the row is filled"))
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
        assert_eq!(sideways.lines[0].glyphs.len(), usize::from(AREA.width));
        assert_eq!(vertical.lines.len(), usize::from(AREA.width));
        assert_eq!(vertical.lines[0].glyphs.len(), usize::from(AREA.height));
    }

    /// Every cell of the area carries a character. What separates this
    /// animation from the band is that it leaves nothing out, so a hole
    /// anywhere in it is the whole point missed.
    #[test]
    fn every_cell_of_the_area_carries_a_character() {
        let text = locked(BandDirection::Right);

        for row in 0..AREA.height {
            for column in 0..AREA.width {
                assert!(
                    text.glyph_at(column, row).is_some(),
                    "no character at {column}, {row}"
                );
            }
        }
    }

    /// A field with no area yet draws nothing, rather than reading its
    /// own emptiness as a line of length zero.
    #[test]
    fn an_unsized_field_has_no_characters() {
        assert_eq!(DriftingText::new().glyph_at(0, 0), None);
    }

    /// A character stands one cell further along its line for every
    /// cell the line has drifted, which is the whole of what makes the
    /// text move.
    #[test]
    fn a_character_travels_along_its_line_as_the_line_drifts() {
        let mut text = locked(BandDirection::Right);
        let before = row_glyphs(&text, 0..AREA.width - 1);

        text.advance(AREA, crossing(1, DEFAULT_TEXT_SPEED));

        for (column, glyph) in before.into_iter().enumerate() {
            let moved = u16::try_from(column).expect("the area is narrow") + 1;
            assert_eq!(
                text.glyph_at(moved, 0),
                Some(glyph),
                "the character at {column} should have moved one cell right"
            );
        }
    }

    /// Drifting left, a character travels toward the left edge instead.
    /// The ring is turned the same way whichever direction it is read
    /// in -- what changes is which end of the line is counted from.
    #[test]
    fn drifting_left_carries_the_characters_the_other_way() {
        let mut text = locked(BandDirection::Left);
        let before = row_glyphs(&text, 1..AREA.width);

        text.advance(AREA, crossing(1, DEFAULT_TEXT_SPEED));

        for (index, glyph) in before.into_iter().enumerate() {
            let moved = u16::try_from(index).expect("the area is narrow");
            assert_eq!(
                text.glyph_at(moved, 0),
                Some(glyph),
                "the character at {} should have moved one cell left",
                moved + 1
            );
        }
    }

    /// The line never runs out: the cell at the edge it drifts from
    /// holds a character that was not on the line before, rather than
    /// the one that just left the far end coming round again.
    #[test]
    fn a_fresh_character_enters_at_the_edge_the_line_drifts_from() {
        let mut text = locked(BandDirection::Right);
        let lap = crossing(u32::from(AREA.width), DEFAULT_TEXT_SPEED);
        let before: Vec<char> = text.lines[0].glyphs.clone();

        text.advance(AREA, lap);

        assert_ne!(
            text.lines[0].glyphs, before,
            "a whole lap should have replaced the line rather than turning it"
        );
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

    /// Every neighbouring pair of lines straddles the field's own
    /// speed, one slower and one faster. This is the whole of what the
    /// reader sees of the spread: a field whose numbers are varied but
    /// whose adjacent lines happen to match reads as one that is not
    /// varied at all.
    #[test]
    fn neighbouring_lines_drift_to_either_side_of_the_field_speed() {
        let mut text = DriftingText::new();
        text.advance(AREA, Duration::ZERO);
        assert_eq!(text.drift, TextDrift::Apart, "the field starts apart");

        let speeds: Vec<u32> = text
            .lines
            .iter()
            .map(|line| line_speed(text.speed, text.spread, false, line.variance))
            .collect();

        assert!(speeds.len() > 1, "the area has lines to compare");
        for (index, pair) in speeds.windows(2).enumerate() {
            let slower = pair[0].min(pair[1]);
            let faster = pair[0].max(pair[1]);
            assert!(
                slower < text.speed && faster > text.speed,
                "lines {index} and {} both sit at {pair:?} against a field at {}",
                index + 1,
                text.speed
            );
        }
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
