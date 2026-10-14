# berth-hook-hang

> **Status: IMPLEMENTATION PLAN — phased, delegate-ready. Fast pass shipped `e82cd2d2` (Fix A); this plan is the second pass.** `cargo-berth hook post-tool-use` reduces a foreign worktree's scopes in near-linear time and works out each holder's protection once per subject, so the hook no longer spins.

> **Production: remove-running-section** — unit `berth-hook-hang-unit`; production doc `docs/cargo-port/remove-running-section-production.md`

## Source

2026-10-13 23:00 EDT

The user (via natedev, 2026-10-13): "If there is a berth bug send it to cargo-liner with the instructions to start a new unit to address it". The showrunner, from here on: find and fix this cargo-berth defect. In /home/natepiano/rust/tool-based-ui-trunk every `cargo-berth hook post-tool-use` runs one thread at 98% CPU, with no child processes and no output, until the 600 s hook timeout ends it, on every tool call since that worktree commit 12375bcb1 (22:18 EDT). No other worktree is affected; `cargo-berth board --json` returns in seconds (733 reservations, 4.3 MB). Trunk has five modified files and one live reservation, 01a13622, with 5 scopes. Full evidence: /home/natepiano/.claude/state/berth-fix/inbox/20261014T025825Z-hana/ (report.md with exact commands and a retry command, meta.json, ledger-snapshot.tar.gz taken 22:58 EDT). Unproven lead: a scan over all 733 reservations or their scopes inside the hook for this reservation shape. Rules: reproduce in a throwaway copy of that worktree at 12375bcb1 with the ledger snapshot; never run experiments in trunk own worktree; never retire or change reservation 01a13622. Deliver: the cause, a fix in crates/cargo-berth with a test that reproduces the spin and ends in well under a second, and a checkpoint for the showrunner to merge and install on natedev and the Mac. This is urgent: the trunk unit loses 10 minutes per tool call. One phase; send the showrunner (cargo-liner) the cause as soon as it is known.

The showrunner (cargo-liner, 2026-10-13 23:08 EDT), after the cause report: "cause accepted. Your plan is approved in advance as one phase with exactly the two fixes you named: a near-linear reduction with duplicates removed, and a foreign worktree's protection computed once per drift run. The test must have this shape (13 reservations, 4,732 scopes, 364 distinct) and end well under a second. Also time the real reproduction copy before and after, and state both figures."

The showrunner (cargo-liner, 2026-10-13 23:2x EDT), relaying the user: "Cargo berth should fix it fast and install the fix and then do the robust testing and all of that after. It needs a test to be sure it's fix works first of course but I don't want to go through the whole dev process to get this fixed". The showrunner: make the smallest correct fix yourself, commit, send the sha with both timings; "After that is installed: write the plan and run the full process (/unit:direct, reviews, the wider tests) as a second pass over the same change."

*Fast pass, done 2026-10-13.* Fix A landed in `e82cd2d2` without seats or review: `reduce` in near-linear time, the pairwise body kept as test-only `reduce_by_pairs`, two equivalence tests and `reduction_of_the_reported_shape_is_immediate` (failed on the old reduction at 2.86 s for one reduction; 19 ms after). Hook in the reproduction copy: before, killed at the 600 s limit with no output; after, 20.4 s, exit 0. This phase is the second pass: it reviews Fix A as landed and builds Fix B.

## Delegation Context

