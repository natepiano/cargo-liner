# cargo-berth: rebased branches settle their reservations

## What it is

A checkpointed (`Outstanding`) reservation protects a phase interval `phase_start_head..protected_tip` until that work reaches its recorded target. Rebasing, amending, or resetting the branch rewrites those commits, so the protected tip may no longer be an ancestor of the target. Three mechanisms let such a reservation settle without a manual resolve. Ordinary reconciliation settles an outstanding reservation on its own once git evidence proves the whole phase reached the recorded target. The committed `reference-transaction` hook captures git's rewrite map so reconciliation can move the phase anchors onto the rewritten commits. And when the proof points at a target commit older than the tip, that commit is recorded as a separate integration witness. Orphan notices also name the disposition that fits the orphan's retained work and the observed tip of the branch it is judged at.

## How it works

Reconciliation judges each reservation at its judged branch: its recorded integration target, or the repository trunk when that target ref is missing. For a lane targeting an integration branch, reaching that branch is final: settlement releases the lane there once the branch has a cover, a reservation held in that branch's checkout against the branch's own target. Until then the lane stays outstanding. The retained field name `trunk_oid` denotes the judged branch's commit in this context. Merge extents and committed-path evidence are shared per worktree and target, so a change unique to the integration branch does not enter the lane's extent.

### Settlement

`build_plan` runs `complete_reconciliation_plan` (`src/reconcile.rs`) on every reconciliation: `board`, `check`, the post-Bash drift hook, the prepared gate, and the leading reconcile inside `release`. After evidence and merge extents are planned, `append_settlement_operations` walks every `Outstanding` reservation and asks `settlement_selection`:

```rust
fn settlement_selection(
    reservation: &Reservation,
    evidence: &IntegrationEvidenceStatus,
    actual_trunk: &JudgedTargetTip,
    merge_extent: &MergeExtent,
    committed_paths: &CommittedMergeEvidence,
) -> SettlementSelection // Unchanged | Release(ReleaseDisposition)
```

Before asking, it skips any reservation for which `cover_is_missing(snapshot, covered_targets, id)` holds: its recorded target is a present non-trunk branch with no cover whose merge extent this pass observed at that branch's tip. A cover appended this pass counts from the next. `settlement_selection` receives `snapshot.target_for(id)`, and committed paths are read for `(worktree, recorded target)`. It releases only when evidence is `Integrated { trunk_oid, proof, witness }`, `trunk_oid` equals that resolved tip, and the merge guard finds no unproven work. The disposition follows the proof:

| `IntegrationProof` | `ReleaseDisposition` |
| --- | --- |
| `ProtectedTipAncestor` | `Integrated` |
| `ScopedPatchEquivalent` or `RewrittenWitnessAncestor` | `RewrittenIntegration(witness.resolve(trunk_oid))` |

A settling reservation gets `EvidenceRevalidated { status }` followed by `Release`. Any earlier `EvidenceRevalidated` for that reservation in the same plan is dropped first. Settlement reads from replayed state plus this pass's evidence, so a crash between the two appends finishes on the next pass. Each settlement is reported as `ReconciledSettlement { reservation_id, disposition }` in `ReconciliationReport.settlements`. Alerts derive from post-settlement state, so a pass that settles raises no orphan alert for that reservation.

`rebuild_settlement_retention` then clears and rebuilds the pass's retention-ref repairs and deletions, so one transaction never both updates and deletes the same ref. A settled or released reservation with no nonterminal dependent loses its ref. Otherwise the ref points at the `RewrittenIntegration` witness, or at the protected tip for other dispositions. A newly settled witness is not in the initial commit batch, so `RetentionCommitResolution::IncludeSettlementWitness` switches the post-append ref write to an unbatched `update_reservation_retention_refs`. The default `InitialObservation(ResolvedBatchCommitCandidates)` reuses the batch.

