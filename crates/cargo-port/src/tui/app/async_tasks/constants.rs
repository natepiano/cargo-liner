// `background_services::dispatch_crates_io_fetches` uses `CRATES_IO_FETCH_WORKERS` to split the
// plan into worker buckets processed sequentially.
/// Number of parallel crates.io fetch workers. Each worker runs its share
/// of the plan sequentially, so peak request concurrency against crates.io
/// is capped at this count while the fetch wall-clock drops to roughly
/// 1/N of the serial chain. A tripped limiter surfaces as a 429 →
/// `ServiceSignal::RateLimited` and the recovery path refetches the
/// misses.
pub(super) const CRATES_IO_FETCH_WORKERS: usize = 10;

// `crates_io_handlers` uses `CRATES_IO_NEW_RELEASE_TITLE` for new-version notifications.
pub(super) const CRATES_IO_NEW_RELEASE_TITLE: &str = "New release on crates.io";

// `crates_io_refresh` uses `CRATES_IO_REFRESH_INTERVAL_SECS` to space one-crate refreshes and
// `CRATES_IO_REFRESH_MIN_AGE_SECS` to bound each crate's query frequency.
/// Minimum gap between two background crates.io refresh requests. One
/// crate is re-queried per gap, so a set of N stale crates takes N
/// minutes to cover — the period stretches instead of the request rate
/// rising.
pub(super) const CRATES_IO_REFRESH_INTERVAL_SECS: u64 = 60;
/// A crate becomes eligible for a background refresh only once its last
/// crates.io query is this old, so no crate is queried more than once an
/// hour however long the session runs.
pub(super) const CRATES_IO_REFRESH_MIN_AGE_SECS: u64 = 3600;

// `poll` bounds work with `MAX_MSGS_PER_FRAME` and refreshes project volume space every
// `PROJECT_STORAGE_REFRESH_SECS`, even when other processes change disk usage.
pub(super) const MAX_MSGS_PER_FRAME: usize = 50;
/// Refresh the project volume's free capacity even when a non-project process
/// changes disk space.
pub(super) const PROJECT_STORAGE_REFRESH_SECS: u64 = 2;

// `repo_handlers` uses `PR_CHECK_POLL_SECS` to space pull-request check polling.
pub(super) const PR_CHECK_POLL_SECS: u64 = 10;
