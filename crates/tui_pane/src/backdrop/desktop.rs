//! The desktop behind this terminal window, one colour per character
//! cell, for a whole display.
//!
//! [`Desktop`] answers what sits *behind* the window rather than what
//! is on the screen: the window server composites this terminal on top
//! of everything else, so a plain screenshot would hand back the app's
//! own output and the attract animation would be drawing itself. The
//! capture excludes every window the terminal owns, and what is left is
//! whatever the window is drawn over -- the wallpaper where nothing
//! else is there, and another application's window where one is.
//!
//! It covers a whole display rather than the window's own rectangle,
//! which is what separates the two clocks this module keeps apart:
//!
//! - [`Desktop::capture`] is a round trip to the window server and takes far longer than a frame,
//!   so it runs on a worker thread. What makes it stale is the desktop behind the window changing
//!   -- another window opening there, a Space switch, the wallpaper turning over -- and none of
//!   that happens at anything like the frame rate.
//! - [`Desktop::placement`] asks only where the window is now. It costs a fraction of a millisecond
//!   and runs every frame, so dragging the window slides the colours with it and nothing waits on a
//!   capture.

use std::fmt::Formatter;

use crossterm::terminal;
use ratatui::style::Color;

/// What the terminal reports about its own grid.
///
/// A capture is reduced at the cell size this gives, so a change to any
/// of it leaves the stored colours the wrong size for the grid being
/// drawn -- which is what asks for a fresh capture.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Metrics {
    /// The tty's own text area in pixels, as `TIOCGWINSZ` reports it.
    text_pixels: (u16, u16),
    /// How many character cells that text area is divided into, across
    /// and down.
    cells:       (u16, u16),
}

