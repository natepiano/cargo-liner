//! The attract-mode animation: a lit strip of characters crossing the
//! grid, each cell wearing the colour of the desktop behind it.
//!
//! The strip travels left to right. Its leading edge draws at full
//! strength and re-rolls each column's characters as it arrives; the
//! tail behind it fades back toward the ground the app is drawn on, so
//! what crosses the screen is a band rather than a filling region. The
//! colours come from a [`Backdrop`], which means the characters look
//! cut out of whatever the terminal is sitting on top of.
//!
//! Position and fade are tracked in whole numbers throughout. A strip
//! that moves a fraction of a column per frame wants sub-column
//! precision, and carrying that as a float would put a truncating cast
//! in the middle of every cell's colour.

use std::time::Duration;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;

use super::Backdrop;
use super::constants::BAND_COLUMNS;
use super::constants::BAND_COLUMNS_PER_SECOND;
use super::constants::CHURN_CELLS_PER_FRAME;
use super::constants::GLYPHS;
use super::constants::MILLIS_PER_SECOND;
use super::constants::SUBCOLUMNS_PER_COLUMN;
use super::constants::XORSHIFT_FALLBACK_SEED;
use super::constants::XORSHIFT_FIRST;
use super::constants::XORSHIFT_SECOND;
use super::constants::XORSHIFT_THIRD;
use crate::theme::blend_color;

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

