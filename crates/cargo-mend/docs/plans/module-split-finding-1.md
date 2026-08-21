# Module Split Plan for Style Finding 1

## Source Finding

Style evaluation flagged six flat Rust modules that meet multiple split criteria:

- `src/compiler/build.rs`
- `src/compiler/persistence.rs`
- `src/config/cli.rs`
- `src/fixes/imports.rs`
- `src/fixes/runner.rs`
- `src/reporting/render.rs`

The governing style rule is `/Users/natemccoy/rust/nate_style/rust/when-to-split-a-module.md`, with related guidance from `/Users/natemccoy/rust/nate_style/rust/types-live-with-their-behavior.md`.

The required order is:

1. Identify top-level types and free helper functions in each file.
2. Move each item that has a single owner to the module that owns it.
3. Split only the code that remains after ownership moves.
4. Preserve existing public and `pub(crate)` paths through the parent module where callers depend on them.

## Non-Goals

- Do not mix this refactor with CI install-command cleanup.
- Do not rename exported concepts unless the rename has its own review and changelog entry.
- Do not split files by line count alone.
- Do not keep empty facade modules that only re-export one child.

## Review Before Editing

For each target file, make a short ownership table before changing code:

| Item | Kind | Current path | Visibility | Current callers | Impl blocks | Co-moving helpers | Test home | Proposed home | Retained path |
|---|---|---|---|---|---|---|---|---|---|

Only move an item when at least one of these is clear:

- One function or module constructs it.
- One module mutates it and the rest only read it.
- One module consumes it.
- It is a free helper used by two or more unrelated siblings of the same parent, in which case `support` may be the correct home.

If none of those is clear, leave the item in the current module until the remaining file is reviewed as a cohesive unit.

Before moving each item, name the full behavior group that moves with it: impl blocks, helper functions, tests, and any narrow re-export needed to keep existing callers compiling.

## Global Execution Rules

Apply these rules to every phase.

### Path and Visibility Contract

Before editing, record every current import path for the target module:

- `crate::...` paths from sibling or descendant modules.
- `super::...` paths from nearby modules.
- `pub(crate)` and `pub(super)` items used outside the target file.
- Unit tests that currently reach private helpers through `super`.

For each moved item, preserve the old path with the narrowest valid re-export unless the phase explicitly chooses to update every caller. Prefer private child modules plus exact parent re-exports:

```rust
mod schema;

pub(super) use schema::StoredReport;
```

Do not widen an item to `pub(crate)` only to make a split compile.

When an item moves into a deeper private child module, preserve the old path with private `mod child;`, `pub` child items, and a narrow parent re-export such as `pub(super) use child::Item;`. This keeps the child path closed while making the item visible enough for the parent re-export. Avoid `pub(in crate::...)`; this repo's own diagnostics forbid that visibility spelling. Reserve `pub(crate)` for items that were already crate-visible.

### File-to-Directory Migration

Each split starts with a no-op source move:

1. Move `module.rs` to `module/mod.rs`.
2. Compile before extracting child files.
3. Never leave both `module.rs` and `module/mod.rs`.
4. Update relative imports whose meaning changes after nesting, especially `super::...` imports.

This keeps the split as a replacement of the flat module, not a parallel module tree.

### Test Placement

Move unit tests with the private behavior they cover. If a test needs to cover coordination across children, keep it in the parent `mod.rs` and use `pub(super)` child APIs rather than widening production items to `pub(crate)`.

### Phase Verification

Each phase must name its path audit and targeted tests before editing. The baseline gate after each phase is:

```bash
cargo check --workspace --all-targets
cargo +nightly fmt --all
cargo nextest run <phase-specific filter>
RUSTC_BOOTSTRAP=1 cargo +stable run -- --workspace --all-targets --fail-on-warn
```

The final gate remains full-workspace and warning-sensitive:

```bash
cargo clippy --workspace --all-targets -- -D warnings
cargo nextest run
RUSTC_BOOTSTRAP=1 cargo +stable run -- --workspace --all-targets --fail-on-warn
```

Skip-sensitive tests must be called out in the phase closeout. If a gate depends on a test that returns early when `CARGO_MEND_SKIP_NETWORK_TESTS` is set, either run with that environment variable unset or pair the gate with non-skipping unit coverage.

After extraction, record the final module-qualified test path or the nonzero `nextest` run result for every targeted filter. This matters when unit tests move into child modules and the old parent test path no longer appears in output.

When tests need to build fixtures with crate-local types from another module, prefer `#[cfg(test)]` parent re-exports or test fixture constructors over widening production visibility.

### Verification Matrix

Use this as the first pass for phase-specific `nextest` filters. Expand it during ownership inventory if a moved helper has more local tests.

| Phase | Required filters |
|---|---|
| 1 - Compiler Build | `plain_building_progress_line_is_treated_as_progress`, `progress_line_with_embedded_warning_is_not_treated_as_progress`, `classify_suppresses_unused_import_when_warning_follows_progress_prefix`, `quiet_builds_do_not_accumulate_compiler_warning_summary_counts`, `json_mode_has_no_progress_status`, `forwarded_diagnostic_stops_progress_before_printing`, `json_success_emits_clean_stdout_and_no_stderr_noise` |
| 2 - Compiler Persistence | Add focused unit tests for malformed JSON ignored, wrong schema/fingerprint rejected, missing crate root rejected, canonical selected roots accepted, empty `package_root` compatibility retained, lib/test `cache_filename_for` separation, and serialized driver reports loading back through `load_report` |
| 3 - Config CLI | `version_flag_prints_version_and_exits_successfully`, `build_info_flag_prints_build_metadata_and_exits_successfully`, `default_invocation_from_package_root_reports_findings`, `positional_manifest_path_reports_findings`, `workspace_flag_from_workspace_root_reports_member_findings`, `workspace_all_targets_includes_example_target_findings`, `lib_flag_limits_analysis_to_library_target`, `named_example_limits_analysis_to_example_target`, `fix_dry_run_smoke_reports_and_preserves_files`, `runs_compiler_fix_true_for_apply_all`, `dry_run_with_fix_all_does_not_mutate` |
| 4 - Import Fixes | `validated_fix_set_allows_adjacent_non_overlapping_ranges`, `fix_rewrites_local_crate_import`, `fix_rolls_back_on_failed_cargo_check`, `dry_run_reports_import_fixes_without_editing_files`, `deep_super_is_flagged_and_fixed`, `triple_super_is_flagged_and_fixed` |
| 4/5 - Shared Fix Contract | `fix_rolls_back_on_failed_cargo_check`, `dry_run_reports_import_fixes_without_editing_files`, `fix_pub_use_rolls_back_on_failed_cargo_check`, `dry_run_reports_pub_use_fixes_without_editing_files`, add focused coverage for overlapping `ShortenImport` and `PreferModuleImport` fixes before extraction |
| 5 - Fix Runner | `fix_reports_when_nothing_is_fixable`, `fix_reports_noop_notice_after_summary`, `fix_reports_applied_notice_after_summary`, `fix_pub_use_reports_when_nothing_is_fixable`, `fix_pub_use_rewrites_pub_super_parent_facade_in_apply_mode`, `fix_all_converges_in_one_invocation` |
| 6 - Reporting Render | `render_human_report_prints_no_findings_when_empty`, `render_human_report_shows_combined_summary_for_mend_and_compiler_findings`, `summary_lists_one_line_per_fix_flag_plus_fix_all_aggregate`, `errors_render_in_their_own_block_above_summary`, `fixture_renders_every_current_diagnostic`, `successive_json_runs_reuse_cached_findings_for_same_scope`, `json_success_emits_clean_stdout_and_no_stderr_noise` |

Skip-sensitive filters currently include `fix_compiler_does_not_remove_reexport_used_only_by_cfg_test_code`, `fix_pub_use_reports_import_cleanup_suggestion_after_summary`, `fix_pub_use_self_heals_unused_imports_left_behind`, and `fix_all_converges_in_one_invocation`.

## Phase 1 - Compiler Build (Complete)

Target: `src/compiler/build.rs`

Current responsibilities:

- Cargo command construction and execution.
- Cargo wrapper environment setup.
- stderr streaming and diagnostic block classification.
- Compiler warning-summary parsing.
- Progress display state and terminal rendering.
- Tests for progress and diagnostic parsing.

Candidate extraction order:

1. Move `src/compiler/build.rs` to `src/compiler/build/mod.rs` and compile before extracting children.
2. Move progress display types and helpers into a child module owned by `build`.
3. Move stderr diagnostic classification helpers into a child module owned by `build`.
4. Keep `run_selection`, `run_cargo_fix`, `SelectionResult`, `BuildOutputMode`, and `CARGO_MANIFEST_FILE` available through `src/compiler/mod.rs`.
5. Recheck whether the remaining command-runner code is still large enough to split.
6. Keep orchestration in `build/mod.rs`: `stderr` returns typed observations and diagnostic blocks; `progress` owns terminal progress state; the private progress trait stays at the boundary used by `flush_diagnostic_block`.

Initial candidate files:

- `src/compiler/build/mod.rs`
- `src/compiler/build/progress.rs`
- `src/compiler/build/stderr.rs`

Verification:

- Path audit for `crate::compiler::build`, `super::constants`, and `super::settings` callers.
- Targeted tests that cover compiler stderr handling, warning-summary parsing, and progress rendering.

Ownership inventory:

| Item | Kind | Current path | Visibility | Current callers | Impl blocks | Co-moving helpers | Test home | Proposed home | Retained path |
|---|---|---|---|---|---|---|---|---|---|
| `CARGO_MANIFEST_FILE` | Constant | `compiler::build::CARGO_MANIFEST_FILE` | `pub(crate)` | Re-exported by `compiler`; used by selection modules | None | None | Existing selection tests | `compiler::build` parent | `crate::compiler::CARGO_MANIFEST_FILE` |
| `BuildOutputMode` | Enum | `compiler::build::BuildOutputMode` | `pub(crate)` | Re-exported by `compiler`; used by fixes runner and build orchestration | None | Progress status selection | Existing build and fixes tests | `compiler::build` parent | `crate::compiler::BuildOutputMode` |
| `SelectionResult` | Struct | `compiler::build::SelectionResult` | `pub(crate)` | Re-exported by `compiler`; used by fixes runner | None | `run_selection` result assembly | Existing integration tests | `compiler::build` parent | `crate::compiler::SelectionResult` |
| `run_selection` | Function | `compiler::build::run_selection` | `pub(crate)` | Re-exported by `compiler`; used by main and fixes runner | None | `run_cargo_check`, `scope_fingerprint_for`, `run_cargo_command` | Existing integration tests | `compiler::build` parent | `crate::compiler::run_selection` |
| `run_cargo_fix` | Function | `compiler::build::run_cargo_fix` | `pub(crate)` | Re-exported by `compiler`; used by main | None | Cargo command setup for fix mode | Existing fix-mode tests | `compiler::build` parent | `crate::compiler::run_cargo_fix` |
| `CommandOutcome` | Struct | `compiler::build::CommandOutcome` | Private | `run_cargo_command` and `run_selection` | None | `StderrObservation` fields | Parent-private behavior | `compiler::build` parent | Private |
| `DiagnosticBlockKind` | Enum | `compiler::build::DiagnosticBlockKind` | `pub(super)` | Build unit tests only | None | `classify_diagnostic_block`, warning-summary parser | Move with stderr tests | `compiler::build::stderr` | Private child API re-exported to parent only if needed |
| `StderrObservation` | Struct | `compiler::build::StderrObservation` | Private | `stream_cargo_stderr`, `run_cargo_command` | None | `SuppressionNotice`, `flush_diagnostic_block` | Move with stderr tests | `compiler::build::stderr` | Private |
| `SuppressionNotice` | Enum | `compiler::build::SuppressionNotice` | Private | `stream_cargo_stderr`, `flush_diagnostic_block`, tests | None | Suppression status notice handling | Move with stderr tests | `compiler::build::stderr` | Private |
| `ProgressStatus` | Enum | `compiler::build::ProgressStatus` | Private | `should_forward_progress_line`, tests | `From<bool>` | Progress-line forwarding predicate | Move with stderr tests | `compiler::build::stderr` | Private |
| `stream_cargo_stderr` | Function | `compiler::build::stream_cargo_stderr` | Private | `run_cargo_command` | None | Diagnostic block flush and progress coordination | Move with stderr tests | `compiler::build::stderr` | `pub(super)` child API |
| `is_progress_line` and `sanitize_for_match` | Functions | `compiler::build::*` | `pub(super)` | Build tests only | None | Progress-prefix constants, ANSI cleanup | Move with stderr tests | `compiler::build::stderr` | Private child API unless later sibling needs it |
| `classify_diagnostic_block` and warning-summary helpers | Functions | `compiler::build::*` | `pub(super)` and private | Build tests only | None | `DiagnosticBlockKind`, unused-import matching | Move with stderr tests | `compiler::build::stderr` | Private child API |
| `flush_diagnostic_block` | Function | `compiler::build::flush_diagnostic_block` | Private | `stream_cargo_stderr`, tests | None | `ProgressDisplay` trait boundary | Move with stderr tests | `compiler::build::stderr` | Private |
| `ProgressDisplay` | Trait | `compiler::build::ProgressDisplay` | Private | `flush_diagnostic_block`, `CargoProgress`, tests | `CargoProgress`, test recorder | Status notice and stop methods | Move with stderr tests | `compiler::build::stderr` | Private |
| `CargoProgress` and `CargoProgressState` | Structs | `compiler::build::*` | Private | `stream_cargo_stderr` | `start`, `stop`, `Drop`, `ProgressDisplay` | Frame formatting and line clearing | Move progress tests | `compiler::build::progress` | `pub(super)` child API for stderr |
| `progress_message_for`, `progress_frame`, `progress_line_width`, `clear_progress_line` | Functions | `compiler::build::*` | Private | `CargoProgress`, tests | None | Progress frames and terminal cleanup | Move progress tests | `compiler::build::progress` | Private child API |

### Retrospective

**What worked:**

- `src/compiler/build.rs` moved to `src/compiler/build/mod.rs`, then compiled before child extraction.
- `build/mod.rs` stayed the command owner; `progress.rs` owns terminal status rendering, and `stderr.rs` owns progress-line classification plus diagnostic block handling.
- Existing `crate::compiler::{run_selection, run_cargo_fix, SelectionResult, BuildOutputMode, CARGO_MANIFEST_FILE}` paths stayed unchanged through `src/compiler/mod.rs`.

**What deviated from the plan:**

- `ProgressDisplay` moved with `progress.rs`, not `stderr.rs`; `stderr.rs` imports the trait as its test boundary.
- `DiagnosticBlockKind`, `is_progress_line`, and `sanitize_for_match` no longer need parent-visible re-exports because only stderr unit tests use them.

**Surprises:**

- No current caller imports `crate::compiler::build::*`; all external use already went through `crate::compiler::*`.
- Moving tests with private helpers split the required Phase 1 filters across `progress.rs`, `stderr.rs`, and the existing CLI smoke test.

**Implications for remaining phases:**

- Later phases should prefer private child APIs plus parent-owned public re-exports, and only retain child-visible APIs when another child uses them.
- Each later phase should record whether targeted filters moved into child-module test paths so nonzero `nextest` matches remain explicit.

### Phase 1 Review

- Phase 2 now names the direct `crate::compiler::persistence` callers in `compiler::visibility` and requires parent-path preservation for schema and sink items.
- Phase 2 now keeps write-side ownership in `persistence/mod.rs` until the load split is complete unless inventory proves a single write-side child.
- Phase 3 now treats `RawManifestCli` plus parsed `ManifestCli` as required because `main.rs` reads `cli.manifest.config`.
- Phase 4/5 preflight now verifies the existing split between `runner.rs` combination logic and `imports.rs` snapshot/apply/restore before either file moves.
- Phase 5 now requires combined-fix tests before moving `FixScans` or `MendRunner::combined_fixes` into a child module.
- Global verification now requires final module-qualified test paths or nonzero `nextest` proof after child-module test moves.

## Phase 2 - Compiler Persistence (Complete)

Target: `src/compiler/persistence.rs`

Current responsibilities:

- Stored report schema.
- Findings directory I/O.
- Report selection matching.
- Cross-compilation intersection.
- Caller-aware suppression.
- Visibility-priority merging.
- Path relativization.
- Cache filename construction.

Candidate extraction order:

1. Move `src/compiler/persistence.rs` to `src/compiler/persistence/mod.rs` and compile before extracting children.
2. Treat stored schema types as an explicit internal schema boundary; preserve existing `crate::compiler::persistence::*` paths for current compiler descendants.
3. Record field-level construction and mutation callers for `StoredReport`, `StoredFinding`, `StoredPubUseFixFact`, and `FindingsSink`.
4. Define the write-side boundary for schema construction, findings sink mutation, and cache filename selection before extracting load helpers.
5. Keep direct parent paths for `compiler::visibility` callers: `StoredReport`, `StoredFinding`, `StoredPubUseFixFact`, `FindingsSink`, `UseSite`, `CacheBuildKind`, and `cache_filename_for` currently come from `crate::compiler::persistence`.
6. Keep `FindingsSink`, schema construction, and cache filename selection in `persistence/mod.rs` until the load split is complete, unless the ownership inventory names a single write-side child and preserves the same parent paths.
7. Move selection matching and path relativization together if they are used only by report loading.
8. Keep the load-time suppression order under one owner: cross-compilation intersection, caller-aware suppression, then visibility-priority merging.
9. Add focused tests for report matching, cross-compilation intersection, caller-aware suppression, and visibility-priority merging before extracting those helpers.
10. Keep `StoredReport`, `UseSite`, `StoredFinding`, `StoredPubUseFixFact`, `FindingsSink`, `prepare_findings_dir`, `load_report`, `CacheBuildKind`, and `cache_filename_for` reachable to existing compiler callers.

Initial candidate files:

- `src/compiler/persistence/mod.rs`
- `src/compiler/persistence/schema.rs`
- `src/compiler/persistence/load.rs`
- `src/compiler/persistence/intersection.rs`
- `src/compiler/persistence/visibility_priority.rs`
- Optional only after inventory: `src/compiler/persistence/write.rs` or `src/compiler/persistence/cache.rs`

Verification:

- Path audit for `crate::compiler::persistence`.
- Focused persistence tests for stored report loading, cross-compilation intersection, caller-aware suppression, and visibility narrowing priority.
- Write-side tests for lib/test cache filename separation and driver report round-trip loading.
- Negative tests for malformed JSON, schema mismatch, analysis fingerprint mismatch, config fingerprint mismatch, missing crate root, selected-root mismatch, canonical selected roots, and empty `package_root` compatibility.
- Record final module-qualified test paths or nonzero `nextest` matches after moving tests into `schema`, `load`, `intersection`, or `visibility_priority` children.

Ownership inventory:

