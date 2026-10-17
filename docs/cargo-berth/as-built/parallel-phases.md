# cargo-berth: parallel phases in one worktree

## What it is

Two or more phase reservations (harness sessions `<sid>.phase-N` in one coordination run) can hold paths in the same worktree at once. Each one's merge extent and integration are judged on that reservation's own paths. A sibling receives only its share of the worktree's merge extent, not the whole surface. A phase cannot release as integrated through `protected_tip_ancestor` while its own paths are dirty or unmerged, even after a sibling's merge puts the shared checkpoint HEAD into trunk. After the merge, one `release` call answers `released`.

## How it works

### Incursions across phase history

A phase's committed range can contain work merged from its integration target. Incursion authorship excludes commits reachable from the local target or its upstream ref already available locally; no fetch occurs. An absent upstream or unavailable upstream object falls back to the local target alone. Committed-only paths carried entirely by those commits raise no incursion. History is also read for paths that are both committed and dirty, while present working-tree writes still count as writes. This authorship filter leaves the local branch used for release and integration judgments unchanged.

Each incident records coverage evidence per entered path. Known entering commits form a non-empty set: once answered, the same commits stay answered as holders or sibling reservations join, and only a new entering commit reopens that path. Working-tree writes and unavailable history have distinct evidence variants. A working-tree answer covers the first commit recording the answered write; two or more entering commits reopen the path. Records from either earlier journal layout retain their holder-subset coverage rule. Coverage is shared only by reservations of the same run and worktree, so `resolve --every-incursion` answers the run's recorded backlog for the next hook check too.

### Each reservation's share of the worktree observation (`crates/cargo-berth/src/reconcile.rs`)

- `derive_merge_extents` calls `derive_merge_extents_with_observer`, whose observer can be swapped out in unit tests. `observe_holder_merge_extents` observes once per holder key `(WorktreeId, IntegrationTarget)`, using the first unreleased reservation it finds for that key. All keys are observed at the same time under `std::thread::scope`.
- `observe_merge_extent` builds the holder's complete surface. It takes `git::unmerged_branch_paths(root, trunk, head)` and adds the tracked and untracked dirty paths from `drift::observe_merge_working_tree`. Every path becomes a `ScopeKind::File` scope.
- `holder_group(reservation, reservations, snapshot) -> Vec<&Reservation>` returns the unreleased reservations that share the reservation's worktree and recorded target, the reservation itself included.
- `reservation_merge_extent(reservation, reservations, snapshot, extent) -> MergeExtent` splits a `MergeExtent::Protected` observation into shares. A reservation keeps a path when its own scopes contain it (`scope.contains(path, PathCase::Sensitive)`), or when no member of the holder group contains it. The result goes through `MergeExtent::derived`, so an empty share becomes `MergeExtent::Empty` under the same key. Observations that are not `Protected` pass through unchanged. A failed observation becomes each reservation's own `merge_extent().unavailable(failure)`.
- Journaling skips a reservation whose new share equals its stored extent. Reservations whose share changed get one `JournalOperation::HolderMergeExtentObserved` per distinct (holder, extent) pair; siblings share a record only when their shares are equal.

**Cache coverage key** (`crates/cargo-berth/src/reservation/merge_extent.rs`). `MergeExtentKey { trunk, head, working_tree, reservation_group }` carries `reservation_group: HolderMergeExtentCoverage`, which has two variants:

- `FullHolder` is the default. It covers a lone reservation or a legacy record, and `skip_serializing_if` leaves it out of the JSON.
- `ReservationShares { reservation_ids }` holds the group's ids, sorted.

On a cache hit, every group member's stored extent `matches_key`. The code then rebuilds the holder surface from the union of the members' `Protected` scopes and splits it again under the current declarations. When a sibling joins, leaves (release or abandon) or is retargeted, the id list changes, the key misses, and Git is queried again.

### The integration judgment (`crates/cargo-berth/src/reservation/evidence.rs`)

```rust
pub(crate) fn judge_integration(
    evidence: &IntegrationEvidenceStatus,
    extent: &MergeExtent,
) -> ReservationIntegrationJudgment
```

`ReservationIntegrationJudgment` has these variants:

- `Integrated { trunk_oid, proof, witness }`
- `Outstanding { pending_paths: Vec<ReservationScopePath>, evidence: PendingIntegrationEvidence }`
- `NotIntegrated`
- `TrunkRewritten`
- `ObjectUnknown`

`PendingIntegrationEvidence` is `Unproven`, `TrunkRewritten` or `ObjectUnknown`.

