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

### Phase 3 — cargo-mend tests run under 1 s  · status: todo

**Depends on:** Phase 2, Phase 4.

#### Work Order

**Goal:** every cargo-mend test takes less than 1 s in the full suite on natedev and on the Mac. No assertion becomes weaker.

**Source:** the Phase 2 full suite on 2026-11-11 (one test thread) passed 1,022 tests, but 37 tests that Phase 2 did not change took 1 s or more. The five slowest:
- `narrow_pub_crate::fix_compiler_does_not_remove_reexport_used_only_by_cfg_test_code` — 3.418 s
- `rendering::fresh_units_with_another_builds_reports_are_examined_on_the_first_run` — 2.299 s
- `prefer_module_import::compiler_fix_validation_restores_cfg_incomplete_edit` — 2.236 s
- `pub_use_fixes::renamed_facade_shares_dry_run_and_apply_with_import_cleanup` — 2.108 s
- `pub_use_outside_subtree_fixes::file_test_module_reaching_the_name_through_a_glob_keeps_a_cfg_test_use` — 1.969 s

**Decision:** showrunner, 2026-11-11: next, as Phase 3. A multi-second test is a defect, not a load flake.

**Constraints from prior phases:**
- The 2026-11-09 test-time repairs are in place. Do not repeat them; find the cost they did not remove:
  1. Tests share retained driver copies, and repeated compiler-fix checks are gone.
  2. rustfmt is resolved once per run.
  3. JSON reports are reused, and compiler work for re-export fixtures is reduced.
  4. The progress thread has no shutdown delay.
  5. The shared driver cache holds a shared retention lock for each run and a short, separate publication lock.
- After those repairs, a full suite passed 1,020 tests with none at 1 s or more (slowest 0.923 s). The regression since then is not yet explained.
- A concurrent relink replaces `target/debug/cargo-mend`. Measure a single sample with a copied executable.
- `inline_path_qualified_type/scan.rs` already holds the ten inline-path cases that compile with rustc in place of a CLI run.
- Phase 4 (checkpoint 90a8065cf) adds `SourcePathKey` (`src/rust_syntax/source_path_key.rs`), the one owner of source file identity. Merge it before any edit to `function_length_plan.rs`, and keep its three `ValidatedFixSet::try_from((plan.fixes, sources))` calls. Take every timing on a tree that holds that merge, because the first lookup of each file now resolves its canonical path.
- `ParsedSources` owns the identity keys of one source snapshot, and the snapshot ends before source writes. Keep shared snapshot keys, the declared include directories in `ModuleMap`, relative diagnostic paths, and a fresh snapshot after each write.

**Spec:**
1. **Measure.** Run the full suite. Record the load average (`uptime`) with each timing. List every test at 1 s or more.
2. **Separate load from cost.** Run the slow tests alone, with the load average recorded. A test that is slow only under load still needs a repair: find the shared resource it waits on.
3. **Find the common cause.** Group the slow tests by the work they share (compiler fix validation, cargo or rustc runs, driver cache, cfg(test) checks). Repair each cause where it lives, in cargo-mend code or its test support.
4. **Keep coverage.** Do not delete, skip or weaken a test. Report each changed assertion with an `Assertion-Change:` line.
5. **Changelog.** Add an entry when cargo-mend's own run time changes.

**Files:**
- `crates/cargo-mend/src/` — the cause of the cost, and test support
- `crates/cargo-mend/tests/` — test support
- `crates/cargo-mend/CHANGELOG.md`

**Workers:** 1 writer.

**State:** none.

**Acceptance gate:** `verify.sh test cargo-mend` passes on natedev and on the Mac with every test under 1 s. Each timing has its load average and the merged revision. Each claimed gain has 3 alternating paired runs at similar load. `verify.sh lint cargo-mend` is clean. No `Change-Review:` line: test time is not visible in normal use.

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

### Phase 5 — finder match letters are readable in every theme  · status: todo

**Depends on:** none. Runs beside Phase 3 in its own worktree.

#### Work Order

