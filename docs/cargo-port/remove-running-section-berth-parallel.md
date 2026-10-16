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
- **Base commit:** `834cffba` (main, merged in at phase 9's start).
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

#### Work Order

**Goal:** Two phase reservations in one worktree each get a merge extent of their own paths. Neither releases as integrated while its own paths hold uncommitted or unintegrated changes, and a release after the merge finishes in one call.

**Spec:**
- **Scoped merge extent** (`src/reconcile.rs`). Keep one worktree observation per `(worktree, target)` in `observe_holder_merge_extents` (~2399), with its concurrency and git cost unchanged. Replace the single `HolderMergeExtentObserved` fan-out (~2315–2359), which writes one extent to every unreleased reservation of that key, with a per-reservation share:
  - each unreleased reservation gets the observed paths its own `scopes()` cover;
  - observed paths no unreleased reservation of that key covers go to every one of them;
  - a key with one unreleased reservation gets exactly today's extent.
  Write the journal records this needs so replay of existing ledgers is unchanged. If one record per reservation is needed, keep the old record readable.
- **Integration judged on own paths** (`src/reservation/evidence.rs` `integration_status` ~131, `outstanding_integration_status` ~168, `observe_integration_status` ~195). Integration needs the reservation's current scoped merge extent to be empty: none of its own paths dirty or committed outside trunk. `ProtectedTipAncestor` and `ScopedPatchEquivalence` stay the proofs of the committed work. Return a type that cannot say integrated while own paths are pending; for example `Outstanding { pending_paths }` beside `Integrated { proof }`. Callers must not check a flag. Both `release` (`src/verb/release.rs` `outstanding_operation` ~514: narrow the "whatever the holder committed or left dirty afterwards" rule at ~553 to paths outside the reservation's scopes) and reconcile settlement (`settlement_selection` ~4279, `append_merged_run_endings` ~4485, `has_unproven_merge_work` in `src/reservation/retention.rs` ~102) use that one judgment. A reservation held back answers outstanding and names its pending paths.
- **One-call release** (`src/verb/release.rs` `execute` ~141–149, `evidence_operation` ~590–594 and ~843–862). When a `release` revalidates the evidence to integrated and nothing else holds the reservation, the same call appends `Release { disposition: Integrated }` and answers `released`. `evidence_revalidated` stays the answer only when the evidence changes without settling the reservation.
- **The proving test** (`tests/parallel_phases.rs`, reuse `open_phase`, `phase_board`, `initialized_repository`, `run_berth_with_session`; the merge pattern is `tests/lifecycle.rs` `stored_integrated_evidence_is_revalidated_before_release` ~708). Use sessions `dir-sid.phase-1` on `file:a.rs` and `dir-sid.phase-2` on `file:b.rs` in one unit worktree on a branch, with b.rs dirty. Assert:
  1. Before any release, each merge extent names only its own path.
  2. Commit a.rs and `release` phase 1 (checkpointed). Merge the branch into trunk. Then one `release` of phase 1 answers `released`, disposition `integrated`.
  3. `release` of phase 2 (checkpoint, then again after the merge) never answers integrated while b.rs is uncommitted. It stays outstanding and names b.rs.
  4. Commit b.rs and merge into trunk. Then one `release` of phase 2 answers `released`, integrated.
  Name tests for the behavior they pin. Each runs under a second.
- CHANGELOG `## [Unreleased]` `### Fixed`: a phase reservation is no longer released as integrated while its own paths hold uncommitted or unmerged work, and a release after the merge finishes in one call.

**Files:**
- `crates/cargo-berth/src/reconcile.rs` — per-reservation merge extent share; settlement through the shared judgment
- `crates/cargo-berth/src/reservation/evidence.rs` — integration judged on own paths, as a type
- `crates/cargo-berth/src/reservation/retention.rs` — `has_unproven_merge_work` follows the shared judgment
- `crates/cargo-berth/src/verb/release.rs` — outstanding rule narrowed; one-call release
- `crates/cargo-berth/src/` — any journal or record type the scoped extent needs
- `crates/cargo-berth/tests/parallel_phases.rs` — the C3 proving test
- `crates/cargo-berth/tests/` — in existing tests, update only expectations this change rightly moves (a third-call `evidence_revalidated` becomes `released`)
- `crates/cargo-berth/CHANGELOG.md` — `Fixed` line

