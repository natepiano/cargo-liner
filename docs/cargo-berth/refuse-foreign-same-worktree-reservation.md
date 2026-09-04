# Refuse a foreign same-worktree reservation

> **Status: IMPLEMENTATION PLAN — phased, delegate-ready.** A second coordination run in one worktree is refused a reservation, and is treated as foreign by the pre-edit and post-commit hooks.

> **As-built disposition: amend** — `docs/cargo-berth/as-built/worktree-coordination.md`

Issue: `/home/natepiano/rust/hanadocs/issues/refuse foreign same worktree reservation.md`

## Delegation Context

- **Project:** `cargo-berth` — the reservation engine that stops concurrent work from occupying the same paths across git worktrees.
- **Stack:** Rust, edition 2024, workspace `cargo-liner`. `serde` + `schemars` for the wire contract, `uuid` v7 for identities, `tempfile` for test repositories. Integration tests drive the built binary through `CARGO_BIN_EXE_cargo-berth`.
- **Layout:**
  - `crates/cargo-berth/src/reservation/` — retained reservation set, holder records, foreignness partition
  - `crates/cargo-berth/src/verb/` — one module per command (`claim`, `check`, `drift`, `release`, …)
  - `crates/cargo-berth/src/coordination_identity.rs` — identity validation and its caller-repairable rejections
  - `crates/cargo-berth/src/ledger/` — journal, locking, worktree context, identity resolution
  - `crates/cargo-berth/src/drift/` — post-commit observation and incursion classification
  - `crates/cargo-berth/tests/` — integration suites driving the binary
  - `docs/cargo-berth/generated/output-contract.json` — the checked-in JSON output contract
- **Key files:**
  - `crates/cargo-berth/src/reservation/partition.rs` — `AuthorizedEditingIdentity` and its `is_foreign` (`:70`) / `identifies_requester` (`:105`) predicates; doc comment at `:64` records why the run term was removed
  - `crates/cargo-berth/src/reservation/retention.rs` — `conflicts_for_claim` (`:151`), `conflicts_for_drift` (`:163`), `blocking_coverage_for_drift` (`:175`), `bind_widened_scopes` (`:196`), `conflicts_for_edit` (`:238`), `conflicts_for_first_touch` (`:250`), `conflicts_for_authorized_edit` (`:268`), `has_other_active_reservation` (`:487`), `conflicts_with_holders` (`:1181`)
  - `crates/cargo-berth/src/reservation/record.rs` — `is_active_for_coordination_run` (`:158`), `is_active_for_coordination_run_and_worktree` (`:171`), `edit_blocking_status` (`:209`)
  - `crates/cargo-berth/src/reservation/lifecycle.rs` — `ReservationLifecycle` (`Active` / `Outstanding` / `Released`) and `EditBlockingStatus`
  - `crates/cargo-berth/src/verb/claim.rs` — `ClaimRunValidation` (`:117`), its `validate` (`:1444`), `validate_first_touch_run` (`:1295`), `FirstTouchClaimRejection` (`:493`), `validate_first_touch_transaction` (`:927`), the non-first-touch claim validation (`:885`)
  - `crates/cargo-berth/src/verb/check.rs` — `CheckDecisionError::CoordinationIdentity` (`:55`) and its `OutputEnvelope::coordination_identity_rejected` arm (`:68`)
  - `crates/cargo-berth/src/coordination_identity.rs` — `CoordinationIdentityRecoveryAction` (`:209`), `CoordinationIdentityRejection` (`:359`), `wire_kind` (`:406`), `reservation_ids` (`:415`), `rendered_recovery_actions` (`:423`), `Display` (`:437`), `validate_marker` (`:667`)
  - `crates/cargo-berth/src/ledger/authorization.rs` — `ResolvedEditAuthorization::for_edit_authorization` (`:26`, new id at `:43`), `EditAuthorization::resolve_from_sources` (`:147`)
  - `crates/cargo-berth/src/ledger/worktree_context.rs` — `publish_coordination_run_marker` (`:246`), `remove_coordination_run_marker` (`:276`)
  - `crates/cargo-berth/src/drift/classification.rs` — the `blocking_coverage_for_drift` call site (`:407`)
  - `crates/cargo-berth/src/output_contract.rs` — schema generation; `CoordinationIdentityRejection` is already a published subject
  - `docs/cargo-berth/generated/output-contract.json` — regenerated artifact
  - `crates/cargo-berth/tests/lifecycle.rs` — `claim` helper returns `Output` (`:921`), so refusals are assertable
  - `crates/cargo-berth/tests/drift.rs` — run constants (`:38`, `:56`, `:59`), `claim` helper asserts success (`:3037`), `post_commit_treats_only_another_worktree_as_foreign` (`:1552`)
  - `crates/cargo-berth/tests/hooks.rs` — pre-edit hook suite
  - `crates/cargo-berth/tests/output_contract.rs` — checked-in contract must match generation