**Merge guard.** `Reservation::has_unproven_merge_work(extent, committed_paths)` (`src/reservation/retention.rs`) holds settlement when any of these is true:

- a dirty tracked or untracked path overlaps a reserved scope,
- a committed path falls outside the reservation's scopes, or
- `key.head != protected_tip` and committed scoped paths exist.

Unrelated dirt does not hold settlement, and neither does a head that moved when no committed scoped paths remain. Committed paths arrive as `CommittedMergeEvidence::{Unavailable, Observed(Vec<ReservationScopePath>)}` per holder worktree. `Unavailable` (including an orphan with no checkout) routes to `has_unproven_retained_merge_work`, which counts every retained scope path as committed.

**Gate purposes.** `GateReconciliationPurpose::{PreparedDecision, CommittedAudit}` is passed by `src/gate/decision.rs` and `src/gate/audit.rs`. `PreparedDecision` projects actual-tip settlements with `RetainedReservationSet::with_pending_settlements(&operations)` before `observe_proposed_target` observes the proposed move of the trunk or a recorded target and builds constraints. `CommittedAudit` keeps the replayed lifecycles so a forced checkpoint still consumes its permit. Evidence against a proposed target never settles.

### Release verb

`release` reconciles first. When the reservation's evidence is `Integrated` but `cover_is_missing` holds for it, `release` returns envelope status `outstanding` with payload `TargetUncovered { reservation_id, target }` and releases nothing. Otherwise it judges the checkpoint at the branch `IntegrationTarget::judging_branch` returns: the recorded target, or the repository trunk when that ref is missing. If the reconcile pass settled the requested reservation, the verb returns `Released { disposition, marker: AlreadyAbsent, session_mapping_publication }` with that reservation's alerts removed. A later call reaches `released_evidence_operation`, which appends `EvidenceRevalidated` and reports `AlreadySettled`. `Abandoned` and `RetiredOrphan` dispositions (`ReleaseRevalidationSubject::None`) are rejected as `AlreadyReleased`.

`ReleaseRetentionPlan` (`src/verb/release.rs`) picks the ref action that runs after the append:

- `Preserve` when evidence is `ObjectUnknown`, or when an `Integrated` disposition's evidence carries a `Historical` witness.
- `RetainProtectedTip { reservation_id, protected_tip }` for ordinary checkpoints.
- `RetainIntegrationWitness { reservation_id, witness }` for `RewrittenIntegration`.

A repeated release after the witness has been collected therefore keeps later branch work intact.

For a `ProtectedTip` subject, including an outstanding checkpoint, `revalidate_proven_integration` tries a materialized `Historical` witness first, through `RewrittenIntegrationTrunkCommit::revalidate_ancestry`. A reachable witness yields `Integrated { trunk_oid: current judging-branch tip, RewrittenWitnessAncestor, same witness }`. `ObjectUnknown` stays unknown. Only `TrunkRewritten` falls back to the `PriorIntegrationStatus::Proven` current-tip replay. `integrated_release_operation` (empty merge extent) maps `EvaluatedTrunk` to `Integrated` with `RetainProtectedTip`, and `Historical(w)` to `RewrittenIntegration(w)` with `RetainIntegrationWitness`.

### Integration witness distinct from evaluated trunk

`src/reservation/lifecycle.rs`:

```rust
#[serde(tag = "kind", content = "commit", rename_all = "snake_case")]
pub(crate) enum IntegrationWitness {
    #[default] EvaluatedTrunk,
    Historical(RewrittenIntegrationTrunkCommit),
}
impl IntegrationWitness {
    pub(crate) fn resolve(&self, evaluated_trunk: &GitObjectId) -> RewrittenIntegrationTrunkCommit;
}

pub(crate) enum IntegrationEvidenceStatus {
    Integrated { trunk_oid: GitObjectId, #[serde(default)] proof: IntegrationProof,
                 #[serde(default)] witness: IntegrationWitness },
    TrunkRewritten, NotIntegrated, ObjectUnknown, ..
}

impl RewrittenIntegrationTrunkCommit {
    pub(crate) fn revalidate_ancestry(
        &self, trunk: &GitObjectId, reachability: impl FnOnce(&GitObjectId) -> Reachability,
    ) -> IntegrationEvidenceStatus;
}
```