**Workers:** 1 writer + 1 tester
- `impl` — every `src/` file and `CHANGELOG.md`; existing-test expectation updates in `tests/lifecycle.rs` and others
- `test` — the C3 proving test in `tests/parallel_phases.rs`, written from the Spec alone

**Constraints from prior phases:**
- A phase's own berth session is `<session>.phase-<N>`; each phase holds its own reservation (as-built `docs/cargo-berth/as-built/phase-reservations.md`).
- `derive_merge_extents` observes holders concurrently on scoped threads and folds in first-seen order (phase 6); keep that.
- A busy ledger in `check` answers `contention`, exit 6 (phase 8).

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh test cargo-berth` green, including the C3 proving test.

### Phase 10 — Overview shows reservations  · status: done

**Depends on:** none

#### Work Order

**Goal:** The board TUI's Overview pane lists every non-terminal reservation.

**Spec:**
- `src/board/tui.rs` (~63–84) splits the model into panes. `OVERVIEW_FIELDS` holds journal position, recovered bypasses, integration order, targets and git cost. The reservation sections (`unconstrained_reservations` and the others) are only in `RESERVATION_FIELDS`. Add one reservation table to Overview, with one row per non-terminal reservation: short id, session, stage, blocking status and race paths. Build it from the rows `board_reservation_snapshots` (`src/board/rows.rs` ~879) already produces. Show an empty table with a one-line "no reservations" row when there are none.
- The Reservations pane, `--json` output and the no-TTY pointer to `--json` stay unchanged.
- Test: render Overview for a board with an active, unconstrained reservation, and assert its row (id and path) appears. Use the TUI's existing test rendering (ratatui `TestBackend` or whatever `tui.rs` tests use). Under a second.
- No CHANGELOG edit here: phase 9 owns `CHANGELOG.md` while both run, and the as-built pass adds this phase's line.

**Files:**
- `crates/cargo-berth/src/board/tui.rs` — Overview reservation table and its test
- `crates/cargo-berth/src/board/rows.rs` — only if a row accessor is needed

**Workers:** 1 writer; nothing splits (one pane, one file)
- `impl` — every file

**Constraints from prior phases:** none.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh test cargo-berth` green, including the Overview test.

### Phase 11 — Robust tests  · status: todo

**Depends on:** Phase 9

#### Work Order

**Goal:** Tests pin every edge of phase 9's per-path judgment; behavior unchanged.

**Spec:** Tests only. Cover:
- an unclaimed dirty file goes to every sibling's extent;
- a lone reservation's extent equals today's;
- reconcile settlement (`settlement_selection`, `append_merged_run_endings`) refuses integrated while own paths are dirty or committed outside trunk;
- patch-equivalent integration of own paths;
- `evidence_revalidated` is still answered when evidence changes without settling;
- the hook and board answers for an outstanding phase name its pending paths.
Each test under a second.

**Files:**
- `crates/cargo-berth/tests/parallel_phases.rs`, `tests/lifecycle.rs`, `tests/hooks.rs`, `tests/board.rs` — new tests

**Workers:** 1 writer + 1 tester
- `impl` — `tests/parallel_phases.rs`, `tests/lifecycle.rs`
- `test` — `tests/hooks.rs`, `tests/board.rs`

**Constraints from prior phases:** phase 9's as-built.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh test cargo-berth` green.

## Next items

- `init` leaves an untracked `.claude/` in trunk (showrunner, `next`).
- The TUI board draws one model and never rebuilds it, so reservations created after it opens never appear.
- `board --reservation <id> --json` carries no ordering edges or overlap answers; add the reservation's edges and answers to it (showrunner, `next`, from C5).
