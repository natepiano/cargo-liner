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

#### As-built

- `KeyedConflictProtection` (in `reservation/retention.rs`) indexes each holder's held scopes once, at probe build, by case-folded component keys: a file-key map, a tree-key map, and a `BTreeMap` of keys for descendant lookup. `DriftCoverageProbe::coverage` finds overlaps from those lookups — a held tree on the candidate's key or an ancestor prefix, a held file on the exact key, and for a candidate tree every held key under it — and builds each `ReservationConflict` through `conflict_for_overlap`, the constructor the scan path also uses: same holders, same order, same `overlapping_scopes`. `conflicts_with_holders` and its other callers keep the scan.
- `ReservationScope::component_keys(&self, path_case: PathCase)` in `scope/mod.rs` is the one keying of a scope; `scope::antichain::reduce` and the probe both use it, and `PathCase::component_key` stays `pub(super)`.
- `CommitAncestryGraph::first_parent_intervals_after` returns every candidate's first-parent interval from one walk of the target's chain, using `first_parent_ancestry_boundary`, a memoized search that finds each candidate's first ancestor on the chain without building a full ancestor set per candidate. Each interval equals what `first_parent_commits_after` returns, including when the walk reaches a root. `classify_candidates_against_target` uses it; `first_parent_commits_after` stays for `earliest_containing_matches`.
- Hook in the matched-checkpoint copy: 5.7 s → 2.69 s, same 234,414-byte answer. Remaining split: git subprocesses 1.11 s of wall time (84 `merge-tree`, 84 `diff`, `update-ref`, `rev-list`, `status`); CPU samples cargo-berth 54 %, git 46 %; the largest cargo-berth item is the incursion check's linear find of each incident's reservation by id (about 19 %), and allocation about 15 %.

**Files:**
- `crates/cargo-berth/src/reservation/retention.rs` — `KeyedConflictProtection`, `conflict_for_overlap`, keyed `coverage`, equivalence test against the scan, 13-holder timing test.
- `crates/cargo-berth/src/scope/mod.rs`, `crates/cargo-berth/src/scope/antichain.rs` — `ReservationScope::component_keys` over the antichain's `pub(super)` `component_keys`.
- `crates/cargo-berth/src/git/reachability.rs` — `first_parent_intervals_after`, `first_parent_ancestry_boundary`, equivalence test against per-candidate walks, large merged-history timing test.

**Binds later work:** the hook baseline is 2.69 s in the matched-checkpoint copy; the incursion check's id find is the next cost.

**Ruled out:** a phase of its own for allocation; allocation is fixed only where it sits on the incursion path.

### Phase 3 — The incursion check finds each reservation by id lookup  · status: done
**Depends on:** Phase 2

#### As-built

- `RetainedReservationSet::incursion_incidents_of_coordination_identity` decides membership with one `HashSet<ReservationId>` of the identity's reservations, collected in one pass over the reservations; an incident charged to an unknown id stays excluded, and retained order is kept. Signature unchanged, so `recovery.rs` and `board/alerts.rs` are untouched.
- `observe_incursion` collects the identity's incidents once and passes `&[&IncursionIncident]` to `incursion_path_coverage`, now an associated fn that keeps its holder-containment rule and Outstanding, then Answered, then Uncovered answer. Blocked paths are borrowed and cloned only into the returned set. `RetainedReservationSet::reservation` and its other callers are unchanged.
- Tests: `incursion_identity_lookup_matches_the_reservation_scan` (ordered incidents equal a linear-scan reference across same run/same worktree, same run/other worktree, other run/same worktree, and an unknown id) and `incursion_observation_of_hundreds_of_incidents_is_under_250_ms` (600 reservations, 600 incidents, 64 paths).
- Hook in the matched-checkpoint copy: same 234,414-byte answer; wall 3.21 s measured at load average 34–64, not comparable to the 2.69 s quiet baseline. Profile: the linear find (about 19 % of samples) is gone and incursion is about 1.2 %; CPU samples git 56 %, cargo-berth 44 %; the largest cargo-berth item is allocation, about 16 %. Git under strace and load: 184 runs — 84 `merge-tree`, 84 `diff`, one ancestry `git log` (`GIT_LOG_COMMAND` in `git/constants.rs`, existing code), 6 `rev-list`, 2 `update-ref`, 2 `status`.

