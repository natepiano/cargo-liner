# port


## Source

2026-10-28 10:41 EDT

you need to create a new unit to handle all cargo port enhancements it should be called port (session name - i think your approach is to call it port-unit for its id) it should be the one to build up context about cargo-port and not hand work to other units such as handler that is port work

## Unit charter

- **Owns:** `crates/cargo-port/` and the port plans in `docs/cargo-port/`. Every cargo-port enhancement and fix comes to this unit; no other unit gets cargo-port work.
- **Shared crates:** when a cargo-port change needs a `tui_pane` change, this unit makes it and claims the file with `cargo-berth --before` the current holder, as earlier units did. It does not hand the change to the tile or handler unit.
- **Also owns:** `crates/cargo-mend/`. A mend bug is always fixed, as a new phase at once.
- **Shipped work:** the first port plan (2026-10-28 to 2026-11-11) is in the as-built docs: `docs/cargo-port/as-built/`, `docs/cargo-mend/as-built/` and `docs/tui_pane/as-built/`.
- **Not this unit's:** cargo-port findings that the style production holds (for example proposal P265, CPU bands, quota mode and pagination states) stay with `cargo-liner-style`.
- **New work:** each request the showrunner sends becomes a new phase in this plan, with its own Work Order, after Phase 1.

## Delegation Context

- **Project:** cargo-port — a TUI for inspecting and managing Rust projects, built on `tui_pane`
- **Project started:** 2026-10-28T14:44:58.152+00:00
- **Stack:** Rust 2024; ratatui, crossterm, tokio, reqwest, notify; `tui_pane` (workspace member)
- **Layout:** `crates/cargo-port/src/` (app source), `crates/cargo-port/tests/`, `crates/cargo-port/themes/`, `docs/cargo-port/as-built/` (as-built docs)
- **Key files:** `crates/cargo-port/src/main.rs` — entry; `crates/cargo-port/src/tui/app/construct.rs` — App construction; `crates/cargo-port/src/watcher/` — file watcher; `crates/cargo-port/src/tui/app/dismiss.rs` — dismiss; `crates/cargo-port/README.md` — crate readme
- **Test lanes:** cargo-port: `crates/cargo-port/tests/`; cargo-mend: `crates/cargo-mend/tests/` and the `scan.rs` test modules
- **Build:** `bash ~/.claude/scripts/direct/verify.sh check <cargo-port|cargo-mend>`
- **Test:** `bash ~/.claude/scripts/direct/verify.sh test <cargo-port|cargo-mend>`
- **Lint:** `bash ~/.claude/scripts/direct/verify.sh lint <cargo-port|cargo-mend>`
- **Invariants:** no test takes a second; a deleted project stays in the list, struck through, until the user dismisses it or rescans (user, 2026-11-05).

## Phases

### Phase 1 — mend --fix keeps a glob caller's name when it removes a re-export  · status: done

**Depends on:** none.

#### As-built

When the `pub_use_outside_subtree` fix removes a re-export, `glob_callers::plan_file_callers` also handles glob imports of the re-exporting module, beside the explicit-caller redirects of `facade_redirect::redirect_callers_in_file`.
- A scope that uses the name gets `use crate::<target>;` after the glob item. A scope that never names the item gets no edit.
- An uncertain caller declines, and the re-export stays: a macro or nested module in the scope, competing globs, a block or unresolved module alias, a local binding of the same name, or a shadowed name.
- Globs of an outside crate or of another crate module are skipped.
- `FacadeRedirect::target_for` returns `CallerRedirect::{Redirect { path, side }, KeepFacade}`.

**Files:**
- `crates/cargo-mend/src/fixes/subtree_reexport/glob_callers.rs` — glob-caller planning (`GlobCallerRewriter`, `GlobSource`, `HiddenNames`) and the table test `glob_callers_keep_removed_reexport_bindings_and_compile`
- `crates/cargo-mend/src/fixes/subtree_reexport/scan.rs` — `build_resolved_round` merges the glob-caller plans and adds declined redirects to `round.unmatched`
- `crates/cargo-mend/src/fixes/facade_redirect/redirect.rs` — `CallerRedirect`
- `crates/cargo-mend/src/fixes/facade_redirect/module_aliases.rs` — `contains_module`
- `crates/cargo-mend/CHANGELOG.md` — entry

**Binds later work:** `GlobCallerRewriter::glob_source` classifies a glob whose first segment is not a crate module as `GlobSource::External`; `ModuleAliases` skips block imports and `extern crate self` aliases.

**Gotchas:** a declined redirect keeps the re-export for the whole crate, so decline only for a glob that can supply the name. A file with a macro call and an owner glob keeps the re-export instead of an import.

**Ruled out:** glob expansion — mend has none; the hana report described the defect wrongly.

### Phase 2 — mend --fix keeps a name imported through an `extern crate self` alias  · status: done

**Depends on:** Phase 1.

#### As-built

