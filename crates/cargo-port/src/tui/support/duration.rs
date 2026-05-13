use std::fmt::Write as _;

/// Format a non-negative second count as a compact, progressively-revealed
/// duration string like `"45s"`, `"1m 32s"`, `"1h 1m 5s"`, `"1d 0h 0m 1s"`,
/// or `"1w 0d 1h 1m 1s"`.
///
/// Rules:
/// - The highest nonzero unit anchors the output; higher units with a zero value are omitted
///   entirely (no `"0w 0d …"` padding).
/// - Units between the highest nonzero unit and the lowest nonzero unit are always shown, even when
///   their value is zero, so a reader never has to infer a hidden middle unit (e.g. `"1h 0m 5s"`,
///   not `"1h 5s"`).
/// - Trailing zero units below the lowest nonzero unit are dropped, so a duration that lands
///   exactly on a unit boundary stays compact (`"1h"` rather than `"1h 0m 0s"`).
/// - Zero is rendered `"0s"`.
pub fn format_progressive(secs: u64) -> String {
    const WEEK: u64 = 7 * 24 * 3600;
    const DAY: u64 = 24 * 3600;
    const HOUR: u64 = 3600;
    const MINUTE: u64 = 60;

    let parts: [(u64, &str); 5] = [
        (secs / WEEK, "w"),
        ((secs % WEEK) / DAY, "d"),
        ((secs % DAY) / HOUR, "h"),
        ((secs % HOUR) / MINUTE, "m"),
        (secs % MINUTE, "s"),
    ];

    let first = parts.iter().position(|&(value, _)| value > 0);
    let last = parts.iter().rposition(|&(value, _)| value > 0);
    let Some((first, last)) = first.zip(last) else {
        return "0s".to_string();
    };

    let mut out = String::new();
    for (value, unit) in &parts[first..=last] {
        if !out.is_empty() {
            out.push(' ');
        }
        let _ = write!(out, "{value}{unit}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_renders_as_zero_seconds() {
        assert_eq!(format_progressive(0), "0s");
    }

    #[test]
    fn sub_minute_shows_only_seconds() {
        assert_eq!(format_progressive(45), "45s");
    }

    #[test]
    fn minute_boundary_shows_only_minutes() {
        assert_eq!(format_progressive(60), "1m");
    }

    #[test]
    fn mixed_minutes_seconds() {
        assert_eq!(format_progressive(61), "1m 1s");
    }

    #[test]
    fn hour_boundary_shows_only_hour() {
        assert_eq!(format_progressive(3600), "1h");
    }

    #[test]
    fn hour_with_trailing_seconds_keeps_zero_minutes() {
        assert_eq!(format_progressive(3605), "1h 0m 5s");
    }

    #[test]
    fn hour_minutes_seconds() {
        assert_eq!(format_progressive(3665), "1h 1m 5s");
    }

    #[test]
    fn day_boundary_collapses_trailing_zeros() {
        assert_eq!(format_progressive(86_400), "1d");
    }

    #[test]
    fn day_with_seconds_keeps_middle_zeros() {
        assert_eq!(format_progressive(86_401), "1d 0h 0m 1s");
    }

    #[test]
    fn week_boundary() {
        assert_eq!(format_progressive(7 * 86_400), "1w");
    }

    #[test]
    fn week_with_mixed_lower_units() {
        assert_eq!(format_progressive(7 * 86_400 + 3661), "1w 0d 1h 1m 1s");
    }
}