**Files:**
- `crates/cargo-berth/src/reservation/retention.rs` — id-set incursion membership, single-collection observation, associated `incursion_path_coverage`, both tests.

**Binds later work:** the quiet hook re-time against the 2.69 s baseline, with the cost split, is owed in the final acceptance gate.

**Gotchas:** journal replay rejects an unknown reservation charge, so the unknown-id equivalence case injects an incident into the retained vector directly.

### Phase 4 — The slow tests run well under a second  · status: todo
**Depends on:** Phase 3

#### Work Order

**Goal:** each test in the `lifecycle`, `board`, `edges`, `liveness`, `gate` and `compaction` binaries that takes a second or more on a quiet machine runs well under a second and proves what it proves today, or its floor is named and measured where real product work sets it.

**Spec:**

The showrunner (cargo-liner, 2026-10-14): "The user's standing rule is that a test taking seconds is a defect to fix … Aim for each well under a second without weakening what it proves. Where a floor is set by real product work (for example, git operations the test must perform), say so with the measured floor rather than forcing it. Measure each test before and after on a quiet machine, and list them in the checkpoint." And: "include the 17 at 1.0–1.5 s as targets too. The bar is each test under a second … If that makes phase 4 too large, split it into 4 and 5 by fixture family."

*Targets* (quiet full-suite run of the phase 2 tree, total 8.07 s). In `cargo-berth::lifecycle` (all but the two module-qualified ones run a scenario in `tests/support/reanchoring.rs` through a one-line wrapper):

| Test | Quiet time |
| --- | --- |
| `checkpoint_ranges_survive_two_rebases_reconciled_before_fast_forward` | 2.17 s |
| `checkpoint_ranges_survive_two_rebases_reconciled_during_integration` | 2.11 s |
| `split_checkpoint_survives_stacked_branches_rewritten_with_update_refs` | 1.83 s |
| `upper_checkpoint_keeps_the_full_split_interval_when_update_refs_writes_lower_first` | 1.77 s |
| `upper_checkpoint_keeps_the_full_split_interval_when_update_refs_writes_upper_first` | 1.74 s |
| `split_checkpoint_survives_an_active_phases_rewritten_retention_ref` | 1.59 s |
| `interactive_split_keeps_both_protected_hunks_in_the_active_phase` | 1.34 s |
| `interactive_split_keeps_both_protected_hunks_in_the_checkpoint` | 1.29 s |
| `integration_cover::cover_ends_after_integration_lands_by_merge_or_fast_forward` | 1.26 s |
| `split_checkpoint_survives_switching_the_issuing_checkout_before_board` | 1.18 s |
| `merge_extent::lanes_release_when_their_integration_target_receives_a_merge_commit_or_fast_forward` | 1.10 s |
| `soft_reset_split_keeps_both_protected_hunks_in_the_active_phase` | 1.05 s |
| `soft_reset_split_keeps_both_protected_hunks_in_the_checkpoint` | 1.02 s |
| `refused_reanchor_preserves_live_and_orphaned_checkpoint_anchors` | 1.01 s |

In the other binaries:

| Test | Binary | Quiet time |
| --- | --- | --- |
| `cold_proof_subjects_bound_git_evaluation_for_distinct_and_duplicate_reservations` | board | 2.36 s |
| `an_existing_reservation_covers_only_when_it_targets_the_integration_parent` | board | 1.58 s |
| `every_reachable_edge_readiness_has_its_own_action_or_settlement` | board | 1.35 s |
| `reachability_integrates_every_outstanding_subject_without_scoped_comparisons` | board | 1.32 s |
| `rewritten_and_unknown_predecessor_evidence_have_distinct_recoveries` | board | 1.15 s |
| `proof_subject_changes_force_rechecks_at_an_unchanged_target` | board | 1.15 s |
| `targets_include_the_repository_trunk_and_only_non_released_reservations` | board | 1.06 s |
| `unconstrained_holders_show_all_liveness_states_and_distinct_history_failures` | board | 1.02 s |
| `successor_round_robin_covers_every_head` | edges | 1.62 s |
| `predecessor_graph_has_fixed_cold_cost` | edges | 1.04 s |
| `checkpoint_readiness_covers_incorporation_and_missing_objects` | edges | 1.03 s |
| `recovery_dispositions_validate_evidence_and_remain_distinct_after_replay` | liveness | 1.57 s |
| `hook_outer_gate_deadline_expires_while_worker_waits_on_the_mutation_lock` | gate | 1.08 s |
| `compaction_changes_no_reader_output_and_a_second_run_finds_nothing` | compaction | 1.03 s |

