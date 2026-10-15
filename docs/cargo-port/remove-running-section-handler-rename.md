# handler-rename

> **Status: IMPLEMENTATION PLAN — phased, delegate-ready.** cargo-handler's census keeps reading progress timers once the run folders and scripts move from `delegate` to `unit`.

> **Production: remove-running-section** — unit `handler-enh-unit`; production doc `docs/cargo-port/remove-running-section-production.md`

## Source

2026-10-15, natedev (owner of `~/.claude`), with the user's approval: `/tmp/claude/delegate/` (with `active/`) becomes `/tmp/claude/unit/`, and `scripts/delegate/` becomes `scripts/unit/`. Both names stay readable until the last old run finishes. The showrunner's ruling: land the census side now as a one-phase plan, and include a test that matches the new names.

## Cause

No production code in `crates/cargo-handler/src/census` names `progress_timer.sh`, `scripts/delegate` or `/tmp/claude/delegate`. Timer detection is generic: it finds a `sleep` under a wrapper shell. The old names appear only in the test fixture `shell_timer` (`crates/cargo-handler/src/census/tree.rs` ≈1155), so nothing pins that the new names work.

## Delegation Context

- **Project:** `cargo-handler`, the agent watcher.
- **Project started:** 2026-10-15T14:46:32.602+00:00
- **Stack:** Rust, cargo-nextest.
- **Worktree:** `/home/natepiano/rust/cargo-liner-handler-enh`, branch `remove-running-section-handler-enh`.
- **Key files:** `crates/cargo-handler/src/census/tree.rs`: the test helper `shell_timer` (≈1155), which builds a wrapper shell running `bash ~/.claude/scripts/delegate/progress_timer.sh /tmp/claude/delegate/5ea59d53 300`, and the timer tests that call it.
- **Test:** `bash ~/.claude/scripts/delegate/verify.sh test cargo-handler`
- **Lint:** `bash ~/.claude/scripts/delegate/verify.sh lint cargo-handler`
- **Invariants:** tests run with nextest. Format with `cargo +nightly fmt` only. No test takes a second.

## Phases

### Phase 1 — The census reads timers under both the old and new run folders  · status: done

**Depends on:** none

#### Work Order

**Goal:** a test proves that a progress timer started from `scripts/unit/progress_timer.sh` under `/tmp/claude/unit/<run>` reads exactly as one under the old names.

**Spec:**
1. Make `shell_timer` take the timer's command line (the script path and its run folder), so one helper builds either name.
2. Add a test that runs the existing timer case with `bash ~/.claude/scripts/unit/progress_timer.sh /tmp/claude/unit/5ea59d53 300` and asserts the same `Timer` as the old-name case.
3. Change no production code unless the new test fails. If it fails, fix the matcher so it accepts both names.

**Files:**
- `crates/cargo-handler/src/census/tree.rs`

**Workers:** 1 writer — one test module in one file; nothing splits.
- `impl` — `crates/cargo-handler/src/census/tree.rs`

**Acceptance gate:** the new test passes beside the old one; the cargo-handler suite and lint are green.