**Goal:** in the finder, the letters that match the query meet a contrast ratio of at least 4.5:1 on `match_bg` in every registered theme.

**Source:** showrunner, 2026-11-11: in the High Contrast Dark theme, the finder draws matched letters White on the LightYellow `match_bg`. That is about 1.1:1, so the matched letters cannot be read. It happens in every finder cell, and it existed before C314.

**Decision:** keep `match_bg` in every theme, because it is the signal of a match. Change only the ink of the matched letters. A theme whose row ink already meets 4.5:1 on its `match_bg` keeps that ink, so its look does not change (director, 2026-11-11).

**Constraints from prior phases:**
- `highlighted_spans` in `crates/cargo-port/src/tui/finder/dispatch.rs` builds each finder cell. Its `highlight_style` is the row `fg` (`text_default()`) on `finder_match_bg()`.
- The registered themes are `crate::themes::builtins()` in `crates/cargo-port/src/themes/builtins.rs`. High Contrast Dark (`DEFAULT_HC_DARK_THEME`) sets `match_bg` to `Color::LightYellow` and `text.default` to `Color::White`.
- `FinderTheme` is in `tui_pane` and cargo-handler and cargo-tile also build it. Keep the fix in cargo-port when it can be. If a `tui_pane` change is necessary, claim the file with `cargo-berth --before` the holder (unit charter), and keep the other crates compiling.
- The test model is `dim_package_values_meet_contrast_in_each_registered_theme` in `crates/cargo-port/src/tui/panes/package/readability_tests.rs`. It installs each builtin theme, draws the pane and computes the WCAG ratio with `dim_value_contrast`. Use one ratio function; do not add a second copy.

**Spec:**
1. **Readable matches.** Each matched letter in a finder cell has an ink with a ratio of at least 4.5:1 on `match_bg`, in every builtin theme. Unmatched letters do not change.
2. **Test.** Add a test in the form of the model test: for each builtin theme, draw the finder with a query that matches letters in each column, read the drawn cells back, and check the ratio of each highlighted cell's `fg` on its `bg`. It fails on the current code in High Contrast Dark.
3. **Changelog.** Add a cargo-port entry: matched finder letters are readable in High Contrast Dark.

**Files:**
- `crates/cargo-port/src/tui/finder/dispatch.rs` — the matched-letter ink
- `crates/cargo-port/src/themes/builtins.rs` — only if the ink belongs in the theme
- `crates/cargo-port/src/tui/panes/package/readability_tests.rs` — the shared ratio function, if the new test reuses it
- `crates/cargo-port/src/tui/finder/` — the new test
- `crates/cargo-port/CHANGELOG.md`

**Workers:** 1 writer.

**State:** none.

**Acceptance gate:** `verify.sh test cargo-port` passes on natedev and on the Mac, every test under 1 s, and the new test fails before the repair. `verify.sh lint cargo-port` is clean. A live finder capture in High Contrast Dark shows the matched letters readable. `Change-Review: The finder's matched letters are readable in High Contrast Dark | On natedev, run cargo port in a terminal, choose the High Contrast Dark theme, open the finder and type a query | The matched letters on the yellow highlight are dark and readable`.

### Phase 6 — the Worktrees title keeps its count on every worktree page  · status: todo

**Depends on:** none. It can run beside Phase 3 and Phase 5; it shares only `crates/cargo-port/CHANGELOG.md` with Phase 5.

#### Work Order

**Goal:** each Git worktree page shows the section title with its count, `Worktrees (13)`, whenever the divider at the same pane width has room for it.

**Source:** showrunner, 2026-11-11 (rows C297 and C301): Phase 4x (21d3cb69c) changed `git_worktree_pages_keep_the_divider_title_and_drawn_page_number`. The test required `Worktrees (13)` on each worktree page. The new `assert_worktree_title` accepts a bare `Worktrees`, and the `├`/`┤` check sits in an `if let Some`, so a page can skip it.

