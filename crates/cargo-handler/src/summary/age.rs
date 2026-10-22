//! How long an agent has been running, as the `age` column writes it.

use crate::constants::AGE_DETAIL_BELOW;
use crate::constants::AGE_UNITS;

/// `seconds` in its largest unit, with the next unit down after it when
/// the leading value is a single digit and the next unit is not zero:
/// `45s`, `5m 3s`, `12m`, `2h 29m`, `21h`, `3d 4h`, `12d`, `110d`.
pub(crate) fn age_label(seconds: u64) -> String {
    AgeParts::of(seconds)
        .0
        .into_iter()
        .filter_map(|slot| match slot {
            AgeSlot::Blank => None,
            AgeSlot::Value { number, unit } => Some(format!("{number}{unit}")),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// A unit slot is empty or contains the number and suffix selected by the age rule.
#[derive(Clone, Copy)]
enum AgeSlot {
    Blank,
    Value { number: u64, unit: &'static str },
}

struct AgeParts([AgeSlot; AGE_UNITS.len()]);

impl AgeParts {
    fn of(seconds: u64) -> Self {
        let mut slots = [AgeSlot::Blank; AGE_UNITS.len()];
        let index = AGE_UNITS
            .iter()
            .position(|(size, _)| seconds >= *size)
            .unwrap_or(AGE_UNITS.len() - 1);
        let (size, unit) = AGE_UNITS[index];
        let number = seconds / size;
        slots[index] = AgeSlot::Value { number, unit };
        if number < AGE_DETAIL_BELOW
            && let Some(&(next_size, unit)) = AGE_UNITS.get(index + 1)
        {
            let number = seconds % size / next_size;
            if number > 0 {
                slots[index + 1] = AgeSlot::Value { number, unit };
            }
        }
        Self(slots)
    }
}

/// The measured unit slots shared by every elapsed age in one column.
#[derive(Clone, Debug, Default)]
pub(crate) struct AgeLayout {
    widths: [usize; AGE_UNITS.len()],
}

impl AgeLayout {
    /// Measure only the units retained by the existing age-label rule.
    pub(crate) fn from_ages(ages: impl IntoIterator<Item = u64>) -> Self {
        let mut layout = Self::default();
        for seconds in ages {
            for (index, slot) in AgeParts::of(seconds).0.into_iter().enumerate() {
                if let AgeSlot::Value { number, unit } = slot {
                    layout.widths[index] =
                        layout.widths[index].max(format!("{number}{unit}").len());
                }
            }
        }
        layout
    }

    /// Total cell width, including one space between present unit slots.
    pub(crate) fn width(&self) -> usize {
        let slots = self.widths.iter().filter(|&&width| width > 0).count();
        self.widths.iter().sum::<usize>() + slots.saturating_sub(1)
    }

    /// Put each displayed number and unit in its measured, right-aligned slot.
    pub(crate) fn format(&self, seconds: u64) -> String {
        self.widths
            .iter()
            .zip(AgeParts::of(seconds).0)
            .filter(|(width, _)| **width > 0)
            .map(|(&width, slot)| {
                let part = match slot {
                    AgeSlot::Blank => String::new(),
                    AgeSlot::Value { number, unit } => format!("{number}{unit}"),
                };
                format!("{part:>width$}")
            })
            .collect::<Vec<_>>()
            .join(" ")
    }
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
