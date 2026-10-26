use objc2_core_graphics::CGPreflightScreenCaptureAccess;

use super::constants::ALPHA;
use super::constants::BLUE;
use super::constants::BYTES_PER_PIXEL;
use super::constants::GREEN;
use super::constants::RED;
use super::display;
use super::display::Display;
use super::stream;
use super::window;
use super::window::Listed;
use crate::backdrop::desktop::CaptureAttemptResult;
use crate::backdrop::desktop::CaptureAttemptSequence;
use crate::backdrop::desktop::CaptureAttemptWindowSelection;
use crate::backdrop::desktop::CaptureFailure;
use crate::backdrop::desktop::CaptureWindowTarget;
use crate::backdrop::desktop::Desktop;
use crate::backdrop::desktop::Metrics;
use crate::backdrop::desktop::candidate;
use crate::backdrop::desktop::candidate::TerminalWindowOwner;
use crate::backdrop::desktop::reduction;

/// Whether macOS has granted this process Screen Recording access.
pub(super) fn screen_capture_access_is_granted() -> bool { CGPreflightScreenCaptureAccess() }

/// See [`Desktop::capture`].
///
/// The capture excludes this terminal's own windows, so it never
/// photographs the animation, whatever is on screen when it is taken.
pub(in crate::backdrop::desktop) fn capture(
    metrics: Metrics,
    capture_window_target: CaptureWindowTarget,
    sequence: CaptureAttemptSequence,
) -> CaptureAttemptResult {
    let windows = Listed::on_screen();
    let displays = display::active_displays();
    if displays.is_empty() {
        return candidate::capture_failure_before_window_selection(
            sequence,
            CaptureFailure::DisplayNotFound,
        );
    }
    if windows.is_empty() {
        return candidate::capture_failure_before_window_selection(
            sequence,
            CaptureFailure::TerminalWindowNotFound,
        );
    }
    let terminal_window_candidates = window::terminal_windows(&windows);
    // `capture_window_target` carries the number pinned to this app's own window,
    // which `candidate::select_capture_window` looks for across all `windows`, not inside the
    // `terminal_window_candidates` set the terminal is thought to own. Which windows those are
    // is a question that can be answered wrongly: an emulator
    // hosting its sessions in a server process is nowhere in this
    // app's parent chain, so the answer there is whichever
    // application is in front -- which stops being this one the
    // moment anything is opened over it. Searching the pinned
    // number inside that set loses this window exactly when another
    // application has taken the front, and what is chosen instead
    // is one of *its* windows, on whichever display it sits on.
    //
    // Falling back to `frontmost_window` size matching is for the run where the marker title
    // never took, and for the window closed since it did.
    let selected = candidate::select_capture_window(
        &windows,
        capture_window_target,
        &terminal_window_candidates,
        |window: &Listed| window.number,
        || {
            window::frontmost_window(
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
    let window_id = chosen.number;
    let window_selection = CaptureAttemptWindowSelection::Selected { window_id, method };

    let desktop_result = capture_selected_window(metrics, chosen, &windows, &displays);
    CaptureAttemptResult::from_desktop_result(sequence, window_selection, desktop_result)
}

/// Capture the display behind the terminal window selected for this attempt.
fn capture_selected_window(
    metrics: Metrics,
    chosen: &Listed,
    windows: &[Listed],
    displays: &[Display],
) -> Result<Desktop, CaptureFailure> {
    let window_id = chosen.number;
    let display = CaptureFailure::DisplayNotFound
        .classify_option(display::display_under(displays, chosen.bounds))?;
    let display_frame = display::display_bounds(display);
    // `stream::capture_display_bgra` is asked for the display's point size and
    // comes back at it, so one cell in points serves both the grid
    // the capture is reduced against and the frame the window is
    // placed by. `Display::bounds` measures in points as the selected window's bounds do, which is
    // why there is no second unit here to convert to.
    //
    // What the terminal reports is the one thing here that is not in points -- it answers in the
    // display's pixels -- so `backing_scale(display)` divides it before a cell comes out of it. See
    // [`Metrics::cell_points`].
    let cell = metrics.cell_points(f64::from(display::backing_scale(display)));
    let output_size = (
        CaptureFailure::DisplayCaptureFailed
            .classify_option(points_to_u32(display_frame.size.width))?,
        CaptureFailure::DisplayCaptureFailed
            .classify_option(points_to_u32(display_frame.size.height))?,
    );
    // `stream::capture_display_bgra` uses a persistent ScreenCaptureKit stream -- one per display,
    // opened once and shared -- rather than a fresh in-process screenshot each refresh, which
    // wedges process-to-process on macOS 26 the moment several instances overlap on the window
    // server. The stream's content filter leaves out every window the emulator owns, so what comes
    // back is the desktop the terminal is drawn over rather than the terminal itself. See the
    // [`stream`] submodule.
    let excluded = match chosen.owner {
        TerminalWindowOwner::Application { pid } => windows
            .iter()
            .filter_map(|window| match window.owner {
                TerminalWindowOwner::Application { pid: owner_pid } if owner_pid == pid => {
                    Some(window.number)
                },
                _ => None,
            })
            .collect::<Vec<u32>>(),
        TerminalWindowOwner::Unnamed => vec![chosen.number],
    };
    let (bytes, width, height, stride) = stream::capture_display_bgra(
        display.id,
        output_size,
        &excluded,
        screen_capture_access_is_granted(),
    )?;
    let rgba = CaptureFailure::PixelExtractionFailed
        .classify_option(bgra_rows_to_rgba(&bytes, width, height, stride))?;
    let image = (
        u32::try_from(width).map_err(|_| CaptureFailure::DisplayCaptureFailed)?,
        u32::try_from(height).map_err(|_| CaptureFailure::DisplayCaptureFailed)?,
    );
    let (columns, rows, colors) = reduction::reduce_capture(&rgba, image, cell)?;
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

/// Round a display's point dimension to the `u32` the capture stream configures its output size
/// in.
///
/// [`None`] for a dimension no `u32` can hold, or one that is not a finite, non-negative length --
/// neither is a size a capture could be asked for.
fn points_to_u32(value: f64) -> Option<u32> {
    if !(value.is_finite() && value >= 0.0 && value <= f64::from(u32::MAX)) {
        return None;
    }
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "bounds-checked, rounded point dimension"
    )]
    let rounded = value.round() as u32;
    Some(rounded)
}

/// Convert padded BGRA rows from CoreGraphics into tightly packed RGBA pixels.
fn bgra_rows_to_rgba(bytes: &[u8], width: usize, height: usize, stride: usize) -> Option<Vec<u8>> {
    let row_bytes = width.checked_mul(BYTES_PER_PIXEL)?;
    let required_bytes = height.checked_mul(stride)?;
    let capacity = height.checked_mul(row_bytes)?;
    if stride < row_bytes || bytes.len() < required_bytes {
        return None;
    }

    let mut rgba = Vec::with_capacity(capacity);
    for row in 0..height {
        let start = row.checked_mul(stride)?;
        let end = start.checked_add(row_bytes)?;
        for pixel in bytes.get(start..end)?.as_chunks::<BYTES_PER_PIXEL>().0 {
            rgba.extend_from_slice(&[pixel[BLUE], pixel[GREEN], pixel[RED], pixel[ALPHA]]);
        }
    }
    Some(rgba)
}

impl CaptureFailure {
    /// Require a stage to have produced its value, classifying absence as this failure.
    fn classify_option<T>(self, value: Option<T>) -> Result<T, Self> { value.ok_or(self) }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    #[test]
    fn bgra_rows_become_tight_rgba_without_padding() {
        let bytes = [
            3, 2, 1, 4, 7, 6, 5, 8, 101, 102, 103, 104, 105, 106, 107, 108, 11, 10, 9, 12, 15, 14,
            13, 16, 109, 110, 111, 112, 113, 114, 115, 116,
        ];

        let rgba = super::bgra_rows_to_rgba(&bytes, 2, 2, 16);

        assert_eq!(
            rgba,
            Some(vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16])
        );
    }

    #[test]
    fn bgra_rows_reject_input_shorter_than_the_padded_image() {
        let bytes = [0_u8; 31];

        assert_eq!(super::bgra_rows_to_rgba(&bytes, 2, 2, 16), None);
    }
}
