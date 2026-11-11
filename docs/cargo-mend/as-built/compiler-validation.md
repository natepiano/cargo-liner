# Compiler selection, wrappers, and validation

cargo-mend uses one retained driver executable and its matching Rust compiler for a run.
It validates composed fixes and preserves the last checked sources when a proposal fails.

Paths below are relative to `crates/cargo-mend/src/`.

## Compiler selection

`compiler/build/driver_compiler.rs` defines `DriverCompiler`.
`DriverCompiler::for_build() -> Result<&'static DriverCompiler>` caches the compiler selected from `MEND_BUILD_SYSROOT`.
The expected compiler is `bin/rustc` inside that sysroot.
A missing compiler reports the expected location.
The selected project's toolchain file does not replace this compiler.

`configure(&self, &mut Command, &Path)` sets `RUSTC` and `RUSTC_WORKSPACE_WRAPPER` together.
It removes the passthrough wrapper setting.
The user's ordinary `RUSTC_WRAPPER` remains available for dependency builds.
Different compiler versions can produce separate dependency artifacts in the same target directory.

On Linux, `configure_runtime_libraries` prepends the selected sysroot's `lib` directory to `LD_LIBRARY_PATH`.
It preserves command-specific inherited paths, or process paths when the command supplied none.
This prevents Cargo and rustup paths from selecting incompatible runtime libraries for build-script compiler probes.
The runtime-library setup is the fallible Linux-specific operation.
`configure` itself returns no result.

## Executable lifetime and cache

`compiler/build/execute.rs` defines `RunningExecutable` and `RunWrapper`.
Startup opens the executable before Cargo metadata resolution.
Linux uses `/proc/self/exe`; other platforms use the current executable path.
The open file preserves the running executable's contents across install-path replacement or removal.

`RunWrapper::resolve(target_directory, &mut executable) -> Result<RunWrapper>` creates or reuses an independent driver copy.
It copies from the open file, using Linux cloning when possible and byte copying otherwise.
A run carries the same `RunWrapper` through analysis, recovery, and validation.
It does not switch to a newly installed executable halfway through the run.

Immutable copies can be shared through the platform cache directory's `cargo-mend` cache.
A cache failure uses the project-local wrapper path when that fallback succeeds.
A missing or unusable retained wrapper produces an error instead of an install-path fallback.
`RunWrapper::configure` checks that its retained executable still exists.

A shared `retention.lock` remains held for the cached run's lifetime.
A separate exclusive `publication.lock` covers publication only.
Eviction requires a successful exclusive retention lock and checks that the lock still belongs to the current cache directory.
A publisher therefore does not wait for every active user of an already published driver.
Measure a first run after relinking to expose publication and cache costs.
Warm reuse alone cannot measure them.

## Batch and fallback validation

`fixes/subtree_reexport/chain.rs` composes independent re-export links before validation.
`fixes/runner/chain.rs` owns `ChainValidation`:

| State | Behavior |
| --- | --- |
| `Batch` | Validate the composed proposal together. |
| `PrunedBatch` | Remove declined links and validate the remaining batch. |
| `IndividualLinks` | Validate links separately when attribution cannot isolate the failed link. |

Each planning step records its files and `StepAttribution`.
`SingleBinding` can associate a failed file with a link.
`SharedBindings` cannot make that association safely.
Its fallback uses individual validation and a dynamic pass limit.
A session snapshot retains earlier checked sources for rollback.
A failed link must not discard an independent validated chain.

`compiler/build/stderr.rs::ErrorLocations` extracts compiler error files from rendered stderr.
Cargo's `json-render-diagnostics` format does not supply the required compiler-message records on stdout.
The stderr reader therefore supplies location evidence.

The workspace-root re-export inventory reuses already selected packages.
A member-directory invocation can require its own workspace lookup.
Representative chain and single-link fallback fixtures use three checks and one metadata request.
Short unchanged functions require no rustfmt process on that path.
Compiler-fix validation reuses usable earlier checks.
`NextAnalysis::{Required, Validated}` carries a checked proposal into the next matching scan.
The final report describes checked sources rather than issuing a redundant report-only validation.

## Progress and calibration

`compiler/build/progress.rs::write_forwarded_output` owns diagnostic forwarding beside the progress line.
It clears the line, writes the diagnostic, adds a missing terminal newline, and redraws progress.
The counter remains active through the end of the pass.
The progress worker exits without an artificial shutdown delay.

Isolated reference medians were about 0.260 seconds for the chain and 0.241 seconds for the fallback.
They describe a measured setup, not a constant performance guarantee.
Launcher and machine-load costs must be separated from compiler work when investigating a slower run.

## Invariants and gotchas

1. Pair the driver with the compiler that built it.
2. Retain one driver identity for the entire run.
3. Keep publication locking separate from retention locking.
4. Preserve earlier checked edits when a later proposal fails.
5. Read compiler error locations from the stream Cargo actually supplies.
6. Keep progress redraws from erasing diagnostics.
7. Reuse validation only while its source and build context remain valid.

Help and build-info currently open the executable before their early returns.
That read-access requirement remains a documented follow-up.
See [import fixes](import-fixes.md) for syntax, cfg, and shared-source safety.
