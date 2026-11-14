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

### Phase 9 — cargo-mend CLI-boundary tests run under 1 s  · status: done

#### As-built

- cargo fix and its validation check run with the mend wrapper in passthrough (`RunWrapper::configure_passthrough`: driver on, config root removed) and share the run's `ToolchainProbes`. Cargo keeps one compiler identity, so the analysis after a fix does not rebuild units. A project with an unusable project compiler can run `--fix-all`.
- The function-length bound uses `body_bytes` (body braces included) in place of the full item range, so a long header no longer sends a small body to the formatter.
- `FixtureCompiler` resolves the production wrapper; `prepare_fresh_cache_units` no longer copies it by hand.
- `package_filtered_reexport_used_through_raw_include_stays_manual` is split in two: the qualified raw include is now `package_filtered_reexport_used_through_qualified_raw_include_stays_manual`. Both share `assert_raw_include_reexport_stays_manual`. No assertion changed.
- Root `Cargo.toml` sets `[profile.test.package.cargo-mend] opt-level = 1`. It changes only cargo-mend test builds. Mac pairs at matched threads gave fewer slow tests every time: 122 → 36 and 238 → 143.
- An in-process CLI entry was measured and rejected: no startup saving.
- Final runs on checkpoint `0a2e89d65`:
  1. natedev: 1,038 pass, 0 tests at 1 s or more (slowest 0.706 s) at load 55-60; 29 (slowest 2.206 s) at load 63, 10 of 32 threads.
  2. Mac: 1,036 pass, 97 tests at 1 s or more, slowest 1.914 s, load 16.36, 6 of 12 threads.
- The acceptance gate is not met. The natedev gate holds only at moderate load. A workspace suite at load 65-99 (tile-unit, 2026-11-13) had 108 cargo-mend tests at 1 s or more; the slowest were `rendering::fresh_units_without_reports_are_examined_on_the_first_run` (9.0 s) and `rendering::fresh_units_with_another_builds_reports_are_examined_on_the_first_run` (8.3 s). The Mac per-test gate is unmet. The showrunner ruled to record it and close (2026-11-13).
- Tests: `fix_all_uses_build_compiler_with_external_path_dependency_and_unusable_project_rustc` (`cli_smoke.rs`); a function-length test with two long headers and a test-only formatter call counter.

**Files:**
- `crates/cargo-mend/src/compiler/build/execute.rs`, `build/mod.rs`, `compiler/mod.rs` — passthrough wrapper and shared probes for cargo fix
- `crates/cargo-mend/src/fixes/runner/mend_runner.rs` — fix and validation runs
- `crates/cargo-mend/src/fixes/runner/function_length/function_collector.rs`, `function_length_plan.rs`, `formatter.rs` — body-byte bound and test-only counter
- `crates/cargo-mend/tests/cli_smoke.rs`, `tests/diagnostics/pub_use_outside_subtree_fixes.rs`, `tests/diagnostics/rendering.rs` — tests
- `Cargo.toml` — cargo-mend test profile
- `crates/cargo-mend/CHANGELOG.md` — two Fixed entries

**Gotchas:** each cargo-mend CLI test starts Cargo and rustc, so its time follows host load more than any code change. The attempts log is `attempts-p9-mend-test-time.md` in the project memory folder.

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

**Depends on:** Phase 8 (its idle-read repairs), Phase 11 (both edit `crates/cargo-port/CHANGELOG.md`), Phase 13b (both edit the watcher and the CHANGELOG), and Phase 13c (both edit cargo-port and its CHANGELOG).

#### Work Order

**Goal:** an idle cargo-port reads almost nothing from disk, and each remaining read has a file change that the app shows as its cause.

