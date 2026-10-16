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

#### As-built

- `ReleasePendingPaths` (`Observed` / `Unavailable`) feeds both the checkpoint and the `Resnapshotted` fallback in `outstanding_operation`. `Unavailable` answers `ReleasePayload::ResnapshottedPendingPathsUnavailable` (envelope `outstanding`, no `pending_paths` member, message ending "pending paths unavailable."); an observed-empty `Resnapshotted` omits the pending-paths clause, so no release message ends "pending paths: .".
- Release message text is composed in `release_message`, outside `OutputEnvelope::released`.
- `judge_live_integration` leaves an orphaned snapshot's `integration_status` untouched, so its protected-tip evidence reaches the orphan alert: an orphan whose work landed on trunk gets the `VerifyResolvedTrunk` suggestion, one whose work did not land keeps the unproven alert. An orphan released without a fresh merge extent reports `integrated`.
- The README describes the one-call release after a merge; the CHANGELOG `[Unreleased]` `Fixed` entries name no removed function and state current behavior.

**Files:**
- `crates/cargo-berth/src/verb/release.rs` — `ReleasePendingPaths` and the fallback's observed/unavailable split
- `crates/cargo-berth/src/output.rs` — `ResnapshottedPendingPathsUnavailable`, `release_message`
- `crates/cargo-berth/src/reconcile.rs` — orphan skip in `judge_live_integration`
- `crates/cargo-berth/tests/board.rs` — `release_reports_unavailable_pending_paths_after_a_trunk_rewrite`, `an_orphan_whose_work_landed_suggests_verifying_resolved_trunk`, `an_orphan_whose_work_did_not_land_keeps_the_unproven_alert` (alert text pinned)
- `crates/cargo-berth/tests/liveness.rs` — `release_retains_an_orphan_without_a_fresh_merge_extent` expects `integrated`
- `docs/cargo-berth/generated/output-contract.json` — the unavailable release payload
- `docs/cargo-berth/as-built/parallel-phases.md` — fixed pending-path behavior and orphan alert
- `crates/cargo-berth/README.md`, `crates/cargo-berth/CHANGELOG.md` — one-call release, corrected `Fixed` entries

**Gotchas:** An orphan's journal judgment stays conservative; only the live snapshot keeps its protected-tip evidence. `OutputEnvelope::released` sits near clippy's line limit, so new release message text belongs in `release_message`.

**Ruled out:** special-casing the orphan alert text instead of keeping the snapshot's evidence.

## Next items

See `docs/cargo-port/remove-running-section-berth-parallel-next.md`.
