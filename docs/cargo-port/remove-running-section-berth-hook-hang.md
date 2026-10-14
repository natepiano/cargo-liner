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
**Depends on:** none

#### Work Order

**Goal:** `cargo-berth hook post-tool-use` in the reproduction copy of tool-based-ui-trunk returns instead of spinning, because scope reduction is near-linear and a holder's protection is worked out once per subject instead of once per changed path.

**Spec:**

*The measured cause.* Three stack samples of the spinning hook all read `scope::antichain::reduce` ← `RetainedReservationSet::protection_for_conflict` ← `conflicts_with_holders` ← `blocking_coverage_for_drift` ← `drift::classification::blocking_coverage` ← `PreLockForeignPathClassification::build`. `build` asks coverage once per changed path; each ask runs `conflicts_with_holders` twice; each run calls `protection_for_conflict` for every foreign worktree; that joins the protected scopes of every blocking reservation the worktree holds, duplicates kept, and reduces them. `reduce` compares every scope with every other, and each comparison allocates two strings and two vectors. tool-based-ui-geometry-material held 13 blocking reservations whose merge extents total 4,732 scopes, 364 distinct: 22.4 million comparisons per reduction.

*Fix A — `reduce` in near-linear time, `crates/cargo-berth/src/scope/antichain.rs`. Landed in `e82cd2d2`; seats do not rewrite it. Review reads it against this spec, and a confirmed finding is repaired in that file.* Keep the signature `pub(super) fn reduce(scopes: ReservationScopeSet, path_case: PathCase) -> ReservationScopeSet` and return exactly what today's function returns, in the same order, for every input. Today's rule, restated: a scope is dropped when (1) a `Tree` scope sits on one of its strict ancestor paths, or (2) it is a `File` and a `Tree` scope has the same path, wherever that tree stands in the list, or (3) an earlier scope has the same path and the same kind. Everything else is kept in its original position order. A `File` never removes a `Tree`. Two paths are the same when every component is equal under `PathCase::component_eq`, so the key for a path is its components, each lower-cased with `str::to_lowercase` under `PathCase::Insensitive` and untouched under `PathCase::Sensitive`. Build the keys once, hold the tree keys and the seen `(key, kind)` pairs in hash sets, and decide each scope by looking up its own key and each of its ancestor prefixes. No pair of scopes is ever compared. Leave `contains` and `overlaps` as they are: their callers compare one pair. Keep the empty-result fallback `ReservationScopeSet::try_from(reduced).unwrap_or(scopes)`.

Tests in the same file:
- Keep today's quadratic body as a test-only function `reduce_by_pairs` and assert `reduce` equals it, scope for scope, on a generated corpus: nested trees, a tree and a file on one path in both orders, repeated scopes, sibling files, paths that differ only in case, under both `PathCase` values.
- `reduction_of_the_reported_shape_is_immediate`: 13 copies of 364 distinct file scopes (4,732 scopes) reduce to those 364 in first-occurrence order, and the call returns in under 100 ms, measured with `std::time::Instant`.

*Fix B — a holder's protection once per subject, `crates/cargo-berth/src/reservation/retention.rs` and `crates/cargo-berth/src/drift/classification.rs`.* Add

```rust
/// What blocks one subject's changed paths, with every holder's protection worked out once.
pub(crate) struct DriftCoverageProbe<'reservations> { /* private */ }

impl RetainedReservationSet {
    pub(crate) fn drift_coverage_probe(
        &self,
        subject_worktree_id: WorktreeId,
        subject_coordination_run_id: CoordinationRunId,
        path_case: PathCase,
    ) -> DriftCoverageProbe<'_>;
}

impl DriftCoverageProbe<'_> {
    pub(crate) fn coverage(&self, candidate: &ReservationScopeSet) -> DriftBlockingCoverage;
}
```

The probe holds the two holder lists `blocking_coverage_for_drift` reads today: the holders standing with the subject (the predicate `!holder.is_foreign_to_coordination_run_in_worktree(run, worktree)`) and the foreign ones (the predicate without the `!`). Each entry is the holder `conflicts_with_holders` would pick, with the `ConflictProtection::Protected` representative and reduced scopes from `protection_for_conflict`, computed when the probe is built. Split `conflicts_with_holders` into that selection step and a per-candidate overlap step, so `conflicts_with_holders` itself stays one call of each and every other caller (`conflicts`, `bind_widened_scopes`, `conflicts_for_authorized_edit`) is untouched. `coverage` runs only the overlap step: `NoForeignStanding` when the standing list overlaps the candidate, otherwise `Foreign(conflicts)` or `Unclaimed` from the foreign list, exactly as lines 445–466 decide today. `RecordedAt::now()` for `activity` is read once when the probe is built. `blocking_coverage_for_drift` remains and becomes `self.drift_coverage_probe(..).coverage(candidate)`, so its existing tests prove the probe.