Rules, applied in order:

1. If the extent is `Protected`, the result is `Outstanding` and `pending_paths` holds the extent's scopes. This holds whatever the evidence says, including `Integrated`, which maps to `Unproven`.
2. Otherwise, evidence of `TrunkRewritten` or `ObjectUnknown` passes through as that variant.
3. An extent that is `NotDerived` or `Unavailable` gives `ObjectUnknown`.
4. Only an `Empty` extent passes `Integrated` or `NotIntegrated` through.

`evidence_status()` maps the judgment back onto the existing `IntegrationEvidenceStatus` vocabulary: `Outstanding/Unproven` becomes `NotIntegrated`. `integration_status` and `outstanding_integration_status` compute committed-work evidence from protected-tip ancestry or scoped patch equivalence; the judgment combines that evidence with the live extent.

Callers:

- **reconcile.rs.** `complete_reconciliation_plan` runs these steps in order: `append_cover_claims`, `derive_merge_extents`, `judge_live_integration`, `append_evidence_operations`, `append_settlement_operations`, then `append_merged_run_endings` (only under `MergedRunEndings::End`).
  - `judge_live_integration` judges each `Outstanding` repository snapshot again against the planned extent, or the stored one if none is planned. It rewrites a non-orphan snapshot's `integration_status` so board and hook readers agree with the journal. It leaves an orphaned snapshot's `integration_status` untouched, so the protected-tip ancestry or scoped-work proof reaches the orphan alert through `OrphanIntegrationEvidence::from(&snapshot.evidence)`: an orphan whose work landed on trunk gets the `VerifyResolvedTrunk` suggestion, and one whose work did not land keeps the unproven alert. For every snapshot, orphans included, it then replaces any planned `ReconciledEvidence` for that id with the full judgment, adding one only when the result differs from the materialized status. The orphan's journal judgment against its unavailable extent therefore stays conservative, and the orphan cannot settle automatically.
  - `settlement_selection` returns `Release` only when all three hold: the lifecycle is `Outstanding`, the judgment is `Integrated`, and its `trunk_oid` equals the resolved actual target tip. A `ProtectedTipAncestor` proof becomes `Integrated`. `ScopedPatchEquivalent` and `RewrittenWitnessAncestor` become `RewrittenIntegration(witness.resolve(trunk))`. `append_settlement_operations` then appends `EvidenceRevalidated` and `Release` with source `Reconciliation`.
  - `append_merged_run_endings` handles an active run that did work, has an `Empty` extent, and has a head that is an ancestor of trunk. It appends `Checkpoint`, `EvidenceRevalidated` and `Release`, but only when `judge_integration` returns `Integrated`.
- **`verb/release.rs`.** `outstanding_operation` calls it (see below).

### One-call integrated release (`crates/cargo-berth/src/verb/release.rs`, `crates/cargo-berth/src/ledger/handle.rs`)

`execute` runs `reconcile::reconcile` first, then `release_reconciled_request`, which checks in this order:

1. If this reconcile settled the id, it answers `Released` with that disposition.
2. If the evidence is integrated but the cover is missing, it answers `TargetUncovered`.
3. If the reservation was already released and the evidence changed, it answers `EvidenceRevalidated`.
4. Otherwise it runs `execute_release` under the ledger lock. It tries `ActiveCheckpointOnly` first, and retries with `AnyRetainedLifecycle` on `RequiresReconcileFirst`.

`validate_release_transaction` always returns `CommittedActionValidation::AppendWithPrerequisites { prerequisites: Vec<JournalOperation>, operation: Box<JournalOperation>, action }`. The ledger handle appends each prerequisite in order and then the operation, all in the same mutation transaction, before the committed action runs. It merges their session-mapping publication results.

`operation_for_state` builds `ReleaseRepositoryContext` with `merge_extent: reservation.merge_extent()`. That is the share the reconcile at the top of `execute` just wrote to the journal; release does not observe the worktree itself.

