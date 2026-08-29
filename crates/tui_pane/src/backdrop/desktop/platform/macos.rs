//! The macOS capture backend.
//!
//! `ScreenCaptureKit` takes the screenshot with every window this
//! terminal owns excluded, and CoreGraphics answers the cheap
//! per-window questions -- where a window stands, what it is titled --
//! that the drawing threads ask far more often.

use std::collections::HashSet;
use std::env;
use std::ffi::c_void;

use objc2_core_foundation::CFArray;
use objc2_core_foundation::CFDictionary;
use objc2_core_foundation::CFNumber;
use objc2_core_foundation::CFString;
use objc2_core_foundation::CFType;
use objc2_core_foundation::CGRect as CoreGraphicsRect;
use objc2_core_graphics::CGDisplayBounds;
use objc2_core_graphics::CGDisplayCopyDisplayMode;
use objc2_core_graphics::CGDisplayMode;
use objc2_core_graphics::CGPreflightScreenCaptureAccess;
use objc2_core_graphics::CGRectMakeWithDictionaryRepresentation;
use objc2_core_graphics::CGWindowListCopyWindowInfo;
use objc2_core_graphics::CGWindowListOption;
use objc2_core_graphics::kCGNullWindowID;
use objc2_core_graphics::kCGWindowBounds;
use objc2_core_graphics::kCGWindowLayer;
use objc2_core_graphics::kCGWindowName;
use objc2_core_graphics::kCGWindowNumber;
use objc2_core_graphics::kCGWindowOwnerName;
use objc2_core_graphics::kCGWindowOwnerPID;
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

use crate::backdrop::constants::EMULATOR_NAME_FLOOR;
use crate::backdrop::constants::POSITION_TOLERANCE;
use crate::backdrop::constants::SAMPLES_PER_CELL;
use crate::backdrop::constants::TERM_PROGRAM_ENV;
use crate::backdrop::desktop;
use crate::backdrop::desktop::CaptureAttemptResult;
use crate::backdrop::desktop::CaptureAttemptSequence;
use crate::backdrop::desktop::CaptureAttemptWindowSelection;
use crate::backdrop::desktop::CaptureFailure;
use crate::backdrop::desktop::CaptureWindowTarget;
use crate::backdrop::desktop::Desktop;
use crate::backdrop::desktop::Frame;
use crate::backdrop::desktop::Metrics;
use crate::backdrop::desktop::TerminalWindowSearchOutcome;
use crate::backdrop::desktop::TitledWindow;
use crate::backdrop::desktop::WindowTitle;
use crate::backdrop::desktop::candidate;
use crate::backdrop::desktop::candidate::TerminalWindowCandidate;
use crate::backdrop::desktop::candidate::TerminalWindowCandidates;
use crate::backdrop::desktop::candidate::TerminalWindowOwner;
use crate::process;

/// How many bytes one pixel of the captured image occupies.
const BYTES_PER_PIXEL: usize = 4;
/// Where the red channel sits in the captured RGBA pixel.
const RED: usize = 0;
/// Where the green channel sits in the captured RGBA pixel.
const GREEN: usize = 1;
/// Where the blue channel sits in the captured RGBA pixel.
const BLUE: usize = 2;

/// Whether macOS has granted this process Screen Recording access.
fn screen_capture_access_is_granted() -> bool { CGPreflightScreenCaptureAccess() }

/// Classify a failed shareable-content query from the process's access state.
const fn shareable_content_failure(access_granted: bool) -> CaptureFailure {
    if access_granted {
        CaptureFailure::ShareableContentQueryFailed
    } else {
        CaptureFailure::ScreenRecordingAccessNotGranted
    }
}

/// Keep the first window for each id while preserving window-server order.
fn deduplicate_windows_by_id<T>(
    windows: impl IntoIterator<Item = T>,
    mut window_id: impl FnMut(&T) -> u32,
) -> Vec<T> {
    let mut seen = HashSet::new();
    windows
        .into_iter()
        .filter(|window| seen.insert(window_id(window)))
        .collect()
}

