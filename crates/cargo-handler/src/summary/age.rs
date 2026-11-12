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

/// Details retain every age unit after the first nonzero unit.
pub(crate) fn long_age_label(seconds: u64) -> String {
    let minutes = seconds / 60;
    let hours = minutes / 60;
    let days = hours / 24;
    let unit = |value: u64, singular: &str| {
        format!("{value} {singular}{}", if value == 1 { "" } else { "s" })
    };
    let mut parts = Vec::new();
    if days > 0 {
        parts.push(unit(days, "day"));
    }
    if hours > 0 {
        parts.push(unit(hours % 24, "hour"));
    }
    parts.push(unit(minutes % 60, "minute"));
    parts.join(", ")
}

/// The next increase in a shared age column's width.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AgeWidthChange {
    /// No displayed age can gain another column within the clock's range.
    Stable,
    /// Seconds until the oldest displayed age gains a column.
    After(u64),
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

    /// The oldest label reaches each wider minute, hour or day format first.
    pub(crate) fn next_width_growth(ages: impl IntoIterator<Item = u64>) -> AgeWidthChange {
        let Some(oldest) = ages.into_iter().max() else {
            return AgeWidthChange::Stable;
        };
        let mut boundary = [10 * 60, 60 * 60, 10 * 60 * 60, 24 * 60 * 60]
            .into_iter()
            .find(|at| *at > oldest);
        let mut days = 10_u64;
        while boundary.is_none() {
            let Some(at) = days.checked_mul(24 * 60 * 60) else {
                return AgeWidthChange::Stable;
            };
            if at > oldest {
                boundary = Some(at);
            } else if let Some(next) = days.checked_mul(10) {
                days = next;
            } else {
                return AgeWidthChange::Stable;
            }
        }
        boundary.map_or(AgeWidthChange::Stable, |at| {
            AgeWidthChange::After(at - oldest)
        })
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

    #[test]
    fn shared_age_width_deadline_follows_each_larger_label_format() {
        for (age, seconds) in [
            (599, 1),
            (600, 3000),
            (3599, 1),
            (3600, 32400),
            (35999, 1),
            (36000, 50400),
            (86399, 1),
            (86400, 9 * DAY),
            (10 * DAY - 1, 1),
            (100 * DAY - 1, 1),
        ] {
            assert_eq!(
                AgeLayout::next_width_growth([0, age]),
                AgeWidthChange::After(seconds)
            );
            assert!(age_label(age + seconds).len() > age_label(age).len());
            assert_eq!(age_label(age + seconds - 1).len(), age_label(age).len());
        }
        assert_eq!(AgeLayout::next_width_growth([]), AgeWidthChange::Stable);
        assert_eq!(
            AgeLayout::next_width_growth([u64::MAX]),
            AgeWidthChange::Stable
        );
    }

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