- **Active reservation: `checkpoint_operation`.** It appends a `Checkpoint` operation. The payload seed is `Checkpointed` with `pending_paths: ReleasePendingPaths::from(merge_extent)`.
- **Outstanding reservation: `outstanding_operation`.** If the materialized evidence is integrated, it uses `revalidate_proven_integration`; otherwise `outstanding_integration_status`. An unreadable current trunk gives `ObjectUnknown` evidence. It then calls `judge_integration`, so `ObjectUnknown` evidence against a `Protected` extent still answers `outstanding` with pending paths:
  - `Integrated`: `integrated_release_operation` builds the `Release` with disposition `Integrated` and `RetainProtectedTip` for an `EvaluatedTrunk` witness, or `RewrittenIntegration` and `RetainIntegrationWitness` for a `Historical` witness. An `EvidenceRevalidated { status: judgment.evidence_status(), .. }` prerequisite is pushed ahead of it. The answer is `released`.
  - `Outstanding { pending_paths }`: if the evidence is not `ObjectUnknown`, the call comes from the holder worktree, and HEAD has moved past the protected tip, it runs `resnapshot_operation`, whose seed derives `ReleasePendingPaths` from the stored extent. Otherwise `judged_evidence_operation` appends `EvidenceRevalidated` with the payload `Outstanding { pending_paths }`.
  - `NotIntegrated`, `TrunkRewritten` or `ObjectUnknown`: it resnapshots if the materialized evidence was not integrated, the call comes from the holder worktree, the evidence is `NotIntegrated` or `TrunkRewritten`, and HEAD moved. This fallback goes through the same `resnapshot_operation`, so an `Unavailable` or `NotDerived` extent answers `ResnapshottedPendingPathsUnavailable`, never an empty list. Otherwise it answers `evidence_revalidated`.
- **Released reservation: `released_evidence_operation`** answers `AlreadySettled`.

`evidence_revalidated` is therefore the answer only when the evidence changes without settling the reservation and no pending paths are known.

### Pending paths in release answers (`crates/cargo-berth/src/output.rs`)

`ReleasePendingPaths` (in `verb/release.rs`) is `Observed(Vec<ReservationScopePath>)` or `Unavailable`. `From<&MergeExtent>` maps `Protected` to `Observed(scopes)`, `Empty` to `Observed(vec![])`, and `NotDerived` or `Unavailable` to `Unavailable`. Both checkpoint and resnapshot seeds carry this type. `ReleasePayloadSeed::into_payload` turns `Observed` into `Checkpointed` or `Resnapshotted`, and `Unavailable` into `CheckpointedPendingPathsUnavailable` or `ResnapshottedPendingPathsUnavailable`. The private `release_message(&ReleasePayload)` in `output.rs` composes every release message; `OutputEnvelope::released` calls it and stays under clippy's line limit. No release message ends "pending paths: .".

| `ReleasePayload` variant | `payload.data.status` | envelope `status` | `pending_paths` | message |
|---|---|---|---|---|
| `Checkpointed` | `checkpointed` | `outstanding` | yes, possibly `[]` | `checkpoint_release_message`: "Reservation <id> is outstanding at protected tip <tip>." plus "; pending paths: a, b." when the list is non-empty |
| `CheckpointedPendingPathsUnavailable` | `checkpointed_pending_paths_unavailable` | `outstanding` | member absent | "...at protected tip <tip>; pending paths unavailable." |
| `Outstanding` | `outstanding` | `outstanding` | yes | "Reservation <id> remains outstanding; pending paths: <paths>." |
| `Resnapshotted` | `resnapshotted` | `outstanding` | yes, possibly `[]` | "Reservation <id> now retains protected tip <tip>." plus "; pending paths: a, b." when the list is non-empty |
| `ResnapshottedPendingPathsUnavailable` | `resnapshotted_pending_paths_unavailable` | `outstanding` | member absent | "Reservation <id> now retains protected tip <tip>; pending paths unavailable." |

On both checkpoint variants, `message_with_session_mapping_publication` appends the session-mapping note after the pending-path clause. `docs/cargo-berth/generated/output-contract.json` is generated from the DTO schemas in the test binary (`src/output_contract.rs`), and tests require an exact byte match, so any payload change means regenerating it.

### Board Overview Reservations table (`crates/cargo-berth/src/board/rows.rs`, `crates/cargo-berth/src/board/tui.rs`)

- `BoardReservationSnapshot` carries `session: BoardReservationSession` under `#[serde(skip)]`. The variants are `Mapped { primary: HarnessSessionId, aliases: Vec<HarnessSessionId> }`, `RunOnly(CoordinationRunId)` and `Unavailable` (the `Default`).
- `board_reservation_sessions(repository_root, reservations) -> HashMap<ReservationId, BoardReservationSession>` reads `session::phase_reservation_sessions(ledger_directory, run)` once per coordination run and caches it per run. The first label is `primary` and the rest are `aliases`. In `board_reservation_snapshots`, a reservation without a mapping falls back to `RunOnly(actor.run)`. A failed `WorktreeContext::discover` or store read also ends at `RunOnly`.
- `overview_cells() -> [String; 5]` fills ID, Session, Stage, Blocking and Race paths:
  - ID and Session go through `overview_short_id`. Session is shown as `primary, aliases…`, as `run <short>`, or as `unavailable`.
  - Stage is `active`, `outstanding` or `released`.
  - Blocking is `blocking` or `clear`.
  - Race paths are the `RaceExtent::Editing` scopes joined with ", ", or `(ended)`.
  - Session labels and paths are passed through `escape_debug`.
