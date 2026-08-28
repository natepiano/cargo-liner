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
  - `crates/cargo-berth/src/recovery.rs` — `fn resolve` (~L109), `execute_one_incursion_resolution` (~L271), identity resolution through `ledger::resolve_identity` (~L220, ~L277, ~L367), `recovery_operation` `--integrated-as` gate (~L553), `RecoveryRejection` (~L749) whose already-resolved variant is now `IncursionIncidentAlreadyResolvedByDifferentCoordinationActor` (~L752), `IncursionResolutionNotAppended` (~L774), `RecoveryAction` (~L686), the retention-deletion loop (~L697).
  - `crates/cargo-berth/src/verb/release.rs` — release verb; journals blocking status (~L554-559).
  - `crates/cargo-berth/src/verb/claim.rs` — claim verb; stale-identity predicate (~L1181), retry message (~L1541).
  - `crates/cargo-berth/src/verb/check.rs` — check verb; cross-worktree identity check.
  - `crates/cargo-berth/src/verb/sequence.rs` — sequencing verb; duplicate stale-identity predicate (~L272).
  - `crates/cargo-berth/src/verb/integrate.rs` — integration verb; forced-permit consumption (~L114).
  - `crates/cargo-berth/src/edge/mod.rs` — `Edge::readiness` (~L316), which consults the snapshot's successor-incorporation evidence.
  - `crates/cargo-berth/src/edge/snapshot.rs` — successor/predecessor snapshot state; `SuccessorIncorporationEvidence` (~L69) and `PredecessorSuccessorIncorporation` (~L82). `SuccessorHeadReachability` no longer exists.
  - `crates/cargo-berth/src/alert.rs` — `Alert` enum (~L26), carrying both `LostIntegrationEvidence` (~L33) and `OrphanedOutstanding` (~L35).
  - `crates/cargo-berth/src/output.rs` — `OutputEnvelope` (~L74), `OutputStatus` (~L130), `ResolvePayload` (~L449), alert attachment/rendering (~L1765), wildcard consumer arm (~L1547), `first_touch_disposition_description` (~L1826).
  - `crates/cargo-berth/src/board/mod.rs` — board assembly; `ReservationRow` (~L134), omitted-row logic (~L625), row build (`reservation_rows` ~L767), `reservation_visibility` (~L859).
  - `crates/cargo-berth/src/board/tests.rs` — `assert_trunk_rewritten_action` (~L435).
  - `crates/cargo-berth/src/drift/classification.rs` — `PriorClassification` (pre-lock foreign-path role).
  - `crates/cargo-berth/src/drift/execution.rs` — drift driver; no-change fast return (~L161-170), fingerprint publish (~L219-220), claim rejection (~L423).
  - `crates/cargo-berth/src/drift/provenance.rs` — `commits_for_paths`, `path_commits` (~L80-105), `commit_origin` (~L124-145).
  - `crates/cargo-berth/src/drift/observation.rs` — `observe_full` (~L289) NUL-delimited path encoding.
  - `crates/cargo-berth/src/drift/identity.rs` — worktree/run identity handling for drift.
  - `crates/cargo-berth/src/drift/constants.rs` — git argument constants (~L26).
  - `crates/cargo-berth/src/gate/mod.rs` — `evaluate_reference_transaction` (~L370), `branch_rewrites` (~L479), `reanchor_rewritten_phases` (~L516), `commit_forced_permit_audits` (~L606).
  - `crates/cargo-berth/src/gate/install.rs` — `ManagedHook::script` (~L253), whose classifier and pass-through body spans ~L253-438; hook-path discovery use (~L109).
  - `crates/cargo-berth/src/git/command.rs` — `GitHookExecutionPolicy` (~L17), `GitCommandExecution` (~L25) and the shared `git_command` constructor (~L39).
  - `crates/cargo-berth/src/git/refs.rs` — private retention-ref name/write/transaction; `write` (~L38), `apply_transaction` (~L66); both suppress repository hooks.
  - `crates/cargo-berth/src/git/mod.rs` — git module surface; `hooks_directory` (~L295), `update_local_branch` (~L1175), `reachability` (~L1331), `reachability_to_target` (~L1357), `update_reservation_retention_refs` (~L1521) — the single batched repair-and-delete transaction.
  - `crates/cargo-berth/src/ledger/mod.rs` — `WorktreeContext` (~L102), `WorktreeContext::discover` (~L272), `worktree_identity`; append-only ledger in the common git dir.
  - `crates/cargo-berth/src/ledger/journal.rs` — `JournalActor` (~L116), journal event read/append, `ResolveIncursion` records (~L457).
  - `crates/cargo-berth/src/cli.rs` — CLI parsing/dispatch; `run_reference_transaction` (~L1050), malformed-input handling (~L1068-1072), phase dispatch (~L1115-1119), stdin read (~L1122-1126), bypass audit (~L1148-1205), embedded trunk ref at init (~L994-999).
  - `crates/cargo-berth/tests/drift.rs` — drift/attribution integration tests; `a_committed_incursion_names_the_commits_that_introduced_its_paths`, answered-incursion exit-0 case (~L384).
  - `crates/cargo-berth/tests/board.rs` — board JSON integration tests; `release_dispositions_remain_resolved_when_trunk_rewrites` (~L586) with its `resolved_audit`/`clear` assertions (~L691-701); the cold-proof cardinality guard `distinct_cold_proof_subjects_are_bounded_to_one_git_evaluation_per_target` (~L2461).
  - `crates/cargo-berth/src/git/mod.rs` — `reachability_to_target` (~L1357, two fixed invocations), `descendant_commits` (~L1408) with its `--ignore-missing` per-item membership guard (~L1426), and `DescendantCommitQuery` (~L1634).
  - `crates/cargo-berth/src/reservation/mod.rs` — `ReservationReplayError` (~L1960).
  - `crates/cargo-berth/tests/gate.rs` — git-gate integration tests; committed-phase permit consumption (~L1098, ~L1167-1176).
  - `docs/cargo-berth/json-contract.md` — the stable JSON wire contract for envelopes and journal records.
  - `docs/cargo-berth/berth-fix-evidence.md` — Appendix A (released-reservation investigation) and Appendix B (hook-cost measurements).
  - `/Users/natemccoy/.claude/scripts/berth/claim_state.py` — Python coordinator: board/claim/check dispatch, `STATUS_PAYLOAD_KINDS` (~L64) and `FIXED_STATUS_EXIT_CODES` (~L107) tables, `EnvelopePayload.alerts` (~L196) and `EnvelopePayload.data` (~L197), `CoordinationIdentityRejectionValue` (~L714) and `ForeignActorIncursionResolutionValue` (~L770) with the four recovery-action types and `RenderedCoordinationIdentityRecoveryActionValue` (~L721, its `kind: str` at ~L724), Phase 13's `ReleaseDispositionValue` (~L239), `ReservationLifecycleValue` (~L275), `UnknownReservationLifecycleValue` (~L291), `ReservationLifecycleQueryStateValue` (~L298) and `reservation_lifecycle_state` (~L2119), the public `coordinator_state` (~L2204) — Phase 13's rename of `_generic_state`, called by production `invoke` dispatch — whose generic `invalid_input` branch (~L2229) must stay below both identity classifiers — `_coordination_identity_classification` (~L2360) and `_foreign_actor_incursion_resolution_classification` (~L2386), each of which refuses any envelope whose status is not `invalid_input` — `installed_engine_binary` (~L2010) and the argument parser (~L2449).
  - `/Users/natemccoy/.claude/scripts/berth/install/hooks/berth_pre_edit.sh` — canonical PreToolUse shim; `valid_coordination_identity_rejection` (~L202), `render_coordination_identity_recovery_actions` (~L241), and the exit-5 branch that consumes them (~L397).
  - `/Users/natemccoy/.claude/scripts/berth/install/hooks/berth_post_bash.sh` — canonical PostToolUse shim; JSON validation (~L21), `typed_drift_feedback` (~L172), `valid_live_incursion_state` (~L254) requiring exclusive membership across the board's outstanding and recorded sections, and the gated single board read (~L424-441).
  - `/Users/natemccoy/.claude/scripts/berth/install/hooks/berth_session_start.sh` — canonical SessionStart shim; unchanged by the recovery-action work because `board` cannot emit identity rejections.
  - `/Users/natemccoy/.claude/scripts/berth/tests/test_hook_rendering.py` — the only external oracle for the three shims; eleven stdlib `unittest` fixtures driving them against a stubbed engine, run as `python3 ~/.claude/scripts/berth/tests/test_hook_rendering.py`. `pytest` is not installed on this machine.
  - `/Users/natemccoy/.claude/commands/plan/delegate.md` — `/plan:delegate`; lost-release recovery (~L1735-1799), which invokes `python3 -m berth.claim_state reservation` (~L1748) and reads the validated coordinator `state` — `kind = reservation_lifecycle` at exit 0 with exactly one lifecycle alternative, `kind = unknown_reservation` at exit 5 (~L1754-1772). It never consults the retention ref.
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
- **Hook-shim ownership** (established by Phase 7, binds every phase touching the PostToolUse or PreToolUse path): the running shims and the Python coordinator live at `~/.claude/scripts/berth/` — **exactly one copy of each exists, and there is no copy in this repository**. They therefore can never join a phase checkpoint commit; a phase that edits them says so in its report and leaves them uncommitted. They also have no in-repository test oracle: `tests/gate.rs` exercises the `reference-transaction` hook, not `berth_post_bash.sh`, so any assertion about shim rendering or shim cost belongs in the external fixture file `~/.claude/scripts/berth/tests/test_hook_rendering.py`, named explicitly by the phase that needs it.
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

### Phase 5 — Lost-evidence alert and `--integrated-as` eligibility  · status: done

#### As-built

`Alert` (`alert.rs`) is a two-variant enum: `OrphanedOutstanding` and `LostIntegrationEvidence`, the latter carrying reservation id, protected tip, evidence status, and a `LostEvidenceRecovery` split into `VerifyResolvedTrunk { trunk_oid: GitObjectId, action: RecoveryAction }` and `ResolveTrunkFirst { action: RecoveryAction }`, so the unresolved-trunk case is representable without emitting an unusable `--integrated-as <trunk-oid>` instruction. `ReconciliationAction::commit` derives the alert on every reconciliation from replayed journal state and the already-materialized `RepositorySnapshot::trunk()`, so the *first* drift envelope detecting a rewrite reports it; the derivation is pure — `recovery_evidence_query_count()` returns 0 for it, adding no Git subprocess and no per-reservation cost. The `recovery_operation` matcher (`recovery.rs:567`) admits `IntegrationEvidenceStatus::NotIntegrated` alongside `TrunkRewritten` and `ObjectUnknown`, keys on `evidence_state()` rather than blocking status, and rejects non-Git dispositions through `revalidation_subject`, so `--integrated-as` repairs a released, Git-backed row degraded by deferred comparison; the row itself still reports `Clear`. `output.rs` carries no alert-specific code — `with_alerts` is generic over `Alert`, and the new variant reaches the drift envelope unmodified. The alert text names plain `board --json` for inspection.

**Files:**
- `crates/cargo-berth/src/alert.rs` — `Alert::LostIntegrationEvidence` and `LostEvidenceRecovery`
- `crates/cargo-berth/src/reconcile.rs` — alert derivation from post-reconciliation evidence
- `crates/cargo-berth/src/recovery.rs` — `--integrated-as` eligibility keyed on lost Git evidence
- `crates/cargo-berth/src/board/mod.rs` — `BoardAlert::LostIntegrationEvidence`, recomputed per read
- `crates/cargo-berth/tests/{board,drift,liveness}.rs` — board visibility, first-envelope reporting, survival across invocations, unknown protected tip, unresolved trunk, legacy release-then-resnapshot replay
- `crates/cargo-berth/tests/gate.rs` — hook Git-cost expectation corrected to five `rev-list` calls
- `~/.claude/scripts/berth/install/hooks/berth_post_bash.sh`, `berth_session_start.sh` — validate and render `lost_integration_evidence` in both recovery forms
- `docs/cargo-berth/json-contract.md` — both serialized recovery forms

