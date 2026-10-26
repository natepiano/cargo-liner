use super::cached_wallpaper::reconstructed_wallpaper;
use super::display;
use super::display::Output;
use super::display::OutputSelection;
use super::display::TopologyRead;
use super::held_composite::composited_display;
use super::window;
use super::window::ListedWindow;
use crate::backdrop::desktop::CaptureAttemptResult;
use crate::backdrop::desktop::CaptureAttemptSequence;
use crate::backdrop::desktop::CaptureAttemptWindowSelection;
use crate::backdrop::desktop::CaptureFailure;
use crate::backdrop::desktop::CaptureWindowTarget;
use crate::backdrop::desktop::Desktop;
use crate::backdrop::desktop::Metrics;
use crate::backdrop::desktop::candidate;

/// See [`Desktop::capture`].
///
/// What is assembled is only the windows standing below this terminal's own, so it never captures
/// the animation, whatever is on screen when it is taken.
pub(in crate::backdrop::desktop) fn capture(
    metrics: Metrics,
    capture_window_target: CaptureWindowTarget,
    sequence: CaptureAttemptSequence,
) -> CaptureAttemptResult {
    let TopologyRead::Read(outputs) = display::active_outputs() else {
        return failure_before_selection(sequence, CaptureFailure::DisplayNotFound);
    };
    if outputs.is_empty() {
        return failure_before_selection(sequence, CaptureFailure::DisplayNotFound);
    }
    let windows = window::for_capture(capture_window_target);
    if windows.is_empty() {
        return failure_before_selection(sequence, CaptureFailure::TerminalWindowNotFound);
    }
    let terminal_window_candidates = window::candidates(&windows);
    let selected = candidate::select_capture_window(
        &windows,
        capture_window_target,
        &terminal_window_candidates,
        |window| window.handle,
        || {
            window::closest_size_match(&terminal_window_candidates.windows, &outputs, metrics)
                .ok_or(CaptureFailure::TerminalWindowNotFound)
        },
    );
    let Ok((chosen, method)) = selected else {
        return failure_before_selection(sequence, CaptureFailure::TerminalWindowNotFound);
    };
    let window_id = chosen.handle;
    let window_selection = CaptureAttemptWindowSelection::Selected { window_id, method };
    let desktop_result = capture_selected_window(metrics, chosen, &outputs);
    CaptureAttemptResult::from_desktop_result(sequence, window_selection, desktop_result)
}

/// Assemble what stands under `chosen` on its output, or reconstruct that output's wallpaper where
/// no picture can be taken.
fn capture_selected_window(
    metrics: Metrics,
    chosen: &ListedWindow,
    outputs: &[Output],
) -> Result<Desktop, CaptureFailure> {
    let output = match display::under(outputs, chosen.frame) {
        OutputSelection::Containing(output) | OutputSelection::Nearest(output) => output,
        OutputSelection::NoActiveOutputs => return Err(CaptureFailure::DisplayNotFound),
    };
    let cell = metrics.cell_points(output.scale);
    let (columns, rows, colors) = match composited_display(metrics, chosen.handle, output) {
        Some(reduced) => reduced,
        None => reconstructed_wallpaper(metrics, output)?,
    };
    Ok(Desktop {
        window_id: chosen.handle,
        metrics,
        origin: output.origin,
        cell,
        columns,
        rows,
        colors,
    })
}

/// Build a capture failure produced before a terminal window was selected.
const fn failure_before_selection(
    sequence: CaptureAttemptSequence,
    failure: CaptureFailure,
) -> CaptureAttemptResult {
    candidate::capture_failure_before_window_selection(sequence, failure)
}
