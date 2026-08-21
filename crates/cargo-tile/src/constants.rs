//! Constants for `cargo-tile`.

// configuration
/// Directory under the OS config root holding `config.toml`,
/// `keymap.toml`, and `themes/`.
pub(crate) const CONFIG_DIRNAME: &str = "cargo-tile";
/// App configuration file, read at startup for its `[appearance]` section.
pub(crate) const CONFIG_FILENAME: &str = "config.toml";
/// Keymap overrides loaded by [`tui_pane::KeymapBuilder::load_toml`].
pub(crate) const KEYMAP_FILENAME: &str = "keymap.toml";
/// Per-user theme directory scanned by
/// [`tui_pane::ThemeRegistry::from_dir_with_builtins`].
pub(crate) const THEMES_DIRNAME: &str = "themes";

// settings overlay
/// Values `appearance.mode` cycles through, in stepper order.
pub(crate) const APPEARANCE_MODES: [&str; 3] = ["auto", "light", "dark"];
/// Rows of popup border above and below the settings body.
pub(crate) const POPUP_CHROME_HEIGHT: u16 = 2;
/// Columns of popup border left and right of the settings body.
pub(crate) const POPUP_CHROME_WIDTH: u16 = 2;
/// Cells the selection cursor occupies to the left of a row label.
pub(crate) const CURSOR_WIDTH: usize = 2;
/// Cells between a row label and its value.
pub(crate) const LABEL_VALUE_GAP: usize = 2;
/// Cells `< ` and ` >` add around a stepper row's value.
pub(crate) const STEPPER_DECORATION_WIDTH: usize = 4;
/// Minimum width of the settings popup in cells. Long rows widen it, a
/// narrow terminal caps it.
pub(crate) const SETTINGS_POPUP_WIDTH: u16 = 64;

// lifecycle
/// Fallback executable name when the running binary's path cannot be
/// resolved for a restart.
pub(crate) const BINARY_NAME: &str = "cargo-tile";

// startup
/// Shown in the settings overlay when a path cannot be resolved on this
/// platform.
pub(crate) const UNRESOLVED_PATH: &str = "unavailable";

// status line and overlays
/// Section heading the keymap overlay gives this app's globals scope.
pub(crate) const APP_GLOBALS_SECTION: &str = "App Shortcuts";
/// Rows the status line occupies along the bottom of the terminal.
pub(crate) const STATUS_LINE_HEIGHT: u16 = 1;
/// Body text of the template's placeholder pane.
pub(crate) const PLACEHOLDER_BODY: &str = "Your app's content goes here.";

/// Comment block the keymap editor writes above the generated tables.
pub(crate) const KEYMAP_TOML_HEADER: &str = "\
# cargo-tile keymap configuration\n\
# Edit bindings below. Format: action = \"key\" or \"modifier-key\"\n\
# Modifiers: ctrl, alt, shift.  Examples: \"ctrl-k\", \"shift-tab\", \"q\"\n\
# Chord steps are space-separated, e.g. \"g g\".\n\n";
