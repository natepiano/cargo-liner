use std::time::Duration;

use ratatui::style::Color;

// ── Shared icons ─────────────────────────────────────────────────────

pub(crate) const PASSING: &str = "🟢";
pub(crate) const FAILING: &str = "🔴";
pub(crate) const CANCELLED: &str = "🌑";
pub(crate) const IN_SYNC: &str = "☑️";
pub(crate) const NO_REMOTE_SYNC: &str = "──";
pub(crate) const SYNC_UP: &str = "↑";
pub(crate) const SYNC_DOWN: &str = "↓";

// ── Lint status icons ────────────────────────────────────────────────

pub(crate) const LINT_PASSED: &str = "🟢";
pub(crate) const LINT_FAILED: &str = "🔴";
pub(crate) const LINT_STALE: &str = "⚫";
pub(crate) const LINT_NO_LOG: &str = " ";
pub(crate) const NO_CI_RUNS: &str = "No CI runs";
pub(crate) const NO_CI_UNPUBLISHED_BRANCH: &str = "unpublished branch";
pub(crate) const NO_CI_WORKFLOW: &str = "No CI workflow configured";
pub(crate) const NO_LINT_RUNS: &str = "No lint runs";
pub(crate) const NO_LINT_RUNS_NOT_RUST: &str = "No lint runs — not a Rust project";

// ── Git UI constants ─────────────────────────────────────────────────

pub(crate) const GIT_LOCAL: &str = "📁";
pub(crate) const GIT_CLONE: &str = "👯";
pub(crate) const GIT_FORK: &str = "🔱";
pub(crate) const WORKTREE: &str = "🌲";
pub(crate) const GIT_STATUS_CLEAN: &str = "✨";
pub(crate) const GIT_STATUS_UNTRACKED: &str = "🆕";
pub(crate) const GIT_STATUS_MODIFIED: &str = "🟠";
pub(crate) const GIT_MODIFIED_COLOR: Color = Color::Indexed(208);
pub(crate) const GIT_UNTRACKED_COLOR: Color = Color::Green;
pub(crate) const GIT_IGNORED_COLOR: Color = Color::DarkGray;

// ── CI constants ──────────────────────────────────────────────────────

pub(crate) const GH_TIMEOUT: Duration = Duration::from_secs(5);

// ── Cache constants ───────────────────────────────────────────────────

pub(crate) const CI_CACHE_DIR: &str = "ci";
pub(crate) const LINTS_CACHE_DIR: &str = "lint-runs";

// ── Scan constants ────────────────────────────────────────────────────

pub(crate) const NO_MORE_RUNS_MARKER: &str = ".no_more_runs";
pub(crate) const SCAN_DISK_CONCURRENCY: usize = 2;
/// `cargo metadata --no-deps` per workspace root, capped so a large multi-
/// workspace tree doesn't monopolize the blocking pool. Runs briefly —
/// milliseconds per invocation on typical workspaces — so a small cap is
/// enough to preserve fairness with other enrichment tasks.
pub(crate) const SCAN_METADATA_CONCURRENCY: usize = 4;
/// Hard wall-clock cap for a single `cargo metadata` invocation. Beyond
/// this the result is discarded and the workspace enters the error toast
/// path.
pub(crate) const CARGO_METADATA_TIMEOUT: Duration = Duration::from_secs(10);
// ── HTTP constants ───────────────────────────────────────────────────

pub(crate) const GITHUB_API_BASE: &str = "https://api.github.com";
pub(crate) const GITHUB_GRAPHQL_URL: &str = "https://api.github.com/graphql";
pub(crate) const CRATES_IO_API_BASE: &str = "https://crates.io/api/v1";
pub(crate) const CRATES_IO_USER_AGENT: &str = "cargo-port";
pub(crate) const SERVICE_RETRY_SECS: u64 = 1;

// ── Watcher constants ─────────────────────────────────────────────────

/// Wait for build/clean activity to settle before recalculating.
pub(crate) const DEBOUNCE_DURATION: Duration = Duration::from_millis(500);

/// Maximum time before forcing a recalc even if events keep arriving.
pub(crate) const MAX_WAIT: Duration = Duration::from_secs(1);

/// Extra settling time for new project directories (e.g. `cargo init`).
pub(crate) const NEW_PROJECT_DEBOUNCE: Duration = Duration::from_secs(2);

/// How often the watcher thread checks for expired timers.
pub(crate) const POLL_INTERVAL: Duration = Duration::from_millis(500);
pub(crate) const WATCHER_DISK_CONCURRENCY: usize = 2;
pub(crate) const WATCHER_GIT_CONCURRENCY: usize = 2;

// ── Lint history constants ───────────────────────────────────────────

pub(crate) const LINTS_LATEST_JSON: &str = "latest.json";
pub(crate) const LINTS_HISTORY_JSONL: &str = "history.jsonl";

/// A `started` entry older than this is considered stale (crashed watcher).
pub(crate) const STALE_TIMEOUT: Duration = Duration::from_mins(30);

// ── Config constants ──────────────────────────────────────────────────

pub(crate) const APP_NAME: &str = "cargo-port";
pub(crate) const CONFIG_FILE: &str = "config.toml";
pub(crate) const KEYMAP_FILE: &str = "keymap.toml";