**Binds later work:** `Alert` is two variants; any match on it must handle `LostIntegrationEvidence`. The alert's wire shape lives in two hook shims that must change together. Its inspection command stays plain `board --json` — the board's per-reservation selector must not appear in this text. `alert::RecoveryAction` and `recovery::RecoveryAction` are unrelated types sharing one name, left for the naming pass. The alert's `evidence_status` field is typed `IntegrationEvidenceStatus`, which is wider than the three values the wire emits; that gap matters first to generated-schema work.

**Gotchas:** `ReleasedWithoutCheckpoint` cannot raise the alert — it carries no `integration_status` at all. `verify.sh test <pkg> <target>` is per-target, so any phase touching the PostToolUse Git path must name `gate` explicitly or the non-scaling invariant goes unmeasured. `hook_git_cost_scales_with_protected_graph_predecessors` pins absolute counts at a single graph size despite its name and cannot distinguish a constant increment from a per-reservation one.

**Ruled out:** a single alert payload with a mandatory `trunk_oid`, unconstructible for `ObjectUnknown`; a bare `Option<GitObjectId>`, which states neither state's meaning; narrowing `LostIntegrationEvidenceStatus` as a defect fix, since its constructor returns early on an integrated reservation and the impossible state is unreachable by construction.

### Phase 6 — Worktree identity: reproduce first, then one helper  · status: done

#### As-built

`ledger::resolve_identity(&WorktreeContext)` is the single entry point for actor identity. It
returns `ResolvedJournalMutationActor`, carrying `worktree_id`, `coordination_run_id`, and the
`EditAuthorization` resolved in that same read; `with_coordination_run_id` replaces the
context-selected run when a command owns its run identity. Every journal-mutating path routes
through it — claim, check, release, sequence, gate, permit, recovery, reconcile, and drift — and the
duplicated `mutation_run_id` helpers in `recovery.rs` and `verb/release.rs` are gone.
`WorktreeContext` distinguishes its two directories by type: `WorktreeAdministrativeDirectory` owns
worktree and run identity markers, `SharedLedgerDirectory` owns the journal and session mappings.

Every journal record carries `identity_inputs`, the process inputs available when its actor was
resolved: the invocation directory plus `CARGO_BERTH_SESSION_ID`, `CARGO_BERTH_RUN`, `GIT_DIR`, and
`GIT_COMMON_DIR`. Each is a tagged state rather than a bare string — the directory as
`utf8`/`too_long`/`non_utf8`/`unavailable`, each environment value as
`unset`/`utf8`/`too_long`/`non_utf8` — and each is bounded at
`MAXIMUM_RECORDED_IDENTITY_INPUT_VALUE_BYTES` (256 JSON-content bytes), with `too_long` retaining
only `observed_bytes`. The field is additive: records written before it omit it, and the internal
`Unrecorded` state never serializes.

**The reported misattribution does not reproduce.** A reproducer built strictly from what Appendix A
Defect 2 recorded — invocation directory, command route, journalled actors, marker contents — passes
against unmodified code: a linked worktree's resolve is attributed to the linked worktree. The
pre-existing helpers passed the administrative and shared-ledger directories in the correct order,
so the transposition the phase was written to correct never existed. The phase therefore took its
acceptance gate's specified alternative — instrumentation that makes a recurrence diagnosable — and
`docs/cargo-berth/berth-fix-evidence.md` records the negative result so it is not re-derived.

**Files:**
- `crates/cargo-berth/src/ledger/mod.rs` — `resolve_identity`, `ResolvedJournalMutationActor`, and the two typed directories on `WorktreeContext`
- `crates/cargo-berth/src/ledger/journal.rs` — `JournalMutationIdentityInputs` and the bounded `InvocationDirectoryAtMutation` and `EnvironmentValueAtMutation` states
- `crates/cargo-berth/src/ledger/constants.rs` — the 16 KiB record cap and the 256-byte identity-input ceiling
- `crates/cargo-berth/src/verb/{claim,check,release,sequence}.rs`, `gate/{mod,permit}.rs`, `recovery.rs`, `reconcile.rs`, `drift/{execution,identity}.rs`, `session/mod.rs` — call sites routed through the single entry point
- `crates/cargo-berth/tests/ledger.rs` — coverage for the recorded shape, the `too_long` overflow at 32 KiB, and actor attribution per worktree layout
- `docs/cargo-berth/json-contract.md` — the `identity_inputs` envelope and its nested states
- `docs/cargo-berth/berth-fix-evidence.md` — the recorded reproduction result

**Binds later work:** `resolve_identity` returns an actor, not paths — a consumer needing the
repository root reads it from `WorktreeContext`. Identity is resolved exactly once per invocation
and the `EditAuthorization` is taken from that result; a second read can disagree with the first
when a concurrent release retires the session mapping and marker between them. The wire's optional
`identity_inputs` envelope and its nested per-value states must be enumerated by the generated
contract, and its serialized bytes are journal evidence rather than replay state.

**Gotchas:** `Journal::append` hard-fails `RecordTooLarge` above 16 KiB, so anything copied into a
record needs a bound; five identity values at 256 bytes each leave ample headroom, but a sixth
recorded value must re-check it. The installed binary on a developer's path is routinely older than
the worktree build, so records written through hook-invoked commands carry no `identity_inputs` —
a smoke test must read the field from a record the fresh build wrote.

**Ruled out:** The transposition hypothesis — the source passed its directories in the correct
order. A reproducer synthesized from an environment the incident never recorded, which would have
proved only that the fixture matched itself. Making the instrumentation optional or raising the
record cap to accommodate unbounded values.

### Phase 7 — Report a resolve by what it accomplished  · status: done

#### As-built

A resolve of an incursion incident reports what it accomplished: `recorded_now` (exit 0, status `incursion_resolved`) when the invocation appended the disposition; `already_recorded_by_same_coordination_actor` (exit 0, same status) when the retained actor's worktree and coordination-run ids equal the caller's; `already_recorded_by_different_coordination_actor` (exit 5, status `invalid_input`, payload kind `resolve`) naming the resolving worktree id, coordination run id, resolution event id, and resolution time in typed fields. `IncursionIncidentStatus::Resolved` retains `resolving_actor: JournalActor` beside `resolution_event_id` and `resolved_at`; replay reconstructs it from the record's own `event.actor`, so earlier records replay unchanged and no journal lookup exists. `JournalActor::has_coordination_identity(worktree_id, coordination_run_id) -> bool` is the single comparison — responsibility means equality of the ids the journal recorded, never sameness of process. `IncursionResolutionNotAppended { AlreadyRecordedBySameCoordinationActor, Rejected(RecoveryRejection) }` separates a success that appended nothing from a rejection, and `RecoveryRejection::IncursionIncidentAlreadyResolvedByDifferentCoordinationActor { reservation_id, incident_id, resolving_actor, resolution_event_id, resolved_at }` replaces the id-only variant. `ResolvePayload::IncursionResolved` has no producer and is retained so older envelopes stay decodable. The PostToolUse shim gates its `STOP. Resolve with …` text on live board state, taking one constant `board --json` read only when a drift response names an incursion.

**Files:**
- `crates/cargo-berth/src/recovery.rs` — the three outcomes; `execute_one_incursion_resolution` (L271) checks `incident.reservation_id() != reservation_id` (L306) before classifying status; `RecoveryAction` (L686), retention-deletion loop (L697), `RecoveryRejection` (L749), `IncursionResolutionNotAppended` (L774)
- `crates/cargo-berth/src/reservation/mod.rs` — `IncursionIncidentStatus::Resolved` retains the actor (L416)
- `crates/cargo-berth/src/ledger/journal.rs` — `JournalActor::has_coordination_identity`
- `crates/cargo-berth/src/output.rs` — `ResolvePayload` (L416): three variants plus the different-actor envelope constructor
- `crates/cargo-berth/src/board/mod.rs` — the exhaustive `IncursionIncidentStatus` match, which gained a field
- `crates/cargo-berth/tests/drift.rs` — `linked_worktree_resolve_reports_recorded_same_actor_and_foreign_actor_outcomes` and its assertion helpers; reservation-mismatch case at L448
- `docs/cargo-berth/json-contract.md` — `## Resolve incursion outcomes`

**Gotchas:**
- `RetainedReservationSet::incursion_incident` is a global lookup across every reservation's incidents. Status must never be classified before the incident-belongs-to-reservation check, or a resolve naming an unrelated reservation exits 0 with a success payload naming an unvalidated id.
- The Python coordinator's `STATUS_PAYLOAD_KINDS` gates which payload kinds a status may carry; an engine payload variant is unreachable through the coordinator until that table names it (`invalid_input` gained `resolve` by hand).
- `_generic_state`'s generic `invalid_input` early return is order-sensitive: `sequence` and `integrate` inactive-identity rejections carry the same status, so it must stay below `_inactive_identity_classification` or those typed states are silently erased.
- The shims and coordinator live at `~/.claude/scripts/berth/`, outside this repository, and cannot be committed with a phase.
- Integration tests that shell out to the binary supply replay coverage for free: every repeat `resolve` is a fresh process reconstructing the incident from the journal.

**Ruled out:** resolving `resolution_event_id` back through the journal to recover the actor — returns a bare optional at a boundary that must be total; deleting `ResolvePayload::IncursionResolved` once it lost its producer — older envelopes must stay decodable; treating the foreign-actor outcome as a `CoordinationIdentityRejection` — both identities are valid, so an identity-clearing recovery does not apply.

### Phase 8 — Git-hook phase/ref dispatch table  · status: done

#### As-built

The generated `reference-transaction` hook classifies phase/ref pairs in shell and spawns the binary only for actionable ones: `preparing`, `aborted`, and unknown phases exit before the binary; `prepared` invokes only when the transaction names the configured trunk ref exactly (complete third field, never a substring); `committed` invokes for any local `refs/heads/*`. The same filter gates the bypass recording. A three-commit rebase costs 1.03s live and 0.88s with the release valve, against 7.97s and 5.44s before. Two classifier stages run per fire at fixed cost independent of ref count: `LC_ALL=C grep -q` for any byte outside tab and printable ASCII routes straight to the binary — grep's own error exit counts as a bad byte — then one `awk` pass classifies the surviving records. Stdin is copied to a protected temporary file and the **unchanged bytes** are redirected into the binary; a buffering failure refuses and prints a retry instruction rather than replaying a partial transaction, and malformed `prepared` input still reaches the binary so the deliberate parse failure and the unconfirmed-bypass audit stay live.

Trunk-rename refresh keys on the **deletion alone**, since `git branch -m` emits only the delete: `local_branch_replacement_tip_matches` finds branches sharing the deleted trunk's tip, and `local_branch_rename_proof` admits a candidate only when its newest reflog subject matches `Branch: renamed {deleted} to {candidate}` exactly. `LocalBranchRenameProof` accumulates and short-circuits to `MultipleMatches` at the second proof; zero or several proofs leave the hook untouched. The rewrite runs in the hidden `refresh-managed-hook-after-trunk-deletion` subcommand, spawned detached because it cannot run inside the hook that triggered it, and `PendingManagedHookReplacement` keeps the swap atomic so a failed write leaves the previous hook rather than an empty permissive one. When the embedded ref names a branch that no longer exists, the `prepared` row **invokes** — skipping is never a failure mode this dispatch table produces. Engine-side git calls go through the typed boundary (`git/command.rs` helpers, literals in `git/constants.rs`); git inside the rendered shell script is outside that boundary.

**Files:**
- `crates/cargo-berth/src/gate/install.rs`, `crates/cargo-berth/src/gate/mod.rs` — the rendered dispatch script and its placeholder substitution
- `crates/cargo-berth/src/git/mod.rs` — `local_branch_replacement_tip_matches`, `local_branch_rename_proof`, `LocalBranchRenameProof`, `GitError::InvalidReferenceName`
- `crates/cargo-berth/src/git/constants.rs` — for-each-ref and reflog literals; `GIT_COUNT_THREE_REFS_ARG` removed
- `crates/cargo-berth/src/cli.rs` — refresh-path diagnostics
- `crates/cargo-berth/tests/gate.rs` — classifier, replay, rename-proof, and ambiguity regressions, including `renamed_trunk_refreshes_dispatch_before_next_prepared_update`, `deleting_trunk_with_two_proven_same_tip_renames_leaves_dispatch_unchanged`, and `managed_hook_never_replays_a_partial_transaction_after_buffering_fails`

