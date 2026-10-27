//! Labels and formatting for the account table.

use ratatui::style::Color;

// display
pub(super) const ACCOUNTS_HEADING: &str = "accounts";
pub(super) const ACCOUNT_READ_DOT: &str = "●";
pub(super) const ACCOUNT_ROLE_SEPARATOR: &str = ", ";
pub(super) const LOW_WEEKLY_USAGE_COLOR: Color = Color::Red;
/// Round reset timestamps to the nearest minute before drawing them.
pub(super) const RESET_ROUNDING_SECONDS: i64 = 30;
pub(super) const RESET_TIME_FORMAT: &str = "%a %H:%M";
pub(super) const UNLIMITED_CREDITS_LABEL: &str = "unlimited";