| Item | Kind | Current path | Visibility | Current callers | Impl blocks | Co-moving helpers | Test home | Proposed home | Retained path |
|---|---|---|---|---|---|---|---|---|---|
| `StoredReport` | Struct | `compiler::persistence::StoredReport` | `pub(super)` | `visibility_context` constructs; `load_report` deserializes and consumes | Serde derives | `StoredFinding`, `StoredPubUseFixFact`, `UseSite` fields | Schema round-trip and load tests | `compiler::persistence::schema` | `crate::compiler::persistence::StoredReport` |
| `UseSite` | Struct | `compiler::persistence::UseSite` | `pub(super)` | `visibility::use_sites` constructs; caller-aware suppression reads | Serde derives | Caller module/target def-path fields | Caller-aware suppression tests | `compiler::persistence::schema` | `crate::compiler::persistence::UseSite` |
| `StoredFinding` | Struct | `compiler::persistence::StoredFinding` | `pub(super)` | `visibility::source` constructs; load/intersection/priority read | Serde derives | Narrowing def-path fields | Intersection and priority tests | `compiler::persistence::schema` | `crate::compiler::persistence::StoredFinding` |
| `StoredPubUseFixFact` | Struct | `compiler::persistence::StoredPubUseFixFact` | `pub(super)` | `visibility::scan::record` constructs; load converts | Serde derives | Pub-use fact fields | Load round-trip tests | `compiler::persistence::schema` | `crate::compiler::persistence::StoredPubUseFixFact` |
| `FindingsSink` | Struct | `compiler::persistence::FindingsSink` | `pub(super)` | `visibility::field`, `visibility::scan::record`, `visibility::scan::visit`, and `visibility_context` mutate | `Default` derive | Schema vectors only | Write-side driver report tests | `compiler::persistence` parent | `crate::compiler::persistence::FindingsSink` |
| `prepare_findings_dir` | Function | `compiler::persistence::prepare_findings_dir` | `pub(super)` | `compiler::build` | None | Findings directory name constant | Existing build integration tests | `compiler::persistence` parent | `crate::compiler::persistence::prepare_findings_dir` |
| `load_report` | Function | `compiler::persistence::load_report` | `pub(super)` | `compiler::build` | None | Selection matching, suppression order, path relativization, sorting/dedup | Load tests | `compiler::persistence::load` | `crate::compiler::persistence::load_report` |
| Selection matching helpers | Functions | `compiler::persistence::*` | Private | `load_report` only | None | Root canonicalization, schema/fingerprint checks, crate-root existence | Load negative tests | `compiler::persistence::load` | Private child API |
| Report conversion helpers | Functions | `compiler::persistence::*` | Private | `load_report` only | None | `selection_root_string`, `relativize_path`, finding/pub-use dedup | Load round-trip tests | `compiler::persistence::load` | Private child API |
| Caller-aware suppression helpers | Functions | `compiler::persistence::*` | Private | `load_report` only | None | `def_path_is_descendant` | Caller-aware tests | `compiler::persistence::caller_aware` | `pub(super)` child API |
| Cross-compilation intersection helpers | Functions | `compiler::persistence::*` | Private | `load_report` only | None | Agreement predicate and intersection key | Intersection tests | `compiler::persistence::intersection` | `pub(super)` child API |
| Visibility-priority helpers | Functions | `compiler::persistence::*` | Private | `load_report` only | None | Priority key | Priority tests | `compiler::persistence::visibility_priority` | `pub(super)` child API |
| `CacheBuildKind` and `cache_filename_for` | Enum and function | `compiler::persistence::*` | `pub(super)` | `visibility_context` chooses cache filenames | Hash/Copy derives | Package root, crate root, build kind hash | Cache filename tests | `compiler::persistence` parent | `crate::compiler::persistence::{CacheBuildKind, cache_filename_for}` |

### Retrospective

**What worked:**

- `src/compiler/persistence.rs` moved to `src/compiler/persistence/mod.rs`, then compiled before child extraction.
- `schema.rs`, `load.rs`, `caller_aware.rs`, `intersection.rs`, and `visibility_priority.rs` now own the behavior named in the inventory.
- Existing `crate::compiler::persistence::{StoredReport, StoredFinding, StoredPubUseFixFact, FindingsSink, UseSite, CacheBuildKind, cache_filename_for, prepare_findings_dir, load_report}` paths still compile.
- Focused persistence tests were added before extraction, then moved with `load`, `caller_aware`, `intersection`, and `visibility_priority`.

**What deviated from the plan:**

- Added `caller_aware.rs` as a distinct child because caller-aware suppression has its own data pass.
- Used private child modules with `pub` child items and exact `pub(super)` parent re-exports instead of `pub(in crate::compiler)`, because this repo forbids `pub(in crate::...)`.
- Added a test-only `SelectionScope` re-export in `src/selection/mod.rs` so persistence unit tests can construct `Selection`.
- Ran the checkout self-audit during the phase and fixed Phase 1 deep-`super` imports in `src/compiler/build/progress.rs` and `src/compiler/build/stderr.rs`.

**Surprises:**

- Persistence had no existing unit tests, so the phase needed fixture helpers for stored report loading.
- The checkout self-audit found Phase 1 import warnings before it found any persistence visibility issue.

**Implications for remaining phases:**

- For private child modules that need parent re-exports, prefer private `mod child;`, `pub` child items, and narrow parent `pub(super) use` exports over `pub(in crate::...)`.
- Run the checkout self-audit after each remaining phase when new child modules introduce imports.
- If unit tests need to construct crate-local types from another module, add a test-only re-export or constructor instead of widening production visibility.

### Phase 2 Review

- Global visibility rules now replace the old `pub(in crate::...)` example with private child modules, `pub` child items, and narrow parent re-exports.
- Phase 3 now requires a two-layer CLI path audit: `crate::config::*` facade paths plus direct `config::cli` sibling paths.
- Phase 4/5 preflight now treats rollback, dry-run, and conflict filtering as existing coverage to prove by name, then adds only missing `ShortenImport` / `PreferModuleImport` overlap tests.
- Phase 4 now requires deciding `snapshot_files`, `apply_fixes`, and `restore_files` ownership before import scan/path extraction.
- Per-phase verification now includes the checkout `cargo mend --fail-on-warn` self-audit.
- Global test guidance now prefers `#[cfg(test)]` re-exports or fixture constructors over widened production visibility.

## Phase 3 - Config CLI (Complete)

Target: `src/config/cli.rs`

Current responsibilities:

- Public parsed CLI types.
- Raw `clap` parser structs.
- Workspace and target-selection groups.
- Fix-action groups.
- Conversion from raw parser structs to internal config.
- Run-mode tests.

Candidate extraction order:

1. Move `src/config/cli.rs` to `src/config/cli/mod.rs` and compile before extracting children.
2. Record both retained caller layers before extraction: `crate::config::*` facade re-exports from `config/mod.rs`, and direct `crate::config::cli::*` / `super::cli::*` sibling paths used by `config/run_mode.rs` and its tests.
3. Keep public parsed types in the parent module if callers import them from `config::cli`.
4. Move raw `clap` structs into a parser child module if they are file-private implementation details.
5. Split `ManifestCli` into private `RawManifestCli` plus parsed `ManifestCli`; `main.rs` reads `cli.manifest.config`, so keep parsed `ManifestCli` as the parent-facing result unless this phase intentionally updates that caller.
6. Move conversion impls next to the raw structs unless the public types own the conversion behavior.
7. Move fix-action parser structs into a child module only if it can keep a small API to the parent.
8. Keep raw `clap` structs private to the CLI module tree; expose typed parsed results from the parent.
9. Keep `parse`, `Cli`, `CargoCheckCli`, `ManifestCli`, `FixCli`, `FixRequest`, `WorkspaceSelection`, `TargetSelection`, `FixExecution`, `BuildInfoMode`, and `WarningPolicy` available to current callers.

Initial candidate files:

- `src/config/cli/mod.rs`
- `src/config/cli/raw.rs`
- `src/config/cli/fix.rs`
- `src/config/cli/target.rs`

Verification:

- Path audit for `crate::config::*`, `super::cli`, and `crate::config::cli`.
- CLI parser tests moved with their private raw parser behavior.
- CLI smoke tests that cover install/check/fix argument parsing.
- Record final module-qualified test paths or nonzero `nextest` matches after moving parser tests into `raw`, `fix`, or `target` children.

Ownership inventory:

| Item | Kind | Current path | Visibility | Current callers | Impl blocks | Co-moving helpers | Test home | Proposed home | Retained path |
|---|---|---|---|---|---|---|---|---|---|
| `parse` | Function | `config::cli::parse` | `pub(crate)` | Re-exported by `config`; used by `main` | None | `normalized_args`, top-level raw parser | CLI smoke tests | `config::cli` parent delegates to `raw` | `crate::config::parse` |
| `Cli` | Struct | `config::cli::Cli` | `pub(crate)` | Returned by `parse`; fields read by `main` | None | `BuildInfoMode`, `WarningPolicy`, parsed child values | CLI smoke tests | `config::cli` parent | `crate::config::cli::Cli` |
| `BuildInfoMode` | Enum | `config::cli::BuildInfoMode` | `pub(crate)` | Re-exported by `config`; used by `main` | None | Top-level raw conversion | CLI smoke tests | `config::cli` parent | `crate::config::BuildInfoMode` |
| `WarningPolicy` | Enum | `config::cli::WarningPolicy` | `pub(crate)` | Re-exported by `config`; used by `main` | None | Top-level raw conversion | CLI smoke tests | `config::cli` parent | `crate::config::WarningPolicy` |
| `ManifestCli` | Struct | `config::cli::ManifestCli` | `pub(crate)` | `main` reads `cli.manifest.config` | `clap::Args` derive currently mixed into parsed type | Split raw manifest parser from parsed result | CLI smoke tests | Parsed type in `config::cli` parent; raw parser in `raw` | `crate::config::cli::ManifestCli` |
| `CargoCheckCli` | Struct | `config::cli::CargoCheckCli` | `pub(crate)` | Re-exported by `config`; used by selection modules and tests | `explicit_manifest_path` | Workspace and target parsed enums, raw cargo parser conversion | CLI smoke and selection tests | `config::cli::target` | `crate::config::CargoCheckCli` and `crate::config::cli::CargoCheckCli` |
| `WorkspaceSelection` | Enum | `config::cli::WorkspaceSelection` | `pub(crate)` | Re-exported by `config`; used by selection metadata tests | None | Raw workspace flag conversion | CLI smoke and selection tests | `config::cli::target` | `crate::config::WorkspaceSelection` and `crate::config::cli::WorkspaceSelection` |
| `TargetSelection` | Enum | `config::cli::TargetSelection` | `pub(crate)` | Re-exported by `config`; used by display filter and metadata tests | None | Raw target flag conversion | CLI smoke, display filter, and metadata tests | `config::cli::target` | `crate::config::TargetSelection` and `crate::config::cli::TargetSelection` |
| `FixCli` | Struct | `config::cli::FixCli` | `pub(crate)` | `config::run_mode`, its tests, and `main` through `Cli` | `includes`, `runs_compiler_fix` | Fix execution enum, fix request enum, raw fix parser conversion | CLI parser and run-mode tests | `config::cli::fix` | `crate::config::cli::FixCli` |
| `FixExecution` | Enum | `config::cli::FixExecution` | `pub(crate)` | Re-exported by `config`; used by `main`, `run_mode`, and tests | None | Raw execution flag conversion | CLI parser and run-mode tests | `config::cli::fix` | `crate::config::FixExecution` and `crate::config::cli::FixExecution` |
| `FixRequest` | Enum | `config::cli::FixRequest` | `pub(crate)` | `config::run_mode` and its tests | None | Raw fix flag conversion | CLI parser and run-mode tests | `config::cli::fix` | `crate::config::cli::FixRequest` |
| `RawCli` and `RawManifestCli` | Structs | `config::cli::RawCli`, `config::cli::ManifestCli` | Private | `parse` and parser unit tests | `From<RawCli> for Cli`, `From<RawManifestCli> for ManifestCli` | `normalized_args` | Parser tests that build `clap` matches | `config::cli::raw` | Private to `config::cli` tree |
| Raw cargo parser structs | Structs | `config::cli::RawCargoCheckCli` and target groups | Private | `RawCli` flatten and cargo conversion | `From<RawCargoCheckCli> for CargoCheckCli` | Workspace and target flag groups | CLI smoke and selection tests | `config::cli::target` | Private to `config::cli` tree |
| Raw fix parser structs | Structs | `config::cli::RawFixCli` and fix groups | Private | `RawCli` flatten and fix conversion | `From<RawFixCli> for FixCli` | Fix flag and execution flag groups | CLI parser and run-mode tests | `config::cli::fix` | Private to `config::cli` tree |

### Retrospective

**What worked:**

- `src/config/cli.rs` moved to `src/config/cli/mod.rs`, then compiled before child extraction.
- `raw.rs` owns the top-level `clap` parser and `RawManifestCli`; parsed `ManifestCli` stays in the parent so `main` can keep reading `cli.manifest.config`.
- `target.rs` owns `CargoCheckCli`, workspace selection, target selection, and raw cargo target groups.
- `fix.rs` owns `FixCli`, `FixExecution`, `FixRequest`, raw fix groups, and fix-mode unit tests.
- Existing `crate::config::{parse, BuildInfoMode, CargoCheckCli, FixExecution, TargetSelection, WarningPolicy, WorkspaceSelection}` paths still compile through `config/mod.rs`.
- Existing direct `crate::config::cli::{FixCli, FixExecution, FixRequest}` and `super::cli::*` paths in `config/run_mode.rs` still compile through exact parent re-exports.

**What deviated from the plan:**

- `FixCli`, `FixExecution`, `FixRequest`, `CargoCheckCli`, `WorkspaceSelection`, and `TargetSelection` moved into owner-named child modules, with exact parent re-exports preserving `config::cli` paths.
- Methods that external callers use through re-exported types changed from `pub(crate)` to `pub` inside private child modules, because the checkout self-audit rejects newly nested `pub(crate)` methods there.

**Surprises:**

- The first checkout self-audit found three nested `pub(crate)` methods and three test-only deep `super` imports after extraction.
- The targeted unit-test filters moved to `config::cli::fix::tests::runs_compiler_fix_true_for_apply_all` and `config::cli::raw::tests::dry_run_with_fix_all_does_not_mutate`.

**Verification results:**

- `cargo check --workspace --all-targets` passed after the no-op move and after extraction.
- `cargo +nightly fmt --all` passed.
- `cargo nextest run -E '<phase 3 filter set>'` ran 11 tests and passed all 11.
- Final target paths:
  - `tests/cli_smoke.rs::version_flag_prints_version_and_exits_successfully`
  - `tests/cli_smoke.rs::build_info_flag_prints_build_metadata_and_exits_successfully`
  - `tests/cli_smoke.rs::default_invocation_from_package_root_reports_findings`
  - `tests/cli_smoke.rs::positional_manifest_path_reports_findings`
  - `tests/cli_smoke.rs::workspace_flag_from_workspace_root_reports_member_findings`
  - `tests/cli_smoke.rs::workspace_all_targets_includes_example_target_findings`
  - `tests/cli_smoke.rs::lib_flag_limits_analysis_to_library_target`
  - `tests/cli_smoke.rs::named_example_limits_analysis_to_example_target`
  - `tests/cli_smoke.rs::fix_dry_run_smoke_reports_and_preserves_files`
  - `src/config/cli/fix.rs::tests::runs_compiler_fix_true_for_apply_all`
  - `src/config/cli/raw.rs::tests::dry_run_with_fix_all_does_not_mutate`
- `RUSTC_BOOTSTRAP=1 cargo +stable run -- --workspace --all-targets --fail-on-warn` first failed on the nested method visibility and deep test imports, then passed with `No findings`.

**Implications for remaining phases:**

- When a `pub(crate)` method moves into a private child but callers use it through a re-exported type, prefer `pub` on the method plus a narrow parent re-export of the type.
- Keep raw parser or raw scan structs private to their module tree, and expose typed parsed or validated results through the parent.
- Continue running the checkout self-audit after each phase because it catches nested visibility and import issues that `cargo check` allows.

### Phase 3 Review

- Review found no implementation defects and no user decisions.
- Retained paths were confirmed for `config::cli` parent re-exports, `config/mod.rs` facade re-exports, and `main.rs` access to `cli.manifest.config`.
- The only review finding was documentation completeness: Phase 3 needed exact final test paths for all 11 filters, now recorded above.
- Phase 4/5 should apply the Phase 3 visibility lesson when moving re-exported child-owned types: use private child modules, exact parent re-exports, and `pub` methods on re-exported types when callers need those methods.

## Phase 4/5 Preflight - Fix Edit Boundary (Complete)

Targets: `src/fixes/imports.rs` and `src/fixes/runner.rs`

This is a blocking gate before either file is moved. Verify and document the existing shared edit contract, prove existing coverage by test name, then add the missing overlap coverage before moving either file:

- `UseFix`
- `ImportGroup`
- `ValidatedFixSet`
- `snapshot_files`
- `apply_fixes`
- `restore_files`
- `combined_fixes` in `runner.rs`
- conflicting import-group filtering in `runner.rs`
- the owner for any snapshot newtype

Current producers and consumers include multiple fix modules, not only `runner`. Keep all producers returning `UseFix`, all consumers accepting `ValidatedFixSet`, and preserve `fixes::imports::{UseFix, ImportGroup, ValidatedFixSet}` unless the phase explicitly updates every caller.

Treat snapshot/apply/restore in `imports.rs` as one preserved API until the runner transaction boundary is settled. Consider a private snapshot newtype during implementation so rollback code cannot receive an arbitrary `Vec<(PathBuf, String)>`.

Before splitting `imports.rs`, decide whether `snapshot_files`, `apply_fixes`, and `restore_files` stay parent-owned in `imports/mod.rs` or move to runner apply/rollback. Do not move scan/path helpers until that edit boundary is written down.

Record the ordered combination contract before extraction:

1. Collect `PreferModuleImport` fix ranges for overlap detection.
2. Add `ShortenImport` fixes only when they do not overlap `PreferModuleImport`.
3. Collect all remaining fix producers.
4. Drop conflicting `ImportGroup`s.
5. Validate replacement ranges through `ValidatedFixSet`.

Existing coverage to prove by name before new tests:

- `fix_rolls_back_on_failed_cargo_check`
- `dry_run_reports_import_fixes_without_editing_files`
- `fix_pub_use_rolls_back_on_failed_cargo_check`
- `dry_run_reports_pub_use_fixes_without_editing_files`
- `no_conflicts_pass_through_unchanged`
- `same_bare_name_different_paths_drops_all_tagged`
- `same_bare_name_same_full_path_kept`
- `conflict_isolated_per_file`
- `untagged_fixes_always_pass_through_even_with_conflicts`

The preflight is complete only after those existing tests pass and focused overlapping `ShortenImport`/`PreferModuleImport` tests around `MendRunner::combined_fixes` are named or added and passing.

Add or preserve the combined-fix tests while `FixScans` and `MendRunner::combined_fixes` are still easy to test inside `runner.rs`. Do this before moving combination logic into any `runner` child module.

Preflight contract inventory:

| Item | Current owner | Producers | Consumers | Preflight decision |
|---|---|---|---|---|
| `UseFix` | `src/fixes/imports.rs` | `imports`, `prefer_module_import`, `inline_path_qualified_type`, `unused_pub`, `narrow_pub_crate`, `field_visibility`, `imports_at_top`, `pub_use_fixes` | `ValidatedFixSet`, `MendRunner::combined_fixes`, `snapshot_files`, `apply_fixes` | Preserve `fixes::imports::UseFix` for Phases 4 and 5. |
| `ImportGroup` | `src/fixes/imports.rs` | Grouped import producers such as `prefer_module_import`, `inline_path_qualified_type`, and `imports_at_top` | `drop_conflicting_import_groups` | Preserve `fixes::imports::ImportGroup`; keep conflict filtering in runner until Phase 5 inventory. |
| `ValidatedFixSet` | `src/fixes/imports.rs` | Each scan converts proposed `UseFix` values through `ValidatedFixSet::try_from` | Runner apply/dry-run flow, `snapshot_files`, `apply_fixes` | Preserve `fixes::imports::ValidatedFixSet`; keep non-overlap validation before source writes. |
| `snapshot_files` / `apply_fixes` / `restore_files` | `src/fixes/imports.rs` | Runner apply path | Runner rollback and validation path | Keep together in `imports/mod.rs` during Phase 4; Phase 5 may add a private snapshot newtype only after runner ownership inventory. |
| `MendRunner::combined_fixes` | `src/fixes/runner.rs` | Reads all optional `FixScans` | Produces a `ValidatedFixSet` for apply/notice flow | Keep in `runner` during Phase 4/5 preflight; move only during Phase 5 after tests pin ordering. |
| `drop_conflicting_import_groups` | `src/fixes/runner.rs` | Reads combined `UseFix` list | Called only by `combined_fixes` | Keep with combination logic for Phase 5. |

Preflight coverage proof:

| Contract | Test path |
|---|---|
| Import rollback restores files after failed validation | `tests/diagnostics/import_fixes.rs::fix_rolls_back_on_failed_cargo_check` |
| Import dry-run leaves files unchanged | `tests/diagnostics/import_fixes.rs::dry_run_reports_import_fixes_without_editing_files` |
| Pub-use rollback restores files after failed validation | `tests/diagnostics/pub_use_fixes.rs::fix_pub_use_rolls_back_on_failed_cargo_check` |
| Pub-use dry-run leaves files unchanged | `tests/diagnostics/pub_use_fixes.rs::dry_run_reports_pub_use_fixes_without_editing_files` |
| Adjacent ranges are accepted by `ValidatedFixSet` | `src/fixes/imports.rs::tests::validated_fix_set_allows_adjacent_non_overlapping_ranges` |
| No group conflicts pass unchanged | `src/fixes/runner.rs::tests::no_conflicts_pass_through_unchanged` |
| Same bare name with different paths drops every tagged fix | `src/fixes/runner.rs::tests::same_bare_name_different_paths_drops_all_tagged` |
| Same bare name and same full path is kept | `src/fixes/runner.rs::tests::same_bare_name_same_full_path_kept` |
| Group conflicts are isolated per file | `src/fixes/runner.rs::tests::conflict_isolated_per_file` |
| Untagged fixes survive grouped conflicts | `src/fixes/runner.rs::tests::untagged_fixes_always_pass_through_even_with_conflicts` |

Added coverage:

- `src/fixes/runner.rs::tests::combined_fixes_drops_shorten_import_when_prefer_module_import_overlaps`
- `src/fixes/runner.rs::tests::combined_fixes_keeps_adjacent_shorten_import_and_prefer_module_import`

Verification results:

- `cargo check --workspace --all-targets` passed.
- `cargo +nightly fmt --all` passed.
- `cargo nextest run -E '<phase 4/5 preflight filter set>'` ran 12 tests and passed all 12.
- `RUSTC_BOOTSTRAP=1 cargo +stable run -- --workspace --all-targets --fail-on-warn` passed with `No findings`.

### Retrospective

**What worked:**

- The preflight kept implementation scope to `src/fixes/runner.rs` tests and the plan doc.
- Existing rollback, dry-run, `ValidatedFixSet`, and import-group tests already covered most of the edit contract.
- New `MendRunner::combined_fixes` tests now pin the `ShortenImport` / `PreferModuleImport` overlap rule before either file moves.

**What deviated from the plan:**

- No snapshot newtype was introduced. The preflight decision is to keep `snapshot_files`, `apply_fixes`, and `restore_files` together in `imports/mod.rs` during Phase 4 and revisit any snapshot wrapper in Phase 5.

**Surprises:**

- `MendFailure` does not implement `std::error::Error`, so the new tests convert it to an `anyhow::Error` inside a test helper instead of using `?` directly.

**Implications for remaining phases:**

- Phase 4 should move scan/path code only after preserving `UseFix`, `ImportGroup`, `ValidatedFixSet`, `scan_selection`, `snapshot_files`, `apply_fixes`, and `restore_files` through `fixes::imports`.
- Phase 5 should keep `MendRunner::combined_fixes` and `drop_conflicting_import_groups` together unless its inventory identifies a single coordination child.
- Phase 5 may add a private snapshot newtype only if it keeps the rollback path and `imports` transaction helpers together.

### Phase 4/5 Preflight Review

- Phase 4 now treats `snapshot_files`, `apply_fixes`, and `restore_files` as retained `imports/mod.rs` APIs, not a Phase 4 relocation choice.
- Phase 4 now names the edit contract home: `UseFix`, `ImportGroup`, and `ValidatedFixSet` stay in `imports/mod.rs` for the first extraction, with `contract.rs` allowed only if exact parent re-exports are kept.
- Phase 5 now keeps `MendRunner::combined_fixes` and `drop_conflicting_import_groups` together and names optional `combine.rs` as the only child candidate for that logic.
- Phase 5 no longer reopens the Phase 4 transaction-helper decision; it only evaluates whether runner ownership needs a private snapshot newtype.
- Phase 5 verification now includes the two new `combined_fixes` overlap tests.
- Phase 5 notice extraction now requires a small notice input type or parent helper before moving notice construction into `notices.rs`.

## Phase 4 - Import Fixes (Complete)

Target: `src/fixes/imports.rs`

Current responsibilities:

- Import scan results.
- Fix validation and application.
- Source snapshot and restore.
- `syn` visitor scanning.
- Import candidate construction.
- Path formatting and byte-offset helpers.
- Tests.

Ownership inventory:

| Item | Kind | Current path | Visibility | Current callers | Impl blocks | Co-moving helpers | Test home | Proposed home | Retained path |
|---|---|---|---|---|---|---|---|---|---|
| `ImportScan` | Struct | `fixes::imports::ImportScan` | `pub(crate)` | `MendRunner::plan`, `MendRunner::build_selection`, runner tests | None | `scan_selection` result assembly | Import diagnostics tests | `fixes::imports` parent | `crate::fixes::imports::ImportScan` |
| `ImportGroup` | Struct | `fixes::imports::ImportGroup` | `pub(crate)` | `prefer_module_import`, `inline_path_qualified_type`, `imports_at_top`, runner conflict tests | None | Grouped `UseFix` values | Runner conflict tests | `fixes::imports` parent | `crate::fixes::imports::ImportGroup` |
| `UseFix` | Struct | `fixes::imports::UseFix` | `pub(crate)` | All fix producers and runner tests | None | `ImportGroup` field, source edit ranges | Import and runner tests | `fixes::imports` parent | `crate::fixes::imports::UseFix` |
| `ValidatedFixSet` | Struct | `fixes::imports::ValidatedFixSet` | `pub(crate)` | Fix scans, runner apply flow, tests | `TryFrom<Vec<UseFix>>`, `is_empty`, `iter` | Range sorting, dedup, overlap validation | Parent unit test | `fixes::imports` parent | `crate::fixes::imports::ValidatedFixSet` |
| `scan_selection` | Function | `fixes::imports::scan_selection` | `pub(crate)` | `MendRunner::plan`, `MendRunner::build_selection` | None | `scan_selection_with_fixes`, `scan_file` | Import diagnostics tests | Parent wrapper over `scan` child | `crate::fixes::imports::scan_selection` |
| `apply_fixes` | Function | `fixes::imports::apply_fixes` | `pub(crate)` | Runner apply flow | None | Reverse edit ordering | Rollback and dry-run diagnostics tests | `fixes::imports` parent | `crate::fixes::imports::apply_fixes` |
| `snapshot_files` | Function | `fixes::imports::snapshot_files` | `pub(crate)` | Runner apply flow | None | Unique edited path collection | Rollback diagnostics tests | `fixes::imports` parent | `crate::fixes::imports::snapshot_files` |
| `restore_files` | Function | `fixes::imports::restore_files` | `pub(crate)` | Runner rollback flow | None | Snapshot write-back | Rollback diagnostics tests | `fixes::imports` parent | `crate::fixes::imports::restore_files` |
| `ShortenImportFact` and `ImportFinding` | Structs | `fixes::imports::*` | Private | `scan_selection` path only | `From<ShortenImportFact> for Finding` | Finding conversion and `UseFix` payload | Import diagnostics tests | `fixes::imports::scan` | Private child behavior |
| `scan_selection_with_fixes`, `scan_file`, `UseVisitor` | Functions and struct | `fixes::imports::*` | Private | `scan_selection` only | `Visit` for `UseVisitor` | File walk, parse, module tracking | Import diagnostics tests | `fixes::imports::scan` | Private child behavior |
| `ImportCandidate`, `FlattenedImport`, and path helpers | Structs and functions | `fixes::imports::*` | Private | `UseVisitor::visit_item_use` only | None | `analyze_use_tree`, `analyze_deep_super`, `build_relative_path`, `format_path`, `line_offsets`, `offset` | Import diagnostics tests | `fixes::imports::path` | Private child behavior |

