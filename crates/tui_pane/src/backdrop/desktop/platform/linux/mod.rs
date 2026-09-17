//! The KDE Wayland backend.
//!
//! `kdotool` supplies `KWin` window UUIDs, `KWin` supplies current window geometry over D-Bus, and
//! `kscreen-doctor` supplies the logical output layout.
//!
//! What stands underneath the terminal is assembled rather than photographed: [`compose`] reads
//! `KWin`'s stacking order and draws every window below this one over the display's own desktop
//! window, so what the animation is drawn from is the monitor as it stands, other windows and all,
//! without this terminal in it. Where no picture can be taken -- `KWin` refusing the screenshot
//! interface, a layout that moved under the read, a stack this window is not in -- Plasma's
//! wallpaper configuration is read over D-Bus and rendered at the selected output's coordinates
//! instead, which is what this backend did for every capture before.

mod compose;
mod constants;
mod display;
mod wallpaper;
mod window;

use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;
use std::io;
use std::io::ErrorKind;
use std::io::Read;
use std::ops::ControlFlow;
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::process::Child;
use std::process::Command;
use std::process::Stdio;
use std::sync::Mutex;
use std::thread;
use std::time::Duration;
use std::time::Instant;

use compose::Layout;
use display::Output;
use display::OutputSelection;
use display::TopologyRead;
use ratatui::style::Color;
use window::ListedWindow;
use zbus::blocking::Connection;

use self::constants::COMPOSITE_HOLD;
use self::constants::DESKTOP_READ_POLL_INTERVAL;
use self::constants::DESKTOP_RETRY_INTERVAL;
use self::constants::DESKTOP_STDOUT_CHUNK_BYTES;
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
/// The composite the animation is being drawn from, held until a frame that draws nothing over
/// the grid may replace it.
static HELD_COMPOSITE: Mutex<Option<HeldComposite>> = Mutex::new(None);

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

/// Inputs that decide whether a held composite still describes the display.
#[derive(Clone, Eq, PartialEq)]
struct CompositeKey {
    /// Terminal geometry used to size each colour cell.
    metrics:     Metrics,
    /// The output's top-left logical coordinate, encoded without floating-point equality.
    origin_bits: (u64, u64),
    /// Physical dimensions of the output.
    output:      (u32, u32),
    /// The output scale encoded without floating-point equality.
    scale_bits:  u64,
    /// The windows the picture was assembled from, and where they stood.
    layout:      Layout,
}

impl CompositeKey {
    /// The key one output and one arrangement of windows answer to.
    const fn of(metrics: Metrics, output: &Output, layout: Layout) -> Self {
        Self {
            metrics,
            origin_bits: (output.origin.0.to_bits(), output.origin.1.to_bits()),
            output: output.size,
            scale_bits: output.scale.to_bits(),
            layout,
        }
    }
}

/// One composite of the display, already reduced to terminal-sized colour cells.
struct HeldComposite {
    /// Inputs that produced this grid.
    key:      CompositeKey,
    /// Cells across and down.
    grid:     (u16, u16),
    /// Row-major colours for the grid.
    colors:   Vec<Color>,
    /// When the picture was assembled.
    taken_at: Instant,
}

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

/// Return the composite already held, or assemble and reduce a new one.
///
/// The stacking order is read every time, because it is what says whether the picture already
/// held still describes the display and it costs a few milliseconds against the round trip per
/// window a picture costs.
///
/// [`None`] wherever nothing can be captured, which leaves the caller reconstructing the
/// wallpaper.
fn composited_display(
    metrics: Metrics,
    handle: u32,
    output: &Output,
) -> Option<(u16, u16, Vec<Color>)> {
    let uuid = window::uuid_of(handle)?;
    let layout = compose::layout_below(&uuid, output)?;
    let key = CompositeKey::of(metrics, output, layout.clone());
    let now = Instant::now();
    if let Ok(held) = HELD_COMPOSITE.lock()
        && let Some(held) = held.as_ref()
        && !composite_due(held, &key, now)
    {
        return Some((held.grid.0, held.grid.1, held.colors.clone()));
    }
    let composite = layout.capture()?;
    let cell = metrics.cell_points(output.scale / composite.ratio);
    let reduced =
        reduction::reduce_capture(composite.image.as_raw(), composite.image.dimensions(), cell)
            .ok()?;
    if let Ok(mut held) = HELD_COMPOSITE.lock() {
        *held = Some(HeldComposite {
            key,
            grid: (reduced.0, reduced.1),
            colors: reduced.2.clone(),
            taken_at: now,
        });
    }
    Some(reduced)
}

