//! Constants for the KDE Wayland wallpaper backend.

use std::time::Duration;

// connection recovery
/// Failed desktop services retry slowly without creating traffic on every capture tick.
pub(super) const DESKTOP_RETRY_INTERVAL: Duration = Duration::from_secs(30);

// desktop commands
/// Check subprocess completion promptly without spinning while a backend is busy.
pub(super) const DESKTOP_READ_POLL_INTERVAL: Duration = Duration::from_millis(10);
/// Bound each stdout read so continuous output returns to the deadline check.
pub(super) const DESKTOP_STDOUT_CHUNK_BYTES: usize = 8192;
/// Allow either desktop command five seconds to answer before terminating it.
pub(super) const TOPOLOGY_READ_DEADLINE: Duration = Duration::from_secs(5);

// desktop configuration
pub(super) const DEFAULT_LOOK_AND_FEEL_PACKAGE: &str = "org.kde.breeze.desktop";
pub(super) const DEFAULT_WALLPAPER_PACKAGE: &str = "Next";
pub(super) const KDE_GLOBALS_FILE: &str = "kdeglobals";
pub(super) const LOOK_AND_FEEL_DEFAULTS_PATH: &str = "plasma/look-and-feel";
pub(super) const WALLPAPERS_PATH: &str = "wallpapers";
pub(super) const XDG_CONFIG_DEFAULT: &str = ".config";
pub(super) const XDG_DATA_DEFAULT: &str = ".local/share";
pub(super) const XDG_DATA_DIRS_DEFAULT: &str = "/usr/local/share:/usr/share";

// display composite
/// How long `KWin` has to run the stacking script and call back.
///
/// Measured at well under a second for a stack of twenty-nine windows, so five seconds leaves room
/// for a busy compositor while still returning inside the monitor's own attempt deadline.
pub(super) const COMPOSITE_ANSWER_DEADLINE: Duration = Duration::from_secs(5);
/// How far a window's frame may stand from a display's own rectangle and still be the desktop
/// covering it.
pub(super) const COMPOSITE_COVERAGE_TOLERANCE: f64 = 0.01;
/// How long one composite stands before another is assembled.
///
/// Capturing every window standing under this one costs far more than a frame, so the picture is
/// held rather than retaken. What the picture is assembled *from* is read every capture instead --
/// the stack read measures at five milliseconds -- and the key the picture is held under carries
/// it, so a window that moves, opens, closes, or is raised over another replaces the picture at
/// once. This governs only how soon changed *contents* of a window that has not moved show
/// through.
pub(super) const COMPOSITE_HOLD: Duration = Duration::from_secs(20);
/// How far the picture's two axes may disagree about pixels per logical coordinate.
pub(super) const COMPOSITE_RATIO_TOLERANCE: f64 = 0.01;
/// Keeps one process's scripts apart from another's in a shared directory, and names the plugin
/// `KWin` unloads each one by.
pub(super) const COMPOSITE_SCRIPT_PREFIX: &str = "tui-pane-backdrop-";
/// Where the stacking sink answers on this process's own bus connection.
pub(super) const SINK_PATH: &str = "/";
/// Replaced with the bus name the script calls back on, which is only known once connected.
pub(super) const SINK_PLACEHOLDER: &str = "SINK_NAME";
/// Reads `KWin`'s stacking order, bottom window first, one window to the line.
///
/// `KWin` runs this in its own process and gives it no way to return a value, so it calls the
/// waiting connection back instead. `workspace.windowList()` is deliberately not used: it reports
/// creation order, not stacking order, and everything here turns on which windows stand below ours.
///
/// The frame is the one rectangle read of each window, because it is the one a capture covers --
/// see [`SCREENSHOT_SHADOW_OPTION`]. `bufferGeometry` is a different rectangle in each direction
/// depending on who draws the decoration, and matches the capture in neither: on a window `KWin`
/// decorates it is the client area inside the title bar, and on one that decorates itself it is
/// the frame grown by the shadow the client drew.
pub(super) const STACKING_SCRIPT: &str = r#"
const rows = workspace.stackingOrder.map(w => [
    w.internalId,
    w.output ? w.output.name : "",
    w.minimized,
    w.onAllDesktops,
    (w.desktops || []).map(d => d.id).join(","),
    w.frameGeometry.x, w.frameGeometry.y, w.frameGeometry.width, w.frameGeometry.height,
].join("\t")).join("\n");
callDBus("SINK_NAME", "/", "dev.tui_pane.Backdrop", "result", rows);
"#;

// display topology
pub(super) const DBUS_OWNER_CHANGED_SIGNAL: &str = "NameOwnerChanged";
pub(super) const DBUS_SERVICE: &str = "org.freedesktop.DBus";
pub(super) const KSCREEN_CHANGE_SIGNAL: &str = "configChanged";
pub(super) const KSCREEN_COMMAND: &str = "kscreen-doctor";
pub(super) const KSCREEN_INTERFACE: &str = "org.kde.kscreen.Backend";
pub(super) const KSCREEN_JSON_ARGUMENT: &str = "-j";
pub(super) const KSCREEN_LOADER_PATH: &str = "/";
pub(super) const KSCREEN_PATH: &str = "/backend";
pub(super) const KSCREEN_REQUEST_BACKEND: &str = "requestBackend";
pub(super) const KSCREEN_SERVICE: &str = "org.kde.KScreen";
/// Buffer layout and owner changes while a topology read is in progress.
pub(super) const TOPOLOGY_SIGNAL_CAPACITY: usize = 64;

