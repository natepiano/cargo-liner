//! Constants for the settings overlay: the values the framework's
//! steppers walk, the widths its rows are measured with, and the
//! labels of the rows the framework owns.

// stepping
/// Values `appearance.mode` steps through, in stepper order.
pub(super) const APPEARANCE_MODES: [&str; 3] = ["auto", "light", "dark"];

// row widths
/// Cells the selection cursor occupies to the left of a row label.
pub(super) const CURSOR_WIDTH: usize = 2;
/// Cells between a row label and its value.
pub(super) const LABEL_VALUE_GAP: usize = 2;
/// Cells `< ` and ` >` add around a stepper row's value.
pub(super) const STEPPER_DECORATION_WIDTH: usize = 4;

// popup
/// Minimum width of the settings popup in cells. Long rows widen it, a
/// narrow terminal caps it.
pub(super) const SETTINGS_POPUP_WIDTH: u16 = 64;
/// Title on the settings popup's top border.
pub(super) const SETTINGS_TITLE: &str = " Settings ";

// sections and labels
/// Section holding the three `[appearance]` steppers.
pub(super) const APPEARANCE_SECTION: &str = "Appearance";
/// Label of the `appearance.mode` stepper.
pub(super) const MODE_LABEL: &str = "mode";
/// Label of the `appearance.light_theme` stepper.
pub(super) const LIGHT_THEME_LABEL: &str = "light theme";
/// Label of the `appearance.dark_theme` stepper.
pub(super) const DARK_THEME_LABEL: &str = "dark theme";
/// Label of the `tiles.initial_rows` stepper.
pub(super) const INITIAL_ROWS_LABEL: &str = "initial rows";
/// Section listing where the app's files live.
pub(super) const FILES_SECTION: &str = "Files";
/// Label of the `config.toml` path row.
pub(super) const CONFIG_LABEL: &str = "config";
/// Label of the `themes/` directory row.
pub(super) const THEMES_LABEL: &str = "themes";
/// Label of the `keymap.toml` path row.
pub(super) const KEYMAP_LABEL: &str = "keymap";
/// Section holding the notices the app hands over.
pub(super) const NOTICES_SECTION: &str = "Notices";
/// Shown in place of a path that cannot be resolved on this platform.
pub(super) const UNRESOLVED_PATH: &str = "unavailable";

// navigation
/// Section heading the keymap overlay gives the navigation scope.
pub(super) const NAVIGATION_SECTION: &str = "Navigation";
