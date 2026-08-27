# cargo-berth fix list

> **Status: IMPLEMENTATION PLAN — phased, delegate-ready.** Closes the cargo-berth defects observed in live use on 2026-08-26 — a released reservation that blocked forever, misattributed worktree identity, a resolve that reports success as failure, and git-hook cost that scales without bound — then unifies the output contract and its front ends.

> **As-built disposition: amend** — fold into `docs/cargo-berth/as-built/worktree-coordination.md` and `docs/cargo-berth/json-contract.md`.

Field evidence for every defect below: [`berth-fix-evidence.md`](berth-fix-evidence.md).

## Delegation Context

- **Project:** `cargo-berth` (workspace member of `cargo-liner`) — a git-worktree reservation engine coordinating path ownership and merge order between worktrees.
- **Project started:** 2026-08-27T01:07:16.573+00:00
- **Stack:** Rust, edition 2024 (workspace), `clap` (derive), `serde`/`serde_json`, `crossterm`/`ratatui` (board TUI), `uuid`, `tempfile` (dev). No `lib.rs`; `main.rs` declares all modules as a binary crate.
- **Layout:** `crates/cargo-berth/src/` — `reservation/` (lifecycle+evidence engine), `verb/` (subcommand handlers: board, check, claim, drift, integrate, release, sequence), `edge/` (successor/predecessor graph + snapshot), `drift/` (classification, execution, provenance, observation, identity, constants, fingerprint, git_output, ordering, report, selection), `gate/` (git-hook install + permit), `git/` (command/refs/constants wrappers), `ledger/` (append-only journal, projection, lock), `board/` (mod, tests, tui), plus top-level `alert.rs`, `output.rs`, `reconcile.rs`, `recovery.rs`, `cli.rs`. `crates/cargo-berth/tests/` holds integration tests: `answers.rs`, `board.rs`, `drift.rs`, `edges.rs`, `gate.rs`, `ledger.rs`, `lifecycle.rs`, `liveness.rs`, `overlap.rs`. Front-end/hook layer lives outside the repo under `~/.claude/scripts/berth/` and `~/.claude/commands/plan/`.
- **Key files:**
  - `crates/cargo-berth/src/reservation/mod.rs` — reservation engine core; `apply_release` (~L999), `apply_evidence` (~L1031), `conflicts_with_holders` (~L1104), computed `edit_blocking_status` (~L1281), `ReservationEvidenceState` (~L491) and `evidence_state` (~L1799). Replay tests: `replay_retains_active_outstanding_released_and_rewritten_states` (~L1580), `replay_ignores_a_journaled_blocking_status_after_release` (~L1660), `replay_rejects_widen_after_release` (~L1690).
  - `crates/cargo-berth/src/reservation/lifecycle.rs` — `ReservationLifecycle`, `IntegrationEvidenceStatus::edit_blocking_status` mapping, `ReleaseDisposition`.
  - `crates/cargo-berth/src/reservation/evidence.rs` — integration-proof evaluation; `integration_status` (~L65), `outstanding_integration_status` (~L85).
  - `crates/cargo-berth/src/reconcile.rs` — revalidation loop over retained reservations (~L723-730); journals recomputed blocking status.
  - `crates/cargo-berth/src/recovery.rs` — `fn resolve` (~L104), `execute_one_incursion_resolution` (~L264), identity/run-id handling (~L632-636), `recovery_operation` `--integrated-as` gate (~L555), "already resolved" rejection (~L802).
  - `crates/cargo-berth/src/verb/release.rs` — release verb; journals blocking status (~L554-559).
  - `crates/cargo-berth/src/verb/claim.rs` — claim verb; stale-identity predicate (~L1181), retry message (~L1541).
  - `crates/cargo-berth/src/verb/check.rs` — check verb; cross-worktree identity check.
  - `crates/cargo-berth/src/verb/sequence.rs` — sequencing verb; duplicate stale-identity predicate (~L272).
  - `crates/cargo-berth/src/verb/integrate.rs` — integration verb; forced-permit consumption (~L114).
  - `crates/cargo-berth/src/edge/mod.rs` — `Edge::readiness` (~L316), which consults the snapshot's successor-incorporation evidence.
  - `crates/cargo-berth/src/edge/snapshot.rs` — successor/predecessor snapshot state; `SuccessorIncorporationEvidence` (~L69) and `PredecessorSuccessorIncorporation` (~L82). `SuccessorHeadReachability` no longer exists.
  - `crates/cargo-berth/src/alert.rs` — `Alert` enum (~L26), currently only `OrphanedOutstanding`.
  - `crates/cargo-berth/src/output.rs` — `OutputEnvelope` (~L67), `OutputStatus` (~L120), alert attachment/rendering (~L1545), wildcard consumer arm (~L1380).
  - `crates/cargo-berth/src/board/mod.rs` — board assembly; `ReservationRow` (~L131), omitted-row logic (~L625), row build (~L788), `reservation_visibility` (~L812).
  - `crates/cargo-berth/src/board/tests.rs` — `assert_trunk_rewritten_action` (~L435).
  - `crates/cargo-berth/src/drift/classification.rs` — `PriorClassification` (pre-lock foreign-path role).
  - `crates/cargo-berth/src/drift/execution.rs` — drift driver; no-change fast return (~L161-170), fingerprint publish (~L219-220), claim rejection (~L423).
  - `crates/cargo-berth/src/drift/provenance.rs` — `commits_for_paths`, `path_commits` (~L80-105), `commit_origin` (~L124-145).
  - `crates/cargo-berth/src/drift/observation.rs` — `observe_full` (~L289) NUL-delimited path encoding.
  - `crates/cargo-berth/src/drift/identity.rs` — worktree/run identity handling for drift.
  - `crates/cargo-berth/src/drift/constants.rs` — git argument constants (~L26).
  - `crates/cargo-berth/src/gate/mod.rs` — `evaluate_reference_transaction` (~L327), `branch_rewrites` (~L360), `commit_forced_permit_audits` (~L400), `reanchor_rewritten_phases` (~L417), `IntegrationRejectionKind` (~L1165).
  - `crates/cargo-berth/src/gate/install.rs` — `ManagedHook::script` (~L214), stdin pass-through (~L235), bypass binary call (~L236), hook-path discovery use (~L109).
  - `crates/cargo-berth/src/git/command.rs` — shared `git_command` constructor (~L14-21); no hook suppression today.
  - `crates/cargo-berth/src/git/refs.rs` — private retention-ref read/write/delete (~L36-75); target for scoped hook suppression.
  - `crates/cargo-berth/src/git/mod.rs` — git module surface; `reachability` (~L221), `update_local_branch` (~L350-383), `hooks_directory` (~L119).
  - `crates/cargo-berth/src/ledger/mod.rs` — `WorktreeContext` (~L102), `WorktreeContext::discover` (~L272), `worktree_identity`; append-only ledger in the common git dir.
  - `crates/cargo-berth/src/ledger/journal.rs` — `JournalActor` (~L104), journal event read/append, `resolve_incursion` records (~L598).
  - `crates/cargo-berth/src/cli.rs` — CLI parsing/dispatch; `run_reference_transaction` (~L1050), malformed-input handling (~L1068-1072), phase dispatch (~L1115-1119), stdin read (~L1122-1126), bypass audit (~L1148-1205), embedded trunk ref at init (~L994-999).
  - `crates/cargo-berth/tests/drift.rs` — drift/attribution integration tests; `a_committed_incursion_names_the_commits_that_introduced_its_paths`, answered-incursion exit-0 case (~L384).
  - `crates/cargo-berth/tests/board.rs` — board JSON integration tests; `release_dispositions_remain_resolved_when_trunk_rewrites` (~L586) with its `resolved_audit`/`clear` assertions (~L691-701).
  - `crates/cargo-berth/tests/gate.rs` — git-gate integration tests; committed-phase permit consumption (~L1098, ~L1167-1176).
  - `docs/cargo-berth/json-contract.md` — the stable JSON wire contract for envelopes and journal records.
  - `docs/cargo-berth/berth-fix-evidence.md` — Appendix A (released-reservation investigation) and Appendix B (hook-cost measurements).
  - `/Users/natemccoy/.claude/scripts/berth/claim_state.py` — Python coordinator: board/claim/check dispatch, `STATUS_PAYLOAD_KINDS`/`FIXED_STATUS_EXIT_CODES` tables (~L24), prose recovery (~L543), board-argv validator (~L2027).
  - `/Users/natemccoy/.claude/scripts/berth/install/hooks/berth_pre_edit.sh` — canonical PreToolUse shim; invalid-input refusal (~L345).
  - `/Users/natemccoy/.claude/scripts/berth/install/hooks/berth_post_bash.sh` — canonical PostToolUse shim; JSON validation (~L21), `typed_drift_feedback` (~L172).
  - `/Users/natemccoy/.claude/scripts/berth/install/hooks/berth_session_start.sh` — canonical SessionStart shim.
  - `/Users/natemccoy/.claude/commands/plan/delegate.md` — `/plan:delegate`; recovery call (~L1641), lifecycle classification (~L1659).
- **Build:** `bash ~/.claude/scripts/delegate/verify.sh check cargo-berth`
- **Test:** every one of these eleven commands, all green. The first runs the
  crate's own unit tests; the other ten are the integration suites in
  `crates/cargo-berth/tests/`.
  ```
  bash ~/.claude/scripts/delegate/verify.sh test cargo-berth
  bash ~/.claude/scripts/delegate/verify.sh test cargo-berth answers
  bash ~/.claude/scripts/delegate/verify.sh test cargo-berth board
  bash ~/.claude/scripts/delegate/verify.sh test cargo-berth drift
  bash ~/.claude/scripts/delegate/verify.sh test cargo-berth edges
  bash ~/.claude/scripts/delegate/verify.sh test cargo-berth gate
  bash ~/.claude/scripts/delegate/verify.sh test cargo-berth ledger
  bash ~/.claude/scripts/delegate/verify.sh test cargo-berth lifecycle
  bash ~/.claude/scripts/delegate/verify.sh test cargo-berth liveness
  bash ~/.claude/scripts/delegate/verify.sh test cargo-berth overlap
  ```
  **The bare `verify.sh test cargo-berth` line is not the test gate on its own.**
  It resolves targets through `target_flags`, which selects only lib and bin
  targets, so it cannot see `crates/cargo-berth/tests/` at all. Phase 1 passed its
  gate that way and shipped four failing integration tests: a released
  reservation's blocking contract in `lifecycle.rs` (two), the
  `resolve --integrated-as` recovery gate in `liveness.rs`, and a widen against an
  outstanding reservation in `answers.rs`. Naming each target is what makes them
  visible.
- **Lint:** `bash ~/.claude/scripts/delegate/verify.sh lint cargo-berth`
- **Style:** `phase-end /clippy style-only auto-proceed`
- **Invariants:**
  - Workspace `[workspace.lints]` denies at crate scope: `clippy::all`/`cargo`/`nursery`/`pedantic` as groups (priority -1), plus explicit `expect_used`, `unwrap_used`, `panic`, `unreachable`, `undocumented_unsafe_blocks`, `allow_attributes_without_reason`, `self_named_module_files`; `rust::missing_docs` and `rust::unsafe_code` deny. `multiple_crate_versions` and `redundant_pub_crate` are allowed exceptions. No crate-root overrides.
  - `cargo-berth` has no `lib.rs` — it is a pure binary crate; `main.rs` owns the `mod` declarations.
  - The ledger/journal is **append-only** and lives in the repository's common git directory, shared identically across every linked worktree. Never rewrite a journal record; correct state by appending a new one.
  - `docs/cargo-berth/json-contract.md` is the stable wire contract. Additions arrive as new variants; fields are never renamed or removed. Any phase changing serialized shape updates it in the same phase.
  - `edit_blocking_status` ends as a computed projection of `(lifecycle, integration_status)`, never independently stored authoritative state (Phase 1).
  - `Released` is a terminal lifecycle: no `resnapshot` reopening, ever. Settled with the user 2026-08-26 — lost integration evidence raises an alert and never re-arms a block.
  - The canonical hook shims under `~/.claude/scripts/berth/` are outside this repository but are part of the delivered contract; a phase that changes engine output updates them in the same phase.
  - Git subprocess counts on the PostToolUse path must not scale with the number of paths, commits, or reservations involved.

## Phases

### Phase 1 — Computed edit-blocking status; `Released` is terminal  · status: done

#### As-built

`Reservation` no longer retains an `edit_blocking_status` field. `Reservation::edit_blocking_status()` (`reservation/mod.rs:1281`) is a `const` projection of lifecycle: `Active` is `Blocking`, `Outstanding` defers to `IntegrationEvidenceStatus::edit_blocking_status()`, and `Released` is `Clear` unconditionally. `conflicts_with_holders` (~L1104) and board serialization both call it; the blocking filter runs before either identity predicate, so a `Clear` holder is dropped before foreignness is consulted.

`Released` is terminal. `ReservationLifecycle::resnapshot` rejects both `Active` and `Released` with `LifecycleTransitionError::ResnapshotRequiresOutstanding` ("resnapshot requires an outstanding reservation"). `apply_widen` rejects a widen outside `Active` with `ReservationReplayError::WidenRequiresActive`. `apply_resnapshot` returns early for a `Released` reservation so legacy release-then-resnapshot journals replay to `Released` without reopening.

`BoardReservationVisibility::ReblockedActiveConstraint` remains as a reserved v1 wire value that no fresh state produces; `reservation_visibility` (`board/mod.rs`) maps every `Released` reservation to `ResolvedAudit` and is `const` after the unreachable arm was deleted. The v1 `edit_blocking_status` journal field is retained for audit but is not authoritative on replay: a journaled `Released` + `Blocking` contradiction replays to an effective `Clear`.