`trunk_oid` always names the judged-branch tip the evidence was evaluated against, and `witness` names the commit that contains the work. `IntegrationProof::RewrittenWitnessAncestor` (wire `rewritten_witness_ancestor`) means the judged branch contains the witness. It makes no claim about the original checkpoint. `revalidate_ancestry` issues one ancestry query and maps `Ancestor` to `Integrated { trunk, RewrittenWitnessAncestor, Historical(self) }`, `NotAncestor` to `TrunkRewritten`, and `ObjectUnknown` to `ObjectUnknown`. It never runs the scoped replay.

A released `RewrittenIntegration` reservation revalidates only through that ancestry query, in `observe_released_repository_evidence` (`src/reconcile.rs`) and in `released_evidence_operation`. In `apply_release`, replaying a `RewrittenIntegration` release keeps any existing `Integrated` status. If no `Integrated` status exists, it creates `Integrated { witness, RewrittenWitnessAncestor, Historical(witness) }`. It also advances the proof-subject revision.

Ordinary observations in `src/reservation/evidence.rs` always construct `EvaluatedTrunk`. A `Historical` witness enters evidence only through candidate certification (below) or witness revalidation.

**Successors.** `PredecessorEvidenceStanding::of` selects `SuccessorIncorporationSubject::CheckpointTip(tip)` for outstanding reservations and ordinary releases, and `RewrittenIntegrationWitness(commit)` for `RewrittenIntegration` releases. Only `CheckpointTip` enters scoped comparison. A successor whose ancestry contains the witness is fulfilled. A successor that holds only equivalent content, without the witness commit, keeps holding.

### Rewrite capture in the committed hook

`evaluate_reference_transaction` (`src/gate/reference_transaction.rs`) calls `rewrite::branch_rewrites` only in the `Committed` phase. A `BranchRewrite { reference, previous, proposed }` is a local branch update whose previous tip is not an ancestor of the proposed one.

Apply-backend rebases commit with a zero previous OID. `apply_rebase_previous_tip` (`src/gate/rewrite.rs`) recovers the previous tip only when `rebase-apply/rewritten` exists:

- **Stopped rebase** (`rebase-apply/onto` present): the tip comes from `orig-head` when `head-name` names this branch. `detached HEAD` or another branch skips the update. A `head-name` outside `refs/heads/` is an error.
- **Uninterrupted rebase**: the tip is `<ref>@{1}`, after checking that `<ref>@{0}` equals the proposed tip.

In both cases every map `old` commit must be an ancestor of the recovered tip, or capture fails.

`capture_branch_rewrites` reads the operation's map and base once through `read_rewrite_map`, which returns `RewriteBase`:

| Situation | `RewriteBase` | Pairs |
| --- | --- | --- |
| `rebase-merge/rewritten-list`, or stopped apply with `onto` | `RebaseOnto(onto)` | the map |
| `rebase-apply/rewritten` without `onto` | `RecordedDestinations` | the map |
| no map (amend, reset) | `TipReplacement` | `previous → proposed` per rewrite |

Created commits per branch come from `capture_rewrite_created_commits`, which runs `git --git-dir=<canonical admin dir> rev-list --ignore-missing <new> --not [onto] <pair.olds> <previous>`. Under `RecordedDestinations` the result is filtered to map destinations. `RewriteCreatedCommits::branch_pairs` keeps only pairs whose destination this branch created. Every event is computed before any marker is written, so a capture failure surfaces as a hook error, git completes the ref update, and no marker exists.

