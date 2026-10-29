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
        let header_name: HeaderName = (*name).parse().expect("test header name is valid");
        let value = HeaderValue::from_str(value).expect("test header value is valid");
        headers.insert(header_name, value);
    }
    headers
}

/// Initialize a repository with one commit, including any existing fixture files.
pub(crate) fn init_git_repo(dir: &Path) { init_git_repo_on_branch(dir, "main"); }

/// Reuse the empty commit fixture so empty repositories need no Git subprocess.
pub(crate) fn init_git_repo_on_branch(dir: &Path, branch: &str) {
    let has_files = std::fs::read_dir(dir)
        .expect("read fixture directory")
        .next()
        .is_some();
    install_git_seed(dir, branch);
    if !has_files {
        return;
    }
    let commands: [&[&str]; 2] = [
        &["add", "."],
        &[
            "-c",
            "user.name=cargo-port-tests",
            "-c",
            "user.email=cargo-port-tests@example.com",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "maintenance.auto=false",
            "commit",
            "--amend",
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

/// These loose objects contain an empty tree and its single root commit.
/// Real Git probes still read the fixture; only repeated setup commands are removed.
fn install_git_seed(dir: &Path, branch: &str) {
    const EMPTY_TREE_ID: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";
    const INITIAL_COMMIT_ID: &str = "b952c6511a0362a3689f650fc2b8f358085dfbbf";
    let git_dir = dir.join(".git");
    std::fs::create_dir_all(git_dir.join("refs/heads")).expect("Git fixture refs");
    for (id, bytes) in [
        (
            EMPTY_TREE_ID,
            include_bytes!("test_support/git_seed/4b825dc642cb6eb9a060e54bf8d69288fbee4904.zlib")
                .as_slice(),
        ),
        (
            INITIAL_COMMIT_ID,
            include_bytes!("test_support/git_seed/b952c6511a0362a3689f650fc2b8f358085dfbbf.zlib")
                .as_slice(),
        ),
    ] {
        let (directory, name) = id.split_at(2);
        let object_directory = git_dir.join("objects").join(directory);
        std::fs::create_dir_all(&object_directory).expect("Git fixture object directory");
        std::fs::write(object_directory.join(name), bytes).expect("Git fixture object");
    }
    std::fs::write(git_dir.join("HEAD"), format!("ref: refs/heads/{branch}\n"))
        .expect("fixture HEAD");
    std::fs::write(
        git_dir.join("refs/heads").join(branch),
        format!("{INITIAL_COMMIT_ID}\n"),
    )
    .expect("fixture branch");
    std::fs::write(
        git_dir.join("config"),
        format!("[core]\nrepositoryformatversion = 0\nbare = false\nfilemode = {}\nlogallrefupdates = true\n", cfg!(unix)),
    ).expect("fixture configuration");
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
    // `ALLOCATION_COUNTER.try_with` tolerates thread teardown so allocations still reach `System`.
    let _ = ALLOCATION_COUNTER.try_with(|allocation_counter| {
        if matches!(
            allocation_counter.scope.get(),
            AllocationCounting::Measuring
        ) {
            allocation_counter
                .calls
                .set(allocation_counter.calls.get().saturating_add(1));
        }
    });
}

#[allow(
    unsafe_code,
    reason = "a test-only GlobalAlloc impl must be unsafe; it forwards every call to System"
)]
// SAFETY: every `ThreadCountingAllocator` method forwards the original pointer and layout to
// `System`. `ALLOCATION_COUNTER` uses allocation-free thread-local cells; counting never unwinds.
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
    fn start(allocation_counter: &'a AllocationCounter) -> Self {
        assert!(
            matches!(allocation_counter.scope.get(), AllocationCounting::Disabled),
            "nested allocation measurement"
        );
        allocation_counter.calls.set(0);
        allocation_counter.scope.set(AllocationCounting::Measuring);
        Self {
            counter: allocation_counter,
        }
    }

    fn finish(self) -> usize { self.counter.calls.get() }
}

impl Drop for AllocationMeasurement<'_> {
    fn drop(&mut self) { self.counter.scope.set(AllocationCounting::Disabled); }
}

/// Count allocation and reallocation calls in one synchronous operation.
/// Fixture setup and other threads are excluded. Nested measurements are rejected.
pub(crate) fn count_allocations<R>(operation: impl FnOnce() -> R) -> (R, usize) {
    ALLOCATION_COUNTER.with(|allocation_counter| {
        let measurement = AllocationMeasurement::start(allocation_counter);
        let result = operation();
        (result, measurement.finish())
    })
}

#[cfg(test)]
mod git_fixture_tests {
    use super::Command;
    use super::git_binary;
    use super::init_git_repo;

    #[test]
    fn seeded_repository_commits_fixture_files_once_and_starts_clean() {
        let directory = tempfile::tempdir().expect("fixture directory");
        std::fs::write(directory.path().join("tracked.txt"), "fixture contents\n")
            .expect("fixture file");
        init_git_repo(directory.path());
        for (args, expected) in [
            (vec!["rev-list", "--count", "HEAD"], "1\n"),
            (vec!["status", "--porcelain"], ""),
            (vec!["show", "HEAD:tracked.txt"], "fixture contents\n"),
        ] {
            let output = Command::new(git_binary())
                .args(args)
                .current_dir(directory.path())
                .output()
                .expect("probe fixture repository");
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(String::from_utf8_lossy(&output.stdout), expected);
        }
    }
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