**Files:**
- `crates/cargo-berth/src/reservation/mod.rs` — computed `edit_blocking_status()` (~L1281); `ReservationEvidenceState` (~L211) and `evidence_state()` (~L1306); `apply_widen` active-only gate; replay tests `replay_retains_active_outstanding_released_and_rewritten_states` (~L1580), `replay_ignores_a_journaled_blocking_status_after_release` (~L1660), `replay_rejects_widen_after_release` (~L1690), `an_active_reservation_blocks_another_worktree_and_never_its_own` (~L1866)
- `crates/cargo-berth/src/reservation/lifecycle.rs` — `resnapshot` accepts only `Outstanding`; `ResnapshotRequiresOutstanding`; `EditBlockingStatus` documented as the effective decision derived from lifecycle and evidence
- `crates/cargo-berth/src/board/mod.rs` — `const fn reservation_visibility`; released rows are always `ResolvedAudit`
- `crates/cargo-berth/src/reconcile.rs`, `crates/cargo-berth/src/verb/release.rs` — no longer journal a recomputed blocking status as authoritative
- `crates/cargo-berth/tests/board.rs` — `release_dispositions_remain_resolved_when_trunk_rewrites` (~L586) asserts `resolved_audit`/`clear` across dispositions (~L691-701)
- `docs/cargo-berth/json-contract.md` — `edit_blocking_status` recorded as informational on replay; `unconstrained_reservations` documented as `active` and `outstanding` rows only

**Binds later work:** `edit_blocking_status` is computed, never stored — any consumer populating it does so by calling the method. A `Released` reservation reports `Clear` unconditionally, so no gate may key repair eligibility on its blocking status; `ReservationEvidenceState` and `evidence_state()` already supply the four lifecycle classifications (`Active`, `Outstanding`, `Released`, `ReleasedWithoutCheckpoint`) with their protected tips, and are the intended source for both the lost-evidence repair gate and the named-reservation lifecycle query. `ReleaseDisposition::revalidation_subject()` returns `ReleaseRevalidationSubject::None` for `Abandoned` and `RetiredOrphan`, which is what separates a lost Git proof from a disposition that never had one. `WidenRequiresActive` and `ResnapshotRequiresOutstanding` are new replay failures that currently collapse into one untyped `ledger_unreadable` envelope, and `reblocked_active_constraint` must stay accepted as a reserved wire value while never being emitted — both belong to the generated status/exit-code/payload contract.

**Gotchas:** `apply_widen`'s `Active` requirement fails the whole replay of an append-only journal, so a pre-existing post-release widen would be unrecoverable. Every `Widen` append site is already gated on `Active` (`drift/selection.rs:70,146,157`; `verb/claim.rs:865` via `own_active_reservations`) and the live journal contains none, but a new append path must preserve that gate. Coverage can be lost by changing a fixture's lifecycle rather than by deleting an assertion: the self-blocking guard was carried by a test whose subject this phase made permanently `Clear`, so the blocking filter dropped it before the identity predicates ran and it passed while checking nothing.

**Ruled out:** Guarding `apply_evidence` instead of removing the retained field — it leaves the invalid `Released` + `Blocking` pair constructible from every other writer and from replay. Reopening a released reservation when its integration evidence is lost — settled with the user 2026-08-26: vanished work is announced, not gated, because a wrong evidence verdict otherwise becomes a permanent phantom blocker with no operator path to clear it.

### Phase 2 — Scoped patch equivalence as integration proof  · status: done

#### As-built

- Integration proof is scoped patch equivalence, not commit identity: when a reservation's protected tip stops being an ancestor of trunk, the change the reservation made inside its own scopes — measured from `phase_start_head` — is compared against current trunk history. An amended, rebased, or squashed commit whose scoped content survives still certifies; the same paths carrying different content do not.
- `IntegrationEvidenceStatus::Integrated { trunk_oid, proof }` carries `proof: IntegrationProof`, with variants `ProtectedTipAncestor` and `ScopedPatchEquivalent`. Records written before the field existed decode as `ProtectedTipAncestor` via `#[serde(default)]` on `lifecycle.rs`.
- `integration_status` and `outstanding_integration_status` (`reservation/evidence.rs`) take `phase_start_head` and a `ReservationScopeSet`. The ancestry-success path issues no extra git subprocess; the ancestry-failure fallback batches every scope into one comparison composed of merge-base, rev-list, tree/index, merge-tree, and diff — roughly a dozen git invocations per evaluation, run once per retained reservation during reconciliation.
- Every one of those subprocesses routes through the typed `GitCommandExecution` boundary, so a git that could not start stays distinct from a git that ran and answered no: merge-base exit 1 (unrelated histories) and merge-tree exit 1 (conflict) both resolve to a definitive `Different`, never `Unavailable`.
- `apply_widen` resets an `Outstanding` reservation's `integration_status` to `NotIntegrated` whenever its scopes grow, so a widened reservation blocks again until re-proven.
- First-touch claim acquisition (`verb/claim.rs`) records `TrunkObservationAtClaim` — a resolved commit or an unresolvable reference — persisted in the claim record and threaded through the ledger and reservation replay. A released reservation with an unresolvable trunk reference answers `clear` instead of failing with `ledger_unreadable`.

**Files:**
- `crates/cargo-berth/src/reservation/evidence.rs` — the scoped equivalence predicate and both status entry points
- `crates/cargo-berth/src/reservation/lifecycle.rs` — `IntegrationProof`; `Integrated` carries it
- `crates/cargo-berth/src/reservation/mod.rs` — `apply_widen`'s evidence reset; `TrunkObservationAtClaim` in replay
- `crates/cargo-berth/src/git/mod.rs`, `git/command.rs`, `git/constants.rs` — the batched content query behind `GitCommandExecution`
- `crates/cargo-berth/src/ledger/journal.rs`, `ledger/mod.rs` — `TrunkObservationAtClaim` in the claim record
- `crates/cargo-berth/src/verb/claim.rs` — records the trunk observation at first-touch acquisition
- `crates/cargo-berth/src/reconcile.rs`, `crates/cargo-berth/src/verb/release.rs` — pass scopes and phase start into the status calls
- `docs/cargo-berth/json-contract.md` — `IntegrationProof` variants and the untagged `trunk_at_claim` union

**Binds later work:** `apply_widen`'s reset to `NotIntegrated` on scope growth must be preserved by any later cache or persisted proof built on `IntegrationEvidenceStatus`, or a stale `Clear` verdict will extend to scope the proof never checked. `IntegrationEvidenceStatus::Integrated`'s `proof` field is not yet asserted anywhere in `tests/board.rs` — the board's JSON rendering of `integration_evidence.status` still needs that coverage.

**Gotchas:** `verify.sh test <package>` resolves targets through `target_flags`, which selects only lib and bin targets — it cannot see `crates/cargo-berth/tests/`, so a scoped package test run reports green while integration suites go unrun. `GitCommandExecution` is `pub(super)` in `git/command.rs` and unreachable outside the `git` module; `drift/git_output.rs` and `verb/claim.rs` still spawn git directly and cannot express the spawn-failure distinction.

**Ruled out:** Path existence as the fallback predicate — it accepts a file whose reservation edits were later removed. Whole-blob equality as the fallback predicate — it rejects proof the moment trunk legitimately edits the same file again. The checkpoint trunk snapshot as the patch baseline — it would attribute trunk's own concurrent commits to the reservation. Rewriting `tests/lifecycle.rs` to match a `ledger_unreadable` answer — the wrong component was named; the defect was fixed at its cause in first-touch acquisition instead.

### Phase 3 — Cache the equivalence proof against what proved it  · status: done

#### As-built

The scoped-patch content proof is cached durably against the pair that produced it. `IntegrationProofSubjectRevision` (`reservation/mod.rs:75`) versions the baseline, protected content, and scopes a proof was checked under, and advances on `Widen`, `Resnapshot`, and release-disposition replacement — never on ordinary revalidation, which is why the pre-existing `advance_revision` counter could not serve as the key. `ScopedPatchEquivalenceCache` (`reservation/mod.rs:122`) retains definitive `ScopedPatchEquivalenceVerdict` values — `Integrated`, `NotIntegrated`, `TrunkRewritten` — for the two most recent targets, and `ScopedPatchEquivalenceCacheLookup` answers `Hit(DurableScopedPatchComparison)` or `Miss`. An `ObjectUnknown` comparison is never cached: it is a transient environment fact, and storing it would make one failed subprocess durable across restarts. `ScopedPatchEquivalenceChecked` and `ScopedPatchComparisonAttempted` (`ledger/journal.rs:254`, `:265`) carry the cache and its schedule into the append-only journal, so both survive a restart from replay alone.

Reconciliation admits **one** scoped comparison per trunk target per pass, through `ScopedPatchEvaluationMemo` (`reconcile.rs:328`). Targets that lose the slot are scheduled round-robin by `ScopedPatchEvaluationPriority` (`reservation/mod.rs:137`) over a bounded attempt history, so a subject that was skipped is preferred next pass and a subject whose comparison keeps returning `ObjectUnknown` cannot starve the others.

A deferral is not neutral. When the comparison slot is spent, `DeferredScopedPatchIntegrationStatus` (`reservation/evidence.rs:67`) decides what the materialized evidence still proves: `StillValid` only for a `ScopedPatchEquivalent` proof bound to the trunk actually observed, `Degraded` to `NotIntegrated` for a protected-tip proof reachability has just refuted and for an equivalence proof bound to an earlier target. Degradation is durable — it appends `EvidenceRevalidated` before the schedule update, so the correct in-memory answer is the one that replays.

`git::reachability` is batched through `BatchedIntegrationReachability`: the per-reservation form spends one subprocess per reservation and breaks the plan-wide invariant that PostToolUse git subprocess counts must not scale.

**Files:**
- `crates/cargo-berth/src/reservation/mod.rs` — `IntegrationProofSubjectRevision` (L75), `ScopedPatchEquivalenceVerdict` (L84), `DurableScopedPatchComparison` (L94), `ScopedPatchEquivalenceCache` (L122), `ScopedPatchEquivalenceCacheLookup` (L128), `ScopedPatchEvaluationPriority` (L137), the attempt-history schedule (L154), and three new `ReservationReplayError` variants (L1716)
- `crates/cargo-berth/src/reservation/evidence.rs` — `IntegrationEvidenceObservation` and `DeferredScopedPatchIntegrationStatus` (L57, L67)
- `crates/cargo-berth/src/reservation/constants.rs` — new; `SCOPED_PATCH_TARGET_RETENTION_LIMIT = 2`
- `crates/cargo-berth/src/reconcile.rs` — cache consultation, the per-target comparison budget (L328), and the durable append ordering
- `crates/cargo-berth/src/ledger/journal.rs` — `ScopedPatchEquivalenceChecked` (L254) and `ScopedPatchComparisonAttempted` (L265) and their replay
- `crates/cargo-berth/src/git/mod.rs` — batched reachability; `reachability_to_target` (L1182) answers a slice of candidate ancestors in one invocation
- `crates/cargo-berth/src/edge/graph.rs`, `crates/cargo-berth/src/gate/mod.rs` — replay and gating handle the new journal operations
- `crates/cargo-berth/tests/board.rs`, `crates/cargo-berth/tests/gate.rs` — the cache, deferral, scheduling, and non-scaling fixtures
- `docs/cargo-berth/json-contract.md` — the persisted proof record and the new journal operations

**Binds later work:**
- The proof-subject and cache model is the one later phases reuse rather than reinvent: key on `(target, IntegrationProofSubjectRevision)`, cache only definitive verdicts, journal them, replay them. A successor-equivalence cache needs its own key because it compares against a successor head rather than trunk.
- **The comparison budget is one per target, which is not a bound on a phase with many targets.** Any later phase whose targets multiply — successor heads, distinct phase-start anchors — must batch across targets or persist its own fixed-budget schedule.
- The non-scaling standard is **exact argv equality**, not a sublinear trend: `distinct_cold_proof_subjects_are_bounded_to_one_git_evaluation_per_target` (`tests/board.rs:2254`) asserts 14 git argv for an equivalent cold check, 13 for a different one, identical command-name sequences at one reservation and at twenty, and zero `merge-base --is-ancestor` calls.
- `git::reachability_to_target` (`git/mod.rs:1182`) is the batched ancestry primitive; a per-anchor loop over it reintroduces the scaling.
- A released, Git-backed reservation degraded to `NotIntegrated` is reachable in ordinary operation — `deferred_comparison_rejects_a_refuted_ancestor_proof` (`tests/board.rs:2026`) constructs one — and currently has no `--integrated-as` repair path.
- Three replay hard stops — `IntegrationProofSubjectRevisionExhausted`, `ActiveScopedPatchComparison`, `IntegrationProofSubjectMismatch` — join the roughly twenty `ReservationReplayError` variants that all collapse into `ledger_unreadable`, exit 4, and a `NoFacts` payload.
- `ScopedPatchEquivalenceChecked` records one event per `(reservation, trunk target)` and fires even when the reported status does not change, raising the rate at which the journal grows.

**Gotchas:**
- `ReservationLifecycle::Released` maps to `EditBlockingStatus::Clear` unconditionally (`reconcile.rs:1256`), so no `Released` fixture can demonstrate an edit-blocking regression. A fixture proving blocking behaviour must leave the reservation `Outstanding`.
- `EvidenceRevalidationObservation::PreserveMaterialized` early-returns before the journal write. Reporting a degraded status in memory is not the same as persisting it; the `EvidenceRevalidated` append must precede the schedule update or the stale affirmative proof is what replays.
- A fixture edited until it passes stops proving its property. Four did so here — call counts asserted without statuses, a helper filtering out the calls that had begun to scale, a gate fixture whose actual trunk was answerable by cheap reachability, and a `Released` fixture that could not show blocking status. Every assertion change needs to say what it still proves.

