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

#### As-built

- The census's timer tests build their fixture through `shell_timer(installation, below)`, where the test-only `ProgressTimerInstallation` (`Delegate`, `Unit`) selects the script path and run folder: `~/.claude/scripts/<folder>/progress_timer.sh /tmp/claude/<folder>/5ea59d53 300`.
- `a_timer_under_the_unit_folders_reads_as_under_the_delegate_folders` pins that a timer under the `unit` names reads the same `Timer` (start and deadline) as under `delegate`.
- No production code changed: timer detection was already independent of folder names.

**Files:**
- `crates/cargo-handler/src/census/tree.rs` — timer fixture and tests

**Gotchas:** timer detection (`Walker::timer`, `sleep_timer`) keys on the one `sleep` leaf under a shell wrapper, never on a script name or folder; keeping it that way is what lets a folder rename need no matcher change.
