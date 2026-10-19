use std::time::Duration;

// `cache_size_index::index_path` places `INDEX_FILENAME` under the lint cache root.
/// Hidden filename at the cache root holding the byte count as plain
/// text decimal (one line, no trailing newline required).
pub(super) const INDEX_FILENAME: &str = ".cache_size";

// `DEFER_RETRY`, `DELETE_LINT_DEBOUNCE`, `LINT_DEBOUNCE` and `STOP_POLL` control lint
// retry, scheduling and stop polling.
/// Shortest wait before retrying a run a lint command deferred with
/// `DEFER_EXIT_CODE`. With `idle_before_lint_secs = 0` it keeps a busy agent
/// from setting off a call every `LINT_DEBOUNCE`.
pub(super) const DEFER_RETRY: Duration = Duration::from_secs(30);
pub(super) const DELETE_LINT_DEBOUNCE: Duration = Duration::from_millis(1500);
pub(super) const LINT_DEBOUNCE: Duration = Duration::from_millis(750);
pub(super) const STOP_POLL: Duration = Duration::from_millis(250);

// `runtime::command` uses `DEFER_EXIT_CODE` for deferral and
// `FILE_LOCK_WAIT_MARKER` to classify output.
/// Exit status (`EX_TEMPFAIL`) a lint command returns to say "not now": the
/// run leaves no result and is retried after the longer of the idle time and
/// `DEFER_RETRY`.
pub(super) const DEFER_EXIT_CODE: i32 = 75;
/// Substring cargo prints on stderr while it waits for a file lock, as in
/// `Blocking waiting for file lock on build directory`. Matching the tail of
/// the line sidesteps the ANSI escapes cargo wraps around the leading
/// `Blocking` word. Cargo prints nothing when it finally acquires the lock, so
/// the next line that does not match is the acquire signal.
pub(super) const FILE_LOCK_WAIT_MARKER: &str = "waiting for file lock";