**Pending branch-rewrite marker** (`src/gate/permit.rs`). Markers live in the common git dir under the shared `cargo-berth-pending-bypass-<uuid>.json` naming and create-and-fsync protocol. The payload is the untagged `PendingReconciliationMarker::{BranchRewrite, EnvironmentBypass}`, and a rewrite marker carries `kind: "branch_rewrite"`:

```rust
pub(crate) struct PendingBranchRewrite {
    kind: BranchRewriteMarkerKind,
    pub(crate) pairs: Vec<RewriteMapPair>,
    pub(crate) worktree_administrative_directory: PathBuf,
    pub(crate) new_tips: Vec<GitObjectId>,              // optional on the wire
    pub(crate) created_commits: Vec<GitObjectId>,
    pub(crate) completed_subjects: Vec<CompletedRewriteSubject>, // optional on the wire
}
```

`pending_environment_bypass_count` and bypass import skip any payload whose `kind` is `branch_rewrite`, so rewrite markers never count, audit, or notify as bypasses.

**Active reservations** re-anchor in the same hook invocation from the in-memory `BranchRewriteEvent`s. `reanchor_rewritten_phases` runs its own reconciliation transaction and appends `Resnapshot { Active { claim_snapshot } }` for each `Active` reservation claimed on the rewritten branch. When no map pair touches the old phase, `active_phase_anchor` uses the patch-position anchor `git::rewritten_phase_anchor`. Otherwise it needs `map_phase_interval` to return `Mapped` and scoped replay against `MappedDestinations` to return `Equivalent`. Any other result keeps the old anchor.

### Outstanding re-anchoring from the marker

`prepare_rewrite_reconciliation(worktree_context, ledger, berth_config) -> RewriteReconciliationPreflight` runs before the ledger lock in ordinary reconciliation, the prepared gate, and the committed audit. It reads the journal lock-free with `Ledger::read_validated_events`, which rereads up to three times on `ProjectionError::CacheAhead`. It skips any marker whose `worktree_administrative_directory` still has a rebase in progress (`git::rewrite_in_progress`). For each remaining marker and each `Outstanding` reservation not listed in `completed_subjects` at its current proof-subject revision, the preflight does the following:

1. `gate::rewrite_phase_commits` reads the old phase. `gate::rewritten_first_parent_history` reads each new tip.
2. `rewrite_map::map_phase_interval(pairs, old_phase, new_history, created)` returns `PhaseRewriteMapping::{Mapped(MappedPhaseInterval { phase_start_head, protected_tip, destinations }), Refused}`. Created commits that are not pair destinations and sit directly below the first destination are absorbed into the interval (split commits), and the base walks down past them.
3. Preflight resolves each reservation's recorded target tip once per distinct branch. `compare_mapped_phase` charges its budget key to that tip and runs scoped replay with `ScopedPatchTargetHistory::MappedDestinations { commits }`. The mapped variant skips `target_scoped_change_position`: an empty destination list is `Unproven`, and a non-empty one is `Contiguous`. Shared-history and protected-change queries still run.
4. `Equivalent` appends `Resnapshot { Outstanding { protected_tip, trunk_oid, phase_start_head: Some(..) } }`, where `trunk_oid` is the reservation target's tip. `Different` records a `CompletedRewriteSubject`. `Unavailable`, `HistoricalEvidenceUnavailable`, a budget deferral, unreadable history, or an unreadable target tip defers the subject and keeps the marker with its progress.

Under the lock, `RewriteReconciliationPreflight::project` checks each `RewriteSubjectValidation { reservation_id, subject }` against the locked replay and then applies accepted resnapshots with `with_pending_resnapshots`, before repository observation. A changed subject rejects the transaction without appending, and `retry_rewrite_reconciliation` retries the whole attempt up to three times. Retention refs move to the accepted tips, and markers are rewritten (`update_branch_rewrite_markers`, temp file plus rename) or deleted (`delete_branch_rewrite_markers`), only in the committed action after the append succeeds.