**Ruled out:**
- Reusing `advance_revision` (`reservation/mod.rs:1240`) as the cache key — it advances on evidence and lifecycle events, invalidating a still-valid proof on every revalidation.
- Caching only the positive verdict — `TrunkRewritten` costs the same to recompute and is what a rewritten reservation actually holds.
- Caching an `ObjectUnknown` comparison — it would make one failed subprocess durable across restarts.
- A bare `Option<T>` for the stored verdict — never evaluated and evaluated-as-different are different facts, and an optional states neither.
- Copying materialized evidence through a deferral unchanged — it re-affirmed proofs reachability had just refuted, and proofs bound to a trunk target that no longer contained the work. Both are false positives, the class this plan ranks strictly worse than a false negative.

### Phase 4 — Successor edges use scoped patch equivalence  · status: done

#### As-built

A successor holding equivalent rewritten content is `Fulfilled` even without the predecessor's original protected tip. `SuccessorIncorporationEvidence` (`ProtectedTipAncestor`, `ScopedPatchEquivalent`, `NotIncorporated`, `ObjectUnknown`) and `PredecessorSuccessorIncorporation` (per-predecessor, including `QueryFailed`) replaced `SuccessorHeadReachability`, which no longer exists — a type defined as containment must not carry a value that is not containment. `Edge::readiness` only reads the snapshot; the Git work is `reconcile.rs::successor_incorporation_evidence`, which assembles every predecessor's subject into one `git::descendant_commits` call. Both bounding strategies ship, on different axes: successor heads batch into a single invocation, and a fixed-budget round-robin admits exactly one cold scoped comparison per reconciliation, ordered by a persisted attempt generation so a transient `ObjectUnknown` head rotates behind the others rather than starving them; a deferred head keeps reporting `AwaitingSuccessorIncorporation` — a deferral never reads as incorporation. Verdicts persist in a successor-equivalence cache keyed by the predecessor's `IntegrationProofSubjectRevision` and the successor head (distinct from the trunk-keyed cache), journalled as `SuccessorScopedPatchEquivalenceChecked` and `SuccessorScopedPatchComparisonAttempted` and replayed on restart. Four costs that scaled are now one invocation each regardless of count: predecessor ancestry, worktree ahead/behind, retention-ref availability, retention-ref repair.

**Files:**
- `crates/cargo-berth/src/edge/snapshot.rs` — `SuccessorIncorporationEvidence` (L69), `PredecessorSuccessorIncorporation` (L82)
- `crates/cargo-berth/src/edge/mod.rs` — `Edge::readiness` (L316) consults the snapshot's incorporation evidence
- `crates/cargo-berth/src/edge/graph.rs` — replay ignores the two successor-cache operations
- `crates/cargo-berth/src/reconcile.rs` — `successor_incorporation_evidence`; shared successor evaluation budget (L342); `GateReconciliation::into_committed_hook_operations` (L875) retains the successor-cache operations; the per-reservation retention deletion loop (L1699) is still unbatched
- `crates/cargo-berth/src/git/mod.rs` — `ahead_behind_for_heads` (L1158, one `rev-list --parents --ignore-missing --stdin` over trunk plus all heads; `Unrelated` is exactly disjoint ancestor sets), batched descendant classification (L1297), `DescendantCommitQuery` (L1531), `repair_reservation_retention_refs` (L1410), `delete_reservation_retention_ref` (L1457)
- `crates/cargo-berth/src/git/constants.rs` — `GIT_IGNORE_MISSING_ARG` (L49)
- `crates/cargo-berth/src/reservation/mod.rs` — the successor-equivalence cache, its lookup, the comparison-attempt schedule
- `crates/cargo-berth/src/reservation/constants.rs` — `SUCCESSOR_SCOPED_PATCH_TARGET_RETENTION_LIMIT = 512` (L8), independent of the trunk-target limit of 2
- `crates/cargo-berth/src/ledger/journal.rs` — the two operations (L275, L286) and their replay
- `crates/cargo-berth/src/board/mod.rs` — consumes the batched ahead/behind vector, mapped back to sorted worktree ids
- `crates/cargo-berth/tests/edges.rs` — successor fixtures, `predecessor_graph_has_fixed_cold_cost` (L766)
- `crates/cargo-berth/tests/board.rs` — cost assertions pinning the batched command shapes (L1701)
- `docs/cargo-berth/json-contract.md` — both operation records

**Binds later work:** `SuccessorHeadReachability` is gone; the shipped role-bearing names are `SuccessorIncorporationEvidence`, `PredecessorSuccessorIncorporation`, and `DescendantCommitQuery` — the semantic-roles rename phase must cover both halves or the trunk and successor sides disagree. Retention-ref repair now arrives as one batched `update-ref --stdin` transaction, which the hook-suppression phase must own. Two journal records were added, not one, for the generated-contract phase. The one-versus-twenty raw-trace standard is the gate the fixed-subprocess-count-for-drift-provenance phase must adopt; equal subprocess counts do not imply a stable wall clock, so the PostToolUse latency phase still needs shallow/deep/divergent cells at both cardinalities.

**Gotchas:**
- Prove non-scaling on a raw unfiltered argv total, never an allowlisted one — the old allowlist let `cat-file --batch-check`, `rev-parse`, `for-each-ref`, and plain `rev-list` escape, hiding 31 calls at twenty subjects against 12 at one. The standard is exact equality at one and at twenty, not a sublinear trend.
- `rev-list --stdin` exits 128 on a single unknown object, blanking every result; `--ignore-missing` plus a per-item membership check confines the damage to the item actually missing. `AheadBehind::Unavailable` is per-head as a result (regression fixture at `git/mod.rs:1767`).
- `verify.sh test <package>` does not cover integration suites — a green package report coexisted with a failing `tests/board.rs`. Name each integration suite explicitly.
- The 512 successor retention limit and the unbounded `maximum_reservations` config are unconnected: unreachable at the default of 128, but above ~513 the round-robin cannot close over the head set. Conservative in direction — a hold, never a false release.
- Two batched queries build the same flags in different orders (`--ignore-missing --parents` vs `--parents --ignore-missing`) and `tests/board.rs` pins both spellings exactly; normalize when the file is next touched.

**Ruled out:** an equivalence variant on `SuccessorHeadReachability` (containment type carrying a non-containment value); bounding only per target (twenty heads are twenty targets); treating the 512-entry bound as a defect here (unreachable at default, conservative, and a bounded constant was required).

### Phase 5 — Lost-evidence alert and `--integrated-as` eligibility  · status: todo

#### Work Order

**Goal:** Lost integration evidence is visible to a hook-only agent, now that it no longer blocks, and its repair path covers every eligible row.

**Spec:**

**1. `--integrated-as` eligibility already keys on lost Git evidence — audit what is left.** This half of the phase was largely absorbed while Phase 2's trunk-observation defect was being fixed. `recovery_operation` (`recovery.rs:520`) already resolves `reservation.evidence_state().map_err(RecoveryRejection::Replay)?` and matches:

```rust
ReservationEvidenceState::Released {
    disposition: superseded,
    integration_status:
        IntegrationEvidenceStatus::TrunkRewritten
        | IntegrationEvidenceStatus::ObjectUnknown,
    ..
} if !matches!(
    superseded.revalidation_subject(),
    ReleaseRevalidationSubject::None,
) => { ... }
```

It no longer tests blocking status, it rejects a non-Git disposition through `revalidation_subject` (`reservation/lifecycle.rs:192`), and both the success and the rejection paths have coverage. `ReleasedWithoutCheckpoint` is excluded by construction — it carries no `integration_status` at all.

**What remains is the `NotIntegrated` row, and Phase 3 settled it: admit the row.** The shipped matcher (`recovery.rs:567`) admits `TrunkRewritten` and `ObjectUnknown` but not `NotIntegrated`. That combination is no longer hypothetical — Phase 3 shipped the fixture that constructs it. `deferred_comparison_rejects_a_refuted_ancestor_proof` (`tests/board.rs:2027`, over `warmed_ancestor_proof_after_trunk_rewrite` at `tests/board.rs:3017`) releases a reservation with `disposition: {"kind": "integrated"}` — a Git-backed disposition whose `revalidation_subject` is not `None` — then amends the commit so the protected tip is no longer reachable, and asserts the released row reports `not_integrated`. A released, Git-backed, `NotIntegrated` reservation is therefore reachable in ordinary operation and currently has **no** `--integrated-as` repair path.

Admit `NotIntegrated` alongside `TrunkRewritten` and `ObjectUnknown` in that matcher, and cover it. The released row still reports `Clear` — Phase 1 made `Released` terminal and that does not change — and the lost-evidence alert of part 2 is what makes the row visible. Degradation on an **outstanding** reservation stays Phase 3's blocking surface and is not this phase's concern.

**2. Lost evidence must be visible.** `Alert` (`alert.rs:23`) carries only `OrphanedOutstanding`. PostToolUse validates that `.payload.alerts` is an array (`berth_post_bash.sh:34`) but `typed_drift_feedback` (~L172) never renders its entries, so a trunk rewrite detected mid-session is invisible to an agent that sees only hook output.

Add a typed alert carrying reservation id, protected tip, evidence status, and a typed resolution action. Render it when first detected by PostToolUse, and persist it on the board and at SessionStart.

**The alert cannot require a trunk oid.** `RepositoryTrunk::ObjectUnknown` means exactly that no current trunk object resolved, yet `--integrated-as <trunk-oid>` needs one — so a single payload carrying a mandatory `trunk_oid` is unconstructible for the very case it must report, and a bare `Option<GitObjectId>` would state neither state's meaning. Split the payload:

```rust
enum LostEvidenceRecovery {
    /// Trunk resolved; the operator can confirm it carries the released work.
    VerifyResolvedTrunk { trunk_oid: GitObjectId, action: RecoveryAction },
    /// No trunk object resolved; trunk must be resolved before any repair.
    ResolveTrunkFirst { action: RecoveryAction },
}
```

Human text, substituting real ids — `VerifyResolvedTrunk`:

> INTEGRATION EVIDENCE LOST: released reservation `<id>` remains non-blocking, but trunk `<trunk-oid>` no longer proves protected tip `<tip>`. If trunk `<trunk-oid>` contains the released work, run `cargo-berth resolve <id> --integrated-as <trunk-oid>`. Otherwise restore the work first. Inspect `cargo-berth board --json`.

and `ResolveTrunkFirst`:

> INTEGRATION EVIDENCE LOST: released reservation `<id>` remains non-blocking, and trunk does not currently resolve to a known object, so protected tip `<tip>` cannot be proved either way. Resolve trunk first, then rerun. Inspect `cargo-berth board --json`.

**The inspection command is plain `board --json`, not `board --reservation`.** The `--reservation` selector arrives in Phase 13, which follows Phase 12, which extends this phase's alert rendering — naming it here would make the three phases circular. A released reservation is not among the rows the board omits (`board/mod.rs:625` omits a waiting successor and either endpoint of an unresolved overlap), so `board --json` already shows it and the alert loses nothing.

**3. The alert must be generated from current evidence, not a pre-reconciliation clone.** `build_plan` clones each `AlertSubject` before the current repository evidence is computed, and commit-time alert generation reads those clones — so a rewrite detected during an invocation would go unreported until some later command. Generate the alert from the current repository snapshot or from post-append replay, and require it in the **first** drift envelope that detects the loss, not a subsequent one.

**Files:**
- `crates/cargo-berth/src/alert.rs` — the new typed alert variant and `LostEvidenceRecovery`
- `crates/cargo-berth/src/reconcile.rs` — generate the alert from post-reconciliation evidence, not a pre-computation clone
- `crates/cargo-berth/src/recovery.rs` — `--integrated-as` eligibility keys on lost Git evidence via `evidence_state` (`recovery_operation` L520, the admitted-status matcher L567)
- `crates/cargo-berth/src/output.rs` — alert attachment and rendering (~L1545)
- `crates/cargo-berth/src/board/mod.rs` — the alert persists on the board
- `crates/cargo-berth/tests/board.rs` — board-visible alert fixtures
- `crates/cargo-berth/tests/drift.rs` — the drift-envelope alert fixture
- `crates/cargo-berth/tests/liveness.rs` — the alert survives across invocations
- `/Users/natemccoy/.claude/scripts/berth/install/hooks/berth_post_bash.sh` — `typed_drift_feedback` renders `.payload.alerts` (~L172)
- `/Users/natemccoy/.claude/scripts/berth/install/hooks/berth_session_start.sh` — surfaces outstanding alerts
- `docs/cargo-berth/json-contract.md` — the new alert variant

**Constraints from prior phases:** Phase 1 made `Released` terminal and `edit_blocking_status` computed, so `Released` always reports `Clear` unconditionally — this is exactly why the `recovery_operation` gate keys on `ReservationEvidenceState` instead, which it already does. Phase 1 also added `ReservationReplayError::WidenRequiresUnreleased` (the variant is named for the release boundary, not the active one — Phase 2 widened it to accept `Outstanding`) and renamed `ResnapshotRequiresGitEvidence` to `ResnapshotRequiresOutstanding`; `evidence_state` returns a replay error, so the repair path must map it rather than assume success. Phase 1 further made legacy release-then-resnapshot records replay without reopening — those records raise this alert. Phase 2 added `IntegrationProof` and the `ScopedPatchEquivalent` verdict; the alert fires only when *no* proof is available, not when equivalence proved integration. Phase 3 added the deferred-comparison path: when the bounded comparison slot is spent, a stale or refuted affirmative proof degrades to `NotIntegrated` rather than being preserved (`DeferredScopedPatchIntegrationStatus`, `reservation/evidence.rs:67`). That degradation is what makes a released, Git-backed, `NotIntegrated` row reachable, and it is why this phase admits that row. Phase 4 added two successor-cache journal operations that `build_plan` now emits; derive the lost-evidence alert from post-append replay and do not drop or filter those operations on the way through.