When the `pub_use_outside_subtree` fix removes a re-export, a caller that reaches it through `extern crate self as <alias>;` keeps the name. Root, block and absolute (`::<alias>`) forms resolve to the crate root, for glob and explicit callers.
- `SelfCrateAlias::classify` is the one recognizer of the alias. `ModuleAliases`, `CallerRewriter` and `GlobCallerRewriter` use it.
- `ModuleAliases::module_target` returns `ModuleAliasResolution::{Bound, Unbound, Declared, Blocked}`. Two different Bound targets, or a Blocked source, give Blocked. A glob cycle gives Unbound.
- `target()` maps Unbound to the root alias, and Declared or Blocked to None. With None, the caller keeps the re-export.

**Files:**
- `crates/cargo-mend/src/fixes/facade_redirect/self_crate_alias.rs` — `SelfCrateAlias`
- `crates/cargo-mend/src/fixes/facade_redirect/module_aliases.rs` — root aliases, `CrateUse`, `ModuleAliasResolution`
- `crates/cargo-mend/src/fixes/facade_redirect/caller_rewrite.rs` — explicit callers through local, block and absolute aliases
- `crates/cargo-mend/src/fixes/subtree_reexport/glob_callers.rs` — block self aliases; `ReexportExpectation`; the 20-row alias table in `glob_callers_keep_removed_reexport_bindings_and_compile`
- `crates/cargo-mend/src/fixes/inline_path_qualified_type/scan.rs` — ten inline-path cases that compile the fixed sources with rustc, in place of a CLI `--fix` run
- `crates/cargo-mend/CHANGELOG.md` — entry

**Gotchas:** the glob-source loop in `module_target` never returns early on one source's result, because another glob can supply the binding. mend does not track module visibility, so a private module of the alias name in a glob source reads as Declared.

**Ruled out:** a visibility model for glob sources — when that module is visible, rustc already reports an ambiguity.

### Phase 3 — cargo-mend tests run under 1 s  · status: done

#### As-built

The cargo-mend suite has fewer tests at 1 s or more than main, but the 1 s goal is open: Mac 89 against main 130 (load 20.74), natedev 32 against main 102. The cost that remains is in tests that need the CLI and Cargo boundary.
- Each test thread uses a private Cargo home, so tests do not wait on Cargo cache locks.
- Syntax-only fix cases run in process through the production `MendRunner::combined_function_plan`, then one real `cargo check`. Inline-path cases run in `scan.rs`.
- Compiler fixture cases run real compiler callbacks in process through `compiler::analyze_fixture_json`. Each output is named `{package_index}-{target_index}-{mode}.rmeta`, so same-name library and binary targets keep separate reports.
- The diagnostics tests compile into the `cargo-mend` bin test through `src/diagnostic_tests.rs`. There is no separate `diagnostics` test target.

**Files:**
- `crates/cargo-mend/src/compiler/driver/fixture.rs` — in-process compiler fixture analysis (cfg(test))
- `crates/cargo-mend/src/fixes/fixture_analysis.rs`, `crates/cargo-mend/src/fixes/runner/fixture.rs` — in-process fix fixtures (cfg(test))
- `crates/cargo-mend/src/diagnostic_tests.rs` — registers each `tests/diagnostics/*.rs` file once

**Binds later work:** the open 1 s goal; the fixture report naming and exactly-once test registration; the lib and bin caller-key and display-filter defects found during this work.

**Gotchas:**
- A slow count means nothing without its load, thread count, tree and binary. The same tree gave Mac 103 and natedev 218 at load 115-119.
- A concurrent relink replaces `target/debug/cargo-mend`. Measure a single sample with a copied executable.

**Ruled out:** a cargo-mend job queue in the tests and a nextest thread group (no gain, removed); a shared writable fixture target (rollback and cache-repair tests need their own report state).

### Phase 4 — mend --fix redirects `extern crate self` alias callers on macOS  · status: done

#### As-built

- `SourcePathKey` is the one owner of source file identity: the canonical path, else the canonical ancestor plus the suffix, else the normalized path.
- Source caches, seeded Cargo target maps, module maps and import planners store and look up this one key form. A symlinked source root (macOS `/var -> /private/var`) thus resolves the module of an `extern crate self` alias caller.
- `ModuleMap` keeps the declared path beside each key. Relative `include!` resolves from the declared directory.
- `ParsedSources` caches keys per source snapshot, and a cache hit makes no filesystem call. Diagnostic paths stay relative to the analysis root when that root is a link.

**Files:**
- `crates/cargo-mend/src/rust_syntax/source_path_key.rs` — the `SourcePathKey` identity type
- `crates/cargo-mend/src/rust_syntax/parsed_sources.rs`, `module_map.rs` — key storage, declared paths, key cache
- `crates/cargo-mend/src/fixes/imports/`, `subtree_reexport/`, `prefer_module_import/`, `imports_at_top/`, `inline_path_qualified_type/`, `pub_use_fixes/` — callers that use the key
- `crates/cargo-mend/CHANGELOG.md` — entry for alias callers on macOS

**Binds later work:** canonical resolution runs on the first lookup of each file, so test-time measurements must come from a tree with this merge. `function_length_plan.rs` builds fix sets with `ValidatedFixSet::try_from((plan.fixes, sources))` at three calls; edits there start from this form.

