//! Reading KDE's active output layout.

use std::collections::HashMap;
use std::ops::ControlFlow;
use std::process::Command;
use std::sync::Mutex;
use std::sync::Once;
use std::thread;
use std::time::Duration;

use futures_lite::StreamExt;
use serde::Deserialize;
use zbus::MatchRule;
use zbus::blocking::MessageIterator;
use zbus::blocking::Proxy;
use zbus::message::Type;
use zbus::zvariant::OwnedValue;

use super::SessionBus;
use super::constants::DBUS_OWNER_CHANGED_SIGNAL;
use super::constants::DBUS_SERVICE;
use super::constants::DESKTOP_RETRY_INTERVAL;
use super::constants::KSCREEN_CHANGE_SIGNAL;
use super::constants::KSCREEN_COMMAND;
use super::constants::KSCREEN_INTERFACE;
use super::constants::KSCREEN_JSON_ARGUMENT;
use super::constants::KSCREEN_LOADER_PATH;
use super::constants::KSCREEN_PATH;
use super::constants::KSCREEN_REQUEST_BACKEND;
use super::constants::KSCREEN_SERVICE;
use super::constants::TOPOLOGY_READ_DEADLINE;
use super::constants::TOPOLOGY_SIGNAL_CAPACITY;
use super::read_desktop_command;
use super::session_connection;
use crate::backdrop::desktop::Frame;

/// One enabled `KScreen` output.
#[derive(Clone, Debug)]
pub(super) struct Output {
    /// Where the output begins in `KWin`'s logical coordinate space.
    pub(super) origin:       (f64, f64),
    /// Plasma's index for the enabled screen.
    pub(super) screen_index: u32,
    /// Physical pixel dimensions used to render the wallpaper.
    pub(super) size:         (u32, u32),
    /// Physical pixels per `KWin` logical coordinate.
    pub(super) scale:        f64,
}

impl Output {
    /// The output's logical width and height.
    pub(super) fn logical_size(&self) -> (f64, f64) {
        (
            f64::from(self.size.0) / self.scale,
            f64::from(self.size.1) / self.scale,
        )
    }
}

/// The portion of `kscreen-doctor -j` used by the capture backend.
#[derive(Deserialize)]
struct KScreenDocument {
    /// Every connected and disconnected output known to `KScreen`.
    outputs: Vec<KScreenOutput>,
}

/// Geometry and state for one `KScreen` output.
#[derive(Deserialize)]
struct KScreenOutput {
    /// Whether the connector currently participates in the desktop.
    connected: bool,
    /// Whether the output is currently enabled.
    enabled:   bool,
    /// Its top-left logical coordinate.
    pos:       KScreenPoint,
    /// `KScreen`'s rotation flag.
    rotation:  u32,
    /// Physical pixels per logical coordinate.
    scale:     f64,
    /// Current physical pixel dimensions.
    size:      KScreenSize,
}

/// An integer `KScreen` coordinate.
#[derive(Deserialize)]
struct KScreenPoint {
    /// Horizontal coordinate.
    x: i32,
    /// Vertical coordinate.
    y: i32,
}

/// A physical `KScreen` extent.
#[derive(Deserialize)]
struct KScreenSize {
    /// Height in pixels.
    height: u32,
    /// Width in pixels.
    width:  u32,
}

/// The most recent topology read, preserving a valid desktop with no active outputs.
#[derive(Clone, Debug)]
pub(super) enum TopologyRead {
    /// The backend supplied a valid layout, possibly empty.
    Read(Vec<Output>),
    /// The layout could not be obtained or decoded.
    Unreadable,
}

/// The lifetime of the process-wide display layout and its notification subscription.
#[derive(Debug)]
enum DisplayTopology {
    /// The topology worker has not completed its first attempt.
    Unread,
    /// The layout is current and a held connection watches for changes.
    Watching(Vec<Output>),
    /// The watch or read failed; retain the last successful layout during retry.
    Recovering(TopologyRead),
}

