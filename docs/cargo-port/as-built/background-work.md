# Background work and bounded memory

cargo-port retains worker pools and owner-scoped caches for repeated scans and enrichment.
It replaces pending work for an owner instead of queuing every request.

Paths below are relative to `crates/cargo-port/src/`.

## Workers and pending requests

`scan/background_workers.rs` retains separate discovery, watcher, metadata, and file-measurement pools.
`BackgroundWork` selects the pool.
`BackgroundWorkers::{Ready, Unavailable}` records whether pool construction succeeded.
Pool sizes come from the corresponding `SCAN_*_CONCURRENCY` constants.
The defaults are one discovery worker, one watcher worker, four metadata workers, and six file-measurement workers.
The file-measurement pool serves disk, source, language, and test-count work.

Discovery submits work through `spawn_discovery_for_roots`.
Metadata uses `run_metadata`.
File measurement uses `measure_files`.
A watcher join adapter preserves the caller's join contract while the retained worker owns large registration buffers.
If pool construction fails, the code uses the operation's fallback path.
Some fallbacks start a thread; metadata and file measurement can run on the caller.

`scan/pending_work.rs` defines `PendingWork<K, T>` and `PendingWorker<K, T>`.
`WorkSubmission::{StartWorker, JoinedWorker}` identifies whether submission needs a worker.
`NextWork::{Run, Finished}` identifies the next action.
For each owner, the pending slot retains only the latest replacement request.
`PendingWorker::drop` removes pending state if cancellation interrupts the worker.

A metadata refresh takes a permit for each refresh.
A continuous series of edits cannot keep one permit indefinitely.
Rescan requests for the same root set share a worker and replace its pending request.

## Cache ownership

`scan/source_measurements.rs` caches source measurements and skips unchanged files.
It avoids parser allocations for prose-only documentation comments.
Retirement removes the owner's retained source data.

`WorkspaceMetadataStore` and metadata dispatch generations reject retired checkout results.
`TargetDirIndex` replaces a checkout's members after accepted metadata changes.
It does not accumulate target memberships from earlier results.

Detail, language, and test-count jobs capture their cache ownership before dispatch.
A late job cannot reacquire a retired owner's cache and restore that owner.
Watcher retirement removes entries from `projects`, `project_parents`, and `pending_disk`.

## Allocator calibration

On GNU/Linux, `main.rs` sets `M_ARENA_MAX` to `GNU_MALLOC_ARENA_LIMIT`, which is 2.
This limits retained glibc arenas after repeated parallel scans.
It does not cap a growing application collection.
A controlled ten-minute run retained about 192–194 MB after warm-up.

Measure resident memory and swap together when investigating retained memory.
Separate live allocations from allocator pages before changing a cache.
A size cap can conceal an ownership leak.

Startup detail preparation still reads `.gitmodules` on the caller.
A submodule project can also require `git ls-tree` there.
Captured cache ownership must precede this preparation and asynchronous dispatch.

## Invariants and rationale

1. Retire caches and dispatch generations with their owners.
2. Replace each owner's pending request instead of appending every request.
3. Keep permits scoped to individual refreshes.
4. Reuse retained workers in the normal path.
5. Replace target-directory membership after metadata changes.

These rules bound retained state by current owners and active work.
Worker reuse also permits repeated jobs to reuse allocator storage.