/// See [`Desktop::capture`].
pub(in crate::backdrop::desktop) fn capture(
    metrics: Metrics,
    capture_window_target: CaptureWindowTarget,
    sequence: CaptureAttemptSequence,
) -> CaptureAttemptResult {
    let Ok(content) = SCShareableContent::get() else {
        return candidate::capture_failure_before_window_selection(
            sequence,
            shareable_content_failure(screen_capture_access_is_granted()),
        );
    };
    let windows = content.windows();
    let displays = content.displays();
    let terminal_window_candidates = terminal_windows(&windows);
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
    let selected = candidate::select_capture_window(
        &windows,
        capture_window_target,
        &terminal_window_candidates,
        SCWindow::window_id,
        || {
            frontmost_window(
                &terminal_window_candidates.windows,
                &displays,
                metrics.text_area,
            )
            .ok_or(CaptureFailure::TerminalWindowNotFound)
        },
    );
    let Ok((chosen, method)) = selected else {
        return candidate::capture_failure_before_window_selection(
            sequence,
            CaptureFailure::TerminalWindowNotFound,
        );
    };
    let window_id = chosen.window_id();
    let window_selection = CaptureAttemptWindowSelection::Selected { window_id, method };

    let desktop_result = capture_selected_window(metrics, chosen, &windows, &displays);
    CaptureAttemptResult::from_desktop_result(sequence, window_selection, desktop_result)
}

/// Capture the display behind the terminal window selected for this attempt.
fn capture_selected_window(
    metrics: Metrics,
    chosen: &SCWindow,
    windows: &[SCWindow],
    displays: &[SCDisplay],
) -> Result<Desktop, CaptureFailure> {
    let window_id = chosen.window_id();
    let display =
        CaptureFailure::DisplayNotFound.classify_option(display_under(displays, chosen.frame()))?;
    let display_frame = display_bounds(display);
    // The capture is asked for at the display's own point size and
    // comes back at it, so one cell in points serves both the grid
    // the capture is reduced against and the frame the window is
    // placed by. `SCDisplay` measures in points as `CGDisplayBounds`
    // does, which is why there is no second unit here to convert to.
    //
    // What the terminal reports is the one thing here that is not in
    // points -- it answers in the display's pixels -- so the scale
    // divides it before a cell comes out of it. The scale is asked
    // of the display rather than worked out from the window, so
    // which window was matched cannot change the size of a cell.
    // See [`Metrics::cell_points`].
    let cell = metrics.cell_points(backing_scale(display));

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
    let above = windows_above(window_id);
    // Asked of the application that owns the window this app is
    // drawn in, rather than of whichever one is in front, for the
    // same reason the window itself is.
    let owned = chosen
        .owning_application()
        .map(|application| application.process_id())
        .map_or_else(Vec::new, |owner| {
            candidate::windows_owned_by(windows, |pid| pid == owner)
        });
    let excluded = deduplicate_windows_by_id(
        owned.into_iter().chain(
            windows
                .iter()
                .filter(|window| above.contains(&window.window_id())),
        ),
        |window| window.window_id(),
    );
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

    let captured = CaptureFailure::ScreenshotFailed
        .classify_result(SCScreenshotManager::capture_image(&filter, &configuration))?;
    let pixels = CaptureFailure::PixelExtractionFailed.classify_result(captured.rgba_data())?;
    let (columns, rows, colors) = reduce_capture(&pixels, image, cell)?;
    Ok(Desktop {
        window_id,
        metrics,
        origin: (display_frame.origin.x, display_frame.origin.y),
        cell,
        columns,
        rows,
        colors,
    })
}

/// See [`desktop::window_titles`].
///
/// CoreGraphics rather than `ScreenCaptureKit`, for the reason
/// [`Listed::on_screen`] gives: this is called from the thread that
/// draws.
pub(in crate::backdrop::desktop) fn window_titles() -> Vec<TitledWindow> {
    terminal_windows(&Listed::on_screen())
        .windows
        .into_iter()
        .map(|window| TitledWindow {
            window_id: window.number,
            title:     window.title.clone(),
        })
        .collect()
}

/// See [`desktop::window_titled`].
///
/// CoreGraphics rather than `ScreenCaptureKit`, for the reason
/// [`Listed::on_screen`] gives.
pub(in crate::backdrop::desktop) fn window_titled(marker: &str) -> TerminalWindowSearchOutcome {
    terminal_windows(&Listed::on_screen())
        .windows
        .into_iter()
        .find(|window| match &window.title {
            WindowTitle::Reported(title) => title.contains(marker),
            WindowTitle::Withheld => false,
        })
        .map_or(TerminalWindowSearchOutcome::NotFound, |window| {
            TerminalWindowSearchOutcome::Found {
                window_id: window.number,
            }
        })
}

