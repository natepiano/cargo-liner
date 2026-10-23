//! Online logical processor capacity from Linux's system-wide statistics.

use std::num::NonZeroUsize;

/// Linux reports every online logical processor in this system-wide table.
const MACHINE_CPU_STAT_FILE: &str = "/proc/stat";
/// Prefix of the per-processor rows in Linux's CPU statistics.
const MACHINE_CPU_STAT_PREFIX: &str = "cpu";

/// Read online logical processors without collecting their counters and brand strings.
pub(super) fn online_processors() -> Option<NonZeroUsize> {
    let statistics = std::fs::read_to_string(MACHINE_CPU_STAT_FILE).ok()?;
    NonZeroUsize::new(processor_rows(&statistics))
}

/// Count per-processor entries, excluding the aggregate and unrelated statistics.
fn processor_rows(statistics: &str) -> usize {
    statistics
        .lines()
        .filter_map(|line| line.split_whitespace().next())
        .filter(|label| {
            label
                .strip_prefix(MACHINE_CPU_STAT_PREFIX)
                .is_some_and(|number| {
                    !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit())
                })
        })
        .count()
}

#[cfg(test)]
mod tests {
    use super::processor_rows;

    #[test]
    fn processor_capacity_excludes_the_aggregate_and_unrelated_statistics() {
        assert_eq!(
            processor_rows("cpu 1 2 3\ncpu0 1 2 3\ncpu7 1 2 3\nintr 4\nprocesses 8\ncpu_other 9\n"),
            2
        );
        assert_eq!(processor_rows("cpu 1 2 3\nintr 4\n"), 0);
    }
}
