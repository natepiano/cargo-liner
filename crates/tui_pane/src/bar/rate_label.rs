//! Decimal precision shared by the status rates.

use super::constants::RATE_DECIMAL_BELOW;

/// Format one decimal below the threshold after rounding to tenths.
pub(super) fn rate_label(value: f64) -> String {
    if (value * 10.0).round() / 10.0 < RATE_DECIMAL_BELOW {
        format!("{value:.1}")
    } else {
        format!("{value:.0}")
    }
}
