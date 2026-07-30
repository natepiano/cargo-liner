// src tui app async_tasks background_services
/// Number of parallel crates.io fetch workers. Each worker runs its share
/// of the plan sequentially, so peak request concurrency against crates.io
/// is capped at this count while the fetch wall-clock drops to roughly
/// 1/N of the serial chain. A tripped limiter surfaces as a 429 →
/// `ServiceSignal::RateLimited` and the recovery path refetches the
/// misses.
pub(super) const CRATES_IO_FETCH_WORKERS: usize = 10;

// src tui app async_tasks crates_io_handlers
pub(super) const CRATES_IO_NEW_RELEASE_TITLE: &str = "New release on crates.io";

// src tui app async_tasks crates_io_refresh
/// Minimum gap between two background crates.io refresh requests. One
/// crate is re-queried per gap, so a set of N stale crates takes N
/// minutes to cover — the period stretches instead of the request rate
/// rising.
pub(super) const CRATES_IO_REFRESH_INTERVAL_SECS: u64 = 60;
/// A crate becomes eligible for a background refresh only once its last
/// crates.io query is this old, so no crate is queried more than once an
/// hour however long the session runs.
pub(super) const CRATES_IO_REFRESH_MIN_AGE_SECS: u64 = 3600;

// src tui app async_tasks poll
pub(super) const MAX_MSGS_PER_FRAME: usize = 50;
/// Refresh the project volume's free capacity even when a non-project process
/// changes disk space.
pub(super) const PROJECT_STORAGE_REFRESH_SECS: u64 = 2;

// src tui app async_tasks repo_handlers
pub(super) const PR_CHECK_POLL_SECS: u64 = 10;
