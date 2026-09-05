//! Discovering terminal windows and reading their `KWin` geometry.

use std::collections::HashMap;
use std::env;
use std::process::Command;
use std::sync::LazyLock;
use std::sync::Mutex;

use zbus::blocking::Proxy;
use zbus::zvariant::OwnedValue;

use super::display;
use super::display::Output;
use super::session_connection;
use crate::backdrop::constants::POSITION_TOLERANCE;
use crate::backdrop::constants::TERM_PROGRAM_ENV;
use crate::backdrop::desktop::CaptureWindowTarget;
use crate::backdrop::desktop::Frame;
use crate::backdrop::desktop::Metrics;
use crate::backdrop::desktop::TerminalWindowSearchOutcome;
use crate::backdrop::desktop::TitledWindow;
use crate::backdrop::desktop::WindowTitle;
use crate::backdrop::desktop::candidate;
use crate::backdrop::desktop::candidate::TerminalWindowCandidate;
use crate::backdrop::desktop::candidate::TerminalWindowCandidates;
use crate::backdrop::desktop::candidate::TerminalWindowOwner;

/// `KWin`'s D-Bus interface.
const KWIN_INTERFACE: &str = "org.kde.KWin";
/// `KWin`'s D-Bus object path.
const KWIN_PATH: &str = "/KWin";
/// `KWin`'s D-Bus service.
const KWIN_SERVICE: &str = "org.kde.KWin";

/// Process-local handles for `KWin`'s string UUIDs.
static WINDOW_REGISTRY: LazyLock<Mutex<WindowRegistry>> =
    LazyLock::new(|| Mutex::new(WindowRegistry::default()));
/// Prevent overlapping temporary `KWin` scripts from colliding inside `kdotool`.
static KDO_TOOL_ACCESS: Mutex<()> = Mutex::new(());

/// One terminal window returned by `KWin`.
pub(super) struct ListedWindow {
    /// Process-local numeric handle used by the cross-platform monitor.
    pub(super) handle: u32,
    /// Current frame in `KWin`'s logical coordinates.
    pub(super) frame:  Frame,
    /// Current window title.
    title:             WindowTitle,
}

impl TerminalWindowCandidate for ListedWindow {
    fn owner(&self) -> TerminalWindowOwner { TerminalWindowOwner::Application { pid: 0 } }

    fn frontmost(&self) -> bool { false }
}

/// `KWin` facts used internally before a UUID receives a numeric handle.
struct WindowInfo {
    /// Current frame in logical coordinates.
    frame: Frame,
    /// Current window title.
    title: WindowTitle,
    /// `KWin`'s stable identifier for this window.
    uuid:  String,
}

/// Bidirectional conversion between `KWin` UUIDs and the public numeric window-id shape.
#[derive(Default)]
struct WindowRegistry {
    /// Numeric handles indexed by `KWin` UUID.
    by_uuid:   HashMap<String, u32>,
    /// `KWin` UUIDs indexed by numeric handle.
    by_handle: HashMap<u32, String>,
}

impl WindowRegistry {
    /// Return the existing handle for `uuid` or allocate the next one.
    fn register(&mut self, uuid: String) -> Option<u32> {
        if let Some(handle) = self.by_uuid.get(&uuid) {
            return Some(*handle);
        }
        let handle = u32::try_from(self.by_uuid.len().checked_add(1)?).ok()?;
        self.by_uuid.insert(uuid.clone(), handle);
        self.by_handle.insert(handle, uuid);
        Some(handle)
    }

    /// Resolve a process-local handle back to `KWin`'s UUID.
    fn uuid(&self, handle: u32) -> Option<String> { self.by_handle.get(&handle).cloned() }
}

/// Every window belonging to the terminal emulator named by `TERM_PROGRAM`.
pub(super) fn terminal_windows() -> Vec<ListedWindow> {
    let Ok(program) = env::var(TERM_PROGRAM_ENV) else {
        return Vec::new();
    };
    search_uuids("--class", &program)
        .into_iter()
        .filter_map(|uuid| listed_window(&uuid))
        .collect()
}

/// Windows needed by one capture, avoiding a new search while its pinned UUID remains valid.
pub(super) fn for_capture(target: CaptureWindowTarget) -> Vec<ListedWindow> {
    match target {
        CaptureWindowTarget::PreferWindow { window_id } => {
            registered_window(window_id).map_or_else(terminal_windows, |window| vec![window])
        },
        CaptureWindowTarget::TerminalWindowHeuristic => terminal_windows(),
    }
}

/// Classify the already terminal-filtered windows for shared selection diagnostics.
pub(super) fn candidates(windows: &[ListedWindow]) -> TerminalWindowCandidates<'_, ListedWindow> {
    candidate::terminal_window_candidates(windows, |_| false, |_| true)
}

/// The terminal window whose frame most nearly contains the reported text area.
pub(super) fn closest_size_match<'a>(
    windows: &[&'a ListedWindow],
    outputs: &[Output],
    metrics: Metrics,
) -> Option<&'a ListedWindow> {
    let score = |window: &ListedWindow| {
        let scale = display::under(outputs, window.frame).map_or(1.0, |output| output.scale);
        let text = (
            f64::from(metrics.text_area.0) / scale,
            f64::from(metrics.text_area.1) / scale,
        );
        mismatch(window.frame.size.0, text.0) + mismatch(window.frame.size.1, text.1)
    };
    windows
        .iter()
        .min_by(|left, right| score(left).total_cmp(&score(right)))
        .copied()
}

