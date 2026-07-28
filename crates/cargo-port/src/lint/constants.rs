use std::time::Duration;

// src lint cache_size_index
/// Hidden filename at the cache root holding the byte count as plain
/// text decimal (one line, no trailing newline required).
pub(super) const INDEX_FILENAME: &str = ".cache_size";

// src lint runtime
pub(super) const STOP_POLL: Duration = Duration::from_millis(250);

// src lint runtime command
/// Substring cargo prints on stderr while it waits for a file lock, as in
/// `Blocking waiting for file lock on build directory`. Matching the tail of
/// the line sidesteps the ANSI escapes cargo wraps around the leading
/// `Blocking` word. Cargo prints nothing when it finally acquires the lock, so
/// the next line that does not match is the acquire signal.
pub(super) const FILE_LOCK_WAIT_MARKER: &str = "waiting for file lock";

// src lint trigger
pub(super) const DELETE_LINT_DEBOUNCE: Duration = Duration::from_millis(1500);
pub(super) const LINT_DEBOUNCE: Duration = Duration::from_millis(750);