impl DisplayTopology {
    /// Read only on startup, a layout notification, or subscription recovery.
    fn refresh(&mut self, read: impl FnOnce() -> TopologyRead) {
        *self = match read() {
            TopologyRead::Read(outputs) => Self::Watching(outputs),
            TopologyRead::Unreadable => Self::Recovering(TopologyRead::Unreadable),
        };
    }

    /// Retain the last layout when the notification connection ends.
    fn interrupted(&mut self) { *self = Self::Recovering(self.snapshot()); }

    /// A capture tick reads held geometry without querying `KScreen`.
    fn snapshot(&self) -> TopologyRead {
        match self {
            Self::Unread => TopologyRead::Unreadable,
            Self::Watching(outputs) => TopologyRead::Read(outputs.clone()),
            Self::Recovering(read) => read.clone(),
        }
    }
}

/// Held topology shared by the capture worker and the notification worker.
static TOPOLOGY: Mutex<DisplayTopology> = Mutex::new(DisplayTopology::Unread);
/// Start exactly one topology subscription for the process lifetime.
static TOPOLOGY_WORKER: Once = Once::new();

/// The held output layout; capture ticks never spawn the topology reader.
pub(super) fn active_outputs() -> TopologyRead {
    TOPOLOGY_WORKER.call_once(|| {
        thread::spawn(watch_topology);
    });
    TOPOLOGY
        .lock()
        .map_or(TopologyRead::Unreadable, |topology| topology.snapshot())
}

/// Refresh the topology on notifications, retrying only after backend or read failure.
fn watch_topology() {
    retry_topology_watch(&TOPOLOGY, watch_connected_topology, |delay| {
        thread::sleep(delay);
        ControlFlow::Continue(())
    });
}

/// Retry an interrupted subscription while preserving its last readable topology.
fn retry_topology_watch(
    topology: &Mutex<DisplayTopology>,
    mut watch: impl FnMut() -> Result<(), String>,
    mut wait: impl FnMut(Duration) -> ControlFlow<()>,
) {
    loop {
        if let Err(error) = watch() {
            tracing::debug!(%error, "display topology watch interrupted");
        }
        if let Ok(mut topology) = topology.lock() {
            topology.interrupted();
        }
        if wait(DESKTOP_RETRY_INTERVAL).is_break() {
            break;
        }
    }
}