- **Test lanes:** `cargo-berth` — `crates/cargo-berth/tests`; `cargo-berth-test-support` — none.
- **Build:** `bash ~/.claude/scripts/delegate/verify.sh check cargo-berth`
- **Test:** `bash ~/.claude/scripts/delegate/verify.sh test cargo-berth`
- **Lint:** `bash ~/.claude/scripts/delegate/verify.sh lint cargo-berth`
- **Style:** `run-end /clippy style-only auto-proceed`
- **Invariants:**
  - The worktree stays the coordination unit for everything except the one new rule. `identifies_requester` (`partition.rs:105`) stays worktree-only: overlap answers bind the worktree that recorded them and a later run inherits them, so adding a run term there strands recorded answers.
  - `has_other_active_reservation` (`retention.rs:487`) keeps its run-only, cross-worktree reach and calls `is_active_for_coordination_run`. It asks whether the run still owns live work anywhere, which is a different question from what one worktree may edit.
  - Coordination-run identity resolution does not change. `ResolvedEditAuthorization::for_edit_authorization` creates a new `CoordinationRunId` only when nothing identifies the caller; the order is harness session mapping, then `CARGO_BERTH_RUN`, then the worktree's `cargo-berth-run-id` slot, then a new id. A second harness session in one checkout therefore adopts the incumbent run and is not a second run. Two runs in one worktree arise only from an explicitly supplied identity. Do not "fix" the adoption chain — it is what keeps this change free for two Claude sessions sharing a checkout.
  - A holder that already released its reservation but whose work has not reached the trunk (`ReservationLifecycle::Outstanding`) must never block another run in the same worktree. Commit `1b74ad02` removed the run term precisely because a worktree was blocking itself with what its previous session left behind; the `Active`-only restriction is what keeps that fixed.
  - The blocking filter runs before either identity predicate (`conflicts_with_holders`, `retention.rs:1190`), so a `Released` holder never reaches a foreignness question.
  - `bind_widened_scopes` (`retention.rs:210`) already compares run and worktree and is not revised.
  - Rejections are caller-repairable: every one names the offending identity and carries an executable recovery action. Do not add a rejection without one.

## Phases

### Phase 1 — Refuse a second coordination run in one worktree, at claim and at both hooks  · status: todo

#### Work Order

**Goal:** While one coordination run holds an `Active` reservation in a worktree, a second coordination run is refused a reservation there, is refused edits into the holder's scopes, and has commits into those scopes reported as incursions.

**Spec:**

The defect: `AuthorizedEditingIdentity::is_foreign` (`reservation/partition.rs:70`) compares worktree identity alone, so two coordination runs in one worktree can both hold reservations over the same paths and neither `claim` nor `check` refuses. `bind_widened_scopes` already refuses this case, so the engine is internally inconsistent.

Two mechanisms, because the two questions differ.

**1. Worktree occupancy, at claim acquisition.**

`ClaimRunValidation::validate` (`verb/claim.rs:1444`) is the one place both claim paths converge: each calls it inside the locked transaction with the replayed `RetainedReservationSet` — the first-touch path through `validate_first_touch_run` (`:1295`) and the full claim path directly (`:889`). Today its two `Independent*` variants return `Ok(())` unconditionally, which is the hole.

Add the occupancy check to `ClaimRunValidation::validate` for **both** `Independent` variants:

- `IndependentWithPresentedIdentity(run)` — an explicit `--run` or `CARGO_BERTH_RUN`. This is the case the issue is about.
- `IndependentWithoutPresentedIdentity { actor_run_id }` — nothing identified the caller, so a fresh id was created. Reaching here while an `Active` reservation exists in this worktree means the worktree's `cargo-berth-run-id` slot was absent; the same refusal applies, and its `reconcile` recovery action is the repair.

`ResolvedIdentityRequired` keeps its current path unchanged: a session mapping or slot already names a run, and `validate_coordination_identity` owns it.

Add to `RetainedReservationSet` (`reservation/retention.rs`) a predicate returning the incumbent, not a boolean — the rejection needs its facts:

