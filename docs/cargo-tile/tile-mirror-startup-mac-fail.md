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
- **Project started:** 2026-11-25T14:11:07.087+00:00
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

### Phase 1 — The native mirror startup test passes on every Mac run  · status: done

#### As-built

- `hook.rs` `capture_terminal()` checks stdout and stderr with `rustix::io::fcntl_getfd`, not `/dev/fd` metadata. Each failed check returns its own fallback cause: `stdout descriptor check failed: …`, `stderr descriptor check failed: …`, or `<n> free descriptors in 3-9 (need MIN_MIRROR_FREE_DESCRIPTORS)`.
- The cause of the Mac failure: macOS can copy test-runner descriptors 3-9 into a child (a pipe and `FD_CLOEXEC` race). The shim then found too few free descriptors and fell back.
- `shim_modes.rs` tests close inherited descriptors with `CLOSE_CAPTURE_DESCRIPTORS` before their own setup; `OCCUPY_CAPTURE_DESCRIPTORS` fills 3-9. `mirror_startup_command` always runs `/bin/sh` with the close step. The `OccupiedDescriptors` row parses the free-descriptor count and expects 0. The `DescriptorLimit` row uses limit 4 on macOS and 6 on Linux.
- The `hook.rs` test `a_directory_arriving_at_publication_preserves_original_cargo` closes 3-8 in its shell wrapper for the same reason.

**Files:**
- `crates/cargo-tile/src/hook.rs` — the stream checks and fallback causes; the publication test wrapper.
- `crates/cargo-tile/tests/shim_modes.rs` — the mirror startup rows and descriptor constants.
- `crates/cargo-tile/CHANGELOG.md` — the Fixed line.

**Gotchas:** A native-shim test that starts a child through a shell must close descriptors 3-9 first, or it fails on the Mac now and then.

### Phase 2 — The ownership scenario ends its writer group on every exit path  · status: done

#### As-built

- In `assert_scenario_releases_resources`, the caller no longer kills the owner before the owner ends its cleanup. Its `finally` closes the lifetime pipe and the release pipe, kills only the intermediary in the caller-SIGKILL case, and waits up to `OWNERSHIP_CLEANUP_SECONDS` (half of `HARD_STOP_GRACE_SECONDS`) for the owner to close its output pipes.
- On a cleanup timeout, the caller does not kill or reap the owner, so the owner can still end its own groups. The caller kills the recorded groups in every case. It raises `TimeoutError` with the exit path and the owner PID, or adds that cause as a note to an earlier error.
- The cause of the Nov 15 Mac leak: a readiness or deadline failure before the caller recorded the group IDs let the caller SIGKILL the owner during cleanup.

**Files:**
- `crates/cargo-tile/src/shim_registration/reader_scenario_cases.py` — the caller's cleanup and `OWNERSHIP_EXIT_SECONDS` / `OWNERSHIP_CLEANUP_SECONDS`.

**Gotchas:** The cleanup wait must stay shorter than `HARD_STOP_GRACE_SECONDS`, or the faulthandler hard stop can end the caller before the group kill.

**Ruled out:** A stalled-owner test row (test rule 2 does not require it).
