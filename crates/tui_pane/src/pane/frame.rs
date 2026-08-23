//! The single-line frame drawn around and between neighbouring panes.
//!
//! Neighbours share a border rather than each drawing their own, so a
//! boundary is one line instead of two and the point where four panes
//! meet is a junction rather than a pile of corners. That takes a pass
//! over the whole area: the glyph a cell wants depends on every pane
//! touching it, which no single pane knows.
//!
//! [`GridLines`] collects the four edges of every [`PaneFrame`] into a
//! per-cell record of which sides carry a line, then draws each cell as
//! the box-drawing glyph matching the sides it ended up with. A pane in
//! mid-flight contributes its edges displaced and cut off exactly as its
//! contents are, so a border travels with the pane it belongs to.
//!
//! A pane laid out this way does not draw its own [`Block`]: its rect
//! reaches onto the lines it shares with its neighbours, the caller
//! draws contents through [`draw_clipped`], and the frame goes down over
//! the top of them all in one pass afterwards.
//!
//! Everything written on those lines comes the same way: the title
//! through [`add_titled`](GridLines::add_titled), and anything else
//! sitting on a border -- a scroll affordance, a marker -- through
//! [`add_label`](GridLines::add_label). A pane that wrote its own would
//! be writing under whichever neighbour draws its border next; handing
//! them to the pass that owns the lines is what puts them over the top.
//!
//! A pane's own interior rules go in through
//! [`add_rule`](GridLines::add_rule) rather than being drawn separately,
//! so where a rule meets the pane's border the crossing is worked out
//! with every other crossing instead of being named by hand.
//!
//! [`Block`]: ratatui::widgets::Block

use ratatui::buffer::Buffer;
use ratatui::layout::Margin;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::symbols::line;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Widget;
use unicode_width::UnicodeWidthStr;

use super::chrome;
use super::chrome::PaneChrome;
use super::constants::BORDER_LINE_WIDTH;

/// Where one pane's box sits for a single frame.
///
/// A pane standing still is [`PaneFrame::new`] and nothing more. A pane
/// in motion is [`PaneFrame::shifted`]: drawn displaced from where it
/// was laid out, and cut off at bounds of its own, so a pane sliding out
/// of a column stops at that column's edge rather than running over its
/// neighbour.
///
/// The shift cannot be folded into the rect ahead of time. Displacing a
/// rect and intersecting it with the clip yields a smaller rect, and a
/// border drawn around that is a closed box -- a pane sliding upward out
/// of view would grow a new top edge sealing it shut. The sides are
/// worked out against the full rect and each cell is dropped
/// individually where it lands outside the clip, which is what leaves
/// the cut edge open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PaneFrame {
    /// Where the pane sits with no motion applied, its border lines
    /// included -- so it overlaps each neighbour by the line they share.
    rect:    Rect,
    /// Rows to displace `rect` by when drawing, negative for upward.
    shift:   i32,
    /// Bounds the drawn pane is cut off at.
    clip:    Rect,
    /// Whether the pane holds focus, which decides whether its contents
    /// sit on the focus tint.
    focused: bool,
}

impl PaneFrame {
    /// A pane standing still at `rect`, clipped to nothing beyond it.
    #[must_use]
    pub const fn new(rect: Rect) -> Self {
        Self {
            rect,
            shift: 0,
            clip: rect,
            focused: false,
        }
    }

    /// A pane laid out at `rect`, drawn `shift` rows away from there and
    /// cut off at `clip`.
    #[must_use]
    pub const fn shifted(rect: Rect, shift: i32, clip: Rect) -> Self {
        Self {
            rect,
            shift,
            clip,
            focused: false,
        }
    }

    /// This frame, marked as holding focus or not.
    ///
    /// Focus is carried by the frame rather than passed alongside it
    /// because everything that reads it -- the fill under the contents,
    /// the shade of the title -- is handed the frame and nothing else.
    #[must_use]
    pub const fn with_focus(self, focused: bool) -> Self { Self { focused, ..self } }

    /// Where the pane sits with no motion applied, borders included.
    #[must_use]
    pub const fn rect(self) -> Rect { self.rect }

    /// Rows the pane is displaced by when drawn, negative for upward.
    #[must_use]
    pub const fn shift(self) -> i32 { self.shift }

    /// The bounds the drawn pane is cut off at.
    #[must_use]
    pub const fn clip(self) -> Rect { self.clip }

    /// Whether the pane holds focus.
    #[must_use]
    pub const fn is_focused(self) -> bool { self.focused }

    /// The rect inside the pane's borders, which is where its contents
    /// go.
    #[must_use]
    pub const fn inner(self) -> Rect { frame_inner(self.rect) }