**Gotchas:**
- Use the declared path for relative resolution and the key for identity. A mix breaks `include!` resolution or diagnostic paths.
- `realpath` costs about 10x a lexical normalize (28.6 ms vs 2.7 ms for 10,000 calls). Only the key cache hides this cost, and no test counts filesystem calls on a cache hit.

**Ruled out:** a lookup-only repair in `module_imports.rs`, because stored keys stay lexical; a test that counts resolution calls, because it is a new check outside the scope.

### Phase 5 — finder match letters are readable in every theme  · status: done

#### As-built

- Matched finder letters keep the row ink when it has a ratio of at least 4.5:1 on `finder_match_bg`. Else they use black, else white. `MatchInkSelection` (`Preserve`, `Black`, `White`) and `match_ink` in `tui/finder/dispatch.rs` make this choice.
- Every theme keeps its match background. Only the ink of the matched letters changes.
- `tui_pane::contrast_ratio(foreground, background) -> Option<f64>` gives the WCAG ratio with the palette decoder of `blend_color`. It gives `None` for `Color::Reset`.
- `READABLE_TEXT_CONTRAST_FLOOR = 4.5` in cargo-port `tui/constants.rs` is the one floor. Production code and both contrast tests use it.

**Files:**
- `crates/cargo-port/src/tui/finder/dispatch.rs` — match ink and `finder_matches_meet_contrast_in_each_registered_theme`
- `crates/cargo-port/src/tui/constants.rs` — `READABLE_TEXT_CONTRAST_FLOOR`
- `crates/tui_pane/src/theme/blend.rs` — `contrast_ratio`
- `crates/cargo-port/src/tui/panes/package/readability_tests.rs` — uses `tui_pane::contrast_ratio`
- `crates/cargo-port/CHANGELOG.md`, `crates/tui_pane/CHANGELOG.md` — entries

**Ruled out:** a cargo-port copy of the ANSI palette decoder.

### Phase 6 — the Worktrees title keeps its count on every worktree page  · status: done

#### As-built

- A section on the first content row draws its title from `outer.x + 1` with `outer width - 2` cells, the same start column and width as a divider title. `push_section_overlays` in `tui/panes/git/render.rs` sets this area. The count drops only when a divider at that width also drops it.
- `section_title_with_count` keeps its shortening order: `Worktrees (N of M)`, then `Worktrees (M)`, then `Worktrees`, then a cut word with an ellipsis.
- `assert_worktree_title` in `tui/render/readability_tests.rs` requires `Worktrees (13)` on each worktree page in both `ColorMode`s. On a divider row it checks both side junctions; on the first row it checks the rule above the title.

**Files:**
- `crates/cargo-port/src/tui/panes/git/render.rs` — first-row title area
- `crates/cargo-port/src/tui/render/readability_tests.rs` — `assert_worktree_title`
- `crates/cargo-port/CHANGELOG.md` — entry

### Phase 7 — an idle cargo-port reads almost nothing from disk  · status: done

#### As-built

- No timed walk of project or target trees runs. A file event records its changed path on a `DiskUsageTracker` (`record_change`) in `scan/disk_measurements.rs`, and `refresh` measures only the recorded paths.
- A path-less `Flag::Rescan` event calls `request_rescan` on every owned tracker. This is the one full walk.
- `DiskUsageMeasurements` shares one tracker across nested projects. `DiskUpdateInputs` in `watcher/refresh.rs` records every requested deletion before it groups requests by tracker, then publishes every project entry that the tracker returns.
- `record_project_disk_change` in `watcher/events.rs` takes a dispatch reference and a `ProjectDiskChange`. `schedule_disk_refresh` in `watcher/refresh.rs` is the explicit refresh path.
- Hard links resolve through an identity-to-path index. Disk update traces carry `visited_entries` and `identity_entries_examined`.
- `DiskSizeAggregation` in `scan/disk_usage.rs` is the one size sum for both collectors.

**Files:**
- `crates/cargo-port/src/scan/disk_measurements.rs` — `DiskUsageMeasurements`, `DiskUsageTracker`, the identity index and the tracker tests
- `crates/cargo-port/src/scan/disk_usage.rs` — `DiskSizeAggregation`
- `crates/cargo-port/src/watcher/refresh.rs` — `DiskUpdateInputs`, `schedule_disk_refresh`
- `crates/cargo-port/src/watcher/events.rs` — `ProjectDiskChange`, `record_project_disk_change`
- `crates/cargo-port/src/watcher/disk_burst_tests.rs` — burst, grouped deletion and idle regression tests
- `crates/cargo-port/CHANGELOG.md` — entry

**Binds later work:** the idle-read follow-up keeps `grouped_deletion_checks_keep_root_and_child_changes`, `shared_disk_changes_publish_every_project`, `pathless_rescan_updates_every_owned_disk_tracker` and the idle regression test green, and uses the trace counters for per-update cost.

**Gotchas:** no timed walk repairs a missed event update, so each event path must be exact by itself.

