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

### Phase 8 — an idle cargo-port reads from disk only for changes it shows  · status: todo

**Depends on:** Phase 7 (event-only disk updates). Runs in its own worktree `/home/natepiano/rust/cargo-liner-port-8`, branch `port-phase-8`, from main at `b1d063fae`.

#### Work Order

**Goal:** an idle cargo-port reads almost nothing from disk.

**Source:** Phase 7 live run, 2026-11-12. With event-only disk updates, a 10-minute idle run on natedev read 935,518,208 bytes at load 29-297. The logs show 195 Git refreshes and 189 disk updates, and no idle source scan. The reads are not fully attributed. The old binary read 2,311,979,008 bytes in 5 minutes.

**Constraints from prior phases:**
- Phase 7 (checkpoint 5696fd2cd) removed the timed full walks. `DiskUsageTracker` in `scan/disk_measurements.rs` keeps each changed path (`record_change`), and `refresh` measures only those paths. `request_rescan` is the one full walk, for a path-less `Flag::Rescan` event.
- `DiskUsageMeasurements` shares one tracker across nested projects. `DiskUpdateInputs` in `watcher/refresh.rs` groups requests by tracker and publishes every project entry that the tracker returns.
- `record_project_disk_change` in `watcher/events.rs` takes a dispatch reference and a `ProjectDiskChange`. `schedule_disk_refresh` in `watcher/refresh.rs` is the explicit refresh path.
- Disk update traces carry `visited_entries` and `identity_entries_examined`. Use them for the per-update cost in the attribution.
- Keep these tests green: `grouped_deletion_checks_keep_root_and_child_changes`, `shared_disk_changes_publish_every_project`, `pathless_rescan_updates_every_owned_disk_tracker`, and the idle regression test in `watcher/disk_burst_tests.rs`.

**Spec:**
1. **Cause.** Attribute each idle read to its trigger: the event path that starts each Git refresh and each disk update, and the bytes that each one reads.
2. **Fix.** Remove each read that no change the app shows needs. Examples: a Git refresh for an event in an ignored or target path, or a second refresh for one burst of events.
3. **Test.** Add a test that fails while an event that the app does not show starts a Git refresh or a disk read.
4. **Changelog.** Add a cargo-port entry.

**Files:**
- `crates/cargo-port/src/` — the watcher and Git refresh code that the trace names
- `crates/cargo-port/CHANGELOG.md` — entry for the idle Git and disk reads

**Workers:** 1 writer.

**State:** none.

**Acceptance gate:** the test fails before the repair. `verify.sh test cargo-port` passes on natedev and the Mac, every test under 1 s. Lint clean. A live idle run on natedev reads almost nothing over 10 minutes, and each remaining read has a file change that the app shows as its cause. Compare with the Phase 7 binary in 3 pairs at similar load.

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

**Spec:**
1. **Baseline.** Measure `cba42a650` on both machines. Record per-test times, the suite time, the thread count, Cargo jobs, compiler version, cache state and other load.
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

**Acceptance gate:** `verify.sh test cargo-mend` passes on natedev and on the Mac with every test under 1 s. Each timing has its load average and the merged revision. Each claimed gain has 3 alternating paired runs under matched conditions, with process counts, per-test times and the suite time. `verify.sh lint cargo-mend` is clean. No `Change-Review:` line: test time is not visible in normal use.

### Phase 10 — cargo-mend keeps a library and a binary apart when they share a source folder  · status: todo

**Depends on:** Phase 3.

#### Work Order

**Goal:** when a package has a library and a binary with the same name or in the same source folder, cargo-mend reports and fixes each target's findings correctly. A finding in one target does not hide or remove a finding in the other.

**Source:** an Astra review during Phase 3 repair round 4 (2026-11-12, `/tmp/claude/astra/20261112T220344Z-864733/review.md`) found two production defects. Phase 3 did not cause them, so they come here. Neither is reproduced yet.
1. `crates/cargo-mend/src/compiler/persistence/caller_aware.rs:34`: `CallerKey` is `(package_root, target_def_path)`. A library item and a binary item with the same definition path share one caller set. A binary caller can then hide a valid library finding. The visibility reconciler (`compiler/persistence/visibility_constraint.rs:947`) reads the same merged set.
2. `crates/cargo-mend/src/selection/display_filter.rs:189`: `target_directory` reduces a target to its source folder. `src/lib.rs` and `src/main.rs` both become `src`. With `--lib`, `src` is in both the allowed and the excluded folders, and the check near line 125 can exclude library findings. Named binaries or examples in one folder share one boundary too.

**Constraints:**
- `compiler/persistence/caller_aware.rs:123` skips caller suppression for `OverbroadPubCrate` and `ForbiddenPubInCrate`. Those diagnostics cannot show the caller defect.
- `compiler::analyze_fixture_json` does not apply `DisplayFilter`. The display defect needs CLI tests.
- `UseSite` (`compiler/visibility/use_sites.rs`) has no declaration source identity. The caller repair starts at that producer and reaches `persistence/sink.rs` and `persistence/schema.rs`.
- Keep `--all-targets` analysis, and keep target selection display-only, as `main.rs:243` states.
- Phase 9 also edits `crates/cargo-mend/tests/support/` and `crates/cargo-mend/CHANGELOG.md`. This phase merges before Phase 9's final measurements.

**Spec:**
1. **Reproduce first.** For each defect, write a failing test with a fixture project.
   1. Callers: a library and a binary with the same item path in separate files, with a caller-sensitive diagnostic.
   2. Display: CLI runs with `--lib` that show library findings, hide excluded binary findings, separate named targets in one folder, and handle shared sources.
   If a test passes on the unrepaired code, trace the branch it covers before you drop the defect. Report the trace.
2. **Callers.** Add a declaration identity type with the canonical source identity. Use it in the caller key. Keep the union of callers for one physical declaration across compilations.
3. **Caller result.** Replace `callers_for_package(...) -> Option<&ItemCallers>` with an enum such as `CallerEvidence::{Recorded, NoneRecorded}`. Use the same identity and result in suppression and in the visibility reconciler.
4. **Reports.** If the report schema changes, add a test that an old report is read or rebuilt correctly.
5. **Display filter.** Select findings by target ownership or source membership, not by folder.
6. **Changelog.** Add a fix entry for each defect that a user sees.

**Files:**
- `crates/cargo-mend/src/compiler/persistence/`
- `crates/cargo-mend/src/compiler/visibility/use_sites.rs`
- `crates/cargo-mend/src/compiler/constants.rs`
- `crates/cargo-mend/src/diagnostic_tests.rs`
- `crates/cargo-mend/src/selection/`
- `crates/cargo-mend/tests/`
- `crates/cargo-mend/CHANGELOG.md`

**Workers:** 1 writer.

**State:** the saved mend report, when the caller key changes its schema (`compiler/persistence/schema.rs`).

**Acceptance gate:** each new test fails before its repair and passes after. `verify.sh test cargo-mend` passes on natedev and on the Mac. No new or changed test takes 1 s or more, and no other test is newly at 1 s or more against the Phase 3 final slow lists (`phases/3/port-test-final-mac-slow-tests.json` and `port-test-final-natedev-slow-tests.json` in the unit session folder) at similar load. `verify.sh lint cargo-mend` is clean. No `Change-Review:` line: the tests show the result.
