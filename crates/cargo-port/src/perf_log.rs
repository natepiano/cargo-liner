use std::fs::File;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::OnceLock;
use std::time::Duration;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

static PERF_LOG: OnceLock<Mutex<Option<File>>> = OnceLock::new();
static PERF_LOG_PATH: OnceLock<PathBuf> = OnceLock::new();

const SLOW_FRAME_MS: u128 = 100;
const SLOW_BG_BATCH_MS: u128 = 50;
const SLOW_WORKER_MS: u128 = 25;
const SLOW_INPUT_EVENT_MS: u128 = 25;

fn log_path() -> PathBuf { std::env::temp_dir().join("cargo-port-tui-perf.log") }

fn previous_log_path() -> PathBuf { std::env::temp_dir().join("cargo-port-tui-perf.prev.log") }

fn timestamp_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis())
}

fn write_line(message: &str) {
    let Some(lock) = PERF_LOG.get() else {
        return;
    };
    let Ok(mut guard) = lock.lock() else {
        return;
    };
    let Some(file) = guard.as_mut() else {
        return;
    };
    let _ = writeln!(file, "{} {}", timestamp_millis(), message);
    let _ = file.flush();
}

pub fn init() -> PathBuf {
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
        .open(&path)
        .ok();
    let _ = PERF_LOG.set(Mutex::new(file));
    let _ = PERF_LOG_PATH.set(path.clone());
    log_event(&format!(
        "session_start pid={} perf_log={} previous_perf_log={}",
        std::process::id(),
        path.display(),
        previous_path.display()
    ));
    path
}

pub fn log_event(message: &str) { write_line(message); }

pub fn log_duration(label: &str, elapsed: Duration, details: &str, threshold_ms: u128) {
    let elapsed_ms = elapsed.as_millis();
    if elapsed_ms < threshold_ms {
        return;
    }
    write_line(&format!("{label} elapsed_ms={elapsed_ms} {details}"));
}

pub const fn slow_frame_threshold_ms() -> u128 { SLOW_FRAME_MS }

pub const fn slow_bg_batch_threshold_ms() -> u128 { SLOW_BG_BATCH_MS }

pub const fn slow_worker_threshold_ms() -> u128 { SLOW_WORKER_MS }

pub const fn slow_input_event_threshold_ms() -> u128 { SLOW_INPUT_EVENT_MS }
