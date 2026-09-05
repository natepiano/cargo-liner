//! The desktop behind this terminal window, one colour per character
//! cell, for a whole display.
//!
//! [`Desktop`] answers what is aligned under the window without
//! capturing the terminal's own output. The macOS backend captures the
//! display with the terminal application's windows excluded. The KDE
//! Wayland backend reconstructs Plasma's wallpaper at the output's
//! physical size and maps the terminal through `KWin`'s logical
//! coordinates.
//!
//! It covers a whole display rather than the window's own rectangle,
//! which is what separates the two clocks this module keeps apart:
//!
//! - [`Desktop::capture`] is a round trip to the desktop services and takes far longer than a
//!   frame, so it runs on a worker thread. A captured desktop changes with macOS window contents or
//!   when Plasma's wallpaper changes, neither of which needs frame-rate polling.
//! - [`Desktop::placement`] asks only where the window is now. It costs a fraction of a millisecond
//!   and runs every frame, so dragging the window slides the colours with it and nothing waits on a
//!   capture.

mod candidate;
mod capture_attempt;
mod platform;
#[cfg(any(target_os = "linux", target_os = "macos", test))]
mod reduction;

use std::fmt::Formatter;

use crossterm::terminal;
use ratatui::style::Color;

pub(super) use self::candidate::CaptureWindowTarget;
pub(super) use self::candidate::TerminalWindowSearchOutcome;
pub(super) use self::candidate::TitledWindow;
pub(super) use self::candidate::WindowTitle;
pub(super) use self::candidate::capture_attempt_for_test;
pub use self::capture_attempt::CaptureAttemptResult;
pub use self::capture_attempt::CaptureAttemptSequence;
pub use self::capture_attempt::CaptureAttemptTestCase;
pub use self::capture_attempt::CaptureAttemptWindowSelection;
pub use self::capture_attempt::CaptureFailure;
pub use self::capture_attempt::CaptureWindowSelectionMethod;
pub use self::capture_attempt::CompletedCaptureAttemptDiagnostic;
pub use self::capture_attempt::TerminalWindowCandidateSource;

/// What the terminal reports about its own grid.
///
/// A capture is reduced at the cell size this gives, so a change to any
/// of it leaves the stored colours the wrong size for the grid being
/// drawn -- which is what asks for a fresh capture.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Metrics {
    /// The tty's own text area, as `TIOCGWINSZ` reports it, in the
    /// display's pixels rather than the window server's points.
    ///
    /// An emulator on macOS multiplies the view it hands the tty by the
    /// backing scale factor of the window that view is drawn in, both
    /// axes alike -- iTerm2 sets `ws_xpixel` and `ws_ypixel` from
    /// `viewSize * scaleFactor` and has no path that treats the two
    /// differently. Measured on a Retina panel: a window standing 864
    /// by 1084 points, divided into 122 by 67 cells, reported as 1708
    /// by 2144, which is 854 by 1072 points once the factor of two
    /// comes out. Only [`cell_points`](Self::cell_points) may read
    /// this, and only with the scale that converts it.
    text_area: (u16, u16),
    /// How many character cells that text area is divided into, across
    /// and down.
    cells:     (u16, u16),
}

impl Metrics {
    /// Stable terminal metrics for a client crate's synchronous capture test driver.
    pub(super) const fn for_capture_test() -> Self {
        Self {
            text_area: (1, 1),
            cells:     (1, 1),
        }
    }

    /// What the terminal reports about itself right now, or [`None`]
    /// where it will not say.
    ///
    /// A terminal that reports no pixel size draws nothing rather than
    /// something taken from elsewhere. The text area is the one
    /// measurement that tells this window apart from the emulator's
    /// others; without it the window has to be picked by area instead,
    /// which reliably chooses the largest window the emulator has open
    /// rather than the one this app is drawn in.
    pub(super) fn read() -> Option<Self> {
        let size = terminal::window_size().ok()?;
        let reported = size.width > 0 && size.height > 0 && size.columns > 0 && size.rows > 0;
        reported.then_some(Self {
            text_area: (size.width, size.height),
            cells:     (size.columns, size.rows),
        })
    }