- `overview_short_id` keeps the last `-`-separated group of the identity before any `.phase-` suffix, then appends the suffix. For example, `…-123.phase-2` becomes `123.phase-2`.
- `overview_reservation_table(model)` collects rows from `ready_now`, `waiting` and `unconstrained_reservations`, drops `BoardReservationVisibility::ResolvedAudit` (terminal) rows, then sorts and dedups by id, so a waiting reservation with several holds appears once. Columns are padded by Unicode width under the title "Reservations". An empty table gets a "no reservations" row.
- `BoardPaneDocuments::from_model` puts the table at the top of the Overview pane text, followed by a blank line and the `OVERVIEW_FIELDS` JSON. The Reservations pane, `board --json` and the no-TTY pointer are unchanged.

## Invariants

- **Journal replay of existing ledgers is unchanged.** New behavior comes only from live observation and release decisions. `reservation_group` defaults to `FullHolder` and is omitted when it is `FullHolder`, so old `MergeExtentKey` records replay and re-serialize byte-identically. The judgment writes through `evidence_status()` into the existing `IntegrationEvidenceStatus` vocabulary; there is no new journal status.
- **A lone reservation keeps its prior merge extent.** A group of one gets the `FullHolder` key, and the split keeps every path (`a_lone_phase_extent_contains_the_whole_worktree_surface`). Its integration also requires that whole-worktree extent to be empty. Integrated checkpoint work with later unclaimed commits or dirt stays outstanding and names those paths (`release_keeps_unclaimed_later_branch_work_outstanding`, `rebased_integrated_checkpoint_retains_unclaimed_dirt` in `tests/lifecycle.rs`).
- **Integration that requires clean own paths is a type, not a caller flag.** `ReservationIntegrationJudgment::Integrated` comes only from `judge_integration` with an `Empty` extent. Release, `settlement_selection`, `append_merged_run_endings` and `judge_live_integration` all match on the judgment. No path may release or settle on `IntegrationEvidenceStatus::Integrated` alone.
- **The journal never records `integrated` evidence for a reservation whose extent is `Protected`.** Its judgment is `Outstanding`, which records as `not_integrated`, `trunk_rewritten` or `object_unknown`.
- **Unclaimed observed paths belong to every sibling's share.** When a sibling departs (abandon, retarget), its paths return to the remaining shares (`abandoning_…` and `retargeting_a_sibling_restores_its_unclaimed_paths_to_the_remaining_extent`).
- **An integrated release appends `EvidenceRevalidated` and then `Release` in one locked transaction**, in that replay order (`release_records_a_budget_deferred_proof_before_its_integrated_disposition`). A revalidation that does not settle appends no `Release` (`stored_integrated_evidence_is_revalidated_before_release`).
- **Pending paths are named only from an observed extent.** An unobserved extent is its own payload variant with no `pending_paths` member.
- **The Overview table and the Reservations pane are built from the same `BoardReservationSnapshot` rows.** Session identity is display-only and never serialized.

Key end-to-end tests:

- `tests/parallel_phases.rs`:
  - `parallel_phases_release_in_one_call_after_their_own_work_is_clean_and_merged`
  - `parallel_phase_merge_extents_contain_only_each_phases_own_paths`
  - `unclaimed_dirty_paths_belong_to_both_live_phase_extents`
  - `reconciliation_keeps_a_checkpoint_outstanding_while_its_own_path_is_{dirty,unmerged}`
  - `patch_equivalent_own_paths_settle_without_adopting_unmerged_sibling_work`
- `tests/hooks.rs`: `pre_tool_use_names_only_the_outstanding_phases_pending_path_after_a_sibling_merges`
- `tests/board.rs`:
  - `board_names_an_outstanding_phases_dirty_paths_after_its_sibling_settles`
  - `release_names_pending_paths_when_trunk_was_rewritten`
  - `…_when_integration_objects_are_unknown`
  - `release_reports_unavailable_pending_paths_after_a_trunk_rewrite`
  - `an_orphan_whose_work_landed_suggests_verifying_resolved_trunk`
  - `an_orphan_whose_work_did_not_land_keeps_the_unproven_alert`
