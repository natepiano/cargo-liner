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

// Plasma D-Bus
pub(super) const IMAGE_PLUGIN: &str = "org.kde.image";
pub(super) const PLASMA_INTERFACE: &str = "org.kde.PlasmaShell";
pub(super) const PLASMA_PATH: &str = "/PlasmaShell";
pub(super) const PLASMA_SERVICE: &str = "org.kde.plasmashell";

// terminal windows
/// The default getwindowid target is only the first match; the inventory needs every UUID.
pub(super) const KDO_TOOL_ALL_MATCHES_ARGUMENT: &str = "%@";
pub(super) const KDO_TOOL_ALL_WINDOWS_ARGUMENT: &str = "--name";
/// Hold marker candidates even when TERM_PROGRAM is missing or does not match the terminal class.
pub(super) const KDO_TOOL_ALL_WINDOWS_PATTERN: &str = ".*";
pub(super) const KDO_TOOL_COMMAND: &str = "kdotool";
pub(super) const KDO_TOOL_ID_ARGUMENT: &str = "getwindowid";
pub(super) const KDO_TOOL_SEARCH_ARGUMENT: &str = "search";
pub(super) const KWIN_INTERFACE: &str = "org.kde.KWin";
pub(super) const KWIN_PATH: &str = "/KWin";
pub(super) const KWIN_SERVICE: &str = "org.kde.KWin";