/// See [`desktop::window_at`].
///
/// CoreGraphics rather than `ScreenCaptureKit`, for the reason
/// [`Listed::on_screen`] gives. Nothing here filters for windows
/// that are on screen because the list asked for holds no others.
pub(in crate::backdrop::desktop) fn window_at(origin: (f64, f64)) -> TerminalWindowSearchOutcome {
    terminal_windows(&Listed::on_screen())
        .windows
        .into_iter()
        .map(|window| (window.number, away(window.bounds, origin)))
        .filter(|(_, away)| *away <= POSITION_TOLERANCE)
        .min_by(|(_, left), (_, right)| left.total_cmp(right))
        .map_or(TerminalWindowSearchOutcome::NotFound, |(window_id, _)| {
            TerminalWindowSearchOutcome::Found { window_id }
        })
}

/// How far a window's corner stands from `origin`, for
/// [`window_at`] to sort on.
///
/// Along both axes added together rather than as a diagonal. The
/// question is which of a handful of windows is nearest, and every
/// ordering that answers it agrees; a square root would only cost
/// what it does not settle.
fn away(frame: CoreGraphicsRect, origin: (f64, f64)) -> f64 {
    (frame.origin.x - origin.0).abs() + (frame.origin.y - origin.1).abs()
}

/// See [`desktop::window_frame`].
///
/// CoreGraphics rather than `ScreenCaptureKit` because of what the
/// two cost: `SCShareableContent::get` describes every window on
/// the machine and takes about seventy milliseconds, where asking
/// CoreGraphics about one window by number takes a few hundred
/// microseconds. Both report the same rectangle, so nothing is
/// given up by asking the cheaper of them.
pub(in crate::backdrop::desktop) fn window_frame(window: u32) -> Option<Frame> {
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
        .filter_map(|index| match number(entry(&list, index)?) {
            DescribedWindowNumber::Numbered { window_id } => Some(window_id),
            DescribedWindowNumber::Unnumbered => None,
        })
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

/// Which of the entries describing a window is wanted.
///
/// The names are `CFString` constants the framework builds as it
/// loads, and reading an `extern` static is unchecked -- so they
/// are read in the one place below rather than at every use.
#[derive(Clone, Copy)]
enum Key {
    /// The window server's own number for the window.
    Number,
    /// Where the window stands and how big it is.
    Bounds,
    /// Which layer it is drawn on. Ordinary windows are on nought;
    /// the menu bar, the dock and the desktop picture are not.
    Layer,
    /// The pid of the application that owns it.
    Owner,
    /// The name that application answers to.
    OwnerName,
    /// What the window is titled.
    Title,
}

impl Key {
    /// The CoreGraphics constant naming this entry.
    #[allow(
        unsafe_code,
        reason = "an `extern` static has no safe binding; the SAFETY \
                  comment below covers every one of them at once"
    )]
    fn name(self) -> &'static CFString {
        // SAFETY: reading an `extern` static is unchecked because
        // nothing on this side knows the symbol was ever
        // initialised. These are `CFString` constants CoreGraphics
        // builds as the framework loads -- which is before any code
        // that could reach this line -- and keeps for the life of
        // the process, so whichever reference is read out is live
        // and cannot dangle while the caller holds it.
        unsafe {
            match self {
                Self::Number => kCGWindowNumber,
                Self::Bounds => kCGWindowBounds,
                Self::Layer => kCGWindowLayer,
                Self::Owner => kCGWindowOwnerPID,
                Self::OwnerName => kCGWindowOwnerName,
                Self::Title => kCGWindowName,
            }
        }
    }
}

/// What a window's dictionary holds under `key`, or [`None`] where
/// it holds nothing under it.
///
/// A window server that will not answer a question leaves the entry
/// out rather than emptying it, so this is missing far more often
/// than it looks: [`Key::Title`] is absent for every window the
/// process has no Screen Recording permission to read.
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
fn value(described: &CFDictionary, key: Key) -> Option<&CFType> {
    let key = std::ptr::from_ref(key.name()).cast::<c_void>();
    // SAFETY: `CFDictionaryGetValue` wants a valid key pointer,
    // which is what `key` was just made from a live constant, and a
    // dictionary whose generic matches its contents, which an
    // untyped `CFDictionary` has no way to fail. It answers null for
    // a key the dictionary does not hold.
    let value = unsafe { described.value(key) };
    // SAFETY: the value is a Core Foundation object by this
    // function's invariant, so a non-null pointer to one points at a
    // live `CFType`, and `as_ref` answers `None` for the null.
    // `described` holds a reference to it and outlives this call.
    unsafe { value.cast::<CFType>().as_ref() }
}

