# berth-parallel

> **Status: IMPLEMENTATION PLAN — phased, delegate-ready.** A phase reservation's merge extent and its integration are judged on that phase's own paths. A release after the merge finishes in one call, and the board's Overview shows reservation rows.

> **Production: remove-running-section** — unit `berth-parallel-unit`; production doc `docs/cargo-port/remove-running-section-production.md`

## Source

2026-10-16, from the showrunner, after this unit's C3 live check (two phase sessions in one scratch worktree, the installed cargo-berth):

`<sid>.phase-1` checked a.rs and `<sid>.phase-2` checked b.rs. Each got its own reservation, and each race extent held only its own path. But both merge extents listed a.rs and b.rs. Then a.rs was committed and merged into trunk, and phase 1 released as integrated. Next, phase 2 released as integrated by `protected_tip_ancestor` although b.rs was never committed. Its checkpoint had snapshotted the shared worktree HEAD, which phase 1's merge put into trunk. That release took three calls: `checkpointed`, then `evidence_revalidated` with the board still outstanding, then `released`.

The showrunner's ruling, under the standing approval for berth bugs (fix fast, install on both machines, robust tests after):

- **(1) + (3)** A phase's reservation must not release as integrated while its own paths hold uncommitted or unintegrated changes. Its merge extent is its own claimed paths, not the whole worktree, and its integration is judged on those paths alone.
- **(2)** A release after a merge records its disposition in one call. `evidence_revalidated` with nothing left to do finishes the release.
- **(4)** The TUI Overview shows reservation rows. A plain `board` with no TTY that prints a pointer to `--json` stays as it is.
- **(5), next:** `init` leaves an untracked `.claude/` in trunk.

## Findings before planning

Mapped 2026-10-16 at `834cffba` (main).

- **Merge extents are observed per worktree, not per reservation.** `derive_merge_extents` (`src/reconcile.rs` ~2282) keys holders by `(worktree, recorded target)` in `observe_holder_merge_extents` (~2399). It observes only the first unreleased reservation for each key, then writes that extent to every unreleased reservation with the same key through one `HolderMergeExtentObserved` (~2315–2359). `observe_merge_extent` (~2463) builds the scopes from `git::unmerged_branch_paths(root, trunk, head)` plus every dirty path from `drift::observe_merge_working_tree` (`git status --porcelain -uall` on the whole worktree). It never reads `reservation.scopes()`, so siblings in one worktree get the same whole-worktree extent.
- **Integration is judged on the tip alone.** `checkpoint_operation` (`src/verb/release.rs` ~484) snapshots the worktree's `HEAD` through `git::reservation_checkpoint_commits` (`src/git/reachability.rs` ~374). `reservation::integration_status` (`src/reservation/evidence.rs` ~131) returns `Integrated { ProtectedTipAncestor }` once that tip is an ancestor of trunk, without looking at the reservation's paths. Scopes matter only in the `scoped_patch_equivalence` fallback. `outstanding_operation` (`release.rs` ~514, comment at ~553: "integrated checkpoint work ends it whatever the holder committed or left dirty afterwards") then releases it as integrated through `integrated_release_operation` (~598).
- **Reconcile settles by the same evidence.** `settlement_selection` (`reconcile.rs` ~4279) maps `ProtectedTipAncestor` to integrated. It holds back only for `has_unproven_merge_work` (`src/reservation/retention.rs` ~102), which compares committed paths with the reservation's scopes, unless `later_work_cover` (~4422) finds a sibling. `append_merged_run_endings` (~4485) checkpoints and integrates an active run with an empty merge extent whose `HEAD` is in trunk.
- **The third call.** A `release` on an outstanding reservation returns `EvidenceRevalidated` without releasing in two ways. One is when the reconcile at the top of `execute` just planned evidence for that id (`release.rs` ~141–149). The other is `evidence_operation` (~590–594, ~843–862), which appends the integrated evidence and stops. Only the next call sees the stored integrated status and releases.
- **The Overview pane holds no reservation fields.** `src/board/tui.rs` ~63–84 splits the model into panes. `OVERVIEW_FIELDS` holds only journal position, recovered bypasses, integration order, targets and git cost. `unconstrained_reservations` and the other reservation sections are in `RESERVATION_FIELDS`, the separate Reservations pane. `event_loop` (~580–605) draws one model and never rebuilds it.
- **Test scaffolding exists.**
  - `tests/parallel_phases.rs` runs two phase sessions in one worktree. `a_phase_checkpoint_does_not_adopt_a_released_siblings_committed_paths` (~291) is closest, but it never merges into trunk. Its helpers are `open_phase`, `phase_board`, `initialized_repository` and `run_berth_with_session`.
  - `tests/lifecycle.rs` `stored_integrated_evidence_is_revalidated_before_release` (~708) has the checkpoint → merge → release pattern.
  - `cargo-berth-test-support` has `merge_by_commit`, `merge_fast_forward`, `commit_file` and `reservation_row`.

