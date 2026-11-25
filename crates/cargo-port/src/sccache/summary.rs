use super::aligned_field;
use super::aligned_field::SccacheStatFieldMatch;
use super::command_output;
use super::command_output::StatsResult;
use crate::constants::SCCACHE_CACHE_SIZE_LABEL;
use crate::constants::SCCACHE_HIT_RATE_LABEL;

/// The two `sccache --show-stats` values the status line shows, carried as
/// the strings sccache printed so the units stay sccache's own.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum SccacheSummary {
    /// sccache answered and reported both fields.
    Reported {
        /// [`SCCACHE_CACHE_SIZE_LABEL`]'s value, e.g. `101 GiB`.
        cache_size: String,
        /// [`SCCACHE_HIT_RATE_LABEL`]'s value, e.g. `4.90 %`.
        hit_rate:   String,
    },
    /// sccache is not installed, the command failed, or the output carried
    /// neither field — the status line shows nothing.
    Unavailable,
}

/// Run `sccache --show-stats` and keep only the two status-line fields.
pub(crate) fn read_summary() -> SccacheSummary {
    match command_output::read_stats() {
        StatsResult::Ready(lines) => summary_from_lines(&lines),
        StatsResult::Failed(_) => SccacheSummary::Unavailable,
    }
}

fn summary_from_lines(lines: &[String]) -> SccacheSummary {
    let field = |label: &str| {
        lines.iter().find_map(
            |line| match aligned_field::split_aligned_stat(line.trim()) {
                SccacheStatFieldMatch::Field {
                    label: found,
                    value,
                } if found == label => Some(value.to_string()),
                SccacheStatFieldMatch::Field { .. } | SccacheStatFieldMatch::NotAlignedField => {
                    None
                },
            },
        )
    };
    field(SCCACHE_CACHE_SIZE_LABEL)
        .zip(field(SCCACHE_HIT_RATE_LABEL))
        .map_or(SccacheSummary::Unavailable, |(cache_size, hit_rate)| {
            SccacheSummary::Reported {
                cache_size,
                hit_rate,
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stats_lines() -> Vec<String> {
        [
            "Compile requests                   6411",
            "Cache hits rate                    4.90 %",
            "Cache hits rate (Rust)             7.10 %",
            "Cache location                  Local disk: \"/tmp/sccache\"",
            "Cache size                          101 GiB",
            "Max cache size                      128 GiB",
        ]
        .iter()
        .map(|line| (*line).to_string())
        .collect()
    }

    #[test]
    fn summary_keeps_the_size_and_overall_hit_rate() {
        assert_eq!(
            summary_from_lines(&stats_lines()),
            SccacheSummary::Reported {
                cache_size: "101 GiB".to_string(),
                hit_rate:   "4.90 %".to_string(),
            },
        );
    }

    #[test]
    fn summary_is_unavailable_when_either_field_is_missing() {
        let without_size: Vec<String> = stats_lines()
            .into_iter()
            .filter(|line| !line.starts_with(SCCACHE_CACHE_SIZE_LABEL))
            .collect();

        assert_eq!(
            summary_from_lines(&without_size),
            SccacheSummary::Unavailable
        );
        assert_eq!(summary_from_lines(&[]), SccacheSummary::Unavailable);
    }
}
