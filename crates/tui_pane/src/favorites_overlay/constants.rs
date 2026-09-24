//! Constants for the favorites overlay.

use std::time::Duration;

// favorites overlay
/// Cells between two parameter columns in a favorite's row.
pub(super) const COLUMN_GAP: usize = 2;
/// Rows the favorites table keeps even when the popup is squeezed.
pub(super) const CONTENT_MIN_HEIGHT: u16 = 1;
/// TOML table name the favorites overlay's bindings are read from.
pub(super) const FAVORITES_SCOPE: &str = "favorites";
/// Section heading the keymap overlay gives the favorites scope.
pub(super) const FAVORITES_SECTION: &str = "Favorites";
/// How long a deleted favorite's row stays on screen, fading, before
/// the table closes over it.
pub(super) const FAVORITE_REMOVAL_FADE: Duration = Duration::from_millis(400);
/// Cells reserved for a favorite row's selection, currency, and separator.
pub(super) const FAVORITE_ROW_PREFIX_WIDTH: usize = 3;
/// Rows the favorites popup reserves along its bottom for the footer.
pub(super) const FOOTER_HEIGHT: u16 = 1;
/// Widest the favorites popup grows, however wide the terminal is.
pub(super) const POPUP_MAX_WIDTH: u16 = 110;
/// Columns left clear either side of the favorites popup.
pub(super) const POPUP_SIDE_MARGIN: u16 = 4;
