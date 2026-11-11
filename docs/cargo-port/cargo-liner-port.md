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

**Depends on:** Phase 2.

#### Work Order

**Goal:** every cargo-mend test takes less than 1 s in the full suite on natedev. No assertion becomes weaker.

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

**Acceptance gate:** `verify.sh test cargo-mend` on natedev passes with every test under 1 s, and each timing has its load average. `verify.sh lint cargo-mend` is clean. No `Change-Review:` line: test time is not visible in normal use.