- `src/board/tui.rs` unit tests:
  - `overview_distinguishes_sibling_phase_sessions_in_one_run`
  - `overview_phase_session_and_alias_leave_race_paths_visible_at_100_columns`

## Calibration / gotchas

- **Only a fresh `MergeExtent::Protected` names pending paths.** A `NotDerived` or `Unavailable` extent, even one that retains `Empty` evidence, judges as `ObjectUnknown`, or `TrunkRewritten` when the evidence says so. It never reports a clean phase and never reaches `Outstanding` (`unobserved_and_unavailable_extents_do_not_report_pending_paths`).
- **Release judges against the stored extent.** It is correct only because the reconcile at the top of `execute` has just written a fresh share to the journal. A release path that skips that reconcile would judge against a stale share.
- **An unclaimed dirty or committed file keeps every sibling's extent non-empty**, so every sibling stays outstanding until someone claims, commits or reverts it.
- **Share membership uses `PathCase::Sensitive`.**
- **A cache hit still runs `git status` every pass.** The cache skips only the unmerged-branch path query. Any change in group membership forces the full query.
- **With siblings, expect several `HolderMergeExtentObserved` records per holder per pass**, one per distinct changed share.
- **A checkout-less orphan never settles on its own.** With no checkout its extent is `Unavailable`, so the journal's judgment is `ObjectUnknown` and it stays outstanding until `resolve`, even when its work reached trunk. Its repository snapshot keeps the committed-work proof, so the board row's `integration_evidence` reads `integrated` when the work is in trunk, while `lifecycle.stage` stays `outstanding`, the extent reads `unavailable`, and a repeated `release` answers `evidence_revalidated` with the `orphaned_outstanding` alert (`release_retains_an_orphan_without_a_fresh_merge_extent` in `tests/liveness.rs`). Landed work suggests `VerifyResolvedTrunk`; work not shown to have landed keeps the unproven alert.
- **Session column fallbacks.** A board deserialized from JSON has no session identity and shows `unavailable`. A failed session-store read shows `run <short>`.
- **The raw Overview metadata JSON still appears below the table** (`journal_position`, `recovered_bypasses_this_invocation`, `integration_order`, `targets`, `git_cost`; `BoardPaneDocuments::from_model`). The TUI draws one model and never rebuilds it (`event_loop`), so reservations created after the board opens do not appear.
- **The Overview table must contain no blank line.** The test helper `reassembled_model_value` recovers the JSON by splitting the Overview text at the first `"\n\n"`.
- **Ruled out: an empty `pending_paths` list for an unavailable extent.** It would read as nothing pending; `CheckpointedPendingPathsUnavailable` and `ResnapshottedPendingPathsUnavailable` exist instead.

## Why

- **The worktree is observed once per `(worktree, target)` and split afterwards.** Each holder costs one `git status` and one unmerged-path query however many siblings it has, and the split itself is pure and needs no Git.
- **Unclaimed observed paths go to every sibling** so that work nobody claimed stays protected and keeps the worktree's reservations outstanding until it is accounted for. This is the conservative choice; the alternative would let unclaimed work leave protection when one sibling settles.
- **A lone reservation keeps the whole surface.** With no sibling to separate from, everything in the worktree is its work, which matches the extent it had before.
- **Group coverage is part of the cache key** because a share depends on who else is in the group. Trunk, HEAD and the working tree can all stay the same while a sibling leaves, and a reused share would then hide the departed sibling's paths.
- **There is one judgment function** so that release and reconcile settlement cannot disagree about whether a reservation settles. Returning a type, not exposing a flag, means no caller can release without the clean-extent check.
- **Integrated evidence and the `Release` go into one transaction** because the evidence must come before the release in replay order, and one transaction lets a post-merge release finish in one call.
- **An unavailable extent gets its own payload variant** because an empty `pending_paths` list would read as nothing pending. The distinct `checkpointed_pending_paths_unavailable` and `resnapshotted_pending_paths_unavailable` statuses keep the envelope `outstanding` while saying that the paths are unknown. An observed empty list omits the pending-path clause from the message.
- **The Overview table reuses `BoardReservationSnapshot` rows** so that its cells cannot drift from the Reservations pane and JSON. Session labels stay out of serialization because they are a display aid read from the session store, not ledger state.
