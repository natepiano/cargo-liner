use std::process::Command;
use std::process::Output;

use crate::constants::SCCACHE_BINARY;
use crate::constants::SCCACHE_CACHE_SIZE_LABEL;
use crate::constants::SCCACHE_HIT_RATE_LABEL;
use crate::constants::SCCACHE_STATS_ARG;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum StatsResult {
    Ready(Vec<String>),
    Failed(Vec<String>),
}

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
    match read_stats() {
        StatsResult::Ready(lines) => summary_from_lines(&lines),
        StatsResult::Failed(_) => SccacheSummary::Unavailable,
    }
}

fn summary_from_lines(lines: &[String]) -> SccacheSummary {
    let field = |label: &str| {
        lines.iter().find_map(|line| {
            let (found, value) = split_aligned_stat(line.trim())?;
            (found == label).then(|| value.to_string())
        })
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

/// Split one `sccache --show-stats` line at its value column: the first run
/// of two or more spaces separates the label from the value.
pub(crate) fn split_aligned_stat(text: &str) -> Option<(&str, &str)> {
    let mut gap_start = None;
    let mut gap_len = 0;
    for (idx, ch) in text.char_indices() {
        if ch.is_whitespace() {
            gap_start.get_or_insert(idx);
            gap_len += 1;
            continue;
        }
        if gap_len >= 2 {
            let start = gap_start?;
            let label = text[..start].trim_end();
            let value = text[idx..].trim();
            if !label.is_empty() && !value.is_empty() {
                return Some((label, value));
            }
        }
        gap_start = None;
        gap_len = 0;
    }
    None
}

pub(crate) fn read_stats() -> StatsResult {
    Command::new(SCCACHE_BINARY)
        .arg(SCCACHE_STATS_ARG)
        .output()
        .map_or_else(
            |err| StatsResult::Failed(vec![format!("Unable to run sccache: {err}")]),
            |output| stats_from_output(&output),
        )
}

fn stats_from_output(output: &Output) -> StatsResult {
    let lines = output_lines(output);
    if output.status.success() {
        return StatsResult::Ready(non_empty_lines(
            lines,
            "sccache returned no stats".to_string(),
        ));
    }
    let code = output
        .status
        .code()
        .map_or_else(|| "signal".to_string(), |code| code.to_string());
    let mut failed = vec![format!("sccache --show-stats failed with status {code}")];
    failed.extend(lines);
    StatsResult::Failed(failed)
}

fn output_lines(output: &Output) -> Vec<String> {
    let mut lines = text_lines(&output.stdout);
    lines.extend(text_lines(&output.stderr));
    lines
}

fn text_lines(bytes: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(bytes)
        .lines()
        .map(str::trim_end)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

fn non_empty_lines(lines: Vec<String>, fallback: String) -> Vec<String> {
    if lines.is_empty() {
        vec![fallback]
    } else {
        lines
    }
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
