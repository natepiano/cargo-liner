use std::time::Duration;

// ── CI constants ──────────────────────────────────────────────────────

pub const CONCLUSION_SUCCESS: &str = "✓";
pub const CONCLUSION_FAILURE: &str = "✗";
pub const CONCLUSION_CANCELLED: &str = "⊘";
pub const GH_TIMEOUT: Duration = Duration::from_secs(5);

// ── Scan constants ────────────────────────────────────────────────────

pub const CACHE_DIR: &str = "cargo-port/ci-cache";
pub const NO_MORE_RUNS_MARKER: &str = ".no_more_runs";
pub const OLDER_RUNS_FETCH_INCREMENT: u32 = 5;
pub const CONNECTIVITY_TIMEOUT_SECS: &str = "2";
pub const CRATES_IO_TIMEOUT_SECS: &str = "5";

// ── Watcher constants ─────────────────────────────────────────────────

/// Wait for build/clean activity to settle before recalculating.
pub const DEBOUNCE_DURATION: Duration = Duration::from_millis(500);

/// Maximum time before forcing a recalc even if events keep arriving.
pub const MAX_WAIT: Duration = Duration::from_secs(1);

/// Extra settling time for new project directories (e.g. `cargo init`).
pub const NEW_PROJECT_DEBOUNCE: Duration = Duration::from_secs(2);

/// How often the watcher thread checks for expired timers.
pub const POLL_INTERVAL: Duration = Duration::from_millis(500);

// ── Config constants ──────────────────────────────────────────────────

pub const APP_NAME: &str = "cargo-port";
pub const CONFIG_FILE: &str = "config.toml";

/// Default configuration TOML written on first run.
pub const DEFAULT_CONFIG_TOML: &str = r#"[mouse]
invert_scroll = true

[tui]
inline_dirs = ["crates"]
ci_run_count = 5

# Directories to skip when scanning. Edit this list for your setup.
exclude_dirs = [
    "Library",
    "Applications",
    "Downloads",
    "Documents",
    "Movies",
    "Music",
    "Pictures",
    "Public",
    "vendor",
]

# Include non-Rust projects (git repos without Cargo.toml).
include_non_rust = false

# Editor application name, opened via `open -a <editor> <path>`.
editor = "zed"

# How long (in seconds) the status bar flash is shown (e.g. "no new runs found").
status_flash_secs = 3.0
"#;