/// Whether a new composite is due.
///
/// Assembling one costs a capture of every window standing under this one, so it is not done per
/// frame: a composite stands for [`COMPOSITE_HOLD`] unless the display, the terminal's own
/// geometry, or the arrangement of the windows it was drawn from has changed under it, all of
/// which the key carries.
fn composite_due(held: &HeldComposite, key: &CompositeKey, now: Instant) -> bool {
    held.key != *key || now.duration_since(held.taken_at) >= COMPOSITE_HOLD
}

/// Render Plasma's configured wallpaper for one output and reduce it.
fn reconstructed_wallpaper(
    metrics: Metrics,
    output: &Output,
) -> Result<(u16, u16, Vec<Color>), CaptureFailure> {
    let wallpaper = wallpaper::snapshot(output.screen_index, output.size)
        .ok_or(CaptureFailure::DisplayCaptureFailed)?;
    reduced_wallpaper(metrics, output, wallpaper, metrics.cell_points(1.0))
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

impl Display for ConnectionFailure {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
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

/// Why a desktop subprocess could not supply its stdout.
#[derive(Clone, Debug, Eq, PartialEq)]
enum DesktopReadFailure {
    /// The child or its stdout did not finish before the shared deadline.
    Expired,
    /// Starting, reading, or completing the command failed.
    Failed(String),
}

/// Read one desktop command with the same cleanup for every backend, bounded by `deadline`.
fn read_desktop_command(
    command: &mut Command,
    deadline: Duration,
) -> Result<Vec<u8>, DesktopReadFailure> {
    let deadline = Instant::now() + deadline;
    let mut process = DesktopSubprocess::spawn(command)
        .map_err(|error| DesktopReadFailure::Failed(error.to_string()))?;
    read_desktop_process(&mut process, |delay| wait_for_desktop_read(deadline, delay))
}

/// Poll without holding the desktop state lock, terminating failed or expired reads before retry.
fn read_desktop_process(
    process: &mut impl DesktopProcess,
    mut wait: impl FnMut(Duration) -> ControlFlow<()>,
) -> Result<Vec<u8>, DesktopReadFailure> {
    let failure = loop {
        match process.try_read() {
            Ok(ControlFlow::Break(read)) => return Ok(read),
            Ok(ControlFlow::Continue(())) => {},
            Err(error) => break DesktopReadFailure::Failed(error.to_string()),
        }
        if wait(DESKTOP_READ_POLL_INTERVAL).is_break() {
            break DesktopReadFailure::Expired;
        }
    };
    let _ = process.kill();
    let _ = process.reap();
    Err(failure)
}

/// Stop at the deadline, including when the final polling sleep consumes its remaining time.
fn wait_for_desktop_read(deadline: Instant, delay: Duration) -> ControlFlow<()> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if !remaining.is_zero() {
        thread::sleep(delay.min(remaining));
    }
    if Instant::now() >= deadline {
        ControlFlow::Break(())
    } else {
        ControlFlow::Continue(())
    }
}

/// Subprocess operations used by a desktop read and its deterministic expiry tests.
trait DesktopProcess {
    /// Read available stdout and report a completed stdout only after the child has exited.
    fn try_read(&mut self) -> io::Result<ControlFlow<Vec<u8>>>;

    /// Stop a child whose read cannot complete.
    fn kill(&mut self) -> io::Result<()>;

    /// Collect the child status after stopping it, even if killing it failed.
    fn reap(&mut self) -> io::Result<()>;
}

/// One desktop child and its stdout, drained without blocking the deadline check.
struct DesktopSubprocess {
    /// The child is reaped by completion polling or by termination on failure.
    child:  Child,
    /// A socket permits safe nonblocking reads without adding a platform dependency.
    stdout: UnixStream,
    /// Bytes accumulated before the child finishes and stdout is drained.
    bytes:  Vec<u8>,
}

impl DesktopSubprocess {
    /// Connect stdout before spawning so every setup failure leaves no child running.
    fn spawn(command: &mut Command) -> io::Result<Self> {
        let (stdout, writer) = UnixStream::pair()?;
        stdout.set_nonblocking(true)?;
        let child = command
            .stdin(Stdio::null())
            .stdout(Stdio::from(OwnedFd::from(writer)))
            .stderr(Stdio::null())
            .spawn()?;
        Ok(Self {
            child,
            stdout,
            bytes: Vec::new(),
        })
    }
}

impl DesktopProcess for DesktopSubprocess {
    fn try_read(&mut self) -> io::Result<ControlFlow<Vec<u8>>> {
        // Observe exit first so a final write between polling and reading cannot be missed.
        let status = self.child.try_wait()?;
        let mut buffer = [0; DESKTOP_STDOUT_CHUNK_BYTES];
        let count = match self.stdout.read(&mut buffer) {
            Ok(count) => count,
            Err(error) if error.kind() == ErrorKind::WouldBlock => 0,
            Err(error) if error.kind() == ErrorKind::Interrupted => {
                return Ok(ControlFlow::Continue(()));
            },
            Err(error) => return Err(error),
        };
        self.bytes.extend_from_slice(&buffer[..count]);
        match status {
            Some(status) if !status.success() => Err(io::Error::other(format!(
                "desktop command exited with {status}"
            ))),
            Some(_) if count == 0 => Ok(ControlFlow::Break(std::mem::take(&mut self.bytes))),
            Some(_) | None => Ok(ControlFlow::Continue(())),
        }
    }

    fn kill(&mut self) -> io::Result<()> { self.child.kill() }

    fn reap(&mut self) -> io::Result<()> { self.child.wait().map(|_| ()) }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    /// A subprocess whose completion, read error, and cleanup can be checked without spawning.
    struct ScriptedDesktopProcess {
        completion:  io::Result<ControlFlow<Vec<u8>>>,
        calls:       Vec<&'static str>,
        kill_result: io::Result<()>,
    }

    impl DesktopProcess for ScriptedDesktopProcess {
        fn try_read(&mut self) -> io::Result<ControlFlow<Vec<u8>>> {
            self.calls.push("poll");
            std::mem::replace(&mut self.completion, Ok(ControlFlow::Continue(())))
        }

        fn kill(&mut self) -> io::Result<()> {
            self.calls.push("kill");
            std::mem::replace(&mut self.kill_result, Ok(()))
        }

        fn reap(&mut self) -> io::Result<()> {
            self.calls.push("reap");
            Ok(())
        }
    }

    #[test]
    fn desktop_deadline_kills_and_reaps_even_when_kill_fails() {
        for kill_result in [Ok(()), Err(io::Error::other("already exited"))] {
            let mut process = ScriptedDesktopProcess {
                completion: Ok(ControlFlow::Continue(())),
                calls: Vec::new(),
                kill_result,
            };
            let deadline = Instant::now();
            let result = read_desktop_process(&mut process, |delay| {
                assert_eq!(delay, DESKTOP_READ_POLL_INTERVAL);
                wait_for_desktop_read(deadline, delay)
            });
            assert_eq!(result, Err(DesktopReadFailure::Expired));
            assert_eq!(process.calls, vec!["poll", "kill", "reap"]);
        }
    }

    #[test]
    fn failed_desktop_read_kills_and_reaps_before_returning() {
        let mut process = ScriptedDesktopProcess {
            completion:  Err(io::Error::other("read failed")),
            calls:       Vec::new(),
            kill_result: Ok(()),
        };
        let result = read_desktop_process(&mut process, |_| ControlFlow::Break(()));
        assert_eq!(
            result,
            Err(DesktopReadFailure::Failed("read failed".to_owned()))
        );
        assert_eq!(process.calls, vec!["poll", "kill", "reap"]);
    }

    #[test]
    fn completed_desktop_read_returns_stdout_without_termination() {
        let mut process = ScriptedDesktopProcess {
            completion:  Ok(ControlFlow::Break(b"{terminal-uuid}\n".to_vec())),
            calls:       Vec::new(),
            kill_result: Ok(()),
        };
        let result = read_desktop_process(&mut process, |_| ControlFlow::Break(()));
        assert_eq!(result, Ok(b"{terminal-uuid}\n".to_vec()));
        assert_eq!(process.calls, vec!["poll"]);
    }

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