Deferred subjects stay conservative in the gate. `DeferredRewriteIntegrationSubject.rewritten_tips` follows later markers (`follow_rewrite`), and `GateReconciliation::deferred_rewrite_enters` counts the reservation as entering the proposed target when any nominated destination becomes reachable, so ordering holds still apply.

In the committed hook, `GateReconciliation::into_committed_hook_action(additional_operations)` keeps only `Resnapshot` and the four scoped-verdict and attempt records. It appends no evidence or lifecycle operations. `CommittedHookReconciliationAction::commit` repairs refs for outstanding resnapshot tips and publishes marker progress.

`ReservationSnapshot::Outstanding.phase_start_head` is optional on the wire. Replay converts it into `PhaseStartHeadUpdate::{Preserve, Replace}`: a missing value, as in legacy records or `release`'s own resnapshot, keeps the baseline. A resnapshot advances `IntegrationProofSubjectRevision`, which clears retained verdicts.

### Historical trunk candidate

When the protected tip is not an ancestor of the judged branch's tip, the admitted scoped evaluation for a reservation is `evaluate_reservation_scoped_integration`, which calls:

```rust
fn evaluate_historical_then_current_trunk(
    target: &GitObjectId,
    discover: impl FnOnce() -> HistoricalIntegrationCandidateDiscovery,
    compare: impl FnMut(&GitObjectId) -> ScopedPatchComparison,
) -> ScopedPatchIntegrationEvaluation
```

1. `git::discover_historical_integration_candidate(repository_root, phase_start, protected_tip, target, target_histories)` (`src/git/patch.rs`) returns `HistoricalIntegrationCandidateDiscovery::{Nominated(GitObjectId), NoMatch, Unavailable}`. It takes right-only cherry-mark matches from `phase_equivalent_commits` (`rev-list --cherry-mark --left-right --right-only --no-merges <tip>...<target> ^<phase_start>`). `PhaseStartTargetFirstParentHistories::earliest_containing_matches` (`src/git/reachability.rs`) then walks the target's first-parent chain after the phase start, oldest first, and nominates the first commit whose full ancestry, merge parents included, contains every match. That walk reuses the reconciliation's batched graph when its target matches and otherwise reads one `target_commit_history`. A missing parent link, or a match never located, returns `Unavailable`.
2. A nominated candidate is certified with `compare_scoped_patch` against the candidate. `Equivalent` returns `Equivalent(IntegrationWitness::Historical(candidate))`.
3. Otherwise current-tip replay runs. If it returns `Different` after discovery or certification was `Unavailable`, the result is `HistoricalEvidenceUnavailable`. Every other result maps directly.

`ScopedPatchIntegrationEvaluation::{Equivalent(IntegrationWitness), Different, HistoricalEvidenceUnavailable, Unavailable}` lives in `src/reservation/evidence.rs`. It is the cached value in `ReconciliationScopedPatchEvaluationBudget`, which is keyed by `ScopedPatchEvaluationKey { phase_start_head, protected_tip, target_trunk, scopes, context, destination }`. Only `target_trunk`, the observed judged-branch tip, is the admission key, so all three steps share one admission per observed tip, and duplicate subjects reuse the historical witness. `ScopedPatchComparisonDestination::{Trunk, Mapped { tip, destinations }}` keeps mapped comparisons in distinct cache entries while they charge the observed tip's single slot, so a mapped tip never becomes an admission key.

`scoped_patch_journal_update` journals `HistoricalEvidenceUnavailable` as `ScopedPatchComparisonAttempted`, never as a negative verdict. Positive verdicts journal `ScopedPatchEquivalenceChecked` with the witness.

`target_scoped_change_position` computes contiguity over `TargetFirstParentHistory::scoped_commits`, the first-parent commits that touch protected paths. An intervening commit that touches no protected path does not separate the integration. A protected-path gap still does.

