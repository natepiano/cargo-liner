//! How long an agent has been running, as the `age` column writes it.

/// Elapsed minutes, with hours and days when present.
pub(crate) fn age_label(seconds: u64) -> String {
    let minutes = seconds / 60;
    let hours = minutes / 60;
    let days = hours / 24;
    if days > 0 {
        format!("{days}:{:02}:{:02}", hours % 24, minutes % 60)
    } else if hours > 0 {
        format!("{hours}:{:02}", minutes % 60)
    } else {
        minutes.to_string()
    }
}

/// The widest age label in one drawn column.
#[derive(Clone, Debug, Default)]
pub(crate) struct AgeLayout {
    width: usize,
}

impl AgeLayout {
    /// Measure the complete labels in this group.
    pub(crate) fn from_ages(ages: impl IntoIterator<Item = u64>) -> Self {
        Self {
            width: ages
                .into_iter()
                .map(|seconds| age_label(seconds).len())
                .max()
                .unwrap_or(0),
        }
    }

    /// Cell width of the widest label.
    pub(crate) const fn width(&self) -> usize { self.width }

    /// Align a label to the column's right edge.
    pub(crate) fn format(&self, seconds: u64) -> String {
        format!("{:>width$}", age_label(seconds), width = self.width)
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
            (0, "0"),
            (7, "0"),
            (45, "0"),
            (10 * MINUTE, "10"),
            (59 * MINUTE, "59"),
            (HOUR, "1:00"),
            (HOUR + 10 * MINUTE, "1:10"),
            (10 * HOUR + 10 * MINUTE, "10:10"),
            (DAY, "1:00:00"),
            (DAY + 2 * HOUR + 10 * MINUTE, "1:02:10"),
            (110 * DAY, "110:00:00"),
            (2 * HOUR + 29 * MINUTE + 59, "2:29"),
        ];
        for (seconds, expected) in cases {
            assert_eq!(age_label(seconds), expected, "{seconds} seconds");
        }
    }
}
