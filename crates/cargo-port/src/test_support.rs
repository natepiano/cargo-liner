//! Shared helpers for unit tests.

#![allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]

use std::alloc::GlobalAlloc;
use std::alloc::Layout;
use std::alloc::System;
use std::cell::Cell;
use std::path::Path;
use std::process::Command;
use std::sync::OnceLock;

use reqwest::header::HeaderMap;
use reqwest::header::HeaderName;
use reqwest::header::HeaderValue;
use tokio::runtime::Runtime;

pub(crate) fn normalize_line_endings(text: &str) -> String {
    let normalized = text.replace("\r\n", "\n");
    normalized.trim_end_matches(['\r', '\n']).to_string()
}

/// Process-wide tokio runtime for tests that need a `Handle` or
/// `block_on`. Created once on first use and shared thereafter so each
/// test isn't paying for runtime startup.
pub(crate) fn test_runtime() -> &'static Runtime {
    static TEST_RT: OnceLock<Runtime> = OnceLock::new();
    TEST_RT.get_or_init(|| tokio::runtime::Runtime::new().expect("test runtime starts"))
}

/// Build a `HeaderMap` from `(name, value)` pairs. Panics on invalid
/// header names or values — tests should fail loudly on a typo.
pub(crate) fn header_map(entries: &[(&str, &str)]) -> HeaderMap {
    let mut headers = HeaderMap::new();
    for (name, value) in entries {
        let name: HeaderName = (*name).parse().expect("test header name is valid");
        let value = HeaderValue::from_str(value).expect("test header value is valid");
        headers.insert(name, value);
    }
    headers
}

/// Initialize a repository with one commit, including any existing fixture files.
pub(crate) fn init_git_repo(dir: &Path) {
    let commands: [&[&str]; 3] = [
        &["init"],
        &["add", "."],
        &[
            "-c",
            "user.name=cargo-port-tests",
            "-c",
            "user.email=cargo-port-tests@example.com",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--allow-empty",
            "-m",
            "init",
        ],
    ];
    for args in commands {
        let output = Command::new(git_binary())
            .args(args)
            .current_dir(dir)
            .output()
            .expect("run git fixture command");
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

pub(crate) fn git_binary() -> &'static str {
    if Path::new("/usr/bin/git").is_file() {
        "/usr/bin/git"
    } else {
        "git"
    }
}

/// System allocator that counts allocation calls only on the measured test thread.
pub(crate) struct ThreadCountingAllocator;

/// Measurement is disabled outside an explicit allocation scope.
#[derive(Clone, Copy)]
enum AllocationCounting {
    Disabled,
    Measuring,
}

/// The counter stays valid while the measurement guard controls its scope.
struct AllocationCounter {
    scope: Cell<AllocationCounting>,
    calls: Cell<usize>,
}

thread_local! {
    static ALLOCATION_COUNTER: AllocationCounter = const {
        AllocationCounter {
            scope: Cell::new(AllocationCounting::Disabled),
            calls: Cell::new(0),
        }
    };
}

fn record_allocation() {
    // Allocations during thread teardown must still reach the system allocator.
    let _ = ALLOCATION_COUNTER.try_with(|counter| {
        if matches!(counter.scope.get(), AllocationCounting::Measuring) {
            counter.calls.set(counter.calls.get().saturating_add(1));
        }
    });
}

#[allow(
    unsafe_code,
    reason = "a test-only GlobalAlloc impl must be unsafe; it forwards every call to System"
)]
// SAFETY: every method forwards the original pointer and layout to System.
// The counter uses allocation-free thread-local cells and never unwinds.
unsafe impl GlobalAlloc for ThreadCountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        record_allocation();
        // SAFETY: the caller supplies the valid allocation layout required by System.
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        record_allocation();
        // SAFETY: the caller supplies the valid allocation layout required by System.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: all allocations use System, and the caller supplies their layout.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        record_allocation();
        // SAFETY: the caller supplies a System allocation and a valid new size.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

/// Owns the active scope and resets it even when the measured operation panics.
struct AllocationMeasurement<'a> {
    counter: &'a AllocationCounter,
}

impl<'a> AllocationMeasurement<'a> {
    fn start(counter: &'a AllocationCounter) -> Self {
        assert!(
            matches!(counter.scope.get(), AllocationCounting::Disabled),
            "nested allocation measurement"
        );
        counter.calls.set(0);
        counter.scope.set(AllocationCounting::Measuring);
        Self { counter }
    }

    fn finish(self) -> usize { self.counter.calls.get() }
}

impl Drop for AllocationMeasurement<'_> {
    fn drop(&mut self) { self.counter.scope.set(AllocationCounting::Disabled); }
}

/// Count allocation and reallocation calls in one synchronous operation.
/// Fixture setup and other threads are excluded. Nested measurements are rejected.
pub(crate) fn count_allocations<R>(operation: impl FnOnce() -> R) -> (R, usize) {
    ALLOCATION_COUNTER.with(|counter| {
        let measurement = AllocationMeasurement::start(counter);
        let result = operation();
        (result, measurement.finish())
    })
}

#[cfg(test)]
mod allocation_tests {
    use super::count_allocations;

    #[test]
    fn allocation_measurement_counts_live_allocations_and_resets_after_a_panic() {
        let measured = || count_allocations(|| std::hint::black_box(vec![1_u8; 32]));
        let (bytes, first) = measured();
        drop(bytes);
        assert!(first > 0, "an owned buffer must register its allocation");
        assert_eq!(count_allocations(|| ()).1, 0, "a new scope starts at zero");

        let panic = std::panic::catch_unwind(|| {
            count_allocations(|| std::panic::resume_unwind(Box::new("measured operation failed")));
        });
        assert!(panic.is_err());
        let (bytes, recovered) = measured();
        drop(bytes);
        assert_eq!(
            recovered, first,
            "unwinding must release the measurement scope"
        );
    }
}
