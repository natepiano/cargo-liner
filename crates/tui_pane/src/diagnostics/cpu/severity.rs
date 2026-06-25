use super::Color;
use super::PERCENT_PER_CELL;
use super::theme;

/// Severity bucket for a CPU utilization percentage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CpuSeverity {
    /// At or below the low-utilization threshold.
    Low,
    /// Above low utilization and at or below the medium-utilization threshold.
    Medium,
    /// Above the medium-utilization threshold.
    High,
}

impl CpuSeverity {
    /// Resolve this severity to its framework theme color.
    #[must_use]
    pub fn color(self) -> Color {
        match self {
            Self::Low => theme::success_color(),
            Self::Medium => theme::title_color(),
            Self::High => theme::error_color(),
        }
    }
}

/// Number of filled 10%-bucket cells for a given percentage, rounding up.
#[must_use]
pub fn filled_cells(percent: u8) -> usize {
    let clamped = if percent > 100 { 100 } else { percent };
    usize::from(clamped).div_ceil(PERCENT_PER_CELL)
}

/// Map a percentage to a [`CpuSeverity`] using caller-supplied thresholds.
#[must_use]
pub const fn severity(
    percent: u8,
    low_utilization_max_percent: u8,
    medium_utilization_max_percent: u8,
) -> CpuSeverity {
    if percent <= low_utilization_max_percent {
        CpuSeverity::Low
    } else if percent <= medium_utilization_max_percent {
        CpuSeverity::Medium
    } else {
        CpuSeverity::High
    }
}

/// Color used to render the empty (unfilled) cells of a CPU bar.
#[must_use]
pub fn blank_bar_color() -> Color { theme::inactive_border_color() }

#[cfg(test)]
mod tests {
    use super::filled_cells;

    #[test]
    fn filled_cells_rounds_up_per_ten_percent_bucket() {
        assert_eq!(filled_cells(0), 0);
        assert_eq!(filled_cells(1), 1);
        assert_eq!(filled_cells(10), 1);
        assert_eq!(filled_cells(11), 2);
        assert_eq!(filled_cells(100), 10);
    }
}