/// A whole number out of a window's dictionary.
///
/// The type is asked rather than assumed, here and in every reader
/// below it: `downcast_ref` compares `CFGetTypeID` and hands back
/// `None` for anything else, so a window server answering something
/// other than what it documents costs a missing field and not a
/// misread one.
fn integer(described: &CFDictionary, key: Key) -> Option<i32> {
    value(described, key)?.downcast_ref::<CFNumber>()?.as_i32()
}

/// A string out of a window's dictionary.
fn text(described: &CFDictionary, key: Key) -> Option<String> {
    Some(
        value(described, key)?
            .downcast_ref::<CFString>()?
            .to_string(),
    )
}

/// Whether the window server numbered a window in the dictionary
/// describing it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DescribedWindowNumber {
    /// The description carried no window number a caller could use:
    /// the key was absent, held something other than a number, or held
    /// one no `u32` can represent. A window that cannot be named is of
    /// no use to either caller, so both drop it.
    Unnumbered,
    /// The window server numbers the described window this way.
    Numbered {
        /// The window server's own number for the window.
        window_id: u32,
    },
}

/// A window's own number, out of the dictionary describing it, where
/// the window server supplied one a caller can use.
///
/// # Invariants
///
/// `described` must be one of [`entry`]'s dictionaries, and so must
/// hold Core Foundation objects for the same reason its array does.
fn number(described: &CFDictionary) -> DescribedWindowNumber {
    // A window number is a `u32` that CoreGraphics reports as a
    // signed one.
    integer(described, Key::Number)
        .and_then(|number| u32::try_from(number).ok())
        .map_or(DescribedWindowNumber::Unnumbered, |window_id| {
            DescribedWindowNumber::Numbered { window_id }
        })
}