## Decisions made here

- **A reservation's merge extent is its own share of the worktree's changes.** The worktree is still observed once per `(worktree, target)`. Each unreleased reservation in it then gets the observed paths its own scopes cover. Observed paths that no unreleased reservation in that worktree covers go to every one of them, so unclaimed work stays protected. A lone reservation gets exactly today's extent. Risk: an unclaimed dirty file keeps every sibling's extent non-empty, and so outstanding, until someone claims, commits or reverts it. That is the conservative side.
- **Integration needs the reservation's own paths to be clean and in trunk.** The judgment stays in one function, which both `release` and reconcile settlement use. Integrated evidence stands only while the reservation's current scoped merge extent is empty, meaning none of its paths is dirty or committed outside trunk. Otherwise the reservation stays outstanding, and the release answer names those paths. This narrows the "whatever the holder left dirty afterwards" rule at `release.rs` ~553 to paths outside the reservation's scopes.
- **One call releases.** When a `release` revalidates evidence to integrated and nothing else stands in the way, the same call appends the release with disposition `integrated` and answers `released`. `evidence_revalidated` remains the answer only when the evidence changes without settling the reservation.
- **Overview shows a reservation table.** The Overview pane gets one row per non-terminal reservation with its id, session, stage, blocking status and race paths. It uses the rows `board_reservation_snapshots` already builds, and the Reservations pane keeps its detail. The board staying a one-time model is out of scope and goes to next items.
- **Robust tests go in their own phase.** Phase 9 lands the fix with the tests that prove (1) and (2). After it is installed, phase 11 widens the coverage. The showrunner's two phases become three only by splitting off that standing "robust tests after" step.
- **Install from this branch.** After phase 9 checkpoints, install cargo-berth on natedev from this worktree. Install it on the Mac from the pushed branch over `ssh mac`, then tell the showrunner.

## Delegation Context