**Source:** the showrunner's landing call for Phase 8, 2026-11-13. Phase 8 landed with finding F001 open: the remaining idle reads are not all tied to a shown change. This phase starts with that finding.
- Three idle pairs on natedev, Phase 7 binary against the Phase 8 binary: 496,861,184 → 149,938,176 bytes; 813,076,480 → 252,506,112 bytes; the third Phase 8 run read 47.7 MB. Shown events explain 30.3%, 22.8% and 21.3% of the Phase 8 reads.
- Bounded trace run 20261113T134151-2254388-a1ec: 71,749,632 bytes over 120 s at load 40-48. 47,218,688 bytes during syscalls, 10,878,976 between syscalls, 13,651,968 in Git child processes. 1,882 of 1,908 major-fault samples hit heap or anonymous memory.
- natedev had 16 GB of swap in use, and its swap-in count was close to its major-fault count. Swap-in of the app's own memory is not an app read.

**Constraints from prior phases:**
- Phase 8 added Git event visibility (`crates/cargo-port/src/project/git/event_visibility.rs`), measured Git output (`project/git/measured_output.rs`), disk read observation (`process_observation/disk_reads.rs`) and `scan/disk_totals.rs`. Keep their tests green.
- Phase 7: `DiskUsageTracker` in `scan/disk_measurements.rs` measures only changed paths; `request_rescan` is the one full walk.
- Phase 13: a hard-link alias group in `DiskUsageIndex` is a sorted set; a new link updates only its own and the counted link's totals. Keep `hard_link_group_scan_reads_each_alias_a_few_times` green.
- Phase 13b drops Cargo home folder events other than the Cargo config file at the watcher (`RegisteredRoots::config_source_event_matters`). Events under `/tmp` and `~/.claude` stay, because they are user `include_dirs`. Idle reads against the `bd55fde2d` baseline then include that change; name its share apart from this phase's repairs.
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

#### As-built

- Cause: cargo-berth templates under `target/tmp/cargo-berth-templates` hard-link Git hooks 524 and 827 times. Phase 8's `DiskUsageIndex::replace_file` reapplied every alias of a group for each new link and took the smallest path per alias. The startup walks took 129 s and 135 s, and the Disk usage row timed out at 120 s.
- `DiskUsageIndex` keeps each alias group as a `BTreeSet`; the first path counts the group's bytes. A new link updates only its own, the counted link's and the next link's totals (`removal_contributors`). Alias records change only when the file metadata changes (`alias_record`).
- Proof on natedev, paired starts at the same load: Disk usage done at 115.6 s with no timeout, against 120.8 s for `596c2c75a` (before Phase 8). The cargo-liner walk fell from 129.3 s to 10.6 s.
- Test: `hard_link_group_scan_reads_each_alias_a_few_times` (64 aliases, at most 3 record reads per alias; 4096 before the fix).

**Files:**
- `crates/cargo-port/src/scan/disk_measurements.rs` — alias groups, `alias_record`, `removal_contributors`, the test
- `crates/cargo-port/CHANGELOG.md` — fix entry

**Gotchas:** the remaining startup time under high load is the main loop's background apply (one frame took 18.5 s for 50 results), older than Phase 8. Phase 13b owns it.

### Phase 13b — cargo-port applies startup results fast under high load  · status: done

#### As-built

- Cause: for each git, lint, CI, language or metadata result, `handle_bg_msg` built `project_row_presentation` (every row, every target expanded) before and after the apply. That was about 99% of the slow apply time; the apply itself was about 1%.
- `poll_background` applies a frame batch with one `RowPresentationBatch`: the first row message captures the before copy, and `finish_row_batch` compares once after the batch, only when no handler already raised `row_generation`. `handle_bg_msg` is now test-only and runs a batch of one.
- `RegisteredRoots::config_source_event_matters` drops Cargo home folder events other than the Cargo config file, such as `.global-cache-journal`. Events under `/tmp` and `~/.claude` stay, because they are user `include_dirs`.
- `poll_background_slow_msg` logs each background message that takes `SLOW_BG_MSG_MS` (10 ms) or more, with its kind (`BackgroundMsg` derives `strum::IntoStaticStr`).
- Proof on natedev, paired starts at similar load: Disk usage done at 79.5 s against 165.5 s (timeout) for the Phase 13 build. Slowest frame 15.9 s → 2.6 s; saturated frames 149 → 53.
- Tests: `row_message_burst_builds_row_presentation_at_most_twice` (20 row messages, at most 2 builds) and `cargo_home_routes_only_its_config_file`.