/// A window's bounds, out of the dictionary describing it.
///
/// # Invariants
///
/// `described` must be one of [`entry`]'s dictionaries, and so must
/// hold Core Foundation objects for the same reason its array does.
#[allow(
    unsafe_code,
    reason = "`CGRectMakeWithDictionaryRepresentation` has no safe \
              binding: it writes through an out pointer, which the \
              caller supplies and the SAFETY comment below covers"
)]
fn bounds(described: &CFDictionary) -> Option<CoreGraphicsRect> {
    // Documented to be a rect in dictionary form, and asked rather
    // than assumed, because the call below is undefined on something
    // that is not a dictionary at all. Given one that is but does
    // not describe a rect, it answers false and writes nothing.
    let value = value(described, Key::Bounds)?.downcast_ref::<CFDictionary>()?;
    let mut rect = CoreGraphicsRect::default();
    // SAFETY: the out pointer is to a live, aligned, already
    // initialised `CGRect` in this frame, so it stays valid to write
    // for the whole of a call that cannot outlast the frame; and the
    // dictionary is the one type-checked just above.
    unsafe { CGRectMakeWithDictionaryRepresentation(Some(value), std::ptr::from_mut(&mut rect)) }
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
fn display_bounds(display: &SCDisplay) -> CoreGraphicsRect { CGDisplayBounds(display.display_id()) }

/// How many cells a grid needs to cover a span of that many cells.
///
/// Rounded up, because a display is rarely a whole number of cells
/// tall and the remainder is where the bottom row of a full-height
/// window sits. Rounded down it falls outside the grid, `color_at`
/// gives back nothing for it, and the last row of the window is
/// left unpainted. [`reduce`] shares the image out across whatever
/// count it is handed, so a grid reaching a little past the display
/// costs a fraction of a pixel per cell and nothing else.
///
/// [`None`] where the answer is not a count a grid could have --
/// nothing at all, or more cells than a `u16` holds.
fn whole_cells(cells: f64) -> Option<u16> {
    u16::try_from(desktop::cell_index(cells.ceil())?)
        .ok()
        .filter(|count| *count > 0)
}

/// Reduce captured RGBA pixels to the terminal-cell grid implied by `image` and `cell`.
fn reduce_capture(
    pixels: &[u8],
    image: (u32, u32),
    cell: (f64, f64),
) -> Result<(u16, u16, Vec<Color>), CaptureFailure> {
    let columns = CaptureFailure::ImageReductionFailed
        .classify_option(whole_cells(f64::from(image.0) / cell.0))?;
    let rows = CaptureFailure::ImageReductionFailed
        .classify_option(whole_cells(f64::from(image.1) / cell.1))?;
    let colors = CaptureFailure::ImageReductionFailed.classify_option(reduce(
        pixels,
        image,
        (columns, rows),
    ))?;
    Ok((columns, rows, colors))
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

/// Application-name facts used to match a window to `TERM_PROGRAM`.
trait TerminalProgramWindowCandidate {
    /// Every folded name the owning application answers to.
    fn names(&self) -> Vec<String>;
}

impl TerminalWindowCandidate for SCWindow {
    fn owner(&self) -> TerminalWindowOwner {
        self.owning_application()
            .map_or(TerminalWindowOwner::Unnamed, |application| {
                TerminalWindowOwner::Application {
                    pid: application.process_id(),
                }
            })
    }

    fn frontmost(&self) -> bool {
        self.is_active() && self.is_on_screen() && self.window_layer() == 0
    }
}

impl TerminalProgramWindowCandidate for SCWindow {
    fn names(&self) -> Vec<String> {
        self.owning_application()
            .map(|application| {
                vec![
                    folded(&application.application_name()),
                    folded(&application.bundle_identifier()),
                ]
            })
            .unwrap_or_default()
    }
}

/// One window of a `CGWindowList` answer, read out of the
/// dictionary describing it.
struct Listed {
    /// The window server's own number for it.
    number: u32,
    /// Which application owns it. [`TerminalWindowOwner::Unnamed`]
    /// means the dictionary named no owner to match to a process.
    owner:  TerminalWindowOwner,
    /// The name that application answers to, folded.
    name:   Option<String>,
    /// What it is titled. [`window_at`] is the path that does not need
    /// the window server to report a title.
    title:  WindowTitle,
    /// Where it stands, in the space `CGDisplayBounds` measures.
    bounds: CoreGraphicsRect,
    /// Which layer it is drawn on.
    layer:  i32,
}

impl Listed {
    /// Every window on screen, in the front-to-back order the
    /// window server lists them in.
    ///
    /// This is the cheap way of asking, and the reason the drawing
    /// thread can ask at all: `SCShareableContent::get` describes
    /// the same windows and takes about seventy milliseconds over
    /// it, which is a frame lost every time it is called and was
    /// several frames lost every pass. A run that cannot draw looks
    /// exactly like one whose screen never came on. See
    /// [`window_frame`] for the same comparison.
    fn on_screen() -> Vec<Self> {
        let Some(list) =
            CGWindowListCopyWindowInfo(CGWindowListOption::OptionOnScreenOnly, kCGNullWindowID)
        else {
            return Vec::new();
        };
        (0..list.count())
            .filter_map(|index| Self::read(entry(&list, index)?))
            .collect()
    }

    /// One window, out of the dictionary describing it.
    ///
    /// [`None`] where the window server named no number for it or
    /// would not say where it stands, since neither the search nor
    /// the caller has any use for a window it cannot name or place.
    /// Everything else is optional here because it is optional
    /// there.
    fn read(described: &CFDictionary) -> Option<Self> {
        let DescribedWindowNumber::Numbered { window_id } = number(described) else {
            return None;
        };
        Some(Self {
            number: window_id,
            owner:  integer(described, Key::Owner).map_or(TerminalWindowOwner::Unnamed, |pid| {
                TerminalWindowOwner::Application { pid }
            }),
            name:   text(described, Key::OwnerName).map(|name| folded(&name)),
            title:  text(described, Key::Title)
                .map_or(WindowTitle::Withheld, WindowTitle::Reported),
            bounds: bounds(described)?,
            layer:  integer(described, Key::Layer).unwrap_or_default(),
        })
    }
}

impl TerminalWindowCandidate for Listed {
    fn owner(&self) -> TerminalWindowOwner { self.owner }

    fn frontmost(&self) -> bool { self.layer == 0 }
}

impl TerminalProgramWindowCandidate for Listed {
    fn names(&self) -> Vec<String> { self.name.clone().into_iter().collect() }
}

/// Every on-screen window belonging to this process or one of its
/// ancestors -- which is how the terminal emulator hosting this app
/// is found, since the window is the emulator's and not this
/// process's.
fn terminal_windows<W: TerminalWindowCandidate + TerminalProgramWindowCandidate>(
    windows: &[W],
) -> TerminalWindowCandidates<'_, W> {
    let ancestors = ancestor_pids();
    // An emulator that hosts its sessions in a server process of its
    // own is nowhere in this app's parent chain: iTerm2's shell hangs
    // off `iTermServer`, and the process drawing the window is not an
    // ancestor of anything running in it. Ask the emulator who it is
    // instead -- it says so in the environment it handed down.
    let named = named_emulator_windows(windows);
    candidate::terminal_window_candidates(
        windows,
        |pid| ancestors.contains(&pid),
        |window| {
            named
                .iter()
                .any(|named_window| std::ptr::eq(*named_window, window))
        },
    )
}

/// Every on-screen window of the terminal emulator named by
/// `TERM_PROGRAM`.
///
/// Every emulator worth the name sets this in the environment it
/// hands the shell -- `iTerm.app`, `Apple_Terminal`, `WezTerm`,
/// `ghostty` -- and it survives the walk down to this process
/// however many shells stand in between, which is exactly what the
/// parent chain does not. It names the application rather than the
/// window, so it cannot tell two windows of one emulator apart;
/// what it does rule out is choosing a window belonging to some
/// other application entirely, which is the way this went wrong.
///
/// Empty where the variable is unset, where it names nothing on
/// screen, or where this is not a terminal at all.
fn named_emulator_windows<W: TerminalProgramWindowCandidate>(windows: &[W]) -> Vec<&W> {
    let Ok(program) = env::var(TERM_PROGRAM_ENV) else {
        return Vec::new();
    };
    // `TERM_PROGRAM` names a bundle, extension and all: iTerm2 sets
    // `iTerm.app`. Folding keeps every letter, so the extension
    // survives it as `itermapp` -- which is inside neither the
    // application's own name, `iterm2`, nor its bundle identifier,
    // `comgooglecodeiterm2`. The containment test then fails in
    // both directions and every window of the emulator is passed
    // over, leaving whichever application stands in front to be
    // taken for the terminal.
    let wanted = folded(program.strip_suffix(".app").unwrap_or(&program));
    if wanted.len() < EMULATOR_NAME_FLOOR {
        return Vec::new();
    }
    windows
        .iter()
        .filter(|window| {
            window
                .names()
                .iter()
                .any(|found| names_agree(&wanted, found))
        })
        .collect()
}

/// `text` with everything but its letters and digits taken out and
/// the rest put in lower case, so that the several ways one
/// emulator writes its own name can be compared.
///
/// `TERM_PROGRAM` gives `iTerm.app`, the application calls itself
/// `iTerm2` and its bundle is `com.googlecode.iterm2`; folded, all
/// three carry `iterm` -- once the `.app` extension has been taken
/// off the first, which [`named_emulator_windows`] does before
/// folding it. This keeps letters and digits, so an extension left
/// on survives as part of the name.
fn folded(text: &str) -> String {
    text.chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|character| character.to_ascii_lowercase())
        .collect()
}