**Decision:** the bare title is not a rule; it is a width defect (director, 2026-11-11). In `push_section_overlays` (`crates/cargo-port/src/tui/panes/git/render.rs`), a divider row gives its title `outer width - 2` cells from `outer.x + 1`. A section on the first content row draws a text label instead, from `inner.x + 1` with `inner width - 1` cells: one cell less, and one cell to the right of a divider title. In the 16-column Git pane of the test, `Worktrees (13)` is 14 cells, so the divider fits it and the first-row label drops the count. The first-row label takes the same start column and width as a divider title, so both rows fit the same title. The count drops only when the divider would also drop it.

**Constraints from prior phases:**
- `section_title_with_count` (`git/render.rs`) shortens `Worktrees (N of M)` to `Worktrees (M)`, then to `Worktrees`, then cuts the word with an ellipsis, by width. Keep that order.
- A section on the first content row sits under the top border or the description divider, so it draws its name as text and no second rule (Phase 4x). Keep that.
- The test sits in `crates/cargo-port/src/tui/render/readability_tests.rs` and runs both `ColorMode::Colored` and `ColorMode::NoColor`.

**Spec:**
1. **Label width.** The first-row section label starts at the column of a divider title and has the same width. No other row moves.
2. **Strict test.** Restore the strict rule in `git_worktree_pages_keep_the_divider_title_and_drawn_page_number`: each page that draws a worktree row also draws `Worktrees (13)`. On a divider row, the left and right cells are `├`/`┤` (`┣`/`┫` with no colour). On the first content row, the cell above the title is `┌` or `├` (`┏` or `┣`). Each page runs one of the two checks; no page skips both. Take the row from the found `Worktrees (13)` cell, not from a bare `Worktrees` search. The restored assert fails on the current code.
3. **Changelog.** Add a cargo-port entry: the Worktrees title keeps its count at the top of a Git page.

**Files:**
- `crates/cargo-port/src/tui/panes/git/render.rs` — the first-row label area
- `crates/cargo-port/src/tui/render/readability_tests.rs` — `assert_worktree_title` and the test loop
- `crates/cargo-port/CHANGELOG.md`

**Workers:** 1 writer.

**State:** none.

**Acceptance gate:** `verify.sh test cargo-port` passes on natedev and on the Mac, every test under 1 s, and the restored assert fails before the repair. `verify.sh lint cargo-port` is clean. A live capture of the Git pane at 80 columns shows `Worktrees (N)` at the top of a worktree page. `Change-Review: The Worktrees title keeps its count at the top of a Git page | On natedev, run cargo port at 80 columns, select a project with many worktrees, focus Git and page down to the worktree list | Each worktree page shows the Worktrees title with its count, also when the title is on the first row`.


### Phase 7 — an idle cargo-port reads almost nothing from disk  · status: todo

**Depends on:** none. Urgent (user via the showrunner, 2026-11-12); runs before Phase 3 round 3, Phase 5 and Phase 6 work.

#### Work Order

**Goal:** an idle cargo-port reads almost nothing from disk.

**Source:** user, 2026-11-12, via builds and the showrunner: "tell cargo liner it needs to do something about the cargo-port app." On natedev, an idle `cargo-port port` (up 23 h 50 min) read about 70 MB/s without a stop. IO pressure full avg10 was 25-44%, and the user's keyboard stuck.

**Spec:**
1. **Cause.** Trace an idle cargo-port: which paths it reads and which loop starts the reads (watcher, rescan, cache refresh). Suspects: target folders, `~/rust/repro`, `/tmp/claude` copies.
2. **Fix.** Remove the repeated reads at their cause.
3. **Test.** Add a test that fails while an idle app repeats the reads.
4. **Changelog.** Add a cargo-port entry.

**Files:**
- `crates/cargo-port/src/` — the watcher, scan or cache code that the trace names
- `crates/cargo-port/CHANGELOG.md` — entry for the idle disk reads

**Workers:** 1 writer.

**State:** none.

**Acceptance gate:** the test fails before the repair. `verify.sh test cargo-port` passes on natedev and the Mac, every test under 1 s. Lint clean. A live idle run on natedev reads almost nothing over 10 minutes.