**Binds later work:** The two classifier children are a fixed per-fire cost berth's own retention-ref writes still pay — scoped hook suppression on retention-ref writes must reach **zero classifier children**, not merely zero binary invocations, expressed inside the classifier and preserving the invariant that anything unclassifiable invokes the binary. `grep` and `awk` run inside the git hook, before the shim, so they sit outside the PostToolUse 0.20-second budget; proving that budget requires a raw argv trace. `tests/gate.rs` drives the git `reference-transaction` hook and never executes `berth_post_bash.sh`, so it is not a PostToolUse shim oracle. Reflog queries scale with the number of local branches sharing the deleted trunk's tip, bounded by early exit at the second proof and run once per trunk deletion — outside the non-scaling axis (paths, commits, reservations) that fixed-subprocess-count drift provenance governs.

**Gotchas:**
- `awk` silently truncates a record at NUL (host BWK 20200816: `printf 'aaaa\0bbbb\ncccc\n'` yields a 4-byte first record), so the byte scan must run *before* awk — that is why `grep` is a separate process instead of an awk condition.
- An absent reflog proves nothing: reflogs can be disabled, and `NotRecorded` leaves the hook alone rather than inferring a rename from a shared tip.
- `for-each-ref` sorts lexicographically, so any discovery cap silently decides which candidates a downstream proof filter ever sees.
- The rename refresh fires only when the deleted ref *is* the hook's current trunk; building proofs by hand requires hooks disabled (`git -c core.hooksPath=/dev/null`).
- `verify.sh test cargo-berth` resolves lib and bin targets only and cannot see `crates/cargo-berth/tests/`; every integration target must be named separately — eleven commands for this package.
- After a proven rename the hook and `.claude/config/berth.toml` disagree on the trunk name, and re-running `cargo berth init` reverts the hook to the stale configured value, silently undoing the refresh; reconciling the two is on the next-items backlog.

**Ruled out:**
- Bounding candidate discovery with `--count=N` — a cap above a proof filter decides `ExactlyOne` over an arbitrary truncated subset; the second-proof short-circuit bounds it instead.
- Shared-tip inheritance alone as rename proof — one unrelated same-tip branch silently repointed the hook, and because that branch exists the stale-ref fail-safe cannot catch it.
- Treating a non-printable byte as skippable — a false negative drops the trunk gate silently, a false positive costs one invocation.
- A prepared-only filter — `committed` reanchors phase starts after local branch rewrites and consumes forced-integration permits, so skipping it leaves stale anchors and reusable permits.

### Phase 9 — Scoped hook suppression on retention-ref writes  · status: done

#### As-built

`git/command.rs` carries `pub(super) enum GitHookExecutionPolicy { Enabled, SuppressedForRetentionRef }`; `git_command` defaults to `Enabled` and only the private retention-ref writes in `git/refs.rs` name `SuppressedForRetentionRef`, so `cargo-berth init` hook discovery and the `cargo-berth integrate` trunk update still fire hooks. Suppression sets `core.hooksPath=/dev/null` through the `GIT_CONFIG_COUNT`/`GIT_CONFIG_KEY_n`/`GIT_CONFIG_VALUE_n` environment overlay rather than a `-c` argv flag, appending after whatever overlay the environment already carries and treating a count at `usize::MAX` as absent. Repair and deletion are one call, `git::update_reservation_retention_refs(repairs, deletions)`, issuing a single `update-ref --stdin` transaction per pass; `git::delete_reservation_retention_ref` and `refs::delete` are gone rather than kept as suppressed wrappers, and `reconcile.rs`, `recovery.rs`, and `verb/release.rs` each call the batched API in place of their per-reservation loops.

**Files:**
- `crates/cargo-berth/src/git/command.rs` — `GitHookExecutionPolicy`, the env-overlay suppression, and re-exec-self unit tests for overlay preservation and the `usize::MAX` fallback
- `crates/cargo-berth/src/git/refs.rs` — retention-ref writes and deletions, the only suppressed call sites
- `crates/cargo-berth/src/git/mod.rs` — `update_reservation_retention_refs`
- `crates/cargo-berth/src/git/constants.rs` — `GIT_DELETE_REF_ARG` removed with its last caller
- `crates/cargo-berth/src/{reconcile.rs,recovery.rs,verb/release.rs}` — batched retention-ref call sites
- `crates/cargo-berth/tests/gate.rs` — `retention_ref_writes_and_deletions_suppress_the_repository_root_hook`, `retention_ref_transactions_have_constant_git_invocations_across_cardinalities`, `assert_one_suppressed_ref_transaction` (pins argv `git --no-optional-locks update-ref --stdin`, ~L3341), plus `RAW_TRACING_GIT_WRAPPER`, `RawGitInvocation`, `run_berth_with_raw_git_trace`, and the argv-comparison helpers

**Binds later work:** Hook suppression is deliberately invisible in recorded argv, so later phases measuring raw unfiltered argv traces extend these helpers rather than creating them, and a change moving suppression onto the command line shifts every recorded trace without failing a test. A suppressed retention-ref write spawns zero `grep` and zero `awk` classifier children by construction — the classifiers live inside the hook script, which never runs.

**Gotchas:** `core.hooksPath = ""` does not resolve to the repository root on this machine's git — git rejects the empty path outright (`fatal: The empty string is not a valid path`), so a hook configured that way never fires under any condition; the suppression test therefore installs its sentinel in a real `core.hooksPath` directory, proves the hook fires there with an unsuppressed control ref, and filters on `" refs/cargo-berth/"` entries. Live-tree line anchors for `git/command.rs`, `git/refs.rs`, and `git/mod.rs` had drifted by up to ~1,050 lines from the plan's Delegation Context.

**Ruled out:** `-c core.hooksPath=/dev/null` on argv — it makes every before/after argv trace incomparable. Suppression inside `git_command` — it would break `init` hook discovery, stop `integrate`'s permit-consuming trunk transaction, and silently skip a user's unmanaged hook. A suppressed compatibility wrapper for the old deletion helper — a second deletion path is how the per-reservation scaling returns. Keeping `GIT_DELETE_REF_ARG` — it would need a module-wide dead-code suppression blinding the lint for every future constant there.

### Phase 10 — Fixed subprocess count for drift provenance  · status: done

#### As-built

Incursion attribution runs a fixed number of git subprocesses regardless of how many paths, commits, or reservations a run involves. Three batched queries replace the per-path loop: one union-base resolution over the usable phase-start anchors, one path log covering every entered path, and one range-membership query covering every anchor.

`IncursionAttributionAnchorState` (`UsableAncestor` / `NotAncestorOfHead` / `ObjectUnknown`) records each phase-start anchor's relation to the target, so an unreadable anchor reports nothing rather than defaulting into a false `Unchanged`. `IncursionCommitOriginBasis` (`ResolvedTrunk` / `CannotClassifyOrigin`) and `IncursionCommitOriginMembership` (`Classified` / `CannotClassifyOrigin`) carry origin classification as a semantic state: a failed trunk lookup or failed origin query costs only the `origin` field, and the commits, subjects, and paths already established are still reported.

Every incursion commit in the `drift` payload carries `origin`, one of `phase_authored`, `already_on_trunk`, or `unknown`. `GitCommandExecution` (`Completed(Output)` / `CouldNotRun(io::Error)`) is `pub(crate)` with `From<io::Result<Output>>`, keeping spawn failure and completed non-zero exit distinct at every read-only git call site so the errno survives. `IncursionPathLogInvocation` returns the executed argument vector beside the execution outcome, so a failed path log names the command that actually ran.

**Files:**
- `crates/cargo-berth/src/drift/provenance.rs` — batched attribution, anchor and origin state, per-commit origin assignment
- `crates/cargo-berth/src/drift/observation.rs` — anchor reachability, the batched phase-start diff, the anchor-state map
- `crates/cargo-berth/src/drift/git_output.rs` — the process-outcome boundary and the path-log parser
- `crates/cargo-berth/src/git/mod.rs` — the batched primitives and `IncursionPathLogInvocation`
- `crates/cargo-berth/src/git/command.rs` — `GitCommandExecution` and `git_execution`
- `crates/cargo-berth/src/output.rs`, `crates/cargo-berth/src/drift/report.rs` — the `origin` field on the wire
- `docs/cargo-berth/json-contract.md` — documents `origin` and its three values
- `crates/cargo-berth/tests/gate.rs`, `crates/cargo-berth/tests/drift.rs` — the raw-argv cardinality oracle, differential fixtures, and selective git-failure injection

**Binds later work:** `GitCommandExecution` is `pub(crate)`, carries `Completed`/`CouldNotRun`, and converts from `io::Result<Output>`, so the typed coordination-identity rejections reuse that boundary instead of adding a parallel one. The generated status, exit-code, and payload contract must enumerate `origin` and its three wire values, and must express `unknown` as a normal value of a complete response rather than a degraded one. The semantic-roles audit finds `GitCommandExecution` and the three attribution state types already named for their roles and free of bare `Option<T>`.

**Gotchas:** Batching a per-item query silently converts a degradable failure into a fatal one — the origin query maps its failure to `CannotClassifyOrigin` and must never propagate with `?`, while the surrounding errors in the same function still propagate deliberately. A filter that drops unreadable anchors combined with a collector that defaults the gap produces a confident wrong answer instead of an error. `anchor..HEAD` and "descendants of anchor" are different sets: a commit merged from a branch that forked before the anchor is in the first and not the second. The cardinality oracle must record every invocation's complete argv before classifying; a command-name-only trace hides real scaling defects. `crates/cargo-berth/src/drift/observation.rs:485` still passes the label `["diff-tree", "batched phase starts"]` where the executed argument vector belongs, so a failure of the batched phase-start diff reports prose rather than a runnable command; the repair is the named-invocation type `IncursionPathLogInvocation` already uses.

**Ruled out:** Widening `GitCommandExecution` through a facade in `git` — it is `pub(crate)` directly instead. Reusing `TracedDrift::fingerprint_commands` or the command-name-only wrapper as the cardinality gate — the raw-argv helpers in `tests/gate.rs` were extended instead.

### Phase 11 — Typed coordination-identity rejections  · status: done

#### As-built

One `validate_coordination_identity` serves the git gate and every ordinary verb; the gate's private `ActingRun` enum is gone. It returns `CoordinationIdentityRejection` — stale session mapping, stale marker run, or session/worktree mismatch — each carrying a non-empty `CoordinationIdentityRecoveryActions`, so the human message and the machine payload render from one source.

Every published recovery `argv` is a `RunnableRecoveryCommandLine(Vec<String>)`, produced only through a fallible conversion from the lossless boundary type `RecoveryCommandLine(Vec<OsString>)`. A command that cannot be represented as text is omitted from the action set rather than published in degraded form: `RerunFromHoldingWorktree` is the only omittable action, and `ClaimSeparatelyHere` always remains, so the set is non-empty and every member is directly executable.

The installed `reference-transaction` hook exports `CARGO_BERTH_REFERENCE_TRANSACTION_ISSUING_DIRECTORY=$PWD` before it changes directory to the policy worktree, and the binary reads it into `ReferenceTransactionIssuingDirectory` (`CapturedByManagedHook(PathBuf)` / `MissingFromLegacyHook`). There is no fallback. A hook installed before this phase yields `MissingFromLegacyHook`, and the gate returns `GateError::LegacyReferenceTransactionHook` before resolving any worktree; the refusal exits 5 and names both repairs — rerun `cargo-berth init` to reinstall the hook, or set `CARGO_BERTH_BYPASS=1` to proceed immediately.

`identity clear-session` is a real CLI verb with `CommandVerb::Identity`, the statuses `SessionMappingCleared` and `SessionMappingUnavailable`, and three `IdentityPayload` outcomes distinguishing a removed mapping, an already-absent one, and an unavailable current session. `ledger::ResolvedEditAuthorization::with_coordination_run_id` is gone; `journal_mutation_actor_for` is the single path, called from gate, reconcile, claim, sequence, and drift. Unenrolled `drift` no longer creates a worktree-id file before answering `unconfigured`.