```rust
/// Return an `Active` reservation in this worktree held by a different coordination run.
///
/// The worktree is the coordination unit, so one run holds it at a time. `Active` only:
/// an `Outstanding` holder has released and is awaiting integration, and blocking on it
/// would lock a worktree out of paths its own previous session released.
pub(crate) fn active_reservation_held_by_another_run(
    &self,
    worktree_id: WorktreeId,
    coordination_run_id: CoordinationRunId,
) -> Option<&Reservation>
```

Express the `Active` lifecycle test by reusing the eligibility predicates on `Reservation` (`reservation/record.rs:158`, `:171`) rather than re-writing `matches!(self.lifecycle, Active)` — the record doc comment states that test is written in one place. Do not add the worktree term to `is_active_for_coordination_run`; `has_other_active_reservation` depends on its cross-worktree reach.

Add the rejection variant to `CoordinationIdentityRejection` (`coordination_identity.rs:359`):

```rust
/// Another coordination run already holds active work in the issuing worktree.
WorktreeHeldByAnotherRun {
    /// The run already holding active work here.
    incumbent_coordination_run_id: CoordinationRunId,
    /// The incumbent's active reservation.
    incumbent_reservation_id:      ReservationId,
    /// The run this command presented.
    issuing_coordination_run_id:   CoordinationRunId,
    /// The worktree both runs name.
    issuing_worktree_id:           WorktreeId,
    /// The canonical checkout both runs name.
    issuing_root:                  CanonicalWorktreeRoot,
    /// The executable repair when the incumbent is no longer live.
    recovery_actions:              CoordinationIdentityRecoveryActions,
},
```

Follow `StaleMarkerRun` (`:370`) as the model throughout — it is the closest existing shape and already resolves its `reconcile` command line through `CoordinationIdentityRecoveryCommands`. Update every match in the file:

- `wire_kind` (`:406`) — `"worktree_held_by_another_run"`.
- `reservation_ids` (`:415`) — `vec![*incumbent_reservation_id]`.
- `rendered_recovery_actions` (`:423`) — the variant's `recovery_actions`.
- `Display` (`:437`) — name the incumbent run and reservation, the issuing root, and what to do. Model the wording on the `StaleMarkerRun` arm: state the situation, then `Run {}` with the rendered recovery, then that the correct move is a separate checkout, then that no state changed. The recovery action is `CoordinationIdentityRecoveryAction::ReconcileAndSweepMarker` (`:218`), which is the repair when the incumbent is gone rather than live.

Thread it out through the existing rejection plumbing: `FirstTouchClaimRejection::CoordinationIdentity` (`verb/claim.rs:500`) and `ClaimRejection`'s equivalent already carry a `CoordinationIdentityRejection`, and `verb/check.rs` already surfaces one through `CheckDecisionError::CoordinationIdentity` (`:55`) into `OutputEnvelope::coordination_identity_rejected`. No new output shape.

The rejection is a published wire subject, so regenerate `docs/cargo-berth/generated/output-contract.json`. `crates/cargo-berth/tests/output_contract.rs` compares the checked-in artifact against generation and will fail until it is regenerated.

**2. Run-aware foreignness, at the two hook sites.**

The occupancy refusal prevents the situation from forming; it does not stop a writer. The pre-edit hook never asks whether the writer holds a reservation, only whether the path belongs to another worktree, so a refusal at claim time and nothing at write time leaves the board reading as protective when it is not. Give the two hook sites the same rule, for situations that formed anyway — a reservation acquired before this change shipped, an explicit run that never claimed, or a recorded bypass.

`AuthorizedEditingIdentity::is_foreign` (`partition.rs:70`) becomes: a holder is foreign when it belongs to a different worktree, **or** when it belongs to this worktree, is `Active`, and belongs to a different coordination run. `Unidentified` stays foreign to everything. Rewrite the doc comment at `:64` — it currently records the removal of the run term as settled and must state the narrower rule and why `Outstanding` is excluded.

`AuthorizedEditingIdentity`'s identified variants already carry `coordination_run_id`, so no signature changes there.

The same term goes into the drift predicates in `retention.rs`:

- `conflicts_for_drift` (`:163`) and `blocking_coverage_for_drift` (`:175`) currently close over `holder.actor.worktree != acting_worktree_id`. They need the acting run as well, and `blocking_coverage_for_drift`'s `SameIdentity` probe (`:181`) needs the matching inverse — same worktree and either the same run or a non-`Active` holder.
- The one caller is `drift/classification.rs:407`, which passes `subject.actor().worktree` and has `subject.actor().run` available on the same value.

Leave `conflicts_for_claim` (`:151`) alone: the occupancy refusal fires before it in both claim paths, so a run term there is unreachable.

**Test rewrites this forces.**