**Ruled out:** a timed fallback walk.

### Phase 8 — an idle cargo-port reads from disk only for changes it shows  · status: done

#### As-built

- Git ignore rules set event visibility. An event in an ignored path, a target path or a Git lock file starts no Git refresh. `GitEventVisibilityCache` keeps the rules for the life of the watcher. It checks each rule file's size and time before use, and drops the rules of a replaced directory. A change to a shared `info/exclude` refreshes each linked checkout once.
- Each shown event queues one Git refresh (`GitEventRefresh`, `GitRefreshRequest`). There is no immediate probe.
- Disk totals update from the changed files and their hard-link aliases, with no new walk.
- Git commands run without the shared metadata store lock. A test hook in `project/git/command.rs` checks the lock at each Git command start.
- Traces carry a per-command Git read measure (`DiskReadBytes`) and a process disk-read observation.
- Idle reads fell about 70% in three paired natedev traces. The remaining reads are not yet linked to shown events.

**Files:**
- `crates/cargo-port/src/project/git/event_visibility.rs` — the ignore-rule cache and event visibility
- `crates/cargo-port/src/project/git/measured_output.rs` — Git output with the per-command read measure
- `crates/cargo-port/src/project/git/command.rs` — Git command start and the lock test hook
- `crates/cargo-port/src/process_observation/disk_reads.rs` — process disk-read observation
- `crates/cargo-port/src/scan/disk_totals.rs` — disk totals from changed files and hard-link aliases
- `crates/cargo-port/src/watcher/events.rs`, `refresh.rs`, `runtime.rs` — event routing and refresh demand
- `crates/cargo-port/CHANGELOG.md` — entry

**Binds later work:** the low-load idle-read measurement builds on `event_visibility.rs`, `measured_output.rs` and `process_observation/disk_reads.rs`, and uses `bd55fde2d` as its fixed baseline.

**Gotchas:**
- On natedev at high load, swap-ins (`pswpin` about equal to `pgmajfault`) make up many idle reads. They are not app file reads.
- Trace logging changes Git output capture in `measured_output::run`.
- A trace run can hang and leave the traced app alive in its own process group.

**Ruled out:** closing the idle-read gate on high-load measurements.

### Phase 9 — cargo-mend CLI-boundary tests run under 1 s  · status: todo

**Depends on:** Phase 3. Phase 10 merges before this phase's final measurements, because both edit `tests/support/` and `CHANGELOG.md`.

#### Work Order

**Goal:** every cargo-mend test takes less than 1 s in the full suite on natedev and on the Mac. No assertion becomes weaker.

**Source:** Phase 3 reached its 3-round repair cap with this goal open (finding F004, routed here). Phase 3 round 3 final suites, 2026-11-12:
- Mac 20261112T164244-3431635-f6bd: 1,027 pass, 89 tests at 1 s or more, slowest 2.377 s, load 20.74, 6 of 12 threads. Main at ad8977d87 on the Mac: 130 at 1 s or more, slowest 2.353 s, load 15.81.
- natedev 20261112T164244-3431634-648f: 1,029 pass, 32 at 1 s or more, slowest 2.169 s, load 23.17, 16 of 32 threads. Round 2 had 3 at load 40.1; the rise is not explained. Main at ad8977d87 on natedev: 102 at 1 s or more, slowest 2.384 s, load 40.27.
- The slowest Mac tests: `pub_use_outside_subtree_fixes` multi-pass re-export fixes, `rendering::fresh_units_with_another_builds_reports_are_examined_on_the_first_run`, several `pub_use_fixes` tests, `fix_rollback::failed_later_pass_restores_the_edits_earlier_passes_left_on_disk`, `suspicious_pub_fixes` signature boundaries. Their assertions need the CLI and Cargo boundary.

