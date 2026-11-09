# berth: open phases

> **Status: not built.** These phases are not built, and nothing runs from this plan until a unit is assigned.


## Phases

### Phase 49 — Four slow lifecycle and drift tests run in under a second each  · status: todo

> Note: the checkpoint replay test (1.2 s alone on the Mac) and the other cargo-berth tests over 1 s on the Mac also belong to this phase.

**Depends on:** 41

**Why:** (showrunner, 2026-10-30, row C153 follow-up) `lifecycle::upper_checkpoint_keeps_the_full_split_interval_when_update_refs_writes_lower_first` took 5.3 s in the phase 46 full Mac suite. The showrunner added three drift tests from a natedev run the same day: `drift::a_merge_keeps_the_units_own_committed_paths` (2.2 s), `drift::a_working_tree_edit_after_an_integrated_fast_forward_still_counts` (1.8 s) and `drift::an_integrated_fast_forward_is_not_a_post_write` (1.5 s). A test that takes seconds is a defect to fix, not a load flake. Queued behind the CPU work (phases 45, 47, 48 first).

**Work Order:**
1. Measure where each test's time goes (fixture setup, git children, ledger replay, waits) on the Mac, with the test alone.
2. Remove the cost at its cause (a smaller fixture, a shared template, no fixed waits) without weakening what each test proves.

**Verification:** each of the four passes alone on the Mac in under 1 s, with the same assertions. `cargo-berth` lint and the package suite pass.

**Files:** `crates/cargo-berth/tests/lifecycle.rs`, `crates/cargo-berth/tests/drift.rs`, test support under `crates/cargo-berth/tests/`

### Phase 50 — A merged phase's reservations are released, so the reservation cap is never reached  · status: todo

**Depends on:** 48

**Why:** (showrunner, 2026-10-30) 100 of hana's 128 live reservations were stale phase reservations that were never released (Codex worker phases). At the 128 cap no new reservation can open.

**Work Order:**
1. When a phase's commit is integrated into its trunk, release that phase's reservations automatically; or give one command that releases every stale reservation whose work is integrated. Prefer the automatic path if it needs no new state.
2. A reservation with unintegrated work is never released by this path.

**Verification:** a red-first test (under 1 s): a phase reservation whose commit is merged into trunk is released without a release call; a stale reservation with unmerged work stays. `cargo-berth` lint and the related tests pass.

**Files:** `crates/cargo-berth/src/`, `crates/cargo-berth/tests/`

