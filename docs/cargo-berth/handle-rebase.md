# handle-rebase — a rebased branch that fast-forwards into trunk releases its reservations

> **Status: IMPLEMENTATION PLAN — phased, delegate-ready.** Checkpointed reservations on a rebased branch settle automatically once their work reaches trunk, and orphan notices name the disposition that fits.

## Delegation Context

- **Project:** `cargo-berth` (`crates/cargo-berth`), a git-worktree reservation engine shipped as a binary only (`src/main.rs`, no `lib.rs`).
- **Project started:** 2026-09-15T08:28:17.103+00:00
- **Stack:** Rust edition 2024 (workspace, resolver 3), no rust-toolchain file. Workspace lints deny clippy `all`/`pedantic`/`nursery`/`cargo`, `unwrap_used`/`expect_used`/`panic`/`unreachable`, `missing_docs`, and `unsafe_code`. `serde` and `serde_json` carry journal records and pending markers, `schemars` 1 the output contract, and `uuid` v7 marker names. Git runs as a subprocess (`src/git/command.rs:50`); test fixtures shell out to `git` (`src/git/fixture.rs:152`). Dev-dependencies: `tempfile`, `cargo-berth-test-support`.
- **Layout:** all under `crates/cargo-berth/`:
  - `src/reconcile.rs`: reconciliation.
  - `src/reservation/{lifecycle,evidence,retention,scoped_patch_evaluation}.rs`: reservation state, evidence, and replay.
  - `src/ledger/journal.rs`: journal wire format.
  - `src/verb/release.rs`: release verb.
  - `src/gate/{reference_transaction,rewrite,permit,mod}.rs`: committed hook, rewrite detection, pending markers.
  - `src/git/{discovery,patch,reachability,fixture,constants,mod}.rs`: git queries and the patch fixture.
  - `src/alert.rs`, `src/board/alerts.rs`: alerts and the board renderer that hook notices also use.
  - `src/output_contract.rs` with `docs/cargo-berth/generated/output-contract.json` and `docs/cargo-berth/json-contract.md`: output contract.
  - `tests/{board,lifecycle,edges,gate}.rs`: integration tests. `tests/fixtures/reader_compat/`: frozen fixtures.