**Acceptance gate:** **Every `Test` command in Delegation Context** green. A fixture releases a reservation `integrated`, rewrites trunk so no proof survives, and asserts: `edit_blocking_status` stays `Clear`; the typed alert appears in `board --json` and in **the first** drift envelope that detects the loss; and `cargo-berth resolve <id> --integrated-as <oid>` succeeds. Separate fixtures cover an unresolved trunk (`ObjectUnknown` → `ResolveTrunkFirst`, and `--integrated-as` unavailable), an unresolved protected tip, and a legacy release-then-resnapshot record replaying to the alert without reopening. The PostToolUse shim renders both recovery variants' text. The eligibility half is complete when a fixture shaped like `deferred_comparison_rejects_a_refuted_ancestor_proof` drives a released, Git-backed, `NotIntegrated` reservation through `cargo-berth resolve <id> --integrated-as <oid>` and it succeeds, with the row reporting `Clear` and raising the lost-evidence alert on the **first** invocation that detects it; the already-covered `Abandoned` and `RetiredOrphan` rejections need no new fixture.

### Phase 6 — Worktree identity: reproduce first, then one helper  · status: todo

#### Work Order

**Goal:** Every operation is journalled against the worktree that ran it — with the actual cause demonstrated before anything changes.

**Spec:**

**Reproduce before fixing. The stated diagnosis is a hypothesis and the source does not support it.**

Observed: a `resolve` invoked from `/Users/natemccoy/rust/cargo-tile-favorites` was journalled with the **main checkout's** worktree and run ids. The original report attributed this to the resolve path reading `$GIT_COMMON_DIR/cargo-berth-*-id` where the incursion path reads `$GIT_DIR/cargo-berth-*-id`, and explicitly labelled that "hypothesis, not verified in source".

Review found the code already does the right thing. `execute_one_incursion_resolution` (`recovery.rs:264`) reads the actor through:

```rust
let worktree_context = WorktreeContext::discover(&invocation_directory)?;
let worktree_identity = ledger::worktree_identity(
    worktree_context.administrative_directory(),
    worktree_context.worktree_kind(),
)?;
```

and its run identity passes the administrative directory separately from the shared ledger directory (`recovery.rs:632-636`). `WorktreeContext::discover` (`ledger/mod.rs:272`) already handles coincident main-checkout directories, linked-worktree `.git` files plus `commondir`, and separate administrative directories.

**So a linked-worktree fixture written against the hypothesis would pass while the observed misattribution stays unexplained.** Step one of this phase is a reproducer matching the incident's invocation directory, command route, session environment, and git environment, failing before any fix. Appendix A Defect 2 in `berth-fix-evidence.md` has both journal events. If it does not reproduce, find the real cause before changing anything — do not ship a fixture that passes against a cause that was never demonstrated.

Then, whatever the cause: expose identity resolution as **one** entry point so a call site cannot transpose two `&Path` arguments.

```rust
struct WorktreeAdministrativeDirectory(PathBuf);   // per-worktree
struct SharedLedgerDirectory(PathBuf);             // common git dir

fn resolve_identity(context: &WorktreeContext) -> Result<CoordinationIdentity, LedgerError>
```

Both wrapper types live inside `WorktreeContext`; callers never construct them. No trait, no generic.

Preserve filesystem discovery when git environment variables are absent — the current behaviour works outside hook-launched commands and must not be replaced by environment-dependent behaviour. State the precedence for supplied relative values. Retain the existing bare-repository `RepositoryNotFound` rejection.

**Files:**
- `crates/cargo-berth/src/ledger/mod.rs` — the wrapper types, `resolve_identity`, `WorktreeContext` (~L102, ~L272)
- `crates/cargo-berth/src/recovery.rs` — use the single entry point (~L264, ~L632-636)
- `crates/cargo-berth/src/drift/identity.rs` — same
- `crates/cargo-berth/tests/ledger.rs` — the reproducer and the identity fixtures

**Constraints from prior phases:** None binding — this phase is independent of Phases 1–5.

**Acceptance gate:** **Every `Test` command in Delegation Context** green. A reproducer exists that fails before the fix and passes after, **or** the phase reports that the hypothesis did not reproduce and names the demonstrated cause. Fixtures cover main checkout, linked worktree, separate git dir, submodule, unset and relative environment variables, and the retained bare-repository rejection. Every command fixture asserts the journalled actor equals the invocation worktree.

### Phase 7 — Report a resolve by what it accomplished  · status: todo

#### Work Order

**Goal:** A resolve that leaves the incident resolved and this caller responsible exits 0 with a payload naming what it did.

**Spec:**

Observed: a resolve of a live incident returned `exit_code: 5`, `status: invalid_input`, "incursion incident … is already resolved", while the journal recorded exactly one `resolve_incursion` event within seconds of that call. The caller cannot distinguish "you already did this" from "another actor did this" from "this succeeded and is being described badly", and exit 5 makes any wrapper treat a completed resolve as a failure.

`LedgerTransactionOutcome::Appended` already proves the current invocation wrote the resolution and already returns success, so the work is entirely in the **pre-existing-resolution** branch — the `else` arm at `recovery.rs:287-298` that yields `RecoveryRejection::IncursionIncidentAlreadyResolved`, not the `Display` text at `recovery.rs:816`.

`IncursionIncidentStatus::Resolved` (`reservation/mod.rs:412`) retains only:

```rust
Resolved { resolution_event_id: EventId, resolved_at: RecordedAt }
```

It does **not** retain the resolving actor, so either retain it or resolve `resolution_event_id` back to its `JournalEvent` (`ledger/journal.rs:104` has `JournalActor`).

Responsibility means **equality of the correctly resolved worktree and coordination-run ids** — not the same process invocation, which the journal cannot express. Outcomes:

- Wrote it now → exit 0, payload `recorded_now`.
- Already recorded by the same coordination actor → exit 0, payload `already_recorded_by_same_coordination_actor`.
- Recorded by a genuinely different actor → keep `invalid_input`/exit 5, and name that actor's worktree id, run id, event id, and resolution time in typed payload fields.

Gate the PostToolUse `STOP. Resolve with …` text on the incident's live resolved state, so it stops naming a command that has already succeeded.

Exit-code safety is already verified: the three hook shims invoke `check`, `drift`, and `board` rather than `resolve`, and `claim_state.py` already maps `incursion_resolved`/`resolve` to exit 0 and `invalid_input` to exit 5.

**Files:**
- `crates/cargo-berth/src/recovery.rs` — the three outcomes (~L104, ~L264, ~L287-298)
- `crates/cargo-berth/src/reservation/mod.rs` — `IncursionIncidentStatus::Resolved` retains the actor (~L414)
- `crates/cargo-berth/src/ledger/journal.rs` — actor lookup by event id if not retained
- `crates/cargo-berth/src/output.rs` — the two new success payloads and the enriched rejection
- `/Users/natemccoy/.claude/scripts/berth/install/hooks/berth_post_bash.sh` — STOP text gated on live resolved state
- `docs/cargo-berth/json-contract.md` — the new payload variants

**Constraints from prior phases:** **Depends on Phase 6.** Deciding "this caller is responsible" compares worktree and run ids, which is exactly the identity Phase 6 establishes is resolved through `resolve_identity(&WorktreeContext)`. Until that lands, a resolve issued from a linked worktree can be attributed to the main checkout and misclassified as foreign, preserving the original exit-5 failure in the very worktrees where it was observed.

**Acceptance gate:** **Every `Test` command in Delegation Context** green. **Linked-worktree** fixtures for first resolve (exit 0, `recorded_now`), same-actor repeat (exit 0, `already_recorded_by_same_coordination_actor`), and foreign-actor repeat (exit 5, `invalid_input`, foreign actor named in the payload). The PostToolUse shim emits no STOP text for an incident already resolved.

### Phase 8 — Git-hook phase/ref dispatch table  · status: todo

#### Work Order

**Goal:** The generated `reference-transaction` script stops spawning the binary for events berth does not act on, without dropping any event it does.

**Spec:**

A 3-commit rebase in this repository costs 0.23s with hooks disabled and 7.97s with them live; the same rebase in a hookless repository is 0.03s. Git delivers 75 `reference-transaction` invocations for those three commits, and `gate/install.rs:214 ManagedHook::script` spawns the binary for all of them before anything is known about whether they matter. `CARGO_BERTH_BYPASS=1` is not a fast path: its branch runs `cargo-berth __reference-transaction` on every invocation — 5.2 of the 5.44 seconds that mode costs — because the bypass binary call at `gate/install.rs:236` sits **above** the existing prepared-phase marker filter.

**The source report's premise was wrong and three reviewers caught it independently.** It claimed berth answers `Clear` for every phase except `prepared`. It does not. `evaluate_reference_transaction` (`gate/mod.rs:328`) runs `branch_rewrites` (~L437) on `Committed` — which reanchors active reservation phase starts after **any** local branch rewrite, and whose own comment says omitting it makes rebases look like newly authored work and creates false incursions — then `reanchor_rewritten_phases` (~L474) for detected rewrites and `commit_forced_permit_audits` (~L566) for committed trunk updates. `tests/gate.rs:1167-1176` explicitly requires the `committed` phase and verifies permit consumption. A prepared-only filter would leave stale phase anchors and reusable forced-integration permits. The "about 1 invocation" target is invalid.

Implement a **phase/ref dispatch table** in the generated script:

| Phase | Action |
|---|---|
| `preparing`, `aborted`, unknown | exit before the binary |
| `prepared` | invoke only when the transaction names the configured trunk ref **exactly** |
| `committed` | invoke when the transaction names any local `refs/heads/*` ref |

Required properties:

- **Preserve stdin bytes.** The current script passes git's stdin straight through and the binary consumes it with `read_to_string` (`cli.rs:1122-1126`). Any shell filter that reads stdin leaves EOF, and **empty input parses as a zero-entry transaction that permits** — so a naive `grep` filter could let a prepared trunk update bypass the gate. Copy stdin to a protected temporary file, inspect complete three-field records, then redirect the **unchanged bytes** into the binary.
- **Exact ref matching.** Compare the complete third field, never a substring — `refs/heads/main-old` must not satisfy a `main` trunk.
- **Malformed `prepared` input still reaches the binary.** Skipping it would convert the deliberate failure at `cli.rs:1068-1072` and the deliberate unconfirmed-bypass audit at `cli.rs:1148-1205` into silent success.
- **Apply the same filter to the bypass recording**, which currently runs unconditionally.
- **State how the embedded trunk ref is refreshed.** It is written at `init` (`cli.rs:994-999`) and goes stale when trunk is renamed.

**Files:**
- `crates/cargo-berth/src/gate/install.rs` — `ManagedHook::script`, the dispatch table, stdin buffering, the bypass filter (~L214-236)
- `crates/cargo-berth/src/cli.rs` — trunk-ref refresh path (~L994-999)
- `crates/cargo-berth/tests/gate.rs` — the fixtures below

**Constraints from prior phases:** No behavioral dependency on Phases 1–7. One boundary does bind: Phase 2 routed every git subprocess it added through a single typed execution boundary in `git/command.rs`, which is what keeps "git could not start" distinct from "git ran and answered no". Any git invocation this phase adds on the engine side goes through that boundary rather than spawning directly, and must preserve the same distinction.

**Acceptance gate:** **Every `Test` command in Delegation Context** green, including the existing `tests/gate.rs:1167-1176` committed-phase permit test. Fixtures cover: prefix refs (`main-old` vs `main`), trunk rename, detached HEAD, fetch/push remote refs, malformed records, **stdin byte preservation**, committed rebase reanchoring, and committed permit consumption. An instrumented no-op `reference-transaction` hook reports scenario-specific invocation counts for a prepared trunk update, a committed feature-branch rebase, and a committed forced trunk integration — not one universal number. At least ten no-hook, filtered-bypass, and filtered-live rebases report median and maximum wall time. A missing executable still permits with the printed warning.

### Phase 9 — Scoped hook suppression on retention-ref writes  · status: todo

#### Work Order

**Goal:** Berth stops gate-evaluating its own bookkeeping writes, without suppressing hooks on anything a user would expect to fire.

**Spec:**

`git_command` (`git/command.rs:30`) builds every subprocess as `git --no-optional-locks -C <root> …` with no hook suppression, so berth's retention-ref writes fire `reference-transaction` and berth gate-evaluates its own bookkeeping. This is a correctness point as much as a cost one: berth's internal ref writes are not user history.

**This phase also owns the deletion path, which still scales.** Phase 4 batched retention-ref *repair* into one `update-ref --stdin` transaction (`git::repair_reservation_retention_refs`, `git/mod.rs:1410`), but `ReconciliationAction::commit` still calls `git::delete_reservation_retention_ref` (`git/mod.rs:1457`) once per reservation in a loop at `reconcile.rs:1699`. That loop violates the plan-wide invariant at `berth-fix.md:90` and suppressing hooks on it would leave the subprocess scaling untouched. Combine reconciliation's repairs and deletions into **one** suppressed retention-ref transaction; do not suppress hooks on a per-reservation loop and call the phase done.

**Do not put the suppression in `git_command`.** Three reviewers found three separate harms from blanket suppression:

1. `hooks_directory()` (`git/mod.rs:238`) asks `git rev-parse --git-path hooks`, which is where hooks are installed — suppressing it breaks `cargo-berth init` hook discovery (`gate/install.rs:109`).
2. `git::update_local_branch` (`git/mod.rs:1015`) uses the same constructor and `cargo-berth integrate` (`verb/integrate.rs:114`) uses it to move trunk — suppressing it stops the committed transaction that consumes a forced-integration permit.
3. It would silently skip a user's **unmanaged** hook during a real trunk update. Berth deliberately preserves an unmanaged hook already occupying the hook path.

Apply hook suppression **only** to the private retention-ref writes and deletions in `git/refs.rs:36-75`, through an explicit hook-policy command mode. That is an in-process selection, no extra subprocess.

**Name the policy for what it means, not for how it is represented.** A boolean parameter or a bare optional setting states nothing about which writes may suppress hooks or why:

```rust
enum GitHookExecutionPolicy {
    Enabled,
    SuppressedForRetentionRef,
}
```

Every existing helper and every helper Phase 2 added stays `Enabled` by default; only retention-ref writes name `SuppressedForRetentionRef`.

**The suppression value is `/dev/null`, not the empty string.** Probed on both Homebrew git 2.55.0 and Apple git 2.50.1, an empty `core.hooksPath` resolves to `./`, so a repo-root executable named `reference-transaction` would still run; `/dev/null` resolves to `/dev/null`. The test must place such a repo-root sentinel, so suppression is proved rather than passing by the sentinel's absence.

**Files:**
- `crates/cargo-berth/src/git/command.rs` — `GitHookExecutionPolicy`; `git_command` default stays `Enabled` (~L30)
- `crates/cargo-berth/src/git/refs.rs` — retention-ref writes and deletions opt into suppression (~L36-75)
- `crates/cargo-berth/src/git/mod.rs` — `repair_reservation_retention_refs` (L1410) and `delete_reservation_retention_ref` (L1457) merge into one suppressed transaction
- `crates/cargo-berth/src/reconcile.rs` — the per-reservation deletion loop at L1699 becomes one batched call
- `crates/cargo-berth/tests/gate.rs` — the sentinel fixtures

**Constraints from prior phases:** Phase 2 added several git helpers behind the typed execution boundary in `git/command.rs`; all of them stay hook-enabled, and the suppression added here is opt-in per call site rather than a change to the shared constructor's default. Phase 8 rewrote the generated `reference-transaction` script to filter by phase and ref before spawning the binary. That reduces how often berth's own `update-ref` calls reach the binary but does not stop the hook from firing at all — this phase does. Measure the two independently. **Phase 4 already batched retention-ref repair** into a single `update-ref --stdin` transaction and batched the availability query that precedes it, so the "two `update-ref` calls per drift run" and "8 spawns per drift run" figures this Work Order was written against no longer hold; re-measure rather than quoting them. Phase 4 left the deletion path unbatched, which is why this phase now owns it. Phase 4 also established the measurement standard: a raw unfiltered argv trace, never an allowlisted one.

**Acceptance gate:** **Every `Test` command in Delegation Context** green. With an executable sentinel `reference-transaction` at the repository root: retention-ref writes and deletions record **zero** hook fires; `cargo-berth integrate` records **one** transaction lifecycle for the trunk update and consumes its forced permit; `cargo-berth init` discovers a configured custom hooks directory and installs successfully. A raw unfiltered git trace — every invocation's full argv, recorded before any classification — drives one reservation and twenty reservations through both a repair-only pass and a deletion-only pass; the total invocation counts and command sequences must be **equal** across the two cardinalities, with exactly one ref-mutating transaction per pass.

### Phase 10 — Fixed subprocess count for drift provenance  · status: todo

#### Work Order

**Goal:** `drift --full` stops spawning one git process per incursion path, without regressing the common small-path case.

**Spec:**

With a 33-path incursion outstanding, `drift --full` spawns 62 git subprocesses taking about 1.5s — `commits_for_paths` (`drift/provenance.rs:80`) calls `path_commits` (~L129) once per committed incursion path, and `commit_origin` (~L109) performs one ancestry query per unique commit. After the incursions were resolved the same command spawned 17, with `log` at zero. **The subprocess count is the incursion's path count**, so an incursion left open makes every later commit slower without bound.

**Batch with pathspecs retained, not dropped.** An unqualified `git log <base>..HEAD --name-only` walks and emits every changed path in the range and **regresses the common case**. Measured in this repository:

| Range | Current 1 path | Current 4 paths | Current 33 paths | Unfiltered bulk |
|---|---:|---:|---:|---:|
| 25 commits | 12 ms | 48 ms | 365 ms | 15 ms |
| 500 commits | 13 ms | 51 ms | 449 ms | 75 ms |

The unfiltered crossover is between one and four paths at 25 commits and about six paths at 500. A single batched command carrying **all** selected pathspecs took 11–13 ms over 25 commits and 13–21 ms over 500, including the one-path case. So:

```
git log -z --name-only --no-renames --format=<unambiguous-record-format> <range> -- <path1> … <pathN>
```

NUL-delimited so tab-, newline-, and non-ASCII-bearing paths parse losslessly and git path quoting cannot corrupt them — matching the encoding `observe_full` (`drift/observation.rs:289`) already uses via `drift/constants.rs:26`. Skip the command entirely when no entered path is also committed.

**State the merge-diff policy explicitly** — a merge commit emits no path names without one, and a conflict resolution can introduce the incursion.

**Validate the phase-start ancestry precondition, in one batched call.** When the phase start is not an ancestor of HEAD, `<phase_start>..HEAD` sweeps in unrelated commits reachable only from HEAD, and a fixed subprocess count would then produce wrong attribution faster. Return a typed stale-anchor result or reanchor before attributing; `drift/git_output.rs` owns that result type, since it already owns how a git invocation's outcome is typed for this module. Do **not** issue one ancestry query per phase start — `git::reachability_to_target` (`git/mod.rs:1246`) already takes a **slice** of candidate ancestors and one target and answers all of them in a single batched invocation. Reuse it; a per-anchor loop reintroduces exactly the scaling this phase exists to remove.

**One bad anchor must not poison the batch.** Phase 4 established this: `rev-list --stdin` exits 128 on a single unknown object, which blanked every result until `--ignore-missing` plus a per-item membership check confined the damage to the item actually missing (`git/mod.rs:1158`). Apply the same shape here — when one of twenty phase starts is missing, unrelated, or stale, the other nineteen reservations must still produce valid attribution, and only the bad anchor reports the typed stale-anchor result.

**Retire the bare optionality on the way through.** `trunk_object_id` (`drift/provenance.rs:73`) returns `Option<GitObjectId>`, and `commits_for_paths` and `commit_origin` thread that optionality onward, so "trunk did not resolve" and "this commit has no trunk origin" are the same value at every call site. Since both functions are being rewritten here anyway, replace the boundary with a semantic type rather than carrying `Option<T>` into the new code:

```rust
enum CommitOriginTrunk {
    Resolved(GitObjectId),
    CannotClassifyOrigin,
}
```

Keep the two `<base>` concepts distinct: the log range uses each reservation's **phase start**; commit-origin membership uses **trunk**.

**The bound is per drift invocation, not per phase-start/trunk pair.** A per-pair bound reads as fixed and is not: distinct phase starts grow with the number of reservations in the repository, so twenty reservations at twenty anchors buy twenty subprocess groups — which contradicts the plan-wide invariant that git subprocess counts on the PostToolUse path must not scale with the number of paths, commits, **or reservations** involved. Batch across anchors as well as across paths: one invocation covering every relevant phase start and every entered path per drift run. The union range `<earliest-common-ancestor-of-all-anchors>..HEAD` with the full pathspec set, attributed back to each reservation's own anchor in memory, is the shape that satisfies this; any other shape is acceptable only if it too reports the same argv total at one reservation and at twenty.

Replace the per-commit ancestry queries with one `git rev-list <base>..HEAD` and set membership: measured 9–10 ms over both 25- and 500-commit ranges against roughly 120–127 ms for fourteen warmed `merge-base` calls.

**Files:**
- `crates/cargo-berth/src/drift/provenance.rs` — batched `commits_for_paths` (~L80); `commit_origin` via `rev-list` set membership (~L109); `path_commits` retired (~L129); `trunk_object_id` returns `CommitOriginTrunk` (~L73)
- `crates/cargo-berth/src/drift/git_output.rs` — spawn through the `git` module's typed execution boundary; the typed stale-anchor result (~L68)
- `crates/cargo-berth/src/git/command.rs` — `GitCommandExecution` and the facade that makes it reachable from `drift` (~L16, ~L30)
- `crates/cargo-berth/src/git/mod.rs` — the batched log/rev-list invocations behind that boundary
- `crates/cargo-berth/src/drift/constants.rs` — the git argument constants for the batched form
- `crates/cargo-berth/tests/drift.rs` — differential and shim fixtures

**Constraints from prior phases:** **This phase is not independent of Phase 2.** Phase 2 routed every git subprocess it added through one typed execution boundary — `GitCommandExecution` in `git/command.rs`, currently `pub(super)` and therefore reachable only inside the `git` module — so that a spawn failure and a completed non-zero exit stay distinct outcomes. `drift/git_output.rs:68` still spawns `Command::new(GIT_BINARY)` directly and so cannot express that distinction. The batched invocations this phase introduces route through that boundary, which means either widening its visibility or adding a public facade in `git`; a second private spawn path is not an option. Phase 15 measures the PostToolUse budget and depends on this landing first. Phase 3 established the standard this phase's non-scaling proof is held to — exact argv equality between a one-subject and a twenty-subject trace, not a sublinear trend — and shipped `git::reachability_to_target` (`git/mod.rs:1246`) as the batched ancestry primitive to build the anchor precondition on. **Phase 4 sharpened that standard into a hard requirement about the trace itself: it must be raw.** An allowlisted or command-name-only trace hid a real scaling defect for a full review round. `TracedDrift::fingerprint_commands` and the command-name-only wrapper at `tests/drift.rs:41-45` are exactly that shape and must not be reused as this phase's gate — record every invocation's complete argv before any classification, then compare.

**Acceptance gate:** **Every `Test` command in Delegation Context** green, including `a_committed_incursion_names_the_commits_that_introduced_its_paths`. A `git` shim varying path count and unique commit count independently (paths 0/1/4/33 crossed with commits 0/1/14/100) shows a **fixed** process count for the whole drift invocation. A separate shim fixture varies the **reservation** count instead — one reservation at one phase start against twenty reservations at twenty distinct phase starts, same entered paths — and asserts the **exact** git argv total is equal in both, with identical command-name sequences. Sublinear is not sufficient; the two totals must match, the same standard Phase 3's `distinct_cold_proof_subjects_are_bounded_to_one_git_evaluation_per_target` (`tests/board.rs:2253`) holds the equivalence path to. A benchmark matrix over short and long ranges shows the one-path cell not regressing while the 33-path cell improves. Differential fixtures against the current per-path implementation cover merges, conflict-resolution-only paths, renames, deletions, tabs, newlines, and non-ASCII names. A fixture with an unresolved trunk reports `CommitOriginTrunk::CannotClassifyOrigin` rather than an absent value, and a non-ancestor phase start returns the typed stale-anchor result instead of attributing. No `Option<GitObjectId>` remains in `trunk_object_id`, `commits_for_paths`, or `commit_origin`. Separate fixtures prove the boundary distinction survives on the new path: a git binary that cannot be spawned and a git invocation that completes with a non-zero exit produce **different** typed outcomes, not one collapsed failure. Cost is proved on a raw unfiltered argv trace across path, commit, and reservation cardinalities, with equal totals and identical command sequences at one subject and at twenty; a mixed-anchor fixture holds nineteen valid phase starts against one missing, one unrelated, and one stale anchor and asserts the valid reservations still attribute correctly.

### Phase 11 — Typed coordination-identity rejections  · status: todo

#### Work Order

**Goal:** One engine-owned rejection enum replaces the single stringly "retry the command" message across every verb that checks coordination identity.

**Spec:**

A stale session mapping or worktree marker survives an ordinary rerun, but `ClaimError::into_output` advises only a rerun, so PreToolUse can repeat the same refusal indefinitely. The live message reads "harness session mapping for coordination run `<id>` no longer names an active reservation" and directs the reader to run `cargo-berth drift --reservation <id> --json` by hand — a second unqualified rerun. A third case shares the message and is not stale at all: a `check` run from a worktree whose session maps to a reservation held by a *different* worktree gets the same text though the reservation is alive and `active`.

**The defect is wider than claim/check/drift.** `verb/claim.rs:1181`, `verb/sequence.rs:272`, and `gate/mod.rs:1165` each repeat the same compound predicate:

```rust
reservation.id() == reservation_id
    && reservation.actor().run == coordination_run_id
    && reservation.actor().worktree == worktree_id
    && matches!(reservation.lifecycle(), ReservationLifecycle::Active)
```

Every failed term collapses into `InactiveSessionMapping(coordination_run_id)`. Drift loses even that through `ClaimRejected(String)` (`drift/execution.rs:423`). `IntegrationRejectionKind` and `SequenceRejectionKind` separately duplicate two variants carrying only the run id. Fixing three call sites would leave the same defect in the other two and add another set of enums that can diverge before Phase 14 freezes the contract.

Define one enum and one validator, reused by claim, check, drift, sequence, integration, and gate handling. **Find the reservation first, then classify lifecycle, run, and worktree independently:**