/// Whether two folded names are the same emulator.
///
/// Either may be the longer: `appleterminal` holds `terminal`, and
/// `comgooglecodeiterm2` holds `iterm`. Both are held to
/// [`EMULATOR_NAME_FLOOR`] first, because a name of two or three
/// letters is inside half the bundle identifiers on the machine.
fn names_agree(wanted: &str, found: &str) -> bool {
    found.len() >= EMULATOR_NAME_FLOOR && (found.contains(wanted) || wanted.contains(found))
}

/// The one of the emulator's windows this app is drawn in.
///
/// An emulator commonly has several windows open, and every one of
/// them answers to the same application. Size is what tells them
/// apart: `TIOCGWINSZ` reports this tty's own text area, and the
/// window that area belongs to is the one whose frame it very
/// nearly fills -- short by a title bar and whatever padding the
/// emulator draws, and no more.
///
/// Neither of the obvious answers works. The biggest window is
/// whichever one happens to be biggest, and `is_active` is set on
/// every window of the active application rather than on the key
/// one. Both pick a sibling window as readily as this one, and what
/// arrives then is the desktop behind something else.
fn frontmost_window<'a>(
    windows: &'a [&'a SCWindow],
    displays: &[SCDisplay],
    text_pixels: (u16, u16),
) -> Option<&'a SCWindow> {
    // Each candidate is scored against the scale of the display it
    // stands on rather than one scale for all of them, since a
    // machine with a Retina panel and an external monitor is
    // carrying both at once.
    let score = |window: &SCWindow| {
        let frame = window.frame();
        let scale = display_under(displays, frame).map_or(1, backing_scale);
        mismatch(frame, text_pixels, scale)
    };
    windows
        .iter()
        .filter(|window| window.is_on_screen())
        .min_by(|left, right| score(left).total_cmp(&score(right)))
        .copied()
}

