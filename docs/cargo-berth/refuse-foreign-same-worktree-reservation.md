# Refuse a foreign same-worktree reservation

> **Status: IMPLEMENTATION PLAN — phased, delegate-ready.** A second coordination run in one worktree is refused a reservation, and is treated as foreign by the pre-edit and post-commit hooks.

> **As-built disposition: amend** — `docs/cargo-berth/as-built/worktree-coordination.md`

Issue: `/home/natepiano/rust/hanadocs/issues/refuse foreign same worktree reservation.md`

## Delegation Context

- **Project:** `cargo-berth` — the reservation engine that stops concurrent work from occupying the same paths across git worktrees.
- **Project started:** 2026-09-04T10:34:38-04:00
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

### Phase 1 — Refuse a second presented coordination run in one worktree, at claim and at both hooks  · status: done

#### As-built

A worktree admits one coordination run at a time. A second run that presents its own identity is refused the ability to take or widen a reservation there, at claim, at the pre-edit hook, and at the post-commit hook. The refusal governs acquisition only: observation and classification still run, so a refused run is still told what it entered and its report is still recorded.

Both sides of the rule are narrow, which is why upgrading an existing repository never arrives as a lockout. A holder occupies only if its own identity was presented — `Reservation::occupies_worktree_for_another_coordination_run(coordination_run_id, worktree_id)` (`reservation/record.rs`) requires same worktree, different run, `is_active()`, and `CoordinationIdentityProvenance::Presented`; `RetainedReservationSet::active_reservation_held_by_another_run(worktree_id, coordination_run_id) -> Option<&Reservation>` returns the incumbent, not a boolean, because the rejection needs its facts. The acting side is guarded separately at three call sites, each matching an environment-supplied identity first.

The refusal is `CoordinationIdentityRejection::WorktreeHeldByAnotherRun`, carrying `CoordinationIdentityRecoveryAction::ReleaseIncumbentReservation`, and it names two remedies: a runnable command that releases the incumbent, and a separate checkout when the incumbent is still working. On the drift path it travels as a two-state value on the report, `DriftScopeAcquisition::{Permitted, RefusedToSecondRun { rejection }}` (`drift/identity.rs`), so one envelope carries both the refusal and the drift outcome rather than an early error cutting the report short. A refused run is told what its own invocation wrote and never what the incumbent committed itself, including when its own commit is a merge or the repository's first commit.

**Files:**
- `crates/cargo-berth/src/reservation/record.rs` — the occupancy predicate and the recorded identity provenance it reads
- `crates/cargo-berth/src/reservation/retention.rs` — `active_reservation_held_by_another_run`; the drift predicates carry the acting run
- `crates/cargo-berth/src/coordination_identity.rs` — `CoordinationIdentityProvenance`, the rejection, and the release remedy it names
- `crates/cargo-berth/src/drift/identity.rs`, `verb/claim.rs`, `verb/check.rs` — the three sites deciding whether the rule is asked
- `crates/cargo-berth/src/drift/execution.rs`, `classification.rs`, `observation.rs` — where the refusal is decided, what it withholds, and what a refused run can be told it wrote
- `crates/cargo-berth/src/git/paths.rs`, `git/constants.rs`, `drift/git_output.rs` — one batched committed-history read answering both the incumbent's ranges and the newcomer's own commit
- `crates/cargo-berth/src/output.rs` — a refusal is presented as its own block, with no suggestion to re-run a command this rule would refuse
- `crates/cargo-berth/tests/drift.rs`, `tests/hooks.rs` — the acceptance cases
- `docs/cargo-berth/generated/output-contract.json` — the published payload gained the refusal

**Gotchas:**
- A commit range is not an authorship record. The incumbent's own commits sit inside `phase_start..HEAD`, so attributing the range to whoever runs next accuses the wrong party — and the accusation carries a blocking exit code, making it a wrong decision rather than a wrong message.
- `git diff-tree --stdin` accepts a lone commit line beside its pair lines and prefixes that record with the commit's own id. That is what lets one invocation answer two questions and holds the pinned process budgets where they are; keep the budget assertions rather than relax them.
- An anchor already standing at the target is dropped from the pair lines. The caller supplies that empty range itself, and the same line carries the target's own diff out — the subtlest line in the change.
- A live session mapping outranks the run-identity environment variable, so a caller inside a mapped session cannot present a second run and the rule is never asked for it. The refusal depends on this and nothing in the code states it.
- `OutputStatus::InvalidInput` now means two things: a request that never ran, and a run that completed a report and was refused only its acquisition. The presented text distinguishes them; the status does not.
- `CoordinationIdentityProvenance` is durable in the journal but read only on the holder's side. The acting side is guarded separately at each of the three call sites, so the rule's two terms live apart and a fourth call site added without that guard applies the rule to one side only.

**Ruled out:**
- Refusing a run that presents no identity at all — it would refuse the engine's own markerless first touch, which reaches the same validation.
- A second git process to read the newcomer's own commit — batched onto the existing invocation instead, keeping the budget guards in place.
- A new output status for the completed-but-refused run during this phase, because it regenerates the published contract; carried as a next item instead.
- Threading the acting side's provenance into the occupancy predicate — unreachable today, and it crosses the whole foreignness surface.