**Files:**
- `crates/cargo-berth/src/coordination_identity.rs` — the shared validation, the rejection and its recovery actions, and both command-line types
- `crates/cargo-berth/src/gate/mod.rs` — `ReferenceTransactionIssuingDirectory` threaded through gate evaluation; `GateError::LegacyReferenceTransactionHook` and its message
- `crates/cargo-berth/src/gate/install.rs` — the hook script template that exports the issuing directory
- `crates/cargo-berth/src/cli.rs` — builds the issuing-directory value from the environment, renders the legacy-hook refusal, and hosts `identity clear-session`
- `crates/cargo-berth/src/verb/integrate.rs` — exhaustive handling of the new gate error
- `crates/cargo-berth/src/output.rs` — `CommandVerb::Identity`, both statuses, the three identity payload outcomes, and the top-level `coordination_identity` facts
- `crates/cargo-berth/tests/gate.rs` — legacy-hook refusal, current-hook success, and the linked-worktree regression
- `docs/cargo-berth/json-contract.md` — the guarantee that a published `argv` is directly executable

**Binds later work:** Recovery actions expose `argv`/`cwd` pairs that are directly executable, and the set's membership varies — a front end must not assume a rerun action is present. The generated status, exit-code, and payload contract must enumerate `CommandVerb::Identity`, both new statuses, all three identity outcomes, and the rejection in both wire placements — top-level `coordination_identity` and nested inside the integration payload — as one type rather than two. `integrate` currently folds `GateError::LegacyReferenceTransactionHook` into `ledger_unreadable`, so that contract also owns giving it a status meaning "the installed hook is out of date". The semantic-roles work must not re-propose replacing `RecoveryCommandLine`'s inner `Vec<OsString>`: the representability question is answered by the separate runnable type, and `with_coordination_run_id` no longer exists to be audited.

**Gotchas:** The hook script changes directory to the policy worktree before invoking the binary, so the process's own working directory is never the checkout that issued the transaction — gate work needing the issuing checkout reads the exported variable, never `current_dir`. `install_managed_hooks` does refresh a stale script, but only `cargo-berth init` calls it, so nothing refreshes a hook on an ordinary invocation and an out-of-date hook is a reachable state that must be refused rather than assumed away. A recovery command is built from `std::env::args_os()`, so whether it is representable depends on how the process was invoked, not on anything the ledger holds.

**Ruled out:** Falling back to the process's working directory when the hook reports nothing — that is the silent wrong answer this phase removes. Skipping identity validation for a legacy hook — it would reopen the hole this phase closed, and the bypass variable is already the deliberate escape. Rendering a non-representable command with replacement characters and a notice — the argv is meant to be executed verbatim, so a damaged one is omitted instead.

### Phase 12 — Front ends render recovery actions without parsing messages  · status: done

#### As-built

Every canonical front end renders a coordination-identity rejection from its typed
`recovery_actions` — `argv` plus `cwd` — and never from `message`. `claim_state.py`
carries `CoordinationIdentityRejectionValue` (`:621`) covering Phase 11's three
rejections in both wire placements, direct under `payload.kind = coordination_identity`
and nested under `integrate.data.reason`, alongside four recovery-action types and
`RenderedCoordinationIdentityRecoveryActionValue` (`:628`), which pairs each typed
action with the directly runnable `cd <cwd> && <argv>` line the shims print.
`ForeignActorIncursionResolutionValue` (`:677`) classifies a resolve answered by another
worktree from its typed actor, run, event, and time fields, preserving exit 5.

**Both classifiers are bound to the envelope's own status.** Each returns its
no-rejection state unless `envelope.status == "invalid_input"`, so an engine response
tagged as a success can never be rendered as a refusal. Rendering is driven by whichever
actions the payload carries, never by an expectation that a particular action is present.

`berth_post_bash.sh` reads the live board exactly once when the drift response names an
incursion and not at all when it does not, and `valid_live_incursion_state` requires an
incident to appear in exactly one of the board's outstanding and recorded sections
(`$is_outstanding != $is_recorded`), so a contradictory or unreadable board emits the
`STOP` text rather than silently suppressing it. `berth_session_start.sh` needed no
change: `board` cannot emit these rejections, and both its lost-evidence branches are
now covered.

**Files:**
- `~/.claude/scripts/berth/claim_state.py` — the identity and foreign-resolver value
  types, their status-bound classifiers, and the runnable-command renderer
- `~/.claude/scripts/berth/install/hooks/berth_pre_edit.sh` — renders recovery actions in
  the exit-5 branch; `.message` survives only where the payload is prose by design
- `~/.claude/scripts/berth/install/hooks/berth_post_bash.sh` — the same rendering, plus
  the gated single board read and the exclusive-membership incursion check
- `~/.claude/scripts/berth/tests/test_hook_rendering.py` — nine stdlib `unittest`
  fixtures driving the real shims against a stubbed engine, run as
  `python3 ~/.claude/scripts/berth/tests/test_hook_rendering.py`

**Binds later work:** `test_hook_rendering.py` is the only external oracle for the three
shims and the file every later shim assertion extends rather than replaces; nothing in
the repository executes them. Every envelope in it carries a sentinel `message` and every
assertion is that the sentinel is absent from the rendered output — that is how
message-free rendering is proved, so a new fixture must follow the same shape. The
exclusive-membership board check asserts agreement between two sections and cannot be
replaced by schema validation of either one. `RenderedCoordinationIdentityRecoveryActionValue.kind`
is `str` where the four action tags are closed literals; it is invisible today only
because `tagged()` erases the value to `dict[str, object]`, so the phase that removes that
erasure must close the tag at the same time.

**Gotchas:** all four files live outside any git repository, so this work can never appear
in a commit or a `git diff` — an empty diff here is correct, not a missing implementation.
The coordinator resolves `cargo-berth` through `shutil.which`, which finds the installed
binary rather than any build under test, so a fixture setup that does not pin the intended
binary on `PATH` proves nothing about its own changes. The installed binary has drifted
behind this plan and cannot replay a journal a fresh build wrote; inside a throwaway
repository that surfaces as `journal record 1 is corrupt`, which is version skew rather
than a defect. A throwaway repository must be created with `git init -b main`, because
`cargo-berth` resolves `refs/heads/main` and reports the ledger unreadable on a `master`
default. `pytest` is not installed on this machine.

**Ruled out:** the two-name `inactive_session_mapping` / `inactive_marker_run` taxonomy,
which described a shape the engine had already stopped emitting. Rendering keyed to a
fixed roster of actions, since an action whose command cannot be reproduced faithfully is
omitted from the set rather than degraded. A separate exit-code assertion beside a status
assertion, because the fixed status-to-exit table makes binding the status sufficient.

### Phase 13 — Named reservation lifecycle query  · status: done

#### As-built

- `cargo-berth board --reservation <reservation-id> --json` returns a placement-independent lifecycle read for one reservation, covering rows the board deliberately omits (a waiting successor, either endpoint of an unresolved overlap); `--reservation` requires `--json`, is rejected at the command line otherwise, and a fixture proves the selector never reaches the TUI path.
- `ReservationLifecycleSnapshot` (`reservation/mod.rs:529`) is projected from `Reservation::evidence_state` rather than re-matched: `Active`, `Outstanding { protected_tip }`, `ReleasedAfterCheckpoint { protected_tip, disposition }`, `ReleasedWithoutCheckpoint { disposition }`. An unknown id rejects as `ReservationLifecycleQueryRejection::UnknownReservation { reservation_id }` (`output.rs:365`), never `Option`.
- The out-of-repo coordinator (`claim_state.py`) gained a validated `reservation` entry point; `/plan:delegate`'s lost-release recovery (`delegate.md:1765–1799`) now calls it, triggered by a `released` disposition as well as `released_after_checkpoint`, so a reservation released between the checkpoint append and the observed reply still clears the durable record.
- The freshly built engine was installed to `PATH` before the coordinator switch, proved by a fixture asserting `argv[0]` is the installed binary.

**Files:**
- `crates/cargo-berth/src/cli.rs` — the selector, its `--json` requirement, conversion into `BoardOutputSelection`
- `crates/cargo-berth/src/verb/board.rs` — dispatches on the selector
- `crates/cargo-berth/src/board/mod.rs` — placement-independent lookup (~L632-646)
- `crates/cargo-berth/src/reservation/mod.rs:529` — `ReservationLifecycleSnapshot`
- `crates/cargo-berth/src/output.rs` — untagged `ReservationLifecycleQueryPayload`; `UnknownReservation` rejection (`:365`)
- `crates/cargo-berth/tests/board.rs` — four-state, unknown-id, waiting-successor, overlap-endpoint fixtures
- `/Users/natemccoy/.claude/scripts/berth/claim_state.py` — `reservation` entry point and validator; public `coordinator_state()` and `installed_engine_binary()`; the value types listed below
- `/Users/natemccoy/.claude/scripts/berth/tests/test_hook_rendering.py` — eleven fixtures
- `/Users/natemccoy/.claude/commands/plan/delegate.md` — lost-release recovery calls the entry point
- `docs/cargo-berth/json-contract.md` — the new payload

**Binds later work:** The `reservation` payload is two wire shapes under one payload kind (`ReservationLifecycleQueryPayload` is `#[serde(untagged)]`): success is `board`/`board_ready`/exit 0/kind `reservation` carrying `data.lifecycle.status`; rejection is `board`/`invalid_input`/exit 5/kind `reservation` carrying `data.status = unknown_reservation`. Neither member exists on the other shape, and `invalid_input` now admits the `reservation` kind. The snapshot's four alternatives reach `ProtectedReservationTip` (`reservation/evidence.rs:27`), `ReleaseDisposition` (`reservation/lifecycle.rs:179`), and `ReservationId` (`ids.rs:105`) recursively. `BoardArguments.reservation: Option<ReservationId>` sits solely at the clap boundary, converted by `into_output_selection()` before any internal function sees it. `_generic_state` was renamed to the public `coordinator_state`; `installed_engine_binary` is also now public; both have production callers. `ReleaseDispositionValue` (four members, `integrated` carries no `evidence`), `ReservationLifecycleValue` (four members), `UnknownReservationLifecycleValue` (`kind = unknown_reservation` — the normalized form, distinct from the wire rejection's `data.status`), `ReservationLifecycleQueryStateValue`, and the validator `reservation_lifecycle_state` ship here and are to be lifted or renamed later, never recreated.

**Gotchas:** `/Users/natemccoy/.claude` is its own Git checkout with its own `pyrightconfig.json`; basedpyright must run from that directory as project root, and a type-check gate must name every edited file explicitly — naming only `claim_state.py` once silently skipped `test_hook_rendering.py` (9 errors, 24 warnings). Files under `~/.claude` can never join a `cargo-berth` checkpoint commit. `pytest` is not installed on this machine; the shim oracle runs as `python3 <path>`.

**Ruled out:** a typed capability fallback for the pre-install coordinator window — once the engine is installed the fallback has no consumer, and permanent surface for a transitional state is the speculative API this plan avoids elsewhere.

### Phase 14 — Generated status, exit-code, and payload contract  · status: done

#### As-built

Four declaration macros pair each variant with its pinned wire name in one list and generate both a test-visible inventory constant and an exhaustive match: `declare_output_contract_metadata!` and `declare_wire_enum!` (statuses, verbs, and their fixed exit codes), `declare_journal_operations!`, and `declare_trunk_observation_at_claim!`. `output_contract.rs` generates the contract in-crate rather than from a build script — `generate_output_contract()`, `consumer_artifacts_from_contract()`, `embedded_consumer_artifacts()` — one test writes `docs/cargo-berth/generated/output-contract.json` on request and the ordinary run byte-compares a fresh in-memory regeneration against it. The contract's unit is the whole outcome tuple (verb, envelope status, exit code, payload kind, nested discriminants); retained legacy outcomes are tuples marked `decodable_only` rather than omitted, and `reblocked_active_constraint` is reserved that way. Every `schemars` definition name is pinned to its wire name, so no Rust rename can move the generated bytes. `ReplayFailurePayload { reason, subject, effect }` types every replay hard stop: `reason` is generated exhaustively from `ReservationReplayError` and `ForcedIntegrationPermitReplayError`, `subject` is a three-arm identity union (`Reservation` / `IncursionIncident` / `ForcedIntegrationPermit`), and `effect` is `HardStop`. `OutputStatus::LegacyHookOutdated` carries exit `LedgerUnreadable` and is emitted at `verb/integrate.rs:195` so the repair is `cargo-berth init` without reading `message`. `LostIntegrationEvidenceStatus` (three variants) replaces the wider integration-evidence type at the alert's single construction site, making the fourth wire value unrepresentable. Python-side, `classify_claim` and `_validate_board` both route through `_replay_failure_classification`; typed board replay states carry a tagged `operator_route` instead of the generic `remedy` string. The three shims load their generated `jq` fragment from disk at startup and expose an installation state of `Ready` or `NeedsRepair`, the latter naming the unreadable validator by path.