    /// Whether the pane is drawn exactly where it was laid out.
    const fn is_still(self) -> bool { self.shift == 0 && within(self.rect, self.clip) }

    /// Where the row at `y` lands once the shift is applied, or `None`
    /// when that carries it off the buffer entirely.
    fn shifted_row(self, y: u16) -> Option<u16> {
        i32::from(y)
            .checked_add(self.shift)
            .and_then(|row| u16::try_from(row).ok())
    }
}

/// Grow `rect` onto the border it shares with the neighbour below and
/// the one to its right, so each boundary is one line drawn by both
/// rather than two lines drawn side by side.
///
/// A rect already ending at `area`'s edge has no neighbour there and is
/// left alone. Applied to every rect a [`Layout`] split produces, this
/// turns a flush tiling into an overlapping one -- which is what
/// [`GridLines`] expects.
///
/// [`Layout`]: ratatui::layout::Layout
#[must_use]
pub const fn share_borders(rect: Rect, area: Rect) -> Rect {
    Rect {
        width: if rect.right() < area.right() {
            rect.width.saturating_add(BORDER_LINE_WIDTH)
        } else {
            rect.width
        },
        height: if rect.bottom() < area.bottom() {
            rect.height.saturating_add(BORDER_LINE_WIDTH)
        } else {
            rect.height
        },
        ..rect
    }
}

/// The rect inside a pane's borders, which is where its contents go.
///
/// The counterpart of [`share_borders`] for a pane drawn into a shared
/// frame: the pane is handed a rect that reaches onto the lines it
/// shares, and this is what is left once those lines are taken off.
#[must_use]
pub const fn frame_inner(rect: Rect) -> Rect {
    rect.inner(Margin::new(BORDER_LINE_WIDTH, BORDER_LINE_WIDTH))
}

/// Draw a pane's contents through `draw`, displaced and cut off the way
/// its [`PaneFrame`] says.
///
/// A pane standing still inside its clip goes straight to `buffer`,
/// which is both the common case and the only path a pane that never
/// moves takes. A pane in mid-flight is drawn into a scratch buffer
/// instead, and only the rows still inside its clip are copied across --
/// that is what makes a pane leaving a column vanish at the column's
/// edge rather than running over its neighbour.
///
/// `draw` is handed the pane's [`inner`](PaneFrame::inner) rect and the
/// buffer to draw into, which is the scratch one when the pane is
/// moving. No border is drawn here; [`GridLines`] lays every line down
/// afterwards.
pub fn draw_clipped(buffer: &mut Buffer, frame: PaneFrame, draw: impl FnOnce(&mut Buffer, Rect)) {
    let inner = frame.inner();
    if inner.is_empty() {
        return;
    }
    if frame.is_still() {
        fill_pane(buffer, frame, inner);
        draw(buffer, inner);
        return;
    }
    let mut scratch = Buffer::empty(frame.rect);
    fill_pane(&mut scratch, frame, inner);
    draw(&mut scratch, inner);
    blit(buffer, &scratch, frame);
}

/// Lay a pane's tint down under its contents.
///
/// A pane drawn this way has no [`Block`] to carry the tint as its own
/// background, so it goes down first instead. What the pane draws over
/// it keeps it: a cell's style is patched rather than replaced, so a
/// span setting only a foreground leaves the tint showing through.
///
/// Only [`inner`](PaneFrame::inner) is filled, so the tint stops
/// inside the ring of border cells and the lines [`GridLines`] draws
/// keep the terminal's own background. A border is a cell two panes
/// share; tinting it would hand that cell to whichever pane drew last.
///
/// [`Block`]: ratatui::widgets::Block
fn fill_pane(buffer: &mut Buffer, frame: PaneFrame, inner: Rect) {
    if let Some(fill) = chrome::pane_fill(frame.focused) {
        buffer.set_style(inner, fill);
    }
}

/// Copy a drawn pane onto `target` displaced by its shift, keeping only
/// what lands inside its clip.
fn blit(target: &mut Buffer, scratch: &Buffer, frame: PaneFrame) {
    let (rect, clip) = (frame.rect, frame.clip);
    for y in clip.top()..clip.bottom() {
        let Some(source) = i32::from(y)
            .checked_sub(frame.shift)
            .and_then(|row| u16::try_from(row).ok())
        else {
            continue;
        };
        if source < rect.top() || source >= rect.bottom() {
            continue;
        }
        for x in rect.left().max(clip.left())..rect.right().min(clip.right()) {
            target[(x, y)] = scratch[(x, source)].clone();
        }
    }
}