/// Hold filtered layout and service-owner streams until either subscription is interrupted.
fn watch_connected_topology() -> Result<(), String> {
    let connection = match session_connection() {
        SessionBus::Connected(connection) => connection,
        SessionBus::Unavailable(failure) => return Err(failure.to_string()),
    };
    let changes = MatchRule::builder()
        .msg_type(Type::Signal)
        .sender(KSCREEN_SERVICE)
        .and_then(|rule| rule.path(KSCREEN_PATH))
        .and_then(|rule| rule.interface(KSCREEN_INTERFACE))
        .and_then(|rule| rule.member(KSCREEN_CHANGE_SIGNAL))
        .map_err(|error| error.to_string())?
        .build();
    let owners = MatchRule::builder()
        .msg_type(Type::Signal)
        .sender(DBUS_SERVICE)
        .and_then(|rule| rule.interface(DBUS_SERVICE))
        .and_then(|rule| rule.member(DBUS_OWNER_CHANGED_SIGNAL))
        .and_then(|rule| rule.add_arg(KSCREEN_SERVICE))
        .map_err(|error| error.to_string())?
        .build();
    let mut changes =
        MessageIterator::for_match_rule(changes, &connection, Some(TOPOLOGY_SIGNAL_CAPACITY))
            .map_err(|error| error.to_string())?
            .into_inner();
    let mut owners =
        MessageIterator::for_match_rule(owners, &connection, Some(TOPOLOGY_SIGNAL_CAPACITY))
            .map_err(|error| error.to_string())?
            .into_inner();
    let loader = Proxy::new(
        &connection,
        KSCREEN_SERVICE,
        KSCREEN_LOADER_PATH,
        KSCREEN_SERVICE,
    )
    .map_err(|error| error.to_string())?;
    let reply = loader
        .call_method(
            KSCREEN_REQUEST_BACKEND,
            &("", HashMap::<String, OwnedValue>::new()),
        )
        .map_err(|error| error.to_string())?;
    let backend_available: bool = reply
        .body()
        .deserialize()
        .map_err(|error| error.to_string())?;
    if !backend_available {
        return Err("KScreen refused to load its display backend".to_owned());
    }
    let owner = reply
        .header()
        .sender()
        .ok_or_else(|| "KScreen backend reply has no sender".to_owned())?
        .to_string();
    // Subscribe before reading so a layout change during the subprocess cannot be lost.
    let notifications = std::iter::from_fn(|| {
        Some(futures_lite::future::block_on(futures_lite::future::or(
            async {
                match changes.next().await {
                    Some(Ok(_)) => TopologyNotification::Changed,
                    Some(Err(_)) | None => TopologyNotification::Interrupted,
                }
            },
            async {
                while let Some(Ok(message)) = owners.next().await {
                    let Ok((_, _, new_owner)) =
                        message.body().deserialize::<(String, String, String)>()
                    else {
                        break;
                    };
                    // Ignore the activation signal for the owner that answered requestBackend.
                    if new_owner != owner {
                        break;
                    }
                }
                TopologyNotification::Interrupted
            },
        )))
    });
    follow_topology(&TOPOLOGY, notifications, read_outputs)
}

/// Read at startup and on change, retaining geometry when the notification stream ends.
fn follow_topology(
    topology: &Mutex<DisplayTopology>,
    notifications: impl IntoIterator<Item = TopologyNotification>,
    mut read: impl FnMut() -> TopologyRead,
) -> Result<(), String> {
    refresh_topology(topology, &mut read)?;
    for notification in notifications {
        match notification {
            TopologyNotification::Changed => refresh_topology(topology, &mut read)?,
            TopologyNotification::Interrupted => break,
        }
    }
    topology
        .lock()
        .map_err(|error| error.to_string())?
        .interrupted();
    Ok(())
}

/// A layout notification or loss of continuity in the backend subscription.
enum TopologyNotification {
    /// `KScreen` published a new configuration.
    Changed,
    /// The bus or `KScreen` owner ended its subscription lifetime.
    Interrupted,
}

/// Publish a newly read layout without holding the state lock during subprocess execution.
fn refresh_topology(
    topology: &Mutex<DisplayTopology>,
    read: &mut impl FnMut() -> TopologyRead,
) -> Result<(), String> {
    let read = read();
    let mut topology = topology.lock().map_err(|error| error.to_string())?;
    topology.refresh(|| read);
    match *topology {
        DisplayTopology::Watching(_) => Ok(()),
        DisplayTopology::Unread | DisplayTopology::Recovering(_) => {
            Err("KScreen topology could not be read".to_owned())
        },
    }
}

/// Read a layout once; failures are distinct from a successfully decoded empty layout.
fn read_outputs() -> TopologyRead {
    read_desktop_command(
        Command::new(KSCREEN_COMMAND).arg(KSCREEN_JSON_ARGUMENT),
        TOPOLOGY_READ_DEADLINE,
    )
    .map_or(TopologyRead::Unreadable, |bytes| parse_outputs(&bytes))
}