/// How far one frame axis is from containing the corresponding text axis.
fn mismatch(frame: f64, text: f64) -> f64 {
    let difference = frame - text;
    if difference < 0.0 {
        -difference * 2.0
    } else {
        difference
    }
}

/// Current frame for a previously registered window handle.
pub(super) fn frame(handle: u32) -> Option<Frame> {
    let uuid = WINDOW_REGISTRY.lock().ok()?.uuid(handle)?;
    query_window(&uuid).map(|window| window.frame)
}

/// Titles of every current terminal window.
pub(super) fn titles() -> Vec<TitledWindow> {
    terminal_windows()
        .into_iter()
        .map(|window| TitledWindow {
            window_id: window.handle,
            title:     window.title,
        })
        .collect()
}

/// The terminal window currently wearing `marker`.
pub(super) fn titled(marker: &str) -> TerminalWindowSearchOutcome {
    search_uuids("--name", marker)
        .into_iter()
        .filter_map(|uuid| listed_window(&uuid))
        .find(|window| match &window.title {
            WindowTitle::Reported(title) => title.contains(marker),
            WindowTitle::Withheld => false,
        })
        .map_or(TerminalWindowSearchOutcome::NotFound, |window| {
            TerminalWindowSearchOutcome::Found {
                window_id: window.handle,
            }
        })
}

/// The terminal window nearest the reported origin.
pub(super) fn at(origin: (f64, f64)) -> TerminalWindowSearchOutcome {
    terminal_windows()
        .into_iter()
        .map(|window| {
            let distance =
                (window.frame.origin.0 - origin.0).abs() + (window.frame.origin.1 - origin.1).abs();
            (window.handle, distance)
        })
        .filter(|(_, distance)| *distance <= POSITION_TOLERANCE)
        .min_by(|(_, left), (_, right)| left.total_cmp(right))
        .map_or(TerminalWindowSearchOutcome::NotFound, |(window_id, _)| {
            TerminalWindowSearchOutcome::Found { window_id }
        })
}

/// Run a read-only `kdotool search` and return its `KWin` UUIDs.
fn search_uuids(field: &str, pattern: &str) -> Vec<String> {
    let Ok(_access) = KDO_TOOL_ACCESS.lock() else {
        return Vec::new();
    };
    let Ok(command) = Command::new("kdotool")
        .args(["search", field, pattern, "getwindowid"])
        .output()
    else {
        return Vec::new();
    };
    if !command.status.success() {
        return Vec::new();
    }
    String::from_utf8_lossy(&command.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Read a `KWin` window and assign its process-local numeric handle.
fn listed_window(uuid: &str) -> Option<ListedWindow> {
    let window = query_window(uuid)?;
    let handle = WINDOW_REGISTRY.lock().ok()?.register(window.uuid)?;
    Some(ListedWindow {
        handle,
        frame: window.frame,
        title: window.title,
    })
}

/// Read a previously registered window without launching `kdotool` again.
fn registered_window(handle: u32) -> Option<ListedWindow> {
    let uuid = WINDOW_REGISTRY.lock().ok()?.uuid(handle)?;
    let window = query_window(&uuid)?;
    Some(ListedWindow {
        handle,
        frame: window.frame,
        title: window.title,
    })
}

/// Read `KWin`'s current facts for one UUID.
fn query_window(uuid: &str) -> Option<WindowInfo> {
    let proxy = Proxy::new(
        session_connection()?,
        KWIN_SERVICE,
        KWIN_PATH,
        KWIN_INTERFACE,
    )
    .ok()?;
    let properties: HashMap<String, OwnedValue> = proxy.call("getWindowInfo", &uuid).ok()?;
    if properties.is_empty() {
        return None;
    }
    Some(WindowInfo {
        frame: Frame {
            origin: (
                property_f64(&properties, "x")?,
                property_f64(&properties, "y")?,
            ),
            size:   (
                property_f64(&properties, "width")?,
                property_f64(&properties, "height")?,
            ),
        },
        title: property_text(&properties, "caption")
            .map_or(WindowTitle::Withheld, WindowTitle::Reported),
        uuid:  property_text(&properties, "uuid").unwrap_or_else(|| uuid.to_owned()),
    })
}

/// Read a string property from `KWin`'s variant map.
fn property_text(properties: &HashMap<String, OwnedValue>, key: &str) -> Option<String> {
    <&str>::try_from(properties.get(key)?)
        .ok()
        .map(str::to_owned)
}

/// Read a floating-point geometry property from `KWin`'s variant map.
fn property_f64(properties: &HashMap<String, OwnedValue>, key: &str) -> Option<f64> {
    let value = properties.get(key)?;
    f64::try_from(value)
        .ok()
        .or_else(|| i32::try_from(value).ok().map(f64::from))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_short_frame_axis_is_penalized_more_than_extra_space() {
        assert!(mismatch(90.0, 100.0) > mismatch(110.0, 100.0));
    }

    #[test]
    fn registry_returns_the_same_handle_for_one_uuid() {
        let mut registry = WindowRegistry::default();
        let first = registry.register("{one}".to_owned());
        let second = registry.register("{one}".to_owned());
        assert_eq!(first, second);
    }
}