### Journal and wire compatibility

| Record or type | Addition | Absent means |
| --- | --- | --- |
| `IntegrationEvidenceStatus::Integrated` | `witness` | `evaluated_trunk` |
| `IntegrationProof` | `rewritten_witness_ancestor` | (older proofs default `protected_tip_ancestor`) |
| `scoped_patch_equivalence_checked` | `witness`, `evaluator_version` | `evaluated_trunk`, `legacy` |
| `successor_scoped_patch_equivalence_checked` | `evaluator_version` | `legacy` |
| `ReservationSnapshot::Outstanding` | `phase_start_head` | preserve baseline |
| Board orphan `resolution` | `recover_with_trunk { recovery }` | legacy `recover { flag }` still decodes |

`ScopedPatchEvaluatorVersion::{Legacy (default), HistoricalCandidate}` (`src/reservation/scoped_patch_evaluation.rs`) is `Ord`, and `evaluator_version` is omitted from serialization when it is `Legacy`. Every new verdict writes `HistoricalCandidate`. `RetainedScopedPatchTargetVerdicts` and `RetainedSuccessorScopedPatchTargetVerdicts` store the witness (targets only) and version per entry. Lookup ignores a `Legacy` negative, so the subject is reevaluated under the current rules, while a `Legacy` positive still hits. Insertion never replaces an entry with a newer version, so replay order does not matter. `integration_status_from_retained_scoped_patch_comparison` rebuilds evidence with the stored witness, so a settlement delayed by reserved dirt, or a process restart, still releases at the historical commit.

Schema names `integration_witness` and `scoped_patch_evaluator_version` are pinned in `docs/cargo-berth/generated/output-contract.json`, and `docs/cargo-berth/as-built/json-contract.md` documents them.

### Orphan notice

`OrphanResolutionAction::new(&OrphanedOutstandingAlert, &JudgedTargetTip)` (`src/alert.rs`) uses the already-observed tip of the orphan's judged branch and the alert's `OrphanIntegrationEvidence`, which reconciliation derives from the same snapshot row (`Outstanding` with `Integrated { trunk_oid, witness }` becomes `Proven(witness.resolve(trunk_oid))`, anything else `Unproven`):

- `Recover(LostEvidenceRecovery::VerifyResolvedTrunk { trunk_oid, .. })`, for `Proven(commit)`, names that carrying commit and offers `resolve <id> --recovered` and `resolve <id> --integrated-as <commit>`.
- `Recover(NameCarryingTrunkCommit { trunk_oid, .. })`, for `Unproven` with a resolved tip, offers `--recovered`, `--retire-orphan --why <reason>`, and `--abandon --why <reason>`, and states that `trunk_oid` does not contain the protected tip. That tip is never offered as the `--integrated-as` argument, because it does not carry the work.
- `Recover(ResolveTrunkFirst { .. })`, for an `ObjectUnknown` tip, offers `--recovered` and says the branch must resolve before an integration commit can be named.
- `RetireOrAbandon` (verdict `CommitUnavailable`) offers `--retire-orphan --why <reason>` and `--abandon --why <reason>`.

`src/board/rows.rs` passes `|id| report.target_for(id).clone()` into `board_alerts(.., target_for)`, so each orphan's action uses its own judged tip. `alert::for_lost_integration_evidence` takes the same per-reservation tip. The wire adapter `BoardOrphanResolutionAction::{Recover { flag }, RetireOrAbandon { flags }, RecoverWithTrunk { recovery }}` (`src/board/alerts.rs`) emits `RecoverWithTrunk` for new alerts. SessionStart and post-tool notices render through the board model to `board_alert_detail`. `src/output.rs` has no orphan renderer.

## Invariants