- **Project:** `cargo-berth`, the worktree reservation ledger and hook engine.
- **Project started:** 2026-10-16T18:20:00.000+00:00
- **Worktree:** `/home/natepiano/rust/cargo-liner-berth-parallel`, branch `remove-running-section-berth-parallel`. Every seat works only here, and every Work Order names it.
- **Base commit:** `f3fbdef3` (main, merged in at phase 11's start as `5ad2908c`).
- **Test lanes:** `crates/cargo-berth/tests/`.
- **Stack:** Rust workspace, cargo-nextest. The crate has no lib target, so unit tests live in `cargo-berth::bin/cargo-berth` (`--bins`) and integration tests in `crates/cargo-berth/tests/`.
- **Key files:**
  - `crates/cargo-berth/src/reconcile.rs` — `derive_merge_extents`, `observe_holder_merge_extents`, `observe_merge_extent`, `settlement_selection`, `append_merged_run_endings`.
  - `crates/cargo-berth/src/reservation/evidence.rs` — `integration_status`, `outstanding_integration_status`, `observe_integration_status`.
  - `crates/cargo-berth/src/reservation/retention.rs` — `has_unproven_merge_work`.
  - `crates/cargo-berth/src/verb/release.rs` — `execute`, `checkpoint_operation`, `outstanding_operation`, `evidence_operation`, `integrated_release_operation`.
  - `crates/cargo-berth/src/board/tui.rs`, `src/board/rows.rs` — panes and reservation rows.
  - `crates/cargo-berth/tests/parallel_phases.rs`, `tests/lifecycle.rs`, `tests/board.rs`.
  - `docs/cargo-berth/as-built/phase-reservations.md`, `worktree-coordination.md`.
  - `crates/cargo-berth/CHANGELOG.md` — `## [Unreleased]`.
- **Build:** `bash ~/.claude/scripts/delegate/verify.sh check cargo-berth`
- **Test:** `bash ~/.claude/scripts/delegate/verify.sh test cargo-berth`
- **Lint:** `bash ~/.claude/scripts/delegate/verify.sh lint cargo-berth`
- **Style:** run-end /clippy style-only auto-proceed
- **Invariants:**
  - Journal replay of existing ledgers is unchanged. New behavior comes from how live observations and release decisions are made, not from reinterpreting stored records.
  - A worktree holding one unreleased reservation behaves exactly as today.
  - Invalid states are unrepresentable. Integration that requires clean own paths is a type the judgment returns, not a flag checked by callers.
  - No test takes a second (memory `no-multi-second-tests`). Format with `cargo +nightly fmt` only. Run one `verify.sh` at a time.

## Phases

### Phase 9 — A phase is judged on its own paths  · status: done

**Depends on:** none

#### As-built

- Each unreleased reservation in a `(worktree, target)` group gets its own share of the one worktree observation (`reservation_merge_extent`, `holder_group` in `src/reconcile.rs`): the observed paths its scopes cover, plus every observed path no sibling covers. A lone reservation keeps the whole-worktree extent. The cache key carries `HolderMergeExtentCoverage` (`FullHolder` or `ReservationShares` with sorted ids, `src/reservation/merge_extent.rs`), so a sibling joining or leaving invalidates cached extents.
- `reservation::judge_integration` (`src/reservation/evidence.rs`) returns `ReservationIntegrationJudgment`: `Integrated`, `Outstanding { pending_paths, evidence: PendingIntegrationEvidence }` (`Unproven`, `TrunkRewritten`, `ObjectUnknown`), `NotIntegrated`, `TrunkRewritten`, `ObjectUnknown`. Release and reconcile settlement (`judge_live_integration`) both use it. It replaced `retention.rs::has_unproven_merge_work`.
- An integrated release appends `EvidenceRevalidated` and then `Release` in one locked transaction (`CommittedActionValidation::AppendWithPrerequisites`, `src/ledger/handle.rs`), so a release after the merge answers `released` in one call.
- `ReleasePayload::Outstanding` and `Resnapshotted` carry `pending_paths`, and the message names them. `docs/cargo-berth/generated/output-contract.json` regenerated. CHANGELOG `Fixed` line.

**Files:**
- `crates/cargo-berth/src/reconcile.rs` — per-reservation share, cache coverage, settlement through the judgment
- `crates/cargo-berth/src/reservation/evidence.rs` — the judgment type and `judge_integration`
- `crates/cargo-berth/src/reservation/merge_extent.rs` — `HolderMergeExtentCoverage`
- `crates/cargo-berth/src/ledger/handle.rs` — `AppendWithPrerequisites`
- `crates/cargo-berth/src/verb/release.rs`, `crates/cargo-berth/src/output.rs` — one-call release, pending paths in answers
- `crates/cargo-berth/tests/parallel_phases.rs` — `parallel_phases_release_in_one_call_after_their_own_work_is_clean_and_merged`, the abandon/retarget unclaimed-path tests
- `crates/cargo-berth/tests/board.rs` — trunk-rewritten, objects-unknown and budget-deferred release tests

**Gotchas:**
- Only a fresh `MergeExtent::Protected` names pending paths. An unobserved or unavailable extent answers through `PendingIntegrationEvidence` and never reports a clean phase.
- The first, checkpointing release of a dirty phase does not name its pending paths; phase 11 fixes it.

### Phase 10 — Overview shows reservations  · status: done

**Depends on:** none

#### As-built

- The board TUI's Overview pane opens with a Reservations table: ID, Session, Stage, Blocking, Race paths, one row per non-terminal reservation, or a "no reservations" row. The Reservations pane, `--json` and the no-TTY pointer are unchanged.
- `BoardReservationSession` (`src/board/rows.rs`) is `Mapped { primary, aliases }`, `RunOnly(CoordinationRunId)` or `Unavailable`. `board_reservation_sessions` reads `session::phase_reservation_sessions` once per run per coordination run. `overview_short_id` keeps the last `-` group plus any `.phase-N` suffix, for both the ID and Session columns; the fallback shows `run <short>`.

**Files:**
- `crates/cargo-berth/src/board/rows.rs` — session labels for reservation rows
- `crates/cargo-berth/src/board/tui.rs` — the Overview table; tests `overview_distinguishes_sibling_phase_sessions_in_one_run`, `overview_phase_session_and_alias_leave_race_paths_visible_at_100_columns` at 100 columns

**Gotchas:**
- A board deserialized from JSON carries no session identity and shows `unavailable`.
- The raw `git_cost`/`journal_position` JSON still sits below the table; the TUI model rebuild next item removes it.

### Phase 11 — The checkpoint names pending paths, and robust tests  · status: todo

**Depends on:** Phase 9

#### Work Order

**Goal:** The first release of a phase, the one that checkpoints it, names the phase's own pending paths. Tests pin every edge of phase 9's per-path judgment.

**Spec:**
- **The checkpoint names pending paths** (showrunner C19, 2026-10-16). A live check found a gap. Take a phase whose own path b.rs is dirty. Its first `release` checkpoints and answers envelope `outstanding`, payload `checkpointed`, with the message "Reservation … is outstanding at protected tip …". Neither the payload nor the message names b.rs. Only the next `release` answers `outstanding` with `pending_paths: ["b.rs"]`. The fix:
  - `ReleasePayload::Checkpointed` (`src/output.rs` ~1540) carries `pending_paths: Vec<ReservationScopePath>`, like `Outstanding` and `Resnapshotted`.
  - `checkpoint_operation` (`src/verb/release.rs` ~508) fills it from the reservation's current scoped merge extent (`release_repository_context.merge_extent`). Phase 9's rule applies: only a fresh `MergeExtent::Protected` names pending paths. An unobserved or unavailable extent must not report an empty list as if the phase were clean. Give that case its own variant under the Type Design Contract; never use an empty `Vec` to stand for it.
  - The message (`output.rs` ~2943) ends "; pending paths: b.rs." whenever the list is non-empty. That covers own paths that are dirty and own paths committed but not yet in trunk.
  - Regenerate `docs/cargo-berth/generated/output-contract.json` the way phase 9 did. The output-contract tests in `src/output_contract.rs`, `tests/board.rs` ~392 and `tests/overlap.rs` ~1123 must pass.
  - CHANGELOG `## [Unreleased]` `### Fixed`: the checkpointing release now names the phase's pending paths.
- **Test for the fix** (`tests/parallel_phases.rs`): two phases in one unit worktree, as in `parallel_phases_release_in_one_call_after_their_own_work_is_clean_and_merged`. Phase 2's b.rs is dirty. Its first `release` answers payload `checkpointed` with `pending_paths: ["b.rs"]`, and the message names b.rs. A phase whose own a.rs is committed but not merged also names a.rs at checkpoint.
- **Robust tests.** Before writing any of these, check the tests that already exist (see Constraints) and do not duplicate them. Cover:
  - an unclaimed dirty file, c.rs, goes to both live siblings' extents;
  - a lone reservation's extent equals the whole-worktree extent;
  - reconcile settlement (`settlement_selection` `src/reconcile.rs` ~4338, `append_merged_run_endings` ~4467) refuses integrated while own paths are dirty or committed outside trunk;
  - patch-equivalent integration of own paths;
  - `evidence_revalidated` is still answered when evidence changes without settling;
  - the hook and board answers for an outstanding phase name its pending paths.
- Each test runs under a second.

**Files:**
- `crates/cargo-berth/src/output.rs` — the `Checkpointed` pending paths and message
- `crates/cargo-berth/src/verb/release.rs` — `checkpoint_operation` fills them
- `docs/cargo-berth/generated/output-contract.json` — regenerated
- `crates/cargo-berth/CHANGELOG.md` — the `Fixed` line
- `crates/cargo-berth/tests/parallel_phases.rs`, `tests/lifecycle.rs`, `tests/hooks.rs`, `tests/board.rs` — new tests, plus expectations the `Checkpointed` change rightly moves

**Workers:** 1 writer + 1 tester
- `impl` owns every `src/` file, the contract JSON, `CHANGELOG.md`, `tests/parallel_phases.rs` and `tests/lifecycle.rs`.
- `test` owns `tests/hooks.rs` and `tests/board.rs`.

**Constraints from prior phases:**
- Phase 9 deleted `retention.rs::has_unproven_merge_work`. `reservation::judge_integration` (`src/reservation/evidence.rs` ~194) is now the one judgment. It returns `ReservationIntegrationJudgment` with the variants Integrated, `Outstanding{pending_paths, evidence: PendingIntegrationEvidence}`, NotIntegrated, TrunkRewritten and ObjectUnknown. Reconcile calls it through `judge_live_integration` (`src/reconcile.rs` ~2772).
- The cache key carries `HolderMergeExtentCoverage` (FullHolder or ReservationShares), so a sibling joining or leaving invalidates cached extents.
- An integrated release appends `EvidenceRevalidated` and then `Release` in one locked transaction (`CommittedActionValidation::AppendWithPrerequisites`, `src/ledger/handle.rs`).
- These tests already exist; do not repeat them:
  - `tests/parallel_phases.rs`: `abandoning_`/`retargeting_a_sibling_restores_its_unclaimed_paths_to_the_remaining_extent` and `parallel_phases_release_in_one_call_after_their_own_work_is_clean_and_merged`;
  - `tests/board.rs`: `release_names_pending_paths_when_trunk_was_rewritten`, `release_names_pending_paths_when_integration_objects_are_unknown` and `release_records_a_budget_deferred_proof_before_its_integrated_disposition`;
  - `tests/lifecycle.rs`: `release_keeps_unclaimed_later_branch_work_outstanding` and `rebased_integrated_checkpoint_retains_unclaimed_dirt`.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh test cargo-berth` green.

## Next items

- `init` leaves an untracked `.claude/` in trunk (showrunner, `next`).
- The TUI board draws one model and never rebuilds it, so reservations created after it opens never appear. With the rebuild, take the raw `git_cost` and `journal_position` JSON out of the Overview, below its Reservations table: diagnostics are not Overview content (showrunner, from the phase 10 shot).
- `board --reservation <id> --json` carries no ordering edges or overlap answers; add the reservation's edges and answers to it (showrunner, `next`, from C5).
