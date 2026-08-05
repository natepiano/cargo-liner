//! How much work the exposure scan's whole-crate sweeps actually do.
//!
//! [`sibling_boundary_signature_exposes_item`] walks every file that holds a
//! module scope inside the item's parent boundary, once per analyzed item. A
//! CPU profile shows which frames burn
//! time but not how many of the enumerated files and module scopes survive each
//! filter, and that ratio is what decides whether restructuring the sweep is
//! worth the risk. These counters answer it exactly.
//!
//! Compiled only under the `test-counters` feature: the record calls sit in the
//! sweep's innermost loop, where an atomic add on every iteration would perturb
//! the measurement it exists to take. Without the feature every entry point
//! below is an empty `const fn`.
//!
//! [`sibling_boundary_signature_exposes_item`]: crate::compiler::exposure
//! [`SourceCache::source_files_under`]: crate::compiler::source_cache::SourceCache::source_files_under

#[cfg(feature = "test-counters")]
use std::sync::atomic::AtomicU64;
#[cfg(feature = "test-counters")]
use std::sync::atomic::Ordering;
use std::time::Duration;

/// One tally per question asked of the sweep loop.
///
/// Every field is a running total across the whole process, which analyzes one
/// crate. The `_enumerated` / `_scanned` and `_enumerated` / `_analyzed` pairs
/// are the ones that matter: their ratio is the share of iterated work the
/// filters immediately discard.
#[cfg(feature = "test-counters")]
struct SweepCounters {
    /// Sweeps that got past the parent-boundary lookup and entered the file
    /// loop.
    sweeps:             AtomicU64,
    /// Files the sweep loop iterated, summed over every sweep.
    files_enumerated:   AtomicU64,
    /// Files that mentioned the item's name and reached the module-scope loop.
    files_scanned:      AtomicU64,
    /// Module scopes the sweep loop iterated, summed over every sweep.
    scopes_enumerated:  AtomicU64,
    /// Module scopes that passed the ancestry filter and were analyzed.
    scopes_analyzed:    AtomicU64,
    /// Calls to `SourceCache::source_files_under`, across all of its call
    /// sites.
    file_list_requests: AtomicU64,
    /// Requests that missed the memo and rebuilt the list.
    file_list_builds:   AtomicU64,
    /// Path entries those calls collected into freshly allocated vectors.
    file_list_entries:  AtomicU64,
    /// `SignatureExposureCache` lookups answered from the memo. Each one skips
    /// a whole sweep.
    signature_hits:     AtomicU64,
    /// Lookups with no memoized answer, which then walk the item.
    signature_misses:   AtomicU64,
    /// Completed walks the cycle-cut gate refused to memoize. This is the
    /// recoverable share: every refusal is a sweep the next lookup repeats.
    signature_refused:  AtomicU64,
}

#[cfg(feature = "test-counters")]
impl SweepCounters {
    const fn new() -> Self {
        Self {
            sweeps:             AtomicU64::new(0),
            files_enumerated:   AtomicU64::new(0),
            files_scanned:      AtomicU64::new(0),
            scopes_enumerated:  AtomicU64::new(0),
            scopes_analyzed:    AtomicU64::new(0),
            file_list_requests: AtomicU64::new(0),
            file_list_builds:   AtomicU64::new(0),
            file_list_entries:  AtomicU64::new(0),
            signature_hits:     AtomicU64::new(0),
            signature_misses:   AtomicU64::new(0),
            signature_refused:  AtomicU64::new(0),
        }
    }
}

#[cfg(feature = "test-counters")]
static COUNTERS: SweepCounters = SweepCounters::new();

#[cfg(feature = "test-counters")]
pub(super) fn record_sweep(files_enumerated: usize) {
    COUNTERS.sweeps.fetch_add(1, Ordering::Relaxed);
    COUNTERS
        .files_enumerated
        .fetch_add(files_enumerated as u64, Ordering::Relaxed);
}

#[cfg(feature = "test-counters")]
pub(super) fn record_file_scanned(scopes_enumerated: usize) {
    COUNTERS.files_scanned.fetch_add(1, Ordering::Relaxed);
    COUNTERS
        .scopes_enumerated
        .fetch_add(scopes_enumerated as u64, Ordering::Relaxed);
}

