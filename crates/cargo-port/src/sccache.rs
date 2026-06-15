use std::env;
use std::path::Path;
use std::process::Command;
use std::process::Output;

use crate::constants::SCCACHE_BINARY;
use crate::constants::SCCACHE_BINARY_WINDOWS;
use crate::constants::SCCACHE_STATS_ARG;
use crate::constants::WRAPPER_ENV_KEYS;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Config {
    Configured { source: String },
    NotConfigured,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum StatsResult {
    Ready(Vec<String>),
    Failed(Vec<String>),
}

pub(crate) fn config_from_env() -> Config {
    let vars = WRAPPER_ENV_KEYS
        .iter()
        .filter_map(|key| env::var(key).ok().map(|value| (*key, value)));
    config_from_vars(vars)
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

fn config_from_vars(vars: impl IntoIterator<Item = (&'static str, String)>) -> Config {
    for (key, value) in vars {
        if wrapper_is_sccache(&value) {
            return Config::Configured {
                source: format!("{key}={value}"),
            };
        }
    }
    Config::NotConfigured
}

fn wrapper_is_sccache(value: &str) -> bool {
    let trimmed = value.trim();
    let Some(name) = Path::new(trimmed)
        .file_name()
        .and_then(|name| name.to_str())
    else {
        return false;
    };
    matches!(name, SCCACHE_BINARY | SCCACHE_BINARY_WINDOWS)
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

    #[test]
    fn wrapper_detection_accepts_plain_or_path_sccache() {
        assert!(wrapper_is_sccache("sccache"));
        assert!(wrapper_is_sccache("/usr/local/bin/sccache"));
        assert!(wrapper_is_sccache("C:/tools/sccache.exe"));
    }

    #[test]
    fn wrapper_detection_rejects_missing_or_different_wrappers() {
        assert!(!wrapper_is_sccache(""));
        assert!(!wrapper_is_sccache("rustc"));
        assert!(!wrapper_is_sccache("/usr/local/bin/not-sccache"));
    }

    #[test]
    fn config_uses_first_sccache_wrapper() {
        let config = config_from_vars([
            ("RUSTC_WRAPPER", "rustc".to_string()),
            ("CARGO_BUILD_RUSTC_WRAPPER", "/opt/bin/sccache".to_string()),
        ]);

        assert_eq!(
            config,
            Config::Configured {
                source: "CARGO_BUILD_RUSTC_WRAPPER=/opt/bin/sccache".to_string(),
            }
        );
    }
}