/// Decode active output geometry at the external JSON boundary.
fn parse_outputs(bytes: &[u8]) -> TopologyRead {
    let Ok(document) = serde_json::from_slice::<KScreenDocument>(bytes) else {
        return TopologyRead::Unreadable;
    };
    let mut outputs = Vec::new();
    for raw in document.outputs {
        if !(raw.connected && raw.enabled && raw.scale.is_finite() && raw.scale > 0.0) {
            continue;
        }
        let size = if matches!(raw.rotation, 2 | 8) {
            (raw.size.height, raw.size.width)
        } else {
            (raw.size.width, raw.size.height)
        };
        if size.0 == 0 || size.1 == 0 {
            continue;
        }
        let Ok(screen_index) = u32::try_from(outputs.len()) else {
            break;
        };
        outputs.push(Output {
            origin: (f64::from(raw.pos.x), f64::from(raw.pos.y)),
            screen_index,
            size,
            scale: raw.scale,
        });
    }
    TopologyRead::Read(outputs)
}

/// The output holding the centre of a window, or the nearest output when the centre is off-screen.
pub(super) fn under(outputs: &[Output], frame: Frame) -> OutputSelection<'_> {
    let center = (
        frame.origin.0 + frame.size.0 / 2.0,
        frame.origin.1 + frame.size.1 / 2.0,
    );
    if let Some(output) = outputs.iter().find(|output| holds(output, center)) {
        return OutputSelection::Containing(output);
    }
    outputs
        .iter()
        .min_by(|left, right| {
            distance_squared(left, center).total_cmp(&distance_squared(right, center))
        })
        .map_or(OutputSelection::NoActiveOutputs, OutputSelection::Nearest)
}