/// One of the four directions a line can leave a cell in.
#[derive(Clone, Copy)]
enum Side {
    /// On toward the row above.
    Up,
    /// On toward the next column.
    Right,
    /// On toward the row below.
    Down,
    /// On toward the previous column.
    Left,
}

impl Side {
    /// The bit this side holds in a [`Sides`] set.
    const fn bit(self) -> u8 {
        match self {
            Self::Up => 1,
            Self::Right => 2,
            Self::Down => 4,
            Self::Left => 8,
        }
    }
}

/// The sides of one cell that carry a line.
#[derive(Clone, Copy)]
struct Sides(u8);

impl Sides {
    /// A cell nothing runs through.
    const fn none() -> Self { Self(0) }

    /// This set with `side` added.
    const fn with(self, side: Side) -> Self { Self(self.0 | side.bit()) }

    /// Every side either set holds.
    const fn merge(self, other: Self) -> Self { Self(self.0 | other.0) }

    /// Whether `side` carries a line.
    const fn has(self, side: Side) -> bool { self.0 & side.bit() != 0 }
}

/// What one cell of the grid ended up holding.
#[derive(Clone, Copy)]
struct GridCell {
    /// The sides a line leaves this cell by.
    sides:   Sides,
    /// Whether a focused pane put any of them there.
    ///
    /// Read only under [`PaneBorders::Separate`], where a cell belongs
    /// to exactly one pane and lighting it takes nothing from anybody.
    focused: bool,
}

impl GridCell {
    /// A cell nothing reaches.
    const fn empty() -> Self {
        Self {
            sides:   Sides::none(),
            focused: false,
        }
    }
}

/// One piece of text a pane wants written on the lines around it.
#[derive(Clone, Debug)]
pub struct PaneFrameLabel {
    /// Where it goes -- a row of a border line, most of the time.
    pub area:  Rect,
    /// The text.
    pub text:  String,
    /// The shade to write it in.
    pub style: Style,
}

/// Text held back until every line is down, so it lands over them.
struct Overlay {
    /// The pane it belongs to, so it travels, clips and takes its shade
    /// with the pane rather than with the grid.
    frame: PaneFrame,
    /// Where it goes, before the pane's shift is applied.
    rect:  Rect,
    /// The text.
    text:  String,
    /// The shade to write it in, or `None` for the chrome's title style
    /// under the pane's own focus -- which is what a title takes.
    style: Option<Style>,
}

/// Whether neighbouring panes share the cells their borders fall on.
///
/// The two apps built on this framework want opposite answers, and the
/// answer decides more than the glyphs: a cell one pane owns outright
/// can carry that pane's focus, while a cell two panes share cannot,
/// because lighting it for one of them dims the boundary for the other.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PaneBorders {
    /// Neighbours meet on one line and the grid reads as a single
    /// lattice. Focus is carried by the background tint alone.
    Shared,
    /// Every pane closes its own box, with its neighbour's line beside
    /// rather than under it. The focused pane's border lights up.
    Separate,
}

impl PaneBorders {
    /// Whether a focused pane's border takes the lit shade.
    #[must_use]
    pub const fn lights_focused_border(self) -> bool { matches!(self, Self::Separate) }

    /// A pane's drawn rect under this layout: reaching one line onto
    /// the neighbours below and to the right of it when they share, and
    /// left exactly as the layout resolved it when they do not.
    #[must_use]
    pub const fn pane_area(self, rect: Rect, area: Rect) -> Rect {
        match self {
            Self::Shared => share_borders(rect, area),
            Self::Separate => rect,
        }
    }
}

/// The lines every pane in one rect contributes, and the titles that go
/// over them.
pub struct GridLines {
    /// The rect the grid fills, and what `cells` is indexed against.
    area:   Rect,
    /// One entry per cell of `area`, row-major from its top left.
    cells:  Vec<GridCell>,
    /// Pane and rule titles, written over the finished lines exactly
    /// where they were placed.
    titles: Vec<Overlay>,
    /// Everything else held back for the lines -- pagers and the like --
    /// written after the titles, around them.
    labels: Vec<Overlay>,
}

impl GridLines {
    /// A frame over `area` with nothing drawn in it yet.
    #[must_use]
    pub fn new(area: Rect) -> Self {
        Self {
            area,
            cells: vec![GridCell::empty(); usize::from(area.width) * usize::from(area.height)],
            titles: Vec::new(),
            labels: Vec::new(),
        }
    }