    /// One character cell, in the window server's points, given the
    /// `scale` the display carries pixels to the point at.
    ///
    /// The terminal answers in pixels and everything the answer is
    /// measured against -- window frames, display bounds, the capture
    /// itself -- is in points, so the report is divided down before the
    /// cells come out of it. Read undivided on a Retina panel the cell
    /// is twice its size in both axes and the grid the capture reduces
    /// to covers half the window. See
    /// [`text_area`](Self#structfield.text_area) for the measurement.
    #[cfg(any(target_os = "linux", target_os = "macos", test))]
    fn cell_points(self, scale: f64) -> (f64, f64) {
        (
            f64::from(self.text_area.0) / scale / f64::from(self.cells.0),
            f64::from(self.text_area.1) / scale / f64::from(self.cells.1),
        )
    }
}

/// Where a window stands, in the window server's global point space.
///
/// A rectangle of four numbers rather than the platform's own type, so
/// that the thread asking the window server where the terminal is can
/// hand the answer back without any platform handle crossing with it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Frame {
    /// The frame's top-left corner.
    pub(super) origin: (f64, f64),
    /// The frame's width and height.
    pub(super) size:   (f64, f64),
}

/// Where this terminal's character grid sits on the display a
/// [`Desktop`] covers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Placement {
    /// The column of the display's grid that the terminal's own column
    /// zero falls in. Signed, because a window dragged past the left
    /// edge puts its first column off the display.
    column: i32,
    /// The row of the display's grid that the terminal's own row zero
    /// falls in.
    row:    i32,
}

/// One display's desktop, reduced to one colour per character cell.
///
/// The grid is laid out from the display's own top-left corner rather
/// than the window's, so a window that moves is read at a different
/// offset into the same colours instead of needing a fresh capture.
pub(super) struct Desktop {
    /// The window the terminal's grid is drawn in, as the window server
    /// numbers it. Reading its position back is the one window-server
    /// call the render thread makes.
    window_id: u32,
    /// What the terminal reported when this capture was reduced.
    metrics:   Metrics,
    /// The display's top-left corner in the window server's global
    /// point space, which is what a window's position is measured
    /// against.
    origin:    (f64, f64),
    /// One character cell in the display's points, which is what turns
    /// a window's position into a column and a row.
    cell:      (f64, f64),
    /// Cells across.
    columns:   u16,
    /// Cells down.
    rows:      u16,
    /// Row-major, `columns * rows` entries.
    colors:    Vec<Color>,
}

impl Desktop {
    /// Capture the display this terminal is on and reduce it to cells
    /// of the size `metrics` describes.
    ///
    /// The failure identifies the capture stage that could not produce
    /// a desktop. The animation remains decoration, so callers may keep
    /// drawing the last successful capture while exposing that status.
    /// `capture_window_target` carries the exact window this app was found to be drawn in, or
    /// requests the terminal-window candidate heuristic. The heuristic cannot tell two windows of
    /// the same size apart.
    pub(super) fn capture(
        metrics: Metrics,
        capture_window_target: CaptureWindowTarget,
        sequence: CaptureAttemptSequence,
    ) -> CaptureAttemptResult {
        platform::capture(metrics, capture_window_target, sequence)
    }

    /// The window this capture was taken for, as the window server
    /// numbers it, so that its position can be asked for from a thread
    /// that is not holding the capture.
    pub(super) const fn window_id(&self) -> u32 { self.window_id }

    /// Where the terminal's grid sits on this display, given the frame
    /// its window was last seen standing at.
    ///
    /// Arithmetic and nothing else. Asking the window server where the
    /// window is costs a round trip that the render thread must not
    /// pay -- see [`window_frame`] -- so the question and the answer
    /// are kept apart: the frame arrives from elsewhere and this turns
    /// it into a column and a row.
    ///
    /// A window carried onto a *different* display still answers, with
    /// an offset that runs off the end of this one -- every cell of it
    /// then falls outside [`color_at`](Self::color_at) and the drawing
    /// thins away to nothing while the next capture, of the display the
    /// window has arrived on, is on its way.
    pub(super) fn placement(&self, frame: Frame) -> Option<Placement> {
        let (columns, rows) = self.metrics.cells;
        let text_area = (
            self.cell.0 * f64::from(columns),
            self.cell.1 * f64::from(rows),
        );
        // The frame is the window; the grid is the text area inside it,
        // short by whatever the emulator draws around it. Left and right
        // are even, so half of what the width leaves over is the padding
        // on one side.
        //
        // The height is measured up from the bottom rather than shared
        // out evenly. A title bar, a tab bar, a status bar -- everything
        // an emulator stacks around its grid sits above it, and every
        // one of them can be switched on and off while this is running.
        // Under the grid there is only padding, so the bottom edge is
        // the one that holds still; anchor to it and the top may be
        // whatever it likes.
        let padding = (frame.size.0 - text_area.0) / 2.0;
        let left = frame.origin.0 - self.origin.0 + padding;
        let top = frame.origin.1 - self.origin.1 + (frame.size.1 - text_area.1 - padding).max(0.0);
        let column = cell_index(left / self.cell.0)?;
        let row = cell_index(top / self.cell.1)?;
        Some(Placement { column, row })
    }

