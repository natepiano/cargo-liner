use std::fs::File;
use std::fs::OpenOptions;
use std::path::PathBuf;
use std::sync::OnceLock;

use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt::time::SystemTime;
use tracing_subscriber::prelude::*;

static PERF_LOG_PATH: OnceLock<PathBuf> = OnceLock::new();

pub(crate) const SLOW_FRAME_MS: u128 = 100;
pub(crate) const SLOW_BG_BATCH_MS: u128 = 50;
pub(crate) const SLOW_WORKER_MS: u128 = 25;
pub(crate) const SLOW_INPUT_EVENT_MS: u128 = 25;

/// Saturating conversion from `u128` milliseconds to `u64` for tracing fields.
pub(crate) fn ms(millis: u128) -> u64 { u64::try_from(millis).unwrap_or(u64::MAX) }

fn log_path() -> PathBuf { std::env::temp_dir().join("cargo-port-tui-perf.log") }

fn previous_log_path() -> PathBuf { std::env::temp_dir().join("cargo-port-tui-perf.prev.log") }

/// Initialize the tracing subscriber that writes to the perf log file.
///
/// Rotates the previous log and returns the path to the current log file.
pub(crate) fn init() -> PathBuf {
    let path = log_path();
    let previous_path = previous_log_path();
    if path.is_file() {
        let _ = std::fs::remove_file(&previous_path);
        let _ = std::fs::rename(&path, &previous_path);
    }

    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&path);

    let filter =
        EnvFilter::try_from_env("CARGO_PORT_LOG").unwrap_or_else(|_| EnvFilter::new("info"));

    if let Ok(file) = file {
        init_file_subscriber(file, filter);
    }

    let _ = PERF_LOG_PATH.set(path.clone());
    tracing::info!(
        pid = std::process::id(),
        perf_log = %path.display(),
        previous_perf_log = %previous_path.display(),
        "session_start"
    );
    path
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