```rust
enum CoordinationIdentityRejection {
    StaleSessionMapping { coordination_run_id, reservation_id },
    StaleMarkerRun { coordination_run_id, issuing_worktree_id },
    SessionWorktreeMismatch {
        coordination_run_id, reservation_id,
        holding_worktree_id, issuing_worktree_id,
        holding_root, issuing_root,
    },
}
```

`SessionWorktreeMismatch` is the precise name — the failed identity is a session-to-reservation mapping. The variant carries **canonical roots**, not only opaque ids, because the next action is to run from the holder's checkout.

Each variant carries typed `recovery_actions` with `argv` and `cwd`, serialized in the rejection payload. Human text:

> Harness session mapping points to inactive reservation `<reservation-id>` for coordination run `<run-id>`. Run `cargo-berth identity clear-session --json`, then rerun `<original-command>`. No reservation or edit decision changed.

> Worktree `<issuing-root>` has an inactive marker for coordination run `<run-id>`. Run `cargo-berth board --json` to reconcile and sweep the marker, then rerun `<original-command>`. Retrying first will repeat this rejection.

> Reservation `<reservation-id>` for coordination run `<run-id>` is active in `<holding-root>` (`<holding-worktree-id>`), but this command ran in `<issuing-root>` (`<issuing-worktree-id>`). Run `cd '<holding-root>' && <original-command>`, or start a separate harness session and claim work in `<issuing-root>`. No state changed.

**The first message requires a new engine-owned command**, `cargo-berth identity clear-session`, that removes only the current `CARGO_BERTH_SESSION_ID` entry. A valid session mapping outranks `CARGO_BERTH_RUN` (`docs/cargo-berth/operations.md:21`), so setting the run cannot repair a stale mapping while the hook keeps supplying the session id, and no existing command removes just that entry. Without it the recovery is prose, not an action. **The removal lands in `session/mod.rs`**, which owns the `CARGO_BERTH_SESSION_ID` mapping (~L28); no current operation removes only the active entry, so this is new behaviour there rather than a new caller of existing behaviour.

**`output.rs` serializes the rejection; it does not own the identity rule.** Put the shared validator in its own domain module — `coordination_identity.rs` — and declare it in `main.rs` alongside the other top-level modules. Six verbs plus the git gate consume it; leaving the rule inside the serialization module makes every one of them depend on output formatting to answer an identity question.

**Files:**
- `crates/cargo-berth/src/coordination_identity.rs` — new module: `CoordinationIdentityRejection` and the shared validator
- `crates/cargo-berth/src/main.rs` — declare the new module
- `crates/cargo-berth/src/session/mod.rs` — remove only the active `CARGO_BERTH_SESSION_ID` mapping (~L28)
- `crates/cargo-berth/src/output.rs` — serialize the rejection payload and `recovery_actions` (~L373)
- `crates/cargo-berth/src/verb/claim.rs` — use the shared validator (~L1181, ~L1541)
- `crates/cargo-berth/src/verb/check.rs` — same
- `crates/cargo-berth/src/verb/sequence.rs` — same (~L272)
- `crates/cargo-berth/src/verb/integrate.rs` — `IntegrationRejectionKind` folds into the shared enum
- `crates/cargo-berth/src/drift/execution.rs` — replace `ClaimRejected(String)` (~L423)
- `crates/cargo-berth/src/gate/mod.rs` — same (~L1165)
- `crates/cargo-berth/src/cli.rs` — the `identity clear-session` subcommand
- `docs/cargo-berth/json-contract.md` — the rejection payload and `recovery_actions`

**Constraints from prior phases:** Phase 6 established `resolve_identity(&WorktreeContext)` as the single identity entry point returning per-worktree and shared-ledger paths as distinct types — the validator here uses it to obtain `issuing_worktree_id` and `issuing_root`, and must not re-derive them.

**Acceptance gate:** **Every `Test` command in Delegation Context** green. Fixtures prove all three rejection paths across claim, check, drift, sequence, integrate, and the git gate; each carries `recovery_actions` with `argv` and `cwd`; **none recommends an unqualified rerun**. `cargo-berth identity clear-session --json` removes only the current session entry and leaves other mappings intact.

### Phase 12 — Front ends render recovery actions without parsing messages  · status: todo

#### Work Order

**Goal:** A hook-only agent can act on an identity rejection without a human and without reading `message`.

**Spec:**

`claim_state.py:543` currently carries prose-only recovery, and `berth_pre_edit.sh:345` prints `.message` and refuses the edit while PostToolUse appends another manual drift command. Neither can act on the typed rejections Phase 11 produces.

Every canonical consumer renders `recovery_actions` — `argv` plus `cwd` — from the payload, never by parsing `message`. The front end already owns the original argv, so it combines the `rerun_from_worktree` action with that argv to produce a runnable command.

**Files:**
- `/Users/natemccoy/.claude/scripts/berth/claim_state.py` — classify and render the three rejections from typed fields (~L543)
- `/Users/natemccoy/.claude/scripts/berth/install/hooks/berth_pre_edit.sh` — render `recovery_actions` instead of `.message` (~L345)
- `/Users/natemccoy/.claude/scripts/berth/install/hooks/berth_post_bash.sh` — same
- `/Users/natemccoy/.claude/scripts/berth/install/hooks/berth_session_start.sh` — same

**Constraints from prior phases:** Phase 11 defined `CoordinationIdentityRejection::{StaleSessionMapping, StaleMarkerRun, SessionWorktreeMismatch}`, each carrying typed `recovery_actions` with `argv` and `cwd`, and added `cargo-berth identity clear-session`. `SessionWorktreeMismatch` carries `holding_root` and `issuing_root` as canonical paths, which is what the `cd '<holding-root>' && <original-command>` action needs. Phase 5 added alert rendering to `typed_drift_feedback` in `berth_post_bash.sh` — extend that rendering rather than replacing it.

**Acceptance gate:** The shim fixtures pass. For each of the three rejections, the PreToolUse shim prints a runnable recovery command derived from typed fields with `message` unread, and `claim_state.py` classifies it without a `cast`. basedpyright reports zero errors and zero warnings for `claim_state.py`.

### Phase 13 — Named reservation lifecycle query  · status: todo

#### Work Order

**Goal:** `/plan:delegate` can prove whether a named reservation is outstanding or released after a lost release reply.

**Spec:**

The board deliberately omits lifecycle-bearing rows for a waiting successor and either endpoint of an unresolved overlap (`board/mod.rs:625`). After a lost release reply, `/plan:delegate` can therefore observe `ReservationPresentWithoutProtectedTip` but cannot prove whether that reservation is outstanding or released; a matching retention ref proves only commit reachability.

Add a read-only selector independent of board placement:

```
cargo-berth board --reservation <reservation-id> --json
```

returning, in a payload that also echoes `reservation_id`:

```rust
enum ReservationLifecycleSnapshot {
    Active,
    Outstanding { protected_tip },
    ReleasedAfterCheckpoint { protected_tip, disposition },
    ReleasedWithoutCheckpoint { disposition },
}
```

**`ReservationLifecycleSnapshot`, not `NamedReservationLifecycle`.** "Named" states how the value was obtained — by id rather than by board placement — which is the caller's business, not the type's. The type is a point-in-time reading of one reservation's lifecycle, and the name should say so.

**Project it from `ReservationEvidenceState`; do not restate the lifecycle rules.** `Reservation::evidence_state` (`reservation/mod.rs:1799`) already returns exactly these four classifications — `Active`, `Outstanding`, `Released`, `ReleasedWithoutCheckpoint` (`ReservationEvidenceState`, `reservation/mod.rs:491`) — each carrying its protected tip where one exists. Map from it and drop the evidence fields this caller does not need. A second hand-written lifecycle match is a second place for the Phase 1 invariant to drift out of.

An unknown id is a typed invalid-input result, never `Option`. The caller needs the exact protected tip and which of the four states applies; it does **not** need current integration evidence.

**The engine selector alone is inert.** `claim_state.py:2027` rejects every board argv except exactly `["board", "--json"]`, and `/plan:delegate` reaches the board only through that coordinator — so a new engine query would pass every crate test while lost-release recovery still dead-ends. Add a validated coordinator entry point:

```sh
PYTHONPATH="$HOME/.claude/scripts" python3 -m berth.claim_state reservation \
  --cwd "${WORKING_DIR}" --reservation "${RESERVATION_ID}"
```

Its validator requires the echoed id, exactly one lifecycle alternative, the protected tip where the alternative carries one, exit 0 for a known id, and a typed invalid-input reason for an unknown one. Update `/plan:delegate` to use it after a lost release reply — the lost-release reasoning now sits at `delegate.md:1726`, `:1733`, and `:1743`, where `ReservationPresentWithoutProtectedTip` is interpreted.

**Files:**
- `crates/cargo-berth/src/cli.rs` — the `--reservation` selector on `board`
- `crates/cargo-berth/src/verb/board.rs` — board execution and response dispatch route the new selector
- `crates/cargo-berth/src/board/mod.rs` — the placement-independent lookup (~L625)
- `crates/cargo-berth/src/reservation/mod.rs` — `ReservationLifecycleSnapshot`, projected from `evidence_state` (`ReservationEvidenceState` L377, `evidence_state` L1609)
- `crates/cargo-berth/src/output.rs` — the payload and the typed unknown-id rejection
- `crates/cargo-berth/tests/board.rs` — waiting-successor and both overlap-endpoint fixtures
- `/Users/natemccoy/.claude/scripts/berth/claim_state.py` — the `reservation` entry point and validator (~L2027)
- `/Users/natemccoy/.claude/commands/plan/delegate.md` — use it after a lost release reply (~L1726–L1751)
- `docs/cargo-berth/json-contract.md` — the new payload

**Constraints from prior phases:** Phase 1 made `edit_blocking_status` computed and `Released` terminal, so `ReleasedAfterCheckpoint` and `ReleasedWithoutCheckpoint` are genuinely terminal states here. `Reservation::evidence_state` already supplies the four classifications this phase projects — do not duplicate the lifecycle match. Phase 5's lost-evidence alert directs the reader to plain `board --json`, **not** to this selector, precisely so Phase 5 does not depend on this phase; do not retarget that alert text here. Phase 12 established that front ends render typed payload fields without parsing `message` — the coordinator validator follows the same rule.

**Acceptance gate:** **Every `Test` command in Delegation Context** green. Fixtures cover a waiting successor and both unresolved-overlap endpoints — all three omitted from board rows, all three resolvable by id. Existing board JSON stays byte-compatible. The coordinator entry point returns exit 0 for a known id and a typed invalid-input reason for an unknown one.

### Phase 14 — Generated status, exit-code, and payload contract  · status: todo

#### Work Order

**Goal:** An engine status or enum-variant addition cannot pass engine tests while leaving any front-end consumer stale.

**Spec:**

`claim_state.py:24` hand-maintains `STATUS_PAYLOAD_KINDS` and `FIXED_STATUS_EXIT_CODES`, while the canonical hook shims separately hand-maintain accepted payload tags and required fields in `jq`. The PostToolUse validator had to be taught the valid `first_touch_claimed` and `post_write_incursion` variants by hand after the Python classifier had already been updated, so engine tests demonstrably do not keep every consumer synchronized. `OutputEnvelope` (`output.rs:67`) permits independent construction of `status`, `exit_code`, and `payload`, and `output.rs:1380` contains a wildcard consumer arm (`_ => PostCommitRendering::Warning(...)`), so a new status does not force even that Rust consumer to be reviewed.

**A checked-in manifest alone is insufficient.** `serde` exposes no supported variant inventory, and `strum` can enumerate statuses but cannot describe tagged payload fields or legal status/payload combinations. So:

1. Declare `OutputStatus` and its fixed exit/status metadata through **one macro or declaration table** that also generates the enum and its complete variant list.
2. Rust consumers match **exhaustively, with no wildcard arms** — remove the one at `output.rs:1380`.
3. Generate a versioned JSON contract from that metadata plus schemas derived from the serialized envelope and payload DTOs. `schemars` as a build/test dependency is the realistic mechanism; its cost is one code-generation dependency plus generated-file review churn.
4. Generate the Python tables and the static `jq` validation fragments from that contract, check them in, and **byte-compare regenerated output** in engine tests.
5. Execute Python and `jq` against generated valid and invalid fixture envelopes in tests.

**Generation alone cannot type the replay failures, and the set is not two.** Every variant of `ReservationReplayError` (`reservation/mod.rs:1716`) collapses into one `ledger_unreadable` status, exit code 4, a `NoFacts` payload, and free-form message text — so there is no typed payload for a schema to expose, and generating the contract over today's shape would freeze the ambiguity. Add a semantic replay-failure payload carrying the offending reservation id and the exact reason, and enumerate it in the generated contract.

**Derive that payload exhaustively from the enum; do not special-case a chosen few.** Phase 1's `WidenRequiresUnreleased` and `ResnapshotRequiresOutstanding` were the first two named here, but Phase 3 shipped three more — `IntegrationProofSubjectRevisionExhausted`, `ActiveScopedPatchComparison`, and `IntegrationProofSubjectMismatch` — and the enum already carries a dozen others (`DuplicateClaim`, `UnknownReservation`, `EmptyScopeSet`, `RevisionExhausted`, `InvalidLifecycleTransition`, `SnapshotStateMismatch`, `IntegratedReleaseWithoutEvidence`, `ActiveEvidenceRevalidation`, `DecisionHasNoGitEvidence`, `MissingProtectedTip`, `MissingTrunkSnapshot`, `WorktreeRelocationMismatch`, and the incursion-incident variants). Hand-listing a subset is the exact failure this phase exists to end: the list would go stale the next time a phase adds a variant. The payload's reason must be **generated from `ReservationReplayError` itself**, through the same declaration table that generates `OutputStatus`, so a new variant fails engine tests until every consumer regenerates.