- **Project:** `cargo-berth` — the reservation ledger that keeps worktrees of one repository from editing each other's files; its hooks run on every tool call.
- **Project started:** 2026-10-14T03:18:30.095+00:00
- **Worktree:** `/home/natepiano/rust/cargo-liner-berth-hook-hang`, branch `remove-running-section-berth-hook-hang`. All work happens here.
- **Stack:** Rust (workspace toolchain), `serde`/`serde_json`; tests run under `cargo nextest` through `verify.sh`; format only with `cargo +nightly fmt`.
- **Layout:** `crates/cargo-berth/src/scope/` (scope sets and their reduction), `crates/cargo-berth/src/reservation/` (the replayed reservation set and conflict answers), `crates/cargo-berth/src/drift/` (the drift comparison the hook runs).
- **Key files:**
  - `crates/cargo-berth/src/scope/antichain.rs` — `reduce` (line 8), `overlaps`, `contains`; 100 lines with two tests.
  - `crates/cargo-berth/src/scope/mod.rs` — `PathCase::component_eq` (line 48: exact, or `to_lowercase` on both components), `minimal_antichain` (line 146), `into_minimal_antichain` (line 88).
  - `crates/cargo-berth/src/ledger/journal.rs` — `ReservationScope::covers_path_by` (line 1263), `ReservationScopeSet` (line 1294, non-empty, `TryFrom<Vec<ReservationScope>>`).
  - `crates/cargo-berth/src/reservation/retention.rs` — `blocking_coverage_for_drift` (line 438), `protection_for_conflict` (line 1765), `conflicts_with_holders` (line 1821); its `mod tests` (line 1917) replays journal events built with `json!`, e.g. `lifecycle_events()` (line 3658) and the merge-extent events near line 3491.
  - `crates/cargo-berth/src/reservation/record.rs` — `edit_blocking_status` (line 423), `protection_in_worktree` (line 337), `ConflictProtection` (line 113).
  - `crates/cargo-berth/src/drift/classification.rs` — `PreLockForeignPathClassification::build` (line 70) and its `blocking_coverage` method (line 125), `classify_locked` (line 338, coverage asked at line 373), the free `blocking_coverage` (line 644).
