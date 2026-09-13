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
use std::time::Instant;

use display::Output;
use display::OutputSelection;
use display::TopologyRead;
use ratatui::style::Color;
use window::ListedWindow;
use zbus::blocking::Connection;

use self::constants::DESKTOP_RETRY_INTERVAL;
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
static SESSION_CONNECTION: Mutex<SessionConnection<Connection>> =
    Mutex::new(SessionConnection::Unconnected);
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

/// Reconstruct the wallpaper for the output holding `chosen`.
fn capture_selected_window(
    metrics: Metrics,
    chosen: &ListedWindow,
    outputs: &[Output],
) -> Result<Desktop, CaptureFailure> {
    let output = match display::under(outputs, chosen.frame) {
        OutputSelection::Containing(output) | OutputSelection::Nearest(output) => output,
        OutputSelection::NoActiveOutputs => return Err(CaptureFailure::DisplayNotFound),
    };
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

/// Access to the held session connection, cloned without opening another socket.
enum SessionBus<C = Connection> {
    /// The bus is available for a desktop query.
    Connected(C),
    /// No usable connection exists yet, or the previously held connection closed.
    Unavailable(ConnectionFailure),
}

/// Why the desktop session bus cannot be accessed.
#[derive(Clone, Debug)]
enum ConnectionFailure {
    /// The session bus has never accepted this process's connection.
    NeverConnected(String),
    /// A previously working connection closed and reconnection failed.
    Disconnected(String),
    /// Another worker panicked while accessing the connection state.
    StatePoisoned,
}

/// Lifetime and retry deadline of the shared desktop session connection.
enum SessionConnection<C> {
    /// No connection attempt has been made.
    Unconnected,
    /// A single connection serves all desktop workers.
    Connected(C),
    /// An unavailable bus is retried independently of the capture cadence.
    Retrying {
        /// Whether the bus has ever connected and the latest error.
        failure:  ConnectionFailure,
        /// Earliest next attempt.
        retry_at: Instant,
    },
}

impl<C: Clone> SessionConnection<C> {
    /// Reuse a live handle, or reconnect once the retry deadline permits it.
    fn access(
        &mut self,
        now: Instant,
        is_closed: impl FnOnce(&C) -> bool,
        connect: impl FnOnce() -> Result<C, String>,
    ) -> SessionBus<C> {
        match self {
            Self::Connected(connection) if !is_closed(connection) => {
                return SessionBus::Connected(connection.clone());
            },
            Self::Retrying { failure, retry_at } if now < *retry_at => {
                return SessionBus::Unavailable(failure.clone());
            },
            _ => {},
        }
        match connect() {
            Ok(connection) => {
                *self = Self::Connected(connection.clone());
                SessionBus::Connected(connection)
            },
            Err(error) => {
                let failure = match self {
                    Self::Unconnected
                    | Self::Retrying {
                        failure: ConnectionFailure::NeverConnected(_),
                        ..
                    } => ConnectionFailure::NeverConnected(error),
                    Self::Connected(_) | Self::Retrying { .. } => {
                        ConnectionFailure::Disconnected(error)
                    },
                };
                *self = Self::Retrying {
                    failure:  failure.clone(),
                    retry_at: now + DESKTOP_RETRY_INTERVAL,
                };
                SessionBus::Unavailable(failure)
            },
        }
    }
}

impl std::fmt::Display for ConnectionFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NeverConnected(error) => write!(formatter, "session bus unavailable: {error}"),
            Self::Disconnected(error) => write!(formatter, "session bus disconnected: {error}"),
            Self::StatePoisoned => formatter.write_str("session connection state poisoned"),
        }
    }
}

/// The process-wide connection to the desktop session bus.
fn session_connection() -> SessionBus {
    let Ok(mut connection) = SESSION_CONNECTION.lock() else {
        return SessionBus::Unavailable(ConnectionFailure::StatePoisoned);
    };
    connection.access(Instant::now(), Connection::is_closed, || {
        Connection::session().map_err(|error| error.to_string())
    })
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

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    #[test]
    fn session_bus_unavailable_retries_only_after_deadline() {
        let mut connection = SessionConnection::<u32>::Unconnected;
        let now = Instant::now();
        let attempts = Cell::new(0);
        let connect = || {
            attempts.set(attempts.get() + 1);
            Err("bus absent".to_owned())
        };
        assert!(
            matches!(connection.access(now, |_| false, connect), SessionBus::Unavailable(ConnectionFailure::NeverConnected(error)) if error == "bus absent")
        );
        assert!(matches!(
            connection.access(now, |_| false, connect),
            SessionBus::Unavailable(ConnectionFailure::NeverConnected(_))
        ));
        assert_eq!(attempts.get(), 1);
        assert!(matches!(
            connection.access(now + DESKTOP_RETRY_INTERVAL, |_| false, connect),
            SessionBus::Unavailable(ConnectionFailure::NeverConnected(_))
        ));
        assert_eq!(attempts.get(), 2);
    }

    #[test]
    fn session_bus_recovered_connection_is_retained() {
        let mut connection = SessionConnection::<u32>::Unconnected;
        let now = Instant::now();
        let attempts = Cell::new(0);
        assert!(matches!(
            connection.access(
                now,
                |_| false,
                || {
                    attempts.set(attempts.get() + 1);
                    Err("bus absent".to_owned())
                }
            ),
            SessionBus::Unavailable(ConnectionFailure::NeverConnected(_))
        ));
        let connect = || {
            attempts.set(attempts.get() + 1);
            Ok(7)
        };
        for _ in 0..60 {
            assert!(matches!(
                connection.access(now + DESKTOP_RETRY_INTERVAL / 2, |_| false, connect),
                SessionBus::Unavailable(ConnectionFailure::NeverConnected(_))
            ));
        }
        assert_eq!(attempts.get(), 1);
        let recovered_at = now + DESKTOP_RETRY_INTERVAL;
        assert!(matches!(
            connection.access(recovered_at, |_| false, connect),
            SessionBus::Connected(7)
        ));
        for _ in 0..60 {
            assert!(matches!(
                connection.access(recovered_at, |_| false, connect),
                SessionBus::Connected(7)
            ));
        }
        assert_eq!(attempts.get(), 2);
    }

    #[test]
    fn session_bus_closed_connection_reports_disconnection_and_recovers() {
        let mut connection = SessionConnection::Connected(7);
        let now = Instant::now();
        let attempts = Cell::new(0);
        assert!(matches!(connection.access(now, |_| true, || {
            attempts.set(attempts.get() + 1);
            Err("bus stopped".to_owned())
        }), SessionBus::Unavailable(ConnectionFailure::Disconnected(error)) if error == "bus stopped"));
        let connect = || {
            attempts.set(attempts.get() + 1);
            Ok(8)
        };
        for _ in 0..60 {
            assert!(matches!(
                connection.access(now + DESKTOP_RETRY_INTERVAL / 2, |_| false, connect),
                SessionBus::Unavailable(ConnectionFailure::Disconnected(_))
            ));
        }
        assert_eq!(attempts.get(), 1);
        assert!(matches!(
            connection.access(now + DESKTOP_RETRY_INTERVAL, |_| false, connect),
            SessionBus::Connected(8)
        ));
        assert_eq!(attempts.get(), 2);
    }

    #[test]
    fn session_bus_closed_connection_can_reconnect_immediately() {
        let mut connection = SessionConnection::Connected(7);
        assert!(matches!(
            connection.access(Instant::now(), |_| true, || Ok(8)),
            SessionBus::Connected(8)
        ));
    }
}
