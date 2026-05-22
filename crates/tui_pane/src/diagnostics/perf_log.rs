//! Tracing subscriber + slow-threshold constants for the framework's
//! perf log.
//!
//! [`init`] rotates the previous log to `previous_log_path` and
//! installs a file-based tracing subscriber writing to
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

/// Frame paint exceeding this is logged as a slow frame (ms).
pub const SLOW_FRAME_MS: u128 = 30;
/// Background-message batch processing exceeding this is logged (ms).
pub const SLOW_BG_BATCH_MS: u128 = 50;
/// Per-input-event handling exceeding this is logged (ms).
pub const SLOW_INPUT_EVENT_MS: u128 = 10;

/// Saturating conversion from `u128` milliseconds to `u64` for tracing fields.
#[must_use]
pub fn ms(millis: u128) -> u64 { u64::try_from(millis).unwrap_or(u64::MAX) }

/// Initialize the perf-log tracing subscriber.
///
/// Rotates the file at `current_log_path` to `previous_log_path` if
/// it exists, then opens `current_log_path` for write-truncate and
/// installs a file-based tracing subscriber filtered by the
/// `CARGO_PORT_LOG` env var (default `info`).
///
/// Idempotent at the app level (callers invoke once at startup);
/// repeated invocations rotate the file again but `set_global_default`
/// only takes the first subscriber.
pub fn init(current_log_path: &Path, previous_log_path: &Path) {
    if current_log_path.is_file() {
        let _ = std::fs::remove_file(previous_log_path);
        let _ = std::fs::rename(current_log_path, previous_log_path);
    }

    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(current_log_path);

    let filter =
        EnvFilter::try_from_env("CARGO_PORT_LOG").unwrap_or_else(|_| EnvFilter::new("info"));

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