impl Metrics {
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
            text_pixels: (size.width, size.height),
            cells:       (size.columns, size.rows),
        })
    }

    /// One character cell, in pixels.
    fn cell_pixels(self) -> (f64, f64) {
        (
            f64::from(self.text_pixels.0) / f64::from(self.cells.0),
            f64::from(self.text_pixels.1) / f64::from(self.cells.1),
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
    window:  u32,
    /// What the terminal reported when this capture was reduced.
    metrics: Metrics,
    /// The display's top-left corner in the window server's global
    /// point space, which is what a window's position is measured
    /// against.
    origin:  (f64, f64),
    /// One character cell in the display's points, which is what turns
    /// a window's position into a column and a row.
    cell:    (f64, f64),
    /// Cells across.
    columns: u16,
    /// Cells down.
    rows:    u16,
    /// Row-major, `columns * rows` entries.
    colors:  Vec<Color>,
}

impl Desktop {
    /// Capture the display this terminal is on and reduce it to cells
    /// of the size `metrics` describes.
    ///
    /// [`None`] where the platform has no capture backend, where the
    /// Screen Recording permission was refused, or where this terminal
    /// has no window the window server will describe. Every one of
    /// those is a case for drawing nothing rather than for an error:
    /// the animation this feeds is decoration.
    /// `pinned` is the window this app has been found to be drawn in,
    /// where [`window_titled`] settled it. Without one the window is
    /// picked by size alone, which cannot tell two windows of the same
    /// size apart.
    pub(super) fn capture(metrics: Metrics, pinned: Option<u32>) -> Option<Self> {
        platform::capture(metrics, pinned)
    }

    /// The window this capture was taken for, as the window server
    /// numbers it, so that its position can be asked for from a thread
    /// that is not holding the capture.
    pub(super) const fn window(&self) -> u32 { self.window }

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
        Some(Placement {
            column: cell_index(left / self.cell.0)?,
            row:    cell_index(top / self.cell.1)?,
        })
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
pub(super) fn window_titles() -> Vec<(u32, Option<String>)> { platform::window_titles() }

/// The emulator's window whose title holds `marker`, or [`None`] while
/// the window server has yet to see the title change.
///
/// This is how one of an emulator's windows is told from another. Size
/// cannot do it -- two windows opened side by side are commonly the
/// same size to the pixel -- and neither can ownership, because every
/// window of the emulator answers to the same application. A title
/// only this process knows is unambiguous, and the terminal will wear
/// one for as long as it takes to ask.
pub(super) fn window_titled(marker: &str) -> Option<u32> { platform::window_titled(marker) }

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
            .field("window", &self.window)
            .field("metrics", &self.metrics)
            .field("origin", &self.origin)
            .field("cell", &self.cell)
            .field("columns", &self.columns)
            .field("rows", &self.rows)
            .finish_non_exhaustive()
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use std::collections::HashSet;
    use std::ffi::c_void;

    use objc2_core_foundation::CFArray;
    use objc2_core_foundation::CFDictionary;
    use objc2_core_foundation::CFNumber;
    use objc2_core_foundation::CFType;
    use objc2_core_foundation::CGRect as CoreGraphicsRect;
    use objc2_core_graphics::CGDisplayBounds;
    use objc2_core_graphics::CGRectMakeWithDictionaryRepresentation;
    use objc2_core_graphics::CGWindowListCopyWindowInfo;
    use objc2_core_graphics::CGWindowListOption;
    use objc2_core_graphics::kCGWindowBounds;
    use objc2_core_graphics::kCGWindowNumber;
    use ratatui::style::Color;
    use screencapturekit::cg::CGPoint;
    use screencapturekit::cg::CGRect;
    use screencapturekit::cg::CGSize;
    use screencapturekit::screenshot_manager::CGImageExt;
    use screencapturekit::screenshot_manager::SCScreenshotManager;
    use screencapturekit::shareable_content::SCDisplay;
    use screencapturekit::shareable_content::SCShareableContent;
    use screencapturekit::shareable_content::SCWindow;
    use screencapturekit::stream::configuration::SCStreamConfiguration;
    use screencapturekit::stream::content_filter::SCContentFilter;
    use sysinfo::Pid;
    use sysinfo::ProcessRefreshKind;
    use sysinfo::ProcessesToUpdate;
    use sysinfo::System;

    use super::Desktop;
    use super::Frame;
    use super::Metrics;
    use super::cell_index;
    use crate::backdrop::constants::SAMPLES_PER_CELL;
    use crate::process;

    /// How many bytes one pixel of the captured image occupies.
    const BYTES_PER_PIXEL: usize = 4;
    /// Where the red channel sits in the captured RGBA pixel.
    const RED: usize = 0;
    /// Where the green channel sits in the captured RGBA pixel.
    const GREEN: usize = 1;
    /// Where the blue channel sits in the captured RGBA pixel.
    const BLUE: usize = 2;

    /// See [`Desktop::capture`].
    pub(super) fn capture(metrics: Metrics, pinned: Option<u32>) -> Option<Desktop> {
        let content = SCShareableContent::get().ok()?;
        let windows = content.windows();
        let terminal_windows = terminal_windows(&windows);
        // The number `identify` pinned to this app's own window is
        // looked for across every window on the machine, not inside the
        // set the terminal is thought to own. Which windows those are
        // is a question that can be answered wrongly: an emulator
        // hosting its sessions in a server process is nowhere in this
        // app's parent chain, so the answer there is whichever
        // application is in front -- which stops being this one the
        // moment anything is opened over it. Searching the pinned
        // number inside that set loses this window exactly when another
        // application has taken the front, and what is chosen instead
        // is one of *its* windows, on whichever display it sits on.
        //
        // Falling back to size is for the run where the marker title
        // never took, and for the window closed since it did.
        let chosen = pinned
            .and_then(|pinned| windows.iter().find(|window| window.window_id() == pinned))
            .or_else(|| frontmost_window(&terminal_windows, metrics.text_pixels))?;
        let window = chosen.window_id();

        let displays = content.displays();
        let display = display_under(&displays, chosen.frame())?;
        let display_frame = display_bounds(display);
        // The display's points and its pixels are the same rectangle at
        // two resolutions, and the capture is asked for in points but
        // arrives in pixels, so a cell has to be known in both.
        let scale = f64::from(display.width()) / display_frame.size.width;
        let cell_pixels = metrics.cell_pixels();
        let cell = (cell_pixels.0 / scale, cell_pixels.1 / scale);

        // Every window the terminal owns comes out of the capture, and
        // so does every window standing in front of the one the app is
        // drawn in -- another application's window on top of this one
        // is not something this one is drawn over, and a capture that
        // keeps it shows the terminal whatever is covering it.
        //
        // What is left is what this window is drawn over. That is the
        // whole reason the capture can outlive a move: excluding a
        // window does not leave a hole where it stood, it composites
        // the display as though the window were not there at all, and
        // that answer does not depend on where the window is.
        let above = windows_above(window);
        // Asked of the application that owns the window this app is
        // drawn in, rather than of whichever one is in front, for the
        // same reason the window itself is.
        let owned = chosen
            .owning_application()
            .map(|application| application.process_id())
            .map_or_else(Vec::new, |owner| owned_by(&windows, |pid| pid == owner));
        let excluded: Vec<&SCWindow> = owned
            .into_iter()
            .chain(
                windows
                    .iter()
                    .filter(|window| above.contains(&window.window_id())),
            )
            .collect();
        let filter = SCContentFilter::create()
            .with_display(display)
            .with_excluding_windows(&excluded)
            .build();
        // The whole display, asked for at its own pixel size. Nothing is
        // scaled on the way out, which is what keeps a cell's share of
        // the image the same size in both axes -- ask for anything else
        // and the window server has two aspect ratios to reconcile and
        // reconciles them by pressing the capture into a corner of the
        // canvas.
        let image = (display.width(), display.height());
        let configuration = SCStreamConfiguration::new()
            .with_source_rect(CGRect {
                origin: CGPoint { x: 0.0, y: 0.0 },
                size:   CGSize {
                    width:  display_frame.size.width,
                    height: display_frame.size.height,
                },
            })
            .with_width(image.0)
            .with_height(image.1);

        let captured = SCScreenshotManager::capture_image(&filter, &configuration).ok()?;
        let pixels = captured.rgba_data().ok()?;
        let columns = whole_cells(f64::from(image.0) / cell_pixels.0)?;
        let rows = whole_cells(f64::from(image.1) / cell_pixels.1)?;
        let colors = reduce(&pixels, image, (columns, rows))?;
        Some(Desktop {
            window,
            metrics,
            origin: (display_frame.origin.x, display_frame.origin.y),
            cell,
            columns,
            rows,
            colors,
        })
    }

    /// See [`super::window_titles`].
    pub(super) fn window_titles() -> Vec<(u32, Option<String>)> {
        let Ok(content) = SCShareableContent::get() else {
            return Vec::new();
        };
        let windows = content.windows();
        terminal_windows(&windows)
            .into_iter()
            .map(|window| (window.window_id(), window.title()))
            .collect()
    }

    /// See [`super::window_titled`].
    pub(super) fn window_titled(marker: &str) -> Option<u32> {
        let content = SCShareableContent::get().ok()?;
        let windows = content.windows();
        terminal_windows(&windows)
            .into_iter()
            .find(|window| window.title().is_some_and(|title| title.contains(marker)))
            .map(SCWindow::window_id)
    }

    /// See [`super::window_frame`].
    ///
    /// CoreGraphics rather than `ScreenCaptureKit` because of what the
    /// two cost: `SCShareableContent::get` describes every window on
    /// the machine and takes about seventy milliseconds, where asking
    /// CoreGraphics about one window by number takes a few hundred
    /// microseconds. Both report the same rectangle, so nothing is
    /// given up by asking the cheaper of them.
    pub(super) fn window_frame(window: u32) -> Option<Frame> {
        let list = CGWindowListCopyWindowInfo(CGWindowListOption::OptionIncludingWindow, window)?;
        // Gone, if the window has been closed since the capture.
        let described = entry(&list, 0)?;
        let rect = bounds(described)?;
        Some(Frame {
            origin: (rect.origin.x, rect.origin.y),
            size:   (rect.size.width, rect.size.height),
        })
    }

    /// Every window standing in front of `window` on screen, by
    /// number.
    ///
    /// CoreGraphics for the same reason [`window_frame`] uses it, and
    /// for a second one: this is the question CoreGraphics answers
    /// directly. `SCShareableContent` describes every window on the
    /// machine and says nothing about which of them is in front of
    /// which, so the same answer taken from there would rest on the
    /// order its list happens to arrive in.
    fn windows_above(window: u32) -> Vec<u32> {
        let Some(list) =
            CGWindowListCopyWindowInfo(CGWindowListOption::OptionOnScreenAboveWindow, window)
        else {
            return Vec::new();
        };
        (0..list.count())
            .filter_map(|index| number(entry(&list, index)?))
            .collect()
    }

    /// The dictionary describing one window of a `CGWindowList` answer.
    ///
    /// # Invariants
    ///
    /// `list` must have come from [`CGWindowListCopyWindowInfo`], whose
    /// elements are Core Foundation objects the array holds a reference
    /// to. Nothing weaker will do: a `CFArray` can be built with null
    /// callbacks and filled with bare integers instead of objects, and
    /// an array that was cannot be told from one that was not.
    #[allow(
        unsafe_code,
        reason = "CoreFoundation collections have no safe binding: their \
                  accessors hand back untyped pointers, and the caller \
                  carries the invariant above in their place"
    )]
    fn entry(list: &CFArray, index: isize) -> Option<&CFDictionary> {
        if !(0..list.count()).contains(&index) {
            return None;
        }
        // SAFETY: `CFArrayGetValueAtIndex` is undefined outside the
        // array's own index space, and the check above is what holds
        // `index` inside it -- both ends of it, since a `CFIndex` is
        // signed and a negative one is as far out of the space as a
        // large one. The binding's own requirement is that the array's
        // generic match its contents, which an untyped `CFArray` has no
        // way to fail.
        let value = unsafe { list.value_at_index(index) };
        // SAFETY: the element is a Core Foundation object by this
        // function's invariant, so a non-null pointer to one points at a
        // live `CFType`; null is what `as_ref` answers `None` for. The
        // array owns a reference to that object for as long as the array
        // is alive, and the borrow handed back is tied to `list` by
        // lifetime elision, so nothing can read it after the array has
        // gone.
        let value = unsafe { value.cast::<CFType>().as_ref() }?;
        // That the element is a dictionary in particular is asked rather
        // than assumed: `downcast_ref` compares `CFGetTypeID` and hands
        // back `None` for anything else.
        value.downcast_ref::<CFDictionary>()
    }

    /// A window's own number, out of the dictionary describing it.
    ///
    /// # Invariants
    ///
    /// `described` must be one of [`entry`]'s dictionaries, and so must
    /// hold Core Foundation objects for the same reason its array does.
    #[allow(
        unsafe_code,
        reason = "CoreFoundation collections have no safe binding: their \
                  accessors hand back untyped pointers, and the caller \
                  carries the invariant above in their place"
    )]
    fn number(described: &CFDictionary) -> Option<u32> {
        // SAFETY: reading an `extern` static is unchecked because
        // nothing on this side knows the symbol was ever initialised.
        // This one is a `CFString` constant CoreGraphics builds as the
        // framework loads -- which is before any code that could reach
        // this line -- and keeps for the life of the process, so the
        // reference read out is live and the pointer taken to it cannot
        // dangle while the call below runs.
        let key = unsafe { std::ptr::from_ref(&**kCGWindowNumber) }.cast::<c_void>();
        // SAFETY: `CFDictionaryGetValue` wants a valid key pointer,
        // which is what `key` was just made, and a dictionary whose
        // generic matches its contents, which an untyped `CFDictionary`
        // has no way to fail. It answers null for a key the dictionary
        // does not hold.
        let value = unsafe { described.value(key) };
        // SAFETY: the value is a Core Foundation object by this
        // function's invariant, so a non-null pointer to one points at a
        // live `CFType`, and `as_ref` answers `None` for the null.
        // `described` holds a reference to it and outlives this call.
        let value = unsafe { value.cast::<CFType>().as_ref() }?;
        // Documented to be a number, and asked rather than assumed for
        // the same reason the bounds are. A window number is a `u32`
        // that CoreGraphics reports as a signed one.
        let number = value.downcast_ref::<CFNumber>()?.as_i32()?;
        u32::try_from(number).ok()
    }

    /// A window's bounds, out of the dictionary describing it.
    ///
    /// # Invariants
    ///
    /// `described` must be one of [`entry`]'s dictionaries, and so must
    /// hold Core Foundation objects for the same reason its array does.
    #[allow(
        unsafe_code,
        reason = "CoreFoundation collections have no safe binding: their \
                  accessors hand back untyped pointers, and the caller \
                  carries the invariant above in their place"
    )]
    fn bounds(described: &CFDictionary) -> Option<CoreGraphicsRect> {
        // SAFETY: reading an `extern` static is unchecked because
        // nothing on this side knows the symbol was ever initialised.
        // This one is a `CFString` constant CoreGraphics builds as the
        // framework loads -- which is before any code that could reach
        // this line -- and keeps for the life of the process, so the
        // reference read out is live and the pointer taken to it cannot
        // dangle while the call below runs.
        let key = unsafe { std::ptr::from_ref(&**kCGWindowBounds) }.cast::<c_void>();
        // SAFETY: `CFDictionaryGetValue` wants a valid key pointer,
        // which is what `key` was just made, and a dictionary whose
        // generic matches its contents, which an untyped `CFDictionary`
        // has no way to fail. It answers null for a key the dictionary
        // does not hold.
        let value = unsafe { described.value(key) };
        // SAFETY: the value is a Core Foundation object by this
        // function's invariant, so a non-null pointer to one points at a
        // live `CFType`, and `as_ref` answers `None` for the null.
        // `described` holds a reference to it and outlives this call.
        let value = unsafe { value.cast::<CFType>().as_ref() }?;
        // Documented to be a rect in dictionary form, and asked rather
        // than assumed, because the call below is undefined on something
        // that is not a dictionary at all. Given one that is but does
        // not describe a rect, it answers false and writes nothing.
        let value = value.downcast_ref::<CFDictionary>()?;
        let mut rect = CoreGraphicsRect::default();
        // SAFETY: the out pointer is to a live, aligned, already
        // initialised `CGRect` in this frame, so it stays valid to write
        // for the whole of a call that cannot outlast the frame; and the
        // dictionary is the one type-checked just above.
        unsafe {
            CGRectMakeWithDictionaryRepresentation(Some(value), std::ptr::from_mut(&mut rect))
        }
        .then_some(rect)
    }

    /// Where a display stands in the coordinate space every window is
    /// placed in, and how big it is there.
    ///
    /// CoreGraphics rather than `SCDisplay::frame`, which does not
    /// answer in that space. Measured against it, a window standing on
    /// any display but the primary one fell inside none of them at all,
    /// and the search below then settled for the first display it was
    /// given -- the primary -- so the terminal drew the desktop of a
    /// screen it was not on. `CGDisplayBounds` is read in the space
    /// window frames already are, so the two can be compared, and the
    /// same rectangle is what the capture is placed by afterwards.
    fn display_bounds(display: &SCDisplay) -> CoreGraphicsRect {
        CGDisplayBounds(display.display_id())
    }

    /// How many whole cells fit into a span of that many cells.
    ///
    /// [`None`] where the answer is not a count a grid could have --
    /// nothing at all, or more cells than a `u16` holds.
    fn whole_cells(cells: f64) -> Option<u16> {
        u16::try_from(cell_index(cells.floor())?)
            .ok()
            .filter(|count| *count > 0)
    }

    /// Average each cell's share of the captured display down to the one
    /// colour that cell is drawn in.
    ///
    /// The image is the display's own pixel size rather than a fixed
    /// block per cell, so a cell's share is worked out here: as many
    /// pixels across as the font is wide, and as many down as it is
    /// tall. [`SAMPLES_PER_CELL`] points are read along each axis of
    /// that share, which is what keeps the cost of a cell the same
    /// whatever size the display came back at.
    fn reduce(pixels: &[u8], image: (u32, u32), grid: (u16, u16)) -> Option<Vec<Color>> {
        let width = usize::try_from(image.0).ok()?;
        let height = usize::try_from(image.1).ok()?;
        let stride = width * BYTES_PER_PIXEL;
        let block = usize::try_from(SAMPLES_PER_CELL).ok()?.max(1);
        let columns = usize::from(grid.0);
        let rows = usize::from(grid.1);
        let mut colors = Vec::with_capacity(columns * rows);
        for row in 0..rows {
            let top = row * height / rows;
            let bottom = ((row + 1) * height / rows).max(top + 1);
            for column in 0..columns {
                let left = column * width / columns;
                let right = ((column + 1) * width / columns).max(left + 1);
                let mut totals = [0_u32; 3];
                let mut counted = 0_u32;
                for sample_row in 0..block {
                    for sample_column in 0..block {
                        let y = top + (bottom - top) * sample_row / block;
                        let x = left + (right - left) * sample_column / block;
                        let offset = y * stride + x * BYTES_PER_PIXEL;
                        let Some(pixel) = pixels.get(offset..offset + BYTES_PER_PIXEL) else {
                            continue;
                        };
                        totals[RED] += u32::from(pixel[RED]);
                        totals[GREEN] += u32::from(pixel[GREEN]);
                        totals[BLUE] += u32::from(pixel[BLUE]);
                        counted += 1;
                    }
                }
                if counted == 0 {
                    return None;
                }
                let channel = |total: u32| u8::try_from(total / counted).unwrap_or(u8::MAX);
                colors.push(Color::Rgb(
                    channel(totals[RED]),
                    channel(totals[GREEN]),
                    channel(totals[BLUE]),
                ));
            }
        }
        Some(colors)
    }

    /// Every on-screen window belonging to this process or one of its
    /// ancestors -- which is how the terminal emulator hosting this app
    /// is found, since the window is the emulator's and not this
    /// process's.
    fn terminal_windows(windows: &[SCWindow]) -> Vec<&SCWindow> {
        let ancestors = ancestor_pids();
        let owned = owned_by(windows, |pid| ancestors.contains(&pid));
        if !owned.is_empty() {
            return owned;
        }
        // An emulator that hosts its sessions in a server process of its
        // own is nowhere in this app's parent chain: iTerm2's shell hangs
        // off `iTermServer`, and the process drawing the window is not an
        // ancestor of anything running in it. The window this app is
        // drawn in is then the one in front, because the attract screen
        // only runs where somebody is looking at it.
        let Some(front) = frontmost_owner(windows) else {
            return Vec::new();
        };
        owned_by(windows, |pid| pid == front)
    }

    /// Every window whose owning application's pid `wanted` accepts.
    fn owned_by(windows: &[SCWindow], wanted: impl Fn(i32) -> bool) -> Vec<&SCWindow> {
        windows
            .iter()
            .filter(|window| {
                window
                    .owning_application()
                    .is_some_and(|application| wanted(application.process_id()))
            })
            .collect()
    }

    /// The one of the emulator's windows this app is drawn in.
    ///
    /// An emulator commonly has several windows open, and every one of
    /// them answers to the same application. Size is what tells them
    /// apart: `TIOCGWINSZ` reports this tty's own text area in pixels,
    /// and the window that area belongs to is the one whose frame it
    /// very nearly fills -- short by a title bar and whatever padding
    /// the emulator draws, and no more.
    ///
    /// Neither of the obvious answers works. The biggest window is
    /// whichever one happens to be biggest, and `is_active` is set on
    /// every window of the active application rather than on the key
    /// one. Both pick a sibling window as readily as this one, and what
    /// arrives then is the desktop behind something else.
    fn frontmost_window<'a>(
        windows: &'a [&'a SCWindow],
        text_pixels: (u16, u16),
    ) -> Option<&'a SCWindow> {
        let width = f64::from(text_pixels.0);
        let height = f64::from(text_pixels.1);
        windows
            .iter()
            .filter(|window| window.is_on_screen())
            .min_by(|left, right| {
                mismatch(left.frame(), width, height).total_cmp(&mismatch(
                    right.frame(),
                    width,
                    height,
                ))
            })
            .copied()
    }

    /// How far a window's frame is from holding a text area of `width`
    /// by `height` pixels, for [`frontmost_window`] to sort on.
    ///
    /// A frame narrower or shorter than the text area cannot be the one
    /// holding it, so falling short counts double and a frame that is
    /// merely larger by a title bar stays the nearest.
    fn mismatch(frame: CGRect, width: f64, height: f64) -> f64 {
        let axis = |frame: f64, text: f64| {
            let difference = frame - text;
            if difference < 0.0 {
                -difference * 2.0
            } else {
                difference
            }
        };
        axis(frame.size.width, width) + axis(frame.size.height, height)
    }

    /// The pid of the application owning the active window, which is the
    /// one the reader is looking at.
    fn frontmost_owner(windows: &[SCWindow]) -> Option<i32> {
        windows
            .iter()
            .find(|window| {
                window.is_active() && window.is_on_screen() && window.window_layer() == 0
            })
            .and_then(SCWindow::owning_application)
            .map(|application| application.process_id())
    }

    /// This process's pid and every pid above it, as the window server
    /// numbers them.
    fn ancestor_pids() -> HashSet<i32> {
        let mut system = System::new();
        system.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing(),
        );
        let mut pids = HashSet::new();
        let mut current = Pid::from_u32(std::process::id());
        // A cycle in the parent chain would spin here forever, so a pid
        // already seen ends the walk as surely as a missing parent does.
        while let Ok(pid) = i32::try_from(current.as_u32()) {
            if !pids.insert(pid) {
                break;
            }
            // sysinfo leaves the parent unset for a process it cannot
            // read, and `/usr/bin/login` between the emulator and the
            // shell is exactly that -- so the kernel answers where the
            // scan will not, or the walk never reaches the emulator
            // whose window this capture is about.
            match system
                .process(current)
                .and_then(sysinfo::Process::parent)
                .or_else(|| process::kernel_parent(current))
            {
                Some(parent) => current = parent,
                None => break,
            }
        }
        pids
    }

    /// The display holding the centre of `window_frame`.
    fn display_under(displays: &[SCDisplay], window_frame: CGRect) -> Option<&SCDisplay> {
        let center_x = window_frame.origin.x + window_frame.size.width / 2.0;
        let center_y = window_frame.origin.y + window_frame.size.height / 2.0;
        displays
            .iter()
            .find(|display| {
                let frame = display_bounds(display);
                center_x >= frame.origin.x
                    && center_x < frame.origin.x + frame.size.width
                    && center_y >= frame.origin.y
                    && center_y < frame.origin.y + frame.size.height
            })
            .or_else(|| displays.first())
    }
}

#[cfg(not(target_os = "macos"))]
mod platform {
    use super::Desktop;
    use super::Frame;
    use super::Metrics;

    /// No capture backend outside macOS, so nothing is drawn.
    pub(super) const fn capture(_: Metrics, _: Option<u32>) -> Option<Desktop> { None }

    /// Nothing to ask, where there is no capture to ask about.
    pub(super) const fn window_frame(_: u32) -> Option<Frame> { None }

    /// No windows to describe, so no title tells one from another.
    pub(super) const fn window_titles() -> Vec<(u32, Option<String>)> { Vec::new() }

    /// Nothing wears the marker where nothing can be asked.
    pub(super) const fn window_titled(_: &str) -> Option<u32> { None }
}
