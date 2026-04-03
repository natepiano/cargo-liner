use std::time::Duration;

use ratatui::style::Color;

// ── Shared icons ─────────────────────────────────────────────────────

pub const PASSING: &str = "🟢";
pub const FAILING: &str = "🔴";
pub const CANCELLED: &str = "🌑";
pub const IN_SYNC: &str = "☑️";
pub const SYNC_UP: &str = "↑";
pub const SYNC_DOWN: &str = "↓";

// ── Lint status icons ────────────────────────────────────────────────

pub const LINT_PASSED: &str = "🟢";
pub const LINT_FAILED: &str = "🔴";
pub const LINT_STALE: &str = "⚫";
pub const LINT_NO_LOG: &str = " ";

// ── Git UI constants ─────────────────────────────────────────────────

pub const GIT_LOCAL: &str = "📁";
pub const GIT_CLONE: &str = "📥";
pub const GIT_FORK: &str = "🔱";
pub const WORKTREE: &str = "🌲";
pub const GIT_MODIFIED_COLOR: Color = Color::Indexed(208);
pub const GIT_UNTRACKED_COLOR: Color = Color::Green;
pub const GIT_IGNORED_COLOR: Color = Color::DarkGray;

// ── CI constants ──────────────────────────────────────────────────────

pub const GH_TIMEOUT: Duration = Duration::from_secs(5);

// ── Cache constants ───────────────────────────────────────────────────

pub const CI_CACHE_DIR: &str = "ci";
pub const PORT_REPORT_CACHE_DIR: &str = "port-report";

// ── Scan constants ────────────────────────────────────────────────────

pub const NO_MORE_RUNS_MARKER: &str = ".no_more_runs";
pub const OLDER_RUNS_FETCH_INCREMENT: u32 = 5;
pub const SCAN_DISK_CONCURRENCY: usize = 2;
pub const SCAN_HTTP_CONCURRENCY: usize = 8;
pub const SCAN_LOCAL_CONCURRENCY: usize = 8;
// ── HTTP constants ───────────────────────────────────────────────────

pub const GITHUB_API_BASE: &str = "https://api.github.com";
pub const GITHUB_GRAPHQL_URL: &str = "https://api.github.com/graphql";
pub const CRATES_IO_API_BASE: &str = "https://crates.io/api/v1";
pub const CRATES_IO_USER_AGENT: &str = "cargo-port";
pub const SERVICE_RETRY_SECS: u64 = 1;

// ── Watcher constants ─────────────────────────────────────────────────

/// Wait for build/clean activity to settle before recalculating.
pub const DEBOUNCE_DURATION: Duration = Duration::from_millis(500);

/// Maximum time before forcing a recalc even if events keep arriving.
pub const MAX_WAIT: Duration = Duration::from_secs(1);

/// Extra settling time for new project directories (e.g. `cargo init`).
pub const NEW_PROJECT_DEBOUNCE: Duration = Duration::from_secs(2);

/// How often the watcher thread checks for expired timers.
pub const POLL_INTERVAL: Duration = Duration::from_millis(500);
pub const WATCHER_DISK_CONCURRENCY: usize = 2;
pub const WATCHER_GIT_CONCURRENCY: usize = 2;

// ── Port-report constants ────────────────────────────────────────────

pub const PORT_REPORT_LATEST_JSON: &str = "latest.json";
pub const PORT_REPORT_HISTORY_JSONL: &str = "history.jsonl";

/// A `started` entry older than this is considered stale (crashed watcher).
pub const STALE_TIMEOUT: Duration = Duration::from_secs(30 * 60);

// ── Config constants ──────────────────────────────────────────────────

pub const APP_NAME: &str = "cargo-port";
pub const CONFIG_FILE: &str = "config.toml";