All of them are **hard stops**: the ledger cannot be replayed, so no reconciling command can proceed. A consumer must be able to tell that from the payload and route the operator to journal review or a confirmed reinitialization without parsing `message`.

The generated board contract must also accept `reblocked_active_constraint` as a **reserved** wire value — Phase 1 retained the variant for v1 compatibility while making it unreachable — and prove that fresh engine fixtures never emit it.

Validators stay **static** — no manifest parsing or filesystem read per hook invocation, so runtime cost is unchanged.

**Files:**
- `crates/cargo-berth/src/output.rs` — the declaration table, exhaustive matches, no wildcard; the typed replay-failure payload (~L67, L120, L1380)
- `crates/cargo-berth/src/reservation/mod.rs` — `ReservationReplayError` (L1716) surfaces its reservation and reason to the payload, generated exhaustively rather than variant by variant
- `crates/cargo-berth/src/ledger/journal.rs` — the journal operation inventory the contract enumerates, including Phase 3's `ScopedPatchEquivalenceChecked` (L254) and `ScopedPatchComparisonAttempted` (L265)
- `crates/cargo-berth/build.rs` — the versioned contract artifact generator
- `crates/cargo-berth/Cargo.toml` — `schemars` as a build/dev dependency
- `/Users/natemccoy/.claude/scripts/berth/claim_state.py` — generated tables replace the hand-kept ones (~L24)
- `/Users/natemccoy/.claude/scripts/berth/install/hooks/{berth_pre_edit,berth_post_bash,berth_session_start}.sh` — generated `jq` fragments
- `docs/cargo-berth/json-contract.md` — regenerate from the contract

**Pending decision: which copy of the hook shims is canonical.**

Item 5 of the Spec as originally written said "Keep canonical hook templates in the repository; installation copies them to `~/.claude/scripts`." That contradicts the live installation contract. `~/.claude/scripts/berth/install/README.md` states the opposite in its own words: the shims there are "the canonical, durable copies", they "run from this" directory in place, "Do not copy these scripts into a repository. A copy stops receiving fixes made together — a stale copy rejects output the current engine emits", and they are "the canonical copies, not an installation artifact". Both models are buildable and only one can hold.

- **Keep the external scripts canonical (no plan change beyond deleting item 5).** The generated `jq` fragments are written into `~/.claude/scripts/berth/install/hooks/` by the generator, and the repository holds only the contract they are generated from. Matches today's README and needs no new install step. The cost: an engine test cannot byte-compare a checked-in artifact against the file that actually runs, so the generated-vs-installed check is weaker than item 5 intended.
- **Make the repository canonical (item 5 as written).** Requires an explicit sync/install owner, updated hook registration, a rewritten README, and a byte-comparison of installed files against the repository templates. `build.rs` must **not** mutate the home directory to achieve it.

This one reaches the user because it changes how a berth installation is maintained outside this repository, and a wrong choice is not a one-line revert. Until it is settled, this phase generates the contract and the artifacts but does not relocate the shims.

**Constraints from prior phases:** **Must follow Phases 1, 4, 5, 7, 11, 12, and 13** — every one adds statuses, journal records, or payload variants that this contract must enumerate. Phase 4 is on that list because it adds the distinct successor-equivalence cache records, and the contract cannot be frozen before they exist. Specifically: Phase 1 added `ReservationReplayError::{WidenRequiresUnreleased, ResnapshotRequiresOutstanding}` — the widen failure is named for the release boundary, and Phase 2 widened it to accept an `Outstanding` reservation, so every fixture and message must say "unreleased", never "active" — both currently untyped in the envelope, and retained `reblocked_active_constraint` as a reserved-but-unreachable board wire value; Phase 2 added the nested `IntegrationProof` variants and the untagged `trunk_at_claim` alternatives, which the generated inventory must cover; Phase 3 added the journal records `ScopedPatchEquivalenceChecked` and `ScopedPatchComparisonAttempted` (`ledger/journal.rs:254`, `:265`) and the types they carry — `IntegrationProofSubjectRevision` (`reservation/mod.rs:75`) and `ScopedPatchEquivalenceVerdict` (`reservation/mod.rs:84`) — all of which the generated journal inventory must enumerate, plus the three replay hard stops named in the Spec; Phase 4 adds **two** journal records, not one — `SuccessorScopedPatchEquivalenceChecked` (`ledger/journal.rs:275`) and `SuccessorScopedPatchComparisonAttempted` (`:286`) — plus the nested `SuccessorScopedPatchEquivalenceVerdict::{Equivalent, Different}` wire values they carry; Phase 5 added the lost-evidence alert with its two `LostEvidenceRecovery` variants; Phase 7 added `recorded_now` and `already_recorded_by_same_coordination_actor` success payloads and an enriched `invalid_input` rejection; Phase 11 added `CoordinationIdentityRejection` with `recovery_actions`; Phase 13 added `ReservationLifecycleSnapshot` and a typed unknown-id rejection. Unifying the contract before these land means doing it twice.

**Acceptance gate:** **Every `Test` command in Delegation Context** green. Adding a new `OutputStatus` variant fails engine tests until the generated Python and `jq` artifacts are regenerated and checked in. Malformed status/payload/exit combinations remain rejected. Python and `jq` validators execute against generated valid and invalid fixture envelopes. No hook invocation reads a manifest at runtime. A fixture per replay failure proves each is distinguishable from the payload alone, names its reservation, and identifies itself as a hard stop without reading `message` — starting with a `Release` → `Widen` journal, a `Released` `Resnapshot` journal, and Phase 3's three (`IntegrationProofSubjectRevisionExhausted`, `ActiveScopedPatchComparison`, `IntegrationProofSubjectMismatch`). Adding a variant to `ReservationReplayError` must fail engine tests until the generated reason inventory and every consumer artifact regenerate; a test asserts the generated inventory covers the enum exhaustively rather than a hand-kept subset. The generated journal inventory likewise covers every `JournalOperation` variant, including Phase 3's `ScopedPatchEquivalenceChecked` and `ScopedPatchComparisonAttempted` and Phase 4's `SuccessorScopedPatchEquivalenceChecked` and `SuccessorScopedPatchComparisonAttempted`, including the nested `SuccessorScopedPatchEquivalenceVerdict::{Equivalent, Different}` wire values. The generated board contract accepts `reblocked_active_constraint` as a reserved value while no fresh engine fixture emits it. The generated inventory also proves that **nested** enum variants stay synchronized across all three consumers, not only top-level statuses: `IntegrationProof::{ProtectedTipAncestor, ScopedPatchEquivalent}` inside `IntegrationEvidenceStatus::Integrated`, and the untagged `trunk_at_claim` alternatives, must appear in the Rust envelope, the generated Python tables, and the generated `jq` validators, and adding a variant to any nested enum must fail engine tests until all three regenerate.

### Phase 15 — Prove the PostToolUse path stays inside 0.20 seconds  · status: todo

#### Work Order

**Goal:** Every PostToolUse outcome is measured under a reproducible protocol and lands inside the published bound.

**Spec:**

The complete two-reservation PostToolUse call was measured at 0.259 seconds and one automatic-widen invocation at 0.180 seconds. One sample and one outcome do not satisfy the published bound, and this cost is paid after every Bash call in an enrolled repository.

**State what the bound covers**: `berth_post_bash.sh` alone, not every globally registered PostToolUse command — the matcherless random-ack and context-usage hooks in `~/.claude/settings.json` are outside berth's control.

**Every timed sample starts from an independently restored state, restored outside the timer.** Five consecutive calls against one live state cannot retain the named outcomes: an ordinary widen changes the scopes on the first call, first-touch creates the reservation on the first call, and a successful non-blocking call publishes a new fingerprint at `drift/execution.rs:219-220` so the next call observes no delta.

Where the cost actually is, so the work targets the right term: the wrapper's fixed process cost dominates most outcomes — a clear outcome launches eight executables (Bash, `cargo-berth`, four `jq`, `mktemp`, `rm`) and a rendered widen/incursion/collision response launches ten. Provenance batching (Phase 10) helps only the attribution outcome: `name_incursion_commits` runs after classification, a no-change call returns at `drift/execution.rs:161-170`, and a widen-only result performs no provenance git query.

Using the globally registered canonical path and production JSON, take at least **five cold and five warm** samples for each of: typed clear, ordinary widen, first-touch acquisition, foreign-only incursion, `post_write_incursion` with `protection.status = acquired`, `post_write_incursion` with `protection.status = not_acquired`, collision, attribution, and **the Phase 5 lost-evidence alert**. Each timed berth invocation finishes within 0.20 seconds.

The lost-evidence alert is not a free ninth row: it adds envelope validation and rendering to the PostToolUse path, and its own generation runs against post-reconciliation evidence. "Every outcome" excludes it only if it is never measured.

**Separate the two temperatures the word "cold" hides.** Process-cache temperature (first invocation versus a warmed page cache) and durable proof-cache state (a cache entry that has never been evaluated versus one already stored in the journal) are independent, and a matrix that conflates them cannot tell a slow first run from a cache that is not working. Label every sample with both.

Four expensive outcomes Phase 2 and its cachers introduced belong in the matrix as their own rows, each measured at both proof-cache states: trunk scoped-equivalence **positive** and **negative**, and successor-equivalence **positive** and **negative**. A miss on any of them composes roughly a dozen git invocations, so a single uncached retained reservation can consume a large share of the whole budget on its own.

Record **child executable and git argv counts alongside wall time**: clear and widen execute zero provenance `log`/`rev-list` calls; incursion attribution executes exactly one of each; a proof-cache hit executes zero equivalence invocations.

**Timing alone cannot catch renewed scaling — carry Phase 3's cardinality guard into the matrix.** A one-reservation sample can stay comfortably inside 0.20 seconds while a per-reservation git call has quietly returned, and this matrix's samples are exactly that shape. Phase 3 shipped the guard that catches it — `distinct_cold_proof_subjects_are_bounded_to_one_git_evaluation_per_target` (`tests/board.rs:2253`) asserts **exact** totals on a cold pass: 14 git argv for an equivalent target, 13 for a different one, byte-identical command-name sequences between the one-reservation and twenty-reservation traces, and **zero** `merge-base --is-ancestor` invocations. Add the same dimension to this matrix for both cache-miss rows: trunk scoped-equivalence and successor scoped-equivalence each report exact argv totals and identical command sequences for one reservation and for twenty, and the two totals must be equal. A row that meets its wall-clock bound but whose twenty-reservation total exceeds its one-reservation total fails this phase.

**Files:**
- `crates/cargo-berth/tests/edges.rs` — the shipped non-scaling guards at L674 and L767
- `crates/cargo-berth/tests/board.rs` — the shipped cost guard at L2255
- `/Users/natemccoy/.claude/scripts/berth/install/hooks/berth_post_bash.sh` — reduce fixed process count where the measurement shows it dominates
- `crates/cargo-berth/src/drift/execution.rs` — any engine-side cost the measurement isolates
- `crates/cargo-berth/src/git/mod.rs` — the scoped-equivalence invocations, if the measurement shows the cold path over budget
- `crates/cargo-berth/src/reconcile.rs` — the per-reservation reconciliation pass and its cache consultation
- `crates/cargo-berth/src/reservation/mod.rs` — the proof caches, if their key or storage is what the measurement isolates
- `crates/cargo-berth/tests/drift.rs` — the subprocess-count assertions

**Constraints from prior phases:** **Must follow Phases 3, 4, 5, 8, 9, 10, 12, and 14** — each changes what this path costs or what it must render. Phase 3 cached the scoped-equivalence proof so reconcile stops issuing a per-reservation diff on every call. Phase 4 added the successor-equivalence query and its own cache to the same reconcile pass, and batched four costs that previously scaled: predecessor ancestry, worktree ahead/behind, retention-ref availability, and retention-ref repair are each one invocation regardless of how many reservations are involved, with the successor round-robin admitting exactly one cold scoped comparison per reconciliation. Phase 5 added the lost-evidence alert, which this phase must time as one of its outcomes. Phase 8 filtered the generated git-hook script by phase and ref before spawning the binary. Phase 9 stopped berth's own retention-ref writes from firing `reference-transaction` and batched the retention-ref deletion path. **The "8 spawns per drift run" figure this Work Order was written against is stale** after Phases 4 and 8; measure it, do not quote it. Phase 10 gave provenance a fixed subprocess count. Phase 12 changed what the shims render for every rejection. Phase 14 replaced the shims' hand-kept `jq` validation with generated fragments, which is what actually executes per call. Measuring before these land measures the wrong thing.

**Pending decision: whether ledger-projection bounding becomes its own Work Order before this phase.**

Since this decision was written, Phase 4 raised the durable event rate again: it appends a definitive record for every successor-head verdict and an attempt record for every transient successor comparison that cannot be cached. Its 512-entry retention bounds replayed state only — neither the append-only journal nor the projection is compacted by it. The two choices below are unchanged; the growth they weigh is larger than stated.

The architect review after Phase 3 holds that this phase cannot prove its 0.20-second bound for the lifetime of a repository while `Projection` stores `events: Vec<JournalEvent>` (`ledger/projection.rs:40`) and `Projection::from_replay` clones, serializes, and fsyncs the complete event vector on every publish. Reconciliation runs on the PostToolUse path, so that per-edit cost grows with the total number of journal events ever written, and Phase 3 raised the rate: `ScopedPatchEquivalenceChecked` records one event per `(reservation, trunk target)` and fires even when the reported status does not change.

The growth predates this plan and no phase owns it. Phase 3's non-scaling invariant constrains git subprocess counts, not ledger size, and its bounded per-reservation retention cannot reach durable state. The proposed fix is journal compaction or a projection that stores replayed facts instead of raw events — a change to `ledger/`, not a repair to any phase's diff.

