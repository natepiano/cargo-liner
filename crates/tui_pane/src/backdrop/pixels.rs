//! The third attract-mode animation: the desktop drawn as itself, with
//! a band of coarseness sweeping across it.
//!
//! The other two draw something *over* the desktop -- a strip of
//! characters, a window of drifting lines -- and the colours are what
//! those things are cut out of. This draws nothing over it. Every cell
//! wears the colour the [`Backdrop`] has for it, which is as sharp as a
//! character grid can render a display, and what travels is a wave that
//! takes that sharpness away and gives it back.
//!
//! Inside the wave the cells clump: a block of them is averaged into
//! one colour and every cell in it wears that colour, so the picture
//! goes to blocks. Outside it every cell keeps its own. The wave has no
//! edges to speak of -- its profile is a [`smoothstep`] rising from
//! nothing at either side to the whole of it in the middle -- so what
//! crosses the screen is a picture coarsening and resolving rather than
//! a rectangle of blur with a border round it.
//!
//! # The block grid does not move
//!
//! Blocks are cut from the area's own origin and stay cut there. Only
//! how coarse each one stands changes as the wave arrives and leaves.
//! A grid that travelled with the wave would re-cut itself under the
//! colours every frame, and a block boundary landing somewhere new each
//! time reads as the picture boiling rather than as blocks resolving.
//!
//! What does move through a block is the wave, which is read at the
//! cell rather than at the block -- see [`ResolvingPixels::color_at`].
//! The cells of a block still answer to one colour, the block's, and
//! differ only in how far they have been carried toward it, so a wave
//! narrower than a block crosses it instead of turning the whole of it
//! over at once and back a step later.
//!
//! # Three ways back
//!
//! [`PixelResolve`] says what a block does as the wave leaves it, and
//! the three are genuinely different to watch: a crossfade, a stepped
//! subdivision that reads like an image loading in, and a scatter that
//! gives the cells back one at a time.

use std::time::Duration;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;

use super::Backdrop;
use super::BandDirection;
use super::cell_pixels;
use super::constants::DEFAULT_BLOCK_COLUMNS;
use super::constants::DEFAULT_PIXEL_SPEED;
use super::constants::DEFAULT_PIXEL_WAVE_PERCENT;
use super::constants::MAX_BLOCK_COLUMNS;
use super::constants::MAX_PIXEL_SPEED;
use super::constants::MAX_PIXEL_WAVE_PERCENT;
use super::constants::MICROS_PER_SECOND;
use super::constants::MIN_BLOCK_COLUMNS;
use super::constants::MIN_PIXEL_SPEED;
use super::constants::MIN_PIXEL_WAVE_PERCENT;
use super::constants::PIXEL_BEHIND_FADE;
use super::constants::PIXEL_STEP_LEVELS;
use super::constants::PIXEL_WAVE_START_PERCENT;
use super::constants::SHADES;
use super::constants::SUBCELLS_PER_CELL;
use super::constants::WHOLE_PERCENT;
use super::random::Xorshift;
use super::smoothstep;
use crate::theme;

/// Which of the grid's two axes a wave of coarseness sweeps along.
///
/// A [`BandDirection`] says both this and which end the wave enters by,
/// and the two questions want separate answers: turning a wave round on
/// the axis it is already on keeps every block where it stands, while
/// turning it onto the other axis is a different lap altogether.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SweepAxis {
    /// Across the columns, so a line of the field is a column.
    Columns,
    /// Down the rows, so a line of the field is a row.
    Rows,
}

impl From<BandDirection> for SweepAxis {
    fn from(direction: BandDirection) -> Self {
        match direction {
            BandDirection::Left | BandDirection::Right => Self::Columns,
            BandDirection::Up | BandDirection::Down => Self::Rows,
        }
    }
}

/// What a block of a [`ResolvingPixels`] does as the wave of coarseness
/// leaves it and its cells come back.
///
/// [`next`](Self::next) steps through all three, which is what one key
/// cycling them walks along.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum PixelResolve {
    /// The block crossfades between the one colour it averaged to and
    /// the colours its cells carry of their own.
    ///
    /// The smoothest of the three and where the display starts: nothing
    /// in it steps, so a wave crossing slowly has something to show on
    /// every frame.
    #[default]
    Blend,
    /// The block halves, and halves again, through
    /// `PIXEL_STEP_LEVELS` sizes before its cells stand on their own.
    ///
    /// Nothing is mixed here -- a cell wears the average of whichever
    /// size of block it currently belongs to -- so what the reader
    /// watches is the picture arriving in passes, the way an image
    /// loading over a slow line does.
    Step,
    /// Each cell of the block comes back at a moment of its own, drawn
    /// once when the field was sized.
    ///
    /// The block does not shrink and does not fade; it thins. Cells
    /// return to their own colours in a scattered order, so the coarse
    /// picture is eaten away from the inside rather than replaced.
    Scatter,
}

impl PixelResolve {
    /// The next of the three, wrapping back to the first.
    #[must_use]
    pub const fn next(self) -> Self {
        match self {
            Self::Blend => Self::Step,
            Self::Step => Self::Scatter,
            Self::Scatter => Self::Blend,
        }
    }
}

/// What a cell of a [`ResolvingPixels`] is painted with, once its
/// colour has been settled.
///
/// The colour is the same either way. This is only how the cell is made
/// to wear it, and what it changes is the texture of the whole field.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum PixelFill {
    /// The whole cell in its colour, with no character standing on it.
    /// The plainest reading of the desktop, and where the display
    /// starts.
    #[default]
    Solid,
    /// One of `SHADES`, picked by how bright the cell is against the
    /// rest of the field.
    Shades,
}

impl PixelFill {
    /// The other of the two, which is all the key that toggles them
    /// asks for.
    #[must_use]
    pub const fn next(self) -> Self {
        match self {
            Self::Solid => Self::Shades,
            Self::Shades => Self::Solid,
        }
    }
}

/// The parameters that steering can change on [`ResolvingPixels`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PixelSettings {
    /// Which way the wave sweeps.
    pub direction:     BandDirection,
    /// How far the wave travels each second, in cells.
    pub speed:         u32,
    /// How much of the field the wave covers, as a percentage.
    pub wave_percent:  u32,
    /// How many columns one block covers at its coarsest.
    pub block_columns: u32,
    /// What blocks do as the wave leaves them.
    pub resolve:       PixelResolve,
    /// What each cell is painted with.
    pub fill:          PixelFill,
}

/// The channels summed over one block, and how many cells went into
/// them.
///
/// Summed rather than averaged as it goes, because a running mean over
/// whole numbers loses a little of every cell it folds in and a block
/// dozens of cells across would come out visibly darker than the cells
/// it was read from.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct BlockSum {
    /// Red summed over every cell of the block the backdrop had a
    /// colour for.
    red:   u32,
    /// Green over those same cells.
    green: u32,
    /// Blue over those same cells.
    blue:  u32,
    /// How many cells that was, which is what the sums are divided by
    /// and what says whether there is an average at all.
    cells: u32,
}