    /// Add one pane's four edges, displaced and cut off the way the pane
    /// itself is drawn.
    ///
    /// Edges are added rather than assigned, so where two panes share a
    /// border the second one asks for nothing the first has not already
    /// put there and the boundary stays one line wide.
    ///
    /// Focus never enters into it. Every line draws in one shade, so a
    /// cell two panes share belongs to the boundary rather than to
    /// either side of it, and the grid comes out as one lattice.
    pub fn add(&mut self, frame: PaneFrame) {
        let rect = frame.rect;
        if rect.is_empty() {
            return;
        }
        let (left, right) = (rect.left(), rect.right().saturating_sub(1));
        let (top, bottom) = (rect.top(), rect.bottom().saturating_sub(1));
        for x in left..=right {
            let sides = run(x, left, right, Side::Left, Side::Right);
            self.mark(frame, x, top, sides);
            self.mark(frame, x, bottom, sides);
        }
        for y in top..=bottom {
            let sides = run(y, top, bottom, Side::Up, Side::Down);
            self.mark(frame, left, y, sides);
            self.mark(frame, right, y, sides);
        }
    }

    /// Add one pane's edges the way [`add`](Self::add) does, and hold
    /// its title to be written over the top border afterwards.
    ///
    /// The title goes where a ratatui [`Block`] puts a left-aligned one:
    /// the top border line, inset a cell at each end so it clears the
    /// corners.
    ///
    /// [`Block`]: ratatui::widgets::Block
    pub fn add_titled(&mut self, frame: PaneFrame, title: impl Into<String>) {
        self.add(frame);
        let rect = frame.rect;
        self.titles.push(Overlay {
            frame,
            rect: Rect {
                x:      rect.left().saturating_add(BORDER_LINE_WIDTH),
                y:      rect.top(),
                width:  rect
                    .width
                    .saturating_sub(BORDER_LINE_WIDTH.saturating_mul(2)),
                height: 1,
            },
            text: title.into(),
            style: None,
        });
    }

    /// Add one rule running inside a pane -- a column divider, or a line
    /// splitting the pane in two.
    ///
    /// The segment joins the pass the pane's own edges went into, so
    /// where it meets a border the crossing works itself out along with
    /// every other crossing: a rule reaching the top line draws a `T`
    /// there and one reaching both draws its mirror at the bottom,
    /// without the caller naming either glyph.
    pub fn add_rule(&mut self, frame: PaneFrame, rect: Rect) {
        if rect.is_empty() {
            return;
        }
        let (left, right) = (rect.left(), rect.right().saturating_sub(1));
        let (top, bottom) = (rect.top(), rect.bottom().saturating_sub(1));
        for y in top..=bottom {
            for x in left..=right {
                let sides = run(y, top, bottom, Side::Up, Side::Down).merge(run(
                    x,
                    left,
                    right,
                    Side::Left,
                    Side::Right,
                ));
                self.mark(frame, x, y, sides);
            }
        }
    }

    /// Hold one label to be written over the finished lines.
    ///
    /// This is how anything that sits on a border survives the pass: a
    /// pane drawing it itself would have whichever neighbour draws that
    /// line next come down over the top of it.
    ///
    /// Labels go down after every title, and around them: a border a pane
    /// shares with the one below carries that pane's title too, so a
    /// label landing on an already-written stretch slides along the line
    /// to the nearest clear one rather than covering it.
    pub fn add_label(&mut self, frame: PaneFrame, label: PaneFrameLabel) {
        self.labels.push(Overlay {
            frame,
            rect: label.area,
            text: label.text,
            style: Some(label.style),
        });
    }

    /// Draw every cell a line reaches, then write the titles over them
    /// and fit the labels around what the titles took.
    ///
    /// `borders` decides each line's shade. Under
    /// [`PaneBorders::Separate`] a cell belongs to one pane alone, so a
    /// focused pane lights its whole box. Under
    /// [`PaneBorders::Shared`] every line draws in `chrome`'s inactive
    /// style instead: a border is then a cell two panes share, and
    /// lighting it for the focused one takes the boundary away from the
    /// other, so focus is left to the background tint under the pane's
    /// contents.
    pub fn render(&self, buffer: &mut Buffer, chrome: PaneChrome, borders: PaneBorders) {
        let focused_line = focus_tinted(chrome.active_border);
        for y in self.area.top()..self.area.bottom() {
            for x in self.area.left()..self.area.right() {
                let Some(cell) = self.cells.get(self.index(x, y)).copied() else {
                    continue;
                };
                let Some(glyph) = glyph(cell.sides) else {
                    continue;
                };
                let lit = borders.lights_focused_border() && cell.focused;
                buffer[(x, y)].set_symbol(glyph).set_style(if lit {
                    focused_line
                } else {
                    chrome.inactive_border
                });
            }
        }
        let mut written = vec![false; self.cells.len()];
        for overlay in &self.titles {
            if let Some(row) = self.overlay_row(overlay) {
                Self::write_overlay(buffer, chrome, overlay, row);
                self.mark_written(&mut written, Self::written_row(overlay, row));
            }
        }
        for overlay in &self.labels {
            let Some(row) = self.overlay_row(overlay) else {
                continue;
            };
            let row = Self::written_row(overlay, row);
            if let Some(row) = self.clear_run(&written, overlay.frame, row) {
                Self::write_overlay(buffer, chrome, overlay, row);
                self.mark_written(&mut written, row);
            }
        }
    }

