# berth-parallel

> **Status: IMPLEMENTATION PLAN — phased, delegate-ready.** An edit check of a path the session already holds answers at once, and a reconcile observes every holder's worktree at the same time, so `cargo-berth check` stays fast under heavy load.

> **Production: remove-running-section** — unit `berth-parallel-unit`; production doc `docs/cargo-port/remove-running-section-production.md`

## Source

2026-10-16, from the showrunner, reported by hana:

Under heavy load on natedev (load average 100 on 32 cores), the edit hook's `cargo-berth check` hits its 25 s limit ("timed_out: cargo-berth check did not answer within 25 seconds") and blocks a writer's edit. That happened 3 times to inspectors Phase 7. By hand the same check takes 4–7 s, and the reservation was clean. Fix both sides. (1) A clean edit the session already holds must not be blocked by a timeout: when the path is already in this session's reservation, answer from that before any slow work. (2) Find where the 4–7 s goes, and make the check fast under load. The user's standing rule for berth bugs: fix it fast and install it on both machines, with a test proving the fix works first, then do the full testing after.

## Findings before planning

Measured 2026-10-16 10:26–10:30 on natedev (load average 63–127) in `/home/natepiano/rust/hana_catalyst-inspectors`, whose repository has 17 worktrees. The measurements used `cargo-berth board --json`, which runs the same `reconcile::reconcile` as `check`'s slow path, with `strace` and `GIT_TRACE2_EVENT`.

- **Every check reconciles.** `check::decide` (`verb/check.rs` ~286) sets `MergeObservationRequirement::Required` whenever any other worktree holds a reservation that is not released. In hana that is always true, so `execute` always goes to `reconcile_and_retry`, which runs a full `reconcile::reconcile` under the mutation lock. That happens even when every requested path is already in the caller's own reservation.
- **The reconcile holds the lock for ~10 s under load.** One traced run waited 2.6 s for `mutation.lock`, held it from 2.57 s to 12.74 s, and ended at 14.3 s. Of the ~10 s under the lock, 5.5 s was 11 `git status --porcelain -z` calls run **one after another**, one per live holder worktree (0.15–1.32 s each). That is `derive_merge_extents` → `observe_merge_extent` → `drift::observe_merge_working_tree` (`reconcile.rs` ~2278 and ~2407). `update-ref --stdin` took 1.2 s and the rev-lists ~1.5 s. Of four runs, one ended at 5.8 s, one at 23 s (git status alone took 9.8 s), and two at the 10 s lock deadline (exit 6), waiting behind other sessions' reconciles.
- **So the timeout is lock queueing.** Every session's hook reconciles under the one lock. While each holds it for 5–10 s, a check waits for the reconciles queued ahead of it, then pays for its own, and reaches 25 s.
- **Ledger decode is a smaller cost.** hana's `journal.ndjson` is 356 MB and `replay-checkpoint.json` is 25 MB. A check decodes the checkpoint up to four times (lock-free `decide`, the reconcile's locked read, the retried `decide`, and `acquire_first_touch`'s transaction). One decode measured 135 ms. It is not addressed here, and Phase 6 measures it again.
- **Both edit paths share `check::execute`.** Claude Code edits reach it through `hook/pre_tool_use.rs` ~232. Codex seat edits reach it through `cargo-berth check --json`, which `~/.claude/scripts/delegate/phase_reservation.py` runs with a 25 s timeout. A fix in `check::execute` covers both.
- **The answer for an already-held path exists.** `ReservationAcquisitionKind::AlreadyHeld` renders "No foreign reservation overlaps the requested paths; the acting run already holds them." (`output.rs` ~2686).
- **The code matches main.** `origin/main` has no cargo-berth change that this branch lacks (checked at c74a3a70), so installing from this branch drops nothing.

## Decisions made here