impl BlockSum {
    /// The colour these cells average to, or [`None`] where the
    /// backdrop had a colour for none of them.
    fn mean(self) -> Option<Color> {
        if self.cells == 0 {
            return None;
        }
        Some(Color::Rgb(
            channel(self.red / self.cells),
            channel(self.green / self.cells),
            channel(self.blue / self.cells),
        ))
    }

    /// Fold one cell's colour in.
    fn add(&mut self, red: u8, green: u8, blue: u8) {
        self.red = self.red.saturating_add(u32::from(red));
        self.green = self.green.saturating_add(u32::from(green));
        self.blue = self.blue.saturating_add(u32::from(blue));
        self.cells = self.cells.saturating_add(1);
    }
}

/// The field summed into blocks of one size.
#[derive(Clone, Debug, Eq, PartialEq)]
struct Blocks {
    /// Cells across one block.
    columns: u32,
    /// Cells down one block.
    rows:    u32,
    /// How many blocks stand across the field, so [`Self::sums`] can be
    /// indexed from a column and a row.
    across:  u32,
    /// Row-major, one entry per block.
    sums:    Vec<BlockSum>,
}

impl Blocks {
    /// An empty grid of blocks `columns` by `rows` cells, over a field
    /// of `width` by `height` of them.
    fn over(width: u32, height: u32, columns: u32, rows: u32) -> Self {
        let across = width.div_ceil(columns).max(1);
        let down = height.div_ceil(rows).max(1);
        Self {
            columns,
            rows,
            across,
            sums: vec![BlockSum::default(); usize::try_from(across * down).unwrap_or(0)],
        }
    }

    /// Where in [`Self::sums`] the block holding the cell at `column`,
    /// `row` sits.
    fn index(&self, column: u32, row: u32) -> usize {
        let block = (row / self.rows)
            .saturating_mul(self.across)
            .saturating_add(column / self.columns);
        usize::try_from(block).unwrap_or(0)
    }

    /// The colour the block holding the cell at `column`, `row`
    /// averages to.
    fn mean_at(&self, column: u32, row: u32) -> Option<Color> {
        self.sums.get(self.index(column, row)).copied()?.mean()
    }
}

/// The field summed into blocks at every size one is drawn at, and how
/// dark and how bright it runs.
///
/// Read once per frame and thrown away with it. The colours come from a
/// capture the monitor replaces on its own clock, so an average carried
/// across frames would draw the last capture's blocks over this one's
/// cells.
#[derive(Clone, Debug, Eq, PartialEq)]
struct Coarsened {
    /// One entry per level, finest first, so
    /// [`PIXEL_STEP_LEVELS`] - 1 is the whole block.
    levels:    Vec<Blocks>,
    /// The dimmest the desktop runs anywhere the field covers, on the
    /// scale [`brightness`] reads.
    dimmest:   u32,
    /// The brightest it runs there, on that same scale. Together with
    /// [`Self::dimmest`] this is what [`SHADES`] is stretched across.
    brightest: u32,
}

impl Coarsened {
    /// The blocks at the coarsest size, which is the one every resolve
    /// but [`PixelResolve::Step`] reads.
    fn coarsest(&self) -> Option<&Blocks> { self.levels.last() }
}

/// The desktop drawn as itself, with a band of coarseness sweeping
/// across it.
///
/// An app holds one, hands it a [`Rect`] and the time since the last
/// frame through [`advance`](Self::advance), and draws it with
/// [`render`](Self::render). What the reader steers is
/// [`set_direction`](Self::set_direction), [`speed_up`](Self::speed_up)
/// and [`slow_down`](Self::slow_down), [`coarsen`](Self::coarsen) and
/// [`sharpen`](Self::sharpen), [`wider`](Self::wider) and
/// [`narrower`](Self::narrower), [`cycle_resolve`](Self::cycle_resolve)
/// and [`cycle_fill`](Self::cycle_fill). Each is clamped here rather
/// than at the call site, so an app can hand a held key straight
/// through without working out where the limits are.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvingPixels {
    /// Cells across the area the field was last sized to.
    columns:       u16,
    /// Cells down that same area.
    rows:          u16,
    /// Which way the wave of coarseness sweeps, and so which edge it
    /// enters by.
    direction:     BandDirection,
    /// Where the middle of the wave stands, in sub-cells from that
    /// edge.
    middle:        u32,
    /// How far the wave travels each second, in cells.
    speed:         u32,
    /// How much of the field the wave covers, as a percentage of the
    /// grid's extent along the axis it sweeps.
    ///
    /// A percentage rather than a count of cells, unlike
    /// [`TravelingBand`](super::TravelingBand)'s depth. What the wave
    /// is read as is how much of the field is coarse at once, and that
    /// is a share of the field rather than a distance on the screen --
    /// so a share is what survives a resize and a turn onto the other
    /// axis unchanged.
    wave_percent:  u32,
    /// How many columns one block covers at its coarsest.
    block_columns: u32,
    /// One character cell across and down, in pixels scaled by
    /// [`PIXEL_PRECISION`](super::constants::PIXEL_PRECISION), or
    /// zeroes where the terminal will not say.
    ///
    /// What this is for is the block's rows: a cell is taller than it
    /// is wide, so a block that reads square on the screen is more
    /// columns than rows.
    cell_pixels:   (u32, u32),
    /// When each cell comes back while the field is scattering, on the
    /// scale a coarseness is held on, row-major over
    /// `columns * rows`.
    ///
    /// Drawn once with the area rather than per frame. A threshold
    /// re-drawn every frame would have each cell flickering between its
    /// own colour and its block's for as long as the wave stood over
    /// it, which is static rather than a scatter.
    grains:        Vec<u8>,
    /// What a block does as the wave leaves it.
    resolve:       PixelResolve,
    /// What a cell is painted with once its colour is settled.
    fill:          PixelFill,
    /// Source of the moments the cells come back at.
    xorshift:      Xorshift,
    /// How far the whole field is carried toward the ground it is drawn
    /// on, on the alpha scale [`blend_color`](theme::blend_color)
    /// reads: zero draws it at full strength, [`u8::MAX`] draws
    /// nothing.
    faded:         u8,
}

impl Default for ResolvingPixels {
    fn default() -> Self {
        Self {
            columns:       0,
            rows:          0,
            direction:     BandDirection::default(),
            middle:        0,
            speed:         DEFAULT_PIXEL_SPEED,
            wave_percent:  DEFAULT_PIXEL_WAVE_PERCENT,
            block_columns: DEFAULT_BLOCK_COLUMNS,
            cell_pixels:   (0, 0),
            grains:        Vec::new(),
            resolve:       PixelResolve::default(),
            fill:          PixelFill::default(),
            xorshift:      Xorshift::default(),
            faded:         0,
        }
    }
}