    /// What the terminal reported when this capture was reduced, for
    /// comparing against what it reports now.
    pub(super) const fn metrics(&self) -> Metrics { self.metrics }

    /// The colour behind the terminal's cell at `column`, `row`, both
    /// counted from the terminal's own top-left corner.
    ///
    /// [`None`] for a cell that is not on this display at all, which is
    /// what a window hanging over an edge leaves.
    pub(super) fn color_at(&self, placement: Placement, column: u16, row: u16) -> Option<Color> {
        let column = u16::try_from(placement.column.checked_add(i32::from(column))?).ok()?;
        let row = u16::try_from(placement.row.checked_add(i32::from(row))?).ok()?;
        if column >= self.columns || row >= self.rows {
            return None;
        }
        let index = usize::from(row) * usize::from(self.columns) + usize::from(column);
        self.colors.get(index).copied()
    }
}

/// Where a window stands now, in the window server's global point
/// space, or [`None`] where the window server no longer describes it --
/// which is what a window closed since the capture leaves.
///
/// # Cost
///
/// A round trip to the window server: a few hundred microseconds when
/// it is free, but tens of milliseconds when it is not, because the
/// process's connection to it is serial and a capture in flight is
/// ahead of this in the queue. That is why this is never called from
/// the thread that is drawing.
pub(super) fn window_frame(window: u32) -> Option<Frame> { platform::window_frame(window) }

/// Every window the emulator has open, as the window server numbers
/// and titles them.
///
/// Read before the terminal is asked to wear a marker title, so that
/// whatever it was wearing can be put back once the marker has done
/// its work.
pub(super) fn window_titles() -> Vec<TitledWindow> { platform::window_titles() }

/// Whether the emulator has a window whose title holds `marker`.
///
/// This is how one of an emulator's windows is told from another. Size
/// cannot do it -- two windows opened side by side are commonly the
/// same size to the pixel -- and neither can ownership, because every
/// window of the emulator answers to the same application. A title
/// only this process knows is unambiguous, and the terminal will wear
/// one for as long as it takes to ask.
pub(super) fn window_titled(marker: &str) -> TerminalWindowSearchOutcome {
    platform::window_titled(marker)
}

/// Whether an emulator window stands at `origin`.
///
/// This is the other way of telling one of an emulator's windows from
/// another, and the better one: rather than making the terminal wear a
/// marker and asking the window server who is wearing it -- which a
/// terminal may refuse, and which a title the reader has pinned
/// overrides -- the emulator is asked outright where its window is.
/// Position is the one thing that separates two windows the size
/// heuristic cannot, because two windows cannot stand in the same
/// place.
///
/// Near enough rather than exactly, by [`POSITION_TOLERANCE`]: an
/// emulator may report the corner of its text area where the window
/// server reports the corner of the window around it.
pub(super) fn window_at(origin: (f64, f64)) -> TerminalWindowSearchOutcome {
    platform::window_at(origin)
}

/// A distance measured in cells, as a whole number of them.
///
/// The window server measures in floating-point points, and a
/// character cell is a font advance that divides into them evenly
/// only by accident, so a cell index is arrived at as a float and
/// has to be met as one somewhere. This is that place, and the
/// rounding it does is the reason a window dragged by less than a
/// cell does not change what is drawn.
#[allow(
    clippy::cast_possible_truncation,
    reason = "rounding points to whole cells is what this converts, and \
              a float too large to be a cell index saturates to one no \
              display can hold, which the bounds check rejects"
)]
fn cell_index(cells: f64) -> Option<i32> { cells.is_finite().then(|| cells.round() as i32) }