- **Test lanes:** `crates/cargo-berth/tests/` exists, but every test of this phase is a unit test inside the source file it proves; an integration test of this shape would need a real git fixture with 13 claims and would run for seconds.
- **Build:** `bash ~/.claude/scripts/delegate/verify.sh check cargo-berth`
- **Test:** `bash ~/.claude/scripts/delegate/verify.sh test cargo-berth`
- **Lint:** `bash ~/.claude/scripts/delegate/verify.sh lint cargo-berth`
- **Style:** run-end /clippy style-only auto-proceed
- **Invariants:**
  - Every answer `cargo-berth` gives is unchanged: the same conflicts, the same scopes in the same order, the same JSON. This phase changes cost only. (Unit director's rule, from the user's ranking: correctness is the floor.)
  - No test takes a second. (User rule.)
  - Never run an experiment in `/home/natepiano/rust/tool-based-ui-trunk`, and never retire or change reservation `01a13622`. (Showrunner's rule, in Source.)

## Phases

### Phase 1 — The hook reduces scopes in near-linear time and asks each holder once  · status: done

#### As-built

- `pub(super) fn reduce(scopes: ReservationScopeSet, path_case: PathCase) -> ReservationScopeSet` keys each path by its components, case-folded under `PathCase::Insensitive` (`PathCase::component_key`), and decides each scope by hash lookups of its own key and each ancestor prefix; no pair of scopes is compared. Its output equals the old quadratic rule scope for scope and in order: drop a scope under a strict-ancestor `Tree`, a `File` sharing a path with any `Tree`, or a repeat of an earlier (path, kind); a `File` never removes a `Tree`. The empty-result fallback `ReservationScopeSet::try_from(reduced).unwrap_or(scopes)` remains. `contains` and `overlaps` stay pairwise.
- `pub(crate) struct DriftCoverageProbe<'reservations>` holds one subject's standing and foreign holders, each with its `ConflictProtection::Protected` representative and reduced scopes computed once at build, and `RecordedAt::now()` read once. `RetainedReservationSet::drift_coverage_probe(&self, subject_worktree_id: WorktreeId, subject_coordination_run_id: CoordinationRunId, path_case: PathCase) -> DriftCoverageProbe<'_>` builds it; `DriftCoverageProbe::coverage(&self, candidate: &ReservationScopeSet) -> DriftBlockingCoverage` runs only the overlap step and answers `NoForeignStanding`, `Foreign(conflicts)` or `Unclaimed` as before.
- `conflicts_with_holders` is `protected_conflict_holders` (selection) followed by `conflicts_overlapping_holders` (per-candidate overlap); `conflicts`, `bind_widened_scopes` and `conflicts_for_authorized_edit` are unchanged. `conflicts_for_drift` is gone; `blocking_coverage_for_drift` is a `#[cfg(test)]` wrapper over the probe.
- `PreLockForeignPathClassification::build` and `classify_locked` build one probe per subject reservation, from `reservation.actor().worktree` and `.run`, before `changes.visit_paths`; both `blocking_coverage` functions take the probe.
- Hook on the reported ledger: 600 s (killed) → 20.4 s with keyed reduction → 7.8 s with the probe, same answer.

**Files:**
- `crates/cargo-berth/src/scope/antichain.rs`, `crates/cargo-berth/src/scope/mod.rs` — keyed `reduce`, test-only `reduce_by_pairs` as reference, equivalence test over both `PathCase` values, 4,732-scope timing test.
- `crates/cargo-berth/src/reservation/retention.rs` — `DriftCoverageProbe`, the selection/overlap split, the test wrapper, the 13-reservation timing test.
- `crates/cargo-berth/src/drift/classification.rs` — one probe per subject per pass.

**Binds later work:** the probe and keyed reduction stay as landed; a probe is built per subject per pass. The 3.7 s journal-replay figure predates the probe; profile the current build before acting on it.

**Gotchas:** never share a probe between the pre-lock and locked passes — they read different reservation sets (snapshot vs. replay under the lock), so a shared probe answers from a stale ledger.

### Phase 2 — The hook replays the journal in a fraction of its 3.7 s  · status: todo
**Depends on:** Phase 1

#### Work Order

**Goal:** journal replay in the reproduction copy's hook run costs a fraction of its measured 3.7 s, with every replayed answer unchanged.

**Spec:**

The showrunner (cargo-liner, 2026-10-13): "journal replay (3.7 s) becomes your next phase in this unit, without waiting to ask me; add it to the plan now. After that, report the time left and its split, and I decide whether anything further is worth a phase."

*Measured (perf, frame-pointer build of `e82cd2d2`).* `Ledger::begin_locked_transaction` → `Journal::replay_repairing_tail_from_checkpoint` → `ReplayCheckpoint::replay_journal` → `JournalReplay::extended_over` costs 3.7 s. Of that, `decode_complete_records` takes 3.2 s, and its record split (`position` on a byte slice inside `decode_complete_records`) alone takes 2.3 s. A gdb sample also landed in `Reservation::retain_target_proof` under `RetainedReservationSet::apply_evidence`, in `memcmp`.

*Before dispatch, the unit director profiles the replay alone in the copy, names the cut that removes the largest share, and writes it here as the concrete Spec.* The candidates the profile must decide between: the record split scanning the whole journal once per record, rather than once overall; a checkpoint that does not cover the journal tail the hook replays; and the evidence retention comparison. The answer stays identical: the same `RetainedReservationSet` after replay, checked by a test that replays the same journal through the old and the new paths.

Test: a unit test replays a journal of the copy's size class (thousands of records), asserts the replayed set equals the reference, and ends in well under a second.

**Files:**
- `crates/cargo-berth/src/ledger/journal.rs` — `decode_complete_records`, `JournalReplay::extended_over`.
- `crates/cargo-berth/src/ledger/replay_checkpoint.rs` — `ReplayCheckpoint::replay_journal`.
- `crates/cargo-berth/src/reservation/record.rs` — `retain_target_proof`, only if the profile names it.

**Seats:** 2 writers — set when the concrete Spec is written; by default `impl` owns `ledger/`, `test` owns `reservation/record.rs` and the replay equivalence test.
- `impl` — `crates/cargo-berth/src/ledger/journal.rs`, `crates/cargo-berth/src/ledger/replay_checkpoint.rs`.
- `test` — opens as impl: `crates/cargo-berth/src/reservation/record.rs`.

**Constraints from prior phases:** Phase 1's probe and reduction stay as landed.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh check cargo-berth`, `bash ~/.claude/scripts/delegate/verify.sh test cargo-berth` and `bash ~/.claude/scripts/delegate/verify.sh lint cargo-berth` all green; the replay equivalence test passes; the unit director states the hook time in the copy and the split of what is left.
