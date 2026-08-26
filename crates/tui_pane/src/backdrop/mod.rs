//! The desktop behind the terminal window, and the attract-mode
//! animation drawn from it.
//!
//! [`Backdrop`] is what sits *behind* some rectangle of this terminal's
//! grid -- the window server composites the terminal on top of
//! everything else, so the capture excludes every window the terminal
//! owns and what is left is whatever the window is drawn over --
//! reduced to one colour per character cell. It is taken over a
//! [`Rect`] of the grid, so the same call serves one pane or the whole
//! screen.
//!
//! [`BackdropMonitor`] is what an app holds. It keeps two clocks: a
//! capture of the whole display on a worker thread and a lazy timer,
//! and the window's own position re-read every frame. Dragging the
//! window therefore moves the colours with it at the frame rate,
//! because only the offset into an image already in hand has changed.
//!
//! [`TravelingBand`] is what an app draws with the result: a lit strip
//! of characters crossing the grid, each cell wearing the colour of
//! what is behind it. Nothing here supplies a colour of its own -- the
//! ground the strip fades toward comes from the app's theme.
//!
//! Every entry point answers [`None`] or draws nothing where the
//! platform has no capture backend or the Screen Recording permission
//! was refused. This is decoration, and a refusal is a case for
//! drawing nothing rather than for an error.

mod band;
mod constants;
mod desktop;
mod monitor;
mod query;
mod random;
mod text;

pub use band::BandDirection;
pub use band::BandFraying;
pub use band::TravelingBand;
use desktop::Desktop;
use desktop::Placement;
pub use monitor::BackdropMonitor;
use ratatui::layout::Rect;
use ratatui::style::Color;
pub use text::DriftingText;
pub use text::TextDrift;
pub use text::TextFill;

/// One captured colour per character cell over some rectangle of the
/// terminal grid.
///
/// The rectangle is the one passed to
/// [`refresh`](BackdropMonitor::refresh), and
/// [`color_at`](Self::color_at) is indexed relative to it -- a backdrop
/// for one pane is addressed from that pane's own top-left, not the
/// terminal's.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Backdrop {
    /// Cells across, matching the [`Rect`] this was read for.
    width:  u16,
    /// Cells down, matching that same [`Rect`].
    height: u16,
    /// Row-major, `width * height` entries.
    colors: Vec<Color>,
}

impl Backdrop {
    /// The colour under the cell at `column`, `row`, both relative to
    /// the area this was read for.
    ///
    /// [`None`] past the edge of the area, and equally for a cell the
    /// display does not reach: a window carried off the side of a
    /// screen has cells with nothing under them at all, and the answer
    /// for those is that there is nothing to draw.
    #[must_use]
    pub fn color_at(&self, column: u16, row: u16) -> Option<Color> {
        if column >= self.width || row >= self.height {
            return None;
        }
        let index = usize::from(row) * usize::from(self.width) + usize::from(column);
        self.colors
            .get(index)
            .copied()
            .filter(|color| *color != Color::Reset)
    }

    /// Cells across.
    #[must_use]
    pub const fn width(&self) -> u16 { self.width }

    /// Cells down.
    #[must_use]
    pub const fn height(&self) -> u16 { self.height }

    /// Read `area`'s cells out of a captured display.
    ///
    /// This is the per-frame half of the work and it is deliberately
    /// only arithmetic: `placement` says which of the display's cells
    /// the terminal's own cell zero fell in this frame, so a window
    /// that has moved since the capture is read at a different offset
    /// into the same colours. Cells the display does not reach are
    /// given [`Color::Reset`], which [`color_at`](Self::color_at)
    /// reports as no colour at all.
    fn read(desktop: &Desktop, placement: Placement, area: Rect) -> Self {
        let mut colors = Vec::with_capacity(usize::from(area.width) * usize::from(area.height));
        for row in 0..area.height {
            for column in 0..area.width {
                let color = desktop
                    .color_at(
                        placement,
                        area.x.saturating_add(column),
                        area.y.saturating_add(row),
                    )
                    .unwrap_or(Color::Reset);
                colors.push(color);
            }
        }
        Self {
            width: area.width,
            height: area.height,
            colors,
        }
    }

    /// A backdrop of one flat colour over `area`, for tests that draw
    /// over one without asking the window server for anything.
    #[cfg(test)]
    fn flat(area: Rect, color: Color) -> Self {
        Self {
            width:  area.width,
            height: area.height,
            colors: vec![color; usize::from(area.width) * usize::from(area.height)],
        }
    }
}