/// Why an output was selected for a terminal window.
pub(super) enum OutputSelection<'a> {
    /// The output contains the window's centre.
    Containing(&'a Output),
    /// An off-screen window uses the output with the nearest centre.
    Nearest(&'a Output),
    /// The successfully read layout has no active output.
    NoActiveOutputs,
}

/// Whether an output contains a logical coordinate.
fn holds(output: &Output, point: (f64, f64)) -> bool {
    let size = output.logical_size();
    point.0 >= output.origin.0
        && point.0 < output.origin.0 + size.0
        && point.1 >= output.origin.1
        && point.1 < output.origin.1 + size.1
}

/// Squared distance from a point to the centre of an output.
fn distance_squared(output: &Output, point: (f64, f64)) -> f64 {
    let size = output.logical_size();
    let x = output.origin.0 + size.0 / 2.0 - point.0;
    let y = output.origin.1 + size.1 / 2.0 - point.1;
    x.mul_add(x, y * y)
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::cell::RefCell;

    use super::*;

    /// A high-density output beginning at the desktop origin.
    const OUTPUT: Output = Output {
        origin:       (0.0, 0.0),
        screen_index: 0,
        size:         (3840, 2160),
        scale:        2.0,
    };

    #[test]
    fn output_containment_uses_logical_dimensions() {
        assert!(holds(&OUTPUT, (1919.0, 1079.0)));
        assert!(!holds(&OUTPUT, (1920.0, 1080.0)));
    }

    #[test]
    fn topology_startup_reads_once_and_capture_ticks_reuse_it() {
        let reads = Cell::new(0);
        let mut topology = DisplayTopology::Unread;
        assert!(matches!(topology.snapshot(), TopologyRead::Unreadable));
        topology.refresh(|| {
            reads.set(reads.get() + 1);
            TopologyRead::Read(vec![OUTPUT])
        });
        for _ in 0..60 {
            assert!(
                matches!(topology.snapshot(), TopologyRead::Read(outputs) if outputs.len() == 1)
            );
        }
        assert_eq!(reads.get(), 1);
        assert!(matches!(topology, DisplayTopology::Watching(_)));
    }

    #[test]
    fn topology_change_replaces_geometry_and_recovery_reads_again() {
        let reads = Cell::new(0);
        let mut topology = DisplayTopology::Unread;
        let read = || {
            reads.set(reads.get() + 1);
            TopologyRead::Read(vec![OUTPUT])
        };
        topology.refresh(read);
        topology.refresh(|| {
            reads.set(reads.get() + 1);
            TopologyRead::Read(Vec::new())
        });
        assert!(matches!(topology, DisplayTopology::Watching(ref outputs) if outputs.is_empty()));
        topology.interrupted();
        assert!(
            matches!(topology, DisplayTopology::Recovering(TopologyRead::Read(ref outputs)) if outputs.is_empty())
        );
        topology.refresh(read);
        assert!(matches!(topology, DisplayTopology::Watching(ref outputs) if outputs.len() == 1));
        assert_eq!(reads.get(), 3);
    }

    #[test]
    fn topology_unreadable_startup_and_failed_change_recover() {
        let mut topology = DisplayTopology::Unread;
        topology.refresh(|| TopologyRead::Unreadable);
        assert!(matches!(
            topology,
            DisplayTopology::Recovering(TopologyRead::Unreadable)
        ));
        topology.refresh(|| TopologyRead::Read(vec![OUTPUT]));
        topology.interrupted();
        assert!(matches!(topology.snapshot(), TopologyRead::Read(outputs) if outputs.len() == 1));
        topology.refresh(|| TopologyRead::Unreadable);
        assert!(matches!(topology.snapshot(), TopologyRead::Unreadable));
        topology.refresh(|| TopologyRead::Read(vec![OUTPUT]));
        assert!(matches!(topology, DisplayTopology::Watching(_)));
    }

    #[test]
    fn topology_empty_document_is_distinct_from_unreadable() {
        assert!(
            matches!(parse_outputs(br#"{"outputs":[]}"#), TopologyRead::Read(outputs) if outputs.is_empty())
        );
        assert!(matches!(
            parse_outputs(b"invalid JSON"),
            TopologyRead::Unreadable
        ));
        assert!(matches!(parse_outputs(b"{}"), TopologyRead::Unreadable));
    }

    #[test]
    fn no_active_outputs_has_named_selection() {
        assert!(matches!(
            under(
                &[],
                Frame {
                    origin: (0.0, 0.0),
                    size:   (100.0, 100.0),
                }
            ),
            OutputSelection::NoActiveOutputs
        ));
    }

    #[test]
    fn off_screen_window_selects_nearest_output() {
        let outputs = [
            OUTPUT,
            Output {
                origin: (1920.0, 0.0),
                screen_index: 1,
                ..OUTPUT
            },
        ];
        let selection = under(
            &outputs,
            Frame {
                origin: (5000.0, 0.0),
                size:   (100.0, 100.0),
            },
        );
        assert!(matches!(selection, OutputSelection::Nearest(output) if output.screen_index == 1));
        let selection = under(
            &outputs,
            Frame {
                origin: (0.0, 0.0),
                size:   (100.0, 100.0),
            },
        );
        assert!(
            matches!(selection, OutputSelection::Containing(output) if output.screen_index == 0)
        );
    }

    #[test]
    fn topology_notification_loop_reads_at_startup_change_and_recovery() -> Result<(), String> {
        let topology = Mutex::new(DisplayTopology::Unread);
        let reads = Cell::new(0);
        let read = || {
            reads.set(reads.get() + 1);
            let output = match reads.get() {
                1 => OUTPUT,
                2 => Output {
                    origin: (1920.0, -1080.0),
                    size: (2560, 1440),
                    ..OUTPUT
                },
                _ => Output {
                    origin: (-1280.0, 720.0),
                    size: (1280, 720),
                    ..OUTPUT
                },
            };
            TopologyRead::Read(vec![output])
        };
        let mut events = [
            TopologyNotification::Changed,
            TopologyNotification::Interrupted,
        ]
        .into_iter();
        let notifications = std::iter::from_fn(|| {
            // A silent interval only exposes cached snapshots to capture callers.
            let before = reads.get();
            for _ in 0..60 {
                assert!(matches!(
                    topology.lock().ok()?.snapshot(),
                    TopologyRead::Read(_)
                ));
            }
            assert_eq!(reads.get(), before);
            events.next()
        });
        follow_topology(&topology, notifications, read)?;
        assert_eq!(reads.get(), 2);
        let snapshot = topology
            .lock()
            .map_err(|error| error.to_string())?
            .snapshot();
        assert!(
            matches!(snapshot, TopologyRead::Read(outputs) if matches!(outputs.as_slice(), [output]
            if output.origin.0.to_bits() == 1920.0_f64.to_bits()
                && output.origin.1.to_bits() == (-1080.0_f64).to_bits()
                && output.size == (2560, 1440)))
        );
        assert!(matches!(
            *topology.lock().map_err(|error| error.to_string())?,
            DisplayTopology::Recovering(TopologyRead::Read(_))
        ));
        follow_topology(&topology, [TopologyNotification::Interrupted], read)?;
        assert_eq!(reads.get(), 3);
        let snapshot = topology
            .lock()
            .map_err(|error| error.to_string())?
            .snapshot();
        assert!(
            matches!(snapshot, TopologyRead::Read(outputs) if matches!(outputs.as_slice(), [output]
            if output.origin.0.to_bits() == (-1280.0_f64).to_bits()
                && output.origin.1.to_bits() == 720.0_f64.to_bits()
                && output.size == (1280, 720)))
        );
        Ok(())
    }

    #[test]
    fn topology_notification_loop_retries_an_unreadable_startup() -> Result<(), String> {
        let topology = Mutex::new(DisplayTopology::Unread);
        assert!(follow_topology(&topology, [], || TopologyRead::Unreadable).is_err());
        assert!(matches!(
            *topology.lock().map_err(|error| error.to_string())?,
            DisplayTopology::Recovering(TopologyRead::Unreadable)
        ));
        follow_topology(&topology, [TopologyNotification::Interrupted], || {
            TopologyRead::Read(Vec::new())
        })?;
        assert!(
            matches!(topology.lock().map_err(|error| error.to_string())?.snapshot(), TopologyRead::Read(outputs) if outputs.is_empty())
        );
        Ok(())
    }

    #[test]
    fn topology_worker_retries_failed_and_ended_watches_with_new_layout() -> Result<(), String> {
        let topology = Mutex::new(DisplayTopology::Unread);
        let attempts = Cell::new(0);
        let reads = Cell::new(0);
        let waits = RefCell::new(Vec::new());
        retry_topology_watch(
            &topology,
            || {
                attempts.set(attempts.get() + 1);
                match attempts.get() {
                    1 => Err("watch unavailable".to_owned()),
                    2 => {
                        assert!(matches!(
                            topology
                                .lock()
                                .map_err(|error| error.to_string())?
                                .snapshot(),
                            TopologyRead::Unreadable
                        ));
                        follow_topology(&topology, [TopologyNotification::Interrupted], || {
                            reads.set(reads.get() + 1);
                            TopologyRead::Read(vec![OUTPUT])
                        })
                    },
                    _ => {
                        assert!(matches!(
                            *topology.lock().map_err(|error| error.to_string())?,
                            DisplayTopology::Recovering(TopologyRead::Read(_))
                        ));
                        follow_topology(&topology, [TopologyNotification::Interrupted], || {
                            reads.set(reads.get() + 1);
                            TopologyRead::Read(vec![Output {
                                size: (1280, 720),
                                ..OUTPUT
                            }])
                        })
                    },
                }
            },
            |delay| {
                waits.borrow_mut().push(delay);
                if attempts.get() < 3 {
                    ControlFlow::Continue(())
                } else {
                    ControlFlow::Break(())
                }
            },
        );
        assert_eq!(attempts.get(), 3);
        assert_eq!(reads.get(), 2);
        assert_eq!(*waits.borrow(), vec![DESKTOP_RETRY_INTERVAL; 3]);
        let snapshot = topology
            .lock()
            .map_err(|error| error.to_string())?
            .snapshot();
        assert!(
            matches!(snapshot, TopologyRead::Read(outputs) if matches!(outputs.as_slice(), [output] if output.size == (1280, 720)))
        );
        Ok(())
    }
}