**Constraints from prior phases:**
- Phase 3 tries, in order (attempts log `attempts-p3-mend-test-time`): own job queue (removed), private Cargo home per thread (kept), nextest thread group (removed), less driver-cache output, warm driver copies, debug 0 and incremental 0 fixtures, one rustfmt call, 38 syntax-only cases moved in process through `MendRunner::combined_function_plan`, 15 inline cases in `scan.rs`, compiler fixtures on real callbacks (`src/compiler/driver/fixture.rs`), smaller batches. Do not repeat them.
- Phase 3 checkpoint is `cba42a650`. The attempts log is `~/.claude/projects/-home-natepiano-rust-cargo-liner/memory/attempts-p3-mend-test-time.md`; it names every run id. The round 2 tree (3 slow on natedev) is the uncommitted state recorded in that log's row 9; rebuild it from the run records under `/tmp/claude/unit/386d8d90-e9fc-4c2c-bc14-c3db0779daf3/` if a diff is needed.
- The 89 and 32 counts are earlier observations. The same Phase 3 tree at load 115-119 gave Mac 103 (run `20261112T172442-2561348-f986`, slowest 2.799 s) and natedev 218 (run `20261112T172442-2561193-823f`, slowest 5.707 s). Tie every count to its tree and binary.
- `fixes/runner/apply.rs:81` keeps the validation plan, and `fixes/runner/plan.rs:69` uses it with no second Cargo check. `ToolchainProbes` caches compiler queries within one run. Do not assume duplicate checks without a trace.
- `main.rs:143` `run()` opens the running executable as the compiler wrapper (`compiler/build/execute.rs:144`). In a test process that executable is the test harness, so an in-process CLI entry needs an explicit driver executable.
- The rollback and cache-repair fixtures have no dependencies, and cache-repair tests prepare missing or incompatible reports on purpose. Keep writable targets and mend reports per test.
- The diagnostics tests compile into the `cargo-mend` bin test through `src/diagnostic_tests.rs`. Keep each diagnostic test registered exactly once there.
- `compiler::analyze_fixture_json` (`src/compiler/driver/fixture.rs`) names each compiler output `{package_index}-{target_index}-{mode}.rmeta`. Keep one report per package, target and mode, and keep the same-name test `a_lib_and_bin_target_pair_advertises_and_applies_one_rewrite` green when shared targets or a CLI entry change.
- The function_length formatter groups all proposed sources into one rustfmt call per run, with a per-file fallback when the grouped call fails. Read `fixes/runner/function_length/formatter.rs` for the exact grouping.
- Not every slow test is a CLI test: `inline_path_fixes::synthesized_import_keeps_only_mixed_cfg_attr_gate` (1.018 s, Mac run `20261112T164244-3431635-f6bd`) uses the in-process rewrite helper and four Cargo checks. Attribute every slow test.
- Record the actual nextest thread count and the cache state (warm or cold driver cache, target dir) with each timing. Hold both constant within each alternating pair.
- An Astra review of this goal ran at the Phase 3 close: `/tmp/claude/astra/20261112T225040Z-351564/review.md`. Read it first and follow its ranking.
- Phase 10 (checkpoint `00d23f5c3`) Mac run: 1,034 of 1,034 pass at load 16.22, 49 tests at 1 s or more, all on the Phase 3 final slow list. Do not claim a gain that Phase 10 already shipped.
- Phase 10 keys callers by `DeclarationIdentity` (source file and definition path, `compiler/persistence/schema.rs`) and keeps the caller union for one physical declaration. Suppression and the visibility reconciler read `CallerEvidence::{Recorded, NoneRecorded}`. Keep both.
- The saved report is at `FINDINGS_SCHEMA_VERSION` 37 (`compiler/constants.rs:95`), and `UseSite` carries `declaration: DeclarationIdentity`. A warm-cache fixture must write this format, or the automatic rebuild changes its timing and cache assertions.
- `compiler::analyze_fixture_json` and `compiled_report` skip `DisplayFilter::apply` (`selection/display_filter.rs:112`). An in-process CLI entry must still run it. `DisplayFilter` parses sources only when target facts are incomplete.

**Spec:**
1. **Baseline.** Measure `00d23f5c3` (the Phase 10 checkpoint) on both machines. Use `cba42a650` only for the natedev rise in step 2. Record per-test times, the suite time, the thread count, Cargo jobs, compiler version, cache state and other load.
2. **Natedev rise.** Find why natedev went from 3 to 32 slow tests between Phase 3 rounds 2 and 3. Compare matching tests on the two trees under matched conditions. Load average alone does not explain it.
3. **Trace.** Trace the 10 slowest Mac tests and the slow syntax cases. Split each into startup, Cargo work, compiler queries, compilation, formatting and wait. Count a child's time once. Measure acceptance without the tracer.
4. **Classify.** Give each slow test its cost pattern, with helper calls and fixture loops. `pub_use_outside_subtree_fixes::package_filtered_reexport_used_through_raw_include_stays_manual` runs two independent fixtures. Split independent cases into separate tests. Keep dependent sequences, such as rollback and repeated-run cache tests, in one test.
5. **Repair the common cost,** in this order. Pick the first repair from the trace.
   1. Remove Cargo calls that the trace shows are duplicates. Record each call's purpose, source state, targets, features and cache result. Keep validation after edits, feature checks and cache recovery.
   2. Remove fixture work that no assertion needs.
   3. Reuse dependency artifacts only where the trace shows repeated dependency builds. Use a read-only seed, never a shared writable target.
   4. Add an in-process CLI entry only after a measurement shows the startup saving. Pass an explicit driver executable. Keep subprocess tests for process behavior.
6. **Keep coverage.** Do not delete, skip or weaken a test. Report each changed assertion with an `Assertion-Change:` line.
7. **Changelog.** Add an entry when cargo-mend's own run time changes.

**Files:**
- `crates/cargo-mend/src/`
- `crates/cargo-mend/tests/`
- `crates/cargo-mend/CHANGELOG.md`

**Workers:** 1 writer.

**State:** none.