    /// The single row a held-back overlay lands on, once its pane's shift
    /// has moved it and its clip has cut it down. `None` when nothing of
    /// it is left on screen.
    fn overlay_row(&self, overlay: &Overlay) -> Option<Rect> {
        let y = overlay.frame.shifted_row(overlay.rect.top())?;
        let row = Rect {
            y,
            height: 1,
            ..overlay.rect
        }
        .intersection(overlay.frame.clip)
        .intersection(self.area);
        (!row.is_empty()).then_some(row)
    }

    /// Where a label fits on its row: its own place when nothing has
    /// been written there, otherwise the nearest run of untouched cells
    /// wide enough for it, anywhere along the pane's own span. `None`
    /// when the row has no such run left -- the line is full, and a
    /// pager is worth less there than the titles it would cover.
    fn clear_run(&self, written: &[bool], frame: PaneFrame, row: Rect) -> Option<Rect> {
        let span = Rect {
            y: row.y,
            height: 1,
            ..frame.rect
        }
        .intersection(frame.clip)
        .intersection(self.area);
        let last = span.right().checked_sub(row.width)?;
        (span.left()..=last)
            .filter(|&x| {
                (x..x.saturating_add(row.width)).all(|cell| {
                    !written
                        .get(self.index(cell, row.y))
                        .copied()
                        .unwrap_or(true)
                })
            })
            .min_by_key(|&x| x.abs_diff(row.x))
            .map(|x| Rect { x, ..row })
    }

    /// The cells an overlay's text actually covers, which is narrower than
    /// the rect it was given whenever the text falls short of it -- a
    /// title is handed the whole top border to sit on and uses a few
    /// cells of it, and the rest of that line stays open for a label.
    fn written_row(overlay: &Overlay, row: Rect) -> Rect {
        let text = u16::try_from(overlay.text.width()).unwrap_or(u16::MAX);
        Rect {
            width: text.min(row.width),
            ..row
        }
    }

    /// Note every cell an overlay just covered, so the next one placed on
    /// that row goes around it.
    fn mark_written(&self, written: &mut [bool], row: Rect) {
        for x in row.left()..row.right() {
            if let Some(cell) = written.get_mut(self.index(x, row.y)) {
                *cell = true;
            }
        }
    }

    /// Write one held-back piece of text over the finished lines.
    fn write_overlay(buffer: &mut Buffer, chrome: PaneChrome, overlay: &Overlay, row: Rect) {
        let focused = overlay.frame.focused;
        let style = overlay.style.unwrap_or_else(|| chrome.title_style(focused));
        Line::from(Span::styled(overlay.text.as_str(), style)).render(row, buffer);
    }

    /// Union `sides` into the cell the pane puts at (`x`, `y`).
    fn mark(&mut self, frame: PaneFrame, x: u16, y: u16, sides: Sides) {
        let focused = frame.focused;
        if let Some(cell) = self.cell_at(frame, x, y) {
            cell.sides = cell.sides.merge(sides);
            cell.focused |= focused;
        }
    }

    /// The cell the pane puts at (`x`, `y`) -- where its shift moves
    /// that cell to, and `None` at all when the shift carries it out of
    /// the pane's clip or off the grid.
    fn cell_at(&mut self, frame: PaneFrame, x: u16, y: u16) -> Option<&mut GridCell> {
        let y = frame.shifted_row(y)?;
        if !holds(frame.clip, x, y) || !holds(self.area, x, y) {
            return None;
        }
        let index = self.index(x, y);
        self.cells.get_mut(index)
    }

    /// Where the cell at (`x`, `y`) sits in `cells`.
    fn index(&self, x: u16, y: u16) -> usize {
        usize::from(y.saturating_sub(self.area.y)) * usize::from(self.area.width)
            + usize::from(x.saturating_sub(self.area.x))
    }
}