impl ResolvingPixels {
    /// A field that has not been sized yet. The first
    /// [`advance`](Self::advance) settles its area.
    #[must_use]
    pub fn new() -> Self { Self::default() }

    /// The field's current steerable parameters.
    #[must_use]
    pub const fn settings(&self) -> PixelSettings {
        PixelSettings {
            direction:     self.direction,
            speed:         self.speed,
            wave_percent:  self.wave_percent,
            block_columns: self.block_columns,
            resolve:       self.resolve,
            fill:          self.fill,
        }
    }

    /// Restores steerable parameters through the same transitions as
    /// the individual steering methods.
    pub fn apply(&mut self, settings: PixelSettings) {
        self.set_direction(settings.direction);
        self.set_resolve(settings.resolve);
        self.set_fill(settings.fill);
        self.set_speed(settings.speed);
        self.set_wave(settings.wave_percent);
        self.set_block_columns(settings.block_columns);
    }

    /// Generates steerable parameters deterministically from `seed`.
    #[must_use]
    pub fn random_settings(&self, seed: u64) -> PixelSettings {
        let mut xorshift = Xorshift::seeded(seed);
        PixelSettings {
            direction:     match xorshift.index(4) {
                0 => BandDirection::Left,
                1 => BandDirection::Right,
                2 => BandDirection::Up,
                _ => BandDirection::Down,
            },
            speed:         xorshift.u32_inclusive(MIN_PIXEL_SPEED, MAX_PIXEL_SPEED),
            wave_percent:  xorshift.u32_inclusive(MIN_PIXEL_WAVE_PERCENT, MAX_PIXEL_WAVE_PERCENT),
            block_columns: xorshift.u32_inclusive(MIN_BLOCK_COLUMNS, MAX_BLOCK_COLUMNS),
            resolve:       match xorshift.index(3) {
                0 => PixelResolve::Blend,
                1 => PixelResolve::Step,
                _ => PixelResolve::Scatter,
            },
            fill:          match xorshift.index(2) {
                0 => PixelFill::Solid,
                _ => PixelFill::Shades,
            },
        }
    }

    /// Carry the wave one frame further across the field, sizing it to
    /// `area` first.
    ///
    /// The wave wraps rather than running clear of the grid and
    /// starting over, for the reason
    /// [`TravelingBand`](super::TravelingBand) wraps: a wave that
    /// finished each pass would leave the field entirely sharp for as
    /// long as it took to cross again, which on a wide grid at a slow
    /// speed is most of the time the reader is watching.
    pub fn advance(&mut self, area: Rect, elapsed: Duration) {
        self.resize(area);
        let span = self.span();
        if span == 0 {
            return;
        }
        let elapsed_micros = u64::try_from(elapsed.as_micros()).unwrap_or(u64::MAX);
        let travel = u64::from(self.speed)
            .saturating_mul(u64::from(SUBCELLS_PER_CELL))
            .saturating_mul(elapsed_micros)
            / MICROS_PER_SECOND;
        self.middle = self
            .middle
            .saturating_add(u32::try_from(travel).unwrap_or(u32::MAX))
            % span;
    }

    /// Carry the whole field this far toward the ground it is drawn on.
    /// Zero draws it at full strength and [`u8::MAX`] draws nothing.
    pub const fn fade(&mut self, faded: u8) { self.faded = faded; }

    /// Paint the cells solid or draw them with shading characters,
    /// whichever they are not wearing now.
    ///
    /// Costs the field nothing: the colours are worked out the same way
    /// either way and this only changes what is written into the cell,
    /// so the wave carries on from exactly where it stood.
    pub const fn cycle_fill(&mut self) { self.set_fill(self.fill.next()); }

    /// Step to the next of the three ways a block gives its cells back
    /// -- see [`PixelResolve::next`].
    ///
    /// The wave is not moved and the blocks are not re-cut, so what the
    /// reader sees is the same wave over the same picture, resolving it
    /// another way.
    pub const fn cycle_resolve(&mut self) { self.set_resolve(self.resolve.next()); }

    /// Which way the wave sweeps, and so which edge it enters by.
    ///
    /// Turned round on the axis it is already on, the wave keeps its
    /// place on the screen and sets off back the way it came: the lines
    /// are counted from the other end now, so the same place is the
    /// same distance from the other end of the lap. Turned onto the
    /// other axis it keeps its share of a lap instead, a lap along the
    /// rows being a different length from one along the columns.
    ///
    /// Either way it is not put back at the edge. A wave that restarted
    /// on every press would make the arrows read as a reset rather than
    /// as a direction.
    pub fn set_direction(&mut self, direction: BandDirection) {
        if self.direction == direction {
            return;
        }
        let span = self.span();
        let turned = SweepAxis::from(self.direction) != SweepAxis::from(direction);
        self.direction = direction;
        if span == 0 {
            return;
        }
        self.middle = if turned {
            let lap = self.span();
            u32::try_from(u64::from(self.middle) * u64::from(lap) / u64::from(span)).unwrap_or(0)
        } else {
            span.saturating_sub(self.middle)
        };
    }

    /// Speed the wave up by `cells_per_second`, never past the fastest
    /// it is allowed to travel.
    pub fn speed_up(&mut self, cells_per_second: u32) {
        self.set_speed(self.speed.saturating_add(cells_per_second));
    }

    /// Slow the wave down by `cells_per_second`, never past the slowest:
    /// a wave stopped dead is one the reader cannot tell from a frozen
    /// display.
    pub fn slow_down(&mut self, cells_per_second: u32) {
        self.set_speed(self.speed.saturating_sub(cells_per_second));
    }

    /// Draw the blocks `columns` wider, never past the widest one is
    /// drawn.
    pub fn coarsen(&mut self, columns: u32) {
        self.set_block_columns(self.block_columns.saturating_add(columns));
    }

    /// Draw the blocks `columns` narrower, never past the narrowest.
    pub fn sharpen(&mut self, columns: u32) {
        self.set_block_columns(self.block_columns.saturating_sub(columns));
    }

    /// Stand the wave `percent` deeper, as a share of the axis it
    /// sweeps, up to the whole field standing at one coarseness.
    pub fn wider(&mut self, percent: u32) {
        self.set_wave(self.wave_percent.saturating_add(percent));
    }

    /// Stand the wave `percent` shallower, never past the narrowest it
    /// is drawn.
    pub fn narrower(&mut self, percent: u32) {
        self.set_wave(self.wave_percent.saturating_sub(percent));
    }

