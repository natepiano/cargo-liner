// tui_pane src overlays global_shortcuts
pub(super) const GLOBAL_SHORTCUTS_POPUP_MAX_HEIGHT: u16 = 22;
pub(super) const GLOBAL_SHORTCUTS_POPUP_MIN_WIDTH: u16 = 48;
pub(super) const SHORTCUT_DESCRIPTION_WIDTH: usize = 28;

// tui_pane src overlays keymap_ui
pub(super) const BASE_POPUP_WIDTH: u16 = 52;
pub(super) const KEYMAP_POPUP_HEIGHT_PERCENT: u16 = 80;
/// Compatibility constant for the old fixed-height keymap popup.
///
/// The current keymap popup height is percentage-based; this constant remains
/// exported so existing callers do not break.
pub const KEYMAP_POPUP_MAX_HEIGHT: u16 = 43;
pub(super) const PERCENT_DENOMINATOR: u32 = 100;
pub(super) const POPUP_BORDER_HEIGHT: u16 = 2;
