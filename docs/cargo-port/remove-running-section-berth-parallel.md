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

**Depends on:** none

#### Work Order

**Worktree:** `/home/natepiano/rust/cargo-liner-berth-parallel`, branch `remove-running-section-berth-parallel`.

**Goal:** an edit check of a path the caller's own reservation already holds answers in milliseconds, even while another process holds the berth lock. Every other check's reconcile observes the live holders' worktrees concurrently, not one after another.

**Spec:**
1. **The fast path (`verb/check.rs` `execute`).** After the first lock-free `decide`, return `OutputEnvelope::clear_check` with an `AlreadyHeld` acquisition when all three of these hold:
   - its conflicts are empty;
   - the caller's identity selects one active reservation of its run in this worktree: the session mapping under `SessionMappingOrSingleActive`/`SessionMappingOrNew`, or the named id under `Explicit` only when the session already maps to that id or the invocation has no harness session. Naming another reservation switches the session's mapping, which is a durable write, so it keeps the locked path;
   - that reservation covers every requested scope under the same coverage rule the locked path uses to answer `AlreadyHeld`.

   This return comes before `reconcile_and_retry` and `acquire_first_touch`. Reuse that coverage rule and acquisition construction; do not write a second one. The envelope must be byte-identical to the locked path's answer for the same state:
   - message;
   - payload `acquisition` (`reservation_id`, `coordination_run_id`, `phase_start_head`, kind);
   - `session_mapping_publication`;
   - exit code.
2. **Every other case runs today's path unchanged.** That covers: no mapping, a single-active fallback without a mapping, a path not held, any conflict, a released or another run's reservation, and `Unconfigured`.
3. **Model the outcome as a type** that the first decision returns, for example `CheckDecision` carrying `HeldBySelectedReservation(acquisition)` against `NeedsLockedDecision`. Do not add a boolean.
4. **Concurrent holder observation (`reconcile.rs` `derive_merge_extents`).** Collect the distinct holder keys `(worktree, recorded target)` in today's first-seen order. Run `observe_merge_extent` for each on its own `std::thread::scope` thread, each with its own `MergeExtentGitCost`, and sum the costs afterwards. Then run the existing loop over the precomputed results. Operations, their order, `observed_by_holder`, `committed_by_holder` and the git cost totals stay identical. If a shared input is not `Sync`, fix that at the type rather than cloning per thread.
5. **Proof tests first, each under 1 s.**
   - (a) Integration (`tests/lifecycle.rs`):
     - set up a second worktree with a live reservation, so merge observation is required;
     - a session claims path P through `check`;
     - the test process takes an exclusive `flock` on `<common git dir>/cargo-berth/mutation.lock` and holds it;
     - the same session runs `check P --json`, which returns `clear` with the `AlreadyHeld` message and the same reservation id in well under 1 s;
     - with the lock still held, `check Q` for an unheld Q does not answer before the lock is released. Assert that it blocks briefly or fails at the lock deadline, using any existing test deadline override.
   - (b) Unit (`reconcile.rs` tests): derive merge extents for three live holders with an injected observer that waits on a `std::sync::Barrier` of three, under a timeout. If the observations run one after another, the barrier never fills and the test fails, so it proves they run concurrently. The observer is a generic or function parameter that production fills with `observe_merge_extent`. Keep it test-facing only, with no new public surface.
6. **CHANGELOG.** Add one `### Fixed` line under `[Unreleased]`.
7. **As-built doc.** Add a short "check under load" section to `docs/cargo-berth/as-built/hook-cost.md` with the measured numbers from Findings and the two changes.

**Files:**
- `crates/cargo-berth/src/verb/check.rs` — fast path and its outcome type
- `crates/cargo-berth/src/verb/claim/mod.rs` — expose the existing coverage rule and acquisition construction to `check` (crate-private)
- `crates/cargo-berth/src/ledger/handle.rs` — snapshot accessors, if needed
- `crates/cargo-berth/src/reconcile.rs` — concurrent observation, the injected observer, and the barrier unit test
- `crates/cargo-berth/tests/lifecycle.rs` — the held-lock proof test
- `crates/cargo-berth/CHANGELOG.md`, `docs/cargo-berth/as-built/hook-cost.md`

**Workers:** 1 writer + 1 tester.
- `impl` — `src/` files, the CHANGELOG and the as-built doc.
- `test` — `tests/lifecycle.rs`. The barrier unit test lives in `src/reconcile.rs`; `impl` writes it.

**Acceptance gate:**
- `verify.sh lint cargo-berth` and `verify.sh test cargo-berth` are green, and every new test runs under 1 s.
- The director re-runs the hana trace and records the before and after numbers.
- The director installs on natedev and the Mac, then sends the showrunner the install notice.
- The checkpoint carries `Change-Review:` for the fast answer and the concurrent statuses.

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

**Files:** `crates/cargo-berth/tests/lifecycle.rs`, `crates/cargo-berth/tests/hooks.rs`, `crates/cargo-berth/src/reconcile.rs` (unit tests only).

**Workers:** 1 tester (`test`); `impl` only for a defect the tests find.

**Acceptance gate:** `verify.sh lint cargo-berth` and `verify.sh test cargo-berth` are green, and every new test runs under 1 s.