// image selection
pub(super) const ASPECT_RATIO_DISTANCE_WEIGHT: f64 = 25_000.0;
/// Plasma classifies a palette as dark below this `qGray` value.
pub(super) const DARK_PALETTE_THRESHOLD: u32 = 192;
pub(super) const IMAGE_EXTENSIONS: [&str; 4] = ["jpeg", "jpg", "png", "webp"];
pub(super) const QGRAY_BLUE_WEIGHT: u32 = 5;
pub(super) const QGRAY_DIVISOR: u32 = 32;
pub(super) const QGRAY_GREEN_WEIGHT: u32 = 16;
pub(super) const QGRAY_RED_WEIGHT: u32 = 11;
pub(super) const UPSCALE_DISTANCE_MULTIPLIER: f64 = 2.0;

// KWin screenshots
/// Capture the title bar and borders with the window, so what comes back covers the whole frame
/// rather than the client area inside it.
pub(super) const SCREENSHOT_DECORATION_OPTION: &str = "include-decoration";
/// Leave the shadow out of the capture, which is what fixes the rectangle it covers.
///
/// `KWin` takes the picture of `visibleGeometry` -- the frame grown by however far the shadow
/// reaches -- unless this is passed, and it defaults it to true. `visibleGeometry` is not in the
/// scripting API, so a picture that carried the shadow could not be placed: it was drawn at the
/// frame's corner and stood a shadow's width right of and below where the window really is, which
/// on Breeze is tens of pixels. Turned off, the picture covers exactly `frameGeometry`, which the
/// stacking script reads.
pub(super) const SCREENSHOT_SHADOW_OPTION: &str = "include-shadow";
pub(super) const SCREENSHOT_HEIGHT_RESULT: &str = "height";
/// `KWin` answers this interface only for a process whose desktop entry declares it in
/// `X-KDE-DBUS-Restricted-Interfaces`; without one every call returns `NoAuthorized` and the
/// backend reconstructs Plasma's wallpaper instead.
pub(super) const SCREENSHOT_INTERFACE: &str = "org.kde.KWin.ScreenShot2";
pub(super) const SCREENSHOT_PATH: &str = "/org/kde/KWin/ScreenShot2";
/// Bytes from one row of the picture to the next, which is not always the row's own width.
pub(super) const SCREENSHOT_STRIDE_RESULT: &str = "stride";
pub(super) const SCREENSHOT_WIDTH_RESULT: &str = "width";
pub(super) const SCREENSHOT_WINDOW_METHOD: &str = "CaptureWindow";

// KWin scripting
pub(super) const KWIN_LOAD_SCRIPT_METHOD: &str = "loadScript";
pub(super) const KWIN_RUN_SCRIPT_METHOD: &str = "run";
pub(super) const KWIN_SCRIPTING_INTERFACE: &str = "org.kde.kwin.Scripting";
pub(super) const KWIN_SCRIPTING_PATH: &str = "/Scripting";
pub(super) const KWIN_SCRIPT_INTERFACE: &str = "org.kde.kwin.Script";
/// Each loaded script answers at this path followed by the number `loadScript` returned.
pub(super) const KWIN_SCRIPT_PATH_PREFIX: &str = "/Scripting/Script";
pub(super) const KWIN_UNLOAD_SCRIPT_METHOD: &str = "unloadScript";

// Plasma D-Bus
pub(super) const IMAGE_PLUGIN: &str = "org.kde.image";
pub(super) const PLASMA_INTERFACE: &str = "org.kde.PlasmaShell";
pub(super) const PLASMA_PATH: &str = "/PlasmaShell";
pub(super) const PLASMA_SERVICE: &str = "org.kde.plasmashell";

// terminal windows
/// The default getwindowid target is only the first match; the inventory needs every UUID.
pub(super) const KDO_TOOL_ALL_MATCHES_ARGUMENT: &str = "%@";
pub(super) const KDO_TOOL_ALL_WINDOWS_ARGUMENT: &str = "--name";
/// Hold marker candidates even when `TERM_PROGRAM` is missing or does not match the terminal class.
pub(super) const KDO_TOOL_ALL_WINDOWS_PATTERN: &str = ".*";
pub(super) const KDO_TOOL_COMMAND: &str = "kdotool";
pub(super) const KDO_TOOL_ID_ARGUMENT: &str = "getwindowid";
pub(super) const KDO_TOOL_SEARCH_ARGUMENT: &str = "search";
pub(super) const KWIN_INTERFACE: &str = "org.kde.KWin";
pub(super) const KWIN_PATH: &str = "/KWin";
pub(super) const KWIN_SERVICE: &str = "org.kde.KWin";
