//! Tracing subscriber + slow-threshold constants for the framework's
//! perf log.
//!
//! [`init`] installs a file-based tracing subscriber only when
//! `TUI_PANE_LOG` is set. When enabled, it rotates the previous log
//! to `previous_log_path` and writes the new session to
//! `current_log_path`. The `SLOW_*` constants are the thresholds the
//! framework and app agree on for "this took too long" tracing
//! events; [`ms`] is a saturating `u128`→`u64` cast for tracing
//! fields.

use std::fs::File;
use std::fs::OpenOptions;
use std::path::Path;

use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt::time::SystemTime;
use tracing_subscriber::prelude::*;

use super::constants::DEFAULT_PERF_LOG_FILTER;
use super::constants::PERF_LOG_ENV;
pub use super::constants::SLOW_BG_BATCH_MS;
pub use super::constants::SLOW_FRAME_MS;
pub use super::constants::SLOW_INPUT_EVENT_MS;

/// Saturating conversion from `u128` milliseconds to `u64` for tracing fields.
#[must_use]
pub fn ms(millis: u128) -> u64 { u64::try_from(millis).unwrap_or(u64::MAX) }

/// Initialize the perf-log tracing subscriber if `TUI_PANE_LOG` is set.
///
/// When enabled, rotates the file at `current_log_path` to
/// `previous_log_path` if it exists, then opens `current_log_path`
/// for write-truncate and installs a file-based tracing subscriber
/// filtered by the `TUI_PANE_LOG` env var (default `info` for
/// invalid values).
///
/// Idempotent at the app level (callers invoke once at startup);
/// repeated invocations rotate the file again but `set_global_default`
/// only takes the first subscriber.
pub fn init(current_log_path: &Path, previous_log_path: &Path) {
    let Some(filter) = perf_log_filter() else {
        return;
    };
    init_with_filter(current_log_path, previous_log_path, filter);
}

fn perf_log_filter() -> Option<EnvFilter> {
    std::env::var_os(PERF_LOG_ENV)?;
    Some(
        EnvFilter::try_from_env(PERF_LOG_ENV)
            .unwrap_or_else(|_| EnvFilter::new(DEFAULT_PERF_LOG_FILTER)),
    )
}

fn init_with_filter(current_log_path: &Path, previous_log_path: &Path, filter: EnvFilter) {
    if current_log_path.is_file() {
        let _ = std::fs::remove_file(previous_log_path);
        let _ = std::fs::rename(current_log_path, previous_log_path);
    }

    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(current_log_path);

    if let Ok(file) = file {
        init_file_subscriber(file, filter);
    }

    tracing::info!(
        pid = std::process::id(),
        perf_log = %current_log_path.display(),
        previous_perf_log = %previous_log_path.display(),
        "session_start"
    );
}

fn init_file_subscriber(file: File, filter: EnvFilter) {
    let fmt_layer = tracing_subscriber::fmt::layer()
        .with_writer(std::sync::Mutex::new(file))
        .with_timer(SystemTime)
        .with_ansi(false)
        .with_target(false);

    let subscriber = tracing_subscriber::registry().with(filter).with(fmt_layer);

    let _ = tracing::subscriber::set_global_default(subscriber);
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use super::*;

    #[test]
    fn disabled_perf_log_does_not_create_current_log() {
        if std::env::var_os(PERF_LOG_ENV).is_some() {
            return;
        }
        let unique = format!("tui-pane-perf-log-disabled-{}", std::process::id());
        let dir = std::env::temp_dir().join(unique);
        let current = dir.join("current.log");
        let previous = dir.join("previous.log");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");

        init(&current, &previous);

        assert!(!current.exists());
        assert!(!previous.exists());

        let _ = std::fs::remove_dir_all(dir);
    }
}