**Acceptance gate:** `verify.sh test cargo-mend` passes on natedev and on the Mac with every test under 1 s. These tests keep their assertions: the five `target_identity_*` tests in `tests/diagnostics/target_identity.rs`, `same_line_facades_in_separate_namespaces_keep_macro_callers`, `compiler_mounts_filter_without_reading_sources`, and `wrong_schema_or_fingerprint_reports_are_rejected`. Each timing has its load average and the merged revision. Each claimed gain has 3 alternating paired runs under matched conditions, with process counts, per-test times and the suite time. `verify.sh lint cargo-mend` is clean. No `Change-Review:` line: test time is not visible in normal use.

### Phase 10 — cargo-mend keeps a library and a binary apart when they share a source folder  · status: done

#### As-built

- Callers are keyed by `DeclarationIdentity` (`compiler/persistence/schema.rs`): the declaration's source file as a `SourcePathKey` and its definition path. `DeclarationIdentity::new(source_file: &SourcePathKey, def_path: String)`. Callers of one physical declaration are unioned across compilations. A library item and a binary item with the same definition path in separate files keep separate caller sets.
- `UseSite` carries `declaration: DeclarationIdentity`. `FINDINGS_SCHEMA_VERSION` is 37 (`compiler/constants.rs`); a report in an older schema is rejected and rebuilt.
- Caller lookup returns `CallerEvidence::{Recorded, NoneRecorded}`. Suppression in `caller_aware.rs` and `caller_repair(declaration, evidence, requirement)` in `visibility_constraint.rs` read it; `CallerRequirement::{Naming, Reaching}` names what the caller must do. `DeclarationCallers` is defined before `UseSiteIndex` in `sink.rs`.
- Same-line facades key on `(parent_path, line, child_item_name)` with a set of child paths, so facades in separate namespaces keep their own macro callers.
- `DisplayFilter` (`selection/display_filter.rs`) selects findings by target source membership, not by source folder. `--lib`, named binaries and named examples in one folder each keep their own findings. Its source set is a `OnceCell<BTreeSet<SourcePathKey>>`, parsed only when the compiler target facts are incomplete.
- `rust_syntax/parsed_sources.rs` has a `#[cfg(test)]` thread-local source read counter, `source_read_attempts()`.
- The cargo-mend changelog has a fix entry for each defect.

**Files:**
- `crates/cargo-mend/src/compiler/persistence/caller_aware.rs` — caller suppression with `CallerEvidence` and the facade key
- `crates/cargo-mend/src/compiler/persistence/schema.rs` — `DeclarationIdentity` and `UseSite.declaration`
- `crates/cargo-mend/src/compiler/persistence/sink.rs` — `DeclarationCallers` and `UseSiteIndex`
- `crates/cargo-mend/src/compiler/persistence/visibility_constraint.rs` — `caller_repair` with `CallerEvidence` and `CallerRequirement`
- `crates/cargo-mend/src/compiler/visibility/use_sites.rs` — use-site producer with declaration identity
- `crates/cargo-mend/src/selection/display_filter.rs` — source-membership selection with a lazy parse
- `crates/cargo-mend/src/rust_syntax/parsed_sources.rs` — test-only source read counter
- `crates/cargo-mend/tests/diagnostics/target_identity.rs` — caller identity and CLI target-selection tests
- `crates/cargo-mend/tests/diagnostics/facade_subjects.rs` — same-line facade namespace test

**Binds later work:** cargo-mend test-time work keeps `DeclarationIdentity`, the caller union, `CallerEvidence`, schema 37 and the `DisplayFilter::apply` stage in any in-process CLI entry. The five `target_identity_*` tests, `same_line_facades_in_separate_namespaces_keep_macro_callers`, `compiler_mounts_filter_without_reading_sources` and `wrong_schema_or_fingerprint_reports_are_rejected` keep their assertions.

**Gotchas:** `compiler::analyze_fixture_json` and `compiled_report` skip `DisplayFilter::apply`, so target-selection behavior needs CLI tests. `caller_aware.rs` skips caller suppression for `OverbroadPubCrate` and `ForbiddenPubInCrate`.

### Phase 11 — cargo-port has one default binding for project row expand and collapse  · status: done

#### As-built

- `ExpansionAction::key_bindings()` in tui_pane takes no argument. It binds `ExpandRow` to plain Right and `CollapseRow` to plain Left.
- `ResolvedKeymap::defaults()` builds `project_list` from `ProjectListAction::key_bindings()`, the same source as the framework keymap. cargo-handler `globals.rs` uses the same call.
- A new keymap file binds `expand_row = "right"` and `collapse_row = "left"`. An existing keymap file keeps its bindings and its bytes.
- Doc comments in `tui/app/mod.rs` no longer name the removed `handle_normal_key`.
- Tests: `project_list_defaults_match_framework_bindings` and the default-text test in `resolved_keymap.rs`; `existing_shift_arrow_keymap_keeps_its_bindings_and_file` in `load.rs`.

**Files:**
- `crates/tui_pane/src/keymap/expansion_action.rs` — `ExpansionAction::key_bindings`
- `crates/cargo-port/src/tui/keymap/resolved_keymap.rs` — `ResolvedKeymap::defaults`, comparison and default-text tests
- `crates/cargo-port/src/tui/keymap/load.rs` — the existing-file test
- `crates/cargo-handler/src/globals.rs` — the third caller
- `crates/cargo-port/CHANGELOG.md`, `crates/tui_pane/CHANGELOG.md` — entries; the tui_pane entry is marked Breaking