In `classification.rs`, build one probe per subject reservation before `changes.visit_paths` in both `PreLockForeignPathClassification::build` (line 79) and `classify_locked` (line 369), from `reservation.actor().worktree` and `reservation.actor().run`, and pass it to the free `blocking_coverage` and to `PreLockForeignPathClassification::blocking_coverage` in place of the reservation set. The two passes read different reservation sets, the snapshot before the lock and the replay under it, so no probe is shared between them: "once per drift run" is once per subject per pass. (Unit director's reading of the showrunner's words; sharing across the passes would give a stale answer.)

Test in `retention.rs` `mod tests`, `drift_coverage_of_the_reported_shape_ends_well_under_a_second`: replay a journal in which one foreign worktree holds 13 blocking reservations, each with a merge extent of the same 364 file scopes (4,732 in all), and a subject reservation in another worktree. Build one probe for the subject and ask coverage for 200 changed paths: 100 inside the 364 and 100 outside. Assert every inside path answers `Foreign` with one conflict naming the first of the 13 as representative, every outside path answers `Unclaimed`, and the whole test body ends in under one second by `Instant`. Before the fix the same questions through `blocking_coverage_for_drift` cost 400 reductions of 22.4 million comparisons each.

*Timing in the reproduction copy (unit director, not a seat).* The copy is `/tmp/claude-1000/-home-natepiano-rust-cargo-liner-berth-hook-hang/65ab15a1-f7b8-4ef7-b1d0-7d7b24f0df2d/scratchpad/repro`: hana's `.git` copied without its ledger, the ledger snapshot from `/home/natepiano/.claude/state/berth-fix/inbox/20261014T025825Z-hana/ledger-snapshot.tar.gz` extracted into it, trunk's files at `12375bcb1` with `.git` pointed at the copy, and `payload.json` naming session `b83cfc96-7997-43ed-b401-7525476cb633`. `scratchpad/time_hook.sh <repro dir> <binary> <limit seconds>` restores the snapshot and times one hook run. Before: the installed `~/.cargo/bin/cargo-berth` with a 600 s limit. After: `target/release/cargo-berth` built from this branch. Report both figures. Measured after the fast pass (2026-10-13, strace plus a perf profile of a frame-pointer build of `e82cd2d2`, 24.8 s wall, 17.5 s CPU): drift coverage 9.8 s (`protection_for_conflict` 8.6 s of it: path keys 3.1 s, `remaining_protection` 2.1 s, scope clones 2.0 s; the overlap step about 1 s), journal replay 3.7 s, `commit_target_reachability` 1.6 s, git subprocesses 5.4 s wall (the 84 merge-tree/diff pairs 1.4 s). Fix B removes the largest part. The showrunner set cutting the 20 s as the main goal; report the after figure and the next-largest part, and change nothing beyond Fix B in this phase.

**Files:**
- `crates/cargo-berth/src/scope/antichain.rs` — landed in `e82cd2d2` (`reduce`, `reduce_by_pairs`, equivalence and shape tests); reviewed in this phase, edited only by a repair.
- `crates/cargo-berth/src/reservation/retention.rs` — `DriftCoverageProbe`, `drift_coverage_probe`, the split of `conflicts_with_holders`, `blocking_coverage_for_drift` as a wrapper, and the shape test.
- `crates/cargo-berth/src/drift/classification.rs` — one probe per subject in `build` and `classify_locked`; `blocking_coverage` takes the probe.
- `crates/cargo-berth/src/reservation/mod.rs` — re-export `DriftCoverageProbe` if `drift` cannot name it otherwise.

**Seats:** 2 writers — Fix A has landed, so the work is Fix B, split by file: the probe in `reservation`, its use in `drift`. The probe's signatures above are fixed, so `test` codes against them while `impl` writes them.
- `impl` — `crates/cargo-berth/src/reservation/retention.rs` (the probe, the split of `conflicts_with_holders`, the wrapper, the shape test); hub: `crates/cargo-berth/src/reservation/mod.rs` (the re-export).
- `test` — opens as impl: `crates/cargo-berth/src/drift/classification.rs` (one probe per subject in `build` and `classify_locked`; both `blocking_coverage` functions take the probe).

**Constraints from prior phases:** none.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh check cargo-berth`, `bash ~/.claude/scripts/delegate/verify.sh test cargo-berth` and `bash ~/.claude/scripts/delegate/verify.sh lint cargo-berth` all green; `reduction_of_the_reported_shape_is_immediate`, the `reduce_by_pairs` equivalence test and `drift_coverage_of_the_reported_shape_ends_well_under_a_second` pass; every existing `blocking_coverage_for_drift` test passes unchanged; the unit director states the before and after hook times in the reproduction copy.

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