/// How far a window's frame is from holding a text area of
/// `text_pixels`, for [`frontmost_window`] to sort on.
///
/// A frame narrower or shorter than the text area cannot be the one
/// holding it, so falling short counts double and a frame that is
/// merely larger by a title bar stays the nearest.
///
/// The reported area is divided by the `scale` of the display this
/// candidate stands on before anything is compared. Scored as
/// reported, a text area on a Retina panel reads as twice the
/// window holding it, and every candidate on that panel is judged
/// short by the same doubled amount -- which is no ordering at all.
fn mismatch(frame: CGRect, text_pixels: (u16, u16), scale: u32) -> f64 {
    let scale = f64::from(scale);
    let width = f64::from(text_pixels.0) / scale;
    let height = f64::from(text_pixels.1) / scale;
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

/// This process's pid and every pid above it, as the window server
/// numbers them.
fn ancestor_pids() -> HashSet<i32> {
    let mut system = System::new();
    system.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::nothing());
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

/// How many pixels `display` carries to the point.
///
/// Read off the display's own mode, which is the only place the
/// pixel count is available at all: `SCDisplay` and `CGDisplayBounds`
/// both measure in points. Asking the display rather than working
/// the ratio out from a window's frame keeps the cell size out of
/// reach of the window match, which can be wrong and has been --
/// a wrong match would otherwise carry its error into every cell.
///
/// The framebuffer is a whole multiple of the point grid on every
/// mode macOS offers, so the ratio is taken in whole numbers. A
/// display that will not answer is read as one, which is what a
/// panel carrying one pixel to the point would have answered.
fn backing_scale(display: &SCDisplay) -> u32 {
    let Some(mode) = CGDisplayCopyDisplayMode(display.display_id()) else {
        return 1;
    };
    let points = CGDisplayMode::width(Some(&mode));
    let pixels = CGDisplayMode::pixel_width(Some(&mode));
    if points == 0 {
        return 1;
    }
    u32::try_from(pixels / points).unwrap_or(1).max(1)
}

/// The centre of `window_frame`, in the same points
/// [`display_bounds`] answers in.
fn window_center(window_frame: CGRect) -> (f64, f64) {
    (
        window_frame.origin.x + window_frame.size.width / 2.0,
        window_frame.origin.y + window_frame.size.height / 2.0,
    )
}

/// Whether `bounds` holds the point `center`.
fn holds(bounds: CoreGraphicsRect, center: (f64, f64)) -> bool {
    center.0 >= bounds.origin.x
        && center.0 < bounds.origin.x + bounds.size.width
        && center.1 >= bounds.origin.y
        && center.1 < bounds.origin.y + bounds.size.height
}

/// How far `center` sits from the middle of `bounds`, squared.
///
/// Squared because only the ordering is wanted and the root would
/// not change it.
fn away_from(bounds: CoreGraphicsRect, center: (f64, f64)) -> f64 {
    let x = bounds.origin.x + bounds.size.width / 2.0 - center.0;
    let y = bounds.origin.y + bounds.size.height / 2.0 - center.1;
    x.mul_add(x, y * y)
}

/// The display holding the centre of `window_frame`.
///
/// A centre that lands inside no display at all -- a window
/// straddling the gap between two panels, or hanging off an edge --
/// resolves to the display whose own centre is nearest. The first
/// display is the primary, and the primary is the one display a
/// window that is demonstrably somewhere else is least likely to be
/// on; answering with it names a whole different desktop and leaves
/// no sign that the containment test found nothing.
fn display_under(displays: &[SCDisplay], window_frame: CGRect) -> Option<&SCDisplay> {
    let center = window_center(window_frame);
    displays
        .iter()
        .find(|display| holds(display_bounds(display), center))
        .or_else(|| {
            displays.iter().min_by(|left, right| {
                away_from(display_bounds(left), center)
                    .total_cmp(&away_from(display_bounds(right), center))
            })
        })
}

impl CaptureFailure {
    /// Replace an underlying stage error with this compact classification.
    fn classify_result<T, E>(self, result: Result<T, E>) -> Result<T, Self> {
        result.map_err(|_| self)
    }

    /// Require a stage to have produced its value, classifying absence as this failure.
    fn classify_option<T>(self, value: Option<T>) -> Result<T, Self> { value.ok_or(self) }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use ratatui::style::Color;

    use super::CaptureFailure;

    /// The three names iTerm2 answers to, as
    /// [`named_emulator_windows`](super::named_emulator_windows)
    /// meets them: `TERM_PROGRAM`, the application's own name, and
    /// its bundle identifier.
    const TERM_PROGRAM: &str = "iTerm.app";
    /// What `SCRunningApplication` calls iTerm2.
    const APPLICATION: &str = "iTerm2";
    /// iTerm2's bundle identifier.
    const BUNDLE: &str = "com.googlecode.iterm2";

    #[test]
    fn failed_shareable_content_query_with_access_reports_query_failure() {
        assert_eq!(
            super::shareable_content_failure(true),
            CaptureFailure::ShareableContentQueryFailed
        );
    }

    #[test]
    fn failed_shareable_content_query_without_access_reports_permission_denial() {
        assert_eq!(
            super::shareable_content_failure(false),
            CaptureFailure::ScreenRecordingAccessNotGranted
        );
    }

    #[test]
    fn image_reduction_rejects_a_cell_too_large_for_the_image() {
        assert_eq!(
            super::reduce_capture(&[], (1, 1), (f64::INFINITY, 1.0)),
            Err(CaptureFailure::ImageReductionFailed)
        );
    }

    #[test]
    fn image_reduction_returns_the_implied_grid_and_colors() {
        let pixels = [1, 2, 3, 255, 4, 5, 6, 255];

        assert_eq!(
            super::reduce_capture(&pixels, (2, 1), (1.0, 1.0)),
            Ok((2, 1, vec![Color::Rgb(1, 2, 3), Color::Rgb(4, 5, 6)]))
        );
    }

    #[test]
    fn exclusion_windows_are_deduplicated_by_id_in_original_order() {
        let windows = [
            (17, "terminal-owned"),
            (23, "terminal-owned"),
            (17, "above-selected"),
            (41, "above-selected"),
            (23, "above-selected"),
        ];

        let deduplicated = super::deduplicate_windows_by_id(windows, |window| window.0);

        assert_eq!(
            deduplicated,
            vec![
                (17, "terminal-owned"),
                (23, "terminal-owned"),
                (41, "above-selected"),
            ]
        );
    }

    #[test]
    fn folding_keeps_an_extension_that_is_left_on() {
        assert_eq!(super::folded(TERM_PROGRAM), "itermapp");
    }

    #[test]
    fn folding_a_stripped_term_program_gives_the_bare_name() {
        let stripped = TERM_PROGRAM.strip_suffix(".app").expect("names a bundle");
        assert_eq!(super::folded(stripped), "iterm");
    }

    #[test]
    fn an_extension_left_on_agrees_with_neither_name() {
        let wanted = super::folded(TERM_PROGRAM);
        assert!(!super::names_agree(&wanted, &super::folded(APPLICATION)));
        assert!(!super::names_agree(&wanted, &super::folded(BUNDLE)));
    }

    #[test]
    fn a_stripped_term_program_agrees_with_both_names() {
        let stripped = TERM_PROGRAM.strip_suffix(".app").expect("names a bundle");
        let wanted = super::folded(stripped);
        assert!(super::names_agree(&wanted, &super::folded(APPLICATION)));
        assert!(super::names_agree(&wanted, &super::folded(BUNDLE)));
    }

    #[test]
    fn an_emulator_naming_no_bundle_is_unaffected() {
        for (program, application) in [
            ("Apple_Terminal", "Terminal"),
            ("WezTerm", "WezTerm"),
            ("ghostty", "Ghostty"),
        ] {
            let stripped = program.strip_suffix(".app").unwrap_or(program);
            assert!(
                super::names_agree(&super::folded(stripped), &super::folded(application)),
                "{program} should agree with {application}"
            );
        }
    }
}
