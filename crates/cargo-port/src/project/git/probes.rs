use std::thread::ScopedJoinHandle;
/// Run local probes on their own threads so network work in rayon cannot delay them.
pub(super) fn run_independent_probes<A: Send, B: Send>(
    left: impl FnOnce() -> A + Send,
    right: impl FnOnce() -> B + Send,
) -> (A, B) {
    std::thread::scope(|scope| {
        let left = scope.spawn(left);
        let right = scope.spawn(right);
        let left = finish_probe(left);
        let right = finish_probe(right);
        (left, right)
    })
}

/// Preserve a probe panic on the caller after its scoped thread completes.
pub(super) fn finish_probe<T>(probe: ScopedJoinHandle<'_, T>) -> T {
    probe
        .join()
        .unwrap_or_else(|error| std::panic::resume_unwind(error))
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use super::run_independent_probes;

    #[test]
    fn independent_git_probes_run_outside_the_callers_rayon_pool() {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(1)
            .build()
            .expect("create probe caller pool");
        pool.install(|| {
            assert!(rayon::current_thread_index().is_some());
            let (left, right) =
                run_independent_probes(rayon::current_thread_index, rayon::current_thread_index);
            assert!(
                left.is_none(),
                "left probe must not queue behind rayon work"
            );
            assert!(
                right.is_none(),
                "right probe must not queue behind rayon work"
            );
        });
    }
}
