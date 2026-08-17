//! Values shared by build-session classification and its support caches.

// build lock contention
/// CPU percent at or below which a Cargo root counts as parked rather than
/// working. A root waiting on the build-directory lock sits in `flock` and
/// consumes none; one resolving dependencies, reading `cargo metadata`, or
/// checking fingerprints consumes measurably more.
pub(super) const PARKED_ROOT_CPU_PERCENT_CEILING: f32 = 1.0;

// cache eviction
use std::time::Duration;
/// Most build sessions whose observed build directory is remembered at once.
/// The key is exec-sensitive, so a retired entry can never be re-matched by a
/// later session and is dropped by least recent use alone.
pub(super) const MAX_BUILD_DIRECTORY_ENTRIES: usize = 1_024;
/// Most dependency source roots whose package identity is cached at once.
/// Beyond this the least recently used entries are dropped and re-read on
/// demand.
pub(super) const MAX_DEPENDENCY_MANIFEST_ENTRIES: usize = 4_096;
/// Most process incarnations the first-seen ledger remembers at once. A long
/// build execs thousands of compilers, so the ledger is bounded as well as
/// pruned against the current snapshot.
pub(super) const MAX_FIRST_SEEN_ENTRIES: usize = 8_192;

// cargo argument spellings
pub(super) const MANIFEST_PATH_ARGUMENT: &str = "--manifest-path";
pub(super) const PROFILE_ARGUMENT: &str = "--profile";
pub(super) const RELEASE_ARGUMENT: &str = "--release";
pub(super) const TARGET_ARGUMENT: &str = "--target";
pub(super) const TARGET_DIRECTORY_ARGUMENT: &str = "--target-dir";

// cargo selector spellings, in the order `cargo --help` lists them
pub(super) const PACKAGE_ARGUMENT: &str = "--package";
pub(super) const PACKAGE_SHORT_ARGUMENT: &str = "-p";
pub(super) const WORKSPACE_ARGUMENT: &str = "--workspace";
pub(super) const ALL_PACKAGES_ARGUMENT: &str = "--all";
pub(super) const LIBRARY_ARGUMENT: &str = "--lib";
pub(super) const BINARY_ARGUMENT: &str = "--bin";
pub(super) const ALL_BINARIES_ARGUMENT: &str = "--bins";
pub(super) const EXAMPLE_ARGUMENT: &str = "--example";
pub(super) const ALL_EXAMPLES_ARGUMENT: &str = "--examples";
pub(super) const TEST_ARGUMENT: &str = "--test";
pub(super) const ALL_TESTS_ARGUMENT: &str = "--tests";
pub(super) const BENCHMARK_ARGUMENT: &str = "--bench";
pub(super) const ALL_BENCHMARKS_ARGUMENT: &str = "--benches";
pub(super) const ALL_TARGETS_ARGUMENT: &str = "--all-targets";

// cargo profiles
/// The build directory Cargo writes under for the `dev` profile.
pub(super) const DEBUG_BUILD_DIRECTORY: &str = "debug";
pub(super) const DEV_PROFILE: &str = "dev";
pub(super) const RELEASE_PROFILE: &str = "release";

// cargo manifests
/// The manifest file name searched for above a dependency's source root.
pub(super) const MANIFEST_FILE_NAME: &str = "Cargo.toml";
/// How far above a dependency source root a manifest is searched for. A
/// registry package's deepest real source file sits a few directories below its
/// own manifest.
pub(super) const MAX_MANIFEST_SEARCH_DEPTH: usize = 6;
/// How far above a compiler's source root an indexed package member root is
/// searched for before the dependency-manifest path takes over.
pub(super) const MAX_MEMBER_ROOT_SEARCH_DEPTH: usize = 6;

// compiler argument spellings
pub(super) const CRATE_NAME_ARGUMENT: &str = "--crate-name";
pub(super) const OUT_DIRECTORY_ARGUMENT: &str = "--out-dir";

// executable names
pub(super) const BUILD_SCRIPT_PREFIX: &str = "build-script-";
pub(super) const CARGO_PLUGIN_PREFIX: &str = "cargo-";
pub(super) const CLIPPY_DRIVER_EXECUTABLE: &str = "clippy-driver";
pub(super) const RUSTC_EXECUTABLE: &str = "rustc";
pub(super) const RUSTDOC_EXECUTABLE: &str = "rustdoc";

// process walk
/// How far a descendant may sit below its Cargo root before the parent chain
/// is treated as unproven. A build script's linker is the deepest real case.
pub(crate) const MAX_DESCENDANT_WALK_DEPTH: usize = 8;

// termination transaction
/// Minimum delay between observer passes when live signaled targets remain.
pub(super) const TERMINATION_DESCENDANT_REFRESH_INTERVAL: Duration =
    std::time::Duration::from_millis(50);
