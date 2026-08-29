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
//! Three animations are drawn from the result, and none of them
//! supplies a colour of its own -- the ground they fade toward comes
//! from the app's theme. [`TravelingBand`] is a lit strip of characters
//! crossing the grid, [`DriftingText`] fills the window with lines of
//! them travelling at speeds of their own, and [`ResolvingPixels`]
//! draws the desktop itself with a band of coarseness sweeping across
//! it.
//!
//! A monitor keeps its last successful drawing after a capture failure
//! and reports the latest attempt through [`BackdropStatus`]. This is
//! decoration, so a failure still leaves renderers free to draw nothing
//! or reuse the last good backdrop.

mod band;
mod constants;
mod desktop;
mod monitor;
mod pixels;
mod query;
mod random;
mod text;

pub use band::BandDirection;
pub use band::BandFraying;
pub use band::BandSettings;
pub use band::TravelingBand;
use crossterm::terminal;
pub use desktop::CaptureAttemptResult;
pub use desktop::CaptureAttemptSequence;
pub use desktop::CaptureAttemptTestCase;
pub use desktop::CaptureAttemptWindowSelection;
pub use desktop::CaptureFailure;
pub use desktop::CaptureWindowSelectionMethod;
pub use desktop::CompletedCaptureAttemptDiagnostic;
use desktop::Desktop;
use desktop::Placement;
pub use desktop::TerminalWindowCandidateSource;
pub use monitor::BackdropMonitor;
pub use monitor::BackdropMonitorCaptureTestDriver;
pub use monitor::BackdropStatus;
pub use monitor::CaptureTestDriverError;
pub use monitor::LastSuccessfulCaptureWindowId;
pub use monitor::LatestCaptureAttemptWindowSelection;
pub use monitor::WindowIdentification;
pub use pixels::PixelFill;
pub use pixels::PixelResolve;
pub use pixels::PixelSettings;
pub use pixels::ResolvingPixels;
use ratatui::layout::Rect;
use ratatui::style::Color;
pub use text::DriftingText;
pub use text::TextDrift;
pub use text::TextFill;
pub use text::TextSettings;

use self::constants::PIXEL_PRECISION;

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

    /// A backdrop whose colour steps once per cell across `area`, for
    /// tests that need neighbouring cells to differ.
    ///
    /// A flat one cannot say whether a block was averaged: every
    /// average of one colour is that colour, so a block wearing its
    /// neighbours' mean and a block wearing its own reads the same.
    #[cfg(test)]
    fn stepped(area: Rect) -> Self {
        let mut colors = Vec::with_capacity(usize::from(area.width) * usize::from(area.height));
        for row in 0..area.height {
            for column in 0..area.width {
                let level = u8::try_from(usize::from(row) + usize::from(column)).unwrap_or(u8::MAX);
                colors.push(Color::Rgb(level, level, level));
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

/// How many pixels one character cell measures across and down, each
/// scaled by [`PIXEL_PRECISION`], or [`None`] where the terminal will
/// not say.
///
/// Wanted by any animation that has to hold a distance steady across
/// the two axes: a cell is taller than it is wide, so the same count of
/// them is a different distance on the screen depending on which way
/// they stack. [`TravelingBand`] reads it to keep its depth when it
/// turns, and [`ResolvingPixels`] to make a block read square.
fn cell_pixels() -> Option<(u32, u32)> {
    let size = terminal::window_size().ok()?;
    if size.width == 0 || size.height == 0 || size.columns == 0 || size.rows == 0 {
        return None;
    }
    Some((
        u32::from(size.width) * PIXEL_PRECISION / u32::from(size.columns),
        u32::from(size.height) * PIXEL_PRECISION / u32::from(size.rows),
    ))
}

/// Smoothstep across `unit`: both ends where they were, and the travel
/// between them slowest at either end and fastest in the middle.
///
/// The scale comes from the caller because the two readers hold their
/// fractions on different ones -- [`DriftingText`] interpolates its
/// lanes in a fixed point fine enough for a lane hundreds of lines
/// deep, and [`ResolvingPixels`] carries its coarseness on the same
/// byte the alpha it becomes is written on.
fn smoothstep(fraction: u32, unit: u32) -> u32 {
    let unit = u64::from(unit);
    let along = u64::from(fraction).min(unit);
    if unit == 0 {
        return 0;
    }
    let eased = along * along * (3 * unit - 2 * along) / (unit * unit);
    u32::try_from(eased).unwrap_or(fraction)
}