- No path releases a reservation whose work did not reach its judged branch, and settlement at a present non-trunk target waits for that branch's cover. A rewrite map, a cherry-mark match, or a nominated candidate only nominates a location. Scoped replay must show that the whole `phase_start_head..protected_tip` is contained, and scoped work past the protected tip in the merge extent blocks release.
- Only ordinary reconciliation settles. The committed hook appends no evidence or lifecycle records, and evidence against a proposed target never settles. Settlement requires `trunk_oid ==` the judged branch's actual tip.
- `trunk_oid` is always the evaluated tip of the judged branch, and the released commit always comes from `witness.resolve(trunk_oid)`. Code must never read the witness from `trunk_oid` directly.
- Witness revalidation is ancestry only. A released `RewrittenIntegration` never reaches scoped replay, and only `CheckpointTip` successor subjects enter scoped comparison. Because ancestry alone keeps the witness valid, `recovery::verify_integration_commit_carries_work` runs `integration_status` against the named commit before `resolve --integrated-as` records it, and refuses a commit that neither contains the protected tip nor carries an equivalent of its scoped changes.
- Replay performs no git writes. Retention refs move, and rewrite markers are rewritten or deleted, only in the committed action after the append succeeds. A failed append leaves the old ref and the marker.
- Rewrite consumers use the `created_commits` and pairs stored at capture. They never reconstruct them from current refs. No consumer may assume hook stdin carries the previous tip.
- Proposed-target projection and lifecycle rules are chosen by `GateReconciliationPurpose`. `CommittedAudit` must not project settlements.
- Git work stays bounded at one cold scoped admission per observed target tip per pass. Historical discovery, certification, current-tip replay, and mapped rewrite acceptance all share that slot.
- A historical proof that could not be evaluated is never journalled as a negative verdict. It stays retryable.
- Older journal records and pending markers decode and replay unchanged. New fields are optional, and an absent field means preserve or legacy. Bytes under `tests/fixtures/reader_compat` stay unchanged.
- The wire contract grows only by additive variants and fields. `output-contract.json` matches the generator byte for byte.

## Calibration and gotchas

- Settlement can happen in the prepared gate or the drift hook before `board` runs, so "a rerun appends nothing" counts from whichever pass settled. A release loop expects `released` on the settling call.
- Every journal append republishes the session mapping, so a settled reservation's mapping entry disappears. Stale-mapping fixtures must restore stale bytes after the last append.
- Projecting settlement in the committed audit silently drops forced-permit consumption, because `entering_reservations` in `src/gate/decision.rs` filters out released reservations.
- A fixture that needs a reservation to stay outstanding after its work lands must dirty a reserved source. An orphan with an unavailable checkout settles automatically once its work reaches its judged branch.
- After a re-anchor, settlement passes the merge guard only if the resnapshot moved the protected tip to the rebased head.
- The merge guard compares dirty paths with case-insensitive overlap and committed paths with case-sensitive containment. Both choices lean toward holding.
- Git 2.54 `--update-refs` writes each branch in its own transaction, upper branch first. Every transaction sees the same `rebase-merge/onto` and the full rewritten list, which is why pairs are filtered per branch by created commits.
- An uninterrupted apply rebase leaves only `rebase-apply/rewritten`. A stopped one also writes `onto`, `orig-head`, and `head-name`. `<ref>@{1}` silently skips a reflog gap (it only warns), so a stopped apply rebase uses `orig-head`.
- Trim only `\n` and `\r` from rebase metadata, because ref names may end in Unicode whitespace.
- Scoped replay compares the whole target tree, so every created commit must lie inside the protected interval. Otherwise a too-narrow interval is accepted. This is why `map_phase_interval` absorbs unrecorded split commits.
- Known limits: a reset between rebase steps widens the created set, which can only cause non-acceptance. `--root` rebases are refused. `branch_rewrites` discovers the worktree and reads the apply map for every zero-previous update before the enrollment check. A rebase run while the binary is missing captures no map. Squash merges still rely on the replay, with `--integrated-as` as the manual exit.
- A rebase that was never captured still settles through the historical trunk candidate, without a marker.
- `phase_equivalent_commits` is shared with `rewritten_phase_anchor`. `--right-only` is safe there only because that caller tests trunk-side membership.
- Explicit release reconciles first, which may restore cached `ScopedPatchEquivalent` evidence before the release path runs. A test of release-produced `RewrittenWitnessAncestor` has to inspect the journal after the first release.
- Witness ancestry does not prove the original checkpoint still exists, so release never recreates a retention ref to it. This is why `Preserve` is used for an `Integrated` disposition with historical evidence.
- When the nominated candidate equals the target and certification is `Different`, the same comparison runs twice.
- Changing the witness, proof, or evaluator shape requires regenerating the contract with `CARGO_BERTH_REGENERATE_OUTPUT_CONTRACT=1`, or the reproducibility test fails.