- **Key files:**
  - `crates/cargo-berth/src/reconcile.rs` — reconciliation. `candidate_ancestors` :380, `target_history_after_phase_start` :434, `ScopedPatchEvaluationKey` :470, `ReconciliationScopedPatchEvaluationBudget` :512 (`evaluate` :740), `SuccessorIncorporationSubject` :551, `enum PredecessorEvidenceStanding` :604 with `of` :615, `GateReconciliationPurpose` :764, `pending_bypass_imports` field :804 taken :1102 applied :2829, `RetentionCommitResolution` :812, `reconcile_for_drift` :876, `prepare_reconciliation_transaction` :1081, `CommittedMergeEvidence` :1121, `complete_reconciliation_plan` :1379, `reconcile_observed_reservations` :1489, `into_committed_hook_operations` :1711, `observe_released_repository_evidence` :1879 (`RewrittenIntegration` arm :1893), `integration_status_with_retained_verdict` :1949, `integration_status_from_retained_scoped_patch_comparison` :2089, `append_evidence_and_retention` :2115, `settlement_selection` :2208, `append_settlement_operations` :2251, successor retained-verdict early return :2696, successor comparator :2743.
  - `crates/cargo-berth/src/verb/release.rs` — release verb. `resnapshot_operation` :560 (its retention callback is the model for moving refs after append), snapshot constructor :572, `released_evidence_operation` :587 (`RewrittenIntegration` arm :620), `ReleaseRetentionPlan` :770 (`commit` :790).
  - `crates/cargo-berth/src/recovery.rs` — `resolve` dispositions; `--integrated-as` writes `ReleaseDisposition::RewrittenIntegration` at :603.
  - `crates/cargo-berth/src/reservation/lifecycle.rs` — `IntegrationProof` :27, `IntegrationEvidenceStatus` :147, `ReleaseDisposition` :195, `ReleaseRevalidationSubject` :233, `RewrittenIntegrationTrunkCommit` :321 (`revalidate_ancestry` :325).
  - `crates/cargo-berth/src/reservation/evidence.rs` — integration status observation. `integration_status` :125, `observe_integration_status` :187.
  - `crates/cargo-berth/src/reservation/retention.rs` — replay. `has_unproven_retained_merge_work` :72, `has_unproven_merge_work` :90, `with_pending_settlements` :252, `apply_resnapshot` :1146, `apply_release` :1183, `apply_evidence` :1230; tests from :1557, including `replay_retains_positive_and_negative_scoped_patch_verdicts` :1799 and `proof_subject_changes_remove_retained_verdicts_after_widen_resnapshot_and_replacement` :1893.
  - `crates/cargo-berth/src/reservation/scoped_patch_evaluation.rs` — `IntegrationProofSubjectRevision` :22, `RetainedScopedPatchTargetVerdicts` :71, `RetainedSuccessorScopedPatchTargetVerdicts` :106.
  - `crates/cargo-berth/src/ledger/journal.rs` — `enum JournalOperation` :335 with `ScopedPatchEquivalenceChecked` :431 (wire `scoped_patch_equivalence_checked`) and `SuccessorScopedPatchEquivalenceChecked` :451; `enum ReservationSnapshot` :1359 with `Outstanding` :1366; tests from :1914.
  - `crates/cargo-berth/src/ledger/handle.rs` — bypass `recoverable_operations` append after ordinary reconciliation operations :456.
  - `crates/cargo-berth/src/gate/install.rs` — managed hook records its issuing directory, then changes to the policy worktree :300.
  - `crates/cargo-berth/src/gate/reference_transaction.rs` — committed hook. `evaluate_reference_transaction` :243, `rewrite::branch_rewrites` call :284, context from `invocation_directory` :294.
  - `crates/cargo-berth/src/gate/rewrite.rs` — `BranchRewrite` :29, `branch_rewrites` :42, `reanchor_rewritten_phases` :79 (validation closure replays state under the lock at :92-111), `resnapshot_operations` :130.
  - `crates/cargo-berth/src/gate/permit.rs` — pending markers. `PENDING_BYPASS_FILE_PREFIX` :46, payload `PendingEnvironmentBypass` :77, `PendingBypassRecovery` :110, `PendingBypassMarkerImport` :117, `write_pending_marker` :248 (common git dir), `pending_environment_bypass_count` :277, `prepare_pending_bypass_recovery` :292; tests from :486.
  - `crates/cargo-berth/src/git/discovery.rs` — `rewrite_in_progress` :118, checking the `rebase-merge`/`rebase-apply` constants at `src/git/constants.rs:158-160`.
  - `crates/cargo-berth/src/git/mod.rs` — re-export hub (`mod patch` :15, `mod reachability` :17, `mod fixture` :21, `rewrite_in_progress` :28).
  - `crates/cargo-berth/src/git/reachability.rs` — `PhaseStartTargetFirstParentHistories` :110, `unmerged_branch_paths` :276.
  - `crates/cargo-berth/src/git/patch.rs` — scoped patch comparison. `ScopedPatchTargetHistory` :116, `TargetFirstParentHistory` :164, `phase_equivalent_commits` :238, `compare_scoped_patch` :349, `target_scoped_change_position` :618, `classify_target_phase_integration_commits` :670; tests from :979, including `intervening_unrelated_commit_does_not_separate_the_phase_integration` :1552, `duplicate_context_does_not_relocate_the_protected_change` :1576, `one_equivalent_commit_does_not_certify_partial_integration` :1735, `separated_target_equivalents_do_not_prove_one_replayed_phase` :1757.
  - `crates/cargo-berth/src/git/fixture.rs` — `PatchEquivalenceFixture` :36.
  - `crates/cargo-berth/src/alert.rs` — `LostEvidenceRecovery` :162, `OrphanResolutionAction` :193, `OrphanedOutstandingAlert` :266, `RecoverabilityVerdict` :381, orphan alert construction requires `Outstanding` and `Orphaned` :457.
  - `crates/cargo-berth/src/board/alerts.rs` — board renderer; hook notices render through it. `BoardAlert` orphan `resolution` :163, `BoardOrphanResolutionAction` :212 (wire `recover`, `recover_with_trunk`, `retire_or_abandon`), `board_alert_detail` :285, orphan flag rendering :338, `board_alert` :589 (builds the action at :625); tests from :730.
  - `crates/cargo-berth/src/board/rows.rs` — board rows; unit fixture snapshot constructor :1134.
  - `crates/cargo-berth/src/output_contract.rs` — schema generator; check test `generated_artifacts_are_reproducible_from_the_checked_in_contract` :302.
  - `docs/cargo-berth/generated/output-contract.json` — checked-in machine contract; `docs/cargo-berth/json-contract.md` is its prose companion.
  - `crates/cargo-berth/tests/board.rs` — `retained_scoped_patch_verdicts_reuse_both_results_after_process_restart` :2438, `reachability_integrates_every_outstanding_subject_without_scoped_comparisons` :2762, `cold_proof_subjects_bound_git_evaluation_for_distinct_and_duplicate_reservations` :3230, fixture fn `warmed_ancestor_proof_after_trunk_rewrite` :4137.
  - `crates/cargo-berth/tests/lifecycle.rs` — `failed_journal_append_does_not_move_the_retention_ref` :486, `releasing_an_integrated_checkpoint_again_preserves_later_branch_work` :2878.
  - `crates/cargo-berth/tests/edges.rs` — `rewritten_successor_content_is_cached_for_fulfilled_and_holding_edges` :852, `witness_survives_pruning_and_controls_successors` :933.
  - `crates/cargo-berth/tests/gate.rs` — prepared-gate settlement and forced-permit consumption coverage at :2130 and :2315.
