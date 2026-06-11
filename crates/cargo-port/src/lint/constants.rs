use std::time::Duration;

// src lint cache_size_index
/// Hidden filename at the cache root holding the byte count as plain
/// text decimal (one line, no trailing newline required).
pub(super) const INDEX_FILENAME: &str = ".cache_size";

// src lint runtime
pub(super) const STOP_POLL: Duration = Duration::from_millis(250);

// src lint trigger
pub(super) const DELETE_LINT_DEBOUNCE: Duration = Duration::from_millis(1500);
pub(super) const LINT_DEBOUNCE: Duration = Duration::from_millis(750);
