// src tui app async_tasks background_services
/// Number of parallel crates.io fetch workers. Each worker runs its share
/// of the plan sequentially, so peak request concurrency against crates.io
/// is capped at this count while the fetch wall-clock drops to roughly
/// 1/N of the serial chain. A tripped limiter surfaces as a 429 →
/// `ServiceSignal::RateLimited` and the recovery path refetches the
/// misses.
pub(super) const CRATES_IO_FETCH_WORKERS: usize = 10;

// src tui app async_tasks repo_handlers
pub(super) const PR_CHECK_POLL_SECS: u64 = 10;