**Gotchas:** `ResolvedKeymap.project_list` has no dispatch reader. It sets only the default file text, Vim conflict checks and load results.

### Phase 12 — an idle cargo-port reads from disk only for changes it shows, measured at low load  · status: todo

**Depends on:** Phase 8 (its idle-read repairs) and Phase 11 (both edit `crates/cargo-port/CHANGELOG.md`).

#### Work Order

**Goal:** an idle cargo-port reads almost nothing from disk, and each remaining read has a file change that the app shows as its cause.

**Source:** the showrunner's landing call for Phase 8, 2026-11-13. Phase 8 landed with finding F001 open: the remaining idle reads are not all tied to a shown change. This phase starts with that finding.
- Three idle pairs on natedev, Phase 7 binary against the Phase 8 binary: 496,861,184 → 149,938,176 bytes; 813,076,480 → 252,506,112 bytes; the third Phase 8 run read 47.7 MB. Shown events explain 30.3%, 22.8% and 21.3% of the Phase 8 reads.
- Bounded trace run 20261113T134151-2254388-a1ec: 71,749,632 bytes over 120 s at load 40-48. 47,218,688 bytes during syscalls, 10,878,976 between syscalls, 13,651,968 in Git child processes. 1,882 of 1,908 major-fault samples hit heap or anonymous memory.
- natedev had 16 GB of swap in use, and its swap-in count was close to its major-fault count. Swap-in of the app's own memory is not an app read.

**Constraints from prior phases:**
- Phase 8 added Git event visibility (`crates/cargo-port/src/project/git/event_visibility.rs`), measured Git output (`project/git/measured_output.rs`), disk read observation (`process_observation/disk_reads.rs`) and `scan/disk_totals.rs`. Keep their tests green.
- Phase 7: `DiskUsageTracker` in `scan/disk_measurements.rs` measures only changed paths; `request_rescan` is the one full walk.
- A trace tool must stop at its time limit even when every traced thread is in a blocking syscall. Give each trace run a hard limit from outside the tool too.
- Phase 8's trace tool was a session-local script that no longer exists. Keep the trace tool in the repository or in a path this Work Order's run records, so a later run can repeat it.
- The traced app runs in its own process group. On normal exit, failure and SIGTERM, the tool stops every process group it started: the app, its Git children and the recorder. One Phase 8 trace hung and left an orphaned app.
- `measured_output::run` (`project/git/measured_output.rs`) captures Git output differently when trace logging turns on read measurement. `DiskReadBytes::Unavailable` means the measure is missing, not zero.

**Spec:**
1. **Measure at low load.** Run each idle measurement when the natedev 1-minute load average is under 8. Record the load and the `pswpin` and `pgmajfault` counts from `/proc/vmstat` before and after each run.
2. **Swap apart from app reads.** Host counters cannot name the process that swapped in. Exclude bytes as swap-in only with per-process evidence, such as the app's own major-fault count or fault samples on its anonymous mappings. Report bytes with no attribution as unexplained; do not drop them.
3. **Two run kinds.** Attribution runs turn on trace logging. Acceptance runs use normal logging, so the measure does not change Git capture. Count an unavailable measure as missing evidence.
4. **Fixed baseline and workload.** The baseline is the Phase 8 checkpoint `bd55fde2d`. Keep a copy of each executable. Both binaries watch the same roots with the same configuration. Start each measured interval after the startup scans and deferred startup work finish.
5. **Cause.** Attribute each remaining read to its source: a file syscall with its path, a Git child process with its command and trigger event, or a page fault with its mapping.
6. **Fix only shown defects.** Remove each read that no change the app shows needs. For each removed read, add a test that fails while an event that the app does not show starts that read, and add a cargo-port changelog entry. If the low-load measurements show no such read, the phase closes on the measurements alone, with no code change.

**Files:**
- `crates/cargo-port/src/` — the watcher, Git and disk code that the trace names
- `crates/cargo-port/CHANGELOG.md` — entry for the removed idle reads

**Workers:** 1 writer.

**State:** none.

**Acceptance gate:** a 10-minute idle acceptance run on natedev at load under 8 reads almost nothing apart from swap-in that per-process evidence proves. Each remaining read has a file change that the app shows as its cause, and unexplained bytes are listed. Compare with the `bd55fde2d` binary in 3 alternating pairs at similar load and on the same workload. When the phase changes code: each new test fails before its repair, `verify.sh test cargo-port` passes on natedev and the Mac with every test under 1 s, and lint is clean.

### Phase 13 — cargo-port finishes its startup disk usage step  · status: done

**Depends on:** Phase 11 (both edit `crates/cargo-port/CHANGELOG.md`). Runs before Phase 12, in its own worktree.

#### Work Order

**Goal:** at startup, the Disk usage row completes for every project. It does not show "Disk usage failed".