/// The sides a cell at `at` contributes to a run of line from `start` to
/// `end`: it reaches back toward `start` unless it is the start, and on
/// toward `end` unless it is the end.
///
/// A cell in the middle of a run therefore asks for both and draws as
/// the run's own axis, while the two ends ask for one side each and pick
/// up their second from whichever edge crosses them.
const fn run(at: u16, start: u16, end: u16, back: Side, on: Side) -> Sides {
    let mut sides = Sides::none();
    if at > start {
        sides = sides.with(back);
    }
    if at < end {
        sides = sides.with(on);
    }
    sides
}

/// The box-drawing glyph for a cell, or `None` when nothing reaches it.
const fn glyph(sides: Sides) -> Option<&'static str> {
    let up = sides.has(Side::Up);
    let right = sides.has(Side::Right);
    let down = sides.has(Side::Down);
    let left = sides.has(Side::Left);
    match (up, right, down, left) {
        (false, false, false, false) => None,
        // A run passing straight through, plus the one-sided stubs a
        // clipped pane leaves behind, which read as their own axis.
        (_, false, true, false) | (true, false, false, false) => Some(line::VERTICAL),
        (false, true, false, _) | (false, false, false, true) => Some(line::HORIZONTAL),
        (false, true, true, false) => Some(line::TOP_LEFT),
        (false, false, true, true) => Some(line::TOP_RIGHT),
        (true, true, false, false) => Some(line::BOTTOM_LEFT),
        (true, false, false, true) => Some(line::BOTTOM_RIGHT),
        (true, true, true, false) => Some(line::VERTICAL_RIGHT),
        (true, false, true, true) => Some(line::VERTICAL_LEFT),
        (false, true, true, true) => Some(line::HORIZONTAL_DOWN),
        (true, true, false, true) => Some(line::HORIZONTAL_UP),
        (true, true, true, true) => Some(line::CROSS),
    }
}

/// `base` laid over the focus tint, when the tint is on.
///
/// A focused pane's contents sit on the tint, so its border sits on it
/// too rather than on a strip of bare terminal around them.
fn focus_tinted(base: Style) -> Style {
    chrome::pane_fill(true).map_or(base, |fill| fill.patch(base))
}

/// Whether (`x`, `y`) falls inside `rect`.
const fn holds(rect: Rect, x: u16, y: u16) -> bool {
    x >= rect.x && x < rect.right() && y >= rect.y && y < rect.bottom()
}

/// Whether `inner` sits entirely inside `outer`.
const fn within(inner: Rect, outer: Rect) -> bool {
    inner.x >= outer.x
        && inner.y >= outer.y
        && inner.right() <= outer.right()
        && inner.bottom() <= outer.bottom()
}

#[cfg(test)]
mod tests {
    use ratatui::style::Color;

    use super::*;

    /// Border and title shades a test can tell apart.
    fn chrome() -> PaneChrome {
        PaneChrome {
            active_border:   Style::default().fg(Color::Green),
            inactive_border: Style::default().fg(Color::Red),
            active_title:    Style::default().fg(Color::Green),
            inactive_title:  Style::default().fg(Color::Red),
        }
    }

    /// Frame `frames` over `area` and hand back what they drew into.
    fn framed(area: Rect, frames: &[PaneFrame]) -> Buffer {
        let mut grid_lines = GridLines::new(area);
        for &frame in frames {
            grid_lines.add(frame);
        }
        let mut buffer = Buffer::empty(area);
        grid_lines.render(&mut buffer, chrome(), PaneBorders::Shared);
        buffer
    }

    /// The same, for panes that each close their own box.
    fn framed_separately(area: Rect, frames: &[PaneFrame]) -> Buffer {
        let mut grid_lines = GridLines::new(area);
        for &frame in frames {
            grid_lines.add(frame);
        }
        let mut buffer = Buffer::empty(area);
        grid_lines.render(&mut buffer, chrome(), PaneBorders::Separate);
        buffer
    }

    /// A buffer read back a row at a time, so a test states the picture
    /// it expects.
    fn rows(buffer: &Buffer, area: Rect) -> Vec<String> {
        (area.top()..area.bottom())
            .map(|y| {
                (area.left()..area.right())
                    .map(|x| buffer[(x, y)].symbol())
                    .collect()
            })
            .collect()
    }

    /// [`framed`] read back as a picture.
    fn rendered(area: Rect, frames: &[PaneFrame]) -> Vec<String> {
        rows(&framed(area, frames), area)
    }

    /// [`rendered`] for panes that stand still.
    fn boxed(area: Rect, rects: &[Rect]) -> Vec<String> {
        let frames: Vec<PaneFrame> = rects.iter().copied().map(PaneFrame::new).collect();
        rendered(area, &frames)
    }