**Files:**
- `crates/cargo-port/src/tui/app/async_tasks/dispatch.rs` — `handle_batched_bg_msg`, `finish_row_batch`, `RowPresentationBatch`, `affects_row_presentation`
- `crates/cargo-port/src/tui/app/async_tasks/poll.rs`, `constants.rs` — batch loop, slow-message trace
- `crates/cargo-port/src/tui/app/mod.rs` — test-only `ROW_PRESENTATION_BUILDS` counter
- `crates/cargo-port/src/watcher/roots.rs`, `events.rs` — config source filter
- `crates/cargo-port/src/scan/background.rs` — `IntoStaticStr`
- `crates/cargo-port/src/tui/terminal/event_loop_tests.rs`, `src/watcher/watch_role_tests.rs` — tests
- `crates/cargo-port/CHANGELOG.md` — fix entry

**Gotchas:** one `RepoInfo` result can still take 2.6 s on the frame path, and the largest disk walks take 36 to 65 s. Phase 13c owns the frame time.

### Phase 13c — no cargo-port startup frame takes more than 100 ms  · status: todo

**Depends on:** Phase 13b on main (this phase changes its apply path). Runs before Phase 12: Phase 12 needs natedev load under 8, and this phase measures under high load.

#### Work Order

**Goal:** at startup under high load, no frame takes more than 100 ms. Heavy result work runs off the frame path, so the app never stalls.

**Source:** the showrunner, 2026-11-13, ruling on the Phase 13b proof: a 2.6 s frame is a stall the user sees. Phase 13b paired natedev starts (load 86 to 74) gave these results:
1. Slowest frame 2.6 s, 53 saturated frames.
2. Slow messages (`poll_background_slow_msg`, 10 ms or more): `RepoInfo` 11 messages, 13.1 s in total, at most 2.6 s; `LanguageStatsBatch` 77, 2.0 s; `CiRuns` 7, 0.66 s.
3. The first row message of each frame batch also carries the before copy of the row presentation (`RowPresentationBatch::Captured`), so its time includes one full presentation build.

**Constraints from prior phases:**
- Phase 3: cargo-port draws a frame only when something changed; `row_generation` drives the redraw. Keep that rule.
- Phase 13b: `poll_background` compares the row presentation once per frame batch (`finish_row_batch`). Keep `row_message_burst_builds_row_presentation_at_most_twice` green.
- `handle_repo_info` (`tui/app/async_tasks/repo_handlers.rs`) keeps `first_commit` and compares `last_fetched` before it invalidates the GitHub fetch cache. Keep that behavior.

**Spec:**
1. **Measure.** Use `poll_background_slow_msg` and frame timing to find each message kind and step over 100 ms. Add perf fields where the log cannot tell which step costs the time.
2. **Off the frame path.** Move the `RepoInfo` result work and any other heavy apply work off the frame path, to a background task or into bounded steps. The frame applies only cheap state changes.
3. **Row presentation.** If one presentation build costs more than the frame budget under load, make it cheaper or incremental. Keep the draw-only-on-change rule.
4. **Test.** Add tests that fail before each fix and pass after. They count work, not time, and run under 1 s.
5. **Changelog.** Add a cargo-port fix entry.

**Files:**
- `crates/cargo-port/src/tui/app/async_tasks/` — the apply path and `repo_handlers.rs`
- `crates/cargo-port/src/tui/app/mod.rs` — `project_row_presentation`, if the build cost is the cause
- `crates/cargo-port/CHANGELOG.md` — entry

**Workers:** 1 writer.

**State:** none.

**Acceptance gate:**
1. The tests fail before the repair and pass after.
2. `verify.sh test cargo-port` passes on natedev and the Mac, and every test runs under 1 s. Lint is clean.
3. Paired natedev starts with the same `include_dirs` and at the same load: the Phase 13b build, then this build. In this build no frame takes more than 100 ms, and Disk usage done is not later than in the Phase 13b build.
4. The checkpoint carries a `Change-Review:` line: start cargo-port on natedev under load, and the app answers keys at once while startup runs.