- **Test lanes:** `crates/cargo-berth/tests` — each file is its own test binary (no `[[test]]` entries); shared helpers come in through `#[path = "support/timing.rs"] mod timing;` and `support/reader_compat_hooks.rs`. Unit tests are `#[cfg(test)] mod tests` blocks in `src`. Run one integration binary by name, e.g. `bash ~/.claude/scripts/delegate/verify.sh test cargo-berth board`.
- **Build:** `bash ~/.claude/scripts/delegate/verify.sh check cargo-berth`
- **Test:** `bash ~/.claude/scripts/delegate/verify.sh test cargo-berth`
- **Lint:** `bash ~/.claude/scripts/delegate/verify.sh lint cargo-berth`
- **Style:** `run-end /clippy style-only auto-proceed`
- **Invariants:**
  - (plan) No path may release a reservation whose work did not reach trunk. A rewrite map or a patch match only nominates a location; the scoped replay must show the whole `phase_start_head..protected_tip` is contained. A merge extent showing scoped work past the protected tip blocks release.
  - (plan) Settlement happens only in ordinary reconciliation. The committed reference-transaction hook keeps its lifecycle filter, and evidence against a proposed trunk never releases.
  - (plan) Replay performs no git writes. Retention refs move in the committed-action callback after the append succeeds; a failed append leaves the old ref.
  - (plan) Older journal records and pending markers still decode and replay. New fields are optional; absent means preserve or legacy.
  - (plan) Bytes under `tests/fixtures/reader_compat` stay unchanged; its README pins the journal SHA-256.
  - (plan) Git work stays bounded at one distinct cold proof subject per trunk target per pass.
  - (json-contract.md) The wire contract grows only by adding variants, never by renaming fields; schema definition names are stable wire names.
  - (code) `docs/cargo-berth/generated/output-contract.json` matches the generator byte for byte. Regenerate with `CARGO_BERTH_REGENERATE_OUTPUT_CONTRACT=1` through the test at `src/output_contract.rs:301`; `tests/overlap.rs` also `include_str!`s it.
  - (code) The crate has no public API: items are `pub(crate)`/`pub(super)`, every item carries a doc comment, and nothing unwraps, expects, or panics outside test modules with a reasoned allow.
  - (plan author's scope choice) Not in this plan: squash merges still rely on the replay with `--integrated-as` as the exit; a rebase run while the binary is missing captures no map; managed hooks not following a relative or per-worktree `core.hooksPath` is a gap that predates this plan and affects every managed hook; the volume of `merge_extent_observed` journal events.

## Phases

### Phase 1 — Settlement, witness, and orphan notice  · status: done

#### As-built

- **Settlement.** `build_plan` runs `complete_reconciliation_plan` (`src/reconcile.rs`) for every reconciliation (`board`, `check`, the post-Bash drift hook, the prepared gate, the release verb's leading reconcile). It selects from state through `settlement_selection`: an `Outstanding` reservation whose evidence is `IntegrationEvidenceStatus::Integrated { trunk_oid, proof }` with `trunk_oid` equal to actual trunk. `append_settlement_operations` then appends `EvidenceRevalidated { Integrated }` and `Release` idempotently, so a crash between the two finishes on the next pass. `IntegrationProof::ProtectedTipAncestor` releases as `ReleaseDisposition::Integrated`; `IntegrationProof::ScopedPatchEquivalent` releases as `ReleaseDisposition::RewrittenIntegration(RewrittenIntegrationTrunkCommit::from(trunk_oid))`. Alerts derive from post-settlement state, so a settling pass raises no orphan alert.
- **Gate purpose.** `src/gate/decision.rs` and `src/gate/audit.rs` pass `GateReconciliationPurpose::{PreparedDecision, CommittedAudit}`. `PreparedDecision` projects actual-trunk settlement with `RetainedReservationSet::with_pending_settlements` before proposed-trunk observation and constraints. `CommittedAudit` keeps replayed lifecycles so a forced checkpoint still consumes its permit. Proposed-trunk evidence never settles, and `into_committed_hook_operations` appends no lifecycle operations.
- **Merge-work guard.** `has_unproven_merge_work(extent, committed_paths)` (`src/reservation/retention.rs`) holds settlement only for dirty reserved paths or committed scoped work outside the proof. `head != protected_tip` blocks only when committed scoped paths exist. Under `CommittedMergeEvidence::{Unavailable, Observed(paths)}`, `Unavailable` uses `has_unproven_retained_merge_work`, which treats every retained scope as committed. A rebased branch whose head is in trunk settles despite unrelated dirt.
- **Witness.** The `RewrittenIntegration` commit is the witness. `observe_released_repository_evidence` and `released_evidence_operation` (`src/verb/release.rs`) revalidate `ReleaseRevalidationSubject::RewrittenIntegration` by ancestry only, through `RewrittenIntegrationTrunkCommit::revalidate_ancestry`, and never reach the scoped replay. An ancestor of trunk yields `Integrated`, a non-ancestor yields the non-affirmative result that raises lost evidence, and an unknown object yields `ObjectUnknown`. `PredecessorEvidenceStanding::of` reads the commit the subject names, and `apply_release` keeps an already-`Integrated` status. Under `SuccessorIncorporationSubject::{CheckpointTip, RewrittenIntegrationWitness}`, only `CheckpointTip` enters scoped comparison: a successor containing the witness fulfills, and one holding only equivalent content holds. `RetentionCommitResolution::IncludeSettlementWitness` retains a newly settled witness in the same pass.
- **Release verb.** If its leading reconciliation settled the reservation (`ReconciliationReport.settlements: Vec<ReconciledSettlement>`), `release` returns `Released { disposition, marker: AlreadyAbsent, session_mapping_publication }`, and a later call reports `already_settled`. A repeated release with a collected witness takes `ReleaseRetentionPlan::Preserve`, which keeps later branch work.
- **Orphan notice.** `OrphanResolutionAction` (`src/alert.rs`) is built from the `OrphanedOutstandingAlert` and the already-observed `RepositoryTrunk`; `src/board/rows.rs` passes trunk into `board_alerts`, and nothing new is stored on the alert. `Recover(LostEvidenceRecovery)` names `resolve <id> --recovered` or `resolve <id> --integrated-as <trunk_oid>`, or says to resolve trunk first when trunk is unresolvable. The payload-free `RetireOrAbandon` covers an unavailable commit and yields both the retire and abandon flags. The wire adapter `BoardOrphanResolutionAction` (`src/board/alerts.rs`) adds `recover_with_trunk`, whose `recovery` uses the `lost_evidence_recovery` shape (`verify_resolved_trunk` with `trunk_oid`, or `resolve_trunk_first`), and still accepts legacy `recover`. SessionStart and post-tool notices render through the board model to `board_alert_detail`; `src/output.rs` has no orphan renderer.

**Files:**
- `crates/cargo-berth/src/reconcile.rs` — settlement selection and append, gate purposes, `CommittedMergeEvidence`, successor subjects, witness revalidation and retention
- `crates/cargo-berth/src/reservation/{retention,lifecycle}.rs` — merge-work guard, `with_pending_settlements`, `apply_release`; ancestry-only witness revalidation
- `crates/cargo-berth/src/verb/release.rs`, `crates/cargo-berth/src/gate/{audit,decision}.rs` — witness revalidation arm, leading-settlement report, `ReleaseRetentionPlan`; the gates pass the reconciliation purpose
- `crates/cargo-berth/src/alert.rs`, `crates/cargo-berth/src/board/{alerts,rows}.rs` — `OrphanResolutionAction` with its `orphan_resolution_action` unit test; `BoardOrphanResolutionAction`, notice rendering, trunk into `board_alerts`
- `docs/cargo-berth/generated/output-contract.json`, `docs/cargo-berth/json-contract.md` — additive `recover_with_trunk`
- `crates/cargo-berth/tests/{answers,board,drift,edges,gate,hooks,lifecycle,liveness}.rs` — coverage of settlement, the witness (`witness_survives_pruning_and_controls_successors`), the gate (`prepared_gate_settles_actual_trunk_evidence_but_never_a_proposed_witness`, `forced_checkpoint_consumes_its_permit_before_ordinary_reconciliation_settles_it`), the merge guard, and release reporting; fixtures updated for automatic settlement

**Binds later work:** Settlement can already happen in the prepared gate or the drift hook before `board` runs, so "a rerun appends nothing" counts from whichever pass settles. Lifecycle projection is chosen by `GateReconciliationPurpose`. After a re-anchor, settlement passes the merge guard only if the resnapshot moves the protected tip to the rebased head (Rebase re-anchor from git's rewrite map). `settlement_selection` requires `trunk_oid == actual trunk` and uses that same `trunk_oid` as the witness, so a proven commit older than the trunk tip cannot settle as its own witness until evidence carries the witness separately from the evaluated trunk (Integration witness distinct from evaluated trunk; Historical trunk candidate). Witness revalidation is ancestry-only. The orphan action types are `OrphanResolutionAction::{Recover, RetireOrAbandon}` and wire `recover_with_trunk`.

**Gotchas:**
- Every journal append republishes the session mapping, so a settled reservation's entry disappears; stale-mapping fixtures restore stale bytes after the last append.
- Projecting settlement in the committed audit silently drops forced-permit consumption, because `entering_reservations` in `src/gate/decision.rs` filters out released reservations.
- A fixture that needs a reservation to stay outstanding after its work reaches trunk has to dirty a reserved source. Release loops expect `released` on the settling call.
- An orphaned reservation with an unavailable checkout settles automatically once its work reaches trunk.

**Ruled out:**
- Rendering the orphan notice in `src/output.rs` (`presentation_from_actionable_alerts`): that branch had no caller.
- Requiring every mapped checkpoint tip to equal the rebased branch head before settling: too strict, because committed paths vanish once the whole head lands.
- Dropping the reservation id argument from the orphan action's commands: `RetireOrAbandon` carries no id.

### Phase 2 — Rebase re-anchor from git's rewrite map  · status: done

#### As-built

- The reference-transaction hook captures a branch rewrite only in the `Committed` phase and writes a pending branch-rewrite marker (common git dir, beside bypass markers) holding `created_commits` and per-branch old→new pairs. Rewrite markers are decoded apart from bypass markers and never count or report as bypasses.
- Created commits are `rev-list --ignore-missing <new> --not [onto] <pair.olds> <previous tip>`, selected by `RewriteBase::{RebaseOnto(GitObjectId), RecordedDestinations, TipReplacement}`: merge backend and stopped apply use `RebaseOnto`; an uninterrupted apply rebase uses `RecordedDestinations` (filtered to map destinations); no map (amend, reset) uses `TipReplacement` (previous→proposed).
- Apply rebases commit the branch with a zero previous OID. `apply_rebase_previous_tip` recovers it: with `rebase-apply/onto` present, from `orig-head` when `head-name` names this branch (`detached HEAD` and other branches skip; a non-`refs/heads/` name errors); otherwise from the branch reflog `@{1}` after checking `@{0}` equals the proposed tip. Every map old commit must be an ancestor of the recovered tip. Capture failure prints a hook error, Git finishes, and no marker is written.
- Reconciliation re-anchors `Outstanding` reservations from the marker only when scoped replay against the new tip certifies the whole interval; `Active` reservations re-anchor from in-memory rewrite events. A refusal keeps the old anchors. `ReservationSnapshot::Outstanding` carries an optional `phase_start_head`, converted to `PhaseStartHeadUpdate::{Preserve, Replace}` before replay.
- Accepted resnapshots are projected before repository observation; deferred rewrite destinations are preserved in ordering decisions; retention refs and marker deletion change only after the append succeeds.

**Files:**
- `crates/cargo-berth/src/gate/rewrite.rs` — capture, `RewriteBase`, apply-rebase previous-tip recovery, created-commit computation, marker writing
- `crates/cargo-berth/src/gate/rewrite_map.rs` — parsing git's rewritten list
- `crates/cargo-berth/src/gate/{reference_transaction,mod,audit,decision,permit}.rs` — `Committed`-phase capture wiring, marker kind, ordering of deferred rewrites
- `crates/cargo-berth/src/git/patch.rs` — mapped `ScopedPatchTargetHistory` variant
- `crates/cargo-berth/src/reconcile.rs` — marker import, re-anchoring, projection, `GateReconciliation::into_committed_hook_action`
- `crates/cargo-berth/src/{ledger/journal,ledger/handle,reservation/retention,board/rows,verb/release}.rs` — resnapshot records, replay, consumers
- `crates/cargo-berth/tests/{lifecycle,gate}.rs`, `tests/support/{reanchoring,split_rebase}.rs` — real-git re-anchoring suite and ordering regressions

**Binds later work:** Rewrite-marker readers ("Integration witness distinct from evaluated trunk", "Historical trunk candidate") use the stored `created_commits` and per-branch pairs and never reconstruct them from current refs. Apply rebases supply a zero previous OID in hook stdin, so no consumer may assume the previous tip is present. `ScopedPatchTargetHistory` has a mapped variant that skips `target_scoped_change_position` and proceeds as contiguous. A resnapshot advances the proof subject revision and clears retained verdicts. Mapped acceptance charges the observed trunk's shared subject budget, with the comparison destination kept separate so mapped tips never become admission keys. Resnapshots are projected before observation; retention and marker changes happen only after append. The committed-hook entry point is `GateReconciliation::into_committed_hook_action`. An uncaptured rebase settles through "Historical trunk candidate" without a marker.

**Gotchas:** Git 2.54 `--update-refs` writes each branch in its own transaction, upper branch first, and every transaction sees the same `rebase-merge/onto` and full rewritten list. An uninterrupted apply rebase leaves only `rebase-apply/rewritten`; a stopped one also writes `onto`, `orig-head`, `head-name`. `<ref>@{1}` returns the preceding record's new value across a reflog gap with only a warning, so a stopped rebase uses `orig-head`. Trim only `\n`/`\r` from rebase metadata, since ref names may end in Unicode whitespace. Scoped replay compares the whole target tree, so every created commit must sit inside the protected interval or a too-narrow interval is accepted. Known limits: a reset between rebase steps widens the created set (non-acceptance only); `--root` rebases are refused; `branch_rewrites` discovers the worktree and reads the apply map for every zero-previous update before the enrollment check.

**Ruled out:** durable marker recovery for active reservations when the resnapshot append fails after capture (rare IO race, single-user scope); a separate capture-failure acceptance case in "Historical trunk candidate" (its no-marker settlement test covers it); reconstructing created commits at reconcile time or from transaction-wide exclusions; legacy HEAD inference; reading the reflog for a stopped apply rebase.

### Phase 3 — Integration witness distinct from evaluated trunk  · status: done

#### As-built

- `IntegrationWitness::{EvaluatedTrunk, Historical(RewrittenIntegrationTrunkCommit)}` (default `EvaluatedTrunk`) with `resolve(&GitObjectId) -> RewrittenIntegrationTrunkCommit`; serde is snake_case, tagged `kind` with content `commit`.
- `IntegrationEvidenceStatus::Integrated` carries a defaulted `witness` alongside `trunk_oid`, which stays the evaluated trunk. Ordinary ancestry and equivalence observations in `evidence.rs` and the explicit constructors in `board/rows.rs` and `git/patch.rs` use `EvaluatedTrunk`.
- `IntegrationProof::RewrittenWitnessAncestor` is a distinct, additive proof. `RewrittenIntegrationTrunkCommit::revalidate_ancestry` reports it with `Historical(self)` instead of `ProtectedTipAncestor`; synthesized rewritten release and replacement evidence in `retention.rs` uses the same witness ancestry with the explicit rewritten commit.
- `settlement_selection` releases `ScopedPatchEquivalent | RewrittenWitnessAncestor` as `RewrittenIntegration(witness.resolve(trunk_oid))`, only when the evaluated trunk equals actual trunk.
- Journal `ScopedPatchEquivalenceChecked` carries a defaulted `witness`; `RetainedScopedPatchTargetVerdicts` stores it per verdict, replay restores it, and `integration_status_from_retained_scoped_patch_comparison` rebuilds evidence with it, so settlement delayed by reserved dirt and a process restart keep the witness.

**Files:**
- `crates/cargo-berth/src/reservation/lifecycle.rs` — `IntegrationWitness`, `RewrittenWitnessAncestor`, witness revalidation
- `crates/cargo-berth/src/reservation/evidence.rs` — observations construct `EvaluatedTrunk`; deferred evidence handles the new proof
- `crates/cargo-berth/src/reservation/scoped_patch_evaluation.rs` — witness retained per verdict
- `crates/cargo-berth/src/reservation/retention.rs` — journal replay of the witness; rewritten release evidence uses witness ancestry
- `crates/cargo-berth/src/reconcile.rs` — witness carried into evidence, journal, and settlement
- `crates/cargo-berth/src/ledger/journal.rs` — `witness` field with legacy default
- `crates/cargo-berth/src/output_contract.rs`, `docs/cargo-berth/generated/output-contract.json`, `docs/cargo-berth/json-contract.md` — witness and proof in the schema
- `crates/cargo-berth/tests/edges.rs` — `witness_survives_pruning_and_controls_successors` checks witness-ancestry proof and distinct identities
- `crates/cargo-berth/tests/board.rs` — `retained_historical_witness_survives_dirty_settlement_and_process_restart`

**Binds later work:** the historical trunk candidate records its commit as `IntegrationWitness::Historical` and settlement obtains the released commit through `resolve()`; no historical-candidate discovery exists yet, so every non-revalidation observation produces `EvaluatedTrunk`.

**Gotchas:**
- Changing the witness or proof shape requires regenerating the output contract with `CARGO_BERTH_REGENERATE_OUTPUT_CONTRACT=1`, or the reproducibility test fails.
- An absent `witness` in legacy journals and `EvidenceRevalidated` records decodes as `EvaluatedTrunk`; frozen `tests/fixtures/reader_compat` bytes stay unchanged.

### Phase 4 — Historical trunk candidate  · status: todo

#### Work Order

**Goal:** A trunk commit patch-equivalent to a phase proves integration even when the rewrite map was never captured, and old negative verdicts, for reservations and for their successors, do not block the new evaluation.

**Spec:**

*Candidate.*
- Covers a rebase the gate did not capture, rebases before this fix, and cherry-picks.
- Matches come from the existing `phase_equivalent_commits` cherry-mark query (`src/git/patch.rs:240`, `rev-list --cherry-mark --left-right --no-merges <a>...<b> ^<phase_start>`), run between the protected tip and trunk with `--right-only` to list trunk-side `=` commits. No new patch-id pipeline.
- The candidate is the **earliest** first-parent trunk commit that has every match as an ancestor, read from the batched first-parent history in `target_history_after_phase_start` (`src/reconcile.rs:441`). The latest such commit is the trunk tip, which is the replay that already fails.
- Candidate history: `PhaseStartTargetFirstParentHistories` (`src/git/reachability.rs:110`) supplies an interval only when the old phase start is an ancestor of the target, so a later checkpoint whose start was itself rebased gets `NeedsGitQueries`, and its commit slice carries no parent relationships. Provide bounded candidate-history discovery when the old phase start is outside trunk ancestry, and ancestry evidence that locates a match introduced through a merge's other parent on trunk's first-parent chain. `git::patch` is private (`src/git/mod.rs:15`); export the candidate API through `src/git/mod.rs`.
- Certify with `compare_scoped_patch` (phase start, scopes, protected tip, candidate). A pass yields `ScopedPatchEquivalent` evaluated against actual trunk with the candidate as the historical integration witness (the Phase 3 witness state), which settles as `RewrittenIntegration(candidate)`. A fail falls through to the current-trunk replay in `integration_status_with_retained_verdict` (`src/reconcile.rs:2547`).
- Discovery returns `HistoricalIntegrationCandidateDiscovery::{Nominated(GitObjectId), NoMatch, Unavailable}`, never `Option<GitObjectId>`. `Unavailable` (history could not be read) stays retryable, the same distinction Phase 2 keeps for unavailable history (`src/reconcile.rs:1483`): when current-trunk replay then returns `Different`, the pass records no versioned negative that would suppress a later historical proof.
- Order within a subject: ancestry, then candidate, then current-trunk replay.
- Budget: `ReconciliationScopedPatchEvaluationBudget::evaluate` (`src/reconcile.rs:762`) admits work by `ScopedPatchEvaluationKey.target_trunk` (:477). Charge historical-candidate discovery to the observed trunk's shared subject budget and keep the comparison destination (the candidate) separate, so a candidate never becomes an admission key. Candidate certification and the current-trunk fallback run within one admitted subject evaluation. The budget unit stays one distinct cold proof subject per trunk target per pass.
- The shared cache `ReconciliationScopedPatchEvaluationBudget::comparisons` (`src/reconcile.rs:533`) stores only `ScopedPatchComparison`, which drops a historical witness when a duplicate subject reuses a result. Replace it with a semantic evaluation result carrying the certified witness, and carry that result through the cache, the observation API in `src/reservation/evidence.rs`, and the journal update.

*Contiguity rule.*
- In `target_scoped_change_position` (`src/git/patch.rs:628`), take positions in `TargetFirstParentHistory::scoped_commits` (:168) instead of `commits`.
- A gap commit touching no protected path is then allowed. One touching a protected path stays in `scoped_commits`, is not identified as a match, and still rejects.
- The same comparator serves current-trunk and successor checks, which stay safe because the replay still validates content. Check `intervening_unrelated_commit_does_not_separate_the_phase_integration` (:2111) against the new rule.

*Evaluator version.*
- Serialize an evaluator version on `ScopedPatchEquivalenceChecked` (`src/ledger/journal.rs:431`) and `SuccessorScopedPatchEquivalenceChecked` (:451); the wire field is optional and omitted when absent. Convert it before replay into `ScopedPatchEvaluatorVersion`, with `Legacy` for an absent field and an explicitly named variant for the historical-candidate evaluator. Retained domain records and replay APIs carry that type, never a bare `Option`.
- In `RetainedScopedPatchTargetVerdicts` (`src/reservation/scoped_patch_evaluation.rs:71`) and `RetainedSuccessorScopedPatchTargetVerdicts` (:106), a legacy negative permits one evaluation and its versioned replacement stops retries. For one subject and target the newer version wins.
- Successors need it too: successor evaluation uses the same comparator (`settle_pending_scoped_patch_comparisons`, `src/reconcile.rs:3323`), but a retained `Different` returns `NotIncorporated` before evaluation (`unreached_successor_evidence`, :3286-3316), so without a version existing successor holds never see the new rule. Rewritten-integration witnesses stay ancestry-only and never enter the comparison.
- Ship the version in the same step as the candidate and the contiguity rule, so no binary writes versioned negatives from the old evaluator. A resnapshot clears existing verdicts, but later passes repopulate both caches, so the version check applies after re-anchoring too.

*Tests.*
12. Unit candidate cases in one `PatchEquivalenceFixture` in `src/git/patch.rs`:
    - extend `one_equivalent_commit_does_not_certify_partial_integration` (:2294) with final-only and reordered matches;
    - extend `duplicate_context_does_not_relocate_the_protected_change` (:2135): equal patch ids nominate the wrong site and the comparator rejects it;
    - add a sibling carrying only a prefix of the phase, and a later trunk commit that rewrites the hunk so the earliest candidate is pinned;
    - update `separated_target_equivalents_do_not_prove_one_replayed_phase` (:2316) for the relaxed rule, with a protected-path gap that still rejects;
    - in `src/git/reachability.rs`: candidate history for an old phase start outside trunk ancestry, and a match introduced through a merge's other parent located on the first-parent chain.
13. Extend `cold_proof_subjects_bound_git_evaluation_for_distinct_and_duplicate_reservations` (`tests/board.rs:3230`) so candidate discovery and certification count under the observed trunk's budget, and duplicate subjects receive the same historical witness without another evaluation, including when mapped acceptance has already consumed the target's budget; plus a unit test of call order in `src/reconcile.rs`: ancestry, candidate, then current-trunk fallback inside one admitted evaluation.
14. Extend `replay_retains_positive_and_negative_scoped_patch_verdicts` (`src/reservation/retention.rs:1843`) with legacy decoding and version precedence in both orders, for reservation and successor verdicts. Extend `retained_scoped_patch_verdicts_reuse_both_results_after_process_restart` (`tests/board.rs:2438`) with a legacy negative: one evaluation, then zero across restart.
15. Integration settlement in `tests/board.rs`: with no rewrite marker, an outstanding reservation whose successful candidate predates actual trunk and whose current-trunk replay fails ends `Released`, the disposition names the candidate OID, the evaluation evidence names actual trunk, and no orphan notice appears. Also cover settlement delayed by reserved dirt followed by a process restart, where the witness survives, and a restart after unavailable candidate discovery plus a negative current-trunk fallback, where the historical proof is still attempted and settles. In `tests/edges.rs`, extend `witness_survives_pruning_and_controls_successors` (:933) for a historical witness: retention, lost-evidence recovery, and successor ancestry.
16. Successor regression in `tests/edges.rs`, modeled on `rewritten_successor_content_is_cached_for_fulfilled_and_holding_edges` (:852): a legacy negative successor verdict gets exactly one reevaluation under the new rule, its versioned replacement persists across restart, and rewritten witnesses keep ancestry-only treatment.

**Files:**
- `crates/cargo-berth/src/git/patch.rs` — candidate matches, contiguity rule; unit test 12
- `crates/cargo-berth/src/git/reachability.rs` — bounded candidate-history discovery and off-first-parent match ancestry; unit test 12 history cases
- `crates/cargo-berth/src/git/mod.rs` — export the candidate API
- `crates/cargo-berth/src/reconcile.rs` — candidate nomination and ordering within one admitted subject evaluation; successor verdict versioning; call-order unit test
- `crates/cargo-berth/src/ledger/journal.rs` — evaluator version on both verdict records
- `crates/cargo-berth/src/reservation/scoped_patch_evaluation.rs` — `ScopedPatchEvaluatorVersion`; version precedence in both retained verdict sets
- `crates/cargo-berth/src/reservation/retention.rs` — replay of versioned records; unit test 14
- `crates/cargo-berth/src/reservation/evidence.rs` — observation API carries the witness-bearing evaluation result
- `crates/cargo-berth/src/reservation/{lifecycle,mod}.rs` — only where a historical witness needs a constructor on the Phase 3 witness state
- `crates/cargo-berth/src/output_contract.rs`, `docs/cargo-berth/generated/output-contract.json`, `docs/cargo-berth/json-contract.md` — only if a serialized output changes
- `crates/cargo-berth/tests/board.rs` — tests 13, 14, and 15 integration parts
- `crates/cargo-berth/tests/edges.rs` — test 15 witness parts and test 16

**Seats:** 2 writers + 1 tester — git history and the comparator split from reconciliation, the journal, and retained verdicts by file group.
- `impl` — `src/git/{patch,reachability,mod}.rs`; candidate discovery, ancestry and history access, contiguity rule, unit test 12; hub: `src/git/mod.rs` (candidate API export)
- `review` — opens as impl: `src/reconcile.rs`, `src/ledger/journal.rs`, `src/reservation/{lifecycle,evidence,scoped_patch_evaluation,retention,mod}.rs`, `src/output_contract.rs`, the generated contract, `docs/cargo-berth/json-contract.md`; candidate nomination, witness recording, both verdict caches, evaluator version, replay, call-order test, unit test 14; hub: `src/reconcile.rs`. It calls the candidate API through the signatures `impl` posts on the board.
- `test` — `tests/*.rs` and integration support, excluding frozen `tests/fixtures/reader_compat` bytes; tests 13 to 16 integration parts and affected integration fixtures

**Constraints from prior phases:**
- Phase 1: actual-trunk settlement is selected in `complete_reconciliation_plan`; prepared decisions project it before proposed-trunk constraints, committed audits preserve lifecycles for forced-permit consumption, and proposed-trunk evidence never settles. Keep `forced_checkpoint_consumes_its_permit_before_ordinary_reconciliation_settles_it` (`tests/gate.rs:2615`) and `prepared_gate_settles_actual_trunk_evidence_but_never_a_proposed_witness` (:2804) green, including the bypass audit record.
- Phase 1: witness revalidation is ancestry-only; `SuccessorIncorporationSubject::RewrittenIntegrationWitness` (`src/reconcile.rs:572`) never enters scoped comparison; `RetentionCommitResolution::IncludeSettlementWitness` (:839) retains a newly settled witness in the same pass.
- Phase 2: `ScopedPatchTargetHistory` has a mapped variant that skips `target_scoped_change_position`; the contiguity change here applies only to the unmapped path.
- Phase 2: a resnapshot advances the proof subject revision and clears existing retained verdicts; evaluator-version checks still apply to every subsequently retained reservation or successor verdict, including verdicts recorded after re-anchoring.
- Phase 2: consume stored per-branch pairs and created_commits without reconstructing them from current refs or assuming hook stdin contains the previous tip; apply rebases can supply a zero previous OID.
- Phase 2: project accepted resnapshots before repository observation, preserve deferred rewrite destinations in ordering decisions, and perform retention and marker changes only after append; keep the lifecycle re-anchoring suite and the `pending_rebase_checkpoint_obeys_ordering_*` and `budget_deferred_rewrite_obeys_ordering_*` gate tests (`tests/gate.rs:2320`, `:2416`) green. The committed-hook entry point is `GateReconciliation::into_committed_hook_action` (`src/reconcile.rs:2241`).
- Audit surfaces: outstanding markers remain internal because board/check, reservation intervals, ordering denials, and orphan recovery expose their consequences; zero-previous non-rebase branch creation creates no reservation outcome and needs no additional surface, with coverage retained in `zero_previous_without_an_apply_map_keeps_skipping_capture` and `side_branch_creation_during_stopped_apply_rebase_preserves_checkpoint_capture`.
- Phase 2: mapped acceptance charges the observed trunk's shared subject budget and keeps the comparison destination separate; candidate discovery joins that same budget.
- Phase 3: `IntegrationEvidenceStatus::Integrated { trunk_oid, proof, witness: IntegrationWitness }` (`src/reservation/lifecycle.rs`) keeps `trunk_oid` as the evaluated trunk; `IntegrationWitness::{EvaluatedTrunk (default), Historical(RewrittenIntegrationTrunkCommit)}` serializes as `{"kind":"evaluated_trunk"}` / `{"kind":"historical","commit":"<oid>"}` and `IntegrationWitness::resolve(&GitObjectId) -> RewrittenIntegrationTrunkCommit` yields the commit to settle on. Witness ancestry records `IntegrationProof::RewrittenWitnessAncestor`. `settlement_selection` (`src/reconcile.rs`) releases `ScopedPatchEquivalent | RewrittenWitnessAncestor` as `RewrittenIntegration(witness.resolve(trunk_oid))` only when the evaluated trunk equals actual trunk. The journal `ScopedPatchEquivalenceChecked` carries a defaulted `witness` field, and `scoped_patch_evaluation.rs` retains the witness with each verdict and returns it on lookup, replayed by `retention.rs`. Record the historical candidate as `IntegrationWitness::Historical(candidate)`; keep the output contract regenerated (`CARGO_BERTH_REGENERATE_OUTPUT_CONTRACT=1`) when the witness shape changes.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh check cargo-berth`, `bash ~/.claude/scripts/delegate/verify.sh test cargo-berth`, and `bash ~/.claude/scripts/delegate/verify.sh lint cargo-berth` green; `verify.sh test cargo-berth board` and `verify.sh test cargo-berth edges` green with tests 13 to 16; unit tests 12, 14, and the call-order test pass; frozen reader fixture bytes unchanged. Behavior: a phase whose rewrite map was never captured settles once a trunk commit contains every match and the replay certifies it, with that commit recorded as the witness.
