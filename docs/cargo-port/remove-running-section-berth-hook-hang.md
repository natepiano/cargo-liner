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

### Phase 2 — The hook finds overlaps by key lookup and walks each target's history once  · status: done
**Depends on:** Phase 1

#### Work Order

**Goal:** the post-tool-use hook in the matched-checkpoint copy runs well under its 5.7 s baseline, with the same 234,414-byte answer.

**Spec:**

The showrunner (cargo-liner, 2026-10-14): "retarget Phase 2 as you recommend, with both cuts. 5.7 s on every tool call is still too slow to stop at. Measure from the matched-checkpoint copy from now on and state that baseline (5.7 s) in the plan. After the phase, report the hook time and the remaining split, git wall time included."

*Baseline.* The replay checkpoint is keyed to the journal file's device and inode, so a restored snapshot replays the whole 346 MB journal; a real ledger replays only the tail past its checkpoint. With the checkpoint matched to the restored journal the hook runs 5.7 s. A perf profile of that run (inclusive): target reachability about 30 %, the drift overlap step about 23 %; journal replay is no longer a top item.

*Cut 1 — overlap by key lookup (`reservation/retention.rs`).* `DriftCoverageProbe::coverage` runs `conflicts_overlapping_holders`, which scans every held scope of every holder for each changed path and calls `ReservationScope::overlaps`/`contains`, allocating strings per comparison. When the probe is built, index each holder's held scopes by case-folded component keys (`ReservationScope::component_keys`, shared with `scope::antichain::reduce`): a file key set, a tree key set, and for each held tree its key. A candidate scope overlaps a held scope exactly when one contains the other: a held tree on the candidate's key or any ancestor prefix of it, a held file on the candidate's exact key, or, for a candidate tree, any held scope whose key starts with the candidate's key. `coverage` answers from lookups and builds each `ReservationConflict` exactly as `conflicts_overlapping_holders` does today: same holders, same order, same `overlapping_scopes` (the contained side of each overlap, then `minimal_antichain`). `conflicts_with_holders` and its other callers keep the scan. Keep the scan as the reference in a test that asks both the index and `conflicts_overlapping_holders` the same candidates (files and trees, nested, case-only differences, both `PathCase` values) and asserts equal answers.

*Cut 2 — one ancestry walk per target (`git/reachability.rs`).* `classify_candidates_against_target` calls `CommitAncestryGraph::first_parent_commits_after(target, candidate)` once per resolved candidate, and that calls `ancestors_including(candidate)`, building a `HashSet<GitObjectId>` of the candidate's entire history every time. Compute every candidate's interval from one walk: walk the target's first-parent chain once, then find, for each candidate, the first chain commit that is an ancestor of the candidate (inclusive), without building a full ancestor set per candidate (for example, one marking pass from all candidates that stops at chain commits, or deduplicating candidates and memoizing). Each interval is the chain prefix before that commit, the exact `Vec<GitObjectId>` `first_parent_commits_after` returns today, including when the walk reaches a root. `first_parent_commits_after` keeps its one other caller, `CommitAncestryGraph::earliest_containing_matches`, and serves as the reference in a test of both on graphs with merges, shared ancestry, repeated candidates, candidates on and off the first-parent chain, and a chain that reaches a root.

Each cut gets a timing test of the reported size class (hundreds of holders' scopes; a history of thousands of commits with dozens of candidates) that ends in well under a second.

**Files:**
- `crates/cargo-berth/src/reservation/retention.rs` — the probe's key index, `DriftCoverageProbe::coverage`, the equivalence and timing tests.
- `crates/cargo-berth/src/scope/mod.rs`, `crates/cargo-berth/src/scope/antichain.rs` — `ReservationScope::component_keys`, the one keying of a scope that `reduce` and the probe share.
- `crates/cargo-berth/src/git/reachability.rs` — `classify_candidates_against_target`, `CommitAncestryGraph::first_parent_commits_after`, `ancestors_including`, the equivalence and timing tests.

**Seats:** 2 writers
- `impl` — `crates/cargo-berth/src/reservation/retention.rs`; `crates/cargo-berth/src/scope/mod.rs` and `scope/antichain.rs` (only `ReservationScope::component_keys`).
- `test` — opens as impl: `crates/cargo-berth/src/git/reachability.rs`.

**Constraints from prior phases:** the keyed reduction and `DriftCoverageProbe` stay as landed; a probe is built per subject per pass and never shared between the pre-lock and locked passes.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh check cargo-berth`, `bash ~/.claude/scripts/delegate/verify.sh test cargo-berth` and `bash ~/.claude/scripts/delegate/verify.sh lint cargo-berth` all green; both equivalence tests and both timing tests pass; the unit director states the hook time in the matched-checkpoint copy against the 5.7 s baseline and the remaining split, git wall time included.
