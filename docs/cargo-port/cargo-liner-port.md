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

### Phase 2 — mend --fix keeps a name imported through an `extern crate self` alias  · status: todo

**Depends on:** Phase 1.

#### Work Order

**Goal:** when `mend --fix` removes a re-export, a caller that reaches it through `extern crate self as <alias>;` keeps the name. The rewrite compiles, with no rollback.

**Source:** Phase 1 review finding F008. This valid crate loses its binding after the fix, and rustc reports E0425:

```rust
extern crate self as this;
use this::rig_plugin::constants::*;

pub(super) fn radius() -> f32 {
    LOCAL_SCALE * BOOT_BASE_RADIUS_SCALE
}
```

**Decision:** showrunner, 2026-11-11: current; add as Phase 2 before the shrink and the final gate.

**Constraints from prior phases:**
- `GlobCallerRewriter::glob_source` in `crates/cargo-mend/src/fixes/subtree_reexport/glob_callers.rs` returns `GlobSource::External` when the first path segment does not resolve to a crate module. `ModuleAliases` (`crates/cargo-mend/src/fixes/facade_redirect/module_aliases.rs`) skips `extern crate self` aliases, so the alias reads as an outside crate.
- A declined redirect goes to `round.unmatched` and keeps the re-export for the whole crate. Decline only when the glob can supply the name.
- The table test `glob_callers_keep_removed_reexport_bindings_and_compile` in `glob_callers.rs` runs `scan_file`, `apply_fixes` and rustc, then checks that a second pass changes nothing. Add rows to it; do not add a new test harness.
- No test takes a second.

**Spec:**
1. **Alias resolution.** Resolve a path whose first segment is an `extern crate self as <alias>` binding to the crate root. Then classify the glob as for `crate::`.
2. **Explicit callers.** Check that an explicit `use <alias>::…::<name>;` caller of the removed re-export is redirected or keeps the re-export. Fix it in the same way if it loses the binding.
3. **Fallback.** When mend cannot resolve the alias, keep the re-export.
4. **Test.** Add rows to the table test for a glob caller and an explicit caller through the alias. rustc accepts each result, and a second pass changes nothing.
5. **Changelog.** Extend the Phase 1 cargo-mend entry.

**Files:**
- `crates/cargo-mend/src/fixes/` — the alias resolution and its tests
- `crates/cargo-mend/CHANGELOG.md`

**Workers:** 1 writer.

**State:** none.

**Acceptance gate:** `verify.sh test cargo-mend` and `verify.sh lint cargo-mend` clean; no test takes a second. The new rows fail without the repair. No `Change-Review:` line: the case is a rare crate shape that the user cannot check in normal use.