**Files:**
- `crates/cargo-berth/src/output_contract.rs` — the generator, the outcome-tuple rules, and the fixture and byte-compare tests
- `crates/cargo-berth/src/output.rs` — the metadata declaration table, `ReplayFailurePayload` / `ReplayFailureSubject` / `ReplayFailureEffect`, `LegacyHookOutdated`, exhaustive consumer matches with no wildcard arm
- `crates/cargo-berth/src/alert.rs` — `LostIntegrationEvidenceStatus` and the conversion at construction
- `crates/cargo-berth/src/ledger/journal.rs` — the journal-operation and trunk-observation declaration macros
- `docs/cargo-berth/generated/output-contract.json` — the one generated artifact in this repository
- `/Users/natemccoy/.claude/scripts/berth/generated/{status_payload_tables.py,envelope_validation.jq}` — derived from that contract alone; canonical outside this repository
- `/Users/natemccoy/.claude/scripts/berth/install/install.sh` — builds the engine, places it on `PATH`, and regenerates both derived artifacts in one run; names the failing step and gates rollback on per-destination `Untouched` / `ReplacementStarted` publication state

**Binds later work:** `claim_state.py` now has six classifiers, not five — `_replay_failure_classification` returns a `ReplayFailure` reached through `.tagged()` on all four classification paths, and the replay failure arrives under `ledger_unreadable`, never `invalid_input`. Current classifier lines: `classify_claim` 1485, `classify_check` 1714, `_validate_board` 1800, `render_board` 1893, `coordinator_state` 2190, `_replay_failure_classification` 2375. `STATUS_PAYLOAD_KINDS` and `FIXED_STATUS_EXIT_CODES` are no longer hand-kept; they are generated into `scripts/berth/generated/status_payload_tables.py`. Any timing measurement of `berth_post_bash.sh` must state which installation state it measured, because the `NeedsRepair` branch returns before the engine is invoked. Rust type renames are provably contract-neutral. The external shim oracle is now fourteen tests, covering replay routing, unreadable-validator diagnostics, and installer rollback ordering.

**Gotchas:** The installer's rollback trap must be armed only after its backups exist — armed earlier, a failure destroys a working installation with nothing to restore. A stub fallback that fails closed is not safe: the shims once refused the edit correctly while blaming the engine for a broken local installation, sending the operator to the wrong place. The installed copies under `~/.claude/scripts/berth/` are byte-compared by no engine test; drift between the two trees is visible only in that repository's history, and `install.sh` refreshing engine and artifacts together is what keeps the gap small.

**Ruled out:** A `build.rs` generator, and a library target added to expose private DTOs to one — `cargo-berth` is a pure binary and a build script cannot see its types. A checked-in hand-maintained manifest, and four independent inventories of statuses, exit codes, payload kinds, and nested statuses — every value is individually legal, so only the tuple rejects a success envelope carrying a rejection sub-status. One `reservation` payload shape with an optional nested status, which would make a mixed envelope legal to both wire shapes at once. Relocating the canonical shims into this repository. `schemars` default `$defs` keys, which track Rust identifiers.

### Phase 15 — Prove the PostToolUse path stays inside 0.20 seconds  · status: todo

#### Work Order

**Goal:** Every PostToolUse outcome is measured under a reproducible protocol and lands inside the published bound.

**Spec:**

The complete two-reservation PostToolUse call was measured at 0.259 seconds and one automatic-widen invocation at 0.180 seconds. One sample and one outcome do not satisfy the published bound, and this cost is paid after every Bash call in an enrolled repository.

**State what the bound covers**: `berth_post_bash.sh` alone, not every globally registered PostToolUse command — the matcherless random-ack and context-usage hooks in `~/.claude/settings.json` are outside berth's control.

**The bound also has to state its largest input, because one of the rendered payloads is not fixed-size.** Phase 12 made both shims render every published recovery action as a runnable command line, and nothing on the wire caps how many arguments an action carries or how long they are. A wall-clock claim measured against the shapes the producer happens to emit today is a claim about those shapes, not about the payload. Scope the published bound rather than inventing a wire limit: state the maximum action set the bound was proved against, and measure the shapes that actually occur — the session-mismatch set and the worktree-mismatch set, which differ in both action count and argument length. A wire byte bound is deliberately not added: it is new protocol surface with no consumer asking for it, and the honest published bound is the one naming what it measured. If a later producer emits a larger set, the bound is re-measured, not silently inherited.

**Every timed sample starts from an independently restored state, restored outside the timer.** Five consecutive calls against one live state cannot retain the named outcomes: an ordinary widen changes the scopes on the first call, first-touch creates the reservation on the first call, and a successful non-blocking call publishes a new fingerprint at `drift/execution.rs:193` and `:248` so the next call observes no delta.

Where the cost actually is, so the work targets the right term: the wrapper's fixed process cost dominates most outcomes — a clear outcome launches eight executables (Bash, `cargo-berth`, four `jq`, `mktemp`, `rm`) and a rendered widen/incursion/collision response launches ten. Provenance batching (Phase 10) helps only the attribution outcome: `name_incursion_commits` runs after classification, a no-change call returns at `drift/execution.rs:188-196`, and a widen-only result performs no provenance git query.

Using the globally registered canonical path and production JSON, take at least **five cold and five warm** samples for each of: typed clear, ordinary widen, first-touch acquisition, foreign-only incursion, `post_write_incursion` with `protection.status = acquired`, `post_write_incursion` with `protection.status = not_acquired`, collision, attribution, and **the Phase 5 lost-evidence alert**. Each timed berth invocation finishes within 0.20 seconds.

The lost-evidence alert is not a free ninth row: it adds envelope validation and rendering to the PostToolUse path, and its own generation runs against post-reconciliation evidence. "Every outcome" excludes it only if it is never measured.

**One alert sample is not enough for that row.** Its two recoveries take different validation and rendering branches — a resolved trunk offers `--integrated-as`, an unresolved trunk directs the reader to resolve trunk first — and reconciliation can emit one alert per released-but-unproven reservation even though each adds zero git queries. Cross both recovery variants with one alert and with twenty in `lost_evidence_post_tool_use_cost_is_bounded`, recording wall time, child process count, and hook-level git argv for each cell.

**Separate the two temperatures the word "cold" hides.** Process-cache temperature (first invocation versus a warmed page cache) and durable proof-cache state (a cache entry that has never been evaluated versus one already stored in the journal) are independent, and a matrix that conflates them cannot tell a slow first run from a cache that is not working. Label every sample with both.

**Phase 11 added three rendered outcomes this matrix must carry.** PostToolUse can now receive a stale-session-mapping, stale-marker-run, or session/worktree-mismatch rejection from `drift`, each rendering a typed recovery-action set rather than a message, so each is a distinct render cost and gets its own row. They are cheap in git terms — the rejection is decided before any comparison runs — which is exactly why a matrix that omits them would publish a bound it never measured against the shim's rejection path.

Four expensive outcomes Phase 2 and its cachers introduced belong in the matrix as their own rows, each measured at both proof-cache states: trunk scoped-equivalence **positive** and **negative**, and successor-equivalence **positive** and **negative**. A miss on any of them composes roughly a dozen git invocations, so a single uncached retained reservation can consume a large share of the whole budget on its own.

Record **child executable and git argv counts alongside wall time**: clear and widen execute zero provenance `log`/`rev-list` calls; incursion attribution executes exactly one of each; a proof-cache hit executes zero equivalence invocations.

**Timing alone cannot catch renewed scaling — carry Phase 3's cardinality guard into the matrix.** A one-reservation sample can stay comfortably inside 0.20 seconds while a per-reservation git call has quietly returned, and this matrix's samples are exactly that shape. Phase 3 shipped the guard that catches it — `distinct_cold_proof_subjects_are_bounded_to_one_git_evaluation_per_target` (`tests/board.rs:2739`) asserts **exact** totals on a cold pass: 14 git argv for an equivalent target, 13 for a different one, byte-identical command-name sequences between the one-reservation and twenty-reservation traces, and **zero** `merge-base --is-ancestor` invocations. Add the same dimension to this matrix for both cache-miss rows: trunk scoped-equivalence and successor scoped-equivalence each report exact argv totals and identical command sequences for one reservation and for twenty, and the two totals must be equal. A row that meets its wall-clock bound but whose twenty-reservation total exceeds its one-reservation total fails this phase.

**Files:**
- `crates/cargo-berth/tests/edges.rs` — the shipped non-scaling guards starting at L721 and L814
- `crates/cargo-berth/tests/board.rs` — the shipped cost guard at L2739
- `/Users/natemccoy/.claude/scripts/berth/install/hooks/berth_post_bash.sh` — reduce fixed process count where the measurement shows it dominates
- `crates/cargo-berth/src/drift/execution.rs` — any engine-side cost the measurement isolates
- `crates/cargo-berth/src/git/mod.rs` — the scoped-equivalence invocations, if the measurement shows the cold path over budget
- `crates/cargo-berth/src/reconcile.rs` — the per-reservation reconciliation pass and its cache consultation
- `crates/cargo-berth/src/reservation/mod.rs` — the proof caches, if their key or storage is what the measurement isolates
- `crates/cargo-berth/tests/drift.rs` — the subprocess-count assertions
- `crates/cargo-berth/tests/gate.rs` — the git `reference-transaction` hook-path cost oracle; it drives a real installed git hook but never executes `berth_post_bash.sh`
- `/Users/natemccoy/.claude/scripts/berth/tests/test_hook_rendering.py` — created by Phase 12; extended here with the full-shim timing cells, the only place the PostToolUse path is actually executed

**Constraints from prior phases:** **Must follow Phases 3, 4, 5, 8, 9, 10, 12, and 14** — each changes what this path costs or what it must render. Phase 3 cached the scoped-equivalence proof so reconcile stops issuing a per-reservation diff on every call. Phase 4 added the successor-equivalence query and its own cache to the same reconcile pass, and batched four costs that previously scaled: predecessor ancestry, worktree ahead/behind, retention-ref availability, and retention-ref repair are each one invocation regardless of how many reservations are involved, with the successor round-robin admitting exactly one cold scoped comparison per reconciliation. Phase 5 added the lost-evidence alert, which this phase must time as one of its outcomes. Phase 8 filtered the generated git-hook script by phase and ref before spawning the binary, at a fixed cost of two classifier children per hook fire — one `LC_ALL=C grep -q` byte scan and one `awk` pass — independent of ref count. Phase 9 stopped berth's own retention-ref writes from firing `reference-transaction` and batched the retention-ref deletion path. **Those two facts compose into a requirement this phase must assert:** after Phase 9, an internal retention-ref write spawns **zero** Phase 8 classifier children, not merely zero `cargo-berth` invocations. The `grep` and `awk` run inside the git hook, before the PostToolUse shim is reached at all, so they sit outside the 0.20-second shim bound and a budget measured only at the shim would never see them. **The "8 spawns per drift run" figure this Work Order was written against is stale** after Phases 4 and 8; measure it, do not quote it. Phase 10 gave provenance a fixed subprocess count and moved the cardinality oracle into `tests/gate.rs`, which measures the git `reference-transaction` hook path; this phase asserts against the same suite for the same reason. **That suite is not the PostToolUse shim oracle** — it never executes `berth_post_bash.sh`. Phase 10 is settled as engine- and git-hook-side: it creates no external shim fixture, and Phase 12 owns creating `test_hook_rendering.py`, which this phase extends for full-shim timing. This phase's 0.20-second bound must therefore state which of the two paths it was proved against rather than naming both as one. **`tests/board.rs` and `tests/drift.rs` measure the engine below the hook and cannot see what a user pays for** — Phase 4 changed the hook path, named no `gate.rs` line in its own gate, and shipped a checkpoint carrying a failing `gate.rs` cost assertion that Phase 5 had to repair. Phase 12 changed what the shims render for every rejection. Phase 14 replaced the shims' hand-kept `jq` validation with generated fragments, which is what actually executes per call. **Phase 14 shipped two facts about `berth_post_bash.sh` that this phase's protocol must state.** First, the shim reads `/Users/natemccoy/.claude/scripts/berth/generated/envelope_validation.jq` from disk once at startup and splices it into the `jq` program, so per-call cost now includes that read on every invocation, not a compiled-in constant. Second, the shim has a new `generated_envelope_validation_installation_state` branch: when that file is unreadable it emits STOP feedback and **returns before the engine is invoked at all**, which is a dramatically cheaper path than any measured outcome. A bound quoted without naming which state it was measured in is meaningless, so **every timing sample names its installation state**: the published engine bound is the `Ready` state, and `NeedsRepair` is measured as its own non-engine row reported beside that bound rather than folded into it. Phase 14 also refreshed the engine and both derived consumer artifacts through one command, `/Users/natemccoy/.claude/scripts/berth/install/install.sh`; use it so a timing run cannot measure a new shim against a stale binary. Measuring before these land measures the wrong thing. **Phase 9 suppressed hooks on retention-ref writes without touching the command line.** Suppression is delivered through `GIT_CONFIG_COUNT` / `GIT_CONFIG_KEY_n` / `GIT_CONFIG_VALUE_n` environment variables rather than a `-c core.hooksPath=/dev/null` argument, specifically so that recorded argv stays byte-identical and the traces this phase compares remain comparable across the change. A later move of that setting onto the command line would shift every recorded trace, and Phase 9 already ships the guard that catches it: `assert_one_suppressed_ref_transaction` (`tests/gate.rs:4285`) pins the transaction's exact argv as `git --no-optional-locks update-ref --stdin`, so inserting `-c core.hooksPath=/dev/null` fails it immediately. Treat the delivery mechanism as load-bearing, and treat that assertion as a guard this phase inherits rather than one it has to build.

