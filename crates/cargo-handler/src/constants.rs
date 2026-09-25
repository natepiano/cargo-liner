//! Constants for `cargo-handler`.

// configuration
/// Directory under the OS config root holding `config.toml`,
/// `keymap.toml`, `favorites.toml` and `themes/`.
pub(crate) const CONFIG_DIRNAME: &str = "cargo-handler";
/// Id of the built-in dark variant, and the `appearance.dark_theme`
/// default. Defined in [`crate::theme`], not in `tui_pane`: theme
/// content belongs to the app.
pub(crate) const DEFAULT_DARK_THEME: &str = "Default Dark";
/// Id of the built-in high-contrast dark variant.
pub(crate) const DEFAULT_HC_DARK_THEME: &str = "High Contrast Dark";
/// Id of the built-in high-contrast light variant.
pub(crate) const DEFAULT_HC_LIGHT_THEME: &str = "High Contrast Light";
/// Id of the built-in light variant, and the `appearance.light_theme`
/// default.
pub(crate) const DEFAULT_LIGHT_THEME: &str = "Default Light";

// lifecycle
/// The binary's own name: what the command line calls itself in help
/// and in anything it reports going wrong, and the fallback executable
/// name when the running binary's path cannot be resolved for a
/// restart. Distinct from [`APP_NAME`], which is padded for the status
/// line.
pub(crate) const BINARY_NAME: &str = "cargo-handler";
/// The one line `--help` opens with. A placeholder until the tool's
/// purpose is written up.
pub(crate) const CLI_ABOUT: &str = "A terminal UI cargo tool";
/// The word cargo knows this tool by, which is the binary's name with
/// cargo's own prefix taken off. Cargo runs `cargo handler ...` by
/// finding `cargo-handler` on the path and handing it this word ahead
/// of every other argument, so the command line drops it before
/// parsing.
pub(crate) const SUBCOMMAND_NAME: &str = "handler";

// settings overlay
/// Section heading the settings overlay puts the grid's rows under.
pub(crate) const TILES_SETTINGS_SECTION: &str = "Tiles";

// status line and overlays
/// Section heading the keymap overlay gives this app's globals scope.
pub(crate) const APP_GLOBALS_SECTION: &str = "App Shortcuts";
/// Label leading the status line's version note. The spaces around it
/// are its padding -- the framework adds none.
pub(crate) const APP_NAME: &str = " cargo-handler ";
/// Version shown beside [`APP_NAME`], read from the manifest at compile
/// time so a running instance always says which build it is.
pub(crate) const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
/// What the status line says while the attract screen is being shown
/// because it was asked for: the grid is still there, it is being
/// drawn over.
pub(crate) const ATTRACT_NOTE_LABEL: &str = "attract";
/// Comment block the keymap editor writes above the generated tables.
pub(crate) const KEYMAP_TOML_HEADER: &str = "\
# cargo-handler keymap configuration\n\
# Edit bindings below. Format: action = \"key\" or \"modifier-key\"\n\
# Modifiers: ctrl, alt, shift.  Examples: \"ctrl-k\", \"shift-tab\", \"q\"\n\
# Chord steps are space-separated, e.g. \"g g\".\n\n";
/// Rows the status line occupies along the bottom of the terminal.
pub(crate) const STATUS_LINE_HEIGHT: u16 = 1;

// tiles
/// Shown in the summary cell, which has nothing to list yet.
pub(crate) const EMPTY_SUMMARY_NOTE: &str = "nothing to show yet";
/// The summary cell's title, set into its top border. The leading space
/// holds the word off the corner glyph the title is set against.
pub(crate) const SUMMARY_CELL_TITLE: &str = " summary";