Candidate extraction order:

1. Move `src/fixes/imports.rs` to `src/fixes/imports/mod.rs` and compile before extracting children.
2. Keep `UseFix`, `ImportGroup`, `ValidatedFixSet`, `ImportScan`, `scan_selection`, `snapshot_files`, `apply_fixes`, and `restore_files` in `imports/mod.rs` for the first extraction.
3. Do not move `UseFix`, `ImportGroup`, or `ValidatedFixSet` into `scan.rs` or `path.rs`; if inventory later proves a contract child is useful, use `contract.rs` and preserve exact parent re-exports.
4. Keep `snapshot_files`, `apply_fixes`, and `restore_files` parent-owned in Phase 4; Phase 5 may revisit a private snapshot newtype during runner ownership inventory.
5. Move `syn` visitor scanning into a child module owned by imports.
6. Move import candidate and path-formatting helpers together if they form one local API.
7. Keep `ImportScan`, `ImportGroup`, `UseFix`, `ValidatedFixSet`, `scan_selection`, `apply_fixes`, `snapshot_files`, and `restore_files` available to all current fix-module callers.

Initial candidate files:

- `src/fixes/imports/mod.rs`
- `src/fixes/imports/scan.rs`
- `src/fixes/imports/path.rs`
- Optional only after a later inventory: `src/fixes/imports/contract.rs`

Verification:

- Path audit for `crate::fixes::imports`.
- Import-fix tests and diagnostics tests that assert rewritten `use` statements.
- Rollback and dry-run tests for source mutation behavior.
- Record final module-qualified test paths or nonzero `nextest` matches after moving tests into `scan`, `path`, or `apply` children.

Verification results:

- `cargo check --workspace --all-targets` passed after the no-op move and after extraction.
- `cargo +nightly fmt --all` passed.
- `cargo nextest run -E '<phase 4 filter set>'` ran 6 tests and passed all 6.
- Final target paths:
  - `src/fixes/imports/mod.rs::tests::validated_fix_set_allows_adjacent_non_overlapping_ranges`
  - `tests/diagnostics/import_fixes.rs::fix_rewrites_local_crate_import`
  - `tests/diagnostics/import_fixes.rs::fix_rolls_back_on_failed_cargo_check`
  - `tests/diagnostics/import_fixes.rs::dry_run_reports_import_fixes_without_editing_files`
  - `tests/diagnostics/import_fixes.rs::deep_super_is_flagged_and_fixed`
  - `tests/diagnostics/import_fixes.rs::triple_super_is_flagged_and_fixed`
- `RUSTC_BOOTSTRAP=1 cargo +stable run -- --workspace --all-targets --fail-on-warn` first found an inline `crate::config::DiagnosticCode` type in `scan.rs`, then passed with `No findings`.

### Retrospective

**What worked:**

- `src/fixes/imports.rs` moved to `src/fixes/imports/mod.rs`, then compiled before extraction.
- `imports/mod.rs` retains the edit contract and transaction helpers; `scan.rs` owns file walking and `UseVisitor`; `path.rs` owns import-path analysis and byte offsets.
- Existing sibling callers kept their `fixes::imports::{UseFix, ImportGroup, ValidatedFixSet}` paths while visibility narrowed to `pub(super)`.

**What deviated from the plan:**

- `apply.rs` was not created because the preflight required `snapshot_files`, `apply_fixes`, and `restore_files` to stay parent-owned for Phase 4.

**Surprises:**

- The checkout self-audit caught the one inline type path in `scan.rs`; `cargo check` allowed it.

**Implications for remaining phases:**

- Phase 5 can rely on `imports` retaining edit-application and rollback helpers.
- Phase 5 should treat `UseFix`, `ImportGroup`, and `ValidatedFixSet` as sibling-owned contract types, not runner-private data.
- Later splits should keep running the checkout self-audit after extraction because nested modules introduce import-style findings quickly.

### Phase 4 Review

- Phase 5 now treats `runner/apply.rs` as orchestration-only; `imports` transaction helpers stay in `imports`.
- Phase 5 now keeps `MendRunner::build_selection` parent-owned for the first extraction unless inventory proves a narrow `selection.rs` child.
- Phase 5 verification now includes existing conflict-filter tests and direct fix-notice unit tests.
- Phase 5 child modules must use parent-owned `RunPlan` / `FixScans` data through `pub(super)` fields or parent helpers, not `pub(crate)` widening.
- Phase 6 now starts after Phase 5 fix-notice CLI cases pass, so rendering consumes existing notice objects.

## Phase 5 - Fix Runner (Complete)

Target: `src/fixes/runner.rs`

Current responsibilities:

- Run planning.
- Read-only and build-report execution.
- Apply and rollback flow.
- Cross-diagnostic fix combination.
- Conflicting import-group filtering.
- Fix-notice construction.
- Test helpers.

Candidate extraction order:

1. Move `src/fixes/runner.rs` to `src/fixes/runner/mod.rs` and compile before extracting children.
2. Map `MendRunner` methods by responsibility before moving code.
3. Keep `RunPlan` and `FixScans` parent-owned in `runner/mod.rs` for the first extraction; if a child needs fields, use `pub(super)` fields or parent helpers instead of widening to `pub(crate)`.
4. Move planning-only state and helpers into a child module if they are not coupled to apply/rollback state.
5. Keep `MendRunner::combined_fixes` and `drop_conflicting_import_groups` together in `runner/mod.rs` unless inventory proves a small `combine.rs` child with parent-owned `FixScans`.
6. Move apply/rollback orchestration only after deciding whether runner ownership needs a private snapshot newtype; do not move `imports` transaction helpers out of `imports` during this phase.
7. Move fix-notice construction only after adding a small notice input type or parent helper; `notices.rs` should not require direct access to every scan struct.
8. Keep `MendRunner::build_selection` parent-owned for the first extraction unless inventory proves a small `selection.rs` child with a narrow input and output.
9. Keep `MendRunner` as the parent-facing type exported through `src/fixes/mod.rs`.

Initial candidate files:

- `src/fixes/runner/mod.rs`
- `src/fixes/runner/plan.rs`
- Optional after inventory: `src/fixes/runner/combine.rs`
- `src/fixes/runner/apply.rs`
- `src/fixes/runner/notices.rs`
- Optional after inventory: `src/fixes/runner/selection.rs`

Verification:

- Path audit for `crate::fixes::runner` and `crate::fixes::imports`.
- Fix-runner tests.
- Diagnostics tests that run `--fix`, dry-run, rollback, and fix-notice cases.
- `combined_fixes_drops_shorten_import_when_prefer_module_import_overlaps`.
- `combined_fixes_keeps_adjacent_shorten_import_and_prefer_module_import`.
- `no_conflicts_pass_through_unchanged`.
- `same_bare_name_different_paths_drops_all_tagged`.
- `same_bare_name_same_full_path_kept`.
- `conflict_isolated_per_file`.
- `untagged_fixes_always_pass_through_even_with_conflicts`.
- `field_visibility_scan_emits_import_fix_notice`.
- `empty_field_visibility_scan_emits_noop_import_fix_notice`.
- Record final module-qualified test paths or nonzero `nextest` matches after moving tests into `plan`, `apply`, or `notices` children.

Ownership inventory:

| Item | Kind | Current path | Visibility | Current callers | Impl blocks | Co-moving helpers | Test home | Proposed home | Retained path |
|---|---|---|---|---|---|---|---|---|---|
| `MendRunner` | Struct | `fixes::runner::MendRunner` | `pub(crate)` re-exported by `fixes` | `src/main.rs` through `crate::fixes::MendRunner` | `new`, `run`, `execute`, `build_selection`, child method impls | Parent-owned selection/config fields | CLI and diagnostics tests | `fixes::runner::mod` | `crate::fixes::MendRunner` |
| `RunPlan` | Struct | `fixes::runner::RunPlan` | Private | `plan`, `execute`, `apply` | `fix_scans` | Scan fields and run metadata | Covered through runner behavior tests | `fixes::runner::mod` with private fields | Private parent type |
| `FixScans` | Struct | `fixes::runner::FixScans` | Private | `apply`, `combined_fixes`, `build_fix_notice` | `import_fix_notice_count` | Optional scan references | Combine and notice unit tests | `fixes::runner::mod`; count helper in `notices.rs` | Private parent type |
| `MendRunner::plan` | Method | `fixes::runner::MendRunner::plan` | Private | `run` | Method body only | Diagnostic enablement checks and scan calls | Diagnostics fix-mode tests | `fixes::runner::plan` | Private child method visible to parent |
| `MendRunner::execute` | Method | `fixes::runner::MendRunner::execute` | Private | `run` | Method body only | Read-only and dry-run result assembly | CLI and diagnostics tests | `fixes::runner::mod` | Private parent method |
| `MendRunner::apply` | Method | `fixes::runner::MendRunner::apply` | Private | `execute` | Method body only | Snapshot, apply, validation, rollback orchestration | Rollback and apply diagnostics tests | `fixes::runner::apply` | Private child method visible to parent |
| `MendRunner::combined_fixes` | Method | `fixes::runner::MendRunner::combined_fixes` | Private | `apply`, unit tests | Method body only | Cross-diagnostic fix ordering | Combine unit tests | `fixes::runner::combine` | Private child method visible to runner siblings |
| `drop_conflicting_import_groups` | Function | `fixes::runner::drop_conflicting_import_groups` | Private | `combined_fixes`, unit tests | None | Import-group conflict map | Combine unit tests | `fixes::runner::combine` | Private child helper |
| `MendRunner::build_fix_notice` | Method | `fixes::runner::MendRunner::build_fix_notice` | Private | `execute`, `apply`, unit tests | Method body only | `FixScans::import_fix_notice_count` | Notice unit tests | `fixes::runner::notices` | Private child method visible to runner siblings |
| `MendRunner::build_selection` | Method | `fixes::runner::MendRunner::build_selection` | Private | `plan`, `apply` | Method body only | Compiler run plus style-finding scans | CLI and diagnostics tests | `fixes::runner::mod` for first extraction | Private parent method |

