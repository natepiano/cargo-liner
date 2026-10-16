# berth-followup

> **Status: IMPLEMENTATION PLAN — phased, delegate-ready.** A release never prints an empty pending-path list for paths it cannot observe, an orphan whose work landed gets the verify-resolved-trunk suggestion again, and the cargo-berth README and CHANGELOG describe the code that exists.

> **Production: remove-running-section** — unit `berth-parallel-unit`; production doc `docs/cargo-port/remove-running-section-production.md`

## Source

2026-10-16, from the showrunner, after the berth-parallel as-built pass (as-built `9e8fbb36`, merged as main `a9991bf8`). It runs under the berth standing approval: fast fix with tests, install on both machines.

- **(1)** The release fallback in `outstanding_operation` returns `Resnapshotted` with `pending_paths: []` when the merge extent is unavailable and the evidence is `TrunkRewritten`, so its message ends "pending paths: .". It must take the unavailable form, as the checkpoint does. An empty list never stands for unknown paths.
- **(2)** Regression from the per-phase merge extents: an orphan with no checkout always has an `Unavailable` extent and judges `object_unknown`, so `OrphanIntegrationEvidence::from(snapshot.evidence)` is always `Unproven`. The alert can no longer suggest `VerifyResolvedTrunk`, and it says trunk lacks the work even after the work landed. An orphan whose work landed on trunk gets `VerifyResolvedTrunk` again; a test pins the alert text in both cases.
- **(3)** `crates/cargo-berth/README.md` (~61–69, 86–93, 119) still describes the three-call release after a merge.
- **(4)** An older `[Unreleased]` `Fixed` entry in `crates/cargo-berth/CHANGELOG.md` names functions that no longer exist (`later_work_cover`, `has_unproven_merge_work`).
- (3) and (4) go in the same checkpoint as the fixes.

## Delegation Context

- **Project:** `cargo-berth`, the worktree reservation ledger and hook engine.
- **Project started:** 2026-10-16T22:40:00.000+00:00
- **Worktree:** `/home/natepiano/rust/cargo-liner-berth-parallel`, branch `remove-running-section-berth-parallel`. Every seat works only here, and every Work Order names it.
- **Base commit:** `a9991bf8` (main, merged in as `3ae85d4a`).
- **Test lanes:** `crates/cargo-berth/tests/`.
- **Stack:** Rust workspace, cargo-nextest. The crate has no lib target, so unit tests live in `cargo-berth::bin/cargo-berth` and integration tests in `crates/cargo-berth/tests/`.
- **Key files:**
  - `crates/cargo-berth/src/verb/release.rs` — `outstanding_operation`, `checkpoint_operation`, `CheckpointPendingPaths` (`From<&MergeExtent>`, `Observed` / `Unavailable`), `ReleasePayloadSeed`.
  - `crates/cargo-berth/src/output.rs` — `ReleasePayload` (`Checkpointed`, `CheckpointedPendingPathsUnavailable`, `Resnapshotted`), release messages, `checkpoint_release_message`.
  - `crates/cargo-berth/src/reconcile.rs` — orphan alert, `OrphanIntegrationEvidence::from(snapshot.evidence)` (~5301), `derive_merge_extents`, `observe_holder_merge_extents`.
  - `crates/cargo-berth/src/reservation/evidence.rs` — integration judgment.
  - `crates/cargo-berth/tests/lifecycle.rs`, `tests/parallel_phases.rs`, `tests/board.rs`.
  - `docs/cargo-berth/generated/output-contract.json` — regenerate with `CARGO_BERTH_REGENERATE_OUTPUT_CONTRACT=1`.
  - `docs/cargo-berth/as-built/parallel-phases.md` — records (1) as a known defect; remove that note once fixed.
  - `crates/cargo-berth/README.md`, `crates/cargo-berth/CHANGELOG.md`.
- **Build:** `bash ~/.claude/scripts/delegate/verify.sh check cargo-berth`
- **Test:** `bash ~/.claude/scripts/delegate/verify.sh test cargo-berth`
- **Lint:** `bash ~/.claude/scripts/delegate/verify.sh lint cargo-berth`
- **Style:** run-end /clippy style-only auto-proceed
- **Invariants:**
  - Journal replay of existing ledgers is unchanged.
  - Invalid states are unrepresentable: unknown pending paths are a distinct variant, never an empty list.
  - No test takes a second. Format with `cargo +nightly fmt` only. Run one `verify.sh` at a time.

## Phases

### Phase 1 — Unknown paths are never an empty list, and the orphan alert sees landed work  · status: done

**Depends on:** none

#### Work Order

**Goal:** A release never reports an empty pending-path list for an unavailable extent, an orphan whose work landed on trunk gets the verify-resolved-trunk suggestion again, and the README and CHANGELOG match the code.

**Spec:**
1. In `outstanding_operation` (`src/verb/release.rs`), the fallback that answers `Resnapshotted` builds its pending paths through the same observed/unavailable split the checkpoint uses. When the extent is not derived or unavailable, it answers a distinct unavailable payload (a `ReleasePayload` variant with no `pending_paths` member, envelope `outstanding`, message ending "pending paths unavailable."), following `CheckpointedPendingPathsUnavailable`. No release message may end "pending paths: .". Regenerate the output contract.
2. Find where the orphan alert lost its landed-work case. An orphan with no checkout must still be judged on whether its protected tip (or its scoped work) is in trunk, so `OrphanIntegrationEvidence` can be the landed form and the alert suggests `VerifyResolvedTrunk`. Fix it at the cause (the evidence the orphan snapshot carries), not by special-casing the alert text.
3. Tests: (a) the release fallback with an unavailable extent and `TrunkRewritten` evidence answers the unavailable form and never "pending paths: ."; (b) an orphan whose work landed on trunk gets the `VerifyResolvedTrunk` suggestion, with its alert text pinned; (c) an orphan whose work did not land keeps the unproven alert, with its alert text pinned.
4. `crates/cargo-berth/README.md`: describe the one-call release after a merge (around lines 61–69, 86–93, 119), matching `docs/cargo-berth/as-built/parallel-phases.md`.
5. `crates/cargo-berth/CHANGELOG.md`: rewrite the older `[Unreleased]` `Fixed` entry so it names no removed function and states current behavior; add a `Fixed` line for (1) and (2).
6. `docs/cargo-berth/as-built/parallel-phases.md`: drop the known-defect note for (1) and state the fixed behavior; amend any as-built line about the orphan alert that (2) changes.

**Files:**
- `crates/cargo-berth/src/verb/release.rs` — release fallback pending-path split
- `crates/cargo-berth/src/output.rs` — unavailable release payload and message
- `crates/cargo-berth/src/reconcile.rs` — orphan integration evidence
- `crates/cargo-berth/src/reservation/` — orphan judgment, if the cause lives there
- `crates/cargo-berth/tests/` — the tests in Spec 3
- `docs/cargo-berth/generated/output-contract.json` — regenerated contract
- `docs/cargo-berth/as-built/parallel-phases.md` — known-defect note and orphan alert
- `crates/cargo-berth/README.md` — one-call release
- `crates/cargo-berth/CHANGELOG.md` — corrected entry and new Fixed line

**Workers:** 1 writer + 1 tester
- writer: `src/`, generated contract, README, CHANGELOG, as-built doc, lint and full gate.
- tester: the Spec 3 tests in `tests/`.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh test cargo-berth` green; no release message ends "pending paths: ."; both orphan alert texts pinned.

## Next items

See `docs/cargo-port/remove-running-section-berth-parallel-next.md`.