## Why

**Settlement is automatic but narrow.** Nothing else in the engine removes state on inference, but this is not inference. Git proves the full scoped phase reached the judged branch's actual tip, and the merge guard proves no reserved work remains outside that proof. Tying settlement to `trunk_oid ==` that actual tip keeps a proposed target, or a branch that has since moved, from releasing anything. At an integration branch the cover wait keeps a lane from releasing before the branch's own work is held against its target.

**Settlement lives only in ordinary reconciliation.** The committed hook runs inside git's transaction for every local branch move and must stay cheap and non-destructive. Its audit must also see pre-settlement lifecycles, or forced permits are never consumed. The prepared gate projects settlements without owning them, so an already-landed predecessor does not block a proposed integration.

**The witness is separate from the evaluated trunk.** Before the split, the only commit a proof could name was the trunk it was evaluated against, so a phase that landed several commits back could only settle at the current tip. That broke two ways: later trunk commits touching the same paths made current-tip replay fail, and the retained ref named a commit unrelated to the work. Carrying `witness` next to `trunk_oid` lets evidence say both where it was checked and where the work is, and `resolve()` keeps the legacy case as a single code path.

**Rewritten integration revalidates by ancestry.** Once a witness is certified, the question is whether the judged branch still contains that commit, which a single ancestry query answers. Replaying the original phase again would need the checkpoint, which may already be collected, and would fail as trunk keeps editing the same files.

**The rewrite map is captured at commit time and certified by replay.** Git removes the rebase state as soon as the rebase ends, so the map has to be copied while the committed transaction still sees it. Created commits are fixed then too, because later ref moves make them impossible to recompute. The map is still only a nomination: many-to-one pairs, dropped commits, and conflict-resolution splits all make endpoints wrong in ways only scoped replay detects.

**Candidate discovery nominates the earliest containing commit.** The latest containing commit is the target's tip, and its replay has already failed. The earliest first-parent commit containing every trunk-side match is the first point where the work could be whole. Cherry-mark matches are reused instead of adding a patch-id pipeline.

**Contiguity counts only protected-path commits.** An unrelated commit landing between the replayed phase commits is ordinary on a shared trunk and says nothing about whether the work arrived whole. A gap made of protected-path commits does, so it still rejects.

**Evaluator versions ignore legacy negatives only.** Earlier rules could reject work the current rules certify, so an old negative must not suppress reevaluation. An old positive was never wrong under stricter rules, so it stays usable. Refusing to overwrite a newer version makes the cache independent of replay order.

**An unavailable historical proof is recorded as an attempt.** A negative verdict is durable. Journalling one because discovery could not run would permanently hide a proof that a later pass could establish, and the round-robin attempt schedule already retries attempts fairly.

**Orphan actions come from the observed judged tip.** Reconciliation already resolves every target's tip, so building the action from it adds no git work. A payload-free `RetireOrAbandon` covers the one case where no commit can support recovery. Offering `--integrated-as <trunk_oid>` only when the judged branch resolved means every published command can actually run.
