use std::process::Command;
use std::process::Output;

use crate::constants::SCCACHE_BINARY;
use crate::constants::SCCACHE_STATS_ARG;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum StatsResult {
    Ready(Vec<String>),
    Failed(Vec<String>),
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
