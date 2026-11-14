use std::thread::ScopedJoinHandle;

/// Preserve a probe panic on the caller after its scoped thread completes.
pub(super) fn finish_probe<T>(scoped_join_handle: ScopedJoinHandle<'_, T>) -> T {
    scoped_join_handle
        .join()
        .unwrap_or_else(|error| std::panic::resume_unwind(error))
}