*Method.* For each target, find where its time goes before changing it: count the `cargo-berth` process launches and git subprocesses it runs, and time fixture setup apart from the behavior under test. Cut cost the assertions do not need: repeated repository setup (build once, or start from `RepositoryTemplate`/`IntegrationRepository` in `cargo-berth-test-support`), repeated full `cargo-berth` launches where one launch answers several assertions, git operations that rebuild state already present, and waits on time. Every assertion stays, and so does every scenario step the test's name promises (two rebases, both `update-refs` write orders, interactive and soft-reset splits, merge and fast-forward landings).

*Bounds and deadlines.* A test whose name asserts a cost bound (`predecessor_graph_has_fixed_cold_cost`, `cold_proof_subjects_bound_git_evaluation_…`) keeps its bound. The deadline test (`hook_outer_gate_deadline_expires_…`) may shorten its deadline only through the existing test-facing control, `GATE_DEADLINE` in `tests/gate.rs`, which reaches the binary as `CARGO_BERTH_TEST_GATE_DEADLINE_MS` (`GATE_DEADLINE_ENVIRONMENT`); production timing (`TOTAL_GATE_DEADLINE` in `src/cli.rs`) never changes.

*Floors.* Where the remaining time is product work the test must perform (a fixed number of git operations or `cargo-berth` invocations the scenario needs), stop and record the measured floor and what sets it in the seat's summary instead of weakening the test.

*Measurement.* Each seat records each target's time before and after from `verify.sh test cargo-berth --no-cache --filter <name>` output: `--no-cache` because a cached `PASS (recorded)` runs no test. Record the load average (`uptime`) beside each timing; a timing taken under heavy load is not a before or after. `RepositoryTemplate` rebuilds its cache when either executable changes, so time a target's first run after a build apart from a run that reuses the template, and report the reuse time. The unit director re-measures every target on a quiet machine at the checkpoint and lists them there.

**Files:**
- `crates/cargo-berth/tests/lifecycle.rs` — the lifecycle targets and their wrappers.
- `crates/cargo-berth/tests/support/split_rebase.rs`, `crates/cargo-berth/tests/support/reanchoring.rs` — the split-rebase and reanchoring scenarios the targets run.
- `crates/cargo-berth/tests/board.rs` — the board targets.
- `crates/cargo-berth/tests/edges.rs`, `crates/cargo-berth/tests/liveness.rs`, `crates/cargo-berth/tests/gate.rs`, `crates/cargo-berth/tests/compaction.rs` — the other targets.
- `crates/cargo-berth-test-support/src/lib.rs`, `crates/cargo-berth-test-support/src/integration_repository.rs`, `crates/cargo-berth-test-support/src/repository_template.rs` — shared fixtures, only where a target's setup cost lives there.

**Seats:** 2 writers
- `impl` — the 14 lifecycle targets: `crates/cargo-berth/tests/lifecycle.rs`, `crates/cargo-berth/tests/support/reanchoring.rs`, `crates/cargo-berth/tests/support/split_rebase.rs`, and the hub `crates/cargo-berth-test-support/src/` (sole owner; `test` messages it for a fixture change).
- `test` — opens as impl, the 14 other targets: `crates/cargo-berth/tests/board.rs`, `crates/cargo-berth/tests/edges.rs`, `crates/cargo-berth/tests/liveness.rs`, `crates/cargo-berth/tests/gate.rs`, `crates/cargo-berth/tests/compaction.rs`.

**Constraints from prior phases:** production code is not changed by this phase; the incursion id lookup, `KeyedConflictProtection`, `DriftCoverageProbe` and `first_parent_intervals_after` stay as landed.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh lint cargo-berth` then `bash ~/.claude/scripts/delegate/verify.sh test cargo-berth` green; every target under one second on a quiet machine or carrying a named, measured floor; the unit director lists every target's before and after time in the checkpoint notice. The unit director, not a seat, also times the post-tool-use hook on a quiet machine against the matched-checkpoint reproduction, confirms the same answer, and reports the time against the 2.69 s baseline with the remaining cost split, git wall time included.
