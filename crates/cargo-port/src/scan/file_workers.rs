//! Keep file measurements on the shared process-lifetime workers.

#[cfg(test)]
use super::background_workers;

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;
    use crate::constants::SCAN_DISK_CONCURRENCY;

    #[test]
    fn repeated_walks_reuse_the_same_bounded_worker_set() {
        let threads: HashSet<_> = (0..1_024)
            .map(|_| background_workers::measure_files(|| std::thread::current().id()))
            .collect();
        assert!(threads.len() <= SCAN_DISK_CONCURRENCY);
        assert!(!threads.contains(&std::thread::current().id()));
        for _ in 0..32 {
            let name = background_workers::measure_files(|| {
                std::thread::current().name().map(str::to_string)
            });
            assert!(name.is_some_and(|name| name.starts_with("port-files-")));
        }
    }
}