#[cfg(feature = "test-counters")]
pub(super) fn record_scope_analyzed() { COUNTERS.scopes_analyzed.fetch_add(1, Ordering::Relaxed); }

#[cfg(feature = "test-counters")]
pub(super) fn record_file_list_request() {
    COUNTERS.file_list_requests.fetch_add(1, Ordering::Relaxed);
}

#[cfg(feature = "test-counters")]
pub(super) fn record_file_list_build(entries: usize) {
    COUNTERS.file_list_builds.fetch_add(1, Ordering::Relaxed);
    COUNTERS
        .file_list_entries
        .fetch_add(entries as u64, Ordering::Relaxed);
}

#[cfg(feature = "test-counters")]
pub(super) fn record_signature_hit() { COUNTERS.signature_hits.fetch_add(1, Ordering::Relaxed); }

#[cfg(feature = "test-counters")]
pub(super) fn record_signature_miss() { COUNTERS.signature_misses.fetch_add(1, Ordering::Relaxed); }

#[cfg(feature = "test-counters")]
pub(super) fn record_signature_refused() {
    COUNTERS.signature_refused.fetch_add(1, Ordering::Relaxed);
}

/// Append this crate's totals to the file named by `CARGO_MEND_SWEEP_COUNTERS`.
///
/// mend runs as a `RUSTC_WRAPPER`, so a workspace produces one line per compiled
/// target — a package's lib, each example, each bench, each test binary — and the
/// totals are summed afterwards. `analysis_ms` is the wall time
/// [`super::visibility::collect_and_store_findings`] took for that target, which is the
/// only per-target timing available: cargo prints one status line per package
/// however many targets it holds, so its output cannot attribute time.
///
/// The line cannot go to stderr: the parent mend process reads the wrapper's
/// stderr and treats an unrecognized line as a failed unit, which suppresses the
/// findings summary. With the variable unset this writes nothing, so an ordinary
/// run is unaffected.
#[cfg(feature = "test-counters")]
pub(super) fn report(crate_name: &str, analysis_elapsed: Duration) {
    let Ok(path) = std::env::var("CARGO_MEND_SWEEP_COUNTERS") else {
        return;
    };
    let read = |counter: &AtomicU64| counter.load(Ordering::Relaxed);
    let line = format!(
        "mend-sweep-counters crate={crate_name} analysis_ms={} sweeps={} files_enumerated={} \
         files_scanned={} scopes_enumerated={} scopes_analyzed={} file_list_requests={} \
         file_list_builds={} file_list_entries={} signature_hits={} signature_misses={} \
         signature_refused={}\n",
        analysis_elapsed.as_millis(),
        read(&COUNTERS.sweeps),
        read(&COUNTERS.files_enumerated),
        read(&COUNTERS.files_scanned),
        read(&COUNTERS.scopes_enumerated),
        read(&COUNTERS.scopes_analyzed),
        read(&COUNTERS.file_list_requests),
        read(&COUNTERS.file_list_builds),
        read(&COUNTERS.file_list_entries),
        read(&COUNTERS.signature_hits),
        read(&COUNTERS.signature_misses),
        read(&COUNTERS.signature_refused),
    );
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = std::io::Write::write_all(&mut file, line.as_bytes());
    }
}

#[cfg(not(feature = "test-counters"))]
pub(super) const fn record_sweep(_: usize) {}

#[cfg(not(feature = "test-counters"))]
pub(super) const fn record_file_scanned(_: usize) {}

#[cfg(not(feature = "test-counters"))]
pub(super) const fn record_scope_analyzed() {}

#[cfg(not(feature = "test-counters"))]
pub(super) const fn record_file_list_request() {}

#[cfg(not(feature = "test-counters"))]
pub(super) const fn record_file_list_build(_: usize) {}

#[cfg(not(feature = "test-counters"))]
pub(super) const fn record_signature_hit() {}

#[cfg(not(feature = "test-counters"))]
pub(super) const fn record_signature_miss() {}

#[cfg(not(feature = "test-counters"))]
pub(super) const fn record_signature_refused() {}

#[cfg(not(feature = "test-counters"))]
pub(super) const fn report(_: &str, _: Duration) {}