### Retrospective

**What worked:**

- `src/fixes/runner.rs` moved to `src/fixes/runner/mod.rs`, then compiled before extraction.
- `plan.rs`, `apply.rs`, `combine.rs`, and `notices.rs` now own the expected method groups while `MendRunner`, `RunPlan`, `FixScans`, dispatch, and build-selection remain parent-owned.
- The required nextest filter ran 19 tests with final paths under `fixes::runner::combine`, `fixes::runner::notices`, `import_fixes`, and `pub_use_fixes`.

**What deviated from the plan:**

- `RunPlan` and `FixScans` fields stayed private, not `pub(super)`, because the types are private and descendant modules can access parent-private fields.
- No private snapshot newtype was introduced; `apply.rs` still calls the existing `imports` transaction helpers.

**Surprises:**

- The checkout self-audit flagged `pub(super)` fields on private parent-owned types as dead visibility even though `cargo check` accepted them.
- The final path audit had no source hits for `crate::fixes::runner`, `super::runner`, `pub(in ...)`, wildcard imports, or `super::super`; matches were plan-doc text only.

**Implications for remaining phases:**

- Phase 6 should avoid field visibility on private parent-owned render coordination structs; rely on parent-private access or helper methods first.
- Phase 6 can start from a passing fix-notice gate: `fix_reports_*`, `fix_pub_use_*`, rollback, dry-run, and `fix_all_converges_in_one_invocation` all passed with `CARGO_MEND_SKIP_NETWORK_TESTS` unset.

Verification record:

- `cargo check --workspace --all-targets` passed.
- `cargo +nightly fmt --all` passed.
- `cargo nextest run -E '<Phase 5 filter>'` ran 19 tests and passed all 19. Final module-qualified test paths included:
  - `fixes::runner::combine::tests::combined_fixes_drops_shorten_import_when_prefer_module_import_overlaps`
  - `fixes::runner::combine::tests::combined_fixes_keeps_adjacent_shorten_import_and_prefer_module_import`
  - `fixes::runner::combine::tests::conflict_isolated_per_file`
  - `fixes::runner::combine::tests::no_conflicts_pass_through_unchanged`
  - `fixes::runner::combine::tests::same_bare_name_different_paths_drops_all_tagged`
  - `fixes::runner::combine::tests::same_bare_name_same_full_path_kept`
  - `fixes::runner::combine::tests::untagged_fixes_always_pass_through_even_with_conflicts`
  - `fixes::runner::notices::tests::empty_field_visibility_scan_emits_noop_import_fix_notice`
  - `fixes::runner::notices::tests::field_visibility_scan_emits_import_fix_notice`
  - `import_fixes::fix_reports_when_nothing_is_fixable`
  - `import_fixes::fix_reports_noop_notice_after_summary`
  - `import_fixes::fix_reports_applied_notice_after_summary`
  - `import_fixes::fix_rolls_back_on_failed_cargo_check`
  - `import_fixes::dry_run_reports_import_fixes_without_editing_files`
  - `pub_use_fixes::fix_pub_use_reports_when_nothing_is_fixable`
  - `pub_use_fixes::fix_pub_use_rewrites_pub_super_parent_facade_in_apply_mode`
  - `pub_use_fixes::fix_all_converges_in_one_invocation`
  - `pub_use_fixes::fix_pub_use_rolls_back_on_failed_cargo_check`
  - `pub_use_fixes::dry_run_reports_pub_use_fixes_without_editing_files`
- `RUSTC_BOOTSTRAP=1 cargo +stable run -- --workspace --all-targets --fail-on-warn` passed with "No findings."
- `git diff --check` passed.
- Path audit for `src/fixes/runner`, `src/fixes/imports`, and the plan doc found no source-code hits for `crate::fixes::runner`, `super::runner`, wildcard imports, `super::super`, or `pub(in ...)`.

### Phase 5 Review

- Phase 6 entry gate now records that Phase 5 already passed the fix-notice CLI cases; Phase 6 keeps them as regression coverage.
- Phase 6 path audit now includes reporting facade callers in addition to direct `reporting::render` paths.
- Phase 6 now keeps `render_human_report` orchestration parent-owned for the first extraction.
- Phase 6 now treats fix-notice rendering as cross-module regression coverage, not render-split ownership.
- Snapshot-newtype work is closed unless a new rollback defect appears.
- The whole-refactor after-each-phase gate now includes the warning-sensitive self-audit command.
- Phase 5 now records the concrete final targeted test paths.
- Phase 6 now requires a renderer ownership inventory before editing.

## Phase 6 - Reporting Render (Complete)

Target: `src/reporting/render.rs`

Current responsibilities:

- Color and output mode policy.
- Summary-row layout.
- Diagnostic detail rendering.
- Timing rendering.
- Fixability summaries.
- Row-width calculations.
- Rendering tests.

Candidate extraction order:

1. Record the renderer ownership inventory before editing.
2. Move `src/reporting/render.rs` to `src/reporting/render/mod.rs` and compile before extracting children.
3. Keep `ColorMode`, `OutputFormat`, `CompilerStats`, `render_human_report`, and `render_timing` reachable through `src/reporting/mod.rs`.
4. Treat `ColorMode` and `OutputFormat` as reporting policy types, not render-private color helpers; preserve `crate::reporting::{ColorMode, OutputFormat}` for current non-render callers.
5. Keep `render_human_report` orchestration in `render/mod.rs` for the first extraction; it sequences finding detail rows, error block rendering, and summary rendering.
6. Move summary row structs and width calculations together.
7. Move diagnostic-detail rendering into a child module if it only needs `Finding`, `Severity`, and `ColorMode`.
8. Keep raw color paint helpers in render-private color code; keep severity labels with diagnostic rendering, summary row formatting with summary rendering, and timing text with timing rendering.
9. Do not move `ExecutionNotice`, `FixNotice`, `PubUseNotice`, or notice text rendering into `render`; fix-notice tests are cross-module regression gates.
10. Split tests by output concern if the test module becomes easier to navigate after the production split.

Initial candidate files:

- `src/reporting/render/mod.rs`
- `src/reporting/render/summary.rs`
- `src/reporting/render/diagnostic.rs`
- `src/reporting/render/color.rs`
- `src/reporting/render/timing.rs`

Ownership inventory:

| Item | Kind | Current path | Visibility | Current callers | Impl blocks | Co-moving helpers | Test home | Proposed home | Retained path |
|---|---|---|---|---|---|---|---|---|---|
| `ColorMode` | Enum | `reporting::render::ColorMode` | `pub(crate)` re-exported by `reporting` | `main`, compiler build, fixes runner, config parsing tests | `is_enabled` | `paint`, color helpers | Render and main color tests | `reporting::render` parent | `crate::reporting::ColorMode` |
| `OutputFormat` | Enum | `reporting::render::OutputFormat` | `pub(crate)` re-exported by `reporting` | `main`, config CLI, fixes runner | None | Output-mode dispatch in callers | CLI and diagnostics tests | `reporting::render` parent | `crate::reporting::OutputFormat` |
| `CompilerStats` | Struct | `reporting::render::CompilerStats` | `pub(crate)` re-exported by `reporting` | `main`, render tests | None | Summary classification and rows | Render summary tests | `reporting::render` parent | `crate::reporting::CompilerStats` |
| `Findings` | Enum | `reporting::render::Findings` | Private | `render_human_report` | `classify` | `CompilerStats` classification | Render no-findings and compiler-only tests | `reporting::render` parent unless summary child owns all callers | Private |
| `render_human_report` | Function | `reporting::render::render_human_report` | `pub(crate)` re-exported by `reporting` | `main` | None | Finding loop, error block, summary line | Render human-report tests | `reporting::render` parent | `crate::reporting::render_human_report` |
| `render_timing` | Function | `reporting::render::render_timing` | `pub(crate)` re-exported by `reporting` | `main` | None | `paint` color helper | Timing rendering tests | `reporting::render::timing` with parent re-export | `crate::reporting::render_timing` |
| `render_finding` and diagnostic labels | Functions | `reporting::render::*` | Private | `render_human_report`, render tests | None | `severity_label`, `severity_marker`, `diagnostic_label`, `blue_bold` | Diagnostic rendering tests | `reporting::render::diagnostic` | Private child API |
| `SummaryRow` / `SummaryFixable` | Structs | `reporting::render::*` | Private | `summary_line`, `render_summary_rows` | None | Width and continuation-row calculations | Summary-row rendering tests | `reporting::render::summary` | Private child types |
| `summary_line`, `render_summary_rows`, `fixable_category_count`, `digit_count`, `errors_block` | Functions | `reporting::render::*` | Private | `render_human_report`, render tests | None | Summary and error block formatting | Summary and error rendering tests | `reporting::render::summary` | Private child API |
| `paint`, `dim`, `blue_bold` | Functions | `reporting::render::*` | Private | Diagnostic, summary, timing helpers | None | ANSI constants and color policy | Color-sensitive render tests | `reporting::render::color` | Private child API |

Verification:

- Entry gate status: Phase 5 passed the fix-notice CLI cases in `tests/diagnostics/import_fixes.rs` and `tests/diagnostics/pub_use_fixes.rs`; rerun them as Phase 6 regression coverage.
- Path audit for `crate::reporting::render`, `crate::reporting::{ColorMode, OutputFormat, CompilerStats, render_human_report, render_timing}`, and facade callers through `src/reporting/mod.rs`.
- Rendering tests.
- CLI smoke and diagnostics rendering tests that assert stdout, stderr, JSON cleanliness, human summaries, and fix notices.
- Record final module-qualified test paths or nonzero `nextest` matches after moving tests into `summary`, `diagnostic`, `color`, or `timing` children.

### Retrospective

**What worked:**

- `src/reporting/render.rs` moved to `src/reporting/render/mod.rs`, then compiled before extraction.
- `render/mod.rs` kept `ColorMode`, `OutputFormat`, `CompilerStats`, `Findings`, and `render_human_report`; child modules now own color helpers, diagnostic detail rendering, summary/error rows, and timing text.
- `render_human_report` kept the parent-owned orchestration role and the existing `crate::reporting::{ColorMode, OutputFormat, CompilerStats, render_human_report, render_timing}` paths stayed intact.

**What deviated from the plan:**

- `render_timing` moved to `timing.rs` with a parent re-export, while `render_human_report` stayed in the parent.
- Existing render unit tests moved together into `summary.rs` because they assert summary, error, and fixability output through `render_human_report`; no separate diagnostic or timing test modules were added.

**Surprises:**

- The checkout self-audit caught direct function imports from the new `color` module; the final code uses module-qualified `color::paint`, `color::dim`, and `color::blue_bold` calls.
- The render path audit found only facade calls through `reporting::render_human_report`, `reporting::render_timing`, and render unit tests; no source code imports `crate::reporting::render`.

**Implications for remaining phases:**

- No implementation phases remain; remaining work is final plan closeout and any completion-criteria cleanup from phase-review.
- Future render edits should keep `render_human_report` parent-owned unless a later behavior change creates a narrower owner.

Verification record:

- `cargo check --workspace --all-targets` passed.
- `cargo +nightly fmt --all` passed.
- `cargo nextest run -E '<Phase 6 filter>'` ran 15 tests and passed all 15. Final module-qualified test paths included:
  - `reporting::render::summary::tests::render_human_report_prints_no_findings_when_empty`
  - `reporting::render::summary::tests::render_human_report_shows_combined_summary_for_mend_and_compiler_findings`
  - `reporting::render::summary::tests::summary_lists_one_line_per_fix_flag_plus_fix_all_aggregate`
  - `reporting::render::summary::tests::errors_render_in_their_own_block_above_summary`
  - `rendering::fixture_renders_every_current_diagnostic`
  - `rendering::successive_json_runs_reuse_cached_findings_for_same_scope`
  - `json_success_emits_clean_stdout_and_no_stderr_noise`
  - `import_fixes::fix_reports_when_nothing_is_fixable`
  - `import_fixes::fix_reports_noop_notice_after_summary`
  - `import_fixes::fix_reports_applied_notice_after_summary`
  - `import_fixes::dry_run_reports_import_fixes_without_editing_files`
  - `pub_use_fixes::fix_pub_use_reports_when_nothing_is_fixable`
  - `pub_use_fixes::fix_pub_use_rewrites_pub_super_parent_facade_in_apply_mode`
  - `pub_use_fixes::fix_all_converges_in_one_invocation`
  - `pub_use_fixes::dry_run_reports_pub_use_fixes_without_editing_files`
- `cargo clippy --workspace --all-targets -- -D warnings` passed.
- `cargo nextest run` ran 333 tests and passed all 333.
- `RUSTC_BOOTSTRAP=1 cargo +stable run -- --workspace --all-targets --fail-on-warn` passed with "No findings."
- `git diff --check` passed.
- Path audit for `src/reporting/render`, `src/reporting/mod.rs`, `src/main.rs`, `src/config`, `src/fixes/runner`, and `src/compiler/build` found no direct `crate::reporting::render` imports and no new `pub(in ...)`, wildcard imports, or `super::super` source hits in the render split.

### Phase 6 Review

- Final verification is already recorded by the Phase 6 gate results; rerun final gates only if implementation files change after this review.
- Added this Phase 6 review closeout block because no implementation phases remain.
- Added a consolidated retained-path audit under Final Closeout so completion criteria do not rely on scattered retrospectives.
- Added a child-module style audit covering all files created by the split; `src/reporting/render/summary.rs` is over 500 total lines only because its tests moved with summary behavior.
- Interpreted the no-new-allow criterion by semantic diff: the Phase 6 `summary.rs` allow is the existing render test-module allow moved unchanged from `src/reporting/render.rs`.
- Added `git diff --check` to the final gate checklist.

## Whole-Refactor Verification

Run these after each phase that compiles:

```bash
cargo check --workspace --all-targets
cargo +nightly fmt --all
cargo nextest run <phase-specific filter>
RUSTC_BOOTSTRAP=1 cargo +stable run -- --workspace --all-targets --fail-on-warn
```

Run these after the final phase:

```bash
cargo clippy --workspace --all-targets -- -D warnings
cargo nextest run
RUSTC_BOOTSTRAP=1 cargo +stable run -- --workspace --all-targets --fail-on-warn
git diff --check
```

## Completion Criteria

- Each target file is either split according to the style rule or documented as no longer meeting two split criteria after ownership moves.
- Existing parent-module exports still satisfy current callers.
- Each phase closeout compares every ownership row's `Current path` and `Retained path`, listing the retained re-export or the explicit caller update.
- Each targeted `nextest` filter records a nonzero match/run result; newly added tests must have concrete names before extraction starts.
- No new `#[allow(...)]` attributes or lint-table `allow` entries are introduced.
- All final verification commands pass.

## Final Closeout

Final verification is satisfied as of Phase 6:

- `cargo clippy --workspace --all-targets -- -D warnings` passed.
- `cargo nextest run` ran 333 tests and passed all 333.
- `RUSTC_BOOTSTRAP=1 cargo +stable run -- --workspace --all-targets --fail-on-warn` passed with "No findings."
- `git diff --check` passed.

Retained-path audit:

| Phase | Retained paths |
|---|---|
| Phase 1 - Compiler Build | `src/compiler/mod.rs` still re-exports `BuildOutputMode`, `CARGO_MANIFEST_FILE`, `SelectionResult`, `run_cargo_fix`, and `run_selection` from `compiler::build`. |
| Phase 2 - Compiler Persistence | `src/compiler/persistence/mod.rs` still exposes `load_report`, `StoredFinding`, `StoredPubUseFixFact`, `StoredReport`, `UseSite`, `FindingsSink`, `CacheBuildKind`, `cache_filename_for`, and `prepare_findings_dir` at `crate::compiler::persistence::*` for compiler descendants. |
| Phase 3 - Config CLI | `src/config/mod.rs` still re-exports `BuildInfoMode`, `CargoCheckCli`, `FixExecution`, `TargetSelection`, `WarningPolicy`, `WorkspaceSelection`, and `parse` from `config::cli`. |
| Phase 4 - Import Fixes | `src/fixes/imports/mod.rs` keeps `ImportScan`, `ImportGroup`, `UseFix`, `ValidatedFixSet`, `scan_selection`, `snapshot_files`, `apply_fixes`, and `restore_files` reachable to sibling fix modules through the private `imports` module. |
| Phase 5 - Fix Runner | `src/fixes/mod.rs` still re-exports `MendRunner`; `RunPlan` and `FixScans` remain private runner coordination types. |
| Phase 6 - Reporting Render | `src/reporting/mod.rs` still re-exports `ColorMode`, `CompilerStats`, `OutputFormat`, `render_human_report`, and `render_timing`; `render/mod.rs` re-exports `timing::render_timing`. |

Child-module style audit:

- All split child modules compile and pass the checkout self-audit.
- New child module line counts are recorded from actual files. `src/compiler/persistence/load.rs` has 541 lines and `src/reporting/render/summary.rs` has 515 lines, but both are test-heavy behavior modules. `load.rs` owns report loading plus its negative/round-trip tests; `summary.rs` owns summary/error/fixability rows plus moved render unit tests.
- Neither large child currently meets two split criteria after excluding test weight: production code remains one behavior group in each file, and each module's tests exercise that same behavior.

Allow audit:

- Phase 6 did not add a new semantic allow. The `#[allow(clippy::expect_used, reason = "tests should panic on unexpected values")]` block in `src/reporting/render/summary.rs` is the existing render test-module allow moved from `src/reporting/render.rs`.
- Current split-tree allows are test-module boilerplate only and use the pre-authorized reason text from the style guide; no production `#[allow(...)]` attributes or lint-table allow entries were added.

## Team Review State

- Cycle 1 accepted refinements: path/visibility contracts, behavior-group ownership rows, file-to-directory migration, test placement, stronger phase gates, fixes edit preflight, build stderr/progress boundary, persistence suppression order, CLI raw parser privacy, and render color dependency direction.
- Cycle 2 accepted refinements: blocking fixes edit preflight, runner coordination-type ownership, persistence write-side and field access contracts, `ManifestCli` parser/result decision, fix combination ordering, exact verification matrix, skip-sensitive test handling, persistence negative cases, and reporting policy type placement.
- Cycle 3 accepted refinements: deeper-child visibility translation, deterministic `ManifestCli` and runner coordination rules, passing shared-contract preflight tests, final self-audit through the checkout binary, nonzero targeted-test proof, and retained-path closeout checks.
- Proposed user decisions: none.