/// A lit strip of characters travelling across the grid.
///
/// [`advance`](Self::advance) moves it on by one frame's worth of time
/// and [`render`](Self::render) draws it over a [`Backdrop`]. The strip
/// sizes itself to whatever [`Rect`] it is advanced against, so a
/// resize costs a fresh set of characters and nothing more.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TravelingBand {
    /// Where the leading edge stands, in sub-columns from the left edge
    /// of the area.
    leading_edge:   u32,
    /// The character each cell is drawing, row-major over
    /// `width * height`.
    glyphs:         Vec<char>,
    /// Cells across.
    width:          u16,
    /// Cells down.
    height:         u16,
    /// How many columns the leading edge has re-rolled on this pass.
    rolled_through: u16,
    /// Source of the character churn.
    xorshift:       Xorshift,
    /// How far the whole strip is carried toward the ground it is drawn
    /// on, on the alpha scale [`blend_color`] reads: zero draws it at
    /// full strength, [`u8::MAX`] draws nothing.
    faded:          u8,
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
        if self.width == 0 || self.height == 0 {
            return;
        }
        let elapsed_millis = u32::try_from(elapsed.as_millis()).unwrap_or(u32::MAX);
        let travel = BAND_COLUMNS_PER_SECOND
            .saturating_mul(SUBCOLUMNS_PER_COLUMN)
            .saturating_mul(elapsed_millis)
            / MILLIS_PER_SECOND;
        self.leading_edge = self.leading_edge.saturating_add(travel);

        // The strip runs a whole band-width past the right edge before
        // coming back, so its tail clears the grid before its head
        // returns to the left.
        let span = (u32::from(self.width) + BAND_COLUMNS) * SUBCOLUMNS_PER_COLUMN;
        if self.leading_edge >= span {
            self.leading_edge %= span;
            self.rolled_through = 0;
        }
        self.roll_reached_columns();
        self.churn();
    }

    /// Carry the whole strip `faded` of the way toward the ground it is
    /// drawn on, which is how it leaves when the screen it decorates
    /// has something real to show. Zero is full strength, [`u8::MAX`]
    /// draws nothing at all.
    pub const fn fade(&mut self, faded: u8) { self.faded = faded; }

    /// Draw the strip over `area`, colouring each cell by the
    /// [`Backdrop`] underneath it and fading it toward `ground`.
    ///
    /// Cells outside the strip are left untouched rather than painted,
    /// so whatever the terminal shows through stays visible. A cell the
    /// backdrop has no colour for is skipped for the same reason.
    pub fn render(&self, area: Rect, backdrop: &Backdrop, ground: Color, buffer: &mut Buffer) {
        if self.faded == u8::MAX {
            return;
        }
        for row in 0..self.height.min(area.height) {
            for column in 0..self.width.min(area.width) {
                let (Some(alpha), Some(color)) =
                    (self.alpha_at(column), backdrop.color_at(column, row))
                else {
                    continue;
                };
                let Some(&glyph) = self.glyphs.get(self.cell_index(column, row)) else {
                    continue;
                };
                let lit = blend_color(blend_color(color, ground, alpha), ground, self.faded);
                if let Some(cell) = buffer.cell_mut((area.x + column, area.y + row)) {
                    cell.set_char(glyph);
                    cell.set_fg(lit);
                }
            }
        }
    }

    /// How far toward the ground a column is carried by its distance
    /// behind the leading edge, or [`None`] for a column the strip does
    /// not cover this frame.
    fn alpha_at(&self, column: u16) -> Option<u8> {
        let behind = self
            .leading_edge
            .checked_sub(u32::from(column) * SUBCOLUMNS_PER_COLUMN)?;
        let band = BAND_COLUMNS * SUBCOLUMNS_PER_COLUMN;
        if behind > band {
            return None;
        }
        u8::try_from(behind * u32::from(u8::MAX) / band).ok()
    }

    /// Where the cell at `column`, `row` sits in [`Self::glyphs`].
    fn cell_index(&self, column: u16, row: u16) -> usize {
        usize::from(row) * usize::from(self.width) + usize::from(column)
    }

    /// Re-size to `area`, drawing a fresh set of characters and putting
    /// the strip back at the left edge. Does nothing when the area has
    /// not changed.
    fn resize(&mut self, area: Rect) {
        if self.width == area.width && self.height == area.height {
            return;
        }
        self.width = area.width;
        self.height = area.height;
        self.leading_edge = 0;
        self.rolled_through = 0;
        let cells = usize::from(area.width) * usize::from(area.height);
        let mut glyphs = Vec::with_capacity(cells);
        for _ in 0..cells {
            glyphs.push(random_glyph(&mut self.xorshift));
        }
        self.glyphs = glyphs;
    }

    /// Draw a fresh character for every column the leading edge has
    /// reached since the last frame.
    fn roll_reached_columns(&mut self) {
        while self.rolled_through < self.width
            && u32::from(self.rolled_through) * SUBCOLUMNS_PER_COLUMN <= self.leading_edge
        {
            for row in 0..self.height {
                let index = self.cell_index(self.rolled_through, row);
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

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use super::*;

    /// An area big enough that the strip covers only part of it, so a
    /// covered and an uncovered column can both be asserted on.
    const AREA: Rect = Rect::new(0, 0, 80, 10);

    #[test]
    fn a_strip_sizes_itself_to_the_area_it_is_advanced_against() {
        let mut band = TravelingBand::new();

        band.advance(AREA, Duration::ZERO);

        assert_eq!(band.width, AREA.width);
        assert_eq!(band.height, AREA.height);
        assert_eq!(band.glyphs.len(), usize::from(AREA.width * AREA.height));
    }

    /// The leading edge is what the eye follows, so it draws at full
    /// strength and everything behind it is carried toward the ground.
    #[test]
    fn the_leading_edge_is_the_brightest_part_of_the_strip() {
        let mut band = TravelingBand::new();
        band.advance(AREA, Duration::ZERO);
        band.leading_edge = BAND_COLUMNS * SUBCOLUMNS_PER_COLUMN;

        let edge = band.alpha_at(u16::try_from(BAND_COLUMNS).unwrap_or(u16::MAX));
        let tail = band.alpha_at(0);

        assert_eq!(edge, Some(0));
        assert_eq!(tail, Some(u8::MAX));
    }

    /// A column the strip has not reached, and one its tail has already
    /// left, are both drawn as nothing at all.
    #[test]
    fn columns_outside_the_strip_are_not_covered() {
        let mut band = TravelingBand::new();
        band.advance(AREA, Duration::ZERO);
        band.leading_edge = BAND_COLUMNS * SUBCOLUMNS_PER_COLUMN;

        let ahead = band.alpha_at(u16::try_from(BAND_COLUMNS).unwrap_or(u16::MAX) + 1);

        assert_eq!(ahead, None);
        assert_eq!(TravelingBand::new().alpha_at(0), Some(0));
    }

    /// The strip has to come back round, or the attract screen shows
    /// one pass and then nothing.
    #[test]
    fn the_strip_returns_to_the_left_after_clearing_the_right_edge() {
        let mut band = TravelingBand::new();
        band.advance(AREA, Duration::ZERO);
        let span = (u32::from(AREA.width) + BAND_COLUMNS) * SUBCOLUMNS_PER_COLUMN;

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

        // The leading edge sits on column zero after the first advance,
        // so that cell is drawn at full strength and in the colour the
        // backdrop carries.
        let cell = buffer
            .cell((AREA.x, AREA.y))
            .expect("area covers its own origin");
        assert_eq!(cell.fg, color);
        assert!(GLYPHS.contains(&cell.symbol().chars().next().unwrap_or(' ')));
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
}