**Source:** the showrunner, 2026-11-13. On natedev the installed cargo-port from main `13182d0dc` (Phase 8) showed "Disk usage failed" in its startup panel. `include_dirs` were `/etc`, `/tmp`, `~/.claude`, `~/rust` and `~/worktrees`. Load was about 198. The director traced the failure condition:
1. `sweep_startup_timeouts` in `tui/app/async_tasks/startup_phase/tracker.rs:397` marks the Disk usage row failed when the row stays visible past `STARTUP_ROW_TIMEOUT` (2 minutes, `tui/constants.rs:67`) without completing.
2. The row completes when `startup.disk.seen` holds every expected path. `tui/app/async_tasks/disk_handlers.rs:114` and `:126-133` insert into `seen`. `maybe_complete_startup_disk` (`tracker.rs:502`) logs `seen` and `expected` on completion.
3. No perf log from that startup exists, so the cause is not proven. Two causes are possible: the startup walk took longer than 2 minutes at that load, or some expected path never reaches `seen`. Phase 8 changed disk totals to update from changed files and hard-link aliases (`scan/disk_totals.rs`).

**Spec:**
1. **Cause.** Reproduce on natedev with the same `include_dirs`, in a terminal you start yourself, with the perf log on. Record `seen`, `expected`, the paths that never reach `seen`, the walk time and the load. Compare with the `596c2c75a` binary (before Phase 8) once. Post the cause on the board as soon as you know it.
2. **Fix.** When an expected path never reaches `seen`, make each startup disk result insert its paths. When the walk is slow, find the slow part and remove the waste. Do not hide the defect with a longer timeout only.
3. **Test.** Add a test that fails before the fix: a startup disk pass whose expected paths all reach `seen`, for the input that the cause names. It runs under 1 s and does not depend on the timing of the real disk.
4. **Changelog.** Add a cargo-port fix entry.

**Files:**
- `crates/cargo-port/src/` — the startup disk path that the cause names
- `crates/cargo-port/CHANGELOG.md` — entry

**Workers:** 1 writer.

**State:** none.

**Acceptance gate:** the test fails before the repair and passes after. `verify.sh test cargo-port` passes on natedev and the Mac, every test under 1 s. Lint is clean. A natedev startup with the same `include_dirs` completes the Disk usage row. The checkpoint carries a `Change-Review:` line: start cargo-port on natedev, and the Disk usage row completes with no failure.

### Phase 13b — cargo-port applies startup results fast under high load  · status: done

**Depends on:** Phase 13 (both edit the startup disk path and `crates/cargo-port/CHANGELOG.md`). Runs next, before Phase 12, in its own worktree.

#### Work Order

**Goal:** at startup under high load, the Disk usage row completes well under the 2 minute limit. Each background result applies to the app in little time.

**Source:** the showrunner, 2026-11-13, add-on ruling on the Phase 13 proof. Paired natedev starts at the same load (about 53 to 145) gave these results:
1. Before Phase 8 (`596c2c75a`): Disk usage done at 120.8 s.
2. Phase 13 fix: done at 115.6 s. The cargo-liner walk took 10.6 s.
3. In both builds the main loop applies at most `MAX_MSGS_PER_FRAME` (50, `tui/app/async_tasks/constants.rs:28`) results per frame in `tui/app/async_tasks/poll.rs`. One frame took 18.5 s to apply 50 results. The apply time per run sums to 111-132 s.
4. In 100 s the perf log held 2461 `watcher_git_event_probes` and 901 `watcher_project_event` lines (`watcher/events.rs`). Many came from files under `/tmp` and `~/.claude` that are not project files, for example Claude session `.jsonl` files.
5. At load 128 the Disk usage row timed out in both builds.

**Spec:**
1. **Slow apply.** Find which result kinds make one frame apply 50 results in 18.5 s. Use the perf log and add perf fields where the log cannot tell. Make each apply cheap; remove the waste, not the work the screen needs.
2. **Watcher events.** Find why watcher events from `/tmp` and `~/.claude` that are not project files reach cargo-port's queue. Drop events outside the watched project roots at the source, in the watcher, before they cost a Git probe or a queue message.
3. **Test.** Add tests that fail before each fix and pass after. They run under 1 s and do not depend on real disk timing or load.
4. **Changelog.** Add a cargo-port fix entry.

**Files:**
- `crates/cargo-port/src/tui/app/async_tasks/` — the background apply path that the cause names
- `crates/cargo-port/src/watcher/` — event filtering at the source
- `crates/cargo-port/CHANGELOG.md` — entry

**Workers:** 1 writer.

**State:** none.

**Acceptance gate:**
1. The tests fail before the repair and pass after.
2. `verify.sh test cargo-port` passes on natedev and the Mac, and every test runs under 1 s. Lint is clean.
3. Paired natedev starts with the same `include_dirs` and at the same load: the Phase 13 build, then this build. The Disk usage row in this build completes well under the 120 s limit.
4. The checkpoint carries a `Change-Review:` line: start cargo-port on natedev under load, and the Disk usage row completes with no failure.
