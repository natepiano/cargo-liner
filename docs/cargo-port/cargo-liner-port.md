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

#### Work Order

**Goal:** when `mend --fix` removes a re-export, a glob import that used it keeps the name. The rewrite compiles, with no rollback.

**Source:** showrunner, 2026-11-11, from hana (cables) through builds. The report read as a glob expansion, but mend has no glob expansion. The case (hana_conduit):
1. `rig_plugin/constants.rs` has `pub(super) use crate::rig::BOOT_BASE_RADIUS_SCALE;`.
2. `rig_plugin/look.rs` has `use super::constants::*;` and uses `BOOT_BASE_RADIUS_SCALE`.
3. The `pub_use_outside_subtree` fix removes the re-export and redirects explicit callers. The glob caller stays unchanged, so rustc reports E0425 and mend rolls back. cables is not blocked.

**Decision:** showrunner, 2026-11-11: a mend bug is always fixed. Director, 2026-11-11: add an explicit import to a glob caller that uses the removed name; keep the re-export when mend cannot prove the caller's use.

**Constraints from prior phases:**
- `docs/cargo-mend/as-built/import-fixes.md`: import rewrites keep cfg attributes, leading `::`, glob-provided names and child-module access through `super`; the scan tests compile the result with rustc and check that a second pass changes nothing.
- No test takes a second: the tests run `scan_file`, `apply_fixes` and rustc, not the full `--fix` path.

**Spec:**
1. **Glob callers.** When the fix removes a re-export, find each glob import that targets the re-exporting module. If the scope uses the name, add an explicit import of the new path. Skip a glob that cannot supply the name: an outside crate, or another crate module.
2. **Uncertain callers.** Keep the re-export when mend cannot prove the use: a macro or nested module in the scope, competing globs, an unresolved module alias, or a shadowed name. A scope that never names the item needs no edit.
3. **Test.** A table test runs `scan_file`, `apply_fixes` and rustc on the hana shape and on the uncertain cases. rustc accepts each result, and a second pass changes nothing.
4. **Changelog.** Add the cargo-mend entry.

**Files:**
- `crates/cargo-mend/src/` — the re-export removal fix and its tests
- `crates/cargo-mend/CHANGELOG.md`

**Workers:** 1 writer.

**State:** none.

**Acceptance gate:** `verify.sh test cargo-mend` and `verify.sh lint cargo-mend` clean; no test takes a second. The fixed binary runs `mend --fix` on the hana_conduit `rig` case (or a copy of it) with no rollback. The checkpoint carries a `Change-Review:` line.