    /// Draw the field where it currently stands, moving nothing.
    ///
    /// Every cell is drawn, as in
    /// [`DriftingText`](super::DriftingText) and for the same reason:
    /// what the reader is looking at is the desktop, and a cell left
    /// out is a piece of it missing rather than an edge to read. A cell
    /// the backdrop has no colour for is skipped, so whatever the
    /// terminal shows through stays visible.
    ///
    /// Leaving goes toward whatever each cell is already painted on,
    /// `ground` only standing in where the cell is painted on nothing
    /// at all.
    pub fn render(&self, area: Rect, backdrop: &Backdrop, ground: Color, buffer: &mut Buffer) {
        if self.faded == u8::MAX {
            return;
        }
        let coarsened = self.coarsen_field(backdrop);
        for row in 0..self.rows.min(area.height) {
            for column in 0..self.columns.min(area.width) {
                let Some(own) = backdrop.color_at(column, row) else {
                    continue;
                };
                let color = self.color_at(&coarsened, own, u32::from(column), u32::from(row));
                let Some(cell) = buffer.cell_mut((area.x + column, area.y + row)) else {
                    continue;
                };
                let toward = match cell.bg {
                    Color::Reset => ground,
                    background => background,
                };
                let drawn = theme::blend_color(color, toward, self.faded);
                match self.fill {
                    PixelFill::Solid => {
                        cell.set_char(' ');
                        cell.set_fg(drawn);
                        cell.set_bg(drawn);
                    },
                    PixelFill::Shades => {
                        cell.set_char(shade(color, coarsened.dimmest, coarsened.brightest));
                        cell.set_fg(drawn);
                        cell.set_bg(theme::blend_color(drawn, toward, PIXEL_BEHIND_FADE));
                    },
                }
            }
        }
    }

    /// Which axis the wave is sweeping along.
    fn axis(&self) -> SweepAxis { SweepAxis::from(self.direction) }

    /// How many lines the wave crosses to make one lap: the columns
    /// while it sweeps sideways and the rows while it sweeps up or
    /// down.
    fn lines(&self) -> u32 {
        match self.axis() {
            SweepAxis::Columns => u32::from(self.columns),
            SweepAxis::Rows => u32::from(self.rows),
        }
    }

    /// One lap of the wave, in sub-cells, or zero before the field has
    /// been sized.
    fn span(&self) -> u32 { self.lines().saturating_mul(SUBCELLS_PER_CELL) }

    /// How far the wave reaches either side of its middle, in
    /// sub-cells: the radius inside which every block is as coarse as
    /// the field goes, and the radius at which the field is sharp
    /// again.
    ///
    /// The first hundred percent opens the wave out from nothing to the
    /// whole of the axis, coarsest at the middle and falling away the
    /// whole distance. The second hundred flattens that fall -- the
    /// coarse middle grows until it reaches as far as the wave does, at
    /// which point every block on the screen is at one coarseness and
    /// there is no wave left to watch travel, only the picture in
    /// blocks.
    ///
    /// The sharp radius is half a cell at the narrowest however small
    /// the percentage is drawn: a wave thinner than the cells it
    /// crosses stands between two of them and coarsens neither, which
    /// is a display with nothing on it.
    fn wave_radii(&self) -> (u32, u32) {
        let half = self.span() / 2;
        let opening = self.wave_percent.min(WHOLE_PERCENT);
        let flattening = self.wave_percent.saturating_sub(WHOLE_PERCENT);
        let sharp = (half.saturating_mul(opening) / WHOLE_PERCENT).max(SUBCELLS_PER_CELL / 2);
        let coarse = half.saturating_mul(flattening) / WHOLE_PERCENT;
        (coarse.min(sharp), sharp)
    }

    /// How far along the axis the wave sweeps the cell at `column`,
    /// `row` stands, counted from the edge the wave enters by.
    ///
    /// Counting from the entering edge rather than from the grid's own
    /// origin is what lets [`Self::middle`] only ever grow: the
    /// direction decides which end the count starts at, and the travel
    /// is the same arithmetic all four ways.
    fn along(&self, column: u32, row: u32) -> u32 {
        match self.direction {
            BandDirection::Right => column,
            BandDirection::Left => u32::from(self.columns)
                .saturating_sub(1)
                .saturating_sub(column),
            BandDirection::Down => row,
            BandDirection::Up => u32::from(self.rows).saturating_sub(1).saturating_sub(row),
        }
    }

    /// How coarse the field stands at the line through the cell at
    /// `column`, `row`: zero is every cell wearing its own colour and
    /// [`u8::MAX`] is a whole block of them wearing one.
    ///
    /// Distance to the wave's middle is measured the short way round
    /// the ring, so a wave part way off one edge is still arriving at
    /// the other.
    fn coarse_at(&self, column: u32, row: u32) -> u8 {
        let span = self.span();
        let (coarse, sharp) = self.wave_radii();
        if span == 0 {
            return 0;
        }
        // The cell's own middle rather than its near edge, so a cell
        // is read where it stands rather than half a cell early.
        let at = self
            .along(column, row)
            .saturating_mul(SUBCELLS_PER_CELL)
            .saturating_add(SUBCELLS_PER_CELL / 2)
            % span;
        let ahead = (at + span - self.middle % span) % span;
        let away = ahead.min(span - ahead);
        // The flat middle is asked about first, because at the widest
        // the wave is drawn the two radii meet: everything the field
        // has is inside the coarse one, and the ramp below has nothing
        // left to run across.
        if away <= coarse {
            return u8::MAX;
        }
        if away >= sharp {
            return 0;
        }
        let whole = u32::from(u8::MAX);
        let across = whole.saturating_sub(whole.saturating_mul(away - coarse) / (sharp - coarse));
        u8::try_from(smoothstep(across, whole)).unwrap_or(u8::MAX)
    }

    /// How many cells one block covers across and down at its coarsest.
    ///
    /// The rows come from the columns and the cell's own measurements
    /// rather than being steered separately, so one key governs how
    /// coarse the field goes and a block reads square whatever the
    /// terminal's cell measures. Where the terminal will not say, the
    /// two counts are equal -- which is what a block was before there
    /// was anything to scale it by.
    fn block_size(&self) -> (u32, u32) {
        let (across, down) = self.cell_pixels;
        if across == 0 || down == 0 {
            return (self.block_columns, self.block_columns);
        }
        // Rounded rather than truncated, so a block an odd number of
        // columns wide does not come out a row shallower than it should.
        let rows = (self.block_columns.saturating_mul(across) + down / 2) / down;
        (self.block_columns, rows.max(1))
    }

    /// How many cells one block covers across and down at `level`,
    /// where zero is the finest size the field is drawn at and
    /// [`PIXEL_STEP_LEVELS`] - 1 is the whole block.
    fn level_size(&self, level: u32) -> (u32, u32) {
        let (columns, rows) = self.block_size();
        let halvings = PIXEL_STEP_LEVELS.saturating_sub(1).saturating_sub(level);
        let divisor = 1_u32.checked_shl(halvings).unwrap_or(1).max(1);
        ((columns / divisor).max(1), (rows / divisor).max(1))
    }

