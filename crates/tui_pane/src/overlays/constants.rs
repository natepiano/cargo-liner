// tui_pane src overlays global_shortcuts
pub(super) const GLOBAL_SHORTCUTS_POPUP_MAX_HEIGHT: u16 = 24;
pub(super) const GLOBAL_SHORTCUTS_POPUP_MIN_WIDTH: u16 = 48;
pub(super) const OVERLAY_RIGHT_PADDING_WIDTH: usize = 1;
pub(super) const SHORTCUT_DESCRIPTION_WIDTH: usize = 34;
/// Blank columns between the widest description and the key column,
/// so a key reads as a key rather than as the last word of the
/// description it sits beside.
pub(super) const DESCRIPTION_KEY_GAP: usize = 2;

// tui_pane src overlays keymap_ui
pub(super) const BASE_POPUP_WIDTH: u16 = 52;
/// Blank columns between one column of the keymap overlay and the next,
/// so the keys of one column read apart from the descriptions of the
/// next rather than running into them.
pub(super) const KEYMAP_COLUMN_GAP: u16 = 3;
/// The blank line the overlay opens with and the one it closes with.
pub(super) const KEYMAP_MARGIN_HEIGHT: u16 = 2;
pub(super) const KEYMAP_POPUP_HEIGHT_PERCENT: u16 = 80;
/// Compatibility constant for the old fixed-height keymap popup.
///
/// The current keymap popup height is percentage-based; this constant remains
/// exported so existing callers do not break.
pub const KEYMAP_POPUP_MAX_HEIGHT: u16 = 43;
pub(super) const PERCENT_DENOMINATOR: u32 = 100;
pub(super) const POPUP_BORDER_HEIGHT: u16 = 2;
pub(super) const POPUP_BORDER_WIDTH: u16 = 2;
/// Terminal cells the keymap popup leaves either side of itself, so it
/// reads as a popup over the display rather than as the display.
pub(super) const POPUP_SIDE_MARGIN_WIDTH: u16 = 4;