    /// One titled pane, read back as a picture.
    fn titled(area: Rect, frame: PaneFrame, title: &str) -> Vec<String> {
        let mut grid_lines = GridLines::new(area);
        grid_lines.add_titled(frame, title);
        let mut buffer = Buffer::empty(area);
        grid_lines.render(&mut buffer, chrome(), PaneBorders::Shared);
        rows(&buffer, area)
    }

    #[test]
    fn one_pane_draws_a_plain_box() {
        assert_eq!(
            boxed(Rect::new(0, 0, 4, 3), &[Rect::new(0, 0, 4, 3)]),
            ["┌──┐", "│  │", "└──┘"]
        );
    }

    /// The two rects overlap by a column, which is what a shared border
    /// is: both panes own `x = 3`, and it draws once.
    #[test]
    fn side_by_side_panes_share_one_border() {
        assert_eq!(
            boxed(
                Rect::new(0, 0, 7, 3),
                &[Rect::new(0, 0, 4, 3), Rect::new(3, 0, 4, 3)]
            ),
            ["┌──┬──┐", "│  │  │", "└──┴──┘"]
        );
    }

    #[test]
    fn stacked_panes_share_one_border() {
        assert_eq!(
            boxed(
                Rect::new(0, 0, 4, 5),
                &[Rect::new(0, 0, 4, 3), Rect::new(0, 2, 4, 3)]
            ),
            ["┌──┐", "│  │", "├──┤", "│  │", "└──┘"]
        );
    }

    /// The case the whole pass exists for: four panes meeting draw one
    /// crossing, not four corners piled on the same cell.
    #[test]
    fn four_panes_meeting_draw_a_crossing() {
        let quad = [
            Rect::new(0, 0, 4, 3),
            Rect::new(3, 0, 4, 3),
            Rect::new(0, 2, 4, 3),
            Rect::new(3, 2, 4, 3),
        ];
        assert_eq!(
            boxed(Rect::new(0, 0, 7, 5), &quad),
            ["┌──┬──┐", "│  │  │", "├──┼──┤", "│  │  │", "└──┴──┘"]
        );
    }

    /// A pane shifted up past its clip leaves only the part still inside
    /// it, which is how a pane crossing columns stops at the edge.
    #[test]
    fn a_shifted_pane_is_cut_off_at_its_clip() {
        let area = Rect::new(0, 0, 4, 4);
        assert_eq!(
            rendered(area, &[PaneFrame::shifted(Rect::new(0, 0, 4, 3), -2, area)]),
            ["└──┘", "    ", "    ", "    "]
        );
    }

    /// The flush rects a `Layout` split hands back become overlapping
    /// ones, so the boundary between them is a single shared line.
    #[test]
    fn sharing_borders_grows_a_rect_onto_its_neighbours() {
        let area = Rect::new(0, 0, 10, 10);
        assert_eq!(
            share_borders(Rect::new(0, 0, 5, 5), area),
            Rect::new(0, 0, 6, 6)
        );
    }

    /// A rect against the area's own edge has no neighbour to share
    /// with, so it keeps the border it draws itself.
    #[test]
    fn sharing_borders_leaves_the_outer_edges_alone() {
        let area = Rect::new(0, 0, 10, 10);
        assert_eq!(
            share_borders(Rect::new(5, 5, 5, 5), area),
            Rect::new(5, 5, 5, 5)
        );
    }

    /// Focus leaves the lattice alone. The shared border stays the pair
    /// of `T`s the boundary really is rather than closing into the
    /// focused pane's own corners, so the frame around the grid carries
    /// on through them unbroken.
    #[test]
    fn focus_does_not_close_a_shared_border() {
        assert_eq!(
            rendered(
                Rect::new(0, 0, 4, 5),
                &[
                    PaneFrame::new(Rect::new(0, 0, 4, 3)).with_focus(true),
                    PaneFrame::new(Rect::new(0, 2, 4, 3)),
                ]
            ),
            [
                "\u{250c}\u{2500}\u{2500}\u{2510}",
                "\u{2502}  \u{2502}",
                "\u{251c}\u{2500}\u{2500}\u{2524}",
                "\u{2502}  \u{2502}",
                "\u{2514}\u{2500}\u{2500}\u{2518}"
            ]
        );
    }