    /// Sum the desktop into blocks at every size one is drawn at, and
    /// note how dark and how bright it runs.
    ///
    /// One pass over the cells, folding each into every level at once.
    /// A pass per level would read the backdrop
    /// [`PIXEL_STEP_LEVELS`] times over, and the read is the expensive
    /// half.
    fn coarsen_field(&self, backdrop: &Backdrop) -> Coarsened {
        let width = u32::from(self.columns);
        let height = u32::from(self.rows);
        let mut levels: Vec<Blocks> = (0..PIXEL_STEP_LEVELS)
            .map(|level| {
                let (columns, rows) = self.level_size(level);
                Blocks::over(width, height, columns, rows)
            })
            .collect();
        let mut dimmest = u32::MAX;
        let mut brightest = 0;
        for row in 0..self.rows {
            for column in 0..self.columns {
                let Some(Color::Rgb(red, green, blue)) = backdrop.color_at(column, row) else {
                    continue;
                };
                let lit = brightness(red, green, blue);
                dimmest = dimmest.min(lit);
                brightest = brightest.max(lit);
                for blocks in &mut levels {
                    let index = blocks.index(u32::from(column), u32::from(row));
                    if let Some(sum) = blocks.sums.get_mut(index) {
                        sum.add(red, green, blue);
                    }
                }
            }
        }
        Coarsened {
            levels,
            // A field the backdrop had no colour for anywhere leaves
            // these crossed over, and a range read the wrong way round
            // would put every cell at one end of the ramp. Nothing is
            // drawn in that case, so either answer will do -- and a
            // range of nothing is the one that says so.
            dimmest: dimmest.min(brightest),
            brightest,
        }
    }

    /// What the cell at `column`, `row` is painted, given the colour the
    /// desktop has for it and the blocks the field was summed into.
    ///
    /// The wave is read at the cell, not at the block's centre. Read
    /// once per block, a wave narrower than a block has the whole of
    /// one turn sharp and turn back a step later, so what travels is
    /// not the wave but the block boundaries it lights up. Read per
    /// cell, the cells of a block still answer to one colour -- the
    /// block's -- and differ only in how far they have been carried
    /// toward it, so the wave crosses a block the way it crosses the
    /// field.
    fn color_at(&self, coarsened: &Coarsened, own: Color, column: u32, row: u32) -> Color {
        let coarse = self.coarse_at(column, row);
        if coarse == 0 {
            return own;
        }
        match self.resolve {
            PixelResolve::Blend => coarsened
                .coarsest()
                .and_then(|blocks| blocks.mean_at(column, row))
                .map_or(own, |mean| theme::blend_color(own, mean, coarse)),
            PixelResolve::Step => coarsened
                .levels
                .get(usize::try_from(step_level(coarse)).unwrap_or(0))
                .and_then(|blocks| blocks.mean_at(column, row))
                .unwrap_or(own),
            PixelResolve::Scatter => {
                let index = usize::try_from(
                    row.saturating_mul(u32::from(self.columns))
                        .saturating_add(column),
                )
                .unwrap_or(0);
                let grain = self.grains.get(index).copied().unwrap_or(u8::MAX);
                if coarse <= grain {
                    return own;
                }
                coarsened
                    .coarsest()
                    .and_then(|blocks| blocks.mean_at(column, row))
                    .unwrap_or(own)
            },
        }
    }

    /// Re-size to `area`, drawing a fresh moment for every cell to come
    /// back at. Does nothing when the area has not changed.
    fn resize(&mut self, area: Rect) {
        if self.columns == area.width && self.rows == area.height {
            return;
        }
        self.columns = area.width;
        self.rows = area.height;
        if let Some(measured) = cell_pixels() {
            self.cell_pixels = measured;
        }
        self.middle = self.span().saturating_mul(PIXEL_WAVE_START_PERCENT) / WHOLE_PERCENT;
        let cells = usize::from(area.width) * usize::from(area.height);
        let mut grains = Vec::with_capacity(cells);
        for _ in 0..cells {
            grains.push(self.xorshift.byte());
        }
        self.grains = grains;
    }

    /// Set how far the wave travels, held inside the range it is
    /// allowed to.
    fn set_speed(&mut self, cells_per_second: u32) {
        self.speed = cells_per_second.clamp(MIN_PIXEL_SPEED, MAX_PIXEL_SPEED);
    }

    /// Set how wide a block is drawn, held inside the range one is
    /// drawn at.
    fn set_block_columns(&mut self, columns: u32) {
        self.block_columns = columns.clamp(MIN_BLOCK_COLUMNS, MAX_BLOCK_COLUMNS);
    }

    /// Set how deep the wave stands, held inside the range it is drawn
    /// at.
    fn set_wave(&mut self, percent: u32) {
        self.wave_percent = percent.clamp(MIN_PIXEL_WAVE_PERCENT, MAX_PIXEL_WAVE_PERCENT);
    }

    /// Set what blocks do as the wave leaves them.
    const fn set_resolve(&mut self, resolve: PixelResolve) { self.resolve = resolve; }

    /// Set what each cell is painted with.
    const fn set_fill(&mut self, fill: PixelFill) { self.fill = fill; }
}

/// How much light a colour carries, as the sum of its three channels.
///
/// Not a perceptual measure and not meant to be: this picks one of the
/// four characters in [`SHADES`], and the weights that separate a
/// perceptual luminance from this never move a cell across one of three
/// boundaries.
fn brightness(red: u8, green: u8, blue: u8) -> u32 {
    u32::from(red) + u32::from(green) + u32::from(blue)
}

/// One averaged channel back on the scale a colour is written on.
fn channel(mean: u32) -> u8 { u8::try_from(mean).unwrap_or(u8::MAX) }

/// Which of [`SHADES`] a cell wearing `color` draws, with the ramp
/// stretched from the dimmest the field runs to the brightest.
///
/// Stretched rather than read against the whole of what a colour could
/// be, for the reason [`DriftingText`](super::DriftingText) stretches
/// its bars: a desktop of dark greys occupies a narrow band near the
/// bottom of the absolute scale, and read against that scale every cell
/// of it rounds to the same character -- so the picture that is there
/// goes undrawn.
fn shade(color: Color, dimmest: u32, brightest: u32) -> char {
    let last = SHADES.last().copied().unwrap_or(' ');
    let Color::Rgb(red, green, blue) = color else {
        return last;
    };
    let steps = u32::try_from(SHADES.len()).unwrap_or(1).max(1);
    let across = brightest.saturating_sub(dimmest).max(1);
    let level = brightness(red, green, blue)
        .saturating_sub(dimmest)
        .saturating_mul(steps)
        / across;
    SHADES
        .get(usize::try_from(level.min(steps - 1)).unwrap_or(0))
        .copied()
        .unwrap_or(last)
}

