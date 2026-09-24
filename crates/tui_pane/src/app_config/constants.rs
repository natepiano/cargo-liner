//! Constants for an app's configuration directory: the names of the
//! files in it, and the `[appearance]` default no app overrides.

// file names
/// App configuration file, read at startup and written back by
/// [`super::LoadedConfig`].
pub(super) const CONFIG_FILENAME: &str = "config.toml";
/// Keymap overrides loaded by [`crate::KeymapBuilder::load_toml`].
pub(super) const KEYMAP_FILENAME: &str = "keymap.toml";
/// Per-user theme directory scanned by
/// [`crate::ThemeRegistry::from_dir_with_builtins`].
pub(super) const THEMES_DIRNAME: &str = "themes";

// appearance
/// The `appearance.mode` default: follow the terminal's own appearance.
pub(super) const DEFAULT_APPEARANCE_MODE: &str = "auto";

// tiles
/// `tiles.initial_rows` default: rows the grid grows to in a single
/// column before it starts arranging itself into a square.
pub(super) const DEFAULT_INITIAL_ROWS: usize = 4;
/// Ceiling the settings stepper walks `tiles.initial_rows` up to.
pub(super) const MAX_INITIAL_ROWS: usize = 8;
