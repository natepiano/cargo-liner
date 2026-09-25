//! How long an agent has been running, as the `age` column writes it.

use std::fmt::Write;

use crate::constants::AGE_DETAIL_BELOW;
use crate::constants::AGE_UNITS;
use crate::constants::ZERO_AGE;

/// `seconds` in its largest unit, with the next unit down after it when
/// the leading value is a single digit and the next unit is not zero:
/// `45s`, `5m 3s`, `12m`, `2h 29m`, `21h`, `3d 4h`, `12d`, `110d`.
pub(crate) fn age_label(seconds: u64) -> String {
    let mut units = AGE_UNITS.iter().skip_while(|(size, _)| seconds < *size);
    let Some((size, suffix)) = units.next() else {
        return ZERO_AGE.to_string();
    };
    let leading = seconds / size;
    let mut label = format!("{leading}{suffix}");
    if leading < AGE_DETAIL_BELOW
        && let Some((next_size, next_suffix)) = units.next()
    {
        let next = seconds % size / next_size;
        if next > 0 {
            let _ = write!(label, " {next}{next_suffix}");
        }
    }
    label
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Seconds in a minute.
    const MINUTE: u64 = 60;
    /// Seconds in an hour.
    const HOUR: u64 = 60 * MINUTE;
    /// Seconds in a day.
    const DAY: u64 = 24 * HOUR;

    /// Every example the column's rule was written against.
    #[test]
    fn ages_read_in_their_largest_unit() {
        let cases = [
            (0, "0s"),
            (7, "7s"),
            (45, "45s"),
            (5 * MINUTE + 3, "5m 3s"),
            (5 * MINUTE, "5m"),
            (12 * MINUTE + 40, "12m"),
            (2 * HOUR + 29 * MINUTE + 59, "2h 29m"),
            (2 * HOUR + 30, "2h"),
            (21 * HOUR + 59 * MINUTE, "21h"),
            (3 * DAY + 4 * HOUR + 50 * MINUTE, "3d 4h"),
            (9 * DAY + 23 * HOUR, "9d 23h"),
            (12 * DAY + 23 * HOUR, "12d"),
            (110 * DAY + 6 * HOUR, "110d"),
        ];
        for (seconds, expected) in cases {
            assert_eq!(age_label(seconds), expected, "{seconds} seconds");
        }
    }
}