**Pending decision: whether ledger-projection bounding becomes its own Work Order before this phase.**

Since this decision was written, Phase 4 raised the durable event rate again: it appends a definitive record for every successor-head verdict and an attempt record for every transient successor comparison that cannot be cached. Its 512-entry retention bounds replayed state only — neither the append-only journal nor the projection is compacted by it. The two choices below are unchanged; the growth they weigh is larger than stated.

The architect review after Phase 3 holds that this phase cannot prove its 0.20-second bound for the lifetime of a repository while `Projection` stores `events: Vec<JournalEvent>` (`ledger/projection.rs:40`) and `Projection::from_replay` clones, serializes, and fsyncs the complete event vector on every publish. Reconciliation runs on the PostToolUse path, so that per-edit cost grows with the total number of journal events ever written, and Phase 3 raised the rate: `ScopedPatchEquivalenceChecked` records one event per `(reservation, trunk target)` and fires even when the reported status does not change.

The growth predates this plan and no phase owns it. Phase 3's non-scaling invariant constrains git subprocess counts, not ledger size, and its bounded per-reservation retention cannot reach durable state. The proposed fix is journal compaction or a projection that stores replayed facts instead of raw events — a change to `ledger/`, not a repair to any phase's diff.

- **Insert a dedicated Work Order before this phase.** It preserves the append-only journal, stores bounded replay state, and tests a small journal against a long one. This phase then measures a path whose cost does not grow without bound, and its 0.20-second claim survives repository age. The cost is one more phase and a change to the ledger's most load-bearing type.
- **Leave it in the backlog and scope this phase's bound to a fresh repository.** No new phase; this phase's Acceptance gate states explicitly that the bound is proved at the journal sizes the fixtures produce and says nothing about a long-lived repository. The published bound is then narrower than it reads today.

This reaches the user because inserting a phase changes the plan's spine and its remaining scope, and because the second option narrows a bound the plan has already published. The matching backlog item in `berth-fix-next.md` is held pending this decision and is removed only if the first option is taken. Until it is settled, this phase runs as written.

**Equal subprocess counts do not imply a stable wall clock.** Phase 4's `ahead_behind_for_heads` (`git/mod.rs:1276`) and `descendant_commits` (`git/mod.rs:1590`) each hold a fixed invocation count, but both consume union ancestry and recompute ancestor sets per head, so their cost grows with history depth and branch divergence rather than with subject count. A 0.20-second bound proved only on shallow fixture repositories is not the bound this phase claims to publish. Add shallow, deep, and divergent-history cells at one subject and at twenty to the timing protocol; if a cell cannot be met, narrow the published bound in the phase's own words to the repository sizes actually measured rather than leaving the wider claim standing.

**Acceptance gate:** All seventeen engine outcomes — the original nine, trunk-equivalence positive and negative, successor-equivalence positive and negative, Phase 11's three coordination-identity rejections, and Phase 14's typed replay failure — run with `generated_envelope_validation_installation_state=Ready` and finish within 0.20 seconds across five samples at each combination of process-cache temperature and durable proof-cache state, from independently restored state. The replay row uses the largest generated replay-failure fixture and verifies typed reason, subject, hard-stop effect, and operator-route rendering with `message` unread. **The `NeedsRepair` early return is its own row, not an exclusion note.** It runs five cold and five warm samples against a fixture shim whose generated validator is missing or unreadable, finishes within 0.20 seconds, invokes `cargo-berth` zero times, and records durable proof-cache state as not applicable because it returns before the engine is reached; it is reported beside the engine bound rather than inside it. Child executable and git argv counts are recorded per outcome and match the stated expectations. Both scoped-equivalence cache-miss rows additionally report exact git argv totals and identical command-name sequences for one reservation and for twenty, with the two totals equal, matching the guard at `tests/board.rs:2739`. `post_tool_use_git_subprocess_count_is_cardinality_invariant` in `tests/gate.rs` asserts that same equality through the hook path itself. The lost-evidence row runs `lost_evidence_post_tool_use_cost_is_bounded` across both recovery variants at one alert and at twenty, with equal git argv totals and identical command sequences. **Phase 7's board read gets its own row, varied on incident cardinality independently of reservation count** — one incursion incident against fifty, outstanding and resolved alike, holding reservations fixed — because `incursion_sections` serializes a record per retained incident and so consumes budget without consuming subprocesses. Both cells stay inside 0.20 seconds and report equal git argv totals; if the budget fails, the repair is one batched incident-status query, never one call per incident, path, or reservation. The timing protocol runs its shallow, deep, and divergent-history cells at both cardinalities, and the phase reports the repository sizes its published bound is proved against. **The globally registered hook resolves whatever binary is installed, which is routinely older than the build under test, so a timing run can measure stale behavior and report it as this phase's result.** The measurement setup pins the freshly built binary before timing begins and proves it did: a preflight mutation's journal record carries `identity_inputs.status = "recorded"`, which only a binary carrying Phase 6 writes. An internal retention-ref write after Phase 9 spawns zero `grep` and zero `awk` children from the generated git hook, proved on the same raw unfiltered argv trace rather than by counting `cargo-berth` invocations. The published bound names the path it covers — the git hook, the shim, or both — and does not claim a path it never executed. The published bound names the largest recovery-action set it was proved against, and both mismatch shapes — session and worktree — are measured as their own cells rather than folded into one identity-rejection row. The shim fixtures pass, including Phase 12's recorded-incursion, outstanding-incursion, malformed-board, and contradictory-board cases, which this phase inherits as timing subjects rather than rewriting; **Every `Test` command in Delegation Context** and `bash ~/.claude/scripts/delegate/verify.sh lint cargo-berth` green.

### Phase 16 — Semantic roles and bounded optionality  · status: todo

#### Work Order

**Goal:** Eighteen types name what they are, and every bare `Option<T>` carrying a domain state becomes a semantic type at its boundary.

**Spec:**

`PriorClassification` does not name its pre-lock foreign-path role; `ReservationRow` names a display representation rather than what it holds; `CommandExecution` does not state who owns presenting the result; and `GitCommandExecution` (`git/command.rs:25`), added by Phase 2, names an action when its actual guarantee is whether a completed process output exists — `CouldNotRun`, which carries the spawn `io::Error`, versus a completed `Output`, which is precisely an availability, not an execution. `overlap_authorization_request` exposes six bare `Option<T>` parameters and `EditAuthorization::resolve_from_sources` accepts `Option<OsString>`, so readers must infer overlap-selection and environment-identity states from representation and control flow. Two further bare optionals carry domain states of their own: `comparable_worktree` (`drift/execution.rs:287`) returns `Result<Option<WorktreeId>>`, which collapses "identity is missing" and "comparison is deferred pending a rewrite" into the same absent value; and `first_touch_disposition_description` (`output.rs:2235`) returns `Option<String>`, leaving "no disposition applies" indistinguishable from "a disposition applies but has no text".

Phase 3 shipped four more names that state their representation rather than their semantic role. `ScopedPatchEquivalenceCache` (`reservation/mod.rs:122`) is not a cache in the sense the name implies — it is a bounded set of retained verdicts for specific targets. `ScopedPatchEquivalenceCacheLookup` (`reservation/mod.rs:128`) names the act of looking up; its two cases say whether a verdict is available. `ScopedPatchComparisonAttemptHistory` (`reservation/mod.rs:154`) is not a history anyone reads back — it is the round-robin schedule that decides which target is compared next. `ScopedPatchEvaluationMemo` (`reconcile.rs:328`) is not a memo — it is the one-comparison-per-target budget for a single reconciliation pass, and its lifetime is exactly that pass.

Renames:
- `PriorClassification` → `PreLockForeignPathClassification`
- `ReservationRow` → **`BoardReservationSnapshot`**
- `CommandExecution` → `CommandOutputOwnership::{CallerRendersResponse, BoardPresentedAndTerminalRestored}`
- `GitCommandExecution` → `GitCommandOutputAvailability::{Available(Output), Unavailable(io::Error)}` — **`Unavailable` keeps the `io::Error`.** Today's `CouldNotRun` carries it and `GitError` is constructed from it, so a fieldless `Unavailable` would silently discard every spawn diagnostic; the acceptance gate proves those diagnostics are unchanged.
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
- `LocalBranchReplacementTipMatches` → `LocalBranchRenameTargetResolution`, variants `{NoMatches, ExactlyOne(FullRefName), MultipleMatches}` → `{NotProven, Unique(FullRefName), Ambiguous}`
- `alert::RecoveryAction` → `LostEvidenceRecoveryCommand`
- `recovery::RecoveryAction` → `PostCommitRecoveryMarkerAction`

**Phase 5 left two unrelated types sharing one name.** `alert::RecoveryAction` (`alert.rs:165`) names the command a reader runs to prove a release integrated; `recovery::RecoveryAction` (`recovery.rs:686`) names what a post-commit marker asks the harness to do. Rust keeps them apart by path, but a reader meeting either one in isolation cannot tell which concept is in play, and neither name states its own role. Rename both for what they are.

**Phase 4's names are not already semantic — they are the same representation-over-role pattern, duplicated.** Phase 4 mirrored Phase 3's cache model for the successor path and inherited its naming with it: a "cache" that is a bounded set of retained verdicts, a "lookup" whose cases state availability, an "attempt history" nobody reads back that is really the round-robin schedule, and a "budget" whose lifetime is one reconciliation pass. Renaming the trunk-side four while leaving their successor twins untouched would be worse than renaming neither, because the two halves would then disagree about what the same structure is called. `DescendantCommitQuery` (`git/mod.rs:~1642`) names an act where its variants state a classification outcome — `Classified` versus `AncestorObjectUnknown`.