/// Which of the sizes a block is drawn at `coarse` calls for: zero is
/// the finest and [`PIXEL_STEP_LEVELS`] - 1 the whole block.
fn step_level(coarse: u8) -> u32 {
    let whole = u32::from(u8::MAX).saturating_add(1);
    u32::from(coarse).saturating_mul(PIXEL_STEP_LEVELS) / whole
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use super::*;

    /// The area every test here sizes its field to. Wide enough to hold
    /// several default blocks along either axis, so a wave standing
    /// over one of them leaves others for it to be read against.
    const AREA: Rect = Rect::new(0, 0, 80, 24);
    /// The gap between two frames the tests walk the wave by. Any
    /// non-zero span will do -- what is being asked is whether the
    /// blocks move when the wave does, not how far it got.
    const FRAME: Duration = Duration::from_millis(20);

    /// A field sized to [`AREA`] with the wave put where the caller
    /// wants it, in sub-cells from the edge it enters by.
    fn field_with_wave_at(middle: u32) -> ResolvingPixels {
        let mut pixels = ResolvingPixels::new();
        pixels.advance(AREA, Duration::ZERO);
        pixels.middle = middle;
        pixels
    }

    /// Where the wave's middle stands for a field whose leftmost column
    /// is the coarsest thing on it.
    const AT_THE_FIRST_COLUMN: u32 = SUBCELLS_PER_CELL / 2;

    /// The wave is coarsest where its middle stands and sharp well away
    /// from it. Without that the field is either wholly coarse or
    /// wholly sharp, and there is no wave to watch.
    #[test]
    fn the_wave_is_coarsest_at_its_middle_and_sharp_away_from_it() {
        let pixels = field_with_wave_at(AT_THE_FIRST_COLUMN);

        assert_eq!(
            pixels.coarse_at(0, 0),
            u8::MAX,
            "the line the wave's middle stands on should be as coarse \
             as the field goes"
        );
        // Half the columns along, which at the default width is well
        // past half the wave's own depth.
        let far = u32::from(AREA.width) / 2;
        assert_eq!(
            pixels.coarse_at(far, 0),
            0,
            "and a line the wave does not reach should be sharp"
        );
    }

    /// The wave has no edge: coarseness rises from nothing at its sides
    /// to the whole of it in the middle. An edge would draw a rectangle
    /// of blur with a border round it rather than a picture coarsening.
    #[test]
    fn the_wave_rises_to_its_middle_rather_than_standing_at_an_edge() {
        let pixels = field_with_wave_at(AT_THE_FIRST_COLUMN);
        let half = pixels.wave_radii().1 / SUBCELLS_PER_CELL;

        let mut last = u8::MAX;
        for column in 0..half {
            let coarse = pixels.coarse_at(column, 0);
            assert!(
                coarse <= last,
                "coarseness should fall away from the middle, but \
                 column {column} stands at {coarse} against {last}"
            );
            last = coarse;
        }
        assert!(
            last < u8::MAX / 4,
            "and it should have fallen most of the way by the wave's \
             own edge, not dropped there"
        );
    }

    /// At the widest the wave is drawn the field stands at one
    /// coarseness the whole way round, so the screen can be asked for
    /// the whole picture in blocks.
    ///
    /// Stopping the wave at the whole of its axis left a fall-off at
    /// either end that nothing could take out, and a field with a
    /// permanent soft edge is the one thing this screen cannot show.
    #[test]
    fn the_widest_wave_leaves_no_sharp_field() {
        let mut pixels = field_with_wave_at(AT_THE_FIRST_COLUMN);
        pixels.wider(MAX_PIXEL_WAVE_PERCENT);

        for column in 0..u32::from(AREA.width) {
            assert_eq!(
                pixels.coarse_at(column, 0),
                u8::MAX,
                "column {column} should stand as coarse as the field goes"
            );
        }
    }

    /// The wave crosses a block a cell at a time rather than turning
    /// the whole of it over at once.
    ///
    /// Read once per block, a wave narrower than a block had the whole
    /// of one turn sharp and turn back a step later, so what travelled
    /// was not the wave but the boundaries it lit up.
    #[test]
    fn the_wave_crosses_a_block_a_cell_at_a_time() {
        let mut pixels = field_with_wave_at(AT_THE_FIRST_COLUMN);
        pixels.narrower(MAX_PIXEL_WAVE_PERCENT);
        let (columns, _) = pixels.block_size();

        let across: Vec<u8> = (0..columns)
            .map(|column| pixels.coarse_at(column, 0))
            .collect();

        assert!(
            across.windows(2).any(|pair| pair[0] != pair[1]),
            "the cells of one block should not all stand at one \
             coarseness while the wave is inside it, but read {across:?}"
        );
    }

    /// The blocks are cut from the area's own origin and stay cut there
    /// while the wave crosses them. A grid that travelled with the wave
    /// would re-cut itself under the colours every frame, which reads as
    /// the picture boiling rather than as blocks resolving.
    #[test]
    fn the_blocks_stay_where_they_were_cut_while_the_wave_moves() {
        let mut pixels = field_with_wave_at(AT_THE_FIRST_COLUMN);
        let backdrop = Backdrop::stepped(AREA);
        let before = pixels.coarsen_field(&backdrop);

        for _ in 0..u8::MAX {
            pixels.advance(AREA, FRAME);
        }
        let after = pixels.coarsen_field(&backdrop);

        assert_eq!(
            before.levels, after.levels,
            "the same desktop should sum into the same blocks however \
             far the wave has travelled"
        );
    }

    /// A block reads square on the screen rather than in cells. A cell
    /// is taller than it is wide, so a block the same count of them
    /// either way stands as a tall rectangle.
    #[test]
    fn a_block_reads_square_rather_than_standing_tall() {
        let mut pixels = ResolvingPixels::new();
        pixels.advance(AREA, Duration::ZERO);
        // A cell twice as tall as it is wide, which is about what an
        // ordinary terminal font gives.
        pixels.cell_pixels = (1, 2);

        let (columns, rows) = pixels.block_size();

        assert_eq!(
            rows,
            columns / 2,
            "a block on a cell twice as tall as it is wide wants half \
             as many rows as columns"
        );
    }

    /// Where the terminal will not say how big a cell is, a block is
    /// the same count of cells either way -- which is what it was
    /// before there was anything to scale it by.
    #[test]
    fn a_block_falls_back_to_square_in_cells() {
        let mut pixels = ResolvingPixels::new();
        pixels.advance(AREA, Duration::ZERO);
        pixels.cell_pixels = (0, 0);

        assert_eq!(
            pixels.block_size(),
            (DEFAULT_BLOCK_COLUMNS, DEFAULT_BLOCK_COLUMNS)
        );
    }

    /// The stepped sizes each halve the last, so a coarser size's
    /// boundaries are also the finer one's. Sizes that did not divide
    /// into each other would have a block re-cut itself under the
    /// colours as it stepped.
    #[test]
    fn each_stepped_size_halves_the_one_above_it() {
        let mut pixels = ResolvingPixels::new();
        pixels.advance(AREA, Duration::ZERO);
        pixels.cell_pixels = (0, 0);
        pixels.set_block_columns(MAX_BLOCK_COLUMNS);

        for level in 1..PIXEL_STEP_LEVELS {
            let (finer, _) = pixels.level_size(level - 1);
            let (coarser, _) = pixels.level_size(level);
            assert_eq!(
                coarser,
                finer * 2,
                "level {level} should be twice level {} across",
                level - 1
            );
        }
    }

    /// A sharp cell wears the desktop's own colour, whatever the
    /// resolve. The field away from the wave is the picture the wave is
    /// read against, and a resolve that touched it would leave nothing
    /// sharp on the screen.
    #[test]
    fn a_cell_the_wave_does_not_reach_wears_its_own_colour() {
        let backdrop = Backdrop::stepped(AREA);
        let far = u32::from(AREA.width) / 2;

        for resolve in [
            PixelResolve::Blend,
            PixelResolve::Step,
            PixelResolve::Scatter,
        ] {
            let mut pixels = field_with_wave_at(AT_THE_FIRST_COLUMN);
            pixels.resolve = resolve;
            let coarsened = pixels.coarsen_field(&backdrop);
            let own = backdrop.color_at(u16::try_from(far).unwrap_or(0), 0);

            assert_eq!(
                Some(pixels.color_at(&coarsened, own.unwrap_or(Color::Reset), far, 0)),
                own,
                "{resolve:?} should leave a cell the wave has not \
                 reached alone"
            );
        }
    }

    /// Where the wave has a block whole, every cell of it wears the one
    /// colour its cells averaged to. Short of that the block is
    /// somewhere between that colour and the picture.
    #[test]
    fn a_blending_block_the_wave_has_whole_wears_one_colour() {
        let backdrop = Backdrop::stepped(AREA);
        let mut pixels = field_with_wave_at(AT_THE_FIRST_COLUMN);
        let coarsened = pixels.coarsen_field(&backdrop);
        let (columns, rows) = pixels.block_size();

        // Widened until the field stands at one coarseness the whole
        // way round, which is the one state that has every cell of a
        // block as coarse as the field goes.
        pixels.wider(MAX_PIXEL_WAVE_PERCENT);

        let mean = coarsened
            .coarsest()
            .and_then(|blocks| blocks.mean_at(0, 0))
            .expect("the stepped backdrop gives every cell a colour");
        for row in 0..rows {
            for column in 0..columns {
                let own = backdrop
                    .color_at(
                        u16::try_from(column).unwrap_or(0),
                        u16::try_from(row).unwrap_or(0),
                    )
                    .expect("the stepped backdrop gives every cell a colour");
                assert_eq!(
                    pixels.color_at(&coarsened, own, column, row),
                    mean,
                    "cell {column},{row} should wear its block's colour \
                     where the wave is coarsest"
                );
            }
        }
    }

    /// A scattering block gives its cells back one at a time: two cells
    /// dealt different moments come back at different coarsenesses. A
    /// scatter whose cells all turned together would be the blend
    /// without the crossfade.
    #[test]
    fn a_scattering_block_gives_its_cells_back_one_at_a_time() {
        let backdrop = Backdrop::stepped(AREA);
        let mut pixels = field_with_wave_at(AT_THE_FIRST_COLUMN);
        pixels.resolve = PixelResolve::Scatter;
        // Two cells of the first block, dealt moments a long way apart,
        // so the coarseness that has brought one back has not yet
        // reached the other.
        pixels.grains.splice(..2, [0, u8::MAX]);
        let coarsened = pixels.coarsen_field(&backdrop);

        let first = pixels.color_at(&coarsened, Color::Rgb(0, 0, 0), 0, 0);
        let second = pixels.color_at(&coarsened, Color::Rgb(0, 0, 0), 1, 0);

        assert_ne!(
            first, second,
            "cells dealt moments at opposite ends of the range should \
             not come back together"
        );
    }

    /// Turning the wave round on the axis it is already on leaves it
    /// where it stands and sends it back the other way. Putting it back
    /// at the edge would make the arrows read as a reset.
    #[test]
    fn turning_the_wave_round_leaves_it_where_it_stands() {
        let mut pixels = field_with_wave_at(0);
        // A quarter of the way across, so neither end of the lap
        // answers by accident.
        pixels.middle = pixels.span() / 4;
        let coarse = pixels.coarse_at(u32::from(AREA.width) / 4, 0);

        pixels.set_direction(BandDirection::Left);

        assert_eq!(
            pixels.coarse_at(u32::from(AREA.width) / 4, 0),
            coarse,
            "the same column should stand as coarse after the turn as \
             before it"
        );
    }

    /// Turning the wave onto the other axis keeps its share of a lap. A
    /// lap along the rows is a different length from one along the
    /// columns, so the sub-cells it stood at mean nothing there.
    #[test]
    fn turning_the_wave_onto_the_other_axis_keeps_its_share_of_a_lap() {
        let mut pixels = field_with_wave_at(0);
        let across = pixels.span();
        pixels.middle = across / 4;

        pixels.set_direction(BandDirection::Down);

        assert_eq!(
            pixels.middle,
            pixels.span() / 4,
            "a wave a quarter of the way along the columns should stand \
             a quarter of the way down the rows"
        );
    }

    /// Every cell inside the area is painted. A cell left out is a
    /// piece of the desktop missing rather than an edge to read.
    #[test]
    fn every_cell_of_the_area_is_painted() {
        let mut pixels = ResolvingPixels::new();
        pixels.advance(AREA, Duration::ZERO);
        let mut buffer = Buffer::empty(AREA);

        pixels.render(AREA, &Backdrop::stepped(AREA), Color::Black, &mut buffer);

        for row in 0..AREA.height {
            for column in 0..AREA.width {
                let cell = buffer
                    .cell((column, row))
                    .expect("every cell of the area is in its own buffer");
                assert_ne!(
                    cell.bg,
                    Color::Reset,
                    "cell {column},{row} should have been painted"
                );
            }
        }
    }

    /// Faded the whole way out the field draws nothing at all, which is
    /// what hands the grid back the terminal it is arriving on.
    #[test]
    fn a_field_faded_out_draws_nothing() {
        let mut pixels = ResolvingPixels::new();
        pixels.advance(AREA, Duration::ZERO);
        pixels.fade(u8::MAX);
        let mut buffer = Buffer::empty(AREA);

        pixels.render(AREA, &Backdrop::stepped(AREA), Color::Black, &mut buffer);

        assert_eq!(buffer, Buffer::empty(AREA));
    }

    /// Every steering key is clamped here rather than at the call site,
    /// so an app can hand a held key straight through.
    #[test]
    fn the_steering_keys_are_clamped_at_both_ends() {
        let mut pixels = ResolvingPixels::new();

        pixels.slow_down(u32::MAX);
        assert_eq!(pixels.speed, MIN_PIXEL_SPEED);
        pixels.speed_up(u32::MAX);
        assert_eq!(pixels.speed, MAX_PIXEL_SPEED);

        pixels.sharpen(u32::MAX);
        assert_eq!(pixels.block_columns, MIN_BLOCK_COLUMNS);
        pixels.coarsen(u32::MAX);
        assert_eq!(pixels.block_columns, MAX_BLOCK_COLUMNS);

        pixels.narrower(u32::MAX);
        assert_eq!(pixels.wave_percent, MIN_PIXEL_WAVE_PERCENT);
        pixels.wider(u32::MAX);
        assert_eq!(pixels.wave_percent, MAX_PIXEL_WAVE_PERCENT);
    }

    /// Both cycles come back to where they started, so a reader who
    /// keeps pressing reaches every setting and none of them is a door
    /// that only opens one way.
    #[test]
    fn the_cycles_come_back_to_where_they_started() {
        let mut resolve = PixelResolve::default();
        for _ in 0..3_u8 {
            resolve = resolve.next();
        }
        assert_eq!(resolve, PixelResolve::default());

        assert_eq!(PixelFill::default().next().next(), PixelFill::default());
    }

    /// Settings taken from one sized field restore exactly on another
    /// field sized to the same area.
    #[test]
    fn pixel_settings_round_trip_between_sized_fields() {
        let mut source = ResolvingPixels::new();
        source.advance(AREA, Duration::ZERO);
        source.set_direction(BandDirection::Down);
        source.speed_up(17);
        source.coarsen(5);
        source.wider(13);
        source.cycle_resolve();
        source.cycle_fill();
        let settings = source.settings();
        let mut restored = ResolvingPixels::new();
        restored.advance(AREA, Duration::ZERO);

        restored.apply(settings);

        assert_eq!(restored.settings(), settings);
    }

    /// Applying direction, resolve, and fill settings to a running
    /// field has the same runtime result as the steering calls.
    #[test]
    fn applying_pixel_settings_uses_the_steering_transitions() {
        let mut starting = ResolvingPixels::new();
        starting.advance(AREA, Duration::ZERO);
        starting.set_direction(BandDirection::Left);
        starting.speed_up(17);
        starting.coarsen(5);
        starting.wider(13);
        starting.cycle_resolve();
        starting.cycle_fill();
        starting.advance(AREA, FRAME);

        for direction in [
            BandDirection::Left,
            BandDirection::Right,
            BandDirection::Up,
            BandDirection::Down,
        ] {
            for resolve in [
                PixelResolve::Blend,
                PixelResolve::Step,
                PixelResolve::Scatter,
            ] {
                for fill in [PixelFill::Solid, PixelFill::Shades] {
                    let mut settings = starting.settings();
                    settings.direction = direction;
                    settings.resolve = resolve;
                    settings.fill = fill;
                    let mut expected = starting.clone();
                    expected.set_direction(direction);
                    while expected.resolve != resolve {
                        expected.cycle_resolve();
                    }
                    if expected.fill != fill {
                        expected.cycle_fill();
                    }
                    let mut applied = starting.clone();

                    applied.apply(settings);

                    assert_eq!(
                        applied, expected,
                        "direction {direction:?}, resolve {resolve:?}, fill {fill:?}"
                    );
                }
            }
        }
    }

    /// Values outside every numeric range are normalized by
    /// [`ResolvingPixels::apply`].
    #[test]
    fn applying_pixel_settings_clamps_every_numeric_field() {
        let mut pixels = ResolvingPixels::new();

        pixels.apply(PixelSettings {
            direction:     BandDirection::Right,
            speed:         0,
            wave_percent:  0,
            block_columns: 0,
            resolve:       PixelResolve::Blend,
            fill:          PixelFill::Solid,
        });
        assert_eq!(pixels.speed, MIN_PIXEL_SPEED);
        assert_eq!(pixels.wave_percent, MIN_PIXEL_WAVE_PERCENT);
        assert_eq!(pixels.block_columns, MIN_BLOCK_COLUMNS);

        pixels.apply(PixelSettings {
            direction:     BandDirection::Right,
            speed:         u32::MAX,
            wave_percent:  u32::MAX,
            block_columns: u32::MAX,
            resolve:       PixelResolve::Blend,
            fill:          PixelFill::Solid,
        });
        assert_eq!(pixels.speed, MAX_PIXEL_SPEED);
        assert_eq!(pixels.wave_percent, MAX_PIXEL_WAVE_PERCENT);
        assert_eq!(pixels.block_columns, MAX_BLOCK_COLUMNS);
    }

    /// A seed always produces the same settings, while the fixed seed
    /// corpus varies every field and reaches every enum variant.
    #[test]
    fn random_pixel_settings_are_deterministic_and_cover_every_field() {
        let pixels = ResolvingPixels::new();
        let samples: Vec<PixelSettings> =
            (1..=512).map(|seed| pixels.random_settings(seed)).collect();
        let first = samples[0];

        assert_eq!(pixels.random_settings(41), pixels.random_settings(41));
        assert!(
            samples
                .iter()
                .any(|settings| settings.direction != first.direction)
        );
        assert!(samples.iter().any(|settings| settings.speed != first.speed));
        assert!(
            samples
                .iter()
                .any(|settings| settings.wave_percent != first.wave_percent)
        );
        assert!(
            samples
                .iter()
                .any(|settings| settings.block_columns != first.block_columns)
        );
        assert!(
            samples
                .iter()
                .any(|settings| settings.resolve != first.resolve)
        );
        assert!(samples.iter().any(|settings| settings.fill != first.fill));
        for direction in [
            BandDirection::Left,
            BandDirection::Right,
            BandDirection::Up,
            BandDirection::Down,
        ] {
            assert!(
                samples
                    .iter()
                    .any(|settings| settings.direction == direction)
            );
        }
        for resolve in [
            PixelResolve::Blend,
            PixelResolve::Step,
            PixelResolve::Scatter,
        ] {
            assert!(samples.iter().any(|settings| settings.resolve == resolve));
        }
        for fill in [PixelFill::Solid, PixelFill::Shades] {
            assert!(samples.iter().any(|settings| settings.fill == fill));
        }
    }
}
