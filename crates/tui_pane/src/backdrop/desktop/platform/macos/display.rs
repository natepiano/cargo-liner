use objc2_core_foundation::CGRect as CoreGraphicsRect;
use objc2_core_graphics::CGDirectDisplayID;
use objc2_core_graphics::CGDisplayBounds;
use objc2_core_graphics::CGDisplayCopyDisplayMode;
use objc2_core_graphics::CGDisplayMode;
use objc2_core_graphics::CGError;
use objc2_core_graphics::CGGetActiveDisplayList;

/// An active display, read from CoreGraphics. Replaces `SCDisplay` in the capture path so the
/// capture makes no `ScreenCaptureKit` call.
pub(super) struct Display {
    /// The window server's id for the display; every geometry query keys on it.
    pub(super) id: CGDirectDisplayID,
    /// Where the display stands, in the points `CGDisplayBounds` answers in.
    bounds:        CoreGraphicsRect,
}

/// Every display currently active in the window server.
#[expect(
    unsafe_code,
    reason = "CGGetActiveDisplayList writes through caller-provided count and display pointers"
)]
pub(super) fn active_displays() -> Vec<Display> {
    let mut count = 0_u32;
    // SAFETY: `CGGetActiveDisplayList` receives a null display array for the count-only query, and
    // `count` points to a live local `u32` for the duration of the call.
    let error =
        unsafe { CGGetActiveDisplayList(0, std::ptr::null_mut(), std::ptr::from_mut(&mut count)) };
    if error != CGError::Success || count == 0 {
        return Vec::new();
    }

    let max_displays = count;
    let Ok(capacity) = usize::try_from(max_displays) else {
        return Vec::new();
    };
    let mut display_ids = Vec::with_capacity(capacity);
    // SAFETY: `display_ids` has spare capacity for `max_displays` ids, its pointer stays valid for
    // the call, and `count` points to a live local `u32`. CoreGraphics writes at most
    // `max_displays` initialized ids.
    let error = unsafe {
        CGGetActiveDisplayList(
            max_displays,
            display_ids.as_mut_ptr(),
            std::ptr::from_mut(&mut count),
        )
    };
    if error != CGError::Success || count > max_displays {
        return Vec::new();
    }
    let Ok(length) = usize::try_from(count) else {
        return Vec::new();
    };
    // SAFETY: the successful call initialized the first `count` entries, and `count` does not
    // exceed the vector's `max_displays` capacity.
    unsafe { display_ids.set_len(length) };

    display_ids
        .into_iter()
        .map(|id| Display {
            id,
            bounds: CGDisplayBounds(id),
        })
        .collect()
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
pub(super) const fn display_bounds(display: &Display) -> CoreGraphicsRect { display.bounds }

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
pub(super) fn backing_scale(display: &Display) -> u32 {
    let Some(mode) = CGDisplayCopyDisplayMode(display.id) else {
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
fn window_center(window_frame: CoreGraphicsRect) -> (f64, f64) {
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
pub(super) fn display_under(
    displays: &[Display],
    window_frame: CoreGraphicsRect,
) -> Option<&Display> {
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