**Phase 8's resolution type names a count where its callers need a guarantee.** `LocalBranchReplacementTipMatches::{NoMatches, ExactlyOne, MultipleMatches}` (`git/mod.rs`, beside `local_branch_replacement_tip_matches`) states how many candidates survived, but the only thing the detached refresh worker may act on is whether exactly one rename target was *proven* by reflog subject — a candidate sharing the deleted tip without that proof is not a match at all. `LocalBranchRenameTargetResolution::{NotProven, Unique(FullRefName), Ambiguous}` states that guarantee directly, and `Ambiguous` says why the hook is left untouched where `MultipleMatches` only says how many there were. Phase 8 introduced no new domain-boundary `Option<T>`; this is its only entry here.

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
- **New at the clap boundary (not a rename):** `OverlapSelection::{NoOverlapRequested, RequesterBeforeHolder { blocker_reservation_id, authorization_reason, proposal_submission }, RequesterAfterHolder { blocker_reservation_id, authorization_reason, proposal_submission }, Defer { blocker_reservation_id, authorization_reason, proposal_submission }, Override { blocker_reservation_id, authorization_reason, proposal_submission }}`, converted into the existing `OverlapAuthorizationRequest` before any internal helper receives them. `Absent` would name the representation; `NoOverlapRequested` names the domain fact. **`Before` and `After` name a direction without saying whose**, which is exactly the caller knowledge this phase exists to remove — a reader has to find a call site to learn whether the requester goes before or after the reservation already holding the scope. `RequesterBeforeHolder` and `RequesterAfterHolder` state it in the name. The three fields state their roles for the same reason: `id` could be either party's, `reason` could be a refusal's, and `proposal` could be a stored record — `blocker_reservation_id`, `authorization_reason`, and `proposal_submission` each say which. Each permissive variant owns its reservation id **and** its reason and proposal, so the type covers all six of the bare `Option<T>` parameters rather than four — a variant set that drops the ids, or that leaves reason and proposal beside the enum, just moves those parameters one level in.
- `EnvironmentCoordinationRunSelection::{NotSupplied, UnusableFallbackToMarker, Identified(id)}` replacing `Option<OsString>` in `EditAuthorization::resolve_from_sources`. A bare `Invalid` names the input's defect but not the guarantee that follows it; `UnusableFallbackToMarker` states the marker-fallback policy the serialized behaviour depends on, which is the whole reason the variant is not simply an error. The type is named for the coordination run it selects, not for the environment it was read from: `EnvironmentRunSelection` reads as a selection among environments.

These are two independent boundaries, not one — overlap selection and environment identity fail differently and are consumed by different callers.

`OverlapSelection` covers **all six** of `overlap_authorization_request`'s optional parameters, not the four that name a reservation; a fifth and sixth left bare would keep the caller inferring state from representation for exactly the reason this phase exists. **Every permissive selection variant carries the reason and the proposal submission with it**, rather than leaving them as two bare parameters beside a four-selector enum — otherwise the type covers four of six optionals and the caller still infers the remaining two from representation. `comparable_worktree` (`drift/execution.rs:287`) returns `WorktreeComparability::{Comparable(WorktreeId), IdentityUnavailable, DeferredPendingRewrite}` — three states, because "identity is missing" and "comparison is deferred pending a rewrite" are different facts that today share one absent value. `first_touch_disposition_description` (`output.rs:2235`) returns `FirstTouchDisposition::{NotApplicable, Described(String)}`, separating "no disposition applies" from "a disposition applies and here is its text". `None` would restate absence; `NotApplicable` states which of the two facts holds.

Leave bare `Option<T>` only in clap-owned fields and externally required trait signatures. Keep serialized payloads unchanged.

**The eighteen renames are global and mechanical, and the user performs them.** This is a standing preference, not a suggestion: their editor applies a project-wide rename accurately in one action, where a hand-applied sweep across this many symbols is slow and error-prone. The orchestrator presents the eighteen old-to-new pairs and waits for the user to apply them; implementation work in this phase is everything else — the new semantic types, the boundary conversions, and the audit — and none of it is blocked on the renames landing first.

**Files:**
- `crates/cargo-berth/src/drift/classification.rs` — `PreLockForeignPathClassification`
- `crates/cargo-berth/src/board/mod.rs` — `BoardReservationSnapshot` (L135, L797)
- `crates/cargo-berth/src/cli.rs` — `CommandOutputOwnership`, `OverlapSelection`
- `crates/cargo-berth/src/ledger/mod.rs` — `EnvironmentCoordinationRunSelection`
- `crates/cargo-berth/src/git/command.rs` — `GitCommandOutputAvailability` (~L16)
- `crates/cargo-berth/src/reservation/mod.rs` — `RetainedScopedPatchTargetVerdict` (L118), `RetainedScopedPatchTargetVerdicts` (L126), `ScopedPatchTargetVerdictAvailability` (L132), `ScopedPatchTargetEvaluationSchedule` (L193), and Phase 4's successor twins: `RetainedSuccessorScopedPatchTargetVerdict` (L153), `RetainedSuccessorScopedPatchTargetVerdicts` (L161), `SuccessorScopedPatchTargetVerdictAvailability` (L167), `SuccessorScopedPatchTargetEvaluationSchedule` (L236)
- `crates/cargo-berth/src/reconcile.rs` — `ReconciliationScopedPatchEvaluationBudget` (L332) and `ReconciliationSuccessorScopedPatchEvaluationBudget` (L342)
- `crates/cargo-berth/src/git/mod.rs` — `GitCommandOutputAvailability` call sites, plus `ProtectedTipSuccessorHeadClassification` (L1531) and its call sites, including the `drift/git_output.rs` facade Phase 10 adds
- `crates/cargo-berth/src/alert.rs` — `LostEvidenceRecoveryCommand` (L165)
- `crates/cargo-berth/src/recovery.rs` — `PostCommitRecoveryMarkerAction` (L686)
- `crates/cargo-berth/src/drift/execution.rs` — the semantic result replacing `comparable_worktree`'s `Result<Option<WorktreeId>>` (L284)
- `crates/cargo-berth/src/output.rs` — the semantic result replacing `first_touch_disposition_description`'s `Option<String>` (L2235)

**Constraints from prior phases:** Phase 1 made `edit_blocking_status` a computed method on `Reservation` and removed the retained field; `BoardReservationSnapshot` populates its field by calling that method, which the shipped `reservation_visibility` already does — the requirement is satisfied and this rename must not reintroduce stored state. Phase 1 also left `BoardReservationVisibility::ReblockedActiveConstraint` in place as a reserved wire value that is unreachable for a released reservation; the rename keeps the variant. Phase 2 added `IntegrationProof` inside `IntegrationEvidenceStatus::Integrated`, which `BoardIntegrationEvidence` surfaces, and introduced `GitCommandExecution` — the fourth rename subject, whose two cases are a completed process output and no output at all. Phase 3 shipped `ScopedPatchEquivalenceCacheLookup` (`Hit`/`Miss`) as a semantic state rather than a bare optional, and `DeferredScopedPatchIntegrationStatus` (`StillValid`/`Degraded`) likewise — those need no optionality repair, but four of Phase 3's names state representation rather than role and are renamed above. Phase 9 adds `GitHookExecutionPolicy`, specified as a semantic state rather than a boolean, so this phase audits rather than repairs it. **Phase 4's successor cache was specified as a semantic state but named on Phase 3's representation-first pattern**, so its seven types are renamed here alongside their trunk-side twins; Phase 4 introduced no new bare domain `Option<T>`. Phase 11 introduced `CoordinationIdentityRejection` — do not duplicate its identity concepts in `EnvironmentCoordinationRunSelection`. **Phase 11 moved the authorization off the mutation actor**, so Phase 6's original statement no longer describes the code: `ResolvedEditAuthorization` now carries the coherent authorization resolved in one read, and `journal_mutation_actor_for` produces an authorization-free `ResolvedJournalMutationActor` from it. `with_coordination_run_id` no longer exists — do not key any rename or call-site sweep on it. `EnvironmentCoordinationRunSelection` is still converted **before** that single read and never re-derived after it; a rename that reintroduces a second resolution undoes the one-read guarantee. **`RecoveryCommandLine`'s representability is owned by Phase 11's F008 repair, not by this phase** — it already guarantees a published `argv` is runnable and omits the action otherwise, so do not re-open that shape here. Phase 6's own diagnostic enums (`JournalMutationIdentityInputs`, `InvocationDirectoryAtMutation`, `EnvironmentValueAtMutation`) already name their states and introduce no bare `Option<T>`, so they are not rename subjects. **Phase 10 renamed only `GitCommandExecution`'s variants to `Completed` and `CouldNotRun`**, which state the outcome rather than the representation; the type name itself still states an action where its actual guarantee is whether a completed process output exists, so it remains a rename subject. Apply the planned `GitCommandOutputAvailability::{Available(Output), Unavailable(io::Error)}` rename above, preserving the carried `io::Error` and byte-identical diagnostics. Phase 10 also introduced three attribution states that already name their roles and carry no bare `Option<T>` — `IncursionAttributionAnchorState` (`UsableAncestor`/`NotAncestorOfHead`/`ObjectUnknown`), `IncursionCommitOriginBasis` (`ResolvedTrunk`/`CannotClassifyOrigin`), and `IncursionCommitOriginMembership` (`Classified`/`CannotClassifyOrigin`) — so this phase audits them rather than repairing them. **Phase 13's two new types already satisfy this phase's contract and are audit subjects, not rename or repair subjects.** `ReservationLifecycleSnapshot` (`reservation/mod.rs:529`) names the reading it is — one reservation's lifecycle as the board projects it — and its four alternatives state their own states; `BoardOutputSelection::{CompleteBoard, ReservationLifecycleFor(ReservationId)}` (`verb/board.rs`) is the semantic type that keeps the query out of the internals. Phase 13's only bare `Option<T>` is `BoardArguments.reservation: Option<ReservationId>` in `cli.rs`, which sits **solely** at the clap boundary and is converted into `BoardOutputSelection` by `into_output_selection()` before any internal function sees it — exactly the clap-owned exemption this phase's rule already grants, so it stays as it is. **Phase 14 removed this phase's only cross-phase ordering risk.** The generated wire contract pins every `schemars` definition name to its wire name rather than the Rust identifier, and a test renames a Rust type and asserts every generated artifact is byte-identical — so none of this phase's renames can move `docs/cargo-berth/generated/output-contract.json` or either derived consumer artifact. Run the byte-compare test after the renames as confirmation, not as a risk to manage. Phase 14 did add three new rename-audit subjects in `output.rs`, each already naming its role: `OutputPayload` and its `OutputFacts` union, `ReplayFailurePayload` with `ReplayFailureSubject` (`Reservation`/`IncursionIncident`), and `LostIntegrationEvidenceStatus`, whose three variants make the fourth wire value unrepresentable rather than merely unemitted. Audit them; do not rename them.

**Acceptance gate:** **Every `Test` command in Delegation Context** and `bash ~/.claude/scripts/delegate/verify.sh lint cargo-berth` green. Spawn diagnostics are byte-identical before and after the `GitCommandOutputAvailability` rename, proved by a fixture that makes the git binary unspawnable and compares the resulting `GitError` text. Serialized payloads are byte-identical before and after — proved by the existing board and drift JSON fixtures — and so are Phase 14's generated contract artifacts, since its schema definition names are pinned to wire names rather than Rust identifiers. No bare `Option<T>` remains in `overlap_authorization_request`, `EditAuthorization::resolve_from_sources`, `comparable_worktree`, or `first_touch_disposition_description`; all six of `overlap_authorization_request`'s optional parameters are covered and no reservation id is dropped on the way through `OverlapSelection`. An audit of the caches and policies added by Phases 3, 4, and 9 confirms none carries a bare `Option<T>` for a domain state or a representation-level boolean for the hook policy; any that does is repaired here. All eighteen renames are applied with no other change: serialized field names and wire values are unaffected, because none of the Phase 3 or Phase 4 types is serialized under its own name. **Phase 11 split `ResolvedJournalMutationActor` into a resolved-authorization state and a mutation-actor state**, so any constraint here that still names the single combined type refers to a type that no longer exists; key on Phase 11's two names instead.

### Phase 17 — Typed coordinator classifiers  · status: todo

#### Work Order

**Goal:** The Python coordinator returns tagged unions instead of `dict[str, object]` reached through `cast`.

**Spec:**

`classify_claim` (`claim_state.py:1485`), `classify_check` (`:1714`), `_validate_board` (`:1800`), `render_board` (`:1893`), and `coordinator_state` (`:2190`) all return `dict[str, object]` and reach it through `cast`, so every tagged union the coordinator builds is erased at the one boundary a reader inspects. `ProposalAwaitingApprovalStateValue.proposal` is a bare `dict[str, object]` for the same reason. The weakness predates the coordinator work — `classify_claim` returned `dict[str, Any]` at `831e34a` — and repairing only one symbol would leave an inconsistent surface.

