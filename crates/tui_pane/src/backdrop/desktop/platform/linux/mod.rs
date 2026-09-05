//! The KDE Wayland wallpaper backend.
//!
//! `kdotool` supplies `KWin` window UUIDs, `KWin` supplies current window geometry over D-Bus, and
//! `kscreen-doctor` supplies the logical output layout. Plasma's wallpaper configuration is read
//! over D-Bus and rendered at the selected output's coordinates.

mod constants;
mod display;
mod wallpaper;
mod window;

use std::sync::Mutex;
use std::sync::OnceLock;

use display::Output;
use ratatui::style::Color;
use window::ListedWindow;
use zbus::blocking::Connection;

use self::wallpaper::WallpaperSnapshot;
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
use crate::backdrop::desktop::candidate;
use crate::backdrop::desktop::reduction;

/// The shared session-bus connection used by the capture and position workers.
static SESSION_CONNECTION: OnceLock<Connection> = OnceLock::new();
/// The last reduced wallpaper grid, reused while its inputs remain unchanged.
static WALLPAPER_CACHE: Mutex<Option<CachedWallpaper>> = Mutex::new(None);

/// Inputs that determine the reduced wallpaper grid.
#[derive(Clone, Eq, PartialEq)]
struct WallpaperCacheKey {
    /// Terminal geometry used to size each color cell.
    metrics:    Metrics,
    /// Physical dimensions of the output.
    output:     (u32, u32),
    /// The output scale encoded without floating-point equality.
    scale_bits: u64,
    /// Plasma wallpaper settings and source-file timestamp.
    wallpaper:  WallpaperSnapshot,
}

/// A wallpaper already reduced to terminal-sized color cells.
struct CachedWallpaper {
    /// Inputs that produced this grid.
    key:    WallpaperCacheKey,
    /// Cells across and down.
    grid:   (u16, u16),
    /// Row-major colors for the grid.
    colors: Vec<Color>,
}

/// See [`Desktop::capture`].
pub(in crate::backdrop::desktop) fn capture(
    metrics: Metrics,
    capture_window_target: CaptureWindowTarget,
    sequence: CaptureAttemptSequence,
) -> CaptureAttemptResult {
    let outputs = display::active_outputs();
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

/// Reconstruct the wallpaper for the output holding `chosen`.
fn capture_selected_window(
    metrics: Metrics,
    chosen: &ListedWindow,
    outputs: &[Output],
) -> Result<Desktop, CaptureFailure> {
    let output = display::under(outputs, chosen.frame).ok_or(CaptureFailure::DisplayNotFound)?;
    let wallpaper = wallpaper::snapshot(output.screen_index, output.size)
        .ok_or(CaptureFailure::DisplayCaptureFailed)?;
    let cell = metrics.cell_points(output.scale);
    let reduction_cell = metrics.cell_points(1.0);
    let (columns, rows, colors) = reduced_wallpaper(metrics, output, wallpaper, reduction_cell)?;
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

/// Return a cached color grid or render and reduce a new one.
fn reduced_wallpaper(
    metrics: Metrics,
    output: &Output,
    wallpaper: WallpaperSnapshot,
    cell: (f64, f64),
) -> Result<(u16, u16, Vec<Color>), CaptureFailure> {
    let key = WallpaperCacheKey {
        metrics,
        output: output.size,
        scale_bits: output.scale.to_bits(),
        wallpaper,
    };
    if let Ok(cache) = WALLPAPER_CACHE.lock()
        && let Some(cached) = cache.as_ref()
        && cached.key == key
    {
        return Ok((cached.grid.0, cached.grid.1, cached.colors.clone()));
    }
    let image = key
        .wallpaper
        .render(key.output)
        .ok_or(CaptureFailure::DisplayCaptureFailed)?;
    let reduced = reduction::reduce_capture(image.as_raw(), key.output, cell)?;
    if let Ok(mut cache) = WALLPAPER_CACHE.lock() {
        *cache = Some(CachedWallpaper {
            key,
            grid: (reduced.0, reduced.1),
            colors: reduced.2.clone(),
        });
    }
    Ok(reduced)
}

/// Build a capture failure produced before a terminal window was selected.
const fn failure_before_selection(
    sequence: CaptureAttemptSequence,
    failure: CaptureFailure,
) -> CaptureAttemptResult {
    candidate::capture_failure_before_window_selection(sequence, failure)
}

/// The process-wide connection to the desktop session bus.
fn session_connection() -> Option<&'static Connection> {
    if let Some(connection) = SESSION_CONNECTION.get() {
        return Some(connection);
    }
    let connection = Connection::session().ok()?;
    let _ = SESSION_CONNECTION.set(connection);
    SESSION_CONNECTION.get()
}

/// See [`crate::backdrop::desktop::window_frame`].
pub(in crate::backdrop::desktop) fn window_frame(window: u32) -> Option<Frame> {
    self::window::frame(window)
}

/// See [`crate::backdrop::desktop::window_titles`].
pub(in crate::backdrop::desktop) fn window_titles() -> Vec<TitledWindow> { self::window::titles() }

/// See [`crate::backdrop::desktop::window_titled`].
pub(in crate::backdrop::desktop) fn window_titled(marker: &str) -> TerminalWindowSearchOutcome {
    self::window::titled(marker)
}

/// See [`crate::backdrop::desktop::window_at`].
pub(in crate::backdrop::desktop) fn window_at(origin: (f64, f64)) -> TerminalWindowSearchOutcome {
    self::window::at(origin)
}