- **Fast path scope (showrunner ruling for (1)).** Fast path scope (the showrunner's ruling for (1)). When every requested path is already covered by the one active reservation that the caller's identity selects, and the lock-free snapshot shows no conflicts, `check` answers `AlreadyHeld` from that snapshot before it takes any lock or runs any reconcile. The identity selects a reservation through its session mapping, or through `--reservation <id>` when given. An unmapped caller, a path not yet held, or any conflict keeps today's path. Risk: a foreign worktree may have dirtied a path this session holds since the last reconcile. The fast path does not see that until the next reconcile, which is the foreign holder's incursion to answer and not this edit's.
- **The holder observations run concurrently.** `derive_merge_extents` runs each distinct holder's observation on its own scoped thread, then folds the results in the existing order. Every journal operation, order and JSON stays byte-identical. Risk: up to one `git status` per live worktree runs at once, about 20 processes for a short time instead of one after another.
- **Moving observation out of the lock is not part of this run.** It would shorten the lock hold further, but it needs a preflight-and-validate design. After Phase 6 the trace is taken again, and the remaining cost decides whether it becomes a follow-up.
- **Install from this branch.** After Phase 6 checkpoints, install cargo-berth on natedev from this worktree. Install it on the Mac from the pushed branch over `ssh mac`, then tell the showrunner.

## Delegation Context

- **Project:** `cargo-berth`, the worktree reservation ledger and hook engine.
- **Project started:** 2026-10-16T14:31:00.000+00:00
- **Worktree:** `/home/natepiano/rust/cargo-liner-berth-parallel`, branch `remove-running-section-berth-parallel`. Every seat works only here, and every Work Order names it.
- **Base commit:** `c74a3a70`.
- **Test lanes:** `crates/cargo-berth/tests/`.
- **Stack:** Rust workspace, cargo-nextest. The crate has no lib target: unit tests live in `cargo-berth::bin/cargo-berth` (`--bins`); integration tests in `crates/cargo-berth/tests/`.
- **Key files:**
  - `crates/cargo-berth/src/verb/check.rs` — `execute`, `decide`, `MergeObservationRequirement`.
  - `crates/cargo-berth/src/verb/claim/mod.rs`:
    - `CheckReservationSelection` (~481) and `acquire_first_touch_with_reservation_selection` (~885);
    - the locked already-held path and how it builds `ReservationAcquisition` with `AlreadyHeld` and its `session_mapping_publication`.
  - `crates/cargo-berth/src/ledger/handle.rs` — `read_for_edit_check` (~308).
  - `crates/cargo-berth/src/reconcile.rs` — `derive_merge_extents` (~2278), `observe_merge_extent`, `MergeExtentGitCost`.
  - `crates/cargo-berth/src/output.rs` — `clear_check` (~2671).
  - `crates/cargo-berth/tests/lifecycle.rs`, `tests/hooks.rs` — check and pre-tool-use tests; `run_berth_with_session`.
  - `docs/cargo-berth/as-built/hook-cost.md` — the hook cost record; `worktree-coordination.md` — check's behavior.
  - `crates/cargo-berth/CHANGELOG.md` — `## [Unreleased]`.
- **Build:** `bash ~/.claude/scripts/delegate/verify.sh check cargo-berth`
- **Test:** `bash ~/.claude/scripts/delegate/verify.sh test cargo-berth`
- **Lint:** `bash ~/.claude/scripts/delegate/verify.sh lint cargo-berth`
- **Style:** run-end /clippy style-only auto-proceed
- **Invariants:**
  - Identity resolves once per invocation through `ledger::resolve_identity`. The session mapping outranks `CARGO_BERTH_RUN` and the marker (as-built `worktree-coordination.md`).
  - The fast path writes nothing and takes no lock. Its answer is the same envelope the locked path gives for the same state.
  - Reconcile output is unchanged by concurrency: the same operations in the same order, and the same git cost counts.
  - Invalid states are unrepresentable: model the fast-path outcome as a type, not a flag.
  - No test takes a second (memory `no-multi-second-tests`). Format with `cargo +nightly fmt` only. Run one `verify.sh` at a time.

## Phases

### Phase 6 — A held path answers at once, and holders are observed together  · status: done

#### As-built

- `check` answers `clear` with an `AlreadyHeld` acquisition straight from the lock-free `EditCheckLedgerSnapshot`, before `reconcile_and_retry` and `acquire_first_touch`, when no conflict exists and the caller's selected reservation already covers every requested exact-file scope. No lock, no reconcile, no writes; the envelope (message, acquisition, `session_mapping_publication`, exit code) matches the locked path's answer for the same state.
- The decision is a type: `CheckDecision` carries an acquisition from `selected_reservation_check_acquisition`, a `CheckAcquisitionDecision` built from the same helpers the locked path uses (`partition_first_touch_protected_scopes`, `already_held_acquisition`, `first_touch_acquisition`). Every other case (no mapping, unheld path, any conflict, released or foreign reservation, `Unconfigured`) takes the locked path.
- `derive_merge_extents` collects distinct holder keys in first-seen order and observes each on its own `std::thread::scope` thread via `observe_holder_merge_extents`, each with its own `MergeExtentGitCost`; results fold in first-seen order and costs are summed. `derive_merge_extents_with_observer` injects the observer for a three-party `Barrier` unit test.

**Files:**
- `crates/cargo-berth/src/verb/check.rs` — `CheckDecision` and the fast return
- `crates/cargo-berth/src/verb/claim/mod.rs` — `CheckAcquisitionDecision`, `selected_reservation_check_acquisition` and the shared acquisition helpers
- `crates/cargo-berth/src/ledger/handle.rs` — `EditCheckLedgerSnapshot` keeps the full `BerthConfig`
- `crates/cargo-berth/src/reconcile.rs` — concurrent holder observation, the injectable observer, the barrier test
- `crates/cargo-berth/tests/lifecycle.rs` — `a_session_checks_its_held_path_without_waiting_for_the_mutation_lock`
- `docs/cargo-berth/as-built/hook-cost.md` — "Check under load" section; `crates/cargo-berth/CHANGELOG.md` — `### Fixed` line

**Gotchas:**
- An explicit `--reservation` other than the session's mapped one keeps the locked path, since remapping is a durable write.
- The fast path reports marker and mapping publication as `published` without writing; that holds because the mapping already exists.
- The spawn loop collects every handle before joining; clippy `needless_collect` rejects the map/collect form.
- When check's locked reconcile fails at the lock deadline, it answers `blocked_by_overlap` exit 1 with no conflicts, not contention exit 6.
- At a load average near 30 each holder's `git status` still costs about 10 ms, so the concurrency gain shows only under heavier load.

**Ruled out:** a lock-free fast path for an explicit remap (needs a durable write); a new "already current" mapping-publication variant (`published` is already true).

### Phase 7 — Robust tests  · status: pending

**Depends on:** Phase 6

#### Work Order

**Worktree:** `/home/natepiano/rust/cargo-liner-berth-parallel`, branch `remove-running-section-berth-parallel`.

**Goal:** pin every edge of the fast path and of concurrent observation.

**Spec:** Each test runs under 1 s.
1. **The fast path declines when it should.**
   - (a) A path held by a sibling phase reservation of the same run but not by the caller's mapped one goes to today's path (`left_unmapped` answer unchanged).
   - (b) A tree scope partially covered goes to today's path.
   - (c) A conflict present in the snapshot goes to today's path and is refused as today.
   - (d) An unmapped session with one active run reservation goes to today's path.
   - (e) `--reservation <id>` naming the reservation the session already maps to takes the fast path. Naming a different one of the run's reservations remaps the session under the lock and does not.
   - (f) A released mapped reservation goes to today's path.
2. **Envelope equality.** For the same state, the fast answer's JSON equals the locked path's `AlreadyHeld` JSON. Get the locked answer by running with no foreign holder, where no reconcile runs, then compare field by field, ids normalized.
3. **The pre-tool-use hook takes the fast path.** A Claude Code `Edit` payload for a held path, while the test holds the lock, is allowed in under 1 s (`tests/hooks.rs`).
4. **Concurrent reconcile output is unchanged.** Two holders on the same worktree with different targets, plus one holder whose observation fails (an unavailable worktree), give the same operations, order and cost counts as a serial reference computed in the test.

**Constraints from prior phases:**
- The fast path is `claim::selected_reservation_check_acquisition` (`verb/claim/mod.rs`), returning `CheckAcquisitionDecision::HeldBySelectedReservation`; `check::execute` returns it before any reconcile. It reports `marker_publication` and `session_mapping_publication` as `published` without writing, because the mapping already names that reservation.
- An explicit `--reservation` other than the one the session maps to returns `NeedsLockedDecision`; with no harness session, an explicit id takes the fast path when that reservation covers the scopes.
- Holder observation runs through `derive_merge_extents_with_observer` (`reconcile.rs`), whose observer argument is the injection point for item 4. The spawn loop must collect every handle before joining.
- Existing proof test: `a_session_checks_its_held_path_without_waiting_for_the_mutation_lock` (`tests/lifecycle.rs`). Its unheld control asserts only a nonzero exit and a status that is not `clear`, because a check whose reconcile hits the lock deadline answers `blocked_by_overlap` (exit 1) today; phase 8 changes that answer.

**Files:**
- `crates/cargo-berth/tests/lifecycle.rs`
- `crates/cargo-berth/tests/hooks.rs`
- `crates/cargo-berth/src/reconcile.rs`

**Workers:** 1 tester (`test`), which edits `reconcile.rs` only inside its `tests` module; `impl` only for a defect the tests find.

**Acceptance gate:** `verify.sh lint cargo-berth` and `verify.sh test cargo-berth` are green, and every new test runs under 1 s.

### Phase 8 — A busy ledger answers as busy  · status: pending

**Depends on:** Phase 7

#### Work Order

**Worktree:** `/home/natepiano/rust/cargo-liner-berth-parallel`, branch `remove-running-section-berth-parallel`.

**Goal:** when `check`'s reconcile fails, the answer names the real failure, never an overlap with no holder.

**Spec:** Each test runs under 1 s.
1. In `crates/cargo-berth/src/verb/check.rs` `reconcile_and_retry`, the `Err(_)` arm returns `OutputEnvelope::blocked_check(fallback.scopes, fallback.conflicts, fallback.approver)`: a lock deadline reads as `blocked_by_overlap`, exit 1, with an empty conflict list. Replace it with the reconcile error's own answer, `ReconcileError::into_output(CommandVerb::Check)` (`reconcile.rs`), so a lock deadline answers `contention`, exit 6, with the busy diagnostic, and every other reconcile failure answers as that failure. Drop `fallback` if nothing else reads it.
2. Tighten `a_session_checks_its_held_path_without_waiting_for_the_mutation_lock`'s unheld control (`tests/lifecycle.rs`) to status `contention` and exit 6.
3. Add a test that the pre-tool-use hook (`tests/hooks.rs`), given a check that answers `contention`, tells the writer the ledger was busy and names no holder.
4. CHANGELOG `Fixed` line: a busy ledger during an edit check now answers as busy, not as an overlap.

**Files:**
- `crates/cargo-berth/src/verb/check.rs`
- `crates/cargo-berth/tests/lifecycle.rs`
- `crates/cargo-berth/tests/hooks.rs`
- `crates/cargo-berth/CHANGELOG.md`

**Workers:** 1 writer + 1 tester. `impl` owns `src/` and `CHANGELOG.md`; `test` owns `tests/`.

**Acceptance gate:** `verify.sh lint cargo-berth` and `verify.sh test cargo-berth` are green, and every new test runs under 1 s. After the checkpoint, install on natedev and the Mac.