The coordinator's state classifiers return a tagged union rather than `dict[str, object]`; the locked proposal carries a semantic type validated at envelope conversion; and no `cast` stands between a tagged value and its return. Name the returns rather than leaving the implementer to invent them: `classify_claim` returns `ClaimClassificationValue`, `classify_check` returns `CheckClassificationValue`, `_validate_board` returns `BoardValidationValue`, `render_board` returns `RenderedBoardValue`, and `coordinator_state` returns `EngineInvocationStateValue` — the value is the coordinator's reading of one engine invocation, and the old `Generic` in its former name said how many cases it covers rather than what it is. **`coordinator_state` is the only classifier to retype here, not two.** Phase 13 renamed `_generic_state` to the public `coordinator_state`; there is no longer a private twin, and any instruction to retype both names a symbol that no longer exists. Phase 13 also made `installed_engine_binary` (`:1968`) public, and both are production entry points rather than test helpers — `_invoke_engine` calls `installed_engine_binary` and `invoke` dispatch calls `coordinator_state` — so this phase owns `coordinator_state`'s semantic return union. `installed_engine_binary() -> str` is already precisely typed and needs no work here. `ProposalAwaitingApprovalStateValue.proposal` carries `ValidatedOverlapProposalValue` — an overlap-authorization proposal, not a proposal in general, and a `…Value` like every other member of this surface, and the alert union is `EnvelopeAlertValue` with a named member per alert the engine emits. **Naming the return aliases is not enough on its own: the erasure that produces the ordering hazard is upstream of them.** `EnvelopePayload.data` is still an untyped mapping (`claim_state.py:87`, `NotRequired[dict[str, object]]`), so every classifier re-inspects an untyped mapping and the selection between rejections is made by the order of `if` returns rather than by the shape of the value. **The repair is the payload type itself, not one status arm of it.** At the JSON boundary, convert `EnvelopePayload` into a semantic union whose variants distinguish no payload facts, each classifier-owned typed fact, and an explicitly uninspected residual; no domain-owned payload type retains a `NotRequired` member, because representation-level absence is precisely the protocol state this phase exists to name. Its `invalid_input` reading covers Phase 11's three identity rejections, Phase 7's `already_recorded_by_different_coordination_actor`, **Phase 13's `ReservationLifecycleQueryRejection::UnknownReservation` as its own `UnknownReservationLifecycleQueryValue` member** (`payload.kind = reservation`, `data.status = unknown_reservation`), and a residual generic member; its `ledger_unreadable` reading carries Phase 14's replay failure under the `(ledger_unreadable, replay_failure)` tuple described in the constraints below, reusing the shipped replay types rather than rebuilding them. Select with an exhaustive match, and replace `ReplayFailure.tagged()`'s `dict[str, object]` conversion with an explicit replay-hard-stop member in every affected return union — a tagged type erased at its own conversion is the same defect as an untyped classifier. **That member is a wire-rejection reading and must stay distinguishable from the normalized coordinator state Phase 13 already ships.** `UnknownReservationLifecycleValue` (`claim_state.py:181`) is the *normalized* form the `reservation` operation returns, keyed `kind = unknown_reservation`; the envelope union's member is the *wire* rejection, keyed `data.status = unknown_reservation` under `payload.kind = reservation`. They carry the same fact in two roles, and creating two similarly named "unknown lifecycle" types would leave a reader unable to tell which one a value is. Keep Phase 13's normalized type under its existing name, give the wire member the query-specific `UnknownReservationLifecycleQueryValue` name this phase already planned, and convert between them at exactly one explicit site. Phase 13 committed to that outcome being typed; routing it through the residual generic member would erase it at the one boundary this phase exists to type. Every branch is gated with `message` unread. Done this way the ordering constraint below is discharged by construction rather than preserved as a comment.

**`EnvelopePayload.alerts` is the same erasure and is included here.** It is typed `list[object]` at `claim_state.py:86`, so every alert the engine emits — Phase 5's lost-evidence alert and both its recovery forms among them — reaches a caller as an untyped value it must inspect by hand, which is precisely what this phase removes everywhere else. Either model the alert union, both `LostEvidenceRecovery` variants included, or convert it at validation into a named state that says the coordinator deliberately ignores alert contents. A `list[object]` surviving the tagged-union phase is not a third option.

**One tag Phase 12 left open closes here.** `RenderedCoordinationIdentityRecoveryActionValue` (`claim_state.py:611`) types its `kind` as `str` (`:614`) where the four recovery-action tags are closed literals. That was implementation-only while `tagged()` erased the whole value to `dict[str, object]` — nothing downstream could match on it. This phase removes that erasure, which turns the tag into externally typed domain state, so it becomes the closed four-tag literal (or a rendered union with one member per action) at the same time. Widening a closed set to `str` at the moment it becomes matchable would leave the one hole this phase exists to close.

**Phase 13 already built part of this surface — lift or rename it, never recreate it.** `ReleaseDispositionValue` (`claim_state.py:129`) is the four-member `Literal`-tagged union over `integrated`, `rewritten_integration`, `abandoned`, and `retired_orphan`, with `integrated` correctly carrying no `evidence` member; `ReservationLifecycleValue` (`:165`) is the four-member lifecycle union; `ReservationLifecycleQueryStateValue` (`:188`) is the normalized query state; and `reservation_lifecycle_state` (`:2105`) is the strict validator that produces it, with `_release_disposition` (`:2028`) beneath it. All of that already satisfies this phase's contract. **What is left untyped, and is this phase's actual work:** `EnvelopePayload.data: NotRequired[dict[str, object]]` (`:87`) and `EnvelopePayload.alerts: list[object]` (`:86`); the five classifier returns above; `RenderedCoordinationIdentityRecoveryActionValue.kind: str` (`:614`); `ProposalAwaitingApprovalStateValue.proposal` (`:374`); and every `cast` standing between a tagged value and its return or dispatch.

**The coordinator and its shim oracle are versioned in a separate repository.** `/Users/natemccoy/.claude` is itself a Git checkout, so this phase can read its own diff there and must run project-root tooling from that root — `~/.claude/pyrightconfig.json` supplies the `executionEnvironments` the type check depends on, and running basedpyright from anywhere else silently applies different settings. Those files still cannot join a `cargo-berth` checkpoint commit; the existing commit boundary is unchanged.

Project rules that bind here: never use file-level type ignores; avoid `Any` — annotate all signatures, use `TypedDict` for dicts with known keys, and for stdlib `Any` returns (`json.loads()` etc.) annotate with a `TypedDict` or specific type. Line-level `# pyright: ignore[reportAny]` is a last resort on the specific line only.

**Files:**
- `/Users/natemccoy/.claude/scripts/berth/claim_state.py` — the five classifiers, the envelope `data` and `alerts` members, the recovery-action tag, and the proposal type
- `/Users/natemccoy/.claude/scripts/berth/tests/test_hook_rendering.py` — created by Phase 12 and extended by Phases 13 and 14 to fourteen tests, including typed replay routing, unreadable-validator diagnostics, and installer rollback ordering; extended here with the classifier fixtures this phase's gate names

Both files live in the `/Users/natemccoy/.claude` Git checkout, not in this repository. Read their diff there, run every type check and test with `/Users/natemccoy/.claude` as the project root, and expect no part of this phase to appear in a `cargo-berth` checkpoint commit.

**Constraints from prior phases:** Phase 12 already added typed rendering of `CoordinationIdentityRejection` and its `recovery_actions` to this file, and Phase 13 added the `reservation` entry point, its strict validator `reservation_lifecycle_state`, the four-member `ReleaseDispositionValue`, the four-member `ReservationLifecycleValue`, the normalized `ReservationLifecycleQueryStateValue`, and the public `coordinator_state` and `installed_engine_binary`. Phase 13's value types are already correct — lift or rename them, never rebuild them. Phase 14 generated `STATUS_PAYLOAD_KINDS` and `FIXED_STATUS_EXIT_CODES` from the engine contract — they are now imported from `berth.generated.status_payload_tables` (`claim_state.py:22`, `:27`) and are inputs here, not hand-edited targets. **Phase 14 also added a sixth classifier this phase must type, and it does not arrive under `invalid_input`.** `_replay_failure_classification` (`claim_state.py:2375`) returns a `ReplayFailure` (`:739`) reached through `.tagged()`, and it is called from all four classification entry points — `classify_claim` (`:1493`), `classify_check` (`:1733`), `_validate_board` (`:1801`), and `coordinator_state` (`:2196`) — ahead of each one's generic `ledger_unreadable` branch. It keys on `envelope.status == "ledger_unreadable"` with `payload.kind == "replay_failure"`, so it belongs to the `ledger_unreadable` reading rather than the `invalid_input` union enumerated above, and its `ReplayFailureValue` (`:730`) already exists — lift it into the typed surface rather than rebuilding it. The generic `ledger_unreadable` branch that remains beneath it in each of those four functions is still reachable for an untyped replay failure and must stay below the typed classifier for the same ordering reason the `invalid_input` branch must. Phase 14 further added `OutputStatus::LegacyHookOutdated` as its own wire status, so `ledger_unreadable` is no longer the catch-all it was when this Work Order was written. **Phase 7 left `_generic_state` order-sensitive and that ordering is a correctness constraint, not a style preference.** `sequence` and `integrate` identity rejections arrive with status `invalid_input`, so the generic `invalid_input` early return (`coordinator_state` at `claim_state.py:2190`, its generic branch at `:2218`) must stay *below* the two typed classifiers; placed above them, the typed identity states are silently erased into a diagnostic string. **`_inactive_identity_classification` no longer exists** — Phase 12 replaced it with `_coordination_identity_classification` (`:2349`) and `_foreign_actor_incursion_resolution_classification` (`:2422`); do not key any edit or call-site sweep on the old name. **Phase 12 already replaced the obsolete `inactive_session_mapping` / `inactive_marker_run` pair with a semantic `CoordinationIdentityRejectionValue` covering Phase 11's three variants in both their wire placements, plus the matching recovery-action union** — lift **both** of its value types into this phase's exhaustive envelope union rather than recreating them — `CoordinationIdentityRejectionValue` (`claim_state.py:604`) covering the three identity rejections in both wire placements, and `ForeignActorIncursionResolutionValue` (`:660`) covering the foreign-resolver rejection — and treat this part of the identity work as already satisfied. **Phase 12 also bound both classifiers to the envelope's own status**, refusing any envelope whose status is not `invalid_input`, which is the property this phase's exhaustive match must preserve rather than rediscover: a success-tagged envelope carrying a lookalike sub-status stays an ordinary engine response. A tagged union whose members are matched exhaustively removes the hazard rather than preserving the ordering — prefer that, and do not ship a version that merely re-encodes the current sequence of `if` returns.

**Acceptance gate:** `basedpyright scripts/berth/claim_state.py scripts/berth/tests/test_hook_rendering.py`, run with `/Users/natemccoy/.claude` as the working directory so its `pyrightconfig.json` applies, reports **zero errors, zero warnings, and zero notes** for **both** files — the fixture file is edited by this phase and is inside the gate, and both files are clean on the tree this phase starts from, so any diagnostic is this phase's own. `python3 /Users/natemccoy/.claude/scripts/berth/tests/test_hook_rendering.py` passes in full. No file-level type ignore exists in the file, and every remaining `# pyright: ignore` is line-level with a named rule. A fixture carrying Phase 13's `unknown_reservation` rejection reaches a caller as `UnknownReservationLifecycleQueryValue`, not as the residual generic member. Replay fixtures exercise `classify_claim`, `classify_check`, `_validate_board`, `render_board`, and `coordinator_state`; each returns a typed replay-hard-stop member carrying typed reason, subject, effect, and operator route, with no `dict` return, no `cast`, and no `message` read, while a generic `ledger_unreadable` carrying no payload facts stays distinct from it. `EnvelopePayload.alerts` is no longer `list[object]`: a fixture carrying both lost-evidence recovery forms reaches a caller as a typed value, or as an explicitly named ignored-alert state. Two fixtures carry a **success-tagged** envelope whose payload holds an identity-rejection and a foreign-resolution sub-status, and each reaches the caller as an ordinary engine response — the Phase 12 property, re-proved through the exhaustive match rather than assumed. `RenderedCoordinationIdentityRecoveryActionValue.kind` accepts exactly the four action tags, proved by an exhaustive check that fails when a fifth tag is added and when the field is widened back to `str`. Fixtures cover **every** lifecycle alternative and **every** release disposition, not the `active` and `unknown_reservation` pair Phase 13's fixture already exercises: one fixture per `active`, `outstanding`, `released_after_checkpoint`, and `released_without_checkpoint`, and one per `integrated`, `rewritten_integration`, `abandoned`, and `retired_orphan`, each reaching a caller as its own typed member.
