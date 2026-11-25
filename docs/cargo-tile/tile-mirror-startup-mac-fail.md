# tile-mirror-startup-mac-fail

> **Status: IMPLEMENTATION PLAN — phased, direct-ready.** The native mirror startup test passes on every Mac run.

> **As-built disposition: amend** — `docs/cargo-tile/as-built/tile-fixes.md`

## Source

2026-11-25 06:09 PST (showrunner, defect for tile):

1. `cargo-tile::shim_modes tests::a_native_mirror_keeps_cargo_streams_without_shell_utilities` (`crates/cargo-tile/tests/shim_modes.rs`) failed once on the Mac. Run: merge test 1 of cargo-liner-style ticket 1524 (2deb0e67c, a cargo-port move only) on main cfe49779d. FAIL in 0.585 s; 1362 of 1363 passed.
2. The same test passed on the Mac in the four merges before it, with no cargo-tile change since.
3. Log: `/session/cargo-liner/claude-1000/-home-natepiano-rust-cargo-liner-docs-cargo-port/4dbdb78e-a83b-4248-b8f9-ff278dcbc60c/scratchpad/merge_2deb0e6_test.log`, line 2046. The log has no panic text.
4. Find the cause and fix the test or the code. Ticket 1524 merges in the same batch as the fix.

## Delegation Context

- **Project:** cargo-tile — a terminal grid of the cargo processes on the machine. Its capture shim stands in for `cargo` and mirrors cargo's output streams.
- **Stack:** Rust 2024; Linux (natedev) and macOS (the Mac).
- **Layout:**
  - `crates/cargo-tile/tests/shim_modes.rs` — shim mode tests; `MirrorStartup`, `assert_mirror_startup`, `mirror_startup_command` (about lines 1279-1420).
  - `crates/cargo-tile/src/shim_registration/` — the shim and its native mirror.
- **Test lanes:** `crates/cargo-tile/tests/`.
- **Build:** `bash ~/.claude/scripts/direct/verify.sh check cargo-tile`
- **Test:** `bash ~/.claude/scripts/direct/verify.sh test cargo-tile` (natedev); add `--mac` for the Mac.
- **Lint:** `bash ~/.claude/scripts/direct/verify.sh lint cargo-tile`
- **Invariants:** every test runs in under 1 s alone (user rule). No `#[allow]`. No duplicated code without approval. A failure is a defect to fix, never a load flake (user rule).

## Phases

### Phase 1 — The native mirror startup test passes on every Mac run  · status: todo

#### Work Order

**Goal:** Find why `a_native_mirror_keeps_cargo_streams_without_shell_utilities` failed once on the Mac, and fix the cause in the test or the code.

**Spec:**

1. The test runs four `MirrorStartup` cases in parallel (`MissingShell`, `MissingTee`, `FailedShell`, `InheritedFallback`) through `parallel_cases::run` and `assert_mirror_startup`.
2. The Mac log has no panic text. Get the failing assertion first:
   - Read the full nextest output for that run on the Mac, if it still exists.
   - Else run the test on the Mac many times in a loop through verify.sh, and capture the panic text of a failure.
3. Find the cause from the panic text and the code. Candidates: a race between the four parallel cases, a stream read that ends before the child writes, a timing or wait limit, or a macOS difference in `/bin/sh` or `tee`.
4. Fix the cause where it lives. If the cause is in the shim, fix the shim. Do not add retries or longer waits that hide a race.
5. Prove the fix: the test passes on the Mac in a repeated loop with no failure, and still runs in under 1 s alone.
6. If the cause also applies to `a_native_mirror_setup_failure_runs_original_cargo_without_artifacts`, fix it there too.

**Files:**
- `crates/cargo-tile/tests/shim_modes.rs` — the test and its helpers.
- `crates/cargo-tile/src/shim_registration/` — only if the cause is in the shim.
- `crates/cargo-tile/CHANGELOG.md` — one line, only if the shim changes.

**Workers:** 1 writer.
- `impl` — all files above.

**State:** none

**Constraints from prior phases:**
- `docs/cargo-tile/as-built/tile-fixes.md` holds the earlier tile work.

**Acceptance gate:**
- `bash ~/.claude/scripts/direct/verify.sh test cargo-tile` passes on natedev and on the Mac.
- `bash ~/.claude/scripts/direct/verify.sh lint cargo-tile` is clean.
- The repeated Mac loop of the test has no failure.