impl std::fmt::Debug for Desktop {
    /// Without the colours, which run to tens of thousands of entries
    /// and say nothing a reader of a debug line wants.
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Desktop")
            .field("window_id", &self.window_id)
            .field("metrics", &self.metrics)
            .field("origin", &self.origin)
            .field("cell", &self.cell)
            .field("columns", &self.columns)
            .field("rows", &self.rows)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use super::*;

    /// What the probe read off a 16-inch panel: a window divided into
    /// 122 by 67 cells whose text area iTerm2 reports as 1708 by 2144.
    /// Both numbers are in the display's pixels -- the emulator scales
    /// the view it hands the tty by the window's backing scale factor,
    /// and does it to both axes alike.
    const RETINA: Metrics = Metrics {
        text_area: (1708, 2144),
        cells:     (122, 67),
    };

    /// Where that window stood, in points: the right half of the panel,
    /// under the menu bar.
    const FRAME: Frame = Frame {
        origin: (864.0, 33.0),
        size:   (864.0, 1084.0),
    };

    /// The display it stood on, in points.
    const DISPLAY: (f64, f64) = (1728.0, 1117.0);

    /// How many pixels that display carries to the point.
    const SCALE: u32 = 2;

    /// Whether two lengths are the same to within what floating point
    /// can represent, which is all an exact division can be asked for.
    fn near(left: f64, right: f64) -> bool { (left - right).abs() < f64::EPSILON }

    /// How many whole cells of `cell` points fit across `span`, which
    /// is the division `capture` reduces the display by. The platform
    /// module's own is behind `cfg(target_os = "macos")`, and the
    /// arithmetic this checks is not.
    fn grid_cells(span: f64, cell: f64) -> u16 {
        let cells = cell_index((span / cell).ceil()).expect("a finite count of cells");
        u16::try_from(cells).expect("a display holds fewer cells than a u16 counts")
    }

    #[test]
    fn the_reported_text_area_is_divided_by_the_display_scale() {
        let (width, height) = RETINA.cell_points(f64::from(SCALE));
        assert!(
            near(width, 7.0) && near(height, 16.0),
            "1708 by 2144 pixels over 122 by 67 cells is a cell of 14 \
             by 32 pixels, which on a panel carrying two pixels to the \
             point is 7 by 16 of them"
        );
    }

    #[test]
    fn a_display_carrying_one_pixel_to_the_point_divides_by_nothing() {
        let (width, height) = RETINA.cell_points(1.0);
        assert!(
            near(width, 14.0) && near(height, 32.0),
            "where a pixel is a point the report needs no converting, \
             and the cell is exactly what the terminal said"
        );
    }

    #[test]
    fn the_converted_cell_fits_the_window_it_was_measured_in() {
        let (width, height) = RETINA.cell_points(f64::from(SCALE));
        assert!(
            width * f64::from(RETINA.cells.0) <= FRAME.size.0
                && height * f64::from(RETINA.cells.1) <= FRAME.size.1,
            "the grid has to fit inside the window holding it, which is \
             the check that catches a report left in pixels -- 122 \
             cells of 14 points stand 1708 wide in an 864 point window"
        );
    }

    #[test]
    fn the_reduce_grid_covers_every_cell_the_window_has() {
        let cell = RETINA.cell_points(f64::from(SCALE));
        let desktop = Desktop {
            window_id: 0,
            metrics: RETINA,
            origin: (0.0, 0.0),
            cell,
            columns: grid_cells(DISPLAY.0, cell.0),
            rows: grid_cells(DISPLAY.1, cell.1),
            colors: Vec::new(),
        };
        let placement = desktop
            .placement(FRAME)
            .expect("the window is on the display");
        let last_column = placement.column + i32::from(RETINA.cells.0);
        let last_row = placement.row + i32::from(RETINA.cells.1);
        assert!(
            last_column <= i32::from(desktop.columns) && last_row <= i32::from(desktop.rows),
            "the grid the capture reduces to has to reach the far corner \
             of the window, or `color_at` gives back nothing past the \
             edge and the field dies where the grid stops"
        );
    }

    #[test]
    fn an_undivided_report_leaves_the_grid_short() {
        let cell = RETINA.cell_points(1.0);
        assert!(
            grid_cells(DISPLAY.1, cell.1) < RETINA.cells.1,
            "this is the defect itself: a cell twice its height divides \
             the display into 35 rows where the window alone needs 67, \
             which is the backdrop covering the full width and dying \
             just under halfway down"
        );
    }
}