`post_commit_treats_only_another_worktree_as_foreign` (`tests/drift.rs:1552`) exists to assert the behavior being reversed, and its `claim(repository.path(), "file:subject.txt", SECOND_RUN)` (`:1555`) is refused once the occupancy gate lands — the file's `claim` helper (`:3037`) asserts success, so the test panics rather than failing an assertion. Restructure it:

- Rename to state the new rule.
- The second run no longer claims. It commits into the holder's scopes with no reservation of its own, and the assertion flips: an incursion **is** raised, naming the holder's reservation id in `foreign_reservation_ids`.
- Keep the genuinely-foreign-worktree half (`:1573` onward) exactly as it is.

Nothing else needs a new fixture: `foreign_worktree` is a per-file local helper for real `git worktree add` cases, and the same-worktree two-run case is already expressible through the existing `claim(root, scope, run)` helpers with distinct run constants.

**Files:**
- `crates/cargo-berth/src/reservation/partition.rs` — `is_foreign` gains the `Active`-only run term; its doc comment is rewritten; `identifies_requester` untouched
- `crates/cargo-berth/src/reservation/retention.rs` — new `active_reservation_held_by_another_run`; `conflicts_for_drift` and `blocking_coverage_for_drift` gain the acting run
- `crates/cargo-berth/src/reservation/record.rs` — reuse or extend the eligibility predicates so the `Active` test stays in one place
- `crates/cargo-berth/src/drift/classification.rs` — pass the acting run at `:407`
- `crates/cargo-berth/src/coordination_identity.rs` — the `WorktreeHeldByAnotherRun` variant and its four matches
- `crates/cargo-berth/src/verb/claim.rs` — the occupancy check in `ClaimRunValidation::validate` for both `Independent` variants
- `crates/cargo-berth/src/verb/check.rs` — confirm the new rejection reaches output unchanged; no edit expected
- `crates/cargo-berth/src/output_contract.rs` — the new variant reaches the published schema
- `docs/cargo-berth/generated/output-contract.json` — regenerated
- `crates/cargo-berth/tests/lifecycle.rs` — claim and check refusal tests, the `Outstanding` regression guard, the adopted-run guard
- `crates/cargo-berth/tests/drift.rs` — restructure `post_commit_treats_only_another_worktree_as_foreign`
- `crates/cargo-berth/tests/hooks.rs` — pre-edit refusal test

**Seats:** 2 writers + 1 tester — split by mechanism: the rejection and its claim-side wiring, versus the foreignness predicates and their drift call site.
- `impl` — `src/coordination_identity.rs`, `src/verb/claim.rs`, `src/verb/check.rs`, `src/output_contract.rs`, `docs/cargo-berth/generated/output-contract.json`; hub: `src/coordination_identity.rs` (the rejection variant every other seat's behavior is described against)
- `review` — opens as impl: `src/reservation/partition.rs`, `src/reservation/record.rs`, `src/drift/classification.rs`; hub: `src/reservation/retention.rs` (holds both the occupancy predicate `impl` calls and the drift predicates this seat edits — `impl` messages this owner for the predicate signature)
- `test` — `crates/cargo-berth/tests/lifecycle.rs`, `crates/cargo-berth/tests/drift.rs`, `crates/cargo-berth/tests/hooks.rs`; the Spec fixes the refusal's wire kind, the run constants, and which helper returns `Output`, so every case below is writable before the implementation lands

**Constraints from prior phases:** none — first phase.

**Acceptance gate:**

- `bash ~/.claude/scripts/delegate/verify.sh check cargo-berth` green
- `bash ~/.claude/scripts/delegate/verify.sh test cargo-berth` green
- `bash ~/.claude/scripts/delegate/verify.sh lint cargo-berth` green
- With two distinct run ids in one worktree, the second run's `claim` is refused with wire kind `worktree_held_by_another_run`, naming the incumbent run id, the incumbent reservation id, the issuing root, and a runnable `reconcile` recovery action
- `check` from the second run is refused the same way
- The pre-edit hook refuses the second run's edit into the incumbent's scopes
- A commit by the second run into the incumbent's scopes is reported as an incursion naming the incumbent's reservation id in `foreign_reservation_ids`
- An `Outstanding` same-worktree holder belonging to a different run blocks nothing: a new run claims, checks, edits, and commits over those paths without refusal or incursion
- A second harness session in the same checkout with no explicit run adopts the incumbent run and is refused nothing
- `crates/cargo-berth/tests/output_contract.rs` green against the regenerated artifact
- Every existing cross-worktree test unchanged and green, including the second half of the restructured `tests/drift.rs` test