    /// Where four panes meet, focus leaves the crossing a crossing. A
    /// focused pane closing its corner there would take half an arm off
    /// each of the two neighbours past it.
    #[test]
    fn focus_does_not_close_a_crossing() {
        let quad = [
            PaneFrame::new(Rect::new(0, 0, 4, 3)).with_focus(true),
            PaneFrame::new(Rect::new(3, 0, 4, 3)),
            PaneFrame::new(Rect::new(0, 2, 4, 3)),
            PaneFrame::new(Rect::new(3, 2, 4, 3)),
        ];
        assert_eq!(
            rendered(Rect::new(0, 0, 7, 5), &quad)[2],
            "\u{251c}\u{2500}\u{2500}\u{253c}\u{2500}\u{2500}\u{2524}",
            "the crossing stays a crossing and both edges stay Ts"
        );
    }

    /// Panes that do not share their border cells: the focused one owns
    /// every cell of its own box, so the whole box lights up and the
    /// neighbour beside it stays dim.
    #[test]
    fn a_separate_focused_pane_lights_its_whole_box() {
        let buffer = framed_separately(
            Rect::new(0, 0, 8, 3),
            &[
                PaneFrame::new(Rect::new(0, 0, 4, 3)).with_focus(true),
                PaneFrame::new(Rect::new(4, 0, 4, 3)),
            ],
        );
        for cell in [(0, 0), (3, 0), (0, 1), (3, 1), (0, 2), (3, 2)] {
            assert_eq!(
                buffer[cell].fg,
                Color::Green,
                "the focused pane lights {cell:?}, a cell of its own box"
            );
        }
        assert_eq!(
            buffer[(4, 1)].fg,
            Color::Red,
            "the neighbour's own left edge stays unfocused"
        );
    }

    /// The same two panes sharing a border cell instead: focus does not
    /// reach the lines at all, because the cell between them belongs to
    /// the boundary rather than to either side of it.
    #[test]
    fn a_shared_border_ignores_focus() {
        let buffer = framed(
            Rect::new(0, 0, 7, 3),
            &[
                PaneFrame::new(Rect::new(0, 0, 4, 3)).with_focus(true),
                PaneFrame::new(Rect::new(3, 0, 4, 3)),
            ],
        );
        assert_eq!(buffer[(3, 1)].fg, Color::Red, "the shared line stays dim");
        assert_eq!(
            buffer[(0, 1)].fg,
            Color::Red,
            "so does the focused pane's far side"
        );
    }

    /// The title lands on the top border line one cell in from the
    /// corner, and the line carries on past its end.
    #[test]
    fn a_title_is_written_over_the_top_border() {
        let area = Rect::new(0, 0, 8, 3);
        assert_eq!(titled(area, PaneFrame::new(area), " Git ")[0], "┌ Git ─┐");
    }

    /// A title wider than the pane is cut off inside the corner rather
    /// than writing over it.
    #[test]
    fn a_title_stops_inside_the_corner() {
        let area = Rect::new(0, 0, 6, 3);
        assert_eq!(titled(area, PaneFrame::new(area), " Targets ")[0], "┌ Tar┐");
    }

    /// A title travels with the pane it belongs to, landing on whichever
    /// row the pane's top edge ends up on.
    #[test]
    fn a_shifted_pane_carries_its_title_with_it() {
        let area = Rect::new(0, 0, 8, 4);
        let frame = PaneFrame::shifted(Rect::new(0, 0, 8, 3), 1, area);
        let picture = titled(area, frame, " Git ");
        assert_eq!(picture[0], "        ", "the row the pane left stays empty");
        assert_eq!(picture[1], "┌ Git ─┐");
    }

    /// The still path draws straight through to the target buffer, at
    /// the rect inside the pane's borders.
    #[test]
    fn a_still_pane_draws_at_its_inner_rect() {
        let area = Rect::new(0, 0, 6, 3);
        let mut buffer = Buffer::empty(area);
        draw_clipped(&mut buffer, PaneFrame::new(area), |target, inner| {
            assert_eq!(inner, Rect::new(1, 1, 4, 1));
            target[(inner.x, inner.y)].set_symbol("x");
        });
        assert_eq!(buffer[(1, 1)].symbol(), "x");
    }

    /// A pane drawn displaced lands its contents at the shifted row, and
    /// keeps nothing that falls outside the clip.
    #[test]
    fn a_shifted_pane_keeps_only_what_lands_in_its_clip() {
        let area = Rect::new(0, 0, 6, 4);
        let clip = Rect::new(0, 2, 6, 2);
        let mut buffer = Buffer::empty(area);
        draw_clipped(
            &mut buffer,
            PaneFrame::shifted(Rect::new(0, 0, 6, 3), 2, clip),
            |target, inner| {
                target[(inner.x, inner.y)].set_symbol("x");
            },
        );
        assert_eq!(buffer[(1, 1)].symbol(), " ");
        assert_eq!(buffer[(1, 3)].symbol(), "x");
    }
}