- **Insert a dedicated Work Order before this phase.** It preserves the append-only journal, stores bounded replay state, and tests a small journal against a long one. This phase then measures a path whose cost does not grow without bound, and its 0.20-second claim survives repository age. The cost is one more phase and a change to the ledger's most load-bearing type.
- **Leave it in the backlog and scope this phase's bound to a fresh repository.** No new phase; this phase's Acceptance gate states explicitly that the bound is proved at the journal sizes the fixtures produce and says nothing about a long-lived repository. The published bound is then narrower than it reads today.

This reaches the user because inserting a phase changes the plan's spine and its remaining scope, and because the second option narrows a bound the plan has already published. The matching backlog item in `berth-fix-next.md` is held pending this decision and is removed only if the first option is taken. Until it is settled, this phase runs as written.

**Equal subprocess counts do not imply a stable wall clock.** Phase 4's `ahead_behind_for_heads` (`git/mod.rs:1158`) and `descendant_commits` (`git/mod.rs:1297`) each hold a fixed invocation count, but both consume union ancestry and recompute ancestor sets per head, so their cost grows with history depth and branch divergence rather than with subject count. A 0.20-second bound proved only on shallow fixture repositories is not the bound this phase claims to publish. Add shallow, deep, and divergent-history cells at one subject and at twenty to the timing protocol; if a cell cannot be met, narrow the published bound in the phase's own words to the repository sizes actually measured rather than leaving the wider claim standing.

**Acceptance gate:** All thirteen outcomes — the original nine plus trunk-equivalence positive and negative and successor-equivalence positive and negative — finish within 0.20 seconds across five samples at each combination of process-cache temperature and durable proof-cache state, from independently restored state. Child executable and git argv counts are recorded per outcome and match the stated expectations. Both scoped-equivalence cache-miss rows additionally report exact git argv totals and identical command-name sequences for one reservation and for twenty, with the two totals equal, matching the guard at `tests/board.rs:2255`. The timing protocol runs its shallow, deep, and divergent-history cells at both cardinalities, and the phase reports the repository sizes its published bound is proved against. The shim fixtures pass; **Every `Test` command in Delegation Context** and `bash ~/.claude/scripts/delegate/verify.sh lint cargo-berth` green.

### Phase 16 — Semantic roles and bounded optionality  · status: todo

#### Work Order

**Goal:** Fifteen types name what they are, and six bare `Option<T>` parameters become one semantic type at the boundary.

**Spec:**

`PriorClassification` does not name its pre-lock foreign-path role; `ReservationRow` names a display representation rather than what it holds; `CommandExecution` does not state who owns presenting the result; and `GitCommandExecution` (`git/command.rs:16`), added by Phase 2, names an action when its actual guarantee is whether a completed process output exists — `CouldNotRun` versus a completed `Output`, which is precisely an availability, not an execution. `overlap_authorization_request` exposes six bare `Option<T>` parameters and `EditAuthorization::resolve_from_sources` accepts `Option<OsString>`, so readers must infer overlap-selection and environment-identity states from representation and control flow.

Phase 3 shipped four more names that state their representation rather than their semantic role. `ScopedPatchEquivalenceCache` (`reservation/mod.rs:122`) is not a cache in the sense the name implies — it is a bounded set of retained verdicts for specific targets. `ScopedPatchEquivalenceCacheLookup` (`reservation/mod.rs:128`) names the act of looking up; its two cases say whether a verdict is available. `ScopedPatchComparisonAttemptHistory` (`reservation/mod.rs:154`) is not a history anyone reads back — it is the round-robin schedule that decides which target is compared next. `ScopedPatchEvaluationMemo` (`reconcile.rs:328`) is not a memo — it is the one-comparison-per-target budget for a single reconciliation pass, and its lifetime is exactly that pass.

Renames:
- `PriorClassification` → `PreLockForeignPathClassification`
- `ReservationRow` → **`BoardReservationSnapshot`**
- `CommandExecution` → `CommandOutputOwnership::{CallerRendersResponse, BoardPresentedAndTerminalRestored}`
- `GitCommandExecution` → `GitCommandOutputAvailability::{Available(Output), Unavailable}`
- `ScopedPatchEquivalenceCache` → `RetainedScopedPatchTargetVerdicts`
- `ScopedPatchEquivalenceCacheLookup` → `ScopedPatchTargetVerdictAvailability`
- `ScopedPatchComparisonAttemptHistory` → `ScopedPatchTargetEvaluationSchedule`
- `ScopedPatchEvaluationMemo` → `ReconciliationScopedPatchEvaluationBudget`
- `ScopedPatchEquivalenceCacheEntry` → `RetainedScopedPatchTargetVerdict`
- `SuccessorScopedPatchEquivalenceCache` → `RetainedSuccessorScopedPatchTargetVerdicts`
- `SuccessorScopedPatchEquivalenceCacheEntry` → `RetainedSuccessorScopedPatchTargetVerdict`
- `SuccessorScopedPatchEquivalenceCacheLookup` → `SuccessorScopedPatchTargetVerdictAvailability`
- `SuccessorScopedPatchComparisonAttemptHistory` → `SuccessorScopedPatchTargetEvaluationSchedule`
- `SuccessorScopedPatchEvaluationBudget` → `ReconciliationSuccessorScopedPatchEvaluationBudget`
- `DescendantCommitQuery` → `ProtectedTipSuccessorHeadClassification`

**Phase 4's names are not already semantic — they are the same representation-over-role pattern, duplicated.** Phase 4 mirrored Phase 3's cache model for the successor path and inherited its naming with it: a "cache" that is a bounded set of retained verdicts, a "lookup" whose cases state availability, an "attempt history" nobody reads back that is really the round-robin schedule, and a "budget" whose lifetime is one reconciliation pass. Renaming the trunk-side four while leaving their successor twins untouched would be worse than renaming neither, because the two halves would then disagree about what the same structure is called. `DescendantCommitQuery` (`git/mod.rs:1531`) names an act where its variants state a classification outcome — `Classified` versus `AncestorObjectUnknown`.

**Leave `DeferredScopedPatchIntegrationStatus` and `ScopedPatchEvaluationPriority` alone.** Both already state a semantic role — the validity of materialized evidence under a deferred comparison, and the scheduling order for uncached comparisons — and renaming them would be churn.

**`BoardReservationSnapshot`, not `BoardReservationState`.** The type (`board/mod.rs:131`) combines journal-derived lifecycle with computed integration evidence, visibility, freshness, holder liveness, and live `ahead_behind_main`:

```rust
lifecycle:            ReservationLifecycle,
integration_evidence: BoardIntegrationEvidence,
edit_blocking_status: EditBlockingStatus,
visibility:           BoardReservationVisibility,
freshness:            ReservationFreshness,
ahead_behind_main:    AheadBehind,
```

Naming it "State" claims an authority it does not have and conflicts with Phase 1 making the blocking decision computed. Its `edit_blocking_status` is populated from Phase 1's computed method.

New semantic types:
- `OverlapSelection::{Absent, Before(id), After(id), Defer(id), Override(id)}` at the clap boundary, converted with the reason and proposal into the existing `OverlapAuthorizationRequest` before any internal helper receives them. Four of the five selections name a reservation; a variant set that drops those ids just moves the six bare `Option<T>` parameters one level in.
- `EnvironmentRunSelection::{NotSupplied, UnusableFallbackToMarker, Identified(id)}` replacing `Option<OsString>` in `EditAuthorization::resolve_from_sources`. A bare `Invalid` names the input's defect but not the guarantee that follows it; `UnusableFallbackToMarker` states the marker-fallback policy the serialized behaviour depends on, which is the whole reason the variant is not simply an error.

These are two independent boundaries, not one — overlap selection and environment identity fail differently and are consumed by different callers.

Leave bare `Option<T>` only in clap-owned fields and externally required trait signatures. Keep serialized payloads unchanged.

**The renames are global and mechanical — hand them to the user to run in their editor rather than performing them by hand.**

**Files:**
- `crates/cargo-berth/src/drift/classification.rs` — `PreLockForeignPathClassification`
- `crates/cargo-berth/src/board/mod.rs` — `BoardReservationSnapshot` (~L132, ~L776)
- `crates/cargo-berth/src/cli.rs` — `CommandOutputOwnership`, `OverlapSelection`
- `crates/cargo-berth/src/ledger/mod.rs` — `EnvironmentRunSelection`
- `crates/cargo-berth/src/git/command.rs` — `GitCommandOutputAvailability` (~L16)
- `crates/cargo-berth/src/reservation/mod.rs` — `RetainedScopedPatchTargetVerdict` (L115), `RetainedScopedPatchTargetVerdicts` (L123), `ScopedPatchTargetVerdictAvailability` (L129), `ScopedPatchTargetEvaluationSchedule` (L188), and Phase 4's successor twins: `RetainedSuccessorScopedPatchTargetVerdict` (L148), `RetainedSuccessorScopedPatchTargetVerdicts` (L156), `SuccessorScopedPatchTargetVerdictAvailability` (L162), `SuccessorScopedPatchTargetEvaluationSchedule` (L231)
- `crates/cargo-berth/src/reconcile.rs` — `ReconciliationScopedPatchEvaluationBudget` (L332) and `ReconciliationSuccessorScopedPatchEvaluationBudget` (L342)
- `crates/cargo-berth/src/git/mod.rs` — `GitCommandOutputAvailability` call sites, plus `ProtectedTipSuccessorHeadClassification` (L1531) and its call sites, including the `drift/git_output.rs` facade Phase 10 adds

**Constraints from prior phases:** Phase 1 made `edit_blocking_status` a computed method on `Reservation` and removed the retained field; `BoardReservationSnapshot` populates its field by calling that method, which the shipped `reservation_visibility` already does — the requirement is satisfied and this rename must not reintroduce stored state. Phase 1 also left `BoardReservationVisibility::ReblockedActiveConstraint` in place as a reserved wire value that is unreachable for a released reservation; the rename keeps the variant. Phase 2 added `IntegrationProof` inside `IntegrationEvidenceStatus::Integrated`, which `BoardIntegrationEvidence` surfaces, and introduced `GitCommandExecution` — the fourth rename subject, whose two cases are a completed process output and no output at all. Phase 3 shipped `ScopedPatchEquivalenceCacheLookup` (`Hit`/`Miss`) as a semantic state rather than a bare optional, and `DeferredScopedPatchIntegrationStatus` (`StillValid`/`Degraded`) likewise — those need no optionality repair, but four of Phase 3's names state representation rather than role and are renamed above. Phase 9 adds `GitHookExecutionPolicy`, specified as a semantic state rather than a boolean, so this phase audits rather than repairs it. **Phase 4's successor cache was specified as a semantic state but named on Phase 3's representation-first pattern**, so its seven types are renamed here alongside their trunk-side twins; Phase 4 introduced no new bare domain `Option<T>`. Phase 11 introduced `CoordinationIdentityRejection` — do not duplicate its identity concepts in `EnvironmentRunSelection`.

**Acceptance gate:** **Every `Test` command in Delegation Context** and `bash ~/.claude/scripts/delegate/verify.sh lint cargo-berth` green. Serialized payloads are byte-identical before and after — proved by the existing board and drift JSON fixtures. No bare `Option<T>` remains in `overlap_authorization_request` or `EditAuthorization::resolve_from_sources`, and no reservation id is dropped on the way through `OverlapSelection`. An audit of the caches and policies added by Phases 3, 4, and 9 confirms none carries a bare `Option<T>` for a domain state or a representation-level boolean for the hook policy; any that does is repaired here. All fifteen renames are applied with no other change: serialized field names and wire values are unaffected, because none of the Phase 3 or Phase 4 types is serialized under its own name.

### Phase 17 — Typed coordinator classifiers  · status: todo

#### Work Order

**Goal:** The Python coordinator returns tagged unions instead of `dict[str, object]` reached through `cast`.

**Spec:**

`classify_claim`, `classify_check`, `render_board`, `_validate_board`, and `_generic_state` all return `dict[str, object]` and reach it through `cast`, so every tagged union the coordinator builds is erased at the one boundary a reader inspects. `ProposalAwaitingApprovalStateValue.proposal` is a bare `dict[str, object]` for the same reason. The weakness predates the coordinator work — `classify_claim` returned `dict[str, Any]` at `831e34a` — and repairing only one symbol would leave an inconsistent surface.

The coordinator's state classifiers return a tagged union rather than `dict[str, object]`; the locked proposal carries a semantic type validated at envelope conversion; and no `cast` stands between a tagged value and its return.

Project rules that bind here: never use file-level type ignores; avoid `Any` — annotate all signatures, use `TypedDict` for dicts with known keys, and for stdlib `Any` returns (`json.loads()` etc.) annotate with a `TypedDict` or specific type. Line-level `# pyright: ignore[reportAny]` is a last resort on the specific line only.

**Files:**
- `/Users/natemccoy/.claude/scripts/berth/claim_state.py` — the five classifiers and the proposal type

**Constraints from prior phases:** Phase 12 already added typed rendering of `CoordinationIdentityRejection` and its `recovery_actions` to this file, and Phase 13 added the `reservation` entry point and its validator. Phase 14 generated `STATUS_PAYLOAD_KINDS` and `FIXED_STATUS_EXIT_CODES` from the engine contract — those generated tables are inputs here, not hand-edited targets.

**Acceptance gate:** basedpyright reports **zero errors and zero warnings** for `claim_state.py`. The shim fixtures pass. No file-level type ignore exists in the file, and every remaining `# pyright: ignore` is line-level with a named rule.
